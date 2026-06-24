//! Single-shot, pre-engine EL-sync JUMP — the deep cold-start fast-forward.
//!
//! A fresh / deeply-behind node WITH AN UPSTREAM needs a deep catch-up as a
//! cold-start prep step — run AFTER the discriminator resolves the anchor and
//! BEFORE the OuterEngine/executor task starts, so the inlet+marshal then close
//! the residual gap by ordinary live pulls. The JUMP issues ONE forkchoice
//! (read-side fast-forward) toward the committee-attested derived tip and lets
//! reth's devp2p backfill canonicalize it, then re-seeds the cold-start anchor
//! at the landing.
//!
//! **Single-writer safety.** [`cold_start_jump`] runs strictly before
//! `OuterBuilder::build` (and thus before the executor task starts), the SAME
//! mutual-exclusion property [`crate::dpos`]'s `recover_finalized_tail_into_reth`
//! relies on — there is exactly one writer touching reth at this point. It is a
//! cold-start prep path, NOT a long-lived second writer.
//!
//! **Forward-only.** The jump only moves the anchor/finalized FORWARD (a
//! landing that does not advance the resolved anchor is dropped) — reth
//! ancestor-skips a backward FCU, so a backward jump is both useless and unsafe.
//!
//! **Authenticated (fail-closed).** Two gates protect the jump:
//!
//!   1. [`verify_jump_structural`] runs BEFORE `sync_to` and rejects a
//!      structurally-broken target (`cert.payload != block.digest()`) so reth's
//!      devp2p backfill is never aimed at a tip whose served body does not match
//!      its cert.
//!   2. [`verify_jump_authenticated`] runs AFTER `sync_to` lands — at which point
//!      we DO hold the target state, so we read `committee[E]` at the landing
//!      hash and BLS-verify the finalization against it. A mismatch FAILS CLOSED
//!      (the launcher's `?` aborts the launch: the node refuses to participate on
//!      an unauthenticated branch rather than signing/serving it).
//!
//! This is the structural dual of the post-jump L1 `holds()` re-assert: both run
//! AFTER `sync_to` because the thing they check (the target's committee / the
//! L1-finalized ancestor) only becomes locally readable once reth has been driven
//! onto the branch. A pre-`sync_to` committee read is the chicken-and-egg trap —
//! `committee[far_epoch]` is NOT committed in the state at the stale resolved
//! anchor, so it was structurally unreachable exactly in the deep-catch-up case
//! it exists to protect. Reading it post-sync at the landing closes that hole
//! WITHOUT requiring an L1 checkpoint (it is the trustless default).
//!
//! The committee read is sound at the landing because committees are
//! ahead-committed at epoch-(E-1)-start and the landing (`tip − K`, K=3) shares
//! the tip's epoch (interval ≥ 32 ≫ K) — so `committee[E_tip]` is committed at
//! the landing state. So a malicious far-ahead upstream cannot steer the EL onto
//! an unagreed branch: a forged cert fails BLS against the genuine committee read
//! at the synced state.

use crate::{
    application::BeaconEngineLike,
    cert_follow::UpstreamFinalized,
    cert_inlet::CommitteeSource,
    order_block::K,
};
use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState;
use commonware_parallel::Sequential;
use commonware_runtime::{tokio::Context, Clock};
use eyre::{ensure, eyre, WrapErr as _};
use rand_core::CryptoRngCore;
use reth_storage_api::{BlockHashReader, BlockNumReader};
use std::{future::Future, time::Duration};
use tracing::info;

/// One L1 batch = 1024 blocks = ~17 min of chain time at the committed
/// 1 blk/s. Above this gap the cold-start re-runs the EL-sync phase instead of
/// resuming block-by-block — batched pipeline sync avoids the per-height RPC
/// round-trip + per-block engine-API cost. Also the serving-window cap (a
/// downstream follower's repairable gap and its own jump threshold coincide
/// by construction).
pub const JUMP_THRESHOLD: u64 = 1024;

