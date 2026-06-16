//! The lean cert-follower reconciler loop.
//!
//! One sequential loop replaces the marshal + driver + resolver + stubs
//! assembly: pull `(cert, OrderBlock)` by height from the upstream, verify
//! (parent linkage → payload==digest → BLS against the per-epoch committee →
//! `result` cross-check), derive+execute, advance two-tier FCU, rotate epoch
//! schemes inline when the cursor crosses a boundary.
//!
//! Failure policy: errors attributable to the upstream's DATA (tampered cert,
//! mismatched artifact, wrong-epoch cert) rotate to the next configured
//! upstream and re-pull — capped at [`MAX_UPSTREAM_FAULTS`] consecutive
//! faults, after which the loop halts fail-closed. Everything else (local
//! divergence, derivation/import/FCU errors) halts immediately: a follower
//! that cannot verify its own chain must not keep serving, and the
//! supervisor cancels the node.
//!
//! Under deferred execution every block must be derived sequentially from its
//! parent, so out-of-order intake buys nothing — the loop IS the order.

use super::upstream::{CertUpstream, UpstreamFinalized};
use crate::{
    application::{
        derive_with_visibility_retry, BeaconEngineLike, DerivedBlock as _, DerivedBlockBuilder,
        ExecutedChain,
    },
    digest::Digest,
    order_block::{result_final_height, result_target, ResultTarget, K},
};
use alloy_primitives::B256;
use alloy_rpc_types_engine::{ForkchoiceState, PayloadStatusEnum};
use commonware_consensus::types::Height;
use commonware_parallel::Sequential;
use commonware_runtime::Clock;
use eyre::{ensure, eyre, OptionExt as _, WrapErr as _};
use fluentbase_bls::Scheme as BlsScheme;
use rand_core::CryptoRngCore;
use std::{collections::BTreeMap, future::Future, time::Duration};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// One L1 batch = 1024 blocks = ~17 min of chain time at the committed
/// 1 blk/s (research Addendum 06-12; pacing landed in 844d3826).
/// Dual-use invariant:
/// (a) above this gap the loop re-runs the EL-sync phase instead of pulling
///     block-by-block (estimate, not a measured crossover: batched pipeline
///     sync avoids the per-height RPC round-trip + per-block engine-API cost);
/// (b) the serving-window size — so a downstream follower's repairable gap
///     and its own jump threshold coincide by construction.
pub const JUMP_THRESHOLD: u64 = 1024;

/// Poll cadence while waiting for the upstream to produce `cursor + 1`.
const PULL_RETRY: Duration = Duration::from_millis(500);

/// Sustained-starvation jump trigger: if the wanted height stays unservable
/// for this long while the upstream's tip is ahead, re-run EL-sync regardless
/// of gap size. Covers every unservable-gap shape the gap-size rule misses:
/// heights evicted by an upstream window restart, the gap == JUMP_THRESHOLD
/// boundary, and a stale `highest_live_seen` when live events were dropped.
pub(crate) const STARVATION_JUMP: Duration = Duration::from_secs(60);

/// Consecutive upstream-data faults tolerated before halting fail-closed.
/// Each fault rotates to the next configured upstream URL first — one buggy
/// or malicious upstream must not bring the follower down when a healthy
/// backup is configured; identical faults from every upstream mean the fault
/// is ours (or the chain's) and halting is correct.
const MAX_UPSTREAM_FAULTS: u32 = 3;

/// Verification failure attributable to the upstream's DATA (tampered or
/// mismatched cert/artifact) — recoverable by rotating to another upstream,
/// unlike local divergence/derivation errors which stay immediately fatal.
#[derive(Debug, thiserror::Error)]
#[error("upstream data fault: {0}")]
pub(crate) struct UpstreamDataFault(pub String);

/// Per-epoch BLS verifier source. Committees are epoch-frozen on-chain arrays
/// (immutable once committed), so reading at ANY executed state hash at/after
/// the commit yields the same committee — the loop always reads at the
/// last-executed block (`cursor.evm_hash`), which is at/after the commit
/// point for both the current epoch and the next (ahead-commit pipeline:
/// epoch E+1 is committed at the first block of epoch E).
pub(crate) trait CommitteeSource: Send + Sync + 'static {
    fn scheme_at(&self, epoch: u64, at_hash: B256) -> eyre::Result<BlsScheme>;
}

/// EL-sync seam shared by the cold-start phase and the loop's jump rule:
/// drive reth onto the attested tip of `latest` (FCU + devp2p backfill, with
/// the no-progress stall detector) and return the `(height, hash)` it landed
/// on. Implemented in `mod.rs` over the provider + beacon engine.
pub(crate) trait ElSync: Send + Sync {
    fn sync_to(
        &self,
        latest: &UpstreamFinalized,
    ) -> impl Future<Output = eyre::Result<(u64, B256)>> + Send;

    /// Whether the local chain holds `hash` canonically — the post-jump L1
    /// trust-root re-assert (same rule as the launch-time assert: wherever we
    /// land, the chain must be a descendant of the L1-finalized block).
    fn holds(&self, hash: B256) -> eyre::Result<bool>;
}

/// Verified-pair sink: the node side turns these into `CertifiedBlock`s for
/// the serving window (D4). `None` for a non-serving follower.
pub type VerifiedTx = mpsc::UnboundedSender<UpstreamFinalized>;

