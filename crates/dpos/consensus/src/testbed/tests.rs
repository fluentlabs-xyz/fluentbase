//! Stand tests. Every one runs on the deterministic runner without the
//! `external` feature.

use super::{
    fakes::{ElEvent, UpstreamCounters, DPOS_ACTIVATION_BLOCK},
    stand::{
        CertInletCfg, CertInletSource, Committees, Divergence, Outcome, PeerSet, Progress, Role,
        Stand, StandConfig, CHAIN_ID,
    },
};
use crate::beacon::{
    prev_randao_from_seed,
    testing::{decode_artifact, group_public_key},
    Seed,
};
use alloy_primitives::B256;
use commonware_codec::Encode as _;
use fluentbase_bls::{
    beacon::{seed_namespace, verify_seed, GroupPublic},
    fluent_namespace,
};
use fluentbase_staking_reader::{epoch_transition::TransitionOutcome, reader::epoch_at_block};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

/// (A) With no candidate peer the frontier fetch parks and `get_latest` returns
/// `None` after `FRONTIER_FETCH_TIMEOUT` of virtual time.
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
                // No records and no geometry: the fetch only needs to park and time out.
                crate::committee::testing::SchemeCommittee::new(|_| None),
                UpstreamCounters::default(),
                // Nothing is delivered, so nothing can be blocked; the spy only
                // satisfies the signature.
                super::stand::BlockerSpy::default().at(super::stand::BLOCKER_SITE_FRONTIER),
                #[cfg(feature = "dpos-devnet-byzantine")]
                Default::default(),
                #[cfg(feature = "dpos-devnet-byzantine")]
                super::byzantine_roles::ForgeMode::Watch,
                #[cfg(feature = "dpos-devnet-byzantine")]
                None,
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

/// (3) Node 2 derives a different block at height 3: the others reject its
/// proposals and its own executor safety-halts it.
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

/// (3b) Node 3 isolated for eight views: on the catch-up it derives the missed
/// range with guard #2 armed and one derive diverging, so the verdict falls to
/// the backward cross-check at `DIVERGE + K`, not to guard #2. Guard #2 reads the
/// canonical hash before that block's own FCU, sees `None`, and passes.
#[test]
fn guard_two_on_the_catch_up_path_reads_a_pre_fcu_height() {
    const DIVERGE: u64 = 6;
    let k = crate::order_block::K;
    let mut stand = Stand::new(StandConfig::honest(4, 1));
    stand.node(3).role(Role::DivergentResult { at: DIVERGE });
    stand
        .partition(&[0, 1, 2], &[3])
        .after_height(3)
        .for_views(8);
    let out = stand.run_until(
        |p| p.min_height_of(&[0, 1, 2]) >= 24 && p.halted[3],
        Duration::from_secs(180),
    );

    let guard2 = out.logs_containing("guard #2 at");
    let backward = out.logs_containing("result divergence at height");
    eprintln!(
        "(3b) heights={:?} halted={:?} diverged={:?}\n     guard2={:?}\n     backward={:?}\n     \
         parts={:?} timed_out={} virtual={:?} real={:?}",
        out.heights,
        out.halted,
        out.diverged,
        guard2.iter().map(|l| &l.text).collect::<Vec<_>>(),
        backward.iter().map(|l| &l.text).collect::<Vec<_>>(),
        out.partitions,
        out.timed_out,
        out.virtual_elapsed,
        out.real_elapsed
    );
    let around = |node: usize| -> Vec<ElEvent> {
        out.el_events[node]
            .iter()
            .copied()
            .filter(|e| {
                let h = match e {
                    ElEvent::Derived(h, _) | ElEvent::Canonicalized(h, _) => *h,
                };
                h + 2 >= DIVERGE && h <= DIVERGE + k + 1
            })
            .collect()
    };
    eprintln!(
        "(3b) node3 el_events around DIVERGE={DIVERGE}: {:?}\n     node0: {:?}",
        around(3),
        around(0)
    );
    assert!(!out.timed_out, "heights {:?}", out.heights);
    // The two log observations below are the branch discriminator, so a dead
    // capture must fail the test rather than silently skip them.
    assert!(
        out.log_capture_live,
        "log capture is not live — this test's guard-#2 observation would be skipped"
    );

    let part = &out.partitions[0];
    assert!(
        part.heights_at_heal[3] + k <= part.heights_at_heal[0],
        "node 3 was not K behind at the heal: {part:?}"
    );
    assert_eq!(part.heights_at_cut[3], part.heights_at_heal[3]);

    // `Outcome::hashes` substitutes `B256::ZERO` for a height a node never
    // finalized, so these accessors read `el_events` directly.
    let derived_at = |node: usize, h: u64| -> Option<usize> {
        out.el_events[node]
            .iter()
            .position(|e| matches!(e, ElEvent::Derived(x, _) if *x == h))
    };
    let canonicalized_at = |node: usize, h: u64| -> Option<usize> {
        out.el_events[node]
            .iter()
            .position(|e| matches!(e, ElEvent::Canonicalized(x, _) if *x == h))
    };
    let canonical_hash = |node: usize, h: u64| -> Option<B256> {
        out.el_events[node].iter().rev().find_map(|e| match e {
            ElEvent::Canonicalized(x, hash) if *x == h => Some(*hash),
            _ => None,
        })
    };

    for h in DIVERGE..DIVERGE + k {
        let mine = canonical_hash(3, h)
            .unwrap_or_else(|| panic!("node 3 never canonicalized {h}; el_events {:?}", around(3)));
        let theirs = canonical_hash(0, h)
            .unwrap_or_else(|| panic!("node 0 never canonicalized {h}; el_events {:?}", around(0)));
        assert_ne!(mine, theirs, "height {h} did not fork");
    }

    // At DIVERGE the block was executed but not yet canonical: the only state
    // guard #2 could have read.
    let derived = derived_at(3, DIVERGE).expect("node 3 derived DIVERGE");
    let canonicalized = canonicalized_at(3, DIVERGE).expect("node 3 canonicalized DIVERGE");
    assert!(
        derived < canonicalized,
        "node 3 canonicalized DIVERGE at index {canonicalized} but derived it at {derived} — \
         the fake regressed to land-at-derive; el_events {:?}",
        around(3)
    );
    // The catch-up derives before the verdict were all canonicalized, and the
    // height the verdict landed on was not — the halt front-ran its FCU.
    for h in DIVERGE..DIVERGE + k {
        assert!(
            canonicalized_at(3, h).is_some(),
            "node 3 did not canonicalize {h} before halting; el_events {:?}",
            around(3)
        );
    }
    assert!(
        derived_at(3, DIVERGE + k).is_some(),
        "node 3 never derived {} — it parked instead of rendering a verdict; el_events {:?}",
        DIVERGE + k,
        around(3)
    );
    assert!(
        canonicalized_at(3, DIVERGE + k).is_none(),
        "node 3 canonicalized {} — the halt did not front-run that block's FCU; el_events {:?}",
        DIVERGE + k,
        around(3)
    );

    // Guard #2 never fired; the backward cross-check rendered the verdict K
    // heights later.
    assert!(
        guard2.is_empty(),
        "guard #2 rendered the verdict — the fake canonicalized before the FCU: {:?}",
        guard2.iter().map(|l| &l.text).collect::<Vec<_>>()
    );
    assert!(
        backward.iter().any(|l| l
            .text
            .contains(&format!("result divergence at height {}", DIVERGE + k))),
        "no backward cross-check verdict at {}: {:?}",
        DIVERGE + k,
        backward.iter().map(|l| &l.text).collect::<Vec<_>>()
    );
    assert_eq!(
        out.heights[3],
        DIVERGE + k - 1,
        "node 3 stopped at {} — expected the K−1 blocks above DIVERGE to have been \
         FCU'd and finalized-cursor-advanced before the verdict landed",
        out.heights[3]
    );
    assert_eq!(
        out.diverged,
        Some(Divergence::Minority {
            node: 3,
            height: DIVERGE
        })
    );

    assert_eq!(out.halted.len(), 1, "{:?}", out.halted);
    assert_eq!(out.halted[0].0, 3);
    assert!(
        out.halted[0].1.contains("ResultDivergence"),
        "reason: {}",
        out.halted[0].1
    );
    out.assert_lockstep_except(&[3]);
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

/// (4a) `epoch_len = 5`, committee 4 → 3 from epoch 1, every node tracked
/// throughout: the members cross the boundaries and the rotated-out node follows.
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

/// (4b) The same rotation with the peer set narrowed to the committees and every
/// link of the outsider severed: after the cut not one by-height pull is answered
/// and it stands while the members go on.
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
    // Node 3's links fall when its peers track epoch 2, i.e. at
    // `start(2) = 2 * epoch_len`.
    const SEVERED_AT: u64 = 2 * 5;
    assert!(
        out.heights[3] < SEVERED_AT,
        "node 3 executed into epoch 2, which it has no peer to reach: {:?}",
        out.heights
    );
    let u3 = out.upstream[3];
    // A hung `fetch_one` would leave exactly one call; the virtual-clock timeout
    // makes the executor issue another.
    assert!(
        u3.latest_calls >= 2,
        "node 3's first frontier fetch never expired (a hung fetch_one): {u3:?}"
    );
    // Non-vacuity: node 3 had peers before the cut.
    assert!(
        u3.latest_delivered > 0,
        "node 3 was severed before it ever had a peer, so the assertions below prove \
         nothing about severance: {u3:?}"
    );
    assert!(
        u3.latest_calls > u3.latest_delivered,
        "every probe was answered — node 3 never lost its peers: {u3:?}"
    );
    // Past the severance line not one by-height pull is answered.
    let pulls = &out.upstream_served[3];
    assert!(
        pulls.iter().any(|p| p.delivered),
        "node 3 was never served by height at all, so the refusal below is not a \
         severance: {pulls:?}"
    );
    assert!(
        pulls.iter().any(|p| p.height >= SEVERED_AT),
        "node 3 never asked for a height past the severance, so its not being \
         served there is vacuous: {pulls:?}"
    );
    assert!(
        pulls
            .iter()
            .all(|p| !(p.delivered && p.height >= SEVERED_AT)),
        "a pull at or above the severance line came back — something served node 3 \
         across a link the tracked set does not cover: {pulls:?}"
    );
    // Refusals are a tail: once the links are gone nothing is served again.
    let last_served = pulls
        .iter()
        .rposition(|p| p.delivered)
        .expect("served once");
    assert!(
        pulls[last_served + 1..].iter().all(|p| !p.delivered),
        "deliveries resumed after they stopped: {pulls:?}"
    );
    eprintln!(
        "(4b) heights={:?} views={:?} u3={:?} pulls3={:?} virtual={:?} real={:?}",
        out.heights,
        views,
        out.upstream[3],
        out.upstream_served[3],
        out.virtual_elapsed,
        out.real_elapsed
    );
}

/// (4c) The same rotation with the outsider's `FRONTIER_CHANNEL` links left in
/// place: it keeps following through the upstream by-height plane.
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
    eprintln!("(Д4/4c) gap on fcu={:?}", out.head_gap_on_fcu);
    eprintln!(
        "(4c) heights={:?} upstream={:?} virtual={:?} real={:?}",
        out.heights, out.upstream, out.virtual_elapsed, out.real_elapsed
    );
}

/// (4d) The same tracked set with every link left in place: the simulated network
/// delivers regardless of the tracked set, so this does not model production; it
/// pins the heights the stand's hand-modelled severance has to reproduce.
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

/// (5) `[0,1] | [2,3]` cut for five views after height 3: no half can finalize,
/// and one chain returns after the heal.
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
    eprintln!("(Д4/5) gap on fcu={:?}", out.head_gap_on_fcu);
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

/// (5′) The same cut with the upstream plane live on every node: the resolver's
/// same-side fallback answers every probe, so the plane cannot bridge the cut.
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

/// (C1) `first_divergence` on a 2×2 split is a `Tie` naming the height and both
/// hashes, not a minority picked by hash-map order.
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
    let hashes = vec![vec![(1, a)], vec![(1, b)]];
    assert_eq!(
        super::stand::first_divergence(&[1, 1], &hashes),
        Some(Divergence::Tie {
            height: 1,
            hashes: vec![a, b],
        })
    );
}