/// Terminal classification of a (cold-start or steady-state) jump attempt. The
/// steady-state spawn owns the entire backfill wait, so there is no in-progress
/// variant — `cold_start_jump` only ever returns a terminal outcome.
pub enum JumpOutcome {
    /// A jump landed: re-seed the anchor + finalized cursor at `(landing, hash)`
    /// and advance the running marshal floor to `floor` (= landing − K).
    Landed { landing: u64, hash: B256, floor: u64 },
    /// Shallow gap / stale-or-backward landing / no `get_latest` — no-op (the
    /// inlet's ordinary pulls still cover the residual gap).
    Lagging,
    /// `EL_SYNC_NO_PROGRESS` transport stall — NON-fatal: the steady-state caller
    /// retries on the next `Update::Tip` (the marshal inlet keeps storing
    /// frontier certs while contiguous dispatch is stalled). The cold-start
    /// single-shot caller deliberately re-fuses this to fatal (see
    /// [`crate::dpos::jump_landing_or_abort`]).
    Stalled(eyre::Report),
    /// `verify_jump_structural` / `verify_jump_authenticated` / L1 `holds()`
    /// rejected the target — FATAL (a forged / unagreed branch; fail closed).
    AuthFailed(eyre::Report),
}

/// Fatal only when reth's EL-sync makes NO forward progress for this long. An
/// absolute deadline is wrong: a deep backfill (millions of blocks behind the
/// DPoS activation point) legitimately takes hours — the failure signal is
/// *stalled* progress, not elapsed time. Progress is read via
/// `last_block_number()` (NEVER `best_number`, which freezes during pipeline
/// backfill on reth 2.x — see project memory `reth-sync-progress`). Relocated
/// from `cert_follow/mod.rs`.
const EL_SYNC_NO_PROGRESS: Duration = Duration::from_secs(120);

/// EL-sync seam: drive reth onto the attested tip of `latest` (FCU + devp2p
/// backfill, with the no-progress stall detector) and return the
/// `(height, hash)` it landed on. Implemented over the provider + beacon engine
/// by [`RethElSync`]; a fake implements it in tests. Relocated from
/// `follow.rs::ElSync`.
pub trait ElSync: Send + Sync {
    fn sync_to(
        &self,
        latest: &UpstreamFinalized,
    ) -> impl Future<Output = eyre::Result<(u64, B256)>> + Send;

    /// Whether the local chain holds `hash` canonically — the post-jump L1
    /// trust-root re-assert (the synced head must be a descendant of the
    /// L1-finalized block).
    fn holds(&self, hash: B256) -> eyre::Result<bool>;
}

/// [`ElSync`] over the node's reth: FCU toward the attested derived hash (the
/// upstream tip's `result` = derived hash of tip − K) and wait for devp2p
/// backfill to canonicalize it, failing only on stalled progress. Relocated
/// from `cert_follow/mod.rs::RethElSync`.
pub struct RethElSync<Provider, BeaconEngine> {
    ctx: Context,
    provider: Provider,
    beacon_engine: BeaconEngine,
    genesis_hash: B256,
    activation: u64,
}

impl<Provider, BeaconEngine> RethElSync<Provider, BeaconEngine> {
    pub fn new(
        ctx: Context,
        provider: Provider,
        beacon_engine: BeaconEngine,
        genesis_hash: B256,
        activation: u64,
    ) -> Self {
        Self {
            ctx,
            provider,
            beacon_engine,
            genesis_hash,
            activation,
        }
    }
}

impl<Provider, BeaconEngine> RethElSync<Provider, BeaconEngine>
where
    Provider: BlockHashReader + BlockNumReader + Clone + Send + Sync + 'static,
{
    /// The height/hash reth currently sits at, clamped to ≥ activation (the
    /// ordering chain starts there; pre-activation blocks carry no certs).
    fn local_landing(&self) -> eyre::Result<(u64, B256)> {
        let tip = self
            .provider
            .last_block_number()
            .wrap_err("provider failed to report chain head")?
            .max(self.activation);
        let hash = self
            .provider
            .block_hash(tip)?
            .ok_or_else(|| eyre!("reth does not hold its own reported tip {tip}"))?;
        Ok((tip, hash))
    }
}

