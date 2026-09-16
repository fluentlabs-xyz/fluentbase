//! The stand's `CertInlet`: the second producer into a node's marshal. These tests
//! see the inlet's verify gate, its non-fault deferral, its data-fault rotation,
//! and that the beacon clock of a node fed by an inlet is its marshal's
//! tip. The frontier plane's `deliver` builds its verifier without an oracle, so
//! the inlet is where a swapped σ slot is first judged; it also refuses a
//! certificate whose epoch this node cannot read, so the deferral is reachable
//! only through `CertInletSource::PeerArchive`. `CertInletFacts::delivered` is
//! recorded at the marshal seam (`MarshalSink::verify_block`). The negative
//! control is `StandConfig::cert_inlet = None`, and every test asserts
//! `only_these_ran_inlets`.

use super::stand::{
    CertInletCfg, CertInletFacts, CertInletSource, Committees, Outcome, Progress, Stand,
    StandConfig, BLOCKER_SITE_CONSENSUS, BLOCKER_SITE_FRONTIER,
};
use crate::beacon::testing::DETERMINISTIC_BOOTSTRAP_EPOCH;
use metrics_util::debugging::Snapshotter;
use std::{sync::Arc, time::Duration};

/// The epoch length every test sets on its config; the height arithmetic below is
/// computed from this constant.
const EPOCH_LEN: u64 = 32;
/// The first height of the bootstrap epoch; epochs below it run seedless, and
/// `FORGE_WINDOW` starts here.
const EPOCH_2_START: u64 = DETERMINISTIC_BOOTSTRAP_EPOCH * EPOCH_LEN;

fn reached(h: u64) -> impl Fn(&Progress) -> bool + Send + 'static {
    move |p| p.min_height() >= h
}

/// Exactly the nodes `expected` names ran an inlet, and every other node's slot
/// is `None` — the file's negative control as an assertion.
fn only_these_ran_inlets(out: &Outcome, expected: &[usize]) {
    let ran: Vec<usize> = out
        .cert_inlet
        .iter()
        .enumerate()
        .filter_map(|(j, f)| f.is_some().then_some(j))
        .collect();
    assert_eq!(
        ran, expected,
        "the inlets that ran are not the ones the config named"
    );
}

/// The facts of node `i`'s inlet, or a panic naming the whole vector; an
/// assertion about an inlet that was never spawned is a fixture error.
fn inlet(out: &Outcome, i: usize) -> &CertInletFacts {
    out.cert_inlet[i]
        .as_ref()
        .unwrap_or_else(|| panic!("node {i} ran no cert-inlet: {:?}", out.cert_inlet))
}

/// Node `i`'s two clock gauges at the end of the run, `(ordering, dkg)`; both
/// `expect`, since a node that publishes neither ran no beacon actor.
fn clock_pair(out: &Outcome, i: usize) -> (u64, u64) {
    let ordering = out
        .metric(i, "dpos_ordering_finalized_height")
        .expect("FluentApp gauges the ordering tip on a node with a registered PlaneClock");
    let dkg = out
        .metric(i, "dpos_dkg_clock_height")
        .expect("the DkgActor gauges its clock on a Beacon::Live node");
    (ordering as u64, dkg as u64)
}

/// `epoch >= 2 ⇒ committee is {0,1,2}`, so a node without `PK_2` exists.
fn drop_the_last_two_from_epoch_two() -> Committees {
    Committees::Schedule(Arc::new(|epoch, n| {
        Some(if epoch >= 2 {
            vec![0, 1, 2]
        } else {
            (0..n).collect()
        })
    }))
}

/// (5.0а, the clean path) An inlet on a healthy committee member is a second
/// producer into the marshal its own BFT engine already drives and changes
/// nothing: every attempt is a clean ingest, the by-height walk is contiguous
/// from 1, the beacon clock is the marshal's tip, and the chain stays in lockstep.
#[test]
fn an_inlet_on_a_healthy_member_verifies_every_height_it_is_fed_and_hands_it_to_the_marshal() {
    const TARGET: u64 = 72;
    let mut cfg = StandConfig::live(4, 1);
    cfg.epoch_len = EPOCH_LEN;
    cfg.cert_inlet = Some(CertInletCfg {
        nodes: vec![3],
        source: CertInletSource::NextAboveTier,
    });
    let out = Stand::new(cfg).run_until(reached(TARGET), Duration::from_secs(200));
    assert!(!out.timed_out, "heights {:?}", out.heights);

    // Non-vacuity: an inlet that never saw a certificate satisfies everything
    // below.
    let f = inlet(&out, 3);
    assert!(
        f.ingests >= TARGET / 2,
        "the inlet on node 3 was barely fed ({} ingests over {} heights): {f:?}",
        f.ingests,
        out.heights[3]
    );

    assert_eq!(
        f.delivered.len() as u64,
        f.ingests,
        "an honest run took a fault or a deferral arm: {f:?}"
    );
    assert_eq!(f.rotations, 0, "an honest upstream cost a rotation: {f:?}");
    assert_eq!(f.defers, 0, "an honest run deferred a certificate: {f:?}");

    let expected: Vec<u64> = (1..=f.ingests).collect();
    assert_eq!(
        f.delivered, expected,
        "the delivered list is not the contiguous walk the feeder asked for: {f:?}"
    );

    let delivered_top = *f.delivered.last().expect("non-empty");
    let (ordering, dkg_clock) = clock_pair(&out, 3);
    assert!(
        dkg_clock <= ordering,
        "the DkgActor's clock ({dkg_clock}) is ABOVE the marshal's tip ({ordering}) — a \
         feeder other than the tip is back"
    );
    assert_eq!(
        dkg_clock, ordering,
        "at rest the DkgActor's clock is the marshal's tip, and it is not"
    );
    assert!(
        dkg_clock >= delivered_top,
        "the DkgActor's clock ({dkg_clock}) never reached the highest height the inlet \
         handed the marshal ({delivered_top})"
    );

    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[]);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    only_these_ran_inlets(&out, &[3]);
    eprintln!(
        "(5.0а/clean) heights={:?} ingests={} delivered=[{}..{}] rotations={} defers={} \
         ordering={ordering} dkg_clock={dkg_clock} virtual={:?}",
        out.heights,
        f.ingests,
        f.delivered[0],
        delivered_top,
        f.rotations,
        f.defers,
        out.virtual_elapsed
    );
}