/// (6) Determinism: seed 1 three times gives the byte-identical `(height, view,
/// leader, digest, hash, σ)` trace on node 0; seed 2 differs.
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

/// (Equivocate) Node 1 equivocates on votes (feature `dpos-devnet-byzantine`) and
/// the honest three keep finalizing one chain.
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

/// Phase 1 finalizes six blocks and returns its checkpoint; phase 2 rebuilds
/// every node over the same storage and the chain continues in lockstep.
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

/// (7′) The same replay with every node on the unprefixed production journal
/// names; records the collision the prefixed run prevents.
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
    // Every node's voter replays the one shared `consensus_epoch_0` journal,
    // finds a vote it did not sign, and panics, taking the runtime down.
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

// The live-plane tests below run `StandConfig::live`: `beacon::build` on every
// node and the DKG on the consensus network's beacon channels. The first key is
// minted for epoch 2 during epoch 1, so the boundary a seed must cross is
// height `2 * EPOCH_LEN`.

const EPOCH_LEN: u64 = 32;

/// The executed hash every node agreed on at `height`; asserts agreement so a
/// replay is not seeded from one node's fork.
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

/// `PK_E` out of an artifact's wire bytes, decoded by the production decoder.
fn pk_of(artifact: &[u8]) -> GroupPublic {
    let (proposal, _) = decode_artifact(artifact).expect("the served artifact decodes");
    *group_public_key(&proposal.group_key)
}

/// The number of dealer logs the agreed artifact pins.
fn dealers_of(artifact: &[u8]) -> usize {
    decode_artifact(artifact).expect("decodes").0.logs.len()
}

/// Every listed node's agreed artifact half (target epoch, pinned dealer logs,
/// `PK_E`, polynomial, confirmations) encodes to the same bytes; the finalization
/// half is a multisig over whichever quorum voters each node collected and is not
/// compared. Returns node `nodes[0]`'s wire bytes.
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

/// The agreed half of an artifact as bytes — what a replay has to reproduce.
fn proposal_bytes(artifact: &[u8]) -> Vec<u8> {
    decode_artifact(artifact)
        .expect("decodes")
        .0
        .encode()
        .to_vec()
}

/// The seed at `height` on node `nodes[0]`, asserted present, equal, scoped to
/// `epoch`, and verifying under `pk` on every listed node; also checks that
/// `prev_randao` agrees.
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

/// The nodes that derived `height` — the ones holding a σ record for it. A node
/// that EL-synced a range holds the block but no σ, so a cross-node σ comparison
/// has to ask who derived.
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

/// (B1) N=4 live DKG: the agreement pins one dealer set and every node adopts the
/// same artifact; from height `2 * EPOCH_LEN` every finalization carries a σ every
/// node reads identically and that verifies under the agreed `PK_2`.
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
    eprintln!("(Д5/B1) bodies={:?}", out.bodies);
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

/// The (B2) schedule: 4 → 3 → 4, with epochs 0–2 and 5 all four and epochs 3–4
/// `[0,1,2]`. A lagging node names successive frontier rungs and ends up holding
/// every one of them.
#[test]
fn the_ladder_names_successive_rungs_and_the_lagging_node_reaches_every_one() {
    let mut cfg = StandConfig::live(4, 1);
    cfg.committees = rotate_four_three_four();
    cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD.min(EPOCH_LEN));
    cfg.marshal_tip_series = true;
    let end = 8 * EPOCH_LEN + 8;
    let out = Stand::new(cfg).run_until(
        move |p| p.min_height_of(&[0, 1, 2]) >= end,
        Duration::from_secs(600),
    );
    let steps = &out.frontier_steps[3];
    let tip = out
        .metric(3, "outer_marshal_finalized_height")
        .expect("node 3 publishes a marshal finalized height") as u64;
    // The raw list is one entry per probe tick; collapse it to distinct rungs
    // with a tick count.
    let mut distinct: Vec<(u64, u64, usize)> = Vec::new();
    for (t, h) in steps {
        match distinct.last_mut() {
            Some(last) if last.0 == *t && last.1 == *h => last.2 += 1,
            _ => distinct.push((*t, *h, 1)),
        }
    }
    eprintln!(
        "(4.2 А.2) node3 heights={:?} marshal_tip={tip} steps(T, last(T+1), ticks)={distinct:?} \
         up[3]={:?}",
        out.heights, out.upstream[3],
    );
    assert!(
        !steps.is_empty(),
        "the probe named no ladder step — `T` never reached it, so this run measures nothing"
    );
    assert!(
        distinct.len() > 1,
        "node 3 named ONE rung for the whole run — `T` never advanced, so there is no ladder \
         here to measure: {distinct:?}"
    );
    assert!(
        distinct.windows(2).all(|w| w[1].1 > w[0].1),
        "the rungs do not strictly increase — the step is not climbing: {distinct:?}"
    );
    let highest = distinct.last().expect("non-empty").1;
    assert!(
        tip >= highest,
        "node 3's marshal tip {tip} never reached the highest rung it named ({highest}): \
         {distinct:?}"
    );

    // For each rung, the first tick it was named and the first tick node 3's own
    // marshal tip reached it.
    let named = &out.frontier_steps_named_series[3];
    let tips = &out.marshal_tip_series[3];
    assert!(
        !tips.is_empty(),
        "no marshal-tip samples — `StandConfig::marshal_tip_series` was not set"
    );
    let mut reach: Vec<(u64, usize, Option<usize>)> = Vec::new();
    for (rung_idx, (_t, height)) in steps.iter().enumerate() {
        let Some(named_at) = named.iter().position(|c| *c > rung_idx) else {
            continue; // named after the last sample — nothing to date it against
        };
        let reached_at = tips
            .iter()
            .enumerate()
            .skip(named_at)
            .find(|(_, t)| **t >= *height)
            .map(|(i, _)| i);
        reach.push((*height, named_at, reached_at));
    }
    // Keep one entry per rung, dated from its first naming.
    reach.dedup_by_key(|(h, _, _)| *h);
    eprintln!("(4.2 Б1.6) rung -> (first named tick, first reached tick) = {reach:?}");

    // `frontier_steps` only says a rung was named; `upstream_served` is this
    // node's own pull log, so a rung there left the node as a real fetch. It
    // cannot separate the ladder step from the marshal's ordinary repair.
    let served = &out.upstream_served[3];
    let rungs: Vec<u64> = distinct.iter().map(|(_, h, _)| *h).collect();
    let rung_pulls: Vec<_> = served
        .iter()
        .filter(|p| rungs.contains(&p.height))
        .collect();
    eprintln!(
        "(4.2 В.B1-03) rungs={rungs:?} pulls={} rung pulls={rung_pulls:?}",
        served.len(),
    );
    for rung in &rungs {
        assert!(
            rung_pulls.iter().any(|p| p.height == *rung),
            "rung {rung} was NAMED but never left as a by-height pull — then the ladder is a \
             log line and not a mechanism: {rung_pulls:?}"
        );
    }
    // At least one named rung was served; the run does not witness that the
    // ladder is what carries the node out, only that a served rung arrives.
    assert!(
        rung_pulls.iter().any(|p| p.delivered),
        "no named rung was ever served on this fixture — then `deliver` ⇒ `store_finalization` \
         ⇒ `Update::Tip` has no live witness at all here: {rung_pulls:?}"
    );

    // A peer with no data drops its response unsent (`plane_upstream.rs`) and
    // never reaches `Consumer::deliver`, so an unanswered rung cannot move the
    // refusal counter or block the peer.
    let unserved = rung_pulls.iter().filter(|p| !p.delivered).count();
    assert!(
        unserved > 0,
        "every named rung was served on this fixture — then it holds no witness that an \
         unanswered rung costs nothing: {rung_pulls:?}"
    );
    assert_eq!(
        out.upstream[3].deliveries_rejected, 0,
        "node 3 refused {} frontier answers while {unserved} of its rungs went unanswered — \
         a peer that simply has no data must never reach `deliver`, let alone be excluded: {:?}",
        out.upstream[3].deliveries_rejected, out.upstream[3]
    );
    for (height, named_at, reached_at) in &reach {
        let reached_at = reached_at.unwrap_or_else(|| {
            panic!(
                "node 3's marshal tip never reached rung {height} named at tick {named_at}: \
                 tips={tips:?}"
            )
        });
        assert!(
            reached_at.saturating_sub(*named_at) <= LADDER_REACH_TICKS,
            "rung {height} was named at tick {named_at} and only reached at tick {reached_at} — \
             more than {LADDER_REACH_TICKS} ticks; naming and reaching look unrelated: {reach:?}"
        );
    }
}

/// How many driver ticks a rung may take between being named by node 3's probe and
/// being reached by node 3's own marshal tip. The bound fails if the climb stops
/// after a rung is named; it is not a latency target.
const LADDER_REACH_TICKS: usize = 800;

/// Where the (C9) fixture cuts node 3 off, and how long for. Rotation alone no
/// longer makes a node fall behind — the epoch key is an artifact it can ask a
/// member for — so the lag is a physical consensus-plane cut inside epoch 2, held
/// until the network is a re-jump gate above it.
const CUT_AT: u64 = 2 * EPOCH_LEN + 4;
/// The height the network reaches before the cut heals — `CUT_AT` plus more than
/// the re-jump gate, and below `epoch_start(4)`.
const HEAL_ABOVE: u64 = 3 * EPOCH_LEN + 12;

fn rotate_four_three_four() -> Committees {
    Committees::Schedule(Arc::new(|epoch, n| {
        Some(match epoch {
            3 | 4 => vec![0, 1, 2],
            _ => (0..n).collect(),
        })
    }))
}

/// (B2) Committee rotation over three minting boundaries with the `dkgQual` bit
/// from the schedule: artifacts for the mints 2, 3 and 5 and none for the carried
/// epoch 4, three distinct keys, and every height's σ under the key the history
/// names for its epoch.
#[test]
fn three_boundaries_with_committee_rotation_keep_dkg_qual_honest() {
    let mut cfg = StandConfig::live(4, 1);
    cfg.committees = rotate_four_three_four();
    cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD.min(EPOCH_LEN));
    let end = 5 * EPOCH_LEN + 8;
    let mut stand = Stand::new(cfg);
    stand
        .partition(&[0, 1, 2], &[3])
        .after_height(CUT_AT)
        .heal_above(HEAL_ABOVE);
    let out = stand.run_until(reached(end), Duration::from_secs(400));
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
        // Only the nodes that derived `h` hold a σ for it; the EL-synced
        // rotated-out node drops out and the floor of 3 keeps the check from
        // degenerating into a self-comparison.
        let derivers = derivers_of(&out, h);
        assert!(
            derivers.len() >= 3,
            "only {} node(s) derived height {h}: {derivers:?}",
            derivers.len()
        );
        let seed = seed_agreed_at(&out, &derivers, h, epoch, pk);
        // The carried epoch's σ is under `PK_3`, not under another mint.
        if epoch == 4 {
            for other in [&pk2, &pk5] {
                assert!(
                    !verify_seed(other, &ns, seed.target_round, &seed.signature),
                    "epoch-4 σ at {h} verifies under a key other than PK_{key_epoch}"
                );
            }
        }
    }
    // Without a re-jump the artifacts could have arrived another way, so pin the
    // route.
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

/// (B3) N=4, node 3 brings up no beacon (`Role::AbsentBeacon`): the three dealers
/// finalize a three-log set and mint `PK_2`, while node 3 parks at the bootstrap
/// boundary with no σ source.
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
    // Node 3's absence has no observation of its own: its artifact map is empty by
    // construction and `beacon::absent` registers the same `dkg_*` families with
    // zero counters. The three-log set and the park observe it.
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
    eprintln!("(Д5/B3) bodies={:?}", out.bodies);
    eprintln!(
        "(B3) heights={:?} dealers={} virtual={:?} real={:?} warns={:?}",
        out.heights,
        dealers_of(artifact),
        out.virtual_elapsed,
        out.real_elapsed,
        out.logs.iter().map(|l| l.text.as_str()).collect::<Vec<_>>()
    );
}

