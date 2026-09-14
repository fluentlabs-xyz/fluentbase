//! DPoS layer launcher — assembles 03 (staking-reader), 04 (consensus),
//! and 05 (p2p) given operator keys, reth handles, and config. Spawned
//! by the host adapter at `crates/node/src/dpos.rs`.

use crate::{
    application::{
        derive_with_visibility_retry, BeaconEngineLike, DerivedBlock as _, DerivedBlockBuilder,
        ExecutedChain, OrderingAssembler,
    },
    beacon::{Beacon, Observed, ObservedCertificate, Seed},
    cold_start_jump::ElSync as _,
    digest::Digest,
    epocher::OriginEpocher,
    order_block::{anchor_order_block, OrderBlock, K},
    slasher::actor::SlasherTxSink,
    sync_metrics::{SyncMetrics, SyncReason},
    timeouts::ConsensusTimeouts,
    OuterBuilder,
};
use alloy_consensus::Header;
use alloy_primitives::{Address, B256};
use alloy_rpc_types_engine::ForkchoiceState;
use commonware_consensus::{
    simplex::types::Finalization,
    types::{Epoch, Epocher as _, Height, Round, View},
};
use commonware_cryptography::Signer;
use commonware_runtime::{tokio::Context, Clock as _, Handle, Metrics as _, Spawner as _};
use commonware_storage::{
    archive::{Archive, Identifier},
    metadata::{self, Metadata},
};
use commonware_utils::sequence::U64;
use eyre::{ensure, eyre, OptionExt as _, WrapErr as _};
use fluentbase_bls::{keys::ValidatorBlsKeypair, Scheme as BlsScheme};
use fluentbase_p2p::{NoopBlocker, OracleHandle};
use fluentbase_staking_reader::{
    reader::StakingReaderConfig, EpochTransition, RethStakingStateReader, TransitionOutcome,
};
use prometheus_client::metrics::{counter::Counter, family::Family, gauge::Gauge};
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_evm::ConfigureEvm;
use reth_primitives_traits::SealedBlock;
use reth_storage_api::{
    BlockHashReader, BlockIdReader, BlockNumReader, BlockReader, HeaderProvider,
    StateProviderFactory,
};
use std::{
    num::NonZeroU64,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// The metric every channel's ingress refusal lands on. ONE counter for all of
/// them, labelled by channel and by why — no per-peer counter, no penalty, no
/// timer: the only global ban is the on-chain tombstone (PLAN §8 п.1).
pub const INGRESS_DROPPED_TOTAL: &str = "dpos_ingress_dropped_total";

/// Count one refused frame.
pub fn record_ingress_drop(channel: &'static str, reason: &'static str) {
    metrics::counter!(INGRESS_DROPPED_TOTAL, "channel" => channel, "reason" => reason).increment(1);
}

/// A `commonware_p2p::Receiver` that refuses a frame before anything decodes it,
/// on the peer set this node last registered.
///
/// This is the ONE classification of a sender on the channel, and the only check
/// that can run before a decode: the sender is either in the tracked window or
/// it is not, and a tombstoned sender is out regardless. Binding the sender to
/// the frame's own EPOCH needs the frame's epoch and therefore lives past the
/// decode, and the two channels do it differently: EVIDENCE re-reads this window
/// (`slasher::gossip::ingest_batch`, `Ingress::member_of`); the BEACON asks the
/// window nothing more — its actor keeps an epoch cost gate (`[now, now + 2]`,
/// `beacon::actor::on_message`) and leaves the seat to the frame's consumer (the
/// ceremony's roster, `committee[target_epoch]` for a confirmation; refused
/// `no_seat` there). 5.3-В.
///
/// `members_only` says which tier the channel serves: BEACON and EVIDENCE are
/// committee traffic, so a tier-2 (registry) sender has no business on them;
/// a channel that serves the registry would set it `false` and only lose the
/// untracked and the tombstoned.
///
/// Before the first `track` the window has no opinion and NOTHING is refused —
/// a node in cold start has not read the chain yet, and turning "I do not know"
/// into a drop would silence the plane exactly when it is trying to join.
#[derive(Debug)]
pub struct GatedReceiver<R> {
    inner: R,
    window: fluentbase_p2p::TrackedWindow,
    channel: &'static str,
    members_only: bool,
}

impl<R> GatedReceiver<R> {
    pub fn new(
        inner: R,
        window: fluentbase_p2p::TrackedWindow,
        channel: &'static str,
        members_only: bool,
    ) -> Self {
        Self {
            inner,
            window,
            channel,
            members_only,
        }
    }

    /// Whether this frame survives the window check.
    fn admits(&self, from: &fluentbase_bls::PeerPubkey) -> bool {
        let Some(ingress) = self.window.classify(from) else {
            return true; // no peer set registered yet
        };
        let admitted = match ingress {
            fluentbase_p2p::Ingress::Member { .. } => true,
            fluentbase_p2p::Ingress::Tracked(_) => !self.members_only,
            fluentbase_p2p::Ingress::Dropped => false,
        };
        if !admitted {
            record_ingress_drop(self.channel, ingress.refusal());
        }
        admitted
    }
}

impl<R> commonware_p2p::Receiver for GatedReceiver<R>
where
    R: commonware_p2p::Receiver<PublicKey = fluentbase_bls::PeerPubkey>,
{
    type Error = R::Error;
    type PublicKey = fluentbase_bls::PeerPubkey;

    async fn recv(&mut self) -> Result<commonware_p2p::Message<Self::PublicKey>, R::Error> {
        loop {
            let (from, buf) = self.inner.recv().await?;
            if self.admits(&from) {
                return Ok((from, buf));
            }
        }
    }
}

/// Codeless-tolerant epoch-geometry read: `None` when `ChainConfig` is not
/// deployed (or DPoS not yet scheduled) at `at` — the launch discriminator
/// between "restart datadir / genesis-baked devnet" and "fresh datadir on a
/// runtime-deployed chain", where geometry is only readable AFTER EL-sync. Used
/// by the follower cold-start in [`DposLayer::launch_follower`].
fn read_geometry<Provider, EvmConfig>(
    reader: &RethStakingStateReader<Provider, EvmConfig>,
    at: B256,
) -> eyre::Result<Option<(u64, u64)>>
where
    Provider:
        StateProviderFactory + HeaderProvider<Header = Header> + Clone + Send + Sync + 'static,
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
{
    match reader.scheduled_dpos_activation(at)? {
        None => Ok(None),
        Some(activation) => {
            let interval = reader.epoch_block_interval(at)?;
            ensure!(interval > 0, "epoch_block_interval must be > 0");
            Ok(Some((activation, interval)))
        }
    }
}

/// Re-poke count between successive "boundary still parked" `warn!`s (log-side
/// external wedge detection alongside the `parked_boundary_height` gauge). At
/// `PENDING_RETRY_BACKOFF = 200 ms`, 150 pokes ≈ 30 s.
const PARKED_BOUNDARY_WARN_EVERY: u64 = 150;

/// Partition prefix for the commonware marshal's durable storage (finalizations,
/// finalized blocks, application-metadata). Shared between the cold-start
/// discriminator peek (`read_consensus_archive_last_finalized`) and the marshal
/// itself (`OuterBuilder.partition_prefix`) so the two never drift.
const MARSHAL_PARTITION_PREFIX: &str = "consensus_marshal";

/// Partition for the durable `Round → σ` store behind the
/// `beacon::seed_index::SeedIndex`. Deliberately NOT under
/// [`MARSHAL_PARTITION_PREFIX`]: this is a Fluent-side store beside the marshal's,
/// not part of it, and it must stay independently prunable.
///
/// Renamed from `beacon-seed-journal` when the backing primitive moved from
/// `journal::segmented::fixed` to `ordinal::Ordinal`: the two on-disk formats are
/// incompatible, and pointing at a fresh name lets the retention window simply
/// refill.
///
/// That refill is no longer free. No block body couriers σ any more — the live
/// derive and the crash-survivor replay both key on the block's own round — so a
/// cold store costs the replay its first source and pushes it onto the local
/// certificate, then the upstream, then a defer. **A further format change needs
/// a real migration, not a rename.**
pub(crate) const SEED_JOURNAL_PARTITION: &str = "beacon-seed-ordinal";

/// Partition of the durable mint memo (`epoch → the epoch that MINTED the key in
/// force at it`, `beacon::artifact::MintIndex`). Empty would mean RAM-only.
///
/// NOT the old `beacon-key-ordinal`, and the rename is not cosmetic: that name
/// belonged to the deleted `epoch → PK_epoch` key journal, whose backing primitive
/// was `ordinal::Ordinal`, while this is a `Metadata` store of a different record.
/// A `Metadata` opened over a partition holding another codec's blobs PANICS at
/// init (`.claude/COMMONWARE_INTERNALS.md`, "wrong codec on an existing
/// partition"), and "every net relaunches from a fresh genesis" is a deployment
/// policy, not a property of this code — so the two formats get two names.
pub(crate) const MINT_MEMO_PARTITION: &str = "beacon-mint-metadata";

/// Partition of the durable `epoch → agreement artifact` store
/// (`beacon::artifact::ArtifactStore`). Public because the store is
/// opened by the always-on beacon plane in the node crate, one process-wide
/// instance — a second handle over this partition would be a dual-writer.
pub const ARTIFACT_JOURNAL_PARTITION: &str = "beacon-artifact-metadata";

/// Reth handles needed by the DPoS layer. The host adapter at
/// `crates/node/src/dpos.rs` assembles this from `FullNode<N, AddOns>`;
/// `transaction_pool`, `chain_spec`, and `data_dir` are intentionally
/// absent — `slasher_sink` arrives pre-built via `DposLayerConfig` (so
/// the host owns the `reth-transaction-pool` trait bounds), `chain_spec`
/// reduces to its only used field `chain_id`, and `data_dir` is set
/// host-side in `spawn_dpos` before `runner.start()`.
pub struct RethHandle<Provider, EvmConfig, BeaconEngine> {
    pub provider: Provider,
    pub evm_config: EvmConfig,
    pub beacon_engine_handle: BeaconEngine,
    pub chain_id: u64,
    /// Read-only probe of reth's connected devp2p peer count, built host-side
    /// from `node.network` (which implements `reth_network_api::PeersInfo`). Drives
    /// the cold-start / re-jump EL-sync no-peers net (`cold_start_jump::RethElSync`).
    /// A closure, not a typed handle, keeps the consensus crate free of a
    /// `reth-network-api` dependency.
    pub peer_count: Arc<dyn Fn() -> usize + Send + Sync>,
    /// Disk-loaded canonical state snapshot. Reth's
    /// `BlockchainProvider::with_latest` populates `finalized_block` /
    /// `safe_block` from `ChainState::LastFinalizedBlock` during node
    /// init, so on a graceful-shutdown restart
    /// `get_finalized_num_hash()` returns `Some(disk_finalized.num_hash())`.
    /// Kept as a struct field rather than a trait method because
    /// `canonical_in_memory_state()` is a concrete inherent on
    /// `BlockchainProvider<N>`, not exposed via any reth provider trait.
    pub canonical_state: reth_chain_state::CanonicalInMemoryState<EthPrimitives>,
    /// Pristine-network fallback for when
    /// `canonical_state.get_finalized_num_hash()` returns `None`.
    pub genesis_hash: B256,
}

/// Cold-start `(finalized_num, finalized_hash, head_num, head_hash)` derived
/// purely from reth's `canonical_state` + `genesis_hash` (the non-migration
/// path). Reth's `BlockchainProvider::with_latest` repopulates
/// `canonical_state.finalized_block` on a graceful-restart, so
/// `get_finalized_num_hash()` returns the disk finalized; the genesis fallback
/// covers a pristine network (no FCU yet). Extracted from [`DposLayer::launch`]
/// so the cold-start arithmetic is unit-tested against this production code
/// rather than a copy.
pub fn derive_cold_start_heights(
    canonical_state: &reth_chain_state::CanonicalInMemoryState<EthPrimitives>,
    genesis_hash: B256,
) -> (u64, B256, u64, B256) {
    let finalized = canonical_state
        .get_finalized_num_hash()
        .unwrap_or_else(|| alloy_eips::BlockNumHash::new(0, genesis_hash));
    let head = canonical_state.chain_info();
    (
        finalized.number,
        finalized.hash,
        head.best_number,
        head.best_hash,
    )
}

/// How often a park on an EXTERNAL activation input re-asks: reth for the
/// activation block ([`wait_for_activation_block`]) and, in `launch_follower`'s
/// entry march, the local probe + the upstream + the committee window. ONE constant
/// because it is one cadence for one reason — "ask again, the answer is somebody
/// else's to change" — and two copies of it drift apart silently.
const ACTIVATION_POLL: Duration = Duration::from_secs(2);

/// Wait for reth to hold the DPoS activation block before adopting it as the
/// fresh-migration consensus anchor; returns the block's local-canonical hash.
/// Covers reth still replaying MDBX on restart. There is NO give-up and no
/// timeout: the anchor is external (the sequencer must finalize the activation
/// block before DPoS starts), so the wait polls forever and raises
/// `dpos_sync_degraded{reason=activation_wait}` as the stuck signal instead of
/// failing. No operator-hash compare — the activation height comes from the on-chain
/// `ChainConfig.dposActivationBlock` and the hash is local-canonical at a
/// finalized height (every honest node derives the same hash).
pub(crate) async fn wait_for_activation_block<Provider, C>(
    ctx: &C,
    provider: &Provider,
    activation: u64,
    sync_metrics: &SyncMetrics,
) -> eyre::Result<B256>
where
    Provider: BlockHashReader,
    C: commonware_runtime::Clock,
{
    // #13 SELF-HEAL (retry-forever, Decision A): the activation anchor is EXTERNAL
    // — the sequencer may still be producing/persisting it, and a fatal here would
    // restart-storm every honest joiner at once. Keep polling forever and surface
    // `dpos_sync_degraded{reason=activation_wait}=1` as the stuck signal (a stuck
    // gauge, not a crash). A cold restart legitimately shows the block absent for
    // seconds while reth replays MDBX/static files under multi-node contention.
    let mut waited = false;
    loop {
        if let Some(hash) = provider
            .block_hash(activation)
            .wrap_err("provider.block_hash failed during activation-block wait")?
        {
            if waited {
                sync_metrics.recover(SyncReason::ActivationWait);
            }
            tracing::info!(height = activation, hash = ?hash, "DPoS activation block present in reth");
            return Ok(hash);
        }
        if !waited {
            waited = true;
            warn!(
                height = activation,
                "reth does not yet hold the DPoS activation block; polling (no give-up) — \
                 waiting for the sequencer to produce and persist it"
            );
        }
        sync_metrics.degrade(SyncReason::ActivationWait);
        ctx.sleep(ACTIVATION_POLL).await;
    }
}

/// Peek the marshal's last consensus-finalized height from its durable
/// application-metadata store WITHOUT building the marshal/engine — the
/// restart-vs-fresh-migration discriminator. An empty store (fresh migration)
/// returns 0; a populated store (restart, already migrated) returns the last
/// DPoS-finalized height so the cold-start resumes at the correct epoch.
///
/// Reads the SAME `{partition_prefix}-application-metadata` Metadata store and
/// key that commonware `MarshalActor::init` returns as `last_processed_height`
/// (monorepo `consensus/src/marshal/core/actor.rs:305-317`), so the value is
/// byte-identical to the one the executor-seed path already consumes. The peek
/// opens the store, reads, and drops it before `MarshalActor::init` re-opens it.
pub(crate) async fn read_consensus_archive_last_finalized(
    ctx: &Context,
    partition_prefix: &str,
) -> eyre::Result<u64> {
    // Wire-format invariant: must match commonware marshal `core/actor.rs:58`
    // `const LATEST_KEY: U64 = U64::new(0xFF)` (a private const there). It is a
    // storage-layout constant pinned with the commonware rev in `Cargo.lock`.
    const LATEST_KEY: U64 = U64::new(0xFF);
    let metadata: Metadata<Context, U64, Height> = Metadata::init(
        ctx.with_label("cold_start_archive_peek"),
        metadata::Config {
            partition: format!("{partition_prefix}-application-metadata"),
            codec_config: (),
        },
    )
    .await
    .wrap_err("opening marshal application-metadata for cold-start discriminator")?;
    Ok(metadata
        .get(&LATEST_KEY)
        .copied()
        .unwrap_or(Height::zero())
        .get())
}

/// Outcome of the pre-engine crash-survivor recovery (#8/#12 self-heal,
/// 2026-07-09).
#[derive(Debug)]
enum RecoverOutcome {
    /// reth now holds `target`; carries its local-canonical hash. The pre-engine
    /// marshal→reth replay bridged the gap — either straight from the local
    /// `finalized_blocks` archive (σ for each height comes from the seed store at
    /// the height's OWN round, with the local certificate and the upstream as
    /// fallbacks — no cert is REQUIRED to exist), or, on a below-floor BLOCK hole
    /// (#8), by a BLS-verified by-height re-fetch through the cert upstream
    /// ([`refetch_hole_until_answered`] over [`refetch_verified_archive_hole`])
    /// spliced into the same replay.
    Recovered(B256),
    /// reth is `> MAX_COLD_RECOVER` behind its OWN (INTACT) consensus archive (#12):
    /// the pre-engine replay is capped, so the caller anchors the cold-start at
    /// `provider.best_block_number()` and the EXECUTOR'S STARTUP BACKFILL DRAIN
    /// (`executor.rs::finalized_heights_to_backfill`, seeded from the marshal's own
    /// acked cursor) walks the rest of the tail into reth. NOT the steady-state
    /// jump: it is gated off while that drain is non-empty and its trigger is 0 at
    /// boot (both `last_tip_height` and `ordering_finalized` come from the same
    /// cursor). This is a distance problem, NOT an archive hole — the
    /// marshal holds every block. Needs an upstream
    /// (a no-upstream node is FATAL at the recovery site,
    /// [`crash_recover_defer_or_fatal`]). `gap` = blocks reth is behind `target`
    /// (`crash_recover_gap_blocks` gauge).
    ///
    /// NOTE (why #8 does NOT land here): a #8 hole is BELOW the marshal's finalized
    /// floor (`target == last_processed_height`, the same `LATEST_KEY` the metadata
    /// peek reads). The live marshal repairs only `[floor+1 ..]` (monorepo
    /// `marshal/core/actor.rs:1557` `start = last_processed_height.next()`; it PRUNES
    /// below floor at `:700`, and `HintFinalized` skips `<= floor` at `:633`), so its
    /// `UpstreamResolver` would NEVER repopulate a below-floor hole — and the executor
    /// gap-walk agrees (`executor.rs` "a hole below the floor cannot self-heal"). Nor
    /// does the steady-state jump help: a #8 gap is `<= MAX_COLD_RECOVER (64) <
    /// JUMP_THRESHOLD (1024)`, so the jump is always `Lagging` and never fires. #8 is
    /// therefore healed INLINE by [`refetch_hole_until_answered`], not deferred.
    DeferToElSync { gap: u64 },
}

/// Provider-only reconnect scan for [`recover_finalized_tail_into_reth`] (factored
/// out so the #12 too-deep detection is unit-testable without a marshal archive).
/// Walks `target` downward while reth is missing each parent.
enum ReconnectScan {
    /// reth holds the block at `lowest - 1` (or `lowest == 0`); replay
    /// `lowest..=target` from the marshal archive.
    Reconnect(u64),
    /// reth is missing `>= max_cold_recover` blocks below `target` — beyond a
    /// flush-race tail; #12 defers to devp2p EL-sync.
    TooDeep,
}

fn recover_reconnect_point<Provider>(
    provider: &Provider,
    target: u64,
    max_cold_recover: u64,
) -> eyre::Result<ReconnectScan>
where
    Provider: BlockHashReader,
{
    let mut lowest = target;
    while lowest > 0
        && provider
            .block_hash(lowest - 1)
            .wrap_err("provider.block_hash during crash-survivor recovery")?
            .is_none()
    {
        if target - lowest >= max_cold_recover {
            return Ok(ReconnectScan::TooDeep);
        }
        lowest -= 1;
    }
    Ok(ReconnectScan::Reconnect(lowest))
}

/// The #12 defer-vs-fatal decision for the too-deep trigger (reth `>
/// MAX_COLD_RECOVER` behind its OWN INTACT archive). WITH an upstream: raise the
/// crash-recover gauges + counter and DEFER (the caller anchors at reth's tip; the
/// post-engine devp2p jump backfills the EL — the marshal holds every block, so no
/// consensus-store repair is needed). WITHOUT an upstream: residual FATAL — there is
/// nowhere to devp2p-backfill from, so this is real local data loss (Decision A does
/// not apply to idiosyncratic local corruption). A #8 archive HOLE does NOT route
/// here — it heals inline via [`refetch_hole_until_answered`].
fn crash_recover_defer_or_fatal<Provider>(
    provider: &Provider,
    target: u64,
    has_upstream: bool,
    sync_metrics: &SyncMetrics,
    cause: &str,
) -> eyre::Result<RecoverOutcome>
where
    Provider: BlockNumReader,
{
    if !has_upstream {
        return Err(eyre!(
            "crash-survivor recovery: {cause}, and no --dpos.follower-upstream is configured to \
             devp2p / by-height re-fetch the gap — real EL/consensus data loss; re-sync the EL \
             disk from a snapshot"
        ));
    }
    crash_recover_defer(provider, target, sync_metrics, cause)
}

/// The deferring half alone, for a cause that is NOT local data loss and therefore
/// has no fatal arm.
///
/// The one caller is [`ReplaySeed::Defer`]: "this node does not hold the epoch key
/// yet" is a statement about the BEACON's acquisition, which self-heals off the
/// artifact — the `KeyAvailable` edge settles the σ the replay held — and an
/// upstream is irrelevant to it in both directions. Making it fatal on a node with
/// no cert upstream would turn a bounded wait into a re-sync instruction.
fn crash_recover_defer<Provider>(
    provider: &Provider,
    target: u64,
    sync_metrics: &SyncMetrics,
    cause: &str,
) -> eyre::Result<RecoverOutcome>
where
    Provider: BlockNumReader,
{
    let best = provider
        .best_block_number()
        .wrap_err("best_block_number at crash-survivor defer")?;
    let gap = target.saturating_sub(best);
    warn!(
        finalized_target = target,
        reth_best = best,
        gap,
        "crash-survivor recovery: {cause}; deferring to the post-engine devp2p EL-sync \
         (anchor moves to reth's tip; no give-up)"
    );
    sync_metrics.degrade(SyncReason::CrashRecover);
    sync_metrics.crash_recover_deferred_to_elsync.inc();
    sync_metrics.crash_recover_gap_blocks.set(gap as i64);
    Ok(RecoverOutcome::DeferToElSync { gap })
}

/// #8 below-floor archive-hole heal: BLS-verified by-height re-fetch of a missing
/// `finalized_blocks` / `finalizations` entry through the cert upstream.
///
/// A #8 hole sits BELOW the marshal's finalized floor (`target ==
/// last_processed_height`), which the live marshal's own resolver NEVER re-fetches
/// — it repairs `[floor+1 ..]` only and prunes below floor (monorepo
/// `marshal/core/actor.rs:1557`/`:700`/`:633`), and the steady-state jump can't fire
/// either (a #8 gap `<= MAX_COLD_RECOVER (64) < JUMP_THRESHOLD (1024)` ⇒ always
/// `Lagging`). So a bare defer would leave reth permanently missing the block.
/// Instead we pull the finalization+block from the upstream (the SAME by-height seam
/// the inlet uses) and authenticate it HERE, because this pull reaches neither of the
/// two writers that would otherwise have done it (`store_finalization` after
/// `verify_delivered`, or `FrontierHandler::deliver`): `verify_jump_structural`
/// (payload == digest) + `verify_jump_authenticated` (2f+1 BLS multisig against
/// `committee[E]` read at `at_hash`, the already-recovered parent's materialized
/// state). These two functions exist for exactly this seam and for
/// `cert_follow::fetch_verified_boundary` / [`fetch_verified_entry`] — they are no
/// longer stages of any jump.
/// The caller then derives + imports the verified block into reth, splicing the hole
/// shut in the same replay.
///
/// We do NOT write the re-fetched entry back into the marshal archive, and the
/// reason is NOT that a below-floor write would be discarded — it would not be.
/// `MarshalActor::init` runs no prune, immutable-archive `prune` is a no-op, and a
/// below-floor entry written before the floor rises stays permanently readable
/// (`get_finalized_block` consults no floor); the boundary-seeding path in
/// `outer.rs` / `executor::reseed_forward` depends on exactly that. The real reason
/// is scope: this path runs PRE-engine against a standalone archive handle, and reth
/// is the only local reader that needs the block at that moment. Peer-serving of
/// that below-floor height stays a re-fetch-from-elsewhere concern, unchanged.
///
/// `Err` IS A VERDICT: no `--dpos.follower-upstream` is configured, or every
/// configured upstream answered and none of them holds the record (gone everywhere
/// — real local consensus data loss), or the answer failed authentication.
///
/// `Ok(None)` IS THE ABSENCE OF A VERDICT, and the distinction is the whole point of
/// this signature: not one configured upstream ANSWERED, so nothing was learned about
/// the record and the caller must ask again. Before the entry march made a
/// disconnected WS actor answer its mailbox, this case could not arise — the pull
/// simply never returned — so a negative here was necessarily a real "nobody holds
/// it". It can arise now, and folding it back into the `Err` above would print
/// "re-sync the EL disk from a snapshot" at an operator whose upstream is merely
/// down. That instruction is irreversible; a retry is free (R-131 review,
/// `4.4а-Д-9`).
///
/// The walk in `cert_follow::upstream` is what separates the two, because it is what
/// ASKS: it answers `MissedEverywhere` only when servers rendered a verdict on the
/// height, and `NoneAnswered` when none of them was reachable — and it serves the
/// `_everywhere` pull with or without a live connection, so its negative is never a
/// refusal-without-asking. What the `Option`-typed seam cannot carry across the
/// crate boundary is WHICH of the two it was, so this function asks for the one thing
/// that settles it positively: `get_latest`, whose `Some` can only come from an
/// upstream that answered us. Absent that witness nothing is claimed.
async fn refetch_verified_archive_hole<U, C>(
    upstream: Option<&U>,
    committees: &C,
    verify_ctx: &mut (impl commonware_runtime::Clock + rand_core::CryptoRngCore),
    at_hash: B256,
    height: u64,
    which: &str,
) -> eyre::Result<Option<crate::cert_follow::UpstreamFinalized>>
where
    U: crate::cert_follow::CertUpstream,
    C: crate::cert_inlet::CommitteeSource,
{
    let Some(up) = upstream else {
        return Err(eyre!(
            "crash-survivor recovery: marshal {which} has a below-floor hole at height {height}, \
             and no --dpos.follower-upstream is configured to by-height re-fetch the BLS-verified \
             finalization (the marshal's own resolver cannot repair below its finalized floor) — \
             real local consensus data loss; re-sync the EL disk from a snapshot"
        ));
    };
    // `_everywhere`: the FATAL below tells the operator to re-sync the EL disk from a
    // snapshot. Asking ONE upstream before saying that is not enough when the operator
    // configured several and the block sits on the second.
    let Some(uf) = up.get_finalization_everywhere(Height::new(height)).await else {
        // THE NEGATIVE IS NOT YET A VERDICT. Claim data loss only with positive proof
        // that an upstream answered us at all; `get_latest` is that proof and nothing
        // else in the seam is (a by-height negative is produced by both cases alike).
        // Fail-safe direction: a wrongly-withheld verdict costs one more lap, a
        // wrongly-issued one costs the operator's disk.
        if up.get_latest().await.is_none() {
            return Ok(None);
        }
        return Err(eyre!(
            "crash-survivor recovery: marshal {which} has a below-floor hole at height {height} \
             and the upstream no longer serves it — the consensus record is gone everywhere; \
             re-sync the EL disk from a snapshot"
        ));
    };
    // Nothing else binds the response to the request: `verify_jump_structural` ties
    // the cert only to the block it arrived with, and `verify_jump_authenticated`
    // takes the epoch from the cert's own round. Unpinned, a valid finalization for
    // a DIFFERENT height passes both and is spliced in as if it were this one.
    ensure!(
        uf.block.height == height,
        "upstream served height {} for the marshal {which} hole at height {height}",
        uf.block.height
    );
    crate::cold_start_jump::verify_jump_structural(&uf).wrap_err_with(|| {
        format!(
            "re-fetched finalization for the marshal {which} hole at height {height} is malformed"
        )
    })?;
    crate::cold_start_jump::verify_jump_authenticated(&uf, committees, at_hash, verify_ctx)
        .wrap_err_with(|| {
            format!(
                "BLS-authenticating the re-fetched finalization for the marshal {which} hole at \
             height {height} against committee[E] read at the recovered parent {at_hash:?}"
            )
        })?;
    Ok(Some(uf))
}

/// [`refetch_verified_archive_hole`] until it produces a VERDICT: the record, or a
/// reasoned refusal. The only thing this adds is patience, and it is the block
/// path's policy rather than the seam's — the σ path deliberately does not wait
/// (see `replay_seed`).
///
/// **Asking again is the answer to "not one upstream answered", and it is not a
/// softening of the fatal.** A gone-everywhere verdict still exits, with the same
/// sentence, because that verdict is evidence: servers answered and none holds the
/// record. What may not happen is printing "re-sync the EL disk from a snapshot" at
/// an operator whose upstream is merely unreachable — the instruction is
/// irreversible and the condition is transient. This case became reachable only
/// when the WS actor started answering its mailbox while disconnected (before that
/// the pull never returned at all), which is why the patience arrives with it
/// (R-131 review, `4.4а-Д-9`).
///
/// Retry-forever on the cadence the other external-input waits use
/// (`wait_for_activation_block`, Decision A), under the gauge reason this path
/// already owns (`crash_recover`): the node stays observable instead of exiting on a
/// link. Fork-safety permits it — nothing has been written yet, and the answer, when
/// it comes, is authenticated by the same committee read either way.
async fn refetch_hole_until_answered<U, C>(
    upstream: Option<&U>,
    committees: &C,
    verify_ctx: &mut (impl commonware_runtime::Clock + rand_core::CryptoRngCore),
    at_hash: B256,
    height: u64,
    which: &str,
    sync_metrics: &SyncMetrics,
) -> eyre::Result<crate::cert_follow::UpstreamFinalized>
where
    U: crate::cert_follow::CertUpstream,
    C: crate::cert_inlet::CommitteeSource,
{
    let mut waited = false;
    loop {
        if let Some(uf) =
            refetch_verified_archive_hole(upstream, committees, verify_ctx, at_hash, height, which)
                .await?
        {
            if waited {
                sync_metrics.recover(SyncReason::CrashRecover);
            }
            return Ok(uf);
        }
        if !waited {
            waited = true;
            warn!(
                height,
                which,
                "crash-survivor recovery: a below-floor marshal {which} hole needs a by-height \
                 re-fetch and NOT ONE configured upstream answered — that says nothing about \
                 whether the record still exists, so nothing is concluded from it. Polling (no \
                 give-up); make --dpos.follower-upstream reachable"
            );
        }
        sync_metrics.degrade(SyncReason::CrashRecover);
        verify_ctx.sleep(ACTIVATION_POLL).await;
    }
}

/// One element of the crash-survivor replay walk: the block at `h` from the
/// marshal's OWN `finalized_blocks` archive, or — on a below-floor BLOCK hole
/// (#8) — a BLS-verified by-height re-fetch through the cert upstream. Under
/// Design B′ the walk is BLOCKS-ONLY: a locally-present block with a locally
/// absent finalization cert is a NORMAL state (an ancestry-finalized height may
/// have NO standalone cert anywhere, ever — the soak7 class), so certs are read
/// solely to AUTHENTICATE a re-fetched missing block, never per height.
#[allow(clippy::too_many_arguments)] // mirrors its caller: distinct pre-engine deps, not a cluster
async fn recover_walk_block<A, U, C>(
    archive: &A,
    upstream: Option<&U>,
    committees: &C,
    verify_ctx: &mut Context,
    at_hash: B256,
    sync_metrics: &SyncMetrics,
    h: u64,
) -> eyre::Result<OrderBlock>
where
    A: commonware_storage::archive::Archive<Value = OrderBlock>,
    U: crate::cert_follow::CertUpstream,
    C: crate::cert_inlet::CommitteeSource,
{
    let local = archive
        .get(Identifier::Index(h))
        .await
        .map_err(|e| eyre!("reading marshal finalized_blocks at height {h}: {e}"))?;
    if let Some(order) = local {
        return Ok(order);
    }
    // #8: a hole in the marshal's OWN below-floor BLOCK archive. The live
    // marshal resolver cannot repair below its floor and the deferred jump
    // can't fire at this `<= 64` gap, so a bare defer would strand reth.
    // Re-fetch the BLS-verified finalization+block from the cert upstream and
    // splice the hole shut; no-upstream / gone-everywhere stays fatal.
    //
    // ASKING AGAIN IS THE ANSWER TO "NOBODY ANSWERED", and it is not a softening of
    // the fatal: the fatal is still what a gone-everywhere VERDICT produces (see
    // `refetch_verified_archive_hole`). What may not happen is telling an operator to
    // re-sync the EL disk because this node could not reach any upstream for a moment
    // — that instruction is irreversible and the condition is transient. Retry-forever
    // is the same policy the other external-input waits run (`wait_for_activation_block`,
    // Decision A), on the same cadence, under the reason the gauge already has for this
    // path (`crash_recover`): the node stays observable instead of exiting on a link.
    let uf = refetch_hole_until_answered(
        upstream,
        committees,
        verify_ctx,
        at_hash,
        h,
        "finalized_blocks",
        sync_metrics,
    )
    .await?;
    sync_metrics.crash_recover_refetched.inc();
    warn!(
        height = h,
        "crash-survivor recovery: healed a below-floor marshal block-archive hole by a \
         BLS-verified by-height re-fetch through the cert upstream"
    );
    Ok(uf.block)
}

/// Where σ for a replayed height comes from, decided before any I/O runs.
enum ReplaySeedSource {
    /// The agreed derivation here is `None`: the beacon is not mandatory in this
    /// height's epoch, or the epoch map cannot name an epoch for it at all.
    Inactive,
    /// σ this node already holds for the height's OWN round.
    Held(Seed),
    /// Beacon-active and the store missed — σ for this round has to be found.
    Wanted(Round),
}

/// What the crash-survivor replay may derive a height with.
enum ReplaySeed {
    /// `None` iff [`ReplaySeedSource::Inactive`] — a beacon-inactive link, where
    /// `None` is what every node derives. It is NOT reachable from a σ miss: the
    /// digest fallback on a beacon-active link re-rolls `prev_randao` and forks
    /// the restart, which is the one outcome this whole path exists to prevent.
    Derive(Option<Seed>),
    /// σ is mandatory here and no source has it. The walk stops and defers.
    Unavailable,
    /// σ is mandatory here, a certificate for the round CARRIES it, and this node
    /// cannot check it yet: the epoch key is not resolvable locally. The beacon is
    /// holding the value (`Observed::Pending`) and will settle it on the
    /// `KeyAvailable` edge, so the walk defers instead of deriving — and never
    /// fatally, whatever the upstream configuration.
    Defer,
}

/// σ for `height`'s own round out of this node's own store, or the round to go
/// looking for. The same rule the live executor derives with
/// (`Actor::seed_at_own_round`), applied at replay so a restarted node cannot
/// re-execute a height with a different `prev_randao` than the network.
///
/// PREDICATE FIRST, store second: `mandatory_at(epoch(h))` — network-agreed,
/// independent of anything local — decides BEFORE the store is read. Store-first
/// would let a σ filed at a round the agreed map calls beacon-INACTIVE be USED,
/// and the seed journal this store rehydrates from is replayed WITHOUT
/// re-verification by design (`VerifiedSeed::from_journal`: epoch keys are pruned
/// on an epoch window while σ is kept on a round window, so a re-check is
/// impossible). A stray there is ignored and counted — never obeyed, never fatal:
/// ignoring derives exactly what the rest of the network derives, where halting
/// would turn one bad record into a node that cannot start.
///
/// A height whose epoch the map cannot name (below the epocher origin) is
/// INACTIVE and is never unwrapped: the beacon cannot have been mandatory in an
/// epoch that does not exist.
fn replay_seed_source(
    beacon: &dyn Beacon,
    epocher: &OriginEpocher,
    height: u64,
    proposal_view: u64,
    sync_metrics: &SyncMetrics,
) -> ReplaySeedSource {
    let Some(info) = epocher.containing(Height::new(height)) else {
        return ReplaySeedSource::Inactive;
    };
    let round = Round::new(info.epoch(), View::new(proposal_view));
    if !beacon.mandatory_at(round.epoch().get()) {
        // Read ONLY to count it: the value is never handed on.
        if beacon.seed(round).is_some() {
            sync_metrics.crash_recover_stray_seed.inc();
            warn!(
                height,
                %round,
                "crash-survivor recovery: σ present at a round the agreed epoch map calls \
                 beacon-INACTIVE — IGNORED (the network derives `None` here)"
            );
        }
        return ReplaySeedSource::Inactive;
    }
    match beacon.seed(round) {
        Some(seed) => ReplaySeedSource::Held(seed),
        None => ReplaySeedSource::Wanted(round),
    }
}

/// What one certificate could give the replay for `round`.
enum CertSeed {
    /// Checked under `PK_E` and filed: the index answers for the round.
    Held(Seed),
    /// The key is not resolvable here yet; the beacon is holding the value.
    Pending,
    /// Nothing usable: a certificate for another round, one carrying no σ, or one
    /// whose σ was REFUSED under the epoch's attested key.
    Absent,
}

/// σ out of a finalization, PINNED to the round the caller named and CHECKED under
/// the epoch key.
///
/// σ signs `seed_message(round)`, so a certificate for another round carries a
/// perfectly valid signature over something else; taking it would be the fork the
/// caller is avoiding. One implementation for both cert sources — the local
/// archive and the upstream — because the rule is the same for both.
///
/// THE CHECK IS NEW (E5-03), and what made it possible is that the miss now has an
/// outcome. This walk used to read σ straight out of the archive with no
/// verification, justified as "the epoch key needed for the check is also the one
/// thing a restart may legitimately not have" — true, and the wrong conclusion: the
/// two cases are distinguishable, and the beacon is what distinguishes them.
/// `Observed::Pending` IS "no key here yet", and it defers; `Observed::Refused` is a
/// σ that fails an ATTESTED key, which after П-3 is the only kind there is, so a
/// corrupted or tampered archive record can no longer be derived from. The trust
/// this walk extends to the archive's BLOCK BODIES is unchanged — they are what
/// the derive is of, and the result is cross-checked by consensus.
///
/// The verdict is the beacon's for the same reason the two live ingresses use it:
/// one rule in one place. It also FILES what it checks, so the walk's own read
/// (`Beacon::seed`) is the answer, and a later height of the same round needs no
/// second check.
fn seed_via_beacon(
    beacon: &dyn Beacon,
    round: Round,
    finalization: &Finalization<BlsScheme, Digest>,
) -> CertSeed {
    if finalization.proposal.round != round {
        return CertSeed::Absent;
    }
    match beacon.observe_certificate(ObservedCertificate::Finalization(round, finalization)) {
        Observed::Recorded => match beacon.seed(round) {
            Some(seed) => CertSeed::Held(seed),
            // Unreachable while the index's retention window is measured in
            // thousands of rounds and this read is the same tick as the file: it
            // would take an eviction between the two.
            None => CertSeed::Absent,
        },
        Observed::Pending => CertSeed::Pending,
        Observed::Refused | Observed::Inactive => CertSeed::Absent,
    }
}

/// Resolve σ for one replayed height: this node's store, then the local
/// certificate at that height, then the cert upstream, then defer.
///
/// TRUST CLASSES, and they differ on purpose. The store and the local
/// `finalizations` archive are read WITHOUT re-verification, the same trust the
/// seed journal takes and the same trust this walk already extends to the block
/// bodies it derives from — both were written by this node's own marshal after it
/// verified them, and re-checking one while trusting the other from the same disk
/// would be incoherent (the epoch key needed for the check is also the one thing a
/// restart may legitimately not have). The UPSTREAM read is the only VERIFIED
/// one: `refetch_verified_archive_hole` authenticates it exactly like the
/// cold-start jump landing. Every source is round-pinned by [`seed_via_beacon`],
/// so no source can substitute a neighbouring round's σ.
///
/// A local certificate is often absent and that is normal, not a fault: an
/// ancestry-finalized height may have no standalone cert anywhere, ever. Its σ is
/// then the store's to supply, and a store that lost its journal tail falls
/// through to the upstream — or, failing that, to the caller's defer.
#[allow(clippy::too_many_arguments)]
async fn recover_replay_seed<A, U, C>(
    beacon: &dyn Beacon,
    epocher: &OriginEpocher,
    certs: &A,
    upstream: Option<&U>,
    committees: &C,
    verify_ctx: &mut Context,
    parent_hash: B256,
    sync_metrics: &SyncMetrics,
    order: &OrderBlock,
) -> eyre::Result<ReplaySeed>
where
    A: Archive<Value = Finalization<BlsScheme, Digest>>,
    U: crate::cert_follow::CertUpstream,
    C: crate::cert_inlet::CommitteeSource,
{
    let round = match replay_seed_source(
        beacon,
        epocher,
        order.height,
        order.proposal_view,
        sync_metrics,
    ) {
        ReplaySeedSource::Inactive => return Ok(ReplaySeed::Derive(None)),
        ReplaySeedSource::Held(seed) => return Ok(ReplaySeed::Derive(Some(seed))),
        ReplaySeedSource::Wanted(round) => round,
    };
    let local = certs
        .get(Identifier::Index(order.height))
        .await
        .map_err(|e| {
            eyre!(
                "reading marshal finalizations at height {}: {e}",
                order.height
            )
        })?;
    match local
        .as_ref()
        .map(|cert| seed_via_beacon(beacon, round, cert))
    {
        Some(CertSeed::Held(seed)) => return Ok(ReplaySeed::Derive(Some(seed))),
        Some(CertSeed::Pending) => return Ok(ReplaySeed::Defer),
        Some(CertSeed::Absent) | None => {}
    }
    if upstream.is_some() {
        match refetch_verified_archive_hole(
            upstream,
            committees,
            verify_ctx,
            parent_hash,
            order.height,
            "finalizations (own-round seed)",
        )
        .await
        {
            Ok(Some(uf)) => match seed_via_beacon(beacon, round, &uf.finalization) {
                CertSeed::Held(seed) => {
                    sync_metrics.crash_recover_refetched.inc();
                    return Ok(ReplaySeed::Derive(Some(seed)));
                }
                CertSeed::Pending => return Ok(ReplaySeed::Defer),
                CertSeed::Absent => warn!(
                    height = order.height,
                    %round,
                    "crash-survivor recovery: the upstream's verified finalization carries no σ \
                     this node can use for this round (absent, or refused under the epoch key)"
                ),
            },
            // NO VERDICT (not one upstream answered) — and here that is the SAME
            // answer as a verdict, deliberately: this path never claimed data loss,
            // so it has nothing to withhold. It does not wait either, which is the
            // other half of why the block path and the σ path read this differently:
            // the block must exist before reth can move, while a missing σ only
            // means devp2p carries the EL forward and the caller's defer resumes on
            // `KeyAvailable`. Blocking the replay for a link would be a worse trade
            // than deferring it.
            Ok(None) => warn!(
                height = order.height,
                %round,
                "crash-survivor recovery: not one configured upstream answered the by-height \
                 pull for this round's σ; deferring to devp2p and the caller's resume"
            ),
            // NOT fatal here, where it is fatal for a missing BLOCK: the block is
            // already in hand, so a σ that cannot be fetched is a reason to let
            // devp2p carry the EL forward, not evidence of local data loss. The
            // caller's defer is the self-heal; a forged answer is refused by the
            // same authentication and lands here too, having derived nothing.
            Err(e) => warn!(
                height = order.height,
                %round,
                "crash-survivor recovery: re-fetching the finalization for its σ failed: {e:#}"
            ),
        }
    }
    Ok(ReplaySeed::Unavailable)
}

/// Crash-survivor cold-start recovery: reth is missing the
/// consensus-finalized block at `target` (an ungraceful crash lost reth's
/// unflushed tail while the marshal persisted the finalization). Read the missing
/// block(s) from the marshal's own `finalized_blocks` archive and `new_payload`
/// them into reth, walking ancestors oldest-ward until reth reconnects; return the
/// recovered `target`'s local hash. Standalone archive open (before the engine is
/// built), like the metadata peek — dropped before `MarshalActor::init` re-opens it.
///
/// A gap wider than `MAX_COLD_RECOVER` (#12 — reth
/// deeply behind its INTACT archive) returns [`RecoverOutcome::DeferToElSync`] (WITH
/// an upstream) for the post-engine devp2p jump. A below-floor HOLE in the marshal's
/// own archive (#8) is instead healed INLINE by a BLS-verified by-height re-fetch
/// through `upstream` ([`refetch_verified_archive_hole`]) — the live marshal resolver
/// cannot repair below its floor and the deferred jump can't fire at a `<= 64` gap, so
/// deferring would strand reth. Both no-upstream cases stay fatal.
///
/// A height whose σ cannot be resolved ([`recover_replay_seed`]) takes the SAME
/// defer, mid-walk: the blocks already imported stay, and devp2p carries the EL
/// from reth's new tip. Deriving that height locally is the one thing this
/// function may never do, because a `prev_randao` derived from the digest
/// fallback forks the restart away from the network.
// A single-call pre-engine assembly step: each arg is a distinct reth/consensus
// dependency (engine, provider, deriver, upstream, committee source, checkpoint,
// beacon, epoch map), not a bundleable cluster — an args struct would only add
// indirection.
#[allow(clippy::too_many_arguments)]
async fn recover_finalized_tail_into_reth<Provider, BeaconEngine, D, U, C>(
    ctx: &Context,
    beacon_engine: &BeaconEngine,
    provider: &Provider,
    deriver: &D,
    target: u64,
    upstream: Option<&U>,
    committees: &C,
    sync_metrics: &SyncMetrics,
    beacon: &dyn Beacon,
    epocher: &OriginEpocher,
) -> eyre::Result<RecoverOutcome>
where
    Provider: BlockHashReader + BlockNumReader,
    BeaconEngine: BeaconEngineLike<ExecutionData = D::Derived>,
    D: DerivedBlockBuilder,
    U: crate::cert_follow::CertUpstream,
    C: crate::cert_inlet::CommitteeSource,
{
    // The #8 re-fetch authenticates the upstream cert ITSELF — it is one of the two
    // by-height seams that reach neither `store_finalization` nor
    // `FrontierHandler::deliver`, which is the whole reason `verify_jump_authenticated`
    // still exists after pass Б2. It needs a `&mut Clock + CryptoRngCore`.
    let mut verify_ctx = ctx.clone();
    // An ungraceful crash loses only reth's unflushed tail (typically 1-2 blocks).
    // A larger gap is NOT a recoverable flush race — #12 defers to devp2p EL-sync.
    const MAX_COLD_RECOVER: u64 = 64;

    // Phase 1 (provider-only): find the reconnect point. A gap wider than the
    // flush-race cap is #12 — the pre-engine marshal→reth replay can't bridge it.
    let lowest = match recover_reconnect_point(provider, target, MAX_COLD_RECOVER)? {
        ReconnectScan::Reconnect(lowest) => lowest,
        ReconnectScan::TooDeep => {
            return crash_recover_defer_or_fatal(
                provider,
                target,
                upstream.is_some(),
                sync_metrics,
                &format!("reth is > {MAX_COLD_RECOVER} blocks behind its own finalized archive at {target}"),
            );
        }
    };

    // Phase 2: replay [lowest..=target] from the marshal's OWN finalized_blocks
    // archive. σ for height `h` is resolved at `h`'s OWN round — the SAME key the
    // live executor derives with, so a restarted node can never re-execute a
    // height with a different `prev_randao` than the network (F4).
    //
    // The finalizations archive is opened as a σ FALLBACK only
    // ([`recover_replay_seed`]), never as a gate: a present block with an absent
    // cert is a NORMAL state (an ancestry-finalized height may have no standalone
    // cert anywhere, ever — pre-B′ this was classified as a re-fetchable hole, the
    // upstream could not serve it, and the node could NEVER restart). Nothing here
    // requires a cert to exist.
    let archive = crate::outer::init_finalized_blocks_archive(ctx, MARSHAL_PARTITION_PREFIX).await;
    let certs = crate::outer::init_finalizations_archive(
        ctx,
        MARSHAL_PARTITION_PREFIX,
        commonware_runtime::buffer::paged::CacheRef::from_pooler(
            ctx,
            crate::outer::PAGE_CACHE_PAGE_SIZE,
            crate::outer::PAGE_CACHE_CAPACITY,
        ),
    )
    .await;

    let mut parent_hash = provider
        .block_hash(lowest.saturating_sub(1))
        .wrap_err("provider.block_hash at recovery reconnect point")?
        .ok_or_else(|| {
            eyre!(
                "crash-survivor recovery: no reconnect parent below height {lowest}; \
                 re-sync the EL disk from a snapshot"
            )
        })?;
    for h in lowest..=target {
        // Each walk element is acquired at the top of its OWN iteration. The
        // one-height offset that used to sit here existed only to have `h+1` in
        // hand for its `parent_seed`; nothing reads a child now.
        let order = recover_walk_block(
            &archive,
            upstream,
            committees,
            &mut verify_ctx,
            parent_hash,
            sync_metrics,
            h,
        )
        .await?;
        // The witness-downgrade refusal that stood here is GONE with the datum it
        // read: it compared `h`'s own field against `h+1`'s, an archive-internal
        // monotonicity check over two bodies. Its replacement is stronger, not
        // absent — beacon-activity is now decided by the AGREED epoch map instead
        // of by a block's own bytes, so a corrupted archive cannot assert its way
        // onto either side, and a σ miss on a beacon-active link stops the walk
        // instead of deriving with the digest fallback: the fork the old refusal
        // was really guarding against.
        let seed = match recover_replay_seed(
            beacon,
            epocher,
            &certs,
            upstream,
            committees,
            &mut verify_ctx,
            parent_hash,
            sync_metrics,
            &order,
        )
        .await?
        {
            ReplaySeed::Derive(seed) => seed,
            ReplaySeed::Defer => {
                return crash_recover_defer(
                    provider,
                    target,
                    sync_metrics,
                    &format!(
                        "block {h} sits on a beacon-active link, a certificate for its own round \
                         carries σ, and this node cannot yet check it — the epoch key is not \
                         resolvable here; the beacon holds the value and settles it when the \
                         artifact lands"
                    ),
                );
            }
            ReplaySeed::Unavailable => {
                return crash_recover_defer_or_fatal(
                    provider,
                    target,
                    upstream.is_some(),
                    sync_metrics,
                    &format!(
                        "block {h} sits on a beacon-active link and σ for its own round is in \
                         neither the seed store, the local finalization, nor the upstream — \
                         refusing a digest-fallback derive (it would fork the restart)"
                    ),
                );
            }
        };
        let derived = derive_with_visibility_retry(ctx, deriver, &order, parent_hash, seed)
            .await
            .wrap_err("crash-survivor recovery derivation failed")?;
        parent_hash = derived.evm_hash();
        let status = beacon_engine
            .import_derived(derived)
            .await
            .wrap_err("crash-survivor recovery import failed")?;
        ensure!(
            status.is_valid() || status.is_syncing(),
            "EL rejected recovered finalized block {h}: {status:?}"
        );
        // Per-block FCU, awaited — the SAME visibility sync point the live
        // executor relies on. An InsertExecuted import "adds to canonical
        // chain" but header-by-hash reads do NOT see the block until an FCU
        // lands (observed unbounded, not ms-scale: a 10s retry expired
        // against it), so the next iteration's parent read would fail
        // without this. The retry above still covers the devp2p-concurrent
        // import case, where the canonicalizer is not us.
        let resp = beacon_engine
            .fork_choice_updated(ForkchoiceState {
                head_block_hash: parent_hash,
                safe_block_hash: parent_hash,
                finalized_block_hash: parent_hash,
            })
            .await
            .wrap_err("crash-survivor recovery per-block FCU failed")?;
        // Judged by the next derive, not by the response code: VALID and "parent
        // is visible" diverge in BOTH directions — SYNCING during a backfill does
        // nothing, and INVALID can be returned AFTER canonicalization already
        // happened. The typed `ParentHeaderMissing` on the following iteration is
        // the honest signal; `derive_with_visibility_retry` already absorbs the
        // concurrent-devp2p case here.
        if !resp.is_valid() {
            warn!(
                height = h,
                status = ?resp.payload_status,
                "crash-survivor recovery FCU not VALID; the next derive will report \
                 whether the parent became visible"
            );
        }
    }
    // Released so `MarshalActor::init` can re-open the same partitions.
    drop(certs);
    drop(archive);

    let hash = provider
        .block_hash(target)
        .wrap_err("provider.block_hash after crash-survivor recovery")?
        .ok_or_else(|| {
            eyre!("crash-survivor recovery: block {target} still missing from reth after replay")
        })?;
    Ok(RecoverOutcome::Recovered(hash))
}

/// Operator-supplied per-launch configuration. Keys + JSON-parsed
/// configs arrive pre-loaded (the host crate owns filesystem syscalls
/// and permission checks); the slasher transport arrives pre-built
/// because `PoolTxSink<P, Provider>` carries concrete
/// `reth-transaction-pool` trait bounds that can't compile in this crate.
pub struct DposLayerConfig<D, XC, A, U> {
    pub bls_keypair: ValidatorBlsKeypair,
    pub peer_keypair: commonware_cryptography::ed25519::PrivateKey,
    /// Every per-epoch committee read the layer makes, as one frozen record per
    /// epoch at one anchor. Built in the node crate beside the ordering-finalized
    /// cursor it anchors on (`build_beacon_plane`) and the SAME `Arc` the
    /// beacon's `CommitteeReads` facade views, so the consensus layer and the
    /// beacon plane cannot hold two versions of one epoch's committee.
    pub committee: Arc<dyn crate::committee::Committee>,
    /// `T` — `EpochTransition::last_tracked_epoch`, mirrored into ONE cell whose
    /// single WRITER is this layer's boundary-bridge forwarder (the transition
    /// advances `last_tracked_epoch` only on a successful `boundary_tx.try_send`,
    /// `epoch_transition.rs:768-791`, cold start included — so the epochs the
    /// forwarder drains ARE the epochs that advanced it). Its reader is exactly
    /// ONE: the executor's frontier probe (`executor.rs:656`, read at
    /// `:2099-2103`). `PlaneUpstreamHandle` does not read it — see the warning in
    /// `plane_upstream::PlaneUpstreamHandle::fetch_one`.
    ///
    /// A cell and not a read of the transition: the transition sits behind an
    /// async mutex, and a probe tick must never wait on the epoch machine it is
    /// asking about. `u64::MAX` = nothing tracked yet.
    pub tracked_epoch: Arc<std::sync::atomic::AtomicU64>,
    pub slasher_sink: Arc<dyn SlasherTxSink>,
    /// Evidence-channel bridge to the node's gossip task, which owns both p2p
    /// halves of `EVIDENCE_CHANNEL` ([`crate::slasher::gossip`]).
    pub evidence: crate::slasher::EvidenceBridge,
    pub staking_config: StakingReaderConfig,
    /// Datadir path of the fork-safety halt marker
    /// ([`crate::sync_metrics::SafetyHalt::restoring`]). A marker left by a
    /// previous run brings this node up permanently verify-only — the latch has
    /// no in-process `disengage`, so without it a restart silently cleared a
    /// halt and the node signed again on the same disk. `None` only in tests.
    pub halt_marker: Option<std::path::PathBuf>,
    /// Cert upstream: the marshal's by-height backfill resolver, the frozen-tip
    /// ladder probe, and the steady-state re-jump's EL work all ride it. `Some` for
    /// every launched node since the plane-native default (`node/dpos.rs` wraps both
    /// the `Plane` and the `Ws` branch in `Some`). `None` is a no-upstream validator,
    /// which `resolve_cold_start_kind` refuses for the empty-archive start (nothing
    /// would climb the ladder) and which catches up on the consensus-plane treadmill
    /// otherwise. There is no pre-engine jump any more (pass Б2).
    pub upstream: Option<U>,
    /// OrderBlock → derived-EVM-block execution (node-built over reth-evm).
    pub deriver: D,
    /// Local derived-chain view (node-built over the reth provider).
    pub executed: XC,
    /// Pool-backed ordering assembly (node-built — pool trait bounds live there).
    pub assembler: Arc<A>,
    pub target_gas_limit: u64,
    /// Cert-feed sink (node-built): wired as the marshal's second
    /// application-`Reporter` so a node-side feed actor can serve the
    /// `consensus` RPC. `None` for nodes that don't serve the cert feed.
    pub feed: Option<crate::feed_sink::FeedSink>,
    /// Edge-trigger the executor fires on each finalized-advance — the mid-epoch
    /// promotion trigger for the role reconciler (the executor is the sole reth
    /// writer on a validator; it follows the chain by local derivation).
    pub spawn_unblocked: std::sync::Arc<tokio::sync::Notify>,
    /// The always-on beacon/DKG plane, built ONCE per process in the node crate
    /// (`build_beacon_plane`) and shared across the follower↔signer phase switch.
    /// The
    /// signer engine is a CONSUMER of its shared `ceremony_store` (the per-epoch
    /// `PK_epoch`/share source) and its artifact store plus pull seam, re-uses
    /// its `oracle` (the single network's peer set) + its already-registered
    /// `beacon_metrics`, and CLONES its 5 `MuxHandle`s + `subscribe()`s the vote
    /// backup to wire the OuterEngine's per-promotion sub-channels — it never
    /// re-builds the network, re-spawns the `DkgActor`, re-registers the metrics, or
    /// re-binds `listen`.
    pub beacon_plane: SharedBeaconPlane,
    /// Supervisor handles of the epoch-key agreement instances the beacon plane
    /// starts. Threaded straight through to [`crate::epoch_manager::Actor`], which
    /// owns them and prunes them on the same frontier cutoff as the per-epoch
    /// engines. `None` ⇒ no agreement plane wired.
    pub agreement_intake: Option<mpsc::Receiver<(Epoch, commonware_runtime::Handle<()>)>>,
    /// DEVNET/TEST-ONLY byzantine behaviour (gated behind `dpos-devnet-byzantine`).
    /// Absent — and the field does not exist — in a production build.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub byzantine: Option<crate::byzantine::ByzantineMode>,
}

/// A plane-owned broker handle for one of the 5 non-beacon channels: the single
/// network's `(Sender, Receiver)` pair are owned by a persistent `Muxer` in the
/// always-on plane (node crate); every promotion CLONES this handle (an `Arc`) and
/// registers fresh sub-channels against the SAME broker. A `SubReceiver`
/// auto-deregisters on drop, so a demoted engine that drops its `SubReceiver`s frees
/// the slots and a re-promoted one re-registers — restart-free re-promotion.
///
/// `MuxHandle::register` takes `&mut self`, so the shared handle is wrapped in
/// `Arc<Mutex<_>>`: each `register` (a boundary-rate control-channel round-trip)
/// locks transiently. The derived `Clone` on `MuxHandle<S, R>` carries a spurious
/// `R: Clone` bound that the move-only `DiscReceiver` does NOT satisfy, so the bare
/// `MuxHandle` is itself un-`Clone`able here — the `Arc` is both the sharing
/// mechanism AND the `Clone` we need for `SharedBeaconPlane`.
pub type PlaneMux = Arc<
    Mutex<
        commonware_p2p::utils::mux::MuxHandle<
            fluentbase_p2p::DiscSender<Context>,
            fluentbase_p2p::DiscReceiver,
        >,
    >,
>;

/// The ONE [`EpochTransition`] a validator process runs, handed DOWN from the node
/// crate's always-on plane (where it is built, before the engine, so the geometry it
/// freezes is available to the `DkgActor` and the committee module) together with the
/// receiving half of the boundary bridge it was constructed with.
///
/// It is one instance and not two because two had two `last_tracked_epoch`s, two
/// `anchor_height`s and two `oracle.track` calls per epoch, driven from two different
/// heights: the engine's delivery hook fires on every ordering-finalized block, while
/// the plane's poller reads a COALESCED reth watch — and boundary detection is
/// pointwise, so the poller's driver skips boundaries outright (proved in
/// `staking-reader`'s `a_coalesced_driver_skips_the_boundary_a_stepping_one_enters`).
/// The surviving driver is therefore the delivery hook; the plane keeps only the
/// GEOMETRY FREEZE (`EpochTransition::freeze_geometry`), which writes none of the
/// bootstrap state — so the cold start below is the process's ONE bootstrapper, and
/// the starting epoch is chosen on the ordering scale instead of by a race.
///
/// Both halves travel together because they are one object: `bridge_rx` can only be
/// drained where `OuterEngine::boundary_sender()` exists, which is after `build`.
pub struct PlaneEpochTransition<Provider, EvmConfig> {
    /// The instance itself. `Arc<Mutex<_>>` because the delivery hook, the
    /// executor's read-floor seam and the plane poller all call into it.
    pub transition:
        Arc<Mutex<EpochTransition<RethStakingStateReader<Provider, EvmConfig>, OracleHandle>>>,
    /// Receiving half of the transition's `boundary_tx`. Drained by the
    /// `epoch_bridge` forwarder into `OuterEngine::boundary_sender()`.
    pub bridge_rx: mpsc::Receiver<(u64, fluentbase_staking_reader::reader::ValidatorSetSnapshot)>,
}

/// Why the frontier probe could not put its ladder step this tick, as the
/// `dpos_frontier_step_skipped_total{reason}` label. One function so the label
/// set cannot drift between the two places a probe is built.
pub(crate) fn step_skip_reason(e: &crate::committee::CommitteeError) -> &'static str {
    match e {
        crate::committee::CommitteeError::OutOfWindow { .. } => "out_of_window",
        crate::committee::CommitteeError::NotReadable { .. } => "not_readable",
        crate::committee::CommitteeError::Read(_) => "read_failed",
    }
}

