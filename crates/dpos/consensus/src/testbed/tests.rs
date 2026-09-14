//! The mandatory stand tests (task Э3.2, this session). Every one runs on the
//! deterministic runner without the `external` feature; the real run times are
//! recorded in `.dpos-study/history/E3-2-STAND-1.md` §3.

use super::{
    fakes::{ElEvent, UpstreamCounters, DPOS_ACTIVATION_BLOCK},
    stand::{
        CertInletCfg, CertInletSource, Committees, Divergence, Outcome, PeerSet, Progress, Role,
        Stand, StandConfig, TeeWiring, CHAIN_ID,
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
                // No records and no geometry: this test only needs the fetch to
                // park and time out, and an unfrozen module is the state a node
                // is in before it has read anything at all.
                crate::committee::testing::SchemeCommittee::new(|_| None),
                UpstreamCounters::default(),
                // This probe is about a fetch that PARKS and times out — nothing
                // is ever delivered, so nothing can be blocked. A throwaway spy
                // keeps the signature honest without pretending to observe.
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

/// (3b) R-006 scenario 1 — the CATCH-UP arm of guard #2, at the tier reth
/// actually answers.
///
/// Node 3 is isolated for eight views, so its marshal tip freezes while the
/// other three finalize on; when the links return it derives the whole missed
/// range with `last_tip_height >= h + K`, i.e. with guard #2 ARMED
/// (`executor.rs:3036`, `:3140-3180`). `Role::DivergentResult { at: DIVERGE }`
/// makes exactly one of those catch-up derives seal to a hash nobody else
/// derived, so the committee-attested `result` at `DIVERGE + K` — which is
/// already finalized and already in this node's marshal — contradicts it.
///
/// The point of the case is WHICH check renders the verdict, and that is a
/// property of the EL tier the guard reads:
///
/// * guard #2 reads `spec_executed_hash(DIVERGE)` — reth's CANONICAL chain —
///   from inside `try_derive`, BEFORE that block's own FCU
///   (`executor.rs:3140` sits above the FCU at `:3308`). Under reth semantics
///   the block is `InsertExecutedBlock`-imported and tree-private at that
///   moment, so `block_hash` answers `None`, `result_matches` answers `None`,
///   and the guard — which only fires on `Some(false)` — passes in silence.
///   The verdict then falls to the `h − K` BACKWARD cross-check
///   (`executor.rs:3234`) K heights later, after `DIVERGE ..= DIVERGE + K − 1`
///   have been FCU'd as head/safe and had their finalized cursor advanced.
/// * a fake that canonicalizes AT DERIVE hands guard #2 its own fresh hash and
///   the halt lands at `DIVERGE` itself, before the ack — which is the green
///   test on a false oracle R-006 names.
///
/// BOTH branches are written out and the OBSERVED one is asserted, so a later
/// change of the fake's tier flips this test instead of silently re-labelling
/// it:
///
/// * REPRODUCED (what this run does, 2026-09-10): guard #2 logs nothing, the
///   halt is the backward cross-check at `DIVERGE + K` = 9, node 3's tier-F tip
///   is `DIVERGE + K − 1` = 8, and heights 6, 7 and 8 are visibly forked.
/// * NOT REPRODUCED (what the land-at-derive fake did, kept for the record):
///   `guard #2 at 6: attested result at 9 disagrees with local
///   executed_hash(6)`, halt before the ack, tier-F tip 5, `diverged = None`
///   because node 3 never finalized a height the others also hold differently.
///
/// AN EMPTY GUARD-#2 LOG IS NOT ON ITS OWN THE CLAIM. Two other worlds produce
/// the same silence, and each is excluded by a POSITIVE observation rather than
/// by an absence:
///
/// * "guard #2 never armed" (`last_tip_height < h + K`, `executor.rs:3036`) —
///   excluded by the partition witness (node 3 sits at its cut height while the
///   others reach 17, so its marshal tip is ≥ 6 + K by the time it derives 6)
///   TOGETHER with `Outcome::el_events`, which shows node 3 deriving 4, 5, 6 in
///   order — all of them after the heal, since its tier-F tip at the heal was 3
///   and the executor advances the cursor per block.
/// * "guard #2 armed and PARKED on an absent `h + K` body"
///   (`DeriveOutcome::NeedAttestation`) — excluded because a park is not a halt
///   and stops progress: the node would sit at tier-F tip 5. It reaches 8 and
///   goes on to DERIVE 9, which is where the verdict lands.
///
/// The ordering claim itself — "the guard's read at the derive of `DIVERGE`
/// could only see `None`" — is checked directly in `el_events`: the
/// `Derived(DIVERGE, _)` entry precedes the `Canonicalized(DIVERGE, _)` entry,
/// so at guard-#2 time the block was tree-only. And the halt's placement is read
/// the same way: `Derived(DIVERGE + K, _)` exists on node 3 while
/// `Canonicalized(DIVERGE + K, _)` does NOT — the verdict front-ran that block's
/// own FCU, exactly as `executor.rs:3247` sits above `:3308`.
///
/// What this case does NOT show: whether guard #2 has any reachable arm left at
/// all. `Some(false)` needs a canonical hash at `h` that disagrees, i.e. R-006
/// scenario 2 (a speculative sibling surviving into the finalized derive), which
/// no stand fixture produces — see `15_smoke_cases_as_behavioral_spec_devnet_lo.md`
/// §15.a.
///
/// Falsifier: no halt at all (the isolation did not put the node behind by K, or
/// the divergent height was never derived on the catch-up path — the partition
/// witness below fails first in that case); a halt on an honest node; the
/// divergent hash equalling the honest one (the tamper did not take); a
/// canonicalization of `DIVERGE` at or before its derive (the fake regressed to
/// land-at-derive); log capture not live (then the two log observations would be
/// silently skipped, which is how a green run on no evidence happens).
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
    // `el_events` is the ORDERING evidence — see the doc block. Node 3's slice
    // around the fork, and node 0's for contrast.
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
    // F13: the two log observations below are the branch discriminator. A dead
    // capture would skip them and leave a green run resting on nothing, so the
    // capture is a PRECONDITION of this test, not a conditional.
    assert!(
        out.log_capture_live,
        "log capture is not live — this test's guard-#2 observation would be skipped"
    );

    // The node really was ≥ K behind when it started deriving the missed range:
    // it stood at its cut height while the other three ran on.
    let part = &out.partitions[0];
    assert!(
        part.heights_at_heal[3] + k <= part.heights_at_heal[0],
        "node 3 was not K behind at the heal: {part:?}"
    );
    assert_eq!(part.heights_at_cut[3], part.heights_at_heal[3]);

    // Non-defaulting accessors over `el_events`. `Outcome::hashes` cannot serve
    // here: it substitutes `B256::ZERO` for a height a node never finalized
    // (`stand.rs`), so an `assert_ne!` over it can pass on two placeholders.
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

    // TAMPER SELF-CHECK, before anything is claimed about detection: node 3's
    // own block at DIVERGE — and, because they descend from it, at DIVERGE+1 and
    // DIVERGE+2 — is a hash the honest three do not hold.
    for h in DIVERGE..DIVERGE + k {
        let mine = canonical_hash(3, h)
            .unwrap_or_else(|| panic!("node 3 never canonicalized {h}; el_events {:?}", around(3)));
        let theirs = canonical_hash(0, h)
            .unwrap_or_else(|| panic!("node 0 never canonicalized {h}; el_events {:?}", around(0)));
        assert_ne!(mine, theirs, "height {h} did not fork");
    }

    // THE ORDERING OBSERVATION: at DIVERGE the block was in the executed tree
    // and NOT yet canonical, which is the only state guard #2 could have read.
    let derived = derived_at(3, DIVERGE).expect("node 3 derived DIVERGE");
    let canonicalized = canonicalized_at(3, DIVERGE).expect("node 3 canonicalized DIVERGE");
    assert!(
        derived < canonicalized,
        "node 3 canonicalized DIVERGE at index {canonicalized} but derived it at {derived} — \
         the fake regressed to land-at-derive; el_events {:?}",
        around(3)
    );
    // The three catch-up derives before the verdict were all canonicalized, and
    // the height the verdict landed on was NOT — the halt front-ran its FCU.
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

    // The halt is node 3's alone, and the honest three never left lockstep.
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

/// (4b) The same rotation with the peer set narrowed to the committees and EVERY
/// link of a node outside them severed — consensus plane and upstream plane. Once
/// severed the rotated-out node is an unregistered joiner: no backfill source, and
/// it STANDS while the members go on.
///
/// WHEN it is severed moved in 4.3. Primary is `C[E−1] ∪ C[E] ∪ C[E+1]`, so the
/// node rotated out at epoch 1 is still tracked while `E = 1` — the outgoing
/// committee keeps its links for one more epoch, by design — and its links fall
/// when its peers track `E = 2`, i.e. at `start(2) = 2 * epoch_len = 10`. So a flat
/// `finalized_delivered == 0` is no longer the right shape of the claim: while it
/// had peers it WAS answered, four times.
///
/// The negative property that replaces it is the same statement bounded in time,
/// read off the by-height pull SERIES (`Outcome::upstream_served`, which carries
/// the height and the answer of every `get_finalization` this node made) rather
/// than off a total that cannot see when: AFTER the cut, not one pull is answered.
/// Measured: pulls for 5, 6, 7 and 8 delivered; the pull for 14 — the first one
/// past the severance — refused, and node 3 parks at 8 while the members reach 16.
///
/// Falsifier: node 3 reaching `start(2)` (then something feeds it across the cut);
/// ANY pull at or above the severance line coming back (the cut did not hold); no
/// pull being made past the line (the node stopped asking, so its silence proves
/// nothing); no pull being answered at all (it was cut before it ever had a peer,
/// so the stand proves nothing about severance); or the members not crossing the
/// boundaries.
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
    // The severance line, as a NUMBER and not a guess: node 3's links fall when its
    // peers track epoch 2, whose first block is `start(2) = 2 * epoch_len`. It can
    // never execute into epoch 2, so this is the strict bound the old `< 10` was
    // and the `< 15` that replaced it was not — `15 = start(3)` left the whole of
    // epoch 2 inside the tolerance.
    const SEVERED_AT: u64 = 2 * 5;
    assert!(
        out.heights[3] < SEVERED_AT,
        "node 3 executed into epoch 2, which it has no peer to reach: {:?}",
        out.heights
    );
    let u3 = out.upstream[3];
    // The live form of step A's timeout: the probe's `get_latest` has no peer to
    // reach once the links are gone and expires on the virtual clock (8 s) — the
    // executor then issues the next one. A `fetch_one` that never returned would
    // leave exactly one call.
    assert!(
        u3.latest_calls >= 2,
        "node 3's first frontier fetch never expired (a hung fetch_one): {u3:?}"
    );
    // It HAD peers: cut before that, nothing below would be about severance.
    assert!(
        u3.latest_delivered > 0,
        "node 3 was severed before it ever had a peer, so the assertions below prove \
         nothing about severance: {u3:?}"
    );
    assert!(
        u3.latest_calls > u3.latest_delivered,
        "every probe was answered — node 3 never lost its peers: {u3:?}"
    );
    // THE negative property, and the one the flat `finalized_delivered == 0` used
    // to carry: past the severance line, not one by-height pull is answered.
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
    // ...and the refusals are a TAIL, not a scattering: once the links are gone
    // nothing is served again.
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
    eprintln!("(Д4/4c) gap on fcu={:?}", out.head_gap_on_fcu);
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

/// The (B2) schedule: 4 → 3 → 4. Epochs 0–2 all four (2 is the bootstrap
/// mint), epochs 3–4 `[0,1,2]` (3 is a change ⇒ `dkgQual[3]`, 4 is stable ⇒
/// carry-forward), epoch 5 all four again (a change ⇒ `dkgQual[5]`).
/// (4.2 А.2) THE LADDER IS A LADDER: a lagging node names successive rungs and
/// ends up holding every one of them.
///
/// §5.2 makes the frontier probe put `Finalized{last(T+1)}` at `committee[T+1]`
/// on every tick where the marshal tip is frozen, and "лестница = повторение того
/// же шага после посадки (новый `fin` ⇒ новый `T` ⇒ новое окно)". This test reads
/// that sequence off a real deep-lag run.
///
/// WHAT THIS TEST USED TO SAY, AND WHY IT WAS WRONG (review A2-02). The first
/// version asserted the OPPOSITE — that every named step is at or below this
/// node's own marshal tip, so the ladder can lift nobody — on the arithmetic
/// `last(T+1) <= last(epoch(fin)+2)`. That inequality compares the step with the
/// marshal's CEILING, not with its tip, and `tip == ceiling` holds only for a node
/// that received and stored everything below the ceiling (which the Д3
/// precondition asserts as a property of ITS fixture, `preconditions.rs:261`).
/// The assert passed because it compared every step with the tip the run ENDED on
/// rather than the tip at naming time. The claim was false, and the run below
/// shows why: the steps are DISTINCT successive epoch terminals and the node ends
/// above all of them.
///
/// WHY THE NAMING-TIME TIP IS NOT ASSERTED HERE. "The step stood above my own tip
/// when I named it" is the executor's own comparison (`last_tip_height`), and it
/// is free there and not here: sampling the marshal tip from the PROBE CLOSURE is
/// an extra message into the marshal's select loop and it CHANGES THE RUN —
/// measured, `a_zero_overlap_boundary_is_crossed_by_acquiring_the_other_halfs_key` loses its
/// incoming half's DKG artifact under it (Д-81). That comparison is pinned at the
/// unit level in `executor::tests`.
///
/// WHAT 4.2 Б1 ADDS: THE POSITIVE HALF OF THE LADDER. The driver samples the
/// marshal's own `finalized_height` gauge once per tick — a registry read, no
/// mailbox, so the Д-81 perturbation does not apply — beside a running count of
/// steps NAMED. That dates every rung, and lets the test say the thing the old
/// one could not: each named rung is REACHED by this node's own verified tip, and
/// within a bounded number of ticks of being named. It does NOT claim the step
/// CAUSED the tip to move — the third pass measured that here it mostly did not
/// (see "WHAT IS *NOT* ASSERTED" below), so this bound is a progress check and
/// the causal claim is left to the unit tests that can isolate it.
///
/// The fixture is the deep-lag one: node 3 leaves the committee for epochs 3–4 and
/// its EXECUTION stalls at `last(2) = 95`, so its marshal freezes at the ordering
/// plane's two-epoch ceiling and it climbs out a rung at a time. The re-jump gate
/// is production's own, `min(JUMP_THRESHOLD, interval)`, and that is load-bearing
/// rather than incidental: at the widened gate of 80 this fixture used to run
/// under, the reachable gap after 4.2 Б1.2 is bounded by the ceiling at
/// `2·interval = 64 < 80`, so the jump never arms, `T` never advances and the
/// probe names ONE rung for the whole run (measured: `steps=[(3, 159, 727)]`).
/// The Д3 precondition runs the same shape; this test does not duplicate it (it
/// asserts nothing about archives or landings).
///
/// WHAT IS ASSERTED: the probe named steps at all (the wiring is live and `T`
/// reaches it); the distinct steps STRICTLY INCREASE and there is more than one
/// (a ladder, not one rung repeated forever); node 3's own marshal tip ends at or
/// above the highest rung it named; EVERY named rung left this node as a
/// by-height pull and at least one came back SERVED; that at least one rung went
/// UNANSWERED and cost this node nothing (`deliveries_rejected == 0` — 4.2 Б2.6
/// п.1: a peer with no data never reaches `deliver`, so it is never excluded); and
/// every named rung is reached by that tip within [`LADDER_REACH_TICKS`] driver
/// ticks of the tick it was first named on.
///
/// WHAT IS *NOT* ASSERTED, and the third pass says it in the code rather than
/// only in the journal (review B1-03): that the ladder is what carries this node
/// out. Measured here — rung 159 is served, rungs 191 and 223 are addressed and
/// never answered inside the fetch bound, and node 3 reaches them anyway by
/// contiguous repair plus its two jumps (targets 128 and 160, neither of them a
/// named rung). A rung named two epochs above this node's `fin` runs into the
/// chain's own live edge, which is §5.4's "догон вместо прыжка" and not a defect;
/// but it does mean the causal claim "обслуженная ступень размораживает tip" has
/// only its unit-level witness here (`executor::tests`), not a stand-level one.
///
/// Falsifier: no step named (then `T` never reaches the probe and the run measures
/// nothing); one rung repeated for the whole run (then `T` is frozen and §5.2's
/// "новый `fin` ⇒ новый `T`" does not happen); a rung above the final marshal tip
/// (then the node named a height it never got); a rung that never leaves as a
/// by-height pull (then the naming is a log line); no rung served at all (then the
/// delivery half has no witness here either); every rung served (then the run holds
/// no unanswered-rung case to measure); a non-zero `deliveries_rejected` (then a
/// no-data answer is reaching `deliver` and honest peers are being excluded for
/// this node's own lag); a rung the tip never reaches inside the bound (then the
/// climb stopped).
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
    // DISTINCT steps with how many ticks named each — the raw list is one entry
    // per probe tick and says nothing the summary does not.
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

    // THE POSITIVE PIN. For each rung, the tick it was FIRST named on (the first
    // tick where the running count of named steps passes that rung's index in the
    // raw list) and the first tick at or after it where node 3's own marshal tip
    // stands at or above the rung.
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
    // One entry per DISTINCT rung, the first time it was named.
    reach.dedup_by_key(|(h, _, _)| *h);
    eprintln!("(4.2 Б1.6) rung -> (first named tick, first reached tick) = {reach:?}");

    // THE RUNG ON THE WIRE (review B1-03). `frontier_steps` says the probe NAMED a
    // rung; it says nothing about whether anything was asked for. `upstream_served`
    // is the client end of this node's own by-height pulls, so a rung that appears
    // there left this node as a real fetch. It does NOT separate the ladder step
    // from the marshal's ordinary contiguous repair — both are the same verb on
    // the same wire key, and the addressed form that would separate them was
    // measured and rolled back (`cert_inlet::UpstreamResolver::fetch_targeted`).
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
    // ...and at least one of them was actually SERVED. This is the weak half and
    // it is weak on purpose: MEASURED on this fixture, a minority of the rungs
    // comes back and the rest are asked for and never answered inside the fetch
    // bound — by the tick a rung is named the network has often not produced it
    // yet (the rung is up to two epochs above this node's `fin`, which on a
    // climbing node runs into the chain's own live edge). The node reaches them
    // anyway — by contiguous repair from the floor and by its two jumps, which is
    // exactly §5.4's "догон вместо прыжка". So what this run witnesses is: the
    // rung leaves the node and a served rung does arrive; it does NOT witness that
    // the ladder is what carries this node out, and no assertion here may pretend
    // otherwise. The per-rung numbers are printed above, not frozen here.
    assert!(
        rung_pulls.iter().any(|p| p.delivered),
        "no named rung was ever served on this fixture — then `deliver` ⇒ `store_finalization` \
         ⇒ `Update::Tip` has no live witness at all here: {rung_pulls:?}"
    );

    // (4.2 Б2.6 п.1) A PEER WITH NO DATA COSTS THE ASKER NOTHING. The unserved
    // rungs above are exactly that case, and it is the production default rather
    // than a byzantine role: `FrontierHandler::produce` drops its response channel
    // unsent when the local marshal has no pair for the key
    // (`plane_upstream.rs`, the `produce` impl), the resolver relays that as a
    // no-data answer, and `Consumer::deliver` is therefore NEVER CALLED — so the
    // refusal counter cannot move and commonware's `fetcher.block(peer)` (which only
    // a `deliver == false` reaches) never runs. The node keeps climbing: it names
    // and pulls strictly higher rungs after an unserved one, asserted above.
    //
    // Non-vacuity is asserted first: if every rung came back, this run would not
    // contain the case at all.
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

/// How many driver ticks a rung may take between being NAMED by node 3's probe
/// and being REACHED by node 3's own marshal tip, in
/// [`the_ladder_names_successive_rungs_and_the_lagging_node_reaches_every_one`].
///
/// MEASURED, not chosen, and a BOUND rather than a mechanism (review B1-03).
/// Observed on this fixture: rung 159 named at tick 961 and reached at 1592, rung
/// 191 at 1282 → 1909, rung 223 at 1603 → 2229 — deltas 631, 627, 626. The
/// near-constant delta was once read as evidence that naming CAUSES reaching; it
/// is not, and the third pass corrects that: node 3 climbs at a roughly constant
/// rate here, and a constant rate plus a constant naming cadence gives a constant
/// delta on its own. Two of the three rungs are never even served (see the test
/// doc), so most of that 631-tick delta is contiguous catch-up and two jumps.
///
/// What the bound is still worth: it fails if the climb STOPS after a rung is
/// named — a rung named and never reached is a node that stopped making progress.
/// It is not a latency target, and the ticks are driver ticks (≈ 8 per probe tick
/// on this fixture), not seconds.
const LADDER_REACH_TICKS: usize = 800;

/// Where the (C9) fixture cuts node 3 off, and how long for.
///
/// WHAT THE CUT IS FOR (5.1). The rotation alone no longer makes a node fall
/// behind: since П-3 the epoch key is an artifact any node may ASK a member for
/// over `BEACON_RESOLVER_CHANNEL` (R-121/R-122), and the tracked peer set is
/// `committee[E-1] ∪ committee[E] ∪ committee[E+1]`, so the node rotated out at
/// epoch 3 keeps its consensus links through epoch 3, fetches `PK_3` and stays in
/// lockstep (measured: `heights=[168, 168, 168, 168]`, `rejump_calls=[0, 0, 0, 0]`).
/// The lag is therefore a PHYSICAL cut of node 3 inside epoch 2 — after it has
/// dealt the epoch-2 ceremony, so its ceremony count is unchanged — held until the
/// network is a re-jump gate and more above it, which is what arms the jump these
/// two tests are about. It heals before `epoch_start(4) = 128`, where the epoch-5
/// ceremony opens: node 3 is a member of `committee[5]` and has to deal it.
const CUT_AT: u64 = 2 * EPOCH_LEN + 4;
/// The height the network reaches before the cut heals — `CUT_AT` plus more than
/// the re-jump gate (`min(JUMP_THRESHOLD, 32)`), and below `epoch_start(4)`.
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
/// has no σ source at all (`absent`'s `Beacon::seed` is `None`, `mandatory_at(2)`
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
    // both tiers (`primary = C[E−1] ∪ C[E] ∪ C[E+1]`, `secondary = registry`)
    // are a function of chain state alone, so a per-node difference means two
    // nodes read different committees for one epoch. Counted at the sink over the
    // MEMBERS, not their count — every set is the same SIZE under
    // `PeerSet::AllNodes`, so a length comparison here would be vacuous.
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
    let cfg = StandConfig::live(4, 1);
    // The SAME geometry the stand hands the committee module (`stand.rs`, the
    // `CommitteeStore` watch): reading it off the config rather than restating
    // `0` and `EPOCH_LEN` here is what keeps the epoch bound below honest if a
    // future case shifts the activation or the epoch length.
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
        // NON-VACUITY, by COVERAGE rather than by count. The bound used to be
        // `sum > 50`, which is what a run of this length costs when every
        // consumer issues its own `epoch_committee_snapshot` on every tick; the
        // committee module answers each epoch from ONE frozen record, so the
        // same run now costs ~17 reads (measured: `{0: 5, 1: 4, 2: 5, 3: 3}`)
        // and a threshold in the tens would be asserting the defect rather than
        // the property. What the bound was protecting is that the assertions
        // above are not vacuously true over an empty read log, so it says that
        // directly: every epoch this run passed through was actually read.
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

/// (C7) Zero committee overlap at a boundary is SURVIVABLE: each half acquires
/// the epoch key it had no part in minting, and the chain crosses the boundary.
///
/// # What this test used to say, and why it is inverted
///
/// It used to pin the OPPOSITE fact (R-121/R-122): "zero committee overlap at a
/// boundary halts the chain, and nothing detects it". Its assertions were the halt
/// itself — `timed_out`, the two park heights `[3·EL−1 ×4, 2·EL−1 ×4]`, each half
/// holding exactly ONE artifact (`[2]` for the outgoing, `[3]` for the incoming) —
/// and its falsifier was "any node crossing its park height". The mechanism it
/// recorded was I4: `PK_E` reaches a node that is not in `committee[E]` only as
/// `committee[E]`'s artifact, and nothing on the FRONTIER ever asked for one
/// (`drive_recompute` pulled for members only; the epoch manager's repair sweep
/// excludes `epoch >= frontier` by construction; the cert-inlet's per-certificate
/// `ensure_key` spends the network-free `PinEffort::Local`).
///
/// `DkgActor::acquire_mint_artifacts` is what closes it, and this test is the only
/// live witness of the closure. Its assertions are therefore the inverse of the old
/// ones, one for one:
///
/// | old (the halt) | new (the crossing) |
/// |---|---|
/// | `out.timed_out` | `!out.timed_out` — every node reaches past the boundary |
/// | park heights `[3·EL−1, 2·EL−1]` | `assert_lockstep_except(&[])` — one chain, all eight |
/// | `artifacts[0..4] == [2]`, `artifacts[4..8] == [3]` | every node holds BOTH `[2, 3]` |
///
/// # The two acquisitions are in OPPOSITE directions, and both are required
///
/// `committee[2] = [0,1,2,3]`, `committee[3] = [4,5,6,7]` — no member in common, so
/// neither half can serve itself the other's key:
///
/// - the INCOMING half (4-7) is `committee[3]`, so it dealt epoch 3's ceremony
///   during epoch 2 and holds epoch 3's artifact. It was never in `committee[2]`,
///   so it holds no epoch-2 key and used to park at the last height of epoch 1 —
///   the whole epoch it cannot verify lies BEFORE the epoch it minted for. It has
///   to fetch epoch 2's artifact BACKWARD, from the outgoing half.
/// - the OUTGOING half (0-3) is `committee[2]` and holds epoch 2's artifact. It is
///   not in `committee[3]`, so it has to fetch epoch 3's artifact FORWARD, from the
///   incoming half.
///
/// Only the second of the two is reachable through a window that stops at the
/// actor's current epoch; the first is why
/// [`acquire_mint_artifacts`](crate::beacon) reaches `now + 1`. A fix that closed
/// only the forward direction would leave this test red.
///
/// # The fake does not lie
///
/// A pulled artifact is checked by `verify_artifact` against `committee[minted_at]`
/// read from `FakeStaking` — the same check a validator applies to a peer's answer
/// in production. So "the key arrived" here means a `committee[E]` quorum
/// certificate verified, not that a fixture handed a value over.
///
/// Falsifier, in the new form: `timed_out` (the halt is back); a node holding only
/// one of the two artifacts (the acquisition works in one direction only); a
/// `SafetyHalt` (the crossing is a detected fault rather than a crossing); two nodes
/// deriving different σ for one height, or `diverged` (the halves crossed onto
/// DIFFERENT chains, which would be worse than the halt this replaces).
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
    // THE OUTGOING HALF RUNS THE PRODUCTION CERT-INLET, and that is a fixture
    // change with a reason rather than a knob turned until the test passed.
    //
    // A node that is not in `committee[E]` runs no engine for `E`, so its σ ingress
    // is not `spec_exec`'s notarization reporter — it is `CertInlet::ingest`, which
    // is what calls `observe_certificate` (recording σ) and `ensure_key` (resolving
    // `PK_E` out of the artifact) on a validator syncing plane-natively. The stand
    // had no inlet at all when the old version of this test was written, so on it
    // the outgoing half could hold `PK_3` and STILL park for want of σ — a property
    // of the fixture, not of the node. Measured that way: with the acquisition
    // landed but no inlet, every node holds both artifacts and the outgoing half
    // still sits at `3·EL−1`.
    //
    // The incoming half deliberately gets none: it is `committee[3]`, so its own
    // engine is its σ ingress, and giving it an inlet would blur which of the two
    // halves the acquisition is being read through.
    cfg.cert_inlet = Some(CertInletCfg {
        nodes: vec![0, 1, 2, 3],
        source: CertInletSource::NextAboveTier,
        tee: TeeWiring::Observed,
    });
    let out = Stand::new(cfg).run_until(reached(3 * EPOCH_LEN + 4), Duration::from_secs(400));
    // Printed BEFORE the first assertion on purpose: the height vector alone cannot
    // say WHICH of the two acquisitions failed, and the artifact map can.
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
    // ONE chain across all eight, not two that happen to be equally long.
    out.assert_lockstep_except(&[]);

    // Both artifacts on every node, which is the direct statement of the fix: the
    // half that minted neither `2` nor `3` does not exist here, so every entry
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

    // `prev_randao` is `H(σ)` of the height's own round, so equal σ at every height
    // IS byte-equal `prev_randao`. Asserted on the σ the executors actually derived
    // from, across the boundary the test is about, rather than inferred from the
    // executed hashes alone.
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
    // Non-vacuity: an all-`None` σ map would satisfy the loop above trivially, and
    // that is exactly what a run whose beacon never started looks like.
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

/// (C8) A rotated-out node DOES come back with the jump gate closed, because the
/// key it was missing is now acquired — R-122's other witness, beside C7.
///
/// # What this test used to say
///
/// It pinned the "before" half of the re-jump exit: with `re_jump_threshold` at
/// `None` the rotated-out node PARKED at the last block of epoch 2 and never came
/// back (`heights[3] == 3·EL−1`), holding exactly `artifacts[3] == [2]` — "the
/// parked node acquired an epoch key it has no path to". Its mechanism was R-122
/// read out of the code: the node enters epoch 3 as a verifier holding no artifact
/// for it, and nothing spent a network pull for the LIVE epoch's key — the epoch
/// manager's repair sweep excludes `epoch >= frontier` by construction, the
/// cert-inlet's per-certificate `ensure_key` is contractually network-free, and
/// `drive_recompute`'s pull is gated on membership.
///
/// `DkgActor::acquire_mint_artifacts` spends that pull. The park is gone, and the
/// assertions invert: the node holds BOTH epochs' artifacts and follows the
/// committee without a single re-jump. The re-jump gate stays closed, which is what
/// makes this a statement about the KEY and not about the jump: nothing here is
/// allowed to climb out by jumping.
///
/// The two halves of the old mechanism are told apart on purpose. It parked for
/// want of the KEY, not for want of blocks — the upstream assertion below was
/// already the proof of that and is kept unchanged, because it is what stops this
/// test from passing on a node that is simply being fed everything.
///
/// Falsifier: the node stuck at `3·EL−1` (the acquisition did not reach it); a
/// re-jump (it climbed out by the mechanism this fixture disables, so the claim
/// would be about the jump instead of the key); `artifacts[3]` missing epoch 3 (it
/// followed without the key, which would mean σ is not being checked at all); a
/// halt; the four disagreeing on a hash.
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
    // ONE chain across all four, the rotated-out node included — it is following,
    // not running its own.
    out.assert_lockstep_except(&[]);
    assert!(
        out.artifacts[3].contains_key(&3),
        "the rotated-out node followed WITHOUT epoch 3's artifact, which would mean \
         its certificates were admitted on the multisig half alone: {:?}",
        out.artifacts[3].keys().collect::<Vec<_>>()
    );
    // It is fed by the upstream plane, and it did NOT climb out by jumping: the
    // exit under test is the key. Kept verbatim from the "before" form — the same
    // two facts that stopped that test from pinning the wrong mechanism stop this
    // one from pinning the jump.
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

/// (C9, honest control) The stand's steady-state re-jump IS the production
/// `jump_to_target`, not a re-telling of it: the stand supplies only ONE seam
/// below it (`JumpElSync` for `RethElSync`), and the need-gate, the landing
/// selection and the landing check against the attested `block.result` are
/// production code.
///
/// WHAT THIS TEST ASSERTED BEFORE (4.2 Б2.4). Section (4) read
/// `Outcome::jump_committee_reads` and pinned that `verify_jump_authenticated`
/// RAN and read `committee[E]` AT the landing hash — the trustless post-sync
/// authentication. That stage is gone: the target of a steady-state jump is a pair
/// out of this node's own marshal archive, already 2f+1 under a committee it read
/// itself, so re-reading a committee here was a second opinion on a settled
/// question. Section (4) now records why the observation is no longer available,
/// and the recorder it used has been removed with the seam.
///
/// Everything asserted is OBSERVED rather than inferred, through
/// `Outcome::jump_calls` (every call, its `JumpOutcome` VARIANT, the certificate
/// it consumed and the landing it chose). On an honest frontier the production
/// function lands on exactly the pairs the retired hand-written model landed on —
/// heights 105 and 136 are kept as literals so a change in landing selection fails
/// HERE and not four tests away (they were 125 and 157 while the lag came from the
/// rotation's keylessness; 5.1 makes the lag a physical cut at a named height, so
/// the ladder starts from the cut instead of from `last(2)` and BOTH literals
/// moved — what did not move is the shape asserted on every call right below
/// them: `landing = tip − K`, the landing hash is the consumed certificate's
/// `result`, and the honest three executed that hash) — each landing hash is the `result` of the
/// certificate the call consumed and each landing is that certificate's `tip − K`,
/// and the honest three executed the same hash there.
///
/// Same stand as `three_boundaries_with_committee_rotation_keep_dkg_qual_honest`
/// (the rotated-out node 3 falls behind and the re-jump is what carries it), so
/// the two share a fixture but assert disjoint things: that one asserts the
/// RECOVERY, this one asserts the MECHANISM of the jump itself.
///
/// What this does NOT show: anything about a LYING upstream. Every upstream in
/// the stand serves genuine certificates, so both gates pass on real data — that
/// they REFUSE a forged target is untested here and stays a Э3.3 job.
///
/// Falsifier: any call whose outcome is not `Landed` (a jump that RAN and was
/// REFUSED is invisible otherwise — `ReJump::rotate` is `None`, so the executor's
/// rotation escape is a silent no-op); zero calls (the fixture stopped exercising
/// the jump); a landing pair the model did not produce; a landing hash that is
/// not the `result` of the certificate the call consumed, or a landing that is
/// not that certificate's `tip − K`; a landing the honest three did not execute;
/// a non-jumping node recording a call.
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

    // (1) EVERY call the production jump made LANDED. Without this the rest is
    // worthless: `ReJump::rotate` is `None`, so a refused jump rotates nothing and
    // logs one WARN — a chain where every jump authenticated and FAILED produces
    // the same `rejump_calls` as this one.
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

    // (2) It chose the landings the retired model chose, and each landing hash is
    // the `result` of the upstream CERTIFICATE the call consumed — the attested
    // pair, not a read-back of the chain the landing itself just wrote.
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

    // (3) The branch it landed on is the one the honest three executed — a jump
    // onto a fork would satisfy (2) just as well.
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

    // (4) The jump read NO committee. There is nothing to observe here any more
    // and that is the point: `jump_to_target` takes no `CommitteeSource` parameter
    // at all (pass Б2), so the absence of the post-sync authentication is a
    // COMPILE-TIME fact rather than a runtime one, and the stand no longer builds a
    // committee seam for the jump. What still makes the landing authentic is
    // assertion (2): each landing hash is the `result` of the archive certificate
    // the call consumed, and that certificate only entered the archive through
    // `store_finalization` after `verify_delivered`.

    // (5) Nobody else jumped, so no observation above can be coming from a node
    // that was never behind.
    for i in [0, 1, 2] {
        assert_eq!(out.upstream[i].rejump_calls, 0, "node {i} re-jumped");
        assert!(out.jump_calls[i].is_empty(), "node {i} called the jump");
    }
    eprintln!(
        "(C9) calls={calls:?} virtual={:?} real={:?}",
        out.virtual_elapsed, out.real_elapsed
    );
}

/// (PLAN 4.2, the replacement for R-003's test) A node three epochs behind
/// registers the schemes of the epochs it is behind and spawns an engine for
/// NONE of them — and which epoch is live is decided by its own VERIFIED tip.
///
/// The fixture is the (C8) rotation with one epoch changed: node 3 is out of the
/// committee for epoch 3 ONLY, so it misses `PK_3` and its execution parks at
/// `last(2) = 95` exactly as (C8) pins — but it IS a committee member of every
/// epoch above, including `epoch(fin) + 2 = 4`. That is what makes the
/// no-engine assertion non-vacuous on epoch 4: a node rotated OUT of it would
/// hold a verify-only scheme there whatever any gate said.
///
/// While it is parked its marshal keeps storing what it can VERIFY, up to the
/// ordering plane's two-epoch ceiling `last(epoch(95) + 2) = last(4) = 159`
/// (4.2 Б2 discards anything above the read window). So the state under test is
/// the one §5.1 names: this node's execution is in epoch 2 and its verified tip
/// says the live epoch is `5` — and `is_live_epoch` reads the second, not the
/// first, and not anything a peer said.
///
/// WHAT IS ASSERTED:
///   1. the tip really is at the ceiling, so the live epoch is 5 — without this
///      the run is just "a node that fell behind";
///   2. the schemes of the epochs between the park and the ceiling ARE
///      registered — the certificates that carried the tip there verified under
///      them, and they were registered by the read that verification needed
///      (the committee module is the marshal's `CertProvider`), with no
///      pre-registration span anywhere;
///   3. NO signer scheme for any epoch above the park, epoch 4 included, where
///      this node IS a member and CAN read the committee (the premise is stated
///      at (3) in the body, the property asserted at (5));
///   4. the live epoch itself is above this node's read window
///      (`epoch(anchor) + 2 = 4`), so it holds nothing at all for it;
///   5. the control, in the same run: node 3 IS a signer for exactly 0..=2 —
///      the epochs it reached while its own verified tip was inside them.
///
/// WHAT THIS TEST DOES NOT SEPARATE, and where that is done instead. Nothing
/// here is the liveness gate REFUSING: for epochs 4 and 5 node 3 takes no
/// decision at all — no edge offers them to `reconcile_roles` while its
/// execution is parked in epoch 2 — and even if one did, it holds no DKG share
/// for them, so the share gate would refuse a member too. What this test pins is
/// the layer BELOW the gate: which schemes a node three epochs behind ends up
/// holding, and that none of them is a signer. The gate as the SOLE refusal is
/// pinned by
/// `a_catching_up_member_takes_verify_only_at_every_boundary_below_its_own_tip`
/// (a member with a usable share crossing a boundary below its own verified
/// tip — `is_live_epoch_at → true` reds it), and the rule itself as a unit on
/// the production predicate in
/// `epoch_manager::tests::the_live_epoch_is_the_verified_tips_epoch_and_the_next_one_at_a_terminal`.
///
/// Falsifier: node 3 not parking at 95, or its tip not reaching the ceiling (the
/// fixture stopped producing the state); a signer scheme on node 3 for 3 or 4;
/// node 3 holding no scheme for epoch 4 (then (3) is vacuous — nothing was
/// registered to refuse); node 3 unable to READ committee[4] (then (3) is the
/// module refusing, not the role gate); a signer scheme missing for 0..=2 (the
/// gate refuses more than the epochs the tip has left).
#[test]
fn a_node_three_epochs_behind_registers_the_schemes_and_spawns_no_engine() {
    use commonware_cryptography::certificate::Scheme as _;

    let mut cfg = StandConfig::live(4, 1);
    // Node 3 leaves the committee for epoch 3 only — long enough to miss `PK_3`
    // and park, short enough to stay a MEMBER of `epoch(fin) + 2`.
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
    // WHAT HOLDS THE LAG (5.1). Non-membership no longer does: since П-3 the epoch
    // key is an artifact any node may ASK a member for over
    // `BEACON_RESOLVER_CHANNEL` (R-121/R-122), and on this schedule node 3 keeps
    // its consensus links for the whole of epoch 3 — the tracked set is
    // `committee[2] ∪ committee[3] ∪ committee[4]` — so it fetches `PK_3` and
    // follows the chain (measured: `heights=[192, 192, 192, 192]`). What is left
    // for "an execution cursor that cannot cross a boundary" is a node with no
    // consensus plane at all, and that is exactly the cut below: taken inside
    // epoch 2, so node 3 has already dealt and holds the shares of 0..=2 (the
    // control at (5)), never healed, and on the CONSENSUS plane only — its
    // frontier probe keeps feeding its marshal, which is what carries the tip to
    // the two-epoch ceiling at (1) while the execution stays at `last(2)`.
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
    // The cut is the fixture, so its own observation is a premise: no cut, no lag.
    assert!(
        !out.partitions[0].heights_at_cut.is_empty(),
        "the consensus-plane cut never fired, so nothing held node 3 back: {:?}",
        out.partitions[0]
    );

    // PREMISE (a): node 3's EXECUTION parked at the last block of epoch 2, three
    // epochs below the network.
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

    // (1) Its marshal kept VERIFYING past its own execution, up to the two-epoch
    // ceiling — the tip the live epoch is read off. `tip == last(4)` ⇒ epoch 4 is
    // finished ⇒ the live epoch is 5, three above the epoch this node executes in.
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

    // (2) REGISTERED, with no span: every epoch between the park and the ceiling
    // holds a scheme.
    for e in 3..=4 {
        assert!(
            module.scheme(e).is_some(),
            "node 3 holds no scheme for epoch {e}, so refusing an engine there proves nothing"
        );
    }

    // (3) PREMISE for the control in (5): epoch 4 is one this node IS a member
    // of and CAN read, so the absence of a signer half there is not the module
    // refusing the read. It is a premise and not the property — the property is
    // asserted in (5). And it is an ABSENCE OF A DECISION rather than a
    // refusal: with its execution parked in epoch 2 and the live epoch at 5, no
    // edge ever offers epoch 4 to `reconcile_roles` on this node, so the
    // liveness gate is not even asked. The gate's own refusal is pinned by
    // `a_catching_up_member_takes_verify_only_at_every_boundary_below_its_own_tip`
    // below, where it is the only thing that can say no.
    assert!(
        out.committee_records[3].get(&4).is_some_and(|r| r.is_ok()),
        "premise: node 3 must be able to read committee[4]: {:?}",
        out.committee_records[3].get(&4)
    );

    // (4) The LIVE epoch itself is above this node's own read window
    // (`epoch(anchor) + 2` = 4 at an anchor inside epoch 2), so it is refused
    // before any EVM call and holds nothing. This is why (3) is asserted on
    // epoch 4 and not on epoch 5.
    assert!(
        module.scheme(live).is_none(),
        "the live epoch is outside this node's read window and must hold nothing"
    );

    // (5) CONTROL: it signed exactly the epochs its own verified tip was inside.
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

/// (PLAN 4.2, the liveness gate ON ITS OWN) A node catching up after a partition
/// crosses the boundaries of epochs the network has already left, and takes a
/// VERIFY-ONLY scheme at every one of them — while it is a committee member
/// there, holds a usable share there, and holds the boundary block there.
///
/// This is the state the `a_node_three_epochs_behind…` test above does NOT
/// produce, and the one the rule exists for: the liveness gate is the ONLY
/// refusal on the path. The fixture removes the other two by construction —
/// `Committees::All` makes node 3 a member of every epoch, and
/// `StandConfig::honest` runs `StaticRandomness`, whose `can_participate` is
/// `Ready` for every epoch and whose `signer` always builds a signing scheme
/// (`beacon/surface.rs:849-851`, `:885-...`). The boundary block is in its
/// marshal because it is DERIVING through it. So for every epoch below its own
/// tip, `reconcile_roles` reaches the gate with nothing else able to say no.
///
/// The lag is made by a partition of the single node rather than by rotation:
/// a rotated-out node loses the epoch key with the seat, which is exactly the
/// second refusal this test must not have. The cut holds for long enough that
/// the three members cross two whole epochs without it (`epoch_len = 5`), and
/// the heal lets it catch up — its executor walks the missed range block by
/// block while its marshal already holds the network's certificates, so its own
/// boundary deliveries arrive with the live epoch (the epoch of its VERIFIED
/// tip) already above them.
///
/// WHAT IS ASSERTED:
///   1. the cut produced the state: at the heal node 3 was more than one epoch
///      behind the members (without this the run is not a catch-up at all);
///   2. it caught up — every node ends at the same height, so the missed
///      boundaries were CROSSED and reconciled, not skipped;
///   3. node 3 holds a scheme but NO signer half for every epoch it crossed
///      during the catch-up, while nodes 0..=2 hold the signer half for the
///      same epochs — the gate refused what nothing else could have;
///   4. node 3 IS a signer for the epoch its execution finally caught up in,
///      so the refusal is per-epoch and not a node that stopped signing.
///
/// Falsifier: node 3 not falling behind by an epoch (the partition was too
/// short — assertion 1); node 3 not catching up (then it never reconciled the
/// epochs in question); node 3 holding no scheme at all for a crossed epoch
/// (then (3) is vacuous — nothing was registered to refuse); node 3 holding the
/// signer half everywhere (the gate is not refusing); node 3 holding it nowhere
/// (something other than the gate stopped it — the share gate or the boundary
/// block).
#[test]
fn a_catching_up_member_takes_verify_only_at_every_boundary_below_its_own_tip() {
    use commonware_cryptography::certificate::Scheme as _;

    // 24 and not the stand's usual 32: the shortest epoch that keeps the
    // staking transition's single-park invariant (`interval > MAX_PENDING_ACKS
    // + K` = 16 + 3, `staking-reader/src/epoch_transition.rs:421-431`), so the
    // catch-up burst below cannot have two boundaries parked at once.
    const LEN: u64 = 24;
    let mut cfg = StandConfig::honest(4, 1);
    cfg.epoch_len = LEN;
    cfg.marshal_tip_series = true;
    let mut stand = Stand::new(cfg);
    // Cut inside epoch 0 and hold it for longer than a whole epoch, so the
    // members are two epochs ahead at the heal and node 3's catch-up crosses a
    // boundary the network left long before.
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

    // (1) PREMISE: the cut left node 3 more than a whole epoch behind.
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

    // (2) PREMISE: it caught up, so the missed boundaries were crossed.
    assert_eq!(
        out.heights[3], out.heights[0],
        "node 3 did not catch up: {:?}",
        out.heights
    );

    // (3) The epochs it crossed while its own verified tip was already past
    // them: from the epoch it was in at the heal up to the epoch the members
    // were in at the heal, exclusive of the latter (the one it caught up INTO).
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

    // (4) CONTROL on the same node: it signs the epoch it caught up in.
    assert!(
        signer3.iter().any(|e| *e >= ahead_at_heal),
        "node 3 never re-promoted after the catch-up, so (3) is a node that stopped signing \
         and not a per-epoch gate: signer3={signer3:?}"
    );
}

/// (5.3-В) A REGISTRY-TIER neighbour's dealing is refused at every member's
/// pre-decode gate, exactly as many times as it was sent, and the ceremony
/// completes for everyone regardless.
///
/// # What this pins
///
/// Since 5.3-В the ONE sender classification on the `BEACON_CHANNEL` is the
/// channel's `GatedReceiver` over the peer set the node's own transition last
/// registered (`stand.rs`, `GatedReceiver::new(bcr, window, "beacon", true)` —
/// the production wiring of `node/src/dpos.rs`); the actor holds no membership
/// opinion of its own, and the seat a sender holds in a frame's epoch is the
/// consumer's check (`no_seat`). This run is the first in which the stand
/// EXERCISES that gate: before 5.3-В the stand built neither a window nor a
/// gate, so a registry-tier sender's frames reached every actor's decode.
///
/// # The fixture
///
/// Five nodes, `committee[E] = [0, 1, 2, 3]` for every `E` (`n = 4`, `f = 1`),
/// `PeerSet::AllNodes` — so node 4 is in the registry every member tracks as
/// its SECONDARY tier and in no committee record. Node 4 is the
/// `Role::StrayDealer`: once a second it broadcasts a real `Commitment` dealing
/// for the epoch after its own to everyone. Each member's gate classifies the
/// sender `Tracked` and refuses the frame as `secondary`. The epoch-2 ceremony
/// (the deterministic bootstrap, dealt during epoch 1) runs under that fire.
///
/// # What is asserted
///
///   1. the stray put frames on the wire (`stray_dealer_sends[4] > 0`, every
///      other node `0`);
///   2. the beacon channel refused EXACTLY that many frames as `secondary`,
///      and nothing as `untracked` — the count is attributed to the stray by
///      construction (it is the only sender any gate can refuse: every other
///      node is a member of every record) and CHECKED by the exact equality,
///      which a stray refusal from anywhere else would break;
///   3. nothing reached a consumer that had to refuse it: `no_seat = 0`;
///   4. every member holds epoch 2's artifact, its σ at `epoch_start(2)`
///      verifies under `PK_2` on all of them, and all four dealers are in it;
///   5. no halt, no divergence, one chain.
///
/// Falsifier (M1 of 5.3-В round 2): the stand's `GatedReceiver` removed — the
/// stray's frames reach every actor, `secondary` reads `0` against a positive
/// send count, and the consumer's `no_seat` moves off zero.
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
    // BOTH labels: the family is shared by every gated channel (R-070), so a
    // `reason`-only sum would read another channel's refusals as the beacon's.
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

    // (1) The stray dealt, and nobody else strayed.
    let sent = out.stray_dealer_sends[4];
    assert!(sent > 0, "the stray dealer put nothing on the wire");
    assert!(
        out.stray_dealer_sends[..4].iter().all(|s| *s == 0),
        "{:?}",
        out.stray_dealer_sends
    );

    // PREMISE of (2): node 4 is what every member's transition tracked as
    // SECONDARY (the active registry) and never as PRIMARY (a committee record)
    // — the classification `Tracked` ⇒ the refusal `secondary`. Asserted on the
    // registrations themselves, so the exact label below is a consequence of a
    // premise this run states and not of a fixture detail it silently relies on
    // (a `PeerSet` that dropped node 4 from the registry would make the same
    // frames `untracked`, and this is where that would be said).
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

    // (2) Every one of its frames was refused at a member's gate as `secondary`,
    // and nothing else was.
    assert_eq!(
        (secondary, untracked),
        (sent, 0),
        "the gate refused {secondary} frames as `secondary` and {untracked} as `untracked` \
         against {sent} sent by the stray — the gate is not the one sender classification \
         on the channel"
    );

    // (3) No consumer had to refuse a seatless frame: the gate is in front of it.
    assert_eq!(no_seat, 0, "a stray frame reached a consumer");

    // (4) The key: minted, with every dealer, and the σ under it on every member.
    let artifact2 = artifact_on_every_node(&out, &members, 2);
    let pk2 = pk_of(artifact2);
    seed_agreed_at(&out, &members, 2 * LEN, 2, &pk2);
    assert_eq!(
        dealers_of(artifact2),
        4,
        "a dealer is missing from epoch 2's artifact"
    );

    // (5) One chain.
    out.assert_lockstep_except(&[]);
}

/// The BEACON channel's refusals under `reason` in a drained recorder — both
/// labels, because `dpos_ingress_dropped_total` is one family for every gated
/// channel (R-070) and a `reason`-only sum would count another channel's.
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

/// (5.3-В, third round) A committee member the chain has TOMBSTONED is refused
/// at every other member's pre-decode gate as `untracked` — the tombstone
/// predicate over the node's own `TombstoneSet`, which the stand now fills the
/// way production does (`TombstoneSet::observe` over the snapshot read at every
/// finalized height) — and the ceremony completes without it.
///
/// # The fixture
///
/// Five nodes, `committee[E] = [0..5]` for every `E` (`n = 5`, `f = 1`),
/// `PeerSet::AllNodes`. Node 4 is reported tombstoned by the contract from
/// height 4 on (`StandConfig::tombstoned`), i.e. a whole epoch before the
/// epoch-2 ceremony deals (during epoch 1, from `epoch_start(1) = 32`). Node 4's
/// own beacon plane runs as an honest member's: it deals, acks, reveals and
/// confirms for epoch 2 on the `BEACON_CHANNEL`, and every one of those frames
/// is classified `Dropped` at the four other gates before a byte is decoded.
/// Its acks never reaching a dealer means each dealer reveals node 4's share in
/// its log — ONE reveal, within `f` — and its own dealing, acked by nobody, does
/// not seal. Production severs the peer's transport on top of this; the stand
/// does not model that (`BlockerSpy` counts, the simulated network delivers), so
/// node 4 keeps following the chain and the run stays in lockstep.
///
/// # What is asserted
///
///   1. PREMISE: every member's set observed node 4 before the ceremony dealt
///      (`Outcome::tombstones_observed`, a height below `epoch_start(1)`) — the
///      gate had the verdict when the frames came;
///   2. the beacon channel refused frames as `untracked` and none as `secondary`
///      (node 4 is in every committee record: only the tombstone can make it
///      `Dropped`, and `Dropped` is `untracked`), and no consumer had to refuse
///      one (`no_seat = 0`);
///   3. every member holds epoch 2's artifact, its σ at `epoch_start(2)`
///      verifies under `PK_2` on all of them, and the artifact pins exactly the
///      four dealers `[0, 1, 2, 3]` — node 4's seat is NOT among the pinned
///      logs, because no member ever took its dealing in;
///   4. no halt, no divergence, one chain.
///
/// Falsifier (M1 of 5.3-В round 3): the stand's `TombstoneSet` left unfilled
/// (`observe` removed from the boundary feed) — node 4 is a window `Member`
/// again, `untracked` reads 0 and its dealing is pinned as a fifth log.
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
        // The OTHER consumer of the same set — the refuse-to-bind gate
        // (`application.rs`, `verify_block`) — as a diagnostic: the stand's set
        // now feeds it too.
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

    // (1) PREMISE: the verdict reached every member's set before the ceremony.
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

    // (2) Refused as `untracked` — the tombstone's classification — and nothing
    // reached a consumer.
    assert!(
        untracked > 0,
        "no beacon frame was refused as `untracked`: the tombstoned member's dealing got \
         through every gate"
    );
    assert_eq!(secondary, 0, "a committee member cannot be `secondary`");
    assert_eq!(no_seat, 0, "a tombstoned member's frame reached a consumer");

    // (3) The key: minted by the four others, and the σ under it on every member.
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

    // (4) One chain — the tombstoned node follows it too.
    out.assert_lockstep_except(&[]);
}

