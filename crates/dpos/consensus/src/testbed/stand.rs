//! `Stand`: N `OuterEngine`s on one deterministic runner + one simulated network.

#[cfg(feature = "dpos-devnet-byzantine")]
use super::fakes::{ByzFacts, ByzReport};
use super::{
    capture::{self, Captured, Sink},
    fakes::{
        genesis_sealed, BodyTap, CountingHandler, CountingUpstream, ElEvent, ElNetwork, FakeBeacon,
        FakeChain, FakeDeriver, FakeStaking, JumpCall, JumpCalls, JumpCommitteeReads,
        JumpCommittees, JumpElSync, Members, NoSink, NoTxs, StakingReads, TapReceiver,
        TeeingUpstream, UpstreamCounters, UpstreamStats, DPOS_ACTIVATION_BLOCK,
    },
};
/// A read of one held epoch-key artifact's wire bytes, for the stand's serving
/// side. It used to come out of `beacon::build`; the beacon answers it through
/// `Beacon::artifact_bytes` now, and the stand closes over that.
type ArtifactSource = Arc<dyn Fn(u64) -> Option<Vec<u8>> + Send + Sync>;

#[cfg(feature = "dpos-devnet-byzantine")]
use crate::beacon::testing::CommitteeFor;
use crate::{
    application::ExecutedChain as _,
    beacon::{
        self,
        testing::{absent, StaticRandomness},
        CommitteeReads, Seed, ValidatorInputs,
    },
    cert_follow::CertUpstream as _,
    cold_start_jump::JumpOutcome,
    dpos::VoteBackupItem,
    epoch_manager::EpochEngineMetrics,
    executor::{ExecutorMetrics, FrontierProbeFn, ReJump, ReJumpFn},
    extra_data::decode_production_record,
    order_block::{anchor_order_block, OrderBlock},
    outer::{MarshalMailbox, OuterBuilder},
    plane_upstream::{new_bridge, PlaneUpstreamHandle},
    slasher::TombstoneSet,
    sync_metrics::{PlaneClock, SafetyHalt, SyncMetrics},
    timeouts::ConsensusTimeouts,
};
use alloy_primitives::{Address, B256};
use commonware_codec::DecodeExt as _;
use commonware_consensus::types::{Epoch, Height};
use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
use commonware_math::algebra::Random as _;
use commonware_p2p::{
    simulated::{Config as SimConfig, Link, Network},
    utils::mux::{Builder as _, Muxer},
    Manager as _,
};
use commonware_runtime::{
    deterministic, Clock as _, Metrics as _, Quota, Runner as _, Spawner as _,
};
use commonware_utils::{ordered::Set, NZUsize};
use fluentbase_bls::{fluent_namespace, keys::ValidatorBlsKeypair, BlsPubkey, PeerPubkey};
use fluentbase_p2p::{
    constants::{
        BEACON_CHANNEL, BEACON_RESOLVER_CHANNEL, BROADCAST_CHANNEL, CERT_CHANNEL,
        DKG_SUBCHANNEL_BASE, FRONTIER_CHANNEL, MARSHAL_CHANNEL, MAX_REGISTRY_PEER_SET,
        RESOLVER_CHANNEL, VOTE_CHANNEL,
    },
    NoopBlocker,
};
use fluentbase_staking_reader::{
    epoch_transition::{PeerSetSink, TransitionOutcome, PENDING_RETRY_BACKOFF},
    reader::ValidatorSetSnapshot,
    EpochTransition,
};
use fluentbase_types::staking_protocol::epoch_at_block;
use rand_08::{rngs::StdRng, SeedableRng as _};
use std::{
    collections::BTreeMap,
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
    /// After the run, ask every node's marshal for the `(finalization, block)`
    /// pair at every height up to the highest tip — the exact two local reads
    /// `handle_produce` answers a peer's `Finalized{h}` from (CW
    /// `marshal/core/actor.rs:808-818`, and `plane_upstream::serve`). Off by
    /// default: the scan is a few hundred mailbox round-trips per node, and it
    /// adds yields to the collection phase.
    pub archive_scan: bool,
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
    /// Every node, tracked once at index 0 — production's `active_registry ∪
    /// committee[E]` for a registry that never changes.
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
            shared_engine_partitions: false,
            peer_set: PeerSet::AllNodes,
            beacon: Beacon::Static,
            share_root: PathBuf::new(),
            resume_from: None,
            re_jump_threshold: None,
            upstream_only_link: None,
            archive_scan: false,
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
    /// gate passes). A victim whose frontier probe / re-jump reads this raises its
    /// `upstream_frontier` past any real height and re-jumps every tip.
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
}