impl<Provider, BeaconEngine> ElSync for RethElSync<Provider, BeaconEngine>
where
    Provider: BlockHashReader + BlockNumReader + Clone + Send + Sync + 'static,
    BeaconEngine: BeaconEngineLike + Clone + Send + Sync + 'static,
{
    async fn sync_to(&self, latest: &UpstreamFinalized) -> eyre::Result<(u64, B256)> {
        // F-type: the upstream serves ORDERING artifacts — the only real EVM
        // hash on the wire is the committee-attested `result` (derived hash
        // of tip − K); FCU toward it and let reth devp2p backfill the bodies.
        let tip_hash = latest.block.result;
        let tip_height = latest.block.height.saturating_sub(K);
        if tip_hash == B256::ZERO {
            info!(
                tip = latest.block.height,
                "cold-start jump: upstream tip is inside the pre-K window; nothing to EL-sync"
            );
            return self.local_landing();
        }
        if self
            .provider
            .block_hash(tip_height)
            .wrap_err("block_hash(tip) probe before EL-sync")?
            .is_some()
        {
            return self.local_landing();
        }
        info!(
            tip_height,
            "cold-start jump: driving reth EL-sync toward attested derived hash"
        );
        let _ = self
            .beacon_engine
            .fork_choice_updated(ForkchoiceState {
                head_block_hash: tip_hash,
                safe_block_hash: tip_hash,
                finalized_block_hash: self.genesis_hash,
            })
            .await;

        // Wait for reth's backward-sync to canonicalize the tip (block_hash
        // present ⇒ executed ⇒ committee state queryable). Fail only on a
        // *stall*: the deadline resets whenever `last_block_number()` advances.
        let mut last_progress = self.provider.last_block_number().unwrap_or(0);
        let mut stall_deadline = self.ctx.current() + EL_SYNC_NO_PROGRESS;
        while self
            .provider
            .block_hash(tip_height)
            .wrap_err("block_hash(tip) probe during EL-sync")?
            .is_none()
        {
            let now = self.provider.last_block_number().unwrap_or(last_progress);
            if now > last_progress {
                last_progress = now;
                stall_deadline = self.ctx.current() + EL_SYNC_NO_PROGRESS;
            } else if self.ctx.current() >= stall_deadline {
                return Err(eyre!(
                    "cold-start jump: reth EL-sync stalled for {EL_SYNC_NO_PROGRESS:?} at \
                     height {last_progress} (target tip {tip_height}); check devp2p \
                     peering (trusted peers / firewall) and upstream block-body availability"
                ));
            }
            self.ctx.sleep(Duration::from_secs(2)).await;
        }
        // Clamp BEFORE resolving the hash so the returned pair is always
        // self-consistent (height and hash of the SAME block).
        let landing = tip_height.max(self.activation);
        let hash = self
            .provider
            .block_hash(landing)?
            .ok_or_else(|| eyre!("EL-sync landed but block {landing} vanished"))?;
        Ok((landing, hash))
    }

    fn holds(&self, hash: B256) -> eyre::Result<bool> {
        Ok(self
            .provider
            .block_number(hash)
            .wrap_err("block_number(l1 checkpoint) probe failed")?
            .is_some())
    }
}

/// PRE-`sync_to` structural gate: the served cert must sign the served body
/// (`cert.payload == block.digest()`). This is the ONLY check that can run
/// before `sync_to`, because the per-epoch committee that authenticates the cert
/// is not yet locally readable (the whole point of the jump is to sync TO the
/// state that holds it). A structural mismatch is FATAL — never drive reth's
/// devp2p backfill onto a tip whose served body does not match its cert.
pub(crate) fn verify_jump_structural(latest: &UpstreamFinalized) -> eyre::Result<()> {
    if latest.finalization.proposal.payload != latest.block.digest() {
        return Err(eyre!(
            "jump target cert payload != block digest at height {}",
            latest.block.height
        ));
    }
    Ok(())
}