/// Phase 1 stops six blocks into epoch 2; phase 2 rebuilds every node over
/// the same storage and share dirs, and the chain goes on with σ. Advancing past
/// the re-derived range is what proves the share reloaded from disk.
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

    // `cold_start` derives the epoch from the EL marker the execution layer
    // persisted; a genesis anchor instead leaves the replayed nodes in epoch 0.
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
    // Printed, not asserted: on the stable schedule the actor starts a
    // carry-forward, so the count says nothing about the restart.
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
/// byte-identical trace including σ; seed 2 differs.
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
    // The σ field differs too, not only the leaders and digests around it.
    assert_ne!(
        sigma_1, sigma_2,
        "seed 2 reproduced seed 1's σ at height 70"
    );
    eprintln!(
        "(B5) trace bytes={} real={t_a:?}/{t_b:?}/{t_c:?}/{t_d:?}",
        a.len()
    );
}

/// The negative control for the restart test above: the same restart with every
/// share dir wiped — every node is withheld for epoch 2 and the chain parks at the
/// stop height.
#[test]
fn restart_without_the_share_dirs_parks_the_chain_verify_only() {
    let cfg = StandConfig::live(4, 1);
    let (first, checkpoint) =
        Stand::new(cfg.clone()).run_until_recover(reached(70), Duration::from_secs(200));
    assert!(!first.timed_out, "{:?}", first.heights);
    first.assert_lockstep_except(&[]);
    let resume_from = first.heights.iter().copied().max().unwrap();
    std::fs::remove_dir_all(&cfg.share_root).expect("wipe the share root");

    // The same EL marker as the restart test above; this control differs in the share dirs only.
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

/// Node 1 is cut after the restart, so every later block needs node 3's vote and
/// partial.
#[test]
fn an_absentee_restarted_after_the_seal_heals_its_share_and_signs() {
    const CUT_AT: u64 = 20;
    const HEAL_ABOVE: u64 = 2 * EPOCH_LEN + 2;
    const SEAL_2: u64 = 2 * EPOCH_LEN - crate::beacon::testing::DKG_MARGIN_BLOCKS;
    const STOP_1: u64 = 90;
    let cfg = StandConfig::live(4, 1);
    let mut stand = Stand::new(cfg.clone());
    stand
        .partition(&[0, 1, 2], &[3])
        .after_height(CUT_AT)
        .consensus_only()
        .heal_above(HEAL_ABOVE);
    let (first, checkpoint) =
        Stand::run_until_recover(stand, reached(STOP_1), Duration::from_secs(300));
    assert!(!first.timed_out, "phase 1: {:?}", first.heights);
    assert!(first.halted.is_empty(), "{:?}", first.halted);
    let cut = &first.partitions[0];
    assert!(
        !cut.heights_at_cut.is_empty() && cut.heights_at_cut.iter().all(|h| *h < EPOCH_LEN),
        "the cut must land before epoch 2's deal window opens: {cut:?}"
    );
    assert!(
        cut.heights_at_heal
            .iter()
            .copied()
            .max()
            .is_some_and(|h| h >= HEAL_ABOVE),
        "the cut must hold past the seal and the boundary: {cut:?}"
    );
    let all = [0, 1, 2, 3];
    let artifact1 = artifact_on_every_node(&first, &all, 2).to_vec();
    let pinned1: Vec<u8> = decode_artifact(&artifact1)
        .expect("decodes")
        .0
        .logs
        .iter()
        .map(|(idx, _)| *idx)
        .collect();
    let seats = committee_seats(1, 4);
    assert_eq!(
        pinned1.len(),
        3,
        "PK_2 was not minted over exactly the three present dealers: {pinned1:?}"
    );
    assert!(
        !pinned1.contains(&seats[3]),
        "PK_2 was minted over a set that includes the absentee's log: seat {} in {pinned1:?}",
        seats[3]
    );
    for node in [0, 1, 2] {
        assert!(
            pinned1.contains(&seats[node]),
            "node {node}'s seat {} is not pinned in {pinned1:?}",
            seats[node]
        );
    }
    let pk = pk_of(&artifact1);
    seed_agreed_at(&first, &all, 2 * EPOCH_LEN, 2, &pk);
    assert!(
        first.signable[3].contains(&2),
        "node 3 did not key in-process after the heal: {:?}",
        first.signable[3]
    );
    assert!(
        first.heights[3] >= SEAL_2,
        "node 3's clock is not past seal(2) at the checkpoint"
    );

    let node3 = cfg.share_root.join("node3");
    for file in ["beacon-dkgjournal-e2.bin", "beacon-share-e2.bin"] {
        let path = node3.join(file);
        assert!(path.exists(), "node 3 has no {file} to lose");
        std::fs::remove_file(&path).expect("remove node 3's epoch-2 file");
    }

    let resume_from = STOP_1;
    let mut cfg = cfg;
    cfg.resume_from = Some((resume_from, finalized_hash_at(&first, resume_from)));
    let mut stand = Stand::new(cfg);
    let cut_at = resume_from + 12;
    let stop = cut_at + 28;
    stand
        .partition(&[0, 2, 3], &[1])
        .after_height(cut_at)
        .heal_above(u64::MAX);
    let second = stand.replay(
        checkpoint,
        move |p| p.min_height_of(&[0, 2, 3]) >= stop,
        Duration::from_secs(300),
    );
    let cell = second.logs_containing("no ceremony journal at or after the seal deadline");
    eprintln!(
        "(B4″) phase1 heights={:?} real={:?}; phase2 heights={:?} real={:?} cut={:?} \
         signable={:?} ceremony_ok={:?} unrecoverable={:?} cell_lines={} warns={:?}",
        first.heights,
        first.real_elapsed,
        second.heights,
        second.real_elapsed,
        second.partitions.first().map(|p| p.heights_at_cut.clone()),
        second.signable,
        (0..4)
            .map(|i| second.metric(i, "dkg_ceremony_ok_total"))
            .collect::<Vec<_>>(),
        (0..4)
            .map(|i| second.metric(i, "dpos_dkg_share_unrecoverable_total"))
            .collect::<Vec<_>>(),
        cell.len(),
        second
            .logs
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        !cell.is_empty(),
        "the restart never hit the (NoFile, h ≥ seal) cell — the fixture proves nothing"
    );
    assert!(
        !second.timed_out,
        "the chain parked once node 1 was cut: the absentee did not sign — heights {:?} \
         halted {:?} errors {:?}",
        second.heights,
        second.halted,
        second.errors()
    );
    assert!(second.halted.is_empty(), "{:?}", second.halted);
    assert_eq!(second.diverged, None);
    assert!(second.errors().is_empty(), "{:?}", second.errors());
    second.assert_lockstep_except(&[1]);
    let cut2 = &second.partitions[0];
    assert!(
        !cut2.heights_at_cut.is_empty(),
        "node 1 was never cut, so nothing here needed node 3's partial: {cut2:?}"
    );
    assert_eq!(
        second.metric(3, "dkg_ceremony_ok_total"),
        Some(1.0),
        "node 3 adopted a share other than exactly once (the heal)"
    );
    assert_eq!(
        second.metric(3, "dpos_dkg_share_unrecoverable_total"),
        Some(0.0),
        "node 3's heal ended in the terminal"
    );
    let last_epoch = *second.heights.iter().max().expect("heights") / EPOCH_LEN;
    for e in 2..=last_epoch {
        assert!(
            second.signable[3].contains(&e),
            "node 3's share-gate is not Ready for epoch {e}: {:?}",
            second.signable[3]
        );
    }
    let alive = [0, 2, 3];
    let min = alive.iter().map(|&i| second.heights[i]).min().unwrap();
    for h in cut_at + 1..=min {
        seed_agreed_at(&second, &alive, h, h / EPOCH_LEN, &pk);
    }
    assert_eq!(
        proposal_bytes(artifact_on_every_node(&second, &all, 2)),
        proposal_bytes(&artifact1),
        "the epoch-2 agreed proposal changed across the replay"
    );
}

// (C) The boundary walk through the production `EpochTransition` over the fake
// staking state.

/// The boundary list the transition must produce, computed here from the run's
/// executed hashes and `read_height_for(number) = number − result_lag`: the
/// cold-start entry at the anchor, then one entry per boundary block
/// `B = E * epoch_len − 1` carrying `epoch E` and the executed hash of `B − K`.
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

/// (C2) The chain's committee handoff runs through the production
/// `EpochTransition` over `FakeStaking`, reading the committee at a real executed
/// hash; each node's boundary trace is compared against a list this test
/// recomputes.
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
    // The nodes agree on the first four entries, so the comparison is not four
    // independent claims about four different chains.
    for i in 1..4 {
        assert_eq!(
            &out.et_boundaries[i][..4],
            &out.et_boundaries[0][..4],
            "node {i} entered the first four epochs on different state than node 0"
        );
    }
    // Each transition's peer set is a function of chain state alone, so a per-node
    // difference means two nodes read different committees. The mismatch counter
    // compares members, not sizes, which would be vacuous under `PeerSet::AllNodes`.
    assert_eq!(
        out.tracked_mismatches, 0,
        "two nodes' transitions tracked different peer sets for one epoch"
    );
    for i in 0..4 {
        assert!(
            out.peer_sets[i].len() >= 4,
            "node {i} tracked only {} epochs: {:?}",
            out.peer_sets[i].len(),
            out.peer_sets[i]
                .iter()
                .map(|(e, p, sec)| (*e, p.len(), sec.len()))
                .collect::<Vec<_>>()
        );
    }
    // The exemption is bounded by peer-set registrations that reached the network:
    // an aborted engine can have sends in flight, but drops with no registration
    // would mean the exemption covers something else.
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

/// (C3 + C5) An epoch the contract has not committed at the read height is not
/// readable early, and every staking read resolves at a real executed hash of
/// this node's chain.
#[test]
fn a_committee_not_yet_committed_is_not_read_early() {
    let cfg = StandConfig::live(4, 1);
    // Read the geometry off the config rather than restating it, so the epoch
    // bound stays honest if the activation or length moves.
    let epoch_len = cfg.epoch_len;
    let out = Stand::new(cfg).run_until(reached(72), Duration::from_secs(200));
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
        // Non-vacuity: every epoch this run passed through must have been read, so
        // the assertions above are not true over an empty read log.
        let passed = epoch_at_block(
            *out.heights.iter().max().expect("heights"),
            DPOS_ACTIVATION_BLOCK,
            epoch_len,
        )
        .expect("non-zero epoch length");
        for epoch in 0..=passed {
            assert!(
                reads.committed.get(&epoch).copied().unwrap_or(0) >= 1,
                "node {i} never read committee[{epoch}] the run passed through: {reads:?}"
            );
        }
    }
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
    // On a boundary-aligned anchor `cold_start` enters E+1, so pin that the
    // fixture is not on one.
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
        // The stand's feeder records the anchor it passed in; the production claim
        // is the outcome below.
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

/// (C7) Zero committee overlap at a boundary is survivable: each half acquires
/// the epoch key it had no part in minting, and the chain crosses the boundary.
#[test]
fn a_zero_overlap_boundary_is_crossed_by_acquiring_the_other_halfs_key() {
    let mut cfg = StandConfig::live(8, 1);
    cfg.committees = Committees::Schedule(Arc::new(|epoch, _n| {
        Some(if epoch <= 2 {
            vec![0, 1, 2, 3]
        } else {
            vec![4, 5, 6, 7]
        })
    }));
    // The outgoing half runs the production cert-inlet: a node not in
    // `committee[E]` gets its σ through `CertInlet::ingest`
    // (`observe_certificate` / `ensure_key`), so without it the half could hold
    // `PK_3` and still park for want of σ. The incoming half runs its own engine
    // and deliberately gets none.
    cfg.cert_inlet = Some(CertInletCfg {
        nodes: vec![0, 1, 2, 3],
        source: CertInletSource::NextAboveTier,
    });
    let out = Stand::new(cfg).run_until(reached(3 * EPOCH_LEN + 4), Duration::from_secs(400));
    // Printed before the assertions: the height vector cannot say which
    // acquisition failed, the artifact map can.
    eprintln!(
        "(C7) heights={:?} artifacts={:?} halted={:?} pulls(out)={:?} pulls(in)={:?} \
         virtual={:?} real={:?}",
        out.heights,
        out.artifacts
            .iter()
            .map(|a| a.keys().copied().collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        out.halted,
        out.upstream[0],
        out.upstream[4],
        out.virtual_elapsed,
        out.real_elapsed
    );
    assert!(
        !out.timed_out,
        "a zero-overlap boundary still halts the chain: {:?}",
        out.heights
    );
    assert!(
        out.halted.is_empty(),
        "the crossing was reported as a safety fault: {:?}",
        out.halted
    );
    assert_eq!(out.diverged, None);
    // One chain across all eight, not two equally long ones.
    out.assert_lockstep_except(&[]);

    // Both artifacts on every node: neither half minted both, so every entry
    // beyond a node's own mint was acquired from the other half.
    for i in 0..8 {
        assert_eq!(
            out.artifacts[i].keys().copied().collect::<Vec<_>>(),
            vec![2, 3],
            "node {i} does not hold both epochs' artifacts — the acquisition works \
             in at most one direction: {:?}",
            out.artifacts[i].keys().collect::<Vec<_>>()
        );
    }

    // Equal σ at a height makes `prev_randao` = H(σ) equal; the comparison uses
    // the σ the executors actually derived from.
    let min = *out.heights.iter().min().expect("eight nodes");
    assert!(
        min >= 3 * EPOCH_LEN + 4,
        "premise: every node is past the boundary, or the σ comparison below \
         covers only the pre-boundary epochs: {:?}",
        out.heights
    );
    let mut beacon_active = 0usize;
    for h in 1..=min {
        let first = out.seeds[0].get(&h).cloned().flatten();
        if first.is_some() {
            beacon_active += 1;
        }
        for i in 1..8 {
            assert_eq!(
                out.seeds[i].get(&h).cloned().flatten(),
                first,
                "node {i} derived height {h} from a different σ than node 0 — \
                 prev_randao is H(σ), so this is a randomness fork"
            );
        }
    }
    // Non-vacuity: an all-`None` σ map would satisfy the loop trivially.
    assert!(
        beacon_active > EPOCH_LEN as usize,
        "premise: more than one epoch's worth of heights derived from a real σ \
         ({beacon_active} of {min}) — otherwise the equality above is vacuous"
    );
    eprintln!(
        "(C7) boundaries(out)={:?} boundaries(in)={:?}",
        out.et_boundaries[0]
            .iter()
            .map(|b| (b.epoch, b.block_number))
            .collect::<Vec<_>>(),
        out.et_boundaries[4]
            .iter()
            .map(|b| (b.epoch, b.block_number))
            .collect::<Vec<_>>(),
    );
}

/// (C8) A rotated-out node comes back with the jump gate closed, because the
/// missing epoch key is now acquired: it holds both artifacts and never re-jumps.
#[test]
fn a_rotated_out_node_follows_the_committee_once_it_acquires_the_epoch_key() {
    let mut cfg = StandConfig::live(4, 1);
    cfg.committees = rotate_four_three_four();
    assert_eq!(
        cfg.re_jump_threshold, None,
        "the gate must stay closed here: the exit under test is the KEY, not the jump"
    );
    let members = [0, 1, 2];
    let out = Stand::new(cfg).run_until(
        move |p| p.min_height_of(&members) >= 5 * EPOCH_LEN + 8,
        Duration::from_secs(400),
    );
    eprintln!(
        "(C8) heights={:?} artifacts3={:?} rejumps={:?} ack_drops={} virtual={:?} real={:?}",
        out.heights,
        out.artifacts[3].keys().copied().collect::<Vec<_>>(),
        (0..4)
            .map(|i| out.upstream[i].rejump_calls)
            .collect::<Vec<_>>(),
        out.simulator_ack_drops,
        out.virtual_elapsed,
        out.real_elapsed
    );
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert!(
        out.heights[3] > 3 * EPOCH_LEN - 1,
        "the rotated-out node is still parked at the last block of epoch 2, so the \
         live-epoch acquisition did not reach it: {:?}",
        out.heights
    );
    out.assert_lockstep_except(&[]);
    assert!(
        out.artifacts[3].contains_key(&3),
        "the rotated-out node followed WITHOUT epoch 3's artifact, which would mean \
         its certificates were admitted on the multisig half alone: {:?}",
        out.artifacts[3].keys().collect::<Vec<_>>()
    );
    // It is fed by the upstream plane and did not climb out by jumping: the exit
    // under test is the key.
    let u3 = out.upstream[3];
    assert!(
        u3.latest_delivered > 0 && u3.finalized_delivered > 0,
        "the node was not being served by the upstream plane at all: {u3:?}"
    );
    assert_eq!(
        u3.rejump_calls, 0,
        "the gate was supposed to be closed, but the node re-jumped — the recovery \
         under test would then be the jump's and not the key's"
    );
}

/// (C9) The stand's steady-state re-jump runs the production `jump_to_target`:
/// every call lands on a pair out of this node's own marshal archive, its landing
/// hash is the consumed certificate's `result`, and the honest three executed it.
#[test]
fn the_rejump_runs_the_production_jump_and_lands_on_its_own_archive_pair() {
    let mut cfg = StandConfig::live(4, 1);
    cfg.committees = rotate_four_three_four();
    cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD.min(EPOCH_LEN));
    let end = 5 * EPOCH_LEN + 8;
    let mut stand = Stand::new(cfg);
    stand
        .partition(&[0, 1, 2], &[3])
        .after_height(CUT_AT)
        .heal_above(HEAL_ABOVE);
    let out = stand.run_until(reached(end), Duration::from_secs(400));
    assert!(
        !out.timed_out,
        "heights {:?} halted {:?} errors {:?}",
        out.heights,
        out.halted,
        out.errors()
    );
    assert!(out.halted.is_empty(), "{:?}", out.halted);

    // Without all-landed the rest is worthless: `ReJump::rotate` is `None`, so a
    // refused jump rotates nothing and a chain where every jump failed produces
    // the same call count as this one.
    let calls = &out.jump_calls[3];
    assert!(
        calls.iter().all(|c| c.outcome == "Landed"),
        "a re-jump did not land: {calls:?}"
    );
    assert_eq!(
        out.upstream[3].rejump_calls,
        calls.len() as u64,
        "the call log and the counter disagree: {:?} vs {calls:?}",
        out.upstream[3]
    );

    // Each landing hash is the `result` of the certificate the call consumed, not
    // a read-back of the chain the landing itself just wrote.
    assert_eq!(
        calls.iter().map(|c| c.landed).collect::<Vec<_>>(),
        vec![
            Some((105, out.hashes[3][104].1)),
            Some((136, out.hashes[3][135].1))
        ],
        "the production jump did not land where the model landed: {calls:?}"
    );
    for call in calls {
        let (landing, hash) = call.landed.expect("asserted Landed above");
        let (tip, result) = call.consumed.expect("a landing consumed a certificate");
        assert_eq!(
            hash, result,
            "the landing hash is not the `result` of the cert the jump consumed (tip {tip}): {call:?}"
        );
        assert_eq!(
            landing,
            tip - crate::order_block::K,
            "the landing is not the consumed tip's `tip - K`: {call:?}"
        );
    }

    // The branch it landed on is the one the honest three executed — a jump onto a
    // fork would satisfy the check above just as well.
    assert_eq!(out.diverged, None, "the stand forked");
    for call in calls {
        let (landing, hash) = call.landed.expect("asserted Landed above");
        for i in [0, 1, 2] {
            assert_eq!(
                out.hashes[i][(landing - 1) as usize],
                (landing, hash),
                "node {i} executed a different hash at the landing {landing}"
            );
        }
    }

    // `jump_to_target` takes no committee source, so the absence of a post-sync
    // authentication stage is a compile-time fact.

    // Nobody else jumped, so no observation above comes from a node that was never
    // behind.
    for i in [0, 1, 2] {
        assert_eq!(out.upstream[i].rejump_calls, 0, "node {i} re-jumped");
        assert!(out.jump_calls[i].is_empty(), "node {i} called the jump");
    }
    eprintln!(
        "(C9) calls={calls:?} virtual={:?} real={:?}",
        out.virtual_elapsed, out.real_elapsed
    );
}

/// A node three epochs behind registers the schemes of the epochs it
/// is behind and spawns an engine for none of them; the live epoch is read off its
/// own verified tip. Epoch 4 is one it is a member of and can read, so the absent
/// signer half there is not the module refusing the read.
#[test]
fn a_node_three_epochs_behind_registers_the_schemes_and_spawns_no_engine() {
    use commonware_cryptography::certificate::Scheme as _;

    let mut cfg = StandConfig::live(4, 1);
    // Node 3 leaves the committee for epoch 3 only: long enough to miss `PK_3` and
    // park, short enough to stay a member of `epoch(fin) + 2`.
    cfg.committees = Committees::Schedule(Arc::new(|epoch, n| {
        Some(match epoch {
            3 => vec![0, 1, 2],
            _ => (0..n).collect(),
        })
    }));
    assert_eq!(
        cfg.re_jump_threshold, None,
        "the re-jump gate must stay closed: the lag IS the fixture"
    );
    cfg.marshal_tip_series = true;
    let members = [0, 1, 2];
    let mut stand = Stand::new(cfg);
    // A non-member can ask for the epoch key, so non-membership does not hold a
    // node back; the lag is a consensus-plane cut inside epoch 2 that never heals,
    // and the frontier probe still carries the marshal tip to the two-epoch ceiling
    // while execution stays at `last(2)`.
    stand
        .partition(&[0, 1, 2], &[3])
        .after_height(2 * EPOCH_LEN + 4)
        .consensus_only()
        .for_views(4096);
    let out = stand.run_until(
        move |p| p.min_height_of(&members) >= 6 * EPOCH_LEN,
        Duration::from_secs(400),
    );
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    // The cut is the fixture, so no cut, no lag.
    assert!(
        !out.partitions[0].heights_at_cut.is_empty(),
        "the consensus-plane cut never fired, so nothing held node 3 back: {:?}",
        out.partitions[0]
    );

    // Premise: node 3's execution parked at the last block of epoch 2 while the
    // network ran on.
    assert_eq!(
        out.heights[3],
        3 * EPOCH_LEN - 1,
        "the lagging node did not park at last(2): {:?}",
        out.heights
    );
    assert!(
        out.heights[0] >= 6 * EPOCH_LEN,
        "the network did not run three epochs ahead of it: {:?}",
        out.heights
    );

    // Its marshal kept verifying past its own execution up to the two-epoch
    // ceiling; `tip == last(4)` makes the live epoch 5.
    let ceiling = 5 * EPOCH_LEN - 1; // last(4) = 159
    let tip = *out.marshal_tip_series[3]
        .last()
        .expect("`marshal_tip_series` was enabled");
    assert_eq!(
        tip, ceiling,
        "node 3's marshal tip is not at the two-epoch ceiling, so the live epoch under test \
         is not the one this test names: {:?}",
        out.marshal_tip_series[3]
    );
    let live = 5u64;
    assert_eq!(
        epoch_at_block(out.heights[3], DPOS_ACTIVATION_BLOCK, EPOCH_LEN),
        Some(2),
        "premise: the node executes in epoch 2 while its verified tip says {live}"
    );

    let module = &out.committees[3];

    // Every epoch between the park and the ceiling holds a scheme.
    for e in 3..=4 {
        assert!(
            module.scheme(e).is_some(),
            "node 3 holds no scheme for epoch {e}, so refusing an engine there proves nothing"
        );
    }

    // Premise: this node is a member of epoch 4 and can read its committee, so the
    // absent signer half there is not the module refusing the read.
    assert!(
        out.committee_records[3].get(&4).is_some_and(|r| r.is_ok()),
        "premise: node 3 must be able to read committee[4]: {:?}",
        out.committee_records[3].get(&4)
    );

    // The live epoch is above this node's read window, so it holds nothing there;
    // that is why the non-vacuity check above uses epoch 4.
    assert!(
        module.scheme(live).is_none(),
        "the live epoch is outside this node's read window and must hold nothing"
    );

    // Control: it signed exactly the epochs its own verified tip was inside.
    let signer_epochs: Vec<u64> = (0..=live)
        .filter(|e| module.scheme(*e).is_some_and(|s| s.me().is_some()))
        .collect();
    assert_eq!(
        signer_epochs,
        vec![0, 1, 2],
        "node 3 signed something other than the epochs its verified tip was inside: \
         {signer_epochs:?}"
    );
    eprintln!(
        "(4.2В) heights={:?} tip3={tip} signer3={signer_epochs:?} verifier3={:?}",
        out.heights, out.committee_verifier_epochs[3]
    );
}

/// The liveness gate on its own: a node catching up after a partition
/// crosses boundaries the network has left, taking a verify-only scheme at each
/// while it is a member, holds a usable share and holds the boundary block.
#[test]
fn a_catching_up_member_takes_verify_only_at_every_boundary_below_its_own_tip() {
    use commonware_cryptography::certificate::Scheme as _;

    // Chosen so the staking transition's single-park invariant holds
    // (`interval > MAX_PENDING_ACKS + K`), so the catch-up burst cannot park two
    // boundaries at once.
    const LEN: u64 = 24;
    let mut cfg = StandConfig::honest(4, 1);
    cfg.epoch_len = LEN;
    cfg.marshal_tip_series = true;
    let mut stand = Stand::new(cfg);
    // Cut inside epoch 0 for longer than a whole epoch, so node 3's catch-up
    // crosses a boundary the network left long before.
    stand
        .partition(&[0, 1, 2], &[3])
        .after_height(10)
        .for_views(24);
    let out = stand.run_until(reached(3 * LEN), Duration::from_secs(400));

    let signer_epochs = |node: usize| -> Vec<u64> {
        (0..8)
            .filter(|e| {
                out.committees[node]
                    .scheme(*e)
                    .is_some_and(|s| s.me().is_some())
            })
            .collect::<Vec<u64>>()
    };
    let held = |node: usize| -> Vec<u64> {
        (0..8)
            .filter(|e| out.committees[node].scheme(*e).is_some())
            .collect::<Vec<u64>>()
    };
    eprintln!(
        "(4.2В-gate) heights={:?} timed_out={} parts={:?}\n            held3={:?} signer3={:?} \
         signer0={:?} tip3_last={:?}",
        out.heights,
        out.timed_out,
        out.partitions,
        held(3),
        signer_epochs(3),
        signer_epochs(0),
        out.marshal_tip_series[3].last(),
    );
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert_eq!(out.diverged, None);

    // Premise: the cut left node 3 more than a whole epoch behind.
    let cut = out
        .partitions
        .first()
        .expect("the partition was configured");
    let lag = cut.heights_at_heal[0].saturating_sub(cut.heights_at_heal[3]);
    assert!(
        lag > LEN,
        "node 3 was not even one epoch behind at the heal, so nothing below its tip was \
         crossed: {:?}",
        cut.heights_at_heal
    );

    // Premise: it caught up, so the missed boundaries were crossed.
    assert_eq!(
        out.heights[3], out.heights[0],
        "node 3 did not catch up: {:?}",
        out.heights
    );

    // The epochs node 3 crossed while its own verified tip was already past them,
    // excluding the one it caught up into.
    let behind_at_heal = cut.heights_at_heal[3] / LEN;
    let ahead_at_heal = cut.heights_at_heal[0] / LEN;
    let crossed: Vec<u64> = (behind_at_heal + 1..ahead_at_heal).collect();
    assert!(
        !crossed.is_empty(),
        "no epoch was crossed below the tip: {:?}",
        cut.heights_at_heal
    );
    let signer3 = signer_epochs(3);
    let held3 = held(3);
    for e in &crossed {
        assert!(
            held3.contains(e),
            "node 3 holds no scheme for the crossed epoch {e}, so refusing an engine there \
             proves nothing: held={held3:?}"
        );
        assert!(
            !signer3.contains(e),
            "node 3 took the SIGNER half for epoch {e}, which its own verified tip had already \
             left — the liveness gate is the only thing that could have refused it here: \
             signer3={signer3:?}"
        );
        for control in [0usize, 1, 2] {
            assert!(
                signer_epochs(control).contains(e),
                "control node {control} is not a signer for epoch {e} either, so (3) does not \
                 separate the gate from the fixture: {:?}",
                signer_epochs(control)
            );
        }
    }

    // Control on the same node: it signs the epoch it caught up in.
    assert!(
        signer3.iter().any(|e| *e >= ahead_at_heal),
        "node 3 never re-promoted after the catch-up, so (3) is a node that stopped signing \
         and not a per-epoch gate: signer3={signer3:?}"
    );
}

/// A registry-tier neighbour's dealing is refused at every member's
/// pre-decode gate as `secondary`, exactly as many times as it was sent, and the
/// ceremony still completes.
#[test]
fn a_registry_tier_neighbours_dealing_is_refused_at_the_gate_and_the_key_still_mints() {
    use metrics_util::debugging::DebuggingRecorder;

    const LEN: u64 = EPOCH_LEN;
    let members = [0usize, 1, 2, 3];
    let end = 3 * LEN + 8;
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let out = metrics::with_local_recorder(&recorder, || {
        let mut cfg = StandConfig::live(5, 1);
        cfg.committees = Committees::Schedule(Arc::new(|_epoch, _n| Some(vec![0, 1, 2, 3])));
        cfg.metrics_snapshotter = Some(snap.clone());
        let mut stand = Stand::new(cfg);
        stand.node(4).role(Role::StrayDealer);
        stand.run_until(
            move |p| p.min_height_of(&members) >= end,
            Duration::from_secs(300),
        )
    });
    let drained = &out.metrics_before_collect;
    // The family is shared by every gated channel, so the sum must filter on
    // channel as well as reason.
    let beacon = |reason: &str| beacon_refusals(drained, reason);
    let (secondary, untracked, no_seat) =
        (beacon("secondary"), beacon("untracked"), beacon("no_seat"));
    eprintln!(
        "(5.3-В) heights={:?} stray_sends={:?} beacon refusals: secondary={secondary} \
         untracked={untracked} no_seat={no_seat} epoch={} confirm_window={} virtual={:?} real={:?}",
        out.heights,
        out.stray_dealer_sends,
        beacon("epoch"),
        beacon("confirm_window"),
        out.virtual_elapsed,
        out.real_elapsed
    );
    if out.timed_out {
        for line in &out.logs {
            eprintln!("(5.3-В LOG) {:?} {}", line.level, line.text);
        }
    }
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert_eq!(out.diverged, None);
    assert!(out.errors().is_empty(), "{:?}", out.errors());

    let sent = out.stray_dealer_sends[4];
    assert!(sent > 0, "the stray dealer put nothing on the wire");
    assert!(
        out.stray_dealer_sends[..4].iter().all(|s| *s == 0),
        "{:?}",
        out.stray_dealer_sends
    );

    // Premise: every member's transition tracks node 4 as secondary and never as
    // primary, which is what makes the gate's refusal `secondary`.
    let stray_key = {
        use commonware_cryptography::Signer as _;
        super::stand::keys(1, 5).0[4].public_key()
    };
    for &i in &members {
        assert!(
            !out.peer_sets[i].is_empty(),
            "node {i} tracked nothing — the premise below is about no registration"
        );
        for (epoch, primary, secondary) in &out.peer_sets[i] {
            assert!(
                secondary.contains(&stray_key) && !primary.contains(&stray_key),
                "node {i}'s registration for epoch {epoch} does not carry node 4 as \
                 secondary-only: primary={} secondary={}",
                primary.contains(&stray_key),
                secondary.contains(&stray_key)
            );
        }
    }

    // Every frame the stray sent was refused as `secondary`, and nothing else
    // was.
    assert_eq!(
        (secondary, untracked),
        (sent, 0),
        "the gate refused {secondary} frames as `secondary` and {untracked} as `untracked` \
         against {sent} sent by the stray — the gate is not the one sender classification \
         on the channel"
    );

    assert_eq!(no_seat, 0, "a stray frame reached a consumer");

    let artifact2 = artifact_on_every_node(&out, &members, 2);
    let pk2 = pk_of(artifact2);
    seed_agreed_at(&out, &members, 2 * LEN, 2, &pk2);
    assert_eq!(
        dealers_of(artifact2),
        4,
        "a dealer is missing from epoch 2's artifact"
    );

    out.assert_lockstep_except(&[]);
}

/// The beacon channel's refusals under `reason` in a drained recorder; both
/// channel and reason labels, because the family is shared by every gated channel.
fn beacon_refusals(drained: &[super::stand::CounterSample], reason: &str) -> u64 {
    super::stand::counter_where(
        drained,
        crate::dpos::INGRESS_DROPPED_TOTAL,
        &[
            ("channel", crate::beacon::testing::BEACON_CHANNEL_LABEL),
            ("reason", reason),
        ],
    )
}

/// A committee member the chain has tombstoned is refused at every other
/// member's pre-decode gate as `untracked`, and the ceremony completes without it.
#[test]
fn a_tombstoned_members_dealing_is_refused_at_the_gate_as_untracked_and_the_key_still_mints() {
    use metrics_util::debugging::DebuggingRecorder;

    const LEN: u64 = EPOCH_LEN;
    const TOMBSTONED: usize = 4;
    const TOMBSTONE_FROM: u64 = 4;
    let members = [0usize, 1, 2, 3];
    let end = 3 * LEN + 8;
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let out = metrics::with_local_recorder(&recorder, || {
        let mut cfg = StandConfig::live(5, 1);
        cfg.tombstoned = vec![(TOMBSTONED, TOMBSTONE_FROM)];
        cfg.metrics_snapshotter = Some(snap.clone());
        Stand::new(cfg).run_until(
            move |p| p.min_height_of(&members) >= end,
            Duration::from_secs(400),
        )
    });
    let drained = &out.metrics_before_collect;
    let beacon = |reason: &str| beacon_refusals(drained, reason);
    let (untracked, secondary, no_seat) =
        (beacon("untracked"), beacon("secondary"), beacon("no_seat"));
    eprintln!(
        "(5.3-В/tombstone) heights={:?} observed={:?} beacon refusals: untracked={untracked} \
         secondary={secondary} no_seat={no_seat} epoch={} confirm_window={} \
         proposals_refused_to_bind={} virtual={:?} real={:?}",
        out.heights,
        out.tombstones_observed
            .iter()
            .map(|seen| seen.iter().map(|(h, _)| *h).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        beacon("epoch"),
        beacon("confirm_window"),
        // The same set also feeds the refuse-to-bind gate; this diagnostic counts
        // that consumer.
        super::stand::counter_of(
            drained,
            "dpos_marker_reject_total",
            Some(("reason", "tombstoned_leader"))
        ),
        out.virtual_elapsed,
        out.real_elapsed
    );
    if out.timed_out {
        for line in &out.logs {
            eprintln!("(5.3-В/tombstone LOG) {:?} {}", line.level, line.text);
        }
    }
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert_eq!(out.diverged, None);
    assert!(out.errors().is_empty(), "{:?}", out.errors());

    // Premise: the verdict reached every member's set before the ceremony.
    let tombstoned_key = {
        use commonware_cryptography::Signer as _;
        super::stand::keys(1, 5).0[TOMBSTONED].public_key()
    };
    for &i in &members {
        let seen_at = out.tombstones_observed[i]
            .iter()
            .find(|(_, peer)| *peer == tombstoned_key)
            .map(|(h, _)| *h);
        assert!(
            seen_at.is_some_and(|h| h < LEN),
            "node {i} observed node {TOMBSTONED}'s tombstone at {seen_at:?}, not before the \
             epoch-2 ceremony dealt at {LEN}: {:?}",
            out.tombstones_observed[i]
        );
        assert!(
            out.tombstones_observed[i]
                .iter()
                .all(|(_, peer)| *peer == tombstoned_key),
            "node {i} observed a tombstone nobody was given: {:?}",
            out.tombstones_observed[i]
        );
    }

    // Refused as `untracked`, the tombstone's classification, and nothing reached
    // a consumer.
    assert!(
        untracked > 0,
        "no beacon frame was refused as `untracked`: the tombstoned member's dealing got \
         through every gate"
    );
    assert_eq!(secondary, 0, "a committee member cannot be `secondary`");
    assert_eq!(no_seat, 0, "a tombstoned member's frame reached a consumer");

    let artifact2 = artifact_on_every_node(&out, &members, 2);
    let pk2 = pk_of(artifact2);
    seed_agreed_at(&out, &members, 2 * LEN, 2, &pk2);
    let pinned: Vec<u8> = decode_artifact(artifact2)
        .expect("decodes")
        .0
        .logs
        .iter()
        .map(|(idx, _)| *idx)
        .collect();
    let seat_of_tombstoned = committee_seats(1, 5)[TOMBSTONED];
    assert_eq!(pinned.len(), 4, "epoch 2's artifact pins {pinned:?}");
    assert!(
        !pinned.contains(&seat_of_tombstoned),
        "the tombstoned member's seat {seat_of_tombstoned} is pinned in epoch 2's artifact \
         {pinned:?} — a member took its dealing in"
    );

    out.assert_lockstep_except(&[]);
}

/// `committee[E]` is peer-key ascending, so a node's seat is the position of its
/// peer key in the sorted set; derived from the stand's key schedule.
fn committee_seats(seed: u64, n: usize) -> Vec<u8> {
    use commonware_cryptography::Signer as _;
    let (peers, _) = super::stand::keys(seed, n);
    let by_node: Vec<_> = peers.iter().map(|p| p.public_key()).collect();
    let mut sorted = by_node.clone();
    sorted.sort();
    by_node
        .iter()
        .map(|pk| u8::try_from(sorted.iter().position(|s| s == pk).expect("member")).expect("u8"))
        .collect()
}

// The byzantine beacon and certificate roles: each test asserts its wrapper
// actually tampered before asserting a reaction.

/// Node 1 deals twice: node 0 receives a second, independently dealt and
/// validly signed log of the same dealer over the same `Info`. The victim
/// refetches the pinned log by hash, records the equivocation, and keeps its
/// share.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn a_two_log_dealers_victim_refetches_the_pinned_log_and_keeps_its_share() {
    let mut stand = Stand::new(StandConfig::live(4, 1));
    stand.node(1).role(Role::TwoReveals {
        withhold_partials: false,
    });
    let out = stand.run_until(reached(72), Duration::from_secs(200));

    let byz = &out.byz[1];
    assert!(
        byz.reveals_swapped >= 1,
        "the two-reveal wrapper swapped nothing: {byz:?}"
    );
    assert_eq!(byz.reveals_seen, byz.reveals_swapped, "{byz:?}");
    assert!(
        byz.log1_hash.is_some() && byz.log1_hash != byz.log2_hash,
        "the two logs are the same bytes: {byz:?}"
    );
    assert!(
        byz.both_logs_check,
        "a log the receiver's own `check` would drop is not an equivocation: {byz:?}"
    );
    // The victim's own `ShareConfirm` names the forged hash at the dealer's seat;
    // only `record_checked_log` writes another member's hash, so this observes the
    // split rather than inferring it from the share it ends up with.
    let seat = committee_seats(1, 4);
    let dealer_seat = seat[1];
    let claimed = |node: usize| -> Option<B256> {
        out.byz[node]
            .confirms_sent
            .iter()
            .rfind(|(epoch, _, _)| *epoch == 2)
            .and_then(|(_, _, set)| set.iter().find(|(i, _)| *i == dealer_seat))
            .map(|(_, h)| *h)
    };
    assert_eq!(
        claimed(0),
        byz.log2_hash,
        "the victim's own confirmation does not name the FORGED log at the dealer's seat \
         {dealer_seat} — the second log did not reach `record_checked_log` (confirms {:?})",
        out.byz[0].confirms_sent
    );
    for i in [2, 3] {
        assert_eq!(
            claimed(i),
            byz.log1_hash,
            "node {i} does not name the ORIGINAL log at the dealer's seat {dealer_seat} — the \
             split was not addressed (confirms {:?})",
            out.byz[i].confirms_sent
        );
    }

    // The two branches are mutually exclusive by construction.
    let victim_minted = out.metric(0, "dkg_ceremony_ok_total") == Some(1.0);
    let victim_demoted = out
        .metric(0, "epoch_engine_demoted_no_polynomial_total")
        .unwrap_or(0.0)
        >= 1.0;
    assert_ne!(
        victim_minted,
        victim_demoted,
        "the victim both minted and was demoted, or neither — neither branch of R-002 applies: \
         ok={:?} demote={:?}",
        out.metric(0, "dkg_ceremony_ok_total"),
        out.metric(0, "epoch_engine_demoted_no_polynomial_total")
    );
    eprintln!(
        "(R-002/a) branch = {} | heights={:?} log1={:?} log2={:?} equivocations={:?} virtual={:?} real={:?}",
        if victim_demoted {
            "(b) REPRODUCED — the victim holds no share"
        } else {
            "(a) the victim refetched the pinned log by hash and kept its share"
        },
        out.heights,
        byz.log1_hash,
        byz.log2_hash,
        (0..4)
            .map(|i| out.metric(i, "dpos_dkg_dealer_equivocation_total"))
            .collect::<Vec<_>>(),
        out.virtual_elapsed,
        out.real_elapsed
    );
    assert!(
        victim_minted,
        "branch (b) was observed: the victim holds no share for the epoch — the pinned body \
         was not refetched by hash and R-002's split is back (metrics: ok={:?}, demote={:?})",
        out.metric(0, "dkg_ceremony_ok_total"),
        out.metric(0, "epoch_engine_demoted_no_polynomial_total")
    );

    // Only the victim ever holds both bodies, so only it can prove the
    // equivocation.
    for i in 0..4 {
        assert_eq!(
            out.metric(i, "dpos_dkg_dealer_equivocation_total"),
            Some(if i == 0 { 1.0 } else { 0.0 }),
            "node {i}: the equivocation is proven exactly where both bodies met"
        );
    }
    let warned = out.logs_containing("dealer signed TWO distinct valid logs");
    assert_eq!(
        warned.len(),
        1,
        "one WARN line per (epoch, dealer), on the one node that saw both: {:?}",
        warned.iter().map(|l| l.text.as_str()).collect::<Vec<_>>()
    );
    let (h1, h2) = (
        byz.log1_hash.expect("witnessed above"),
        byz.log2_hash.expect("witnessed above"),
    );
    assert!(
        warned[0].text.contains(&format!("first={h2} "))
            && warned[0].text.contains(&format!("second={h1} "))
            && warned[0].text.contains("evidence=\"journaled\""),
        "the WARN line does not carry the pair in the order the victim met it: {}",
        warned[0].text
    );

    // The load-bearing lines are above; the rest also hold on an honest run.
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    assert_eq!(out.diverged, None);
    out.assert_lockstep_except(&[]);
    for i in 0..4 {
        assert_eq!(
            out.metric(i, "dkg_ceremony_ok_total"),
            Some(1.0),
            "node {i} must have minted — the split is healed by the refetch, not survived"
        );
        assert_eq!(
            out.metric(i, "epoch_engine_demoted_no_polynomial_total"),
            Some(0.0),
            "node {i} must keep its share"
        );
    }
    // The seed of every epoch-2 height is agreed on all four nodes, the victim
    // included.
    let pk = pk_of(artifact_on_every_node(&out, &[0, 1, 2, 3], 2));
    for h in 2 * EPOCH_LEN..=*out.heights.iter().min().unwrap() {
        seed_agreed_at(&out, &[0, 1, 2, 3], h, 2, &pk);
    }
}

/// The same two-log dealer also withholds its seed partial:
/// with the victim's share refetched, the honest signers are exactly quorum and
/// the withholding dealer is one silent node within `f`.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn a_two_log_dealer_that_also_withholds_its_partial_is_one_silent_node_within_f() {
    let mut stand = Stand::new(StandConfig::live(4, 1));
    stand.node(1).role(Role::TwoReveals {
        withhold_partials: true,
    });
    let out = stand.run_until(reached(72), Duration::from_secs(200));

    let byz = &out.byz[1];
    assert!(byz.reveals_swapped >= 1, "{byz:?}");
    assert_eq!(byz.reveals_seen, byz.reveals_swapped, "{byz:?}");
    assert!(
        byz.log1_hash.is_some() && byz.log1_hash != byz.log2_hash,
        "{byz:?}"
    );
    assert!(byz.both_logs_check, "{byz:?}");
    assert!(
        byz.schemes_withheld >= 1,
        "no signer scheme was rebuilt over the verify-only oracle: {byz:?}"
    );
    assert_eq!(
        byz.withhold_probe,
        Some((true, false)),
        "the withholding did not take effect: the honest scheme must sign a probe subject of \
         the epoch and the rebuilt one must not ({byz:?})"
    );

    let crossed = out.heights.iter().any(|h| *h >= 2 * EPOCH_LEN);
    eprintln!(
        "(R-002/b) branch = {} | heights={:?} virtual={:?} real={:?}",
        if crossed {
            "(b2) three honest signers meet quorum(4) — the withholding node is one silent node"
        } else {
            "(b1) REPRODUCED — the chain stops at the bootstrap boundary"
        },
        out.heights,
        out.virtual_elapsed,
        out.real_elapsed
    );
    assert!(
        crossed,
        "branch (b1) was observed: every node parked below {} — the victim is shareless again \
         and one dealer disabled two nodes (heights {:?}, victim ok={:?})",
        2 * EPOCH_LEN,
        out.heights,
        out.metric(0, "dkg_ceremony_ok_total")
    );

    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    assert_eq!(out.diverged, None);
    out.assert_lockstep_except(&[]);
    seedless_on_every_node(&out, &[0, 1, 2, 3], 1..2 * EPOCH_LEN);
    // The victim minted for the same reason as above, and so did everyone else:
    // one partial is withheld and three remain.
    for i in 0..4 {
        assert_eq!(
            out.metric(i, "dkg_ceremony_ok_total"),
            Some(1.0),
            "node {i}"
        );
    }
    for i in 0..4 {
        assert_eq!(
            out.metric(i, "dpos_dkg_dealer_equivocation_total"),
            Some(if i == 0 { 1.0 } else { 0.0 }),
            "node {i}: the equivocation is proven exactly where both bodies met — on the \
             victim's refetch, and nowhere else"
        );
    }
    let pk = pk_of(artifact_on_every_node(&out, &[0, 1, 2, 3], 2));
    for h in 2 * EPOCH_LEN..=*out.heights.iter().min().unwrap() {
        seed_agreed_at(&out, &[0, 1, 2, 3], h, 2, &pk);
    }
}

