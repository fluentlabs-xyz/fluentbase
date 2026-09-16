//! `Stand`: N `OuterEngine`s on one deterministic runner + one simulated network.

#[cfg(feature = "dpos-devnet-byzantine")]
use super::fakes::{ByzFacts, ByzReport};
use super::{
    capture::{self, Captured, Sink},
    fakes::{
        genesis_sealed, BodyTap, BranchCommittees, CountingHandler, CountingUpstream, ElEvent,
        ElNetwork, FakeBeacon, FakeChain, FakeDeriver, FakeStaking, FrontierSteps, JumpCall,
        JumpCalls, JumpElSync, Members, NoSink, NoTxs, Pull, StakingReads, TapReceiver,
        UpstreamCounters, UpstreamStats, DPOS_ACTIVATION_BLOCK,
    },
};
/// A read of one held epoch-key artifact's wire bytes, for the stand's serving side.
type ArtifactSource = Arc<dyn Fn(u64) -> Option<Vec<u8>> + Send + Sync>;

#[cfg(feature = "dpos-devnet-byzantine")]
use crate::beacon::testing::CommitteeFor;
use crate::cert_follow::UpstreamFinalized;
use crate::{
    application::ExecutedChain as _,
    beacon::{
        self,
        testing::{absent, BeaconMessage, DkgBody, DkgCeremony, DkgMsg, StaticRandomness},
        CommitteeReads, Seed, ValidatorInputs,
    },
    cert_follow::CertUpstream as _,
    cold_start_jump::JumpOutcome,
    epoch_manager::EpochEngineMetrics,
    executor::{ExecutorMetrics, FrontierProbeFn, ReJump, ReJumpFn},
    extra_data::decode_production_record,
    order_block::{anchor_order_block, OrderBlock},
    outer::{MarshalMailbox, OuterBuilder},
    plane_upstream::{new_bridge, FrontierMarshal as _, PlaneUpstreamHandle},
    slasher::TombstoneSet,
    sync_metrics::{PlaneClock, SafetyHalt, SyncMetrics},
    timeouts::ConsensusTimeouts,
};
use alloy_primitives::{Address, B256};
use commonware_codec::{DecodeExt as _, Encode as _};
use commonware_consensus::types::{Epoch, Height};
use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
use commonware_math::algebra::Random as _;
use commonware_p2p::{
    simulated::{Config as SimConfig, Link, Network},
    utils::mux::{Builder as _, Muxer},
    Manager as _, Recipients, Sender as _,
};
use commonware_runtime::{
    deterministic, Clock as _, Metrics as _, Quota, Runner as _, Spawner as _,
};
use commonware_utils::{ordered::Set, NZUsize};
use fluentbase_bls::{keys::ValidatorBlsKeypair, BlsPubkey, PeerPubkey};
use fluentbase_p2p::constants::{
    BEACON_CHANNEL, BEACON_RESOLVER_CHANNEL, BROADCAST_CHANNEL, CERT_CHANNEL, FRONTIER_CHANNEL,
    MARSHAL_CHANNEL, MAX_REGISTRY_PEER_SET, RESOLVER_CHANNEL, VOTE_CHANNEL,
};
use fluentbase_staking_reader::{
    epoch_transition::{PeerSetSink, TrackedPeers, TransitionOutcome, PENDING_RETRY_BACKOFF},
    reader::ValidatorSetSnapshot,
    EpochTransition,
};
use fluentbase_types::staking_protocol::{epoch_at_block, MAX_COMMITTEE_LOOKAHEAD_EPOCHS};
use metrics_util::debugging::{DebugValue, Snapshotter};
use rand_08::{rngs::StdRng, SeedableRng as _};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::{NonZeroU32, NonZeroU64},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, Notify};

pub(super) const CHAIN_ID: u64 = 20_994;
const QUOTA: Quota = Quota::per_second(NonZeroU32::MAX);
const MUX_MAILBOX: usize = 256;
/// How often the driver samples the nodes (virtual time).
const POLL: Duration = Duration::from_millis(100);

#[derive(Clone)]
pub(super) struct StandConfig {
    pub n: usize,
    /// Seeds the deterministic runtime and the node keys.
    pub seed: u64,
    pub epoch_len: u64,
    pub latency: Duration,
    /// Per-link drop probability.
    pub loss: f64,
    /// `epoch → member indices`. `None` past the schedule (no committee).
    pub committees: Committees,
    /// The branching committee schedule — see [`BranchCommittees`]. `None` leaves
    /// [`Self::committees`] as the whole answer; `Some` overrides it.
    pub committees_by_branch: Option<BranchCommittees>,
    /// One epoch for which the fake contract answers `weights: None` despite an
    /// intact ring frame; `None` = never.
    pub weights_none_for: Option<u64>,
    /// One epoch whose committee read reverts; the node must keep running.
    /// `None` = never.
    pub reverts_for: Option<u64>,
    /// `(node, from height)` pairs the fake contract reports tombstoned from that
    /// height on.
    pub tombstoned: Vec<(usize, u64)>,
    /// Snapshotter over the caller's `metrics::counter!` recorder, drained by the
    /// stand immediately before the collect phase and left in
    /// [`Outcome::metrics_before_collect`].
    ///
    /// The collect phase calls `Committee::committee` once per node per epoch,
    /// which increments the same counters, so a caller-side drain after
    /// `run_until` returns cannot isolate the run.
    pub metrics_snapshotter: Option<Snapshotter>,
    /// `true` leaves every node's per-epoch journals on the unprefixed production
    /// names (`consensus_epoch_{E}`), shared across the N nodes.
    pub shared_engine_partitions: bool,
    /// What the oracle tracks as the peer set.
    pub peer_set: PeerSet,
    /// Which randomness each node runs — see [`Beacon`].
    pub beacon: Beacon,
    /// Root of the per-node `share_dir`s of a `Beacon::Live` stand. Must be a real
    /// directory: `beacon::build` reloads shares through `std::fs`.
    pub share_root: PathBuf,
    /// The `(height, executed hash)` marker of the finalized block this node's
    /// execution layer persisted; `None` = a fresh chain. Block bodies are not
    /// persisted — the replay re-derives them — and the cold-start epoch is
    /// derived from `height`, not configured.
    pub resume_from: Option<(u64, B256)>,
    /// The steady-state re-jump gate; `None` pins it at `u64::MAX`, so the jump
    /// never fires and every node keeps a contiguous chain.
    pub re_jump_threshold: Option<u64>,
    /// `Some((source, victim))` removes every other upstream-plane link of
    /// `source` and `victim`, confining the victim's frontier discovery to the
    /// source; the consensus plane is untouched.
    pub upstream_only_link: Option<(usize, usize)>,
    /// `Some((victim, source))` removes every upstream-plane link of `victim`
    /// except the one to `source`, which stays fully linked.
    pub upstream_source_only_for: Option<(usize, usize)>,
    /// After the run, scan each node's marshal archive up to the highest tip into
    /// [`Outcome::archive`]. Off by default.
    pub archive_scan: bool,
    /// Sample each node's marshal tip once per driver tick into
    /// [`Outcome::marshal_tip_series`]. Read from the marshal's own
    /// `finalized_height` gauge because asking the marshal adds a message to its
    /// select loop and changes the run.
    pub marshal_tip_series: bool,
    /// Which nodes run the production [`crate::cert_inlet::CertInlet`] as a second
    /// producer into their own marshal — see [`CertInletCfg`].
    pub cert_inlet: Option<CertInletCfg>,
}

/// Which nodes stand up a cert-inlet and which shape of production input they
/// are fed.
///
/// Two of the three sources feed off the node's own frontier plane, so
/// [`StandConfig::upstream_only_link`] and
/// [`StandConfig::upstream_source_only_for`] decide who may answer its pulls.
/// [`CertInletSource::PeerArchive`] reads its donor's archive directly instead.
#[derive(Clone, Debug, Default)]
pub(super) struct CertInletCfg {
    /// The node indices that run an inlet.
    pub nodes: Vec<usize>,
    /// Which certificate the inlet is handed — see [`CertInletSource`].
    pub source: CertInletSource,
}

/// The three shapes of the one stream a production inlet consumes.
///
/// Production hands the inlet a live stream of the upstream's newest
/// finalizations, never a by-height walk; the stand has no WS actor, so the first
/// two shapes are built from [`crate::cert_follow::CertUpstream`]. Neither can
/// reach the inlet's non-fault deferral: `FrontierHandler::deliver` drops the
/// same committee refusals one layer up, capping a plane feeder at the top of
/// this node's own read window, which is why [`Self::PeerArchive`] exists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CertInletSource {
    /// The next height above this node's own tier-F, pulled by height
    /// (`CertUpstream::get_finalization`).
    #[default]
    NextAboveTier,
    /// The upstream's live frontier (`CertUpstream::get_latest`) — production's
    /// own inlet input.
    Frontier,
    /// Node `from`'s marshal archive, walked upward by height
    /// (`FrontierMarshal::pair_at` on that node's own `MarshalMailbox`). The only
    /// shape with no `deliver` gate, so it can hand the inlet a certificate of an
    /// epoch this node's committee anchor cannot read.
    PeerArchive {
        /// The donor node, whose marshal archive is read directly.
        from: usize,
    },
}

/// What one node's cert-inlet task did, snapshotted into [`CertInletFacts`] at
/// collect time.
#[derive(Clone, Default)]
struct CertInletObs {
    /// Certificates handed to [`crate::cert_inlet::CertInlet::ingest`]; every
    /// outcome of one is a skip, so this counts attempts.
    ingests: Arc<AtomicU64>,
    /// [`crate::cert_inlet::RotateUpstream`] invocations, the only externally
    /// visible effect of `record_data_fault` reaching `MAX_UPSTREAM_FAULTS`.
    rotations: Arc<AtomicU64>,
    /// Every height the inlet handed its marshal, in order, recorded at
    /// [`crate::cert_inlet::MarshalSink::verify_block`] — the verify gate on the
    /// clean path.
    delivered: Arc<Mutex<Vec<u64>>>,
    /// The inlet's own `dpos_cert_inlet_committee_read_deferred_total` counter,
    /// shared through `with_committee_read_deferred_metric`.
    defers: prometheus_client::metrics::family::Family<
        crate::cert_inlet::CommitteeReadDeferLabels,
        prometheus_client::metrics::counter::Counter,
    >,
    /// The inlet's own carry-forward verify-failure counter, shared through
    /// `with_carry_forward_fail_metric`.
    carry_forward_fails: prometheus_client::metrics::counter::Counter,
}

impl CertInletObs {
    fn snapshot(&self) -> CertInletFacts {
        CertInletFacts {
            ingests: self.ingests.load(Ordering::SeqCst),
            rotations: self.rotations.load(Ordering::SeqCst),
            delivered: self.delivered.lock().unwrap().clone(),
            defers: self
                .defers
                .get_or_create(&crate::cert_inlet::CommitteeReadDeferLabels {
                    reason: crate::cert_inlet::DEFER_COMMITTEE_NOT_COMMITTED,
                })
                .get(),
            carry_forward_fails: self.carry_forward_fails.get(),
        }
    }
}

/// What one node's cert-inlet did over the whole run — `None` on a node that ran
/// no inlet. See [`CertInletObs`] for what each number is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CertInletFacts {
    pub ingests: u64,
    pub rotations: u64,
    pub delivered: Vec<u64>,
    pub defers: u64,
    pub carry_forward_fails: u64,
}

/// The inlet's marshal behind the production `MarshalSink` seam, recording every
/// height handed to `verify_block`; all other calls forward unchanged.
#[derive(Clone)]
struct RecordingSink {
    inner: MarshalMailbox,
    delivered: Arc<Mutex<Vec<u64>>>,
}

impl crate::cert_inlet::MarshalSink for RecordingSink {
    async fn verify_block(&mut self, round: commonware_consensus::types::Round, block: OrderBlock) {
        self.delivered.lock().unwrap().push(block.height);
        self.inner.verify_block(round, block).await
    }

    async fn report_finalization(
        &mut self,
        finalization: commonware_consensus::simplex::types::Finalization<
            fluentbase_bls::Scheme,
            crate::digest::Digest,
        >,
    ) {
        self.inner.report_finalization(finalization).await
    }
}

/// The [`OuterBuilder::blocker`] slot for the consensus plane. One label serves
/// both consumers because the builder carries one field.
pub(super) const BLOCKER_SITE_CONSENSUS: &str = "consensus";
/// The frontier plane's own resolver blocker slot.
pub(super) const BLOCKER_SITE_FRONTIER: &str = "frontier";

/// Every [`commonware_p2p::Blocker::block`] one node's stack made, and every
/// blocker slot the spy was wired into.
///
/// A counting spy rather than a no-op blocker, because "no peer was excluded"
/// holds vacuously on a no-op for any code. [`Self::sites`] catches a slot that
/// silently reverted to `NoopBlocker`, which would leave [`Self::calls`] empty
/// and every assertion on it green.
#[derive(Clone, Default)]
pub(super) struct BlockerSpy {
    calls: Arc<Mutex<Vec<(&'static str, PeerPubkey)>>>,
    sites: Arc<Mutex<BTreeSet<&'static str>>>,
}

impl BlockerSpy {
    /// Hand this spy to one blocker slot, recording that the slot took it.
    pub(super) fn at(&self, site: &'static str) -> SpyBlocker {
        self.sites.lock().unwrap().insert(site);
        SpyBlocker {
            site,
            calls: self.calls.clone(),
        }
    }

    fn snapshot(&self) -> BlockerFacts {
        BlockerFacts {
            calls: self.calls.lock().unwrap().clone(),
            sites: self.sites.lock().unwrap().iter().copied().collect(),
        }
    }
}

/// One wired blocker slot of one node: counts, never blocks — see
/// [`BlockerSpy`].
#[derive(Clone)]
pub(super) struct SpyBlocker {
    site: &'static str,
    calls: Arc<Mutex<Vec<(&'static str, PeerPubkey)>>>,
}

impl commonware_p2p::Blocker for SpyBlocker {
    type PublicKey = PeerPubkey;