/// `committee[E]` is peer-key ASCENDING (`commitEpochCommittee` sorts it), so a
/// node's seat is the position of its peer key in the sorted set. Derived from the
/// stand's own key schedule, which is a function of the seed alone.
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

// ---------------------------------------------------------------------------
// Э3.3 — the byzantine beacon / certificate roles (register entries R-002, R-008)
//
// Each test states what the register PREDICTED, carries BOTH branches of that
// prediction as mutually exclusive assertions, prints which one the run took, and
// asserts the one that was observed. Every one first asserts that its wrapper
// actually tampered: a green branch over a wrapper that swapped nothing would
// state nothing about production at all.
// ---------------------------------------------------------------------------

/// (R-002) Node 1 deals TWICE: every member but node 0 receives the log its
/// `seal_dealings` broadcast, node 0 receives a second, independently dealt and
/// validly signed log of the same dealer over the same `Info`.
///
/// **What R-002 predicted, and what the run showed until 2026-09-14:** the
/// confirms are hash-sensitive, the agreement pins the majority's hash, and the
/// victim's `all_held` was false FOREVER — no refetch, because the dealer was
/// already in a per-DEALER `recorded` set (`ceremony.rs::record_checked_log`,
/// first-wins) and the resolver key was `{epoch, dealer}` with no way to name the
/// OTHER body. Node 0 finished epoch 2 with `dkg_ceremony_ok = 0` and
/// `epoch_engine_demoted_no_polynomial = 2`: it knew `PK_2`, it held no share
/// under it (branch (b), REPRODUCED 2026-09-09).
///
/// **What the run shows now (5.3 заход Б, log identity `(dealer, hash)`):** the
/// victim records the forged log under `(dealer, h2)`, the artifact pins
/// `(dealer, h1)`, `fetch_missing_logs` asks the roster for exactly `(2, dealer,
/// h1)` (`beacon/actor.rs`, by pinned hash), a peer serves that body and nothing
/// else (`serve_log`, exact `(dealer, hash)`), the victim records it as the
/// dealer's SECOND valid log — the equivocation pair is journaled as evidence, the
/// dealer is locally banned from gossip, one WARN line names both hashes — and the
/// victim finalizes over the pinned set like everyone else. Branch (a): the victim
/// keeps its share, and the split costs the network nothing.
///
/// **Which branch would be vacuous.** Branch (a) is what a COMPLETELY HONEST run
/// also produces, so it would pass over a wrapper that swapped nothing. That is
/// why the tamper's witness runs first and why one of its parts is the victim's
/// OWN `ShareConfirm`: it names the forged hash at the dealer's seat, which no
/// honest run can — and the victim's confirmation KEEPS naming it after the
/// refetch (`signed_log_hash` is the FIRST-recorded hash, and the published index
/// is first-wins per seat), so this witness stays live. The second witness is the
/// evidence itself: exactly one node counted an equivocation, and its WARN line
/// carries the two hashes the wrapper reported.
///
/// Falsifier: the wrapper not swapping (`reveals_swapped == 0`), the two logs
/// hashing equal, either log failing the receiver's own `check`, the victim's
/// confirmation not naming the forged log; a victim WITHOUT a share (branch (b) —
/// then the pinned body was not refetched by hash and R-002 is back); any node
/// demoted; an equivocation counted on a node other than the victim (the wrapper
/// sent the second log to one node only); a halt or an ERROR line.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn a_two_log_dealers_victim_refetches_the_pinned_log_and_keeps_its_share() {
    let mut stand = Stand::new(StandConfig::live(4, 1));
    stand.node(1).role(Role::TwoReveals {
        withhold_partials: false,
    });
    let out = stand.run_until(reached(72), Duration::from_secs(200));

    // (1) THE TAMPER'S OWN WITNESS, before anything about the reaction.
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
    // The split reached `DkgCeremony::record_checked_log` on the VICTIM, and this is
    // an observation rather than an inference from the share it ends up with: a
    // `ShareConfirm` is minted from `recorded_dkg_logs` alone
    // (`beacon/confirmations.rs::mint`), and the only writer of the hash at ANOTHER
    // member's seat is `record_checked_log` (`ceremony.rs`). The ceremony's other
    // two writers of the recorded set cannot produce this entry: `seal_dealings`
    // files only the node's OWN log, and the journal resume replays records those
    // two wrote — and no node restarts here. The refetched pinned body does not
    // move it either: the index publishes the FIRST-recorded hash per seat.
    //
    // Scoped to the epoch the confirmation was framed under, not to "the last one":
    // a longer run mints one per target epoch and the last would then be a property
    // of the run's length.
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

    // (2) The two branches of the prediction, mutually exclusive by construction.
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

    // (3) The evidence: the victim, and only the victim, proved the equivocation —
    // it is the one node that ever held both bodies — and said so once, naming the
    // hashes the wrapper reported (first = the forged log gossip delivered, second =
    // the pinned original the resolver fetched).
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

    // (4) The observed branch, in full: nobody lost anything. Most of what follows
    // an HONEST run also satisfies — it carries "and the chain goes on", not "the
    // tampering worked"; the load-bearing lines are the victim's `dkg_ceremony_ok`
    // and the evidence above.
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
    // Every node knows the KEY and signs under it: the seeds of the epoch are agreed
    // on all four, the victim included.
    let pk = pk_of(artifact_on_every_node(&out, &[0, 1, 2, 3], 2));
    for h in 2 * EPOCH_LEN..=*out.heights.iter().min().unwrap() {
        seed_agreed_at(&out, &[0, 1, 2, 3], h, 2, &pk);
    }
}

