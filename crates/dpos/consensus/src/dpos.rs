//! DPoS layer launcher: assembles the staking reader, consensus, and p2p
//! layers given operator keys, reth handles, and config.

use crate::{
    application::{
        derive_with_visibility_retry, BeaconEngineLike, DerivedBlock as _, DerivedBlockBuilder,
        ExecutedChain, OrderingAssembler,
    },
    beacon::{Beacon, Observed, ObservedCertificate, Seed},
    cert_follow::WalkOutcome,
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

/// The single counter every channel's ingress refusal lands on, labelled by
/// channel and reason: peers are not tracked or penalized individually, and the
/// only ban is the on-chain tombstone.
pub const INGRESS_DROPPED_TOTAL: &str = "dpos_ingress_dropped_total";

pub fn record_ingress_drop(channel: &'static str, reason: &'static str) {
    metrics::counter!(INGRESS_DROPPED_TOTAL, "channel" => channel, "reason" => reason).increment(1);
}

/// A `commonware_p2p::Receiver` that refuses a frame before anything decodes it,
/// based on the peer set this node last registered.
///
/// It is the only sender check that can run before a decode: the sender is
/// either inside the tracked window or it is not, and a tombstoned sender is
/// refused regardless. Binding a sender to the frame's own epoch needs the
/// frame's epoch and so happens past the decode, in each channel's consumer.
///
/// `members_only` selects the tier the channel serves: committee channels set it
/// and thereby refuse a tier-2 registry sender; a channel that serves the
/// registry clears it and admits tracked senders.
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

/// Epoch-geometry read that tolerates a missing `ChainConfig` (or unscheduled
/// DPoS) at `at`, returning `None`; this distinguishes a genesis-baked chain
/// from a runtime-deployed one whose geometry is readable only after EL sync.
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

/// Pokes between successive "boundary still parked" warnings, the log-side
/// complement to the `parked_boundary_height` gauge: at
/// `PENDING_RETRY_BACKOFF = 200 ms`, 150 pokes ≈ 30 s.
const PARKED_BOUNDARY_WARN_EVERY: u64 = 150;

/// Partition prefix for the commonware marshal's durable storage; the cold-start
/// discriminator peek and the marshal's own `OuterBuilder.partition_prefix` must
/// match or the peek opens a different store.
const MARSHAL_PARTITION_PREFIX: &str = "consensus_marshal";

/// Reth handles needed by the DPoS layer. `transaction_pool`, `chain_spec`, and
/// `data_dir` are deliberately absent: `slasher_sink` arrives pre-built via
/// `DposLayerConfig` (so the host owns the `reth-transaction-pool` trait bounds),
/// `chain_spec` reduces to its only used field `chain_id`, and `data_dir` is set
/// host-side before `runner.start()`.
pub struct RethHandle<Provider, EvmConfig, BeaconEngine> {
    pub provider: Provider,
    pub evm_config: EvmConfig,
    pub beacon_engine_handle: BeaconEngine,
    pub chain_id: u64,
    /// Read-only probe of reth's connected devp2p peer count, built host-side.
    /// A closure rather than a typed handle keeps the consensus crate free of a
    /// `reth-network-api` dependency.
    pub peer_count: Arc<dyn Fn() -> usize + Send + Sync>,
    /// Disk-loaded canonical state snapshot; on a graceful-shutdown restart
    /// `get_finalized_num_hash()` returns the disk finalized. A struct field
    /// rather than a trait method because `canonical_in_memory_state()` is a
    /// concrete inherent on `BlockchainProvider<N>`, not exposed via any reth
    /// provider trait.
    pub canonical_state: reth_chain_state::CanonicalInMemoryState<EthPrimitives>,
    /// Pristine-network fallback for when `get_finalized_num_hash()` returns `None`.
    pub genesis_hash: B256,
}

/// Cold-start `(finalized_num, finalized_hash, head_num, head_hash)` from reth's
/// `canonical_state` plus `genesis_hash` on the non-migration path. A graceful
/// restart repopulates `canonical_state.finalized_block`, so
/// `get_finalized_num_hash()` returns the disk finalized; the genesis fallback
/// covers a pristine network with no FCU yet.
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

/// How often a park on an external activation input re-asks: reth for the
/// activation block, and the local probe, upstream, and committee window in the
/// follower's entry march. One constant so the copies cannot drift apart.
const ACTIVATION_POLL: Duration = Duration::from_secs(2);

/// How many times a cold-start committee read that failed permanently — a revert,
/// or this node's own storage — is re-asked before the launch is refused.
const COLD_START_READ_ATTEMPTS: u32 = 5;
/// The pause between two of those attempts, on the runtime clock.
const COLD_START_READ_BACKOFF: Duration = Duration::from_secs(2);

/// A cold-start committee read that did not refuse the launch.
enum ColdStartRead {
    /// The record, and with it the epoch's verify-only scheme in the module.
    Ready(Arc<crate::committee::CommitteeRecord>),
    /// A retryable miss — this process's startup order, not the chain; the boundary
    /// trigger and the module's wake-up retry it after the launch.
    Deferred(crate::committee::CommitteeError),
}

/// Why a cold-start committee read refuses the launch.
#[derive(Debug)]
enum ColdStartRefusal {
    /// The contract answered something no committed epoch can answer.
    Impossible(crate::committee::CommitteeError),
    /// The epoch sits below the module's read window, which never moves down.
    BelowWindow(crate::committee::CommitteeError),
    /// A revert or a storage fault survived every attempt.
    Permanent {
        error: crate::committee::CommitteeError,
        attempts: u32,
    },
}

/// The cold-start read of `committee[epoch]` through the module, with a bounded
/// retry for the permanent-but-not-impossible class.
///
/// A revert or a storage fault under the anchor can be a moment's condition (a
/// provider still opening its static files, a module mid-upgrade), and the store
/// re-reads on every call, so the read is re-asked [`COLD_START_READ_ATTEMPTS`]
/// times [`COLD_START_READ_BACKOFF`] apart before it is the launch's verdict. A
/// transient miss returns at once — the launch queues the epoch for a retry
/// anyway — and an impossible answer or a below-window epoch is refused at once,
/// since no retry can change either.
async fn cold_start_committee_read(
    clock: &impl commonware_runtime::Clock,
    committee: &dyn crate::committee::Committee,
    epoch: u64,
) -> Result<ColdStartRead, ColdStartRefusal> {
    use crate::committee::CommitteeError;
    let mut attempts = 0u32;
    loop {
        attempts = attempts.saturating_add(1);
        let error = match committee.committee(epoch) {
            Ok(record) => return Ok(ColdStartRead::Ready(record)),
            Err(e) if e.is_transient() => return Ok(ColdStartRead::Deferred(e)),
            Err(e) if e.is_contract_impossible() => return Err(ColdStartRefusal::Impossible(e)),
            Err(e @ CommitteeError::OutOfWindow { .. }) => {
                return Err(ColdStartRefusal::BelowWindow(e))
            }
            Err(e) => e,
        };
        if attempts >= COLD_START_READ_ATTEMPTS {
            return Err(ColdStartRefusal::Permanent { error, attempts });
        }
        warn!(
            epoch,
            attempt = attempts,
            of = COLD_START_READ_ATTEMPTS,
            error = %error,
            "cold-start committee read failed permanently (a revert, or this node's own \
             storage) — re-asking after a pause"
        );
        clock.sleep(COLD_START_READ_BACKOFF).await;
    }
}

/// Wait for reth to hold the DPoS activation block, returning its
/// local-canonical hash. There is no give-up and no timeout: the anchor is
/// external (the sequencer must finalize the activation block before DPoS
/// starts), so the wait polls forever and raises
/// `dpos_sync_degraded{reason=activation_wait}` as the stuck signal. The hash is
/// local-canonical at a finalized height, so no operator compare is needed.
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
/// application-metadata store without building the marshal or engine — the
/// restart-vs-fresh-migration discriminator: an empty store returns 0, a
/// populated one the last DPoS-finalized height, so cold start resumes at the
/// correct epoch.
///
/// Reads the same metadata store the marshal opens and drops the handle before
/// the marshal re-opens it.
pub(crate) async fn read_consensus_archive_last_finalized(
    ctx: &Context,
    partition_prefix: &str,
) -> eyre::Result<u64> {
    // Storage-layout invariant: must match the marshal's own private `LATEST_KEY`,
    // pinned with the commonware rev in `Cargo.lock`.
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

/// Outcome of the pre-engine crash-survivor recovery.
#[derive(Debug)]
enum RecoverOutcome {
    /// reth now holds `target`; carries its local-canonical hash. The pre-engine
    /// marshal→reth replay bridged the gap, either from the local
    /// `finalized_blocks` archive or, for a hole below the marshal's finalized
    /// floor, by a BLS-verified by-height re-fetch through the cert upstream.
    Recovered(B256),
    /// reth is more than `MAX_COLD_RECOVER` blocks behind its own intact archive,
    /// beyond what the capped pre-engine replay bridges. The caller anchors the
    /// cold start at reth's tip and the executor's startup backfill drain walks
    /// the rest of the tail in. This is a distance problem, not an archive hole:
    /// the marshal holds every block. Needs an upstream, and a no-upstream node
    /// is fatal at the recovery site. `gap` is how far reth is behind `target`.
    DeferToElSync { gap: u64 },
}

/// Provider-only reconnect scan for [`recover_finalized_tail_into_reth`]: walks
/// `target` downward while reth is missing each parent.
enum ReconnectScan {
    /// reth holds the block at `lowest - 1` (or `lowest == 0`); replay
    /// `lowest..=target` from the marshal archive.
    Reconnect(u64),
    /// reth is missing `>= max_cold_recover` blocks below `target` — beyond a
    /// flush-race tail; the caller defers to devp2p EL sync.
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

/// The defer-vs-fatal decision when reth is more than `MAX_COLD_RECOVER` behind
/// its own intact archive. With an upstream, raise the gauges and defer: the
/// caller anchors at reth's tip and the executor's jump backfills the EL, and the
/// marshal holds every block, so no consensus-store repair is needed. Without an
/// upstream there is nowhere to backfill from, so this is real local data loss
/// and stays fatal. A below-floor archive hole does not route here; it heals
/// inline via [`refetch_hole_until_answered`].
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

/// The deferring half alone, for a cause that is not local data loss and therefore
/// has no fatal arm.
///
/// Its one caller is [`ReplaySeed::Defer`]: "this node does not hold the epoch key
/// yet" is about the beacon's acquisition, which self-heals off the artifact, and
/// an upstream is irrelevant to it either way. Making it fatal on a node with no
/// cert upstream would turn a bounded wait into a re-sync instruction.
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

/// BLS-verified by-height re-fetch of a marshal archive entry missing below the
/// finalized floor, which the live marshal never repairs (it starts at
/// `floor + 1` and prunes below) and the steady-state jump cannot reach.
///
/// The pull authenticates here because it reaches neither writer that normally
/// would (`store_finalization` after verification, or `FrontierHandler::deliver`):
/// a structural check that the payload matches the digest, then 2f+1 BLS
/// authentication against `committee[E]` read at `at_hash`.
///
/// The re-fetched entry is not written back: this path runs pre-engine against a
/// standalone archive handle, and reth is the only local reader that needs the
/// block at that moment.
///
/// `Err` is a verdict: no upstream configured, every upstream answered and none
/// holds the record, or the answer failed authentication. `Ok(None)` is the
/// absence of a verdict: not one upstream answered, so the caller must ask again.
/// Collapsing the two would print "re-sync the EL disk from a snapshot" at an
/// operator whose upstream is merely down, which is irreversible where a retry
/// is free.
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
    // `_everywhere`: the fatal below tells the operator to re-sync the EL disk from a
    // snapshot. Asking ONE upstream before saying that is not enough when the operator
    // configured several and the block sits on the second.
    let uf = match up.get_finalization_everywhere(Height::new(height)).await {
        WalkOutcome::Got(uf) => *uf,
        // Not a verdict: nothing answered, so nothing is concluded. Fail-safe
        // direction — a wrongly-withheld verdict costs one more lap, a wrongly-issued
        // one costs the operator's disk.
        WalkOutcome::NoneAnswered => return Ok(None),
        WalkOutcome::MissedEverywhere => {
            return Err(eyre!(
                "crash-survivor recovery: marshal {which} has a below-floor hole at height \
                 {height} and no configured upstream serves it — the consensus record is gone \
                 everywhere; re-sync the EL disk from a snapshot"
            ));
        }
    };
    // Nothing else binds the response to the request: `verify_jump_structural` ties
    // the cert only to the block it arrived with, and `verify_jump_authenticated`
    // takes the epoch from the cert's own round. Unpinned, a valid finalization for
    // a different height passes both and is spliced in as if it were this one.
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

/// [`refetch_verified_archive_hole`] until it produces a verdict: the record, or a
/// reasoned refusal. The only thing this adds is patience, and it is the block
/// path's policy rather than the seam's — the σ path deliberately does not wait
/// (see `replay_seed`).
///
/// **Asking again is the answer to "not one upstream answered", and it is not a
/// softening of the fatal.** A gone-everywhere verdict still exits, with the same
/// sentence, because that verdict is evidence: servers answered and none holds the
/// record. What may not happen is printing "re-sync the EL disk from a snapshot" at
/// an operator whose upstream is merely unreachable — the instruction is
/// irreversible and the condition is transient. The case is reachable because the
/// WS actor answers its mailbox while disconnected, so a pull returns instead of
/// hanging.
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
/// marshal's own `finalized_blocks` archive, or — on a below-floor block hole —
/// a BLS-verified by-height re-fetch through the cert upstream. The walk is
/// blocks-only: a locally-present block with a locally absent finalization cert
/// is a normal state (an ancestry-finalized height may have no standalone cert
/// anywhere, ever), so certs are read solely to authenticate a re-fetched missing
/// block, never per height.
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
    // A hole below the marshal's finalized floor: the live resolver cannot repair
    // below it and the deferred jump cannot fire at this gap, so the block is
    // re-fetched and BLS-verified through the cert upstream. A gone-everywhere
    // verdict stays fatal; only "not one upstream answered" is retried.
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
    /// σ this node already holds for the height's own round.
    Held(Seed),
    /// Beacon-active and the store missed — σ for this round has to be found.
    Wanted(Round),
}

/// What the crash-survivor replay may derive a height with.
enum ReplaySeed {
    /// `None` only on a beacon-inactive link, where `None` is what every node
    /// derives. It is not reachable from a σ miss: the digest fallback on a
    /// beacon-active link re-rolls `prev_randao` and forks the restart.
    Derive(Option<Seed>),
    /// σ is mandatory here and no source has it. The walk stops and defers.
    Unavailable,
    /// σ is mandatory here, a certificate for the round carries it, and this node
    /// cannot check it yet because the epoch key is not resolvable locally. The
    /// beacon holds the value and settles it on the `KeyAvailable` edge, so the
    /// walk defers rather than deriving — never fatally, whatever the upstream
    /// configuration.
    Defer,
}

/// σ for `height`'s own round out of this node's own store, or the round to go
/// looking for. The same rule the live executor derives with, applied at replay
/// so a restarted node cannot re-execute a height with a different `prev_randao`
/// than the network.
///
/// The agreed predicate comes first and the store second: `mandatory_at(epoch(h))`
/// is network-agreed and independent of anything local. Store-first could use a σ
/// filed at a round the agreed map calls beacon-inactive, and the seed journal
/// this store rehydrates from is replayed without re-verification by design
/// (epoch keys are pruned on an epoch window while σ is kept on a round window,
/// so a re-check is impossible). A stray is ignored and counted, never obeyed or
/// fatal: ignoring derives exactly what the network derives, whereas halting
/// would turn one bad record into a node that cannot start.
///
/// A height whose epoch the map cannot name (below the epocher origin) is
/// inactive and is never unwrapped: the beacon cannot have been mandatory in an
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
        // Read only to count it: the value is never handed on.
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
    /// whose σ was refused under the epoch's attested key.
    Absent,
}

/// σ out of a finalization, pinned to the round the caller named and checked
/// under the epoch key.
///
/// σ signs `seed_message(round)`, so a certificate for another round carries a
/// valid signature over something else; taking it would be the fork the caller is
/// avoiding. One implementation for both cert sources, local archive and upstream,
/// because the rule is the same for both.
///
/// The check is the beacon's, one rule in one place. It also files what it checks,
/// so the walk's own `Beacon::seed` read is the answer and a later height of the
/// same round needs no second check.
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
/// The store and the local `finalizations` archive are read without
/// re-verification, the same trust the seed journal and the block bodies take:
/// both were written by this node's own marshal after it verified them, and
/// re-checking one while trusting the other from the same disk would be
/// incoherent. The upstream read is the only verified one — it is authenticated
/// exactly like a cold-start jump landing. Every source is round-pinned by
/// [`seed_via_beacon`], so no source can substitute a neighbouring round's σ.
///
/// A local certificate is often absent and that is normal, not a fault: an
/// ancestry-finalized height may have no standalone cert anywhere. Its σ is then
/// the store's to supply, and a store that lost its journal tail falls through
/// to the upstream, or failing that to the caller's defer.
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
            // No verdict (not one upstream answered) — and here that is the same
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
            // Not fatal here, where it is fatal for a missing block: the block is
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

/// The crash-survivor recovery's committee read: `committee[E]` at the replayed
/// parent's executed hash, straight off reth.
///
/// The only [`CommitteeSource`] that is not the committee module. Recovery runs
/// before the beacon plane has published the geometry, so the module answers
/// nothing yet, and the replayed heights can sit above the module's read window
/// at reth's finalized tag; the walk's own parent hash is executed by
/// construction and names the committee that finalized the hole.
struct RecoveryCommitteeSource<Provider, EvmConfig> {
    reader: RethStakingStateReader<Provider, EvmConfig>,
    namespace: Vec<u8>,
}

impl<Provider, EvmConfig> RecoveryCommitteeSource<Provider, EvmConfig> {
    fn new(reader: RethStakingStateReader<Provider, EvmConfig>, chain_id: u64) -> Self {
        Self {
            reader,
            namespace: fluentbase_bls::fluent_namespace(chain_id),
        }
    }
}

impl<Provider, EvmConfig> crate::cert_inlet::CommitteeSource
    for RecoveryCommitteeSource<Provider, EvmConfig>
where
    Provider:
        StateProviderFactory + HeaderProvider<Header = Header> + Clone + Send + Sync + 'static,
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
{
    fn scheme_at(
        &self,
        epoch: u64,
        at_hash: B256,
        oracle: Option<Arc<dyn fluentbase_bls::oracle::SeedOracle>>,
    ) -> eyre::Result<BlsScheme> {
        let snap = self.reader.epoch_committee_snapshot(epoch, at_hash)?;
        ensure!(
            !snap.validators.is_empty(),
            "epoch {epoch} has no committed committee at {at_hash}"
        );
        let committee = crate::scheme::epoch_committee_from_snapshot(&snap)
            .map_err(|e| eyre!("epoch {epoch} committee has non-unique participants: {e:?}"))?;
        Ok(fluentbase_bls::scheme::build_verifier(
            &self.namespace,
            committee.bimap,
            epoch,
            oracle,
        ))
    }
}

/// Crash-survivor cold-start recovery: reth is missing the
/// consensus-finalized block at `target` (an ungraceful crash lost reth's
/// unflushed tail while the marshal persisted the finalization). Read the missing
/// blocks from the marshal's own `finalized_blocks` archive and import them into
/// reth, walking ancestors oldest-ward until reth reconnects; return the recovered
/// `target`'s local hash. The archive is opened standalone before the engine is
/// built, like the metadata peek, and dropped before the marshal re-opens it.
///
/// A gap wider than `MAX_COLD_RECOVER` (reth deeply behind its intact archive)
/// returns [`RecoverOutcome::DeferToElSync`] with an upstream, for the post-engine
/// devp2p jump. A below-floor hole is instead healed inline by a BLS-verified
/// by-height re-fetch through `upstream`. Both no-upstream cases stay fatal.
///
/// A height whose σ cannot be resolved takes the same defer, mid-walk: the blocks
/// already imported stay, and devp2p carries the EL from reth's new tip. Deriving
/// that height locally is the one thing this function may never do, because a
/// `prev_randao` derived from the digest fallback forks the restart.
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
    let mut verify_ctx = ctx.clone();
    // An ungraceful crash loses only reth's unflushed tail (typically 1-2 blocks);
    // a larger gap is not a flush race and defers to devp2p EL sync.
    const MAX_COLD_RECOVER: u64 = 64;

    // Provider-only: find the reconnect point. A gap wider than the flush-race
    // cap cannot be bridged by the pre-engine replay.
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

    // Replay `[lowest..=target]` from the marshal's own `finalized_blocks` archive.
    // σ for height `h` is resolved at `h`'s own round, the same key the live
    // executor derives with, so a restarted node cannot re-execute a height with a
    // different `prev_randao` than the network.
    //
    // The finalizations archive is opened as a σ fallback only, never as a gate: a
    // present block with an absent cert is normal (an ancestry-finalized height may
    // have no standalone cert anywhere), so nothing here requires a cert to exist.
    let archive = crate::outer::init_finalized_blocks_archive(ctx, MARSHAL_PARTITION_PREFIX)
        .await
        .wrap_err("crash-survivor recovery: opening the marshal finalized_blocks archive")?;
    let certs = crate::outer::init_finalizations_archive(
        ctx,
        MARSHAL_PARTITION_PREFIX,
        commonware_runtime::buffer::paged::CacheRef::from_pooler(
            ctx,
            crate::outer::PAGE_CACHE_PAGE_SIZE,
            crate::outer::PAGE_CACHE_CAPACITY,
        ),
    )
    .await
    .wrap_err("crash-survivor recovery: opening the marshal finalizations archive")?;

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
        // Per-block FCU, awaited: an InsertExecuted import adds to the canonical
        // chain but header-by-hash reads do not see the block until an FCU lands,
        // so the next iteration's parent read would fail without this.
        let resp = beacon_engine
            .fork_choice_updated(ForkchoiceState {
                head_block_hash: parent_hash,
                safe_block_hash: parent_hash,
                finalized_block_hash: parent_hash,
            })
            .await
            .wrap_err("crash-survivor recovery per-block FCU failed")?;
        // Judged by the next derive, not the response code: SYNCING during a
        // backfill does nothing, and INVALID can arrive after canonicalization.
        // The typed `ParentHeaderMissing` on the next iteration is the honest
        // signal.
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

/// Operator-supplied per-launch configuration. Keys and JSON-parsed configs
/// arrive pre-loaded (the host crate owns filesystem syscalls and permission
/// checks), and the slasher transport arrives pre-built because
/// `PoolTxSink<P, Provider>` carries concrete `reth-transaction-pool` trait
/// bounds that cannot compile in this crate.
pub struct DposLayerConfig<D, XC, A, U> {
    pub bls_keypair: ValidatorBlsKeypair,
    pub peer_keypair: commonware_cryptography::ed25519::PrivateKey,
    /// Every per-epoch committee read the layer makes, as one frozen record per
    /// epoch at one anchor. The same `Arc` the beacon's `CommitteeReads` facade
    /// views, so the consensus layer and the beacon plane cannot hold two versions
    /// of one epoch's committee.
    pub committee: Arc<dyn crate::committee::Committee>,
    /// `EpochTransition::last_tracked_epoch`, mirrored into one atomic cell whose
    /// writer is this layer's boundary-bridge forwarder and whose reader is the
    /// executor's frontier probe. A cell rather than a read of the transition,
    /// which sits behind an async mutex: a probe tick must never wait on the epoch
    /// machine it is asking about. `u64::MAX` means nothing tracked yet.
    pub tracked_epoch: Arc<std::sync::atomic::AtomicU64>,
    pub slasher_sink: Arc<dyn SlasherTxSink>,
    /// Evidence-channel bridge to the node's gossip task, which owns both p2p
    /// halves of `EVIDENCE_CHANNEL`.
    pub evidence: crate::slasher::EvidenceBridge,
    pub staking_config: StakingReaderConfig,
    /// Cert upstream: the marshal's by-height backfill resolver, the frozen-tip
    /// ladder probe, and the steady-state re-jump's EL work all ride it. `Some` for
    /// every launched node; `None` is a no-upstream validator, which
    /// `resolve_cold_start_kind` refuses for the empty-archive start (nothing would
    /// climb the ladder) and which otherwise catches up on the consensus-plane
    /// treadmill.
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
    /// The always-on beacon/DKG plane, built once per process and shared across the
    /// follower↔signer phase switch. The signer engine consumes its shared stores
    /// and reuses its oracle and metrics, cloning its mux handles per promotion; it
    /// never rebuilds the network, re-spawns the `DkgActor`, re-registers metrics,
    /// or re-binds `listen`.
    pub beacon_plane: SharedBeaconPlane,
    /// devnet/test-only byzantine behaviour (gated behind `dpos-devnet-byzantine`).
    /// Absent — and the field does not exist — in a production build.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub byzantine: Option<crate::byzantine::ByzantineMode>,
}

/// A plane-owned broker handle for one of the five non-beacon channels: the single
/// network's `(Sender, Receiver)` pair is owned by a persistent `Muxer` in the
/// always-on plane, and every promotion clones this handle and registers fresh
/// sub-channels against the same broker. A `SubReceiver` auto-deregisters on drop,
/// so a demoted engine frees the slots and a re-promoted one re-registers.
///
/// `MuxHandle::register` takes `&mut self`, so the shared handle is wrapped in
/// `Arc<Mutex<_>>` and each register locks transiently. The derived `Clone` on
/// `MuxHandle<S, R>` carries a spurious `R: Clone` bound the move-only
/// `DiscReceiver` does not satisfy, so the `Arc` is both the sharing mechanism and
/// the `Clone` needed for `SharedBeaconPlane`.
pub type PlaneMux = Arc<
    Mutex<
        commonware_p2p::utils::mux::MuxHandle<
            fluentbase_p2p::DiscSender<Context>,
            fluentbase_p2p::DiscReceiver,
        >,
    >,
>;

/// The one [`EpochTransition`] a validator process runs, handed down from the
/// node crate's always-on plane (where it is built before the engine, so the
/// geometry it freezes is available to the `DkgActor` and the committee module)
/// together with the receiving half of the boundary bridge it was constructed
/// with.
///
/// One instance, not two: boundary detection is pointwise, the engine's delivery
/// hook fires on every ordering-finalized block, and the plane's poller reads a
/// coalesced reth watch that can skip boundaries outright. The delivery hook is
/// therefore the driver; the plane keeps only the geometry freeze, which writes
/// none of the bootstrap state, so the cold start below is the process's one
/// bootstrapper.
///
/// Both halves travel together because `bridge_rx` can only be drained where
/// `OuterEngine::boundary_sender()` exists, which is after `build`.
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

/// The executor's frozen-tip frontier probe, built once here for both launch
/// paths, the plane-native validator and the follower.
///
/// Two requests per tick: the untargeted `Latest`, whose height is the hint
/// driver, and the ladder step `Finalized{last(T+1)}`. This closure only names the
/// step and its addressees; the executor puts it on the marshal's own resolver,
/// which carries the targets to `committee[T+1]` — the set that finalized that
/// height. The step's answer never comes back here; it goes into the marshal and
/// shows up as the tip moving.
///
/// `last(T+1)` and `committee[T+1]` both come from the committee module, one
/// geometry and one committee map per process. An unreadable `committee[T+1]` is
/// not a failure but "this node cannot name the addressee yet": count it and ask
/// `Latest` alone.
///
/// One constructor rather than two closures because the two node classes climb the
/// same ladder; the follower's `Latest`/by-height seam is its upstream instead of
/// the frontier resolver, which is the `U: CertUpstream` argument, not a second
/// body.
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

/// The executor's forward-only re-jump — over a target out of this node's own
/// marshal archive, so no committee source is involved — with the frozen-tip
/// probe as its live-follow driver. One body for both node classes; they differ
/// only in the L1 checkpoint (validator: none) and in where `T` comes from
/// (validator: the mirrored `EpochTransition` epoch; follower:
/// [`local_tracked_epoch`]).
///
/// `threshold` is epoch-relative — `min(JUMP_THRESHOLD, interval)` — so the
/// "≥2 epochs behind" defer deadlock heals within an epoch at any interval.
#[allow(clippy::too_many_arguments)] // one launch-time dependency each, not a cluster
fn re_jump_seam<Provider, BeaconEngine, U>(
    up: &U,
    committee: Arc<dyn crate::committee::Committee>,
    provider: Provider,
    beacon_engine_handle: BeaconEngine,
    ctx: Context,
    peer_count: Arc<dyn Fn() -> usize + Send + Sync>,
    activation: u64,
    l1_checkpoint: Option<B256>,
    threshold: u64,
    tracked_epoch: crate::executor::TrackedEpochFn,
) -> crate::executor::ReJump
where
    Provider: BlockHashReader + BlockNumReader + Clone + Send + Sync + 'static,
    BeaconEngine: BeaconEngineLike + Clone + Send + Sync + 'static,
    U: crate::cert_follow::CertUpstream,
{
    let rotate = up.rotate_callback();
    let probe = frontier_probe(up.clone(), committee);
    let call: crate::executor::ReJumpFn = Arc::new(
        move |from: u64, target: crate::cert_follow::UpstreamFinalized| {
            let provider = provider.clone();
            let beacon_engine_handle = beacon_engine_handle.clone();
            let peer_count = peer_count.clone();
            let jump_ctx = ctx.clone();
            Box::pin(async move {
                let el = crate::cold_start_jump::RethElSync::new(
                    jump_ctx,
                    provider,
                    beacon_engine_handle,
                    activation,
                    peer_count,
                );
                crate::cold_start_jump::jump_to_target(
                    from,
                    target,
                    &el,
                    l1_checkpoint,
                    activation,
                    threshold,
                )
                .await
            }) as futures::future::BoxFuture<'static, _>
        },
    );
    crate::executor::ReJump {
        call,
        threshold,
        rotate: Some(rotate),
        probe: Some(probe),
        tracked_epoch: Some(tracked_epoch),
    }
}

/// The by-height seeding of an epoch-boundary block a jump left below the marshal
/// floor, authenticated against the committee module at the module's own anchor.
/// One body for both node classes: a follower runs the same jump and leaves the
/// same hole, and an upstream-configured node can later be promoted to a seated
/// validator without restarting.
fn boundary_fetch_seam<U>(
    up: &U,
    committee: Arc<dyn crate::committee::Committee>,
    chain_id: u64,
    ctx: Context,
    sync_metrics: SyncMetrics,
) -> crate::cert_follow::BoundaryFetchFn
where
    U: crate::cert_follow::CertUpstream,
{
    let up = up.clone();
    let committees = Arc::new(crate::cert_inlet::ModuleCommitteeSource::new(
        committee, chain_id,
    ));
    Arc::new(move |height: u64, at_hash: B256| {
        let up = up.clone();
        let committees = committees.clone();
        let sync_metrics = sync_metrics.clone();
        // A fresh clone per call because `verify_jump_authenticated` needs
        // `&mut (Clock + CryptoRngCore)`.
        let mut fetch_ctx = ctx.clone();
        Box::pin(async move {
            crate::cert_follow::fetch_verified_boundary(
                &up,
                committees.as_ref(),
                &mut fetch_ctx,
                &sync_metrics,
                at_hash,
                height,
            )
            .await
        }) as futures::future::BoxFuture<'static, _>
    })
}

/// `T` for a node that runs no [`fluentbase_staking_reader::EpochTransition`],
/// the follower.
///
/// A validator mirrors `EpochTransition::last_tracked_epoch` off the boundary
/// bridge; a follower spawns no per-epoch engine and therefore no transition, so
/// it computes the same number from the committee module's geometry and its own
/// ordering-finalized cursor — the same cursor the module anchors its reads on.
///
/// The rule is the transition's, and it is the epoch manager's
/// [`crate::epoch_manager::live_epoch_of`]: `epoch_e + 1` when the finalized
/// block is the last block of its epoch and `epoch_e` otherwise.
///
/// `None` only while the geometry is unfrozen — the one state with no step to
/// take, which the probe counts as `no_tracked_epoch`/`no_geometry` and asks
/// `Latest` alone. A cursor still at its seed below activation is not a second
/// refusal: `Geometry::epoch_of` clamps a pre-activation height to epoch `0`, so
/// the answer there is `Some(0)`.
pub(crate) fn local_tracked_epoch(
    committee: Arc<dyn crate::committee::Committee>,
    cursor: crate::FinalizedCursor,
) -> crate::executor::TrackedEpochFn {
    Arc::new(move || {
        let geometry = committee.geometry()?;
        Some(crate::epoch_manager::live_epoch_of(&geometry, cursor.height()).get())
    })
}

/// The persistent beacon/DKG plane handed down from the node crate's always-on
/// component into each per-promotion signer engine. The node crate owns the single
/// `FluentP2P`, the transition-driven oracle peer set, the ordering-tip watch, and
/// the ceremony store; the signer engine reads the same shared `Arc`s and clones
/// the mux handles per promotion. There is exactly one network, listen bind, peer
/// set, and broker set per process, so a demote→re-promote needs no network
/// rebuild.
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
    /// The beacon plane's tip watch — THE clock as the plane reads it:
    /// `FluentApp` publishes marshal's tip on it beside its own channel
    /// (`with_beacon_tip`), and the plane's `DkgActor` is its ONE parked
    /// receiver. Travels with the plane rather than being created here for the
    /// same reason `plane_clock` does: the actor's receiver was taken from this
    /// sender before the engine existed, and a sender created here would be one
    /// the actor never reads.
    pub beacon_tip: Arc<tokio::sync::watch::Sender<u64>>,
    /// The self-heal / fork-safety metric family, registered once in the node
    /// crate where the beacon plane is built, and the fork-safety latch over it.
    /// Both travel with the plane for the same reason `plane_clock` does: the
    /// beacon's agreement launcher reads the latch (`beacon::ValidatorInputs::
    /// safety_halt`), and the executor, the epoch manager and the OuterEngine
    /// supervisor must read the same one — a latch built here would be one the
    /// beacon never sees engage. The production latch is restored from the
    /// datadir marker ([`crate::sync_metrics::SafetyHalt::restoring`]) at the
    /// node, before the first consensus event; the latch has no in-process
    /// `disengage`, so without the marker a restart silently cleared a halt and
    /// the node signed again on the same disk.
    pub sync_metrics: SyncMetrics,
    pub safety_halt: crate::sync_metrics::SafetyHalt,
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
    /// reth's own finalized tag `(cs_finalized, cs_finalized_hash)` and let the
    /// ladder plus the steady-state jump carry it forward.
    ///
    /// The anchor is not the genesis (where a runtime-deployed ChainConfig is
    /// codeless) and not an upstream's `Latest` (nothing local can check it). It is
    /// the same datum the follower path anchors on, written by exactly one thing:
    /// an FCU this node itself issued, or the pre-DPoS sequencer's.
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
        // finalized tag (`ElFinalized`). An upstream is still required, not to jump
        // at boot (this path does not), but because every route out of the gap runs
        // through a peer: the ladder's `Finalized{last(T+1)}` probe, the marshal's
        // by-height pulls, and the steady-state jump's own target, which only exists
        // once the marshal has stored something. A node with no upstream at all would
        // anchor here and never move.
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

/// What a fresh follower datadir — one with no local `ChainConfig`, so no
/// geometry, no committee and no archive — is allowed to use as its EL entry. The
/// one place in the system where nothing local can check a peer's answer, so the
/// choice is a policy and not a lookup, and it is pure and unit-tested as such.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FreshFollowerEntry {
    /// The operator named a block: `sync_to_checkpoint(hash)`.
    Checkpoint(B256),
    /// Local/test network only: take the upstream's word for its own tip. The one
    /// surviving `get_latest ⇒ sync_to`.
    UpstreamLatest,
}

