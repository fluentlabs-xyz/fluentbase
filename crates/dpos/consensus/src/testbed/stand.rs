//! `Stand`: N `OuterEngine`s on one deterministic runner + one simulated network.

use super::{
    capture::{self, Captured, Sink},
    fakes::{
        genesis_sealed, CountingHandler, CountingUpstream, FakeBeacon, FakeChain, FakeDeriver,
        NoSink, NoTxs, Schedule, SnapshotReader, UpstreamCounters, UpstreamStats,
    },
};
use crate::{
    beacon::{
        self,
        actor::{CommitteeFor, CommitteePairFor},
        carry::DkgQualProbe,
        seed::Seed,
        surface::StaticRandomness,
        ArtifactSource, BeaconConfig, CommitteeSource,
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
    scheme::epoch_committee_from_snapshot,
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
use fluentbase_bls::{
    fluent_namespace, keys::ValidatorBlsKeypair, scheme::build_verifier, BlsPubkey, PeerPubkey,
};
use fluentbase_p2p::{
    constants::{
        BEACON_CHANNEL, BEACON_RESOLVER_CHANNEL, BROADCAST_CHANNEL, CERT_CHANNEL,
        DKG_SUBCHANNEL_BASE, FRONTIER_CHANNEL, MARSHAL_CHANNEL, RESOLVER_CHANNEL, VOTE_CHANNEL,
    },
    NoopBlocker,
};
use fluentbase_staking_reader::reader::{
    is_epoch_boundary, ConsensusKeys, ValidatorSetSnapshot, ValidatorWithKeys,
};
use fluentbase_types::staking_protocol::epoch_at_block;
use rand_08::{rngs::StdRng, SeedableRng as _};
use std::{
    collections::BTreeMap,
    num::{NonZeroU32, NonZeroU64},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
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
    /// The epoch the boundary relay's cold-start arm enters — production's
    /// `EpochTransition::cold_start` at the node's finalized block. `0` for a
    /// fresh chain; a replay sets it to the epoch of the height the nodes
    /// stopped at (the relay then waits for the NEXT boundary block, and
    /// `soft_enter_committees` covers the earlier epochs for verification).
    pub cold_start_epoch: u64,
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
            cold_start_epoch: 0,
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
    /// `Beacon::Live` only: this node brings up NO beacon plane (no share, no
    /// dealer log, no agreement seat) and runs `beacon::absent` instead — a
    /// verifier that never signs. The dealer that is missing from the ceremony.
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
    /// The runner's Prometheus text at the end of the run (every node's
    /// families under its `node{i}_` label).
    pub metrics: String,
    pub logs: Vec<Captured>,
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

    pub(super) fn errors(&self) -> Vec<&Captured> {
        self.logs
            .iter()
            .filter(|l| l.level == tracing::Level::ERROR)
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

fn snapshot(
    epoch: u64,
    epoch_len: u64,
    members: &[usize],
    peers: &[Ed25519PrivateKey],
    bls: &[ValidatorBlsKeypair],
) -> ValidatorSetSnapshot {
    let validators = members
        .iter()
        .map(|&i| ValidatorWithKeys {
            address: Address::with_last_byte(i as u8 + 1),
            keys: ConsensusKeys {
                bls_pubkey: BlsPubkey::decode(bls[i].public_bytes().as_slice())
                    .expect("bls pubkey"),
                peer_pubkey: peers[i].public_key(),
                activation_epoch: 0,
            },
            tombstoned: false,
        })
        .collect();
    ValidatorSetSnapshot {
        block_hash: B256::ZERO,
        block_number: epoch * epoch_len,
        epoch,
        validators,
        // `None` is "the weight ring has wrapped" and the manager refuses the
        // snapshot; equal unit weights are the plain lottery.
        weights: Some(vec![1; members.len()]),
    }
}

struct NodeHandles {
    chain: FakeChain,
    halt: SafetyHalt,
    trace: Arc<Mutex<Vec<TraceEntry>>>,
    upstream: UpstreamCounters,
    /// `Beacon::Live`: the plane's artifact read.
    artifacts: Option<ArtifactSource>,
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

    let schedule: Schedule = {
        let (peers, bls, committees, epoch_len) = (
            peers.clone(),
            bls.clone(),
            cfg.committees.clone(),
            cfg.epoch_len,
        );
        Arc::new(move |epoch: u64| {
            let members = match &committees {
                Committees::All => Some((0..n).collect::<Vec<_>>()),
                Committees::Schedule(f) => f(epoch, n),
            }?;
            Some(snapshot(epoch, epoch_len, &members, &peers, &bls))
        })
    };
    // The one snapshot `StaticRandomness` deals its shares from: every node, so
    // every node derives the SAME sharing whatever the epoch's committee is.
    let full_snapshot = snapshot(0, cfg.epoch_len, &(0..n).collect::<Vec<_>>(), &peers, &bls);

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
            schedule.clone(),
            full_snapshot.clone(),
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
    let elapsed_now =
        |ctx: &deterministic::Context| ctx.current().duration_since(t0).unwrap_or_default();
    loop {
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
            let max_height = progress.heights.iter().copied().max().unwrap_or(0);
            if let Some(epoch) = epoch_at_block(max_height, 0, cfg.epoch_len) {
                if epoch > tracked_epoch {
                    if let Some(snap) = schedule(epoch) {
                        let members: Vec<PeerPubkey> = snap
                            .validators
                            .iter()
                            .map(|v| v.keys.peer_pubkey.clone())
                            .collect();
                        oracle
                            .manager()
                            .track(epoch, Set::from_iter_dedup(members.iter().cloned()))
                            .await;
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
        metrics,
        logs,
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
pub(super) async fn frontier_plane(
    ctx: &deterministic::Context,
    oracle: &Oracle,
    me: PeerPubkey,
    marshal_slot: Arc<OnceLock<MarshalMailbox>>,
    counters: UpstreamCounters,
) -> PlaneUpstreamHandle<deterministic::Context> {
    let (sender, receiver) = oracle
        .control(me.clone())
        .register(FRONTIER_CHANNEL, QUOTA)
        .await
        .expect("frontier channel");
    let (handler, waiters) = new_bridge(marshal_slot);
    let handler = CountingHandler::new(handler, counters);
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
    schedule: Schedule,
    full_snapshot: ValidatorSetSnapshot,
    genesis_hash: B256,
    genesis_block: OrderBlock,
) -> NodeHandles {
    let ctx_i = ctx.with_label(&format!("node{i}"));
    let me = peers[i].public_key();

    // The five plane channels, each behind its own persistent `Muxer`.
    let register = |channel: u64| {
        let control = oracle.control(me.clone());
        async move { control.register(channel, QUOTA).await.expect("channel") }
    };
    let (vs, vr) = register(VOTE_CHANNEL).await;
    let (cs, cr) = register(CERT_CHANNEL).await;
    let (rs, rr) = register(RESOLVER_CHANNEL).await;
    let (bs, br) = register(BROADCAST_CHANNEL).await;
    let (ms, mr) = register(MARSHAL_CHANNEL).await;
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

    let chain = FakeChain::with_genesis(genesis_hash);
    let divergent_at = match role {
        Role::DivergentResult { at } => Some(at),
        _ => None,
    };
    let deriver = FakeDeriver::new(chain.clone(), divergent_at);
    let reader = SnapshotReader {
        schedule: schedule.clone(),
        epoch_len: cfg.epoch_len,
    };
    let (hook_tx, mut hook_rx) = mpsc::unbounded_channel::<OrderBlock>();
    let soft_enter = {
        let schedule = schedule.clone();
        Arc::new(move |from: Epoch, to: Epoch| {
            let span: Vec<(u64, ValidatorSetSnapshot)> = (from.get()..=to.get())
                .filter_map(|e| schedule(e).map(|s| (e, s)))
                .collect();
            Box::pin(async move { span }) as futures::future::BoxFuture<'static, _>
        })
    };
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
        )
        .await,
        upstream_counters.clone(),
    );
    // The executor's frozen-tip frontier probe, wired as `dpos.rs::launch`
    // wires it for a plane validator (`get_latest` → height). The re-jump
    // itself is NOT modelled (it drives reth EL sync): its gate is `u64::MAX`
    // and the callback a counted no-op `Lagging`.
    let re_jump = {
        let probe: FrontierProbeFn = {
            let up = upstream.clone();
            Arc::new(move || {
                let up = up.clone();
                Box::pin(
                    async move { up.get_latest().await.map(|uf| Height::new(uf.block.height)) },
                )
            })
        };
        let rejump_calls = upstream_counters.rejump_calls.clone();
        let call: ReJumpFn = Arc::new(move |_from: u64| {
            rejump_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { JumpOutcome::Lagging })
        });
        ReJump {
            call,
            upstream_frontier: Arc::new(AtomicU64::new(0)),
            threshold: u64::MAX,
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
    let (randomness, artifacts, agreement_intake) = match (cfg.beacon, role) {
        (Beacon::Static, _) => (StaticRandomness::build(CHAIN_ID, full_snapshot), None, None),
        (Beacon::Live, Role::AbsentBeacon) => (beacon::absent(&ctx_i), None, None),
        (Beacon::Live, _) => {
            let (bcs, bcr) = register(BEACON_CHANNEL).await;
            let (brs, brr) = register(BEACON_RESOLVER_CHANNEL).await;
            let roster = {
                let schedule = schedule.clone();
                move |epoch: u64| -> Option<Set<PeerPubkey>> {
                    let snap = schedule(epoch)?;
                    if snap.validators.is_empty() {
                        return None;
                    }
                    Some(Set::from_iter_dedup(
                        snap.validators.iter().map(|v| v.keys.peer_pubkey.clone()),
                    ))
                }
            };
            let committee_for: CommitteeFor = {
                let roster = roster.clone();
                Arc::new(roster)
            };
            let committee_pair_for: CommitteePairFor = {
                let roster = roster.clone();
                Arc::new(move |target| Some((roster(target.checked_sub(1)?)?, roster(target)?)))
            };
            let committee_source: CommitteeSource = {
                let schedule = schedule.clone();
                Arc::new(move |epoch| {
                    let snap = schedule(epoch)?;
                    if snap.validators.is_empty() {
                        return None;
                    }
                    epoch_committee_from_snapshot(&snap).ok()
                })
            };
            // The contract's rule, over the schedule: `dkgQual[e] = committee[e]
            // != committee[e-1]`; "committed" = the schedule has the epoch. The
            // state hash is irrelevant here (every read is the schedule), so
            // `dkg_qual_at` answers a constant.
            let dkg_qual_probe: DkgQualProbe = {
                let roster = roster.clone();
                Arc::new(move |epoch, _at| {
                    let committed = roster(epoch).is_some();
                    let bit = committed && epoch > 0 && roster(epoch) != roster(epoch - 1);
                    Some((bit, committed))
                })
            };
            let plane = beacon::build(
                &ctx_i,
                BeaconConfig {
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
                    committee_for,
                    committee_pair_for,
                    committee_source,
                    dkg_qual_at: Arc::new(|| Some(B256::ZERO)),
                    dkg_qual_probe,
                    heights: dkg_height_rx,
                    plane_clock: plane_clock.clone(),
                    geometry: {
                        let epoch_len = cfg.epoch_len;
                        Box::pin(async move { Some((0, epoch_len)) })
                    },
                    partition_prefix: format!("node{i}-"),
                },
            )
            .await
            .expect("beacon::build");
            // The plane's task handles are detached on drop (commonware `Handle`
            // has no `Drop`); they live until the runner returns.
            (
                plane.randomness,
                Some(plane.artifact_bytes),
                Some(plane.agreement_intake),
            )
        }
    };
    let dkg_height_tx = matches!(cfg.beacon, Beacon::Live).then_some(dkg_height_tx);

    let outer = OuterBuilder {
        me: me.clone(),
        blocker: NoopBlocker,
        provider: oracle.manager(),
        chain_id: CHAIN_ID,
        epoch_length_blocks: NonZeroU64::new(cfg.epoch_len).expect("epoch_len > 0"),
        dpos_activation_block: 0,
        signer_keypair: Some(bls[i].clone()),
        randomness,
        spawn_unblocked: Arc::new(Notify::new()),
        re_jump: Some(re_jump),
        soft_enter_committees: soft_enter,
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
        beacon_engine: FakeBeacon,
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
        slasher_reader: reader,
        slasher_latest_finalized_hash: Arc::new(|| None),
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

    // Cold start: the epoch-0 verifier so the marshal can check certificates
    // before the first boundary (as `dpos.rs::launch` does).
    let snap0 = schedule(0).expect("epoch 0 committee");
    let committee0 = epoch_committee_from_snapshot(&snap0).expect("unique committee");
    outer.cold_start_register(
        Epoch::new(0),
        build_verifier(&fluent_namespace(CHAIN_ID), committee0.bimap, 0, None),
    );

    // Boundary relay — the stand-in for `EpochTransition` (step 5): the
    // cold-start arm enters `cfg.cold_start_epoch`; on the last block of epoch
    // E it hands the manager `(E+1, committee[E+1])` from the schedule.
    let boundary_tx = outer.boundary_sender();
    let trace = Arc::new(Mutex::new(Vec::<TraceEntry>::new()));
    {
        let (schedule, trace, epoch_len, cold) = (
            schedule.clone(),
            trace.clone(),
            cfg.epoch_len,
            cfg.cold_start_epoch,
        );
        ctx_i
            .with_label("boundary_relay")
            .spawn(move |_| async move {
                let mut last_sent: Option<u64> = None;
                if let Some(s0) = schedule(cold) {
                    let _ = boundary_tx.send((Epoch::new(cold), s0)).await;
                    last_sent = Some(cold);
                }
                while let Some(block) = hook_rx.recv().await {
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
                    if !is_epoch_boundary(block.height, 0, epoch_len) {
                        continue;
                    }
                    let Some(epoch) = epoch_at_block(block.height, 0, epoch_len) else {
                        continue;
                    };
                    let next = epoch + 1;
                    if last_sent.is_some_and(|l| next <= l) {
                        continue;
                    }
                    let Some(snap) = schedule(next) else {
                        continue;
                    };
                    if snap.validators.is_empty() {
                        continue;
                    }
                    if boundary_tx.send((Epoch::new(next), snap)).await.is_err() {
                        return;
                    }
                    last_sent = Some(next);
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
        trace,
        upstream: upstream_counters,
        artifacts,
    }
}