/// The schedule of the forged-σ tests: `[0, 1, 2]` is the committee of every epoch, so nodes 3
/// and 4 are pure followers that reach the chain only through the upstream plane.
/// The keyless window has to be cut open inside epoch 0, before the followers' DKG
/// clock enters epoch 1 and their non-member acquisition fetches `PK_2`.
#[cfg(feature = "dpos-devnet-byzantine")]
fn the_first_three_are_the_committee() -> Committees {
    Committees::Schedule(Arc::new(|_epoch, _n| Some(vec![0, 1, 2])))
}

/// The three committee members forge the σ slot of `Finalized{h}` for `h`
/// in `FORGE_WINDOW` with a real σ of another round: a keyless follower admits the
/// certificate, and when the key lands the σ is refused and dropped, while the
/// archive keeps serving the forgery on.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn a_forged_seed_slot_is_admitted_with_no_key_and_refused_when_the_key_lands() {
    /// Inside epoch 0, before the followers' DKG clock enters epoch 1; see
    /// [`the_first_three_are_the_committee`].
    const CUT_AT: u64 = 4;
    /// Above the forge window, so the forged certificates are taken while the
    /// followers are keyless, and low enough that the key lands and the promote
    /// refusals get logged.
    const HEAL_ABOVE: u64 = 3 * EPOCH_LEN;
    let mut cfg = StandConfig::live(5, 1);
    cfg.committees = the_first_three_are_the_committee();
    // Every consensus-plane link is left in place so the two outsiders can acquire
    // `PK_2` after the fact; only the cut below takes those links away, and only
    // for the consensus plane.
    cfg.peer_set = PeerSet::CommitteeTrackedOnly;
    cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD.min(EPOCH_LEN));
    // Node 4 pulls by height from node 3 alone, so a poisoned archive is what
    // serves it the forgery; without the seam the answering peer decides by
    // shuffle.
    cfg.upstream_source_only_for = Some((4, 3));
    let mut stand = Stand::new(cfg);
    for i in 0..3 {
        stand.node(i).role(Role::ForgedSeedUpstream);
    }
    // A non-member asks for the epoch key ahead of need, so non-membership opens
    // no keyless window; a consensus-plane cut leaves the followers without that
    // pull while their frontier probe keeps feeding their marshal.
    stand
        .partition(&[0, 1, 2], &[3, 4])
        .after_height(CUT_AT)
        .consensus_only()
        .heal_above(HEAL_ABOVE);
    let out = stand.run_until(
        |p| p.min_height_of(&[0, 1, 2]) >= 140,
        Duration::from_secs(300),
    );
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(
        !out.partitions[0].heights_at_heal.is_empty(),
        "the consensus-plane cut never fired or never healed, so the keyless window          and the key landing are not this fixture's: {:?}",
        out.partitions[0]
    );

    let forged: Vec<u64> = (0..3)
        .flat_map(|i| out.byz[i].forged_heights.clone())
        .collect();
    assert!(
        !forged.is_empty(),
        "no certificate was forged — heights {:?}, cut {:?}, seen {:?}",
        out.heights,
        out.partitions[0],
        (0..3).map(|i| out.byz[i].certs_seen).collect::<Vec<_>>()
    );
    for i in 0..3 {
        let byz = &out.byz[i];
        if byz.forged_heights.is_empty() {
            continue;
        }
        assert!(
            byz.forged_seed_differs,
            "node {i} served a 'forged' σ that read back as the original: {byz:?}"
        );
        assert!(
            byz.forged_vote_half_intact,
            "node {i}'s forge touched the multisig half: {byz:?}"
        );
        for h in &byz.forged_heights {
            assert!(
                crate::testbed::byzantine_roles::FORGE_WINDOW.contains(h),
                "node {i} forged {h}, outside the window: {byz:?}"
            );
        }
    }

    let followers = [3usize, 4];
    let rejected: u64 = followers
        .iter()
        .map(|&i| out.upstream[i].deliveries_rejected)
        .sum();
    let keyless: f64 = followers
        .iter()
        .map(|&i| {
            out.metric(i, "dpos_seed_verify_no_key_total")
                .unwrap_or(0.0)
        })
        .sum();
    let refusals = out.logs_containing("held seed does not verify");
    let branch = if rejected > 0 {
        "(b) NOT reproduced — a follower rejected the forged certificate outright"
    } else if refusals.is_empty() {
        "(c) PATH NOT REACHED — nothing was admitted keyless, or no key ever landed"
    } else {
        "(a) REPRODUCED — admitted with no key, refused when the key landed"
    };
    eprintln!(
        "(R-008) branch = {branch} | heights={:?} forged={forged:?} keyless={keyless} \
         refusals={} replays={:?} virtual={:?} real={:?}",
        out.heights,
        refusals.len(),
        (0..5)
            .map(|i| out.byz[i].served_seed_replays.clone())
            .collect::<Vec<_>>(),
        out.virtual_elapsed,
        out.real_elapsed
    );
    assert_eq!(
        rejected, 0,
        "branch (b) was observed: a follower rejected the forged certificate, so `NoKey` \
         admission is not the gate R-008 names — upstream {:?}",
        out.upstream
    );
    assert!(
        keyless > 0.0,
        "branch (c) was observed: no keyless admission was counted, the `NoKey` window never \
         opened"
    );
    assert!(
        !refusals.is_empty(),
        "branch (c) was observed: the `NoKey` window opened but no key ever landed, so nothing \
         was promoted and the second half of R-008 was not exercised"
    );

    out.assert_lockstep_except(&followers);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    // The only ERROR lines this run may carry are the promote refusals.
    for line in out.errors() {
        assert!(
            line.text.contains("held seed does not verify"),
            "unexpected ERROR line: {line:?}"
        );
    }
    // Every refused round is a height this wrapper forged: view = h − (2·L − 1).
    let mut refused_heights: Vec<u64> = refusals
        .iter()
        .map(|l| {
            let view = l
                .text
                .rsplit_once("View(")
                .and_then(|(_, rest)| rest.split(')').next())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or_else(|| panic!("no view in {l:?}"));
            assert!(
                l.text.contains("Epoch(2)"),
                "a refusal outside epoch 2: {l:?}"
            );
            view + 2 * EPOCH_LEN - 1
        })
        .collect();
    refused_heights.sort_unstable();
    refused_heights.dedup();
    let mut forged_sorted = forged.clone();
    forged_sorted.sort_unstable();
    forged_sorted.dedup();
    assert_eq!(
        refused_heights, forged_sorted,
        "the rounds refused at promote are not the heights the wrapper forged"
    );
    // No follower ever derived a forged height: the σ was dropped, not consumed.
    for &i in &followers {
        for h in &forged_sorted {
            assert!(
                out.seeds[i].get(h).cloned().flatten().is_none(),
                "follower {i} derived height {h} — a forged σ reached its executor"
            );
        }
        assert!(
            out.metric(i, "dpos_seed_verify_ok_total").unwrap_or(0.0) > 0.0,
            "follower {i} never verified a σ, so it never obtained an epoch key at all"
        );
    }
    // Archive poisoning: a follower handed a forged certificate on to the other.
    let relayed: Vec<u64> = followers
        .iter()
        .flat_map(|&i| out.byz[i].served_seed_replays.clone())
        .collect();
    assert!(
        !relayed.is_empty(),
        "no follower relayed the forgery — the archive-poisoning half of R-008 was not reached \
         (serve counts {:?})",
        out.upstream
    );
}