/// (R-002, second link) The same two-log dealer, now also withholding its seed
/// partial: its signer scheme is rebuilt over the epoch's VERIFY-ONLY oracle, so
/// `SeedOracle::sign_partial` answers `None` (`beacon/oracle.rs:172-189`) and
/// `CombinedScheme::sign` therefore casts NO VOTE at all
/// (`bls/src/combined_scheme.rs:284-287`).
///
/// **What R-002 predicted, and what the run showed until 2026-09-14:** with the
/// victim shareless, the remaining honest signers were `n − 1 − f = t − 1`, so
/// every seed needed the byzantine dealer's partial and withholding it meant no
/// certificate of the epoch could be assembled — all four nodes stopped at 63,
/// silently (branch (b1), REPRODUCED 2026-09-09). ONE byzantine dealer had
/// disabled a SECOND, honest node, and four minus two is below `quorum(4)`.
///
/// **What the run shows now (5.3 заход Б):** the second link is gone with the
/// first. The victim refetches the pinned body by hash and holds its share (the
/// test above), so the honest signers are `n − 1 = 3 = quorum(4)`: nodes 0, 2
/// and 3 attest with their partials, the withholding dealer is ONE silent node —
/// `f`, not `f + 1` — and the chain crosses 64 and runs to 72 with every seed of
/// epoch 2 agreed on every node. Branch (b2). The stop was never a property of
/// the withholding; it was the price of the shareless victim.
///
/// **What this does NOT say, and the old (b2) wording got wrong.** Crossing 64
/// here does not mean a vote with no seed partial was counted: the three votes
/// that form each quorum all carry partials (`verify_attestation` rejects a
/// beacon-epoch vote with `seed: None` whole), and the withholding node casts no
/// vote at all. The seed threshold and the multisig quorum still cannot be
/// separated by any run — `assemble` computes it as `M::quorum(participants)`
/// (`bls/src/combined_scheme.rs:387-388`) — and three honest partials meet both.
///
/// **Which branch would be vacuous.** Branch (b2) is what an honest run produces,
/// so it would pass over a wrapper that withheld nothing;
/// `withhold_probe == Some((true, false))` and `schemes_withheld >= 1` are what
/// rule that out, and the victim's `dkg_ceremony_ok == 1` is what ties the verdict
/// to the refetch rather than to a quorum the withholding node joined.
///
/// Falsifier: `withhold_probe != Some((true, false))` (then the node had no
/// partial to withhold and the crossing says nothing); a stop at 63 (branch (b1)
/// — then the victim is shareless again and one dealer disables two nodes); a
/// halt latch or an ERROR line; a victim without a share.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn a_two_log_dealer_that_also_withholds_its_partial_is_one_silent_node_within_f() {
    let mut stand = Stand::new(StandConfig::live(4, 1));
    stand.node(1).role(Role::TwoReveals {
        withhold_partials: true,
    });
    let out = stand.run_until(reached(72), Duration::from_secs(200));

    // (1) Both tampers' own witnesses.
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

    // (2) The two branches.
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

    // (3) The observed branch, in full: a chain that runs on three signers.
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    assert_eq!(out.diverged, None);
    out.assert_lockstep_except(&[]);
    seedless_on_every_node(&out, &[0, 1, 2, 3], 1..2 * EPOCH_LEN);
    // The victim minted for the same reason as in the test above — the pinned body
    // reached it by hash — and so did everyone else: what is withheld is one
    // PARTIAL, and three remain.
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

