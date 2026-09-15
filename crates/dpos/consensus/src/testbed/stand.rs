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
/// A read of one held epoch-key artifact's wire bytes, for the stand's serving
/// side. It used to come out of `beacon::build`; the beacon answers it through
/// `Beacon::artifact_bytes` now, and the stand closes over that.
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
    /// Seeds the deterministic runtime AND the node keys.
    pub seed: u64,
    pub epoch_len: u64,
    pub latency: Duration,
    /// Per-link drop probability.
    pub loss: f64,
    /// `epoch → member indices`. `None` past the schedule (no committee).
    pub committees: Committees,
    /// The BRANCHING committee schedule — see [`BranchCommittees`]. `None` (the
    /// default) leaves [`Self::committees`] as the whole answer, which is what
    /// every test written before step 5b assumes; `Some` OVERRIDES it, and the
    /// closure is then the only thing that decides membership.
    pub committees_by_branch: Option<BranchCommittees>,
    /// One epoch for which the fake contract answers `weights: None` even
    /// though its ring frame is intact — "the contract answered something no
    /// committed epoch can answer". `None` = never.
    pub weights_none_for: Option<u64>,
    /// One epoch whose committee read REVERTS — the other permanent class, the
    /// one that must NOT stop the node. `None` = never.
    pub reverts_for: Option<u64>,
    /// `(node, from height)` pairs the fake contract reports TOMBSTONED from
    /// that height on, in every epoch that node sits in — the contract's live
    /// equivocation flag, read at the call's own block.
    pub tombstoned: Vec<(usize, u64)>,
    /// A snapshotter over the `metrics::counter!` recorder the CALLER installed
    /// around this run, drained by the stand itself immediately before the
    /// collect phase and left in [`Outcome::metrics_before_collect`].
    ///
    /// Why the stand and not the test: the collect phase calls the production
    /// `Committee::committee` once per node per epoch (see
    /// [`Outcome::committee_records`]), and those calls INCREMENT the same
    /// counters. A test that drains after `run_until` returns therefore reads
    /// "the run plus a post-run poll", and an assertion meant to say "the run
    /// really asked" is satisfied by the poll alone. There is no seam outside
    /// `run_until` that is earlier than collect, so the snapshot has to be taken
    /// from inside. `None` (the default) leaves the field empty and changes
    /// nothing.
    pub metrics_snapshotter: Option<Snapshotter>,
    /// Step-B witness knob: `true` leaves every node's per-epoch journals on the
    /// unprefixed production names (`consensus_epoch_{E}`), so the N nodes
    /// share them on the one in-memory `Storage`.
    pub shared_engine_partitions: bool,
    /// What the oracle tracks as the peer set.
    pub peer_set: PeerSet,
    /// Which randomness each node runs — see [`Beacon`].
    pub beacon: Beacon,
    /// Root of the per-node `share_dir`s of a `Beacon::Live` stand — a real
    /// directory (`beacon::build` reloads shares through `std::fs`), one per
    /// `StandConfig::live` call, reused by a replay over the same config.
    pub share_root: PathBuf,
    /// What this node's EXECUTION layer persisted across a restart: the
    /// `(height, executed hash)` of its finalized block, production's
    /// `(latest_finalized, latest_finalized_hash)` read out of reth before
    /// `EpochTransition::cold_start` (`consensus/src/dpos.rs:2044-2047`).
    /// `None` = a fresh chain, cold-started at genesis.
    ///
    /// The stand's block BODIES are not persisted (the replay re-derives
    /// 1..=height into a fresh `FakeChain`), so this is the marker only. The
    /// EPOCH is NOT configured any more: `cold_start` derives it from `height`
    /// over the geometry it freezes from the state at `hash`.
    pub resume_from: Option<(u64, B256)>,
    /// The steady-state re-jump gate. `None` (the default) pins it at `u64::MAX`
    /// so the jump never fires and the stand keeps a CONTIGUOUS chain on every
    /// node — which every test written before step 5 assumes. `Some(t)` is
    /// production's own gate, `JUMP_THRESHOLD.min(epoch_block_interval)`
    /// (`consensus/src/dpos.rs:2466`); with it a node whose executor has fallen
    /// more than `t` behind the upstream frontier EL-syncs forward and leaves a
    /// hole, exactly as a production node does.
    pub re_jump_threshold: Option<u64>,
    /// `Some((source, victim))`: on the UPSTREAM plane only, leave `victim`
    /// linked to `source` alone (every other OUTGOING upstream link from either
    /// is removed; inbound links to them stay, and carry nothing — the resolver
    /// only answers requests). The CONSENSUS plane is untouched, so both stay healthy
    /// participants — only `victim`'s frontier discovery is confined to `source`.
    /// Used by the lying-upstream roles so the victim's `get_latest` is
    /// deterministically served by the liar (the resolver otherwise picks any
    /// tracked, linked peer). `None` = the default all-to-all upstream mesh.
    pub upstream_only_link: Option<(usize, usize)>,
    /// `(victim, source)`: remove every UPSTREAM-plane link of `victim` EXCEPT the
    /// one to `source`, leaving `source` itself fully linked. The consensus plane
    /// is untouched.
    ///
    /// The asymmetric sibling of [`Self::upstream_only_link`], and the difference
    /// is the point: that one isolates BOTH ends, which is right when the source is
    /// a liar whose reach must be bounded too, and wrong when the source is an
    /// ordinary peer the victim must be able to pull a WHOLE chain from. This one
    /// makes "who answered the victim's by-height pull" a fact of the fixture
    /// instead of a fact of which peer the resolver's shuffle happened to pick
    /// (`resolver/src/p2p/fetcher.rs:233-245`, `:270-341`).
    pub upstream_source_only_for: Option<(usize, usize)>,
    /// After the run, ask every node's marshal for the `(finalization, block)`
    /// pair at every height up to the highest tip — the exact two local reads
    /// `handle_produce` answers a peer's `Finalized{h}` from (CW
    /// `marshal/core/actor.rs:808-818`, and `plane_upstream::serve`). Off by
    /// default: the scan is a few hundred mailbox round-trips per node, and it
    /// adds yields to the collection phase.
    pub archive_scan: bool,
    /// Sample every node's MARSHAL TIP once per driver tick into
    /// [`Outcome::marshal_tip_series`] (and, beside it, how many ladder steps its
    /// probe has named so far). Off by default: the sample is one
    /// `commonware_runtime::Metrics::encode` of the whole runner registry per
    /// tick, which is cheap but not free, and only two tests read the series.
    ///
    /// WHY THE METRIC AND NOT THE MARSHAL. The tip is the marshal's own
    /// `finalized_height` gauge, set on exactly the line that emits `Update::Tip`
    /// (CW `marshal/core/actor.rs:1454-1458`, and the restart seed `:398-401`), so
    /// the number IS the executor's `last_tip_height` rather than a proxy for it.
    /// Reading it out of the registry touches no mailbox: asking the marshal
    /// instead (`get_info(Identifier::Latest)`) puts an extra message into its
    /// select loop per node per tick, and that was measured to CHANGE a run
    /// (Д-81 — `a_zero_overlap_boundary_halts_the_chain_verify_only` loses its
    /// incoming half's DKG artifact under it).
    pub marshal_tip_series: bool,
    /// Which nodes run the production [`crate::cert_inlet::CertInlet`] as a
    /// SECOND producer into their own marshal — see [`CertInletCfg`]. `None` (the
    /// default) leaves every node with the one producer it has always had (its
    /// local BFT engine), so no test written before Э5 5.0а changes behaviour.
    pub cert_inlet: Option<CertInletCfg>,
}

/// Which nodes stand up a cert-inlet and WHICH shape of the one production input
/// they are fed.
///
/// Two of the three sources feed off the node's OWN frontier plane
/// ([`CountingUpstream`] over the production `PlaneUpstreamHandle`), which
/// already reaches the archives of the other nodes over `FRONTIER_CHANNEL`; on
/// production that is the same relationship the WS upstream gives a validator,
/// minus the transport. ADDRESSING a particular peer on the plane is therefore
/// not a new knob either: [`StandConfig::upstream_only_link`] and
/// [`StandConfig::upstream_source_only_for`] already decide who may answer this
/// node's by-height pulls, and they decide it for the inlet's pulls by the same
/// edge they decide it for the marshal's. The third
/// ([`CertInletSource::PeerArchive`]) names its donor itself, because it reads
/// that node's archive directly and no link is involved at all.
#[derive(Clone, Debug, Default)]
pub(super) struct CertInletCfg {
    /// The node indices that run an inlet.
    pub nodes: Vec<usize>,
    /// WHICH certificate the inlet is handed — see [`CertInletSource`]. Inside
    /// this type and not beside it in [`StandConfig`]: it is a property of the
    /// inlet, and the config gains exactly one field either way.
    pub source: CertInletSource,
}

