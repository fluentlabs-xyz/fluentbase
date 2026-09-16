//! The committee module as the stand sees it: one frozen record per epoch, read
//! at one anchor, over a contract that answers differently on a branch the node
//! only speculated on.
//!
//! Every test asserts its premise before its property, so a fixture that stops
//! discriminating fails rather than leaving the test quietly vacuous.

use super::{
    fakes::{Branch, BranchCommittees, ElEvent},
    stand::{
        counter_of, drain_counters, CommitteeFacts, Committees, Outcome, Progress, Stand,
        StandConfig,
    },
};
use alloy_primitives::B256;
use fluentbase_types::staking_protocol::MAX_COMMITTEE_LOOKAHEAD_EPOCHS;
use metrics_util::debugging::DebuggingRecorder;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

const EPOCH_LEN: u64 = 32;

/// The height the record test tombstones node 0 from: the five nodes' anchors
/// for epoch 3 straddle it, so the contract's live flag is true for some reads
/// of that epoch and false for others. The test asserts that straddle rather
/// than assuming it.
const TOMBSTONE_FROM: u64 = 64;

fn last(epoch: u64) -> u64 {
    (epoch + 1) * EPOCH_LEN - 1
}

fn reached(h: u64) -> impl Fn(&Progress) -> bool + Send + 'static {
    move |p| p.min_height() >= h
}

/// True once every node's `SafetyHalt` has been engaged for `ticks` consecutive
/// driver samples: a halt is asserted by what stops happening after it, so the
/// run has to keep going past the edge.
fn halted_for(ticks: usize) -> impl Fn(&Progress) -> bool + Send + 'static {
    let seen = std::cell::Cell::new(0usize);
    move |p: &Progress| {
        if !p.halted.is_empty() && p.halted.iter().all(|h| *h) {
            seen.set(seen.get() + 1);
        } else {
            seen.set(0);
        }
        seen.get() >= ticks
    }
}

/// The 4 → 3 → 4 rotation shared with `super::tests` and `super::preconditions`,
/// node 3 out for epochs 3 and 4.
fn rotate_four_three_four() -> Committees {
    Committees::Schedule(Arc::new(|epoch, n| {
        Some(match epoch {
            3 | 4 => vec![0, 1, 2],
            _ => (0..n).collect(),
        })
    }))
}

/// The record test's rotation: 5 → 4 → 5, node 3 out for epochs 3 and 4, node 0
/// (the tombstoned one) in every epoch. The fifth seat is load-bearing: the
/// tombstoned member takes no part in the ceremony it sits in, so the epoch-3/4
/// DKG has to reach `quorum(4) = 3` from the members left once node 0 is
/// tombstoned and node 3 is rotated out, i.e. nodes 1, 2 and 4; on the four-node
/// roster that count was 2.
fn rotate_five_four_five() -> Committees {
    Committees::Schedule(Arc::new(|epoch, n| {
        Some(match epoch {
            3 | 4 => vec![0, 1, 2, 4],
            _ => (0..n).collect(),
        })
    }))
}

/// [`rotate_five_four_five`] on the canonical branch, and a rotation of the
/// seats on every other hash of the reading node's own tree.
///
/// The canonical half is built from the same closure as the plain schedule, so
/// the two agree member for member. The speculative half rotates rather than
/// subsets: a consumer that read it would still build a working committee and
/// scheme, just the wrong one, so the mismatch is only visible where the records
/// are compared.
fn branching_rotation(n: usize) -> BranchCommittees {
    let Committees::Schedule(flat) = rotate_five_four_five() else {
        unreachable!("rotate_five_four_five is a Schedule");
    };
    Arc::new(
        move |epoch: u64, _at: &B256, _height: u64, branch: Branch| {
            let canonical = flat(epoch, n)?;
            Some(match branch {
                Branch::Canonical => canonical,
                Branch::Speculative => canonical.into_iter().map(|i| (i + 1) % n).collect(),
            })
        },
    )
}