/// The shared fixture for the two lying-`Latest` upstream roles: node 0 leaves the
/// committee at epoch 3, the re-jump gate is on, and its only frontier source is
/// node 3, so the two runs of each test differ only in node 3's role.
#[cfg(feature = "dpos-devnet-byzantine")]
fn lying_upstream_stand(role3: Role) -> super::stand::Outcome {
    let mut cfg = StandConfig::live(4, 1);
    cfg.committees = Committees::Schedule(Arc::new(|epoch, n| {
        Some(match epoch {
            3 | 4 => vec![1, 2, 3],
            _ => (0..n).collect(),
        })
    }));
    cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD.min(EPOCH_LEN));
    cfg.upstream_only_link = Some((3, 0));
    // The tests read every node's marshal tip per tick.
    cfg.marshal_tip_series = true;
    let mut stand = Stand::new(cfg);
    stand.node(3).role(role3);
    // The lag is physical, not rotational: a non-member can ask for the epoch key,
    // node 0 is cut off and the cut heals a gate-and-a-half later, so the production
    // re-jump carries it back.
    stand
        .partition(&[1, 2, 3], &[0])
        .after_height(3 * EPOCH_LEN - 1)
        .heal_above(4 * EPOCH_LEN + 16);
    stand.run_until(
        move |p| p.min_height_of(&[1, 2, 3]) >= 6 * EPOCH_LEN,
        Duration::from_secs(400),
    )
}

