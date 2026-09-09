//! The mandatory stand tests (task Э3.2, this session). Every one runs on the
//! deterministic runner without the `external` feature; the real run times are
//! recorded in `.dpos-study/history/E3-2-STAND-1.md` §3.

use super::{
    fakes::UpstreamCounters,
    stand::{
        Committees, Divergence, Outcome, PeerSet, Progress, Role, Stand, StandConfig, CHAIN_ID,
    },
};
use crate::beacon::{
    artifact::decode_artifact, outcome::group_public_key, seed::prev_randao_from_seed, Seed,
};
use alloy_primitives::B256;
use commonware_codec::Encode as _;
use fluentbase_bls::{
    beacon::{seed_namespace, verify_seed, GroupPublic},
    fluent_namespace,
};
use fluentbase_staking_reader::epoch_transition::TransitionOutcome;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

/// (A) One node, nobody to serve it: `PlaneUpstreamHandle::get_latest` on the
/// deterministic runner. The resolver has no candidate peer (the tracked set is
/// the node itself), so the fetch parks and the call has to come back `None`
/// after `FRONTIER_FETCH_TIMEOUT` (8 s) of VIRTUAL time.
///
/// Falsifier: a panic (the timeout needs a runtime the deterministic runner
/// does not provide), a `Some`, or the virtual clock not having moved by the
/// timeout — and, for the "virtual" claim, real time comparable to 8 s.
#[test]
fn a_frontier_fetch_with_no_peer_times_out_on_the_runtime_clock() {
    use crate::cert_follow::CertUpstream as _;
    use commonware_p2p::{
        simulated::{Config as SimConfig, Network},
        Manager as _,
    };
    use commonware_runtime::{deterministic, Clock as _, Metrics as _, Runner as _};
    use commonware_utils::{ordered::Set, NZUsize};
    use std::time::Instant;

    let started = Instant::now();
    let run = std::panic::catch_unwind(|| {
        let runner = deterministic::Runner::new(deterministic::Config::default().with_seed(1));
        runner.start(|ctx| async move {
            let (peers, _) = super::stand::keys(1, 1);
            let me = commonware_cryptography::Signer::public_key(&peers[0]);
            let (network, oracle) = Network::<_, fluentbase_bls::PeerPubkey>::new(
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
                .track(0, Set::from_iter_dedup([me.clone()]))
                .await;
            let upstream = super::stand::frontier_plane(
                &ctx,
                &oracle,
                me,
                Arc::new(std::sync::OnceLock::new()),
                UpstreamCounters::default(),
            )
            .await;
            let t0 = ctx.current();
            let got = upstream.get_latest().await;
            (
                got.map(|uf| uf.block.height),
                ctx.current().duration_since(t0).unwrap(),
            )
        })
    });
    let real = started.elapsed();
    match &run {
        Ok((got, virt)) => {
            eprintln!("(A) get_latest -> {got:?} after virtual {virt:?}, real {real:?}")
        }
        Err(payload) => eprintln!(
            "(A) get_latest PANICKED after real {real:?}: {}",
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "<non-string panic>".into())
        ),
    }
    let (got, virt) =
        run.unwrap_or_else(|_| panic!("get_latest panicked under the deterministic runner"));
    assert_eq!(got, None);
    assert_eq!(virt, Duration::from_secs(8), "virtual time at return");
    assert!(
        real < Duration::from_secs(4),
        "real time {real:?} — the timeout ran on the wall clock"
    );
}

fn reached(h: u64) -> impl Fn(&Progress) -> bool + Send + 'static {
    move |p| p.min_height() >= h
}

/// (1) N=4 honest, six blocks: one chain on every node.
///
/// Falsifier: a node executing a different hash at any height fails
/// `assert_lockstep_except`; a node not reaching six fails on `timed_out`.
#[test]
fn four_honest_nodes_finalize_six_blocks_in_lockstep() {
    let out = Stand::new(StandConfig::honest(4, 1)).run_until(reached(6), Duration::from_secs(60));
    assert!(
        !out.timed_out,
        "heights {:?} after {:?}",
        out.heights, out.virtual_elapsed
    );
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[]);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    eprintln!(
        "(1) heights={:?} virtual={:?} real={:?} logs={} capture_live={}",
        out.heights,
        out.virtual_elapsed,
        out.real_elapsed,
        out.logs.len(),
        out.log_capture_live
    );
}

/// (2) N=8 honest, six blocks.
#[test]
fn eight_honest_nodes_finalize_six_blocks_in_lockstep() {
    let out = Stand::new(StandConfig::honest(8, 1)).run_until(reached(6), Duration::from_secs(60));
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[]);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    eprintln!(
        "(2) heights={:?} virtual={:?} real={:?}",
        out.heights, out.virtual_elapsed, out.real_elapsed
    );
}

/// (3) Node 2 derives a different block at height 3. Its `result` at height
/// 3+K disagrees with everyone else's: the others reject its proposals, and its
/// own executor trips `ResultDivergence` → `SafetyHalt` — on node 2 only.
///
/// Falsifier: `diverged` names another node/height (the fake's divergence did
/// not reach the executed hash), `halted` is empty (the result gate did not
/// catch it) or names an honest node (the halt spread), the honest three drift.
#[test]
fn one_divergent_deriver_is_isolated_and_safety_halts() {
    let mut stand = Stand::new(StandConfig::honest(4, 1));
    stand.node(2).role(Role::DivergentResult { at: 3 });
    let out = stand.run_until(
        |p| p.min_height_of(&[0, 1, 3]) >= 9 && p.halted[2],
        Duration::from_secs(120),
    );
    assert!(
        !out.timed_out,
        "heights {:?} halted {:?}",
        out.heights, out.halted
    );
    assert_eq!(
        out.diverged,
        Some(Divergence::Minority { node: 2, height: 3 })
    );
    assert_eq!(out.halted.len(), 1, "{:?}", out.halted);
    assert_eq!(out.halted[0].0, 2);
    assert!(
        out.halted[0].1.contains("ResultDivergence"),
        "reason: {}",
        out.halted[0].1
    );
    out.assert_lockstep_except(&[2]);
    if out.log_capture_live {
        assert!(
            !out.logs_containing("SafetyHalt").is_empty(),
            "no SafetyHalt log line captured; errors: {:?}",
            out.errors()
        );
    }
    eprintln!(
        "(3) heights={:?} halted={:?} virtual={:?} real={:?} errors={:?}",
        out.heights,
        out.halted,
        out.virtual_elapsed,
        out.real_elapsed,
        out.errors()
    );
}

fn shrink_to_three() -> Committees {
    Committees::Schedule(Arc::new(|epoch, n| {
        Some(if epoch == 0 {
            (0..n).collect()
        } else {
            vec![0, 1, 2]
        })
    }))
}

fn epoch_first_block_views(
    out: &super::stand::Outcome,
    node: usize,
    epoch_len: u64,
) -> Vec<(u64, u64)> {
    out.traces[node]
        .iter()
        .filter(|e| e.height % epoch_len == 0 && e.height > 0)
        .map(|e| (e.height / epoch_len, e.view))
        .collect()
}

/// (4a) `epoch_len = 5`, committee 4 → 3 from epoch 1, every node tracked as a
/// peer throughout (a registered validator that lost its seat): the three
/// members cross the boundaries and the rotated-out node FOLLOWS the chain.
/// The run stops when every node has reached 16, so node 0's trace holds
/// EXACTLY the first blocks of epochs 1, 2 and 3 (heights 5, 10, 15) — the
/// fourth boundary block (20) is past the predicate. (Session 1 saw four with
/// the members at 24: without the upstream plane the outsider caught up in a
/// burst while the members ran ahead of the predicate.)
///
/// Falsifier: a boundary that does not pass (`epoch_first_block_views` short),
/// or node 3 not reaching 16 (`timed_out`), or two chains.
#[test]
fn epoch_boundaries_pass_with_a_shrinking_committee_and_a_tracked_dropped_node_follows() {
    let mut cfg = StandConfig::honest(4, 1);
    cfg.epoch_len = 5;
    cfg.committees = shrink_to_three();
    let out = Stand::new(cfg).run_until(reached(16), Duration::from_secs(120));
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[]);
    let views = epoch_first_block_views(&out, 0, 5);
    assert_eq!(
        views.iter().map(|(e, _)| *e).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "first blocks of epochs 1..=3 (heights 5, 10, 15) on node 0: {views:?}"
    );
    eprintln!(
        "(4a) heights={:?} epoch-first-block views={:?} upstream={:?} virtual={:?} real={:?}",
        out.heights, views, out.upstream, out.virtual_elapsed, out.real_elapsed
    );
}