    async fn block(&mut self, peer: Self::PublicKey) {
        self.calls.lock().unwrap().push((self.site, peer));
    }
}

/// What one node's blockers did over the whole run. See [`BlockerSpy`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct BlockerFacts {
    /// `(site, peer)` per `Blocker::block` call, in call order.
    pub calls: Vec<(&'static str, PeerPubkey)>,
    /// The slots this node's spy was wired into, sorted. Empty means the spy was
    /// wired nowhere, which makes `calls` say nothing.
    pub sites: Vec<&'static str>,
}

/// The randomness surface every node runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Beacon {
    /// `StaticRandomness`: a fixed anonymous sharing dealt from the full node set,
    /// with σ on demand for any epoch and no DKG plane.
    Static,
    /// The production beacon plane (`beacon::build`) on every node: a live DKG on
    /// `BEACON_CHANNEL` / `BEACON_RESOLVER_CHANNEL`, epoch-key agreement on the
    /// plane's mux sub-channels, and journals under a per-node prefix.
    Live,
}

/// The tracked peer set of the consensus plane and what happens to the links of a
/// node that falls out of it.
///
/// The stand runs a consensus plane and an upstream plane over one simulated
/// network, so "no consensus-plane link, but a link to a frontier provider" is
/// expressible. The upstream plane always tracks every node at index 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PeerSet {
    /// Every node, tracked once at index 0.
    AllNodes,
    /// `committee[E]` only, re-tracked at index `E` when the chain enters epoch
    /// `E`; the consensus-plane links of a node outside the set are removed.
    /// `upstream_link` says whether the outsider keeps its `FRONTIER_CHANNEL`
    /// links.
    Committee { upstream_link: bool },
    /// `committee[E]` tracked as above, but every link left in place — the
    /// boundary between the simulated network and the authenticated transport.
    CommitteeTrackedOnly,
}

impl StandConfig {
    pub(super) fn honest(n: usize, seed: u64) -> Self {
        Self {
            n,
            seed,
            epoch_len: 32,
            latency: Duration::from_millis(10),
            loss: 0.0,
            committees: Committees::All,
            committees_by_branch: None,
            weights_none_for: None,
            reverts_for: None,
            tombstoned: Vec::new(),
            metrics_snapshotter: None,
            shared_engine_partitions: false,
            peer_set: PeerSet::AllNodes,
            beacon: Beacon::Static,
            share_root: PathBuf::new(),
            resume_from: None,
            re_jump_threshold: None,
            upstream_only_link: None,
            upstream_source_only_for: None,
            archive_scan: false,
            marshal_tip_series: false,
            cert_inlet: None,
        }
    }

    /// [`Self::honest`] with the live beacon plane and a fresh share root.
    pub(super) fn live(n: usize, seed: u64) -> Self {
        use std::sync::atomic::AtomicU64;
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let nonce = SEQ.fetch_add(1, Ordering::Relaxed);
        let share_root =
            std::env::temp_dir().join(format!("fluent-testbed-{}-{nonce}", std::process::id()));
        let _ = std::fs::remove_dir_all(&share_root);
        Self {
            beacon: Beacon::Live,
            share_root,
            ..Self::honest(n, seed)
        }
    }
}

/// Who sits in each epoch's committee — the stand's input. When an epoch becomes
/// readable, and at which state hash, is [`FakeStaking`]'s answer.
#[derive(Clone)]
pub(super) enum Committees {
    /// Every node, every epoch.
    All,
    /// `(epoch, n) → members`.
    Schedule(Arc<dyn Fn(u64, usize) -> Option<Vec<usize>> + Send + Sync>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Honest,
    /// The block derived at `at` seals to a hash nobody else derives.
    DivergentResult {
        at: u64,
    },
    /// The devnet vote equivocator on the vote channel.
    #[cfg(feature = "dpos-devnet-byzantine")]
    Equivocate,
    /// This node deals twice: its `BEACON_CHANNEL` sender sends the original
    /// revelation to every other member and a second, validly signed log of the
    /// same dealer over the same `Info` to node 0. `withhold_partials` rebuilds the
    /// signer scheme over the verify-only oracle, so it casts no vote.
    #[cfg(feature = "dpos-devnet-byzantine")]
    TwoReveals {
        withhold_partials: bool,
    },
    /// This node's frontier-plane producer serves `Finalized{h}` with a forged σ
    /// slot for `h` in [`byzantine_roles::FORGE_WINDOW`], leaving the multisig
    /// half untouched.
    #[cfg(feature = "dpos-devnet-byzantine")]
    ForgedSeedUpstream,
    /// This node's frontier-plane producer answers `FrontierKey::Latest` with the
    /// real tip whose `block.height` is inflated by
    /// [`byzantine_roles::LATEST_INFLATION`].
    #[cfg(feature = "dpos-devnet-byzantine")]
    InflatedProbe,
    /// This node's frontier-plane producer answers `FrontierKey::Latest` with the
    /// real tip whose `block.result` points at a branch the honest devp2p peers do
    /// not serve.
    #[cfg(feature = "dpos-devnet-byzantine")]
    LyingUpstream,
    /// This node's frontier-plane producer answers a `Finalized{h}` by-height pull
    /// (for `h` in [`byzantine_roles::WRONG_HEIGHT_WINDOW`]) with its own valid
    /// pair of height `h − 1`.
    #[cfg(feature = "dpos-devnet-byzantine")]
    WrongHeightFinalized,
    /// `Beacon::Live` only: this node brings up no beacon plane and runs
    /// `beacon::absent` — a verifier that never signs, from epoch 0 on.
    AbsentBeacon,
    /// `Beacon::Live` only: a registry-tier neighbour in no committee that deals a
    /// real `Commitment` for the epoch after its own. Every member's pre-decode
    /// gate classifies it `Tracked` and refuses the frame, so the actor never sees
    /// it; `Outcome::stray_dealer_sends[i]` counts what it put on the wire.
    StrayDealer,
}

/// Which of the two simulated networks a [`Partition`] cuts.
///
/// `Both` is a physical partition: neither plane reaches the isolated side.
/// `ConsensusOnly` removes only the consensus-plane links (votes, the marshal's
/// resolver, the beacon channels), leaving `FRONTIER_CHANNEL` links in place. It
/// is the only remaining way to follow the chain without an epoch's key: the key
/// is an artifact any node can ask for over the consensus plane, so
/// non-membership alone does not park execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CutPlanes {
    /// Both planes — a machine that is simply gone.
    Both,
    /// The consensus plane only; the frontier plane keeps delivering.
    ConsensusOnly,
}

#[derive(Clone, Debug)]
pub(super) struct Partition {
    pub a: Vec<usize>,
    pub b: Vec<usize>,
    /// Cut once every node's executed tip is at or above this height.
    pub after_height: u64,
    pub duration: Duration,
    /// Heal once the highest executed tip in the run is at or above this height,
    /// instead of after [`Self::duration`].
    pub heal_above: Option<u64>,
    /// Which planes the cut removes — see [`CutPlanes`].
    pub planes: CutPlanes,
}

pub(super) struct PartitionCfg<'a>(&'a mut Partition);

impl PartitionCfg<'_> {
    pub(super) fn after_height(self, h: u64) -> Self {
        self.0.after_height = h;
        self
    }
    /// Cut the consensus plane only and leave the frontier plane alive — see
    /// [`CutPlanes::ConsensusOnly`].
    pub(super) fn consensus_only(self) -> Self {
        self.0.planes = CutPlanes::ConsensusOnly;
        self
    }
    /// Hold the cut for `views` leader timeouts of `ConsensusTimeouts::fluent_1s`.
    pub(super) fn for_views(self, views: u32) -> Self {
        self.0.duration = ConsensusTimeouts::fluent_1s().leader * views;
        self
    }
    /// Hold the cut until the chain the isolated node is missing reaches `height`.
    ///
    /// A duration only reaches a block lag through the pacing, which varies; a
    /// heal that lands after the run's stopping predicate is a cut that never
    /// healed.
    pub(super) fn heal_above(self, height: u64) -> Self {
        self.0.heal_above = Some(height);
        self
    }
}

pub(super) struct NodeCfg<'a>(&'a mut Role);

impl NodeCfg<'_> {
    pub(super) fn role(self, role: Role) {
        *self.0 = role;
    }
}

/// One finalized block as node `i` saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TraceEntry {
    pub height: u64,
    pub view: u64,
    pub leader: Option<u8>,
    pub digest: B256,
    /// Executed hash at `height` at the end of the run.
    pub hash: Option<B256>,
}

/// One epoch's committee record as the module froze it, reduced to the legs
/// [`crate::committee::CommitteeRecord::same_value`] compares.
///
/// `members` is in contract order, not the sorted projection: two nodes that
/// agree on the set but not the order hold different committees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CommitteeFacts {
    pub members: Vec<PeerPubkey>,
    pub weights: Vec<u128>,
    pub changed: bool,
    /// `(height, executed hash)` this node read the record at — a diagnostic leg
    /// that cross-node equality assertions must exclude.
    pub anchor: (u64, B256),
}

/// Why the module refused an epoch, in the two terms a consumer routes on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CommitteeRefusal {
    pub error: String,
    pub transient: bool,
}

/// One `metrics::counter!` family instance as the debugging recorder holds it:
/// name, labels, value.
pub(super) type CounterSample = (String, Vec<(String, String)>, u64);

/// Every counter the recorder holds right now, taken once.
///
/// `Snapshotter::snapshot` resets every counter it reads, so a second call answers
/// zero for what the first took; draining into a `Vec` is the only way to assert
/// about several families over one window.
pub(super) fn drain_counters(snap: &Snapshotter) -> Vec<CounterSample> {
    snap.snapshot()
        .into_vec()
        .into_iter()
        .filter_map(|(k, .., v)| {
            let key = k.key();
            let DebugValue::Counter(value) = v else {
                return None;
            };
            Some((
                key.name().to_string(),
                key.labels()
                    .map(|l| (l.key().to_string(), l.value().to_string()))
                    .collect(),
                value,
            ))
        })
        .collect()
}

/// The total of one counter family in `drained`, optionally restricted to one
/// label pair.
///
/// Process-wide, not per node: `metrics::counter!` carries no node label and the
/// stand's nodes share one recorder, so every assertion is a sum over nodes.
pub(super) fn counter_of(
    drained: &[CounterSample],
    name: &str,
    label: Option<(&str, &str)>,
) -> u64 {
    drained
        .iter()
        .filter(|(n, labels, _)| {
            n == name && label.is_none_or(|(lk, lv)| labels.iter().any(|(k, v)| k == lk && v == lv))
        })
        .map(|(.., v)| *v)
        .sum()
}

/// [`counter_of`] restricted to every label pair in `labels` at once — for a
/// family carrying more than one label (`dpos_ingress_dropped_total` has `channel`
/// and `reason`).
pub(super) fn counter_where(drained: &[CounterSample], name: &str, labels: &[(&str, &str)]) -> u64 {
    drained
        .iter()
        .filter(|(n, have, _)| {
            n == name
                && labels
                    .iter()
                    .all(|(lk, lv)| have.iter().any(|(k, v)| k == lk && v == lv))
        })
        .map(|(.., v)| *v)
        .sum()
}

/// What one node's marshal archives hold at a height, as the by-height serve
/// path reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ArchiveEntry {
    pub finalization: bool,
    pub block: bool,
}

/// What the driver's predicate sees.
pub(super) struct Progress {
    /// Tier-F (finalized-executed) tip per node.
    pub heights: Vec<u64>,
    pub halted: Vec<bool>,
    pub elapsed: Duration,
}

impl Progress {
    pub(super) fn min_height(&self) -> u64 {
        self.heights.iter().copied().min().unwrap_or(0)
    }
    pub(super) fn min_height_of(&self, nodes: &[usize]) -> u64 {
        nodes.iter().map(|&i| self.heights[i]).min().unwrap_or(0)
    }
}

#[derive(Clone, Debug)]
pub(super) struct PartitionObservation {
    pub heights_at_cut: Vec<u64>,
    pub heights_at_heal: Vec<u64>,
    pub cut_at: Duration,
    pub healed_at: Duration,
}

/// The first height at which the nodes' executed hashes disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Divergence {
    /// A strict majority agrees on one hash at `height`; `node` is the lowest
    /// index holding another.
    Minority { node: usize, height: u64 },
    /// No hash has a strict majority at `height` (a 2×2 split, or 1×1): there is
    /// no "wrong" node to name — `hashes` are the distinct values, sorted.
    Tie { height: u64, hashes: Vec<B256> },
}

/// One node's `PeerSetSink` log: `(epoch, primary, secondary)` per `track` call,
/// in call order. Both tiers, because which tier a key lands in is observable.
pub(super) type TrackedRegistrations = Vec<(u64, Vec<PeerPubkey>, Vec<PeerPubkey>)>;