/// The (R-008) schedule: `[0, 1, 2]` is the committee of EVERY epoch, so nodes 3
/// and 4 are pure followers — never members, and therefore never holders of `PK_2`
/// by their own ceremony. They reach the chain through the upstream plane alone,
/// the (4c) class.
///
/// WHY THEY ARE NEVER MEMBERS (5.1), where they used to leave at epoch 2. The
/// keyless window this fixture is about has to be CUT open now, and the cut has to
/// land before the followers' DKG clock enters epoch 1: that is when
/// `acquire_mint_artifacts` starts pulling the epoch-2 artifact for a NON-MEMBER
/// (`actor.rs`, `lo..=now + 1`), the artifact exists within the first block of the
/// epoch (the ceremony is message-driven, not height-driven), and one pull is all it
/// takes. So the cut must be in place inside epoch 0 — and a cut that early must not
/// cost the chain its quorum, which it does as long as the two are members of
/// `committee[0]` (measured: a cut at height 28 with the old schedule froze every
/// node at 28, three of five cannot finalize; a cut at height 34 with the two
/// dropped from epoch 1 came too late — at the heal they stood at 92 against the
/// committee's 96, i.e. they had `PK_2` and never fell behind).
///
/// Nothing about `PK_2` changes: the committee never changes, so the only mint is
/// the epoch-2 bootstrap one and `minted_at(E) = 2` for every epoch of the run.
#[cfg(feature = "dpos-devnet-byzantine")]
fn the_first_three_are_the_committee() -> Committees {
    Committees::Schedule(Arc::new(|_epoch, _n| Some(vec![0, 1, 2])))
}