/// (4b) The same rotation with the peer set narrowed to `committee[E]` and
/// EVERY link of a node outside it severed — consensus plane and upstream
/// plane: the rotated-out node is an unregistered joiner from epoch 1 on. It
/// has no backfill source and STANDS; the members go on. Its frontier probe
/// keeps asking (`latest_calls > 0`) and nobody answers (`latest_delivered ==
/// 0`): the upstream plane without a peer is not a path.
///
/// Falsifier: node 3 keeping up (then something other than a peer feeds it),
/// the members not crossing the boundaries, or a `Latest` answer arriving
/// over no link.
#[test]
fn a_node_outside_the_tracked_peer_set_stands_at_the_boundary() {
    let mut cfg = StandConfig::honest(4, 1);
    cfg.epoch_len = 5;
    cfg.committees = shrink_to_three();
    cfg.peer_set = PeerSet::Committee {
        upstream_link: false,
    };
    let out = Stand::new(cfg).run_until(
        |p| p.min_height_of(&[0, 1, 2]) >= 16,
        Duration::from_secs(120),
    );
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[3]);
    let views = epoch_first_block_views(&out, 0, 5);
    assert!(
        views.len() >= 3,
        "epochs 1..=3 not all entered on node 0: {views:?}"
    );
    assert!(
        out.heights[3] < 10,
        "node 3 kept following outside the peer set ({:?}) — revisit (4b)/(4c)",
        out.heights
    );
    let u3 = out.upstream[3];
    // The live form of step A's timeout: the probe's first `get_latest` has no
    // peer to reach and expires on the virtual clock (8 s) — the executor then
    // issues the next one. A `fetch_one` that never returned would leave
    // exactly one call.
    assert!(
        u3.latest_calls >= 2,
        "node 3's first frontier fetch never expired (a hung fetch_one): {u3:?}"
    );
    assert_eq!(
        u3.latest_delivered, 0,
        "a Latest answer over no link: {u3:?}"
    );
    assert_eq!(
        u3.finalized_delivered, 0,
        "a by-height answer over no link: {u3:?}"
    );
    eprintln!(
        "(4b) heights={:?} epoch-first-block views={:?} upstream={:?} virtual={:?} real={:?}",
        out.heights, views, out.upstream, out.virtual_elapsed, out.real_elapsed
    );
}

/// (4c) The step-3 target for (4b): the same rotation, the outsider's
/// consensus-plane links severed exactly as in (4b), but its `FRONTIER_CHANNEL`
/// links to the members left in place. Node 3 keeps following as a verifier
/// THROUGH the upstream plane: its frozen-tip probe (`get_latest`) discovers
/// the frontier, the hint makes its marshal pull the certificates by height
/// (`get_finalization` — the `MarshalResolver::Hybrid` upstream arm), the
/// verifier scheme from the boundary snapshot verifies them, and the executor
/// derives.
///
/// Falsifier: node 3 standing at 4 as in (4b) (`timed_out`); or node 3
/// reaching 16 with `latest_delivered == 0` or `finalized_delivered == 0` —
/// then it followed over something other than the plane (the simulated
/// network delivering over a link the tracked set does not cover, session 1
/// §2 #8) and the test proves nothing; or no member having served a request.
#[test]
fn a_node_outside_the_tracked_peer_set_keeps_following_through_the_upstream_plane() {
    let mut cfg = StandConfig::honest(4, 1);
    cfg.epoch_len = 5;
    cfg.committees = shrink_to_three();
    cfg.peer_set = PeerSet::Committee {
        upstream_link: true,
    };
    let out = Stand::new(cfg).run_until(reached(16), Duration::from_secs(120));
    assert!(
        !out.timed_out,
        "heights {:?} upstream {:?}",
        out.heights, out.upstream
    );
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[]);
    let u3 = out.upstream[3];
    assert!(
        u3.latest_delivered > 0,
        "node 3 reached {} with no Latest delivered through the plane: {u3:?}",
        out.heights[3]
    );
    assert!(
        u3.finalized_delivered > 0,
        "node 3 reached {} with no by-height cert delivered through the plane: {u3:?}",
        out.heights[3]
    );
    let served: u64 = (0..3).map(|i| out.upstream[i].serve_requests).sum();
    assert!(
        served > 0,
        "no member served a frontier request: {:?}",
        out.upstream
    );
    assert_eq!(u3.rejump_calls, 0, "the re-jump gate is u64::MAX: {u3:?}");
    eprintln!(
        "(4c) heights={:?} upstream={:?} virtual={:?} real={:?}",
        out.heights, out.upstream, out.virtual_elapsed, out.real_elapsed
    );
}

/// (4d) The boundary between "simulated network" and "authenticated transport":
/// the tracked set narrowed to `committee[E]` as in (4b)/(4c), but EVERY link
/// left in place. `commonware_p2p::simulated` delivers over a link whatever the
/// tracked set says (session 1 §2 #8), so the outsider keeps receiving the
/// plane's broadcasts and follows the chain. On the authenticated transport an
/// untracked peer's connection is killed at the next tick, so this
/// configuration does NOT model a production node — it pins what the stand's
/// `PeerSet::Committee` link severance has to model by hand, and the exact
/// heights the run reaches with it left out: `[16, 16, 16, 15]` (session 1,
/// with no upstream plane in the stand, saw `[16, 16, 16, 14]`; the plane
/// now also serves the outsider — 47 `Latest` and 11 by-height deliveries on
/// node 3 in this run).
///
/// Falsifier: node 3 standing (then the tracked set alone severs delivery and
/// the hand-modelled severance is redundant), or a different height vector.
#[test]
fn a_node_outside_the_tracked_peer_set_with_its_links_intact_follows_the_chain() {
    let mut cfg = StandConfig::honest(4, 1);
    cfg.epoch_len = 5;
    cfg.committees = shrink_to_three();
    cfg.peer_set = PeerSet::CommitteeTrackedOnly;
    let out = Stand::new(cfg).run_until(
        |p| p.min_height_of(&[0, 1, 2]) >= 16,
        Duration::from_secs(120),
    );
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    eprintln!(
        "(4d) heights={:?} upstream={:?} virtual={:?} real={:?}",
        out.heights, out.upstream, out.virtual_elapsed, out.real_elapsed
    );
    assert_eq!(out.heights, vec![16, 16, 16, 15]);
}

/// (5) `[0,1] | [2,3]` cut for five views after height 3: no half can finalize
/// (2 < quorum 3); after the links return there is one chain again.
///
/// Falsifier: a tip advancing while cut (a 2-of-4 half finalized — an unsafe
/// certificate), or two chains after the heal (`assert_lockstep_except`).
#[test]
fn a_two_two_partition_stalls_finalization_and_heals_into_one_chain() {
    let mut stand = Stand::new(StandConfig::honest(4, 1));
    stand
        .partition(&[0, 1], &[2, 3])
        .after_height(3)
        .for_views(5);
    let out = stand.run_until(reached(12), Duration::from_secs(120));
    assert!(
        !out.timed_out,
        "heights {:?} partitions {:?}",
        out.heights, out.partitions
    );
    let part = &out.partitions[0];
    assert!(
        part.healed_at > part.cut_at,
        "partition never applied: {part:?}"
    );
    assert_eq!(
        part.heights_at_heal, part.heights_at_cut,
        "a node finalized inside the partition: {part:?}"
    );
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[]);
    eprintln!(
        "(5) heights={:?} cut={:?}@{:?} heal={:?}@{:?} virtual={:?} real={:?}",
        out.heights,
        part.heights_at_cut,
        part.cut_at,
        part.heights_at_heal,
        part.healed_at,
        out.virtual_elapsed,
        out.real_elapsed
    );
}

/// (5′) The same `[0,1] | [2,3]` cut with the upstream plane live on every
/// node. The partition cuts BOTH planes; the frozen tips fire the frontier
/// probes on every node. What the run showed (2026-09-09): NO fetch expires —
/// the resolver's multi-peer fallback (resolver timeout 5 s < the 8 s window)
/// lands every `get_latest` on the peer on the SAME side of the cut, which
/// answers with the same frozen tip (each node answered exactly as many
/// requests as it made: 8 and 8). The plane cannot bridge the cut: there is no
/// certificate on either side to relay, the heights at cut and at heal are
/// equal, and after the heal there is one chain. The isolated-node form of the
/// timeout is (4b) (`latest_calls >= 2`); the pure form is (A).
///
/// Falsifier: a tip advancing while cut; a probe never firing (the executor's
/// frozen-tip tick is dead); a fetch expiring (then the same-side fallback
/// failed and the window, not the resolver, bounded the call); the run hanging
/// or its real time approaching the virtual cut (a wall-clock timer).
#[test]
fn a_two_two_partition_is_not_bridged_by_the_upstream_plane() {
    let mut stand = Stand::new(StandConfig::honest(4, 1));
    stand
        .partition(&[0, 1], &[2, 3])
        .after_height(3)
        .for_views(5);
    let out = stand.run_until(reached(12), Duration::from_secs(120));
    assert!(
        !out.timed_out,
        "heights {:?} partitions {:?}",
        out.heights, out.partitions
    );
    let part = &out.partitions[0];
    assert!(
        part.healed_at > part.cut_at,
        "partition never applied: {part:?}"
    );
    assert_eq!(
        part.heights_at_heal, part.heights_at_cut,
        "a node finalized inside the partition: {part:?}"
    );
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[]);
    let probes: u64 = out.upstream.iter().map(|u| u.latest_calls).sum();
    let expired: u64 = out
        .upstream
        .iter()
        .map(|u| u.latest_calls - u.latest_delivered)
        .sum();
    assert!(
        probes > 0,
        "no frontier probe fired while cut: {:?}",
        out.upstream
    );
    assert_eq!(
        expired, 0,
        "a probe expired inside a 2|2 cut — the same-side fallback did not answer it: {:?}",
        out.upstream
    );
    assert!(
        out.real_elapsed < Duration::from_secs(8),
        "real time {:?} for a {:?} virtual cut: the clock is the wall clock",
        out.real_elapsed,
        part.healed_at - part.cut_at
    );
    eprintln!(
        "(5') heights={:?} cut={:?}@{:?} heal={:?}@{:?} upstream={:?} expired-fetches={expired} virtual={:?} real={:?}",
        out.heights,
        part.heights_at_cut,
        part.cut_at,
        part.heights_at_heal,
        part.healed_at,
        out.upstream,
        out.virtual_elapsed,
        out.real_elapsed
    );
}