/// The three shapes of the one stream a production inlet consumes.
///
/// Production hands the inlet a LIVE stream of the upstream's newest
/// finalizations (`node/src/cert_inlet.rs`, the WS `finalized_rx` loop) — never a
/// by-height walk; the by-height pull is the MARSHAL's gap repair. The stand has
/// no WS actor, so the first two shapes are built from the same
/// [`crate::cert_follow::CertUpstream`] handle, and BOTH are needed: the walk is
/// the only way to hand the inlet a contiguous run of certificates (and
/// therefore CONSECUTIVE data faults), while the frontier is the only way to
/// hand a node a certificate from an epoch ABOVE its own anchor (the walk asks
/// `tier-F + 1`, whose epoch is at most `epoch(anchor) + 1`, and the committee
/// module answers every epoch up to `epoch(anchor) + 2`:
/// `committee/store.rs::window`, `committee/mod.rs::Geometry::commit_height`).
///
/// Neither PLANE shape can reach the inlet's non-fault deferral, and that is a
/// property of the plane rather than of the fixture: `FrontierHandler::deliver`
/// classifies the same three committee refusals one layer up and step (5) DROPS
/// the answer (`plane_upstream.rs:447-449`, `:485-489`), so the highest
/// certificate a plane feeder can be handed is the top of this node's own read
/// window. [`Self::PeerArchive`] is the third shape for exactly that reason.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CertInletSource {
    /// The next height above this node's own tier-F, pulled by height
    /// (`CertUpstream::get_finalization`) — the plan's shape for 5.0а, and what
    /// a second producer on a node whose EL is behind actually needs to be fed.
    #[default]
    NextAboveTier,
    /// The upstream's live frontier (`CertUpstream::get_latest`) — production's
    /// own inlet input, and the one certificate a node whose execution has
    /// stopped still receives.
    Frontier,
    /// Node `from`'s marshal ARCHIVE, walked upward by height
    /// (`FrontierMarshal::pair_at` on that node's own `MarshalMailbox`) — the
    /// FIRST of the two inputs the plan row names, and the only one with NO
    /// `deliver` gate in front of it.
    ///
    /// It is the stand's stand-in for the production WS stream in the one
    /// respect neither plane shape can model: the WS upstream hands the inlet
    /// whatever it sends, INCLUDING a certificate of an epoch this node's
    /// committee anchor cannot read, which is the inlet's own deferral regime
    /// (`cert_inlet.rs:618-640`). The walk is OURS rather than the plane's
    /// answer, so it climbs strictly and never repeats a height — and that is
    /// also what makes its ingest count a number worth comparing with the
    /// frontier feeder's.
    PeerArchive {
        /// The DONOR node, whose marshal archive is read directly. No link, no
        /// resolver, no `deliver`: the donor's own `MarshalMailbox`, taken by
        /// late binding out of its [`NodeHandles::marshal`] slot.
        from: usize,
    },
}

/// What one node's cert-inlet task did, live — the handles the task writes and
/// the collect phase snapshots into [`CertInletFacts`].
#[derive(Clone, Default)]
struct CertInletObs {
    /// Certificates handed to [`crate::cert_inlet::CertInlet::ingest`]. Every
    /// outcome of one is a skip, so this counts ATTEMPTS and nothing else.
    ingests: Arc<AtomicU64>,
    /// [`crate::cert_inlet::RotateUpstream`] invocations — the ONE externally
    /// visible effect of `record_data_fault` reaching `MAX_UPSTREAM_FAULTS`
    /// (`cert_inlet.rs:758-772`). The inlet's `consecutive_faults` is private, so
    /// this count over a known number of bad certs is what pins it.
    rotations: Arc<AtomicU64>,
    /// Every height the inlet handed its marshal, in order — recorded by
    /// [`RecordingSink`] at `MarshalSink::verify_block`, the first of the two
    /// marshal calls on the clean path of `CertInlet::ingest` and a line no
    /// fault or deferral arm reaches. So this is the list of certificates that
    /// passed the verify gate, observed at the marshal seam the inlet exists to
    /// drive rather than through the beacon-clock tee 5.4-А removed (the tee
    /// stood one statement above the same call and fired on the same set).
    delivered: Arc<Mutex<Vec<u64>>>,
    /// The inlet's own `dpos_cert_inlet_committee_read_deferred_total` family,
    /// held here rather than registered: the counter is shared with the inlet
    /// through `with_committee_read_deferred_metric` (the follower launch wires
    /// it the same way, `consensus/src/dpos.rs`), so reading it needs no
    /// exposition parsing and no label plumbing.
    defers: prometheus_client::metrics::family::Family<
        crate::cert_inlet::CommitteeReadDeferLabels,
        prometheus_client::metrics::counter::Counter,
    >,
    /// The inlet's own carry-forward verify-failure counter, wired the same way
    /// (`with_carry_forward_fail_metric`; the follower launch wires it too,
    /// `consensus/src/dpos.rs`). It increments on ONE line —
    /// `cert_inlet.rs:666-668`, a BLS verify failure taken while the epoch's key
    /// WAS resolvable — so it is the direct witness that a node held the epoch
    /// key at the moment it judged a certificate, which no end-of-run artifact
    /// map can be.
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

/// The inlet's marshal, with the heights it is handed recorded: the production
/// `MarshalMailbox` behind the same `MarshalSink` seam the inlet's own unit
/// tests use for their `FakeMarshal`, so nothing the inlet does changes — every
/// call is forwarded — and the list of verified heights is read where the
/// inlet writes it.
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

/// The [`OuterBuilder::blocker`] slot. ONE label for two consumers, because the
/// builder carries ONE field and splitting it would be a production change:
/// `marshal_p2p::Config.blocker` on both resolver arms (`outer.rs:1267`
/// `Hybrid`, `:1302` `Plane`) and the simplex engine's own blocker
/// (`outer.rs:1057` → `epoch_manager.rs:1750` → `engine.rs:290`), which is the
/// batcher's evidence-free `block!`. A call from either is a call on this label;
/// WHICH of the two made it is read off the `block!` macro's own
/// `tracing::warn!` in [`Outcome::logs`].
pub(super) const BLOCKER_SITE_CONSENSUS: &str = "consensus";
/// The frontier plane's own resolver (`stand.rs::frontier_plane`, modelled on
/// `node/src/dpos.rs::build_beacon_plane`) — the slot `plane_upstream::deliver`
/// step (5) decides with its `bool`.
pub(super) const BLOCKER_SITE_FRONTIER: &str = "frontier";

/// Every [`commonware_p2p::Blocker::block`] one node's stack made, and every
/// blocker SLOT the stand wired this spy into.
///
/// Why a spy and not [`NoopBlocker`], which is what both slots took before 5.2:
/// on a no-op blocker "no peer was excluded" holds by construction, for any
/// code, so an assertion on it is vacuous. This type answers the same question
/// by COUNTING. It still blocks nobody — the run is byte-identical to a
/// `NoopBlocker` run — but a `block(peer)` the production wiring makes is now a
/// fact in the [`Outcome`].
///
/// [`Self::sites`] is the anti-vacuity half, and it is load-bearing rather than
/// decorative: an edit that put `NoopBlocker` back into either slot would leave
/// [`Self::calls`] empty and every assertion on it GREEN. Registration happens
/// in [`BlockerSpy::at`], which is the only way to obtain the blocker at all, so
/// "both slots took this spy" is something the run asserts instead of something
/// the reader has to check in the source.
#[derive(Clone, Default)]
pub(super) struct BlockerSpy {
    calls: Arc<Mutex<Vec<(&'static str, PeerPubkey)>>>,
    sites: Arc<Mutex<BTreeSet<&'static str>>>,
}

impl BlockerSpy {
    /// Hand this spy to ONE blocker slot, recording that the slot took it.
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
    /// The slots this node's spy was wired into, sorted. EMPTY would mean the
    /// node wired the spy nowhere, which makes `calls` say nothing.
    pub sites: Vec<&'static str>,
}

/// The randomness surface every node runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Beacon {
    /// `StaticRandomness`: a fixed anonymous sharing dealt from the full node
    /// set, σ on demand for any epoch, no DKG plane. The regression fixture of
    /// the first two sessions' tests.
    Static,
    /// The production beacon plane (`beacon::build`) on every node: a live DKG
    /// on `BEACON_CHANNEL` / `BEACON_RESOLVER_CHANNEL` of the consensus network,
    /// the epoch-key agreement instances on the plane's mux sub-channels, the
    /// key / seed / artifact journals on the runner's storage under a per-node
    /// prefix, shares in `share_root/node{i}`. `committee_for` and the
    /// `dkgQual` bit come from the schedule (`dkgQual[e] = committee[e] !=
    /// committee[e-1]`, the contract's rule).
    Live,
}

/// The tracked peer set of the CONSENSUS plane (the set the marshal's p2p
/// resolver and the buffered broadcast draw peers from) and what happens to the
/// links of a node that falls out of it.
///
/// The stand runs TWO simulated networks: the consensus plane (vote, cert,
/// resolver, broadcast, marshal channels) and the upstream plane
/// (`FRONTIER_CHANNEL`), so "no consensus-plane link, but a link to a frontier
/// provider" can be expressed at all — the simulated network delivers every
/// channel over the same link. The upstream plane tracks every node at index 0
/// for the whole run. On the authenticated transport there is ONE connection
/// per peer, so the configurations below map onto production like this: an
/// unregistered joiner has neither (`Committee { upstream_link: false }`); a
/// registered validator that lost its seat has both, but its marshal is fed by
/// nothing the plane broadcasts once it has no engine for the epoch — the
/// frontier probe is what keeps it following (§9.6.1) — which
/// `Committee { upstream_link: true }` isolates by leaving it the frontier link
/// only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PeerSet {
    /// Every node, tracked once at index 0 — production's primary
    /// `committee[E-1] ∪ committee[E] ∪ committee[E+1]` for a committee that
    /// never changes.
    AllNodes,
    /// `committee[E]` only, re-tracked at index `E` when the chain enters epoch
    /// `E`, and the consensus-plane links of a node outside the set are REMOVED
    /// (the authenticated transport kills an untracked peer's connection at its
    /// next tick; the simulated network keeps delivering over a link whatever
    /// the tracked set says, so the severance has to be modelled by hand).
    /// `upstream_link` says whether the outsider keeps its `FRONTIER_CHANNEL`
    /// links: `false` is an unregistered joiner (no peers at all), `true` a
    /// node whose only path to the chain is the upstream plane. Not combinable
    /// with `Stand::partition` (both own the links).
    Committee { upstream_link: bool },
    /// `committee[E]` tracked as above, but EVERY link left in place: the
    /// simulated network then keeps delivering to the outsider over the links
    /// the tracked set no longer covers — the boundary between "simulated
    /// network" and "authenticated transport", pinned by a test of its own.
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

/// WHO sits in each epoch's committee — the stand's INPUT. It says nothing
/// about WHEN an epoch becomes readable or at WHICH state hash: that is
/// [`FakeStaking`]'s answer, from the contract's commit rule.
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
    /// `Beacon::Live` only (R-002): this node deals TWICE. Its `BEACON_CHANNEL`
    /// sender splits the one `Reveal` broadcast `seal_dealings` emits — the
    /// original log to every other member, a second, independently dealt but
    /// validly signed log of the same dealer over the same `Info` to node 0.
    /// `withhold_partials` additionally rebuilds this node's signer scheme over
    /// the epoch's verify-only oracle, so it produces no seed partial and
    /// therefore casts no vote (`combined_scheme.rs:284-287`) — the second link
    /// of the register entry.
    #[cfg(feature = "dpos-devnet-byzantine")]
    TwoReveals {
        withhold_partials: bool,
    },
    /// (R-008) This node's frontier-plane `Producer` serves `Finalized{h}`
    /// certificates with a forged σ slot for `h` in
    /// [`byzantine_roles::FORGE_WINDOW`], leaving the multisig half untouched.
    #[cfg(feature = "dpos-devnet-byzantine")]
    ForgedSeedUpstream,
    /// (R-004) This node's frontier-plane `Producer` answers `FrontierKey::Latest`
    /// with the real tip whose `block.height` is inflated by
    /// [`byzantine_roles::LATEST_INFLATION`] (payload re-pointed so the structural
    /// gate passes). Before the frontier trust gate a victim believed the height,
    /// raised its re-jump trigger past any real one and re-jumped every tip.
    #[cfg(feature = "dpos-devnet-byzantine")]
    InflatedProbe,
    /// (R-001 var Б) This node's frontier-plane `Producer` answers
    /// `FrontierKey::Latest` with the real tip whose `block.result` is replaced by
    /// a hash the honest devp2p peer does not serve (payload re-pointed, multisig
    /// left real). A victim re-jumping onto it drives reth's EL-sync at an
    /// unservable branch.
    #[cfg(feature = "dpos-devnet-byzantine")]
    LyingUpstream,
    /// (R-009) This node's frontier-plane `Producer` answers a `Finalized{h}`
    /// by-height pull (for `h` in [`byzantine_roles::WRONG_HEIGHT_WINDOW`]) with its
    /// OWN valid pair of height `h − 1` instead of `h`. Nothing is mutated — the pair
    /// is a wholly real, self-consistent finalization, just of the wrong height. A
    /// validator catching up by the plane path has its by-height gap "satisfied" by a
    /// pair that does not close it, so the gap never fills from this peer.
    #[cfg(feature = "dpos-devnet-byzantine")]
    WrongHeightFinalized,
    /// `Beacon::Live` only: this node brings up NO beacon plane (no share, no
    /// dealer log, no agreement seat, no `BEACON_CHANNEL` registration) and
    /// runs `beacon::absent` instead — a verifier that never signs, from epoch
    /// 0 on. This models a validator WITHOUT a beacon module, not a committee
    /// member that merely failed to deal: such a member still receives the
    /// others' dealings, recovers its share over the pinned set and keeps
    /// signing (`DkgActor::drive_finalization`'s doc). For the ceremony the
    /// effect is the same — one dealer fewer.
    AbsentBeacon,
    /// `Beacon::Live` only (5.3-В): a REGISTRY-TIER neighbour that deals anyway.
    /// The node is in the registry (`PeerSet::AllNodes`) and, by the committee
    /// schedule, in no committee; once a second it puts a real, decodable
    /// `Commitment` dealing for the epoch after its own on the `BEACON_CHANNEL`
    /// to everyone. Every member's pre-decode gate (`GatedReceiver`, the one
    /// sender classification on the channel) classifies it `Tracked` and refuses
    /// the frame as `secondary` before any byte of it is decoded; the actor never
    /// sees it. The node's own beacon plane runs as an honest non-member's would.
    /// `Outcome::stray_dealer_sends[i]` counts the `(frame, recipient)` pairs it
    /// put on the wire, so a test can assert the refusals EXACTLY.
    StrayDealer,
}