pub(super) struct Outcome {
    /// Tier-F (finalized-executed) tip per node.
    pub heights: Vec<u64>,
    /// `hashes[i][h - 1]` = `(h, finalized-executed hash)` for `h in 1..=heights[i]`.
    pub hashes: Vec<Vec<(u64, B256)>>,
    /// The first height whose executed hashes disagree across nodes.
    pub diverged: Option<Divergence>,
    /// What each node's upstream plane did (both ends, see `UpstreamCounters`).
    pub upstream: Vec<UpstreamStats>,
    /// `upstream_served[i]` = every by-height pull node `i`'s upstream client made,
    /// in order — see `Pull`.
    pub upstream_served: Vec<Vec<Pull>>,
    /// Every node whose `SafetyHalt` latch is engaged, with the typed reason.
    pub halted: Vec<(usize, String)>,
    pub traces: Vec<Vec<TraceEntry>>,
    /// `seeds[i][h]` = the σ node `i`'s executor derived height `h` from, for
    /// every `h <= heights[i]` (`None` = seedless).
    pub seeds: Vec<BTreeMap<u64, Option<Seed>>>,
    /// `artifacts[i][e]` = the wire bytes of the agreed epoch-key artifact node `i`
    /// holds for epoch `e` (empty otherwise).
    pub artifacts: Vec<BTreeMap<u64, Vec<u8>>>,
    /// `signable[i]` = the epochs node `i`'s beacon answers `ShareProbe::Ready` for at the end.
    pub signable: Vec<Vec<u64>>,
    /// `et_steps[i]` = every `cold_start` / `on_finalized` node `i`'s
    /// `EpochTransition` answered, in call order.
    pub et_steps: Vec<Vec<EtStep>>,
    /// `et_boundaries[i]` = every boundary trigger node `i`'s transition
    /// delivered to its epoch manager, with the state hash it read at.
    pub et_boundaries: Vec<Vec<EtBoundary>>,
    /// `geometry[i]` = `EpochTransition::frozen_geometry()` of node `i` at the
    /// end of the run: `(dposActivationBlock, epochBlockInterval)` or `None`
    /// when nothing froze it.
    pub geometry: Vec<Option<(u64, u64)>>,
    /// `peer_sets[i]` = every registration node `i`'s transition handed its
    /// `PeerSetSink`, primary and secondary tiers apart.
    pub peer_sets: Vec<TrackedRegistrations>,
    /// How many times two nodes' transitions tracked different peer sets for the
    /// same epoch. Must be zero.
    pub tracked_mismatches: u64,
    /// Peer-set registrations that actually reached the simulated network.
    pub tracked_forwarded: u64,
    /// `staking_reads[i]` = what node `i` asked the fake staking state, split by
    /// whether the epoch was committed at the read height.
    pub staking_reads: Vec<StakingReads>,
    /// `committee_records[i][e]` = what node `i`'s committee module answers for
    /// epoch `e`, up to the two-epoch lookahead. Read at the end of the run, after
    /// [`Self::staking_reads`], so fresh reads do not land in it.
    pub committee_records: Vec<BTreeMap<u64, Result<CommitteeFacts, CommitteeRefusal>>>,
    /// `committee_verifier_epochs[i]` = the epochs node `i`'s module holds a
    /// verify-only scheme for, at the end of the run. A map lookup, no read.
    pub committee_verifier_epochs: Vec<Vec<u64>>,
    /// `committees[i]` = node `i`'s own committee module, held past the run for
    /// questions the two maps above cannot answer (chiefly
    /// [`crate::committee::Committee::scheme`]).
    pub committees: Vec<Arc<dyn crate::committee::Committee>>,
    /// Every `metrics::counter!` value the run produced, drained before the collect
    /// phase. Empty unless [`StandConfig::metrics_snapshotter`] was set.
    pub metrics_before_collect: Vec<CounterSample>,
    /// `jump_calls[i]` = every steady-state re-jump call node `i` made, with the
    /// returned `JumpOutcome` variant — see `fakes::JumpCall`.
    pub jump_calls: Vec<Vec<JumpCall>>,
    /// `el_events[i]` = every EL tier transition node `i` made, in order — see
    /// [`fakes::ElEvent`](super::fakes::ElEvent).
    pub el_events: Vec<Vec<ElEvent>>,
    /// `marshal_tip_series[i]` = node `i`'s marshal tip sampled once per driver
    /// tick, in order. Empty unless [`StandConfig::marshal_tip_series`]; see that
    /// field for why the sample is the marshal's own gauge and not a mailbox read.
    pub marshal_tip_series: Vec<Vec<u64>>,
    /// `frontier_steps_named_series[i][t]` = how many ladder steps node `i`'s probe
    /// had named by driver tick `t`, a running length of [`Self::frontier_steps`].
    pub frontier_steps_named_series: Vec<Vec<usize>>,
    /// `probe_calls[i]` = how many times node `i`'s frozen-tip probe asked the
    /// upstream.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub probe_calls: Vec<u64>,
    /// `frontier_steps[i]` = every ladder step node `i`'s frozen-tip probe named,
    /// as `(T, last(T+1))` in call order, one entry per tick. Whether the executor
    /// then hands the step to the marshal is decided inside `executor::probe_frontier`
    /// and pinned in `executor.rs`'s unit tests instead.
    pub frontier_steps: Vec<Vec<(u64, u64)>>,
    /// `cert_inlet[i]` = what node `i`'s cert-inlet task did, or `None` when the
    /// node ran none. See [`CertInletFacts`].
    pub cert_inlet: Vec<Option<CertInletFacts>>,
    /// `blocked[i]` = every peer node `i`'s stack asked to block, per wired blocker
    /// slot, plus the slots that took the spy. See [`BlockerSpy`].
    pub blocked: Vec<BlockerFacts>,
    /// `byz[i]` = what node `i`'s byzantine wrappers did — all zeros on an honest
    /// node.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub byz: Vec<ByzFacts>,
    /// `head_gap_series[i]` = `best − ordering_finalized` of node `i`, sampled once
    /// per driver tick: the canonical head of its EL minus the height its executor
    /// last finalized-executed. A measurement, never an assertion.
    pub head_gap_series: Vec<Vec<u64>>,
    /// `head_gap_on_fcu[i]` = `gap → count` over every FCU that moved node
    /// `i`'s canonical chain (`FakeChain::head_gap_on_fcu`).
    pub head_gap_on_fcu: Vec<BTreeMap<u64, u64>>,
    /// `head_gap_on_landing[i]` = the same over every jump landing.
    pub head_gap_on_landing: Vec<BTreeMap<u64, u64>>,
    /// `archive[i][h]` = whether node `i`'s marshal holds a finalization certificate
    /// and a finalized block at height `h` at the end of the run. Empty unless
    /// `StandConfig::archive_scan`.
    pub archive: Vec<BTreeMap<u64, ArchiveEntry>>,
    /// `bodies[i]` = distinct payloads node `i` received on the broadcast
    /// channel, by `(mux sub-channel, sender)` — see [`fakes::BodyTap`].
    pub bodies: Vec<BTreeMap<(u64, PeerPubkey), usize>>,
    /// The runner's Prometheus text at the end of the run (every node's
    /// families under its `node{i}_` label).
    pub metrics: String,
    pub logs: Vec<Captured>,
    /// `stray_dealer_sends[i]` = the `(frame, recipient)` pairs node `i` put on
    /// `BEACON_CHANNEL` as a `Role::StrayDealer`; zero for any other role.
    pub stray_dealer_sends: Vec<u64>,
    /// `tombstones_observed[i]` = every `(height, peer)` node `i`'s
    /// `TombstoneSet::observe` newly recorded off the committee snapshot it read at
    /// that finalized height. Empty unless `StandConfig::tombstoned` names someone.
    pub tombstones_observed: Vec<Vec<(u64, PeerPubkey)>>,
    /// How many times the simulated network failed to return a send-ack — see
    /// [`Outcome::SIMULATOR_ACK_DROP`].
    pub simulator_ack_drops: u64,
    pub log_capture_live: bool,
    pub partitions: Vec<PartitionObservation>,
    pub timed_out: bool,
    pub virtual_elapsed: Duration,
    pub real_elapsed: Duration,
}

impl Outcome {
    /// Every node not in `except` executed the same hash at every height they
    /// all reached, and their tips are within one block of each other.
    pub(super) fn assert_lockstep_except(&self, except: &[usize]) {
        let nodes: Vec<usize> = (0..self.heights.len())
            .filter(|i| !except.contains(i))
            .collect();
        let min = nodes.iter().map(|&i| self.heights[i]).min().unwrap();
        let max = nodes.iter().map(|&i| self.heights[i]).max().unwrap();
        assert!(
            max - min <= 1,
            "tips drifted apart: {:?} (nodes {nodes:?})",
            self.heights
        );
        for h in 1..=min {
            let hashes: Vec<B256> = nodes
                .iter()
                .map(|&i| self.hashes[i][(h - 1) as usize].1)
                .collect();
            assert!(
                hashes.iter().all(|x| *x == hashes[0]),
                "executed hashes disagree at height {h} across nodes {nodes:?}: {hashes:?}"
            );
        }
    }

    /// The one ERROR line the stand does not count against a run, matched on the
    /// full `target: message` prefix.
    ///
    /// `simulated::Sender::send` logs it when a task is dropped between enqueue and
    /// ack; nothing is lost on the wire. The same text is printed when the network
    /// drops a message whose origin is untracked, which must not be exempt —
    /// [`Self::SIMULATOR_MESSAGE_DROPPED`] distinguishes the two.
    pub(super) const SIMULATOR_ACK_DROP: &'static str =
        "commonware_p2p::simulated::network: failed to send ack";
    /// The dropped-message form of the same line — never exempt. See above.
    const SIMULATOR_MESSAGE_DROPPED: &'static str =
        "commonware_p2p::simulated::network: failed to send ack err=[]";

    fn is_exempt_ack_drop(line: &Captured) -> bool {
        line.level == tracing::Level::ERROR
            && line.text.starts_with(Self::SIMULATOR_ACK_DROP)
            && !line.text.starts_with(Self::SIMULATOR_MESSAGE_DROPPED)
    }

    pub(super) fn errors(&self) -> Vec<&Captured> {
        self.logs
            .iter()
            .filter(|l| l.level == tracing::Level::ERROR)
            .filter(|l| !Self::is_exempt_ack_drop(l))
            .collect()
    }

    pub(super) fn logs_containing(&self, needle: &str) -> Vec<&Captured> {
        self.logs
            .iter()
            .filter(|l| l.text.contains(needle))
            .collect()
    }

    /// The `(height, view, leader, digest, hash, σ)` trace of node `i`, as bytes,
    /// for cross-run comparison. σ is the seed the executor derived the height from
    /// (`0xFF` = none).
    pub(super) fn trace_bytes(&self, i: usize) -> Vec<u8> {
        use commonware_codec::Encode as _;
        let mut out = Vec::new();
        for e in &self.traces[i] {
            out.extend_from_slice(&e.height.to_be_bytes());
            out.extend_from_slice(&e.view.to_be_bytes());
            out.push(e.leader.unwrap_or(0xFF));
            out.extend_from_slice(e.digest.as_slice());
            out.extend_from_slice(e.hash.unwrap_or(B256::ZERO).as_slice());
            match self.seeds[i].get(&e.height).cloned().flatten() {
                Some(seed) => out.extend_from_slice(&seed.encode()),
                None => out.push(0xFF),
            }
        }
        out
    }

    /// Every `commonware_resolver::p2p` engine's `peers_blocked` gauge in the run,
    /// as `(node, family, value)`.
    ///
    /// The gauge is written in one place, the select loop's top-of-iteration arm,
    /// so a zero says no exclusion had been taken as of that engine's last
    /// iteration; an exclusion taken in the final arm is not in the exposition.
    /// Unlike [`BlockerSpy`] this covers every resolver the runner registered,
    /// including ones the stand hands no blocker to, but carries no peer identity.
    /// Panics on a value that does not parse or a `_peers_blocked` family not under
    /// a `node{i}_` chain.
    pub(super) fn peers_blocked(&self) -> Vec<(usize, String, f64)> {
        self.metrics
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once(' ')?;
                if !key.ends_with("_peers_blocked") {
                    return None;
                }
                // Suffix first, then the node: a `peers_blocked` family that is
                // not under a `node{i}_` chain is a family this stand does not
                // know how to attribute, and it must not vanish into "nothing
                // was excluded".
                let node: usize = key
                    .strip_prefix("node")
                    .and_then(|rest| rest.split_once('_'))
                    .and_then(|(i, _)| i.parse().ok())
                    .unwrap_or_else(|| {
                        panic!("{key}: a peers_blocked family outside the node{{i}}_ chain")
                    });
                let value: f64 = value
                    .trim()
                    .parse()
                    .unwrap_or_else(|e| panic!("{key}: unparsable gauge value {value:?}: {e}"));
                Some((node, key.to_string(), value))
            })
            .collect()
    }

    /// The value of one metric family of node `i` (`node{i}_<name>` in the
    /// runner's exposition), `None` when the family is not registered.
    pub(super) fn metric(&self, i: usize, name: &str) -> Option<f64> {
        // prometheus-client appends `_total` to a counter's registered name.
        let key = format!("node{i}_{name}");
        let counter = format!("{key}_total");
        self.metrics.lines().find_map(|line| {
            let (k, v) = line.split_once(' ')?;
            (k == key || k == counter)
                .then(|| v.trim().parse().ok())
                .flatten()
        })
    }
}

pub(super) struct Stand {
    cfg: StandConfig,
    roles: Vec<Role>,
    partitions: Vec<Partition>,
}

type Pred = Box<dyn Fn(&Progress) -> bool + Send>;

impl Stand {
    pub(super) fn new(cfg: StandConfig) -> Self {
        let roles = vec![Role::Honest; cfg.n];
        Self {
            cfg,
            roles,
            partitions: Vec::new(),
        }
    }

    pub(super) fn node(&mut self, i: usize) -> NodeCfg<'_> {
        NodeCfg(&mut self.roles[i])
    }

    pub(super) fn partition(&mut self, a: &[usize], b: &[usize]) -> PartitionCfg<'_> {
        self.partitions.push(Partition {
            a: a.to_vec(),
            b: b.to_vec(),
            after_height: 0,
            duration: Duration::from_secs(5),
            heal_above: None,
            planes: CutPlanes::Both,
        });
        PartitionCfg(self.partitions.last_mut().unwrap())
    }

    /// Run on a fresh deterministic runner until `pred` holds or `virtual_timeout`
    /// of virtual time has passed.
    pub(super) fn run_until(
        self,
        pred: impl Fn(&Progress) -> bool + Send + 'static,
        virtual_timeout: Duration,
    ) -> Outcome {
        let (outcome, _checkpoint) = self.run_until_recover(pred, virtual_timeout);
        outcome
    }

    /// [`Self::run_until`], also returning the runner's checkpoint (storage, rng,
    /// clock) so [`Self::replay`] can restart the nodes over the same journals.
    pub(super) fn run_until_recover(
        self,
        pred: impl Fn(&Progress) -> bool + Send + 'static,
        virtual_timeout: Duration,
    ) -> (Outcome, deterministic::Checkpoint) {
        let runner =
            deterministic::Runner::new(deterministic::Config::default().with_seed(self.cfg.seed));
        let started = Instant::now();
        let (sink, live) = capture::install();
        let pred: Pred = Box::new(pred);
        let (mut outcome, checkpoint) = runner.start_and_recover(|ctx| async move {
            drive(
                ctx,
                self.cfg,
                self.roles,
                self.partitions,
                pred,
                virtual_timeout,
                sink,
                live,
            )
            .await
        });
        outcome.real_elapsed = started.elapsed();
        (outcome, checkpoint)
    }

    /// Rebuild every node (same keys, same config) over `checkpoint`'s storage —
    /// "drop the process, replay the journals".
    pub(super) fn replay(
        self,
        checkpoint: deterministic::Checkpoint,
        pred: impl Fn(&Progress) -> bool + Send + 'static,
        virtual_timeout: Duration,
    ) -> Outcome {
        let runner = deterministic::Runner::from(checkpoint);
        let started = Instant::now();
        let (sink, live) = capture::install();
        let pred: Pred = Box::new(pred);
        let mut outcome = runner.start(|ctx| async move {
            drive(
                ctx,
                self.cfg,
                self.roles,
                self.partitions,
                pred,
                virtual_timeout,
                sink,
                live,
            )
            .await
        });
        outcome.real_elapsed = started.elapsed();
        outcome
    }
}