/// (C1) `first_divergence` on a 2×2 split: no strict majority, so the outcome
/// is a `Tie` naming the height and both hashes — not a "minority" picked by
/// hash-map order.
///
/// Falsifier: a `Minority` (a winner chosen among equals), or a `Tie` at the
/// wrong height / with the wrong hashes.
#[test]
fn a_two_by_two_split_is_a_tie_not_a_minority() {
    let a = B256::repeat_byte(0xaa);
    let b = B256::repeat_byte(0xbb);
    let common = B256::repeat_byte(0x01);
    let hashes = vec![
        vec![(1, common), (2, a)],
        vec![(1, common), (2, a)],
        vec![(1, common), (2, b)],
        vec![(1, common), (2, b)],
    ];
    let heights = vec![2, 2, 2, 2];
    assert_eq!(
        super::stand::first_divergence(&heights, &hashes),
        Some(Divergence::Tie {
            height: 2,
            hashes: vec![a, b],
        })
    );
    // 3 vs 1 at the same height: a strict majority names the odd node out.
    let hashes = vec![
        vec![(1, common), (2, a)],
        vec![(1, common), (2, b)],
        vec![(1, common), (2, a)],
        vec![(1, common), (2, a)],
    ];
    assert_eq!(
        super::stand::first_divergence(&heights, &hashes),
        Some(Divergence::Minority { node: 1, height: 2 })
    );
    // Two nodes, one each: a tie as well (1 is not more than half of 2).
    let hashes = vec![vec![(1, a)], vec![(1, b)]];
    assert_eq!(
        super::stand::first_divergence(&[1, 1], &hashes),
        Some(Divergence::Tie {
            height: 1,
            hashes: vec![a, b],
        })
    );
}

/// (6) Determinism: seed 1 three times gives the byte-identical
/// `(height, view, leader, digest, executed hash, σ)` trace on node 0; seed 2
/// gives a different one. The keys are a function of the seed too, so the
/// seed-2 run differs in key material as well as in scheduling. (`Beacon::Static`
/// — σ here is the static sharing's; the live-plane form is (B5).)
///
/// Falsifier: any scheduling nondeterminism in the crate (a `HashMap`
/// iteration order reaching a vote, a wall-clock read) shows as a differing
/// view or leader between the three runs.
#[test]
fn the_same_seed_reproduces_the_view_leader_hash_trace_byte_for_byte() {
    let run = |seed: u64| {
        let out =
            Stand::new(StandConfig::honest(4, seed)).run_until(reached(6), Duration::from_secs(60));
        assert!(!out.timed_out, "seed {seed}: {:?}", out.heights);
        (out.trace_bytes(0), out.traces[0].clone(), out.real_elapsed)
    };
    let (a, trace_a, t_a) = run(1);
    let (b, _, t_b) = run(1);
    let (c, _, t_c) = run(1);
    let (d, trace_d, t_d) = run(2);
    assert!(!trace_a.is_empty());
    assert_eq!(a, b, "seed 1, runs 1 and 2 differ");
    assert_eq!(a, c, "seed 1, runs 1 and 3 differ");
    assert_ne!(a, d, "seed 2 reproduced seed 1's trace");
    eprintln!(
        "(6) seed1 trace={:?}\n    seed2 trace={:?}\n    real={:?}/{:?}/{:?}/{:?}",
        trace_a
            .iter()
            .map(|e| (e.height, e.view, e.leader))
            .collect::<Vec<_>>(),
        trace_d
            .iter()
            .map(|e| (e.height, e.view, e.leader))
            .collect::<Vec<_>>(),
        t_a,
        t_b,
        t_c,
        t_d
    );
}

/// (Equivocate) The devnet vote equivocator on node 1 (feature
/// `dpos-devnet-byzantine`): the honest three keep finalizing one chain.
/// What happens to node 1 (a halt, an evidence charge, nothing) is printed and
/// recorded in the report; the stand has no staking contract for the slasher
/// to submit to (`NoSink`).
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn a_vote_equivocator_does_not_stop_the_honest_majority() {
    let mut stand = Stand::new(StandConfig::honest(4, 1));
    stand.node(1).role(Role::Equivocate);
    let out = stand.run_until(
        |p| p.min_height_of(&[0, 2, 3]) >= 6,
        Duration::from_secs(120),
    );
    assert!(
        !out.timed_out,
        "heights {:?} halted {:?}",
        out.heights, out.halted
    );
    out.assert_lockstep_except(&[1]);
    eprintln!(
        "(eq) heights={:?} halted={:?} virtual={:?} real={:?} errors={:?} equivocation-lines={:?}",
        out.heights,
        out.halted,
        out.virtual_elapsed,
        out.real_elapsed,
        out.errors(),
        out.logs_containing("quivoc")
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
    );
}

/// (7) Step-B witness. Phase 1: N=4 finalize six blocks and stop (the runner
/// returns its checkpoint: storage, rng, clock). Phase 2: every node is rebuilt
/// over the same storage — the marshal archives and voter journals replay — and
/// the chain must go on from where it stopped, in lockstep.
///
/// Falsifier: a node that cannot replay its journal (panic / no progress), or
/// nodes that come back on different chains.
#[test]
fn replay_over_prefixed_journals_resumes_every_node() {
    let (first, checkpoint) = Stand::new(StandConfig::honest(4, 1))
        .run_until_recover(reached(6), Duration::from_secs(60));
    assert!(!first.timed_out, "{:?}", first.heights);
    first.assert_lockstep_except(&[]);
    let resume_from = first.heights.iter().copied().max().unwrap();

    let second = Stand::new(StandConfig::honest(4, 1)).replay(
        checkpoint,
        move |p| p.min_height() >= resume_from + 6,
        Duration::from_secs(120),
    );
    assert!(
        !second.timed_out,
        "after replay: heights {:?} halted {:?} errors {:?}",
        second.heights,
        second.halted,
        second.errors()
    );
    assert_eq!(second.diverged, None);
    assert!(second.halted.is_empty(), "{:?}", second.halted);
    second.assert_lockstep_except(&[]);
    // The replayed nodes continued the SAME chain: the first phase's hashes are
    // the prefix of the second phase's.
    for i in 0..4 {
        for (h, hash) in &first.hashes[i] {
            assert_eq!(
                second.hashes[i].get((h - 1) as usize).map(|x| x.1),
                Some(*hash),
                "node {i} height {h} changed across the replay"
            );
        }
    }
    eprintln!(
        "(7) phase1 heights={:?} real={:?}; phase2 heights={:?} real={:?} errors={}",
        first.heights,
        first.real_elapsed,
        second.heights,
        second.real_elapsed,
        second.errors().len()
    );
}

/// (7′) The same replay with every node on the UNPREFIXED production names —
/// what the step-B change prevents. Records what the collision does; the
/// assertion is only that the prefixed run above and this one differ in the
/// way the report states.
#[test]
fn replay_over_shared_journals_records_the_collision() {
    let mut cfg = StandConfig::honest(4, 1);
    cfg.shared_engine_partitions = true;
    let (first, checkpoint) =
        Stand::new(cfg.clone()).run_until_recover(reached(6), Duration::from_secs(60));
    eprintln!(
        "(7') phase1 heights={:?} timed_out={} halted={:?} errors={:?}",
        first.heights,
        first.timed_out,
        first.halted,
        first.errors()
    );
    let resume_from = first.heights.iter().copied().max().unwrap();
    let second = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Stand::new(cfg).replay(
            checkpoint,
            move |p| p.min_height() >= resume_from + 6,
            Duration::from_secs(60),
        )
    }));
    match &second {
        Ok(out) => eprintln!(
            "(7') phase2 heights={:?} timed_out={} halted={:?} errors={:?}",
            out.heights,
            out.timed_out,
            out.halted,
            out.errors()
        ),
        Err(payload) => eprintln!(
            "(7') phase2 panicked: {}",
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "<non-string panic>".into())
        ),
    }
    // The observation (2026-09-09): every node's simplex voter replays the ONE
    // shared `consensus_epoch_0` journal, finds a vote it did not sign, and
    // panics — `replaying notarize from another signer`
    // (`CW:consensus/src/simplex/actors/voter/round.rs:531`), which takes the
    // whole runtime down. Phase 1 runs clean: the collision is invisible while
    // every writer is live, and surfaces only at replay.
    let payload = match second {
        Ok(out) => panic!(
            "shared journals replayed cleanly: heights {:?} halted {:?}",
            out.heights, out.halted
        ),
        Err(payload) => payload,
    };
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(
        message.contains("replaying notarize from another signer"),
        "unexpected panic: {message}"
    );
}