/// WHICH of the two simulated networks a [`Partition`] cuts.
///
/// `Both` is a physical partition: the machine is unreachable, and nothing —
/// neither plane — reaches the isolated side.
///
/// `ConsensusOnly` leaves the `FRONTIER_CHANNEL` links in place and removes the
/// consensus-plane ones (votes, the marshal's own resolver, `BEACON_CHANNEL` and
/// `BEACON_RESOLVER_CHANNEL`). It is the same shape
/// [`PeerSet::Committee { upstream_link: true }`](PeerSet::Committee) already
/// pins as a production regime — "a node whose only path to the chain is the
/// upstream plane" — with the severance taken at a height the test names instead
/// of at a tracked-set transition, which is what a fixture needs whenever the two
/// do not coincide: the tracked set is
/// `committee[E-1] ∪ committee[E] ∪ committee[E+1]`, so a node rotated out at E
/// keeps its consensus links for the whole of epoch E and part of E+1.
///
/// It is the ONLY generator left for "following the chain without an epoch's
/// key". Since П-3 the epoch key is an artifact any node can ASK for over
/// `BEACON_RESOLVER_CHANNEL` (`beacon::artifact::TransportAcquire`), which lives
/// on the consensus plane: a node that still has that plane gets the key whether
/// it is in the committee or not (R-121/R-122), so non-membership no longer holds
/// an execution cursor back. A node cut from the consensus plane is fed
/// certificates by its frontier probe and can verify them vote-only, so its
/// marshal still climbs to the ordering plane's two-epoch ceiling while its
/// EXECUTION parks at the boundary of the last epoch it holds a key for — the
/// state the deep-lag fixtures are about.
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
    /// Heal once the HIGHEST executed tip in the run is at or above this height,
    /// instead of after [`Self::duration`]. `None` = heal on the duration.
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
    /// Hold the cut for `views` leader timeouts of `ConsensusTimeouts::fluent_1s`
    /// — the time the split halves need to nullify that many views.
    pub(super) fn for_views(self, views: u32) -> Self {
        self.0.duration = ConsensusTimeouts::fluent_1s().leader * views;
        self
    }
    /// Hold the cut until the chain the isolated node is missing has reached
    /// `height`, rather than for a wall-clock span — see [`Partition::heal_above`].
    ///
    /// Every fixture that cuts a node in order to make it FALL BEHIND wants the
    /// lag in blocks, and a duration only reaches that through the pacing: at
    /// `leader = 1750ms` a `for_views` count is not the block count it reads as,
    /// and a heal that lands after the run's stopping predicate has fired is a cut
    /// that silently never healed.
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
/// [`crate::committee::CommitteeRecord::same_value`] compares — everything the
/// contract froze and nothing about WHERE it was read.
///
/// `members` is the peer half in CONTRACT ORDER (the consensus index space),
/// not the sorted projection: two nodes that agree on the set but not on the
/// order hold two different committees as far as `leaderIndex` is concerned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CommitteeFacts {
    pub members: Vec<PeerPubkey>,
    pub weights: Vec<u128>,
    pub changed: bool,
    /// `(height, executed hash)` this node read the record AT — the DIAGNOSTIC
    /// leg, and the one a cross-node equality assertion must NOT include. Two
    /// nodes at different heights hold the same record with different anchors,
    /// which is the whole property; it is carried so a test can assert the
    /// anchors really DID differ instead of assuming it.
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

/// Every counter the recorder holds right now, taken ONCE.
///
/// `Snapshotter::snapshot` RESETS every counter it reads
/// (`metrics-util-0.20.4/src/debugging.rs:109`, `c.swap(0, …)`), so a second
/// call answers zero for everything the first one took. Draining into a `Vec`
/// and querying that is the only shape in which more than one family can be
/// asserted about the same window — and the reset is also what makes the
/// stand's pre-collect drain ([`StandConfig::metrics_snapshotter`]) SPLIT the
/// run from the post-run poll instead of shadowing it: whatever the stand takes
/// is gone from the caller's own later drain.
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
/// PROCESS-WIDE, not per node: `metrics::counter!` carries no node label, and
/// the stand's N nodes share one recorder (the deterministic runner is single
/// threaded, which is what makes a thread-local recorder see the whole run at
/// all). Every assertion on it is therefore a sum over the nodes.
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