/// Epoch/anchor geometry, fixed at launch (`finalized_floor` re-set on a
/// jump). `activation` doubles as the ordering-chain anchor (the
/// `result_target` pre-activation window origin).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Geometry {
    pub interval: u32,
    pub activation: u64,
    /// Floor of the result-final cursor: `landing − K` (clamped to
    /// `activation`). The landing block itself is NOT claimed finalized — its
    /// result attestation arrives only K blocks later; claiming it would
    /// overstate finality by K on every restart/jump (two-tier contract).
    pub finalized_floor: u64,
}

/// Loop position: the last verified-and-executed block.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cursor {
    pub height: u64,
    pub evm_hash: B256,
    /// Ordering-chain digest of the block at `height`. `None` right after an
    /// EL-sync jump (linkage restarts; EVM-side binding via `result`).
    pub prev_digest: Option<Digest>,
    /// Last FCU-finalized hash (never ZERO — seeded with the checkpoint hash).
    pub finalized_hash: B256,
}

pub(crate) struct FollowLoop<E, BE, D, XC, U, CS, ES> {
    pub ctx: E,
    pub beacon_engine: BE,
    pub deriver: D,
    pub executed: XC,
    pub upstream: U,
    pub committees: CS,
    pub el_sync: ES,
    /// Live WS events — consumed when contiguous, otherwise a gap signal.
    pub finalized_rx: mpsc::Receiver<UpstreamFinalized>,
    pub verified_tx: Option<VerifiedTx>,
    /// Pruned to {previous, current} on rotation.
    pub schemes: BTreeMap<u64, BlsScheme>,
    pub geometry: Geometry,
    pub cursor: Cursor,
    /// Re-sent as a heartbeat during idle waits (always the LAST forward FCU —
    /// never constructs a backward one, which reth ancestor-skips).
    pub last_fcu: ForkchoiceState,
    pub fcu_heartbeat_interval: Duration,
    /// Highest live height observed — the jump-rule gap signal.
    pub highest_live_seen: u64,
    /// L1 Rollup trust root (when configured): re-asserted after every
    /// EL-sync jump, exactly like the launch-time assert.
    pub l1_checkpoint: Option<B256>,
    /// [`STARVATION_JUMP`] in production; shrunk in tests (a 60-virtual-second
    /// deterministic wait is a 60k-cycle crawl).
    pub starvation_jump: Duration,
    /// Stop cleanly once the cursor reaches this height (used by the unified
    /// supervisor to hand reth to the signer stack at an epoch boundary).
    /// `None` = run forever (standalone `--cert-follow`).
    pub stop_at: Option<u64>,
}

/// Why [`FollowLoop::run`] returned `Ok`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowExit {
    /// `stop_at` reached: the cursor sits at/past the requested height (an
    /// EL-sync jump can overshoot) and the corresponding FCU has been
    /// acknowledged — reth is at a clean handoff point. `evm_hash` is the
    /// cursor's executed hash: the supervisor reads the ahead-committed
    /// committee at exactly this state.
    StoppedAt { height: u64, evm_hash: B256 },
}