/// (R-008) The three committee members serve `Finalized{h}` over the frontier
/// plane with the σ slot of the certificate replaced, for `h` in
/// `byzantine_roles::FORGE_WINDOW`. The planted σ is a REAL σ of another round of
/// the same epoch (harvested from the answer for 64), so it is a valid G1 point
/// that cannot verify for the round it is planted into; the multisig half is
/// re-encoded byte-identically.
///
/// **What R-008 predicted** (`.dpos-study/REGISTER.md`, R-008): `verify_certificate`
/// under `SeedCheck::NoKey` accepts any σ, so a follower with no `PK_E` puts the
/// forged certificate in its archive and HOLDS the σ; when the key lands the
/// σ is refused as `Invalid` and dropped, and the archive keeps serving the
/// forgery to other nodes, which see a data fault and rotate away from the honest
/// follower. `record_data_fault` is never called, so the follower does not rotate
/// away from the LYING upstream.
///
/// **What the run showed (2026-09-09): branch (a) — REPRODUCED, whole, including
/// the archive poisoning.** Node 0 forged the six certificates 65..70. Nodes 3
/// and 4 rejected NOTHING (`deliveries_rejected == 0`) and counted the keyless
/// admission (`dpos_seed_verify_no_key_total` non-zero). Both later obtained
/// `PK_2` and the settle refused exactly the six forged rounds
/// (`beacon/seed_index.rs::settle_epoch`; the ERROR text was
/// "quarantined seed does not verify" before row 5.2 renamed the state to
/// `Pending`), one ERROR line each, six per node — the rounds
/// are `(2, view h − 63)` for exactly the six forged heights, which is what ties
/// the refusal back to THIS wrapper's bytes. Neither node ever derived 64..70:
/// what carried them forward was the production re-jump (EL sync), not the σ.
/// And node 3 SERVED the forged certificates on to node 4 — its
/// `served_seed_replays` names the heights at which it handed out a σ it had
/// already served under a different round, which an honest archive cannot do
/// because σ is unique per `(round, PK)` (`beacon/seed.rs`).
///
/// The re-jump gate is production's own and is load-bearing here rather than
/// decorative: WITHOUT it the follower parks at 63 forever and never acquires
/// `PK_2` at all, because the key-repair sweep only considers epochs STRICTLY
/// BELOW its scheme frontier (`epoch_manager.rs:1677`) and its frontier is driven
/// by boundary deliveries its own parked executor never produces. That run is
/// recorded in the session journal as "the path is not reached", not as "not
/// reproduced".
///
/// **That `verify_certificate` returned TRUE in the `NoKey` branch — not that the
/// certificate was admitted some other way — is pinned by three observations
/// together.** The epoch-2 oracle WAS attached on the follower: the two live
/// callers of `SeedOracle::verify_seed` are `CombinedScheme::verify_certificate`
/// and `VerifiedSeed::check`, and since row 5.2 folded the follower's provider
/// into `LiveBeacon` there is ONE implementation that bumps
/// `dpos_seed_verify_no_key_total` — `BeaconOracle` (`beacon/oracle.rs`) — which
/// is the one this stand builds.
///
/// **What row 5.2 did NOT change here, said out loud.** The refusal is still one
/// ERROR line per refused ROUND, and it has to be: the per-EPOCH latch bounds the
/// SYNCHRONOUS refusal, which is judged per certificate (~1/s for the life of the
/// epoch), while the settle judges each held round exactly once and then drops it.
/// This test's `refused_heights == forged_sorted` equality rests on that, and it
/// is why the two refusal lines are different lines. What 5.2 DID add is a
/// consumer for the machine-readable half (`DataFault` ⇒ the inlet's rotation),
/// which these nodes do not run — `cfg.cert_inlet` is unset here, so nothing takes
/// the `faults()` receiver and nothing is queued behind it. Nothing was
/// rejected at the resolver (`deliveries_rejected == 0`). And the follower later
/// SERVED those very certificates on — a certificate the marshal refused would not
/// be in its archive to serve; the seed slot is checked on EVERY certificate
/// rather than batched away, because `CombinedScheme` overrides only
/// `verify_certificate` and the batch default calls it per item
/// (`CW:cryptography/src/certificate.rs:284-303`).
///
/// **Which branches would be vacuous:** branch (b)'s `deliveries_rejected == 0`
/// (the resolver counts a rejection only on bytes that fail to DECODE — there is
/// no crypto on that path) and branch (c)'s `keyless > 0.0` (a node outside
/// `committee[2]` holds no `PK_2` yet, so an honest run counts keyless admissions
/// too) both hold on an honest run. What carries the claim is
/// `!refusals.is_empty()` and the equality between the refused rounds and the
/// forged heights.
///
/// Falsifier: the wrapper forging nothing, forging a σ that reads back equal to
/// the original, or touching the multisig half; `deliveries_rejected > 0` on a
/// follower (branch (b) — then the `NoKey` admission is not what the entry
/// describes); no promote refusal at all together with a zero keyless-admission
/// count (branch (c) — the path was not reached); a refusal naming a round the
/// wrapper did not forge; a follower deriving one of the forged heights (then a
/// forged σ reached the executor, which would be worse than R-008 says); the
/// three members losing lockstep.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn a_forged_seed_slot_is_admitted_with_no_key_and_refused_when_the_key_lands() {
    /// Inside epoch 0 and long before the epoch-2 ceremony seals — see
    /// [`the_first_three_are_the_committee`] for why it has to be this early and why
    /// that is free of quorum cost.
    const CUT_AT: u64 = 4;
    /// The height the committee reaches before the cut heals — well above the forge
    /// window, so the forged certificates are taken while the followers are keyless,
    /// and far enough below the stop height that the key has time to land and the
    /// promote refusals to be logged.
    const HEAL_ABOVE: u64 = 3 * EPOCH_LEN;
    let mut cfg = StandConfig::live(5, 1);
    cfg.committees = the_first_three_are_the_committee();
    // Every CONSENSUS-plane link left in place BY THE PEER SET, so the two outsiders
    // can still acquire `PK_2` after the fact — the half of R-008 that only exists
    // once the key lands. What takes those links away for a bounded window is the cut
    // below, not this. (Only the consensus plane: `upstream_source_only_for` below
    // cuts node 4's upstream links down to one, see there.)
    cfg.peer_set = PeerSet::CommitteeTrackedOnly;
    cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD.min(EPOCH_LEN));
    // THE ARCHIVE-POISONING SEAM (4.2 Б1.3). The last assertion here is that a
    // follower which accepted a forged certificate SERVES IT ON. That needs node 4
    // to ask node 3 for one of the forged heights — and until this line, which
    // peer answered node 4's by-height pull was the resolver's shuffle
    // (`resolver/src/p2p/fetcher.rs:233-245`), not the fixture. It happened to be
    // node 3 while the ladder step was gated on `servable`, and happened to be a
    // committee member once that gate went (§5.2, review A2-01): same property,
    // different luck — measured, `served_seed_replays` went from `[66..=70]` to
    // empty while the five heights were served by the forgers instead (their
    // `forged_heights` went from 6 entries to 10).
    //
    // So the fixture now SAYS it: node 4 pulls by height from node 3 and from
    // nobody else, while node 3 keeps every link it had (it is the one that must
    // hold a whole poisoned chain to hand on). The property is unchanged — a
    // poisoned archive serves the forgery — and it is no longer decided by timing.
    //
    // WHAT THE SEAM COSTS, named because it is a narrowing (review B1-11): the run
    // no longer witnesses that a poisoned follower is REACHED in an ordinary mesh;
    // it witnesses what happens once it is. Measured BY THE REVIEW (not by this
    // pass) with the seam removed: the
    // primary half of R-008 survives intact (`branch = (a) REPRODUCED`,
    // `keyless=310`, `refusals=10`) and only the last, secondary assertion reddens,
    // with `serve_requests` per node `[91, 70, 840, 43, 44]` — node 2 (a forger)
    // answers node 4's by-height pulls instead of node 3. So the seam fixes the
    // draw, not the property.
    cfg.upstream_source_only_for = Some((4, 3));
    let mut stand = Stand::new(cfg);
    for i in 0..3 {
        stand.node(i).role(Role::ForgedSeedUpstream);
    }
    // WHAT OPENS THE KEYLESS WINDOW (5.1). Non-membership no longer does: since П-3
    // a non-member ASKS a member for the mint's artifact over
    // `BEACON_RESOLVER_CHANNEL` (R-121/R-122), and `acquire_mint_artifacts` issues
    // that pull an epoch AHEAD of the need (`lo..=now + 1`), so both followers hold
    // `PK_2` before the first block of epoch 2 and never fall behind — measured, the
    // forge window is then never asked for at all (`certs_seen: 1,
    // certs_forged: 0`, every node at 194). A CONSENSUS-PLANE cut
    // (`CutPlanes::ConsensusOnly`) is what leaves them without that pull while their
    // frontier probe keeps feeding their marshal: they stand at `last(1) = 63`, walk
    // the forge window by height off the forgers, admit it with NO key — and when the
    // cut heals they acquire `PK_2` and the second half of R-008 runs exactly as
    // before. The cut is the fixture, so its own observation is asserted below.
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

    // (1) THE TAMPER'S OWN WITNESS.
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

    // (2) The three branches. `followers` are the two nodes outside `committee[2]`.
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

    // (3) The observed branch, in full.
    out.assert_lockstep_except(&followers);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    // The ONLY ERROR lines this run may carry are the promote refusals.
    for line in out.errors() {
        assert!(
            line.text.contains("held seed does not verify"),
            "unexpected ERROR line: {line:?}"
        );
    }
    // Every refused round is a height THIS wrapper forged: view = h − (2·L − 1).
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
/// committee at epoch 3 (parks at the last block of epoch 2, `3·EPOCH_LEN − 1`),
/// the production re-jump gate is on, and node 0's ONLY frontier source is node 3
/// (`upstream_only_link = (3, 0)` — the consensus plane is untouched, only frontier
/// discovery is confined to the liar). The two runs of each test differ ONLY in
/// node 3's role, so the control is IN-TEST, not an inferred contrast against C9.
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
    // R-004 reads every node's MARSHAL TIP per tick (§5.2's one frontier).
    cfg.marshal_tip_series = true;
    let mut stand = Stand::new(cfg);
    stand.node(3).role(role3);
    // WHAT MAKES NODE 0 FALL BEHIND (5.1), and why it is a cut rather than the
    // rotation. The rotation used to leave it without `PK_3`: it is not in
    // `committee[3]`, and nothing fetched a non-member's artifact. Since П-3 it ASKS
    // for that artifact over `BEACON_RESOLVER_CHANNEL` (R-121/R-122) — an epoch
    // AHEAD of the need (`acquire_mint_artifacts`, `lo..=now + 1`) — so it crosses
    // the boundary in lockstep and the CONTROL run spawns no re-jump at all
    // (measured: `calls=[]`, every node at 192). The lag is therefore physical: node
    // 0 is cut off at `last(2)`, the height it parked at before, and the cut heals a
    // gate-and-a-half later so production's own re-jump is what carries it back.
    //
    // It is the CONTROL run this restores. The ROLE runs park node 0 at `last(2)`
    // either way, and for the reason the test names — its only frontier source is the
    // liar, `deliver` refuses the forged `Latest`, commonware excludes the peer, and
    // nothing ever TELLS this node a height above its own boundary exists (its
    // consensus-plane peers cannot: it has no engine for `committee[3]`, so no
    // epoch-3 notification is addressed to it). The cut leaves that untouched: it
    // fires at the height node 0 stops at anyway.
    stand
        .partition(&[1, 2, 3], &[0])
        .after_height(3 * EPOCH_LEN - 1)
        .heal_above(4 * EPOCH_LEN + 16);
    stand.run_until(
        move |p| p.min_height_of(&[1, 2, 3]) >= 6 * EPOCH_LEN,
        Duration::from_secs(400),
    )
}