/// (5.0а, Ex-21 stand half — the keyed arm) A σ-forged certificate served over a
/// healthy connection fails the inlet's BLS verify, counts as a data fault, and
/// after `MAX_UPSTREAM_FAULTS` consecutive ones the inlet rotates away from the
/// upstream. The victim is a committee member, so it holds `PK_2` at verify time.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn a_forged_seed_slot_costs_the_upstream_a_rotation_once_the_epoch_key_is_held() {
    // Imported in the body so a build without the byzantine feature has no unused
    // import.
    use super::{byzantine_roles::FORGE_WINDOW, stand::Role};
    const FORGER: usize = 0;
    const VICTIM: usize = 4;
    let mut cfg = StandConfig::live(5, 1);
    cfg.epoch_len = EPOCH_LEN;
    // Node 4 sits in every committee, so it deals the epoch-2 DKG and holds `PK_2`
    // before its tier-F reaches the forged window.
    cfg.committees = Committees::All;
    // The victim's by-height pulls go to the forger alone; the forger keeps every
    // link because it is also an honest committee member.
    cfg.upstream_source_only_for = Some((VICTIM, FORGER));
    cfg.cert_inlet = Some(CertInletCfg {
        nodes: vec![VICTIM],
        source: CertInletSource::NextAboveTier,
    });
    let mut stand = Stand::new(cfg);
    stand.node(FORGER).role(Role::ForgedSeedUpstream);
    let out = stand.run_until(
        reached(*FORGE_WINDOW.end() + EPOCH_LEN),
        Duration::from_secs(300),
    );
    assert!(!out.timed_out, "heights {:?}", out.heights);

    let byz = &out.byz[FORGER];
    assert!(
        byz.certs_forged > 0,
        "the forger served nothing forged: {byz:?}"
    );
    assert!(
        byz.forged_seed_differs,
        "a 'forged' σ read back as the original: {byz:?}"
    );
    assert!(
        byz.forged_vote_half_intact,
        "the forge touched the multisig half — then the frontier's own verifier \
         would have refused it and the inlet was never the gate: {byz:?}"
    );
    for h in &byz.forged_heights {
        assert!(
            FORGE_WINDOW.contains(h),
            "the forger forged {h}, outside its window: {byz:?}"
        );
    }

    assert!(
        out.artifacts[VICTIM].contains_key(&2),
        "node {VICTIM} holds no epoch-2 artifact, so nothing could check a σ: {:?}",
        out.artifacts[VICTIM].keys().collect::<Vec<_>>()
    );

    // The victim's feeder walked the window: every height it delivered there is one
    // the forger did not forge, so the rest were taken as faults.
    let f = inlet(&out, VICTIM);
    let delivered_in_window: Vec<u64> = f
        .delivered
        .iter()
        .copied()
        .filter(|h| FORGE_WINDOW.contains(h))
        .collect();
    let ingested_forged: Vec<u64> = byz
        .forged_heights
        .iter()
        .copied()
        .filter(|h| !delivered_in_window.contains(h))
        .collect();
    assert!(
        ingested_forged.len() >= crate::cert_inlet::MAX_UPSTREAM_FAULTS as usize,
        "the victim did not take {} forged certificates in a run — forged={:?} \
         delivered_in_window={delivered_in_window:?}: {f:?}",
        crate::cert_inlet::MAX_UPSTREAM_FAULTS,
        byz.forged_heights
    );

    // With `defers == 0` every ingest is either clean or a data fault, so the streak
    // resets only at the threshold and `rotations == faults / MAX_UPSTREAM_FAULTS`.
    const T: u64 = crate::cert_inlet::MAX_UPSTREAM_FAULTS as u64;
    assert_eq!(
        f.defers, 0,
        "a deferral muddies the fault arithmetic: {f:?}"
    );
    let faults = f.ingests - f.delivered.len() as u64;
    assert_eq!(
        faults,
        ingested_forged.len() as u64,
        "the victim's faults ({faults}) are not exactly the forged certificates it \
         took ({:?}) — some other ingest failed: {f:?}",
        ingested_forged
    );
    assert_eq!(
        f.rotations,
        faults / T,
        "{faults} consecutive data faults must cost exactly {} rotation(s) at a \
         threshold of {T}: {f:?}",
        faults / T
    );

    // Every verify failure was judged with `PK_2` resolved, on the line that only
    // increments in that regime.
    assert_eq!(
        f.carry_forward_fails,
        ingested_forged.len() as u64,
        "the victim's verify failures judged under a RESOLVABLE epoch key ({}) are \
         not the forged certificates it took ({:?}) — then the keyed arm is not what \
         this run exercised: {f:?}",
        f.carry_forward_fails,
        ingested_forged
    );

    // The forger is a frontier-plane liar only: the committee is whole and the chain
    // is one.
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[]);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    only_these_ran_inlets(&out, &[VICTIM]);
    eprintln!(
        "(5.0а/Ex-21 keyed) heights={:?} forged={:?} ingested_forged={ingested_forged:?} \
         delivered_in_window={delivered_in_window:?} ingests={} rotations={} defers={} \
         carry_forward_fails={} virtual={:?}",
        out.heights,
        byz.forged_heights,
        f.ingests,
        f.rotations,
        f.defers,
        f.carry_forward_fails,
        out.virtual_elapsed
    );
}

