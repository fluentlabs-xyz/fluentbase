//! The committee module (`crate::committee`) as the stand sees it: one frozen
//! record per epoch, read at ONE anchor, over a contract that can answer
//! DIFFERENTLY on a branch the node only speculated on.
//!
//! Same form as [`super::preconditions`]: every test states its PREMISE first —
//! the two branches really differ, the nodes really stood at different heights,
//! the epoch really was outside what the node could see — and only then the
//! property. A premise that stops holding turns the test RED instead of leaving
//! it quietly vacuous, which matters more here than anywhere else in the
//! testbed: before [`super::fakes::BranchCommittees`] the committee was a pure
//! function of the epoch, so "two nodes hold the same record" was a statement
//! about the FAKE and not about the code reading it.

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

/// The height the record test tombstones node 0 from — chosen because the four
/// nodes' anchors for epoch 3 STRADDLE it, so the contract's live flag is
/// `true` for some nodes' read of that epoch and `false` for the others'. The
/// test asserts that straddle rather than assuming it: a fixture that stops
/// discriminating must say so.
const TOMBSTONE_FROM: u64 = 64;

fn last(epoch: u64) -> u64 {
    (epoch + 1) * EPOCH_LEN - 1
}

fn reached(h: u64) -> impl Fn(&Progress) -> bool + Send + 'static {
    move |p| p.min_height() >= h
}

/// True once every node's `SafetyHalt` has been engaged for `ticks` CONSECUTIVE
/// driver samples.
///
/// Not "all halted" on its own, and the difference is the whole observation: a
/// halt is asserted by what stops happening after it, so the run has to keep
/// going past the edge or there is nothing standing still to look at. The
/// counter resets on any tick where a node is not halted, so a single sample
/// taken mid-engage cannot satisfy it.
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

/// The (B2)/(C9) rotation `super::tests` and `super::preconditions` share:
/// 4 → 3 → 4, node 3 out for epochs 3 and 4. Reproduced here rather than
/// imported because the two modules are siblings, and because the branching
/// twin below has to agree with it member for member.
fn rotate_four_three_four() -> Committees {
    Committees::Schedule(Arc::new(|epoch, n| {
        Some(match epoch {
            3 | 4 => vec![0, 1, 2],
            _ => (0..n).collect(),
        })
    }))
}