/// POST-`sync_to` trustless authentication: BLS-verify the jump target's
/// finalization against `committee[E]` read at the now-materialized landing
/// state (`landing_hash`), and FAIL CLOSED on mismatch.
///
/// This runs AFTER `sync_to` — once reth holds the landing block's state, the
/// committee read is local and complete (committees are ahead-committed at
/// epoch-(E-1)-start, and the landing `tip − K` shares the tip's epoch since
/// interval ≥ 32 ≫ K=3, so `committee[E_tip]` is committed at the landing
/// state). A pre-`sync_to` read at the stale resolved anchor was the
/// chicken-and-egg trap: `committee[far_epoch]` is NOT committed there, so the
/// gate was structurally unreachable in the deep-catch-up case it exists to
/// protect. Reading it here closes that hole WITHOUT requiring an L1 checkpoint —
/// it is the trustless default.
///
/// `Err` ⇒ fail closed: the launcher's `?` aborts the launch, so the node never
/// participates (signs / serves) on an unauthenticated branch. The cert is the
/// committee's own attestation of the derived tip, so this is sound even though
/// reth has already devp2p-synced the branch (the executor/engine has NOT yet
/// started — see the single-writer note on [`cold_start_jump`]).
///
/// `l1_checkpoint`: when the trustless committee read is genuinely impossible
/// (`scheme_at` returns `Err` because `committee[E]` is unreadable even at the
/// synced landing — e.g. the upstream served a tip whose state does not commit
/// its own committee), the post-jump L1 `holds()` re-assert in [`cold_start_jump`]
/// is the operator-gated alternative trust anchor: with an L1 checkpoint set the
/// jump still authenticates via the L1 ancestry, so an unreadable committee is a
/// LOUD warn-and-defer-to-L1; WITHOUT one it is FATAL (no trust anchor at all).
pub(crate) fn verify_jump_authenticated<C: CommitteeSource>(
    latest: &UpstreamFinalized,
    committees: &C,
    landing_hash: B256,
    l1_checkpoint: Option<B256>,
    ctx: &mut (impl Clock + CryptoRngCore),
) -> eyre::Result<()> {
    let epoch = latest.finalization.proposal.round.epoch().get();
    match committees.scheme_at(epoch, landing_hash) {
        Ok(scheme) => {
            ensure!(
                latest.finalization.verify(ctx, &scheme, &Sequential),
                "jump target finalization FAILED BLS verification against committee[{epoch}] \
                 read at the synced landing {landing_hash:?} (height {}) — the upstream served \
                 a forged / unagreed branch; refusing to follow",
                latest.block.height
            );
            Ok(())
        }
        Err(read_err) => {
            // The committee is unreadable even at the synced landing state. This
            // is NOT the deep-catch-up case (that committee IS committed at the
            // landing) — it is a degenerate upstream (a tip whose own state does
            // not commit its committee) or a real read fault. Defer to the L1
            // trust anchor when configured; otherwise fail closed.
            ensure!(
                l1_checkpoint.is_some(),
                "jump target committee[{epoch}] is unreadable at the synced landing \
                 {landing_hash:?} (height {}) and no --dpos.l1-checkpoint is configured — \
                 there is NO trust anchor to authenticate the upstream branch; refusing to \
                 follow ({read_err:#})",
                latest.block.height
            );
            tracing::warn!(
                height = latest.block.height,
                epoch,
                error = %read_err,
                "jump target committee unreadable at the synced landing: authenticating via \
                 the configured --dpos.l1-checkpoint instead of the on-chain committee \
                 (the post-jump holds() probe is the trust root)"
            );
            Ok(())
        }
    }
}