/// (R-004, INVERTED by the frontier trust gate) Node 3's frontier-plane
/// `Producer` answers `FrontierKey::Latest` with the real tip whose `block.height`
/// is inflated by `byzantine_roles::LATEST_INFLATION = 10^6` (payload re-pointed so
/// the payload↔digest bind still passes; `result` and the multisig UNCHANGED).
/// Node 0 is rotated out at epoch 3 and its ONLY frontier source is node 3
/// (`upstream_only_link`).
///
/// **What this test asserted before, and what it asserts now.** It used to pin the
/// DEFECT: the inflated height reached `ReJump::upstream_frontier`
/// (`peak == inflate_to == real + 10^6`), the deep-gap trigger fired, and every
/// re-jump came back `Stalled` because no block matches the claimed landing — a
/// rotated-out validator wedged in a per-tick re-jump cycle. It now pins the
/// CLOSURE: `FrontierHandler::deliver` binds the served height to the
/// certificate's own round epoch, and `epoch_of(real + 10^6) != round.epoch` long
/// before the read window has anything to say — so the answer is a LIE, `deliver`
/// returns `false`, commonware excludes node 3 from node 0's frontier fetches, and
/// nothing reaches the executor at all.
///
/// **What the observable is, after 4.2 Б1.** It used to be `upstream_frontier` —
/// the atomic the probe `fetch_max`ed the SERVED height into, whose series a test
/// could watch stay at 0. That atomic is gone: §5.2 leaves the trigger ONE
/// frontier, this node's own marshal tip, so the question a liar can be judged by
/// is now "did the victim's VERIFIED tip move", not "what number did it believe".
/// The series is therefore `marshal_tip_series[0]`, and the property is stronger
/// than the one it replaces: an inflated `Latest` cannot move a tip even if it is
/// believed, because a tip only moves through `store_finalization`. Both halves
/// still hold: no tip growth past the honest chain, and `jump_calls` EMPTY.
///
/// **Which arm of `deliver` fires, and why it is not the window.** §5.2 describes
/// an inflated `Latest` as refused for being outside the READ WINDOW (`true` +
/// drop). By the code it is refused one step earlier and more sharply: step (3),
/// the height↔epoch bind, sees `epoch_of(height) != round.epoch` and calls it a
/// lie. Out-of-window would be reached only by an answer whose height and epoch
/// agree with each other — an honest peer far ahead — which is the case
/// `plane_upstream::tests::an_out_of_window_latest_is_dropped_without_punishing_the_peer`
/// pins: `true`, the peer keeps the channel, and the answer is COUNTED and
/// THROWN AWAY (pass Б2 made the drop §5.2 asks for; pass А passed it on and that
/// unit records why).
///
/// **The control is in-test (F1/F4).** The same fixture is run twice — node 3
/// `InflatedProbe`, then node 3 `Honest`. The separating fact: with an honest
/// source node 0's re-jumps all `Landed` and it recovers to the committee's tip;
/// with the liar it spawns no jump at all and parks at its rotation boundary (95)
/// — not wedged in re-jumps, simply left without a frontier source, which is the
/// correct outcome for a node whose only source lies.
///
/// **Which assertions are vacuous, which load-bearing.** VACUOUS over an honest
/// role: `latest_inflated == 0` on the control. LOAD-BEARING: the tamper witness
/// (`latest_inflated >= 1`, the exact `10^6` delta, the structural pass), the
/// victim's REJECTION (`deliveries_rejected >= 1`), its marshal tip never leaving
/// the honest chain, `jump_calls` empty, and the control landing every jump and
/// recovering past 95.
///
/// Falsifier: the wrapper inflating nothing; a delta != `10^6`; a forged answer
/// failing the payload↔digest bind; the victim NOT rejecting; a victim tip
/// anywhere near the inflation; any jump call at all in the role run; the control
/// not landing or not recovering; the forgery leaking to nodes 1–3.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn an_inflated_latest_probe_is_refused_at_the_frontier_and_spawns_no_re_jump() {
    use super::byzantine_roles::LATEST_INFLATION;
    let role = lying_upstream_stand(Role::InflatedProbe);
    let control = lying_upstream_stand(Role::Honest);

    // (1) THE TAMPER'S OWN WITNESS (role run), before any reaction assert; and the
    // control forged nothing.
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

    // (2) THE VICTIM REFUSED IT. `deliver` returned `false` at least once, which is
    // what costs node 3 the channel for the life of node 0's resolver engine
    // (`resolver/p2p/engine.rs:436-437` → `fetcher.rs:516` `excluded`, never
    // cleared). The honest control is never refused.
    assert!(
        role.upstream[0].deliveries_rejected >= 1,
        "the victim never refused the inflated Latest: {:?}",
        role.upstream[0]
    );
    // AND THE HONEST CONTROL REFUSES NOTHING. This assert was dropped in the first
    // pass because the control DID sometimes refuse: `deliver` verified the
    // frontier multisig under the module's scheme, which carries the epoch's SEED
    // oracle, so a rotated-out node with a stale epoch key failed an HONEST
    // certificate the same way it failed a forged one (Д-75). `deliver` now builds
    // its own verify-only scheme without the oracle, and the assert comes back —
    // without it, `deliveries_rejected >= 1` above cannot tell a refused forgery
    // from a refused honest peer.
    assert_eq!(
        control.upstream[0].deliveries_rejected, 0,
        "the honest control refused a frontier answer: {:?}",
        control.upstream[0]
    );

    // (3) THE INVERSION. The victim's own marshal tip never leaves the honest
    // chain — it can only move through `store_finalization`, and the forgery never
    // gets that far — and no re-jump was ever spawned, so the per-tick
    // wasted-backfill cycle this test used to pin cannot occur.
    //
    // The bound is the ROTATION BOUNDARY, not 0: node 0 is an honest member until
    // epoch 3 and its marshal legitimately holds everything up to 95. What the lie
    // could have bought is a tip beyond that, and it bought nothing.
    assert!(
        peaked < 3 * EPOCH_LEN,
        "the victim's marshal tip ran past its rotation boundary on a forged frontier \
         (branch {branch}): peak {peaked}, series {series:?}"
    );
    assert!(
        calls.is_empty(),
        "a re-jump was spawned on an unverified frontier (branch {branch}): {calls:?}"
    );

    // (4) ATTRIBUTION BY THE CONTROL. The same fixture with an honest node 3 lands
    // every jump and carries node 0 past its rotation boundary; the liar leaves it
    // parked AT the boundary with no frontier source at all.
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

    // (5) No fork, honest lockstep, and no leak of the inflation to nodes 1–3.
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