/// [`rotate_four_three_four`] on the CANONICAL branch, and a committee nobody
/// elected on every other hash of the reading node's own tree.
///
/// The canonical half is not a copy of that schedule, it IS that schedule: the
/// stand builds its peer-set expectations from the plain `Committees`, so the
/// two must agree member for member, and reproducing the `3 | 4 => [0, 1, 2]`
/// table here would have made that agreement a comment. `n` comes from the
/// caller because [`BranchCommittees`] — unlike [`Committees`] — is not handed
/// the roster size, and hard-coding `0..4` would drift silently the first time
/// a fixture runs a different `n`.
///
/// The speculative half is a ROTATION of the seats, not a subset: it has the
/// same size, so a consumer that read it would still build a working committee
/// and a working certificate scheme — just the wrong one, silently, which is
/// the failure this fake exists to make visible. A smaller set would have been
/// caught by the committee-floor guards long before any record was compared,
/// and would therefore have pinned those guards instead of the read cursor.
fn branching_rotation(n: usize) -> BranchCommittees {
    let Committees::Schedule(flat) = rotate_four_three_four() else {
        unreachable!("rotate_four_three_four is a Schedule");
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

/// Every `(epoch, facts)` node `i` holds a RECORD for.
fn records(out: &Outcome, i: usize) -> BTreeMap<u64, CommitteeFacts> {
    out.committee_records[i]
        .iter()
        .filter_map(|(e, answer)| answer.as_ref().ok().map(|f| (*e, f.clone())))
        .collect()
}

/// The first hash that lived in a node's EXECUTED TREE while its canonical
/// chain did not hold it — the state a reader on a speculative cursor would
/// land on, and the premise of anything asserted about `Branch::Speculative`.
/// `None` when every derive was canonicalized by the very next EL event.
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

/// (4.1, record) Four nodes at four different heights hold ONE committee record
/// per epoch — over a contract whose answer depends on the BRANCH the reading
/// hash sits on.
///
/// The fixture is (C9): node 3 rotates out at epoch 3, falls behind, and the
/// production re-jump carries it back, so at every moment of the run the four
/// nodes' anchors are four different heights — which the records themselves
/// then say, because each one carries the `(height, hash)` it was read at.
///
/// WHAT MAKES IT FALL BEHIND (5.1). Not the rotation: since П-3 a rotated-out
/// node fetches the epoch key as an artifact over `BEACON_RESOLVER_CHANNEL`
/// (R-121/R-122) and stays in lockstep, which left this run with one anchor and
/// no re-jump (measured: every node at 168 and `tree_only=[None, None, None,
/// None]`). The lag is therefore built by a PHYSICAL cut of node 3
/// ([`Stand::partition`], both planes) inside epoch 2, healed an epoch later, and
/// the cut has to be physical: a consensus-plane-only cut leaves the frontier
/// probe feeding the node, which then climbs the whole way by JUMPS and DERIVES
/// nothing above its park — and with no derive of its own there is no hash for a
/// landing to arrive over, which is the state PREMISE 2 needs (measured on that
/// variant: `jump_calls[3]` non-empty, `tree_only` still all `None`). Under the
/// physical cut it comes back the way a restarted node does — a burst of
/// certificates its executor derives against while the fork-choice updates trail
/// — and the derive the landing arrives over is the tree-only hash.
///
/// What makes it a test and not a tautology: [`branching_rotation`] hands a
/// DIFFERENT committee of the same size to any read taken off the reading
/// node's canonical chain. So "every node holds the same record" now means
/// "every node read on its own canonical chain", and the three observations
/// below say that three different ways — the values agree, no read resolved on
/// a speculative hash, and every record's anchor hash is the one that node's
/// canonical chain really holds at that height (read out of `Outcome::hashes`,
/// not out of the fake). Of the three only the first and the third can catch a
/// module-level regression: the speculative counter is zero BY CONSTRUCTION for
/// this reader and guards the fake and 4.2's future ones instead — the comment
/// on observation (b) says why.
///
/// The TOMBSTONE is the second half of the same claim, and the reason the run
/// sets one at all: the contract's equivocation flag is read LIVE at the call's
/// own block while everything beside it is frozen, so with the four anchors
/// straddling [`TOMBSTONE_FROM`] the same epoch is read `tombstoned` by some
/// nodes and not by others. The records still have to agree — which is exactly
/// what `CommitteeRecord` carrying no `tombstoned` leg buys, and what folding
/// the flag into the frozen record would break.
///
/// Falsifier: the two branches answering the same thing (then the fake is the
/// pre-step one and the equality is vacuous); no node ever holding a tree-only
/// hash (then `Branch::Speculative` is unreachable state and the counter proves
/// nothing); every node reading at the same anchor (then the equality is
/// "one hash, one answer"); the anchors no longer straddling the tombstone
/// height (then the live flag is the same for everyone and proves nothing);
/// any node's record differing in members, weights or the `dkgQual` bit; any
/// read resolving on a speculative hash; a record whose anchor hash is not the
/// node's canonical hash at that height.
#[test]
fn four_nodes_at_four_heights_hold_one_committee_record_per_epoch() {
    let mut cfg = StandConfig::live(4, 1);
    let roster = cfg.n;
    // The canonical branch and `committees` agree member for member BY
    // CONSTRUCTION — `branching_rotation` is built out of the same closure.
    cfg.committees = rotate_four_three_four();
    cfg.committees_by_branch = Some(branching_rotation(roster));
    // Node 0 sits in every epoch's committee, so the flag reaches every read
    // above the height; node 3 is rotated out of epochs 3 and 4 and would not.
    cfg.tombstoned = vec![(0, TOMBSTONE_FROM)];
    cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD.min(EPOCH_LEN));
    let mut stand = Stand::new(cfg);
    stand
        .partition(&[0, 1, 2], &[3])
        .after_height(2 * EPOCH_LEN + 4)
        .for_views(EPOCH_LEN as u32 + 8);
    let out = stand.run_until(reached(5 * EPOCH_LEN + 8), Duration::from_secs(400));
    let n = out.heights.len();
    assert!(!out.timed_out, "heights {:?}", out.heights);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    assert!(out.errors().is_empty(), "{:?}", out.errors());

    // PREMISE 1: the contract really does answer two different committees. The
    // schedule is asserted here, on the closure itself, so a future edit that
    // collapses the two branches fails HERE rather than turning every
    // assertion below into a truth about nothing.
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

    // PREMISE 2a: the fixture's own two halves — the cut fired and healed, and the
    // gate carried node 3 back by a re-jump. Without both there is no catching-up
    // node and every observation below is about four nodes in lockstep.
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

    // PREMISE 2: `Branch::Speculative` is REACHABLE state on this run — the
    // catching-up node really did hold a hash in its executed tree that its
    // canonical chain never took (the derive the re-jump landed over).
    let tree_only: Vec<Option<(u64, B256)>> =
        (0..n).map(|i| tree_only_hash(&out.el_events[i])).collect();
    assert!(
        tree_only.iter().any(|w| w.is_some()),
        "no node ever held a tree-only hash, so the speculative branch is state this run \
         cannot produce: {tree_only:?}"
    );

    // PREMISE 3: the nodes read at DIFFERENT anchors. Without this the records
    // would agree because they came from ONE state, which is not the property.
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

    // PREMISE 4: the contract's LIVE tombstone really split the nodes — the same
    // epoch was read with the flag by some of them and without it by the
    // others, which is the only arrangement under which a frozen record
    // carrying the flag would differ across nodes.
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

    // OBSERVATION (a): one record per epoch, byte for byte, across every node
    // that holds one — everything the contract froze, and nothing about WHERE
    // it was read.
    //
    // Over `split_anchors` and NOT over every epoch: an epoch every node read
    // at the SAME anchor is one hash and one contract answer, so its equality
    // is arithmetic and would hold for a reader that resolved its hash any way
    // at all (epoch 0 is read at `(0, 0x00…)` by everyone, and the top epoch at
    // the common tip). PREMISE 3 is what makes this set non-empty; the same cut
    // is what `read_back` does in the backfill test below.
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

    // OBSERVATION (b): not one read resolved on a hash off the reading node's
    // canonical chain — counted inside the fake, so it is what the node ASKED
    // and not what the stand arranged.
    //
    // For the COMMITTEE MODULE this is unreachable by construction, and saying
    // so is the honest reading of a zero here. The module's only source of a
    // read hash is `Anchor::executed_hash` (`committee/store.rs:467-477`),
    // which the stand answers out of `FakeChain::spec_hash_at` — the canonical
    // map `branch_of` compares against (`fakes.rs:1085-1090`) — and production
    // answers out of `provider.block_hash` (`executed.rs:57-72`), canonical
    // too. So the counter is not a regression detector for the module: it
    // guards the FAKE (a `by_branch` schedule that started answering off the
    // canonical map would show up here) and the consumers 4.2 introduces, which
    // resolve their own hash off a cursor — the jump's `build_at`
    // (`fakes.rs:1369`) and the tree-only marker a replayed node is handed
    // (`FakeChain::note_hash`). A reader that took a different CANONICAL height
    // would still read `Canonical` and is caught by observation (c), not here.
    for i in 0..n {
        assert!(
            out.staking_reads[i].speculative.is_empty(),
            "node {i} read the staking state on a branch its own canonical chain does not \
             hold: {:?}",
            out.staking_reads[i]
        );
    }

    // OBSERVATION (c): the same claim from OUTSIDE the fake — every anchor a
    // record names is the hash this node's finalized-executed chain holds at
    // that height.
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
         straddled={straddled:?} tombstoned_seen={:?} tree_only={tree_only:?}",
        out.heights,
        per_node
            .iter()
            .map(|r| r.iter().map(|(e, f)| (*e, f.anchor.0)).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        (0..n)
            .map(|i| out.staking_reads[i].tombstoned_seen.clone())
            .collect::<Vec<_>>(),
    );
}

/// (4.1, backfill) A node below the chain refuses the epochs it cannot see
/// WITHOUT an EVM call, and pays exactly one snapshot per epoch it can.
///
/// Two runs over one schedule, because "before the catch-up" and "after it" are
/// two different chains and not two moments of one: with the re-jump gate shut
/// (the stand default) the rotated-out node parks at `last(2)` for good, and
/// with the gate at production's own value the same node comes back.
///
/// WHAT HOLDS THE PARK (5.1). The rotation alone no longer does. Since П-3 the
/// epoch key is an artifact any node may ASK a member for over
/// `BEACON_RESOLVER_CHANNEL` (R-121/R-122), and the tracked peer set is
/// `committee[E-1] ∪ committee[E] ∪ committee[E+1]`, so a node rotated out at
/// epoch 3 keeps its consensus links through epoch 3, fetches `PK_3` and follows
/// the chain (measured: `heights=[168, 168, 168, 168]`). Both runs therefore take
/// the lag from a CONSENSUS-PLANE cut instead
/// ([`super::stand::CutPlanes::ConsensusOnly`]),
/// inside epoch 2 so node 3 still holds every key up to `PK_2`: its frontier
/// probe keeps feeding its marshal — which is what the window refusals below are
/// asked about — while its execution cannot cross into epoch 3. Run A never
/// heals the cut (the park is "for good"); run B heals it two epochs later, which
/// is what "the same node comes back" means now.
///
/// WHICH refusal arm a backfilling node takes is arithmetic, and it is not the
/// one the plan named. `commit_height(E) = start(E − 2)` and the window top is
/// `epoch(anchor) + 2`, so every epoch INSIDE the window has
/// `commit_height(E) <= start(epoch(anchor)) <= anchor` — the
/// `NotReadable{below its commit height}` arm is unreachable for an in-window
/// epoch once the geometry is frozen, and the only thing left for a parked node
/// asking about the live chain is `OutOfWindow{above}`. `NotReadable` still
/// fires on this run, at the ONE anchor where it can: height 0, where epochs 1
/// and 2 are committed by the first executed block and not by genesis. Both
/// arms answer without touching the contract, which is the property; the test
/// asserts them separately so neither can stand in for the other.
///
/// Both counters come out of `Outcome::metrics_before_collect` — the snapshot
/// the stand takes before it polls every module for `committee_records` — so
/// they say what the RUN asked and not what asserting on the run happened to
/// ask. The post-run poll's own share is printed at the end and asserted on
/// nowhere.
///
/// `dpos_committee_not_readable_total` is ONE name over four sites
/// (`committee/store.rs:429, 449, 470, 490`), so the count alone cannot name an
/// arm. The arm is identified by price instead — see the comment on
/// observation (b).
///
/// Falsifier: the parked node not parked at `last(2)`; the members not running
/// past it; a single contract read for an epoch the parked node was refused
/// (then the refusal cost an EVM call after all); a zero `out_of_window{above}`
/// counter over the run (then nobody ever ASKED, and "no read" is vacuous); a
/// zero `not_readable` counter with epoch 1 nonetheless readable at height 0;
/// a snapshot paid for epoch 1 or 2 while the anchor was still below their
/// commit height; a second snapshot for an epoch already held; the caught-up
/// node not holding the epochs it was refused, or holding a different record
/// for them.
#[test]
fn a_node_below_the_chain_refuses_what_it_cannot_see_without_an_evm_call() {
    let members = [0usize, 1, 2];
    let end = 5 * EPOCH_LEN + 8;
    /// Inside epoch 2: node 3 holds every key up to `PK_2` and nothing above it.
    const CUT_AT: u64 = 2 * EPOCH_LEN + 4;
    /// Longer than either run's virtual deadline — the cut never heals.
    const NEVER: u32 = 4096;
    /// Two epochs of cut: at the heal node 3 is far enough behind for
    /// production's own gate to arm, which is what run B is about.
    const HELD_FOR: u32 = 2 * EPOCH_LEN as u32 + 8;

    // Run A — the gate shut: node 3 parks at the last block of epoch 2.
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let parked = metrics::with_local_recorder(&recorder, || {
        let mut cfg = StandConfig::live(4, 1);
        cfg.committees = rotate_four_three_four();
        // The counters the two refusal assertions below stand on are taken by
        // the stand itself, BEFORE it polls every node's module for
        // `committee_records` — see `StandConfig::metrics_snapshotter`.
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
    // What the RUN counted, and — separately — what the post-run poll added on
    // top of it. The second is reported and never asserted on: it is the number
    // this test would have been measuring if it drained once, at the end.
    let drained = &parked.metrics_before_collect;
    let by_collect = drain_counters(&snap);
    assert!(!parked.timed_out, "heights {:?}", parked.heights);
    assert!(parked.halted.is_empty(), "{:?}", parked.halted);
    assert!(parked.errors().is_empty(), "{:?}", parked.errors());

    // PREMISE: node 3 is parked at `last(2)` and the chain is two epochs past it.
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

    // OBSERVATION (a): the refusals cost NOTHING. Not one contract read — of
    // any kind, by any consumer of this node — named an unseen epoch.
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
    // …and they really HAPPENED, IN THE RUN. `drained` is the stand's
    // pre-collect snapshot precisely for this assertion: `committee_records`
    // above is itself produced by calling `committee(e)` for `0..=max+2` on
    // every node, which refuses the parked node's epochs 5, 6 and 7 all over
    // again. Counted at the end of everything, this number can never be zero,
    // and the assertion would have been a statement about its own evidence.
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

    // OBSERVATION (b): the OTHER no-EVM arm, at the one anchor that reaches it.
    // Epoch 1 is committed by the first EXECUTED block and not by genesis, so a
    // node whose anchor is still 0 is refused it arithmetically — and the
    // record it eventually holds proves the anchor had to move first.
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
    // WHICH not-readable arm. The counter is one name over four sites
    // (`committee/store.rs:429, 449, 470, 490`), so it alone says "some arm",
    // not "the arithmetic one". What separates them is PRICE: the commit-height
    // arm (`:449`) refuses before the anchor hash is even resolved, while the
    // empty-contract-answer arm (`:490`) refuses AFTER both staticcalls. So if
    // the refusal of epoch 1 at anchor 0 had gone through `:490`, the module
    // would have paid a snapshot there and another one after the anchor moved,
    // and `module_snapshot[1]` would be at least 2. It is exactly 1 — the read
    // that produced the record asserted just above.
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

    // OBSERVATION (c): ONE snapshot per epoch held, on every node. The refusals
    // above are free and the successes are paid for exactly once — write-once
    // memoisation, counted at the module's own port.
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

    // Run B — the same schedule with production's gate: the node comes back,
    // and the epochs it was refused turn into records with the same one-call
    // price and the same value the members hold.
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
    // The epochs it was refused and then really READ — the tail of `unseen`
    // sits above where this run ever went, and the record the collection phase
    // took for those is not a read the RUN made. Epoch 5 is asserted to be in
    // the set so the price below is not a price of nothing.
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

/// (4.1, impossible answer, R-128) `weights: None` inside the read window is a
/// PERMANENT refusal that STOPS the node: no record, no certificate scheme, one
/// `error!` from the module, the epoch's slot poisoned so no further staticcall
/// is spent on it, and — the part this test exists for since R-128 — the
/// `SafetyHalt` latch engaged with `ContractFork` on every node that had to
/// enter the epoch, after which nothing moves.
///
/// The contract's frozen weights live in a ring of `WEIGHT_RING_EPOCHS` frames
/// and go missing 14 epochs below the reading height — far outside the module's
/// own window (`committee/mod.rs::WINDOW_FITS_THE_WEIGHT_RING`), so a stand
/// cannot reach this arm by running long enough. `StandConfig::weights_none_for`
/// is the switch that makes the contract answer it anyway, which is the whole
/// point: the module treats it as "the contract answered something no committed
/// epoch can answer", not as a missing optional to fall back from.
///
/// Every node halts, and that is the DESIGNED outcome rather than a stand
/// artifact: all four read the same contract, so an impossible answer is
/// correlated by construction — which is exactly why the old behaviour (skip
/// the epoch, keep running, one log line) was the defect. `run_until` therefore
/// waits past the halt with [`halted_for`], so the standing still below can be
/// observed rather than assumed.
///
/// Falsifier: the chain not reaching the end of epoch 1 (then "the chain ran up
/// to the epoch it refused" is untested); the contract not actually answering
/// `weights: None` (then the switch is inert); a record, or a scheme of ANY
/// strength, for the epoch; a refusal that is TRANSIENT (then a consumer would
/// spin on it for ever); a silent refusal, or one logged more than once per
/// node; a node that keeps finalizing after its latch engaged; a second
/// staticcall for the poisoned epoch.
#[test]
fn a_weightless_committee_inside_the_window_stops_every_node_that_must_enter_it() {
    let epoch = 2u64;
    // How many driver ticks the run stays up after the last node halted — the
    // window the marshal tip is then asserted to stand still in. Ten ticks is
    // one virtual second (`stand::POLL`), several block times at the stand's
    // pace, so a chain that was still finalizing would move inside it.
    const AFTER_HALT_TICKS: usize = 10;
    let recorder = DebuggingRecorder::new();
    let snap = recorder.snapshotter();
    let out = metrics::with_local_recorder(&recorder, || {
        let mut cfg = StandConfig::honest(4, 1);
        cfg.weights_none_for = Some(epoch);
        // The run's own counters, snapshotted before the post-run poll of
        // `committee_records` — which re-reads the refused epoch on every node.
        // Since R-128 that re-read is answered from the poisoned slot and costs
        // no contract call, but it still TICKS the refusal counter, which is
        // what this snapshot keeps out of the numbers below.
        cfg.metrics_snapshotter = Some(snap.clone());
        // The marshal tip, sampled once per driver tick — the "and then nothing
        // moved" half of the halt.
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

    // PREMISE 1: the chain RAN THROUGH the epoch below the refused one, and
    // every node was OWED the refused epoch — the halt is the end of a working
    // chain reaching a boundary it must cross, not a node that never started.
    //
    // The ORDERING plane is the one asserted to have completed epoch 1: the
    // boundary that asks for `committee[2]` is delivered off a finalized
    // ORDER block, and execution trails it by `K` under deferred execution, so
    // a node that halts at the boundary stops with its executed tier up to `K`
    // blocks short of `last(1)`. Asserting the executed tier alone would be
    // asserting the lag, not the premise.
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

    // PREMISE 2: the contract really answered `weights: None` for that epoch and
    // for no other, on every node.
    for i in 0..n {
        let weightless: Vec<u64> = out.staking_reads[i].weights_none.keys().copied().collect();
        assert_eq!(
            weightless,
            vec![epoch],
            "node {i}'s contract answered the weightless committee for the wrong epochs: {:?}",
            out.staking_reads[i]
        );
    }

    // OBSERVATION (a): EVERY node is safety-halted, with the typed reason.
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

    // OBSERVATION (b): and then nothing moved. The marshal tip — the node's own
    // VERIFIED frontier — is identical across the last `AFTER_HALT_TICKS`
    // samples, which are the ticks taken after the last latch engaged.
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

    // OBSERVATION (c): no record, and no scheme either — the epoch is absent
    // from the ONE map, so nothing downstream can be built over it.
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
        // `verifier_epochs` lists VERIFY-ONLY schemes alone (`store.rs` filters
        // on `me().is_none()`), so on its own it would miss a SIGNER scheme for
        // the refused epoch — the one an engine spawn installs. Ask the module
        // directly for both.
        assert!(
            out.committees[i].scheme(epoch).is_none(),
            "node {i} holds a certificate scheme for the epoch it refused"
        );
        // The epoch BELOW it is held, so the absence above is this answer and
        // not a node that read nothing at all.
        assert!(
            out.committee_records[i][&(epoch - 1)].is_ok(),
            "node {i} holds no record for epoch {} either: {:?}",
            epoch - 1,
            out.committee_records[i]
        );
    }

    // OBSERVATION (d): the slot is POISONED — the epoch cost the contract
    // EXACTLY ONE snapshot call on each node, for the whole run plus the
    // post-run poll of every consumer's question. `==`, not `>=`: the
    // memoisation is the property (B3-12), and a second call would mean the
    // refusal is being re-derived on the hot path again.
    for i in 0..n {
        assert_eq!(
            out.staking_reads[i].module_snapshot.get(&epoch),
            Some(&1),
            "node {i} paid for the impossible epoch more than once: {:?}",
            out.staking_reads[i]
        );
    }

    // OBSERVATION (e): LOUD, and exactly once per node for each of the two
    // lines this failure owes — the module's refusal and the manager's halt.
    // The stand's capture carries no node label (`super::mod`'s doc), so "once
    // per node" is a count over one process, which is why the ERROR set must
    // contain nothing else.
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
    // The third line each node owes, and the one that says the halt REACHED
    // execution: the executor parks instead of deriving.
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

    // OBSERVATION (f): counted under its own cause, at least once per node.
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

/// (4.1, revert, R-128) A committee read that REVERTS inside the window is the
/// OTHER permanent class: the epoch is refused loudly and is not registered,
/// and the node does NOT stop.
///
/// The contrast is the point. A revert says the read could not be served — a
/// staking-module code error, a read of a module that is not there — and an
/// operator repairs it and restarts; it is not the chain stating a committee
/// that cannot exist, so it does not carry the one thing that justifies
/// stopping a node. The design writes the two lines separately
/// (`E4-CORE-DESIGN.md` §5.4: revert ⇒ `error!`, the epoch is not registered;
/// impossible ⇒ the node stands), and before R-128 the code could not tell them
/// apart because it only ever had the weaker outcome.
///
/// Falsifier: the contract not actually reverting (then the switch is inert); a
/// record for the epoch; a halted node; a run that never reached the epoch
/// below; more than one `error!` per node.
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

    // PREMISE 1: the chain ran up to the epoch whose read reverts.
    for i in 0..n {
        assert!(
            out.heights[i] >= last(epoch - 1),
            "node {i} did not reach the end of epoch {}: {:?}",
            epoch - 1,
            out.heights
        );
    }

    // PREMISE 2: the contract really reverted, for that epoch and no other, on
    // every node.
    for i in 0..n {
        let reverted: Vec<u64> = out.staking_reads[i].reverted.keys().copied().collect();
        assert_eq!(
            reverted,
            vec![epoch],
            "node {i}'s contract reverted for the wrong epochs: {:?}",
            out.staking_reads[i]
        );
    }

    // OBSERVATION (a): NOBODY halted. This is the line that separates the two
    // permanent classes, and it is the whole reason this fixture exists.
    assert!(
        out.halted.is_empty(),
        "a revert stopped a node: {:?}",
        out.halted
    );

    // OBSERVATION (b): the epoch is not registered — no record, no scheme.
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

    // OBSERVATION (c): one `error!` per node, and no halt line among them.
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