/// Single-shot (cold-start) OR steady-state forward-only EL fast-forward.
///
/// At cold-start it runs BEFORE the OuterEngine (mutually exclusive with the
/// executor — the same property `recover_finalized_tail_into_reth` relies on); in
/// steady state the executor spawns it as a READ-ONLY waiter and reacts to its
/// terminal [`JumpOutcome`] (§9.6). It NEVER returns an in-progress variant — the
/// caller (cold-start `?`-abort, or the executor's spawn) owns the wait.
///
/// Classification ([`JumpOutcome`]):
///   - no `get_latest` / shallow gap / stale-or-backward landing ⇒ [`Lagging`];
///   - `verify_jump_structural` / `verify_jump_authenticated` / L1 `holds()`
///     reject ⇒ [`AuthFailed`] (FATAL — a forged / unagreed branch);
///   - `el.sync_to` transport stall ⇒ [`Stalled`] (NON-fatal in steady state —
///     was the `?` that propagated as fatal: THIS is the transient-stall-crash
///     fix);
///   - success ⇒ [`Landed`].
///
/// Gating is the caller's job: the launcher only calls this when
/// `kind != FreshMigration && upstream.is_some()`. Inside, the need-gate is
/// forward-only — a target ≤ `anchor + JUMP_THRESHOLD` is [`Lagging`], and a
/// landing that does not advance `anchor` is dropped (never reseed backward).
///
/// [`Lagging`]: JumpOutcome::Lagging
/// [`AuthFailed`]: JumpOutcome::AuthFailed
/// [`Stalled`]: JumpOutcome::Stalled
/// [`Landed`]: JumpOutcome::Landed
pub async fn cold_start_jump<U, C, ES>(
    anchor: u64,
    upstream: &U,
    committees: &C,
    el: &ES,
    l1_checkpoint: Option<B256>,
    activation: u64,
    ctx: &mut (impl Clock + CryptoRngCore),
) -> JumpOutcome
where
    U: crate::cert_follow::CertUpstream,
    C: CommitteeSource,
    ES: ElSync,
{
    let Some(latest) = upstream.get_latest().await else {
        return JumpOutcome::Lagging;
    };
    // Forward-only need-gate: only re-run the EL-sync phase for a deep gap.
    if latest.block.height <= anchor + JUMP_THRESHOLD {
        return JumpOutcome::Lagging;
    }
    // PRE-sync structural gate: never devp2p-drive reth onto a tip whose served
    // body does not match its cert. The committee-backed BLS authentication is
    // POST-sync (`verify_jump_authenticated` below) — the committee that
    // authenticates a far-ahead target is only locally readable once `sync_to`
    // has materialized its state.
    if let Err(e) = verify_jump_structural(&latest) {
        return JumpOutcome::AuthFailed(e);
    }
    // A `sync_to` transport stall is NON-fatal (was a `?` that propagated as
    // fatal): the steady-state caller retries on the next `Update::Tip`.
    let (landing_h, landing_hash) = match el.sync_to(&latest).await {
        Ok(landing) => landing,
        Err(e) => return JumpOutcome::Stalled(e),
    };
    // A landing that does not advance the resolved anchor (lagging get_latest,
    // upstream reorg) must not reseed backward.
    if landing_h <= anchor {
        return JumpOutcome::Lagging;
    }
    // POST-sync trustless authentication: now that reth holds the landing state,
    // read `committee[E]` at the landing hash and BLS-verify the finalization —
    // FAIL CLOSED on mismatch. This is the missing piece that makes a deep jump
    // trustless without an L1 checkpoint, the structural dual of the L1 `holds()`
    // re-assert below (both run post-sync because both read state the jump just
    // synced).
    if let Err(e) =
        verify_jump_authenticated(&latest, committees, landing_hash, l1_checkpoint, ctx)
    {
        return JumpOutcome::AuthFailed(e);
    }
    // L1 re-assert when configured: the synced head must descend from the
    // L1-finalized block (relocated `reseed_at_jump_landing` L1 probe).
    if let Some(l1) = l1_checkpoint {
        match el.holds(l1).wrap_err("L1 checkpoint probe after jump failed") {
            Ok(true) => {}
            Ok(false) => {
                return JumpOutcome::AuthFailed(eyre!(
                    "L1 Rollup checkpoint {l1:?} is NOT in the local chain after an EL-sync \
                     jump — the synced head does not extend the L1-finalized history (possible \
                     upstream equivocation); refusing to follow"
                ))
            }
            Err(e) => return JumpOutcome::AuthFailed(e),
        }
    }
    // Floor = landing − K (clamped to activation): the K below-landing blocks
    // are derivable via the inlet's ordinary pulls + the executor gap-walk, and
    // the landing itself is not result-attested yet (two-tier contract).
    let floor = landing_h.saturating_sub(K).max(activation);
    info!(
        from = anchor,
        to = landing_h,
        floor,
        "cold-start jump: EL-sync fast-forwarded the anchor"
    );
    JumpOutcome::Landed {
        landing: landing_h,
        hash: landing_hash,
        floor,
    }
}