/// The executor's frozen-tip frontier probe (see [`crate::executor::ReJump::probe`]),
/// built ONCE here for BOTH launch paths — the plane-native validator and the
/// follower.
///
/// TWO requests per tick (§5.2 "Триггер и лестница"): the untargeted `Latest`,
/// whose height is the hint driver, and the LADDER STEP `Finalized{last(T+1)}`.
/// This closure only NAMES the step and its addressees; the executor puts it on
/// the MARSHAL's own resolver (`marshal.hint_finalization(height, targets)`), and
/// it is that resolver which carries the targets to `committee[T+1]` — the set
/// that finalized that height. Both answers are judged by
/// [`crate::plane_upstream::FrontierHandler`]'s `deliver`; the step's never comes
/// back here, it goes into the marshal and shows up as the tip moving.
///
/// `last(T+1)` and `committee[T+1]` both come from the committee module — ONE
/// geometry and ONE committee map per process. An unreadable `committee[T+1]` is
/// not a failure, it is "this node cannot name the addressee yet": count it and
/// ask `Latest` alone.
///
/// ONE constructor and not two closures, because the two node classes have to
/// climb the SAME ladder (review B1-01): the follower used to wire `probe: None`,
/// which after §5.2 removed `upstream_frontier` left it with no way out of the
/// "committee[E] not committed" defer at all. Its `Latest`/by-height seam is its
/// WS upstream instead of the frontier resolver, and that is the only difference
/// — it is the `U: CertUpstream` argument, not a second body.
pub(crate) fn frontier_probe<U: crate::cert_follow::CertUpstream>(
    up: U,
    committee: Arc<dyn crate::committee::Committee>,
) -> crate::executor::FrontierProbeFn {
    Arc::new(move |tracked: Option<u64>| {
        let up = up.clone();
        let committee = committee.clone();
        Box::pin(async move {
            let step = match (tracked, committee.geometry()) {
                (Some(t), Some(geometry)) => {
                    let next = t + 1;
                    match committee.committee(next) {
                        Ok(record) => {
                            let targets: Vec<_> = record.participants.iter().cloned().collect();
                            // An empty `participants` is a readable record with
                            // nobody to ask. It skips the step like every other
                            // unnameable addressee, so it owes the dashboard the
                            // same `reason` the others give.
                            match commonware_utils::vec::NonEmptyVec::try_from(targets) {
                                Ok(t) => Some((Height::new(geometry.last(next)), t)),
                                Err(_) => {
                                    metrics::counter!(
                                        crate::executor::FRONTIER_STEP_SKIPPED,
                                        "reason" => "no_participants",
                                    )
                                    .increment(1);
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            metrics::counter!(
                                crate::executor::FRONTIER_STEP_SKIPPED,
                                "reason" => step_skip_reason(&e),
                            )
                            .increment(1);
                            None
                        }
                    }
                }
                (None, _) => {
                    metrics::counter!(
                        crate::executor::FRONTIER_STEP_SKIPPED,
                        "reason" => "no_tracked_epoch",
                    )
                    .increment(1);
                    None
                }
                (Some(_), None) => {
                    metrics::counter!(
                        crate::executor::FRONTIER_STEP_SKIPPED,
                        "reason" => "no_geometry",
                    )
                    .increment(1);
                    None
                }
            };
            let latest = crate::cert_follow::CertUpstream::get_latest(&up).await;
            crate::executor::ProbeOutcome {
                frontier: latest.map(|uf| Height::new(uf.block.height)),
                step,
            }
        })
    })
}

/// `T` for a node that runs NO [`fluentbase_staking_reader::EpochTransition`] —
/// the follower (review B1-01).
///
/// A validator mirrors `EpochTransition::last_tracked_epoch` off the boundary
/// bridge; a follower spawns no per-epoch engine and therefore no transition, so
/// it computes the SAME number from the two things it does have: the committee
/// module's geometry and its own ordering-finalized cursor — the very cursor the
/// module already anchors its reads on (`RethAnchor(finalized_cursor)`), not a
/// new source.
///
/// THE RULE IS ET'S, restated over those two, not a second convention
/// (`staking-reader/src/epoch_transition.rs:550-585`): the transition tracks
/// `epoch_e + 1` when the finalized block is the LAST block of its epoch (both
/// the cold-start arm `:558` and the boundary arm `:580`) and `epoch_e`
/// otherwise. `geometry.last(epoch_of(fin)) == fin` is that boundary test — the
/// activation-relative one `is_epoch_boundary` makes (`:532`), since
/// `Geometry::last` is built from the same `(activation, interval)` pair.
///
/// `None` while the geometry is unfrozen, and ONLY then — the one state with no
/// step to take, which the probe counts as `no_tracked_epoch`/`no_geometry` and
/// asks `Latest` alone. A cursor still at its seed below activation is not a
/// second refusal, though this doc used to promise one: `Geometry::epoch_of`
/// clamps a pre-activation height to epoch `0` (`epoch_at_block`'s
/// `saturating_sub`, `types/src/staking_protocol.rs:177`), so the answer there
/// is `Some(0)`.
pub(crate) fn local_tracked_epoch(
    committee: Arc<dyn crate::committee::Committee>,
    cursor: crate::FinalizedCursor,
) -> crate::executor::TrackedEpochFn {
    Arc::new(move || {
        let geometry = committee.geometry()?;
        let fin = cursor.height();
        let e = geometry.epoch_of(fin);
        Some(if geometry.last(e) == fin { e + 1 } else { e })
    })
}

/// The persistent beacon/DKG plane handed DOWN from the node crate's always-on
/// component into each per-promotion signer engine. The node crate owns the single
/// `FluentP2P` (beacon halves + `DkgActor` consume their channel there; the 5
/// non-beacon channels are owned by 5 persistent plane `Muxer`s), the
/// EpochTransition-driven Oracle peer-set, the `dkg_height` clock, and reloads the
/// `ceremony_store` from `<datadir>/beacon/` once at startup; the signer engine
/// reads the SAME shared `Arc`s and CLONES the 5 `MuxHandle`s per promotion. There is
/// exactly ONE network / listen bind / peer set / broker set per process — a
/// demote→re-promote within one process needs no network rebuild (the engine drops
/// its `SubReceiver`s on demote; the next promotion re-registers fresh ones).
#[derive(Clone)]
pub struct SharedBeaconPlane {
    /// The single network's Oracle (the one `Clone` p2p handle), used by the
    /// engine's blocker/provider + its EpochTransition peer-set sink.
    pub oracle: fluentbase_p2p::OracleHandle,
    /// The consensus-facing randomness surface, built by `beacon::build`. The
    /// layer threads it and never opens it.
    pub randomness: Arc<dyn crate::beacon::Beacon>,
    /// The 5 plane-owned non-beacon channel broker handles (vote/cert/resolver are
    /// per-epoch register/deregister; broadcast/marshal register subchannel 0 once
    /// per promotion). Cloned per promotion; the Muxer tasks live in the plane.
    pub vote_mux: PlaneMux,
    pub cert_mux: PlaneMux,
    pub resolver_mux: PlaneMux,
    pub broadcast_mux: PlaneMux,
    pub marshal_mux: PlaneMux,
    /// Committee members observed slashed for equivocation. Written by the plane's
    /// tombstone watcher (the sole writer, riding the finalized-height poller);
    /// read by every promoted engine's `FluentApp`. One instance per process, and
    /// the reader must be handed the writer's own handle rather than a second
    /// empty one.
    pub tombstones: crate::slasher::TombstoneSet,
    /// The ordering-vs-DKG clock pair (see [`crate::sync_metrics::PlaneClock`]).
    /// Registered and half-written by the plane's finalized poller; the other
    /// half is written by `FluentApp` off marshal's tip. Same reason as
    /// `tombstones` for arriving from the node crate rather than being defaulted
    /// here: a second instance would be a gauge nothing scrapes.
    pub plane_clock: crate::sync_metrics::PlaneClock,
    /// The beacon plane's height channel, so `FluentApp` can feed marshal's
    /// ordering tip into it. Travels with the plane rather than being created
    /// here for the same reason `plane_clock` does: the receiver is the plane's
    /// `DkgActor`, and a second channel would be a feeder nothing drains.
    pub dkg_height_tx: mpsc::Sender<u64>,
}

/// Cold-start kind resolved from durable state. Pure function of the inputs
/// so the decision is unit-testable without a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColdStartKind {
    /// Empty archive, EL at/inside epoch 0: anchor at the activation block.
    FreshMigration,
    /// Populated archive: resume at its finalized height (real consensus
    /// state always wins).
    Restart,
    /// Empty archive but the EL is already past epoch 0 — a node whose consensus
    /// store was lost or never existed while its reth datadir kept going. Anchor at
    /// reth's OWN finalized tag `(cs_finalized, cs_finalized_hash)` and let the
    /// ladder + the steady-state jump carry it forward (Д-2(а)).
    ///
    /// The anchor is NOT the genesis (`archive_finalized`, where a runtime-deployed
    /// ChainConfig is codeless) and NOT an upstream's `Latest` (nothing local can
    /// check it). It is the same datum the follower path anchors on (`rf_hash`,
    /// `derive_cold_start_heights`), written by exactly one thing — an FCU this node
    /// itself issued, or the pre-DPoS sequencer's.
    ElFinalized,
}

fn resolve_cold_start_kind(
    archive_finalized: u64,
    activation: u64,
    interval: u64,
    cs_finalized: u64,
    has_upstream: bool,
) -> eyre::Result<ColdStartKind> {
    // `0` is the unscheduled sentinel (`setDposActivationBlock` requires a future
    // block, so a live ChainConfig never stores `0` for a genuine migration). At the
    // materialized geometry anchor a real migration always reads nonzero; a `0` here
    // means an unscheduled / mis-configured chain — fail loud rather than anchor DPoS
    // at block 0.
    ensure!(
        activation != 0,
        "dposActivationBlock is 0 (the unscheduled sentinel); DPoS must not launch \
         on an unscheduled ChainConfig"
    );
    if archive_finalized > activation {
        return Ok(ColdStartKind::Restart);
    }
    if cs_finalized >= activation + interval {
        // EL past epoch 0 with an empty consensus archive: anchor at reth's own
        // finalized tag (`ElFinalized`). An UPSTREAM is still required, but the
        // reason changed with pass Б2 — it is no longer "something has to serve a
        // frontier to jump to", because this path no longer jumps at boot. It is
        // that every route out of the gap runs through a peer: the ladder's
        // `Finalized{last(T+1)}` probe (§5.2), the marshal's by-height pulls, and
        // the steady-state jump's own target, which only exists once the marshal has
        // stored something. A node with no upstream at all would anchor here and
        // never move.
        ensure!(
            has_upstream,
            "EL is past epoch 0 (finalized {cs_finalized} >= activation {activation} + interval \
             {interval}) with an empty consensus archive and NO sync upstream (not plane-tracked \
             and no --dpos.follower-upstream): the node would anchor at its own EL-finalized tag \
             and have nobody to climb the ladder from. Register+activate the validator (it then \
             joins the plane and catches up by verified steps), or give it \
             --dpos.follower-upstream (a WS peer this node dials BY URL, so the ladder step and \
             the marshal's by-height pulls stop going through the consensus plane's peer set), \
             or restore the consensus archive."
        );
        return Ok(ColdStartKind::ElFinalized);
    }
    Ok(ColdStartKind::FreshMigration)
}

/// What a FRESH follower datadir — one with no local `ChainConfig`, so no
/// geometry, no committee and no archive — is allowed to use as its EL entry
/// (§5.2 "Правило единое"). The ONE place in the system where nothing local can
/// check a peer's answer, so the choice is a policy and not a lookup: pure, and
/// unit-tested as such ([`fresh_follower_entry`] in `cold_start_kind_tests`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FreshFollowerEntry {
    /// The operator named a block: `sync_to_checkpoint(hash)`.
    Checkpoint(B256),
    /// LOCAL/test network only: take the upstream's word for its own tip. The one
    /// surviving `get_latest ⇒ sync_to`, and it is named for what it is.
    UpstreamLatest,
}