/// Node 3 answers `Latest` with a
/// tip whose height is inflated by `LATEST_INFLATION`: `deliver` binds the served
/// height to the certificate's round epoch, refuses it, and node 0 never jumps.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn an_inflated_latest_probe_is_refused_at_the_frontier_and_spawns_no_re_jump() {
    use super::byzantine_roles::LATEST_INFLATION;
    let role = lying_upstream_stand(Role::InflatedProbe);
    let control = lying_upstream_stand(Role::Honest);

    let byz = &role.byz[3];
    assert!(
        byz.latest_inflated >= 1,
        "the inflated-probe wrapper forged no Latest: {byz:?}"
    );
    assert!(
        byz.inflate_delta_ok,
        "an inflation added something other than {LATEST_INFLATION}: {byz:?}"
    );
    assert!(
        byz.inflate_structural_ok,
        "a forged Latest failed the payload-digest bind: {byz:?}"
    );
    assert_eq!(
        byz.inflate_to,
        byz.inflate_from.map(|f| f + LATEST_INFLATION),
        "inflate_to is not inflate_from + {LATEST_INFLATION}: {byz:?}"
    );
    assert_eq!(
        control.byz[3].latest_inflated, 0,
        "the honest control inflated a Latest: {:?}",
        control.byz[3]
    );

    let calls = &role.jump_calls[0];
    let series = &role.marshal_tip_series[0];
    let peaked = series.iter().copied().max().unwrap_or(0);
    let ctrl_calls = &control.jump_calls[0];
    let branch = if peaked >= LATEST_INFLATION {
        "(a) NOT CLOSED — the forged height moved the victim's own marshal tip"
    } else if !calls.is_empty() {
        "(b) NOT CLOSED — a re-jump was spawned without a verified frontier"
    } else {
        "(c) CLOSED — the forgery was refused at deliver; no tip growth, no re-jump"
    };
    eprintln!(
        "(R-004) branch = {branch} | role: peaked={peaked} calls={} up[0]={:?} probes[0]={} \
         heights={:?} | control: calls={} probes[0]={} heights={:?} | virt role={:?} ctrl={:?} \
         real role={:?} ctrl={:?}",
        calls.len(),
        role.upstream[0],
        role.probe_calls[0],
        role.heights,
        ctrl_calls.len(),
        control.probe_calls[0],
        control.heights,
        role.virtual_elapsed,
        control.virtual_elapsed,
        role.real_elapsed,
        control.real_elapsed,
    );

    // A `false` costs node 3 the channel for the life of node 0's resolver
    // engine, and the control must refuse nothing — otherwise a refused forgery is
    // indistinguishable from a refused honest peer.
    assert!(
        role.upstream[0].deliveries_rejected >= 1,
        "the victim never refused the inflated Latest: {:?}",
        role.upstream[0]
    );
    assert_eq!(
        control.upstream[0].deliveries_rejected, 0,
        "the honest control refused a frontier answer: {:?}",
        control.upstream[0]
    );

    // The victim's marshal tip can only move through `store_finalization`, and the
    // bound is its rotation boundary: an honest member legitimately holds up to
    // 95, and the lie bought nothing beyond it.
    assert!(
        peaked < 3 * EPOCH_LEN,
        "the victim's marshal tip ran past its rotation boundary on a forged frontier \
         (branch {branch}): peak {peaked}, series {series:?}"
    );
    assert!(
        calls.is_empty(),
        "a re-jump was spawned on an unverified frontier (branch {branch}): {calls:?}"
    );

    // The same fixture with an honest node 3 lands every jump and carries node 0
    // past the boundary.
    assert!(
        !ctrl_calls.is_empty() && ctrl_calls.iter().all(|c| c.outcome == "Landed"),
        "the honest control did not land every jump: {ctrl_calls:?}"
    );
    assert_eq!(
        role.heights[0],
        3 * EPOCH_LEN - 1,
        "the role victim did not park at the rotation boundary 95: {:?}",
        role.heights
    );
    assert!(
        control.heights[0] > 3 * EPOCH_LEN - 1,
        "the honest control did not recover past 95 — the contrast is not attributed: {:?}",
        control.heights
    );

    assert!(
        !role.timed_out,
        "the role run timed out: {:?}",
        role.heights
    );
    assert!(
        !control.timed_out,
        "the control run timed out: {:?}",
        control.heights
    );
    assert_eq!(role.diverged, None, "the role run forked");
    role.assert_lockstep_except(&[0]);
    assert!(role.halted.is_empty(), "{:?}", role.halted);
    for i in [1, 2, 3] {
        assert!(
            role.marshal_tip_series[i]
                .iter()
                .all(|&v| v < LATEST_INFLATION),
            "node {i} was reached by the forgery — isolation leaked: {:?}",
            role.marshal_tip_series[i]
        );
    }
}