/// (5.0а, Ex-21 stand half — the keyless arm) A non-member whose EL is held behind
/// the committee, with a live inlet: it admits an epoch-2 certificate without
/// checking the σ (it holds no `PK_2`), and the plane's own gate stops what it can
/// see at the top of its committee-read window.
#[test]
fn a_keyless_admission_is_all_the_plane_lets_an_outrun_inlet_see() {
    use fluentbase_types::staking_protocol::{epoch_at_block, MAX_COMMITTEE_LOOKAHEAD_EPOCHS};
    use metrics_util::debugging::DebuggingRecorder;
    const VICTIM: usize = 4;
    /// Above the victim's read-window top, so the committee really does outrun it.
    const TARGET: u64 = 5 * EPOCH_LEN;
    /// Inside epoch 1 and below the epoch-2 mint: the victim holds `PK_1` and
    /// nothing above it.
    const CUT_AT: u64 = EPOCH_LEN + 4;
    /// Longer than the run's virtual deadline: the cut never heals.
    const NEVER: u32 = 4096;
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let out = metrics::with_local_recorder(&recorder, || {
        let mut cfg = StandConfig::live(5, 1);
        cfg.epoch_len = EPOCH_LEN;
        cfg.committees = drop_the_last_two_from_epoch_two();
        cfg.metrics_snapshotter = Some(snapshotter.clone());
        cfg.cert_inlet = Some(CertInletCfg {
            nodes: vec![VICTIM],
            source: CertInletSource::Frontier,
        });
        let mut stand = Stand::new(cfg);
        stand
            .partition(&[0, 1, 2, 3], &[VICTIM])
            .after_height(CUT_AT)
            .consensus_only()
            .for_views(NEVER);
        stand.run_until(
            move |p| p.min_height_of(&[0, 1, 2]) >= TARGET,
            Duration::from_secs(400),
        )
    });
    assert!(
        !out.timed_out,
        "the committee did not reach {TARGET}: {:?}",
        out.heights
    );

    // Premise: the cut fired and the lag is held; without it the victim fetches
    // `PK_2` and follows.
    assert!(
        !out.partitions[0].heights_at_cut.is_empty(),
        "the consensus-plane cut never fired: {:?}",
        out.partitions[0]
    );
    assert!(
        out.heights[VICTIM] < EPOCH_2_START,
        "node {VICTIM} executed into epoch 2 without a key: {:?}",
        out.heights
    );
    assert!(
        out.jump_calls[VICTIM].is_empty(),
        "node {VICTIM} re-jumped, so the lag was not held: {:?}",
        out.jump_calls[VICTIM]
    );
    assert!(
        !out.artifacts[VICTIM].contains_key(&2),
        "node {VICTIM} holds the epoch-2 artifact, so the keyless arm is vacuous: {:?}",
        out.artifacts[VICTIM].keys().collect::<Vec<_>>()
    );

    let f = inlet(&out, VICTIM);
    assert!(f.ingests > 0, "the inlet was never fed: {f:?}");

    // Keyless admission, counted by the inlet on the line for a beacon-active epoch
    // whose key it could not get.
    let keyless = super::stand::counter_of(
        &out.metrics_before_collect,
        crate::cert_inlet::CERT_VOTE_ONLY_ADMISSIONS,
        None,
    );
    assert!(
        keyless > 0,
        "no certificate was admitted vote-only, so the σ was never left unchecked"
    );
    assert_eq!(
        f.rotations, 0,
        "a missing key, or this node's own lag, cost a healthy upstream a rotation: {f:?}"
    );

    // The bound is the committee-read window, enforced by the plane.
    let anchor_epoch = epoch_at_block(
        out.heights[VICTIM],
        super::fakes::DPOS_ACTIVATION_BLOCK,
        EPOCH_LEN,
    )
    .expect("the victim executed at least one block");
    let window_top = (anchor_epoch + MAX_COMMITTEE_LOOKAHEAD_EPOCHS + 1) * EPOCH_LEN - 1;
    let delivered_top = *f
        .delivered
        .last()
        .expect("the inlet verified at least one certificate");
    assert!(
        delivered_top <= window_top,
        "the inlet verified height {delivered_top}, above the top of its own \
         committee-read window ({window_top}) — `deliver` let an unauthenticatable \
         certificate through"
    );
    assert!(
        delivered_top > (anchor_epoch + MAX_COMMITTEE_LOOKAHEAD_EPOCHS) * EPOCH_LEN - 1,
        "the inlet stopped at {delivered_top}, below the top window epoch — the window \
         is not what stopped it and the bound proves nothing: {:?}",
        out.heights
    );
    let dropped: u64 = ["out_of_window", "not_readable"]
        .iter()
        .map(|reason| {
            super::stand::counter_of(
                &out.metrics_before_collect,
                "dpos_frontier_dropped_total",
                Some(("reason", reason)),
            )
        })
        .sum();
    // `counter_of` sums the family over the whole recorder and the label carries no
    // node, so node 3 is in this sum too; the bound itself is pinned by the
    // victim's own `delivered_top`.
    assert!(
        dropped > 0,
        "the plane dropped nothing, so the bound above has no mechanism behind it"
    );
    assert_eq!(
        f.defers, 0,
        "the inlet deferred after all — then the plane's step-(5) drop is not total \
         and the finding in the doc comment is wrong: {f:?}"
    );

    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[3, VICTIM]);
    only_these_ran_inlets(&out, &[VICTIM]);
    eprintln!(
        "(5.0а/Ex-21 keyless) heights={:?} ingests={} keyless={keyless} rotations={} \
         defers={} delivered_top={delivered_top} window_top={window_top} plane_dropped={dropped} \
         virtual={:?}",
        out.heights, f.ingests, f.rotations, f.defers, out.virtual_elapsed
    );
}