/// The policy itself. On a DEPLOYED network (`deployed_network`, evaluated by the
/// node — `node/dpos.rs::is_deployed_network`) a missing checkpoint is a startup
/// REFUSAL (E4-05): trusting one peer on first use is exactly the unauthenticated
/// entry §5.2 removes, and unlike the other two entries there is no local datum to
/// fall back on.
fn fresh_follower_entry(
    l1_checkpoint: Option<B256>,
    deployed_network: bool,
    chain_id: u64,
) -> eyre::Result<FreshFollowerEntry> {
    match l1_checkpoint {
        Some(hash) => Ok(FreshFollowerEntry::Checkpoint(hash)),
        None if deployed_network => Err(eyre!(
            "cert-follow: fresh datadir on a deployed network (chain_id {chain_id}) with no \
             --dpos.l1-checkpoint. A datadir with no ChainConfig has no geometry, no committee \
             and no archive, so there is NOTHING this node can check an upstream's answer \
             against — syncing to one would be trust-on-first-use against a single peer. \
             Restart with --dpos.l1-checkpoint pointing at the L1 Rollup contract (the \
             checkpoint block is then the entry), or restore a datadir that already holds the \
             ChainConfig."
        )),
        None => Ok(FreshFollowerEntry::UpstreamLatest),
    }
}

/// What a follower that DOES have a local `ChainConfig` (geometry readable at
/// `rf_hash`) uses as its EL entry — the five-way march of `launch_follower`'s
/// `Some((activation, interval))` arm (R-131 / PLAN row 4.4).
///
/// Deliberately NOT [`ColdStartKind`]: that enum is the VALIDATOR discriminator
/// (`resolve_cold_start_kind`), it is named by the staking-reader's doc contract
/// (`fluentbase_staking_reader::reader`, the `activation == 0` sentinel), and its
/// three variants answer a different question (which anchor a populated/empty
/// consensus archive resumes at). One enum serving both marches would tie two
/// unrelated decisions together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FollowerEntry {
    /// reth already holds the activation block (or its finalized tag is already at
    /// or above it): the anchor is local, and no peer is contacted at all. Also the
    /// arm a node that produced the activation block itself lands in.
    Local,
    /// No entry of any kind: no operator checkpoint and no upstream — the honest
    /// sequencer→DPoS migration, where the block will be produced by the pre-DPoS
    /// sequencer on this very chain, so the node waits for reth to hold it
    /// (`wait_for_activation_block`, retry-forever). LAST of the three peer-free
    /// steps, not the second: it is what is left when nothing else can be tried.
    WaitLocal,
    /// An operator checkpoint is configured and not yet consumed. It is the FIRST
    /// entry tried after the local probe, for two independent reasons:
    ///
    /// * before the CERTIFICATE entry, because `assert_l1_checkpoint` runs after the
    ///   match and is counted FROM THE LANDING, and the certificate entry lands on
    ///   the LOWEST legal height (`activation`), so the reverse order would turn a
    ///   survivable park into a fatal refusal (К-73);
    /// * before [`FollowerEntry::WaitLocal`], because `sync_to_checkpoint` needs no
    ///   upstream at all — it FCUs to the operator's hash and lets devp2p backfill.
    ///   Ordering it after the upstream test parked a node that had an entry.
    Checkpoint,
    /// The ordering chain is not usable as an entry yet, for either of two reasons
    /// the caller distinguishes in its `warn!`: the upstream serves no `latest` at
    /// or above `activation + K` (no certificate below `activation + K` carries a
    /// real EVM hash — `order_block::result_target`), or no epoch's committee is
    /// readable at `rf_hash` yet (the window where `setDposActivationBlock` has run
    /// but `commitEpochCommittee(0)` has not). Both are "ask again", never a fatal:
    /// the input is external, exactly the `wait_for_activation_block` argument.
    ChainBelowActivation,
    /// Certificate entry: fetch the finalization for `target` from the upstream,
    /// authenticate it under the committee read at `rf_hash`, and EL-sync to its
    /// attested result. `target` is the HIGHEST height this node can still check —
    /// the one a cascading donor's `JUMP_THRESHOLD` window and a jumped validator's
    /// archive lose LAST — so one request per attempt replaces a by-height walk.
    Certificate { target: u64 },
}

/// The two inputs of [`follower_entry`] that cost a PEER ROUND TRIP, so that the
/// signature says which ones do: everything else in the march is read locally.
/// [`Default`] (both absent) is the peer-free pass the caller runs first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct PeerProbes {
    /// `CertUpstream::get_latest().block.height` — used for ROUTING and as the
    /// height ceiling only; the hash that comes with it is never read.
    latest_height: Option<u64>,
    /// The HIGHEST epoch whose committee reads `Ok` at `rf_hash`, probed over
    /// `0..=MAX_COMMITTEE_LOOKAHEAD_EPOCHS`; `None` when none does.
    ///
    /// "Highest readable" is the right ceiling only because readability is MONOTONIC
    /// FROM ZERO — `commitEpochCommittee` runs upward from epoch 0 and a
    /// pre-activation state has pruned none of them. With a hole (epoch 2 readable,
    /// epoch 0 not) and a tip inside epoch 0, the target would land in the epoch
    /// whose committee is unreadable and the fetch would refuse it forever. Nothing
    /// enforces the monotonicity; it is a property of the commit order, recorded
    /// here because the ceiling depends on it (R-131 review, D-03).
    e_max: Option<u64>,
}

/// The march itself: pure, so the order of the five entries is unit-testable
/// without a node ([`follower_entry`] in `cold_start_kind_tests`).
///
/// The [`PeerProbes`] are `Option` because the caller learns them only by ASKING A
/// PEER, and the first three entries (local anchor, operator checkpoint,
/// wait-for-sequencer — in that order) must not cost a round trip. The
/// caller therefore evaluates this twice: once with both `None` (the peer-free
/// prefix — `Local` / `WaitLocal` / `Checkpoint` are final there), and again with
/// the probes filled in only when the first verdict was
/// [`FollowerEntry::ChainBelowActivation`]. Re-deciding through the same function
/// is what keeps the peer-free prefix from being a second copy of the predicate.
fn follower_entry(
    holds_activation: bool,
    has_upstream: bool,
    has_checkpoint: bool,
    probes: PeerProbes,
    activation: u64,
    interval: u64,
    rf_num: u64,
) -> FollowerEntry {
    // `rf_num >= activation` is today's local path and does not depend on the
    // probe: reth's own finalized tag already sits at or above the activation
    // block, so the anchor is `(rf_num, rf_hash)` whatever a concurrent read of
    // `block_hash(activation)` says.
    if holds_activation || rf_num >= activation {
        return FollowerEntry::Local;
    }
    // THE CHECKPOINT COMES BEFORE THE UPSTREAM TEST, and the order is a fix rather
    // than a preference: `sync_to_checkpoint` needs no `CertUpstream` at all (it FCUs
    // to the operator's hash and lets devp2p backfill), so gating it behind "an
    // upstream is configured" parked a node that had a perfectly good entry.
    if has_checkpoint {
        return FollowerEntry::Checkpoint;
    }
    if !has_upstream {
        return FollowerEntry::WaitLocal;
    }
    // The lowest height whose certificate carries a real EVM hash: `result_target`
    // answers `PreActivation` below `anchor + K`, so a target under it would
    // EL-sync toward `B256::ZERO`.
    let floor = activation.saturating_add(K);
    let (Some(latest), Some(e_max)) = (probes.latest_height, probes.e_max) else {
        return FollowerEntry::ChainBelowActivation;
    };
    if latest < floor {
        return FollowerEntry::ChainBelowActivation;
    }
    // `last(e)` — the terminal height of epoch `e` in absolute numbering. The top
    // of the readable window is `last(e_max)`; the chain's own tip caps it.
    let last_readable = activation
        .saturating_add(e_max.saturating_add(1).saturating_mul(interval))
        .saturating_sub(1);
    // THE READABLE WINDOW CAN END BELOW THE FLOOR — whenever
    // `(e_max + 1) · interval <= K`, which at `e_max = 0` is any `interval <= K` and
    // nothing forbids: the contract rejects only a ZERO interval
    // (`contracts/staking/src/config.rs`, `set_epoch_block_interval`) and
    // `read_geometry` only `> 0`. There is then no height that is both checkable and
    // at or above the floor, which is the same state as "the chain is not there yet"
    // and gets the same answer. Clamping the target UP to the floor instead — the
    // `.max(floor)` this replaced — asked the upstream for a height in an epoch
    // whose committee is NOT readable at `rf_hash`, so `fetch_verified_entry`
    // refused it forever and the node parked on a message about the upstream for a
    // fault of the geometry (R-131 review, D-03).
    if last_readable < floor {
        return FollowerEntry::ChainBelowActivation;
    }
    // No `.max(floor)` here, and none is reachable: both inputs of the `min` are at
    // or above the floor — `latest` by the gate above, `last_readable` by this one.
    let target = latest.min(last_readable);
    FollowerEntry::Certificate { target }
}

/// The entry march's OWN by-height fetch: pull the finalization for `height`, PIN it
/// to the request, bind the certificate to the body it arrived with, and BLS-verify
/// it under `committee[E]` read at `at_hash` — the three §5.2 properties, applied
/// where a failure means "this node has no entry yet and will ask again".
///
/// **Deliberately not `cert_follow::fetch_verified_boundary`, which runs the very
/// same four checks: what differs is the failure SURFACE, and that surface belongs
/// to another consequence.** That seam increments `jump_boundary_refetch_failed` —
/// the epoch-boundary seeding counter two other call sites share — and warns that
/// "this member stays verify-only (no proposals, no votes) until the next epoch
/// boundary", which for a follower still inside `launch_follower` is false twice
/// over: it is not a committee member, it takes no admission, it parks. On a
/// 2-second cadence that put two contradicting diagnoses in the operator's log
/// forever and moved a counter about a different event (R-131 review, D-04). The
/// precedent for a caller owning its own surface over these same two verifiers is
/// [`refetch_verified_archive_hole`].
///
/// The reason comes BACK to the caller instead of being logged here, so the park
/// prints one diagnosis and re-prints only when the reason CHANGES; the metric is
/// the arm's own `dpos_sync_degraded{reason=activation_wait}`, already raised for
/// exactly this park.
async fn fetch_verified_entry<U, C>(
    upstream: &U,
    committees: &C,
    verify_ctx: &mut (impl commonware_runtime::Clock + rand_core::CryptoRngCore),
    at_hash: B256,
    height: u64,
) -> Result<crate::cert_follow::UpstreamFinalized, String>
where
    U: crate::cert_follow::CertUpstream,
    C: crate::cert_inlet::CommitteeSource,
{
    // `_everywhere`: ONE ask per `ACTIVATION_POLL`, and a miss costs this node its
    // entire entry — the shape the method's own doc reserves it for. (The
    // pathological case it warns against is the marshal's per-sweep fan-out, which
    // this is not.)
    let Some(uf) = upstream
        .get_finalization_everywhere(Height::new(height))
        .await
    else {
        return Err("no configured upstream serves the height".to_owned());
    };
    // Nothing else binds the answer to the question: `verify_jump_structural` ties
    // the cert only to the block it came with, and `verify_jump_authenticated` takes
    // the epoch from the cert's own round.
    if uf.block.height != height {
        return Err(format!(
            "an upstream served height {} instead of the one asked for",
            uf.block.height
        ));
    }
    if let Err(e) = crate::cold_start_jump::verify_jump_structural(&uf) {
        return Err(format!(
            "the served certificate does not sign the served block ({e:#})"
        ));
    }
    if let Err(e) =
        crate::cold_start_jump::verify_jump_authenticated(&uf, committees, at_hash, verify_ctx)
    {
        return Err(format!(
            "the served finalization is not a 2f+1 multisig under the committee read at this \
             node's own finalized hash ({e:#})"
        ));
    }
    Ok(uf)
}

/// #17 SELF-HEAL visibility belt: a cold-start / follower landing read of a block
/// reth JUST materialized (via an EL-sync jump / devp2p canonicalization) can
/// transiently return `None` — reth-2.2 canonicalizes on the engine-tree thread a
/// few ms before provider reads see the block (the same race
/// `derive_with_visibility_retry` absorbs on the derive side). Retry `read` on the
/// same 100 ms/10 s cadence under `dpos_sync_degraded{reason=landing_wait}=1`;
/// fatal ONLY after the belt expires (a materialized-but-missing read is then the
/// genuine data-loss fault). Bounded (not retry-forever): unlike an external
/// peer/anchor wait, a landing that never materializes past its own EL-sync is a
/// local fault, not a correlated one.
async fn read_with_visibility_belt<T, F, C>(
    ctx: &C,
    sync_metrics: &SyncMetrics,
    what: &str,
    mut read: F,
) -> eyre::Result<T>
where
    C: commonware_runtime::Clock,
    F: FnMut() -> eyre::Result<Option<T>>,
{
    // Same constants as `derive_with_visibility_retry` (application.rs).
    const RETRY: Duration = Duration::from_millis(100);
    const DEADLINE: Duration = Duration::from_secs(10);
    let deadline = ctx.current() + DEADLINE;
    let mut waited = false;
    loop {
        if let Some(v) = read()? {
            if waited {
                sync_metrics.recover(SyncReason::LandingWait);
            }
            return Ok(v);
        }
        if ctx.current() >= deadline {
            if waited {
                sync_metrics.recover(SyncReason::LandingWait);
            }
            return Err(eyre!(
                "reth does not hold {what} after the visibility belt ({DEADLINE:?}) expired"
            ));
        }
        waited = true;
        sync_metrics.degrade(SyncReason::LandingWait);
        ctx.sleep(RETRY).await;
    }
}

/// Peek the consensus marshal archive's last-finalized height — the same value
/// the cold-start discriminator reads. Returns 0 when the archive is
/// empty/absent (no consensus state ever persisted). The unified supervisor's
/// entry rule uses this to avoid choosing signer-first for an in-committee node
/// that has no consensus state to resume (which would otherwise hit the
/// `resolve_cold_start_kind` "empty archive + EL past epoch 0" fatal instead of
/// following to build the archive first).
pub async fn peek_consensus_archive_last_finalized(ctx: &Context) -> eyre::Result<u64> {
    read_consensus_archive_last_finalized(ctx, MARSHAL_PARTITION_PREFIX).await
}