/// The policy itself. On a deployed network a missing checkpoint is a startup
/// refusal: trusting one peer on first use is exactly the unauthenticated entry
/// this path removes, and unlike the other two entries there is no local datum to
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

/// What a follower that does have a local `ChainConfig` (geometry readable at
/// `rf_hash`) uses as its EL entry — the five-way march of `launch_follower`'s
/// `Some((activation, interval))` arm.
///
/// Deliberately not [`ColdStartKind`]: that enum is the validator discriminator,
/// named by the staking reader's doc contract, and its variants answer a different
/// question (which anchor a populated/empty consensus archive resumes at). One
/// enum serving both marches would tie two unrelated decisions together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FollowerEntry {
    /// reth already holds the activation block (or its finalized tag is already at
    /// or above it): the anchor is local, and no peer is contacted at all. Also the
    /// arm a node that produced the activation block itself lands in.
    Local,
    /// No entry of any kind: no operator checkpoint and no upstream — the honest
    /// sequencer→DPoS migration, where the block will be produced by the pre-DPoS
    /// sequencer on this very chain, so the node waits for reth to hold it
    /// (`wait_for_activation_block`, retry-forever). Last of the three peer-free
    /// steps, not the second: it is what is left when nothing else can be tried.
    WaitLocal,
    /// An operator checkpoint is configured and not yet consumed. Tried first
    /// after the local probe, for two independent reasons: `assert_l1_checkpoint`
    /// runs after the match and is counted from the landing, and the certificate
    /// entry lands on the lowest legal height (`activation`), so the reverse order
    /// would turn a survivable park into a fatal refusal; and
    /// `sync_to_checkpoint` needs no upstream at all, so ordering it after the
    /// upstream test parked a node that had an entry.
    Checkpoint,
    /// The ordering chain is not usable as an entry yet, for either of two reasons
    /// the caller distinguishes in its `warn!`: the upstream serves no `latest` at
    /// or above `activation + K` (no certificate below `activation + K` carries a
    /// real EVM hash — `order_block::result_target`), or no epoch's committee is
    /// readable at `rf_hash` yet. Both are "ask again", never a fatal: the input is
    /// external, exactly the `wait_for_activation_block` argument.
    ChainBelowActivation,
    /// Certificate entry: fetch the finalization for `target` from the upstream,
    /// authenticate it under the committee read at `rf_hash`, and EL-sync to its
    /// attested result. `target` is the highest height this node can still check —
    /// the one a cascading donor's `JUMP_THRESHOLD` window and a jumped validator's
    /// archive lose last — so one request per attempt replaces a by-height walk.
    Certificate { target: u64 },
}