/// (R-001 var Б, INVERTED by the frontier trust gate) Node 3's frontier-plane
/// `Producer` answers `FrontierKey::Latest` with the real tip (real height, real
/// multisig) whose `block.result` is replaced by the tip of a DIVERGENT branch it
/// has seeded into the shared devp2p peer network (`ElNetwork::publish_branch`),
/// with `proposal.payload` re-pointed to the forged block's digest so the
/// payload↔digest bind still passes. Node 0 is rotated OUT at epoch 3 and reads
/// its `Latest` from node 3 alone (`upstream_only_link`).
///
/// **What this test asserted before, and what it asserts now.** It used to pin the
/// LAST LINE OF DEFENCE: the jump LANDED the attacker's branch (the stand's
/// landing model commits a hash list unconditionally) and only then did the
/// POST-sync `verify_jump_authenticated` refuse it (`JumpOutcome::AuthFailed`) —
/// the moved payload breaks the real multisig against any committee. It now pins
/// the FIRST line: the same broken multisig is checked at
/// `FrontierHandler::deliver`, step (4), under `committee[round.epoch]` read from
/// the module — so the forged `Latest` never leaves the channel, node 0 gets no
/// frontier, spawns no jump, and reth is never pointed at the divergent branch at
/// all. The defence moved from after a full EL backfill to before a single FCU.
///
/// **AND THE POST-SYNC GATE IS NOW GONE (4.2 Б2.4), which makes `deliver` the
/// ONLY defence this forgery ever meets.** `verify_jump_authenticated` is no
/// longer a stage of `jump_to_target`: a real jump's target is a pair out of the
/// node's own marshal archive, already 2f+1 under a committee it read itself, so
/// the stage re-checked a settled question. The consequence for THIS test is that
/// its subject is no longer "which of two gates caught it" but "the gate that
/// caught it is the only one there is" — a jump call here would mean the branch
/// reached an EL FCU with nothing left to refuse it, which is why assertion (3)
/// below is now the whole safety claim.
///
/// **Letter convention (F13, unchanged):** branch labels below are Latin
/// `(a)/(b)/(c)`; the register's own consequence letters are Cyrillic and are ITS,
/// not these.
///
/// **What this is NOT (F8/F9, unchanged).** The committee read "at the landing
/// hash" was always a CALL-SHAPE fact here (`FakeStaking` ignores `at_hash`), and
/// the stand's landing model commits bodiless hashes; neither is claimed. Ex-2
/// retired the register's consequence (в) on a real node, and this test does not
/// re-open it.
///
/// **Which assertions are vacuous, which load-bearing.** VACUOUS over an honest
/// role: `result_forged == 0` on the control. LOAD-BEARING: the tamper witness
/// (`result_forged >= 1`, `result_differs`, the structural pass — i.e. the forgery
/// IS the kind that only a multisig check can catch), the victim's REJECTION, the
/// empty `jump_calls`, the absence of any divergent hash in the victim's EL
/// events, and the control landing its jumps.
///
/// Falsifier: the wrapper forging nothing; a forged result equal to the original;
/// a forged answer failing the payload↔digest bind (then a cheaper check caught it
/// and the multisig arm is not what this pins); the victim NOT rejecting; any jump
/// call in the role run; a divergent hash in the victim's EL events; the control
/// not landing; the forgery leaking to nodes 1–3.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn a_lying_upstream_is_refused_at_the_frontier_and_never_lands_its_branch() {
    use super::byzantine_roles::{divergent_hash, LYING_DIVERGE_AT};
    let role = lying_upstream_stand(Role::LyingUpstream);
    let control = lying_upstream_stand(Role::Honest);

    // (1) THE TAMPER'S OWN WITNESS (role run); the control forged nothing.
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
    // There is no post-sync gate to name any more (4.2 Б2.4): ANY jump call here
    // means the forged frontier got past `deliver`, and nothing downstream would
    // have refused it.
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

    // (2) THE VICTIM REFUSED IT — the multisig arm of `deliver`, under a committee
    // it CAN read (the forgery keeps a real certificate over a payload it no longer
    // matches). A `false` costs node 3 node 0's frontier channel for good.
    assert!(
        role.upstream[0].deliveries_rejected >= 1,
        "the victim never refused the forged Latest: {:?}",
        role.upstream[0]
    );
    // AND THE HONEST CONTROL REFUSES NOTHING — see the same assert in R-004 for why
    // it was absent until `deliver` stopped verifying under the module's
    // seed-oracle-carrying scheme (Д-75).
    assert_eq!(
        control.upstream[0].deliveries_rejected, 0,
        "the honest control refused a frontier answer: {:?}",
        control.upstream[0]
    );

    // (3) THE INVERSION. No jump was spawned at all, so nothing pointed reth at the
    // attacker's branch — and the victim's EL never holds one of its hashes.
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

    // (4) ATTRIBUTION BY THE CONTROL, and no collateral damage.
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