// ── Step 4: the live beacon plane ────────────────────────────────────────────
//
// Every test below runs `StandConfig::live`: `beacon::build` on each node, the
// DKG on the consensus network's BEACON channels, `epoch_len = 32` (the deal
// window is `epoch_len − DKG_MARGIN_BLOCKS = 12` blocks). By
// `DETERMINISTIC_BOOTSTRAP_EPOCH = 2` the first key is minted for EPOCH 2 during
// epoch 1 and epochs 0–1 run seedless, so "the boundary" a seed has to cross is
// height 64.

const EPOCH_LEN: u64 = 32;

/// The finalized-executed hash every node agreed on at `height` — the EL marker
/// a restart carries over (`StandConfig::resume_from`). Asserts the nodes agree,
/// so a replay is never seeded from one node's private fork.
fn finalized_hash_at(out: &Outcome, height: u64) -> B256 {
    let hashes: Vec<B256> = (0..out.hashes.len())
        .map(|i| out.hashes[i][(height - 1) as usize].1)
        .collect();
    assert!(
        hashes.iter().all(|h| *h == hashes[0]),
        "nodes disagree on the executed hash at {height}: {hashes:?}"
    );
    hashes[0]
}

/// `PK_E` out of an artifact's wire bytes — the value the plane serves to a peer
/// over `consensus_getEpochArtifact`, decoded by the production decoder.
fn pk_of(artifact: &[u8]) -> GroupPublic {
    let (proposal, _) = decode_artifact(artifact).expect("the served artifact decodes");
    *group_public_key(&proposal.group_key)
}

/// The number of dealer logs the agreed artifact pins.
fn dealers_of(artifact: &[u8]) -> usize {
    decode_artifact(artifact).expect("decodes").0.logs.len()
}

/// Every listed node holds an artifact for `epoch` whose AGREED half — the
/// `DkgProposal`: target epoch, pinned dealer-log set, `PK_E` + public
/// polynomial, share-confirmations — encodes to the same bytes as node
/// `nodes[0]`'s; returns node `nodes[0]`'s wire bytes. The finalization half is
/// NOT byte-compared: it is a multisig aggregate over whichever ≥ quorum voters
/// each node's instance collected, and two honest nodes legitimately hold
/// different signer sets (the certificate-bitmap trap). The run of 2026-09-09
/// showed exactly that: 1682-byte artifacts identical up to byte 1584 and
/// differing in the 49-byte tail on every pair of nodes.
fn artifact_on_every_node<'a>(out: &'a Outcome, nodes: &[usize], epoch: u64) -> &'a [u8] {
    let first = out.artifacts[nodes[0]]
        .get(&epoch)
        .unwrap_or_else(|| panic!("node {} holds no artifact for epoch {epoch}", nodes[0]));
    let (proposal, _) = decode_artifact(first).expect("decodes");
    for &i in &nodes[1..] {
        let mine = out.artifacts[i]
            .get(&epoch)
            .unwrap_or_else(|| panic!("node {i} holds no artifact for epoch {epoch}"));
        let (theirs, _) = decode_artifact(mine).expect("decodes");
        assert_eq!(
            theirs.encode(),
            proposal.encode(),
            "node {i}'s epoch-{epoch} agreed proposal (PK_E, log set, confirms) differs from node {}'s",
            nodes[0]
        );
    }
    first
}

/// The agreed half of an artifact as bytes — what [`artifact_on_every_node`]
/// compares, and what a replay has to reproduce.
fn proposal_bytes(artifact: &[u8]) -> Vec<u8> {
    decode_artifact(artifact)
        .expect("decodes")
        .0
        .encode()
        .to_vec()
}

/// The seed at `height` on node `nodes[0]`, asserted PRESENT, equal (`Seed`
/// value equality: round + BLS signature) on every listed node, scoped to
/// `epoch`, and verifying under `pk` (the seed namespace of the stand's chain
/// id) on EVERY listed node. Also checks `prev_randao` agrees across the nodes.
fn seed_agreed_at(
    out: &Outcome,
    nodes: &[usize],
    height: u64,
    epoch: u64,
    pk: &GroupPublic,
) -> Seed {
    let ns = seed_namespace(&fluent_namespace(CHAIN_ID));
    let first = out.seeds[nodes[0]]
        .get(&height)
        .cloned()
        .flatten()
        .unwrap_or_else(|| panic!("node {} derived height {height} without σ", nodes[0]));
    assert_eq!(
        first.target_round.epoch().get(),
        epoch,
        "σ at height {height} is scoped to the wrong epoch: {:?}",
        first.target_round
    );
    assert!(
        verify_seed(pk, &ns, first.target_round, &first.signature),
        "σ at height {height} does not verify under the given PK"
    );
    for &i in &nodes[1..] {
        let mine = out.seeds[i].get(&height).cloned().flatten();
        assert_eq!(
            mine.as_ref(),
            Some(&first),
            "node {i}'s σ at height {height} differs from node {}'s (or is missing)",
            nodes[0]
        );
        assert!(
            mine.as_ref()
                .is_some_and(|m| verify_seed(pk, &ns, m.target_round, &m.signature)),
            "node {i}'s σ at height {height} does not verify under the given PK"
        );
        assert_eq!(
            mine.as_ref().map(prev_randao_from_seed),
            Some(prev_randao_from_seed(&first)),
            "prev_randao differs at height {height}"
        );
    }
    first
}

/// The nodes that DERIVED `height` — the ones that hold a σ record for it.
///
/// A node that EL-synced a range never called `derive_and_execute` over it: reth
/// backfilled the bodies from peers and executed them, and `prev_randao` was
/// already in the header. So it holds the block and its hash, and holds NO σ for
/// it. Production is the same shape (`cold_start_jump::RethElSync::sync_to`), so
/// a cross-node σ comparison has to ask who derived, not who is present.
fn derivers_of(out: &Outcome, height: u64) -> Vec<usize> {
    (0..out.seeds.len())
        .filter(|&i| out.seeds[i].get(&height).cloned().flatten().is_some())
        .collect()
}

fn seedless_on_every_node(out: &Outcome, nodes: &[usize], heights: std::ops::Range<u64>) {
    for h in heights {
        for &i in nodes {
            assert!(
                out.seeds[i].get(&h).cloned().flatten().is_none(),
                "node {i} derived height {h} WITH a σ in a pre-beacon epoch"
            );
        }
    }
}

/// (B1) N=4, live DKG: committee[2] deals during epoch 1, the agreement pins one
/// dealer set, every node adopts the SAME artifact — compared as bytes between
/// nodes, and as the decoded `PK_2` — and from the first block of epoch 2
/// (height 64) on, every finalization carries a σ that every node reads
/// identically, that verifies under that `PK_2`, and that yields one
/// `prev_randao`. Heights 1..64 are seedless on every node.
///
/// Falsifier: artifact bytes or `PK_2` differing between two nodes; a node
/// deriving any height ≥ 64 without σ, or with a σ another node did not see; a
/// σ that fails `verify_seed` under the agreed key; a σ before the bootstrap
/// boundary; a `dkg_ceremony_fail` or `engine_demoted_no_polynomial` count.
#[test]
fn four_nodes_agree_the_epoch_key_and_carry_the_seed_across_the_boundary() {
    let out = Stand::new(StandConfig::live(4, 1)).run_until(reached(72), Duration::from_secs(200));
    assert!(
        !out.timed_out,
        "heights {:?} halted {:?} errors {:?}",
        out.heights,
        out.halted,
        out.errors()
    );
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    out.assert_lockstep_except(&[]);
    let all = [0, 1, 2, 3];
    let artifact = artifact_on_every_node(&out, &all, 2);
    let pk = pk_of(artifact);
    for i in all {
        assert_eq!(
            pk_of(&out.artifacts[i][&2]).encode(),
            pk.encode(),
            "PK_2 on node {i}"
        );
        for e in [0, 1, 3] {
            assert!(
                !out.artifacts[i].contains_key(&e),
                "node {i} holds an artifact for epoch {e} — only epoch 2 was minted"
            );
        }
        assert_eq!(
            out.metric(i, "dkg_ceremony_ok_total"),
            Some(1.0),
            "node {i}"
        );
        assert_eq!(
            out.metric(i, "dkg_ceremony_fail_total"),
            Some(0.0),
            "node {i}"
        );
        assert_eq!(
            out.metric(i, "epoch_engine_demoted_no_polynomial_total"),
            Some(0.0),
            "node {i}"
        );
    }
    assert_eq!(dealers_of(artifact), 4, "four dealers pinned");
    seedless_on_every_node(&out, &all, 1..2 * EPOCH_LEN);
    let min = *out.heights.iter().min().unwrap();
    for h in 2 * EPOCH_LEN..=min {
        seed_agreed_at(&out, &all, h, 2, &pk);
    }
    eprintln!(
        "(B1) heights={:?} artifact_bytes={} dealers={} σ@64={:?} virtual={:?} real={:?} warns={:?}",
        out.heights,
        artifact.len(),
        dealers_of(artifact),
        out.seeds[0][&64].as_ref().map(|s| s.target_round),
        out.virtual_elapsed,
        out.real_elapsed,
        out.logs.iter().map(|l| l.text.as_str()).collect::<Vec<_>>()
    );
}