/// Node keys: a function of the stand seed only, so a replay gets the same set.
pub(super) fn keys(seed: u64, n: usize) -> (Vec<Ed25519PrivateKey>, Vec<ValidatorBlsKeypair>) {
    let mut rng = StdRng::seed_from_u64(seed ^ 0x5741_4e44_4b45_5953);
    let peers = (0..n)
        .map(|_| Ed25519PrivateKey::random(&mut rng))
        .collect();
    let bls = (0..n)
        .map(|_| ValidatorBlsKeypair::generate(&mut rng))
        .collect();
    (peers, bls)
}

/// One `EpochTransition` call and what it answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct EtStep {
    /// The finalized height handed to `cold_start` / `on_finalized`.
    pub number: u64,
    pub outcome: Result<TransitionOutcome, String>,
}

/// One boundary trigger the transition delivered: the epoch and the state it read
/// the committee at. `block_hash` is the executed hash of `boundary − result_lag`,
/// or the cold-start anchor for the first entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EtBoundary {
    pub epoch: u64,
    pub block_hash: B256,
    pub block_number: u64,
}

/// What every node's `EpochTransition` did, per node.
#[derive(Clone, Default)]
struct EtObserver {
    steps: Arc<Mutex<Vec<EtStep>>>,
    boundaries: Arc<Mutex<Vec<EtBoundary>>>,
    /// `EpochTransition::frozen_geometry()` sampled after every call, because
    /// reading it at collection time needs an `.await` that lets torn-down tasks
    /// emit one more round of network sends.
    geometry: Arc<Mutex<Option<(u64, u64)>>>,
}

/// The `PeerSetSink` half of the production wiring: `EpochTransition` hands the
/// assembled `TrackedPeers` set here and the simulated network's `Manager::track`
/// takes it, as the node hands it to the real `Oracle`.
///
/// Two differences come from having one simulated `Oracle` stand in for N
/// production ones: only the first node's registration of an epoch reaches the
/// network, and a set identical to the last one registered is not re-registered
/// (four tracked sets are retained, and re-registering drops in-flight acks).
#[derive(Clone)]
struct TrackSink {
    node: usize,
    oracle: Oracle,
    shared: Arc<Mutex<Tracked>>,
    /// This node's ingress window, recorded on every `track` before the set is
    /// forwarded, as production's `OracleHandle::track` does. Per node: the one
    /// simulated Oracle forwards only the first registration, but every node's own
    /// transition tracks.
    window: fluentbase_p2p::TrackedWindow,
}

#[derive(Default)]
struct Tracked {
    /// `epoch -> the set the first node's transition tracked for it`. Seeded with
    /// the epoch-0 set the harness registers before any node exists, so the
    /// transitions' own epoch-0 track is a no-op against it.
    by_epoch: BTreeMap<u64, Vec<PeerPubkey>>,
    /// `epoch -> (primary, secondary)` as the first node to reach that epoch split
    /// them. Separate from [`Self::by_epoch`] because that one is the union, which
    /// cannot see two nodes agreeing on who is tracked while disagreeing on which
    /// tier each is in.
    tiers_by_epoch: BTreeMap<u64, (Vec<PeerPubkey>, Vec<PeerPubkey>)>,
    /// `per_node[i]` = every registration node `i`'s transition tracked.
    per_node: Vec<TrackedRegistrations>,
    /// How many times a node tracked a set for an epoch whose tier split differs
    /// from the one already recorded: both tiers are a function of chain state
    /// alone, so a difference means two nodes read different committees.
    mismatches: u64,
    /// How many peer-set registrations actually reached the simulated network —
    /// the only events that can cost an ack (see `Outcome::SIMULATOR_ACK_DROP`).
    forwarded: u64,
}

impl PeerSetSink for TrackSink {
    fn track(
        &mut self,
        epoch: u64,
        peers: TrackedPeers,
    ) -> impl core::future::Future<Output = ()> + Send {
        // Record before registering: a frame from a member of the new set may
        // arrive the instant the network applies it.
        self.window.record(epoch, &peers);
        let primary = peers.primary();
        let primary_members: Vec<PeerPubkey> = primary.iter().cloned().collect();
        let secondary_members: Vec<PeerPubkey> = peers.secondary.iter().cloned().collect();
        // `by_epoch` (what the link-severing model reads) is both tiers: the
        // authenticated transport keeps a connection to a secondary peer too.
        let connectable: Vec<PeerPubkey> = {
            let mut all = primary_members.clone();
            all.extend(secondary_members.iter().cloned());
            all.sort();
            all.dedup();
            all
        };
        let forward = {
            let mut shared = self.shared.lock().unwrap();
            // The disagreement counter reads the tiers, not the union: two nodes
            // that put the same peer in different tiers have read different
            // committees.
            let tiers = (primary_members.clone(), secondary_members.clone());
            match shared.tiers_by_epoch.get(&epoch) {
                Some(recorded) if *recorded != tiers => shared.mismatches += 1,
                Some(_) => {}
                None => {
                    shared.tiers_by_epoch.insert(epoch, tiers);
                }
            }
            shared.per_node[self.node].push((epoch, primary_members, secondary_members));
            let changed = shared
                .by_epoch
                .last_key_value()
                .is_none_or(|(_, last)| *last != connectable);
            match shared.by_epoch.get(&epoch) {
                Some(_) => false,
                None => {
                    shared.by_epoch.insert(epoch, connectable);
                    if changed {
                        shared.forwarded += 1;
                    }
                    changed
                }
            }
        };
        let mut manager = self.oracle.manager();
        let registered = commonware_p2p::TrackedPeers::new(primary, peers.secondary);
        async move {
            if forward {
                manager.track(epoch, registered).await;
            }
        }
    }
}

struct NodeHandles {
    chain: FakeChain,
    halt: SafetyHalt,
    /// Every steady-state re-jump call the node made, with its outcome variant.
    jump_calls: JumpCalls,
    /// Frozen-tip probe invocations.
    #[cfg(feature = "dpos-devnet-byzantine")]
    probe_calls: Arc<AtomicU64>,
    /// Every ladder step this node's probe named — see `Outcome::frontier_steps`.
    frontier_steps: FrontierSteps,
    trace: Arc<Mutex<Vec<TraceEntry>>>,
    upstream: UpstreamCounters,
    /// `Beacon::Live`: the beacon's artifact read.
    artifacts: Option<ArtifactSource>,
    /// The node's one beacon, for the post-run share-gate read.
    beacon: Arc<dyn beacon::Beacon>,
    observer: EtObserver,
    staking: FakeStaking,
    /// The node's one committee module, for the post-run record scan.
    committee: Arc<dyn crate::committee::Committee>,
    /// The node's marshal, for the post-run archive scan.
    marshal: Arc<OnceLock<MarshalMailbox>>,
    /// The optional cert-inlet task's observables — `None` when this node runs
    /// no inlet.
    cert_inlet: Option<CertInletObs>,
    /// This node's one blocker spy, handed to both blocker slots.
    blocker: BlockerSpy,
    bodies: BodyTap,
    /// `(frame, recipient)` pairs this node's stray dealer put on the wire
    /// (`Role::StrayDealer`); zero for every other role.
    stray_sends: Arc<AtomicU64>,
    /// `(height, peer)` per tombstone this node's set newly observed.
    tombstones_observed: Arc<Mutex<Vec<(u64, PeerPubkey)>>>,
    #[cfg(feature = "dpos-devnet-byzantine")]
    byz: ByzReport,
}