/// The two inputs of [`follower_entry`] that cost a peer round trip: everything
/// else in the march is read locally. [`Default`] (both absent) is the peer-free
/// pass the caller runs first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct PeerProbes {
    /// `CertUpstream::get_latest().block.height` — used for routing and as the
    /// height ceiling only; the hash that comes with it is never read.
    latest_height: Option<u64>,
    /// The highest epoch whose committee reads `Ok` at `rf_hash`, probed over
    /// `0..=MAX_COMMITTEE_LOOKAHEAD_EPOCHS`; `None` when none does.
    ///
    /// "Highest readable" is the right ceiling only because readability is
    /// monotonic from zero: `commitEpochCommittee` runs upward from epoch 0 and a
    /// pre-activation state has pruned none of them. With a hole and a tip inside
    /// epoch 0, the target would land in the unreadable epoch and the fetch would
    /// refuse it forever. Nothing enforces the monotonicity; it is a property of
    /// the commit order, recorded because the ceiling depends on it.
    e_max: Option<u64>,
}

/// The march itself: pure, so the order of the five entries is unit-testable
/// without a node.
///
/// The [`PeerProbes`] are `Option` because the caller learns them only by asking a
/// peer, and the first three entries (local anchor, operator checkpoint,
/// wait-for-sequencer — in that order) must not cost a round trip. The caller
/// therefore evaluates this twice: once with both `None` (the peer-free prefix,
/// where `Local` / `WaitLocal` / `Checkpoint` are final), and again with the
/// probes filled only when the first verdict was
/// [`FollowerEntry::ChainBelowActivation`]. Re-deciding through the same function
/// keeps the peer-free prefix from being a second copy of the predicate.
fn follower_entry(
    holds_activation: bool,
    has_upstream: bool,
    has_checkpoint: bool,
    probes: PeerProbes,
    activation: u64,
    interval: u64,
    rf_num: u64,
) -> FollowerEntry {
    // `rf_num >= activation` does not depend on the probe: reth's own finalized
    // tag already sits at or above the activation block, so the anchor is
    // `(rf_num, rf_hash)` whatever a concurrent read of `block_hash(activation)`
    // says.
    if holds_activation || rf_num >= activation {
        return FollowerEntry::Local;
    }
    // The checkpoint comes before the upstream test: `sync_to_checkpoint` needs no
    // `CertUpstream` (it FCUs to the operator's hash and lets devp2p backfill), so
    // gating it behind "an upstream is configured" parked a node that had a good
    // entry.
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
    // The readable window can end below the floor whenever
    // `(e_max + 1) · interval <= K`, which nothing forbids: the contract rejects
    // only a zero interval and `read_geometry` only `> 0`. There is then no height
    // that is both checkable and at or above the floor, the same state as "the chain
    // is not there yet". Clamping the target up to the floor instead asked the
    // upstream for a height in an epoch whose committee is not readable at
    // `rf_hash`, so the fetch refused it forever.
    if last_readable < floor {
        return FollowerEntry::ChainBelowActivation;
    }
    // No `.max(floor)` here, and none is reachable: both inputs of the `min` are at
    // or above the floor — `latest` by the gate above, `last_readable` by this one.
    let target = latest.min(last_readable);
    FollowerEntry::Certificate { target }
}