/// Every `(epoch, facts)` node `i` holds an `Ok` record for.
fn records(out: &Outcome, i: usize) -> BTreeMap<u64, CommitteeFacts> {
    out.committee_records[i]
        .iter()
        .filter_map(|(e, answer)| answer.as_ref().ok().map(|f| (*e, f.clone())))
        .collect()
}

/// The first hash that lived in a node's executed tree while its canonical
/// chain did not hold it, the state a reader on a speculative cursor would land
/// on; `None` when every derive was canonicalized by the very next EL event.
fn tree_only_hash(events: &[ElEvent]) -> Option<(u64, B256)> {
    events.iter().enumerate().find_map(|(i, event)| {
        let ElEvent::Derived(height, hash) = event else {
            return None;
        };
        match events
            .iter()
            .skip(i + 1)
            .position(|e| *e == ElEvent::Canonicalized(*height, *hash))
        {
            // Canonical on the very next event: never observable as a branch.
            Some(0) => None,
            _ => Some((*height, *hash)),
        }
    })
}

/// Five nodes at five different heights hold one committee record per epoch,
/// over a contract whose answer depends on the branch the reading hash sits on.
///
/// The fixture rotates node 3 out at epoch 3 and cuts it physically, so it falls
/// behind and is carried back by the re-jump; a consensus-plane-only cut would
/// leave the frontier probe feeding it and produce no tree-only derive, which the
/// speculative-branch premise needs. Five seats are required because the
/// tombstoned node 0 takes no part in the epoch-3/4 ceremony, so `quorum(4) = 3`
/// has to come from nodes 1, 2 and 4. Node 0 is tombstoned from
/// [`TOMBSTONE_FROM`], and the five anchors straddle that height, so the live
/// flag differs across reads while the records still have to agree.
///
/// The result is not a tautology: [`branching_rotation`] gives every read off
/// the reading node's canonical chain a different committee of the same size, so
/// agreeing records mean every node read on its own canonical chain.
#[test]
fn five_nodes_at_five_heights_hold_one_committee_record_per_epoch() {
    let mut cfg = StandConfig::live(5, 1);
    let roster = cfg.n;
    cfg.committees = rotate_five_four_five();
    cfg.committees_by_branch = Some(branching_rotation(roster));
    // Node 0 sits in every epoch's committee, so its tombstone reaches every
    // read above the height.
    cfg.tombstoned = vec![(0, TOMBSTONE_FROM)];
    cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD.min(EPOCH_LEN));
    let mut stand = Stand::new(cfg);
    stand
        .partition(&[0, 1, 2, 4], &[3])
        .after_height(2 * EPOCH_LEN + 4)
        .for_views(EPOCH_LEN as u32 + 8);
    let out = stand.run_until(reached(5 * EPOCH_LEN + 8), Duration::from_secs(400));
    let n = out.heights.len();
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert!(out.errors().is_empty(), "{:?}", out.errors());

    let schedule = branching_rotation(roster);
    for epoch in 0..=5u64 {
        let canonical = schedule(epoch, &B256::ZERO, 0, Branch::Canonical);
        let speculative = schedule(epoch, &B256::ZERO, 0, Branch::Speculative);
        assert!(
            canonical.is_some() && canonical != speculative,
            "the branching schedule answers the same committee on both branches at epoch \
             {epoch}: {canonical:?}"
        );
    }

    let part = &out.partitions[0];
    assert!(
        !part.heights_at_heal.is_empty(),
        "the cut never fired or never healed: {part:?}"
    );
    assert!(
        !out.jump_calls[3].is_empty(),
        "node 3 never re-jumped, so nothing carried it back over its own derive:          heights={:?} cut={part:?}",
        out.heights
    );

    let tree_only: Vec<Option<(u64, B256)>> =
        (0..n).map(|i| tree_only_hash(&out.el_events[i])).collect();
    assert!(
        tree_only.iter().any(|w| w.is_some()),
        "no node ever held a tree-only hash, so the speculative branch is state this run \
         cannot produce: {tree_only:?}"
    );

    let per_node: Vec<BTreeMap<u64, CommitteeFacts>> = (0..n).map(|i| records(&out, i)).collect();
    let epochs: Vec<u64> = per_node[0].keys().copied().collect();
    let split_anchors: Vec<u64> = epochs
        .iter()
        .copied()
        .filter(|e| {
            per_node
                .iter()
                .filter_map(|r| r.get(e).map(|f| f.anchor))
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                > 1
        })
        .collect();
    assert!(
        !split_anchors.is_empty(),
        "every node read every epoch at the SAME anchor, so the record equality says nothing \
         about two heights: {:?}",
        per_node
            .iter()
            .map(|r| r.iter().map(|(e, f)| (*e, f.anchor.0)).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    );

    let straddled: Vec<u64> = epochs
        .iter()
        .copied()
        .filter(|e| {
            let sides: std::collections::BTreeSet<bool> = per_node
                .iter()
                .filter_map(|r| r.get(e).map(|f| f.anchor.0 >= TOMBSTONE_FROM))
                .collect();
            sides.len() > 1
        })
        .collect();
    assert!(
        !straddled.is_empty(),
        "no epoch was read on both sides of the tombstone height {TOMBSTONE_FROM}, so the live \
         flag was the same for every node: {:?}",
        per_node
            .iter()
            .map(|r| r.iter().map(|(e, f)| (*e, f.anchor.0)).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    );
    assert!(
        (0..n).any(|i| !out.staking_reads[i].tombstoned_seen.is_empty()),
        "the contract never reported the tombstone it was given: {:?}",
        (0..n)
            .map(|i| out.staking_reads[i].tombstoned_seen.clone())
            .collect::<Vec<_>>()
    );

    // Compared only over `split_anchors`: an epoch every node read at the same
    // anchor is one hash and one contract answer, so its equality is arithmetic.
    for epoch in &split_anchors {
        let held: Vec<(usize, &CommitteeFacts)> = (0..n)
            .filter_map(|i| per_node[i].get(epoch).map(|f| (i, f)))
            .collect();
        let (first_node, first) = held[0];
        for (i, facts) in &held[1..] {
            assert_eq!(
                (&facts.members, &facts.weights, facts.changed),
                (&first.members, &first.weights, first.changed),
                "nodes {first_node} and {i} hold DIFFERENT committee records for epoch {epoch} \
                 (anchors {:?} and {:?})",
                first.anchor,
                facts.anchor
            );
        }
    }

    // Unreachable by construction for this module: its only read hash comes from
    // `Anchor::executed_hash`, which the stand and production both answer off
    // the canonical chain. The counter therefore guards the fake rather than the
    // module.
    for i in 0..n {
        assert!(
            out.staking_reads[i].speculative.is_empty(),
            "node {i} read the staking state on a branch its own canonical chain does not \
             hold: {:?}",
            out.staking_reads[i]
        );
    }

    for (i, held) in per_node.iter().enumerate() {
        for (epoch, facts) in held {
            let (height, hash) = facts.anchor;
            if height == 0 {
                continue;
            }
            let canonical = out.hashes[i][(height - 1) as usize].1;
            assert_eq!(
                hash, canonical,
                "node {i} read committee[{epoch}] at {hash} while its canonical chain holds \
                 {canonical} at height {height}"
            );
        }
    }

    out.assert_lockstep_except(&[]);
    eprintln!(
        "(4.1/record) heights={:?} anchors={:?} split_anchors={split_anchors:?} \
         straddled={straddled:?} tombstoned_seen={:?} tree_only={tree_only:?} \
         jump_calls[3]={:?} cut={:?}",
        out.heights,
        per_node
            .iter()
            .map(|r| r.iter().map(|(e, f)| (*e, f.anchor.0)).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        (0..n)
            .map(|i| out.staking_reads[i].tombstoned_seen.clone())
            .collect::<Vec<_>>(),
        out.jump_calls[3],
        out.partitions[0],
    );
}

/// A node below the chain refuses the epochs it cannot see without an EVM call,
/// and pays exactly one snapshot per epoch it can.
///
/// Two runs over one schedule: with the re-jump gate shut the rotated-out node
/// parks at `last(2)` for good, and with the gate at production's value it comes
/// back. Both take the lag from a consensus-plane cut inside epoch 2, because
/// since the epoch key became a fetchable artifact the rotation alone leaves a
/// node following the chain.
///
/// `commit_height(E) = start(E − 2)` and the window top is `epoch(anchor) + 2`,
/// so `NotReadable` is unreachable for an in-window epoch once the geometry is
/// frozen and a parked node asking about the live chain gets `OutOfWindow`; the
/// one anchor where `NotReadable` fires is height 0, where epochs 1 and 2 are
/// committed by the first executed block and not by genesis. Both arms answer
/// without touching the contract, which the test asserts separately so neither
/// can stand in for the other.
///
/// The counters come from `Outcome::metrics_before_collect`, the snapshot taken
/// before the stand polls every module for `committee_records`, so they say what
/// the run asked rather than what asserting asked.
#[test]
fn a_node_below_the_chain_refuses_what_it_cannot_see_without_an_evm_call() {
    let members = [0usize, 1, 2];
    let end = 5 * EPOCH_LEN + 8;
    /// Inside epoch 2, so node 3 holds every key up to `PK_2` and nothing above.
    const CUT_AT: u64 = 2 * EPOCH_LEN + 4;
    /// Longer than either run's virtual deadline, so the cut never heals.
    const NEVER: u32 = 4096;
    /// Two epochs of cut, far enough behind for production's gate to arm.
    const HELD_FOR: u32 = 2 * EPOCH_LEN as u32 + 8;

    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let parked = metrics::with_local_recorder(&recorder, || {
        let mut cfg = StandConfig::live(4, 1);
        cfg.committees = rotate_four_three_four();
        // Taken by the stand before it polls every node's module for
        // `committee_records`.
        cfg.metrics_snapshotter = Some(snap.clone());
        assert_eq!(cfg.re_jump_threshold, None, "the gate must stay shut here");
        let mut stand = Stand::new(cfg);
        stand
            .partition(&[0, 1, 2], &[3])
            .after_height(CUT_AT)
            .consensus_only()
            .for_views(NEVER);
        stand.run_until(
            move |p| p.min_height_of(&members) >= end,
            Duration::from_secs(400),
        )
    });
    // What the run counted, and separately what the post-run poll added on top
    // of it; the second is reported and never asserted on.
    let drained = &parked.metrics_before_collect;
    let by_collect = drain_counters(&snap);
    assert!(!parked.timed_out, "heights {:?}", parked.heights);
    assert!(parked.halted.is_empty(), "{:?}", parked.halted);
    assert!(parked.errors().is_empty(), "{:?}", parked.errors());

    assert_eq!(
        parked.heights[3],
        last(2),
        "the rotated-out node did not park at the last block of epoch 2: {:?}",
        parked.heights
    );
    for i in members {
        assert!(
            parked.heights[i] >= end,
            "member {i} did not run past the parked node: {:?}",
            parked.heights
        );
    }
    // Its window tops out at `epoch(last(2)) + 2 = 4`, so epochs 5 and up are
    // the ones it cannot see. Taken from the module's own refusal text rather
    // than recomputed here, so the two cannot drift apart.
    let unseen: Vec<u64> = parked.committee_records[3]
        .iter()
        .filter_map(|(e, answer)| answer.as_ref().err().map(|r| (*e, r)))
        .filter(|(_, r)| r.error.contains("outside the readable window"))
        .map(|(e, _)| e)
        .collect();
    assert!(
        unseen.contains(&5),
        "epoch 5 is not outside the parked node's window: {:?}",
        parked.committee_records[3]
    );

    let reads = &parked.staking_reads[3];
    for epoch in &unseen {
        assert_eq!(
            (
                reads.module_snapshot.get(epoch),
                reads.committed.get(epoch),
                reads.uncommitted.get(epoch)
            ),
            (None, None, None),
            "the parked node read the contract for epoch {epoch}, which it refuses without one: \
             {reads:?}"
        );
        assert!(
            parked.committee_records[3][epoch]
                .as_ref()
                .is_err_and(|r| r.transient),
            "the refusal of epoch {epoch} is not a retryable one: {:?}",
            parked.committee_records[3][epoch]
        );
    }
    // `drained` is the pre-collect snapshot precisely here: `committee_records`
    // above re-asks the parked node's epochs during collection, so a count taken
    // at the end can never be zero.
    let above = counter_of(
        drained,
        "dpos_committee_out_of_window_total",
        Some(("side", "above")),
    );
    assert!(
        above > 0,
        "no epoch was ever refused above the window during the RUN, so 'refused without an \
         EVM call' is vacuous: {drained:?}"
    );

    // Epoch 1 is committed by the first executed block and not by genesis, so an
    // anchor still at 0 refuses it arithmetically.
    let not_readable = counter_of(drained, "dpos_committee_not_readable_total", None);
    assert!(
        not_readable > 0,
        "nothing was ever refused as not-readable-yet during the run: {drained:?}"
    );
    let epoch_one = records(&parked, 0);
    let epoch_one = epoch_one.get(&1).expect("node 0 holds committee[1]");
    assert!(
        epoch_one.anchor.0 >= 1,
        "committee[1] was read at genesis, so the not-readable arm was never on the path: {:?}",
        epoch_one.anchor
    );
    // The refusals share one counter, so the arm is identified by price: the
    // commit-height arm refuses before resolving the anchor hash, while the
    // empty-answer arm pays the snapshot staticcall. A refusal through the latter
    // would make `module_snapshot[1]` at least 2; it is exactly 1.
    for i in 0..parked.heights.len() {
        for epoch in 1..=MAX_COMMITTEE_LOOKAHEAD_EPOCHS {
            let paid = parked.staking_reads[i].module_snapshot.get(&epoch);
            assert!(
                paid.is_none_or(|calls| *calls == 1),
                "node {i} paid {paid:?} snapshots for committee[{epoch}], so its refusal below \
                 the commit height cost a contract read: {:?}",
                parked.staking_reads[i]
            );
        }
    }

    for i in 0..parked.heights.len() {
        for epoch in records(&parked, i).keys() {
            let calls = parked.staking_reads[i].module_snapshot.get(epoch).copied();
            assert!(
                calls == Some(1) || calls.is_none(),
                "node {i} paid {calls:?} snapshots for committee[{epoch}] it holds once: {:?}",
                parked.staking_reads[i]
            );
        }
    }

    let caught_up = {
        let mut cfg = StandConfig::live(4, 1);
        cfg.committees = rotate_four_three_four();
        cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD.min(EPOCH_LEN));
        let mut stand = Stand::new(cfg);
        stand
            .partition(&[0, 1, 2], &[3])
            .after_height(CUT_AT)
            .consensus_only()
            .for_views(HELD_FOR);
        stand.run_until(reached(end), Duration::from_secs(400))
    };
    assert!(!caught_up.timed_out, "heights {:?}", caught_up.heights);
    assert!(caught_up.halted.is_empty(), "{:?}", caught_up.halted);
    assert!(caught_up.errors().is_empty(), "{:?}", caught_up.errors());
    assert!(
        !caught_up.jump_calls[3].is_empty(),
        "the gate was open but the node never re-jumped"
    );
    let back = records(&caught_up, 3);
    let reference = records(&caught_up, 0);
    for epoch in &unseen {
        let Some(reference) = reference.get(epoch) else {
            continue;
        };
        let held = back
            .get(epoch)
            .unwrap_or_else(|| panic!("the caught-up node still lacks committee[{epoch}]"));
        assert_eq!(
            (&held.members, &held.weights, held.changed),
            (&reference.members, &reference.weights, reference.changed),
            "the caught-up node's committee[{epoch}] is not the members'"
        );
    }
    // The refused epochs this run really read: the tail of `unseen` sits above
    // where the run went, and a collection-phase record is not a read the run
    // made.
    let read_back: Vec<u64> = unseen
        .iter()
        .copied()
        .filter(|e| caught_up.staking_reads[3].module_snapshot.contains_key(e))
        .collect();
    assert!(
        read_back.contains(&5),
        "the caught-up node never read an epoch it had been refused: {read_back:?} of \
         {unseen:?}, reads {:?}",
        caught_up.staking_reads[3]
    );
    for epoch in &read_back {
        assert_eq!(
            caught_up.staking_reads[3].module_snapshot.get(epoch),
            Some(&1),
            "committee[{epoch}] cost the caught-up node more than one snapshot: {:?}",
            caught_up.staking_reads[3]
        );
    }
    eprintln!(
        "(4.1/backfill) parked={:?} unseen={unseen:?} read_back={read_back:?} above={above} \
         not_readable={not_readable} above_by_collect={} not_readable_by_collect={} \
         parked_reads={:?} caught_up={:?} reads={:?}",
        parked.heights,
        counter_of(
            &by_collect,
            "dpos_committee_out_of_window_total",
            Some(("side", "above"))
        ),
        counter_of(&by_collect, "dpos_committee_not_readable_total", None),
        parked.staking_reads[3],
        caught_up.heights,
        caught_up.staking_reads[3]
    );
}

/// `weights: None` inside the read window is a permanent refusal that stops the
/// node: no record or certificate scheme for the epoch, one `error!` from the
/// module, the slot poisoned so no further staticcall is spent on it, and a
/// `SafetyHalt` with `ContractFork`, after which nothing moves.
///
/// The contract's frozen weights live in a ring of `WEIGHT_RING_EPOCHS` frames
/// and go missing far below the reading height, outside the module's window, so
/// running the stand longer cannot reach this arm; `StandConfig::weights_none_for`
/// makes the contract answer it anyway. Every node halts, and that is the
/// designed outcome: all four read the same contract, so an impossible answer is
/// correlated by construction. `run_until` therefore waits past the halt with
/// [`halted_for`], so the standing still can be observed rather than assumed.
#[test]
fn a_weightless_committee_inside_the_window_stops_every_node_that_must_enter_it() {
    let epoch = 2u64;
    // Driver ticks the run stays up after the last halt, the window the marshal
    // tip is asserted to stand still in; ten ticks is one virtual second
    // (`stand::POLL`).
    const AFTER_HALT_TICKS: usize = 10;
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let out = metrics::with_local_recorder(&recorder, || {
        let mut cfg = StandConfig::honest(4, 1);
        cfg.weights_none_for = Some(epoch);
        // Snapshotted before the post-run poll of `committee_records`, which
        // re-reads the refused epoch on every node: the re-read is answered from
        // the poisoned slot but still ticks the refusal counter.
        cfg.metrics_snapshotter = Some(snap.clone());
        cfg.marshal_tip_series = true;
        Stand::new(cfg).run_until(halted_for(AFTER_HALT_TICKS), Duration::from_secs(200))
    });
    let drained = &out.metrics_before_collect;
    let by_collect = drain_counters(&snap);
    let n = out.heights.len();
    assert!(
        !out.timed_out,
        "not every node halted: halted={:?} heights={:?}",
        out.halted, out.heights
    );

    // The ordering plane, not the executed tier, completed epoch 1: execution
    // trails a finalized order block by `K`, so asserting the executed tier would
    // assert the lag.
    for i in 0..n {
        let tip = *out.marshal_tip_series[i]
            .last()
            .expect("`marshal_tip_series` was enabled");
        assert!(
            tip >= last(epoch - 1),
            "node {i}'s ordering plane did not finish epoch {}: tip {tip}",
            epoch - 1
        );
        assert!(
            out.et_boundaries[i].iter().any(|b| b.epoch == epoch),
            "node {i} was never handed the boundary for epoch {epoch} — the halt below would be \
             for an epoch nothing owed it: {:?}",
            out.et_boundaries[i]
        );
        assert!(
            out.heights[i] + crate::order_block::K >= last(epoch - 1),
            "node {i}'s execution fell more than K blocks behind the boundary it halted at: {:?}",
            out.heights
        );
    }
    out.assert_lockstep_except(&[]);

    for i in 0..n {
        let weightless: Vec<u64> = out.staking_reads[i].weights_none.keys().copied().collect();
        assert_eq!(
            weightless,
            vec![epoch],
            "node {i}'s contract answered the weightless committee for the wrong epochs: {:?}",
            out.staking_reads[i]
        );
    }

    assert_eq!(
        out.halted.len(),
        n,
        "not every node halted on an impossible committee: {:?}",
        out.halted
    );
    for (i, reason) in &out.halted {
        assert!(
            reason.contains("ContractFork"),
            "node {i} halted for another reason: {reason}"
        );
    }

    for i in 0..n {
        let tips = &out.marshal_tip_series[i];
        assert!(
            tips.len() > AFTER_HALT_TICKS,
            "node {i} has no samples after the halt: {tips:?}"
        );
        let tail = &tips[tips.len() - AFTER_HALT_TICKS..];
        assert!(
            tail.iter().all(|t| *t == tail[0]),
            "node {i}'s marshal tip kept moving after its SafetyHalt: {tail:?}"
        );
    }

    for i in 0..n {
        let refusal = out.committee_records[i][&epoch]
            .as_ref()
            .expect_err("the weightless epoch must not produce a record");
        assert!(
            !refusal.transient,
            "node {i} was told to retry an answer no retry can change: {refusal:?}"
        );
        assert!(
            refusal.error.contains("no frozen weights"),
            "node {i} refused epoch {epoch} for some other reason: {refusal:?}"
        );
        assert!(
            !out.committee_verifier_epochs[i].contains(&epoch),
            "node {i} built a certificate scheme for an epoch it holds no committee for: {:?}",
            out.committee_verifier_epochs[i]
        );
        // `verifier_epochs` lists verify-only schemes, so it would miss the
        // signer scheme an engine spawn installs; ask the module for both.
        assert!(
            out.committees[i].scheme(epoch).is_none(),
            "node {i} holds a certificate scheme for the epoch it refused"
        );
        // The epoch below is held, so the absence above is this answer and not a
        // node that read nothing.
        assert!(
            out.committee_records[i][&(epoch - 1)].is_ok(),
            "node {i} holds no record for epoch {} either: {:?}",
            epoch - 1,
            out.committee_records[i]
        );
    }

    // `==`, not `>=`: the memoisation is the property, and a second call would
    // mean the refusal is re-derived on the hot path.
    for i in 0..n {
        assert_eq!(
            out.staking_reads[i].module_snapshot.get(&epoch),
            Some(&1),
            "node {i} paid for the impossible epoch more than once: {:?}",
            out.staking_reads[i]
        );
    }

    // The capture carries no node label, so "once per node" is a count over one
    // process and the ERROR set must contain nothing else.
    let permanent: Vec<&super::capture::Captured> = out
        .errors()
        .into_iter()
        .filter(|l| l.text.contains("committee read failed PERMANENTLY"))
        .collect();
    assert_eq!(
        permanent.len(),
        n,
        "expected one permanent-refusal ERROR per node, got {:?}",
        permanent.iter().map(|l| &l.text).collect::<Vec<_>>()
    );
    let halts: Vec<&super::capture::Captured> = out
        .errors()
        .into_iter()
        .filter(|l| l.text.contains("is IMPOSSIBLE"))
        .collect();
    assert_eq!(
        halts.len(),
        n,
        "expected one halt ERROR per node, got {:?}",
        halts.iter().map(|l| &l.text).collect::<Vec<_>>()
    );
    let parked: Vec<&super::capture::Captured> = out
        .errors()
        .into_iter()
        .filter(|l| l.text.contains("executor SafetyHalt — parking"))
        .collect();
    assert_eq!(
        parked.len(),
        n,
        "expected one executor park per node, got {:?}",
        parked.iter().map(|l| &l.text).collect::<Vec<_>>()
    );
    assert_eq!(
        out.errors().len(),
        3 * n,
        "the run produced ERROR lines beyond the refusal, the halt and the park under test: {:?}",
        out.errors()
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
    );
    // Two shapes for one epoch: the module logs a bare `u64`, the manager the
    // `Epoch` newtype it is handed.
    for line in permanent.iter().chain(&halts) {
        assert!(
            line.text.contains(&format!("epoch={epoch}"))
                || line.text.contains(&format!("epoch=Epoch({epoch})")),
            "a line about the refusal names another epoch: {}",
            line.text
        );
    }

    let weights_none = counter_of(
        drained,
        "dpos_committee_read_permanent_total",
        Some(("reason", "weights_none")),
    );
    assert!(
        weights_none >= n as u64,
        "the weightless refusal was not counted once per node during the run: \
         {weights_none} < {n}"
    );
    assert_eq!(
        counter_of(drained, "dpos_committee_read_permanent_total", None),
        weights_none,
        "some OTHER permanent cause fired in this run: {drained:?}"
    );
    eprintln!(
        "(4.1/weights) heights={:?} halted={:?} weights_none={weights_none} \
         weights_none_by_collect={} errors={} reads={:?}",
        out.heights,
        out.halted,
        counter_of(
            &by_collect,
            "dpos_committee_read_permanent_total",
            Some(("reason", "weights_none"))
        ),
        out.errors().len(),
        out.staking_reads[0]
    );
}

/// A committee read that reverts inside the window is the other permanent class:
/// the epoch is refused loudly and not registered, and the node does not stop.
///
/// A revert says the read could not be served — a staking-module code error, a
/// read of a module that is not there — which an operator repairs and restarts;
/// it is not the chain stating a committee that cannot exist, so it does not
/// carry what justifies stopping a node.
#[test]
fn a_reverting_committee_read_inside_the_window_refuses_the_epoch_and_leaves_the_node_up() {
    let epoch = 2u64;
    let out = {
        let mut cfg = StandConfig::honest(4, 1);
        cfg.reverts_for = Some(epoch);
        Stand::new(cfg).run_until(reached(last(epoch - 1)), Duration::from_secs(200))
    };
    let n = out.heights.len();
    assert!(!out.timed_out, "heights {:?}", out.heights);

    for i in 0..n {
        assert!(
            out.heights[i] >= last(epoch - 1),
            "node {i} did not reach the end of epoch {}: {:?}",
            epoch - 1,
            out.heights
        );
    }

    for i in 0..n {
        let reverted: Vec<u64> = out.staking_reads[i].reverted.keys().copied().collect();
        assert_eq!(
            reverted,
            vec![epoch],
            "node {i}'s contract reverted for the wrong epochs: {:?}",
            out.staking_reads[i]
        );
    }

    assert!(
        out.halted.is_empty(),
        "a revert stopped a node: {:?}",
        out.halted
    );

    for i in 0..n {
        let refusal = out.committee_records[i][&epoch]
            .as_ref()
            .expect_err("a reverting read must not produce a record");
        assert!(
            !refusal.transient,
            "node {i} was told to retry a revert: {refusal:?}"
        );
        assert!(
            refusal.error.contains("reverted"),
            "node {i} refused epoch {epoch} for some other reason: {refusal:?}"
        );
        assert!(
            out.committees[i].scheme(epoch).is_none(),
            "node {i} holds a certificate scheme for the epoch its contract refused to answer"
        );
    }

    let permanent: Vec<&super::capture::Captured> = out
        .errors()
        .into_iter()
        .filter(|l| l.text.contains("committee read failed PERMANENTLY"))
        .collect();
    assert_eq!(
        permanent.len(),
        n,
        "expected one permanent-refusal ERROR per node, got {:?}",
        permanent.iter().map(|l| &l.text).collect::<Vec<_>>()
    );
    assert_eq!(
        out.errors().len(),
        n,
        "the run produced ERROR lines beyond the refusal under test: {:?}",
        out.errors()
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
    );
    eprintln!(
        "(4.1/revert) heights={:?} halted={:?} errors={} reads={:?}",
        out.heights,
        out.halted,
        out.errors().len(),
        out.staking_reads[0]
    );
}