/// The (B2) schedule: 4 → 3 → 4. Epochs 0–2 all four (2 is the bootstrap
/// mint), epochs 3–4 `[0,1,2]` (3 is a change ⇒ `dkgQual[3]`, 4 is stable ⇒
/// carry-forward), epoch 5 all four again (a change ⇒ `dkgQual[5]`).
fn rotate_four_three_four() -> Committees {
    Committees::Schedule(Arc::new(|epoch, n| {
        Some(match epoch {
            3 | 4 => vec![0, 1, 2],
            _ => (0..n).collect(),
        })
    }))
}

/// (B2) Committee rotation over three minting boundaries with the `dkgQual` bit
/// derived from the schedule. What the run has to show: an artifact for epochs
/// 2, 3 and 5 (the mints) and NONE for epoch 4 (carried); three DISTINCT keys;
/// the σ of every height verifying under the key the chain's `dkgQual` history
/// names for its epoch — epoch 4's σ under `PK_3`, not under a fourth key — and
/// identical on every node.
///
/// **Runs with the production steady-state re-jump gate**
/// (`JUMP_THRESHOLD.min(epoch_block_interval)`, `consensus/src/dpos.rs:2466`),
/// and that is load-bearing rather than incidental. Once the committee reads are
/// contract-timed (step 5), the rotated-out node cannot acquire `PK_3` while it
/// sits keyless at the live epoch — the repair sweep that would pull it is woken
/// only by a boundary trigger or a local key insert (`epoch_manager.rs:730`,
/// `:817`) and the catch-up span that raises the frontier is neither
/// (`:741-757`, `:1811`) — so it falls behind, and the RE-JUMP is what carries
/// it forward. With the gate at `u64::MAX` (the stand's default) it parks at 95
/// forever; that observation is pinned by
/// `a_rotated_out_node_without_the_rejump_parks`.
///
/// One consequence is recorded rather than asserted away: the node EL-SYNCS the
/// range it jumps over, so it holds those blocks and their hashes but NO σ
/// record for them — reth executed bodies that already carried `prev_randao`.
/// The per-height σ comparison therefore runs over the nodes that DERIVED the
/// height, with a floor of 3 of 4.
///
/// Falsifier: an artifact for epoch 4 (a ceremony ran on an unchanged
/// committee); a key repeated across mints; an epoch-4 σ that fails under
/// `PK_3`; a node that stops at a boundary (memory trap (1): a non-epoch-pure
/// committee overflows the dealer index and the finalize stalls — here the
/// committees are the schedule's, so a stall is a finding, not a stand error);
/// fewer than three nodes deriving a height.
#[test]
fn three_boundaries_with_committee_rotation_keep_dkg_qual_honest() {
    let mut cfg = StandConfig::live(4, 1);
    cfg.committees = rotate_four_three_four();
    cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD.min(EPOCH_LEN));
    let end = 5 * EPOCH_LEN + 8;
    let out = Stand::new(cfg).run_until(reached(end), Duration::from_secs(400));
    assert!(
        !out.timed_out,
        "heights {:?} halted {:?} errors {:?}",
        out.heights,
        out.halted,
        out.errors()
    );
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    out.assert_lockstep_except(&[]);
    let all = [0, 1, 2, 3];
    let members = [0, 1, 2];
    let pk2 = pk_of(artifact_on_every_node(&out, &all, 2));
    let pk3 = pk_of(artifact_on_every_node(&out, &members, 3));
    let pk5 = pk_of(artifact_on_every_node(&out, &all, 5));
    for i in all {
        assert!(
            !out.artifacts[i].contains_key(&4),
            "node {i} holds an artifact for epoch 4 — the stable committee re-minted"
        );
    }
    assert_ne!(pk2.encode(), pk3.encode(), "epoch 3 re-used epoch 2's key");
    assert_ne!(pk3.encode(), pk5.encode(), "epoch 5 re-used epoch 3's key");
    assert_ne!(pk2.encode(), pk5.encode(), "epoch 5 re-used epoch 2's key");
    seedless_on_every_node(&out, &all, 1..2 * EPOCH_LEN);
    let min = *out.heights.iter().min().unwrap();
    let ns = seed_namespace(&fluent_namespace(CHAIN_ID));
    for h in 2 * EPOCH_LEN..=min {
        let epoch = h / EPOCH_LEN;
        let (key_epoch, pk) = match epoch {
            2 => (2, &pk2),
            3 | 4 => (3, &pk3),
            5 => (5, &pk5),
            _ => unreachable!(),
        };
        // Whoever DERIVED `h` agreed on its σ. The rotated-out node EL-syncs
        // part of epoch 3-4 (see the re-jump note above) and holds no σ record
        // for that range, so it drops out of the comparison there — and the
        // floor of 3 keeps the check from degenerating into a self-comparison.
        let derivers = derivers_of(&out, h);
        assert!(
            derivers.len() >= 3,
            "only {} node(s) derived height {h}: {derivers:?}",
            derivers.len()
        );
        let seed = seed_agreed_at(&out, &derivers, h, epoch, pk);
        // The carried epoch's σ is under PK_3 and under NO other mint.
        if epoch == 4 {
            for other in [&pk2, &pk5] {
                assert!(
                    !verify_seed(other, &ns, seed.target_round, &seed.signature),
                    "epoch-4 σ at {h} verifies under a key other than PK_{key_epoch}"
                );
            }
        }
    }
    // The recovery went through the RE-JUMP, and the node came back to deriving.
    // Without both, "artifacts 2/3/5 everywhere" could be reached by some other
    // route and the test would say nothing about the exit it enables.
    assert!(
        out.upstream[3].rejump_calls >= 1,
        "the rotated-out node never re-jumped: {:?}",
        (0..4)
            .map(|i| out.upstream[i].rejump_calls)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        derivers_of(&out, min),
        all.to_vec(),
        "at the last common height {min} not every node was deriving again"
    );
    let ok: Vec<Option<f64>> = all
        .iter()
        .map(|&i| out.metric(i, "dkg_ceremony_ok_total"))
        .collect();
    assert_eq!(
        ok,
        vec![Some(3.0), Some(3.0), Some(3.0), Some(2.0)],
        "ceremonies finalized per node (members: 2, 3, 5; node 3: 2, 5)"
    );
    for i in all {
        assert_eq!(
            out.metric(i, "dkg_ceremony_fail_total"),
            Some(0.0),
            "node {i}"
        );
    }
    eprintln!(
        "(B2) heights={:?} artifacts={:?} ceremonies_ok={ok:?} virtual={:?} real={:?} warns={:?}",
        out.heights,
        out.artifacts
            .iter()
            .map(|a| a.keys().copied().collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        out.virtual_elapsed,
        out.real_elapsed,
        out.logs.iter().map(|l| l.text.as_str()).collect::<Vec<_>>()
    );
}

/// (B3) N=4, node 3 brings up NO beacon (`Role::AbsentBeacon`: no share, no
/// dealer log, no agreement seat, `beacon::absent` as its randomness — a
/// verifier that never signs). By the code, before the run: the dealer quorum
/// at n=4 under `N3f1` is `n − f = 3` (`ceremony.rs`), the agreement entry bar
/// at n=4 is the bare quorum 3 (`dkg_engine.rs`), and the agreement instance is
/// a 4-seat simplex with 3 live seats — so the three dealers finalize, pin a
/// THREE-log set and mint `PK_2`; the chain goes on with σ on the three. Node 3
/// has no σ source at all (`absent::seed_for` is `None`, `mandatory_at(2)`
/// holds), so its executor parks at height 64 (`OwnRoundSeed::Missing`) — it
/// stops at 63 without a halt.
///
/// Falsifier: the three not minting (`dkg_ceremony_ok != 1`, no artifact); a
/// pinned set of four (a log from a node that never dealt); the chain not
/// crossing 64 on the three; a halt anywhere; node 3 crossing 64 (then it
/// derived without σ in a mandatory epoch).
#[test]
fn one_absent_dealer_does_not_stop_the_key() {
    let mut stand = Stand::new(StandConfig::live(4, 1));
    stand.node(3).role(Role::AbsentBeacon);
    let out = stand.run_until(
        |p| p.min_height_of(&[0, 1, 2]) >= 72,
        Duration::from_secs(200),
    );
    assert!(
        !out.timed_out,
        "heights {:?} halted {:?} errors {:?}",
        out.heights,
        out.halted,
        out.errors()
    );
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    let members = [0, 1, 2];
    out.assert_lockstep_except(&[3]);
    let artifact = artifact_on_every_node(&out, &members, 2);
    let pk = pk_of(artifact);
    assert_eq!(
        dealers_of(artifact),
        3,
        "three dealers pinned, the absent one not"
    );
    // "Node 3 ran no plane" has no observation of its own here: the stand's
    // `artifacts[3]` is empty by construction, and `beacon::absent` registers
    // the same `dkg_*` families (`surface.rs::absent`) — its zero counters are
    // structural too. What does observe the absence is the three-log pinned
    // set above and node 3 parking at 63 below.
    for i in members {
        assert_eq!(
            out.metric(i, "dkg_ceremony_ok_total"),
            Some(1.0),
            "node {i}"
        );
        assert_eq!(
            out.metric(i, "dkg_ceremony_fail_total"),
            Some(0.0),
            "node {i}"
        );
    }
    seedless_on_every_node(&out, &[0, 1, 2, 3], 1..2 * EPOCH_LEN);
    let min = out.heights[..3].iter().copied().min().unwrap();
    for h in 2 * EPOCH_LEN..=min {
        seed_agreed_at(&out, &members, h, 2, &pk);
    }
    assert_eq!(
        out.heights[3],
        2 * EPOCH_LEN - 1,
        "node 3 (no σ source) is expected to park at the bootstrap boundary"
    );
    eprintln!(
        "(B3) heights={:?} dealers={} virtual={:?} real={:?} warns={:?}",
        out.heights,
        dealers_of(artifact),
        out.virtual_elapsed,
        out.real_elapsed,
        out.logs.iter().map(|l| l.text.as_str()).collect::<Vec<_>>()
    );
}