/// The entry march's own by-height fetch: pull the finalization for `height`, pin
/// it to the request, bind the certificate to the body it arrived with, and
/// BLS-verify it under `committee[E]` read at `at_hash` — the same checks as
/// `cert_follow::fetch_verified_boundary`, but with a different failure surface.
/// That seam increments `jump_boundary_refetch_failed`, a counter shared with the
/// epoch-boundary seeding, and warns about verify-only admission that does not
/// apply to a follower still inside `launch_follower`, which is not a committee
/// member and just parks.
///
/// The reason comes back to the caller instead of being logged here, so the park
/// prints one diagnosis and re-prints only when the reason changes; the metric is
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
    // One ask per `ACTIVATION_POLL`, and a miss costs this node its entire entry.
    let uf = match upstream
        .get_finalization_everywhere(Height::new(height))
        .await
    {
        WalkOutcome::Got(uf) => *uf,
        WalkOutcome::MissedEverywhere => {
            return Err("no configured upstream holds the height".to_owned());
        }
        WalkOutcome::NoneAnswered => {
            return Err("no configured upstream answered the by-height pull".to_owned());
        }
    };
    // Nothing else binds the answer to the question: the structural check ties the
    // cert only to the block it came with, and authentication takes the epoch from
    // the cert's own round.
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

/// Visibility belt for a cold-start / follower landing read of a block reth just
/// materialized via an EL-sync jump or devp2p canonicalization: such a read can
/// transiently return `None`, because reth canonicalizes on the engine-tree thread
/// a few ms before provider reads see the block. Retry on the same 100 ms / 10 s
/// cadence under `dpos_sync_degraded{reason=landing_wait}=1`, fatal only after the
/// belt expires, since a materialized-but-missing read is then the genuine
/// data-loss fault. Bounded rather than retry-forever: unlike an external peer or
/// anchor wait, a landing that never materializes past its own EL sync is a local
/// fault, not a correlated one.
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
/// the cold-start discriminator reads. Returns 0 when the archive is empty or
/// absent. The unified supervisor's entry rule uses this to avoid choosing
/// signer-first for an in-committee node with no consensus state to resume, which
/// would hit the `resolve_cold_start_kind` "empty archive + EL past epoch 0" fatal
/// instead of following to build the archive first.
pub async fn peek_consensus_archive_last_finalized(ctx: &Context) -> eyre::Result<u64> {
    read_consensus_archive_last_finalized(ctx, MARSHAL_PARTITION_PREFIX).await
}