#[derive(Clone, Debug)]
pub(super) struct Partition {
    pub a: Vec<usize>,
    pub b: Vec<usize>,
    /// Cut once every node's executed tip is at or above this height.
    pub after_height: u64,
    pub duration: Duration,
}

pub(super) struct PartitionCfg<'a>(&'a mut Partition);

impl PartitionCfg<'_> {
    pub(super) fn after_height(self, h: u64) -> Self {
        self.0.after_height = h;
        self
    }
    /// Hold the cut for `views` leader timeouts of `ConsensusTimeouts::fluent_1s`
    /// — the time the split halves need to nullify that many views.
    pub(super) fn for_views(self, views: u32) -> Self {
        self.0.duration = ConsensusTimeouts::fluent_1s().leader * views;
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

pub(super) struct Outcome {
    /// Tier-F (finalized-executed) tip per node.
    pub heights: Vec<u64>,
    /// `hashes[i][h - 1]` = `(h, finalized-executed hash)` for `h in 1..=heights[i]`.
    pub hashes: Vec<Vec<(u64, B256)>>,
    /// The first height whose executed hashes disagree across nodes.
    pub diverged: Option<Divergence>,
    /// What each node's upstream plane did (both ends, see `UpstreamCounters`).
    pub upstream: Vec<UpstreamStats>,
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
    /// `tracked[i]` = every `(epoch, peer set)` node `i`'s transition handed its
    /// `PeerSetSink`.
    pub tracked: Vec<Vec<(u64, Vec<PeerPubkey>)>>,
    /// How many times two nodes' transitions tracked DIFFERENT peer sets for the
    /// same epoch. Must be zero.
    pub tracked_mismatches: u64,
    /// Peer-set registrations that actually reached the simulated network.
    pub tracked_forwarded: u64,
    /// `staking_reads[i]` = what node `i` ASKED the fake staking state, split by
    /// whether the epoch was committed at the read height.
    pub staking_reads: Vec<StakingReads>,
    /// `jump_calls[i]` = every steady-state re-jump call node `i` made, in call
    /// order, with the `JumpOutcome` VARIANT the production function returned,
    /// the certificate it consumed and the landing it chose. A refused jump is
    /// invisible anywhere else in this struct — see `fakes::JumpCall`.
    pub jump_calls: Vec<Vec<JumpCall>>,
    /// `jump_committee_reads[i]` = every `(epoch, executed hash)` node `i`'s
    /// steady-state re-jump read `committee[E]` at — recorded inside the stand's
    /// `CommitteeSource` by production's own `verify_jump_authenticated` call, so
    /// "the committee was read at the LANDING hash" is observed, not inferred.
    pub jump_committee_reads: Vec<Vec<(u64, B256)>>,
    /// `el_events[i]` = every EL tier transition node `i` made, IN ORDER — each
    /// `derive_and_execute` insert into the executed tree and each
    /// canonicalization an FCU (or a jump landing) committed. See
    /// [`fakes::ElEvent`](super::fakes::ElEvent): the two tiers only mean
    /// something relative to each other, so "the guard read `h` while `h` was
    /// still tree-only" is an INDEX comparison in this one log and not the
    /// absence of a line in two.
    pub el_events: Vec<Vec<ElEvent>>,
    /// `upstream_frontier_series[i]` = node `i`'s `ReJump::upstream_frontier`
    /// sampled once per driver tick, in order — the deep-gap re-jump trigger's
    /// atomic, `fetch_max`ed by the frozen-tip probe from the served tip height. A
    /// lying upstream inflates it past any real height; the SERIES lets a test see
    /// it is non-decreasing and reaches the inflation, rather than trusting
    /// `fetch_max`. Read only by the R-004 role test.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub upstream_frontier_series: Vec<Vec<u64>>,
    /// `probe_calls[i]` = how many times node `i`'s frozen-tip probe asked the
    /// upstream. Read only by the R-004 role test.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub probe_calls: Vec<u64>,
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
/// assembled `active_registry ∪ committee[E] ∪ committee[E+1]` set here and the
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
}

#[derive(Default)]
struct Tracked {
    /// `epoch -> the set the FIRST node's transition tracked for it`. Seeded
    /// with the epoch-0 set the harness registers before any node exists (the
    /// engines need a peer set to be there), so the transitions' own epoch-0
    /// track is a no-op against it.
    by_epoch: BTreeMap<u64, Vec<PeerPubkey>>,
    /// `per_node[i]` = every `(epoch, set)` node `i`'s transition tracked.
    per_node: Vec<Vec<(u64, Vec<PeerPubkey>)>>,
    /// How many times a node tracked a set for an epoch that DIFFERS from the
    /// one already recorded for it. Forwarding only the first node's set would
    /// otherwise swallow the disagreement in silence: the union is a function of
    /// chain state alone, so a difference means two nodes read different
    /// committees for one epoch.
    mismatches: u64,
    /// How many peer-set registrations actually reached the simulated network —
    /// the only events that can cost an ack (see `Outcome::SIMULATOR_ACK_DROP`).
    forwarded: u64,
}

impl PeerSetSink for TrackSink {
    fn track(
        &mut self,
        epoch: u64,
        peers: Set<PeerPubkey>,
    ) -> impl core::future::Future<Output = ()> + Send {
        let members: Vec<PeerPubkey> = peers.iter().cloned().collect();
        let forward = {
            let mut shared = self.shared.lock().unwrap();
            shared.per_node[self.node].push((epoch, members.clone()));
            let changed = shared
                .by_epoch
                .last_key_value()
                .is_none_or(|(_, last)| *last != members);
            match shared.by_epoch.get(&epoch) {
                Some(recorded) if *recorded != members => {
                    shared.mismatches += 1;
                    false
                }
                Some(_) => false,
                None => {
                    shared.by_epoch.insert(epoch, members);
                    if changed {
                        shared.forwarded += 1;
                    }
                    changed
                }
            }
        };
        let mut manager = self.oracle.manager();
        async move {
            if forward {
                manager.track(epoch, peers).await;
            }
        }
    }
}

struct NodeHandles {
    chain: FakeChain,
    halt: SafetyHalt,
    /// Every committee read the node's steady-state re-jump made, in call order.
    jump_committee_reads: JumpCommitteeReads,
    /// Every steady-state re-jump call the node made, with its outcome variant.
    jump_calls: JumpCalls,
    /// `ReJump::upstream_frontier` — the deep-gap re-jump trigger's atomic (read by
    /// the R-004 role test only).
    #[cfg(feature = "dpos-devnet-byzantine")]
    upstream_frontier: Arc<AtomicU64>,
    /// Frozen-tip probe invocations (R-004 role test only).
    #[cfg(feature = "dpos-devnet-byzantine")]
    probe_calls: Arc<AtomicU64>,
    trace: Arc<Mutex<Vec<TraceEntry>>>,
    upstream: UpstreamCounters,
    /// `Beacon::Live`: the beacon's artifact read.
    artifacts: Option<ArtifactSource>,
    observer: EtObserver,
    staking: FakeStaking,
    /// The node's marshal, for the post-run archive scan.
    marshal: Arc<OnceLock<MarshalMailbox>>,
    bodies: BodyTap,
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

    // The stand's devp2p EL peer, shared by every node — see `ElNetwork`.
    let el_network = ElNetwork::default();
    let genesis_sealed = genesis_sealed();
    let genesis_hash = genesis_sealed.hash();
    let genesis_block = anchor_order_block(&genesis_sealed).expect("anchor");

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
    // Per-node samples of `ReJump::upstream_frontier`, one per driver tick — so
    // "the frontier never lowers" is OBSERVED as a non-decreasing series, not
    // inferred from `fetch_max` being the only writer (F8).
    #[cfg(feature = "dpos-devnet-byzantine")]
    let mut frontier_series: Vec<Vec<u64>> = vec![Vec::new(); n];
    let mut head_gap_series: Vec<Vec<u64>> = vec![Vec::new(); n];
    let elapsed_now =
        |ctx: &deterministic::Context| ctx.current().duration_since(t0).unwrap_or_default();
    loop {
        #[cfg(feature = "dpos-devnet-byzantine")]
        for (i, node) in nodes.iter().enumerate() {
            frontier_series[i].push(node.upstream_frontier.load(Ordering::SeqCst));
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
                    // A partition is physical: it cuts both planes.
                    for (x, y) in cross_pairs(part) {
                        for o in [&oracle, &upstream_oracle] {
                            o.remove_link(pks[x].clone(), pks[y].clone())
                                .await
                                .expect("remove link");
                        }
                    }
                    part_obs[p].heights_at_cut = progress.heights.clone();
                    part_obs[p].cut_at = progress.elapsed;
                    part_state[p] = PartState::Active(progress.elapsed);
                }
                PartState::Active(since) if progress.elapsed >= since + part.duration => {
                    for (x, y) in cross_pairs(part) {
                        for o in [&oracle, &upstream_oracle] {
                            o.add_link(pks[x].clone(), pks[y].clone(), link.clone())
                                .await
                                .expect("add link");
                        }
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
            // own `active_registry ∪ committee[E] ∪ committee[E+1]`, and it is
            // the set the authenticated transport would keep connections for.
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
    let jump_committee_reads: Vec<Vec<(u64, B256)>> = nodes
        .iter()
        .map(|node| node.jump_committee_reads.lock().unwrap().clone())
        .collect();
    let jump_calls: Vec<Vec<JumpCall>> = nodes
        .iter()
        .map(|node| node.jump_calls.lock().unwrap().clone())
        .collect();
    let el_events: Vec<Vec<ElEvent>> = nodes.iter().map(|node| node.chain.el_events()).collect();
    #[cfg(feature = "dpos-devnet-byzantine")]
    let probe_calls: Vec<u64> = nodes
        .iter()
        .map(|node| node.probe_calls.load(Ordering::SeqCst))
        .collect();
    #[cfg(feature = "dpos-devnet-byzantine")]
    let byz: Vec<ByzFacts> = nodes.iter().map(|node| node.byz.snapshot()).collect();
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
    let metrics = ctx.encode();
    Outcome {
        heights,
        hashes,
        diverged,
        upstream,
        halted,
        traces,
        seeds,
        artifacts,
        et_steps,
        et_boundaries,
        geometry,
        tracked: tracked_sets,
        tracked_mismatches,
        tracked_forwarded,
        staking_reads,
        jump_calls,
        jump_committee_reads,
        el_events,
        #[cfg(feature = "dpos-devnet-byzantine")]
        upstream_frontier_series: frontier_series,
        #[cfg(feature = "dpos-devnet-byzantine")]
        probe_calls,
        #[cfg(feature = "dpos-devnet-byzantine")]
        byz,
        head_gap_series,
        head_gap_on_fcu,
        head_gap_on_landing,
        archive,
        bodies,
        metrics,
        logs,
        simulator_ack_drops,
        log_capture_live,
        partitions: part_obs,
        timed_out,
        virtual_elapsed: elapsed_now(&ctx),
        real_elapsed: Duration::ZERO,
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
/// fills it once the `OuterEngine` is built. `counters` sees both ends.
#[allow(clippy::too_many_arguments)]
pub(super) async fn frontier_plane(
    ctx: &deterministic::Context,
    oracle: &Oracle,
    me: PeerPubkey,
    marshal_slot: Arc<OnceLock<MarshalMailbox>>,
    counters: UpstreamCounters,
    #[cfg(feature = "dpos-devnet-byzantine")] byz: ByzReport,
    #[cfg(feature = "dpos-devnet-byzantine")] mode: super::byzantine_roles::ForgeMode,
    #[cfg(feature = "dpos-devnet-byzantine")] lying: Option<super::byzantine_roles::LyingCfg>,
) -> PlaneUpstreamHandle<deterministic::Context> {
    let (sender, receiver) = oracle
        .control(me.clone())
        .register(FRONTIER_CHANNEL, QUOTA)
        .await
        .expect("frontier channel");
    let (handler, waiters) = new_bridge(marshal_slot);
    let handler = CountingHandler::new(handler, counters);
    // The lying-upstream roles wrap the SERVE side, so the victim's own consumer
    // path is production's verbatim.
    #[cfg(feature = "dpos-devnet-byzantine")]
    let handler = super::byzantine_roles::ForgedSeedProducer::new(handler, byz, mode, lying);
    let (engine, mailbox) = commonware_resolver::p2p::Engine::new(
        ctx.with_label("frontier_resolver"),
        commonware_resolver::p2p::Config {
            peer_provider: oracle.manager(),
            blocker: NoopBlocker,
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

    // Vote-backup forwarder: a frame the vote muxer cannot route is a vote for
    // an epoch this node has no engine for — the epoch manager's catch-up hint.
    // Mirrors `node/src/dpos.rs::forward_vote_backup` (epoch space only).
    let (vb_tx, vb_rx) = mpsc::channel::<VoteBackupItem>(64);
    ctx_i.with_label("vote_backup").spawn(move |_| async move {
        while let Some((subchannel, msg)) = vote_backup_rx.recv().await {
            if subchannel < DKG_SUBCHANNEL_BASE {
                let _ = vb_tx.try_send((Epoch::new(subchannel), msg));
            }
        }
    });

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
    );
    let (hook_tx, mut hook_rx) = mpsc::unbounded_channel::<OrderBlock>();

    // The production epoch state machine — ONE per node, as in production. `bridge_tx`
    // is built FIRST so the transition is wired at construction, and the forwarder that
    // turns `(u64, snapshot)` into the `OuterEngine`'s `(Epoch, snapshot)` is spawned
    // after the engine exists — the shape of the node crate's plane
    // (`node/src/dpos.rs`, `mpsc::channel(64)` + `EpochTransition::new` with
    // `Some(bridge_tx)`) and of the `epoch_bridge` drain at
    // `consensus/src/dpos.rs:2648-2660`.
    let (bridge_tx, mut bridge_rx) = mpsc::channel::<(u64, ValidatorSetSnapshot)>(64);
    let observer = EtObserver::default();
    let et = Arc::new(tokio::sync::Mutex::new(EpochTransition::new(
        staking.clone(),
        TrackSink {
            node: i,
            oracle: oracle.clone(),
            shared: tracked,
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

    // The upstream plane (research §2 #3/#8): the production frontier resolver
    // + `PlaneUpstreamHandle` over `FRONTIER_CHANNEL`, counted at both ends.
    // The marshal slot is filled once the `OuterEngine` is built (as
    // `node/src/dpos.rs::run_dpos_stack` does after the layer launch).
    let upstream_counters = UpstreamCounters::default();
    let marshal_slot: Arc<OnceLock<MarshalMailbox>> = Arc::new(OnceLock::new());
    let upstream = CountingUpstream::new(
        frontier_plane(
            &ctx_i,
            upstream_oracle,
            me.clone(),
            marshal_slot.clone(),
            upstream_counters.clone(),
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
    // The LIVE upstream frontier — production's `LiveFrontierTee::live_height`
    // (`cert_inlet.rs:277`), advanced to the height of an upstream
    // finalization this node has seen. One difference, stated because it is the
    // stand's and not production's: production advances it ONLY past the cert
    // inlet's BLS-verify gate (`cert_inlet.rs:737-739`) so a lying upstream
    // cannot steer the committee read, while the stand has no inlet and tees it
    // where the executor's frontier probe already asks — before any verify.
    // Э3.3 added lying-upstream roles WITHOUT moving this: `live_height` is
    // still teed before any verify, so a role's forged frontier reaches it. A
    // stand-side `CertInlet` is what would put it behind the gate (PLAN 4.0(в)).
    let live_height = Arc::new(AtomicU64::new(0));
    // Every `committee[E]` read the jump made, with the executed hash it read AT
    // — see `JumpCommitteeReads`. Surfaced as `Outcome::jump_committee_reads`.
    let jump_committee_reads: JumpCommitteeReads = Arc::new(Mutex::new(Vec::new()));
    // Every jump call, with the outcome VARIANT it returned — see `JumpCall`.
    let jump_calls: JumpCalls = Arc::new(Mutex::new(Vec::new()));
    // Production's `ReJump::upstream_frontier` (`executor.rs:509`): the inlet /
    // frozen-tip probe `fetch_max`es the served tip height into it, and the
    // re-jump trigger measures the gap against it. Held here so the stand can
    // sample it over the run (a lying upstream inflates it past any real height).
    let upstream_frontier = Arc::new(AtomicU64::new(0));
    // How many times this node's frozen-tip probe actually ASKED the upstream
    // (`FrontierProbeFn` invocations that ran `get_latest`). A healthy node whose
    // tip advances never reaches the probe body (`executor.rs:1927-1930`); an
    // inflated frontier makes every probe productive and drops the cadence to the
    // fast burst, so the count climbs far past once-per-block.
    let probe_calls = Arc::new(AtomicU64::new(0));
    let re_jump = {
        let probe: FrontierProbeFn = {
            let up = upstream.clone();
            let live = live_height.clone();
            let probe_calls = probe_calls.clone();
            Arc::new(move || {
                let up = up.clone();
                let live = live.clone();
                let probe_calls = probe_calls.clone();
                Box::pin(async move {
                    probe_calls.fetch_add(1, Ordering::Relaxed);
                    let uf = up.get_latest().await?;
                    live.fetch_max(uf.block.height, Ordering::Relaxed);
                    Some(Height::new(uf.block.height))
                })
            })
        };
        let rejump_calls = upstream_counters.rejump_calls.clone();
        // The steady-state re-jump is the PRODUCTION
        // `cold_start_jump_with_threshold`, called the way the node calls it
        // (`consensus/src/dpos.rs:2492-2536`): the same forward-only need-gate,
        // the same PRE-sync `verify_jump_structural`, the same POST-sync
        // `verify_jump_authenticated` (a 2f+1 BLS multisig against `committee[E]`
        // read at the LANDING's own executed state), the same `l1_checkpoint =
        // None` on the validator path. Only the two seams below it are the
        // stand's: `JumpElSync` for `RethElSync` (the EL peer is `ElNetwork`,
        // not devp2p) and `JumpCommittees` for `RethCommitteeSource` (the
        // committee comes out of `FakeStaking`'s contract state machine, read by
        // executed hash). Nothing about the landing choice is re-stated here —
        // the production function picks it.
        //
        // The threshold goes to BOTH the executor's arming gate and the jump's
        // own need-gate, as production's `re_jump_threshold` does. At the stand
        // default (`u64::MAX`) `Executor::maybe_re_jump` never arms the waiter
        // (`executor.rs:2195-2203`), so the call — and the `anchor + threshold`
        // it would overflow — is unreachable.
        let threshold = cfg.re_jump_threshold.unwrap_or(u64::MAX);
        let call: ReJumpFn = {
            let up = upstream.clone();
            let chain = chain.clone();
            let staking = staking.clone();
            let jump_reads = jump_committee_reads.clone();
            let calls_log = jump_calls.clone();
            let ctx_jump = ctx_i.clone();
            Arc::new(move |from: u64| {
                // Tee'd fresh per call: the certificate the jump consumes is the
                // one this wrapper sees, and nothing else reads through it.
                let up = TeeingUpstream::new(up.clone());
                let committees = JumpCommittees::new(
                    staking.clone(),
                    fluent_namespace(CHAIN_ID),
                    jump_reads.clone(),
                );
                let el = JumpElSync::new(chain.clone(), ctx_jump.clone(), DPOS_ACTIVATION_BLOCK);
                let calls = rejump_calls.clone();
                let calls_log = calls_log.clone();
                // `verify_jump_authenticated` wants a `&mut (Clock +
                // CryptoRngCore)`; a fresh clone per call, as production does.
                let mut jump_ctx = ctx_jump.clone();
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let outcome = crate::cold_start_jump::cold_start_jump_with_threshold(
                        from,
                        &up,
                        &committees,
                        &el,
                        // No L1 checkpoint on the validator path — the trustless
                        // POST-sync committee read at the landing IS the anchor,
                        // and `ElSync::holds` is therefore never called.
                        None,
                        DPOS_ACTIVATION_BLOCK,
                        threshold,
                        &mut jump_ctx,
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
                            JumpOutcome::BadTarget(_) => "BadTarget",
                            JumpOutcome::InvalidTarget(_) => "InvalidTarget",
                            JumpOutcome::StalledWithPeers(_) => "StalledWithPeers",
                            JumpOutcome::AuthFailed(_) => "AuthFailed",
                            JumpOutcome::L1Fork(_) => "L1Fork",
                        },
                        consumed: up.consumed(),
                        landed: match &outcome {
                            JumpOutcome::Landed { landing, hash, .. } => Some((*landing, *hash)),
                            _ => None,
                        },
                        // The error text of a refusal, so a test can tell the
                        // committee-BLS arm from the unreadable-committee arm.
                        outcome_detail: match &outcome {
                            JumpOutcome::Stalled(e)
                            | JumpOutcome::BadTarget(e)
                            | JumpOutcome::InvalidTarget(e)
                            | JumpOutcome::StalledWithPeers(e)
                            | JumpOutcome::AuthFailed(e)
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
            upstream_frontier: upstream_frontier.clone(),
            threshold,
            rotate: None,
            probe: Some(probe),
        }
    };

    // The beacon plane (step 4): `beacon::build` exactly as `node/src/dpos.rs::
    // build_beacon_plane` calls it, over the consensus network's BEACON /
    // BEACON_RESOLVER channels and the same four mux brokers, with the schedule
    // standing in for the staking reads. Built BEFORE the `OuterBuilder`, which
    // takes its randomness and adopts its agreement instances.
    let (dkg_height_tx, dkg_height_rx) = mpsc::channel::<u64>(256);
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

    let (randomness, artifacts, agreement_intake) = match (cfg.beacon, role) {
        (Beacon::Static, _) => (
            StaticRandomness::build(CHAIN_ID, staking.all_validators_snapshot()),
            None,
            None,
        ),
        (Beacon::Live, Role::AbsentBeacon) => (absent(&ctx_i), None, None),
        (Beacon::Live, _) => {
            let (bcs, bcr) = register(BEACON_CHANNEL).await;
            let (brs, brr) = register(BEACON_RESOLVER_CHANNEL).await;
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
                            namespace: fluentbase_bls::beacon::seed_namespace(&fluent_namespace(
                                CHAIN_ID,
                            )),
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
            let (beacon, beacon_tasks) = beacon::build(
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
                    heights: dkg_height_rx,
                    plane_clock: plane_clock.clone(),
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
            (
                randomness,
                Some(artifacts),
                Some(beacon_tasks.agreement_intake),
            )
        }
    };
    let dkg_height_tx = matches!(cfg.beacon, Beacon::Live).then_some(dkg_height_tx);
    // The committee module's verifier can build now — every epoch it reads from
    // here on gets its scheme bound to THIS node's beacon oracle.
    crate::committee::fill_beacon_slot(&beacon_slot, &randomness);

    let outer = OuterBuilder {
        me: me.clone(),
        blocker: NoopBlocker,
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
        tombstones: TombstoneSet::default(),
        plane_clock,
        dkg_height_tx,
        timeouts: ConsensusTimeouts::fluent_1s(),
        mailbox_size: 256,
        deque_size: 64,
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
        agreement_intake,
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
        ctx_i.with_label("epoch_bridge").spawn(move |_| async move {
            while let Some((epoch, snap)) = bridge_rx.recv().await {
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
                    let parked = {
                        let mut guard = et_feed.lock().await;
                        let outcome = guard.on_finalized(block.height).await;
                        *geometry.lock().unwrap() = guard.frozen_geometry();
                        steps.lock().unwrap().push(EtStep {
                            number: block.height,
                            outcome: outcome.map_err(|e| format!("{e:?}")),
                        });
                        guard.has_pending_boundary()
                    };
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
        vb_rx,
        Some(upstream),
    );

    NodeHandles {
        chain,
        halt,
        jump_committee_reads,
        jump_calls,
        #[cfg(feature = "dpos-devnet-byzantine")]
        upstream_frontier,
        #[cfg(feature = "dpos-devnet-byzantine")]
        probe_calls,
        trace,
        upstream: upstream_counters,
        artifacts,
        observer,
        staking,
        marshal: marshal_slot,
        bodies,
        #[cfg(feature = "dpos-devnet-byzantine")]
        byz,
    }
}