#[allow(clippy::too_many_arguments)]
async fn drive(
    ctx: deterministic::Context,
    cfg: StandConfig,
    roles: Vec<Role>,
    partitions: Vec<Partition>,
    pred: Pred,
    virtual_timeout: Duration,
    sink: Sink,
    log_capture_live: bool,
) -> Outcome {
    let n = cfg.n;
    let (peers, bls) = keys(cfg.seed, n);
    let pks: Vec<PeerPubkey> = peers.iter().map(|p| p.public_key()).collect();

    let members: Members = {
        let committees = cfg.committees.clone();
        Arc::new(move |epoch: u64| match &committees {
            Committees::All => Some((0..n).collect::<Vec<_>>()),
            Committees::Schedule(f) => f(epoch, n),
        })
    };
    // `getRegistryWithKeys()`. `PeerSet::AllNodes` is the production shape: a
    // registry holding every activated validator, so a rotated-out member stays in
    // the tracked union. The committee-only configurations empty it, modelling a
    // node that is not in the registry at all.
    let registry: Vec<PeerPubkey> = match cfg.peer_set {
        PeerSet::AllNodes => pks.clone(),
        PeerSet::Committee { .. } | PeerSet::CommitteeTrackedOnly => Vec::new(),
    };
    let bls_pubkeys: Vec<BlsPubkey> = bls
        .iter()
        .map(|k| BlsPubkey::decode(k.public_bytes().as_slice()).expect("bls pubkey"))
        .collect();
    // Seeded in the same order the sink records: `PeerSetSink::track` takes a
    // sorted set, so seeding in node-index order would read as a disagreement with
    // every node.
    let tracked = Arc::new(Mutex::new(Tracked {
        by_epoch: BTreeMap::from([(
            0u64,
            Set::from_iter_dedup(pks.iter().cloned())
                .iter()
                .cloned()
                .collect::<Vec<PeerPubkey>>(),
        )]),
        tiers_by_epoch: BTreeMap::new(),
        per_node: vec![Vec::new(); n],
        mismatches: 0,
        forwarded: 0,
    }));

    // The consensus plane and the upstream plane. Every node is a tracked peer of
    // every other one on both at index 0, and every pair is linked on both.
    let sim_config = || SimConfig {
        max_size: 4 * 1024 * 1024,
        disconnect_on_block: false,
        tracked_peer_sets: NZUsize!(4),
    };
    let (network, oracle) = Network::<_, PeerPubkey>::new(ctx.with_label("net"), sim_config());
    network.start();
    let (upstream_network, upstream_oracle) =
        Network::<_, PeerPubkey>::new(ctx.with_label("upstream_net"), sim_config());
    upstream_network.start();
    let link = Link {
        latency: cfg.latency,
        jitter: Duration::ZERO,
        success_rate: 1.0 - cfg.loss,
    };
    let mut linked: std::collections::HashSet<(PeerPubkey, PeerPubkey)> =
        std::collections::HashSet::new();
    for o in [&oracle, &upstream_oracle] {
        o.manager()
            .track(0, Set::from_iter_dedup(pks.iter().cloned()))
            .await;
        for a in &pks {
            for b in &pks {
                if a != b {
                    o.add_link(a.clone(), b.clone(), link.clone())
                        .await
                        .expect("link");
                }
            }
        }
    }
    for a in &pks {
        for b in &pks {
            if a != b {
                linked.insert((a.clone(), b.clone()));
            }
        }
    }

    // Confine the victim's frontier discovery to the one source on the upstream
    // plane: remove every upstream link touching `source` or `victim` except the
    // pair between them. The consensus plane is left intact.
    if let Some((source, victim)) = cfg.upstream_only_link {
        for a in [source, victim] {
            for b in 0..n {
                let keep = (a == source && b == victim) || (a == victim && b == source);
                if a != b && !keep {
                    upstream_oracle
                        .remove_link(pks[a].clone(), pks[b].clone())
                        .await
                        .expect("remove upstream link");
                }
            }
        }
    }

    // One-sided isolation: the victim can pull by-height from exactly one peer,
    // which keeps every other link it had.
    if let Some((victim, source)) = cfg.upstream_source_only_for {
        for b in 0..n {
            if b != victim && b != source {
                upstream_oracle
                    .remove_link(pks[victim].clone(), pks[b].clone())
                    .await
                    .expect("remove upstream link");
                upstream_oracle
                    .remove_link(pks[b].clone(), pks[victim].clone())
                    .await
                    .expect("remove upstream link");
            }
        }
    }

    // The stand's devp2p EL peer, shared by every node — see `ElNetwork`.
    let el_network = ElNetwork::default();
    let genesis_sealed = genesis_sealed();
    let genesis_hash = genesis_sealed.hash();
    let genesis_block = anchor_order_block(&genesis_sealed).expect("anchor");

    // One marshal slot per node, built here rather than inside `build_node`: a
    // cert-inlet on the `PeerArchive` source reads another node's archive, and
    // that node's engine may be built after this one's.
    let marshal_slots: Vec<Arc<OnceLock<MarshalMailbox>>> =
        (0..n).map(|_| Arc::new(OnceLock::new())).collect();
    let mut nodes = Vec::with_capacity(n);
    for (i, role) in roles.iter().enumerate() {
        let handles = build_node(
            &ctx,
            i,
            &cfg,
            *role,
            &peers,
            &bls,
            &oracle,
            &upstream_oracle,
            members.clone(),
            el_network.clone(),
            &pks,
            &bls_pubkeys,
            registry.clone(),
            tracked.clone(),
            genesis_hash,
            genesis_block.clone(),
            &marshal_slots,
        )
        .await;
        nodes.push(handles);
    }

    enum PartState {
        Pending,
        Active(Duration),
        Healed,
    }
    let mut part_state: Vec<PartState> = partitions.iter().map(|_| PartState::Pending).collect();
    // What each cut actually removed, per plane, so the heal restores exactly that
    // and no more: a pair can already be unlinked when the cut fires, and
    // re-adding it at the heal would hand the isolated node a source the fixture
    // deliberately took away. `true` = consensus plane, `false` = upstream plane.
    let mut cut_pairs: Vec<Vec<(usize, usize, bool)>> =
        partitions.iter().map(|_| Vec::new()).collect();
    let mut part_obs: Vec<PartitionObservation> = partitions
        .iter()
        .map(|_| PartitionObservation {
            heights_at_cut: vec![],
            heights_at_heal: vec![],
            cut_at: Duration::ZERO,
            healed_at: Duration::ZERO,
        })
        .collect();
    let t0 = ctx.current();
    let mut timed_out = false;
    let mut tracked_epoch = 0u64;
    // Per-node samples of the marshal tip, one per driver tick, with the running
    // count of ladder steps named beside it — see `Outcome::marshal_tip_series`.
    // Both are reads of state the driver already holds, so neither sends a message
    // into a node.
    let mut tip_series: Vec<Vec<u64>> = vec![Vec::new(); n];
    let mut steps_named_series: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut head_gap_series: Vec<Vec<u64>> = vec![Vec::new(); n];
    let elapsed_now =
        |ctx: &deterministic::Context| ctx.current().duration_since(t0).unwrap_or_default();
    loop {
        if cfg.marshal_tip_series {
            let exposition = ctx.encode();
            for (i, node) in nodes.iter().enumerate() {
                tip_series[i].push(marshal_tip_of(&exposition, i));
                steps_named_series[i].push(node.frontier_steps.lock().unwrap().len());
            }
        }
        for (i, node) in nodes.iter().enumerate() {
            head_gap_series[i].push(node.chain.executed_tip().saturating_sub(node.chain.tip()));
        }
        let progress = Progress {
            heights: nodes.iter().map(|h| h.chain.tip()).collect(),
            halted: nodes.iter().map(|h| h.halt.is_engaged()).collect(),
            elapsed: elapsed_now(&ctx),
        };
        for (p, part) in partitions.iter().enumerate() {
            match part_state[p] {
                PartState::Pending if progress.min_height() >= part.after_height => {
                    for (x, y) in cross_pairs(part) {
                        for (consensus_plane, o) in cut_oracles(part, &oracle, &upstream_oracle) {
                            // A pair that was not linked is not an error: the cut is
                            // "no path between these two", and there already is none.
                            if o.remove_link(pks[x].clone(), pks[y].clone()).await.is_ok() {
                                cut_pairs[p].push((x, y, consensus_plane));
                            }
                        }
                    }
                    part_obs[p].heights_at_cut = progress.heights.clone();
                    part_obs[p].cut_at = progress.elapsed;
                    part_state[p] = PartState::Active(progress.elapsed);
                }
                PartState::Active(since)
                    if match part.heal_above {
                        Some(h) => progress.heights.iter().copied().max().unwrap_or(0) >= h,
                        None => progress.elapsed >= since + part.duration,
                    } =>
                {
                    for (x, y, consensus_plane) in cut_pairs[p].drain(..) {
                        let o = if consensus_plane {
                            &oracle
                        } else {
                            &upstream_oracle
                        };
                        o.add_link(pks[x].clone(), pks[y].clone(), link.clone())
                            .await
                            .expect("add link");
                    }
                    part_obs[p].heights_at_heal = progress.heights.clone();
                    part_obs[p].healed_at = progress.elapsed;
                    part_state[p] = PartState::Healed;
                }
                _ => {}
            }
        }
        if cfg.peer_set != PeerSet::AllNodes {
            // The links follow what the nodes' `EpochTransition`s actually tracked,
            // never the schedule: the tracked set is the state machine's own
            // `TrackedPeers` — primary three committee records plus the secondary
            // registry — and both tiers are what the authenticated transport keeps
            // connections for.
            let latest = {
                let t = tracked.lock().unwrap();
                t.by_epoch.last_key_value().map(|(e, m)| (*e, m.clone()))
            };
            if let Some((epoch, members)) = latest {
                if epoch > tracked_epoch {
                    {
                        // Sever every consensus-plane link that touches a node
                        // outside the set (and its upstream-plane links unless the
                        // config keeps them); restore the links among members.
                        let (sever, sever_upstream) = match cfg.peer_set {
                            PeerSet::AllNodes => unreachable!(),
                            PeerSet::Committee { upstream_link } => (true, !upstream_link),
                            PeerSet::CommitteeTrackedOnly => (false, false),
                        };
                        for a in &pks {
                            for b in &pks {
                                if a == b || !sever {
                                    continue;
                                }
                                let inside = members.contains(a) && members.contains(b);
                                let is_linked = linked.contains(&(a.clone(), b.clone()));
                                if inside && !is_linked {
                                    oracle
                                        .add_link(a.clone(), b.clone(), link.clone())
                                        .await
                                        .expect("add link");
                                    if sever_upstream {
                                        upstream_oracle
                                            .add_link(a.clone(), b.clone(), link.clone())
                                            .await
                                            .expect("add upstream link");
                                    }
                                    linked.insert((a.clone(), b.clone()));
                                } else if !inside && is_linked {
                                    oracle
                                        .remove_link(a.clone(), b.clone())
                                        .await
                                        .expect("remove link");
                                    if sever_upstream {
                                        upstream_oracle
                                            .remove_link(a.clone(), b.clone())
                                            .await
                                            .expect("remove upstream link");
                                    }
                                    linked.remove(&(a.clone(), b.clone()));
                                }
                            }
                        }
                    }
                    tracked_epoch = epoch;
                }
            }
        }
        if pred(&progress) {
            break;
        }
        if progress.elapsed >= virtual_timeout {
            timed_out = true;
            break;
        }
        ctx.sleep(POLL).await;
    }

    let heights: Vec<u64> = nodes.iter().map(|h| h.chain.tip()).collect();
    let hashes: Vec<Vec<(u64, B256)>> = nodes
        .iter()
        .zip(&heights)
        .map(|(node, &tip)| {
            (1..=tip)
                .map(|h| (h, node.chain.hash_at(h).unwrap_or(B256::ZERO)))
                .collect()
        })
        .collect();
    let diverged = first_divergence(&heights, &hashes);
    let halted = nodes
        .iter()
        .enumerate()
        .filter(|(_, h)| h.halt.is_engaged())
        .map(|(i, h)| (i, format!("{:?}", h.halt.reason())))
        .collect();
    let traces = nodes
        .iter()
        .map(|node| {
            let mut t = node.trace.lock().unwrap().clone();
            for e in &mut t {
                e.hash = node.chain.hash_at(e.height);
            }
            t
        })
        .collect();
    let logs = sink.lock().unwrap().clone();
    let simulator_ack_drops = logs
        .iter()
        .filter(|l| Outcome::is_exempt_ack_drop(l))
        .count() as u64;
    let upstream = nodes.iter().map(|h| h.upstream.snapshot()).collect();
    let upstream_served = nodes
        .iter()
        .map(|h| h.upstream.served_heights.lock().unwrap().clone())
        .collect();
    let seeds = nodes
        .iter()
        .zip(&heights)
        .map(|(node, &tip)| (0..=tip).map(|h| (h, node.chain.seed_at(h))).collect())
        .collect();
    let max_epoch =
        epoch_at_block(heights.iter().copied().max().unwrap_or(0), 0, cfg.epoch_len).unwrap_or(0);
    let artifacts = nodes
        .iter()
        .map(|node| match &node.artifacts {
            Some(read) => (0..=max_epoch + 2)
                .filter_map(|e| read(e).map(|bytes| (e, bytes)))
                .collect(),
            None => BTreeMap::new(),
        })
        .collect();
    let signable: Vec<Vec<u64>> = nodes
        .iter()
        .map(|node| {
            (0..=max_epoch + 2)
                .filter(|e| {
                    node.beacon.can_participate(Epoch::new(*e)) == beacon::ShareProbe::Ready
                })
                .collect()
        })
        .collect();
    let geometry: Vec<Option<(u64, u64)>> = nodes
        .iter()
        .map(|node| *node.observer.geometry.lock().unwrap())
        .collect();
    let et_steps: Vec<Vec<EtStep>> = nodes
        .iter()
        .map(|node| node.observer.steps.lock().unwrap().clone())
        .collect();
    let et_boundaries: Vec<Vec<EtBoundary>> = nodes
        .iter()
        .map(|node| node.observer.boundaries.lock().unwrap().clone())
        .collect();
    let staking_reads: Vec<StakingReads> = nodes.iter().map(|node| node.staking.reads()).collect();
    let jump_calls: Vec<Vec<JumpCall>> = nodes
        .iter()
        .map(|node| node.jump_calls.lock().unwrap().clone())
        .collect();
    let el_events: Vec<Vec<ElEvent>> = nodes.iter().map(|node| node.chain.el_events()).collect();
    let stray_dealer_sends: Vec<u64> = nodes
        .iter()
        .map(|node| node.stray_sends.load(Ordering::SeqCst))
        .collect();
    let tombstones_observed: Vec<Vec<(u64, PeerPubkey)>> = nodes
        .iter()
        .map(|node| node.tombstones_observed.lock().unwrap().clone())
        .collect();
    #[cfg(feature = "dpos-devnet-byzantine")]
    let probe_calls: Vec<u64> = nodes
        .iter()
        .map(|node| node.probe_calls.load(Ordering::SeqCst))
        .collect();
    #[cfg(feature = "dpos-devnet-byzantine")]
    let byz: Vec<ByzFacts> = nodes.iter().map(|node| node.byz.snapshot()).collect();
    let frontier_steps: Vec<Vec<(u64, u64)>> = nodes
        .iter()
        .map(|node| node.frontier_steps.lock().unwrap().clone())
        .collect();
    let cert_inlet: Vec<Option<CertInletFacts>> = nodes
        .iter()
        .map(|node| node.cert_inlet.as_ref().map(CertInletObs::snapshot))
        .collect();
    let blocked: Vec<BlockerFacts> = nodes.iter().map(|node| node.blocker.snapshot()).collect();
    let (tracked_sets, tracked_mismatches, tracked_forwarded) = {
        let t = tracked.lock().unwrap();
        (t.per_node.clone(), t.mismatches, t.forwarded)
    };
    let bodies: Vec<BTreeMap<(u64, PeerPubkey), usize>> =
        nodes.iter().map(|node| node.bodies.snapshot()).collect();
    let head_gap_on_fcu = nodes
        .iter()
        .map(|node| node.chain.head_gap_on_fcu())
        .collect();
    let head_gap_on_landing = nodes
        .iter()
        .map(|node| node.chain.head_gap_on_landing())
        .collect();
    let mut archive: Vec<BTreeMap<u64, ArchiveEntry>> = vec![BTreeMap::new(); n];
    if cfg.archive_scan {
        let top = heights.iter().copied().max().unwrap_or(0);
        for (i, node) in nodes.iter().enumerate() {
            let marshal = node.marshal.get().expect("marshal slot filled at build");
            for h in 1..=top {
                let height = Height::new(h);
                let finalization = marshal.get_finalization(height).await.is_some();
                let block = marshal.get_block(height).await.is_some();
                archive[i].insert(
                    h,
                    ArchiveEntry {
                        finalization,
                        block,
                    },
                );
            }
        }
    }
    // The run's counter values, taken here — after `staking_reads` and `logs`, and
    // before the committee scan below, which is the only other producer of counter
    // increments. Draining resets the recorder, so the caller's own drain
    // afterwards holds exactly the scan's contribution.
    let metrics_before_collect = match &cfg.metrics_snapshotter {
        Some(snap) => drain_counters(snap),
        None => Vec::new(),
    };
    // The committee module's own answer per node, last, so a record this run never
    // asked for is read without landing in the counters the tests assert on. Two
    // epochs past the highest tip is the module's lookahead ceiling.
    let committee_records: Vec<BTreeMap<u64, Result<CommitteeFacts, CommitteeRefusal>>> = nodes
        .iter()
        .map(|node| {
            (0..=max_epoch + MAX_COMMITTEE_LOOKAHEAD_EPOCHS)
                .map(|e| {
                    let answer = match node.committee.committee(e) {
                        Ok(record) => Ok(CommitteeFacts {
                            members: record.members.iter().map(|m| m.peer.clone()).collect(),
                            weights: record.weights.clone(),
                            changed: record.changed,
                            anchor: record.snapshot,
                        }),
                        Err(error) => Err(CommitteeRefusal {
                            transient: error.is_transient(),
                            error: error.to_string(),
                        }),
                    };
                    (e, answer)
                })
                .collect()
        })
        .collect();
    let committee_verifier_epochs: Vec<Vec<u64>> = nodes
        .iter()
        .map(|node| node.committee.verifier_epochs())
        .collect();
    let committees: Vec<Arc<dyn crate::committee::Committee>> =
        nodes.iter().map(|node| node.committee.clone()).collect();
    let metrics = ctx.encode();
    Outcome {
        heights,
        hashes,
        diverged,
        upstream,
        upstream_served,
        halted,
        traces,
        seeds,
        artifacts,
        signable,
        et_steps,
        et_boundaries,
        geometry,
        peer_sets: tracked_sets,
        tracked_mismatches,
        tracked_forwarded,
        staking_reads,
        committee_records,
        committee_verifier_epochs,
        committees,
        metrics_before_collect,
        jump_calls,
        el_events,
        marshal_tip_series: tip_series,
        frontier_steps_named_series: steps_named_series,
        #[cfg(feature = "dpos-devnet-byzantine")]
        probe_calls,
        frontier_steps,
        cert_inlet,
        blocked,
        #[cfg(feature = "dpos-devnet-byzantine")]
        byz,
        head_gap_series,
        head_gap_on_fcu,
        head_gap_on_landing,
        archive,
        bodies,
        metrics,
        logs,
        stray_dealer_sends,
        tombstones_observed,
        simulator_ack_drops,
        log_capture_live,
        partitions: part_obs,
        timed_out,
        virtual_elapsed: elapsed_now(&ctx),
        real_elapsed: Duration::ZERO,
    }
}

/// Node `i`'s marshal tip out of one runner-registry exposition — the marshal's
/// own `finalized_height` gauge, set on the line that emits `Update::Tip`. `0`
/// when the node has not published one yet.
///
/// The label chain is `node{i}` → `outer` → `marshal`, flattened into
/// `node{i}_outer_marshal_finalized_height`.
fn marshal_tip_of(exposition: &str, i: usize) -> u64 {
    let key = format!("node{i}_outer_marshal_finalized_height");
    exposition
        .lines()
        .find_map(|line| {
            let (k, v) = line.split_once(' ')?;
            (k == key).then(|| v.trim().parse().ok()).flatten()
        })
        .unwrap_or(0)
}

/// The oracles a cut touches, each flagged `true` for the consensus plane: both
/// networks, or the consensus one alone.
fn cut_oracles<'a>(
    p: &Partition,
    consensus: &'a Oracle,
    upstream: &'a Oracle,
) -> Vec<(bool, &'a Oracle)> {
    match p.planes {
        CutPlanes::Both => vec![(true, consensus), (false, upstream)],
        CutPlanes::ConsensusOnly => vec![(true, consensus)],
    }
}

fn cross_pairs(p: &Partition) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    for &a in &p.a {
        for &b in &p.b {
            v.push((a, b));
            v.push((b, a));
        }
    }
    v
}