/// (B4) Phase 1: N=4 live, six blocks into epoch 2 (the key minted, σ flowing),
/// stop, keep the checkpoint. Phase 2: every node rebuilt over the SAME storage
/// (key / seed / artifact journals, voter journals, marshal archives) and the
/// SAME share dirs, cold-starting in epoch 2: `beacon::build` reloads the share
/// from disk, the key journal replays `PK_2`, the seed journal replays σ of
/// rounds 64..70 — which phase 2's executor needs, because it re-derives 1..70
/// into a fresh `FakeChain` — and the chain goes on to 82 with σ.
///
/// What proves the share reload: the chain ADVANCING past 70 in epoch 2. A
/// signer's scheme needs the epoch's material, and after a restart the ceremony
/// store holds only what `load_all(share_dir)` put back — with an empty share
/// dir every node is `Withheld(NoUsableShare)` and the chain parks at 70 (the
/// negative control below). What this test does NOT isolate: the SOURCE of σ
/// for 64..70 in phase 2 — the seed journal replays them, but the re-derive
/// could also take them off the archived certificates (the spec-exec reporter
/// records every recovered σ), and a σ is unique per `(round, PK)`, so "the
/// journal replayed something else" cannot show as a different value, only as
/// a missing one. "No second ceremony for epoch 2" is not what the
/// `dkg_ceremony_ok` count proves either: the actor only ever starts
/// `now + 1`, so the count is printed, not asserted.
///
/// Falsifier: a node that cannot come back (`timed_out`); an epoch-2 agreed
/// proposal that differs from phase 1's; a σ at 64..70 missing or scoped to
/// another round; a `demoted_no_polynomial` count (a share NOT reloaded); an
/// ERROR line; two chains.
#[test]
fn restart_replays_key_and_seed_journals() {
    let cfg = StandConfig::live(4, 1);
    let (first, checkpoint) =
        Stand::new(cfg.clone()).run_until_recover(reached(70), Duration::from_secs(200));
    assert!(!first.timed_out, "{:?}", first.heights);
    first.assert_lockstep_except(&[]);
    let all = [0, 1, 2, 3];
    let artifact1 = artifact_on_every_node(&first, &all, 2).to_vec();
    let pk = pk_of(&artifact1);
    let seeds1: BTreeMap<u64, Seed> = (64..=70)
        .map(|h| (h, seed_agreed_at(&first, &all, h, 2, &pk)))
        .collect();
    let resume_from = first.heights.iter().copied().max().unwrap();

    // The restart hands the nodes the EL marker their execution layer persisted
    // — `(finalized height, its executed hash)`, production's
    // `(latest_finalized, latest_finalized_hash)`. `cold_start` derives the epoch
    // from it; nothing here says "2". With a genesis anchor instead, the replayed
    // nodes re-derive 1..70 and then sit in epoch 0 with no engine for epoch 2
    // (observed 2026-09-09: `[70,70,70,70]`, no halt, no error, timed out).
    let mut cfg = cfg;
    cfg.resume_from = Some((resume_from, finalized_hash_at(&first, resume_from)));
    let second = Stand::new(cfg).replay(
        checkpoint,
        move |p| p.min_height() >= resume_from + 12,
        Duration::from_secs(200),
    );
    assert!(
        !second.timed_out,
        "after replay: heights {:?} halted {:?} errors {:?}",
        second.heights,
        second.halted,
        second.errors()
    );
    assert_eq!(second.diverged, None);
    assert!(second.halted.is_empty(), "{:?}", second.halted);
    assert!(second.errors().is_empty(), "{:?}", second.errors());
    second.assert_lockstep_except(&[]);
    for i in all {
        for (h, hash) in &first.hashes[i] {
            assert_eq!(
                second.hashes[i].get((h - 1) as usize).map(|x| x.1),
                Some(*hash),
                "node {i} height {h} changed across the replay"
            );
        }
    }
    assert_eq!(
        proposal_bytes(artifact_on_every_node(&second, &all, 2)),
        proposal_bytes(&artifact1),
        "the epoch-2 agreed proposal changed across the replay"
    );
    for (h, seed) in &seeds1 {
        assert_eq!(
            &seed_agreed_at(&second, &all, *h, 2, &pk),
            seed,
            "σ at height {h} changed across the replay"
        );
    }
    let min = *second.heights.iter().min().unwrap();
    for h in 71..=min {
        seed_agreed_at(&second, &all, h, 2, &pk);
    }
    // Printed, not asserted: the actor only ever starts `now + 1` = 3, and on
    // the stable schedule that is a carry-forward, so a zero here is the
    // schedule's doing and says nothing about the restart.
    let ceremonies: Vec<Option<f64>> = all
        .iter()
        .map(|&i| second.metric(i, "dkg_ceremony_ok_total"))
        .collect();
    for i in all {
        assert_eq!(
            second.metric(i, "epoch_engine_demoted_no_polynomial_total"),
            Some(0.0),
            "node {i} was demoted after the restart — its share did not come back"
        );
    }
    assert!(
        second.logs_containing("re-agreeing").is_empty(),
        "{:?}",
        second.logs_containing("re-agreeing")
    );
    eprintln!(
        "(B4) phase1 heights={:?} real={:?}; phase2 heights={:?} real={:?} ceremonies={ceremonies:?} warns={:?}",
        first.heights,
        first.real_elapsed,
        second.heights,
        second.real_elapsed,
        second.logs.iter().map(|l| l.text.as_str()).collect::<Vec<_>>()
    );
}

/// (B5) Determinism with the live plane: seed 1 three times gives the
/// byte-identical `(height, view, leader, digest, hash, σ)` trace on node 0 —
/// the σ field is what this adds to (6): the DKG's dealings, the agreement's
/// views and the recovered threshold signatures all have to replay identically;
/// seed 2 gives a different trace.
///
/// Falsifier: a wall-clock read or OS randomness anywhere in the beacon plane
/// (a differing σ or view between two runs); a trace without σ (then the field
/// was not exercised).
#[test]
fn a_live_dkg_run_reproduces_the_seed_trace_byte_for_byte() {
    let run = |seed: u64| {
        let out =
            Stand::new(StandConfig::live(4, seed)).run_until(reached(70), Duration::from_secs(200));
        assert!(!out.timed_out, "seed {seed}: {:?}", out.heights);
        assert!(
            out.seeds[0].get(&70).cloned().flatten().is_some(),
            "seed {seed}: no σ at height 70 — the trace carries no seed"
        );
        let sigma = out.seeds[0].get(&70).cloned().flatten();
        (out.trace_bytes(0), out.real_elapsed, sigma)
    };
    let (a, t_a, sigma_1) = run(1);
    let (b, t_b, _) = run(1);
    let (c, t_c, _) = run(1);
    let (d, t_d, sigma_2) = run(2);
    assert_eq!(a, b, "seed 1, runs 1 and 2 differ");
    assert_eq!(a, c, "seed 1, runs 1 and 3 differ");
    assert_ne!(a, d, "seed 2 reproduced seed 1's trace");
    // The σ field itself differs, not only the leaders and digests around it
    // (a different key set mints a different PK_2, so this is expected — the
    // assert pins that the field is live in the comparison).
    assert_ne!(
        sigma_1, sigma_2,
        "seed 2 reproduced seed 1's σ at height 70"
    );
    eprintln!(
        "(B5) trace bytes={} real={t_a:?}/{t_b:?}/{t_c:?}/{t_d:?}",
        a.len()
    );
}