/// (5.0а, the plan row's own role) A committee member a partition left behind, fed
/// forward by its inlet while it catches up and still a member at the end. Its own
/// marshal repair races the inlet over the same range, so the test claims only
/// that the inlet stayed live and cost the node nothing.
#[test]
fn a_catching_up_committee_member_is_fed_by_its_inlet_while_its_el_is_behind() {
    const LAGGARD: usize = 3;
    /// Early enough not to overlap the epoch-2 DKG, so the artifact assertion is
    /// about membership.
    const CUT_AT: u64 = 8;
    /// Past `epoch_start(2)`, so "still a member" can be read off the artifact of
    /// the first epoch that has one.
    const TARGET: u64 = EPOCH_2_START + EPOCH_LEN;
    let mut cfg = StandConfig::live(4, 1);
    cfg.epoch_len = EPOCH_LEN;
    cfg.cert_inlet = Some(CertInletCfg {
        nodes: vec![LAGGARD],
        source: CertInletSource::NextAboveTier,
    });
    let mut stand = Stand::new(cfg);
    stand
        .partition(&[0, 1, 2], &[LAGGARD])
        .after_height(CUT_AT)
        .for_views(8);
    let out = stand.run_until(reached(TARGET), Duration::from_secs(400));
    assert!(!out.timed_out, "heights {:?}", out.heights);

    // `heights_at_heal` is the driver's per-node tier-F sample when the links came
    // back.
    let part = &out.partitions[0];
    assert!(
        !part.heights_at_heal.is_empty(),
        "the partition never healed (or never fired): {part:?}"
    );
    let lag_tip = part.heights_at_heal[LAGGARD];
    let majority_tip = [0, 1, 2]
        .iter()
        .map(|&i| part.heights_at_heal[i])
        .min()
        .expect("three nodes");
    assert!(
        lag_tip < majority_tip,
        "the isolated member was not behind at heal, so there was no lag to feed \
         across: {part:?}"
    );

    // Only a node inside `committee[E]` deals E's DKG, so its own epoch's artifact
    // is what shows this node is a member.
    assert!(
        out.artifacts[LAGGARD].contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH),
        "node {LAGGARD} holds no epoch-{DETERMINISTIC_BOOTSTRAP_EPOCH} artifact, so it \
         is not the MEMBER this test is about: {:?}",
        out.artifacts[LAGGARD].keys().collect::<Vec<_>>()
    );

    // The inlet fed it across the catch-up window: heights it had not executed at
    // heal and the majority already had.
    let f = inlet(&out, LAGGARD);
    assert!(f.ingests > 0, "the inlet was never fed at all: {f:?}");
    let in_window: Vec<u64> = f
        .delivered
        .iter()
        .copied()
        .filter(|h| *h > lag_tip && *h <= majority_tip)
        .collect();
    assert!(
        !in_window.is_empty(),
        "the inlet verified nothing in the catch-up window ({}..={majority_tip}) — \
         then it was not feeding this node while it was behind: {f:?}",
        lag_tip + 1
    );

    // An honest donor plus this node's own lag must cost no rotation, no deferral,
    // and no verify failure under a resolvable key.
    assert_eq!(
        f.rotations, 0,
        "catching up cost the honest upstream a rotation: {f:?}"
    );
    assert_eq!(f.defers, 0, "a by-height walk deferred: {f:?}");
    assert_eq!(
        f.carry_forward_fails, 0,
        "a certificate failed BLS verify under a resolvable key: {f:?}"
    );

    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[]);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    only_these_ran_inlets(&out, &[LAGGARD]);
    eprintln!(
        "(5.0а/catch-up member) heights={:?} heal=[lag {lag_tip} vs majority \
         {majority_tip}] in_window={in_window:?} ingests={} rotations={} defers={} \
         cff={} virtual={:?}",
        out.heights, f.ingests, f.rotations, f.defers, f.carry_forward_fails, out.virtual_elapsed,
    );
}

/// The held-lag fixture over a donor's archive, shared by the two tests below that
/// differ only in what they observe on it.
const HELD_LAG_VICTIM: usize = 4;
/// The victim; the keyless test above uses the same index.
const HELD_LAG_DONOR: usize = 0;
/// The same target as the keyless test, so the walk has heights the victim cannot
/// read.
const HELD_LAG_TARGET: u64 = 5 * EPOCH_LEN;
/// Inside epoch 1 and below the epoch-2 mint, as in the keyless test.
const HELD_LAG_CUT_AT: u64 = EPOCH_LEN + 4;
/// Longer than the run's virtual deadline: the cut never heals.
const HELD_LAG_NEVER: u32 = 4096;