/// Node 3 answers `Latest` with
/// a real tip whose `result` is a divergent branch it seeded: the broken multisig
/// is refused at `deliver`, so the forged answer never reaches an EL FCU.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn a_lying_upstream_is_refused_at_the_frontier_and_never_lands_its_branch() {
    use super::byzantine_roles::{divergent_hash, LYING_DIVERGE_AT};
    let role = lying_upstream_stand(Role::LyingUpstream);
    let control = lying_upstream_stand(Role::Honest);

    let byz = &role.byz[3];
    assert!(
        byz.result_forged >= 1,
        "the lying-upstream wrapper forged no result: {byz:?}"
    );
    assert!(
        byz.result_differs,
        "a forged result read back equal to the original: {byz:?}"
    );
    assert!(
        byz.result_structural_ok,
        "a forged Latest failed the payload-digest bind — a cheaper check caught it, so this \
         run does not exercise the multisig arm: {byz:?}"
    );
    assert!(
        byz.forged_result_to.is_some() && byz.forged_result_to != byz.forged_result_from,
        "the forged result is not distinct from the real one: {byz:?}"
    );
    assert_eq!(
        control.byz[3].result_forged, 0,
        "the honest control forged a result: {:?}",
        control.byz[3]
    );

    let calls = &role.jump_calls[0];
    // Any jump call here means the forgery passed `deliver`; nothing downstream
    // would refuse it.
    let branch = match calls.last().map(|c| c.outcome) {
        None => "(c) CLOSED — the forgery was refused at deliver; node 0 never jumped",
        Some(other) => other,
    };
    eprintln!(
        "(R-001) branch = {branch} | role: calls={calls:?} up[0]={:?} heights={:?} halted={:?} \
         diverged={:?} | control: calls={} heights={:?} | virt role={:?} ctrl={:?} \
         real role={:?} ctrl={:?}",
        role.upstream[0],
        role.heights,
        role.halted,
        role.diverged,
        control.jump_calls[0].len(),
        control.heights,
        role.virtual_elapsed,
        control.virtual_elapsed,
        role.real_elapsed,
        control.real_elapsed,
    );

    // The forgery keeps a real certificate over a payload it no longer matches, so
    // the multisig arm refuses it.
    assert!(
        role.upstream[0].deliveries_rejected >= 1,
        "the victim never refused the forged Latest: {:?}",
        role.upstream[0]
    );
    assert_eq!(
        control.upstream[0].deliveries_rejected, 0,
        "the honest control refused a frontier answer: {:?}",
        control.upstream[0]
    );

    // No jump was spawned, so nothing pointed reth at the attacker's branch, and
    // the victim's EL never holds one of its hashes.
    assert!(
        calls.is_empty(),
        "a jump ran on an unverified frontier (branch {branch}): {calls:?}"
    );
    let divergent: Vec<u64> = role.el_events[0]
        .iter()
        .filter_map(|e| match e {
            ElEvent::Canonicalized(h, hash) | ElEvent::Derived(h, hash) => {
                (*h >= LYING_DIVERGE_AT && *hash == divergent_hash(*h)).then_some(*h)
            }
        })
        .collect();
    assert!(
        divergent.is_empty(),
        "the victim's EL holds the attacker's branch at {divergent:?} — the forgery was not \
         refused before the FCU"
    );

    assert!(
        !control.jump_calls[0].is_empty()
            && control.jump_calls[0].iter().all(|c| c.outcome == "Landed"),
        "the honest control did not land every jump: {:?}",
        control.jump_calls[0]
    );
    assert!(
        !role.timed_out,
        "the role run timed out: {:?}",
        role.heights
    );
    assert_eq!(role.diverged, None, "the role run forked");
    assert!(role.halted.is_empty(), "role halted: {:?}", role.halted);
    role.assert_lockstep_except(&[0]);
    for i in [0, 1, 2] {
        assert_eq!(
            role.byz[i].result_forged, 0,
            "node {i} forged a result: {:?}",
            role.byz[i]
        );
    }
    for i in [1, 2, 3] {
        assert!(
            role.jump_calls[i].is_empty(),
            "node {i} re-jumped: {:?}",
            role.jump_calls[i]
        );
    }
}