/// (B4′) The negative control for (B4): the same restart with every node's
/// share dir WIPED between the phases. The executor re-derives 1..70 (the
/// height vector says so) — and then every node is `Withheld(NoUsableShare)`
/// for epoch 2 (the demote counter says so), nobody signs, and the chain parks
/// at 70. This is what (B4)'s "advanced past 70" observation rules out. The
/// journals' replay is not asserted here — (B4) covers the proposal and σ.
///
/// Falsifier: the chain advancing past 70 without shares (then a share is not
/// what the signer needs, and (B4) proves nothing about the reload); a
/// `demoted_no_polynomial` count of 0; a halt (parking is verify-only, not a
/// safety fault).
#[test]
fn restart_without_the_share_dirs_parks_the_chain_verify_only() {
    let cfg = StandConfig::live(4, 1);
    let (first, checkpoint) =
        Stand::new(cfg.clone()).run_until_recover(reached(70), Duration::from_secs(200));
    assert!(!first.timed_out, "{:?}", first.heights);
    first.assert_lockstep_except(&[]);
    let resume_from = first.heights.iter().copied().max().unwrap();
    std::fs::remove_dir_all(&cfg.share_root).expect("wipe the share root");

    // Same EL marker as (B4) — this control differs from it in the share dirs
    // only.
    let mut cfg = cfg;
    cfg.resume_from = Some((resume_from, finalized_hash_at(&first, resume_from)));
    let second = Stand::new(cfg).replay(
        checkpoint,
        move |p| p.min_height() > resume_from,
        Duration::from_secs(60),
    );
    assert!(
        second.timed_out,
        "the chain advanced without a share on any node: {:?}",
        second.heights
    );
    assert_eq!(
        second.heights,
        vec![resume_from; 4],
        "re-derived to the stop height and parked"
    );
    assert!(second.halted.is_empty(), "{:?}", second.halted);
    for i in 0..4 {
        let demoted = second.metric(i, "epoch_engine_demoted_no_polynomial_total");
        assert!(
            demoted.is_some_and(|d| d >= 1.0),
            "node {i} was not demoted for a missing share: {demoted:?}"
        );
    }
    eprintln!(
        "(B4') phase2 heights={:?} real={:?} demoted={:?}",
        second.heights,
        second.real_elapsed,
        (0..4)
            .map(|i| second.metric(i, "epoch_engine_demoted_no_polynomial_total"))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// (C) The boundary walk through the production `EpochTransition` over the fake
// staking state. Step 5 of the Э3.2 evaluation.
// ---------------------------------------------------------------------------

/// The boundary list the transition MUST produce, computed here from nothing but
/// the run's finalized-executed hashes and the rule at
/// `epoch_transition.rs:290-293` (`read_height_for(number) = number − result_lag`,
/// clamped to the cold-start anchor): the cold-start entry at the anchor, then
/// one entry per boundary block `B = E * epoch_len − 1`, carrying `epoch E` and
/// the executed hash of `B − K`.
fn expected_boundaries(out: &Outcome, node: usize, epoch_len: u64, upto: u64) -> Vec<(u64, B256)> {
    let hash_at = |h: u64| -> B256 {
        if h == 0 {
            // The genesis anchor is not in `hashes`, which starts at height 1.
            return super::fakes::genesis_sealed().hash();
        }
        out.hashes[node][(h - 1) as usize].1
    };
    let mut want = vec![(0u64, hash_at(0))];
    let mut boundary = epoch_len - 1;
    while boundary <= upto {
        want.push((
            boundary / epoch_len + 1,
            hash_at(boundary - crate::order_block::K),
        ));
        boundary += epoch_len;
    }
    want
}

/// (C2) The chain's committee handoff now runs through the production
/// `EpochTransition` over `FakeStaking`, and the state it reads the committee at
/// is a real executed hash of a real height, not a constant.
///
/// The assert compares each node's boundary trace against a list this test
/// recomputes from `FakeChain`'s hashes and the `number − result_lag` rule —
/// not against anything the stand handed the transition.
///
/// Falsifier: a boundary read at a hash of the wrong height (the epoch-N
/// snapshot carrying `hash_at(B)` instead of `hash_at(B − 3)`, or a constant);
/// two nodes entering an epoch on different state; a geometry frozen to
/// something other than the contract's `(0, 32)`; the transition skipping an
/// epoch or repeating one.
#[test]
fn the_epoch_transition_walks_the_boundaries_from_the_fake_state() {
    let mut cfg = StandConfig::live(4, 1);
    cfg.committees = rotate_four_three_four();
    let out = Stand::new(cfg).run_until(reached(3 * EPOCH_LEN - 1), Duration::from_secs(300));
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);

    for i in 0..4 {
        assert_eq!(
            out.geometry[i],
            Some((0, EPOCH_LEN)),
            "node {i} froze the epoch geometry to something other than the \
             contract's (dposActivationBlock, epochBlockInterval)"
        );
    }
    let want = expected_boundaries(&out, 0, EPOCH_LEN, *out.heights.iter().min().unwrap());
    for i in 0..4 {
        let got: Vec<(u64, B256)> = out.et_boundaries[i]
            .iter()
            .map(|b| (b.epoch, b.block_hash))
            .collect();
        assert!(
            got.len() >= 4,
            "node {i} walked only {} boundaries: {got:?}",
            got.len()
        );
        assert_eq!(
            got.as_slice(),
            &want[..got.len()],
            "node {i}'s boundary walk differs from the one computed from \
             FakeChain.hash_at(boundary − K): {got:?} vs {want:?}"
        );
    }
    // The nodes agree on the first four entries, so "each matches the computed
    // list" is not four independent claims about four different chains.
    for i in 1..4 {
        assert_eq!(
            &out.et_boundaries[i][..4],
            &out.et_boundaries[0][..4],
            "node {i} entered the first four epochs on different state than node 0"
        );
    }
    // The peer set every transition handed its sink is the same on every node:
    // the union `active_registry ∪ committee[E] ∪ committee[E+1]` is a function
    // of chain state alone, so a per-node difference means two nodes read
    // different committees for one epoch. Counted at the sink over the MEMBERS,
    // not their count — the registry union makes every set the same SIZE under
    // `PeerSet::AllNodes`, so a length comparison here would be vacuous.
    assert_eq!(
        out.tracked_mismatches, 0,
        "two nodes' transitions tracked different peer sets for one epoch"
    );
    for i in 0..4 {
        assert!(
            out.tracked[i].len() >= 4,
            "node {i} tracked only {} epochs: {:?}",
            out.tracked[i].len(),
            out.tracked[i]
                .iter()
                .map(|(e, m)| (*e, m.len()))
                .collect::<Vec<_>>()
        );
    }
    // The one exempted ERROR line can only be produced by a peer-set
    // REGISTRATION that actually reached the network, so bound it by those and
    // not by anything looser. The bound is not "one per registration" derived
    // from code — an aborted engine can have several sends in flight — but a
    // stream of drops with no registrations at all would mean the exemption is
    // covering something else entirely, and that this catches.
    assert!(
        out.tracked_forwarded > 0 || out.simulator_ack_drops == 0,
        "{} exempted ack drops with no peer-set registration to cause them",
        out.simulator_ack_drops
    );
    eprintln!(
        "(C2) heights={:?} ack_drops={} boundaries={:?} geometry={:?} virtual={:?} real={:?}",
        out.heights,
        out.simulator_ack_drops,
        out.et_boundaries[0]
            .iter()
            .map(|b| (b.epoch, b.block_number))
            .collect::<Vec<_>>(),
        out.geometry,
        out.virtual_elapsed,
        out.real_elapsed
    );
}

/// (C3 + C5, one observation) An epoch the contract has NOT committed at the
/// read height is not readable early, and every staking read the plane and the
/// transition make resolves at a REAL executed hash of this node's chain.
///
/// The two are one observation on the fake: `StakingReads::uncommitted` counts
/// the reads that came back with an empty committee because the epoch was past
/// the contract's `current_epoch + MAX_COMMITTEE_LOOKAHEAD_EPOCHS` horizon at
/// the read height, and `unknown_state` counts reads at a hash this chain never
/// sealed — which is what a constant `dkg_qual_at` would produce.
///
/// Under the 2-epoch warm-up the only window where an epoch is uncommitted is
/// GENESIS: the bootstrap commits epoch 0 alone, and from block 1 on every
/// epoch up to `epoch(h) + 2` is committed. The transition's own cold start hits
/// it — `track_and_trigger` reads `committee[1]` at the genesis hash for the
/// peer-set union (`epoch_transition.rs:631-638`).
///
/// Falsifier: zero uncommitted reads (the fake answers any epoch, and the
/// "committee not yet committed" branch is unreachable — the state before this
/// step); a read at an unknown hash (the state hash is not the chain's); the DKG
/// failing to mint `PK_2` because of the refusal.
#[test]
fn a_committee_not_yet_committed_is_not_read_early() {
    let out = Stand::new(StandConfig::live(4, 1)).run_until(reached(72), Duration::from_secs(200));
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    for i in 0..4 {
        let reads = &out.staking_reads[i];
        assert_eq!(
            reads.unknown_state, 0,
            "node {i} read the staking state at a hash its chain never sealed \
             ({reads:?}) — the read cursor is not the chain's"
        );
        assert!(
            reads.uncommitted.get(&1).copied().unwrap_or(0) >= 1,
            "node {i} never got the 'not committed yet' answer for epoch 1 at \
             the genesis state: {reads:?}"
        );
        assert!(
            reads.committed.values().sum::<u64>() > 50,
            "node {i} barely read the staking state at all: {reads:?}"
        );
    }
    // And the refusal did not stop the bootstrap mint.
    let all = [0, 1, 2, 3];
    let pk2 = pk_of(artifact_on_every_node(&out, &all, 2));
    seedless_on_every_node(&out, &all, 1..2 * EPOCH_LEN);
    seed_agreed_at(&out, &all, 2 * EPOCH_LEN, 2, &pk2);
    eprintln!(
        "(C3) reads={:?} virtual={:?} real={:?}",
        out.staking_reads[0], out.virtual_elapsed, out.real_elapsed
    );
}

/// (C4) A restarted node is handed only the `(height, hash)` its execution layer
/// persisted; the epoch it resumes in is the transition's own answer.
///
/// Falsifier: the nodes standing up in epoch 0 (the observation of session 3
/// §2 #2, when the stand supplied the epoch from config and a genesis anchor
/// left them there); a first outcome that is not a cold start at the resume
/// height; a geometry that did not freeze on the resumed state.
#[test]
fn cold_start_computes_the_epoch_from_the_finalized_state() {
    let cfg = StandConfig::live(4, 1);
    let (first, checkpoint) =
        Stand::new(cfg.clone()).run_until_recover(reached(70), Duration::from_secs(200));
    assert!(!first.timed_out, "{:?}", first.heights);
    first.assert_lockstep_except(&[]);
    let resume_from = first.heights.iter().copied().max().unwrap();

    let mut cfg = cfg;
    cfg.resume_from = Some((resume_from, finalized_hash_at(&first, resume_from)));
    let second = Stand::new(cfg).replay(
        checkpoint,
        move |p| p.min_height() >= resume_from + 12,
        Duration::from_secs(200),
    );
    assert!(
        !second.timed_out,
        "after replay: heights {:?} halted {:?}",
        second.heights, second.halted
    );
    // On a boundary-aligned anchor `cold_start` enters E+1, not E
    // (`epoch_transition.rs:519-521`) — a real production arm, but not the one
    // this test is about, so pin that the fixture is not on one.
    assert_ne!(
        (resume_from + 1) % EPOCH_LEN,
        0,
        "the resume height is a boundary: cold_start enters E+1 there, and the \
         expectation below would be wrong for a production-correct reason"
    );
    let want_epoch = resume_from / EPOCH_LEN;
    for i in 0..4 {
        assert_eq!(
            second.geometry[i],
            Some((0, EPOCH_LEN)),
            "node {i} did not freeze the geometry on the resumed state"
        );
        let first_step = second.et_steps[i]
            .first()
            .unwrap_or_else(|| panic!("node {i}'s transition made no call at all"));
        // This half is the STAND's own bookkeeping (the feeder records the
        // height it passed in), kept as a guard that the stand fed the anchor it
        // was configured with. The production claim is the outcome below.
        assert_eq!(
            first_step.number, resume_from,
            "node {i} cold-started at a height other than the persisted finalized one"
        );
        assert_eq!(
            first_step.outcome,
            Ok(TransitionOutcome::EpochAdvanced(want_epoch)),
            "node {i} did not enter epoch {want_epoch} = {resume_from} / {EPOCH_LEN} on the \
             cold start"
        );
    }
    eprintln!(
        "(C4) resume_from={resume_from} entered={want_epoch} heights={:?} geometry={:?} real={:?}",
        second.heights, second.geometry, second.real_elapsed
    );
}

/// (C7, verify-only) Zero committee overlap at a boundary HALTS the chain, and
/// nothing detects it. Project memory's trap (7): σ has no backfill, so a
/// committee that shares no member with its predecessor can neither serve the
/// old epoch's key nor be served the new one — enforced nowhere.
///
/// N=8, committee `[0,1,2,3]` through epoch 2 and `[4,5,6,7]` from epoch 3. Both
/// halves walk their boundaries through the transition; then the chain stops.
/// The outgoing half enters epoch 3 as verifiers holding only the epoch-2
/// artifact and parks at the last block of epoch 2; the incoming half minted the
/// epoch-3 artifact (it is `committee[3]`, so it dealt during epoch 2) but never
/// held the epoch-2 key and parked a whole epoch earlier, at the last block of
/// epoch 1.
///
/// This test pins the FACT, not a wish: nothing here says the halt is
/// acceptable. What it forbids is the fact changing silently.
///
/// Falsifier: any node crossing its park height (then zero overlap is
/// survivable and the memory note is wrong); a `SafetyHalt` (then the stop is
/// DETECTED rather than silent, which is a different — and better — world);
/// executed hashes disagreeing (a fork rather than a stop).
#[test]
fn a_zero_overlap_boundary_halts_the_chain_verify_only() {
    let mut cfg = StandConfig::live(8, 1);
    cfg.committees = Committees::Schedule(Arc::new(|epoch, _n| {
        Some(if epoch <= 2 {
            vec![0, 1, 2, 3]
        } else {
            vec![4, 5, 6, 7]
        })
    }));
    let out = Stand::new(cfg).run_until(reached(3 * EPOCH_LEN + 4), Duration::from_secs(400));
    assert!(
        out.timed_out,
        "the chain crossed the zero-overlap boundary: {:?}",
        out.heights
    );
    assert_eq!(
        out.heights,
        vec![
            3 * EPOCH_LEN - 1,
            3 * EPOCH_LEN - 1,
            3 * EPOCH_LEN - 1,
            3 * EPOCH_LEN - 1,
            2 * EPOCH_LEN - 1,
            2 * EPOCH_LEN - 1,
            2 * EPOCH_LEN - 1,
            2 * EPOCH_LEN - 1
        ],
        "the outgoing committee parks at the last block of epoch 2 and the \
         incoming one a whole epoch earlier"
    );
    assert!(
        out.halted.is_empty(),
        "the stop was detected as a safety fault: {:?}",
        out.halted
    );
    assert_eq!(out.diverged, None);
    for i in 0..4 {
        assert_eq!(
            out.artifacts[i].keys().copied().collect::<Vec<_>>(),
            vec![2],
            "outgoing node {i} holds an artifact other than epoch 2's"
        );
    }
    for i in 4..8 {
        assert_eq!(
            out.artifacts[i].keys().copied().collect::<Vec<_>>(),
            vec![3],
            "incoming node {i} holds an artifact other than epoch 3's"
        );
    }
    eprintln!(
        "(C7) heights={:?} boundaries(out)={:?} boundaries(in)={:?} virtual={:?} real={:?}",
        out.heights,
        out.et_boundaries[0]
            .iter()
            .map(|b| (b.epoch, b.block_number))
            .collect::<Vec<_>>(),
        out.et_boundaries[4]
            .iter()
            .map(|b| (b.epoch, b.block_number))
            .collect::<Vec<_>>(),
        out.virtual_elapsed,
        out.real_elapsed
    );
}

/// (C8, verify-only) The "before" half of the re-jump exit: with the jump gate
/// pinned at `u64::MAX` — the stand's default, and what every test written
/// before step 5 assumes — the rotated-out node PARKS and never comes back.
///
/// This is the observation the sweep-wake gap produces, kept so a fix to it has
/// something to move. The mechanism, read out of the code rather than guessed:
/// the node enters epoch 3 as a verifier holding no artifact for it; nothing
/// spends a network pull for the LIVE epoch's key (`epoch_manager.rs:1106-1111`
/// goes to `soft_enter`, and the repair sweep excludes the frontier by
/// construction at `:1677`); the sweep is woken only by a boundary trigger
/// (`:730`) or a local `PK_epoch` insert (`:817`), and the catch-up span that
/// raises the frontier wakes neither (`:741-757`, `:1811`). So it sits at the
/// last block of epoch 2 with the network four epochs ahead.
///
/// It pins the FACT, not a wish: nothing here says parking is acceptable.
///
/// Falsifier: the node moving off 95 with the gate closed (then the wedge has
/// some other exit and the "after" test proves less than it claims); the
/// committee members failing to go on without it; a halt (the park is
/// verify-only, not a safety fault); the three disagreeing on a hash.
#[test]
fn a_rotated_out_node_without_the_rejump_parks() {
    let mut cfg = StandConfig::live(4, 1);
    cfg.committees = rotate_four_three_four();
    assert_eq!(
        cfg.re_jump_threshold, None,
        "the gate must stay closed here"
    );
    let members = [0, 1, 2];
    let out = Stand::new(cfg).run_until(
        move |p| p.min_height_of(&members) >= 5 * EPOCH_LEN + 8,
        Duration::from_secs(400),
    );
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert_eq!(
        out.heights[3],
        3 * EPOCH_LEN - 1,
        "the rotated-out node did not park at the last block of epoch 2: {:?}",
        out.heights
    );
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[3]);
    assert_eq!(
        out.artifacts[3].keys().copied().collect::<Vec<_>>(),
        vec![2],
        "the parked node acquired an epoch key it has no path to"
    );
    // It is parked for want of the KEY, not for want of blocks: its upstream
    // plane is being served the whole time. Without this the same height vector
    // would also be produced by a node nobody feeds, and the test would pin the
    // wrong mechanism.
    let u3 = out.upstream[3];
    assert!(
        u3.latest_delivered > 0 && u3.finalized_delivered > 0,
        "the parked node was not being served by the upstream plane at all: {u3:?}"
    );
    assert_eq!(
        u3.rejump_calls, 0,
        "the gate was supposed to be closed, but the node re-jumped"
    );
    eprintln!(
        "(C8) heights={:?} rejumps={:?} ack_drops={} virtual={:?} real={:?}",
        out.heights,
        (0..4)
            .map(|i| out.upstream[i].rejump_calls)
            .collect::<Vec<_>>(),
        out.simulator_ack_drops,
        out.virtual_elapsed,
        out.real_elapsed
    );
}