/// [`counter_of`] restricted to EVERY label pair in `labels` at once — for a
/// family that carries more than one label (`dpos_ingress_dropped_total` has
/// `channel` AND `reason`), where a one-label filter sums across the other.
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
/// in call order. Both tiers, because which tier a key lands in is what 4.3
/// changed and a union cannot answer it.
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
    /// `upstream_served[i]` = every BY-HEIGHT pull node `i`'s upstream client
    /// made, in order — see `Pull` and `UpstreamCounters::served_heights`. The
    /// ladder test reads it to separate an ADDRESSED RUNG from the contiguous
    /// catch-up traffic beside it (review B1-03); `upstream[i].finalized_calls`
    /// counts the same events without the heights or the targeting.
    pub upstream_served: Vec<Vec<Pull>>,
    /// Every node whose `SafetyHalt` latch is engaged, with the typed reason.
    pub halted: Vec<(usize, String)>,
    pub traces: Vec<Vec<TraceEntry>>,
    /// `seeds[i][h]` = the σ node `i`'s executor derived height `h` from, for
    /// every `h <= heights[i]` (`None` = seedless).
    pub seeds: Vec<BTreeMap<u64, Option<Seed>>>,
    /// `artifacts[i][e]` = the wire bytes of the agreed epoch-key artifact node
    /// `i` holds for epoch `e` (`Beacon::Live`; empty otherwise). Read through
    /// the plane's own `ArtifactSource` — what `consensus_getEpochArtifact` serves.
    pub artifacts: Vec<BTreeMap<u64, Vec<u8>>>,
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
    /// `PeerSetSink`. The two tiers apart, because which tier a key lands in is
    /// what 4.3 changed: primary is the three committee records
    /// (`C[E−1] ∪ C[E] ∪ C[E+1]`), secondary the Active registry.
    pub peer_sets: Vec<TrackedRegistrations>,
    /// How many times two nodes' transitions tracked DIFFERENT peer sets for the
    /// same epoch. Must be zero.
    pub tracked_mismatches: u64,
    /// Peer-set registrations that actually reached the simulated network.
    pub tracked_forwarded: u64,
    /// `staking_reads[i]` = what node `i` ASKED the fake staking state, split by
    /// whether the epoch was committed at the read height.
    pub staking_reads: Vec<StakingReads>,
    /// `committee_records[i][e]` = what node `i`'s committee module answers for
    /// epoch `e`, for every epoch the run passed through plus the two-epoch
    /// lookahead — the frozen record's VALUE legs, or the `Display` of the
    /// refusal with whether it is transient.
    ///
    /// Read through the `Committee` trait at the END of the run, and therefore
    /// AFTER [`Self::staking_reads`] is snapshotted: an epoch the node already
    /// read is a map hit that costs no call, and an epoch it never read is a
    /// fresh read whose cost must not land in the read counters the tests
    /// assert on. The legs are exactly [`crate::committee::CommitteeRecord`]'s
    /// write-once ones — `snapshot` is deliberately absent, because two nodes
    /// holding the same record at different anchors differ there and only there
    /// (`committee/mod.rs::CommitteeRecord::same_value`).
    pub committee_records: Vec<BTreeMap<u64, Result<CommitteeFacts, CommitteeRefusal>>>,
    /// `committee_verifier_epochs[i]` = the epochs node `i`'s module holds a
    /// VERIFY-ONLY scheme for, at the end of the run. A map lookup, no read.
    pub committee_verifier_epochs: Vec<Vec<u64>>,
    /// `committees[i]` = node `i`'s own committee module, held past the end of
    /// the run so a test can ask it the questions the two maps above do not
    /// answer — chiefly [`crate::committee::Committee::scheme`], which covers
    /// the SIGNER schemes [`Self::committee_verifier_epochs`] deliberately
    /// omits (`committee/store.rs:577-587` filters on `me().is_none()`). It is
    /// the same `Arc` the node ran on, not a second module.
    pub committees: Vec<Arc<dyn crate::committee::Committee>>,
    /// Every `metrics::counter!` value the run produced, drained BEFORE the
    /// collect phase — empty unless [`StandConfig::metrics_snapshotter`] was
    /// set. This is the counter set an assertion about what the RUN asked must
    /// read: the caller's own drain after `run_until` returns sees only what
    /// the collect phase added on top (and is therefore the way to measure that
    /// contribution).
    pub metrics_before_collect: Vec<CounterSample>,
    /// `jump_calls[i]` = every steady-state re-jump call node `i` made, in call
    /// order, with the `JumpOutcome` VARIANT the production function returned,
    /// the certificate it consumed and the landing it chose. A refused jump is
    /// invisible anywhere else in this struct — see `fakes::JumpCall`.
    pub jump_calls: Vec<Vec<JumpCall>>,
    /// `el_events[i]` = every EL tier transition node `i` made, IN ORDER — each
    /// `derive_and_execute` insert into the executed tree and each
    /// canonicalization an FCU (or a jump landing) committed. See
    /// [`fakes::ElEvent`](super::fakes::ElEvent): the two tiers only mean
    /// something relative to each other, so "the guard read `h` while `h` was
    /// still tree-only" is an INDEX comparison in this one log and not the
    /// absence of a line in two.
    pub el_events: Vec<Vec<ElEvent>>,
    /// `marshal_tip_series[i]` = node `i`'s MARSHAL TIP sampled once per driver
    /// tick, in order — §5.2's one frontier, and since 4.2 the only input of the
    /// re-jump trigger. Empty unless [`StandConfig::marshal_tip_series`]; see that
    /// field for why the sample is the marshal's own `finalized_height` gauge and
    /// not a mailbox read.
    ///
    /// It replaces `upstream_frontier_series`, which sampled the atomic the
    /// frozen-tip probe `fetch_max`ed the SERVED height into. That atomic is gone,
    /// and with it the thing a lying upstream could inflate: what a test asks now
    /// is whether the victim's own VERIFIED tip moved.
    pub marshal_tip_series: Vec<Vec<u64>>,
    /// `frontier_steps_named_series[i][t]` = how many ladder steps node `i`'s
    /// probe had named by driver tick `t` — a running length of
    /// [`Self::frontier_steps`], sampled in the same loop as
    /// [`Self::marshal_tip_series`] and only when it is enabled.
    ///
    /// It is what turns the step list into something datable: the tick a given
    /// rung was FIRST named is the first `t` where this count exceeds that rung's
    /// index, so "the tip reached the rung within N ticks of it being named" is an
    /// index comparison between two series the driver took for free, with no read
    /// inside the probe closure (Д-81).
    pub frontier_steps_named_series: Vec<Vec<usize>>,
    /// `probe_calls[i]` = how many times node `i`'s frozen-tip probe asked the
    /// upstream. Read only by the R-004 role test.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub probe_calls: Vec<u64>,
    /// `frontier_steps[i]` = every ladder step node `i`'s frozen-tip probe NAMED,
    /// as `(T, last(T+1))` in call order, one entry per tick (§5.2). Empty on a
    /// node whose tip never froze — a healthy validator learns finalizations from
    /// consensus and never reaches the probe body at all.
    ///
    /// NAMED, not PUT: whether the executor then hands the step to the marshal is
    /// decided inside `executor::probe_frontier` against `last_tip_height`, and the
    /// stand has no free seam on that — the executor takes the real `MarshalMailbox`
    /// from `OuterEngine`, and sampling the tip from the probe closure perturbs the
    /// run (see the closure). That half is pinned at the unit level in `executor.rs`.
    pub frontier_steps: Vec<Vec<(u64, u64)>>,
    /// `cert_inlet[i]` = what node `i`'s cert-inlet task did, or `None` when the
    /// node ran none (which is every node unless
    /// [`StandConfig::cert_inlet`] names it). See [`CertInletFacts`].
    pub cert_inlet: Vec<Option<CertInletFacts>>,
    /// `blocked[i]` = every peer node `i`'s stack asked to BLOCK, per wired
    /// blocker slot, plus the slots that took the spy. See [`BlockerSpy`] —
    /// chiefly for why the `sites` half has to be asserted beside the `calls`
    /// half or the whole observable re-vacuums itself.
    pub blocked: Vec<BlockerFacts>,
    /// `byz[i]` = what node `i`'s byzantine wrappers actually did — all zeros on
    /// an honest node and on a build without `dpos-devnet-byzantine`. The tamper's
    /// own witness: a role test asserts THIS before it asserts anything about how
    /// the other nodes reacted.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub byz: Vec<ByzFacts>,
    /// `head_gap_series[i]` = `best − ordering_finalized` of node `i`, sampled
    /// once per driver tick: the canonical (speculative) head of its EL minus
    /// the height its executor last finalized-executed. The width of the band
    /// a committee anchor at `ordering_finalized` gives up against a cursor at
    /// the executed head. A measurement, never an assertion. The tick is
    /// coarser than the speculative lead, so the event-driven histograms below
    /// are the ones that see it.
    pub head_gap_series: Vec<Vec<u64>>,
    /// `head_gap_on_fcu[i]` = `gap → count` over every FCU that moved node
    /// `i`'s canonical chain (`FakeChain::head_gap_on_fcu`).
    pub head_gap_on_fcu: Vec<BTreeMap<u64, u64>>,
    /// `head_gap_on_landing[i]` = the same over every jump landing.
    pub head_gap_on_landing: Vec<BTreeMap<u64, u64>>,
    /// `archive[i][h]` = whether node `i`'s marshal holds a finalization
    /// certificate and a finalized block at height `h` at the end of the run,
    /// for every `h` up to the highest tip — read through the marshal mailbox's
    /// `get_finalization` / `get_block(height)`, the same two archive reads
    /// `handle_produce` serves a peer's `Finalized{h}` from. Empty unless
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
    /// the `BEACON_CHANNEL` as a `Role::StrayDealer`, as the simulated network
    /// accepted them (`Sender::send`'s returned recipients). Zero for any other
    /// role. Every one of them is refused at the recipient's pre-decode gate, so
    /// the sum over nodes is the exact `secondary` count the beacon channel
    /// reports (5.3-В).
    pub stray_dealer_sends: Vec<u64>,
    /// `tombstones_observed[i]` = every `(height, peer)` node `i`'s
    /// `TombstoneSet::observe` newly recorded off the committee snapshot it read
    /// at that finalized height — the delta production reacts to
    /// (`node/src/dpos.rs`, the tombstone watch on the beacon plane poller), in
    /// call order. Empty unless `StandConfig::tombstoned` names someone.
    pub tombstones_observed: Vec<Vec<(u64, PeerPubkey)>>,
    /// How many times the SIMULATED network failed to return a send-ack — see
    /// [`Outcome::SIMULATOR_ACK_DROP`]. Counted rather than hidden: the
    /// exemption in [`Outcome::errors`] must never be able to swallow anything
    /// else, and a run where this grows without bound is worth looking at even
    /// though no single occurrence means a fault.
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

    /// The ONE ERROR line the stand does not count against a run, matched on the
    /// full `target: message` prefix and nothing looser.
    ///
    /// What it is, by code: `simulated::Sender::send` enqueues the message with a
    /// oneshot ack channel and then awaits it
    /// (`CW:p2p/src/simulated/network.rs:879-887`); the network replies at
    /// `:774-776` and logs this line when the reply fails, which happens only if
    /// the receiver was dropped — that is, the task that called `send` was
    /// dropped between the enqueue and the ack. The message itself was already
    /// handed to the transmitter for every recipient the reply names, so nothing
    /// was lost on the wire.
    ///
    /// When it fires here: at an epoch boundary that registers a NEW tracked peer
    /// set. Registration is not the cause but the trigger — it adds an await
    /// inside the network actor's loop (`:406-419` runs `broadcast_peer_list`)
    /// and can make a peer stop being connectable once the oldest of the four
    /// retained sets is evicted (`:317-345`) — while `abort_below`
    /// (`epoch_manager.rs:1381-1395`) is dropping the outgoing epoch's engine,
    /// which is what leaves a send mid-flight. Establishing the mechanism was
    /// code; attributing the dropped task to the aborted engine was experiment
    /// (it appears exactly on the boundaries that register a new set and not
    /// otherwise).
    ///
    /// Why it is not production: the authenticated transport has its own
    /// registration path (`authenticated::discovery`) and no oneshot ack behind
    /// each `send`, so this failure has no counterpart there. The stand cannot
    /// reorder it away either — the ordering that produces it is production's
    /// own (`EpochTransition` tracks the peer set BEFORE it fires the boundary
    /// trigger, `epoch_transition.rs:647` then `:660`, so the INCOMING epoch's
    /// engine never sends on an unregistered set; the send that is lost belongs
    /// to the OUTGOING engine the manager aborts).
    ///
    /// THE SAME MESSAGE IS PRINTED BY A SECOND, UNRELATED BRANCH, and that one
    /// must NOT be exempt: at `:686-695` the network drops a message outright
    /// because its ORIGIN is not in any tracked peer set, and reports it with the
    /// same text. The two are told apart by the field the macro renders: that
    /// branch replies `Vec::new()` (`err=[]`) while the ack path replies the
    /// recipient list it already delivered to (`err=[…]`, non-empty). Only the
    /// non-empty form is exempt, so a genuinely dropped message still fails a
    /// test — which matters under `PeerSet::Committee`, where a node CAN fall out
    /// of the four retained sets.
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

    /// The `(height, view, leader, digest, hash, σ)` trace of node `i`, as
    /// bytes — the object test (6) compares across runs. σ is the seed the
    /// executor derived the height from (`0xFF` = none).
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

    /// Every `commonware_resolver::p2p` engine's `peers_blocked` gauge in the
    /// run, as `(node, family, value)` — every node, every resolver, in
    /// exposition order. The family is the full exposition key
    /// (`node{i}_<label chain>_peers_blocked`), so the caller can tell the
    /// engines apart by the labels the stand gave them.
    ///
    /// The SECOND observable of "was a peer excluded", and it is not a duplicate
    /// of [`BlockerSpy`]: the two see the same LINE but different SETS.
    /// `handle_network_response` runs `block!(self.blocker, peer, ..)` and
    /// `self.fetcher.block(peer)` back to back on a `deliver == false` verdict
    /// (CW `resolver/src/p2p/engine.rs:437-438`), and the second call is the one
    /// no `Blocker` choice can disarm: it inserts the peer into the fetcher's
    /// own `excluded` set (`fetcher.rs:516`), which is APPEND-ONLY — it is read
    /// at `:242` (peers filtered out of every future request) and `:567`, and
    /// nothing anywhere removes from it. So a `NoopBlocker` still leaves the
    /// peer excluded from THAT resolver's fetches for the life of the engine,
    /// and this gauge — `fetcher.len_blocked()` — is where that shows.
    ///
    /// What it covers that the spy does not: EVERY resolver engine the runner
    /// registered, including the ones the stand never hands a blocker to at all
    /// (the beacon plane's and the DKG log's, `beacon/plane.rs:330` and
    /// `beacon/dkg_engine.rs:343`, which build their `NoopBlocker` inside).
    /// What the spy covers that it does not: the peer's identity and the slot —
    /// this is a bare count under a family name.
    ///
    /// WHAT A ZERO PROVES, exactly: the gauge is written in ONE place, the
    /// select loop's `on_start` arm (`engine.rs:174-178`, the only
    /// `peers_blocked` site in the checkout), which runs at the top of every
    /// iteration. So a zero says "no exclusion had been taken as of that
    /// engine's LAST loop iteration". An exclusion taken in an arm after which
    /// the engine never iterated again — the run ending mid-arm, or the engine
    /// being stopped — is not in the exposition. The windowless witness of the
    /// same line is the `block!` macro's own WARN
    /// (`commonware_resolver::p2p::engine: invalid data received`), which lands
    /// in [`Outcome::logs`] synchronously when capture is live; assert the two
    /// together.
    ///
    /// Panics on a value that does not parse AND on a `_peers_blocked` family
    /// that is not under a `node{i}_` chain: a family that is present but
    /// unreadable or unattributable must not vanish into "nothing was excluded".
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