pub struct DposLayerHandle {
    pub consensus_handle: Handle<()>,
    /// Marshal mailbox clone for the node-side cert feed/RPC (by-height
    /// `get_finalization`+`get_block`). The node calls `feed_handle.set_marshal`
    /// with this once `launch` returns — keeping node types out of consensus.
    pub cert_mailbox: crate::outer::MarshalMailbox,
    /// Layer-internal tasks the HOST must supervise alongside
    /// `consensus_handle`, each with the label the host's exit log prints.
    ///
    /// The runtime runs with `with_catch_panics(true)` and a commonware
    /// `Handle` has NO `Drop` impl, so a DROPPED handle detaches its task: a
    /// panic in it logs one line, resolves nothing, and the node keeps passing
    /// liveness checks with the subsystem dead. Handing the handle up instead
    /// lands it in `dpos.rs::supervise`, where a panic surfaces as
    /// `Err(Error::Exited)` → shared-token cancel → node down (the SAME fatal
    /// semantics `("inlet", h)` already gets on the validator path).
    ///
    /// Appended into the host's `Vec<SupervisedHandle>` — same tuple shape.
    pub supervised: Vec<(&'static str, Handle<()>)>,
    /// Layer-internal tasks the host must LET FINISH on a graceful stop, each
    /// with the label the host's drain log prints. Today: the durable
    /// seed-journal writer, whose last act is to append and fsync whatever the
    /// store queued but had not written yet.
    ///
    /// Same tuple shape as `supervised`, OPPOSITE semantics, and the two must
    /// never be merged: a `supervised` handle resolving means "a subsystem
    /// died, cancel the node", while one of these resolving means "the task
    /// finished the work it owed, shutdown may proceed". Putting a drain task
    /// in `supervised` would make its normal completion look like a crash;
    /// putting a supervised task here would make its crash look like success.
    ///
    /// Every one of these tasks is spawned OUTSIDE the consensus engine's
    /// supervision subtree, because the host awaits them only AFTER aborting
    /// that engine — see `crate::outer::OuterBuilder::build`.
    pub drain_on_shutdown: Vec<(&'static str, Handle<()>)>,
    /// Serve one held epoch-key artifact over `consensus_getEpochArtifact`, so a
    /// TIER-2 follower obtains `PK_epoch` from this node exactly as it obtains
    /// certificates from it.
    ///
    /// `Some` on the FOLLOWER path only, and the asymmetry is not an oversight:
    /// the follower's beacon is built one frame below this crate boundary, so this
    /// closure over its `Beacon::artifact_bytes` is the only way out. A validator
    /// builds its own beacon and calls that method directly.
    pub artifact_bytes: Option<ArtifactSource>,
}

/// One held epoch-key artifact's wire bytes, read out of a beacon.
///
/// A closure over [`Beacon::artifact_bytes`](crate::beacon::Beacon::artifact_bytes)
/// rather than the method itself: the RPC feed stores one serving read and does
/// not hold the beacon.
pub type ArtifactSource = Arc<dyn Fn(u64) -> Option<Vec<u8>> + Send + Sync>;

/// Whether `committee[epoch]` is readable for the follower's boundary trigger —
/// the module's own answer, which is also what REGISTERS the epoch's scheme.
/// `false` ⇒ not readable yet — no executed anchor, or the epoch's committee not committed at
/// it — which the trigger treats as "retry on the next finalized block", never as
/// an empty committee.
type FollowerCommitteeAt = Arc<dyn Fn(u64) -> bool + Send + Sync>;

/// Hand one epoch to the epoch manager's boundary receiver. `false` ⇒ the
/// receiver is gone (the manager exited); the trigger stops.
type FollowerBoundaryDeliver =
    Arc<dyn Fn(Epoch) -> futures::future::BoxFuture<'static, bool> + Send + Sync>;

/// One step of the follower's epoch-boundary trigger: deliver the epoch the
/// finalized stream has entered, at most once per epoch.
/// Returns whether the trigger should keep running.
///
/// A validator gets its boundary deliveries from `EpochTransition`, which rides
/// the beacon plane. A follower has no plane, so it derives the same delivery
/// from the finalized `OrderBlock`s its own marshal reports — the seam
/// `boundary_hook` already exists for. Three things downstream need it and all
/// three were dead on a follower without it: `soft_enter` registering the current
/// epoch's verify-only scheme (without which a resolver-delivered cert for any
/// epoch above the cold-start one finds NO scheme and the marshal answers the
/// fetch `true` without storing — a re-request loop that never closes),
/// `highest_entered_epoch` (the repair sweep's frontier evidence in the window
/// before the geometry freezes, where the live epoch is not yet defined), and
/// the reconcile that the live epoch's own edges would otherwise be the only
/// source of.
///
/// **`last_delivered` advances only after a delivery.** An unreadable committee
/// must leave the epoch unconsumed: `committee[E]` is committed during `E-1` but
/// the read runs at the EL-finalized hash, which trails the ordering-finalized
/// height by `K`, so the first blocks of `E` can legitimately read back nothing.
/// Consuming the epoch there would skip its registration until the NEXT boundary.
///
/// **Epochs this step skipped entirely are deliberately not back-filled**, and
/// they are skipped two ways. A FLOOR MOVE (a cold start or a re-jump walks the
/// marshal floor forward) dispatches no block of the skipped epoch, so no
/// certificate of it is ever fetched and there is nothing to register a scheme
/// for. DRIVER LAG is the case where blocks WERE dispatched: this step reads one
/// latest height and derives one epoch from it, so an epoch that both begins and
/// ends while the step sits inside `committee_at`'s EVM snapshot read is passed
/// over with its blocks already reported — and the repair sweep cannot cover for
/// it either, because its work list is the REGISTERED schemes and a skipped epoch
/// registers none. The cost is the one named above, scoped to that epoch: a
/// resolver-delivered certificate for it finds no scheme and the marshal answers
/// the fetch `true` without storing.
///
/// Accepted rather than back-filled because reaching it means the finalized
/// height advancing a whole `interval` of blocks inside one state read — at the
/// production `epochBlockInterval` of 86 400 that is a day of chain against a
/// single snapshot read — and a back-fill would pay for that reachability with a
/// committee read per skipped epoch on exactly the read that is, by hypothesis,
/// the slow one.
async fn enter_finalized_epoch(
    last_delivered: &mut Option<u64>,
    finalized_height: u64,
    activation: u64,
    interval: u64,
    committee_at: &FollowerCommitteeAt,
    deliver: &FollowerBoundaryDeliver,
) -> bool {
    // The caller resolved the interval off a live chain config, where zero is
    // impossible; treat the `None` as "nothing to deliver" rather than dividing.
    let Some(epoch) =
        fluentbase_staking_reader::reader::epoch_at_block(finalized_height, activation, interval)
    else {
        return true;
    };
    if *last_delivered >= Some(epoch) {
        return true;
    }
    if !committee_at(epoch) {
        return true;
    }
    if !deliver(Epoch::new(epoch)).await {
        return false;
    }
    *last_delivered = Some(epoch);
    true
}

/// Namespace type for the launch entry point.
pub struct DposLayer;

impl DposLayer {
    /// Launch the DPoS layer end-to-end: build the 03 reader, the 05 p2p network and
    /// the 04 OuterEngine; cold-start the plane's transition at this node's own
    /// (post-jump) anchor; spawn forwarder + outer + network; return their
    /// `Handle<()>`s for the host to supervise.
    ///
    /// The [`EpochTransition`] is NOT built here: it arrives as
    /// [`PlaneEpochTransition`] from the always-on plane, which built it before this
    /// launch so the geometry it freezes was already available to the `DkgActor` and
    /// the committee module. This layer supplies the two things only it has — the
    /// per-block delivery driver (`boundary_hook`) and the executor's read-floor seam.
    ///
    /// Caller (the host adapter at `crates/node/src/dpos.rs`) is responsible
    /// for the `select!` supervisor over `shutdown` + the two returned
    /// handles. Caller also performs filesystem key loading
    /// and `PoolTxSink` construction before calling.
    #[allow(clippy::too_many_arguments)]
    pub async fn launch<Provider, EvmConfig, BeaconEngine, D, XC, A, U>(
        ctx: Context,
        reth: RethHandle<Provider, EvmConfig, BeaconEngine>,
        cfg: DposLayerConfig<D, XC, A, U>,
        epoch_transition: PlaneEpochTransition<Provider, EvmConfig>,
        shutdown: CancellationToken,
    ) -> eyre::Result<DposLayerHandle>
    where
        Provider: BlockReader<Block = RethBlock>
            + BlockHashReader
            + BlockNumReader
            + BlockIdReader
            + StateProviderFactory
            + HeaderProvider<Header = Header>
            + Clone
            + Send
            + Sync
            + 'static,
        EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
        BeaconEngine: BeaconEngineLike<ExecutionData = D::Derived> + Clone + Send + Sync + 'static,
        D: DerivedBlockBuilder,
        XC: ExecutedChain,
        A: OrderingAssembler,
        U: crate::cert_follow::CertUpstream,
    {
        let DposLayerConfig {
            bls_keypair,
            peer_keypair,
            committee,
            tracked_epoch,
            slasher_sink,
            evidence,
            staking_config,
            halt_marker,
            upstream,
            deriver,
            executed,
            assembler,
            target_gas_limit,
            feed,
            spawn_unblocked,
            beacon_plane,
            agreement_intake,
            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine,
        } = cfg;

        // The plane owns the 5 persistent Muxers; this promotion CLONES their handles
        // (to register fresh per-epoch / subchannel-0 routes against the SAME broker
        // tasks) and `subscribe()`s a fresh vote-backup receiver. No raw move-only
        // halves are consumed here, so a later demote→re-promote re-clones cleanly.
        let SharedBeaconPlane {
            oracle,
            randomness,
            vote_mux,
            cert_mux,
            resolver_mux,
            broadcast_mux,
            marshal_mux,
            tombstones,
            plane_clock,
            dkg_height_tx,
        } = beacon_plane;

        let RethHandle {
            provider,
            evm_config,
            beacon_engine_handle,
            chain_id,
            canonical_state,
            genesis_hash,
            peer_count,
        } = reth;

        // Self-heal stuck-detector: registered ONCE per launch (mirrors
        // `BeaconMetrics`) and cloned into the cold-start + boundary-hook self-heal
        // loops below. `dpos_sync_degraded{reason}` is the observable that replaces
        // the removed cold-start/boundary `process::exit`s (Decision A).
        let sync_metrics = SyncMetrics::default();
        sync_metrics.register(&ctx);
        // Fork-safety latch (Phase 3): shared across the executor, epoch_manager, and
        // the OuterEngine supervisor. Engaging it (result divergence / EL Invalid /
        // L1 fork) halts participation while the node stays up + observable.
        // A marker left by a previous run re-engages it HERE, before the first
        // consensus event, so a halted node never comes back as a signer.
        let safety_halt = match halt_marker {
            Some(path) => crate::sync_metrics::SafetyHalt::restoring(sync_metrics.clone(), path),
            None => crate::sync_metrics::SafetyHalt::new(sync_metrics.clone()),
        };

        // Build the staking-reader layer: reader + cache + EpochTransition.
        let staking_address = staking_config.staking_address;
        let reader = RethStakingStateReader::new(
            provider.clone(),
            evm_config.clone(),
            staking_config.clone(),
        );

        // Reth's `BlockchainProvider::with_latest` populates
        // `canonical_state.finalized_block` from
        // `ChainState::LastFinalizedBlock` during node init, so on a
        // graceful-shutdown restart `get_finalized_num_hash()` returns
        // `Some(disk_finalized.num_hash())`. The genesis fallback
        // handles the pristine-network case (no FCU yet).
        // Head (`head_num`/`head_hash`) is re-read AFTER the cold-start JUMP
        // below (the jump drives reth's canonical head forward), so only the
        // finalized pair is needed here for the discriminator.
        let (cs_finalized, cs_finalized_hash, _head_num, _head_hash) =
            derive_cold_start_heights(&canonical_state, genesis_hash);

        // Activation origin + epoch length, read EARLY (at the reth-restored
        // finalized hash) — the cold-start discriminator below needs them before
        // `initial_epoch`. `dposActivationBlock` is immutable and
        // `epochBlockInterval` is governance-stable across the short
        // migration/restart window, so reading at `cs_finalized_hash` matches
        // reading at the resumed height. The finalized hash is the one reth
        // pre-populates into `canonical_in_memory_state` during init (via
        // `with_latest`), so the read hits the ready in-memory state arm — a
        // by-NUMBER hash would go to the DB historical arm and can revert before
        // it materializes.
        let dpos_activation_block = reader.dpos_activation_block(cs_finalized_hash)?;
        let interval = reader.epoch_block_interval(cs_finalized_hash)?;
        let epoch_length_blocks =
            NonZeroU64::new(interval).ok_or_eyre("epoch_block_interval must be > 0")?;

        // Cold-start discriminator. The marshal's durable application-metadata is the
        // signal: an empty store (height <= activation) is a fresh sequencer→DPoS
        // migration — unless the EL overshot epoch 0, which anchors at reth's own
        // finalized tag instead (`ElFinalized`). A populated store is a restart of an
        // already-migrated node, which MUST resume at its real finalized height so the
        // scheme cascade starts at the correct epoch.
        let archive_finalized =
            read_consensus_archive_last_finalized(&ctx, MARSHAL_PARTITION_PREFIX).await?;
        let kind = resolve_cold_start_kind(
            archive_finalized,
            dpos_activation_block,
            interval,
            cs_finalized,
            upstream.is_some(),
        )?;
        let (latest_finalized, latest_finalized_hash) = match kind {
            ColdStartKind::FreshMigration => {
                // FRESH MIGRATION: anchor ≡ block@dposActivationBlock; wait for reth
                // to hold it, hash derived locally (canonical at a finalized height).
                let hash = wait_for_activation_block(
                    &ctx,
                    &provider,
                    dpos_activation_block,
                    &sync_metrics,
                )
                .await?;
                (dpos_activation_block, hash)
            }
            ColdStartKind::ElFinalized => {
                // EMPTY ARCHIVE, EL PAST EPOCH 0 (§5.2 "Правило единое", Д-2(а)).
                // Anchor at reth's OWN finalized tag — the pair
                // `derive_cold_start_heights` already read, which is the same datum the
                // follower path anchors on (`rf_hash`). It is not state of unknown
                // provenance: `canonical_in_memory_state`'s finalized slot has exactly
                // ONE writer, `update_finalized_block` on an FCU
                // (RETH `crates/engine/tree/src/tree/mod.rs:3109-3140`, reached only from
                // `ensure_consistent_forkchoice_state`, `:3181-3190`), which refuses a
                // hash that is not canonical here, and it survives a restart because the
                // same function stages it to disk and `BlockchainProvider::with_latest`
                // reloads it (`crates/storage/provider/src/providers/blockchain_provider.rs:87-116`).
                // The devp2p pipeline never touches it. So the tag was written by an FCU
                // THIS datadir accepted — this node's own `sync_to`/executor, or the
                // pre-DPoS sequencer — never by a peer's answer.
                //
                // NO jump here (pass Б2): the node boots on this anchor and climbs with
                // the ladder (§5.2) plus the steady-state tip-only jump, whose target is
                // a pair out of its own archive.
                info!(
                    anchor = cs_finalized,
                    anchor_hash = ?cs_finalized_hash,
                    activation = dpos_activation_block,
                    "empty consensus archive with the EL past epoch 0: anchoring at reth's own \
                     EL-finalized tag and catching up by verified steps. Those steps are served \
                     over the consensus plane, which can only ask THIS node's own \
                     C[T-1] u C[T] u C[T+1]; if every committee it remembers has rotated out, \
                     --dpos.follower-upstream (a WS peer dialled by URL) is the one entry that \
                     does not go through that set"
                );
                (cs_finalized, cs_finalized_hash)
            }
            ColdStartKind::Restart => {
                // RESTART (already migrated): resume at the consensus archive's
                // finalized height.
                match provider.block_hash(archive_finalized)? {
                    Some(hash) => (archive_finalized, hash),
                    None => {
                        // CRASH SURVIVOR: an ungraceful crash lost reth's
                        // unflushed finalized tail while the marshal persisted the
                        // finalization (the two stores flush independently). reth is
                        // behind the consensus archive. Recover the missing block(s)
                        // from the marshal's OWN finalized_blocks archive into reth —
                        // the cold-start analog of the executor gap-heal, and how tempo
                        // backfills marshal→reth. fluentbase needs this at cold-start
                        // (not just in the executor backfill) because the committee
                        // read at `latest_finalized_hash` and the genesis read both
                        // require reth to hold the resume block. The later executor
                        // backfill then becomes a no-op.
                        //
                        // The committee source authenticates a #8 below-floor re-fetch
                        // (`committee[E]` read at the already-recovered parent state).
                        let recover_committees = crate::cert_inlet::RethCommitteeSource::new(
                            RethStakingStateReader::new(
                                provider.clone(),
                                evm_config.clone(),
                                staking_config.clone(),
                            ),
                            chain_id,
                        );
                        match recover_finalized_tail_into_reth(
                            &ctx,
                            &beacon_engine_handle,
                            &provider,
                            &deriver,
                            archive_finalized,
                            upstream.as_ref(),
                            &recover_committees,
                            &sync_metrics,
                            randomness.as_ref(),
                            &OriginEpocher::new(dpos_activation_block, epoch_length_blocks),
                        )
                        .await?
                        {
                            RecoverOutcome::Recovered(hash) => (archive_finalized, hash),
                            RecoverOutcome::DeferToElSync { gap } => {
                                // #12 (reth deeply behind an INTACT archive): the pre-engine
                                // replay is capped, so anchor at reth's ACTUAL tip. WHAT
                                // CLOSES THE GAP after pass Б2 is the EXECUTOR'S STARTUP
                                // BACKFILL DRAIN, not a jump: `outer.rs` hands the executor
                                // `last_consensus_finalized_height` (the marshal's own acked
                                // cursor, which an INTACT archive leaves far above reth), and
                                // the executor drains
                                // `(last_execution_finalized_height .. that cursor]` block by
                                // block out of the marshal archive through the same
                                // derive+import path live dispatch uses
                                // (`executor.rs::finalized_heights_to_backfill`).
                                //
                                // The steady-state jump CANNOT serve here and never fires on
                                // this path: `last_tip_height` and `ordering_finalized` are
                                // both seeded from that same cursor, so the heartbeat re-poke
                                // sees a difference of 0, and `maybe_re_jump` refuses to spawn
                                // at all while the drain is non-empty. So
                                // `dpos_sync_degraded{reason=crash_recover}` stays raised until
                                // the drain's LAST height clears it (`executor.rs`, the
                                // `pending_backfill` arm) — there is no pre-engine step left
                                // that could clear it here. Cost of the change: this gap is now
                                // walked by derive+import instead of one devp2p fast-forward.
                                let best = provider.best_block_number()?;
                                info!(
                                    gap,
                                    reth_best = best,
                                    finalized_target = archive_finalized,
                                    "crash-survivor recovery deferred to the executor's startup \
                                     backfill drain; anchoring at reth's tip"
                                );
                                let best_hash = read_with_visibility_belt(
                                    &ctx,
                                    &sync_metrics,
                                    &format!("the crash-recover reth tip {best}"),
                                    || provider.block_hash(best).wrap_err("reading reth tip hash"),
                                )
                                .await?;
                                (best, best_hash)
                            }
                        }
                    }
                }
            }
        };

        // Read the EL head AFTER the crash-survivor recovery above: it imports the
        // missing reth tail, so a pre-recovery snapshot would be stale.
        let (_cs_fin, _cs_fin_hash, head_num, head_hash) =
            derive_cold_start_heights(&canonical_state, genesis_hash);

        // Read AFTER the crash-survivor recovery above: it imports the missing reth
        // tail, and a pre-recovery snapshot would make the executor backfill
        // re-derive exactly those blocks (idempotent but wasted V).
        let last_execution_finalized_height = provider
            .last_block_number()
            .wrap_err("provider failed to report chain head block number at startup")?;

        tracing::info!(
            last_execution_finalized_height,
            archive_finalized,
            ?kind,
            finalized = latest_finalized,
            finalized_hash = ?latest_finalized_hash,
            head_num,
            head_hash = ?head_hash,
            "DPoS init: cold-start discriminator resolved"
        );

        // Clean-halt migration invariant: the pre-DPoS sequencer is production-gated
        // at `dposActivationBlock` (bins/fluent launcher), so on a fresh migration
        // reth's canonical head MUST already equal the activation anchor — there is
        // no orphaned sequencer-era tail to reconcile. A mismatch means the gate did not run
        // (mis-set chain-config, an ungated node, or a hand-rolled migration): fail
        // loud at cold-start rather than wedge silently in the executor ancestor-FCU
        // guard.
        if kind == ColdStartKind::FreshMigration {
            ensure!(
                head_hash == latest_finalized_hash,
                "fresh migration but reth head {head_num} ({head_hash:?}) != activation \
                 anchor {latest_finalized} ({latest_finalized_hash:?}). Either the sequencer \
                 was not production-gated at dposActivationBlock, or this EL was filled by \
                 devp2p with no FCU ever issued: the staged pipeline never writes reth's \
                 `finalized` tag, so `get_finalized_num_hash()` is None, the cold-start \
                 discriminator reads the anchor as height 0 and routes an EL that is far past \
                 activation into this arm. Refusing to anchor DPoS on an orphaned tail"
            );
        }
        let (initial_head_num, initial_head_hash) = (head_num, head_hash);

        let initial_epoch_u64 = fluentbase_staking_reader::reader::epoch_at_block(
            latest_finalized,
            dpos_activation_block,
            interval,
        )
        .ok_or_else(|| eyre!("epochBlockInterval is zero"))?;

        // Enforce the node ↔ contract invariant
        //   `activeValidatorsLength <= fluentbase_p2p::MAX_COMMITTEE_SIZE`.
        let active_validators_length = reader
            .active_validators_length(latest_finalized_hash)
            .wrap_err("failed reading Staking.activeValidatorsLength")?;
        if active_validators_length > fluentbase_p2p::constants::MAX_COMMITTEE_SIZE {
            return Err(eyre!(
                "Staking.activeValidatorsLength ({}) exceeds \
                 fluentbase_p2p::constants::MAX_COMMITTEE_SIZE ({}). Node ↔ contract \
                 cap exceeded — the chain has a configured active-set larger \
                 than the node can encode. Raise the ONE declaration, \
                 fluentbase_types::staking_protocol::MAX_COMMITTEE_SIZE (both \
                 sides import it), then redeploy/upgrade. It must stay <= 255 \
                 while the production record carries a one-byte leader index.",
                active_validators_length,
                fluentbase_p2p::constants::MAX_COMMITTEE_SIZE,
            ));
        }

        info!(
            chain_id,
            interval,
            max_committee_size = fluentbase_p2p::constants::MAX_COMMITTEE_SIZE,
            active_validators_length,
            initial_epoch = initial_epoch_u64,
            latest_finalized,
            head_num,
            "DPoS startup config"
        );

        // The cold-start committee, read through the MODULE — which is also
        // what registers it: the record and this epoch's verify-only scheme land
        // in the same map slot, so the marshal can verify certificates of the
        // starting epoch before any boundary fires. This replaced a direct
        // `epoch_committee_snapshot` here plus an `OuterEngine::cold_start_register`
        // after `build`; between them they were a second producer of an epoch's
        // scheme, and the one that hardcoded `oracle: None`.
        //
        // A refusal is routed by the module's own verdict, and the two arms are
        // the two different facts the old direct read could not tell apart.
        //
        // PERMANENT (`is_transient() == false`: the contract answered something
        // no committed epoch can answer, or the epoch is below the read window)
        // is a statement about the CHAIN, and it stays the loud startup refusal
        // §5.1 prescribes — the operator message in full, and the process does
        // not come up pretending to follow a chain whose committee it cannot
        // read.
        //
        // TRANSIENT is a statement about THIS PROCESS, not the chain. The
        // module's anchor is this node's ordering-finalized cursor (floored by
        // reth's own finalized tag) and its geometry is frozen by the beacon
        // plane's `EpochTransition`; both are seeded by tasks that run
        // CONCURRENTLY with this launch, so a `NotReadable` at this instant says
        // "this process has not finished standing up". The retry is not a hope:
        // `epoch_transition::cold_start` below queues THIS epoch on the bridge
        // (`staking-reader/src/epoch_transition.rs` `track_and_trigger`, the
        // `boundary_tx.try_send`), the forwarder hands it to the manager's
        // `boundary_rx`, and a reconcile that finds the committee still
        // unreadable parks the epoch in `deferred_reconciles`, which the
        // module's own wake-up drains.
        match committee.committee(initial_epoch_u64) {
            Ok(record) => info!(
                epoch = initial_epoch_u64,
                members = record.members.len(),
                "cold-start committee read through the committee module"
            ),
            Err(e) if !e.is_transient() => {
                return Err(eyre!(
                    "committee[{initial_epoch_u64}] is REFUSED PERMANENTLY at this node's \
                     committee anchor (read at finalized block {latest_finalized}): {e}. \
                     `commitEpochCommittee` is a system call the block producer issues from \
                     pre-execution and is SYSTEM_CALLER-only, so there is no operator command \
                     that fixes this. What resolves it is on-chain state plus block \
                     production: `getDposActivationBlock()` must be scheduled, the registry \
                     must hold at least {} activated validators with consensus keys (below \
                     that the FIRST commit, at epoch 0, reverts ERR_COMMITTEE_TOO_SMALL — a \
                     later epoch carries the previous committee forward instead, but epoch 0 \
                     has none to carry), and the producer must then have advanced far enough \
                     for the commit to land and finalize. No retry can change this answer.",
                    fluentbase_staking_reader::reader::MIN_COMMITTEE_LENGTH,
                ));
            }
            Err(e) => warn!(
                epoch = initial_epoch_u64,
                error = %e,
                "no committee for the cold-start epoch at this node's anchor YET (read at \
                 finalized block {latest_finalized}) — a retryable miss, i.e. a statement \
                 about this process's startup order and not about the chain. The cold start \
                 below queues this epoch on the boundary bridge and the committee module's \
                 wake-up re-runs its reconcile; until one succeeds this node registers no \
                 epoch and enters none."
            ),
        }

        // The single `FluentP2P` is built ONCE per process by the node crate's
        // always-on beacon plane (and stays up across the follower↔signer switch);
        // this signer engine consumes a CLONE of that one network's `oracle` plus
        // CLONES of the 5 plane-owned non-beacon `MuxHandle`s. It never re-binds
        // `listen`, rebuilds a `Muxer`, or consumes a raw channel half — so a
        // demote→re-promote within one process re-clones cleanly (no network rebuild).

        // THE process's `EpochTransition`, built by the always-on plane with the
        // sending half of this bridge already wired in, and its receiving half. The
        // forwarder below (spawned after `build`, where `boundary_sender()` exists)
        // drains `bridge_rx` → `outer_boundary_tx`. There is no second instance and no
        // second `oracle.track` per epoch: the plane's poller no longer drives
        // boundaries at all, this layer's per-block delivery hook does.
        let PlaneEpochTransition {
            transition: et_arc,
            mut bridge_rx,
        } = epoch_transition;

        // `T` for the ladder step — see `DposLayerConfig::tracked_epoch`. This
        // layer is its ONE writer (the boundary forwarder below).
        const NO_TRACKED_EPOCH: u64 = u64::MAX;
        let tracked_epoch_cell = tracked_epoch;

        // Cold-start at THIS node's anchor — the one the cold-start discriminator
        // resolved, i.e. AFTER a jump landed, which is strictly at or above the
        // EL-finalized height the plane poller reads. This is the process's ONE
        // bootstrap, and it is here rather than in the poller because only here is the
        // anchor an ORDERING height on the far side of the jump: it picks the starting
        // epoch (the single value the epoch manager ever learns, over the bridge) and
        // sets the read floor to the landing (`cold_start` raises, never lowers — see
        // `raise_anchor_height`).
        //
        // The plane's poller has already done the other two things this call also
        // does: it froze the geometry (through the same path, so whichever ran first
        // the other is a no-op) and it registered a FIRST peer set
        // (`EpochTransition::track_peers`, `crates/node/src/dpos.rs`). That
        // registration is why the jump loop above could finish at all — the frontier
        // resolver dials only tracked peers — and it is NOT a bootstrap: it moves no
        // bootstrap state, so the branch this call takes is still the write-once one.
        // Re-registering its epoch index here is ignored by the Oracle; a higher one
        // is the ordinary advance.
        et_arc
            .lock()
            .await
            .cold_start(latest_finalized_hash, latest_finalized)
            .await
            .wrap_err("epoch_transition cold_start failed")?;
        info!(
            epoch = initial_epoch_u64,
            "DPoS cold_start complete; peer set tracked"
        );

        // Compute the consensus genesis Block.
        //
        // `latest_finalized` is 0 for pristine cold-start (no FCU yet — canonical
        // state empty → falls to `BlockNumHash::new(0, genesis_hash)` at dpos.rs:220)
        // and N for sequencer→DPoS migration (the sequencer's last finalised height read from
        // disk by reth's `BlockchainProvider::with_latest`).
        //
        // For migration, anchoring the consensus genesis at block N (rather than
        // the fluent chain genesis at height 0) makes Simplex voter cache
        // `set_genesis(hash_N)` so view 1's `context.parent = (View::zero(), hash_N)`
        // matches the proposer's block.parent = hash_N. `fetch_parent`'s identity
        // short-circuit then returns the synthetic genesis (= block N),
        // `validate_block` passes, and `application.verify(block_N+1)` proceeds
        // to reth `new_payload` against MDBX-loaded state(hash_N).
        let genesis_unsealed = provider
            .block_by_number(latest_finalized)
            .map_err(|e| {
                eyre!("consensus genesis block read at height {latest_finalized} failed: {e}")
            })?
            .ok_or_else(|| {
                eyre!(
                    "consensus genesis block missing from MDBX at height {latest_finalized} \
                 (canonical_state.finalized claimed it exists). Graceful shutdown must \
                 persist this block before DPoS restart."
                )
            })?;
        let genesis_sealed: SealedBlock<RethBlock> = SealedBlock::seal_slow(genesis_unsealed);
        // F-type: the ordering-chain genesis is the deterministic anchor
        // artifact (its `result` field binds the anchor's EVM hash, so the
        // weak-subjectivity binding of the old executed-block genesis is
        // preserved). Every node computes the identical artifact, so Simplex
        // `set_genesis(digest)` matches view 1's parent on all nodes.
        let genesis_block = anchor_order_block(&genesis_sealed)?;

        // Boundary hook: fires for every `Update::Block`. Spawns
        // fire-and-forget via `ctx.spawn` (NOT `tokio::spawn`, which would
        // depend on the implicit `tokio::Handle::current()` contract under
        // commonware-tokio). The live-DKG epoch clock no longer rides this
        // hook — the always-on plane (node crate) owns the `dkg_height` stream
        // off a persistent finalized-height source, so the `DkgActor` keeps
        // ticking across the follower phase where this signer hook does not run.
        let consecutive_errors = Arc::new(AtomicU32::new(0));
        let et_for_hook = et_arc.clone();
        let ctx_for_hook = ctx.with_label("boundary_hook");
        let errors_for_hook = consecutive_errors.clone();
        let sync_metrics_for_hook = sync_metrics.clone();
        // External wedge detection replacing the deleted give-up: the height of a
        // parked boundary (0 = none). The park has NO internal give-up — a
        // durably-frozen node is caught by a Prometheus `!= 0 for > Xm` alert +
        // the periodic warn! below + the harness recover-stall, never a local
        // counter/clock. Signer-plane twin of the executor cert-budget park's
        // `deferred_height`.
        let parked_boundary_height = Gauge::<i64>::default();
        ctx_for_hook.register(
            "parked_boundary_height",
            "Height of the epoch boundary PARKED awaiting EL state materialization (0 = none). \
             A sustained non-zero value flags a durably-wedged node.",
            parked_boundary_height.clone(),
        );
        // #16: consecutive on_finalized errors, exposed as a gauge instead of a
        // 3-error `shutdown.cancel()`. A boundary-read failure is typically
        // CORRELATED (bad staking state every validator reads) — a fatal here would
        // restart-storm the whole set at once (Decision A). Retry-forever-degraded;
        // a sustained non-zero value (paired with `dpos_sync_degraded{reason=
        // boundary_hook}`) is the external wedge signal.
        let on_finalized_consecutive_errors = Gauge::<i64>::default();
        ctx_for_hook.register(
            "on_finalized_consecutive_errors",
            "Consecutive epoch-boundary on_finalized errors (0 = healthy). Retried-degraded \
             forever (no shutdown); a sustained non-zero value flags a wedged boundary read.",
            on_finalized_consecutive_errors.clone(),
        );
        // The re-poke loop below is the ONE task on this path that genuinely
        // cannot hand a handle to the node supervisor: `enter_boundary` is an
        // `Arc<dyn Fn(u64)>` called from the delivery adapter and from the
        // executor's re-jump landing, spawning one loop PER boundary entry —
        // there is no return path and no fixed set of handles to register. So
        // its death is made LOUD instead: the panic is caught right at the loop
        // (`with_catch_panics(true)` otherwise reduces it to one log line, and a
        // dropped handle detaches the task) and ticked here. Any non-zero value
        // means an epoch boundary lost its re-poke driver — pair it with
        // `parked_boundary_height != 0`, which stays stuck at the height the
        // dead loop was driving.
        let boundary_repoke_panics: Counter = Counter::default();
        ctx_for_hook.register(
            "boundary_repoke_task_panics",
            "Epoch-boundary re-poke tasks killed by a panic (caught by the runtime's \
             catch_panics, so NOT fatal on its own). Non-zero ⇒ a parked boundary has no \
             driver left; expect parked_boundary_height to stay pinned.",
            boundary_repoke_panics.clone(),
        );
        let parked_gauge_for_hook = parked_boundary_height.clone();
        let errors_gauge_for_hook = on_finalized_consecutive_errors.clone();
        // The epoch-entry seam, keyed on a HEIGHT and not on a delivered block, so the
        // steady-state re-jump can drive the SAME entry the delivery path drives without
        // synthesising an `Update::Block`: that update carries an `Exact` ack the executor
        // must fire, and a dropped ack trips marshal's supervisor cascade. The hook only ever
        // read `block.height` anyway.
        let enter_boundary: Arc<dyn Fn(u64) + Send + Sync> = Arc::new(move |number: u64| {
            let et = et_for_hook.clone();
            let ctx_task = ctx_for_hook.clone();
            let errors = errors_for_hook.clone();
            let parked_gauge = parked_gauge_for_hook.clone();
            let errors_gauge = errors_gauge_for_hook.clone();
            let sync_metrics = sync_metrics_for_hook.clone();
            let repoke_panics = boundary_repoke_panics.clone();
            // The old BlockNotFound retry loop is gone: committee reads
            // now resolve at the result-final height (number − K) inside
            // EpochTransition; an unresolved read is Intra + a pending
            // boundary that replays on the next delivery — no race with
            // the executor's import remains.
            drop(ctx_task.spawn(move |ctx_inner| async move {
                // Re-poke loop: a parked boundary replays only on the next
                // on_finalized call, and during epoch catch-up the parked
                // boundary IS the last deliverable block — without the
                // retry no further delivery would ever trigger the replay
                // (catch-up deadlock).
                //
                // NO internal give-up (mirrors the cert-budget executor
                // park): re-poke at PENDING_RETRY_BACKOFF until the park
                // CLEARS (state materialized) or a real error surfaces. A
                // deep re-jump PIPELINE backfill runs up to
                // EL_SYNC_BACKSTOP_CEILING (6 h), which any fixed limit would
                // exhaust — abandoning the last deliverable boundary and
                // wedging epoch E+1 (a recover-stall by a second route). A
                // best_block_number()-progress gate is deliberately NOT used:
                // it is FROZEN for the whole backfill (the exact bug
                // scenario). Liveness against a genuinely-wedged node is
                // EXTERNAL — the parked_boundary_height gauge (+ periodic
                // warn) and the harness recover-stall deadline, never a local
                // counter/clock.
                // LOUD-EXIT rather than supervised (see the
                // `boundary_repoke_task_panics` counter above for why no handle
                // can reach the node supervisor from here): catch the unwind at
                // the loop so a panic in the only driver of THIS boundary becomes
                // a counter tick + an error!, instead of the single anonymous
                // "task panicked" line `with_catch_panics(true)` reduces it to.
                // Only a panic takes the arm below — a clean `break` returns
                // `Ok` and an abort never resolves at all.
                use futures::FutureExt as _;
                let repoke = std::panic::AssertUnwindSafe(async move {
                    let mut pokes = 0u64;
                    loop {
                        let outcome = {
                            let mut et_guard = et.lock().await;
                            et_guard.on_finalized(number).await
                        };
                        match outcome {
                            // Any Ok is a successful boundary read — clear the
                            // consecutive-error streak + the degraded gauge.
                            Ok(TransitionOutcome::EpochAdvanced(_) | TransitionOutcome::Intra) => {
                                if errors.swap(0, Ordering::Relaxed) != 0 {
                                    errors_gauge.set(0);
                                    sync_metrics.recover(SyncReason::BoundaryHook);
                                }
                            }
                            // #16 SELF-HEAL (retry-forever-degraded, Decision A): a
                            // boundary-read failure is typically correlated (bad
                            // staking state every validator reads), so a fatal here
                            // would crash all validators at once. Raise the gauges,
                            // back off, and re-attempt on_finalized — NEVER break/
                            // shutdown (a break would wedge the last-deliverable-
                            // boundary catch-up with no further delivery to re-fire).
                            Err(e) => {
                                let count = errors.fetch_add(1, Ordering::Relaxed) + 1;
                                errors_gauge.set(count as i64);
                                sync_metrics.degrade(SyncReason::BoundaryHook);
                                error!(
                                    block_number = number,
                                    consecutive_errors = count,
                                    error = ?e,
                                    "epoch_transition.on_finalized failed; retrying-degraded \
                                     (no shutdown — a correlated boundary-read failure must not \
                                     crash all validators)"
                                );
                                ctx_inner
                                    .sleep(
                                        fluentbase_staking_reader::epoch_transition::PENDING_RETRY_BACKOFF,
                                    )
                                    .await;
                                continue;
                            }
                        }
                        // BENIGN MISATTRIBUTION: `enter_boundary` spawns one of these
                        // loops per call and both the delivery adapter and the
                        // executor's re-jump landing can call it, so the value read
                        // here may have been parked by the OTHER task. The gauge and
                        // the every-N-pokes warn can therefore credit a park to the
                        // wrong spawn. Nothing is lost by it: the slot is global, both
                        // loops exit only on `None`, and whichever loop sees `None`
                        // clears the gauge — so no wakeup can be dropped and the gauge
                        // still reads "some boundary is parked", which is what the
                        // wedge alert asks.
                        let parked = et.lock().await.pending_boundary();
                        match parked {
                            None => {
                                parked_gauge.set(0);
                                break;
                            }
                            Some(parked_height) => {
                                parked_gauge.set(parked_height as i64);
                                pokes += 1;
                                if pokes.is_multiple_of(PARKED_BOUNDARY_WARN_EVERY) {
                                    warn!(
                                        boundary = parked_height,
                                        pokes,
                                        "epoch boundary still parked awaiting EL state \
                                         materialization; re-poking (no give-up)"
                                    );
                                }
                                ctx_inner
                                    .sleep(
                                        fluentbase_staking_reader::epoch_transition::PENDING_RETRY_BACKOFF,
                                    )
                                    .await;
                            }
                        }
                    }
                })
                .catch_unwind()
                .await;
                if repoke.is_err() {
                    repoke_panics.inc();
                    error!(
                        boundary = number,
                        "epoch-boundary re-poke task PANICKED: this boundary lost its only \
                         driver, so a parked boundary stays parked until some other entry \
                         re-fires it (committee rotation wedges meanwhile). See \
                         boundary_repoke_task_panics + parked_boundary_height."
                    );
                }
            }));
        });

        let boundary_hook: Arc<dyn Fn(crate::order_block::OrderBlock) + Send + Sync> = {
            let enter = enter_boundary.clone();
            Arc::new(move |block: crate::order_block::OrderBlock| enter(block.height))
        };

        // The read-floor seam — the height-floor twin of `enter_boundary`, on the SAME
        // `EpochTransition`. A re-jump landing publishes its result-final floor here and
        // the state machine clamps every later committee read to it, so the entry above
        // reads state this node still holds rather than state the jump left behind.
        // AWAITED by the executor rather than spawned like the entry: the floor has to be
        // in place before the entry's first read, and a spawn would race it.
        let et_for_read_floor = et_arc.clone();
        let read_floor_boundary: crate::executor::BoundaryReadFloorFn =
            Arc::new(move |height: u64| {
                let et = et_for_read_floor.clone();
                Box::pin(async move {
                    et.lock().await.raise_anchor_height(height);
                })
            });

        let me = peer_keypair.public_key();
        info!(peer_pubkey = %me, "DPoS peer identity");

        // The live-DKG `ceremony_store` (written by the always-on `DkgActor`) and the
        // `committee_for` resolver arrive SHARED from the persistent beacon plane
        // (node crate) via [`SharedBeaconPlane`]; this signer engine is a read-only
        // consumer (the `PK_epoch` resolver behind `BeaconVerify`, and the per-epoch
        // signing material via `beacon_resolver`). The disk reload of
        // `<datadir>/beacon/` happened ONCE at the plane's startup — not here.
        //

        // Slasher wiring: the committee module resolves every evidence epoch —
        // no dedicated reader and no finalized-hash closure of its own. The
        // TxPool transport sink arrives pre-built via `cfg.slasher_sink`
        // (host-side construction).

        // Per-epoch threshold beacon resolver for the combined consensus scheme —
        // carry-forward under the frozen on-chain `dkgQual`-bit arbitration, which
        // after П-3 lives in `beacon::artifact::MintIndex` (the bit walk, memoised on
        // disk) over `beacon::artifact::ArtifactStore` (the key); refusal ⇒ the epoch_manager
        // share-gate demotes to verify-only, the recompute-heal re-promotes.

        // Steady-state self-healing re-jump (finding #6): the executor's reaction
        // to its own `Update::Tip` event, and since pass Б2 the ONLY jump in the
        // system — the pre-engine one is gone, and a node with an empty archive
        // anchors at its own EL-finalized tag and climbs from there. Forward-only,
        // over a target out of this node's own marshal archive, re-runnable while
        // the executor runs. Enabled wherever an upstream is configured, which since the
        // plane-native default is EVERY launched node — `node/src/dpos.rs:1683` wraps
        // both the `Plane` and the `Ws` branch in `Some(`, so a plain validator has
        // one too and re-jumps plane-natively. The executor runs it synchronously in
        // its `select!` arm, so its `sync_to` FCU is serialized with every other
        // reth write the executor makes — the executor stays the sole reth writer.
        //
        // Rule Y: the validator-with-upstream re-jump is SYMMETRIC with the
        // follower — same epoch-relative threshold, same `rotate` escape. Since
        // §5.2 its TARGET is not asked for either: the executor reads the
        // `(finalization, block)` pair out of its own marshal archive at the tip it
        // triggered on and hands it in, so this closure has no `upstream` in it at
        // all — only the committee source, the EL seam and the activation height.
        let re_jump_threshold = crate::cold_start_jump::JUMP_THRESHOLD.min(interval);
        let re_jump: Option<crate::executor::ReJump> = upstream.as_ref().map(|up| {
            let up = up.clone();
            // The inlet's SAME upstream-rotation escape (Rule L/Y).
            let rotate = up.rotate_callback();
            // The executor's frozen-tip frontier probe — ONE constructor, shared
            // with the follower (`frontier_probe`, review B1-01).
            let frontier_probe = frontier_probe(up.clone(), committee.clone());
            let provider = provider.clone();
            let beacon_engine_handle = beacon_engine_handle.clone();
            let ctx = ctx.clone();
            let peer_count = peer_count.clone();
            let cb: crate::executor::ReJumpFn = Arc::new(
                move |from: u64, target: crate::cert_follow::UpstreamFinalized| {
                    let provider = provider.clone();
                    let beacon_engine_handle = beacon_engine_handle.clone();
                    let peer_count = peer_count.clone();
                    let jump_ctx = ctx.clone();
                    Box::pin(async move {
                        let el = crate::cold_start_jump::RethElSync::new(
                            jump_ctx,
                            provider.clone(),
                            beacon_engine_handle,
                            dpos_activation_block,
                            peer_count,
                        );
                        // Return the typed terminal `JumpOutcome` verbatim — the
                        // executor's completion arm classifies it (Landed re-seeds;
                        // Stalled is NON-fatal + retried on the next Tip). §9.6.
                        // No committee source and no verify RNG any more: the target
                        // is a pair out of this node's OWN marshal archive, already
                        // 2f+1 under a committee this node read (pass Б2).
                        crate::cold_start_jump::jump_to_target(
                            from,
                            target,
                            &el,
                            // No L1 checkpoint on the validator path.
                            None,
                            dpos_activation_block,
                            re_jump_threshold,
                        )
                        .await
                    }) as futures::future::BoxFuture<'static, _>
                },
            );
            crate::executor::ReJump {
                call: cb,
                // Epoch-relative gate, mirroring the follower (real-prod epochs ≫ 1024
                // keep the serving-window size; a compressed test epoch heals within
                // an epoch).
                threshold: re_jump_threshold,
                // Rule L/Y: the same upstream-rotation escape the follower wires (T2).
                // After pass Б2 the only arm that fires it is a `Stalled` streak —
                // the insta-rotating `BadTarget`/`AuthFailed` arms are gone.
                rotate: Some(rotate),
                // Frozen-tip frontier probe — the live-follow driver for the
                // PLANE-NATIVE validator (no consensus participation while rotated
                // out): the executor puts the ladder step on the marshal and hints
                // it toward any `Latest` above its tip when that tip freezes. Also
                // a harmless backstop on the WS path (the inlet keeps the tip
                // advancing → the probe stays silent).
                probe: Some(frontier_probe),
                // `T` for the ladder step, mirrored off the boundary bridge (see
                // `tracked_epoch_cell`). `None` while nothing is tracked yet.
                tracked_epoch: Some({
                    let cell = tracked_epoch_cell.clone();
                    std::sync::Arc::new(move || {
                        match cell.load(std::sync::atomic::Ordering::Relaxed) {
                            NO_TRACKED_EPOCH => None,
                            epoch => Some(epoch),
                        }
                    })
                }),
            }
        });

        // Boundary seeding seam — the same upstream and the same POST-sync committee
        // read as the re-jump above, but fetching ONE already-finalized block by
        // height rather than a landing. After any jump the epoch-terminal height that
        // `Inline::genesis` (and so the engine-spawn gate) needs sits BELOW the new
        // marshal floor, where no repair path ever fetches it — so a member that did
        // not already hold it parks verify-only for the whole landing epoch, neither
        // proposing nor voting.
        let boundary_fetch: Option<crate::cert_follow::BoundaryFetchFn> =
            upstream.as_ref().map(|up| {
                let up = up.clone();
                let provider = provider.clone();
                let evm_config = evm_config.clone();
                let staking_config = staking_config.clone();
                let ctx = ctx.clone();
                let sync_metrics = sync_metrics.clone();
                Arc::new(move |height: u64, at_hash: B256| {
                    let up = up.clone();
                    let provider = provider.clone();
                    let evm_config = evm_config.clone();
                    let staking_config = staking_config.clone();
                    let sync_metrics = sync_metrics.clone();
                    // `verify_jump_authenticated` needs a `&mut (Clock + CryptoRngCore)`;
                    // a fresh clone per call so the closure stays re-usable.
                    let mut fetch_ctx = ctx.clone();
                    Box::pin(async move {
                        let committees = crate::cert_inlet::RethCommitteeSource::new(
                            RethStakingStateReader::new(
                                provider.clone(),
                                evm_config,
                                staking_config,
                            ),
                            chain_id,
                        );
                        crate::cert_follow::fetch_verified_boundary(
                            &up,
                            &committees,
                            &mut fetch_ctx,
                            &sync_metrics,
                            at_hash,
                            height,
                        )
                        .await
                    }) as futures::future::BoxFuture<'static, _>
                }) as crate::cert_follow::BoundaryFetchFn
            });

        // Core- and executor-owned families, registered against the launch context
        // — the SAME context `BeaconMetrics` is registered against inside
        // `beacon::build`, because commonware prefixes each family with the
        // context's label path and a labelled child would silently rename them.
        // Registered on this path AND on the follower's, since both node classes
        // reach the epoch manager and the executor.
        let epoch_metrics = crate::epoch_manager::EpochEngineMetrics::default();
        epoch_metrics.register(&ctx);
        let executor_metrics = crate::executor::ExecutorMetrics::default();
        executor_metrics.register(&ctx);

        let outer = OuterBuilder {
            me: me.clone(),
            // The beacon plane's epoch-key agreement instances, adopted by the
            // epoch manager so they prune on the engine cutoff.
            agreement_intake,
            // Bug A: no-op the consensus vote/cert-plane blocker (parity with the
            // beacon-resolver NoopBlocker, review [1013]). The simplex batcher's
            // evidence-free `block!(self.blocker, signer, ..)` on a transient
            // batch-verify-failed vote would otherwise 4h-partition an honest
            // committee peer on the shared global transport. Slashing is preserved:
            // equivocation evidence rides `Activity::Conflicting*` independently of
            // `block!`. `provider:` keeps the real oracle for peer-set tracking.
            blocker: NoopBlocker,
            provider: oracle.clone(),
            chain_id,
            epoch_length_blocks,
            dpos_activation_block,
            signer_keypair: Some(bls_keypair),
            randomness: randomness.clone(),
            spawn_unblocked,
            re_jump,
            epoch_metrics: epoch_metrics.clone(),
            executor_metrics: executor_metrics.clone(),
            sync_metrics: sync_metrics.clone(),
            safety_halt: safety_halt.clone(),
            tombstones,
            plane_clock,
            dkg_height_tx: Some(dkg_height_tx),
            timeouts: ConsensusTimeouts::fluent_1s(),
            mailbox_size: 256,
            // BROADCAST body cache: at most 4 order-block bodies retained per
            // PRIMARY sender (`CW:broadcast/src/buffered/engine.rs:319-322`,
            // `:353-359`). 64 was 64 × `MAX_ORDER_BLOCK_SIZE` = 256 MiB per peer,
            // and the primary set used to be the whole registry (R-013, E4-14);
            // 4.3 makes primary the three committee records, and 4 covers the
            // deepest legitimate pipeline (the proposal in flight plus a re-proposal
            // after nullify) with a spare. There is NO byte cap to pair it with:
            // `buffered::Config` carries `deque_size` and nothing else
            // (`CW:broadcast/src/buffered/config.rs:5-22`), so the per-peer memory
            // bound is `deque_size × MAX_ORDER_BLOCK_SIZE` — a library boundary,
            // not a choice made here.
            deque_size: 4,
            partition_prefix: MARSHAL_PARTITION_PREFIX.into(),
            engine_partition_prefix: String::new(),
            resolver_initial: Duration::from_secs(1),
            resolver_timeout: Duration::from_secs(2),
            resolver_fetch_retry: Duration::from_millis(100),

            // FluentApp constructor args.
            genesis: genesis_block,
            beacon_engine: beacon_engine_handle,
            deriver,
            executed,
            assembler,
            target_gas_limit,
            boundary_hook,

            // Executor cold-start state.
            last_execution_finalized_height,
            initial_finalized: (Height::new(latest_finalized), latest_finalized_hash),
            initial_head: (Height::new(initial_head_num), initial_head_hash),
            // DPoS-era floor: the marshal never dispatches pre-anchor history.
            // Fresh migration: anchor = activation. Restart: a raises-only no-op
            // (the archive's floor is already at/above its own finalized).
            // `ElFinalized`: reth's own finalized tag. A later steady-state jump
            // raises the floor to `landing − K` through `set_floor`, not here.
            marshal_floor: Some(Height::new(latest_finalized)),
            boundary_fetch,
            boundary_enter: enter_boundary,
            boundary_read_floor: read_floor_boundary,
            fcu_heartbeat_interval: Duration::from_secs(8),
            fcu_pace: Duration::from_millis(20),
            canonical_state: canonical_state.clone(),

            slasher_staking_address: staking_address,
            committee: committee.clone(),
            slasher_sink,
            slasher_wal_partition: "slasher-wal".into(),
            slasher_evidence: Some(evidence),

            feed,

            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine,
        }
        // `seed_journal_writer` is cloned off `ctx`, NOT off the `outer_engine`
        // context: it must be a sibling of the engine task so `engine.abort()`
        // does not cascade into the writer before it has drained (see
        // `OuterBuilder::build`).
        .build(ctx.with_label("outer_engine"))
        .await?;

        // Bridge forwarder: drains the `(u64, snapshot)` the transition queues and
        // hands the OuterEngine's boundary receiver the EPOCH alone. The snapshot
        // is dropped HERE rather than never produced, because the transition's own
        // trigger type belongs to `staking-reader`; what matters is that no
        // consumer downstream of this line sees a committee that did not come from
        // the committee module.
        let outer_boundary_tx = outer.boundary_sender();
        let shutdown_for_forwarder = shutdown.clone();
        // SUPERVISED, not detached: the `shutdown.cancel()` below is the ERROR
        // path only — a panic inside this loop unwinds straight past it, and
        // under `with_catch_panics(true)` that panic is one log line and nothing
        // else. Dropping the handle would detach the task (no `Drop` on
        // commonware handles), so committee rotation would stop while the node
        // stayed "healthy". Handed to the host supervisor instead, which treats
        // ANY resolution (panic → `Err(Error::Exited)`, or the clean-exit warn)
        // as fatal — so the deliberate fail-fast holds for both paths.
        let tracked_epoch_writer = tracked_epoch_cell.clone();
        let epoch_bridge_handle = ctx.with_label("epoch_bridge").spawn(move |_| async move {
            while let Some((u64_ep, _snap)) = bridge_rx.recv().await {
                // The transition only queues an epoch it has just tracked, and it
                // never goes backwards, so a plain store is the mirror.
                tracked_epoch_writer.store(u64_ep, std::sync::atomic::Ordering::Relaxed);
                if let Err(e) = outer_boundary_tx.send(Epoch::new(u64_ep)).await {
                    error!(
                        epoch = u64_ep,
                        error = %e,
                        "OuterEngine.boundary_sender receiver dropped — initiating shutdown"
                    );
                    shutdown_for_forwarder.cancel();
                    break;
                }
            }
        });

        // Grab the marshal mailbox for the node-side cert feed/RPC BEFORE
        // `start` consumes the engine.
        let cert_mailbox = outer.marshal_mailbox();

        // Start OuterEngine — ctx + the 5 plane-owned non-beacon `MuxHandle`s
        // (vote/cert/resolver per-epoch register, broadcast/marshal subchannel 0) +
        // this promotion's fresh vote-backup receiver. The OuterEngine registers its
        // sub-channels in `run`; on demote it drops them (auto-deregister) — the
        // plane's broker tasks stay live for the next promotion.
        //
        // `upstream` (the `--dpos.follower-upstream` handle, also used by the
        // cold-start jump above by reference) is threaded into the marshal's
        // by-height backfill resolver: when `Some`, the marshal pulls the cold-start
        // `[floor+1 .. first_live]` gap from the UPSTREAM (not consensus-plane peers),
        // so an OUT-OF-COMMITTEE validator (a not-yet-committee external joiner with
        // zero consensus-plane connectivity) backfills exactly like a follower
        // instead of wedging. The resolver keeps its own `upstream` clone alive
        // (which keeps the WS actor alive). A no-upstream validator passes `None` and
        // keeps the consensus-plane p2p resolver (the treadmill, unchanged). The
        // marshal still BLS-verifies every delivered cert — trustless, single-writer
        // intact.
        let consensus_handle = outer.start(
            ctx.with_label("marshal_resolver"),
            vote_mux,
            cert_mux,
            resolver_mux,
            broadcast_mux,
            marshal_mux,
            upstream,
        );

        Ok(DposLayerHandle {
            consensus_handle,
            cert_mailbox,
            supervised: vec![("epoch_bridge", epoch_bridge_handle)],
            // Both journal writers moved into `beacon::build` with the stores
            // they back, so this layer has no drain of its own left. The node
            // collects them off the `Beacon` instead — see its `shutdown`.
            drain_on_shutdown: vec![],
            // The validator serves from its beacon plane's own
            // `Beacon::artifact_bytes`, which the node holds directly.
            artifact_bytes: None,
        })
    }
}

/// Reth handles a near-planeless FOLLOWER needs (Phase 3). Distinct from
/// [`RethHandle`] only in that a follower carries NO slasher/pool transport (it
/// never signs) — the slasher actor is built (cheap) but never started.
pub struct FollowerRethHandle<Provider, EvmConfig, BeaconEngine> {
    pub provider: Provider,
    pub evm_config: EvmConfig,
    pub beacon_engine_handle: BeaconEngine,
    pub chain_id: u64,
    pub canonical_state: reth_chain_state::CanonicalInMemoryState<EthPrimitives>,
    pub genesis_hash: B256,
    /// Read-only probe of reth's connected devp2p peer count — see
    /// [`RethHandle::peer_count`]. Drives the follower EL-sync no-peers net.
    pub peer_count: Arc<dyn Fn() -> usize + Send + Sync>,
}

/// Operator-supplied config for the near-planeless follower (Phase 3). The node
/// owns the WS upstream + the ONE broadcast `Muxer`; this config carries the
/// reth-evm collaborators + the cert-inlet feed channels.
pub struct FollowerLayerConfig<D, XC, A, U> {
    /// This node's ed25519 peer identity (the FluentP2P crypto's public key),
    /// threaded from the node. A standalone follower never gossips, but
    /// `buffered::Engine` + the marshal resolver are keyed on it.
    pub me: commonware_cryptography::ed25519::PublicKey,
    pub staking_config: StakingReaderConfig,
    /// Datadir path of the fork-safety halt marker
    /// ([`crate::sync_metrics::SafetyHalt::restoring`]). A marker left by a
    /// previous run brings this node up permanently verify-only — the latch has
    /// no in-process `disengage`, so without it a restart silently cleared a
    /// halt and the node signed again on the same disk. `None` only in tests.
    pub halt_marker: Option<std::path::PathBuf>,
    /// L1 Rollup-checkpoint hash (B2) — ALSO the operator checkpoint a fresh
    /// datadir syncs to (§5.2 "Правило единое"): `Some` ⇒ the fresh-datadir entry
    /// FCUs to this hash and then fail-closed asserts it
    /// (`cert-follow: L1 Rollup checkpoint …`). `None` on a fresh datadir is a
    /// startup refusal on a deployed network and a `warn!`-ed trust-on-first-use
    /// on a local one — see [`Self::deployed_network`].
    pub l1_checkpoint_hash: Option<B256>,
    /// Whether this chain_id is one of the DEPLOYED networks (devnet / testnet /
    /// mainnet). Evaluated by the node — the chain_id constants live in its
    /// `chainspec`, and this crate must not carry a second copy of that list — and
    /// used here for exactly one decision: a fresh datadir with NO operator
    /// checkpoint refuses to start on a deployed network (E4-05) and falls back to
    /// the upstream's `Latest` only off one.
    pub deployed_network: bool,
    /// OrderBlock → derived-EVM-block execution (node-built over reth-evm).
    pub deriver: D,
    /// Local derived-chain view (node-built over the reth provider).
    pub executed: XC,
    /// The SAME ordering-finalized cursor [`Self::executed`] was built over —
    /// see `crate::ordering::ProviderExecutedChain::with_cursor` in the node
    /// crate. The committee module built below anchors its reads on it, so a
    /// follower reads every committee at the height its own executor has
    /// finalized and at no other.
    pub finalized_cursor: crate::FinalizedCursor,
    /// Pool-backed ordering assembly (node-built — pool trait bounds live there).
    /// A follower never proposes, so this is never exercised; the OuterBuilder
    /// requires it at the type level.
    pub assembler: Arc<A>,
    pub target_gas_limit: u64,
    /// Cert-feed sink (the marshal's 2nd application-`Reporter`) for this node's
    /// `consensus` RPC latest-tier. `None` for nodes that don't serve the feed.
    pub feed: Option<crate::feed_sink::FeedSink>,
    pub fcu_heartbeat_interval: Duration,
    /// Cert upstream. After pass Б2 the follower has no pre-engine jump: what rides
    /// this handle is the marshal's by-height backfill resolver, the frozen-tip
    /// ladder probe, the steady-state re-jump's EL work, and — on a FRESH datadir off
    /// a deployed network only — the one surviving `get_latest ⇒ sync_to`
    /// ([`FreshFollowerEntry::UpstreamLatest`]). A follower ALWAYS has an upstream
    /// (the WS the inlet uses); `None` only in tests.
    pub upstream: Option<U>,
    /// The live finalized-cert stream the node's WS actor pushes — the inlet's
    /// SOLE producer (a follower forms no certs locally).
    pub finalized_rx: mpsc::Receiver<crate::cert_follow::UpstreamFinalized>,
    /// Connection-generation token the node's WS actor bumps on each (re)connect
    /// (#7). Wired into the inlet via `with_connection_token` so the data-fault
    /// streak is scoped to the LIVE connection — a connection-level auto-rotation
    /// (which the inlet cannot otherwise observe) resets the streak, so one
    /// upstream's faults never bleed into the next URL's rotation budget. `None`
    /// in tests.
    pub conn_gen: Option<Arc<std::sync::atomic::AtomicU64>>,
    /// B3 — the serving-window sink: each VERIFIED inlet pair is forwarded here
    /// so a tier-2 follower aligns via THIS node's `consensus` WS window. `None`
    /// = no serving (tests).
    pub verified_tx:
        Option<tokio::sync::mpsc::UnboundedSender<crate::cert_follow::UpstreamFinalized>>,
}

/// A no-op [`SlasherTxSink`] for a follower: the slasher actor is built (cheap)
/// but never started, so `submit` is unreachable — a non-signer follower can
/// never detect-and-submit equivocation. Avoids dragging the node's
/// signer/pool `PoolTxSink` onto a keyless follower.
struct NoopSlasherSink;

impl SlasherTxSink for NoopSlasherSink {
    fn submit<'a>(
        &'a self,
        _target: Address,
        _calldata: alloy_primitives::Bytes,
    ) -> std::pin::Pin<
        Box<dyn core::future::Future<Output = crate::slasher::actor::SubmitOutcome> + Send + 'a>,
    > {
        Box::pin(async {
            crate::slasher::actor::SubmitOutcome::Failed(
                "follower NoopSlasherSink: a non-signer never submits slashing".into(),
            )
        })
    }
}

impl DposLayer {
    /// Launch the near-planeless FOLLOWER: an OuterEngine with
    /// `signer_keypair: None` driven by the cert-inlet (the only producer for a
    /// non-validator) instead of a local BFT engine. The executor is the SOLE reth
    /// writer (plus the single-shot pre-engine `cold_start_jump`).
    ///
    /// Steps:
    ///   1. resolve geometry (RESTART vs FRESH datadir via `read_geometry`) +
    ///      EL-sync onto the upstream's attested tip,
    ///   2. B2 — assert the L1 Rollup checkpoint (verbatim strings) post-EL-sync,
    ///   3. `cold_start_jump` for a deep residual gap (forward-only, BLS-verified),
    ///   4. build the follower OuterEngine (`signer_keypair: None`, no beacon
    ///      plane/signer/slasher-start, `Option<Muxes>::None`),
    ///   5. `start_follower` over the ONE broadcast `Muxer`, with an
    ///      UPSTREAM-backed marshal resolver (`UpstreamResolver`) that backfills
    ///      the by-height floor→frontier gap the live stream never carries,
    ///   6. spawn the cert-inlet over `finalized_rx` → `marshal_mailbox()` (+ the
    ///      B3 serving window).
    #[allow(clippy::too_many_arguments)]
    pub async fn launch_follower<Provider, EvmConfig, BeaconEngine, D, XC, A, U>(
        ctx: Context,
        reth: FollowerRethHandle<Provider, EvmConfig, BeaconEngine>,
        cfg: FollowerLayerConfig<D, XC, A, U>,
        oracle: fluentbase_p2p::OracleHandle,
        broadcast_mux: PlaneMux,
        shutdown: CancellationToken,
    ) -> eyre::Result<DposLayerHandle>
    where
        Provider: BlockReader<Block = RethBlock>
            + BlockHashReader
            + BlockNumReader
            + BlockIdReader
            + StateProviderFactory
            + HeaderProvider<Header = Header>
            + Clone
            + Send
            + Sync
            + 'static,
        EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
        BeaconEngine: BeaconEngineLike<ExecutionData = D::Derived> + Clone + Send + Sync + 'static,
        D: DerivedBlockBuilder,
        XC: ExecutedChain,
        A: OrderingAssembler,
        U: crate::cert_follow::CertUpstream,
    {
        let FollowerRethHandle {
            provider,
            evm_config,
            beacon_engine_handle,
            chain_id,
            canonical_state,
            genesis_hash,
            peer_count,
        } = reth;
        let FollowerLayerConfig {
            me,
            staking_config,
            halt_marker,
            l1_checkpoint_hash,
            deployed_network,
            deriver,
            executed,
            finalized_cursor,
            assembler,
            target_gas_limit,
            feed,
            fcu_heartbeat_interval,
            upstream,
            finalized_rx,
            conn_gen,
            verified_tx,
        } = cfg;

        // Self-heal stuck-detector: registered ONCE for the follower launch (the
        // follower has no beacon plane, so it owns its own — mirrors the fresh
        // `BeaconMetrics` below). Threaded into the cold-start jump / activation-wait
        // / landing-belt self-heal loops.
        let sync_metrics = SyncMetrics::default();
        sync_metrics.register(&ctx);
        // Fork-safety latch (Phase 3): a follower's executor derives+imports off the
        // inlet and can hit result divergence / EL Invalid, so it too must be able to
        // halt-and-stay-up rather than crash.
        let safety_halt = match halt_marker {
            Some(path) => crate::sync_metrics::SafetyHalt::restoring(sync_metrics.clone(), path),
            None => crate::sync_metrics::SafetyHalt::new(sync_metrics.clone()),
        };

        let reader = RethStakingStateReader::new(
            provider.clone(),
            evm_config.clone(),
            staking_config.clone(),
        );

        // Epoch geometry + the cold-start anchor (§5.2 "Правило единое": `sync_to`
        // is only ever called with an input this node can check).
        //
        // RESTART datadir: `ChainConfig` is readable from local state at reth's own
        // finalized hash, so the geometry AND the anchor are local — no `sync_to` at
        // all. FRESH datadir (runtime-deployed cluster): nothing is readable locally,
        // which is the one place in the system with nothing to check a peer against,
        // so the entry is an explicit operator checkpoint or a refusal.
        // THE GEOMETRY READ, and only it, is pinned to a single hash: the arm below
        // re-reads reth's finalized tag on every turn of its own loop (see there),
        // so the anchor and the committee reads are NOT this binding.
        let (_, geometry_at_hash, _h0_num, _h0_hash) =
            derive_cold_start_heights(&canonical_state, genesis_hash);
        let mk_el_sync = |activation: u64| {
            crate::cold_start_jump::RethElSync::new(
                ctx.clone(),
                provider.clone(),
                beacon_engine_handle.clone(),
                activation,
                peer_count.clone(),
            )
        };

        let (activation, interval, anchor_height, anchor_hash) =
            match read_geometry(&reader, geometry_at_hash)? {
                Some((activation, interval)) => {
                    // THE ANCHOR IS `rf_hash` — reth's own EL-finalized tag, the same
                    // datum the validator path anchors on. The `get_latest ⇒ sync_to`
                    // that used to stand here drove the EL onto a height a peer named,
                    // with nothing checking it (§5.2 lists it as one of the three
                    // unauthenticated entries); it is gone. A follower that is behind
                    // climbs from this anchor exactly like a validator: the ladder
                    // probe + the marshal's by-height pulls + the steady-state jump
                    // onto a pair out of its own archive.
                    //
                    // BELOW the activation block the anchor is not local yet, and
                    // `wait_for_activation_block` ALONE (4.2 Б2 .. 4.4) parked such a
                    // node forever whenever the missing blocks were pre-DPoS sequencer
                    // blocks that no ordering plane carries (R-131): this arm had no EL
                    // drive at all, `mk_el_sync` being reachable only from the `None`
                    // arm (`mk_el_sync` was reachable only from the `None` arm; this arm
                    // now calls it too). The march is now the five-way `follower_entry` — local,
                    // operator checkpoint, wait-local, chain-not-there-yet, certificate
                    // — and only the last two cost a peer round trip. §5.2 holds for the
                    // new entry the same way it holds for the jump: the height is
                    // pinned, the payload is bound to the block digest, and the
                    // finalization is BLS-verified under `committee[E]` READ AT
                    // `rf_hash` before reth is driven anywhere
                    // ([`fetch_verified_entry`]).
                    //
                    // The cadence is the shared `ACTIVATION_POLL`: this park and
                    // `wait_for_activation_block`'s are the same wait on the same kind
                    // of external input (R-131 review, D-10).
                    // The operator checkpoint is consumed AT MOST ONCE: a checkpoint on
                    // a pre-DPoS batch lands below the activation block, and re-driving
                    // it would spin on `sync_to_checkpoint`'s "already canonical"
                    // short-circuit instead of falling through to the certificate entry.
                    let mut checkpoint_pending = l1_checkpoint_hash;
                    let mut warned_wait = false;
                    // The LAST refusal reason printed for the certificate entry, so the
                    // park re-prints on a CHANGE of reason and not on every 2-second turn
                    // (R-131 review, D-04).
                    let mut unserved_reason: Option<String> = None;
                    loop {
                        // THE ANCHOR IS RE-READ EVERY TURN, and that is what makes the
                        // poll below honest rather than half-honest. `rf_hash` is
                        // reth's own EL-finalized tag — the same datum the validator
                        // path anchors on — and the pre-DPoS sequencer keeps MOVING it
                        // while this node waits. Pinning it before the loop pinned the
                        // STATE the committee window is read at, so the branch "no
                        // epoch committee is readable yet" (`setDposActivationBlock`
                        // has run, `commitEpochCommittee(0)` has not) could never
                        // change its answer no matter how long the poll ran, while its
                        // own `warn!` promised "Polling (no give-up)". A late commit
                        // appears on a LATER block, so the only way to see it is to
                        // re-read the tag (R-131 review, D-02). Both halves of the
                        // pair come from one `get_finalized_num_hash()`, so the number
                        // and the hash are always the same block.
                        let (rf_num, rf_hash, _, _) =
                            derive_cold_start_heights(&canonical_state, genesis_hash);
                        // LOCAL probe first, every turn: it is the only step with no
                        // network cost, and after a checkpoint landing or a sequencer
                        // block it is the step that ends the loop.
                        let activation_hash = provider
                            .block_hash(activation)
                            .wrap_err("probing whether reth holds the DPoS activation block")?;
                        let peer_free = follower_entry(
                            activation_hash.is_some(),
                            upstream.is_some(),
                            checkpoint_pending.is_some(),
                            PeerProbes::default(),
                            activation,
                            interval,
                            rf_num,
                        );
                        // Only the retry frontier of the peer-free prefix is worth a
                        // round trip; re-deciding through the SAME function is what
                        // keeps this from being a second copy of the predicate.
                        let entry = match (peer_free, upstream.as_ref()) {
                            (FollowerEntry::ChainBelowActivation, Some(up)) => {
                                // `get_latest` is used for ROUTING and as a height
                                // ceiling only; its hash is never read here. A liar can
                                // only pull the target DOWN (never below `activation + K`),
                                // which costs a lower landing and a ladder climb — it
                                // cannot raise it past the readable window, and the
                                // landing hash comes from the attested `result`.
                                let latest_height =
                                    up.get_latest().await.map(|latest| latest.block.height);
                                let committees = crate::cert_inlet::RethCommitteeSource::new(
                                    RethStakingStateReader::new(
                                        provider.clone(),
                                        evm_config.clone(),
                                        staking_config.clone(),
                                    ),
                                    chain_id,
                                );
                                // The readable window at `rf_hash`: devnet genesis has
                                // `committee[0]` only, a pre-activation prod block has up
                                // to `committee[MAX_COMMITTEE_LOOKAHEAD_EPOCHS]`.
                                let e_max = (0..=fluentbase_types::staking_protocol::
                                    MAX_COMMITTEE_LOOKAHEAD_EPOCHS)
                                    .rev()
                                    .find(|e| {
                                        crate::cert_inlet::CommitteeSource::scheme_at(
                                            &committees,
                                            *e,
                                            rf_hash,
                                            None,
                                        )
                                        .is_ok()
                                    });
                                follower_entry(
                                    activation_hash.is_some(),
                                    true,
                                    checkpoint_pending.is_some(),
                                    PeerProbes {
                                        latest_height,
                                        e_max,
                                    },
                                    activation,
                                    interval,
                                    rf_num,
                                )
                            }
                            (final_entry, _) => final_entry,
                        };
                        match entry {
                            FollowerEntry::Local => {
                                if rf_num >= activation {
                                    break (activation, interval, rf_num, rf_hash);
                                }
                                // Third internal invariant of this dispatch, and named
                                // as one like the other two (`Checkpoint`,
                                // `Certificate`): below the activation block `Local` is
                                // returned ONLY for `holds_activation`, which IS
                                // `activation_hash.is_some()` of the very read below —
                                // no second probe stands between the verdict and here,
                                // so a `None` would mean the march and this dispatch
                                // disagree. The text it used to carry described a
                                // concurrent unwind, a state this input cannot produce
                                // (R-131 review, D-09).
                                let hash = activation_hash.ok_or_else(|| {
                                    eyre!(
                                        "cert-follow: internal — the entry march chose the \
                                         local anchor below the activation block {activation} \
                                         with no locally-held activation block"
                                    )
                                })?;
                                if warned_wait || unserved_reason.is_some() {
                                    sync_metrics.recover(SyncReason::ActivationWait);
                                }
                                info!(
                                    height = activation,
                                    hash = ?hash,
                                    "cert-follow: anchoring at the DPoS activation block reth \
                                     already holds"
                                );
                                break (activation, interval, activation, hash);
                            }
                            FollowerEntry::WaitLocal => {
                                // Honest sequencer→DPoS migration: the activation block
                                // is produced on THIS chain, so there is nobody to ask
                                // and nothing to authenticate. Retry-forever, Decision A,
                                // verbatim (`wait_for_activation_block`).
                                let hash = wait_for_activation_block(
                                    &ctx,
                                    &provider,
                                    activation,
                                    &sync_metrics,
                                )
                                .await?;
                                break (activation, interval, activation, hash);
                            }
                            FollowerEntry::Checkpoint => {
                                // `Checkpoint` is returned only for `has_checkpoint`, which IS
                                // `checkpoint_pending.is_some()` — a `None` here would mean the
                                // march and this dispatch disagree, which is a code fault and not
                                // an input the operator can produce.
                                let l1 = checkpoint_pending.take().ok_or_else(|| {
                                    eyre!(
                                        "cert-follow: internal — the entry march chose the \
                                         operator checkpoint with no checkpoint pending"
                                    )
                                })?;
                                info!(
                                    checkpoint = ?l1,
                                    activation,
                                    "cert-follow: below the activation block — EL-syncing to \
                                     the operator checkpoint FIRST (it is the entry the L1 \
                                     assert below is counted against)"
                                );
                                let (h, hash) = mk_el_sync(activation)
                                    .sync_to_checkpoint(l1)
                                    .await
                                    .wrap_err_with(|| {
                                        format!(
                                            "cert-follow: the operator --dpos.l1-checkpoint \
                                             {l1:?} could not be obtained from any peer (bogus \
                                             hash, or every peer is behind it)"
                                        )
                                    })?;
                                if h >= activation {
                                    break (activation, interval, h, hash);
                                }
                                // A checkpoint on a pre-DPoS batch is a legal input and a
                                // legal landing — it just is not a DPoS anchor. Re-run the
                                // march WITHOUT sleeping: the local probe may now hold the
                                // activation block, and otherwise the certificate entry is
                                // next. `assert_l1_checkpoint` after the match then passes
                                // trivially, which is the whole reason this step is first.
                                info!(
                                    landing = h,
                                    activation,
                                    "cert-follow: the operator checkpoint landed BELOW the \
                                     activation block (a pre-DPoS batch); continuing to the \
                                     certificate entry"
                                );
                                continue;
                            }
                            FollowerEntry::ChainBelowActivation => {
                                // NOT a fatal, for the same reason `wait_for_activation_block`
                                // is not (`:290-295`): the input is external, the honest
                                // case is "the chain / the committee commit is not there
                                // yet", and a fatal here restart-storms every honest joiner
                                // at once. Gauge + ONE `warn!` make the park visible and
                                // named instead.
                                if !warned_wait {
                                    warned_wait = true;
                                    warn!(
                                        activation,
                                        floor = activation + K,
                                        "cert-follow: the ordering chain is not an entry yet — \
                                         the upstream serves no finalized block at or above the \
                                         activation block + K, or no epoch committee is readable \
                                         at this node's own finalized hash yet \
                                         (commitEpochCommittee has not run). Polling (no \
                                         give-up)"
                                    );
                                }
                                sync_metrics.degrade(SyncReason::ActivationWait);
                                ctx.sleep(ACTIVATION_POLL).await;
                            }
                            FollowerEntry::Certificate { target } => {
                                // Same invariant as the checkpoint arm: `Certificate` is
                                // returned only for `has_upstream`.
                                let up = upstream.as_ref().ok_or_else(|| {
                                    eyre!(
                                        "cert-follow: internal — the entry march chose the \
                                         certificate entry with no upstream configured"
                                    )
                                })?;
                                let committees = crate::cert_inlet::RethCommitteeSource::new(
                                    RethStakingStateReader::new(
                                        provider.clone(),
                                        evm_config.clone(),
                                        staking_config.clone(),
                                    ),
                                    chain_id,
                                );
                                let mut fetch_ctx = ctx.clone();
                                let verified = fetch_verified_entry(
                                    up,
                                    &committees,
                                    &mut fetch_ctx,
                                    rf_hash,
                                    target,
                                )
                                .await;
                                let uf = match verified {
                                    Ok(uf) => uf,
                                    Err(reason) => {
                                        // A refusal does not distinguish "the upstream is
                                        // not up yet" from "nobody keeps this height any
                                        // more", and a fatal on the first is a restart
                                        // storm — so this parks, with the REASON and both
                                        // operator exits named. Re-printed only when the
                                        // reason changes: the cadence is 2 s and forever,
                                        // so a per-attempt line is noise, while a CHANGED
                                        // reason is the one thing worth a new line.
                                        if unserved_reason.as_deref() != Some(reason.as_str()) {
                                            warn!(
                                                target,
                                                activation,
                                                reason = %reason,
                                                "cert-follow: no upstream served a VERIFIABLE \
                                                 finalization at {target} — the entry below the \
                                                 activation block needs one. Two operator exits: \
                                                 give this node --dpos.l1-checkpoint (the L1 \
                                                 Rollup-finalized block becomes the entry), or \
                                                 point --dpos.follower-upstream at a validator \
                                                 whose archive still covers {target}. Polling \
                                                 (no give-up)"
                                            );
                                            unserved_reason = Some(reason);
                                        }
                                        sync_metrics.degrade(SyncReason::ActivationWait);
                                        ctx.sleep(ACTIVATION_POLL).await;
                                        continue;
                                    }
                                };
                                // A FAILED DRIVE IS A RETRY, NOT A STARTUP FATAL, and the
                                // reason is the #17 visibility race rather than politeness:
                                // `sync_to`'s post-landing `block_hash(landing)` can
                                // transiently miss a block reth has only just
                                // canonicalized, and that maps to `SyncFailure::Stalled`
                                // indistinguishably from a real stall (`From<Report>`), so
                                // `?` turned a race this file absorbs everywhere else into
                                // a dead node (R-131 review, D-15). Re-driving is safe and
                                // cheap: the target is already committee-authenticated, the
                                // next turn short-circuits on `best_block_number >=
                                // tip_height` if the landing did happen, and each attempt
                                // costs a full EL-sync net (≥ 90 s), so this cannot spin.
                                // A genuine wedge now parks OBSERVABLY — gauge up, one line
                                // per attempt — which is what the steady-state re-jump does
                                // with the same `Stalled`.
                                let (landing, hash) =
                                    match mk_el_sync(activation).sync_to(&uf).await {
                                        Ok(pair) => pair,
                                        Err(e) => {
                                            warn!(
                                                target,
                                                activation,
                                                error = %eyre::Report::from(e),
                                                "cert-follow: EL-sync toward the authenticated \
                                                 entry did not land — retrying the entry march \
                                                 (a transient post-landing read, or reth has no \
                                                 peers / a wedged pipeline; its own escape text \
                                                 names which)"
                                            );
                                            sync_metrics.degrade(SyncReason::ActivationWait);
                                            ctx.sleep(ACTIVATION_POLL).await;
                                            continue;
                                        }
                                    };
                                if warned_wait || unserved_reason.is_some() {
                                    sync_metrics.recover(SyncReason::ActivationWait);
                                }
                                info!(
                                    target,
                                    landing,
                                    activation,
                                    "cert-follow: entered below the activation block by an \
                                     authenticated certificate"
                                );
                                break (activation, interval, landing, hash);
                            }
                        }
                    }
                }
                None => {
                    // FRESH DATADIR. No geometry, no committee, no archive — the one
                    // entry where a peer's answer cannot be checked by anything local.
                    // The policy is the pure `fresh_follower_entry`; only the EL work
                    // is here.
                    let (h, hash, entry) =
                        match fresh_follower_entry(l1_checkpoint_hash, deployed_network, chain_id)?
                        {
                            // `sync_to_checkpoint` FCUs to the operator's HASH and learns
                            // the height from the landing, which is why the config needs
                            // no `(height, hash)` pair: reth reports the number once it
                            // holds the block canonically, and a hash it never
                            // canonicalizes stalls instead of landing somewhere else.
                            FreshFollowerEntry::Checkpoint(l1_hash) => {
                                info!(
                                    checkpoint = ?l1_hash,
                                    "cert-follow: fresh datadir — EL-syncing to the operator \
                                     checkpoint (the only entry with nothing local to check)"
                                );
                                let (h, hash) = mk_el_sync(0).sync_to_checkpoint(l1_hash).await?;
                                (h, hash, "the operator --dpos.l1-checkpoint block")
                            }
                            FreshFollowerEntry::UpstreamLatest => {
                                let up = upstream.as_ref().ok_or_else(|| {
                                    eyre!(
                                        "cert-follow: fresh datadir without a local ChainConfig \
                                         needs a reachable upstream to EL-sync from"
                                    )
                                })?;
                                let latest = up.get_latest().await.ok_or_else(|| {
                                    eyre!(
                                        "cert-follow: fresh datadir without a local ChainConfig \
                                         needs a reachable upstream to EL-sync from"
                                    )
                                })?;
                                warn!(
                                    chain_id,
                                    tip = latest.block.height,
                                    "cert-follow: devnet-only trust-on-first-use — a fresh \
                                     datadir with no --dpos.l1-checkpoint is EL-syncing to \
                                     whatever the upstream calls its tip. This entry is REFUSED \
                                     on a deployed network (E4-05)."
                                );
                                let (h, hash) = mk_el_sync(0).sync_to(&latest).await?;
                                (h, hash, "the devnet upstream's own tip")
                            }
                        };
                    let (activation, interval) =
                        read_geometry(&reader, hash)?.ok_or_else(|| {
                            eyre!(
                            "cert-follow: ChainConfig is not deployed at the synced entry {h} — \
                             wrong chain, or the entry predates DPoS activation"
                        )
                        })?;
                    // An entry BELOW the activation block is not a DPoS anchor at all, and
                    // the `h.max(activation)` clamp that used to stand here turned that into a
                    // read for a height reth does not hold: the visibility belt below then
                    // failed 10 s later naming the CLAMPED height, never the operator input
                    // that caused it. Refuse on the input instead (4.2 Б2 fix-1, B2-12).
                    ensure!(
                        h >= activation,
                        "cert-follow: {entry} is block {h}, BELOW the DPoS activation block \
                         {activation} — the ordering chain starts at activation, so nothing at \
                         or under {h} can be a DPoS anchor. Point the entry at a block at or \
                         above {activation}."
                    );
                    let hash = read_with_visibility_belt(
                        &ctx,
                        &sync_metrics,
                        &format!("the fresh-datadir landing {h}"),
                        || {
                            provider
                                .block_hash(h)
                                .wrap_err("reading the fresh-datadir landing hash")
                        },
                    )
                    .await?;
                    (activation, interval, h, hash)
                }
            };

        // B2 — L1 Rollup-checkpoint assert (verbatim strings), post-EL-sync,
        // fail-closed: a bogus-checkpoint cert-cascade follower still errors with
        // "is NOT in the local chain after EL-sync".
        if let Some(l1_hash) = l1_checkpoint_hash {
            crate::cold_start_jump::assert_l1_checkpoint(&provider, l1_hash)?;
        }

        // Two-tier seed: the landing block's own result attestation arrives only
        // K blocks later, so the finalized tier starts at landing − K (clamped to
        // activation). Seed reth's FCU at the floor so it never finalizes ahead of
        // the result tier.
        let finalized_floor = anchor_height.saturating_sub(K).max(activation);
        let finalized_hash = read_with_visibility_belt(
            &ctx,
            &sync_metrics,
            &format!("the finality floor {finalized_floor}"),
            || {
                provider
                    .block_hash(finalized_floor)
                    .wrap_err("reading finality floor hash")
            },
        )
        .await?;
        let _ = beacon_engine_handle
            .fork_choice_updated(ForkchoiceState {
                head_block_hash: anchor_hash,
                safe_block_hash: finalized_hash,
                finalized_block_hash: finalized_hash,
            })
            .await;

        let initial_epoch_u64 =
            fluentbase_staking_reader::reader::epoch_at_block(anchor_height, activation, interval)
                .ok_or_else(|| eyre!("epochBlockInterval is zero"))?;
        info!(
            chain_id,
            activation,
            interval,
            anchor_height,
            initial_epoch = initial_epoch_u64,
            "cert-follow (inlet) cold-start resolved"
        );

        // Consensus genesis = the ordering-chain anchor artifact at the resumed
        // height (every node derives the identical artifact; Simplex
        // `set_genesis(digest)` matches view 1's parent). A follower never
        // proposes, but the marshal/executor still need the genesis anchor.
        let genesis_unsealed = read_with_visibility_belt(
            &ctx,
            &sync_metrics,
            &format!("follower genesis block at height {anchor_height}"),
            || {
                provider
                    .block_by_number(anchor_height)
                    .wrap_err("reading follower genesis block")
            },
        )
        .await?;
        let genesis_sealed: SealedBlock<RethBlock> = SealedBlock::seal_slow(genesis_unsealed);
        let genesis_block = anchor_order_block(&genesis_sealed)?;

        let last_execution_finalized_height = provider
            .last_block_number()
            .wrap_err("provider failed to report chain head block number at startup")?;
        let head_info = canonical_state.chain_info();

        let epoch_length_blocks =
            NonZeroU64::new(interval).ok_or_eyre("epoch_block_interval must be > 0")?;
        // The beacon-owned families are registered by `beacon::build_follower`
        // below, which is this path's randomness provider — NOT here. A standalone
        // `BeaconMetrics::register` next to it would register every one of them
        // TWICE: `prometheus_client::Registry::register` neither deduplicates nor
        // complains, so the only symptom is a duplicated family at scrape time.
        //
        // Registered here, on the SAME context, for the reason the metric-owner
        // split exists: these families are core- and executor-owned on BOTH node
        // classes, and `prometheus_client::Registry::register` neither
        // deduplicates nor complains — a family with no owner on this path would
        // simply stop appearing in follower scrapes.
        let epoch_metrics = crate::epoch_manager::EpochEngineMetrics::default();
        epoch_metrics.register(&ctx);
        let executor_metrics = crate::executor::ExecutorMetrics::default();
        executor_metrics.register(&ctx);

        // EVERY per-epoch committee read this follower makes, as ONE frozen
        // record per epoch at ONE anchor — the same module the validator plane
        // runs, over this node's own executor cursor. A follower's geometry is
        // known HERE (it was read off the chain at the cold-start anchor above),
        // so the watch is created already frozen; the validator's arrives later
        // from its beacon plane, which is why the store takes a watch at all.
        // Consequence, stated because the validator path has to do the opposite:
        // there is NO unfrozen window on this path and therefore no freeze
        // wake-up to publish (`Committee::subscribe`'s second event) — the store
        // is answering from its first call, and the only wake-up left is the
        // executor's anchor advance.
        //
        // The verify-only scheme of every epoch is built HERE too, by the one
        // producer the store owns: `beacon_slot` is filled the moment
        // `beacon::build_follower` returns (below), and until then the store
        // answers "no scheme yet" and retries — the build order makes a value
        // impossible, because the beacon is constructed FROM this store's facade.
        let beacon_slot: crate::committee::BeaconSlot = Arc::new(std::sync::OnceLock::new());
        let committee: Arc<dyn crate::committee::Committee> =
            Arc::new(crate::committee::CommitteeStore::new(
                RethStakingStateReader::new(
                    provider.clone(),
                    evm_config.clone(),
                    staking_config.clone(),
                ),
                Arc::new(crate::committee::RethAnchor::new(
                    finalized_cursor.clone(),
                    provider.clone(),
                )),
                tokio::sync::watch::Sender::new(Some((activation, interval))).subscribe(),
                crate::committee::epoch_verifier(chain_id, beacon_slot.clone()),
            ));

        // Steady-state self-healing re-jump (finding #6): the follower's executor
        // reaction to its own `Update::Tip` event, and since pass Б2 the ONLY jump
        // on the follower path too: the pre-engine one is gone and the anchor above
        // is either `rf_hash` or the operator checkpoint. Same upstream / EL-sync /
        // activation / L1 checkpoint; no committee source (the target is a pair out
        // of this follower's own marshal archive). A follower ALWAYS has an upstream
        // (the WS the inlet uses), so this is set whenever `upstream.is_some()`.
        //
        // Epoch-relative re-jump gate: the defer deadlock is "≥2 epochs behind", so
        // recovery fires at `min(serving-window, 1 epoch)` — real-prod epochs ≫ 1024
        // keep 1024; a compressed test epoch heals within an epoch (see `ReJump::threshold`).
        let re_jump_threshold = crate::cold_start_jump::JUMP_THRESHOLD.min(interval);
        let re_jump: Option<crate::executor::ReJump> = upstream.as_ref().map(|up| {
            let up = up.clone();
            // The inlet's SAME upstream-rotation escape (Rule L): the re-jump's
            // terminal fault (a repeated `Stalled`) rotates the SAME WS
            // actor mailbox the inlet's `inlet_rotate` wraps → coalesced at the WS
            // actor. Bound BEFORE the cb moves `up`.
            let rotate = up.rotate_callback();
            let provider = provider.clone();
            let beacon_engine_handle = beacon_engine_handle.clone();
            let ctx = ctx.clone();
            let peer_count = peer_count.clone();
            let cb: crate::executor::ReJumpFn = Arc::new(
                move |from: u64, target: crate::cert_follow::UpstreamFinalized| {
                    let provider = provider.clone();
                    let beacon_engine_handle = beacon_engine_handle.clone();
                    let peer_count = peer_count.clone();
                    let jump_ctx = ctx.clone();
                    Box::pin(async move {
                        let el = crate::cold_start_jump::RethElSync::new(
                            jump_ctx,
                            provider.clone(),
                            beacon_engine_handle,
                            activation,
                            peer_count,
                        );
                        // Return the typed terminal `JumpOutcome` verbatim — the
                        // executor's completion arm classifies it (§9.6). The target
                        // is a pair out of this follower's OWN marshal archive, so
                        // there is no committee source and no verify RNG (pass Б2).
                        crate::cold_start_jump::jump_to_target(
                            from,
                            target,
                            &el,
                            l1_checkpoint_hash,
                            activation,
                            re_jump_threshold,
                        )
                        .await
                    }) as futures::future::BoxFuture<'static, _>
                },
            );
            crate::executor::ReJump {
                call: cb,
                threshold: re_jump_threshold,
                rotate: Some(rotate),
                // THE SAME LADDER AS THE VALIDATOR (review B1-01), over the SAME
                // constructor — only the `U: CertUpstream` differs (the WS handle
                // here, the frontier resolver there).
                //
                // The follower used to wire `probe: None` on the reasoning that its
                // WS inlet is an always-on live producer, and that was wrong in the
                // one state the jump exists for. The inlet is a SUBSCRIPTION to
                // current finalizations: it replays no intermediate height, and
                // every cert it ingests more than two epochs above this node's own
                // anchor is deferred by the committee read window
                // (`cert_inlet.rs` → `committee.scheme(E)`), storing nothing. With
                // `upstream_frontier` gone (§5.2) the trigger reads the marshal tip
                // alone, so at `fin == tip == last(epoch(fin)+2)` the gap is 0, the
                // tip is frozen and nothing local can unfreeze it — a permanent
                // silent park. The step `Finalized{last(T+1)}` is what unfreezes
                // it: the marshal's own resolver pulls that height by number
                // (`UpstreamResolver` → this node's WS upstream), `verify_delivered`
                // stores it, and `Update::Tip` re-arms the ordinary trigger.
                probe: Some(frontier_probe(up.clone(), committee.clone())),
                // `T` computed LOCALLY: a follower runs no `EpochTransition` to
                // mirror (see below, where the poller is the validator's), so it
                // applies ET's own rule to the committee module's geometry and the
                // ordering-finalized cursor the module is already anchored on —
                // `local_tracked_epoch`.
                tracked_epoch: Some(local_tracked_epoch(
                    committee.clone(),
                    finalized_cursor.clone(),
                )),
            }
        });

        // Boundary seeding seam — see the twin in `launch` for the full rationale. A
        // follower spawns no per-epoch engine, so the `Inline::genesis` precondition
        // this seeds is not its own concern; it is wired here because a follower runs
        // the SAME jump and therefore leaves the SAME below-floor hole, and an
        // upstream-configured node can be promoted to a seated validator by a later
        // committee change without restarting.
        let boundary_fetch: Option<crate::cert_follow::BoundaryFetchFn> =
            upstream.as_ref().map(|up| {
                let up = up.clone();
                let provider = provider.clone();
                let evm_config = evm_config.clone();
                let staking_config = staking_config.clone();
                let ctx = ctx.clone();
                let sync_metrics = sync_metrics.clone();
                Arc::new(move |height: u64, at_hash: B256| {
                    let up = up.clone();
                    let provider = provider.clone();
                    let evm_config = evm_config.clone();
                    let staking_config = staking_config.clone();
                    let sync_metrics = sync_metrics.clone();
                    let mut fetch_ctx = ctx.clone();
                    Box::pin(async move {
                        let committees = crate::cert_inlet::RethCommitteeSource::new(
                            RethStakingStateReader::new(
                                provider.clone(),
                                evm_config,
                                staking_config,
                            ),
                            chain_id,
                        );
                        crate::cert_follow::fetch_verified_boundary(
                            &up,
                            &committees,
                            &mut fetch_ctx,
                            &sync_metrics,
                            at_hash,
                            height,
                        )
                        .await
                    }) as futures::future::BoxFuture<'static, _>
                }) as crate::cert_follow::BoundaryFetchFn
            });

        // The follower's boundary trigger, producer half. `boundary_hook` fires
        // synchronously inside the marshal's reporter for every finalized
        // `OrderBlock`, so it may only stamp and wake — the committee read and the
        // delivery run on the driver task below. `fetch_max` + a permit-storing
        // `Notify` is lossless in the only sense that matters here: the driver
        // reads the LATEST height, and a wake landing while it is not armed is held
        // as a permit rather than dropped.
        let follower_finalized = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let follower_finalized_wake = Arc::new(tokio::sync::Notify::new());
        let boundary_hook: Arc<dyn Fn(OrderBlock) + Send + Sync> = {
            let height = follower_finalized.clone();
            let wake = follower_finalized_wake.clone();
            Arc::new(move |block: OrderBlock| {
                height.fetch_max(block.height, std::sync::atomic::Ordering::Relaxed);
                wake.notify_one();
            })
        };

        // Every staking read the follower's beacon takes, answered from the
        // committee module — the SAME type the validator plane is handed, so the
        // two node classes cannot drift on the cursor or on the reads. The
        // follower's own `CommitteeReads` implementation is gone with it, and
        // with it the `committee() => None` hole it carried: a follower runs no
        // ceremony, but the module has no reason to withhold the roster.
        let follower_committees: Arc<dyn crate::beacon::CommitteeReads> = Arc::new(
            crate::committee::CommitteeReadsFacade::new(committee.clone()),
        );
        // The ONE relationship a follower has. `None` (a test with no upstream)
        // leaves the rung permanently empty, which is exactly the pre-FLU-1167
        // behaviour: vote-only admission, never a fault.
        let artifact_fetch: crate::beacon::ArtifactFetch = match &upstream {
            Some(up) => {
                let up = up.clone();
                Arc::new(move |epoch: u64| {
                    let up = up.clone();
                    Box::pin(async move {
                        crate::cert_follow::CertUpstream::get_epoch_artifact(&up, epoch).await
                    }) as futures::future::BoxFuture<'static, _>
                })
            }
            None => Arc::new(|_| Box::pin(async { None })),
        };
        // A follower's randomness is negative on everything it PRODUCES — no DKG,
        // no ceremony store, no agreement instance, no share, no seed — and real
        // on the one thing it VERIFIES: the key a certificate of an epoch must be
        // checked against. It used to be `beacon::absent`, i.e. negative there
        // too, BY TYPE and for the life of the process; that was the whole of
        // FLU-1167. What was missing was never the check — `verify_artifact` needs
        // only the chain id, an rng and a `committee[epoch]` read the inlet makes
        // per certificate — but a DELIVERY ROUTE, which the three capabilities
        // above now supply over the cert upstream.
        //
        // The durable key journal that used to be opened here (with an empty
        // partition, i.e. RAM-only) stays DELETED: the store this fills is RAM-only
        // by design, and what a restart loses is one fetch per epoch over a link
        // the follower holds open anyway. Its seed journal goes with it, and that
        // one has a real consequence worth stating — a node that ran as a validator
        // and is restarted as `--cert-follow` no longer replays its seed-journal
        // tail. Nothing on the follower path reads a seed it did not just receive,
        // so the tail was already unreadable there; what is lost is the ability to
        // hand it back on a later switch BACK to validator, which is a restart
        // through this path either way.
        let (randomness, beacon_tasks) = crate::beacon::build_follower(
            &ctx,
            crate::beacon::FollowerInputs {
                chain_id,
                committees: follower_committees,
                fetch: artifact_fetch,
            },
        );
        // The committee module's scheme producer can build now: every epoch it
        // reads from here on gets its verify-only scheme with THIS beacon's
        // oracle, and the handful of epochs it may have read before this line
        // pick theirs up on the next `Committee::scheme`.
        crate::committee::fill_beacon_slot(&beacon_slot, &randomness);
        // WEAK for the same reason the validator's is: this closure is handed to
        // the RPC feed, which outlives every task the supervisor aborts. A strong
        // clone here would keep the beacon — and with it any journal sender it
        // owns — alive past shutdown. A failed upgrade answers `None`, which is
        // the right answer once the beacon is gone.
        let artifact_bytes = {
            let beacon = Arc::downgrade(&randomness);
            Arc::new(move |epoch: u64| beacon.upgrade()?.artifact_bytes(epoch))
                as std::sync::Arc<dyn Fn(u64) -> Option<Vec<u8>> + Send + Sync>
        };
        let artifact_fetch_handle = beacon_tasks.supervised;
        // Held and drained like the validator's, not dropped here. `Tasks` is a
        // contract — TWO handles the node owes the beacon — and today's follower
        // drain is an empty task only because a follower opens no journal
        // partition yet. Dropping it would make the day row 5.1 gives the
        // follower a durable artifact store the day its writer goes undrained
        // SILENTLY, with nothing at this call site to change.
        let beacon_drain_handle = beacon_tasks.drain;
        let outer = OuterBuilder {
            me: me.clone(),
            // A follower runs no beacon plane, so it starts no agreement instance
            // and has none to adopt.
            agreement_intake: None,
            // Bug A: no-op the blocker on the follower too. The follower spawns no
            // simplex batcher, but it DOES run the marshal cert resolver, whose
            // `deliver=false` verdict would `block!` a registry-∪-committee peer on
            // the shared global transport — the same self-partition the beacon
            // resolver already no-op'd. `provider:` keeps the real oracle.
            blocker: NoopBlocker,
            provider: oracle.clone(),
            chain_id,
            epoch_length_blocks,
            dpos_activation_block: activation,
            signer_keypair: None,
            // A follower runs no agreement plane, so it never MINTS an artifact,
            // and the plane's pull seam does not reach it: that delivers over
            // `BEACON_RESOLVER_CHANNEL` and this node's p2p identity is ephemeral,
            // bootstrapper-less and never tracked into any committee's peer set.
            // What it does have is one delivery route over its cert upstream, so
            // its ladder is no longer permanently empty: vote-only admission now
            // lasts until the epoch's artifact arrives and verifies against
            // `committee[epoch]`, not until the process exits. The multisig quorum
            // is verified either way; the seed check is what it does without in
            // the meantime.
            randomness: randomness.clone(),
            spawn_unblocked: Arc::new(tokio::sync::Notify::new()),
            re_jump,
            epoch_metrics: epoch_metrics.clone(),
            executor_metrics: executor_metrics.clone(),
            sync_metrics: sync_metrics.clone(),
            safety_halt: safety_halt.clone(),
            // The follower has no beacon plane, so no tombstone watcher fills a
            // set here. It casts no vote and proposes no block, so both readers of
            // this handle are unreachable on the follower path — an empty set is
            // the honest state, not a lost signal.
            tombstones: crate::slasher::TombstoneSet::default(),
            // A follower runs no beacon plane, hence no finalized poller and no
            // DKG clock — there is no second clock here to diverge from the
            // ordering tip. An unregistered handle publishes nothing, which is
            // the honest answer rather than a lag gauge reading the ordering tip
            // against a permanent zero.
            plane_clock: crate::sync_metrics::PlaneClock::default(),
            // Same reason: no beacon plane means no height channel to feed.
            dkg_height_tx: None,
            timeouts: ConsensusTimeouts::fluent_1s(),
            mailbox_size: 256,
            // BROADCAST body cache: at most 4 order-block bodies retained per
            // PRIMARY sender (`CW:broadcast/src/buffered/engine.rs:319-322`,
            // `:353-359`). 64 was 64 × `MAX_ORDER_BLOCK_SIZE` = 256 MiB per peer,
            // and the primary set used to be the whole registry (R-013, E4-14);
            // 4.3 makes primary the three committee records, and 4 covers the
            // deepest legitimate pipeline (the proposal in flight plus a re-proposal
            // after nullify) with a spare. There is NO byte cap to pair it with:
            // `buffered::Config` carries `deque_size` and nothing else
            // (`CW:broadcast/src/buffered/config.rs:5-22`), so the per-peer memory
            // bound is `deque_size × MAX_ORDER_BLOCK_SIZE` — a library boundary,
            // not a choice made here.
            deque_size: 4,
            partition_prefix: MARSHAL_PARTITION_PREFIX.into(),
            engine_partition_prefix: String::new(),
            resolver_initial: Duration::from_secs(1),
            resolver_timeout: Duration::from_secs(2),
            resolver_fetch_retry: Duration::from_millis(100),

            genesis: genesis_block,
            beacon_engine: beacon_engine_handle,
            deriver,
            executed,
            assembler,
            target_gas_limit,
            boundary_hook,

            last_execution_finalized_height,
            initial_finalized: (Height::new(anchor_height), anchor_hash),
            initial_head: (Height::new(head_info.best_number), head_info.best_hash),
            // The follower's DPoS-era floor. A later steady-state jump raises it to
            // `landing − K` through `set_floor`, not here.
            marshal_floor: Some(Height::new(finalized_floor)),
            boundary_fetch,
            // Height-keyed epoch entry, and the read floor a re-jump publishes:
            // both exist to serve `EpochTransition`, which a follower does not
            // run. Its epoch entry rides `boundary_hook` above instead, and it
            // clamps no committee read to a jump landing because the trigger reads
            // at the CURRENT finalized hash, never at a historical one.
            boundary_enter: Arc::new(|_| {}),
            boundary_read_floor: Arc::new(|_| Box::pin(async {})),
            fcu_heartbeat_interval,
            fcu_pace: Duration::from_millis(20),
            canonical_state: canonical_state.clone(),

            slasher_staking_address: staking_config.staking_address,
            committee: committee.clone(),
            slasher_sink: Arc::new(NoopSlasherSink),
            slasher_wal_partition: "slasher-wal".into(),
            // A follower runs no slasher (built, never started) and registers
            // no evidence channel, so there is nothing to bridge to.
            slasher_evidence: None,

            feed,

            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine: None,
        }
        // Sibling of the engine context, not a child — see the validator path
        // and `OuterBuilder::build` for why the writer must sit outside the
        // engine's supervision subtree.
        .build(ctx.with_label("outer_engine"))
        .await?;

        // The starting epoch's verify-only scheme, taken the way every other
        // epoch's is: by READING its committee through the module, which installs
        // the record and the scheme in one slot. The `RethCommitteeSource` +
        // `finalized_hash` closure that used to build a scheme here — and the
        // `cold_start_register` that put it in a second map with `oracle: None` —
        // are gone with the second map.
        //
        // The two arms are the validator path's, for the validator path's
        // reasons: a PERMANENT refusal is a fact about the chain and stays the
        // loud startup refusal, while a retryable miss is this process's startup
        // order. The follower's retry is its own boundary trigger rather than an
        // `EpochTransition`: `enter_finalized_epoch` below leaves the epoch
        // unconsumed while `committee_at` answers `false` and re-delivers it on
        // the next finalized block, and once delivered a reconcile that still
        // cannot read parks it in the manager's `deferred_reconciles`.
        match committee.committee(initial_epoch_u64) {
            Ok(record) => info!(
                epoch = initial_epoch_u64,
                members = record.members.len(),
                "follower cold-start committee read through the committee module"
            ),
            Err(e) if !e.is_transient() => {
                return Err(eyre!(
                    "committee[{initial_epoch_u64}] is REFUSED PERMANENTLY at this follower's \
                     committee anchor: {e}. This is a statement about chain state — the \
                     contract answered something no committed epoch can answer, or the epoch \
                     is below the module's read window — and no retry can change it; the node \
                     would follow certificates it can never verify."
                ));
            }
            Err(e) => warn!(
                epoch = initial_epoch_u64,
                error = %e,
                "follower cold-start committee not readable at this node's anchor YET — a \
                 retryable miss; certificates of this epoch are deferred until the boundary \
                 trigger re-delivers it"
            ),
        }

        // The follower's boundary trigger, consumer half — the twin of the
        // validator path's `epoch_bridge` forwarder, and SUPERVISED for the same
        // reason: if it dies the node keeps following certificates while silently
        // never entering another epoch, so its committee schemes and its repair
        // sweep freeze at whatever the cold-start left.
        //
        // Committee reads run at the CURRENT EL-finalized hash — the same anchor
        // `soft_enter_committees` uses, and the same reason: the committee array
        // and the one-shot keys are frozen storage, so any in-epoch executed hash
        // at or past the commit yields the identical snapshot. `None` (no
        // finalized block yet, or `committee[E]` not committed at that hash) leaves
        // the epoch unconsumed and the next finalized block retries.
        let follower_boundary_tx = outer.boundary_sender();
        let follower_boundary_handle = {
            // The module's own readability answer, and the one call that
            // REGISTERS the epoch: reading `committee[E]` installs the record and
            // its verify-only scheme in one slot, which is what the manager's
            // reconcile then finds. The `ValidatorSetSnapshot` this closure used
            // to project is gone with the channel that carried it — the manager
            // re-reads the record itself.
            let committee = committee.clone();
            let committee_at: FollowerCommitteeAt =
                Arc::new(move |epoch: u64| committee.scheme(epoch).is_some());
            let deliver: FollowerBoundaryDeliver = Arc::new(move |epoch| {
                let tx = follower_boundary_tx.clone();
                Box::pin(async move { tx.send(epoch).await.is_ok() })
                    as futures::future::BoxFuture<'static, bool>
            });
            let wake = follower_finalized_wake.clone();
            let height = follower_finalized.clone();
            ctx.with_label("follower_boundary")
                .spawn(move |_| async move {
                    let mut last_delivered: Option<u64> = None;
                    loop {
                        // `notify_one` stores a permit when nobody is waiting, so
                        // a block reported while this task is inside the read
                        // below wakes the NEXT iteration instead of being lost —
                        // the same object-scoped-permit argument the epoch
                        // manager's own edges rest on.
                        let woken = wake.notified();
                        let finalized = height.load(std::sync::atomic::Ordering::Relaxed);
                        if finalized != 0
                            && !enter_finalized_epoch(
                                &mut last_delivered,
                                finalized,
                                activation,
                                interval,
                                &committee_at,
                                &deliver,
                            )
                            .await
                        {
                            error!(
                                "epoch_manager boundary receiver dropped — follower epoch \
                                 entry stopping"
                            );
                            return;
                        }
                        woken.await;
                    }
                })
        };

        // Two clones: one drives the inlet, one is returned to the node for the
        // `consensus`-RPC feed (`set_marshal`/`set_window`).
        let cert_mailbox = outer.marshal_mailbox();
        let inlet_marshal = outer.marshal_mailbox();

        // Start the follower OuterEngine over the ONE broadcast Muxer. The marshal
        // resolver is UPSTREAM-backed (not p2p): it backfills the by-height gap
        // between the cold-start floor and the upstream's live frontier — the gap
        // the inlet's live stream never carries — by pulling each missing height
        // from the cert upstream and delivering it for the marshal to BLS-verify.
        // `upstream` is `Clone` (CertUpstream), so the resolver gets its own handle
        // while the inlet keeps one alive for the live stream + the WS actor. The
        // executor is the sole reth writer from here.
        let consensus_handle = outer.start_follower(broadcast_mux, ctx.clone(), upstream.clone());

        // The cert-inlet — the SOLE producer for a follower. Drives the marshal
        // (which drives the executor) + the B3 serving window. Runs on a child
        // task; fail-closed on TOTAL upstream loss (finalized_rx close).
        // Observability for the committee-read defer regimes (the reth
        // pipeline-backfill fail-open): registered ONCE on the launch context so
        // the family carries the launch prefix, then cloned into the closure below
        // (probe_inconsistency) and moved into the inlet (state_not_materialized /
        // committee_not_committed).
        let committee_read_deferred: Family<crate::cert_inlet::CommitteeReadDeferLabels, Counter> =
            Family::default();
        ctx.register(
            "dpos_cert_inlet_committee_read_deferred",
            "Upstream certs deferred (not fatal) on a committee read that could not resolve, \
             by `reason` (state_not_materialized / committee_not_committed / probe_inconsistency).",
            committee_read_deferred.clone(),
        );
        // Stale-cursor observability: verify failures under a CARRY-FORWARD seed
        // pin (a key carried forward from an earlier mint) vs genuine forged-
        // upstream data faults. Registered once here, moved into the inlet.
        let carry_forward_verify_failed = Counter::default();
        ctx.register(
            "dpos_cert_inlet_carry_forward_pin_verify_failed",
            "Upstream cert BLS-verify failures where the seed pin was DERIVED by the \
             ladder (the shared store, or the minting epoch's agreement artifact) \
             rather than already held by the cached scheme.",
            carry_forward_verify_failed.clone(),
        );
        // The inlet verifies with the module's own scheme for the cert's epoch —
        // the same map the boundary trigger above registers into and the same one
        // the marshal verifies with. Its private `RethCommitteeSource` over a
        // `max(EL-finalized, live)` cursor with a three-branch executed-state
        // probe is gone; so is the `probe_inconsistency` defer it produced, which
        // was a fault of that closure and not of the chain.
        let inlet_committee = committee.clone();
        let shutdown_for_inlet = shutdown.clone();
        // The SAME provider the epoch manager holds, not a second one over a
        // private store, and on this path that is load-bearing: the σ the inlet
        // files through `observe_certificate` and the key `ensure_key` resolves
        // are the SAME index and the SAME key store the epoch manager and the
        // executor read. A second instance would hold a σ nothing derives from and
        // fetch into a key store the repair sweep never sees. (The `observe_cert`
        // prune this note used to name is gone with row 5.2 — the index measures
        // its own window; the argument does not depend on it.)
        let inlet_randomness = randomness.clone();
        // DATA-fault rotation trigger (#7): after MAX_UPSTREAM_FAULTS consecutive
        // unverifiable certs over a healthy connection the inlet rotates to the
        // next configured upstream URL (connection-level failover can never see a
        // bad PAYLOAD on a live connection). Built from a clone of the SAME
        // `CertUpstream` handle the keepalive holds; `rotate()` drops the
        // connection so the WS actor's run loop advances to the next URL.
        let inlet_rotate: Option<crate::cert_inlet::RotateUpstream> = upstream
            .as_ref()
            .map(crate::cert_follow::CertUpstream::rotate_callback);
        // SUPERVISED, not detached — the follower twin of the validator path's
        // `("inlet", h)` registration (`node/src/cert_inlet.rs` → `dpos.rs`).
        // The `shutdown.cancel()` at the end of the body covers the LOOP exits
        // (total upstream loss / committee fatal); a PANIC skips it entirely and
        // `with_catch_panics(true)` swallows it, which on this path means zero
        // certificate ingestion with a live network and a live RPC — the node
        // looks healthy and follows nothing. The returned handle resolves
        // `Err(Error::Exited)` on that panic, so the host supervisor fails the
        // node closed exactly as the loop exits already do.
        let cert_inlet_handle = ctx.with_label("cert_inlet").spawn(move |c| async move {
            // Hold the WS upstream REQUEST handle alive for the inlet's whole
            // lifetime. The WS actor's `run` loop exits the instant ALL
            // `UpstreamHandle`s drop (its `mailbox_rx` closes → `None => return`),
            // and that SAME actor feeds `finalized_rx` (the inlet's sole producer).
            // The cold-start only borrows it (the devnet fresh-datadir
            // `get_latest`), so without this
            // move it would drop when `launch_follower` returns → the WS actor
            // exits cleanly → the node-stack supervisor tears the follower down
            // before it ever follows. The marshal's `UpstreamResolver` holds its
            // OWN clone of this same handle for by-height gap-repair pulls; this one
            // keeps the live stream + WS actor alive.
            let _upstream_keepalive = upstream;
            // B4: `--cert-follow` is ALWAYS verify (the inlet has no no-verify mode).
            // `with_epoch_math` arms the defense-in-depth height↔epoch bind: the
            // follower fully trusts upstream committee reads, so it binds each
            // cert's round-epoch to its block's height-derived epoch.
            //
            // The live-frontier tee is wired for the ONE cursor it still has, and
            // the follower does not run it: it has no beacon plane, so the DkgActor
            // deal clock is a no-op — drop the receiver and each `try_send` is a
            // benign Closed. (`live_height` went in 4.2 with `upstream_frontier`:
            // the committee module reads at this node's own ordering-finalized
            // anchor, so the frontier-aware read it existed for has no consumer.)
            let (dkg_tx, dkg_rx) = tokio::sync::mpsc::channel::<u64>(1);
            drop(dkg_rx);
            let mut inlet = crate::cert_inlet::CertInlet::new(inlet_marshal, inlet_committee, c)
                .with_epoch_math(activation, interval)
                .with_committee_read_deferred_metric(committee_read_deferred)
                .with_carry_forward_fail_metric(carry_forward_verify_failed)
                .with_randomness(inlet_randomness)
                .with_tee(crate::cert_inlet::LiveFrontierTee {
                    dkg_height_tx: dkg_tx,
                    // Unregistered, like this path's other clock handles: every
                    // `try_send` above is a by-design `Closed`, so counting them
                    // against a registered counter would publish a drop series for
                    // a clock this node deliberately does not run.
                    plane_clock: crate::sync_metrics::PlaneClock::default(),
                });
            if let Some(rotate) = inlet_rotate {
                inlet = inlet.with_rotate(rotate);
            }
            // Per-connection data-fault scoping (#7): a connection-level
            // auto-rotation (the WS actor reconnecting to the next URL on a
            // dropped/failed connection) bumps `conn_gen`; the inlet resets its
            // streak on the change so A's faults never bleed into B's budget.
            if let Some(conn_gen) = conn_gen {
                inlet = inlet.with_connection_token(conn_gen);
            }
            if let Some(tx) = verified_tx {
                inlet = inlet.with_window(tx);
            }
            let mut finalized_rx = finalized_rx;
            info!("cert-inlet follower producer started");
            loop {
                match finalized_rx.recv().await {
                    // INFALLIBLE by type — see `CertInlet::ingest` and the
                    // validator-side twin in `node/src/cert_inlet.rs`. The
                    // fail-closed exit below is TOTAL upstream loss, which is a
                    // different fact and the only one left.
                    Some(uf) => inlet.ingest(uf).await,
                    None => {
                        error!(
                            "cert-inlet WS stream closed (all upstreams dead); exiting fail-closed"
                        );
                        break;
                    }
                }
            }
            // TOTAL upstream loss is fail-closed (Risk-3): cancel the shared
            // shutdown so the host brings the node down (case-cert-cascade A3
            // accepts an `exited` state; a silent hang would fail it).
            shutdown_for_inlet.cancel();
        });

        Ok(DposLayerHandle {
            consensus_handle,
            cert_mailbox,
            supervised: vec![
                ("cert_inlet", cert_inlet_handle),
                ("follower_boundary", follower_boundary_handle),
                // SUPERVISED, not detached: it parks rather than returning, so a
                // clean exit means it died — and a dead fetcher silently returns
                // this node to vote-only admission for the rest of the process,
                // with every liveness check still green.
                ("follower_artifact_fetch", artifact_fetch_handle),
            ],
            // The beacon's drain, registered exactly as the validator's is. A
            // follower opens no journal partition today, so the handle behind it
            // is an empty task that returns at once — the registration is what
            // keeps the contract true when row 5.1 gives it one.
            drain_on_shutdown: vec![("beacon", beacon_drain_handle)],
            artifact_bytes: Some(artifact_bytes),
        })
    }
}

#[cfg(test)]
mod cold_start_kind_tests {
    use super::{
        follower_entry, fresh_follower_entry, resolve_cold_start_kind, ColdStartKind,
        FollowerEntry, FreshFollowerEntry, PeerProbes, K,
    };
    use alloy_primitives::B256;