/// The shared fixture for the wrong-height tests: node 3 leaves the committee at the first boundary
/// and follows purely through the by-height plane with node 0 as its only source;
/// the re-jump gate stays closed so the by-height path is the only repair.
#[cfg(feature = "dpos-devnet-byzantine")]
fn wrong_height_stand(source_role: Role) -> super::stand::Outcome {
    let mut cfg = StandConfig::honest(4, 1);
    cfg.epoch_len = 5;
    cfg.committees = shrink_to_three();
    cfg.peer_set = PeerSet::Committee {
        upstream_link: true,
    };
    cfg.upstream_only_link = Some((0, 3));
    let mut stand = Stand::new(cfg);
    stand.node(0).role(source_role);
    stand.run_until(
        |p| p.min_height_of(&[0, 1, 2]) >= 16,
        Duration::from_secs(120),
    )
}

/// Node 0 answers every
/// `Finalized{h}` with its own valid pair of height `h − 1`: `deliver` binds the
/// key to the delivered height, refuses it, and commonware permanently excludes
/// the liar.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn a_wrong_height_answer_costs_the_liar_the_channel_and_never_satisfies_the_fetch() {
    let role = wrong_height_stand(Role::WrongHeightFinalized);
    let control = wrong_height_stand(Role::Honest);

    assert!(!role.timed_out, "role heights {:?}", role.heights);
    assert!(!control.timed_out, "control heights {:?}", control.heights);

    let byz = &role.byz[0];
    assert!(
        byz.wrong_height_served >= 1,
        "the wrong-height wrapper served nothing: {byz:?}"
    );
    for (h, served) in &byz.wrong_height_pairs {
        assert_eq!(
            *served,
            *h - 1,
            "a served height was not requested-1: {byz:?}"
        );
        assert!(
            crate::testbed::byzantine_roles::WRONG_HEIGHT_WINDOW.contains(h),
            "node 0 served {h}, outside the window: {byz:?}"
        );
    }
    assert!(
        byz.wrong_height_valid,
        "a served pair was not a real finalization of requested-1: {byz:?}"
    );
    assert_eq!(
        control.byz[0].wrong_height_served, 0,
        "the honest control served a wrong height: {:?}",
        control.byz[0]
    );

    let v = 3usize;
    let u = role.upstream[v];
    let branch = if u.deliveries_rejected == 0 {
        "(a) NOT CLOSED — the victim accepted a wrong-height pair"
    } else if byz.wrong_height_served > u.deliveries_rejected {
        "(b) PARTIAL — the liar was refused but kept being asked"
    } else {
        "(c) CLOSED — the first lie was refused and the liar was never asked again"
    };
    eprintln!(
        "(R-009) branch = {branch} | role: served={} pairs={:?} victim_h={} up={u:?} \
         | control: victim_h={} up={:?} | virt role={:?} ctrl={:?} real role={:?} ctrl={:?}",
        byz.wrong_height_served,
        byz.wrong_height_pairs,
        role.heights[v],
        control.heights[v],
        control.upstream[v],
        role.virtual_elapsed,
        control.virtual_elapsed,
        role.real_elapsed,
        control.real_elapsed,
    );

    // Every substitution is refused, and a refusal is what commonware turns into a
    // permanent exclusion, so the counts stop there.
    assert_eq!(
        u.deliveries_rejected, byz.wrong_height_served,
        "the victim did not refuse every wrong-height pair (branch {branch}): {u:?} vs {byz:?}"
    );
    assert_eq!(
        byz.wrong_height_served, 1,
        "the liar was asked for a second by-height pull after being refused — the exclusion \
         did not take (branch {branch}): {byz:?}"
    );
    // A wrong-height answer does not satisfy the fetch.
    assert!(
        u.finalized_calls > u.finalized_delivered,
        "every by-height pull completed — a wrong-height answer still satisfied one: {u:?}"
    );
    assert_eq!(
        u.rejump_calls, 0,
        "a re-jump ran — the by-height path was not the only repair path: {u:?}"
    );
    assert_eq!(
        control.upstream[v].deliveries_rejected, 0,
        "the victim refused an HONEST source: {:?}",
        control.upstream[v]
    );

    // With the liar as its only source the victim stands at its drop boundary; an
    // honest source closes the gap and it follows.
    assert!(
        control.heights[v] >= 14,
        "the honest control did not follow through the by-height plane: {:?}",
        control.heights
    );
    assert_eq!(
        role.heights[v], 5,
        "the victim did not stand at its drop boundary (branch {branch}): {:?}",
        role.heights
    );
    assert!(
        role.heights[v] < control.heights[v],
        "the victim was not starved relative to the control (role {}, control {})",
        role.heights[v],
        control.heights[v]
    );

    // No leak to bystanders: the committee stays in lockstep in both runs.
    role.assert_lockstep_except(&[v]);
    control.assert_lockstep_except(&[v]);
    assert!(role.halted.is_empty(), "role halted: {:?}", role.halted);
    assert!(
        control.halted.is_empty(),
        "control halted: {:?}",
        control.halted
    );
    assert_eq!(role.diverged, None, "role diverged: {:?}", role.diverged);
    assert_eq!(
        control.diverged, None,
        "control diverged: {:?}",
        control.diverged
    );
    assert!(
        role.errors().is_empty(),
        "unexpected ERROR lines (role): {:?}",
        role.errors()
    );
    assert!(
        control.errors().is_empty(),
        "unexpected ERROR lines (control): {:?}",
        control.errors()
    );
}

/// The two refusal arms in `FakeChain::land_jump` are unreached on the honest
/// schedules, so drive them directly: a tip the peer network does not hold, and a
/// divergent block at an already-canonical height.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn land_jump_refuses_an_unservable_tip_and_a_conflicting_prefix() {
    use super::fakes::{ElNetwork, FakeChain, JumpLanding};
    use alloy_primitives::B256;
    let h = |n: u8| B256::repeat_byte(n);

    let el = ElNetwork::default();
    let chain = FakeChain::with_genesis_on(h(0), el.clone());
    assert_eq!(
        chain.land_jump(h(99)),
        JumpLanding::Unservable,
        "a tip the peer network does not hold must be Unservable"
    );

    // Land an honest prefix so arm 2 has a canonical chain to conflict with.
    let honest: Vec<(B256, u64, B256)> = (1..=5u64)
        .map(|n| (h(n as u8), n, h((n - 1) as u8)))
        .collect();
    el.publish_branch(&honest);
    assert_eq!(
        chain.land_jump(h(5)),
        JumpLanding::Landed,
        "the honest prefix must land"
    );

    let div5 = h(200);
    el.publish_branch(&[(div5, 5, h(4))]);
    assert_eq!(
        chain.land_jump(div5),
        JumpLanding::ConflictingPrefix {
            height: 5,
            mine: h(5),
            served: div5,
        },
        "a divergent block at an already-canonical height must be a ConflictingPrefix"
    );
}

/// A node in the registry but in no committee is secondary on every node's peer
/// set, and still follows the chain. The simulated network does not tier delivery,
/// so what this pins is the tier the node's own code assigns.
#[test]
fn a_registry_only_node_is_secondary_on_every_peer_set_and_still_follows() {
    use commonware_cryptography::Signer as _;

    const OUTSIDER: usize = 3;
    let mut cfg = StandConfig::honest(4, 43);
    // The production shape: the registry holds every activated validator.
    assert!(matches!(cfg.peer_set, PeerSet::AllNodes));
    // Node 3 is activated and registered, and on no committee, ever.
    cfg.committees = Committees::Schedule(Arc::new(|_epoch, n| Some((0..n - 1).collect())));
    let members = [0usize, 1, 2];
    let out = Stand::new(cfg).run_until(
        move |p| p.min_height_of(&members) >= 3 * EPOCH_LEN,
        Duration::from_secs(400),
    );
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.halted.is_empty(), "{:?}", out.halted);

    let (peers, _) = super::stand::keys(43, 4);
    let key_of: Vec<_> = peers.iter().map(|p| p.public_key()).collect();
    let outsider = &key_of[OUTSIDER];

    // Premise: every node registered at least two epochs' peer sets, so the
    // assertions below range over something.
    for i in 0..4 {
        assert!(
            out.peer_sets[i].len() >= 2,
            "node {i} tracked {} peer sets: nothing to check",
            out.peer_sets[i].len()
        );
    }
    let mut checked = 0usize;
    for (i, sets) in out.peer_sets.iter().enumerate() {
        for (epoch, primary, secondary) in sets {
            assert!(
                !primary.contains(outsider),
                "node {i}'s epoch-{epoch} PRIMARY holds the registry-only node"
            );
            assert!(
                secondary.contains(outsider),
                "node {i}'s epoch-{epoch} SECONDARY is missing the registry-only node: \
                 {secondary:?}"
            );
            for m in &members {
                assert!(
                    primary.contains(&key_of[*m]),
                    "node {i}'s epoch-{epoch} PRIMARY is missing committee member {m}"
                );
            }
            checked += 1;
        }
    }
    assert!(checked >= 8, "only {checked} peer sets examined");
    assert_eq!(
        out.tracked_mismatches, 0,
        "two nodes tracked different peer sets for one epoch"
    );

    // The outsider is not cut off: it keeps executing the chain the committee
    // finalizes.
    assert!(
        out.heights[OUTSIDER] >= 2 * EPOCH_LEN,
        "the registry-only node stopped following: {:?}",
        out.heights
    );
}