pub struct DposLayerHandle {
    pub consensus_handle: Handle<()>,
    /// Marshal mailbox clone for the node-side cert feed/RPC (by-height
    /// `get_finalization`+`get_block`). The node calls `feed_handle.set_marshal`
    /// with this once `launch` returns — keeping node types out of consensus.
    pub cert_mailbox: crate::outer::MarshalMailbox,
    /// Layer-internal tasks the host must supervise alongside `consensus_handle`,
    /// each with the label the host's exit log prints.
    ///
    /// The runtime runs with `with_catch_panics(true)` and a commonware `Handle`
    /// has no `Drop` impl, so a dropped handle detaches its task: a panic in it logs
    /// one line, resolves nothing, and the node keeps passing liveness checks with
    /// the subsystem dead. Handing the handle up lands it in the host supervisor,
    /// where a panic surfaces as `Err(Error::Exited)` and cancels the node.
    ///
    /// Appended into the host's `Vec<SupervisedHandle>` — same tuple shape.
    pub supervised: Vec<(&'static str, Handle<()>)>,
    /// Layer-internal tasks the host must let finish on a graceful stop, each with
    /// the label the host's drain log prints. Today: the durable seed-journal
    /// writer, whose last act is to append and fsync whatever the store queued but
    /// had not written.
    ///
    /// Same tuple shape as `supervised` but opposite semantics, and the two must not
    /// be merged: a `supervised` handle resolving means a subsystem died and the
    /// node must cancel, while one of these resolving means the task finished the
    /// work it owed, so shutdown may proceed. Every one of these is spawned outside
    /// the consensus engine's supervision subtree, because the host awaits them only
    /// after aborting that engine.
    pub drain_on_shutdown: Vec<(&'static str, Handle<()>)>,
    /// Serve one held epoch-key artifact over `consensus_getEpochArtifact`, so a
    /// Tier-2 follower obtains `PK_epoch` from this node exactly as it obtains
    /// certificates from it.
    ///
    /// `Some` on the follower path only, and the asymmetry is not an oversight:
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
/// the module's own answer, which is also what registers the epoch's scheme.
/// `false` ⇒ not readable yet — no executed anchor, or the epoch's committee not committed at
/// it — which the trigger treats as "retry on the next finalized block", never as
/// an empty committee.
type FollowerCommitteeAt = Arc<dyn Fn(u64) -> bool + Send + Sync>;

/// Hand one epoch to the epoch manager's boundary receiver. `false` ⇒ the
/// receiver is gone (the manager exited); the trigger stops.
type FollowerBoundaryDeliver =
    Arc<dyn Fn(Epoch) -> futures::future::BoxFuture<'static, bool> + Send + Sync>;

/// One step of the follower's epoch-boundary trigger: deliver the epoch the
/// finalized stream has entered, at most once per epoch. Returns whether the
/// trigger should keep running.
///
/// A validator gets its boundary deliveries from `EpochTransition` on the beacon
/// plane; a follower has no plane, so it derives the same delivery from the
/// finalized `OrderBlock`s its own marshal reports.
///
/// `last_delivered` advances only after a delivery. An unreadable committee must
/// leave the epoch unconsumed: `committee[E]` is committed during `E-1` but the
/// read runs at the EL-finalized hash, which trails the ordering-finalized height
/// by `K`, so the first blocks of `E` can legitimately read back nothing.
///
/// Epochs this step skipped entirely are deliberately not back-filled. A floor move
/// dispatches no block of the skipped epoch, so nothing needs a scheme; driver lag
/// is the case where blocks were dispatched but the step saw only the latest
/// height, and the repair sweep cannot cover it because its work list is the
/// registered schemes. Reaching lag means the finalized height advanced a whole
/// `interval` inside one state read, so a back-fill would add a committee read per
/// skipped epoch on the read that is, by hypothesis, the slow one.
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
    /// Launch the DPoS layer end-to-end: build the staking reader, the p2p network,
    /// and the `OuterEngine`; cold-start the plane's transition at this node's own
    /// (post-jump) anchor; spawn forwarder, outer, and network; return their
    /// handles for the host to supervise.
    ///
    /// The [`EpochTransition`] is not built here: it arrives as
    /// [`PlaneEpochTransition`] from the always-on plane, which built it before this
    /// launch so the geometry it freezes was already available to the `DkgActor` and
    /// the committee module. This layer supplies the two things only it has — the
    /// per-block delivery driver (`boundary_hook`) and the executor's read-floor
    /// seam.
    ///
    /// The caller supervises the returned handles and performs filesystem key
    /// loading and `PoolTxSink` construction before calling.
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
            upstream,
            deriver,
            executed,
            assembler,
            target_gas_limit,
            feed,
            spawn_unblocked,
            beacon_plane,
            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine,
        } = cfg;

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
            beacon_tip,
            sync_metrics,
            safety_halt,
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

        let staking_address = staking_config.staking_address;
        let reader = RethStakingStateReader::new(
            provider.clone(),
            evm_config.clone(),
            staking_config.clone(),
        );

        let (cs_finalized, cs_finalized_hash, _head_num, _head_hash) =
            derive_cold_start_heights(&canonical_state, genesis_hash);

        // Read at `cs_finalized_hash`, the block reth pre-populates into
        // `canonical_in_memory_state` during init: a by-number hash would go to the
        // DB historical arm and can revert before it materializes. Equivalent to
        // reading at the resumed height because `dposActivationBlock` is immutable
        // and `epochBlockInterval` is stable across the migration/restart window.
        let dpos_activation_block = reader.dpos_activation_block(cs_finalized_hash)?;
        let interval = reader.epoch_block_interval(cs_finalized_hash)?;
        let epoch_length_blocks =
            NonZeroU64::new(interval).ok_or_eyre("epoch_block_interval must be > 0")?;

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
                match provider.block_hash(archive_finalized)? {
                    Some(hash) => (archive_finalized, hash),
                    None => {
                        // Recovery runs here, not only in the executor backfill, because
                        // the committee read at `latest_finalized_hash` and the genesis
                        // read both require reth to hold the resume block.
                        //
                        // The one committee read that cannot go through the module: it runs
                        // before the plane has published the geometry, and a re-fetched
                        // hole can sit above the module's window at reth's finalized tag,
                        // so the read follows the replayed parent instead.
                        let recover_committees = RecoveryCommitteeSource::new(
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

        // Read both after the crash-survivor recovery, which imports the missing
        // reth tail; a pre-recovery snapshot would be stale and make the executor
        // backfill re-derive those blocks.
        let (_cs_fin, _cs_fin_hash, head_num, head_hash) =
            derive_cold_start_heights(&canonical_state, genesis_hash);

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

        // The pre-DPoS sequencer is production-gated at `dposActivationBlock`, so a
        // fresh migration's reth head must already equal the activation anchor.
        // Failing here beats wedging in the executor's ancestor-FCU guard.
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

        // The cold-start committee read goes through the module, which registers the
        // record and this epoch's verify-only scheme in one slot so the marshal can
        // verify certificates of the starting epoch before any boundary fires. An
        // impossible or below-window refusal is a fact about the chain and stays the
        // loud startup error; a transient miss is this process's startup order, and
        // the cold start below queues the epoch for a retry.
        match cold_start_committee_read(&ctx, committee.as_ref(), initial_epoch_u64).await {
            Ok(ColdStartRead::Ready(record)) => info!(
                epoch = initial_epoch_u64,
                members = record.members.len(),
                "cold-start committee read through the committee module"
            ),
            Err(ColdStartRefusal::Impossible(e) | ColdStartRefusal::BelowWindow(e)) => {
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
            Err(ColdStartRefusal::Permanent { error, attempts }) => {
                return Err(eyre!(
                    "committee[{initial_epoch_u64}] read FAILED PERMANENTLY {attempts} times at \
                     this node's committee anchor (read at finalized block {latest_finalized}): \
                     {error}. This class is a revert or this node's own storage fault, not a \
                     statement about the committed state — repair the cause (the staking \
                     module at GENESIS_STAKING, or this node's reth database) and restart."
                ));
            }
            Ok(ColdStartRead::Deferred(e)) => warn!(
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

        let PlaneEpochTransition {
            transition: et_arc,
            mut bridge_rx,
        } = epoch_transition;

        const NO_TRACKED_EPOCH: u64 = u64::MAX;
        let tracked_epoch_cell = tracked_epoch;

        // The process's one bootstrap: it picks the starting epoch the epoch manager
        // learns over the bridge and raises the read floor to the landing. The plane's
        // poller already froze the geometry and registered a first peer set, which
        // writes no bootstrap state, so this still takes the write-once path.
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

        // Anchor the consensus genesis at `latest_finalized`, not at chain height 0:
        // Simplex caches `set_genesis(hash_N)`, so view 1's `context.parent` must
        // match the proposer's `block.parent`.
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
        let genesis_block = anchor_order_block(&genesis_sealed)?;

        // Fires for every `Update::Block`, spawned via `ctx.spawn`: `tokio::spawn`
        // would depend on the implicit `tokio::Handle::current()` contract under
        // commonware-tokio.
        let consecutive_errors = Arc::new(AtomicU32::new(0));
        let et_for_hook = et_arc.clone();
        let ctx_for_hook = ctx.with_label("boundary_hook");
        let errors_for_hook = consecutive_errors.clone();
        let sync_metrics_for_hook = sync_metrics.clone();
        let parked_boundary_height = Gauge::<i64>::default();
        ctx_for_hook.register(
            "parked_boundary_height",
            "Height of the epoch boundary PARKED awaiting EL state materialization (0 = none). \
             A sustained non-zero value flags a durably-wedged node.",
            parked_boundary_height.clone(),
        );
        let on_finalized_consecutive_errors = Gauge::<i64>::default();
        ctx_for_hook.register(
            "on_finalized_consecutive_errors",
            "Consecutive epoch-boundary on_finalized errors (0 = healthy). Retried-degraded \
             forever (no shutdown); a sustained non-zero value flags a wedged boundary read.",
            on_finalized_consecutive_errors.clone(),
        );
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
        // Keyed on a height rather than a delivered block so the steady-state re-jump can
        // drive the same entry without synthesising an `Update::Block`, whose `Exact`
        // ack the executor must fire.
        let enter_boundary: Arc<dyn Fn(u64) + Send + Sync> = Arc::new(move |number: u64| {
            let et = et_for_hook.clone();
            let ctx_task = ctx_for_hook.clone();
            let errors = errors_for_hook.clone();
            let parked_gauge = parked_gauge_for_hook.clone();
            let errors_gauge = errors_gauge_for_hook.clone();
            let sync_metrics = sync_metrics_for_hook.clone();
            let repoke_panics = boundary_repoke_panics.clone();
            drop(ctx_task.spawn(move |ctx_inner| async move {
                // A parked boundary replays only on the next `on_finalized` call, and
                // during catch-up the parked boundary is the last deliverable block, so
                // the loop re-pokes until the park clears. There is no internal give-up:
                // liveness is the `parked_boundary_height` gauge and the harness
                // recover-stall deadline.
                //
                // Catch the unwind so a panic in the only driver of this boundary ticks
                // `boundary_repoke_task_panics` instead of being reduced by
                // `with_catch_panics(true)` to a single log line.
                use futures::FutureExt as _;
                let repoke = std::panic::AssertUnwindSafe(async move {
                    let mut pokes = 0u64;
                    loop {
                        let outcome = {
                            let mut et_guard = et.lock().await;
                            et_guard.on_finalized(number).await
                        };
                        match outcome {
                            Ok(TransitionOutcome::EpochAdvanced(_) | TransitionOutcome::Intra) => {
                                if errors.swap(0, Ordering::Relaxed) != 0 {
                                    errors_gauge.set(0);
                                    sync_metrics.recover(SyncReason::BoundaryHook);
                                }
                            }
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
                        // Both the delivery adapter and the executor's re-jump landing can
                        // call `enter_boundary`, so the gauge may credit a park to the other
                        // spawn; both loops exit only on `None`, and whichever sees `None`
                        // clears the gauge.
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

        // The executor awaits this rather than spawning it: the floor must be in place
        // before the entry's first committee read, or the read races the jump landing.
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

        // The executor's reaction to its own `Update::Tip`, and the only jump. `T` is
        // the mirrored `EpochTransition` epoch; no L1 checkpoint on the validator path.
        let re_jump_threshold = crate::cold_start_jump::JUMP_THRESHOLD.min(interval);
        let re_jump: Option<crate::executor::ReJump> = upstream.as_ref().map(|up| {
            let cell = tracked_epoch_cell.clone();
            let tracked_epoch: crate::executor::TrackedEpochFn =
                Arc::new(
                    move || match cell.load(std::sync::atomic::Ordering::Relaxed) {
                        NO_TRACKED_EPOCH => None,
                        epoch => Some(epoch),
                    },
                );
            re_jump_seam(
                up,
                committee.clone(),
                provider.clone(),
                beacon_engine_handle.clone(),
                ctx.clone(),
                peer_count.clone(),
                dpos_activation_block,
                None,
                re_jump_threshold,
                tracked_epoch,
            )
        });

        // After a jump the epoch-terminal height `Inline::genesis` needs sits below the
        // new marshal floor, where no repair path fetches it; without this a member
        // that did not already hold it parks verify-only for the landing epoch.
        let boundary_fetch: Option<crate::cert_follow::BoundaryFetchFn> =
            upstream.as_ref().map(|up| {
                boundary_fetch_seam(
                    up,
                    committee.clone(),
                    chain_id,
                    ctx.clone(),
                    sync_metrics.clone(),
                )
            });

        // Register on this context, the same one `BeaconMetrics` uses inside
        // `beacon::build`: commonware prefixes each family with the context's label
        // path, and a labelled child would silently rename them.
        let epoch_metrics = crate::epoch_manager::EpochEngineMetrics::default();
        epoch_metrics.register(&ctx);
        let executor_metrics = crate::executor::ExecutorMetrics::default();
        executor_metrics.register(&ctx);

        let outer = OuterBuilder {
            me: me.clone(),
            // NoopBlocker: the simplex batcher's evidence-free `block!` on a transient
            // batch-verify-failed vote would partition an honest peer on the shared
            // global transport; equivocation evidence rides `Activity::Conflicting*`
            // independently, so slashing is preserved.
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
            beacon_tip: Some(beacon_tip),
            timeouts: ConsensusTimeouts::fluent_1s(),
            mailbox_size: 256,
            // At most 4 order-block bodies per primary sender, matching the deepest
            // legitimate pipeline (proposal in flight plus a re-proposal after
            // nullify); `buffered::Config` has no byte cap, so the per-peer bound is
            // `deque_size × MAX_ORDER_BLOCK_SIZE`.
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
            initial_finalized: (Height::new(latest_finalized), latest_finalized_hash),
            initial_head: (Height::new(initial_head_num), initial_head_hash),
            // The marshal never dispatches pre-anchor history; a later steady-state
            // jump raises the floor to `landing − K` through `set_floor`.
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
        // Clone `seed_journal_writer` off `ctx`, not the engine's context: `engine.abort()`
        // must not cascade into the writer before it drains.
        .build(ctx.with_label("outer_engine"))
        .await?;

        // The transition's trigger type belongs to `staking-reader`, so the snapshot is
        // dropped here; what matters is that no consumer downstream sees a committee
        // that did not come from the committee module.
        let outer_boundary_tx = outer.boundary_sender();
        let shutdown_for_forwarder = shutdown.clone();
        // Supervised, not detached: a panic here would unwind past the
        // `shutdown.cancel()` below and be reduced by `with_catch_panics(true)` to one
        // log line, stopping committee rotation while the node looks healthy.
        let tracked_epoch_writer = tracked_epoch_cell.clone();
        let epoch_bridge_handle = ctx.with_label("epoch_bridge").spawn(move |_| async move {
            while let Some((u64_ep, _snap)) = bridge_rx.recv().await {
                // The transition only queues epochs it has tracked and never goes
                // backwards, so a plain store mirrors it.
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

        let cert_mailbox = outer.marshal_mailbox();

        // `upstream` is threaded into the marshal's by-height backfill resolver so an
        // out-of-committee validator backfills like a follower instead of wedging; the
        // resolver BLS-verifies every delivered cert.
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
            // The journal writers live in `beacon::build` with the stores they back;
            // the node collects them off the `Beacon`.
            drain_on_shutdown: vec![],
            // The validator serves from its beacon plane's own `Beacon::artifact_bytes`.
            artifact_bytes: None,
        })
    }
}

/// Reth handles a follower needs. Distinct from [`RethHandle`] in that a follower
/// carries no slasher/pool transport — it never signs, so the slasher actor is
/// built but never started.
pub struct FollowerRethHandle<Provider, EvmConfig, BeaconEngine> {
    pub provider: Provider,
    pub evm_config: EvmConfig,
    pub beacon_engine_handle: BeaconEngine,
    pub chain_id: u64,
    pub canonical_state: reth_chain_state::CanonicalInMemoryState<EthPrimitives>,
    pub genesis_hash: B256,
    /// Read-only probe of reth's connected devp2p peer count; the follower's
    /// EL-sync no-peers net reads it.
    pub peer_count: Arc<dyn Fn() -> usize + Send + Sync>,
}

/// Operator-supplied config for the follower. The node owns the WS upstream and the
/// one broadcast `Muxer`; this config carries the reth-evm collaborators and the
/// cert-inlet feed channels.
pub struct FollowerLayerConfig<D, XC, A, U> {
    /// This node's ed25519 peer identity (the FluentP2P crypto's public key). A
    /// standalone follower never gossips, but `buffered::Engine` and the marshal
    /// resolver are keyed on it.
    pub me: commonware_cryptography::ed25519::PublicKey,
    pub staking_config: StakingReaderConfig,
    /// Datadir path of the fork-safety halt marker
    /// ([`crate::sync_metrics::SafetyHalt::restoring`]). A marker left by a
    /// previous run brings this node up permanently verify-only — the latch has
    /// no in-process `disengage`, so without it a restart silently cleared a
    /// halt and the node signed again on the same disk. `None` only in tests.
    pub halt_marker: Option<std::path::PathBuf>,
    /// L1 Rollup-checkpoint hash, also the operator checkpoint a fresh datadir syncs
    /// to: `Some` ⇒ the fresh-datadir entry FCUs to this hash and then fail-closed
    /// asserts it. `None` on a fresh datadir is a startup refusal on a deployed
    /// network and a warned trust-on-first-use on a local one — see
    /// [`Self::deployed_network`].
    pub l1_checkpoint_hash: Option<B256>,
    /// Whether this chain_id is a deployed network (devnet / testnet / mainnet).
    /// Evaluated by the node because the chain_id constants live in its chainspec;
    /// used here for exactly one decision: a fresh datadir with no operator
    /// checkpoint refuses to start on a deployed network and falls back to the
    /// upstream's `Latest` only off one.
    pub deployed_network: bool,
    /// OrderBlock → derived-EVM-block execution (node-built over reth-evm).
    pub deriver: D,
    /// Local derived-chain view (node-built over the reth provider).
    pub executed: XC,
    /// The same ordering-finalized cursor [`Self::executed`] was built over. The
    /// committee module anchors its reads on it, so a follower reads every committee
    /// at the height its own executor has finalized and at no other.
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
    /// Cert upstream: the marshal's by-height backfill resolver, the frozen-tip
    /// ladder probe, and the steady-state re-jump's EL work ride it, plus — on a
    /// fresh datadir off a deployed network only — the one surviving `get_latest ⇒
    /// sync_to` ([`FreshFollowerEntry::UpstreamLatest`]). A follower always has an
    /// upstream (the WS the inlet uses); `None` only in tests.
    pub upstream: Option<U>,
    /// The live finalized-cert stream the node's WS actor pushes — the inlet's sole
    /// producer (a follower forms no certs locally).
    pub finalized_rx: mpsc::Receiver<crate::cert_follow::UpstreamFinalized>,
    /// Connection-generation token the node's WS actor bumps on each (re)connect.
    /// Wired into the inlet via `with_connection_token` so the data-fault streak is
    /// scoped to the live connection: a connection-level auto-rotation resets the
    /// streak, so one upstream's faults never bleed into the next URL's budget.
    /// `None` in tests.
    pub conn_gen: Option<Arc<std::sync::atomic::AtomicU64>>,
    /// The serving-window sink: each verified inlet pair is forwarded here so a
    /// tier-2 follower aligns via this node's `consensus` WS window. `None` = no
    /// serving (tests).
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
    /// Launch the near-planeless follower: an `OuterEngine` with `signer_keypair:
    /// None` driven by the cert-inlet instead of a local BFT engine. Resolves the
    /// geometry, EL-syncs to an authenticated entry, builds the follower engine, and
    /// starts it over the one broadcast `Muxer` with an upstream-backed marshal
    /// resolver that backfills the by-height gap the live stream never carries. The
    /// executor is the sole reth writer.
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

        // The follower has no beacon plane, so it registers its own self-heal metrics;
        // threaded into the activation-wait and landing-belt loops.
        let sync_metrics = SyncMetrics::default();
        sync_metrics.register(&ctx);
        // A follower's executor derives and imports off the inlet, so it needs the same
        // fork-safety latch: halt-and-stay-up rather than crash.
        let safety_halt = match halt_marker {
            Some(path) => crate::sync_metrics::SafetyHalt::restoring(sync_metrics.clone(), path),
            None => crate::sync_metrics::SafetyHalt::new(sync_metrics.clone()),
        };

        let reader = RethStakingStateReader::new(
            provider.clone(),
            evm_config.clone(),
            staking_config.clone(),
        );

        // One frozen committee record per epoch at one anchor, over this node's own
        // executor cursor. Built the moment the geometry is known — inside the entry
        // march, because the march's own committee reads go through it — with the
        // watch created already frozen, so there is no freeze wake-up to publish; the
        // verify-only scheme is filled the moment `beacon::build_follower` returns.
        let beacon_slot: crate::committee::BeaconSlot = Arc::new(std::sync::OnceLock::new());
        let build_committee =
            |activation: u64, interval: u64| -> Arc<dyn crate::committee::Committee> {
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
                ))
            };

        // The geometry read is pinned to one hash, but the arm below re-reads reth's
        // finalized tag on every turn of its own loop. On a restart both geometry and
        // anchor are local, so no `sync_to` runs; a fresh datadir has nothing local, so
        // its entry is an operator checkpoint or a refusal.
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

        let (activation, interval, anchor_height, anchor_hash, committee) =
            match read_geometry(&reader, geometry_at_hash)? {
                Some((activation, interval)) => {
                    let committee = build_committee(activation, interval);
                    // The march's two committee reads — the `e_max` probe and the entry
                    // certificate — go through the module at its own anchor, which is
                    // reth's finalized tag here (the cursor is not seeded yet): the same
                    // `(number, hash)` `derive_cold_start_heights` reads each turn.
                    let committees =
                        crate::cert_inlet::ModuleCommitteeSource::new(committee.clone(), chain_id);
                    // The checkpoint is consumed at most once: a checkpoint on a pre-DPoS
                    // batch lands below activation, and re-driving it would spin on
                    // `sync_to_checkpoint`'s already-canonical short-circuit.
                    let mut checkpoint_pending = l1_checkpoint_hash;
                    let mut warned_wait = false;
                    // The last refusal reason printed for the certificate entry, so the
                    // park re-prints only when the reason changes.
                    let mut unserved_reason: Option<String> = None;
                    loop {
                        // Re-read every turn: the pre-DPoS sequencer keeps moving reth's
                        // finalized tag, and a late `commitEpochCommittee(0)` appears only
                        // on a later block, so a pinned tag could never change its answer.
                        let (rf_num, rf_hash, _, _) =
                            derive_cold_start_heights(&canonical_state, genesis_hash);
                        // The local probe is the only step with no network cost, so it
                        // runs first every turn.
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
                        // Only the retry frontier of the peer-free prefix is worth a round
                        // trip; re-deciding through `follower_entry` keeps this from being
                        // a second copy of the predicate.
                        let entry = match (peer_free, upstream.as_ref()) {
                            (FollowerEntry::ChainBelowActivation, Some(up)) => {
                                // `get_latest` is routing and a height ceiling only; its
                                // hash is never read, so a liar can only pull the target
                                // down, and the landing hash comes from the attested result.
                                let latest_height =
                                    up.get_latest().await.map(|latest| latest.block.height);
                                // The readable window at `rf_hash`: devnet genesis has only
                                // `committee[0]`; a pre-activation prod block reaches
                                // `MAX_COMMITTEE_LOOKAHEAD_EPOCHS`.
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
                                    break (activation, interval, rf_num, rf_hash, committee);
                                }
                                // `Local` below activation is returned only for
                                // `holds_activation == activation_hash.is_some()`, so a
                                // `None` would mean the march and this dispatch disagree.
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
                                break (activation, interval, activation, hash, committee);
                            }
                            FollowerEntry::WaitLocal => {
                                // The activation block is produced on this chain, so there
                                // is nobody to ask; `wait_for_activation_block` retries
                                // forever.
                                let hash = wait_for_activation_block(
                                    &ctx,
                                    &provider,
                                    activation,
                                    &sync_metrics,
                                )
                                .await?;
                                break (activation, interval, activation, hash, committee);
                            }
                            FollowerEntry::Checkpoint => {
                                // `Checkpoint` is returned only for `has_checkpoint`,
                                // which is `checkpoint_pending.is_some()`; a `None` here
                                // would mean the march and this dispatch disagree.
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
                                    break (activation, interval, h, hash, committee);
                                }
                                // A checkpoint landing below activation is legal but not a
                                // DPoS anchor; re-run the march without sleeping — the local
                                // probe may now hold the activation block, and otherwise the
                                // certificate entry is next.
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
                                // Not fatal: the input is external, the honest case is
                                // "not there yet", and a fatal here restart-storms every
                                // joiner at once.
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
                                        // A refusal cannot distinguish an upstream that is
                                        // not up yet from one that no longer keeps the
                                        // height, and the first must not be fatal; the park
                                        // re-prints only when the reason changes.
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
                                // A failed drive is a retry, not a startup fatal: `sync_to`'s
                                // post-landing `block_hash` can transiently miss and map to
                                // `Stalled`, while re-driving is safe because the target is
                                // already committee-authenticated.
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
                                break (activation, interval, landing, hash, committee);
                            }
                        }
                    }
                }
                None => {
                    // Fresh datadir: no geometry, committee or archive, so the entry is
                    // the pure `fresh_follower_entry` policy and only the EL work is here.
                    let (h, hash, entry) =
                        match fresh_follower_entry(l1_checkpoint_hash, deployed_network, chain_id)?
                        {
                            // `sync_to_checkpoint` FCUs to the operator's hash and learns the
                            // height from the landing, which is why the config carries no
                            // `(height, hash)` pair.
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
                    // An entry below the activation block is not a DPoS anchor; refusing on
                    // the input beats clamping to a height reth does not hold.
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
                    (
                        activation,
                        interval,
                        h,
                        hash,
                        build_committee(activation, interval),
                    )
                }
            };

        // Fail-closed after EL-sync: a checkpoint that is not in the local chain is a
        // startup error.
        if let Some(l1_hash) = l1_checkpoint_hash {
            crate::cold_start_jump::assert_l1_checkpoint(&provider, l1_hash)?;
        }

        // The landing block's own result attestation arrives only K blocks later, so the
        // finalized tier starts at `landing − K`, clamped to activation; seeding reth's
        // FCU at that floor keeps it from finalizing ahead of the result tier.
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

        // The marshal and executor need the genesis anchor even though a follower never
        // proposes; every node derives the identical artifact.
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
        // The beacon-owned families are registered by `beacon::build_follower` below;
        // registering `BeaconMetrics` here too would duplicate every family silently,
        // because `prometheus_client::Registry::register` neither deduplicates nor
        // complains. These two families are core- and executor-owned on both node
        // classes, so they are registered here.
        let epoch_metrics = crate::epoch_manager::EpochEngineMetrics::default();
        epoch_metrics.register(&ctx);
        let executor_metrics = crate::executor::ExecutorMetrics::default();
        executor_metrics.register(&ctx);

        // The follower's executor reaction to its own `Update::Tip`, and its only jump.
        // The follower's WS inlet is a subscription to current finalizations and
        // replays no intermediate height, so without the ladder step a node whose
        // tip freezes at `last(epoch(fin)+2)` stays parked; a follower runs no
        // `EpochTransition`, so `T` is computed locally.
        let re_jump_threshold = crate::cold_start_jump::JUMP_THRESHOLD.min(interval);
        let re_jump: Option<crate::executor::ReJump> = upstream.as_ref().map(|up| {
            re_jump_seam(
                up,
                committee.clone(),
                provider.clone(),
                beacon_engine_handle.clone(),
                ctx.clone(),
                peer_count.clone(),
                activation,
                l1_checkpoint_hash,
                re_jump_threshold,
                local_tracked_epoch(committee.clone(), finalized_cursor.clone()),
            )
        });

        let boundary_fetch: Option<crate::cert_follow::BoundaryFetchFn> =
            upstream.as_ref().map(|up| {
                boundary_fetch_seam(
                    up,
                    committee.clone(),
                    chain_id,
                    ctx.clone(),
                    sync_metrics.clone(),
                )
            });

        // `boundary_hook` fires synchronously inside the marshal's reporter for every
        // finalized `OrderBlock`, so it only stamps and wakes; the committee read and the
        // delivery run on the driver task below. `fetch_max` plus a permit-storing
        // `Notify` is lossless: the driver reads the latest height, and a wake landing
        // while it is not armed is held as a permit.
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

        // Every staking read the follower's beacon takes is answered from the committee
        // module — the same type the validator plane is handed, so the two node classes
        // cannot drift on the cursor or on the reads.
        let follower_committees: Arc<dyn crate::beacon::CommitteeReads> = Arc::new(
            crate::committee::CommitteeReadsFacade::new(committee.clone()),
        );
        // `None` (a test with no upstream) leaves the rung permanently empty, which is
        // vote-only admission, never a fault.
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
        // A follower's randomness is negative on everything it produces (no DKG, no
        // ceremony store, no share, no seed) and real on the one thing it verifies: the
        // key a certificate of an epoch is checked against. The three capabilities above
        // are the delivery route that verification needs.
        let (randomness, beacon_tasks) = crate::beacon::build_follower(
            &ctx,
            crate::beacon::FollowerInputs {
                chain_id,
                committees: follower_committees,
                fetch: artifact_fetch,
            },
        );
        // The committee module's scheme producer can build now; epochs read before this
        // line pick theirs up on the next `Committee::scheme`.
        crate::committee::fill_beacon_slot(&beacon_slot, &randomness);
        // Weak because this closure is handed to the RPC feed, which outlives every task
        // the supervisor aborts: a strong clone would keep the beacon and its journal
        // sender alive past shutdown.
        let artifact_bytes = {
            let beacon = Arc::downgrade(&randomness);
            Arc::new(move |epoch: u64| beacon.upgrade()?.artifact_bytes(epoch))
                as std::sync::Arc<dyn Fn(u64) -> Option<Vec<u8>> + Send + Sync>
        };
        let artifact_fetch_handle = beacon_tasks.supervised;
        // Held and drained like the validator's: the node owes the beacon two handles,
        // and dropping this one would make a future journal writer go undrained silently.
        let beacon_drain_handle = beacon_tasks.drain;
        let outer = OuterBuilder {
            me: me.clone(),
            // NoopBlocker on the follower too: the marshal cert resolver's `deliver=false`
            // verdict would `block!` a registry-∪-committee peer on the shared global
            // transport.
            blocker: NoopBlocker,
            provider: oracle.clone(),
            chain_id,
            epoch_length_blocks,
            dpos_activation_block: activation,
            signer_keypair: None,
            // A follower runs no agreement plane and never mints an artifact, and the
            // plane's pull seam does not reach it; its one delivery route is the cert
            // upstream, so admission is vote-only only until the epoch's artifact
            // arrives and verifies against `committee[epoch]`.
            randomness: randomness.clone(),
            spawn_unblocked: Arc::new(tokio::sync::Notify::new()),
            re_jump,
            epoch_metrics: epoch_metrics.clone(),
            executor_metrics: executor_metrics.clone(),
            sync_metrics: sync_metrics.clone(),
            safety_halt: safety_halt.clone(),
            // No beacon plane fills a tombstone set here, and a follower neither votes
            // nor proposes, so an empty set is the honest state.
            tombstones: crate::slasher::TombstoneSet::default(),
            // No beacon plane, hence no DKG clock to diverge from the ordering tip; an
            // unregistered handle publishes nothing.
            plane_clock: crate::sync_metrics::PlaneClock::default(),
            // No beacon plane holds a receiver, so the app writes only its own watch.
            beacon_tip: None,
            timeouts: ConsensusTimeouts::fluent_1s(),
            mailbox_size: 256,
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
            marshal_floor: Some(Height::new(finalized_floor)),
            boundary_fetch,
            // Both seams exist to serve `EpochTransition`, which a follower does not run:
            // its epoch entry rides `boundary_hook`, and it never clamps a committee read
            // to a jump landing because the trigger reads at the current finalized hash.
            boundary_enter: Arc::new(|_| {}),
            boundary_read_floor: Arc::new(|_| Box::pin(async {})),
            fcu_heartbeat_interval,
            fcu_pace: Duration::from_millis(20),
            canonical_state: canonical_state.clone(),

            slasher_staking_address: staking_config.staking_address,
            committee: committee.clone(),
            slasher_sink: Arc::new(NoopSlasherSink),
            slasher_wal_partition: "slasher-wal".into(),
            // A follower runs no slasher and registers no evidence channel, so there is
            // nothing to bridge to.
            slasher_evidence: None,

            feed,

            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine: None,
        }
        // Sibling of the engine context, so the writer sits outside the engine's
        // supervision subtree.
        .build(ctx.with_label("outer_engine"))
        .await?;

        // The cold-start committee read goes through the module, which installs the
        // record and its verify-only scheme in one slot. An impossible or below-window
        // refusal is a fact about the chain and stays the loud startup error; a
        // retryable miss is this process's startup order, and the follower's boundary
        // trigger re-delivers the epoch until the module can read it.
        match cold_start_committee_read(&ctx, committee.as_ref(), initial_epoch_u64).await {
            Ok(ColdStartRead::Ready(record)) => info!(
                epoch = initial_epoch_u64,
                members = record.members.len(),
                "follower cold-start committee read through the committee module"
            ),
            Err(ColdStartRefusal::Impossible(e) | ColdStartRefusal::BelowWindow(e)) => {
                return Err(eyre!(
                    "committee[{initial_epoch_u64}] is REFUSED PERMANENTLY at this follower's \
                     committee anchor: {e}. This is a statement about chain state — the \
                     contract answered something no committed epoch can answer, or the epoch \
                     is below the module's read window — and no retry can change it; the node \
                     would follow certificates it can never verify."
                ));
            }
            Err(ColdStartRefusal::Permanent { error, attempts }) => {
                return Err(eyre!(
                    "committee[{initial_epoch_u64}] read FAILED PERMANENTLY {attempts} times at \
                     this follower's committee anchor: {error}. This class is a revert or this \
                     node's own storage fault, not a statement about the committed state — \
                     repair the cause (the staking module at GENESIS_STAKING, or this node's \
                     reth database) and restart."
                ));
            }
            Ok(ColdStartRead::Deferred(e)) => warn!(
                epoch = initial_epoch_u64,
                error = %e,
                "follower cold-start committee not readable at this node's anchor YET — a \
                 retryable miss; certificates of this epoch are deferred until the boundary \
                 trigger re-delivers it"
            ),
        }

        // Supervised: if this trigger dies the node keeps following certificates while
        // never entering another epoch, so its committee schemes freeze at the cold
        // start. Committee reads run at the current EL-finalized hash; `committee[E]`
        // not readable there leaves the epoch unconsumed for the next finalized block.
        let follower_boundary_tx = outer.boundary_sender();
        let follower_boundary_handle = {
            // Reading `committee[E]` installs the record and its verify-only scheme in
            // one slot, which is what the manager's reconcile then finds.
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
                        // `notify_one` stores a permit when nobody is waiting, so a block
                        // reported while this task is inside the read below wakes the next
                        // iteration instead of being lost.
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

        let cert_mailbox = outer.marshal_mailbox();
        let inlet_marshal = outer.marshal_mailbox();

        // The marshal resolver is upstream-backed: it backfills the by-height gap
        // between the cold-start floor and the upstream's live frontier, which the
        // inlet's live stream never carries, and the marshal BLS-verifies each delivered
        // cert. The executor is the sole reth writer from here.
        let consensus_handle = outer.start_follower(broadcast_mux, ctx.clone(), upstream.clone());

        // The cert-inlet is a follower's only producer: it drives the marshal (and so
        // the executor) plus the serving window, and fails closed when `finalized_rx`
        // closes. The defer family is registered once on the launch context so it
        // carries the launch prefix.
        let committee_read_deferred: Family<crate::cert_inlet::CommitteeReadDeferLabels, Counter> =
            Family::default();
        ctx.register(
            "dpos_cert_inlet_committee_read_deferred",
            "Upstream certs deferred (not fatal) on a committee read that could not resolve, \
             by `reason` (state_not_materialized / committee_not_committed / probe_inconsistency).",
            committee_read_deferred.clone(),
        );
        // Verify failures under a carry-forward seed pin, as opposed to genuine forged
        // upstream data.
        let carry_forward_verify_failed = Counter::default();
        ctx.register(
            "dpos_cert_inlet_carry_forward_pin_verify_failed",
            "Upstream cert BLS-verify failures where the seed pin was DERIVED by the \
             ladder (the shared store, or the minting epoch's agreement artifact) \
             rather than already held by the cached scheme.",
            carry_forward_verify_failed.clone(),
        );
        // The inlet verifies with the module's own scheme for the cert's epoch, the same
        // map the boundary trigger and the marshal use.
        let inlet_committee = committee.clone();
        let shutdown_for_inlet = shutdown.clone();
        // The same provider the epoch manager holds, and on this path that is
        // load-bearing: the σ the inlet files and the key it resolves are the same index
        // and key store the epoch manager and executor read.
        let inlet_randomness = randomness.clone();
        // After `MAX_UPSTREAM_FAULTS` consecutive unverifiable certs over a healthy
        // connection the inlet rotates to the next upstream URL, since connection-level
        // failover cannot see a bad payload on a live connection.
        let inlet_rotate: Option<crate::cert_inlet::RotateUpstream> = upstream
            .as_ref()
            .map(crate::cert_follow::CertUpstream::rotate_callback);
        // Supervised: a panic here would be swallowed by `with_catch_panics(true)`, and
        // the node would look healthy while ingesting nothing.
        let cert_inlet_handle = ctx.with_label("cert_inlet").spawn(move |c| async move {
            // Hold the WS upstream alive for the inlet's whole lifetime: the WS actor's
            // `run` loop exits the instant all `UpstreamHandle`s drop, and that actor
            // feeds `finalized_rx`.
            let _upstream_keepalive = upstream;
            // The height↔epoch bind is defense in depth: a follower trusts upstream
            // committee reads, so it pins each cert's round-epoch to its block height.
            let mut inlet = crate::cert_inlet::CertInlet::new(inlet_marshal, inlet_committee, c)
                .with_epoch_math(activation, interval)
                .with_committee_read_deferred_metric(committee_read_deferred)
                .with_carry_forward_fail_metric(carry_forward_verify_failed)
                .with_randomness(inlet_randomness);
            if let Some(rotate) = inlet_rotate {
                inlet = inlet.with_rotate(rotate);
            }
            // A connection-level auto-rotation bumps `conn_gen`; the inlet resets its
            // fault streak on the change so one URL's faults do not bleed into the next.
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
                    // Infallible by type; the fail-closed exit below is total upstream
                    // loss, a different fact.
                    Some(uf) => inlet.ingest(uf).await,
                    None => {
                        error!(
                            "cert-inlet WS stream closed (all upstreams dead); exiting fail-closed"
                        );
                        break;
                    }
                }
            }
            // Total upstream loss fails closed: cancel the shared shutdown so the host
            // brings the node down rather than hanging silently.
            shutdown_for_inlet.cancel();
        });

        Ok(DposLayerHandle {
            consensus_handle,
            cert_mailbox,
            supervised: vec![
                ("cert_inlet", cert_inlet_handle),
                ("follower_boundary", follower_boundary_handle),
                // Supervised: it parks rather than returning, so a clean exit means it
                // died, and a dead fetcher silently returns the node to vote-only
                // admission with every liveness check still green.
                ("follower_artifact_fetch", artifact_fetch_handle),
            ],
            // The beacon's drain, registered as on the validator path; a follower opens
            // no journal partition yet, so the task behind it returns at once.
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
    /// The devnet geometry: `EPOCH_BLOCK_INTERVAL=32`,
    /// `dposActivationBlock=2 * interval`, used where the target arithmetic is what
    /// the smoke case reads back out of the log.
    const DEVNET_ACTIVATION: u64 = 64;
    const DEVNET_INTERVAL: u64 = 32;
    /// Any deployed chain_id would do; the predicate lives in the node, which passes
    /// its answer in.
    const A_DEPLOYED_CHAIN: u64 = 0x5202;
    const A_LOCAL_CHAIN: u64 = 1337;

    /// A fresh-datadir follower entry — the one place with nothing local to check a
    /// peer against. On a deployed network a missing operator checkpoint is a startup
    /// refusal naming the flag; off one it is trust-on-first-use, logged as such. A
    /// checkpoint wins on either.
    ///
    /// Falsifier: a deployed network without a checkpoint that returns an entry; a
    /// local network that refuses; a checkpoint that does not become the entry.
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

    /// The empty-archive anchor: an empty consensus archive with the EL already past
    /// epoch 0, and an upstream, resolves to [`ColdStartKind::ElFinalized`], whose
    /// anchor in `launch` is reth's own finalized pair rather than the genesis or an
    /// upstream's `Latest`. The test pins the kind only, not `launch`'s anchor choice.
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

    /// The same empty archive with the EL still inside epoch 0 is the ordinary
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
        // cs_finalized == activation + interval is the first height past epoch 0
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

    /// The follower march below the activation block. Every case fixes one step of the
    /// five, in the order the arm evaluates them.
    ///
    /// [`PeerProbes`] are the two inputs that cost a peer round trip, so the peer-free
    /// steps are asserted with `PeerProbes::default()`, as the arm calls this function
    /// on its first pass.
    ///
    /// Falsifier: a march that reaches the upstream while reth already holds the
    /// activation block; a march that asks for a certificate before spending a
    /// configured operator checkpoint.
    #[test]
    fn holding_the_activation_block_is_a_local_entry_and_costs_no_peer() {
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

    /// The honest sequencer→DPoS migration: no checkpoint and no upstream, so there is
    /// nobody to ask and the node waits for reth to hold the activation block
    /// (`wait_for_activation_block`, retry-forever).
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

    /// An operator checkpoint with no upstream at all is still an entry:
    /// `ElSync::sync_to_checkpoint` takes a hash and drives reth itself, so testing
    /// `has_upstream` first would park in `wait_for_activation_block` a node whose
    /// operator had already handed it a working entry.
    ///
    /// Falsifier: any verdict but `Checkpoint`; `WaitLocal` is the permanent park.
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

    /// The operator checkpoint goes before the certificate entry: `assert_l1_checkpoint`
    /// runs after the match and is counted from the landing, and the certificate entry
    /// lands on the lowest legal height, so a certificate-first march would turn a
    /// survivable park into a fatal refusal.
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

    /// The ordering chain is not an entry yet: the upstream serves no frontier at all,
    /// its frontier is below `activation + K`, or it is the boundary `activation + K − 1`
    /// (the highest height whose certificate still carries no real EVM hash).
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

    /// The window where `setDposActivationBlock` has run and
    /// `commitEpochCommittee(0)` has not: nothing can authenticate a finalization under
    /// a committee that is not there, so the march waits instead of asking.
    ///
    /// Falsifier: a `Certificate` verdict with `e_max = None`.
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

    /// The certificate target is the highest height this node can still check:
    /// `min(tip, last(e_max))`, floored at `activation + K`, where
    /// `last(e) = activation + (e + 1) · interval − 1`. One request per attempt, aimed
    /// at the height a cascading donor's `JUMP_THRESHOLD` window and a jumped
    /// validator's archive lose last.
    ///
    /// Falsifier: a target above `last(e_max)` (the committee needed to authenticate
    /// it is not readable); a target above the tip (nobody holds it); a target below
    /// `activation + K` (no real EVM hash).
    #[test]
    fn the_certificate_target_is_the_top_of_the_checkable_window() {
        // devnet: one readable epoch, tip well past it. `last(0) = 95`, and the
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
        // minimum: the tip is exactly the first checkable height.
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
        // prod: a pre-activation block reads the whole lookahead window
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

    /// A readable window that ends below the floor is not an entry: `interval <= K`
    /// puts `last(e_max)` under `activation + K`, so no height is both
    /// committee-checkable at `rf_hash` and carries a real EVM hash. Clamping the
    /// target up instead asked for a height in an epoch whose committee is not
    /// readable, which can never authenticate.
    ///
    /// Falsifier: any `Certificate` verdict here; a `ChainBelowActivation` on the line
    /// below, where the window does reach the floor.
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

    /// A demoted engine drops its `SubReceiver`s; a re-promoted engine clones the
    /// plane's `Arc<Mutex<MuxHandle>>` and re-registers the same subchannel against the
    /// same persistent broker, with no network rebuild.
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

            let (s1, r1) = oracle
                .control(pk1.clone())
                .register(7, QUOTA)
                .await
                .unwrap();
            let (mux1, handle1) = Muxer::new(ctx.with_label("mux1"), s1, r1, 8);
            mux1.start();
            let plane_mux = Arc::new(Mutex::new(handle1));

            let (s2, r2) = oracle
                .control(pk2.clone())
                .register(7, QUOTA)
                .await
                .unwrap();
            let (mux2, mut handle2) = Muxer::new(ctx.with_label("mux2"), s2, r2, 8);
            mux2.start();
            let (mut tx2, _rx2) = handle2.register(SUBCHANNEL).await.unwrap();

            // Each promotion clones the same `plane_mux`, registers `SUBCHANNEL`,
            // receives, then drops its `SubReceiver` (auto-deregister) at scope exit.
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

    // The visibility belt: a landing read that returns `None` a few times then
    // materializes must resolve (not fatal), raise `landing_wait` while waiting, and
    // clear it on success.
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

    // A genuinely materialized-but-missing read stays fatal after the belt expires
    // (not retry-forever — a landing local fault, not correlated).
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

    // Activation-wait: a transient absence retries forever and resolves when the
    // sequencer's block lands, raising `activation_wait` while it waits and clearing it
    // on success. Never fatal on a mere transient absence.
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

    // reth missing a small tail below `target` reconnects at the highest present
    // ancestor and replays it (the flush-race path — not a defer).
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

    // reth missing more than `MAX_COLD_RECOVER` below its own archive is TooDeep: the
    // pre-engine replay cannot bridge it, so the recover fn defers to devp2p EL-sync.
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

    // With an upstream: defer to EL-sync and raise the crash-recover gauges and counter;
    // the caller anchors at reth's tip, and the archive is intact so no consensus-store
    // repair is needed.
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

    // Without an upstream there is nowhere to devp2p-backfill from, so real local data
    // loss stays fatal and no gauge is raised.
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

// The σ half of the crash-survivor replay, pinned where the decision is made: the I/O
// tail (local certificate → upstream → defer) is exercised by the smoke suite, and what
// a unit pins is that a miss never reads as "no σ here".
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
        // path's claim is that a σ miss never reads as "no σ here".
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

    // On a beacon-active link a σ miss is "go find it", never "there is no σ here": the
    // second reading is the digest fallback under another name, re-rolling `prev_randao`
    // and forking the restart away from the network.
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

    /// A committee that can produce a real seeded finalization: `n` multisig
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

    /// A 2f+1 finalization for `round` whose certificate carries the round's σ.
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

    /// A provider over an empty index, with `mint` optionally filed — the two
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

    // The archive is not trusted for σ: a certificate is taken only when its round
    // matches and it is checked under the epoch's attested key. Genuine σ under a
    // resolvable key is `Held` and filed; any σ with no key is `Pending` (the caller
    // defers); a σ that fails an attested key is `Absent`.
    #[test]
    fn the_replays_certificate_seed_is_checked_under_the_epoch_key() {
        let c = seeded_committee();
        let round = Round::new(Epoch::new(DETERMINISTIC_BOOTSTRAP_EPOCH), View::new(VIEW));
        let cert = seeded_cert(&c, round);
        let sigma = cert
            .certificate
            .seed()
            .expect("a beacon-active certificate carries the round seed");

        // The key is here and the σ is genuine: taken, and filed for the rest of the walk.
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

        // No key here: held, and the walk defers. Deriving here is the fork this arm
        // exists to prevent — a restart with an empty artifact partition is an ordinary
        // state.
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

        // A σ that fails an attested key — a tampered or corrupt archive record. The
        // genuine σ of a neighbouring round is the forgery: a decodable curve point that
        // verifies for no round here.
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

        // The round pin, unchanged: a certificate for another round carries a perfectly
        // valid signature over something else.
        let other = provider(Some(&c.outcome));
        assert!(
            matches!(
                seed_via_beacon(other.as_ref(), round, &seeded_cert(&c, neighbour)),
                CertSeed::Absent
            ),
            "σ signs the round, so a neighbour's certificate answers nothing here"
        );
    }

    // The ordinary case: σ is found under the block's own round, with no child block read.
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

    // The agreed epoch map comes first: a provider that has σ for the round is still
    // refused when the map says the beacon is not active there and the rest of the
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

    // An epoch the map cannot name at all (below the epocher origin) is inactive and is
    // never unwrapped: the beacon cannot have been mandatory in an epoch that does not
    // exist.
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

// The below-floor archive-hole heal is not a defer: with an upstream it re-populates the
// hole by a BLS-verified by-height re-fetch. These pin
// `refetch_verified_archive_hole` in isolation (the disk-archive replay it splices into
// is exercised by the smoke suite): a valid cert heals, a no-upstream / gone-everywhere /
// forged cert is fatal.
#[cfg(test)]
mod refetch_hole_tests {
    use super::refetch_verified_archive_hole;
    use crate::{
        cert_follow::{CertUpstream, UpstreamFinalized, WalkOutcome},
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

    /// The walk's three verdicts, stated directly: `height` is what the by-height pull
    /// serves; with none, `answered` says whether an upstream rendered that negative
    /// or nothing answered at all — the pair this function separates.
    #[derive(Clone)]
    struct FakeUpstream {
        height: Option<UpstreamFinalized>,
        answered: bool,
    }
    impl FakeUpstream {
        /// Serves the height (and is therefore reachable).
        fn serving(uf: UpstreamFinalized) -> Self {
            Self {
                height: Some(uf),
                answered: true,
            }
        }
        /// Answers, and says it does not hold the height — the only shape that is
        /// evidence about the record, and the only one that may exit fatal.
        fn reachable_but_missing() -> Self {
            Self {
                height: None,
                answered: true,
            }
        }
        /// Nothing answers at all. Evidence about the link, about nothing else.
        fn unreachable() -> Self {
            Self {
                height: None,
                answered: false,
            }
        }
    }
    impl CertUpstream for FakeUpstream {
        async fn get_finalization(&self, _height: Height) -> Option<UpstreamFinalized> {
            self.height.clone()
        }
        async fn get_finalization_everywhere(&self, _height: Height) -> WalkOutcome {
            match (&self.height, self.answered) {
                (Some(uf), _) => WalkOutcome::Got(Box::new(uf.clone())),
                (None, true) => WalkOutcome::MissedEverywhere,
                (None, false) => WalkOutcome::NoneAnswered,
            }
        }
        async fn get_latest(&self) -> Option<UpstreamFinalized> {
            self.height.clone()
        }
        async fn rotate(&self) {}
    }

    /// Unreachable for the first `silent_laps` by-height pulls, then normal — the
    /// operator's upstream that is not up yet when the node crash-recovers. Records the
    /// `crash_recover` gauge as seen on the second lap, so the test can assert the park
    /// was visible and not merely survived.
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
        async fn get_finalization_everywhere(&self, height: Height) -> WalkOutcome {
            // An upstream nobody can reach renders no verdict: silence is
            // `NoneAnswered`, never a miss.
            match self.get_finalization(height).await {
                Some(uf) => WalkOutcome::Got(Box::new(uf)),
                None => WalkOutcome::NoneAnswered,
            }
        }
        async fn get_latest(&self) -> Option<UpstreamFinalized> {
            use std::sync::atomic::Ordering::SeqCst;
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

    // With an upstream serving a committee-signed cert, a below-floor hole is
    // re-populated (returns the verified block and cert to splice into the replay),
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

    // Without an upstream a below-floor hole is unrecoverable local data loss: fatal.
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

    // Upstream reachable but no longer serving the below-floor height, pruned
    // everywhere: fatal gone-everywhere.
    #[test]
    fn upstream_missing_height_is_fatal() {
        deterministic::Runner::default().start(|mut ctx| async move {
            let c = committee(3);
            let up = FakeUpstream::reachable_but_missing();
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

    /// An unreachable upstream is not a data-loss verdict: the same `None` from the
    /// by-height pull as the test above, same height, same committee, and the opposite
    /// answer, because the two negatives are different facts. The block path must ask
    /// again rather than tell the operator to re-sync the EL disk.
    ///
    /// Falsifier: any form where both negatives answer alike — collapse the walk's
    /// two negatives and this fails with the gone-everywhere error it must not produce.
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

    /// The block path asks again until a verdict exists, healing the hole the moment an
    /// upstream comes up. The upstream is silent for two laps and then serves the
    /// record, which is the crash-recovery boot race: the node restarts before the
    /// validator it pulls from is listening.
    ///
    /// Two things beyond the happy end: the lap count proves it retried rather than
    /// concluded, and the gauge read taken on the second lap proves the park was
    /// observable while it waited.
    ///
    /// Falsifier: any form where both negatives answer alike — the first lap exits with
    /// gone-everywhere and no second lap happens.
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

    // A forged cert (signed by a different committee than the trust anchor reads) fails
    // BLS authentication: fatal, so the re-fetch cannot be steered by a malicious
    // upstream.
    #[test]
    fn forged_cert_fails_authentication() {
        deterministic::Runner::default().start(|mut ctx| async move {
            let signer_committee = committee(4);
            let trust_committee = committee(5); // a different committee
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

    /// `boundary_hook` fires per finalized block — about once a second in production —
    /// while `reconcile_roles` prunes, re-registers and sweeps on each delivery. Reds if
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

    /// An unreadable `committee[E]` leaves the epoch unconsumed for the next block, so
    /// its scheme registration is not skipped; reds if `last_delivered` advances anyway.
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

    /// A closed boundary receiver means the epoch manager has exited. The trigger stops
    /// and the supervisor takes the node down, rather than spinning on a dead channel for
    /// every finalized block. Reds if the send result stops being propagated.
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

    /// `T` on the follower is `EpochTransition`'s rule over the module's geometry,
    /// checked at the two points where the rule differs: on an epoch terminal ET tracks
    /// `epoch_of(fin) + 1`, and mid-epoch `epoch_of(fin)`. A follower runs no
    /// `EpochTransition`, so this is the one place the two can drift.
    ///
    /// The terminal point is the case that matters: a node parked on an epoch terminal
    /// would, with `T = epoch_of(fin)`, name a rung it already holds.
    ///
    /// Falsifier: `epoch_of(fin)` at a terminal (one rung too low); `epoch_of(fin) + 1`
    /// mid-epoch (a rung outside the node's own committee read window).
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
mod cold_start_read_tests {
    use super::{
        cold_start_committee_read, ColdStartRead, ColdStartRefusal, COLD_START_READ_ATTEMPTS,
        COLD_START_READ_BACKOFF,
    };
    use crate::committee::{testing::SchemeCommittee, CommitteeError, CommitteeRecord, Member};
    use alloy_primitives::{Address, B256};
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
    use commonware_math::algebra::Random as _;
    use commonware_runtime::{deterministic, Clock as _, Runner as _};
    use commonware_utils::{ordered::Set, TryFromIterator as _};
    use fluentbase_staking_reader::ReadError;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;
    use std::sync::Arc;

    const EPOCH: u64 = 5;

    fn record() -> CommitteeRecord {
        let mut rng = StdRng::seed_from_u64(0xC0);
        let peers: Vec<_> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng).public_key())
            .collect();
        let snap = fluentbase_staking_reader::reader::ValidatorSetSnapshot {
            block_hash: B256::ZERO,
            block_number: 0,
            epoch: EPOCH,
            validators: peers
                .iter()
                .enumerate()
                .map(
                    |(i, peer)| fluentbase_staking_reader::reader::ValidatorWithKeys {
                        address: Address::repeat_byte(i as u8),
                        keys: fluentbase_staking_reader::reader::ConsensusKeys {
                            peer_pubkey: peer.clone(),
                            bls_pubkey: {
                                use commonware_codec::DecodeExt as _;
                                let mut r = StdRng::seed_from_u64(0xB1 + i as u64);
                                let kp =
                                    fluentbase_bls::keys::ValidatorBlsKeypair::generate(&mut r);
                                fluentbase_bls::BlsPubkey::decode(kp.public_bytes().as_slice())
                                    .unwrap()
                            },
                            activation_epoch: 0,
                        },
                        tombstoned: false,
                    },
                )
                .collect(),
            weights: Some(vec![1; 4]),
        };
        CommitteeRecord {
            epoch: EPOCH,
            members: snap
                .validators
                .iter()
                .map(|v| Member {
                    address: v.address,
                    peer: v.keys.peer_pubkey.clone(),
                    bls: v.keys.bls_pubkey,
                })
                .collect(),
            weights: vec![1; 4],
            changed: false,
            snapshot: (0, B256::ZERO),
            participants: Set::try_from_iter(peers.iter().cloned()).unwrap(),
            bls: crate::scheme::epoch_committee_from_snapshot(&snap).unwrap(),
        }
    }

    fn module(faults: Vec<ReadError>) -> Arc<SchemeCommittee> {
        let record = record();
        let module = SchemeCommittee::with_geometry(|_| None, move |_| Some(record.clone()), None);
        module.fault_reads(EPOCH, faults);
        module
    }

    fn backend() -> ReadError {
        ReadError::Backend("no state found".into())
    }

    /// Two faults, then the record: the read is ready on the third attempt, after two
    /// backoffs on the runtime clock.
    #[test]
    fn a_permanent_fault_is_re_asked_and_the_third_answer_is_taken() {
        deterministic::Runner::default().start(|ctx| async move {
            let module = module(vec![backend(), ReadError::CallReverted("revert".into())]);
            let t0 = ctx.current();
            let out = cold_start_committee_read(&ctx, module.as_ref(), EPOCH).await;
            assert!(
                matches!(out, Ok(ColdStartRead::Ready(ref r)) if r.epoch == EPOCH),
                "the record after the faults"
            );
            assert_eq!(
                ctx.current().duration_since(t0).unwrap(),
                COLD_START_READ_BACKOFF * 2,
                "one pause per failed attempt"
            );
        });
    }

    /// Every attempt faults: fatal after the last one, naming the class and the count.
    #[test]
    fn a_fault_that_survives_every_attempt_is_fatal_after_the_last() {
        deterministic::Runner::default().start(|ctx| async move {
            let module = module((0..COLD_START_READ_ATTEMPTS).map(|_| backend()).collect());
            let t0 = ctx.current();
            let out = cold_start_committee_read(&ctx, module.as_ref(), EPOCH).await;
            match out {
                Err(ColdStartRefusal::Permanent { error, attempts }) => {
                    assert_eq!(attempts, COLD_START_READ_ATTEMPTS);
                    assert!(matches!(error, CommitteeError::Read(ReadError::Backend(_))));
                }
                Err(other) => panic!("wrong refusal: {other:?}"),
                Ok(_) => panic!("the read must not succeed"),
            }
            assert_eq!(
                ctx.current().duration_since(t0).unwrap(),
                COLD_START_READ_BACKOFF * (COLD_START_READ_ATTEMPTS - 1),
                "no pause after the last attempt"
            );
        });
    }

    /// A transient miss and an impossible answer are neither retried nor paused: the
    /// first is deferred to the launch's own retry, the second is refused at once.
    #[test]
    fn a_transient_miss_is_deferred_and_an_impossible_answer_refused_without_a_pause() {
        deterministic::Runner::default().start(|ctx| async move {
            let t0 = ctx.current();
            let transient = module(vec![ReadError::StateNotMaterialized { hash: B256::ZERO }]);
            assert!(matches!(
                cold_start_committee_read(&ctx, transient.as_ref(), EPOCH).await,
                Ok(ColdStartRead::Deferred(CommitteeError::Read(
                    ReadError::StateNotMaterialized { .. }
                )))
            ));
            let impossible = module(vec![ReadError::PeerKey]);
            assert!(matches!(
                cold_start_committee_read(&ctx, impossible.as_ref(), EPOCH).await,
                Err(ColdStartRefusal::Impossible(CommitteeError::Read(
                    ReadError::PeerKey
                )))
            ));
            let below =
                SchemeCommittee::with_window(|_| None, |_| None, None, (EPOCH + 1, EPOCH + 3));
            assert!(matches!(
                cold_start_committee_read(&ctx, below.as_ref(), EPOCH).await,
                Err(ColdStartRefusal::BelowWindow(
                    CommitteeError::OutOfWindow { .. }
                ))
            ));
            assert_eq!(
                ctx.current().duration_since(t0).unwrap(),
                std::time::Duration::ZERO,
                "no attempt above waits"
            );
        });
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

    /// The live seam of the tier rule, tested where production runs it.
    ///
    /// `GatedReceiver` is the only thing between the network and a channel's decode, so
    /// by the time `slasher::gossip::ingest_batch` or `beacon::actor::on_message` sees
    /// a frame this has already ruled on its sender, and the tier checks inside those
    /// two are unreachable in production.
    ///
    /// Every arm of `admits` at once, on one queue, so the assertion is the surviving
    /// sequence rather than a per-frame boolean.
    ///
    /// Falsifier: a tombstoned, untracked or registry-tier sender reaching `recv` on a
    /// committee channel; a committee member not reaching it.
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

        // Before the first `track` the window has no membership opinion, so nothing
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

    /// The mask a `Member` carries is per-epoch, which the second half of the rule
    /// (each channel's own entry) reads. The gate itself does not look at it — a member
    /// of any carried record passes the transport seam — so the two halves cannot be
    /// collapsed into one.
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