    const ACTIVATION: u64 = 192;
    const INTERVAL: u64 = 64;
    /// The devnet geometry (`genesis-bootstrap`: `EPOCH_BLOCK_INTERVAL=32`,
    /// `dposActivationBlock=2 * interval`), used where the target arithmetic is what
    /// the smoke case will read back out of the log.
    const DEVNET_ACTIVATION: u64 = 64;
    const DEVNET_INTERVAL: u64 = 32;
    /// Any of the deployed chain_ids would do — the predicate is the node's
    /// (`node/dpos.rs::is_deployed_network`); this crate only receives its answer.
    const A_DEPLOYED_CHAIN: u64 = 0x5202;
    const A_LOCAL_CHAIN: u64 = 1337;

    /// (4.2 Б2.7в) The fresh-datadir follower entry, the ONE place in the system
    /// with nothing local to check a peer against. On a DEPLOYED network a missing
    /// operator checkpoint is a startup REFUSAL naming the flag (E4-05); off one it
    /// is trust-on-first-use, which the caller logs as such. A checkpoint wins on
    /// either.
    ///
    /// Falsifier: a deployed network without a checkpoint that returns an entry (the
    /// refusal is gone); a local network that refuses (devnet cannot start); a
    /// checkpoint that does not become the entry.
    #[test]
    fn a_fresh_datadir_without_a_checkpoint_refuses_on_a_deployed_network() {
        let err = fresh_follower_entry(None, true, A_DEPLOYED_CHAIN)
            .expect_err("a deployed network must refuse trust-on-first-use");
        let text = format!("{err:#}");
        assert!(
            text.contains("--dpos.l1-checkpoint"),
            "the refusal does not name the flag the operator has to set: {text}"
        );
        assert!(
            text.contains(&A_DEPLOYED_CHAIN.to_string()),
            "the refusal does not name the chain it fired on: {text}"
        );

        assert_eq!(
            fresh_follower_entry(None, false, A_LOCAL_CHAIN).expect("a local network may start"),
            FreshFollowerEntry::UpstreamLatest,
            "off a deployed network the upstream's own tip is the (named) fallback"
        );

        let cp = B256::repeat_byte(0x7e);
        for deployed in [true, false] {
            assert_eq!(
                fresh_follower_entry(Some(cp), deployed, A_DEPLOYED_CHAIN)
                    .expect("a checkpoint is always a legal entry"),
                FreshFollowerEntry::Checkpoint(cp),
                "the operator checkpoint is not the entry (deployed = {deployed})"
            );
        }
    }

