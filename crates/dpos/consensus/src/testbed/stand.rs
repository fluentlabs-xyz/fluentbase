//! `Stand`: N `OuterEngine`s on one deterministic runner + one simulated network.

use super::{
    capture::{self, Captured, Sink},
    fakes::{
        genesis_sealed, FakeBeacon, FakeChain, FakeDeriver, NoSink, NoTxs, NoUpstream, Schedule,
        SnapshotReader,
    },
};
use crate::{
    beacon::surface::StaticRandomness,
    dpos::VoteBackupItem,
    epoch_manager::EpochEngineMetrics,
    executor::ExecutorMetrics,
    extra_data::decode_production_record,
    order_block::{anchor_order_block, OrderBlock},
    outer::OuterBuilder,
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
        BROADCAST_CHANNEL, CERT_CHANNEL, DKG_SUBCHANNEL_BASE, MARSHAL_CHANNEL, RESOLVER_CHANNEL,
        VOTE_CHANNEL,
    },
    NoopBlocker,
};
use fluentbase_staking_reader::reader::{
    is_epoch_boundary, ConsensusKeys, ValidatorSetSnapshot, ValidatorWithKeys,
};
use fluentbase_types::staking_protocol::epoch_at_block;
use rand_08::{rngs::StdRng, SeedableRng as _};
use std::{
    collections::HashMap,
    num::{NonZeroU32, NonZeroU64},
    sync::{Arc, Mutex},
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
}

/// The tracked peer set the marshal's by-height resolver draws peers from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PeerSet {
    /// Every node, tracked once at index 0 — production's `active_registry ∪
    /// committee[E]` for a registry that never changes.
    AllNodes,
    /// `committee[E]` only, re-tracked at index `E` when the chain enters epoch
    /// `E`, and the simulated links of a node outside the set are REMOVED (the
    /// authenticated transport kills an untracked peer's connection at its next
    /// tick; the simulated network keeps delivering over a link whatever the
    /// tracked set says, so the severance has to be modelled by hand). A node
    /// outside the committee is then an unregistered joiner: no peers at all.
    /// Not combinable with `Stand::partition` (both own the links).
    Committee,
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

pub(super) struct Outcome {
    /// Tier-F (finalized-executed) tip per node.
    pub heights: Vec<u64>,
    /// `hashes[i][h - 1]` = `(h, finalized-executed hash)` for `h in 1..=heights[i]`.
    pub hashes: Vec<Vec<(u64, B256)>>,
    /// First `(node, height)` whose executed hash disagrees with the majority.
    pub diverged: Option<(usize, u64)>,
    /// Every node whose `SafetyHalt` latch is engaged, with the typed reason.
    pub halted: Vec<(usize, String)>,
    pub traces: Vec<Vec<TraceEntry>>,
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