/// One `EpochTransition` call and what it answered — the stand's window into the
/// state machine that now walks the boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct EtStep {
    /// The finalized height handed to `cold_start` / `on_finalized`.
    pub number: u64,
    pub outcome: Result<TransitionOutcome, String>,
}

/// One boundary trigger the transition delivered: the epoch and the state it
/// read the committee at. `block_hash` is the executed hash of
/// `boundary − result_lag` (`epoch_transition.rs:290-293`), or the cold-start
/// anchor for the first entry.
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
    /// `EpochTransition::frozen_geometry()` sampled after every call to it.
    /// Sampled rather than read at collection time BECAUSE reading it there
    /// needs `.await` on the transition's mutex, and an extra yield in the
    /// collection phase lets the torn-down tasks emit one more round of network
    /// sends whose acks then fail — an ERROR line that says nothing about the
    /// run. The value is write-once in the transition itself
    /// (`epoch_transition.rs:44-65`), so a sample after the last call is the
    /// final value.
    geometry: Arc<Mutex<Option<(u64, u64)>>>,
}

/// The `PeerSetSink` half of the production wiring: `EpochTransition` hands the
/// assembled `TrackedPeers { primary: committee[E-1] ∪ committee[E] ∪
/// committee[E+1], secondary: active_registry }` set here and the
/// simulated network's `Manager::track` takes it, exactly as the node hands it
/// to the real `Oracle` (`node/src/dpos.rs:1566`, `consensus/src/dpos.rs:2036`).
///
/// Two differences the stand cannot avoid, both from having ONE simulated
/// Oracle stand in for N production ones:
///
/// * N nodes track the same epoch. A repeat `track(id, ..)` is refused with a
///   warn and never applied (`CW:p2p/src/simulated/network.rs:269-284`), so
///   only the first node's registration of an epoch reaches the network. Every
///   node's is RECORDED, and the tests compare them.
/// * A set identical to the last one registered is not re-registered. The
///   simulated network keeps only `tracked_peer_sets` (4) sets and evicts the
///   rest, so one id per epoch would push the older sets out for no modelling
///   gain; and registering a new id mid-run drops the ack of a message already
///   in flight, which surfaces as `failed to send ack`
///   (`CW:p2p/src/simulated/network.rs:775`) — a simulator artifact with no
///   consensus meaning. Membership CHANGES are still registered, which is what
///   the peer-set tests observe.
#[derive(Clone)]
struct TrackSink {
    node: usize,
    oracle: Oracle,
    shared: Arc<Mutex<Tracked>>,
    /// THIS node's ingress window — recorded on every `track` BEFORE the set is
    /// forwarded, exactly as production's `OracleHandle::track` records it
    /// (`p2p/src/lib.rs`), and read by this node's beacon `GatedReceiver`, the
    /// one sender classification on that channel. Per node and not shared: the
    /// one simulated Oracle forwards only the FIRST node's registration of an
    /// epoch, but every node's own transition tracks, and the window is what its
    /// own transition said, which is the production shape.
    window: fluentbase_p2p::TrackedWindow,
}