    /// (4.2 Б2.7а) THE EMPTY-ARCHIVE ANCHOR. An empty consensus archive with the EL
    /// already past epoch 0, and an upstream, resolves to [`ColdStartKind::ElFinalized`]
    /// — the kind whose anchor in `launch` is reth's OWN finalized pair
    /// `(cs_finalized, cs_finalized_hash)`, not the genesis (`archive_finalized`,
    /// where a runtime-deployed ChainConfig is codeless) and not an upstream's
    /// `Latest` (nothing local can check it).
    ///
    /// RED BEFORE THIS CHANGE, verbatim: on HEAD `f8ec4939` this arm returned
    /// `ColdStartKind::Restart` — the kind whose `launch` anchor is
    /// `archive_finalized`, i.e. the GENESIS hash for an empty archive, held there
    /// until a `get_latest`-targeted jump landed (and `cold_start_jump_eligible`
    /// answered `true` for it, which is why the retry-forever loop existed). Both the
    /// variant and that function are gone, so this assertion could not compile there.
    ///
    /// WHAT THIS TEST DOES NOT COVER, and the name says so (4.2 Б2 fix-1, B2-07):
    /// the KIND is all it pins. The anchor ITSELF — that `launch`'s `ElFinalized` arm
    /// binds `(cs_finalized, cs_finalized_hash)` and not some other pair — is held by
    /// the compiler and by reading alone: `launch` is never entered from a test
    /// (`testbed/mod.rs` drives the stand below it), so a mutation of that arm's
    /// tuple reddens nothing here.
    ///
    /// Falsifier: a `Restart` (the anchor would be the genesis hash); a
    /// `FreshMigration` (the anchor would be the activation block, orphaning the EL
    /// tail).
    #[test]
    fn empty_archive_with_the_el_past_epoch_zero_resolves_to_el_finalized() {
        let kind = resolve_cold_start_kind(0, ACTIVATION, INTERVAL, ACTIVATION + 500, true)
            .expect("an upstream makes this arm legal");
        assert_eq!(
            kind,
            ColdStartKind::ElFinalized,
            "the empty-archive / EL-past-epoch-0 start must anchor at reth's own finalized tag"
        );
    }

    /// The same empty archive with the EL still INSIDE epoch 0 is the ordinary
    /// sequencer→DPoS migration: anchor at the activation block.
    #[test]
    fn inside_epoch_zero_is_fresh_migration() {
        let kind =
            resolve_cold_start_kind(0, ACTIVATION, INTERVAL, ACTIVATION + 10, true).expect("fresh");
        assert_eq!(kind, ColdStartKind::FreshMigration);
    }

    /// Below activation there is no DPoS anchor to take from the EL at all.
    #[test]
    fn el_below_activation_is_fresh_migration() {
        let kind =
            resolve_cold_start_kind(0, ACTIVATION, INTERVAL, ACTIVATION - 1, true).expect("fresh");
        assert_eq!(kind, ColdStartKind::FreshMigration);
    }