/// The shared fixture for R-009: node 3 leaves the committee at the first boundary
/// (`shrink_to_three`) and follows PURELY through the upstream by-height plane — the
/// (4c) configuration `a_node_outside_the_tracked_peer_set_keeps_following_through_the_
/// upstream_plane` proves an honest source carries it, and (4b) proves no source leaves
/// it standing. `re_jump_threshold` is left `None` so `rejump_calls == 0`: the by-height
/// plane is the ONLY repair path, and the wrong-height starve cannot be masked by a
/// re-jump (which the register names as R-009's eventual exit, R-001 surface). Node 3's
/// frontier discovery is confined to node 0 alone (`upstream_only_link = (0, 3)`), so the
/// two runs differ ONLY in node 0's role and the control is IN-TEST.
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

/// (R-009, INVERTED by the frontier trust gate) `Role::WrongHeightFinalized`: node
/// 0's frontier `Producer` answers every `Finalized{h}` by-height pull (for `h` in
/// `WRONG_HEIGHT_WINDOW`) with its OWN valid pair of height `h − 1` instead of `h`.
/// Nothing is mutated — the pair is a wholly real, self-consistent finalization,
/// just of the wrong height, and the wrapper self-checks both before it lets the
/// answer out. Node 3, dropped from the committee at epoch 1, catches up by the
/// plane path with node 0 as its ONLY source (`upstream_only_link`).
///
/// **What this test asserted before, and what it asserts now.** It used to pin the
/// DEFECT R-009 named: `FrontierHandler::deliver` did not bind the requested key to
/// the delivered content, so a decodable pair of the wrong height SATISFIED the
/// `Finalized{h}` fetch and returned `true`; the marshal then rejected
/// `block.height() != h` silently, the gap at `h` never closed, and the liar was
/// asked again and again. It now pins the CLOSURE: `deliver` compares the key to
/// the delivered height (step 2) and returns `false`, which is a lie signal —
/// commonware `block!`s node 0 and drops it into the fetcher's `excluded` set,
/// never cleared (`resolver/p2p/engine.rs:436-437`, `fetcher.rs:516`, `:242`).
///
/// **The observable that says "excluded", not merely "refused".** The wrapper
/// substitutes on EVERY by-height pull inside its window, so under the old
/// behaviour the substitution count grew with the run. Here it is ONE: the first
/// lie costs node 0 the victim's frontier channel, and no second pull is ever
/// addressed to it. `wrong_height_served == 1 == deliveries_rejected` is that fact
/// stated twice — once from the liar's side, once from the victim's.
///
/// **The victim still starves, and that is the fixture, not the defect.** Node 3's
/// only frontier source IS the liar (`upstream_only_link = (0, 3)`), so excluding
/// it leaves node 3 with nowhere to pull from and it stands at its drop boundary.
/// What changed is WHY: not "a wrong answer keeps satisfying the fetch", but "the
/// one peer it may ask has been refused and will not be asked again". The
/// in-fixture control — the same run with an HONEST node 0 — is what shows the gap
/// closing from an honest source: its victim follows to 15.
///
/// **Which assertions are vacuous, which load-bearing.** VACUOUS over a no-op
/// (honest) role: nothing — the control now differs on every line below. LOAD-BEARING:
/// the tamper witness (`wrong_height_served >= 1`, every served height exactly
/// `requested − 1`, `wrong_height_valid`), the REFUSAL
/// (`deliveries_rejected == wrong_height_served`), the wrong-height pull NOT
/// completing (`finalized_calls > finalized_delivered`), the ONE substitution (the
/// exclusion), and the separation `role.heights[3] < control.heights[3]`.
///
/// Falsifier: the wrapper serving nothing, or a served height not `requested − 1`,
/// or a served pair that is not a self-consistent finalization;
/// `deliveries_rejected == 0` (the victim accepted the lie — then `deliver` still
/// binds nothing); more substitutions than refusals (the liar kept being asked
/// after a `false`); a re-jump running (`rejump_calls > 0` — the by-height path was
/// not the only one); the honest control NOT following; the committee losing
/// lockstep or halting.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn a_wrong_height_answer_costs_the_liar_the_channel_and_never_satisfies_the_fetch() {
    let role = wrong_height_stand(Role::WrongHeightFinalized);
    let control = wrong_height_stand(Role::Honest);

    assert!(!role.timed_out, "role heights {:?}", role.heights);
    assert!(!control.timed_out, "control heights {:?}", control.heights);

    // (1) THE TAMPER'S OWN WITNESS (role), before any reaction assert; control forged
    // nothing.
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

    // The observed branch, from the victim's reaction.
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

    // (2) `deliver` BOUND THE KEY. Every substitution was refused, and a refusal is
    // what commonware turns into a permanent exclusion — so the count of
    // substitutions equals the count of refusals and stops there.
    assert_eq!(
        u.deliveries_rejected, byz.wrong_height_served,
        "the victim did not refuse every wrong-height pair (branch {branch}): {u:?} vs {byz:?}"
    );
    assert_eq!(
        byz.wrong_height_served, 1,
        "the liar was asked for a second by-height pull after being refused — the exclusion \
         did not take (branch {branch}): {byz:?}"
    );
    // The refused pull did NOT complete: a wrong-height answer no longer satisfies
    // the fetch, which is R-009's whole subject stated as a counter.
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

    // (3) THE SEPARATING FACT (load-bearing): with the liar as its only source the
    // victim stands at its drop boundary (the last block of epoch 0 it finalized
    // in-committee, height 5); with an honest source the gap closes and it follows.
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

    // (4) No leak to bystanders: the committee stays in lockstep and reaches 16 in both
    // runs; nothing halts or diverges; the only ERROR lines allowed are the exempt sim
    // ack-drops.
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

/// (F10) The fork-safety the rework moved OUT of `ElNetwork`'s
/// first-writer-wins-by-height map and INTO `FakeChain::land_jump` — its two
/// refusal arms — is exercised by no multi-node test (both stay unreached on the
/// honest schedules). This unit-style test drives them directly on a hand-built
/// `ElNetwork`:
///   * `Unservable` — the served tip hash is not in the peer network at all;
///   * `ConflictingPrefix` — a walked block sits at a height where the jumping
///     node's canonical chain already holds a DIFFERENT hash.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn land_jump_refuses_an_unservable_tip_and_a_conflicting_prefix() {
    use super::fakes::{ElNetwork, FakeChain, JumpLanding};
    use alloy_primitives::B256;
    let h = |n: u8| B256::repeat_byte(n);

    // Arm 1: the peer holds no such hash — the walk falls off the tip at once.
    let el = ElNetwork::default();
    let chain = FakeChain::with_genesis_on(h(0), el.clone());
    assert_eq!(
        chain.land_jump(h(99)),
        JumpLanding::Unservable,
        "a tip the peer network does not hold must be Unservable"
    );

    // Land an honest prefix 1..=5 into the chain's canonical (publish it to the
    // peer, then walk its tip down to the genesis fork point).
    let honest: Vec<(B256, u64, B256)> = (1..=5u64)
        .map(|n| (h(n as u8), n, h((n - 1) as u8)))
        .collect();
    el.publish_branch(&honest);
    assert_eq!(
        chain.land_jump(h(5)),
        JumpLanding::Landed,
        "the honest prefix must land"
    );

    // Arm 2: a divergent block at height 5 (parent = honest 4) contradicts the
    // honest hash the chain now holds canonically at 5.
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

/// A node in the REGISTRY but in no committee is tier 2 on every node's peer
/// set, and still follows the chain (4.3 A.1/A.5в).
///
/// Before 4.3 the whole Active registry was PRIMARY: a registry entry that had
/// never been on a committee still got a `buffered` body deque of its own on
/// every node, a slot in the discovery bit-vec and a place in the resolver's
/// candidate list (R-013, R-037, E4-14). Now the registry is `secondary`:
/// commonware never dials it, never bit-vec gossips it and never caches its
/// bodies (`CW:.../tracker/record.rs:171`,
/// `CW:broadcast/src/buffered/engine.rs:319-322`), but does accept it inbound and
/// does serve it.
///
/// What the SIMULATED network can and cannot show. It models the tiers where they
/// are read: `register_tracked_peer_set` keeps `primary` and `secondary` apart
/// (`CW:p2p/src/simulated/network.rs:263-313`) and `latest_update` hands both to
/// every `Provider::subscribe` consumer (`:624-631`) — which is exactly the input
/// `buffered` filters its cache on. It does NOT tier DELIVERY: `all_connected_peers`
/// returns every peer of every tier (`:639-641`) and `is_connectable` asks only
/// that the key be in some set (`:644-646`). So "the secondary node receives" is
/// not a discriminating claim in this harness — the discriminating evidence for
/// the real transport is the authenticated-transport precondition
/// (`testbed::preconditions::a_secondary_peer_on_the_authenticated_transport_is_accepted_and_heard`),
/// which measures acceptance, service and the dial map on
/// `commonware_p2p::authenticated::discovery`. What this test pins is the half
/// the simulator DOES decide: which tier this node's own code puts the peer in.
///
/// Falsifier: the registry-only key appearing in any node's `primary`, a
/// committee member missing from it, or the outsider's chain stopping.
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

    // PREMISE: every node registered at least two epochs' peer sets, so the
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

    // And the outsider is not cut off: it keeps executing the chain the committee
    // finalizes. (Delivery, not tiering — see the doc block.)
    assert!(
        out.heights[OUTSIDER] >= 2 * EPOCH_LEN,
        "the registry-only node stopped following: {:?}",
        out.heights
    );
}