impl<E, BE, D, XC, U, CS, ES> FollowLoop<E, BE, D, XC, U, CS, ES>
where
    E: Clock + CryptoRngCore + Send + Sync + 'static,
    BE: BeaconEngineLike<ExecutionData = D::Derived>,
    D: DerivedBlockBuilder,
    XC: ExecutedChain,
    U: CertUpstream,
    CS: CommitteeSource,
    ES: ElSync,
{
    pub(crate) async fn run(mut self) -> eyre::Result<FollowExit> {
        let mut consecutive_faults = 0u32;
        loop {
            // Loop-top so every cursor mutation is covered: the seed (a
            // supervisor re-launch may race the boundary), apply(), and the
            // EL-sync jump inside next_finalized (which can overshoot the
            // boundary — the caller anchors at the actual cursor, not at
            // `stop_at`, so `>=` is the correct predicate).
            if let Some(stop) = self.stop_at {
                if self.cursor.height >= stop {
                    return Ok(FollowExit::StoppedAt {
                        height: self.cursor.height,
                        evm_hash: self.cursor.evm_hash,
                    });
                }
            }
            let uf = self.next_finalized().await?;
            match self.apply(uf).await {
                Ok(()) => consecutive_faults = 0,
                // Upstream-data faults rotate to the next configured URL and
                // re-pull the same height; identical faults from every
                // upstream mean the data (or we) are at fault — halt.
                Err(e) if e.downcast_ref::<UpstreamDataFault>().is_some() => {
                    consecutive_faults += 1;
                    if consecutive_faults >= MAX_UPSTREAM_FAULTS {
                        return Err(e.wrap_err(format!(
                            "{MAX_UPSTREAM_FAULTS} consecutive upstream data faults — \
                             halting fail-closed"
                        )));
                    }
                    warn!(
                        error = %e,
                        fault = consecutive_faults,
                        "upstream served an unverifiable pair; rotating upstream and re-pulling"
                    );
                    self.upstream.rotate().await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Verify → derive → FCU → publish → advance. Verification failures
    /// attributable to the upstream's data surface as [`UpstreamDataFault`]
    /// (the run loop rotates and re-pulls); everything else is fatal.
    pub(crate) async fn apply(&mut self, uf: UpstreamFinalized) -> eyre::Result<()> {
        let h = uf.block.height;
        if h != self.cursor.height + 1 {
            return Ok(()); // stale/duplicate delivery: drop
        }
        let digest = uf.block.digest();

        // 1. Ordering-chain linkage (None right after an EL-sync jump).
        if let Some(prev) = self.cursor.prev_digest {
            if uf.block.parent != prev {
                return Err(UpstreamDataFault(format!(
                    "ordering parent mismatch at {h}: cert chain does not extend the cursor"
                ))
                .into());
            }
        }
        // 2. The certificate signs THIS artifact (port of the driver gate).
        if uf.finalization.proposal.payload != digest {
            return Err(UpstreamDataFault(format!(
                "finalization payload != block digest at {h} — tampered or mismatched cert"
            ))
            .into());
        }
        // 3. BLS multisig against the epoch committee.
        let epoch = uf.finalization.proposal.round.epoch().get();
        self.rotate_scheme(epoch)?;
        let scheme = &self.schemes[&epoch];
        if !uf.finalization.verify(&mut self.ctx, scheme, &Sequential) {
            return Err(UpstreamDataFault(format!(
                "finalization cert FAILED BLS verification at {h} (epoch {epoch}) — \
                 refusing to finalize"
            ))
            .into());
        }
        // 4. Result cross-check — the committee-attested derived hash of
        //    height − K must equal OUR derived hash (the divergence detector
        //    the marshal-based follower lacked). A mismatch under a VALID
        //    cert is local divergence (or a corrupt chain) — fatal, never a
        //    rotation candidate.
        match result_target(h, self.geometry.activation) {
            ResultTarget::PreActivation => ensure!(
                uf.block.result == B256::ZERO,
                "non-zero result inside the pre-K window at {h}"
            ),
            ResultTarget::Height(t) => {
                // After a jump `t` can predate local history only if reth
                // lacks it — fail loud rather than skip the check.
                let local = self
                    .executed
                    .executed_hash(t)
                    .ok_or_eyre(format!("result height {t} missing locally at {h}"))?;
                ensure!(
                    uf.block.result == local,
                    "DIVERGENCE at {h}: committee attests {} at height {t}, local derived {local}",
                    uf.block.result
                );
            }
        }
        // 5. Derive + import + two-tier FCU (same call shape as the
        //    crash-survivor recovery path). The derive retries on the
        //    parent-visibility race: after a jump the parent was
        //    canonicalized by reth's own devp2p backfill, not by our awaited
        //    FCU, so its header can lag reader visibility by milliseconds.
        let derived =
            derive_with_visibility_retry(&self.ctx, &self.deriver, &uf.block, self.cursor.evm_hash)
                .await
                .wrap_err_with(|| format!("derivation failed at {h}"))?;
        let head_hash = derived.evm_hash();
        let status = self.beacon_engine.import_derived(derived).await?;
        ensure!(
            status.is_valid() || status.is_syncing(),
            "EL rejected derived block {h}: {status:?}"
        );
        let fin_h = result_final_height(h, self.geometry.finalized_floor);
        let fin_hash = self
            .executed
            .executed_hash(fin_h)
            .unwrap_or(self.cursor.finalized_hash);
        let fcu = ForkchoiceState {
            head_block_hash: head_hash,
            safe_block_hash: fin_hash,
            finalized_block_hash: fin_hash,
        };
        let resp = self.beacon_engine.fork_choice_updated(fcu).await?;
        ensure!(
            !matches!(
                resp.payload_status.status,
                PayloadStatusEnum::Invalid { .. }
            ),
            "EL rejected FCU at {h}: {:?}",
            resp.payload_status
        );
        self.last_fcu = fcu;

        // 6. Publish to the serving window + advance.
        if let Some(tx) = &self.verified_tx {
            let _ = tx.send(uf);
        }
        self.cursor = Cursor {
            height: h,
            evm_hash: head_hash,
            prev_digest: Some(digest),
            finalized_hash: fin_hash,
        };
        Ok(())
    }

    /// Boundary-inline scheme rotation: no EpochTransition, no re-poke. The
    /// cert's epoch must be the one `cursor + 1` falls into (a cert from any
    /// other epoch cannot be valid for this height).
    fn rotate_scheme(&mut self, epoch: u64) -> eyre::Result<()> {
        if self.schemes.contains_key(&epoch) {
            return Ok(());
        }
        let expected = fluentbase_staking_reader::reader::epoch_of_block(
            self.cursor.height + 1,
            self.geometry.interval,
            self.geometry.activation,
        );
        if epoch != expected {
            return Err(UpstreamDataFault(format!(
                "cert epoch {epoch} != expected {expected} for height {}",
                self.cursor.height + 1
            ))
            .into());
        }
        let scheme = self
            .committees
            .scheme_at(epoch, self.cursor.evm_hash)
            .wrap_err_with(|| format!("building verifier for epoch {epoch}"))?;
        info!(epoch, "cert-follower registered epoch verifier");
        self.schemes.insert(epoch, scheme);
        let keep_from = epoch.saturating_sub(1);
        self.schemes.retain(|e, _| *e >= keep_from);
        Ok(())
    }

    /// Live-first intake: a contiguous live event is used directly; anything
    /// else (gap, stale, empty queue) degrades to pull-by-height. Owns the
    /// jump rule: a gap at/above [`JUMP_THRESHOLD`] (the serving window keeps
    /// exactly that many entries, so `want` is already evicted at equality) —
    /// or sustained pull starvation with the tip ahead — re-runs the EL-sync
    /// phase and re-seeds the cursor at the synced height
    /// (`prev_digest = None`).
    async fn next_finalized(&mut self) -> eyre::Result<UpstreamFinalized> {
        let mut last_heartbeat = self.ctx.current();
        let mut starved_since = self.ctx.current();
        loop {
            let want = self.cursor.height + 1;

            // Drain the live queue without blocking.
            loop {
                match self.finalized_rx.try_recv() {
                    Ok(uf) => {
                        self.highest_live_seen = self.highest_live_seen.max(uf.block.height);
                        if uf.block.height == want {
                            return Ok(uf);
                        }
                    }
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        return Err(eyre!("live finalized stream closed (upstream actor gone)"));
                    }
                }
            }

            let gap_jump = self.highest_live_seen.saturating_sub(want) >= JUMP_THRESHOLD;
            let starved = self.ctx.current() >= starved_since + self.starvation_jump;
            if gap_jump || starved {
                if let Some(latest) = self.upstream.get_latest().await {
                    // Refresh the gap signal from the source of truth — live
                    // events can lag (dropped on overflow / WS reconnect).
                    self.highest_live_seen = self.highest_live_seen.max(latest.block.height);
                    if latest.block.height > want {
                        let (synced_height, synced_hash) = self
                            .el_sync
                            .sync_to(&latest)
                            .await
                            .wrap_err("EL-sync jump failed")?;
                        // A landing that does not advance the cursor (lagging
                        // get_latest after a failover rotation, upstream
                        // reorg) must not reseed backward — fall through to
                        // the paced pull instead of spinning the jump branch.
                        if synced_height > self.cursor.height {
                            self.reseed_at_jump_landing(synced_height, synced_hash)?;
                            starved_since = self.ctx.current();
                            continue;
                        }
                    }
                }
                // Unusable jump (no latest / tip behind / stale landing):
                // re-arm the starvation timer so this branch stays paced.
                starved_since = self.ctx.current();
            }

            // Pull the wanted height; the upstream may simply not have
            // produced it yet (steady state at 1 blk/s).
            if let Some(uf) = self.upstream.get_finalization(Height::new(want)).await {
                if uf.block.height == want {
                    return Ok(uf);
                }
            }
            if self.ctx.current() >= last_heartbeat + self.fcu_heartbeat_interval {
                let _ = self.beacon_engine.fork_choice_updated(self.last_fcu).await;
                last_heartbeat = self.ctx.current();
            }
            // Wait for the NEXT live event or the retry tick, whichever comes
            // first — a fixed sleep would park a just-arrived contiguous
            // event for up to PULL_RETRY on every block in steady state.
            tokio::select! {
                uf = self.finalized_rx.recv() => match uf {
                    Some(uf) => {
                        self.highest_live_seen = self.highest_live_seen.max(uf.block.height);
                        if uf.block.height == want {
                            return Ok(uf);
                        }
                    }
                    None => {
                        return Err(eyre!("live finalized stream closed (upstream actor gone)"));
                    }
                },
                _ = self.ctx.sleep(PULL_RETRY) => {}
            }
        }
    }

    /// Reseed the loop at an EL-sync jump landing: re-assert the L1 trust
    /// root, restart linkage, drop cached schemes, and reset the finality
    /// floor to `landing − K` (the landing itself is not result-attested yet).
    fn reseed_at_jump_landing(
        &mut self,
        synced_height: u64,
        synced_hash: B256,
    ) -> eyre::Result<()> {
        if let Some(l1) = self.l1_checkpoint {
            ensure!(
                self.el_sync
                    .holds(l1)
                    .wrap_err("L1 checkpoint probe after jump failed")?,
                "L1 Rollup checkpoint {l1:?} is NOT in the local chain after an EL-sync \
                 jump — the synced head does not extend the L1-finalized history \
                 (possible upstream equivocation); refusing to follow"
            );
        }
        let floor = synced_height
            .saturating_sub(K)
            .max(self.geometry.activation);
        let fin_hash = self.executed.executed_hash(floor).unwrap_or(synced_hash);
        warn!(
            from = self.cursor.height,
            to = synced_height,
            "cert-follower jumped an unservable gap via EL-sync"
        );
        self.geometry.finalized_floor = floor;
        self.cursor = Cursor {
            height: synced_height,
            evm_hash: synced_hash,
            prev_digest: None,
            finalized_hash: fin_hash,
        };
        self.last_fcu = ForkchoiceState {
            head_block_hash: synced_hash,
            safe_block_hash: fin_hash,
            finalized_block_hash: fin_hash,
        };
        self.schemes.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        application::{BeaconEngineLike, DerivedBlockBuilder, ExecutedChain},
        order_block::{OrderBlock, K},
    };
    use alloy_consensus::{Block as AlloyBlock, BlockBody, Header as AlloyHeader};
    use alloy_primitives::{Address, Bytes, U256};
    use alloy_rpc_types_engine::{ForkchoiceUpdated, PayloadStatus};
    use commonware_codec::DecodeExt as _;
    use commonware_consensus::{
        simplex::types::{Finalization, Finalize, Proposal},
        types::{Epoch, Round, View},
    };
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
    use commonware_math::algebra::Random as _;
    use commonware_runtime::{deterministic, Runner as _};
    use commonware_utils::{ordered::BiMap, TryCollect as _};
    use fluentbase_bls::{
        fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_signer, BlsPubkey, PeerPubkey,
    };
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;
    use reth_ethereum_primitives::TransactionSigned;
    use reth_primitives_traits::SealedBlock as RethSealed;
    use std::sync::{Arc, Mutex};

    type RethExecBlock = RethSealed<reth_ethereum_primitives::Block>;

    const CHAIN_ID: u64 = 20_994;
    const COMMITTEE_N: usize = 4;
    const INTERVAL: u32 = 32;
    const ACTIVATION: u64 = 64;

    struct Committee {
        signers: Vec<BlsScheme>,
        verifier: BlsScheme,
    }

    fn committee(seed: u64) -> Committee {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer_sks: Vec<_> = (0..COMMITTEE_N)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls_kps: Vec<_> = (0..COMMITTEE_N)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect();
        let bimap: BiMap<PeerPubkey, BlsPubkey> = peer_sks
            .iter()
            .zip(bls_kps.iter())
            .map(|(p, b)| {
                (
                    p.public_key(),
                    BlsPubkey::decode(b.public_bytes().as_slice()).unwrap(),
                )
            })
            .try_collect()
            .unwrap();
        let ns = fluent_namespace(CHAIN_ID);
        let signers = bls_kps
            .iter()
            .map(|kp| build_signer(&ns, bimap.clone(), kp, None).expect("member"))
            .collect();
        let verifier = fluentbase_bls::scheme::build_verifier(&ns, bimap, None);
        Committee { signers, verifier }
    }

    fn sample_order(parent: Digest, height: u64, result: B256) -> OrderBlock {
        OrderBlock {
            parent,
            height,
            timestamp: 1_700_000_000 + height,
            fee_recipient: Address::ZERO,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result,
            txs: Vec::new(),
            beacon_outcome: None,
            beacon_seed: None,
        }
    }

    /// 2f+1 finalize votes over the block's digest → a REAL finalization cert.
    fn certify(c: &Committee, epoch: u64, block: &OrderBlock) -> UpstreamFinalized {
        let round = Round::new(Epoch::new(epoch), View::new(block.height));
        let prop = Proposal::new(round, View::new(block.height - 1), block.digest());
        let finalizes: Vec<_> = c
            .signers
            .iter()
            .take(3)
            .map(|s| Finalize::sign(s, prop.clone()).expect("sign"))
            .collect();
        let finalization = Finalization::from_finalizes(&c.verifier, finalizes.iter(), &Sequential)
            .expect("quorum");
        UpstreamFinalized {
            finalization,
            block: block.clone(),
        }
    }

    #[derive(Clone, Default)]
    struct FakeChain {
        canonical: Arc<Mutex<std::collections::BTreeMap<u64, B256>>>,
    }

    impl ExecutedChain for FakeChain {
        fn executed_tip(&self) -> u64 {
            self.canonical
                .lock()
                .unwrap()
                .keys()
                .next_back()
                .copied()
                .unwrap_or(0)
        }
        fn executed_hash(&self, height: u64) -> Option<B256> {
            self.canonical.lock().unwrap().get(&height).copied()
        }
    }

    #[derive(Clone)]
    struct FakeDeriver {
        chain: FakeChain,
    }

    impl DerivedBlockBuilder for FakeDeriver {
        type Derived = RethExecBlock;

        async fn derive_and_execute(
            &self,
            order: OrderBlock,
            parent_evm_hash: B256,
        ) -> eyre::Result<RethExecBlock> {
            let header = AlloyHeader {
                parent_hash: parent_evm_hash,
                number: order.height,
                gas_limit: order.gas_limit,
                timestamp: order.timestamp,
                difficulty: U256::ZERO,
                ..Default::default()
            };
            let body: BlockBody<TransactionSigned> = BlockBody::default();
            let sealed = RethSealed::seal_slow(reth_ethereum_primitives::Block::from(
                AlloyBlock::new(header, body),
            ));
            self.chain
                .canonical
                .lock()
                .unwrap()
                .insert(order.height, sealed.hash());
            Ok(sealed)
        }
    }

    #[derive(Clone, Default)]
    struct FakeBeacon {
        fcu_calls: Arc<Mutex<Vec<ForkchoiceState>>>,
    }

    impl BeaconEngineLike for FakeBeacon {
        type ExecutionData = RethExecBlock;

        async fn fork_choice_updated(
            &self,
            state: ForkchoiceState,
        ) -> eyre::Result<ForkchoiceUpdated> {
            self.fcu_calls.lock().unwrap().push(state);
            Ok(ForkchoiceUpdated::from_status(
                alloy_rpc_types_engine::PayloadStatusEnum::Valid,
            ))
        }

        async fn import_derived(&self, _data: RethExecBlock) -> eyre::Result<PayloadStatus> {
            Ok(PayloadStatus::from_status(
                alloy_rpc_types_engine::PayloadStatusEnum::Valid,
            ))
        }
    }

    #[derive(Clone, Default)]
    struct FakeUpstream {
        by_height: Arc<Mutex<std::collections::BTreeMap<u64, UpstreamFinalized>>>,
        latest: Arc<Mutex<Option<UpstreamFinalized>>>,
        rotations: Arc<Mutex<u32>>,
    }

    impl CertUpstream for FakeUpstream {
        async fn get_finalization(&self, height: Height) -> Option<UpstreamFinalized> {
            self.by_height.lock().unwrap().get(&height.get()).cloned()
        }
        async fn get_latest(&self) -> Option<UpstreamFinalized> {
            self.latest.lock().unwrap().clone()
        }
        async fn rotate(&self) {
            *self.rotations.lock().unwrap() += 1;
        }
    }

    struct CannedCommittees {
        verifier: BlsScheme,
        reads: Arc<Mutex<Vec<(u64, B256)>>>,
    }

    impl CommitteeSource for CannedCommittees {
        fn scheme_at(&self, epoch: u64, at_hash: B256) -> eyre::Result<BlsScheme> {
            self.reads.lock().unwrap().push((epoch, at_hash));
            Ok(self.verifier.clone())
        }
    }

    struct FakeElSync {
        landing: (u64, B256),
        calls: Arc<Mutex<u32>>,
        holds_l1: bool,
    }

    impl ElSync for FakeElSync {
        async fn sync_to(&self, _latest: &UpstreamFinalized) -> eyre::Result<(u64, B256)> {
            *self.calls.lock().unwrap() += 1;
            Ok(self.landing)
        }
        fn holds(&self, _hash: B256) -> eyre::Result<bool> {
            Ok(self.holds_l1)
        }
    }

    struct Fixture {
        committee: Committee,
        chain: FakeChain,
        beacon: FakeBeacon,
        upstream: FakeUpstream,
        scheme_reads: Arc<Mutex<Vec<(u64, B256)>>>,
        el_sync_calls: Arc<Mutex<u32>>,
        finalized_tx: mpsc::Sender<UpstreamFinalized>,
        verified_rx: mpsc::UnboundedReceiver<UpstreamFinalized>,
    }

    type TestLoop = FollowLoop<
        deterministic::Context,
        FakeBeacon,
        FakeDeriver,
        FakeChain,
        FakeUpstream,
        CannedCommittees,
        FakeElSync,
    >;

    fn fixture(ctx: deterministic::Context, cursor_height: u64) -> (TestLoop, Fixture) {
        let committee = committee(1);
        let chain = FakeChain::default();
        let checkpoint_hash = B256::repeat_byte(0xc0);
        chain
            .canonical
            .lock()
            .unwrap()
            .insert(cursor_height, checkpoint_hash);
        let beacon = FakeBeacon::default();
        let upstream = FakeUpstream::default();
        let scheme_reads = Arc::new(Mutex::new(Vec::new()));
        let el_sync_calls = Arc::new(Mutex::new(0));
        let (finalized_tx, finalized_rx) = mpsc::channel(64);
        let (verified_tx, verified_rx) = mpsc::unbounded_channel();
        let epoch0 =
            fluentbase_staking_reader::reader::epoch_of_block(cursor_height, INTERVAL, ACTIVATION);
        let lp = FollowLoop {
            ctx,
            beacon_engine: beacon.clone(),
            deriver: FakeDeriver {
                chain: chain.clone(),
            },
            executed: chain.clone(),
            upstream: upstream.clone(),
            committees: CannedCommittees {
                verifier: committee.verifier.clone(),
                reads: scheme_reads.clone(),
            },
            el_sync: FakeElSync {
                landing: (5000, B256::repeat_byte(0xe1)),
                calls: el_sync_calls.clone(),
                holds_l1: true,
            },
            finalized_rx,
            verified_tx: Some(verified_tx),
            schemes: std::collections::BTreeMap::from([(epoch0, committee.verifier.clone())]),
            geometry: Geometry {
                interval: INTERVAL,
                activation: ACTIVATION,
                finalized_floor: cursor_height.saturating_sub(K).max(ACTIVATION),
            },
            cursor: Cursor {
                height: cursor_height,
                evm_hash: checkpoint_hash,
                prev_digest: None,
                finalized_hash: checkpoint_hash,
            },
            last_fcu: ForkchoiceState {
                head_block_hash: checkpoint_hash,
                safe_block_hash: checkpoint_hash,
                finalized_block_hash: checkpoint_hash,
            },
            fcu_heartbeat_interval: Duration::from_secs(8),
            highest_live_seen: cursor_height,
            l1_checkpoint: None,
            starvation_jump: Duration::from_secs(2),
            stop_at: None,
        };
        (
            lp,
            Fixture {
                committee,
                chain,
                beacon,
                upstream,
                scheme_reads,
                el_sync_calls,
                finalized_tx,
                verified_rx,
            },
        )
    }

    #[test]
    fn pre_k_zero_result_accepted_and_cursor_advances() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mut lp, mut fx) = fixture(ctx, ACTIVATION);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), ACTIVATION + 1, B256::ZERO);
            lp.apply(certify(&fx.committee, 0, &block))
                .await
                .expect("ok");
            assert_eq!(lp.cursor.height, ACTIVATION + 1);
            assert_eq!(lp.cursor.prev_digest, Some(block.digest()));
            assert!(fx.verified_rx.try_recv().is_ok(), "published to window");
            assert_eq!(fx.beacon.fcu_calls.lock().unwrap().len(), 1);
        });
    }

    #[test]
    fn pre_k_nonzero_result_rejected() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mut lp, fx) = fixture(ctx, ACTIVATION);
            let block = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                ACTIVATION + 1,
                B256::repeat_byte(0x66),
            );
            let err = lp
                .apply(certify(&fx.committee, 0, &block))
                .await
                .unwrap_err();
            assert!(err.to_string().contains("pre-K"), "{err}");
        });
    }

    #[test]
    fn result_mismatch_halts_with_divergence() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let start = ACTIVATION + K;
            let (mut lp, fx) = fixture(ctx, start);
            // Local derived hash at the result target differs from the attested one.
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(start + 1 - K, B256::repeat_byte(0x11));
            let block = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                start + 1,
                B256::repeat_byte(0x22),
            );
            let err = lp
                .apply(certify(&fx.committee, 0, &block))
                .await
                .unwrap_err();
            assert!(err.to_string().contains("DIVERGENCE"), "{err}");
        });
    }

    #[test]
    fn parent_mismatch_halts() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mut lp, fx) = fixture(ctx, ACTIVATION);
            lp.cursor.prev_digest = Some(Digest(B256::repeat_byte(0x77)));
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), ACTIVATION + 1, B256::ZERO);
            let err = lp
                .apply(certify(&fx.committee, 0, &block))
                .await
                .unwrap_err();
            assert!(
                err.to_string().contains("ordering parent mismatch"),
                "{err}"
            );
        });
    }

    #[test]
    fn tampered_cert_fails_bls() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mut lp, fx) = fixture(ctx, ACTIVATION);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), ACTIVATION + 1, B256::ZERO);
            let mut uf = certify(&fx.committee, 0, &block);
            // Cert signs a DIFFERENT block: payload/digest gate trips first.
            uf.block = sample_order(Digest(B256::repeat_byte(0xab)), ACTIVATION + 1, B256::ZERO);
            let err = lp.apply(uf).await.unwrap_err();
            assert!(err.to_string().contains("payload != block digest"), "{err}");
        });
    }

    #[test]
    fn scheme_rotates_across_boundary_reading_at_cursor_hash() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            // Cursor at the last block of epoch 0 ([64..96)): next height 96 = epoch 1.
            let boundary = ACTIVATION + INTERVAL as u64 - 1;
            let (mut lp, fx) = fixture(ctx, boundary);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), boundary + 1, B256::ZERO);
            // result target = 96 - 3 = 93 (post pre-K window): pre-fill local hash.
            let attested = B256::repeat_byte(0x33);
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(boundary + 1 - K, attested);
            let block = OrderBlock {
                result: attested,
                ..block
            };
            lp.apply(certify(&fx.committee, 1, &block))
                .await
                .expect("ok");
            let reads = fx.scheme_reads.lock().unwrap();
            assert_eq!(reads.len(), 1, "one committee read on rotation");
            assert_eq!(reads[0].0, 1, "epoch 1 requested");
            assert_eq!(
                reads[0].1,
                B256::repeat_byte(0xc0),
                "read at the cursor's executed hash"
            );
        });
    }

    #[test]
    fn wrong_epoch_cert_rejected() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mut lp, fx) = fixture(ctx, ACTIVATION);
            let block = sample_order(Digest(B256::repeat_byte(0xaa)), ACTIVATION + 1, B256::ZERO);
            // Height 65 is epoch 0; an epoch-2 cert must be rejected before BLS.
            let err = lp
                .apply(certify(&fx.committee, 2, &block))
                .await
                .unwrap_err();
            assert!(err.to_string().contains("!= expected"), "{err}");
        });
    }

    #[test]
    fn run_stops_exactly_at_stop_height_after_applying_up_to_it() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mut lp, fx) = fixture(ctx, ACTIVATION);
            lp.stop_at = Some(ACTIVATION + 2);
            let b1 = sample_order(Digest(B256::repeat_byte(0xaa)), ACTIVATION + 1, B256::ZERO);
            let b2 = sample_order(b1.digest(), ACTIVATION + 2, B256::ZERO);
            fx.finalized_tx
                .send(certify(&fx.committee, 0, &b1))
                .await
                .unwrap();
            fx.finalized_tx
                .send(certify(&fx.committee, 0, &b2))
                .await
                .unwrap();
            let exit = lp.run().await.expect("run");
            assert!(
                matches!(exit, FollowExit::StoppedAt { height, .. } if height == ACTIVATION + 2)
            );
        });
    }

    #[test]
    fn run_with_seed_already_at_stop_exits_without_pulling() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mut lp, fx) = fixture(ctx, ACTIVATION);
            lp.stop_at = Some(ACTIVATION);
            let exit = lp.run().await.expect("run");
            assert!(matches!(exit, FollowExit::StoppedAt { height, .. } if height == ACTIVATION));
            assert!(fx.upstream.by_height.lock().unwrap().is_empty());
        });
    }

    #[test]
    fn jump_rule_triggers_el_sync_and_reseeds_cursor() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mut lp, fx) = fixture(ctx, ACTIVATION);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            let live_uf = certify(&fx.committee, 0, &live);
            fx.finalized_tx.send(live_uf.clone()).await.unwrap();
            *fx.upstream.latest.lock().unwrap() = Some(live_uf);
            // After the jump the loop pulls synced+1 — serve it.
            let next = sample_order(Digest(B256::repeat_byte(0xbb)), 5001, B256::ZERO);
            fx.upstream
                .by_height
                .lock()
                .unwrap()
                .insert(5001, certify(&fx.committee, 0, &next));
            let got = lp.next_finalized().await.expect("next");
            assert_eq!(got.block.height, 5001);
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 1, "el_sync ran once");
            assert_eq!(lp.cursor.height, 5000);
            assert_eq!(lp.cursor.prev_digest, None, "linkage restarts after jump");
            assert!(lp.schemes.is_empty(), "schemes cleared after jump");
            assert_eq!(
                lp.geometry.finalized_floor,
                5000 - K,
                "finality floor = landing − K (the landing is not result-attested yet)"
            );
            assert_eq!(
                lp.last_fcu.head_block_hash,
                B256::repeat_byte(0xe1),
                "heartbeat FCU re-seeded at the jump landing"
            );
        });
    }

    #[test]
    fn jump_reasserts_l1_checkpoint_and_fails_closed() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mut lp, fx) = fixture(ctx, ACTIVATION);
            lp.l1_checkpoint = Some(B256::repeat_byte(0x1a));
            lp.el_sync.holds_l1 = false;
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            let live_uf = certify(&fx.committee, 0, &live);
            fx.finalized_tx.send(live_uf.clone()).await.unwrap();
            *fx.upstream.latest.lock().unwrap() = Some(live_uf);
            let err = match lp.next_finalized().await {
                Err(e) => e,
                Ok(uf) => panic!(
                    "expected the L1 re-assert to fail, got height {}",
                    uf.block.height
                ),
            };
            assert!(err.to_string().contains("NOT in the local chain"), "{err}");
        });
    }

    #[test]
    fn stale_jump_landing_does_not_reseed_backward() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mut lp, fx) = fixture(ctx, ACTIVATION);
            // A lagging upstream's sync lands AT the cursor — must not move it.
            lp.el_sync.landing = (ACTIVATION, B256::repeat_byte(0xc0));
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                far,
                B256::repeat_byte(0x44),
            );
            let live_uf = certify(&fx.committee, 0, &live);
            fx.finalized_tx.send(live_uf.clone()).await.unwrap();
            *fx.upstream.latest.lock().unwrap() = Some(live_uf);
            // The pull path must still serve the wanted height.
            let want = sample_order(Digest(B256::repeat_byte(0xbb)), ACTIVATION + 1, B256::ZERO);
            fx.upstream
                .by_height
                .lock()
                .unwrap()
                .insert(ACTIVATION + 1, certify(&fx.committee, 0, &want));
            let got = lp.next_finalized().await.expect("next");
            assert_eq!(got.block.height, ACTIVATION + 1, "fell back to the pull");
            assert_eq!(
                lp.cursor.height, ACTIVATION,
                "cursor untouched by the stale landing"
            );
            assert_eq!(lp.cursor.prev_digest, None, "no reseed happened");
        });
    }

    #[test]
    fn tampered_cert_is_an_upstream_fault_divergence_is_not() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mut lp, fx) = fixture(ctx, ACTIVATION + K);
            // Tampered pair (payload != digest) → rotatable upstream fault.
            let block = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                ACTIVATION + K + 1,
                B256::ZERO,
            );
            let mut uf = certify(&fx.committee, 0, &block);
            uf.block = sample_order(
                Digest(B256::repeat_byte(0xab)),
                ACTIVATION + K + 1,
                B256::ZERO,
            );
            let err = lp.apply(uf).await.unwrap_err();
            assert!(
                err.downcast_ref::<UpstreamDataFault>().is_some(),
                "tampered cert must classify as an upstream data fault: {err}"
            );
            // Result divergence under a VALID cert → fatal, never rotatable.
            fx.chain
                .canonical
                .lock()
                .unwrap()
                .insert(ACTIVATION + 1, B256::repeat_byte(0x11));
            let block = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                ACTIVATION + K + 1,
                B256::repeat_byte(0x22),
            );
            let err = lp
                .apply(certify(&fx.committee, 0, &block))
                .await
                .unwrap_err();
            assert!(
                err.downcast_ref::<UpstreamDataFault>().is_none(),
                "divergence must stay fatal: {err}"
            );
            assert!(err.to_string().contains("DIVERGENCE"), "{err}");
        });
    }

    #[test]
    fn starvation_jump_rescues_an_unservable_small_gap() {
        let runtime = deterministic::Runner::default();
        runtime.start(|ctx| async move {
            let (mut lp, fx) = fixture(ctx, ACTIVATION);
            // Upstream tip is only a few blocks ahead (gap ≪ JUMP_THRESHOLD)
            // but the wanted height is never servable (evicted window after
            // an upstream restart) — only the starvation timer can rescue.
            let tip = sample_order(
                Digest(B256::repeat_byte(0xaa)),
                ACTIVATION + 8,
                B256::repeat_byte(0x44),
            );
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &tip));
            // After the jump the loop pulls landing+1 — serve it.
            let next = sample_order(Digest(B256::repeat_byte(0xbb)), 5001, B256::ZERO);
            fx.upstream
                .by_height
                .lock()
                .unwrap()
                .insert(5001, certify(&fx.committee, 0, &next));
            let got = lp.next_finalized().await.expect("next");
            assert_eq!(got.block.height, 5001);
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 1, "starvation jump ran");
            assert_eq!(lp.cursor.height, 5000);
        });
    }
}