    /// No upstream at all: the node would anchor at its EL-finalized tag and have
    /// nobody to climb the ladder from — refuse at startup, naming the flag.
    #[test]
    fn overshoot_with_empty_archive_and_no_upstream_is_fatal() {
        let err =
            resolve_cold_start_kind(0, ACTIVATION, INTERVAL, ACTIVATION + 500, false).unwrap_err();
        assert!(
            err.to_string().contains("--dpos.follower-upstream"),
            "{err}"
        );
    }

    #[test]
    fn archive_present_is_restart() {
        // A populated archive resumes at its finalized height regardless of upstream.
        let kind = resolve_cold_start_kind(
            ACTIVATION + 300,
            ACTIVATION,
            INTERVAL,
            ACTIVATION + 500,
            false,
        )
        .expect("restart");
        assert_eq!(kind, ColdStartKind::Restart);
    }

    #[test]
    fn boundary_exactly_one_interval_is_overshoot() {
        // cs_finalized == activation + interval is the FIRST height past epoch 0
        // (epoch 0 is [activation, activation + interval)); with no upstream it is
        // the startup refusal.
        let err = resolve_cold_start_kind(0, ACTIVATION, INTERVAL, ACTIVATION + INTERVAL, false)
            .unwrap_err();
        assert!(err.to_string().contains("past epoch 0"), "{err}");
    }

    #[test]
    fn zero_activation_is_the_unscheduled_sentinel_and_fatal() {
        let err = resolve_cold_start_kind(0, 0, INTERVAL, 0, true).unwrap_err();
        assert!(err.to_string().contains("unscheduled sentinel"), "{err}");
    }

    /// The follower march below the activation block (R-131, PLAN row 4.4). Every
    /// case below fixes ONE step of the five, in the order the arm evaluates them;
    /// the last two fix the arithmetic the smoke case reads back out of the log.
    ///
    /// [`PeerProbes`] are the two inputs that cost a peer round trip, so the
    /// peer-free steps are asserted with `PeerProbes::default()` — which is exactly
    /// how the arm calls this function on its first pass.
    ///
    /// Falsifier for the pair: a march that reaches the upstream while reth already
    /// holds the activation block (steps 2..5 firing on `holds = true`); a march that
    /// asks for a certificate before spending a configured operator checkpoint (the
    /// order К-73 forbids, because `assert_l1_checkpoint` is counted from the
    /// landing).
    #[test]
    fn holding_the_activation_block_is_a_local_entry_and_costs_no_peer() {
        // Step 1a: the probe found the block. Upstream AND checkpoint AND a servable
        // frontier are all present, and none of them is reached.
        assert_eq!(
            follower_entry(
                true,
                true,
                true,
                PeerProbes {
                    latest_height: Some(ACTIVATION + 500),
                    e_max: Some(0),
                },
                ACTIVATION,
                INTERVAL,
                0
            ),
            FollowerEntry::Local,
            "a node that holds the activation block must not contact a peer at all"
        );
        // Step 1b: reth's own finalized tag is already at/above activation — today's
        // `rf_num >= activation` path, which does not depend on the probe.
        assert_eq!(
            follower_entry(
                false,
                true,
                true,
                PeerProbes::default(),
                ACTIVATION,
                INTERVAL,
                ACTIVATION + 1
            ),
            FollowerEntry::Local,
            "an EL-finalized tag at or above activation is a local anchor by itself"
        );
    }

    /// Step 3b — nothing to try at all: no checkpoint AND no upstream. That is the
    /// honest sequencer→DPoS migration, where the block is produced on THIS chain, so
    /// there is nobody to ask and nothing to authenticate, and the node waits for reth
    /// to hold it (`wait_for_activation_block`, retry-forever, Decision A). Note what
    /// separates this input from the one above: `has_checkpoint = false`.
    ///
    /// Falsifier: a `ChainBelowActivation`/`Certificate` verdict here would send an
    /// upstream-less node into a loop that can never make a request.
    #[test]
    fn no_upstream_below_activation_waits_for_the_sequencer() {
        assert_eq!(
            follower_entry(
                false,
                false,
                false,
                PeerProbes::default(),
                ACTIVATION,
                INTERVAL,
                0
            ),
            FollowerEntry::WaitLocal
        );
    }

    /// Step 2 — AN OPERATOR CHECKPOINT WITH NO UPSTREAM AT ALL IS STILL AN ENTRY, and
    /// this is the one configuration that decides the order of the two peer-free
    /// fallbacks. `ElSync::sync_to_checkpoint` (`cold_start_jump.rs`) takes `&self`
    /// and a hash and nothing else: it FCUs reth onto the operator's block and lets
    /// devp2p backfill, so it works on a node that has no `CertUpstream` configured.
    /// Testing `has_upstream` first therefore parked — forever, in
    /// `wait_for_activation_block` — a node whose operator had already handed it a
    /// working entry.
    ///
    /// RED BEFORE THIS CHANGE, verbatim: with the two predicates in the order the
    /// design shipped them (`if !has_upstream { WaitLocal }` above
    /// `if has_checkpoint { Checkpoint }`) this input answers
    /// `FollowerEntry::WaitLocal`, and this assertion fails with
    /// `left: WaitLocal / right: Checkpoint`.
    ///
    /// Falsifier: any verdict but `Checkpoint` on this input — `WaitLocal` is the
    /// permanent park, and the two retry verdicts cannot even be reached without an
    /// upstream to ask.
    #[test]
    fn a_checkpoint_without_any_upstream_is_still_an_entry() {
        assert_eq!(
            follower_entry(
                false,
                false,
                true,
                PeerProbes::default(),
                ACTIVATION,
                INTERVAL,
                0
            ),
            FollowerEntry::Checkpoint,
            "an operator checkpoint needs no cert upstream — `sync_to_checkpoint` drives reth by \
             itself, so gating it behind one parks a node that has an entry"
        );
    }

    /// Step 3 — the operator checkpoint goes BEFORE the certificate entry, and the
    /// order is the point: `assert_l1_checkpoint` (`dpos.rs`, after the match) is
    /// counted FROM THE LANDING, and the certificate entry lands on the lowest legal
    /// height, so a certificate-first march turns a survivable park into a fatal
    /// refusal (К-73).
    ///
    /// Falsifier: a `Certificate` verdict on this input — that is the ordering bug.
    #[test]
    fn a_configured_checkpoint_is_spent_before_the_certificate_entry() {
        assert_eq!(
            follower_entry(
                false,
                true,
                true,
                PeerProbes {
                    latest_height: Some(ACTIVATION + 500),
                    e_max: Some(0),
                },
                ACTIVATION,
                INTERVAL,
                0
            ),
            FollowerEntry::Checkpoint,
            "a servable certificate frontier must not pre-empt the operator checkpoint"
        );
    }

    /// Step 4 — the ordering chain is not an entry yet. Three shapes, all "ask
    /// again": the upstream serves no frontier at all, its frontier is below
    /// `activation + K`, and the boundary `activation + K − 1` (the highest height
    /// whose certificate still carries no real EVM hash —
    /// `order_block::result_target` answers `PreActivation` below `anchor + K`).
    ///
    /// Falsifier: a `Certificate { target }` at or below `activation + K − 1` would
    /// EL-sync toward `B256::ZERO`.
    #[test]
    fn a_frontier_below_activation_plus_k_is_not_an_entry() {
        for latest in [None, Some(0), Some(ACTIVATION + K - 1)] {
            assert_eq!(
                follower_entry(
                    false,
                    true,
                    false,
                    PeerProbes {
                        latest_height: latest,
                        e_max: Some(0),
                    },
                    ACTIVATION,
                    INTERVAL,
                    0
                ),
                FollowerEntry::ChainBelowActivation,
                "latest = {latest:?} must not become a certificate target"
            );
        }
        // And the first height that IS an entry, to show the boundary is a boundary.
        assert_eq!(
            follower_entry(
                false,
                true,
                false,
                PeerProbes {
                    latest_height: Some(ACTIVATION + K),
                    e_max: Some(0),
                },
                ACTIVATION,
                INTERVAL,
                0
            ),
            FollowerEntry::Certificate {
                target: ACTIVATION + K
            },
            "the certificate at exactly activation + K carries the activation block's own result"
        );
    }

    /// Step 4, second shape — no epoch committee reads at `rf_hash` yet: the window
    /// where `setDposActivationBlock` has run and `commitEpochCommittee(0)` has not.
    /// Nothing can authenticate a finalization under a committee that is not there,
    /// so the march waits instead of asking (A1b).
    ///
    /// Falsifier: a `Certificate` verdict with `e_max = None` — the fetch would then
    /// always fail authentication, and the park would be reported as a refusal.
    #[test]
    fn an_unreadable_committee_window_is_not_an_entry() {
        assert_eq!(
            follower_entry(
                false,
                true,
                false,
                PeerProbes {
                    latest_height: Some(ACTIVATION + 500),
                    e_max: None,
                },
                ACTIVATION,
                INTERVAL,
                0
            ),
            FollowerEntry::ChainBelowActivation
        );
    }

    /// Step 5 — the certificate target, on the three shapes that matter. The target
    /// is the HIGHEST height this node can still check: `min(tip, last(e_max))`,
    /// floored at `activation + K`, where
    /// `last(e) = activation + (e + 1) · interval − 1`. One request per attempt,
    /// aimed at the height a cascading donor's `JUMP_THRESHOLD` window and a jumped
    /// validator's archive lose LAST.
    ///
    /// Falsifier: a target above `last(e_max)` (the committee needed to authenticate
    /// it is not readable, so the fetch could never succeed); a target above the tip
    /// (nobody holds it); a target below `activation + K` (no real EVM hash).
    #[test]
    fn the_certificate_target_is_the_top_of_the_checkable_window() {
        // DEVNET: one readable epoch, tip well past it. `last(0) = 95`, and the
        // landing the smoke case reads in the log is `95 − K = 92`.
        assert_eq!(
            follower_entry(
                false,
                true,
                false,
                PeerProbes {
                    latest_height: Some(200),
                    e_max: Some(0),
                },
                DEVNET_ACTIVATION,
                DEVNET_INTERVAL,
                0
            ),
            FollowerEntry::Certificate { target: 95 },
            "devnet: the target is last(0) = activation + interval − 1"
        );
        // The tip caps the window when the chain is younger than it.
        assert_eq!(
            follower_entry(
                false,
                true,
                false,
                PeerProbes {
                    latest_height: Some(80),
                    e_max: Some(0),
                },
                DEVNET_ACTIVATION,
                DEVNET_INTERVAL,
                0
            ),
            FollowerEntry::Certificate { target: 80 },
            "a tip inside epoch 0 is itself the top of the checkable window"
        );
        // MINIMUM: the tip is exactly the first checkable height.
        assert_eq!(
            follower_entry(
                false,
                true,
                false,
                PeerProbes {
                    latest_height: Some(DEVNET_ACTIVATION + K),
                    e_max: Some(0),
                },
                DEVNET_ACTIVATION,
                DEVNET_INTERVAL,
                0
            ),
            FollowerEntry::Certificate {
                target: DEVNET_ACTIVATION + K
            },
            "the floor is activation + K, and it is reachable"
        );
        // PROD: a pre-activation block reads the whole lookahead window
        // (`MAX_COMMITTEE_LOOKAHEAD_EPOCHS = 2`), so the top is `last(2)`.
        assert_eq!(
            follower_entry(
                false,
                true,
                false,
                PeerProbes {
                    latest_height: Some(ACTIVATION + 10_000),
                    e_max: Some(2),
                },
                ACTIVATION,
                INTERVAL,
                0
            ),
            FollowerEntry::Certificate {
                target: ACTIVATION + 3 * INTERVAL - 1
            },
            "prod: the target is last(2), never the tip the upstream claims"
        );
    }

    /// A READABLE WINDOW THAT ENDS BELOW THE FLOOR IS NOT AN ENTRY. `interval <= K`
    /// puts `last(e_max)` under `activation + K`, so there is no height that is both
    /// committee-checkable at `rf_hash` and carries a real EVM hash — the same state
    /// as "the chain has not reached the entry yet", and the same verdict.
    ///
    /// RED ON THE FORM THIS REPLACED, which clamped the target UP
    /// (`min(latest, last(e_max)).max(floor)`): it returned
    /// `Certificate { target: activation + K }`, a height inside an epoch ABOVE
    /// `e_max` whose committee the node cannot read, so `fetch_verified_entry`
    /// could never authenticate it and the node parked forever on the upstream's
    /// failure text for a fault of the geometry (R-131 review, D-03). Nothing in the
    /// suite pinned the clamp — deleting it left all fifteen tests green.
    ///
    /// Falsifier: any `Certificate` verdict here; a `ChainBelowActivation` on the
    /// line below, where the window DOES reach the floor and the entry exists.
    #[test]
    fn a_readable_window_below_the_floor_is_not_an_entry() {
        // `interval = 1`, `e_max = 0` ⇒ `last(0) = activation`, floor = activation + 3.
        for e_max in 0..=2 {
            assert_eq!(
                follower_entry(
                    false,
                    true,
                    false,
                    PeerProbes {
                        latest_height: Some(DEVNET_ACTIVATION + 10_000),
                        e_max: Some(e_max),
                    },
                    DEVNET_ACTIVATION,
                    1,
                    0
                ),
                FollowerEntry::ChainBelowActivation,
                "interval 1: last({e_max}) = activation + {e_max} is under the floor, so the \
                 window holds no checkable height — asking for one is worse than waiting"
            );
        }
        // The boundary in the other direction: `(e_max + 1) · interval == K + 1` is
        // the first geometry whose window reaches the floor, and there the entry IS
        // the floor. Without this half the assertion above would also pass on a
        // function that never returns `Certificate` at all.
        assert_eq!(
            follower_entry(
                false,
                true,
                false,
                PeerProbes {
                    latest_height: Some(DEVNET_ACTIVATION + 10_000),
                    e_max: Some(0),
                },
                DEVNET_ACTIVATION,
                K + 1,
                0
            ),
            FollowerEntry::Certificate {
                target: DEVNET_ACTIVATION + K
            },
            "interval K + 1: last(0) is exactly the floor, which is a legal entry"
        );
    }
}

#[cfg(test)]
mod broker_repromote_tests {
    use commonware_cryptography::{ed25519::PrivateKey, Signer as _};
    use commonware_p2p::{
        simulated::{Config as SimConfig, Link, Network},
        utils::mux::Muxer,
        Manager as _, Receiver as _, Recipients, Sender as _,
    };
    use commonware_runtime::{deterministic, IoBuf, Metrics as _, Quota, Runner as _};
    use commonware_utils::{ordered::Set, NZUsize};
    use std::{num::NonZeroU32, sync::Arc, time::Duration};
    use tokio::sync::Mutex;

    const SUBCHANNEL: u64 = 0;
    const QUOTA: Quota = Quota::per_second(NonZeroU32::MAX);

    /// A demoted engine drops its `SubReceiver`s; a re-promoted engine CLONES the
    /// plane's `Arc<Mutex<MuxHandle>>` (the `PlaneMux` sharing) and re-registers the
    /// SAME subchannel against the SAME persistent broker — no network rebuild. This
    /// is the restart-free re-promotion property the broker-in-plane refactor adds.
    #[test]
    fn plane_mux_supports_drop_then_reclone_reregister() {
        let executor = deterministic::Runner::default();
        executor.start(|ctx| async move {
            let (network, oracle) = Network::<_, fluentbase_bls::PeerPubkey>::new(
                ctx.with_label("net"),
                SimConfig {
                    max_size: 1024 * 1024,
                    disconnect_on_block: false,
                    tracked_peer_sets: NZUsize!(1),
                },
            );
            network.start();

            let (pk1, pk2) = (
                PrivateKey::from_seed(1).public_key(),
                PrivateKey::from_seed(2).public_key(),
            );
            oracle
                .manager()
                .track(0, Set::from_iter_dedup([pk1.clone(), pk2.clone()]))
                .await;
            for (a, b) in [(pk1.clone(), pk2.clone()), (pk2.clone(), pk1.clone())] {
                oracle
                    .add_link(
                        a,
                        b,
                        Link {
                            latency: Duration::from_millis(0),
                            jitter: Duration::from_millis(0),
                            success_rate: 1.0,
                        },
                    )
                    .await
                    .unwrap();
            }

            // Plane-side broker over peer 1; the plane shares the registrar behind
            // Arc<Mutex> (PlaneMux).
            let (s1, r1) = oracle
                .control(pk1.clone())
                .register(7, QUOTA)
                .await
                .unwrap();
            let (mux1, handle1) = Muxer::new(ctx.with_label("mux1"), s1, r1, 8);
            mux1.start();
            let plane_mux = Arc::new(Mutex::new(handle1));

            // Sender side over peer 2.
            let (s2, r2) = oracle
                .control(pk2.clone())
                .register(7, QUOTA)
                .await
                .unwrap();
            let (mux2, mut handle2) = Muxer::new(ctx.with_label("mux2"), s2, r2, 8);
            mux2.start();
            let (mut tx2, _rx2) = handle2.register(SUBCHANNEL).await.unwrap();

            // Two promotions: each clones the SAME plane_mux, registers SUBCHANNEL,
            // receives, then drops its SubReceiver (auto-deregister) at scope exit.
            for payload in [b"a".as_ref(), b"b".as_ref()] {
                let p = plane_mux.clone();
                let (_sub_tx, mut sub_rx) = p.lock().await.register(SUBCHANNEL).await.unwrap();
                tx2.send(Recipients::One(pk1.clone()), IoBuf::from(payload), false)
                    .await
                    .unwrap();
                let (from, _) = sub_rx.recv().await.unwrap();
                assert_eq!(from, pk2);
            }
        });
    }
}

#[cfg(test)]
mod visibility_retry_tests {
    use super::*;
    use crate::{application::ParentHeaderMissing, digest::Digest, order_block::OrderBlock};
    use alloy_consensus::{Block as AlloyBlock, BlockBody};
    use alloy_primitives::Bytes;
    use commonware_runtime::{deterministic, Runner as _};
    use reth_ethereum_primitives::TransactionSigned;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn sample_order() -> OrderBlock {
        OrderBlock {
            parent: Digest(B256::ZERO),
            height: 7,
            proposal_view: 0,
            timestamp: 7,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result: B256::ZERO,
            txs: Vec::new(),
            equivocation: None,
        }
    }

    /// Fails the first `transient_failures` calls with [`ParentHeaderMissing`]
    /// (or `fatal` instead, when set), then succeeds.
    struct FlakyDeriver {
        calls: AtomicU32,
        transient_failures: u32,
        fatal: bool,
    }

    impl DerivedBlockBuilder for FlakyDeriver {
        type Derived = SealedBlock<RethBlock>;

        async fn derive_and_execute(
            &self,
            order: OrderBlock,
            parent_evm_hash: B256,
            _seed: Option<crate::beacon::Seed>,
        ) -> eyre::Result<SealedBlock<RethBlock>> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n < self.transient_failures {
                if self.fatal {
                    return Err(eyre!("disk exploded"));
                }
                return Err(ParentHeaderMissing(parent_evm_hash).into());
            }
            let header = Header {
                parent_hash: parent_evm_hash,
                number: order.height,
                ..Default::default()
            };
            let body: BlockBody<TransactionSigned> = BlockBody::default();
            Ok(SealedBlock::seal_slow(RethBlock::from(AlloyBlock::new(
                header, body,
            ))))
        }
    }

    #[test]
    fn transient_parent_miss_is_retried_until_visible() {
        deterministic::Runner::default().start(|ctx| async move {
            let deriver = FlakyDeriver {
                calls: AtomicU32::new(0),
                transient_failures: 3,
                fatal: false,
            };
            let derived =
                derive_with_visibility_retry(&ctx, &deriver, &sample_order(), B256::ZERO, None)
                    .await
                    .expect("recovers once the parent becomes visible");
            assert_eq!(derived.number(), 7);
            assert_eq!(deriver.calls.load(Ordering::SeqCst), 4);
        });
    }

    #[test]
    fn other_derivation_errors_stay_immediately_fatal() {
        deterministic::Runner::default().start(|ctx| async move {
            let deriver = FlakyDeriver {
                calls: AtomicU32::new(0),
                transient_failures: 1,
                fatal: true,
            };
            let err =
                derive_with_visibility_retry(&ctx, &deriver, &sample_order(), B256::ZERO, None)
                    .await
                    .unwrap_err();
            assert!(err.to_string().contains("disk exploded"), "{err}");
            assert_eq!(
                deriver.calls.load(Ordering::SeqCst),
                1,
                "no retry on fatal errors"
            );
        });
    }

    #[test]
    fn persistent_parent_miss_fails_after_deadline() {
        deterministic::Runner::default().start(|ctx| async move {
            let deriver = FlakyDeriver {
                calls: AtomicU32::new(0),
                transient_failures: u32::MAX,
                fatal: false,
            };
            let err =
                derive_with_visibility_retry(&ctx, &deriver, &sample_order(), B256::ZERO, None)
                    .await
                    .unwrap_err();
            assert!(err.downcast_ref::<ParentHeaderMissing>().is_some(), "{err}");
            assert!(
                deriver.calls.load(Ordering::SeqCst) > 1,
                "must have retried before giving up"
            );
        });
    }
}

#[cfg(test)]
mod self_heal_tests {
    use super::{read_with_visibility_belt, wait_for_activation_block, SyncMetrics, SyncReason};
    use alloy_primitives::{BlockNumber, B256};
    use commonware_runtime::{deterministic, Runner as _};
    use reth_storage_api::{errors::provider::ProviderResult, BlockHashReader};
    use std::sync::atomic::{AtomicU32, Ordering};

    // #17 visibility belt: a landing read that returns `None` a few times then
    // materializes must resolve (not fatal), raise `landing_wait` while waiting,
    // and clear it on success.
    #[test]
    fn belt_retries_then_resolves_and_clears_the_gauge() {
        deterministic::Runner::default().start(|ctx| async move {
            let m = SyncMetrics::default();
            let calls = AtomicU32::new(0);
            let hash = B256::repeat_byte(0x11);
            let got = read_with_visibility_belt(&ctx, &m, "the landing", || {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                if n >= 1 {
                    // The first miss must already have raised the gauge.
                    assert_eq!(m.degraded_value(SyncReason::LandingWait), 1);
                }
                Ok(if n < 2 { None } else { Some(hash) })
            })
            .await
            .expect("belt resolves once the block materializes");
            assert_eq!(got, hash);
            assert!(
                calls.load(Ordering::SeqCst) >= 3,
                "retried before resolving"
            );
            assert_eq!(
                m.degraded_value(SyncReason::LandingWait),
                0,
                "cleared on success"
            );
        });
    }

    // #17 belt: a genuinely materialized-but-missing read stays fatal AFTER the
    // belt expires (not retry-forever — a landing local fault, not correlated).
    #[test]
    fn belt_expiry_is_fatal_and_clears_the_gauge() {
        deterministic::Runner::default().start(|ctx| async move {
            let m = SyncMetrics::default();
            let err = read_with_visibility_belt(&ctx, &m, "the landing", || {
                Ok::<_, eyre::Report>(None::<B256>)
            })
            .await
            .expect_err("a never-materializing read is fatal past the belt");
            assert!(err.to_string().contains("visibility belt"), "{err}");
            assert_eq!(
                m.degraded_value(SyncReason::LandingWait),
                0,
                "cleared on expiry"
            );
        });
    }

    /// Returns `None` for the first `misses` reads, then `present`.
    struct FlakyHashReader {
        calls: AtomicU32,
        misses: u32,
        present: B256,
        metrics: SyncMetrics,
    }

    impl BlockHashReader for FlakyHashReader {
        fn block_hash(&self, _number: BlockNumber) -> ProviderResult<Option<B256>> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n >= 1 && n < self.misses {
                // Every wait iteration keeps `activation_wait` raised.
                assert_eq!(self.metrics.degraded_value(SyncReason::ActivationWait), 1);
            }
            Ok(if n < self.misses {
                None
            } else {
                Some(self.present)
            })
        }
        fn canonical_hashes_range(
            &self,
            _start: BlockNumber,
            _end: BlockNumber,
        ) -> ProviderResult<Vec<B256>> {
            Ok(Vec::new())
        }
    }

    // #13 activation-wait: a transient absence RETRIES (the old code fatal-ed after
    // a 120 s deadline; the deadline is now gone — retry-forever) and resolves when
    // the sequencer's block lands, raising `activation_wait` while it waits and
    // clearing it on success. Never fatal on a mere transient absence.
    #[test]
    fn activation_wait_retries_then_resolves() {
        deterministic::Runner::default().start(|ctx| async move {
            let m = SyncMetrics::default();
            let present = B256::repeat_byte(0xA7);
            let provider = FlakyHashReader {
                calls: AtomicU32::new(0),
                misses: 4,
                present,
                metrics: m.clone(),
            };
            let got = wait_for_activation_block(&ctx, &provider, 192, &m)
                .await
                .expect("activation wait resolves, never fatal");
            assert_eq!(got, present);
            assert!(
                provider.calls.load(Ordering::SeqCst) >= 5,
                "retried on each transient miss instead of failing fast"
            );
            assert_eq!(
                m.degraded_value(SyncReason::ActivationWait),
                0,
                "cleared on success"
            );
        });
    }
}

#[cfg(test)]
mod crash_recover_tests {
    use super::{
        crash_recover_defer_or_fatal, recover_reconnect_point, ReconnectScan, RecoverOutcome,
        SyncMetrics, SyncReason,
    };
    use alloy_primitives::{BlockNumber, B256};
    use reth_chainspec::ChainInfo;
    use reth_storage_api::{errors::provider::ProviderResult, BlockHashReader, BlockNumReader};
    use std::collections::HashSet;

    /// reth holds exactly the heights in `present`; `best` is its canonical tip.
    struct SparseProvider {
        present: HashSet<u64>,
        best: u64,
    }

    impl BlockHashReader for SparseProvider {
        fn block_hash(&self, number: BlockNumber) -> ProviderResult<Option<B256>> {
            Ok(self
                .present
                .contains(&number)
                .then(|| B256::repeat_byte(number as u8)))
        }
        fn canonical_hashes_range(
            &self,
            _start: BlockNumber,
            _end: BlockNumber,
        ) -> ProviderResult<Vec<B256>> {
            Ok(Vec::new())
        }
    }

    impl BlockNumReader for SparseProvider {
        fn chain_info(&self) -> ProviderResult<ChainInfo> {
            Ok(ChainInfo::default())
        }
        fn best_block_number(&self) -> ProviderResult<BlockNumber> {
            Ok(self.best)
        }
        fn last_block_number(&self) -> ProviderResult<BlockNumber> {
            Ok(self.best)
        }
        fn block_number(&self, _hash: B256) -> ProviderResult<Option<BlockNumber>> {
            Ok(None)
        }
    }

    // #12: reth missing a small tail below `target` reconnects at the highest
    // present ancestor and replays it (the flush-race path — NOT a defer).
    #[test]
    fn small_gap_reconnects_for_replay() {
        // reth holds up to 998; target 1000 (999, 1000 missing) → reconnect at 999.
        let provider = SparseProvider {
            present: (0..=998).collect(),
            best: 998,
        };
        match recover_reconnect_point(&provider, 1000, 64).expect("scan ok") {
            ReconnectScan::Reconnect(lowest) => assert_eq!(lowest, 999),
            ReconnectScan::TooDeep => panic!("a 2-block tail must NOT be TooDeep"),
        }
    }

    // #12: reth missing MORE than MAX_COLD_RECOVER below its own archive is TooDeep —
    // the pre-engine replay can't bridge it; the recover fn defers to devp2p EL-sync.
    #[test]
    fn deep_gap_is_too_deep_for_replay() {
        // reth holds only up to 100; target 1000 → far more than 64 behind.
        let provider = SparseProvider {
            present: (0..=100).collect(),
            best: 100,
        };
        assert!(
            matches!(
                recover_reconnect_point(&provider, 1000, 64).expect("scan ok"),
                ReconnectScan::TooDeep
            ),
            "a > 64-block gap is TooDeep (#12 defer, not a flush-race replay)"
        );
    }

    // #12 disposition WITH an upstream: DEFER to EL-sync + raise the crash-recover
    // gauges/counter (the caller then anchors at reth's tip so the jump devp2p-backfills
    // the EL — the archive is INTACT, so no consensus-store repair is needed).
    #[test]
    fn defer_with_upstream_raises_gauges_and_reports_gap() {
        let provider = SparseProvider {
            present: (0..=100).collect(),
            best: 100,
        };
        let m = SyncMetrics::default();
        let outcome =
            crash_recover_defer_or_fatal(&provider, 1000, true, &m, "test gap").expect("defers");
        match outcome {
            RecoverOutcome::DeferToElSync { gap } => assert_eq!(gap, 900, "target 1000 − best 100"),
            RecoverOutcome::Recovered(_) => panic!("must defer, not recover"),
        }
        assert_eq!(
            m.degraded_value(SyncReason::CrashRecover),
            1,
            "crash_recover gauge raised"
        );
        assert_eq!(
            m.crash_recover_deferred_to_elsync.get(),
            1,
            "defer counter incremented"
        );
        assert_eq!(m.crash_recover_gap_blocks.get(), 900, "gap gauge set");
    }

    // #12 residual fatal WITHOUT an upstream: there is nowhere to devp2p-backfill from —
    // real local data loss stays FATAL (Decision A does not apply to idiosyncratic local
    // corruption), and no gauge is raised.
    #[test]
    fn defer_without_upstream_is_fatal() {
        let provider = SparseProvider {
            present: (0..=100).collect(),
            best: 100,
        };
        let m = SyncMetrics::default();
        let err = crash_recover_defer_or_fatal(&provider, 1000, false, &m, "test gap")
            .expect_err("no upstream ⇒ fatal");
        assert!(
            err.to_string().contains("--dpos.follower-upstream"),
            "{err}"
        );
        assert_eq!(
            m.degraded_value(SyncReason::CrashRecover),
            0,
            "the residual-fatal path raises no self-heal gauge"
        );
    }
}

// The σ half of the crash-survivor replay, pinned where the DECISION is made.
// The I/O tail it guards (local certificate → upstream → defer) is exercised by
// the smoke suite, as the disk-archive replay itself is; what a unit can pin —
// and what the fork hinges on — is that a MISS never reads as "no σ here".
#[cfg(test)]
mod replay_seed_tests {
    use super::{replay_seed_source, seed_via_beacon, CertSeed, ReplaySeedSource, SyncMetrics};
    use crate::beacon::Beacon;
    use crate::{
        beacon::testing::{
            ArtifactStore, LiveBeacon, LiveBeaconConfig, MintFixture, PkOracle, SeedStore,
            DETERMINISTIC_BOOTSTRAP_EPOCH,
        },
        epocher::OriginEpocher,
    };
    use commonware_consensus::types::{Epoch, Round, View};
    use commonware_cryptography::bls12381::{dkg::deal_anonymous, primitives::variant::MinSig};
    use commonware_utils::{test_rng, N3f1, NZU32};
    use fluentbase_bls::beacon::{recover_seed, seed_namespace, sign_seed_partial};
    use std::{num::NonZeroU64, sync::Arc};

    const EPOCH_LEN: u64 = 8;
    const VIEW: u64 = 5;

    fn epocher(origin: u64) -> OriginEpocher {
        OriginEpocher::new(origin, NonZeroU64::new(EPOCH_LEN).expect("8 != 0"))
    }