/// The first height (walking up from 1) at which the nodes that reached it
/// executed different hashes. A strict majority (more than half of the nodes
/// present at that height) names the minority node; without one the outcome is a
/// `Tie`, never a "winner".
pub(super) fn first_divergence(heights: &[u64], hashes: &[Vec<(u64, B256)>]) -> Option<Divergence> {
    let max = heights.iter().copied().max().unwrap_or(0);
    for h in 1..=max {
        let present: Vec<(usize, B256)> = hashes
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.get((h - 1) as usize).map(|(_, x)| (i, *x)))
            .collect();
        if present.len() < 2 {
            continue;
        }
        let mut counts: BTreeMap<B256, usize> = BTreeMap::new();
        for (_, x) in &present {
            *counts.entry(*x).or_default() += 1;
        }
        if counts.len() < 2 {
            continue;
        }
        let majority = counts
            .iter()
            .find(|(_, c)| **c * 2 > present.len())
            .map(|(x, _)| *x);
        return Some(match majority {
            Some(majority) => {
                let (node, _) = present
                    .iter()
                    .find(|(_, x)| *x != majority)
                    .expect("two distinct hashes, one is the majority");
                Divergence::Minority {
                    node: *node,
                    height: h,
                }
            }
            None => Divergence::Tie {
                height: h,
                hashes: counts.keys().copied().collect(),
            },
        });
    }
    None
}

pub(super) type Oracle = commonware_p2p::simulated::Oracle<PeerPubkey, deterministic::Context>;

/// The upstream plane of one node: `FRONTIER_CHANNEL` registered on the upstream
/// simulated network, a `commonware_resolver::p2p::Engine` over it with the
/// production `plane_upstream::new_bridge` as producer and consumer, and the
/// `PlaneUpstreamHandle` that issues fetches on it.
///
/// `marshal_slot` is the late-bound marshal the serve side reads, filled once the
/// `OuterEngine` is built. `counters` sees both ends; `blocker` sees whether the
/// resolver excluded the peer that answered.
#[allow(clippy::too_many_arguments)]
pub(super) async fn frontier_plane(
    ctx: &deterministic::Context,
    oracle: &Oracle,
    me: PeerPubkey,
    marshal_slot: Arc<OnceLock<MarshalMailbox>>,
    committee: Arc<dyn crate::committee::Committee>,
    counters: UpstreamCounters,
    blocker: SpyBlocker,
    #[cfg(feature = "dpos-devnet-byzantine")] byz: ByzReport,
    #[cfg(feature = "dpos-devnet-byzantine")] mode: super::byzantine_roles::ForgeMode,
    #[cfg(feature = "dpos-devnet-byzantine")] lying: Option<super::byzantine_roles::LyingCfg>,
) -> PlaneUpstreamHandle<deterministic::Context> {
    let (sender, receiver) = oracle
        .control(me.clone())
        .register(FRONTIER_CHANNEL, QUOTA)
        .await
        .expect("frontier channel");
    let (handler, waiters) = new_bridge(marshal_slot, committee.clone(), ctx.clone(), CHAIN_ID);
    let handler = CountingHandler::new(handler, counters);
    // The lying-upstream roles wrap the serve side, so the victim's own consumer
    // path is production's verbatim.
    #[cfg(feature = "dpos-devnet-byzantine")]
    let handler = super::byzantine_roles::ForgedSeedProducer::new(handler, byz, mode, lying);
    let (engine, mailbox) = commonware_resolver::p2p::Engine::new(
        ctx.with_label("frontier_resolver"),
        commonware_resolver::p2p::Config {
            peer_provider: oracle.manager(),
            blocker,
            consumer: handler.clone(),
            producer: handler,
            mailbox_size: MUX_MAILBOX,
            me: Some(me),
            initial: Duration::from_millis(100),
            timeout: Duration::from_secs(5),
            fetch_retry_timeout: Duration::from_millis(500),
            priority_requests: false,
            priority_responses: false,
        },
    );
    engine.start((sender, receiver));
    PlaneUpstreamHandle::new(ctx.clone(), mailbox, waiters)
}