/// Run the held-lag fixture: five live nodes, the victim's inlet over the donor's
/// archive, the victim cut from the consensus plane inside epoch 1 for good.
/// `snapshotter` is assigned unconditionally; `None` means no snapshotter.
fn run_held_lag_over_donor_archive(snapshotter: Option<Snapshotter>) -> Outcome {
    let mut cfg = StandConfig::live(5, 1);
    cfg.epoch_len = EPOCH_LEN;
    cfg.committees = drop_the_last_two_from_epoch_two();
    cfg.metrics_snapshotter = snapshotter;
    cfg.cert_inlet = Some(CertInletCfg {
        nodes: vec![HELD_LAG_VICTIM],
        source: CertInletSource::PeerArchive {
            from: HELD_LAG_DONOR,
        },
    });
    let mut stand = Stand::new(cfg);
    stand
        .partition(&[0, 1, 2, 3], &[HELD_LAG_VICTIM])
        .after_height(HELD_LAG_CUT_AT)
        .consensus_only()
        .for_views(HELD_LAG_NEVER);
    let out = stand.run_until(
        move |p| p.min_height_of(&[0, 1, 2]) >= HELD_LAG_TARGET,
        Duration::from_secs(400),
    );
    assert!(
        !out.timed_out,
        "the committee did not reach {HELD_LAG_TARGET}: {:?}",
        out.heights
    );
    out
}

/// The fixture's premise as assertions: the cut that holds the lag fired, the
/// victim never executed into the bootstrap epoch, and it holds no key for it.
fn assert_the_lag_is_held(out: &Outcome) {
    const VICTIM: usize = HELD_LAG_VICTIM;
    assert!(
        !out.partitions[0].heights_at_cut.is_empty(),
        "the consensus-plane cut never fired: {:?}",
        out.partitions[0]
    );
    assert!(
        out.heights[VICTIM] < EPOCH_2_START,
        "node {VICTIM} executed into epoch {DETERMINISTIC_BOOTSTRAP_EPOCH} without a \
         key: {:?}",
        out.heights
    );
    assert!(
        !out.artifacts[VICTIM].contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH),
        "node {VICTIM} holds the epoch-{DETERMINISTIC_BOOTSTRAP_EPOCH} artifact, so its \
         window is not below the donor's epochs: {:?}",
        out.artifacts[VICTIM].keys().collect::<Vec<_>>()
    );
}

/// (5.0а, A-05) A donor's marshal archive walked upward by height, which reaches
/// the inlet's own non-fault deferral: a certificate for an epoch outside this
/// node's read window is skipped without counting as a data fault.
#[test]
fn a_donors_archive_hands_the_inlet_an_epoch_it_cannot_read_and_it_defers() {
    use fluentbase_types::staking_protocol::{epoch_at_block, MAX_COMMITTEE_LOOKAHEAD_EPOCHS};
    const VICTIM: usize = HELD_LAG_VICTIM;
    const DONOR: usize = HELD_LAG_DONOR;
    const TARGET: u64 = HELD_LAG_TARGET;
    let out = run_held_lag_over_donor_archive(None);

    // The same held lag as the keyless test, with the cut that holds it.
    assert_the_lag_is_held(&out);

    let f = inlet(&out, VICTIM);
    assert!(
        f.defers > 0,
        "the inlet never deferred: every certificate the donor's archive handed it \
         was inside its own read window, so this source is gated after all: {f:?}"
    );
    assert_eq!(
        f.rotations, 0,
        "an unreadable committee — this node's OWN lag — cost the donor a rotation: {f:?}"
    );

    // Every ingest took one of exactly two arms: clean or deferred; no fault arm is
    // reachable here.
    assert_eq!(
        f.ingests,
        f.delivered.len() as u64 + f.defers,
        "an ingest took neither the clean nor the deferral arm: {f:?}"
    );

    // The walk is strictly up, and its top is the top of the read window.
    assert!(
        f.delivered.windows(2).all(|w| w[0] < w[1]),
        "the archive walk repeated or went backwards: {f:?}"
    );
    let anchor_epoch = epoch_at_block(
        out.heights[VICTIM],
        super::fakes::DPOS_ACTIVATION_BLOCK,
        EPOCH_LEN,
    )
    .expect("the victim executed at least one block");
    let window_top = (anchor_epoch + MAX_COMMITTEE_LOOKAHEAD_EPOCHS + 1) * EPOCH_LEN - 1;
    let delivered_top = *f
        .delivered
        .last()
        .expect("the inlet verified at least one certificate");
    assert!(
        delivered_top <= window_top,
        "the inlet verified height {delivered_top}, above the top of its own \
         committee-read window ({window_top})"
    );
    assert!(
        f.ingests > window_top,
        "the walk never climbed past the window top ({window_top}) — then the \
         deferrals above are not the ones this test is about: {f:?}"
    );

    // One ingest per height walked; the plane feeder pays thousands for the same
    // distinct heights.
    assert!(
        f.ingests <= 2 * TARGET,
        "the archive walk cost {} ingests over a donor tip of {} — it is re-asking \
         heights, which is the plane feeder's price and the whole reason this source \
         exists: {f:?}",
        f.ingests,
        out.heights[DONOR]
    );

    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[3, VICTIM]);
    only_these_ran_inlets(&out, &[VICTIM]);
    eprintln!(
        "(5.0а/peer-archive defer) heights={:?} ingests={} delivered={} \
         delivered_top={delivered_top} window_top={window_top} defers={} rotations={} \
         virtual={:?}",
        out.heights,
        f.ingests,
        f.delivered.len(),
        f.defers,
        f.rotations,
        out.virtual_elapsed
    );
}