#[derive(Default)]
struct Tracked {
    /// `epoch -> the set the FIRST node's transition tracked for it`. Seeded
    /// with the epoch-0 set the harness registers before any node exists (the
    /// engines need a peer set to be there), so the transitions' own epoch-0
    /// track is a no-op against it.
    by_epoch: BTreeMap<u64, Vec<PeerPubkey>>,
    /// `epoch -> (primary, secondary)` as the FIRST node to reach that epoch split
    /// them. Separate from [`Self::by_epoch`] on purpose: that one is the
    /// link-severing model's input and is deliberately the UNION, which cannot see
    /// two nodes agreeing on who is tracked while disagreeing on which TIER each
    /// one is in — and the tier split is the whole subject of 4.3. Not seeded with
    /// the harness's epoch-0 registration, which has no tier split of its own:
    /// whichever node registers epoch 0 first sets the reference.
    tiers_by_epoch: BTreeMap<u64, (Vec<PeerPubkey>, Vec<PeerPubkey>)>,
    /// `per_node[i]` = every registration node `i`'s transition tracked.
    per_node: Vec<TrackedRegistrations>,
    /// How many times a node tracked a set for an epoch whose TIER SPLIT differs
    /// from the one already recorded for it. Forwarding only the first node's set
    /// would otherwise swallow the disagreement in silence: both tiers are a
    /// function of chain state alone, so a difference means two nodes read
    /// different committees — or sorted the same peers into different tiers — for
    /// one epoch.
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
        // Record BEFORE registering, as production does: the window is what every
        // ingress check reads, and a frame from a member of the new set may
        // arrive the instant the network applies it.
        self.window.record(epoch, &peers);
        let primary = peers.primary();
        let primary_members: Vec<PeerPubkey> = primary.iter().cloned().collect();
        let secondary_members: Vec<PeerPubkey> = peers.secondary.iter().cloned().collect();
        // `by_epoch` (what the link-severing model reads) is both tiers: the
        // authenticated transport keeps a connection to a secondary peer too — it
        // just never dials it (`CW:.../tracker/record.rs:171`, `:341`).
        let connectable: Vec<PeerPubkey> = {
            let mut all = primary_members.clone();
            all.extend(secondary_members.iter().cloned());
            all.sort();
            all.dedup();
            all
        };
        let forward = {
            let mut shared = self.shared.lock().unwrap();
            // The disagreement counter reads the TIERS, not the union: two nodes
            // that put the same peer in different tiers have read different
            // committees, and the union hides exactly that.
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
    /// Frozen-tip probe invocations (R-004 role test only).
    #[cfg(feature = "dpos-devnet-byzantine")]
    probe_calls: Arc<AtomicU64>,
    /// Every ladder step this node's probe named — see `Outcome::frontier_steps`.
    frontier_steps: FrontierSteps,
    trace: Arc<Mutex<Vec<TraceEntry>>>,
    upstream: UpstreamCounters,
    /// `Beacon::Live`: the beacon's artifact read.
    artifacts: Option<ArtifactSource>,
    observer: EtObserver,
    staking: FakeStaking,
    /// The node's ONE committee module, for the post-run record scan.
    committee: Arc<dyn crate::committee::Committee>,
    /// The node's marshal, for the post-run archive scan.
    marshal: Arc<OnceLock<MarshalMailbox>>,
    /// The optional cert-inlet task's observables — `None` when this node runs
    /// no inlet.
    cert_inlet: Option<CertInletObs>,
    /// This node's ONE blocker spy, handed to both blocker slots.
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
    // `getRegistryWithKeys()`. `PeerSet::AllNodes` is the production shape — a
    // registry that holds every activated validator, so a rotated-out member
    // stays in the tracked union (`epoch_transition.rs:600-606`). The
    // committee-only configurations model the OTHER production case, a node
    // that is not in the registry at all, by emptying it: the tracked set is
    // then `committee[E] ∪ committee[E+1]` and an outsider falls out of it.
    let registry: Vec<PeerPubkey> = match cfg.peer_set {
        PeerSet::AllNodes => pks.clone(),
        PeerSet::Committee { .. } | PeerSet::CommitteeTrackedOnly => Vec::new(),
    };
    let bls_pubkeys: Vec<BlsPubkey> = bls
        .iter()
        .map(|k| BlsPubkey::decode(k.public_bytes().as_slice()).expect("bls pubkey"))
        .collect();
    // Seeded in the SAME order the sink records: `PeerSetSink::track` takes a
    // `commonware_utils::ordered::Set`, which is sorted, so seeding the harness's
    // epoch-0 registration in node-index order would read as a disagreement with
    // every node and re-register the set.
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

    // Two simulated networks (see `PeerSet`): the consensus plane and the
    // upstream plane. Every node is a tracked peer of every other one on both
    // at index 0; every pair is linked on both.
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

    // Upstream-plane isolation (see `StandConfig::upstream_only_link`): confine the
    // victim's frontier discovery to the one source, on the UPSTREAM plane only.
    // Every upstream link touching `source` or `victim` is removed except the pair
    // between them, so the resolver can only serve the victim's `get_latest` from
    // the source. The consensus plane is left intact.
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

    // One-sided upstream-plane isolation (see `StandConfig::upstream_source_only_for`):
    // the VICTIM can pull by-height from exactly one peer; that peer keeps every
    // other link it had.
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

    // One marshal slot per node, built HERE rather than inside `build_node`: a
    // cert-inlet on the `PeerArchive` source reads another node's archive, and
    // that node's engine may be built after this one's. Each node fills its own
    // slot the moment its `OuterEngine` exists; nothing else writes them.
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

    // Drive: sample, apply partitions, stop on `pred` or the virtual deadline.
    enum PartState {
        Pending,
        Active(Duration),
        Healed,
    }
    let mut part_state: Vec<PartState> = partitions.iter().map(|_| PartState::Pending).collect();
    // What each cut actually REMOVED, per plane, so the heal restores exactly that
    // and no more. A pair can already be unlinked when the cut fires —
    // [`StandConfig::upstream_only_link`] and
    // [`StandConfig::upstream_source_only_for`] shape the upstream mesh before the
    // run — and re-adding such a pair at the heal would hand the isolated node a
    // source the fixture deliberately took away (in the lying-upstream stands, an
    // HONEST second source, which is the whole contrast). `true` = the consensus
    // plane, `false` = the upstream plane.
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
    // Per-node samples of the MARSHAL TIP, one per driver tick, with the running
    // count of ladder steps named beside it (see `Outcome::marshal_tip_series`).
    // Both are reads of state the driver already holds — the runner's metric
    // registry and the stand's own step log — so neither sends a message into a
    // node.
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
                    // `CutPlanes::Both` is physical — it cuts both planes;
                    // `ConsensusOnly` leaves the frontier plane delivering.
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
            // The links follow what the nodes' `EpochTransition`s actually
            // TRACKED, never the schedule: the tracked set is the state machine's
            // own `TrackedPeers` — primary `committee[E-1] ∪ committee[E] ∪
            // committee[E+1]` plus the secondary registry — and BOTH tiers are what
            // the authenticated transport would keep connections for (it declines to
            // DIAL a secondary peer, it does not refuse one).
            let latest = {
                let t = tracked.lock().unwrap();
                t.by_epoch.last_key_value().map(|(e, m)| (*e, m.clone()))
            };
            if let Some((epoch, members)) = latest {
                if epoch > tracked_epoch {
                    {
                        // Sever every consensus-plane link that touches a node
                        // outside the set (and its upstream-plane links unless
                        // the config keeps them); restore the links among
                        // members (a re-seated node).
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

    // Collect.
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
    // The run's `metrics::counter!` values, taken HERE — after `staking_reads`
    // and `logs`, and before the committee scan below, which is the only
    // producer of counter increments that is not the run. Draining resets the
    // recorder, so the caller's own drain afterwards holds exactly the scan's
    // contribution.
    let metrics_before_collect = match &cfg.metrics_snapshotter {
        Some(snap) => drain_counters(snap),
        None => Vec::new(),
    };
    // The committee module's own answer per node, LAST — after
    // `staking_reads` above, so a record this run never asked for is read here
    // without landing in the counters the tests assert on. Two epochs past the
    // highest tip is the module's own lookahead ceiling; above that every node
    // answers `OutOfWindow` and the entry says so.
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

/// Node `i`'s MARSHAL TIP out of one runner-registry exposition — the marshal's
/// own `finalized_height` gauge, which it sets on exactly the line that emits
/// `Update::Tip` (CW `marshal/core/actor.rs:1454-1458`, restart seed `:398-401`).
/// `0` when the node has not published one yet.
///
/// The label chain is the stand's: `node{i}` → `outer` (`OuterBuilder::build` is
/// started on `ctx_i.with_label("outer")`) → `marshal` (`outer.rs`'s
/// `MarshalActor::init`), which prometheus-client flattens into
/// `node{i}_outer_marshal_finalized_height`. Same family
/// `testbed::preconditions` already reads through [`Outcome::metric`].
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
/// executed different hashes. A STRICT majority (more than half of the nodes
/// present at that height) names the minority node; without one the outcome is
/// a `Tie`, never a "winner" — the result depends on nothing but the inputs
/// (`BTreeMap` + node order, no `RandomState`).
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

/// The upstream plane of one node: `FRONTIER_CHANNEL` registered on the
/// upstream simulated network, a `commonware_resolver::p2p::Engine` over it
/// with the production `plane_upstream::new_bridge` as producer + consumer
/// (the same shape as `node/src/dpos.rs::build_beacon_plane`, same cadence
/// values), and the `PlaneUpstreamHandle` that issues fetches on it.
/// `marshal_slot` is the late-bound marshal the serve side reads; the caller
/// fills it once the `OuterEngine` is built. `counters` sees both ends, and
/// `blocker` sees the one thing they cannot: whether the resolver excluded the
/// peer that answered (`plane_upstream::deliver`'s `bool` — see [`BlockerSpy`]).
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
    // The lying-upstream roles wrap the SERVE side, so the victim's own consumer
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
    // EVERY node's marshal slot, not just this one's: a cert-inlet fed by
    // `CertInletSource::PeerArchive` reads its DONOR's archive, and the donor may
    // not be built yet when this node is. Own slot = `marshal_slots[i]`, filled
    // below the moment this node's engine exists.
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

    // The vote Muxer's backup channel is DRAINED and dropped, exactly as the
    // plane's four observer channels are (`node/src/dpos.rs`: since 4.2 В all
    // five are observers). Draining rather than leaving it unread keeps the
    // Muxer from blocking on a full backup mailbox when a frame arrives for an
    // epoch this node has no engine for.
    ctx_i
        .with_label("vote_backup")
        .spawn(move |_| async move { while vote_backup_rx.recv().await.is_some() {} });

    // Per-node singletons.
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
    // The SAME instance answers the slasher, the beacon plane's committee reads
    // and the `EpochTransition` below — one contract per node, as in production.
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

    // The production epoch state machine — ONE per node, as in production. `bridge_tx`
    // is built FIRST so the transition is wired at construction, and the forwarder that
    // turns `(u64, snapshot)` into the `OuterEngine`'s `(Epoch, snapshot)` is spawned
    // after the engine exists — the shape of the node crate's plane
    // (`node/src/dpos.rs`, `mpsc::channel(64)` + `EpochTransition::new` with
    // `Some(bridge_tx)`) and of the `epoch_bridge` drain at
    // `consensus/src/dpos.rs:2648-2660`.
    let (bridge_tx, mut bridge_rx) = mpsc::channel::<(u64, ValidatorSetSnapshot)>(64);
    // `T` = `EpochTransition::last_tracked_epoch`, mirrored off the boundary
    // bridge exactly as `consensus/src/dpos.rs` mirrors it: the transition
    // advances `last_tracked_epoch` only on a successful `boundary_tx.try_send`
    // (`epoch_transition.rs:768-791`), cold start included, so the epochs this
    // node's forwarder drains ARE the epochs that advanced it. `u64::MAX` is the
    // "nothing tracked yet" sentinel.
    let tracked_epoch_cell = Arc::new(AtomicU64::new(u64::MAX));
    let observer = EtObserver::default();
    // THE peer-set window the BEACON channel's pre-decode gate on this node
    // reads, as `node/src/dpos.rs` builds one from the Oracle: the set this
    // node's own transition registers (below, from its cold start on) plus the
    // tombstone predicate over the node's ONE `TombstoneSet` — the same object
    // the `OuterBuilder` takes (the refuse-to-bind gate), filled the way
    // production fills it: `TombstoneSet::observe` over the committee snapshot
    // read at every finalized height (the boundary feed below, mirroring the
    // tombstone watch on `node/src/dpos.rs`'s beacon plane poller), so a node
    // `StandConfig::tombstoned` names is `Dropped` at this gate from the first
    // executed read at or above its height — as in production.
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

    // Cold start, at the anchor this node's execution layer persisted: genesis
    // on a fresh chain, the finalized `(height, hash)` on a replay — production
    // resolves exactly that pair (post-jump) and cold-starts the one transition with
    // it, ONCE, in `DposLayer::launch` (`consensus/src/dpos.rs:2100-2105`). The EPOCH
    // is not supplied: `cold_start` freezes the geometry from the state at `hash` and
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
    // `RethAnchor`: the HEIGHT is this node's ordering-finalized tip (which is
    // what `FakeChain::advance_finalized` moves, `executor.rs` calling it with
    // `order.height`), and the HASH is that chain's tier-F hash there.
    //
    // Production reads at `executed_state_hash(ordering_finalized)`, and so does
    // this: [`FakeChain::executed_state_hash`] IS that probe over the fake chain
    // — `Ok(None)` strictly above the executed head (the park), `Ok(Some)` at a
    // materialized height, `Err` for a materialized height with no hash. Taking
    // the probe rather than a bare `hash_at` costs nothing in VALUE (the anchor
    // reads at the tier-F tip, where tier-F and tier-S agree) and keeps the
    // `Err` arm — the input of the anchor-fault branch — reachable from the
    // stand at all, instead of being a shape only the unit tests can produce.
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

    // ONE committee module per stand node, exactly as production builds one per
    // process: the beacon's `CommitteeReads` is a facade over it, the slasher
    // resolves evidence through it, and the executor wakes it on every
    // `advance_finalized`. The `max(EL-finalized, live)` cursor the stand used
    // to keep beside it is gone with the production one it modelled.
    // The module's ONE scheme producer, wired as production wires it: the beacon
    // slot is filled the moment this node's beacon exists (below), and until then
    // the store answers "no scheme yet" and retries — the same build order the
    // node has, because the beacon is constructed from this store's facade.
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

    // The upstream plane (research §2 #3/#8): the production frontier resolver
    // + `PlaneUpstreamHandle` over `FRONTIER_CHANNEL`, counted at both ends.
    // The marshal slot is filled once the `OuterEngine` is built (as
    // `node/src/dpos.rs::run_dpos_stack` does after the layer launch).
    let upstream_counters = UpstreamCounters::default();
    // This node's ONE blocker spy, wired into BOTH of its blocker slots below.
    // It replaces the `NoopBlocker` both took: a no-op blocker makes "no honest
    // peer lost a channel" true for any code at all, and an assertion on it is
    // vacuous (`BlockerSpy`).
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
    // The stand no longer models reth's EL-`finalized` tag at all, and that is a
    // consequence rather than a simplification: every committee read in the
    // process now anchors on the ORDERING-finalized cursor (`StandAnchor` below,
    // `chain.tip()`), and the one read that does not — the jump's — takes its
    // hash as an explicit argument. The `el_finalized`/`finalized_hash` pair that
    // stood here had exactly one consumer left after the previous step and none
    // after this one.
    //
    // What is NOT modelled as a result: production's `RethAnchor` floors its
    // height with reth's persisted tag, so a node whose consensus cursor is
    // unseeded still reads at the height it durably finalized. The stand's anchor
    // is the cursor alone. Pinned by `committee::tests` instead (§7).
    // The executor's frozen-tip frontier probe, wired as `dpos.rs::launch`
    // wires it for a plane validator (`get_latest` → height).
    // Every jump call, with the outcome VARIANT it returned — see `JumpCall`.
    let jump_calls: JumpCalls = Arc::new(Mutex::new(Vec::new()));
    // How many times this node's frozen-tip probe actually ASKED the upstream
    // (`FrontierProbeFn` invocations that ran `get_latest`). A healthy node whose
    // tip advances never reaches the probe body (`executor.rs:1927-1930`); an
    // inflated frontier makes every probe productive and drops the cadence to the
    // fast burst, so the count climbs far past once-per-block.
    let probe_calls = Arc::new(AtomicU64::new(0));
    // `(frame, recipient)` pairs a `Role::StrayDealer` put on the beacon channel.
    let stray_sends = Arc::new(AtomicU64::new(0));
    // Every ladder step this node's probe NAMED, as `(T, last(T+1))` — the stand's
    // window into §5.2's "ступень". Surfaced as `Outcome::frontier_steps`, so a
    // test can assert WHAT was asked for and not merely that something moved.
    //
    // NAMED, not PUT, and recorded WITHOUT the node's own marshal tip beside it.
    // Both are deliberate: the executor decides whether to put the step, against
    // `last_tip_height`, and reading that tip here (`get_info(Identifier::Latest)`)
    // is an extra message into the marshal's select loop per named step — measured,
    // it costs `a_zero_overlap_boundary_halts_the_chain_verify_only` its incoming
    // half's DKG artifact, i.e. the observation changes the run. The step-vs-tip
    // comparison is pinned at the unit level instead (`executor::tests`).
    let frontier_steps: FrontierSteps = Arc::new(Mutex::new(Vec::new()));
    let re_jump = {
        let probe: FrontierProbeFn = {
            let up = upstream.clone();
            let probe_calls = probe_calls.clone();
            let committee_probe = committee.clone();
            let steps = frontier_steps.clone();
            // Production's probe verbatim (`consensus/src/dpos.rs`): `Latest`
            // untargeted plus, when `T` and `committee[T+1]` are both known, the
            // ladder step `Finalized{last(T+1)}` and the peers to address it at —
            // which the EXECUTOR hands to the marshal's own resolver.
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
        // The steady-state re-jump is the PRODUCTION `jump_to_target`, called
        // the way the node calls it (`consensus/src/dpos.rs`): the same
        // forward-only need-gate, the same landing check against the attested
        // `block.result`, the same `l1_checkpoint = None` on the validator path.
        // There are no verify STAGES to mirror any more (pass Б2): the target is a
        // pair the executor read out of this node's own marshal archive, so the jump
        // takes no committee source and no verify RNG. The one seam below it is the
        // stand's: `JumpElSync` for `RethElSync` (the EL peer is `ElNetwork`, not
        // devp2p). Nothing about the landing choice is re-stated here — the
        // production function picks it.
        //
        // The threshold goes to BOTH the executor's arming gate and the jump's
        // own need-gate, as production's `re_jump_threshold` does. At the stand
        // default (`u64::MAX`) `Executor::maybe_re_jump` never arms the waiter
        // (`executor.rs:2195-2203`), so the call — and the `anchor + threshold`
        // it would overflow — is unreachable.
        let threshold = cfg.re_jump_threshold.unwrap_or(u64::MAX);
        let call: ReJumpFn = {
            let chain = chain.clone();
            let calls_log = jump_calls.clone();
            let ctx_jump = ctx_i.clone();
            Arc::new(move |from: u64, target: UpstreamFinalized| {
                let el = JumpElSync::new(chain.clone(), ctx_jump.clone(), DPOS_ACTIVATION_BLOCK);
                let calls = rejump_calls.clone();
                let calls_log = calls_log.clone();
                // The pair the EXECUTOR read out of this node's own marshal
                // archive and handed in — recorded before the jump consumes it, so
                // a test can check the landing against the certificate it came
                // from rather than against the chain the landing just wrote.
                let consumed = Some((target.block.height, target.block.result));
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let outcome = crate::cold_start_jump::jump_to_target(
                        from,
                        target,
                        &el,
                        // No L1 checkpoint on the validator path. `ElSync::holds`
                        // IS called: the landing check asks it for the attested
                        // `block.result` (§5.2).
                        None,
                        DPOS_ACTIVATION_BLOCK,
                        threshold,
                    )
                    .await;
                    // The production return value, recorded verbatim — the
                    // VARIANT as well as the landing, because a jump that ran and
                    // was REFUSED is otherwise indistinguishable here from one
                    // that landed (`ReJump::rotate` is `None`, so the executor's
                    // rotation escape is a silent no-op).
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

    // The beacon plane (step 4): `beacon::build` exactly as `node/src/dpos.rs::
    // build_beacon_plane` calls it, over the consensus network's BEACON /
    // BEACON_RESOLVER channels and the same four mux brokers, with the schedule
    // standing in for the staking reads. Built BEFORE the `OuterBuilder`, which
    // takes its randomness (the agreement instances stay the beacon's own).
    //
    // The beacon plane's tip watch, exactly as `node/src/dpos.rs` makes it: the
    // beacon actor's clock is a receiver taken HERE, before the app exists, and
    // the sender goes into the `OuterBuilder` so `FluentApp::report` publishes
    // the marshal's tip on it in the same statement that publishes the epoch
    // manager's. Its own channel, with the actor as the ONE parked receiver —
    // `FluentApp::beacon_tip` says why a second subscription to the app's own
    // channel would make this run process-random.
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
            // `Role::StrayDealer`: a second handle on the SAME channel sender the
            // node's beacon gets (the simulated `Sender` is a clone-able mailbox),
            // driven by a task that deals for the epoch after this node's own once
            // a second, to everyone. The body is a real `Commitment` — the dealing
            // this node WOULD broadcast if it had a seat — so that a receiver
            // whose gate is missing gets a frame its actor can decode and answer,
            // and the answer is then the consumer's `no_seat`, never silence.
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
            // R-002: the BEACON_CHANNEL sender half, wrapped. `None` on every
            // honest node, and then the wrapper is a pure pass-through — the type
            // is uniform across roles so `beacon::build`'s `Se` does not branch.
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
            // module — the stand's stand-in for the node's facade, and the SAME
            // type production uses. `StandCommitteeReads` and the
            // `max(EL-finalized, live)` cursor behind it are gone: the module
            // reads at this node's ordering-finalized tip and at nothing else.
            let committees: Arc<dyn CommitteeReads> = Arc::new(
                crate::committee::CommitteeReadsFacade::new(committee.clone()),
            );
            // The production pre-decode gate on the BEACON channel
            // (`node/src/dpos.rs`, `GatedReceiver::new(.., "beacon", true)`):
            // committee traffic, so a registry-tier / untracked / tombstoned
            // sender never reaches the actor's decode. The ONE sender
            // classification on the channel; the seat a sender holds in a
            // frame's epoch is the consumer's check inside the actor (`no_seat`).
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
    // Everything the optional cert-inlet task needs that is about to be MOVED
    // into the builders below, captured here and only on the nodes that run one:
    // the node's ONE beacon (a second provider would file the inlet's σ into an
    // index nobody derives from and would never see the key this plane's DKG
    // publishes — `node/src/cert_inlet.rs`).
    let inlet_inputs = cfg
        .cert_inlet
        .as_ref()
        .filter(|c| c.nodes.contains(&i))
        .map(|c| (c.source, randomness.clone()));
    // Under `Beacon::Static` no actor holds a receiver, so the app keeps the
    // watch `FluentApp::new` makes — the follower launch's shape.
    let beacon_tip = matches!(cfg.beacon, Beacon::Live).then_some(beacon_tip);
    // The committee module's verifier can build now — every epoch it reads from
    // here on gets its scheme bound to THIS node's beacon oracle.
    crate::committee::fill_beacon_slot(&beacon_slot, &randomness);

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
        // The stand is the model, so it carries the production number
        // (`consensus/src/dpos.rs`): a body cache four deep per primary sender.
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

    // The SECOND producer into this node's singleton marshal (Э5 5.0а): the
    // production `CertInlet` over the production builders, fed by this node's own
    // frontier plane or by a donor's archive. Spawned HERE, after
    // `OuterBuilder::build`, which is what makes ITS OWN marshal handle a fact
    // rather than something to wait for: the engine exists, so
    // `marshal_mailbox()` answers, exactly as the follower launch takes TWO
    // clones of it off the built engine (`consensus/src/dpos.rs`). The one
    // handle that IS waited for is the DONOR's, on the `PeerArchive` source —
    // that node may still be unbuilt, so the task binds its slot late.
    let cert_inlet = inlet_inputs.map(|(source, beacon)| {
        use crate::cert_inlet::{CertInlet, RotateUpstream};
        let obs = CertInletObs::default();
        // The rotation trigger, counted. `CertUpstream::rotate_callback` is the
        // production default and this is that closure with a counter in front of
        // it: the inlet's `consecutive_faults` is private, so the count of
        // rotations over a known number of bad certificates is the only way the
        // stand can speak about the streak at all.
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
        // Every node's marshal slot, for `CertInletSource::PeerArchive`: the
        // donor's may still be empty when this task starts (nodes are built in
        // index order), so the task binds it late — see the feeder.
        let donor_slots: Vec<Arc<OnceLock<MarshalMailbox>>> = marshal_slots.to_vec();
        let (committee_inlet, up, chain_inlet, obs_task) = (
            committee.clone(),
            upstream.clone(),
            chain.clone(),
            obs.clone(),
        );
        ctx_i.with_label("cert_inlet").spawn(move |c| async move {
            // The validator shape of `node/src/cert_inlet.rs`: rotate and
            // the node's own beacon. No `with_epoch_math` (the validator
            // inlet leaves the height↔epoch bind a no-op — its consensus
            // plane re-derives and cross-checks instead), no `with_window`
            // (no `consensus`-RPC serving window here) and no
            // `with_connection_token` (there is no WS actor to bump a
            // connection generation). The two METRIC builders are wired
            // because each is the only witness of a regime: the defer family
            // counts the non-fault committee-lag skip, and the
            // carry-forward counter is the one line that fires only when a
            // verify failed WITH the epoch key resolvable — the direct
            // "the key was held at verify time". The follower launch wires
            // both the same way.
            let mut inlet = CertInlet::new(marshal, committee_inlet, c.clone())
                .with_rotate(rotate)
                .with_randomness(beacon)
                .with_committee_read_deferred_metric(obs_task.defers.clone())
                .with_carry_forward_fail_metric(obs_task.carry_forward_fails.clone());
            // `PeerArchive`'s cursor, and it is the FEEDER's rather than the
            // plane's answer: it climbs by one on every pair the donor actually
            // held, so this source never re-ingests a height. (The two plane
            // sources have no such cursor — they re-ask whatever the plane is
            // willing to answer, which on `Frontier` is the same certificate
            // thousands of times.)
            let mut walk: u64 = 1;
            let mut donor: Option<MarshalMailbox> = None;
            loop {
                // PACING, and it is NOT the same on the three arms. The two
                // PLANE fetches go through `PlaneUpstreamHandle::fetch_one`,
                // which awaits either a delivery or its own
                // `FRONTIER_FETCH_TIMEOUT` — but a `None` costs that timeout
                // only on the TIMEOUT path. On the step-(5) drop path
                // `deliver` destroys the waiter's oneshot sender and
                // `fetch_one` resolves `None` at ONCE, for zero virtual time
                // (`plane_upstream.rs:485-489` against the `rx` arm `:622`;
                // the code says so itself at `:641-647`). So what bounds this
                // loop on those two arms is the simulated network round trip
                // (`StandConfig::latency`, 10 ms), not the timeout — measured
                // on the keyless run: 3332 ingests + 840 plane drops over
                // 159.8 s of virtual time ≈ 26 iterations/s ≈ 38 ms each ≈ two
                // hops plus scheduler turns. It is still no timer and no poll
                // of OURS, and there is nothing to subscribe to instead: "the
                // upstream now holds h" is not an event any seam of this stand
                // emits.
                let fetched = match source {
                    // The next height above this node's OWN tier-F, read off
                    // the `FakeChain` and not off the marshal: asking the
                    // marshal costs a message in its select loop per tick,
                    // which is measured to change a run (Д-81, see
                    // `StandConfig::marshal_tip_series`).
                    CertInletSource::NextAboveTier => {
                        up.get_finalization(Height::new(chain_inlet.tip() + 1))
                            .await
                    }
                    CertInletSource::Frontier => up.get_latest().await,
                    // The donor's archive, read directly. Two differences from
                    // the arms above, and both are the point of this source:
                    // the read never touches `deliver`, so nothing gates the
                    // certificate's epoch against THIS node's committee window;
                    // and the read is LOCAL to the donor, so there is no
                    // network round trip to borrow pacing from. A miss (the
                    // donor has not finalized `walk` yet) therefore has to be
                    // paced by us — `POLL`, the driver's own sampling cadence,
                    // is that pace. Unpaced it would not be a "spin" in the
                    // harmless sense: the deterministic runner advances 1 ms
                    // per iteration and skips idle time only when NOTHING is
                    // ready, so a task ready in every iteration both pins the
                    // clock to 1 ms/iteration and floods the donor's marshal
                    // select loop (Д-81's hazard, from the other side).
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

    // Cold start: the epoch the transition entered is READ through the committee
    // module, which is also what registers its verify-only scheme so the marshal
    // can check certificates before the first boundary. Production does exactly
    // this (`consensus/src/dpos.rs`, the cold-start committee read), and it is the
    // whole of what `OuterEngine::cold_start_register` used to do from a second
    // map with `oracle: None`.
    //
    // TWO assertions, at the two points where each is the strongest one that is
    // true, because the module's anchor here is this node's ordering-finalized
    // cursor and at BUILD time that is still 0 — the executor has not derived
    // anything, so a node resuming mid-chain legitimately cannot read its own
    // cold-start epoch yet, exactly as production cannot before its plane has
    // finished starting.
    //
    // HERE: never a PERMANENT refusal. An impossible committee at the cold-start
    // epoch is a chain fact, and production now bails on it
    // (`consensus/src/dpos.rs`, both the validator and the follower cold start),
    // so the stand dies on it too.
    //
    // BELOW, inside the boundary feeder: the read must actually SUCCEED with a
    // non-empty committee the moment the anchor is no longer 0 — the assertion
    // the pre-module stand made against the staking fake directly
    // (`HEAD:testbed/stand.rs`: `expect("the cold-start committee reads at the
    // anchor")` + `assert!(!snap.validators.is_empty())`). It moved rather than
    // weakened: the committee is read through the FACADE, at the anchor, at the
    // first height where the anchor can answer.
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

    // Bridge forwarder: the transition's `(u64, snapshot)` becomes the engine's
    // boundary EPOCH — `consensus/src/dpos.rs`, where the snapshot is dropped for
    // the same reason: the manager re-reads the committee from the module. The
    // snapshot is still RECORDED here, because what the transition read and where
    // is exactly what the stand's epoch-machinery assertions observe.
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

    // The boundary feeder: every ORDERING-finalized block reaches `on_finalized`,
    // which is the ONLY boundary driver production has — `boundary_hook` →
    // `enter_boundary(block.height)` → `on_finalized(number)`
    // (`consensus/src/dpos.rs:2345-2348`, `:2209`, `:2256`). NOT the executor side:
    // the EL-finalized cursor (`FakeChain::advance_finalized`) is what the beacon
    // plane's poller reads, and since the merge that poller takes only the epoch
    // GEOMETRY and the FIRST peer-set registration off it (`node/src/dpos.rs`,
    // `freeze_geometry` + `track_peers`) — it drives no
    // boundary, because a coalesced watch skips them. Feeding this instance that
    // cursor would also read K blocks too low: `read_height_for` subtracts K from
    // its input, which is an ordering height by contract.
    let trace = Arc::new(Mutex::new(Vec::<TraceEntry>::new()));
    {
        let (trace, et_feed, steps, geometry) = (
            trace.clone(),
            et.clone(),
            observer.steps.clone(),
            observer.geometry.clone(),
        );
        // The cold-start committee assertion, armed for the first height at which
        // the anchor can answer it: `chain.tip()` IS `StandAnchor::height()`, so
        // `tip >= cold_number` is exactly "the anchor has reached the height the
        // cold start was taken at". At that anchor every gate of the module is
        // satisfied by construction — `commit_height(initial_epoch) <=
        // cold_number <= tip`, the window floor `epoch(tip) - 8` is at or below
        // `initial_epoch` on the FIRST crossing, and the tier-F tip's state is
        // executed — so anything other than a non-empty record is a real defect
        // and not a race.
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
                    // The tombstone watch, on THIS finalized-height feed and no
                    // second timer — production's shape (`node/src/dpos.rs`, the
                    // beacon plane poller: `epoch_at(fin)` + `executed_state_hash
                    // (fin)` → `epoch_committee_snapshot` → `TombstoneSet::observe`).
                    // STATE-GATED as there: a height whose state is not executed
                    // yet (`Ok(None)`) is skipped, not read at a header. The
                    // snapshot goes to the reader directly, not through the
                    // committee module, exactly as the node's `tombstone_reader`
                    // does. What production does with the delta — sever the
                    // peer's transport — the stand does not model: its blocker is
                    // a counting spy wired into two named slots, and the
                    // simulated network severs nothing on it; the delta is
                    // RECORDED (`Outcome::tombstones_observed`) instead.
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
                    // Re-poke loop: a parked boundary replays only on the next
                    // `on_finalized`, and during catch-up the parked boundary IS
                    // the last deliverable block (`consensus/src/dpos.rs:2168-2203`).
                    // One loop at a time is enough here — the single-slot park
                    // means there is only ever one boundary to drive.
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