    /// The `(height, view, leader, digest, hash)` trace of node `i`, as bytes —
    /// the object test (6) compares across runs.
    pub(super) fn trace_bytes(&self, i: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for e in &self.traces[i] {
            out.extend_from_slice(&e.height.to_be_bytes());
            out.extend_from_slice(&e.view.to_be_bytes());
            out.push(e.leader.unwrap_or(0xFF));
            out.extend_from_slice(e.digest.as_slice());
            out.extend_from_slice(e.hash.unwrap_or(B256::ZERO).as_slice());
        }
        out
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

    // One simulated network; every node is a tracked peer of every other one.
    let (network, oracle) = Network::<_, PeerPubkey>::new(
        ctx.with_label("net"),
        SimConfig {
            max_size: 4 * 1024 * 1024,
            disconnect_on_block: false,
            tracked_peer_sets: NZUsize!(4),
        },
    );
    network.start();
    oracle
        .manager()
        .track(0, Set::from_iter_dedup(pks.iter().cloned()))
        .await;
    let link = Link {
        latency: cfg.latency,
        jitter: Duration::ZERO,
        success_rate: 1.0 - cfg.loss,
    };
    let mut linked: std::collections::HashSet<(PeerPubkey, PeerPubkey)> =
        std::collections::HashSet::new();
    for a in &pks {
        for b in &pks {
            if a != b {
                oracle
                    .add_link(a.clone(), b.clone(), link.clone())
                    .await
                    .expect("link");
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
                    for (x, y) in cross_pairs(part) {
                        oracle
                            .remove_link(pks[x].clone(), pks[y].clone())
                            .await
                            .expect("remove link");
                    }
                    part_obs[p].heights_at_cut = progress.heights.clone();
                    part_obs[p].cut_at = progress.elapsed;
                    part_state[p] = PartState::Active(progress.elapsed);
                }
                PartState::Active(since) if progress.elapsed >= since + part.duration => {
                    for (x, y) in cross_pairs(part) {
                        oracle
                            .add_link(pks[x].clone(), pks[y].clone(), link.clone())
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
        if cfg.peer_set == PeerSet::Committee {
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
                        // Sever every link that touches a node outside the set;
                        // restore the links among members (a re-seated node).
                        for a in &pks {
                            for b in &pks {
                                if a == b {
                                    continue;
                                }
                                let inside = members.contains(a) && members.contains(b);
                                let is_linked = linked.contains(&(a.clone(), b.clone()));
                                if inside && !is_linked {
                                    oracle
                                        .add_link(a.clone(), b.clone(), link.clone())
                                        .await
                                        .expect("add link");
                                    linked.insert((a.clone(), b.clone()));
                                } else if !inside && is_linked {
                                    oracle
                                        .remove_link(a.clone(), b.clone())
                                        .await
                                        .expect("remove link");
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
    Outcome {
        heights,
        hashes,
        diverged,
        halted,
        traces,
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

fn first_divergence(heights: &[u64], hashes: &[Vec<(u64, B256)>]) -> Option<(usize, u64)> {
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
        let mut counts: HashMap<B256, usize> = HashMap::new();
        for (_, x) in &present {
            *counts.entry(*x).or_default() += 1;
        }
        let majority = counts
            .iter()
            .max_by_key(|(_, c)| **c)
            .map(|(x, _)| *x)
            .unwrap();
        if let Some((i, _)) = present.iter().find(|(_, x)| *x != majority) {
            return Some((*i, h));
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
async fn build_node(
    ctx: &deterministic::Context,
    i: usize,
    cfg: &StandConfig,
    role: Role,
    peers: &[Ed25519PrivateKey],
    bls: &[ValidatorBlsKeypair],
    oracle: &commonware_p2p::simulated::Oracle<PeerPubkey, deterministic::Context>,
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

    let outer = OuterBuilder {
        me: me.clone(),
        blocker: NoopBlocker,
        provider: oracle.manager(),
        chain_id: CHAIN_ID,
        epoch_length_blocks: NonZeroU64::new(cfg.epoch_len).expect("epoch_len > 0"),
        dpos_activation_block: 0,
        signer_keypair: Some(bls[i].clone()),
        randomness: StaticRandomness::build(CHAIN_ID, full_snapshot),
        spawn_unblocked: Arc::new(Notify::new()),
        re_jump: None,
        soft_enter_committees: soft_enter,
        epoch_metrics,
        executor_metrics,
        sync_metrics,
        safety_halt: halt.clone(),
        tombstones: TombstoneSet::default(),
        plane_clock,
        dkg_height_tx: None,
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
        agreement_intake: None,
        #[cfg(feature = "dpos-devnet-byzantine")]
        byzantine: matches!(role, Role::Equivocate)
            .then_some(crate::byzantine::ByzantineMode::Equivocate),
    };
    let outer = outer
        .build(ctx_i.with_label("outer"))
        .await
        .expect("OuterBuilder::build");

    // Cold start: the epoch-0 verifier so the marshal can check certificates
    // before the first boundary (as `dpos.rs::launch` does).
    let snap0 = schedule(0).expect("epoch 0 committee");
    let committee0 = epoch_committee_from_snapshot(&snap0).expect("unique committee");
    outer.cold_start_register(
        Epoch::new(0),
        build_verifier(&fluent_namespace(CHAIN_ID), committee0.bimap, 0, None),
    );

    // Boundary relay — the stand-in for `EpochTransition` (step 5): the
    // cold-start arm enters epoch 0; on the last block of epoch E it hands the
    // manager `(E+1, committee[E+1])` from the schedule.
    let boundary_tx = outer.boundary_sender();
    let trace = Arc::new(Mutex::new(Vec::<TraceEntry>::new()));
    {
        let (schedule, trace, epoch_len) = (schedule.clone(), trace.clone(), cfg.epoch_len);
        ctx_i
            .with_label("boundary_relay")
            .spawn(move |_| async move {
                let mut last_sent: Option<u64> = None;
                if let Some(s0) = schedule(0) {
                    let _ = boundary_tx.send((Epoch::new(0), s0)).await;
                    last_sent = Some(0);
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
        None::<NoUpstream>,
    );

    NodeHandles { chain, halt, trace }
}