/// An epoch outside this node's committee-read window costs no peer
/// its channel: `deliver`'s step (5) drops the answer for a node-local reason and
/// counts it as self-inflicted, and the run proves that with per-reason counters
/// rather than with the absence of a symptom.
#[test]
fn an_epoch_outside_this_nodes_read_window_costs_no_peer_its_channel() {
    use metrics_util::debugging::DebuggingRecorder;
    use std::collections::BTreeSet;
    const VICTIM: usize = HELD_LAG_VICTIM;
    const N: usize = 5;
    /// Every `reason` under which `deliver` step (5) may drop an answer without
    /// punishing the peer, in the source's order; printed only, so unreached arms
    /// show as zeroes.
    const SELF_INFLICTED: [&str; 4] = [
        "no_geometry",
        "out_of_window",
        "not_readable",
        "read_failed",
    ];
    /// The subset this fixture actually executes, asserted as an exact set against
    /// what the recorder saw.
    const COVERED: [&str; 1] = ["out_of_window"];
    /// The `block!` macro's WARN as the stand's capture renders it (`target:
    /// message`).
    const BLOCK_WARN: &str = "commonware_resolver::p2p::engine: invalid data received";

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let out = metrics::with_local_recorder(&recorder, || {
        run_held_lag_over_donor_archive(Some(snapshotter.clone()))
    });

    // Premise: the lag is held, so the epochs the victim is fed are outside its read
    // window.
    assert_the_lag_is_held(&out);

    // The victim's own inlet counted `scheme(E) == None` past its window, per node
    // rather than process-wide.
    let f = inlet(&out, VICTIM);
    assert!(
        f.defers > 0,
        "the inlet never deferred, so nothing in this run reached \"the scheme \
         cannot be built\" and the zeroes below would mean \"nothing happened\": {f:?}"
    );
    assert_eq!(
        f.rotations, 0,
        "an unreadable committee — this node's OWN lag — cost the donor a rotation: {f:?}"
    );

    // The windowless witness of the resolver's block line, when capture is live.
    let block_warns = out.logs_containing(BLOCK_WARN);

    // No peer was excluded; this is unconditional and does not wait on the premises
    // above.
    assert_eq!(
        out.blocked.len(),
        N,
        "the stand reported blocker facts for {} nodes, not {N}",
        out.blocked.len()
    );
    let blocks: Vec<(usize, &[(&'static str, fluentbase_bls::PeerPubkey)])> = out
        .blocked
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.calls.is_empty())
        .map(|(i, b)| (i, b.calls.as_slice()))
        .collect();
    assert!(
        blocks.is_empty(),
        "a node excluded a peer while its inlet deferred {} certificates for epochs \
         it cannot read — R-129's first half is LIVE in this wiring: {blocks:?} \
         (block! WARN lines captured: {})",
        f.defers,
        block_warns.len()
    );

    // Every peer here is honest, so the observed set of rejection labels must be
    // empty; the printed table names the four self-inflicted arms for the reader.
    let observed = |family: &str| -> BTreeSet<String> {
        out.metrics_before_collect
            .iter()
            .filter(|(name, _, value)| name == family && *value > 0)
            .flat_map(|(_, labels, _)| labels.iter())
            .filter(|(k, _)| k == "reason")
            .map(|(_, v)| v.clone())
            .collect()
    };
    let rejected_table: Vec<(&str, u64)> = SELF_INFLICTED
        .iter()
        .map(|reason| {
            (
                *reason,
                super::stand::counter_of(
                    &out.metrics_before_collect,
                    "dpos_frontier_rejected_total",
                    Some(("reason", reason)),
                ),
            )
        })
        .collect();
    let rejected = observed("dpos_frontier_rejected_total");
    assert!(
        rejected.is_empty(),
        "the frontier plane punished a peer in a run where every peer is honest — \
         the offending labels are `rejected`: {rejected:?}. The table beside it is \
         ONLY the four self-inflicted arms under the same family and reads \
         {rejected_table:?}; a punishing reason is not in it by construction"
    );

    // The stand's own per-node count of `deliver ⇒ false`, independent of the spy
    // and the reason label.
    let rejected_by_node: Vec<(usize, u64)> = out
        .upstream
        .iter()
        .enumerate()
        .map(|(i, u)| (i, u.deliveries_rejected))
        .collect();
    assert!(
        rejected_by_node.iter().all(|(_, n)| *n == 0),
        "a node's frontier plane answered `deliver ⇒ false` in a run where every \
         peer is honest — (node, deliveries_rejected): {rejected_by_node:?}"
    );

    // The resolver-internal exclusion set, which no `Blocker` choice can disarm,
    // asserted by family name per node so a missing engine cannot hide.
    let gauges = out.peers_blocked();
    for i in 0..N {
        for class in [
            "resolver_resolver",
            "frontier_resolver",
            "beacon_log_resolver",
        ] {
            let family = format!("node{i}_{class}_peers_blocked");
            assert!(
                gauges.iter().any(|(_, k, _)| *k == family),
                "no `{family}` in the exposition — that resolver's exclusion set is \
                 unobserved and its zero would be vacuous; families seen: {:?}",
                gauges
                    .iter()
                    .map(|(_, k, _)| k.as_str())
                    .collect::<Vec<_>>()
            );
        }
    }
    assert!(
        gauges
            .iter()
            .any(|(_, k, _)| k.contains("dkg_simplex_resolver")),
        "no DKG agreement resolver family in the exposition — the one class the \
         spy is never handed to is unobserved; families seen: {:?}",
        gauges
            .iter()
            .map(|(_, k, _)| k.as_str())
            .collect::<Vec<_>>()
    );
    let excluded: Vec<&(usize, String, f64)> = gauges.iter().filter(|(.., v)| *v != 0.0).collect();
    assert!(
        excluded.is_empty(),
        "a resolver had excluded a peer from its own fetches as of its last loop \
         iteration — this is the half a `NoopBlocker` does not cover: {excluded:?}"
    );
    if out.log_capture_live {
        assert!(
            block_warns.is_empty(),
            "the resolver's own `block!` line fired — the windowless witness of an \
             exclusion, on any engine: {block_warns:?}"
        );
    } else {
        eprintln!("(5.2В/R-129) log capture NOT live: the `block!` WARN witness is absent");
    }

    // Non-vacuity: every node handed the spy to both blocker slots.
    let mut wanted = vec![BLOCKER_SITE_CONSENSUS, BLOCKER_SITE_FRONTIER];
    wanted.sort_unstable();
    for (i, b) in out.blocked.iter().enumerate() {
        let mut got = b.sites.clone();
        got.sort_unstable();
        assert_eq!(
            got, wanted,
            "node {i} handed the blocker spy to {got:?} instead of both slots — the \
             assertions above are vacuous for that node"
        );
    }

    // The observed label set is the assertion; the printed table is for the reader.
    let drops_table: Vec<(&str, u64)> = SELF_INFLICTED
        .iter()
        .map(|reason| {
            (
                *reason,
                super::stand::counter_of(
                    &out.metrics_before_collect,
                    "dpos_frontier_dropped_total",
                    Some(("reason", reason)),
                ),
            )
        })
        .collect();
    let fired = observed("dpos_frontier_dropped_total");
    let covered: BTreeSet<String> = COVERED.iter().map(|r| r.to_string()).collect();
    assert_eq!(
        fired, covered,
        "the set of step-(5) reasons this fixture produces has changed (known arms: \
         {drops_table:?}). The property above is pinned ONLY for the reasons that \
         actually fire here, and the test's name says which — restate the coverage \
         (and the name) or extend the fixture; do not let the new arm ride along \
         unasserted"
    );

    // The bound is weak and not an invariant: a timed-out request can be re-sent, so a
    // later dropped answer can add a drop with no new fetch call.
    let dropped = super::stand::counter_of(
        &out.metrics_before_collect,
        "dpos_frontier_dropped_total",
        None,
    );
    let plane_calls: u64 = out
        .upstream
        .iter()
        .map(|u| u.latest_calls + u.finalized_calls)
        .sum();
    assert!(
        dropped <= plane_calls,
        "the plane dropped {dropped} answers over {plane_calls} fetches this process \
         issued"
    );

    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[3, VICTIM]);
    only_these_ran_inlets(&out, &[VICTIM]);
    eprintln!(
        "(5.2В/R-129) heights={:?} blocked={:?} sites={:?} peers_blocked_families={} \
         block_warns={} capture_live={} defers={} ingests={} drops={drops_table:?} \
         fired={fired:?} rejected={rejected_table:?} plane_calls={plane_calls} \
         upstream_victim={:?} virtual={:?}",
        out.heights,
        out.blocked
            .iter()
            .map(|b| b.calls.len())
            .collect::<Vec<_>>(),
        out.blocked[VICTIM].sites,
        gauges.len(),
        block_warns.len(),
        out.log_capture_live,
        f.defers,
        f.ingests,
        out.upstream[VICTIM],
        out.virtual_elapsed,
    );
}

/// A committee member catching up through its inlet inside its first
/// epoch's deal window deals that epoch before the seal deadline, on the one
/// beacon clock left — the marshal's tip.
#[test]
fn a_catching_up_validator_deals_its_first_epoch_on_the_live_frontier_without_a_tee() {
    use crate::beacon::testing::decode_artifact;
    use commonware_cryptography::Signer as _;
    use commonware_utils::ordered::Set;
    const LAGGARD: usize = 3;
    const N: usize = 4;
    /// Before the deal window, and before anything the epoch-2 ceremony reads.
    const CUT_AT: u64 = 8;
    /// The epoch-2 deal window on the actor's clock: opens on entering epoch 1,
    /// seals `DKG_MARGIN_BLOCKS` before `epoch_start(2)`.
    const DEAL_OPENS: u64 = EPOCH_LEN;
    const SEAL_AT: u64 = EPOCH_2_START - crate::beacon::testing::DKG_MARGIN_BLOCKS;
    /// The majority's executed tier-F at which the links come back — inside the
    /// window with room to catch up and deal.
    const HEAL_ABOVE: u64 = DEAL_OPENS + 4;
    /// Past `epoch_start(2)`, so the artifact, the share and a σ-carrying epoch are
    /// all facts of the run.
    const TARGET: u64 = EPOCH_2_START + EPOCH_LEN;
    let mut cfg = StandConfig::live(N, 1);
    cfg.epoch_len = EPOCH_LEN;
    // Explicit rather than inherited: the laggard has to be in `committee[2]`.
    cfg.committees = Committees::All;
    cfg.cert_inlet = Some(CertInletCfg {
        nodes: vec![LAGGARD],
        source: CertInletSource::NextAboveTier,
    });
    let seed = cfg.seed;
    let mut stand = Stand::new(cfg);
    stand
        .partition(&[0, 1, 2], &[LAGGARD])
        .after_height(CUT_AT)
        .heal_above(HEAL_ABOVE);
    let out = stand.run_until(reached(TARGET), Duration::from_secs(400));
    assert!(
        !out.timed_out,
        "heights {:?} halted {:?} errors {:?}",
        out.heights,
        out.halted,
        out.errors()
    );

    let part = &out.partitions[0];
    assert!(
        !part.heights_at_cut.is_empty(),
        "the partition never fired: {part:?}"
    );
    assert!(
        !part.heights_at_heal.is_empty(),
        "the partition never healed: {part:?}"
    );
    let lag_tip = part.heights_at_heal[LAGGARD];
    let majority_tip = [0, 1, 2]
        .iter()
        .map(|&i| part.heights_at_heal[i])
        .min()
        .expect("three nodes");
    assert!(
        lag_tip < DEAL_OPENS,
        "the laggard stood at {lag_tip} at heal, inside the deal window that opens at \
         {DEAL_OPENS}: its own chain could have opened the deal and the run says nothing \
         about catching up: {part:?}"
    );
    assert!(
        (DEAL_OPENS..SEAL_AT).contains(&majority_tip),
        "the majority stood at {majority_tip} at heal, outside the deal window \
         [{DEAL_OPENS}, {SEAL_AT}): {part:?}"
    );

    let (peers, _) = super::stand::keys(seed, N);
    let roster: Set<fluentbase_bls::PeerPubkey> =
        Set::from_iter_dedup(peers.iter().map(|k| k.public_key()));
    let seat = roster
        .position(&peers[LAGGARD].public_key())
        .expect("the laggard sits in committee[2]") as u8;
    let artifact = out.artifacts[LAGGARD]
        .get(&DETERMINISTIC_BOOTSTRAP_EPOCH)
        .unwrap_or_else(|| {
            panic!(
                "node {LAGGARD} holds no epoch-{DETERMINISTIC_BOOTSTRAP_EPOCH} artifact: {:?}",
                out.artifacts[LAGGARD].keys().collect::<Vec<_>>()
            )
        });
    let (proposal, _) = decode_artifact(artifact).expect("the served artifact decodes");
    let pinned_seats: Vec<u8> = proposal.logs.iter().map(|(idx, _)| *idx).collect();
    assert!(
        pinned_seats.contains(&seat),
        "the agreed epoch-{DETERMINISTIC_BOOTSTRAP_EPOCH} set pins seats {pinned_seats:?} and \
         not the laggard's ({seat}): it did not deal before the seal — heal=[lag {lag_tip} \
         vs majority {majority_tip}], heights={:?}",
        out.heights
    );
    // The other three dealt too, so the pinned set is the whole committee.
    assert_eq!(
        pinned_seats.len(),
        N,
        "the pinned set is not the whole committee: {pinned_seats:?}"
    );

    assert_eq!(
        out.metric(LAGGARD, "dkg_ceremony_ok_total"),
        Some(1.0),
        "the laggard's ceremony did not finalize into a share"
    );
    assert_eq!(
        out.metric(LAGGARD, "epoch_engine_demoted_no_polynomial_total"),
        Some(0.0),
        "the laggard was demoted for want of a share"
    );

    let (ordering, dkg_clock) = clock_pair(&out, LAGGARD);
    assert!(
        dkg_clock <= ordering,
        "the laggard's DkgActor clock ({dkg_clock}) is ABOVE its marshal's tip ({ordering}): a \
         feeder other than the tip is back"
    );
    assert_eq!(
        dkg_clock, ordering,
        "at rest the laggard's DkgActor clock is its marshal's tip, and it is not"
    );
    assert!(
        dkg_clock >= EPOCH_2_START,
        "the laggard's clock ({dkg_clock}) never reached epoch 2"
    );

    let f = inlet(&out, LAGGARD);
    let in_window: Vec<u64> = f
        .delivered
        .iter()
        .copied()
        .filter(|h| *h > lag_tip && *h <= majority_tip)
        .collect();
    assert!(
        !in_window.is_empty(),
        "the inlet handed the marshal nothing in the catch-up window ({}..={majority_tip}): {f:?}",
        lag_tip + 1
    );
    assert_eq!(
        f.rotations, 0,
        "catching up cost the honest upstream a rotation: {f:?}"
    );
    assert_eq!(f.defers, 0, "a by-height walk deferred: {f:?}");
    assert_eq!(
        f.carry_forward_fails, 0,
        "a certificate failed BLS verify under a resolvable key: {f:?}"
    );
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[]);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    only_these_ran_inlets(&out, &[LAGGARD]);
    eprintln!(
        "(5.4-А/catch-up dealer) heights={:?} heal=[lag {lag_tip} vs majority {majority_tip}] \
         seat={seat} pinned={pinned_seats:?} in_window={}..={} ({} certs) ingests={} \
         ordering={ordering} dkg_clock={dkg_clock} virtual={:?}",
        out.heights,
        in_window.first().copied().unwrap_or(0),
        in_window.last().copied().unwrap_or(0),
        in_window.len(),
        f.ingests,
        out.virtual_elapsed
    );
}