    /// A production provider over a store holding a real threshold σ for each of
    /// `rounds` — the same shape a rehydrated seed journal leaves behind.
    fn holding(rounds: &[Round]) -> Arc<dyn crate::beacon::Beacon> {
        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(5));
        let ns = seed_namespace(b"fluent-test");
        let seeds = SeedStore::new();
        for &round in rounds {
            let partials: Vec<_> = shares
                .iter()
                .map(|share| sign_seed_partial(share, &ns, round))
                .collect();
            let sigma = recover_seed::<N3f1>(&sharing, &partials).expect("the fixture recovers σ");
            seeds.record(PkOracle::new(*sharing.public(), ns.clone()).witness(round, sigma));
        }
        // No key index entries: this fixture is about the σ store, and the replay
        // path's claim is that a σ MISS never reads as "no σ here".
        LiveBeacon::build(LiveBeaconConfig {
            seeds,
            keys: MintFixture::new().keys.clone(),
            ceremony: Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new())),
            acquire: None,
            metrics: crate::beacon::testing::BeaconMetrics::default(),
            chain_id: 1,
            artifacts: ArtifactStore::new(),
            geometry: tokio::sync::watch::channel(Some((0, 1))).1,
        })
    }

    // THE property: on a beacon-active link a σ miss is "go find it", never "there
    // is no σ here". The second reading is the digest fallback under another name —
    // it re-rolls `prev_randao` and forks the restart away from the network.
    #[test]
    fn a_seed_miss_on_a_beacon_active_link_is_wanted_never_inactive() {
        let m = SyncMetrics::default();
        let height = EPOCH_LEN * DETERMINISTIC_BOOTSTRAP_EPOCH;
        match replay_seed_source(holding(&[]).as_ref(), &epocher(0), height, VIEW, &m) {
            ReplaySeedSource::Wanted(round) => assert_eq!(
                round,
                Round::new(Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH), View::new(VIEW)),
                "and it names the block's OWN round, from agreed data"
            ),
            ReplaySeedSource::Inactive => {
                panic!("a σ miss must not read as `no σ here` — that IS the digest fallback")
            }
            ReplaySeedSource::Held(_) => panic!("the store is empty"),
        }
        assert_eq!(
            m.crash_recover_stray_seed.get(),
            0,
            "nothing stray was seen — the counter is for the inactive arm only"
        );
    }

    /// A committee that can produce a REAL seeded finalization: `n` multisig
    /// members over one dealt threshold key, so the σ its certificates carry is a
    /// genuine threshold signature and the beacon's check of it is the shipped one.
    struct Seeded {
        signers: Vec<fluentbase_bls::Scheme>,
        verifier: fluentbase_bls::Scheme,
        outcome: crate::beacon::testing::DkgOutcome,
    }

    fn seeded_committee() -> Seeded {
        use commonware_codec::DecodeExt as _;
        use commonware_cryptography::{
            bls12381::{
                dkg::deal,
                primitives::{sharing::Mode, variant::MinSig},
            },
            ed25519::PrivateKey as Ed25519PrivateKey,
            Signer as _,
        };
        use commonware_math::algebra::Random as _;
        use commonware_utils::{ordered::BiMap, ordered::Set, TryCollect as _};
        use fluentbase_bls::{
            beacon::seed_namespace, fluent_namespace, keys::ValidatorBlsKeypair,
            oracle::SeedOracle, scheme::build_signer, scheme::build_verifier, BlsPubkey,
            PeerPubkey,
        };
        use rand_08::rngs::StdRng;
        use rand_core::SeedableRng as _;

        let mut rng = StdRng::seed_from_u64(5);
        let peers: Vec<_> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls: Vec<_> = (0..4)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect();
        let bimap: BiMap<PeerPubkey, BlsPubkey> = peers
            .iter()
            .zip(bls.iter())
            .map(|(p, b)| {
                (
                    p.public_key(),
                    BlsPubkey::decode(b.public_bytes().as_slice()).expect("bls pubkey"),
                )
            })
            .try_collect()
            .expect("unique committee");
        let players: Set<PeerPubkey> = Set::from_iter_dedup(peers.iter().map(|p| p.public_key()));
        let (outcome, share_map) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                .expect("deal");
        let seed_ns = seed_namespace(&fluent_namespace(1));
        let oracle = |share: Option<_>| {
            Arc::new(crate::beacon::testing::DealtOracle {
                sharing: outcome.public().clone(),
                share,
                namespace: seed_ns.clone(),
            }) as Arc<dyn SeedOracle>
        };
        let ns = fluent_namespace(1);
        let signers = peers
            .iter()
            .zip(bls.iter())
            .map(|(p, kp)| {
                let share = share_map.get_value(&p.public_key()).expect("share").clone();
                build_signer(
                    &ns,
                    bimap.clone(),
                    kp,
                    DETERMINISTIC_BOOTSTRAP_EPOCH,
                    Some(oracle(Some(share))),
                )
                .expect("member")
            })
            .collect();
        let verifier = build_verifier(
            &ns,
            bimap,
            DETERMINISTIC_BOOTSTRAP_EPOCH,
            Some(oracle(None)),
        );
        Seeded {
            signers,
            verifier,
            outcome,
        }
    }

    /// A 2f+1 finalization for `round` whose certificate CARRIES the round's σ.
    fn seeded_cert(
        c: &Seeded,
        round: Round,
    ) -> commonware_consensus::simplex::types::Finalization<
        fluentbase_bls::Scheme,
        crate::digest::Digest,
    > {
        use commonware_consensus::simplex::types::{Finalization, Finalize, Proposal};
        let prop = Proposal::new(
            round,
            View::new(0),
            crate::digest::Digest(alloy_primitives::B256::repeat_byte(0xcc)),
        );
        let finalizes: Vec<_> = c
            .signers
            .iter()
            .take(3)
            .map(|s| Finalize::sign(s, prop.clone()).expect("sign"))
            .collect();
        Finalization::from_finalizes(
            &c.verifier,
            finalizes.iter(),
            &commonware_parallel::Sequential,
        )
        .expect("quorum + recovered seed")
    }

    /// A provider over an EMPTY index, with `mint` optionally filed — the two
    /// states the replay walk has to tell apart.
    fn provider(mint: Option<&crate::beacon::testing::DkgOutcome>) -> Arc<LiveBeacon> {
        let mints = MintFixture::new();
        if let Some(outcome) = mint {
            mints.mint(DETERMINISTIC_BOOTSTRAP_EPOCH, outcome.clone());
        }
        LiveBeacon::build(LiveBeaconConfig {
            seeds: SeedStore::new(),
            keys: mints.keys.clone(),
            ceremony: Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new())),
            acquire: None,
            metrics: crate::beacon::testing::BeaconMetrics::default(),
            chain_id: 1,
            artifacts: mints.artifacts.clone(),
            geometry: tokio::sync::watch::channel(Some((0, 1))).1,
        })
    }

    // (E5-03) THE ARCHIVE IS NO LONGER TRUSTED FOR σ. This walk used to read σ
    // straight out of a certificate with no check, justified as "the key may
    // legitimately not be here" — and the two cases ARE distinguishable, which is
    // what the beacon's verdict says:
    //
    //   - genuine σ + key resolvable ⇒ `Held`, and it is FILED on the way past;
    //   - any σ + no key ⇒ `Pending` ⇒ the caller defers and resumes on
    //     `KeyAvailable`, instead of deriving from bytes nobody checked;
    //   - a σ that FAILS an attested key ⇒ `Absent`: the walk looks elsewhere and
    //     stops rather than forking on a corrupt or tampered record.
    //
    // RED BEFORE THE FIX, on the third assertion: the predecessor
    // (`seed_from_cert`) returned `Some(σ)` for all three. Reproduce the `Pending`
    // half with one line — `Observed::Pending => CertSeed::Absent` in
    // `seed_via_beacon`.
    #[test]
    fn the_replays_certificate_seed_is_checked_under_the_epoch_key() {
        let c = seeded_committee();
        let round = Round::new(Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH), View::new(VIEW));
        let cert = seeded_cert(&c, round);
        let sigma = cert
            .certificate
            .seed()
            .expect("a beacon-active certificate carries the round seed");

        // (1) The key is here and the σ is genuine: taken, and filed for the rest of
        // the walk.
        let keyed = provider(Some(&c.outcome));
        match seed_via_beacon(keyed.as_ref(), round, &cert) {
            CertSeed::Held(seed) => {
                assert_eq!(seed.target_round, round);
                assert_eq!(seed.signature, sigma);
            }
            CertSeed::Pending | CertSeed::Absent => panic!("a genuine σ under its own key"),
        }
        assert_eq!(
            Beacon::seed(keyed.as_ref(), round).map(|s| s.signature),
            Some(sigma),
            "the verdict FILES what it checks, so the walk needs no second check"
        );

        // (2) No key here: HELD, and the walk defers. Deriving here is the fork this
        // arm exists to prevent — a restart with an empty artifact partition is an
        // ordinary state.
        let keyless = provider(None);
        assert!(
            matches!(
                seed_via_beacon(keyless.as_ref(), round, &cert),
                CertSeed::Pending
            ),
            "no `PK_E` ⇒ defer, never a blind derive"
        );
        assert!(
            Beacon::seed(keyless.as_ref(), round).is_none(),
            "and nothing unchecked is served"
        );

        // (3) A σ that fails an ATTESTED key — a tampered or corrupt archive record.
        // The genuine σ of a NEIGHBOURING round is the forgery: a decodable curve
        // point that verifies for no round here.
        let neighbour = Round::new(
            Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH),
            View::new(VIEW + 1),
        );
        let mut tampered = seeded_cert(&c, round);
        tampered.certificate.seed = seeded_cert(&c, neighbour).certificate.seed;
        assert_ne!(
            tampered.certificate.seed, cert.certificate.seed,
            "the splice must change the σ, or this refuses nothing"
        );
        let fresh = provider(Some(&c.outcome));
        assert!(
            matches!(
                seed_via_beacon(fresh.as_ref(), round, &tampered),
                CertSeed::Absent
            ),
            "a σ refused under the epoch's attested key is not something to derive from"
        );
        assert!(
            Beacon::seed(fresh.as_ref(), round).is_none(),
            "and it is not filed either"
        );

        // (4) The round PIN, unchanged: a certificate for another round carries a
        // perfectly valid signature over something else.
        let other = provider(Some(&c.outcome));
        assert!(
            matches!(
                seed_via_beacon(other.as_ref(), round, &seeded_cert(&c, neighbour)),
                CertSeed::Absent
            ),
            "σ signs the round, so a neighbour's certificate answers nothing here"
        );
    }

    // The ordinary case, and the one the whole re-key exists for: σ is found under
    // the block's own round, with no child block read.
    #[test]
    fn a_held_seed_resolves_at_the_blocks_own_round() {
        let round = Round::new(Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH), View::new(VIEW));
        match replay_seed_source(
            holding(&[round]).as_ref(),
            &epocher(0),
            EPOCH_LEN * DETERMINISTIC_BOOTSTRAP_EPOCH,
            VIEW,
            &SyncMetrics::default(),
        ) {
            ReplaySeedSource::Held(seed) => assert_eq!(seed.target_round, round),
            ReplaySeedSource::Wanted(_) | ReplaySeedSource::Inactive => {
                panic!("the store holds σ for exactly this round")
            }
        }
    }

    // PREDICATE FIRST: a provider that HAS σ for the round is still refused, because
    // the agreed epoch map says the beacon is not active there and the rest of the
    // network derives `None`. Counted, not obeyed and not fatal.
    #[test]
    fn a_stray_seed_at_a_beacon_inactive_round_is_ignored_and_counted() {
        let m = SyncMetrics::default();
        let inactive = Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH - 1);
        let round = Round::new(inactive, View::new(VIEW));
        assert!(matches!(
            replay_seed_source(
                holding(&[round]).as_ref(),
                &epocher(0),
                EPOCH_LEN * inactive.get(),
                VIEW,
                &m,
            ),
            ReplaySeedSource::Inactive
        ));
        assert_eq!(m.crash_recover_stray_seed.get(), 1);
    }

    // An epoch the map cannot name at all (below the epocher origin) is INACTIVE and
    // is never unwrapped — the beacon cannot have been mandatory in an epoch that
    // does not exist.
    #[test]
    fn a_height_below_the_epocher_origin_is_inactive_not_a_panic() {
        assert!(matches!(
            replay_seed_source(
                holding(&[]).as_ref(),
                &epocher(1_000),
                10,
                VIEW,
                &SyncMetrics::default(),
            ),
            ReplaySeedSource::Inactive
        ));
    }
}

// #8: the below-floor archive-hole heal is NOT a defer — with an upstream it
// RE-POPULATES the hole by a BLS-verified by-height re-fetch. These pin
// `refetch_verified_archive_hole` in isolation (the disk-archive replay it splices
// into is exercised by the smoke suite): a valid cert heals, a no-upstream / gone-
// everywhere / forged cert is fatal.
#[cfg(test)]
mod refetch_hole_tests {
    use super::refetch_verified_archive_hole;
    use crate::{
        cert_follow::{CertUpstream, UpstreamFinalized},
        cert_inlet::CommitteeSource,
        digest::Digest,
        order_block::OrderBlock,
    };
    use alloy_primitives::{Bytes, B256};
    use commonware_consensus::{
        simplex::types::{Finalization, Finalize, Proposal},
        types::{Epoch, Height, Round, View},
    };
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
    use commonware_math::algebra::Random as _;
    use commonware_parallel::Sequential;
    use commonware_runtime::{deterministic, Runner as _};
    use commonware_utils::{ordered::BiMap, TryCollect as _};
    use fluentbase_bls::{
        fluent_namespace, keys::ValidatorBlsKeypair, oracle::SeedOracle, scheme::build_signer,
        scheme::build_verifier, BlsPubkey, PeerPubkey, Scheme as BlsScheme,
    };
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    const CHAIN_ID: u64 = 20_994;
    const COMMITTEE_N: usize = 4;

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
                use commonware_codec::DecodeExt as _;
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
            .map(|kp| build_signer(&ns, bimap.clone(), kp, 0, None).expect("member"))
            .collect();
        let verifier = build_verifier(&ns, bimap, 0, None);
        Committee { signers, verifier }
    }

    fn sample_order(height: u64) -> OrderBlock {
        OrderBlock {
            parent: Digest(B256::repeat_byte(0xaa)),
            height,
            proposal_view: 0,
            timestamp: 1_700_000_000 + height,
            gas_limit: 30_000_000,
            extra_data: Bytes::new(),
            result: B256::ZERO,
            txs: Vec::new(),
            equivocation: None,
        }
    }

    /// A real 2f+1 finalization cert over `block`'s digest, signed by `c`.
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

    /// THE TWO ANSWERS ARE INDEPENDENT FIELDS, because conflating them is the defect
    /// under test: `height` is what the by-height pull serves, `latest` is what
    /// `get_latest` serves — the only positive proof that an upstream answered us at
    /// all. A fake that derived one from the other could not express "reachable, and
    /// it does not hold the height" apart from "nothing answered", which is exactly
    /// the pair `refetch_verified_archive_hole` has to separate.
    #[derive(Clone)]
    struct FakeUpstream {
        height: Option<UpstreamFinalized>,
        latest: Option<UpstreamFinalized>,
    }
    impl FakeUpstream {
        /// Serves the height (and is therefore reachable).
        fn serving(uf: UpstreamFinalized) -> Self {
            Self {
                height: Some(uf.clone()),
                latest: Some(uf),
            }
        }
        /// ANSWERS, and says it does not hold the height — the only shape that is
        /// evidence about the record, and the only one that may exit fatal.
        fn reachable_but_missing(latest: UpstreamFinalized) -> Self {
            Self {
                height: None,
                latest: Some(latest),
            }
        }
        /// Nothing answers at all. Evidence about the link, about nothing else.
        fn unreachable() -> Self {
            Self {
                height: None,
                latest: None,
            }
        }
    }
    impl CertUpstream for FakeUpstream {
        async fn get_finalization(&self, _height: Height) -> Option<UpstreamFinalized> {
            self.height.clone()
        }
        async fn get_latest(&self) -> Option<UpstreamFinalized> {
            self.latest.clone()
        }
        async fn rotate(&self) {}
    }

    /// Unreachable for the first `silent_laps` by-height pulls, then normal — the
    /// operator's upstream that is simply not up yet when the node crash-recovers.
    /// Records the `crash_recover` gauge as seen ON THE SECOND LAP, so the test can
    /// assert the park was actually visible and not merely survived.
    #[derive(Clone)]
    struct FlakyUpstream {
        served: UpstreamFinalized,
        silent_laps: usize,
        laps: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        metrics: crate::sync_metrics::SyncMetrics,
        gauge_on_second_lap: std::sync::Arc<std::sync::atomic::AtomicI64>,
    }
    impl CertUpstream for FlakyUpstream {
        async fn get_finalization(&self, _height: Height) -> Option<UpstreamFinalized> {
            use std::sync::atomic::Ordering::SeqCst;
            let lap = self.laps.fetch_add(1, SeqCst) + 1;
            if lap == 2 {
                self.gauge_on_second_lap.store(
                    self.metrics
                        .degraded_value(crate::sync_metrics::SyncReason::CrashRecover),
                    SeqCst,
                );
            }
            (lap > self.silent_laps).then(|| self.served.clone())
        }
        async fn get_latest(&self) -> Option<UpstreamFinalized> {
            use std::sync::atomic::Ordering::SeqCst;
            // An upstream nobody can reach answers neither pull: the reachability
            // witness must not be luckier than the pull it corroborates.
            (self.laps.load(SeqCst) > self.silent_laps).then(|| self.served.clone())
        }
        async fn rotate(&self) {}
    }

    /// Hands out a fixed verifier for every epoch (the trust anchor under test).
    struct CannedCommittees(BlsScheme);
    impl CommitteeSource for CannedCommittees {
        fn scheme_at(
            &self,
            _epoch: u64,
            _at_hash: B256,
            _oracle: Option<std::sync::Arc<dyn SeedOracle>>,
        ) -> eyre::Result<BlsScheme> {
            Ok(self.0.clone())
        }
    }

    // WITH an upstream serving a committee-signed cert, a below-floor hole is
    // RE-POPULATED (returns the verified block+cert to splice into the replay),
    // never deferred.
    #[test]
    fn valid_upstream_cert_repopulates_the_hole() {
        deterministic::Runner::default().start(|mut ctx| async move {
            let c = committee(1);
            let block = sample_order(65);
            let uf = certify(&c, 0, &block);
            let up = FakeUpstream::serving(uf);
            let committees = CannedCommittees(c.verifier.clone());
            let out = refetch_verified_archive_hole(
                Some(&up),
                &committees,
                &mut ctx,
                B256::repeat_byte(1),
                65,
                "finalized_blocks",
            )
            .await
            .expect("valid cert heals the hole")
            .expect("a served height is a VERDICT, not a `keep asking`");
            assert_eq!(
                out.block.height, 65,
                "the re-fetched block is returned for replay"
            );
        });
    }

    // WITHOUT an upstream: a below-floor hole is unrecoverable local data loss → FATAL.
    #[test]
    fn no_upstream_is_fatal() {
        deterministic::Runner::default().start(|mut ctx| async move {
            let c = committee(2);
            let committees = CannedCommittees(c.verifier);
            let err = refetch_verified_archive_hole::<FakeUpstream, _>(
                None,
                &committees,
                &mut ctx,
                B256::repeat_byte(1),
                65,
                "finalized_blocks",
            )
            .await
            .err()
            .expect("no upstream ⇒ fatal");
            assert!(
                err.to_string().contains("--dpos.follower-upstream"),
                "{err}"
            );
        });
    }

    // Upstream reachable but no longer serves the below-floor height (pruned
    // everywhere) → FATAL (gone-everywhere).
    //
    // THE WORLD IS NOW EXPLICITLY REACHABLE, and that is the point of the change: the
    // fake used to answer `None` to BOTH the by-height pull and `get_latest`, i.e. it
    // was simultaneously "pruned everywhere" and "nobody home", and the fatal fired on
    // the pair. Only the first of the two licenses this sentence (R-131 review,
    // `4.4а-Д-9`), so the world has to say which one it is.
    #[test]
    fn upstream_missing_height_is_fatal() {
        deterministic::Runner::default().start(|mut ctx| async move {
            let c = committee(3);
            let up = FakeUpstream::reachable_but_missing(certify(&c, 0, &sample_order(1)));
            let committees = CannedCommittees(c.verifier);
            let err = refetch_verified_archive_hole(
                Some(&up),
                &committees,
                &mut ctx,
                B256::repeat_byte(1),
                65,
                "finalized_blocks",
            )
            .await
            .err()
            .expect("upstream lacks the height ⇒ fatal");
            assert!(err.to_string().contains("gone everywhere"), "{err}");
        });
    }

    /// AN UNREACHABLE UPSTREAM IS NOT A DATA-LOSS VERDICT. Same `None` from the
    /// by-height pull as the test above, same height, same committee — and the answer
    /// must be the opposite one, because the two negatives are different facts.
    ///
    /// The stake is irreversible: the sentence this must NOT produce tells the
    /// operator to re-sync the EL disk from a snapshot. Before the entry march made a
    /// disconnected WS actor answer its mailbox, the pull simply never returned here,
    /// so a negative was necessarily "servers answered and none holds it"; the fix for
    /// that hang is what made this case reachable, and this is the test that keeps the
    /// two apart (R-131 review, `4.4а-Д-9`).
    ///
    /// RED on any form where both cases answer the same — delete the `get_latest`
    /// witness and this fails with the gone-everywhere error it must not produce,
    /// while `upstream_missing_height_is_fatal` stays green. That pair is the whole
    /// discrimination.
    #[test]
    fn an_unreachable_upstream_is_not_a_data_loss_verdict() {
        deterministic::Runner::default().start(|mut ctx| async move {
            let c = committee(6);
            let up = FakeUpstream::unreachable();
            let committees = CannedCommittees(c.verifier);
            let out = refetch_verified_archive_hole(
                Some(&up),
                &committees,
                &mut ctx,
                B256::repeat_byte(1),
                65,
                "finalized_blocks",
            )
            .await
            .expect("nothing answered, so there is nothing to be fatal ABOUT");
            assert!(
                out.is_none(),
                "no verdict: `Ok(None)` is what makes the caller ask again instead of \
                 telling the operator to wipe the EL disk"
            );
        });
    }

    /// …and the block path ASKS AGAIN until a verdict exists, healing the hole the
    /// moment an upstream comes up. The upstream is silent for two laps and then
    /// serves the record, which is the crash-recovery boot race: the node restarts
    /// before the validator it pulls from is listening.
    ///
    /// Two things are asserted beyond the happy end, and both are the point: the lap
    /// count proves it RETRIED rather than concluded, and the gauge read taken by the
    /// fake ON THE SECOND LAP proves the park was OBSERVABLE while it waited — a
    /// silent wait would be the other half of the same defect.
    ///
    /// RED on the form where both negatives answer alike: the first lap exits with
    /// "gone everywhere" and no second lap happens.
    #[test]
    fn a_hole_waits_for_an_upstream_instead_of_declaring_data_loss() {
        deterministic::Runner::default().start(|mut ctx| async move {
            use std::sync::{
                atomic::{AtomicI64, AtomicUsize, Ordering::SeqCst},
                Arc,
            };
            let c = committee(7);
            let block = sample_order(65);
            let metrics = crate::sync_metrics::SyncMetrics::default();
            let laps = Arc::new(AtomicUsize::new(0));
            let gauge_on_second_lap = Arc::new(AtomicI64::new(-1));
            let up = FlakyUpstream {
                served: certify(&c, 0, &block),
                silent_laps: 2,
                laps: laps.clone(),
                metrics: metrics.clone(),
                gauge_on_second_lap: gauge_on_second_lap.clone(),
            };
            let committees = CannedCommittees(c.verifier.clone());

            let out = super::refetch_hole_until_answered(
                Some(&up),
                &committees,
                &mut ctx,
                B256::repeat_byte(1),
                65,
                "finalized_blocks",
                &metrics,
            )
            .await
            .expect("an upstream that comes up HEALS the hole; it never means data loss");

            assert_eq!(out.block.height, 65, "the record is what comes back");
            assert_eq!(
                laps.load(SeqCst),
                3,
                "two silent laps were RETRIED and the third served: a single lap means the \
                 unreachable case is being read as a verdict again"
            );
            assert_eq!(
                gauge_on_second_lap.load(SeqCst),
                1,
                "while it waited, `dpos_sync_degraded{{reason=crash_recover}}` was UP — a \
                 silent wait is the other half of this defect"
            );
            assert_eq!(
                metrics.degraded_value(crate::sync_metrics::SyncReason::CrashRecover),
                0,
                "and it is cleared on success, or the node reports a park it left"
            );
        });
    }

    // A forged cert (signed by a DIFFERENT committee than the one the trust anchor
    // reads) FAILS the BLS authentication → FATAL: the re-fetch cannot be steered by
    // a malicious upstream.
    #[test]
    fn forged_cert_fails_authentication() {
        deterministic::Runner::default().start(|mut ctx| async move {
            let signer_committee = committee(4);
            let trust_committee = committee(5); // a DIFFERENT committee
            let block = sample_order(65);
            let uf = certify(&signer_committee, 0, &block);
            let up = FakeUpstream::serving(uf);
            let committees = CannedCommittees(trust_committee.verifier);
            let err = refetch_verified_archive_hole(
                Some(&up),
                &committees,
                &mut ctx,
                B256::repeat_byte(1),
                65,
                "finalized_blocks",
            )
            .await
            .err()
            .expect("cert signed by the wrong committee ⇒ auth failure");
            // The inner cause is the BLS-verify `ensure!`; assert on the full chain.
            assert!(
                format!("{err:#}").contains("FAILED BLS verification"),
                "{err:#}"
            );
        });
    }
}

#[cfg(test)]
mod follower_boundary_tests {
    use super::{enter_finalized_epoch, FollowerBoundaryDeliver, FollowerCommitteeAt};
    use commonware_consensus::types::Epoch;
    use std::sync::{Arc, Mutex};

    const ACTIVATION: u64 = 100;
    const INTERVAL: u64 = 10;

    /// Records what reached the epoch manager, and lets a test make the committee
    /// read or the delivery fail on demand.
    #[derive(Default)]
    struct Recorder {
        delivered: Mutex<Vec<u64>>,
        committee_readable: Mutex<bool>,
        receiver_alive: Mutex<bool>,
    }

    fn seams(
        readable: bool,
        alive: bool,
    ) -> (Arc<Recorder>, FollowerCommitteeAt, FollowerBoundaryDeliver) {
        let rec = Arc::new(Recorder {
            delivered: Mutex::new(Vec::new()),
            committee_readable: Mutex::new(readable),
            receiver_alive: Mutex::new(alive),
        });
        let for_read = rec.clone();
        let committee_at: FollowerCommitteeAt =
            Arc::new(move |_epoch| *for_read.committee_readable.lock().unwrap());
        let for_deliver = rec.clone();
        let deliver: FollowerBoundaryDeliver = Arc::new(move |epoch: Epoch| {
            let rec = for_deliver.clone();
            Box::pin(async move {
                if !*rec.receiver_alive.lock().unwrap() {
                    return false;
                }
                rec.delivered.lock().unwrap().push(epoch.get());
                true
            }) as futures::future::BoxFuture<'static, bool>
        });
        (rec, committee_at, deliver)
    }

    /// `boundary_hook` fires per finalized block — ~1/s in production — while
    /// `reconcile_roles` prunes, re-registers and sweeps on each delivery. Reds if
    /// the once-per-epoch gate is dropped.
    #[tokio::test]
    async fn every_finalized_block_of_one_epoch_delivers_one_boundary() {
        let (rec, committee_at, deliver) = seams(true, true);
        let mut last = None;

        for height in [100, 103, 109, 110, 117] {
            assert!(
                enter_finalized_epoch(
                    &mut last,
                    height,
                    ACTIVATION,
                    INTERVAL,
                    &committee_at,
                    &deliver
                )
                .await
            );
        }

        assert_eq!(*rec.delivered.lock().unwrap(), vec![0, 1]);
        assert_eq!(last, Some(1));
    }

    /// `committee[E]` is committed during `E-1`, but the read runs at the
    /// EL-finalized hash, which trails the ordering-finalized height by K — so the
    /// first blocks of `E` can legitimately read back nothing. Consuming the epoch
    /// there would skip its scheme registration for the whole epoch.
    ///
    /// Reds if `last_delivered` advances on an unreadable committee.
    #[tokio::test]
    async fn an_unreadable_committee_leaves_the_epoch_for_the_next_block() {
        let (rec, committee_at, deliver) = seams(false, true);
        let mut last = None;

        assert!(
            enter_finalized_epoch(
                &mut last,
                100,
                ACTIVATION,
                INTERVAL,
                &committee_at,
                &deliver
            )
            .await
        );
        assert!(rec.delivered.lock().unwrap().is_empty());
        assert_eq!(last, None);

        *rec.committee_readable.lock().unwrap() = true;
        assert!(
            enter_finalized_epoch(
                &mut last,
                101,
                ACTIVATION,
                INTERVAL,
                &committee_at,
                &deliver
            )
            .await
        );
        assert_eq!(*rec.delivered.lock().unwrap(), vec![0]);
    }

    /// A closed boundary receiver means the epoch manager has exited. The trigger
    /// stops and the supervisor takes the node down, rather than spinning on a
    /// dead channel for every finalized block.
    ///
    /// Reds if the send result stops being propagated.
    #[tokio::test]
    async fn a_dropped_boundary_receiver_stops_the_trigger() {
        let (_rec, committee_at, deliver) = seams(true, false);
        let mut last = None;

        assert!(
            !enter_finalized_epoch(
                &mut last,
                100,
                ACTIVATION,
                INTERVAL,
                &committee_at,
                &deliver
            )
            .await
        );
        assert_eq!(last, None, "an undelivered epoch must not be consumed");
    }
}

#[cfg(test)]
mod local_tracked_epoch_tests {
    use super::local_tracked_epoch;
    use crate::committee::{testing::SchemeCommittee, Geometry};
    use std::sync::Arc;

    const ACTIVATION: u64 = 0;
    const INTERVAL: u64 = 32;

    fn probe_t(fin: u64) -> Option<u64> {
        let committee: Arc<dyn crate::committee::Committee> =
            SchemeCommittee::with_geometry(|_| None, |_| None, Geometry::new(ACTIVATION, INTERVAL));
        let cursor = crate::FinalizedCursor::default();
        cursor.advance(fin);
        local_tracked_epoch(committee, cursor)()
    }

    /// `T` on the follower is ET's rule over the module's geometry, checked at the
    /// two points where the rule differs (review B1-01).
    ///
    /// ET tracks `epoch_e + 1` when the finalized block is the LAST block of its
    /// epoch — both its cold-start arm (`epoch_transition.rs:558`) and its boundary
    /// arm (`:580`) — and `epoch_e` otherwise. A follower runs no `EpochTransition`,
    /// so this is the ONE place the two can drift; the test is what stops them.
    ///
    /// The terminal point is not an edge case here, it is THE case: a node whose
    /// execution stalled parks on an epoch terminal (that is where the ordering
    /// plane's two-epoch ceiling puts it), and a `T` that read `epoch_e` there would
    /// name a rung the node already holds — `last(T+1)` at or below its own frozen
    /// tip, which the marshal discards at the floor and which moves nothing.
    ///
    /// Falsifier: `epoch_of(fin)` at a terminal (one rung too low, the ladder never
    /// climbs); `epoch_of(fin) + 1` mid-epoch (a rung two epochs up, outside the
    /// node's own committee read window).
    #[test]
    fn the_followers_t_follows_the_epoch_transition_rule_at_a_terminal_and_mid_epoch() {
        // Mid-epoch: `fin = 100` sits inside epoch 3 (96..=127) ⇒ `T = 3`, rung
        // `last(4) = 159`.
        assert_eq!(
            probe_t(100),
            Some(3),
            "mid-epoch `T` must be `epoch_of(fin)` — ET's non-boundary arm"
        );
        // Terminal: `fin = 95 = last(2)` ⇒ the network is already in epoch 3, ET
        // tracks 3, and the rung is `last(4) = 159` — strictly above the parked
        // node's two-epoch ceiling floor of the same state.
        assert_eq!(
            probe_t(95),
            Some(3),
            "on an epoch terminal `T` must be `epoch_of(fin) + 1` — ET's boundary arm"
        );
        // ...and the two are genuinely different heights, which is the whole point.
        let geometry = Geometry::new(ACTIVATION, INTERVAL).expect("non-zero interval");
        assert_eq!(geometry.epoch_of(95), 2, "95 is the terminal of epoch 2");
        assert_eq!(geometry.last(2), 95, "…and `last(2)` names it");
    }

    /// Before the geometry freezes there is no epoch arithmetic at all, and the
    /// probe must be told so rather than handed `0`: the skipped step is counted
    /// (`no_tracked_epoch` / `no_geometry`) and the tick asks `Latest` alone.
    #[test]
    fn an_unfrozen_geometry_names_no_epoch() {
        let committee: Arc<dyn crate::committee::Committee> =
            SchemeCommittee::with_geometry(|_| None, |_| None, None);
        let cursor = crate::FinalizedCursor::default();
        cursor.advance(95);
        assert_eq!(
            local_tracked_epoch(committee, cursor)(),
            None,
            "an unfrozen geometry must not be answered with epoch 0"
        );
    }
}

#[cfg(test)]
mod gated_receiver_tests {
    use super::GatedReceiver;
    use commonware_cryptography::{ed25519::PrivateKey, Signer as _};
    use commonware_p2p::Receiver as _;
    use commonware_runtime::IoBuf;
    use commonware_utils::ordered::Set;
    use fluentbase_bls::PeerPubkey;
    use fluentbase_p2p::TrackedWindow;
    use fluentbase_staking_reader::TrackedPeers;
    use std::sync::Arc;

    /// A `commonware_p2p::Receiver` that hands back a canned queue, then reports
    /// end-of-stream. The queue is what a real channel would deliver; what the
    /// `GatedReceiver` lets through is the whole subject.
    #[derive(Debug)]
    struct CannedReceiver(std::collections::VecDeque<PeerPubkey>);

    impl commonware_p2p::Receiver for CannedReceiver {
        type Error = std::io::Error;
        type PublicKey = PeerPubkey;

        async fn recv(&mut self) -> Result<commonware_p2p::Message<PeerPubkey>, std::io::Error> {
            match self.0.pop_front() {
                Some(from) => Ok((from, IoBuf::from(b"frame".as_ref()))),
                None => Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)),
            }
        }
    }

    /// The LIVE seam of the 4.3 tier rule, tested where production runs it.
    ///
    /// `GatedReceiver` is the only thing between the network and a channel's
    /// decode, and by the time `slasher::gossip::ingest_batch` or
    /// `beacon::actor::on_message` sees a frame this has already ruled on its
    /// sender — so the tier checks inside those two are unreachable in production
    /// and their tests cannot stand in for this one (P-06/P-24).
    ///
    /// Every arm of `admits` at once, on ONE queue, so the assertion is the
    /// SURVIVING SEQUENCE rather than a per-frame boolean: a gate that dropped or
    /// admitted one frame too many would shift everything after it.
    ///
    /// Falsifier: a tombstoned, untracked or registry-tier sender reaching `recv`
    /// on a committee channel; a committee member NOT reaching it; or the
    /// registry-tier sender being refused on a channel that serves the registry.
    #[test]
    fn a_committee_channel_admits_only_members_and_a_registry_channel_admits_the_tier() {
        let member = PrivateKey::from_seed(1).public_key();
        let registry_only = PrivateKey::from_seed(2).public_key();
        let untracked = PrivateKey::from_seed(3).public_key();
        let tombstoned = PrivateKey::from_seed(4).public_key();

        // The tombstoned peer is a full committee member: the tombstone must beat
        // membership, not merely stand in for its absence.
        let peers = TrackedPeers {
            committees: vec![
                (6, Set::from_iter_dedup([member.clone()])),
                (7, Set::from_iter_dedup([tombstoned.clone()])),
            ],
            secondary: Set::from_iter_dedup([registry_only.clone()]),
        };
        let banned = tombstoned.clone();
        let window =
            TrackedWindow::default().with_tombstones(Arc::new(move |p: &PeerPubkey| *p == banned));

        // Before the first `track` the window has no MEMBERSHIP opinion, so nothing
        // is refused for being in the wrong tier — a node in cold start must not
        // silence its own plane. The tombstone is the exception, and deliberately:
        // it is an on-chain verdict, read before the set is even looked at.
        let queue = || {
            std::collections::VecDeque::from(vec![
                member.clone(),
                registry_only.clone(),
                untracked.clone(),
                tombstoned.clone(),
            ])
        };
        let drain = |mut r: GatedReceiver<CannedReceiver>| {
            futures::executor::block_on(async move {
                let mut seen = Vec::new();
                while let Ok((from, _)) = r.recv().await {
                    seen.push(from);
                }
                seen
            })
        };

        assert_eq!(
            drain(GatedReceiver::new(
                CannedReceiver(queue()),
                window.clone(),
                "beacon",
                true,
            )),
            vec![member.clone(), registry_only.clone(), untracked.clone()],
            "with no peer set registered the gate must pass everything through \
             except the tombstoned peer, whose verdict does not wait for a set"
        );

        window.record(7, &peers);

        assert_eq!(
            drain(GatedReceiver::new(
                CannedReceiver(queue()),
                window.clone(),
                "beacon",
                true,
            )),
            vec![member.clone()],
            "a committee channel must admit the member and nobody else: the \
             registry tier, the untracked peer and the tombstoned member all stop \
             before `recv` returns"
        );

        assert_eq!(
            drain(GatedReceiver::new(
                CannedReceiver(queue()),
                window.clone(),
                "registry",
                false,
            )),
            vec![member.clone(), registry_only.clone()],
            "a channel that serves the registry keeps the tier-2 sender and still \
             loses the untracked and the tombstoned"
        );
    }

    /// The mask a `Member` carries is per-EPOCH, which is what the second half of
    /// the rule (each channel's own entry) reads. The gate itself does not look at
    /// it — a member of ANY carried record passes the transport seam — so the two
    /// halves cannot be collapsed into one.
    #[test]
    fn the_gate_admits_a_member_of_any_carried_record_and_the_mask_says_which() {
        let outgoing = PrivateKey::from_seed(11).public_key();
        let incoming = PrivateKey::from_seed(12).public_key();
        let peers = TrackedPeers {
            committees: vec![
                (5, Set::from_iter_dedup([outgoing.clone()])),
                (7, Set::from_iter_dedup([incoming.clone()])),
            ],
            secondary: Set::default(),
        };
        let window = TrackedWindow::default();
        window.record(6, &peers);

        let mut gate = GatedReceiver::new(
            CannedReceiver(std::collections::VecDeque::from(vec![
                outgoing.clone(),
                incoming.clone(),
            ])),
            window.clone(),
            "beacon",
            true,
        );
        let seen = futures::executor::block_on(async {
            let mut seen = Vec::new();
            while let Ok((from, _)) = gate.recv().await {
                seen.push(from);
            }
            seen
        });
        assert_eq!(
            seen,
            vec![outgoing.clone(), incoming.clone()],
            "the transport seam admits a member of any of the three records"
        );

        // ...and the per-epoch answer, which only the channel's own entry reads, is
        // the one that separates them.
        let ingress = window.classify(&outgoing).expect("a registered window");
        assert!(ingress.member_of(5));
        assert!(
            !ingress.member_of(7),
            "the outgoing member speaks for epoch 5 only"
        );
        let ingress = window.classify(&incoming).expect("a registered window");
        assert!(ingress.member_of(7));
        assert!(
            !ingress.member_of(5),
            "the incoming member speaks for epoch 7 only"
        );
    }
}