#[allow(clippy::too_many_arguments)]
async fn build_node(
    ctx: &deterministic::Context,
    i: usize,
    cfg: &StandConfig,
    role: Role,
    peers: &[Ed25519PrivateKey],
    bls: &[ValidatorBlsKeypair],
    oracle: &Oracle,
    upstream_oracle: &Oracle,
    members: Members,
    el_network: ElNetwork,
    pks: &[PeerPubkey],
    bls_pubkeys: &[BlsPubkey],
    registry: Vec<PeerPubkey>,
    tracked: Arc<Mutex<Tracked>>,
    genesis_hash: B256,
    genesis_block: OrderBlock,
    // Every node's marshal slot: a cert-inlet fed by
    // `CertInletSource::PeerArchive` reads its donor's archive, and the donor may
    // not be built yet when this node is. Own slot = `marshal_slots[i]`.
    marshal_slots: &[Arc<OnceLock<MarshalMailbox>>],
) -> NodeHandles {
    let ctx_i = ctx.with_label(&format!("node{i}"));
    let me = peers[i].public_key();
    #[cfg(feature = "dpos-devnet-byzantine")]
    let byz = ByzReport::default();

    // The five plane channels, each behind its own persistent `Muxer`.
    let register = |channel: u64| {
        let control = oracle.control(me.clone());
        async move { control.register(channel, QUOTA).await.expect("channel") }
    };
    let bodies = BodyTap::default();
    let (vs, vr) = register(VOTE_CHANNEL).await;
    let (cs, cr) = register(CERT_CHANNEL).await;
    let (rs, rr) = register(RESOLVER_CHANNEL).await;
    let (bs, br) = register(BROADCAST_CHANNEL).await;
    let (ms, mr) = register(MARSHAL_CHANNEL).await;
    let vr = TapReceiver::new(vr, None);
    let cr = TapReceiver::new(cr, None);
    let rr = TapReceiver::new(rr, None);
    let br = TapReceiver::new(br, Some(bodies.clone()));
    let mr = TapReceiver::new(mr, None);
    let (mux_vote, vote_handle, mut vote_backup_rx) =
        Muxer::builder(ctx_i.with_label("vote_mux"), vs, vr, MUX_MAILBOX)
            .with_backup()
            .build();
    mux_vote.start();
    let (mux_cert, cert_handle) = Muxer::new(ctx_i.with_label("cert_mux"), cs, cr, MUX_MAILBOX);
    mux_cert.start();
    let (mux_res, res_handle) = Muxer::new(ctx_i.with_label("resolver_mux"), rs, rr, MUX_MAILBOX);
    mux_res.start();
    let (mux_bcast, bcast_handle) =
        Muxer::new(ctx_i.with_label("broadcast_mux"), bs, br, MUX_MAILBOX);
    mux_bcast.start();
    let (mux_marshal, marshal_handle) =
        Muxer::new(ctx_i.with_label("marshal_mux"), ms, mr, MUX_MAILBOX);
    mux_marshal.start();
    let vote_mux = Arc::new(tokio::sync::Mutex::new(vote_handle));
    let cert_mux = Arc::new(tokio::sync::Mutex::new(cert_handle));
    let res_mux = Arc::new(tokio::sync::Mutex::new(res_handle));
    let bcast_mux = Arc::new(tokio::sync::Mutex::new(bcast_handle));
    let marshal_mux = Arc::new(tokio::sync::Mutex::new(marshal_handle));

    // The vote Muxer's backup channel is drained and dropped, as the plane's four
    // observer channels are, so the Muxer never blocks on a full backup mailbox
    // when a frame arrives for an epoch this node has no engine for.
    ctx_i
        .with_label("vote_backup")
        .spawn(move |_| async move { while vote_backup_rx.recv().await.is_some() {} });

    let sync_metrics = SyncMetrics::default();
    sync_metrics.register(&ctx_i);
    let halt = SafetyHalt::new(sync_metrics.clone());
    let epoch_metrics = EpochEngineMetrics::default();
    epoch_metrics.register(&ctx_i);
    let executor_metrics = ExecutorMetrics::default();
    executor_metrics.register(&ctx_i);
    let plane_clock = PlaneClock::default();
    plane_clock.register(&ctx_i);

    let chain = FakeChain::with_genesis_on(genesis_hash, el_network.clone());
    let divergent_at = match role {
        Role::DivergentResult { at } => Some(at),
        _ => None,
    };
    let deriver = FakeDeriver::new(chain.clone(), divergent_at);
    // The staking contract as a state machine over this node's executed heights.
    // The same instance answers the slasher, the beacon plane's committee reads and
    // the `EpochTransition` below, as in production.
    let staking = FakeStaking::new(
        chain.clone(),
        members,
        pks,
        bls_pubkeys,
        registry,
        cfg.epoch_len,
    )
    .with_schedule(
        cfg.committees_by_branch.clone(),
        cfg.weights_none_for,
        Arc::new(cfg.tombstoned.clone()),
    )
    .with_revert(cfg.reverts_for);
    let (hook_tx, mut hook_rx) = mpsc::unbounded_channel::<OrderBlock>();

    // The production epoch state machine, one per node. `bridge_tx` is built first
    // so the transition is wired at construction; the forwarder that turns
    // `(u64, snapshot)` into the `OuterEngine`'s `(Epoch, snapshot)` is spawned
    // after the engine exists.
    let (bridge_tx, mut bridge_rx) = mpsc::channel::<(u64, ValidatorSetSnapshot)>(64);
    // `T` = `EpochTransition::last_tracked_epoch`, mirrored off the boundary bridge
    // as `consensus/src/dpos.rs` mirrors it: the transition advances
    // `last_tracked_epoch` only on a successful `boundary_tx.try_send`, so the
    // epochs this forwarder drains are the epochs that advanced it. `u64::MAX` is
    // the "nothing tracked yet" sentinel.
    let tracked_epoch_cell = Arc::new(AtomicU64::new(u64::MAX));
    let observer = EtObserver::default();
    // The peer-set window the beacon channel's pre-decode gate reads, as
    // `node/src/dpos.rs` builds one from the Oracle: the set this node's own
    // transition registers plus the tombstone predicate over the node's one
    // `TombstoneSet`, filled by `TombstoneSet::observe` at every finalized height.
    let tombstones = TombstoneSet::default();
    let tombstones_observed: Arc<Mutex<Vec<(u64, PeerPubkey)>>> = Arc::new(Mutex::new(Vec::new()));
    let ingress_window = {
        let tombstones = tombstones.clone();
        fluentbase_p2p::TrackedWindow::default()
            .with_tombstones(Arc::new(move |peer: &PeerPubkey| tombstones.contains(peer)))
    };
    let et = Arc::new(tokio::sync::Mutex::new(EpochTransition::new(
        staking.clone(),
        TrackSink {
            node: i,
            oracle: oracle.clone(),
            shared: tracked,
            window: ingress_window.clone(),
        },
        MAX_REGISTRY_PEER_SET as usize,
        Some(bridge_tx),
        {
            let chain = chain.clone();
            Arc::new(move |h| chain.executed_state_hash(h))
        },
        crate::order_block::K,
    )));

    // Cold start at the anchor this node's execution layer persisted: genesis on a
    // fresh chain, the finalized `(height, hash)` on a replay. The epoch is not
    // supplied — `cold_start` freezes the geometry from the state at `hash` and
    // derives it.
    let (cold_number, cold_hash) = cfg.resume_from.unwrap_or((0, genesis_hash));
    chain.note_hash(cold_number, cold_hash);
    let cold_outcome = {
        let mut guard = et.lock().await;
        let out = guard.cold_start(cold_hash, cold_number).await;
        *observer.geometry.lock().unwrap() = guard.frozen_geometry();
        out
    };
    observer.steps.lock().unwrap().push(EtStep {
        number: cold_number,
        outcome: cold_outcome.map_err(|e| format!("{e:?}")),
    });

    let engine_partition_prefix = if cfg.shared_engine_partitions {
        String::new()
    } else {
        format!("node{i}-")
    };

    // The committee module's read anchor, the stand's twin of production's
    // `RethAnchor`: the height is this node's ordering-finalized tip and the hash
    // is that chain's tier-F hash there.
    //
    // Production reads at `executed_state_hash(ordering_finalized)`, and so does
    // this: the probe keeps the `Err` arm — the input of the anchor-fault branch —
    // reachable from the stand.
    struct StandAnchor {
        chain: FakeChain,
    }

    impl crate::committee::Anchor for StandAnchor {
        fn height(&self) -> u64 {
            self.chain.tip()
        }

        fn executed_hash(
            &self,
            height: u64,
        ) -> Result<Option<B256>, fluentbase_staking_reader::ReadError> {
            self.chain.executed_state_hash(height)
        }
    }

    // One committee module per stand node: the beacon's `CommitteeReads` is a
    // facade over it, the slasher resolves evidence through it, and the executor
    // wakes it on every `advance_finalized`. The scheme producer below is wired as
    // production wires it: the beacon slot is filled once this node's beacon
    // exists, and until then the store answers "no scheme yet" and retries.
    let beacon_slot: crate::committee::BeaconSlot = Arc::new(OnceLock::new());
    let committee: Arc<dyn crate::committee::Committee> =
        Arc::new(crate::committee::CommitteeStore::new(
            staking.clone(),
            Arc::new(StandAnchor {
                chain: chain.clone(),
            }),
            tokio::sync::watch::Sender::new(Some((DPOS_ACTIVATION_BLOCK, cfg.epoch_len)))
                .subscribe(),
            crate::committee::epoch_verifier(CHAIN_ID, beacon_slot.clone()),
        ));

    // The upstream plane: the production frontier resolver and
    // `PlaneUpstreamHandle` over `FRONTIER_CHANNEL`, counted at both ends. The
    // marshal slot is filled once the `OuterEngine` is built.
    let upstream_counters = UpstreamCounters::default();
    // This node's blocker spy, wired into both of its blocker slots. A no-op
    // blocker would make "no honest peer lost a channel" vacuous.
    let blocker_spy = BlockerSpy::default();
    let marshal_slot: Arc<OnceLock<MarshalMailbox>> = marshal_slots[i].clone();
    let upstream = CountingUpstream::new(
        frontier_plane(
            &ctx_i,
            upstream_oracle,
            me.clone(),
            marshal_slot.clone(),
            committee.clone(),
            upstream_counters.clone(),
            blocker_spy.at(BLOCKER_SITE_FRONTIER),
            #[cfg(feature = "dpos-devnet-byzantine")]
            byz.clone(),
            #[cfg(feature = "dpos-devnet-byzantine")]
            {
                use super::byzantine_roles::ForgeMode;
                match role {
                    Role::ForgedSeedUpstream => ForgeMode::SeedSlot,
                    Role::InflatedProbe => ForgeMode::InflatedLatest,
                    Role::LyingUpstream => ForgeMode::LyingLatest,
                    Role::WrongHeightFinalized => ForgeMode::WrongHeightFinalized,
                    _ => ForgeMode::Watch,
                }
            },
            #[cfg(feature = "dpos-devnet-byzantine")]
            matches!(role, Role::LyingUpstream).then(|| super::byzantine_roles::LyingCfg {
                chain: chain.clone(),
                el_network: el_network.clone(),
            }),
        )
        .await,
        upstream_counters.clone(),
    );
    // Every jump call, with the outcome variant it returned — see `JumpCall`.
    let jump_calls: JumpCalls = Arc::new(Mutex::new(Vec::new()));
    // How many times this node's frozen-tip probe actually asked the upstream
    // (`FrontierProbeFn` invocations that ran `get_latest`). A healthy node whose
    // tip advances never reaches the probe body, so the count climbs only when the
    // frontier is inflated.
    let probe_calls = Arc::new(AtomicU64::new(0));
    // `(frame, recipient)` pairs a `Role::StrayDealer` put on the beacon channel.
    let stray_sends = Arc::new(AtomicU64::new(0));
    // Every ladder step this node's probe named, as `(T, last(T+1))`, surfaced as
    // `Outcome::frontier_steps` so a test can assert what was asked for.
    //
    // Named, not put, and recorded without the marshal tip: the executor decides
    // whether to put the step, and reading that tip here perturbs the run. The
    // step-vs-tip comparison is pinned in `executor::tests`.
    let frontier_steps: FrontierSteps = Arc::new(Mutex::new(Vec::new()));
    let re_jump = {
        let probe: FrontierProbeFn = {
            let up = upstream.clone();
            let probe_calls = probe_calls.clone();
            let committee_probe = committee.clone();
            let steps = frontier_steps.clone();
            // Production's probe: `Latest` untargeted plus, when `T` and
            // `committee[T+1]` are known, the ladder step `Finalized{last(T+1)}` and
            // the peers to address it at.
            Arc::new(move |tracked: Option<u64>| {
                let up = up.clone();
                let probe_calls = probe_calls.clone();
                let committee = committee_probe.clone();
                let steps = steps.clone();
                Box::pin(async move {
                    probe_calls.fetch_add(1, Ordering::Relaxed);
                    let step = match (tracked, committee.geometry()) {
                        (Some(t), Some(geometry)) => match committee.committee(t + 1) {
                            Ok(record) => {
                                let height = geometry.last(t + 1);
                                let targets: Vec<_> = record.participants.iter().cloned().collect();
                                match commonware_utils::vec::NonEmptyVec::try_from(targets) {
                                    Ok(targets) => {
                                        steps.lock().unwrap().push((t, height));
                                        Some((Height::new(height), targets))
                                    }
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
                            // The same label function production uses, so the two
                            // probes cannot drift apart on the `reason` set.
                            Err(e) => {
                                metrics::counter!(
                                    crate::executor::FRONTIER_STEP_SKIPPED,
                                    "reason" => crate::dpos::step_skip_reason(&e),
                                )
                                .increment(1);
                                None
                            }
                        },
                        _ => None,
                    };
                    let latest = up.get_latest().await;
                    crate::executor::ProbeOutcome {
                        frontier: latest.map(|uf| Height::new(uf.block.height)),
                        step,
                    }
                })
            })
        };
        let rejump_calls = upstream_counters.rejump_calls.clone();
        // The steady-state re-jump is the production `jump_to_target`, called as the
        // node calls it: the same forward-only need-gate, the same landing check
        // against the attested `block.result`, the same `l1_checkpoint = None` on the
        // validator path. The one seam below it is the stand's: `JumpElSync` for
        // `RethElSync`.
        //
        // The threshold goes to both the executor's arming gate and the jump's own
        // need-gate. At the default (`u64::MAX`) the executor never arms the waiter,
        // so the call and the `anchor + threshold` overflow it would guard are
        // unreachable.
        let threshold = cfg.re_jump_threshold.unwrap_or(u64::MAX);
        let call: ReJumpFn = {
            let chain = chain.clone();
            let calls_log = jump_calls.clone();
            let ctx_jump = ctx_i.clone();
            Arc::new(move |from: u64, target: UpstreamFinalized| {
                let el = JumpElSync::new(chain.clone(), ctx_jump.clone(), DPOS_ACTIVATION_BLOCK);
                let calls = rejump_calls.clone();
                let calls_log = calls_log.clone();
                // The pair the executor read out of this node's own marshal archive,
                // recorded before the jump consumes it so a test can check the landing
                // against the certificate it came from.
                let consumed = Some((target.block.height, target.block.result));
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let outcome = crate::cold_start_jump::jump_to_target(
                        from,
                        target,
                        &el,
                        // No L1 checkpoint on the validator path. `ElSync::holds` is
                        // called: the landing check asks it for the attested
                        // `block.result`.
                        None,
                        DPOS_ACTIVATION_BLOCK,
                        threshold,
                    )
                    .await;
                    // The production return value, recorded verbatim — the variant as
                    // well as the landing, because a jump that ran and was refused is
                    // otherwise indistinguishable from one that landed.
                    calls_log.lock().unwrap().push(JumpCall {
                        from,
                        outcome: match &outcome {
                            JumpOutcome::Landed { .. } => "Landed",
                            JumpOutcome::Lagging => "Lagging",
                            JumpOutcome::Stalled(_) => "Stalled",
                            JumpOutcome::InvalidTarget(_) => "InvalidTarget",
                            JumpOutcome::StalledWithPeers(_) => "StalledWithPeers",
                            JumpOutcome::L1Fork(_) => "L1Fork",
                        },
                        consumed,
                        landed: match &outcome {
                            JumpOutcome::Landed { landing, hash, .. } => Some((*landing, *hash)),
                            _ => None,
                        },
                        // The error text of a refusal, so a test can tell the
                        // landing-check arm from a stall.
                        outcome_detail: match &outcome {
                            JumpOutcome::Stalled(e)
                            | JumpOutcome::InvalidTarget(e)
                            | JumpOutcome::StalledWithPeers(e)
                            | JumpOutcome::L1Fork(e) => Some(format!("{e:#}")),
                            JumpOutcome::Landed { .. } | JumpOutcome::Lagging => None,
                        },
                    });
                    outcome
                })
            })
        };
        ReJump {
            call,
            threshold,
            rotate: None,
            probe: Some(probe),
            tracked_epoch: Some({
                let cell = tracked_epoch_cell.clone();
                Arc::new(move || match cell.load(Ordering::Relaxed) {
                    u64::MAX => None,
                    epoch => Some(epoch),
                })
            }),
        }
    };

    // The beacon plane: `beacon::build` as `node/src/dpos.rs::build_beacon_plane`
    // calls it, over the consensus network's beacon channels and the same four mux
    // brokers, with the schedule standing in for the staking reads. Built before the
    // `OuterBuilder`, which takes its randomness.
    //
    // The actor's clock is a receiver taken here and the sender goes into the
    // `OuterBuilder`, so `FluentApp::report` publishes the marshal's tip on it.
    let beacon_tip = Arc::new(tokio::sync::watch::Sender::new(0u64));

    let (randomness, artifacts) = match (cfg.beacon, role) {
        (Beacon::Static, _) => (
            StaticRandomness::build(CHAIN_ID, staking.all_validators_snapshot()),
            None,
        ),
        (Beacon::Live, Role::AbsentBeacon) => (absent(&ctx_i), None),
        (Beacon::Live, _) => {
            let (bcs, bcr) = register(BEACON_CHANNEL).await;
            let (brs, brr) = register(BEACON_RESOLVER_CHANNEL).await;
            // `Role::StrayDealer`: a second handle on the same channel sender the
            // node's beacon gets, driven by a task that deals for the epoch after
            // this node's own once a second. The body is a real `Commitment`, so a
            // receiver whose gate is missing gets a frame its actor can answer.
            if matches!(role, Role::StrayDealer) {
                let mut sender = bcs.clone();
                let sends = stray_sends.clone();
                let chain = chain.clone();
                let me_key = peers[i].clone();
                let everyone: Set<PeerPubkey> = Set::from_iter_dedup(pks.iter().cloned());
                let epoch_len = cfg.epoch_len;
                ctx_i.with_label("stray_dealer").spawn(move |c| async move {
                    loop {
                        c.sleep(Duration::from_secs(1)).await;
                        let epoch = chain.tip() / epoch_len + 1;
                        let Ok((_ceremony, step)) = DkgCeremony::start(
                            b"FLUENT_DPOS_V1_stray",
                            epoch,
                            everyone.clone(),
                            me_key.clone(),
                        ) else {
                            continue;
                        };
                        let Some(body) = step.outgoing.into_iter().find_map(|o| {
                            matches!(o.msg.body, DkgBody::Commitment(_)).then_some(o.msg.body)
                        }) else {
                            continue;
                        };
                        let wire = BeaconMessage::Dkg(
                            DkgMsg {
                                ceremony_epoch: epoch,
                                body,
                            }
                            .encode(),
                        )
                        .encode();
                        if let Ok(sent) = sender.send(Recipients::All, wire, false).await {
                            sends.fetch_add(sent.len() as u64, Ordering::Relaxed);
                        }
                    }
                });
            }
            #[cfg(feature = "dpos-devnet-byzantine")]
            let roster = {
                let committee = committee.clone();
                move |epoch: u64| -> Option<Set<PeerPubkey>> {
                    Some(committee.committee(epoch).ok()?.participants.clone())
                }
            };
            // Only the two-reveal role still needs the roster as a closure: the
            // beacon takes its committee reads through `CommitteeReads` now.
            #[cfg(feature = "dpos-devnet-byzantine")]
            let committee_for: CommitteeFor = {
                let roster = roster.clone();
                Arc::new(roster)
            };
            // The beacon channel's sender half, wrapped. `None` on every honest
            // node, and then the wrapper is a pure pass-through so `beacon::build`'s
            // `Se` does not branch on the role.
            #[cfg(feature = "dpos-devnet-byzantine")]
            let bcs = super::byzantine_roles::TwoRevealSender::new(
                bcs,
                byz.clone(),
                match role {
                    Role::TwoReveals { .. } => {
                        assert_ne!(i, 0, "the two-reveal dealer must not BE its own victim");
                        Some(super::byzantine_roles::TwoRevealCfg {
                            me_key: peers[i].clone(),
                            victim: pks[0].clone(),
                            others: pks
                                .iter()
                                .filter(|p| **p != me && **p != pks[0])
                                .cloned()
                                .collect(),
                            committee_for: committee_for.clone(),
                            namespace: fluentbase_bls::beacon::seed_namespace(
                                &fluentbase_bls::fluent_namespace(CHAIN_ID),
                            ),
                            report: byz.clone(),
                        })
                    }
                    _ => None,
                },
            );
            // Every staking read the beacon takes, answered from the committee
            // module — the same type production uses. The module reads at this
            // node's ordering-finalized tip and nothing else.
            let committees: Arc<dyn CommitteeReads> = Arc::new(
                crate::committee::CommitteeReadsFacade::new(committee.clone()),
            );
            // The production pre-decode gate on the beacon channel: committee
            // traffic, so a registry-tier / untracked / tombstoned sender never
            // reaches the actor's decode. The seat a sender holds in a frame's epoch
            // is the consumer's check inside the actor.
            let bcr = crate::dpos::GatedReceiver::new(bcr, ingress_window.clone(), "beacon", true);
            let (beacon, _beacon_tasks) = beacon::build(
                &ctx_i,
                ValidatorInputs {
                    chain_id: CHAIN_ID,
                    peer_keypair: peers[i].clone(),
                    bls_keypair: bls[i].clone(),
                    share_dir: cfg.share_root.join(format!("node{i}")),
                    share_seal_key: None,
                    peers: oracle.manager(),
                    beacon_channel: (bcs, bcr),
                    resolver_channel: (brs, brr),
                    vote_mux: vote_mux.clone(),
                    cert_mux: cert_mux.clone(),
                    resolver_mux: res_mux.clone(),
                    bodies_mux: bcast_mux.clone(),
                    committees,
                    clock: beacon_tip.subscribe(),
                    plane_clock: plane_clock.clone(),
                    safety_halt: halt.clone(),
                    geometry: tokio::sync::watch::channel(et.lock().await.frozen_geometry()).1,
                    partition_prefix: format!("node{i}-"),
                },
            )
            .await
            .expect("beacon::build");
            // The plane's task handles are detached on drop (commonware `Handle`
            // has no `Drop`); they live until the runner returns.
            #[cfg(feature = "dpos-devnet-byzantine")]
            let randomness = match role {
                Role::TwoReveals {
                    withhold_partials: true,
                } => super::byzantine_roles::WithholdingRandomness::wrap(
                    beacon,
                    CHAIN_ID,
                    byz.clone(),
                ),
                _ => beacon,
            };
            #[cfg(not(feature = "dpos-devnet-byzantine"))]
            let randomness = beacon;
            let artifacts: ArtifactSource = {
                let beacon = randomness.clone();
                Arc::new(move |epoch: u64| beacon.artifact_bytes(epoch))
            };
            (randomness, Some(artifacts))
        }
    };
    // Everything the optional cert-inlet task needs that is about to be moved into
    // the builders below, captured here and only on the nodes that run one: the
    // node's one beacon.
    let inlet_inputs = cfg
        .cert_inlet
        .as_ref()
        .filter(|c| c.nodes.contains(&i))
        .map(|c| (c.source, randomness.clone()));
    // Under `Beacon::Static` no actor holds a receiver, so the app keeps the watch
    // `FluentApp::new` makes.
    let beacon_tip = matches!(cfg.beacon, Beacon::Live).then_some(beacon_tip);
    // Every epoch the verifier reads from here on gets its scheme bound to this
    // node's beacon oracle.
    crate::committee::fill_beacon_slot(&beacon_slot, &randomness);
    let beacon_handle = randomness.clone();

    let outer = OuterBuilder {
        me: me.clone(),
        blocker: blocker_spy.at(BLOCKER_SITE_CONSENSUS),
        provider: oracle.manager(),
        chain_id: CHAIN_ID,
        epoch_length_blocks: NonZeroU64::new(cfg.epoch_len).expect("epoch_len > 0"),
        dpos_activation_block: DPOS_ACTIVATION_BLOCK,
        signer_keypair: Some(bls[i].clone()),
        randomness,
        spawn_unblocked: Arc::new(Notify::new()),
        re_jump: Some(re_jump),
        epoch_metrics,
        executor_metrics,
        sync_metrics,
        safety_halt: halt.clone(),
        tombstones: tombstones.clone(),
        plane_clock,
        beacon_tip,
        timeouts: ConsensusTimeouts::fluent_1s(),
        mailbox_size: 256,
        // A body cache four deep per primary sender, as production carries it.
        deque_size: 4,
        partition_prefix: format!("node{i}-consensus_marshal"),
        engine_partition_prefix,
        resolver_initial: Duration::from_secs(1),
        resolver_timeout: Duration::from_secs(2),
        resolver_fetch_retry: Duration::from_millis(100),
        genesis: genesis_block,
        beacon_engine: FakeBeacon::new(chain.clone()),
        deriver,
        executed: chain.clone(),
        assembler: Arc::new(NoTxs),
        target_gas_limit: 30_000_000,
        boundary_hook: Arc::new(move |block: OrderBlock| {
            let _ = hook_tx.send(block);
        }),
        feed: None,
        last_execution_finalized_height: 0,
        initial_finalized: (Height::new(0), genesis_hash),
        initial_head: (Height::new(0), genesis_hash),
        marshal_floor: Some(Height::new(0)),
        boundary_fetch: None,
        boundary_enter: Arc::new(|_| {}),
        boundary_read_floor: Arc::new(|_| Box::pin(async {})),
        fcu_heartbeat_interval: Duration::from_secs(8),
        fcu_pace: Duration::ZERO,
        canonical_state: reth_chain_state::CanonicalInMemoryState::empty(),
        slasher_staking_address: Address::ZERO,
        committee: committee.clone(),
        slasher_sink: Arc::new(NoSink),
        slasher_wal_partition: format!("node{i}-slasher-wal"),
        slasher_evidence: None,
        #[cfg(feature = "dpos-devnet-byzantine")]
        byzantine: matches!(role, Role::Equivocate)
            .then_some(crate::byzantine::ByzantineMode::Equivocate),
    };
    let outer = outer
        .build(ctx_i.with_label("outer"))
        .await
        .expect("OuterBuilder::build");
    marshal_slot
        .set(outer.marshal_mailbox())
        .unwrap_or_else(|_| panic!("marshal slot filled twice"));

    // The second producer into this node's singleton marshal: the production
    // `CertInlet`, fed by this node's own frontier plane or by a donor's archive.
    // Spawned after `OuterBuilder::build`, so its own marshal handle is already
    // available; the donor's slot is bound late because that node may still be
    // unbuilt.
    let cert_inlet = inlet_inputs.map(|(source, beacon)| {
        use crate::cert_inlet::{CertInlet, RotateUpstream};
        let obs = CertInletObs::default();
        // The rotation trigger, counted: `CertUpstream::rotate_callback` with a
        // counter in front of it, because the inlet's `consecutive_faults` is
        // private.
        let rotate: RotateUpstream = {
            let up = upstream.clone();
            let rotations = obs.rotations.clone();
            Arc::new(move || {
                let up = up.clone();
                let rotations = rotations.clone();
                Box::pin(async move {
                    rotations.fetch_add(1, Ordering::SeqCst);
                    up.rotate().await;
                }) as futures::future::BoxFuture<'static, ()>
            })
        };
        // The inlet's marshal is the node's real one behind [`RecordingSink`],
        // so the verified heights are read at the seam the inlet drives.
        let marshal = RecordingSink {
            inner: outer.marshal_mailbox(),
            delivered: obs.delivered.clone(),
        };
        // Every node's marshal slot, for `CertInletSource::PeerArchive`: the donor's
        // may still be empty when this task starts, so the task binds it late.
        let donor_slots: Vec<Arc<OnceLock<MarshalMailbox>>> = marshal_slots.to_vec();
        let (committee_inlet, up, chain_inlet, obs_task) = (
            committee.clone(),
            upstream.clone(),
            chain.clone(),
            obs.clone(),
        );
        ctx_i.with_label("cert_inlet").spawn(move |c| async move {
            // The validator shape of `node/src/cert_inlet.rs`: rotate and the node's
            // own beacon, with neither a height↔epoch bind nor a serving window nor a
            // connection token. Both metric builders are wired because each is the
            // only witness of a regime: the defer family counts the non-fault
            // committee-lag skip, and the carry-forward counter is the one line that
            // fires only when a verify failed with the epoch key resolvable.
            let mut inlet = CertInlet::new(marshal, committee_inlet, c.clone())
                .with_rotate(rotate)
                .with_randomness(beacon)
                .with_committee_read_deferred_metric(obs_task.defers.clone())
                .with_carry_forward_fail_metric(obs_task.carry_forward_fails.clone());
            // `PeerArchive`'s cursor is the feeder's rather than the plane's answer:
            // it climbs by one on every pair the donor actually held, so this source
            // never re-ingests a height. The two plane sources have no such cursor
            // and re-ask whatever the plane answers.
            let mut walk: u64 = 1;
            let mut donor: Option<MarshalMailbox> = None;
            loop {
                // Pacing differs by arm. The two plane fetches await either a
                // delivery or `FRONTIER_FETCH_TIMEOUT`, but a `None` on the
                // step-(5) drop path resolves at once, so those arms are bounded by
                // the simulated network round trip rather than the timeout.
                // `PeerArchive` is a local read with no round trip to borrow pacing
                // from, so its misses are paced with `POLL`.
                let fetched = match source {
                    // The next height above this node's own tier-F, read off the
                    // `FakeChain`: asking the marshal costs a message in its select
                    // loop per tick, which is measured to change a run.
                    CertInletSource::NextAboveTier => {
                        up.get_finalization(Height::new(chain_inlet.tip() + 1))
                            .await
                    }
                    CertInletSource::Frontier => up.get_latest().await,
                    // The donor's archive, read directly. Two differences from the
                    // arms above: the read never touches `deliver`, so nothing gates
                    // the certificate's epoch against this node's committee window;
                    // and it is local to the donor, so a miss has to be paced.
                    CertInletSource::PeerArchive { from } => {
                        if donor.is_none() {
                            donor = donor_slots[from].get().cloned();
                        }
                        let Some(donor) = donor.as_ref() else {
                            // The donor's engine is not built yet: the slot is
                            // filled at its `OuterBuilder::build`, and nodes are
                            // built in index order.
                            c.sleep(POLL).await;
                            continue;
                        };
                        match donor.pair_at(Height::new(walk)).await {
                            Some((finalization, block)) => {
                                walk += 1;
                                Some(UpstreamFinalized {
                                    finalization,
                                    block,
                                })
                            }
                            None => {
                                c.sleep(POLL).await;
                                None
                            }
                        }
                    }
                };
                let Some(uf) = fetched else {
                    continue;
                };
                obs_task.ingests.fetch_add(1, Ordering::SeqCst);
                inlet.ingest(uf).await;
            }
        });
        obs
    });

    // Cold start: the epoch the transition entered is read through the committee
    // module, which is also what registers its verify-only scheme so the marshal can
    // check certificates before the first boundary.
    //
    // Two assertions, at the two points where each is strongest, because the
    // module's anchor here is this node's ordering-finalized cursor and at build
    // time that is still 0. Here: never a permanent refusal — an impossible
    // committee at the cold-start epoch is a chain fact, so the stand dies on it.
    // Below, in the boundary feeder: the read must succeed with a non-empty
    // committee the moment the anchor is no longer 0.
    let initial_epoch = et
        .lock()
        .await
        .epoch_at(cold_number)
        .expect("geometry frozen by cold_start, so the anchor has an epoch");
    if let Err(e) = committee.committee(initial_epoch) {
        assert!(
            e.is_transient(),
            "the cold-start epoch {initial_epoch} was refused PERMANENTLY at the anchor: {e}"
        );
    }

    // The transition's `(u64, snapshot)` becomes the engine's boundary epoch, as in
    // `consensus/src/dpos.rs`, where the snapshot is dropped for the same reason:
    // the manager re-reads the committee from the module. The snapshot is still
    // recorded here, because what the transition read and where is what the
    // epoch-machinery assertions observe.
    {
        let boundary_tx = outer.boundary_sender();
        let boundaries = observer.boundaries.clone();
        let tracked_epoch_writer = tracked_epoch_cell.clone();
        ctx_i.with_label("epoch_bridge").spawn(move |_| async move {
            while let Some((epoch, snap)) = bridge_rx.recv().await {
                tracked_epoch_writer.store(epoch, Ordering::Relaxed);
                boundaries.lock().unwrap().push(EtBoundary {
                    epoch,
                    block_hash: snap.block_hash,
                    block_number: snap.block_number,
                });
                if boundary_tx.send(Epoch::new(epoch)).await.is_err() {
                    return;
                }
            }
        });
    }

    // The boundary feeder: every ordering-finalized block reaches `on_finalized`,
    // the only boundary driver production has (`boundary_hook` →
    // `enter_boundary(block.height)` → `on_finalized(number)`). Not the executor
    // side: the EL-finalized cursor drives no boundary, because the beacon plane's
    // poller takes only the epoch geometry and the first peer-set registration off
    // it.
    let trace = Arc::new(Mutex::new(Vec::<TraceEntry>::new()));
    {
        let (trace, et_feed, steps, geometry) = (
            trace.clone(),
            et.clone(),
            observer.steps.clone(),
            observer.geometry.clone(),
        );
        // The cold-start committee assertion, armed for the first height at which
        // the anchor can answer it: `chain.tip()` is `StandAnchor::height()`, so
        // `tip >= cold_number` means the anchor has reached the height the cold
        // start was taken at. Every gate of the module is satisfied there by
        // construction, so anything other than a non-empty record is a defect.
        let (cold_chain, cold_committee) = (chain.clone(), committee.clone());
        let (tombstone_chain, tombstone_staking, tombstone_set, tombstone_seen) = (
            chain.clone(),
            staking.clone(),
            tombstones.clone(),
            tombstones_observed.clone(),
        );
        let mut cold_start_asserted = false;
        let repoking = Arc::new(AtomicBool::new(false));
        let ctx_repoke = ctx_i.with_label("boundary_repoke");
        ctx_i
            .with_label("boundary_feed")
            .spawn(move |_| async move {
                while let Some(block) = hook_rx.recv().await {
                    if !cold_start_asserted && cold_chain.tip() >= cold_number {
                        cold_start_asserted = true;
                        let record = cold_committee.committee(initial_epoch).unwrap_or_else(|e| {
                            panic!(
                                "the cold-start committee must read at the anchor: \
                                     committee[{initial_epoch}] at tip {} — {e}",
                                cold_chain.tip()
                            )
                        });
                        assert!(
                            !record.members.is_empty(),
                            "empty committee for the cold-start epoch {initial_epoch} at the \
                             anchor"
                        );
                    }
                    trace.lock().unwrap().push(TraceEntry {
                        height: block.height,
                        view: block.proposal_view,
                        leader: decode_production_record(&block.extra_data)
                            .ok()
                            .flatten()
                            .map(|r| r.leader_index),
                        digest: block.digest().0,
                        hash: None,
                    });
                    let (parked, epoch_here) = {
                        let mut guard = et_feed.lock().await;
                        let outcome = guard.on_finalized(block.height).await;
                        *geometry.lock().unwrap() = guard.frozen_geometry();
                        steps.lock().unwrap().push(EtStep {
                            number: block.height,
                            outcome: outcome.map_err(|e| format!("{e:?}")),
                        });
                        (guard.has_pending_boundary(), guard.epoch_at(block.height))
                    };
                    // The tombstone watch, on this finalized-height feed and no second
                    // timer, as production's beacon-plane poller does: a height whose
                    // state is not executed yet is skipped, not read at a header, and
                    // the snapshot goes to the reader directly. What production does
                    // with the delta — sever the peer's transport — the stand records
                    // instead (`Outcome::tombstones_observed`).
                    if let (Some(epoch), Ok(Some(hash))) =
                        (epoch_here, tombstone_chain.executed_state_hash(block.height))
                    {
                        if let Ok(snap) = fluentbase_staking_reader::reader::StakingStateRead::epoch_committee_snapshot(
                            &tombstone_staking, epoch, hash,
                        ) {
                            let newly = tombstone_set.observe(&snap);
                            if !newly.is_empty() {
                                let mut seen = tombstone_seen.lock().unwrap();
                                seen.extend(newly.into_iter().map(|peer| (block.height, peer)));
                            }
                        }
                    }
                    // A parked boundary replays only on the next `on_finalized`, and
                    // during catch-up the parked boundary is the last deliverable
                    // block. One loop at a time is enough: the single-slot park means
                    // there is only ever one boundary to drive.
                    if parked && !repoking.swap(true, Ordering::SeqCst) {
                        let (et, steps, repoking) =
                            (et_feed.clone(), steps.clone(), repoking.clone());
                        ctx_repoke.clone().spawn(move |c| async move {
                            loop {
                                c.sleep(PENDING_RETRY_BACKOFF).await;
                                let mut guard = et.lock().await;
                                let Some(number) = guard.pending_boundary() else {
                                    break;
                                };
                                let outcome = guard.on_finalized(number).await;
                                steps.lock().unwrap().push(EtStep {
                                    number,
                                    outcome: outcome.map_err(|e| format!("{e:?}")),
                                });
                            }
                            repoking.store(false, Ordering::SeqCst);
                        });
                    }
                }
            });
    }

    let _engine = outer.start(
        ctx_i.with_label("resolver"),
        vote_mux,
        cert_mux,
        res_mux,
        bcast_mux,
        marshal_mux,
        Some(upstream),
    );

    NodeHandles {
        chain,
        halt,
        jump_calls,
        #[cfg(feature = "dpos-devnet-byzantine")]
        probe_calls,
        frontier_steps,
        trace,
        upstream: upstream_counters,
        artifacts,
        beacon: beacon_handle,
        observer,
        staking,
        committee,
        marshal: marshal_slot,
        cert_inlet,
        blocker: blocker_spy,
        bodies,
        stray_sends,
        tombstones_observed,
        #[cfg(feature = "dpos-devnet-byzantine")]
        byz,
    }
}