/// B2 — the cold-start L1 Rollup-checkpoint assert, used by the follower
/// cold-start path ([`crate::dpos::DposLayer::launch_follower`]) so a
/// `--cert-follow` follower fails closed on a bogus L1 checkpoint after EL-sync.
/// The L1-finalized batch's last block hash must be canonical in OUR reth
/// (post-`cold_start_jump` EL-sync); a missing hash means the synced head does
/// not extend the L1-finalized history (possible upstream equivocation).
///
/// The two log/error strings — `"L1 Rollup checkpoint verified against local
/// chain"` and `"is NOT in the local chain after EL-sync"` — are byte-asserted
/// by `case-cert-cascade.sh` (phase-1 `:59` / phase-3 `:89`) and MUST survive
/// verbatim.
pub fn assert_l1_checkpoint<Provider>(provider: &Provider, l1_hash: B256) -> eyre::Result<()>
where
    Provider: reth_storage_api::BlockReader + Send + Sync,
{
    let num = provider
        .block_number(l1_hash)
        .wrap_err("block_number(l1 checkpoint) probe failed")?;
    match num {
        Some(n) => {
            info!(
                hash = ?l1_hash,
                height = n,
                "cert-follow: L1 Rollup checkpoint verified against local chain"
            );
            Ok(())
        }
        None => Err(eyre!(
            "cert-follow: L1 Rollup checkpoint {l1_hash:?} is NOT in the local \
             chain after EL-sync — the synced head does not extend the \
             L1-finalized history (possible upstream equivocation); refusing to follow"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{digest::Digest, order_block::OrderBlock};
    use alloy_primitives::{Address, Bytes};
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
        Scheme as BlsScheme,
    };
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;
    use std::sync::{Arc, Mutex};

    const CHAIN_ID: u64 = 20_994;
    const COMMITTEE_N: usize = 4;
    const ACTIVATION: u64 = 64;
    const ANCHOR_HASH: B256 = B256::repeat_byte(0xc0);

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

    struct CannedCommittees {
        verifier: BlsScheme,
        reads: Arc<Mutex<Vec<(u64, B256)>>>,
        /// `false` ⇒ `scheme_at` returns `Err` (committee unreadable even at the
        /// synced landing — the degenerate-upstream / read-fault case that
        /// `verify_jump_authenticated` defers to L1, or fails closed without one).
        readable: bool,
    }

    impl CommitteeSource for CannedCommittees {
        fn scheme_at(&self, epoch: u64, at_hash: B256) -> eyre::Result<BlsScheme> {
            self.reads.lock().unwrap().push((epoch, at_hash));
            if !self.readable {
                return Err(eyre!("epoch {epoch} has no committed committee at {at_hash}"));
            }
            Ok(self.verifier.clone())
        }
        // The cold-start jump never reads at the finalized tip — it always has a
        // specific `at_hash`; satisfy the trait with a canned verifier.
        fn scheme_at_finalized_tip(&self, _epoch: u64) -> eyre::Result<Option<BlsScheme>> {
            Ok(Some(self.verifier.clone()))
        }
    }

    #[derive(Clone, Default)]
    struct FakeUpstream {
        latest: Arc<Mutex<Option<UpstreamFinalized>>>,
    }

    impl crate::cert_follow::CertUpstream for FakeUpstream {
        async fn get_finalization(
            &self,
            _height: commonware_consensus::types::Height,
        ) -> Option<UpstreamFinalized> {
            None
        }
        async fn get_latest(&self) -> Option<UpstreamFinalized> {
            self.latest.lock().unwrap().clone()
        }
        async fn rotate(&self) {}
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
        upstream: FakeUpstream,
        el: FakeElSync,
        scheme_reads: Arc<Mutex<Vec<(u64, B256)>>>,
        el_sync_calls: Arc<Mutex<u32>>,
    }

    fn fixture(landing: (u64, B256), holds_l1: bool) -> (CannedCommittees, Fixture) {
        fixture_with_committee(landing, holds_l1, true)
    }

    /// `committee_readable == false` makes `scheme_at` return `Err` (committee
    /// unreadable even at the synced landing).
    fn fixture_with_committee(
        landing: (u64, B256),
        holds_l1: bool,
        committee_readable: bool,
    ) -> (CannedCommittees, Fixture) {
        let committee = committee(1);
        let scheme_reads = Arc::new(Mutex::new(Vec::new()));
        let el_sync_calls = Arc::new(Mutex::new(0));
        let committees = CannedCommittees {
            verifier: committee.verifier.clone(),
            reads: scheme_reads.clone(),
            readable: committee_readable,
        };
        let fx = Fixture {
            committee,
            upstream: FakeUpstream::default(),
            el: FakeElSync {
                landing,
                calls: el_sync_calls.clone(),
                holds_l1,
            },
            scheme_reads,
            el_sync_calls,
        };
        (committees, fx)
    }

    /// A deep gap above [`JUMP_THRESHOLD`] drives the EL-sync once and re-seeds
    /// the anchor + floor at the landing (`floor == landing − K`).
    #[test]
    fn jump_triggers_el_sync_and_reseeds_anchor() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let landing = (5000, B256::repeat_byte(0xe1));
            let (committees, fx) = fixture(landing, true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(Digest(B256::repeat_byte(0xaa)), far, B256::repeat_byte(0x44));
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            let JumpOutcome::Landed { landing, hash, floor } = out else {
                panic!("expected Landed, got a different JumpOutcome");
            };
            assert_eq!(landing, 5000, "anchor height = landing");
            assert_eq!(hash, B256::repeat_byte(0xe1), "anchor hash = landing hash");
            assert_eq!(
                floor,
                5000 - K,
                "marshal floor = landing − K (the landing is not result-attested yet)"
            );
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 1, "el_sync ran once");
            assert_eq!(
                fx.scheme_reads.lock().unwrap()[0].1,
                B256::repeat_byte(0xe1),
                "committee read POST-sync at the landing hash (not the stale anchor) — the \
                 far-epoch committee is only committed in the state the jump just synced"
            );
        });
    }

    /// A target within [`JUMP_THRESHOLD`] of the anchor is a no-op — the
    /// residual gap is the inlet's ordinary-pull job; the EL-sync never runs.
    #[test]
    fn shallow_gap_does_not_jump() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let (committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), true);
            let near = ACTIVATION + 8; // well below JUMP_THRESHOLD
            let live = sample_order(Digest(B256::repeat_byte(0xaa)), near, B256::repeat_byte(0x44));
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            assert!(matches!(out, JumpOutcome::Lagging), "shallow gap must not jump");
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 0, "el_sync never ran");
        });
    }

    /// No upstream tip available ⇒ no jump (the caller keeps the discriminator
    /// anchor).
    #[test]
    fn no_latest_does_not_jump() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let (committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), true);
            // upstream.latest stays None.
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            assert!(matches!(out, JumpOutcome::Lagging), "no latest ⇒ no jump");
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 0, "el_sync never ran");
        });
    }

    /// With an L1 checkpoint configured and the synced head NOT descending from
    /// it, the post-jump `holds()` probe fails closed (the trust-root assert).
    #[test]
    fn jump_reasserts_l1_checkpoint_and_fails_closed() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let (committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), false);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(Digest(B256::repeat_byte(0xaa)), far, B256::repeat_byte(0x44));
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                Some(B256::repeat_byte(0x1a)),
                ACTIVATION,
                &mut ctx,
            )
            .await;
            let JumpOutcome::AuthFailed(err) = out else {
                panic!("expected AuthFailed (L1 holds()==false), got a different JumpOutcome");
            };
            assert!(err.to_string().contains("NOT in the local chain"), "{err}");
        });
    }

    /// A structurally-broken jump target (cert payload != block digest) is fatal
    /// regardless of L1 and must NOT drive `sync_to` — the PRE-sync structural
    /// gate (`verify_jump_structural`). The committee-backed BLS authentication is
    /// POST-sync and fail-closed (see `forged_far_ahead_target_is_rejected`).
    #[test]
    fn unverifiable_jump_target_without_l1_fails_before_sync() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let (committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(Digest(B256::repeat_byte(0xaa)), far, B256::repeat_byte(0x44));
            let mut forged = certify(&fx.committee, 0, &live);
            // Cert signs a DIFFERENT block than the one served.
            forged.block = sample_order(Digest(B256::repeat_byte(0xab)), far, B256::repeat_byte(0x44));
            *fx.upstream.latest.lock().unwrap() = Some(forged);
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            let JumpOutcome::AuthFailed(err) = out else {
                panic!("expected AuthFailed (structural payload mismatch), got a different JumpOutcome");
            };
            assert!(err.to_string().contains("payload != block digest"), "{err}");
            assert_eq!(
                *fx.el_sync_calls.lock().unwrap(),
                0,
                "must NOT sync onto an unverified tip"
            );
        });
    }

    /// THE security property: a forged far-ahead target (cert structurally valid
    /// — payload == digest — but signed by an INDEPENDENT committee, so it FAILS
    /// BLS against the genuine committee read at the synced landing) is REJECTED
    /// fail-closed. `sync_to` ran (the upstream is followed before we can read its
    /// committee), but the launch then ABORTS (`Err`) instead of reseeding the
    /// anchor onto the unagreed branch. This is the deep-catch-up case the gate
    /// exists to protect — previously a silent warn-and-proceed.
    #[test]
    fn forged_far_ahead_target_is_rejected() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let landing = (ACTIVATION + 1 + JUMP_THRESHOLD + 10, B256::repeat_byte(0x55));
            let (committees, fx) = fixture(landing, true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(Digest(B256::repeat_byte(0xaa)), far, B256::repeat_byte(0x44));
            // Independent committee: structural check passes, BLS fails against
            // the fixture's genuine committee read at the landing.
            let other = committee(2);
            *fx.upstream.latest.lock().unwrap() = Some(certify(&other, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            let JumpOutcome::AuthFailed(err) = out else {
                panic!("expected AuthFailed (BLS verification), got a different JumpOutcome");
            };
            assert!(
                err.to_string().contains("FAILED BLS verification"),
                "forged target must fail closed, got: {err}"
            );
            assert_eq!(
                *fx.el_sync_calls.lock().unwrap(),
                1,
                "sync_to runs (upstream followed) but the launch aborts after the post-sync \
                 authentication fails — the anchor is NOT reseeded onto the forged branch"
            );
        });
    }

    /// Committee unreadable even at the synced landing (degenerate upstream / read
    /// fault) WITH no L1 checkpoint ⇒ no trust anchor at all ⇒ FAIL CLOSED.
    #[test]
    fn unreadable_committee_without_l1_fails_closed() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let landing = (ACTIVATION + 1 + JUMP_THRESHOLD + 10, B256::repeat_byte(0x55));
            let (committees, fx) = fixture_with_committee(landing, true, false);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(Digest(B256::repeat_byte(0xaa)), far, B256::repeat_byte(0x44));
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            let JumpOutcome::AuthFailed(err) = out else {
                panic!("expected AuthFailed (unreadable committee + no L1), got a different JumpOutcome");
            };
            assert!(
                err.to_string().contains("NO trust anchor"),
                "unreadable committee + no L1 must fail closed, got: {err}"
            );
        });
    }

    /// Committee unreadable at the synced landing BUT an L1 checkpoint is
    /// configured ⇒ the operator-gated alternative trust anchor: defer to the
    /// post-jump L1 `holds()` probe (here it holds) and proceed. The L1 ancestry
    /// authenticates the branch in lieu of the committee read.
    #[test]
    fn unreadable_committee_with_l1_defers_to_checkpoint() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let landing = (ACTIVATION + 1 + JUMP_THRESHOLD + 10, B256::repeat_byte(0x55));
            // holds_l1 = true so the post-jump `holds()` probe passes.
            let (committees, fx) = fixture_with_committee(landing, true, false);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(Digest(B256::repeat_byte(0xaa)), far, B256::repeat_byte(0x44));
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                Some(B256::repeat_byte(0x1a)),
                ACTIVATION,
                &mut ctx,
            )
            .await;
            assert!(
                matches!(out, JumpOutcome::Landed { .. }),
                "must reseed at the landing under the L1 trust anchor"
            );
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 1, "synced to live");
        });
    }

    /// A landing that does not advance the resolved anchor (lagging
    /// get_latest, upstream reorg) must NOT reseed backward — [`JumpOutcome::Lagging`].
    #[test]
    fn stale_jump_landing_does_not_reseed_backward() {
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            // The EL-sync lands AT the anchor — must not move it.
            let (committees, fx) = fixture((ACTIVATION, ANCHOR_HASH), true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(Digest(B256::repeat_byte(0xaa)), far, B256::repeat_byte(0x44));
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &fx.el,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            assert!(
                matches!(out, JumpOutcome::Lagging),
                "stale landing must not reseed backward"
            );
            assert_eq!(*fx.el_sync_calls.lock().unwrap(), 1, "el_sync ran but landing dropped");
        });
    }

    /// A `sync_to` transport stall is classified `Stalled` (NON-fatal) — NOT a
    /// fatal `?`-propagated error. This is the steady-state transient-stall fix:
    /// the executor's completion arm keeps the loop running on `Stalled` and
    /// retries on the next `Update::Tip`. (The cold-start `jump_landing_or_abort`
    /// adapter deliberately re-fuses it to fatal; that mapping is tested in
    /// `dpos.rs`.)
    #[test]
    fn sync_to_stall_is_classified_stalled() {
        struct StallingElSync;
        impl ElSync for StallingElSync {
            async fn sync_to(&self, _latest: &UpstreamFinalized) -> eyre::Result<(u64, B256)> {
                Err(eyre!("reth EL-sync stalled for 120s at height 0"))
            }
            fn holds(&self, _hash: B256) -> eyre::Result<bool> {
                Ok(true)
            }
        }
        let runtime = deterministic::Runner::default();
        runtime.start(|mut ctx| async move {
            let (committees, fx) = fixture((5000, B256::repeat_byte(0xe1)), true);
            let far = ACTIVATION + 1 + JUMP_THRESHOLD + 10;
            let live = sample_order(Digest(B256::repeat_byte(0xaa)), far, B256::repeat_byte(0x44));
            *fx.upstream.latest.lock().unwrap() = Some(certify(&fx.committee, 0, &live));
            let out = cold_start_jump(
                ACTIVATION,
                &fx.upstream,
                &committees,
                &StallingElSync,
                None,
                ACTIVATION,
                &mut ctx,
            )
            .await;
            let JumpOutcome::Stalled(err) = out else {
                panic!("a sync_to transport error must be Stalled, not AuthFailed/Landed");
            };
            assert!(err.to_string().contains("stalled"), "{err}");
        });
    }
}
