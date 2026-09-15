//! Э5 5.0а — the stand's `CertInlet`: the second producer into a node's marshal.
//!
//! What these tests can see that no other stand test can: the inlet's own verify
//! gate (`CertInlet::ingest` → `Committee::scheme(E)` → the certificate verify
//! under the epoch's SEED ORACLE), its non-fault deferral, its data-fault
//! rotation, and — since 5.4-А — that the beacon clock of a node fed by an inlet
//! IS its marshal's tip. The frontier plane's own `deliver` cannot
//! stand in for any of it: it builds its verifier WITHOUT an oracle
//! (`plane_upstream.rs::verifier_for` → `build_verifier(.., None)`), so a
//! certificate whose σ slot has been swapped under an intact multisig passes it
//! and reaches the caller — the inlet is where the σ is first judged, and before
//! 5.0а the stand had no inlet at all (`git grep CertInlet testbed/` was empty).
//!
//! Some tests are fed by the node's own frontier PLANE and some are not, and
//! the split is load-bearing rather than incidental: the plane's `deliver`
//! refuses a certificate whose epoch this node cannot read, so the inlet's own
//! deferral is reachable ONLY through `CertInletSource::PeerArchive` — another
//! node's archive, read directly.
//!
//! The list of VERIFIED heights every test reads (`CertInletFacts::delivered`)
//! is recorded at the marshal seam — `MarshalSink::verify_block`, the first
//! marshal call on the clean path of `ingest` — not through a beacon-clock tee:
//! 5.4-А removed the tee (`LiveFrontierTee`), and with it the `TeeWiring` split
//! (`Observed` = a stand channel drained after `ingest`, `Production` = the
//! node's real channel) that used to decide whether a run could see the list or
//! the ORDER of the tick against the marshal. There is no tick any more: the
//! beacon actor's clock is the marshal's tip itself, so "clock = tip" is a gauge
//! comparison, asserted where it matters below.
//!
//! The NEGATIVE CONTROL of the whole file is `StandConfig::cert_inlet = None`:
//! every other test in the crate runs it, no node spawns an inlet, and none of
//! their numbers move. Every test here also states the positive half —
//! `only_these_ran_inlets` — so "the inlet ran where it was asked and nowhere
//! else" is an assertion in each run rather than a property of the green suite.

use super::stand::{
    CertInletCfg, CertInletFacts, CertInletSource, Committees, Outcome, Progress, Stand,
    StandConfig, BLOCKER_SITE_CONSENSUS, BLOCKER_SITE_FRONTIER,
};
use crate::beacon::testing::DETERMINISTIC_BOOTSTRAP_EPOCH;
use metrics_util::debugging::Snapshotter;
use std::{sync::Arc, time::Duration};

/// The epoch length every test here SETS on its config rather than inheriting:
/// the stand's own default is the same 32 (`StandConfig::honest`), but a default
/// is not a binding, and every height arithmetic below (`EPOCH_2_START`, the
/// window bound, the targets) is computed from this constant.
const EPOCH_LEN: u64 = 32;
/// The first height of the bootstrap epoch — epochs below it run seedless and
/// `DETERMINISTIC_BOOTSTRAP_EPOCH` is the first one whose certificates carry a σ
/// at all, which is why `byzantine_roles::FORGE_WINDOW` starts here.
const EPOCH_2_START: u64 = DETERMINISTIC_BOOTSTRAP_EPOCH * EPOCH_LEN;

fn reached(h: u64) -> impl Fn(&Progress) -> bool + Send + 'static {
    move |p| p.min_height() >= h
}

/// The file's NEGATIVE CONTROL as an assertion instead of a sentence: EXACTLY
/// the nodes `expected` names ran an inlet (ascending), and every other node's
/// slot is `None`. The second half is what "no test written before 5.0а changes
/// behaviour" rests on, and it costs nothing to state per run.
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

/// The facts of node `i`'s inlet, or a panic naming the whole vector — an
/// assertion about an inlet that was never spawned is a fixture error, not a
/// finding.
fn inlet(out: &Outcome, i: usize) -> &CertInletFacts {
    out.cert_inlet[i]
        .as_ref()
        .unwrap_or_else(|| panic!("node {i} ran no cert-inlet: {:?}", out.cert_inlet))
}

/// Node `i`'s two clock gauges at the end of the run, `(ordering, dkg)`: the
/// marshal's tip as `FluentApp::report` gauged it and the `DkgActor`'s clock as
/// its clamp gauged it. Both `expect`: a node that publishes neither ran no
/// beacon actor, which is a fixture error for every test that reads them.
fn clock_pair(out: &Outcome, i: usize) -> (u64, u64) {
    let ordering = out
        .metric(i, "dpos_ordering_finalized_height")
        .expect("FluentApp gauges the ordering tip on a node with a registered PlaneClock");
    let dkg = out
        .metric(i, "dpos_dkg_clock_height")
        .expect("the DkgActor gauges its clock on a Beacon::Live node");
    (ordering as u64, dkg as u64)
}

/// `epoch >= 2 ⇒ committee is {0,1,2}` — nodes 3 and 4 leave at the second
/// boundary. The same schedule the R-008 test uses, so a node without `PK_2`
/// exists at all.
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
/// producer into the marshal its own BFT engine already drives, and it changes
/// nothing: the node stays in lockstep, nothing halts, no ERROR line appears.
/// What the run BUYS is the clean-ingest bookkeeping, which no other stand
/// configuration can show.
///
/// Three things are pinned here and nowhere else in the crate:
///
/// 1. **Every attempt was a clean ingest.** `ingests` counts certificates handed
///    to `CertInlet::ingest`; `delivered` is recorded at `verify_block`, the
///    first marshal call on the clean path only (`cert_inlet.rs`, after
///    `observe_certificate` and after the three fault arms have returned), so
///    `delivered.len() == ingests` says the verify gate passed every time. With
///    `rotations == 0` and `defers == 0` beside it, all three fault arms and the
///    deferral arm are witnessed UNTAKEN on an honest run — which is what makes
///    their being TAKEN, in the two tests below, mean something.
/// 2. **The walk is monotone and contiguous from 1 — and that is a pin on a
///    RATIO OF SPEEDS, not on a mechanism.** The by-height feeder re-reads
///    `tier-F + 1` on every iteration, so nothing in the code forbids it asking
///    the same height twice: that is exactly what happens when the executor has
///    not advanced tier-F between two iterations, and it is what the `Frontier`
///    source below does thousands of times. The equality holds here because the
///    fetch of `tip + 1` effectively waits for the chain to grow (71 ingests over
///    71.9 s of virtual time at one block per second), so what it pins is the
///    ratio — the executor keeps up with the inlet — plus the absence of GAPS,
///    which would mean a certificate the inlet verified for a height it never
///    asked for. One seed (`live(4, 1)`), byte-identical over three runs; a
///    slower executor could legitimately repeat a height without the property
///    itself changing, so the failure message names the shape and not the count.
/// 3. **The beacon clock is the marshal's tip.** 5.4-А: the `DkgActor` takes its
///    clock off the ordering-tip watch `FluentApp::report(Update::Tip)`
///    publishes — the same value the ordering gauge shows — so on a node whose
///    marshal is fed by an inlet `dpos_dkg_clock_height` is never ABOVE
///    `dpos_ordering_finalized_height` (the tee used to fire one call before the
///    marshal saw the certificate, Д-5.4Б-3) and, at rest, equal to it; and it
///    is at least the highest height the inlet handed the marshal, which is
///    what says the inlet's certificates reached the clock at all. NOT
///    exclusive: the node's own engine reports the same tips on a healthy
///    node, so this is "the clock is the tip", not "the inlet moved it" — the
///    catch-up test below is where the inlet is the mover.
///
/// Falsifier: `delivered.len() != ingests` (a fault or a deferral happened on an
/// honest run); a non-contiguous or non-monotone walk; a DKG clock above the
/// ordering tip (a second, earlier feeder is back), below the highest delivered
/// height, or unequal to the tip at rest; any rotation or deferral; the stand
/// losing lockstep, halting, or printing an ERROR — a second producer into the
/// marshal is not allowed to cost any of those.
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

    // The inlet really ran, and it really was fed — an inlet that never saw a
    // certificate satisfies every assertion below vacuously.
    let f = inlet(&out, 3);
    assert!(
        f.ingests >= TARGET / 2,
        "the inlet on node 3 was barely fed ({} ingests over {} heights): {f:?}",
        f.ingests,
        out.heights[3]
    );

    // (1) Every attempt passed the verify gate; no fault arm and no deferral.
    assert_eq!(
        f.delivered.len() as u64,
        f.ingests,
        "an honest run took a fault or a deferral arm: {f:?}"
    );
    assert_eq!(f.rotations, 0, "an honest upstream cost a rotation: {f:?}");
    assert_eq!(f.defers, 0, "an honest run deferred a certificate: {f:?}");

    // (2) The walk is the by-height walk, exactly: 1, 2, 3, … with no gap and no
    // repeat.
    let expected: Vec<u64> = (1..=f.ingests).collect();
    assert_eq!(
        f.delivered, expected,
        "the delivered list is not the contiguous walk the feeder asked for: {f:?}"
    );

    // (3) The beacon clock is the marshal's tip, and the inlet's certificates
    // are in it.
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

    // The second producer is harmless.
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

/// (5.0а, Ex-21 stand half — the KEYED arm) The half of R-008 that only exists
/// once `PK_E` has landed, and the half the stand could not observe before this
/// row: a σ-forged certificate served over a HEALTHY connection fails the
/// inlet's BLS verify, counts as a DATA fault, and after
/// `MAX_UPSTREAM_FAULTS = 3` consecutive ones the inlet ROTATES away from the
/// upstream.
///
/// **WHICH ARM THIS IS, after row 5.2 gave the σ verdict readers.** The verify is
/// still the gate here and the cause is still "BLS verify FAILED" — the mechanism
/// did not move and the numbers did not change. The reason is a wiring fact worth
/// stating, because the plan predicted otherwise: the scheme this inlet verifies
/// with is built by `committee::epoch_verifier`, which passes
/// `Beacon::oracle_for(epoch)` into `build_verifier`, so a certificate whose σ
/// slot was swapped is refused by `verify_certificate` itself and never reaches
/// `observe_certificate`. Row 5.2's synchronous `Refused` reader in
/// `CertInlet::ingest` covers the arm this fixture cannot produce — a verifier
/// built WITHOUT an oracle (the shape `plane_upstream::verifier_for` builds) — and
/// is pinned by the unit
/// `cert_inlet::tests::a_forged_seed_under_an_oracle_less_verifier_is_refused_at_the_ingress`.
/// The LATE verdict (σ admitted keyless, refused when the key lands ⇒ `DataFault`
/// ⇒ rotation) is the OTHER half of R-008 and needs a keyless victim: it is pinned
/// by `cert_inlet::tests::a_late_refusal_costs_the_upstream_a_rotation_once_the_key_lands`,
/// and its stand half is `tests::a_forged_seed_slot_is_admitted_with_no_key_and_refused_when_the_key_lands`,
/// whose nodes run no inlet.
///
/// **Why the victim is a committee member.** The forged window is the first seven
/// heights of epoch 2 (`byzantine_roles::FORGE_WINDOW = 64..=70`), and the
/// forgery is only detectable by a node that HOLDS `PK_2`. A node outside
/// `committee[2]` does not: it admits the certificates keyless (the arm the test
/// below pins) and, having no σ it can verify, cannot execute a single epoch-2
/// block — so its feeder never walks INTO the window at all. A member deals the
/// epoch's DKG, holds the key by height 64, and its own consensus plane walks its
/// tier-F through the window, which is what hands its inlet the forged
/// certificates one after another.
///
/// **Why the rotation count is the observable.** `CertInlet::consecutive_faults`
/// is private and `cert_inlet.rs` is not this row's to touch, so the streak is
/// observed through its ONE external effect: `record_data_fault` invokes
/// `RotateUpstream` exactly when the streak reaches 3, and then resets it. Six
/// forged heights in an unbroken run therefore mean TWO rotations and no more —
/// and that arithmetic is the assertion, not `rotations > 0`, because `> 0`
/// would also hold if every single fault rotated (a broken threshold) or if the
/// streak never reset.
///
/// **What "rotation" is on this plane, said out loud.** The trigger this test
/// counts calls `CertUpstream::rotate`, which on the frontier plane is
/// `PlaneUpstreamHandle::rotate` = `mailbox.cancel(FrontierKey::Latest)`
/// (`plane_upstream.rs:684-692`): it cancels an in-flight `Latest` request and
/// nothing else. For a BY-HEIGHT feeder that is a complete no-op — the victim
/// keeps pulling `Finalized{h}` from the same forger afterwards — and the
/// equality `rotations == faults / MAX_UPSTREAM_FAULTS` holds precisely BECAUSE
/// the rotation changes nothing: an unbroken streak of six stays unbroken. So
/// this test pins the inlet's THRESHOLD, not a production failover; a real
/// failover (a second upstream URL) exists only for the WS path, and whether the
/// plane's rotation defends a validator against a bad upstream at all is a
/// separate question this row only read and did not test (journal §5).
///
/// **The key was held AT VERIFY TIME, and that is a counter rather than an
/// end-of-run artifact.** `with_carry_forward_fail_metric` is wired here (the
/// production follower wires the same builder), and it increments on exactly one
/// line: a BLS verify failure taken while `ensure_key` HAD resolved the epoch's
/// key (`cert_inlet.rs:666-668`). So `carry_forward_fails == |forged taken|` says
/// the victim judged every forged certificate WITH `PK_2` in hand, at the moment
/// it judged it — which `artifacts[VICTIM]` at the end of the run cannot say.
///
/// **The tamper's own witness first.** `ByzFacts` is asserted before anything
/// else: `certs_forged > 0`, the planted σ read back DIFFERENT from the original,
/// the multisig half byte-identical, and every forged height inside the window.
/// Without those, a green run would rest on a forgery that never happened.
///
/// Falsifier: the wrapper forging nothing, forging a σ that reads back equal to
/// the original, or touching the multisig half; node 4 not holding the epoch-2
/// artifact (then the keyed arm was never reached and this is the keyless test);
/// a `carry_forward_fails` count that is not the number of forged certificates
/// taken (then the verify failures were NOT judged under a resolvable key, and
/// the "keyed" in this test's name is wrong); `rotations == 0` (the fault arm was
/// not taken, or the threshold never fired); a rotation count that is not
/// `forged_ingested / 3`; a clean ingest inside the window (then the forgery was
/// not served to this victim); the honest committee losing lockstep, halting, or
/// printing an ERROR line.
#[cfg(feature = "dpos-devnet-byzantine")]
#[test]
fn a_forged_seed_slot_costs_the_upstream_a_rotation_once_the_epoch_key_is_held() {
    // Both names belong to THIS test only — `Role` because the σ-forging role is
    // the byzantine feature's, and `FORGE_WINDOW` because it is that role's own
    // constant. Imported in the body rather than at the top of the file so a build
    // WITHOUT `dpos-devnet-byzantine` has no unused import (the idiom of the
    // neighbouring feature-gated tests: `testbed/tests.rs:3561`, `:3758`).
    use super::{byzantine_roles::FORGE_WINDOW, stand::Role};
    const FORGER: usize = 0;
    const VICTIM: usize = 4;
    let mut cfg = StandConfig::live(5, 1);
    cfg.epoch_len = EPOCH_LEN;
    // Node 4 sits in EVERY committee, so it deals the epoch-2 DKG and holds
    // `PK_2` before its tier-F reaches the forged window. This is also the
    // stand's default, and the assignment is deliberate all the same: the premise
    // of the keyed arm is then a property of THIS fixture rather than of whatever
    // `StandConfig::honest` happens to default to.
    cfg.committees = Committees::All;
    // The victim's by-height pulls go to the forger and to nobody else, so
    // "which peer answered" is a fact of the fixture and not of the resolver's
    // shuffle. ASYMMETRIC (`upstream_source_only_for`, not
    // `upstream_only_link`): the forger keeps every link it had, because it is
    // also an honest committee member whose consensus plane must stay whole.
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

    // (1) THE TAMPER'S OWN WITNESS.
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

    // (2) THE PREMISE OF THE KEYED ARM: the victim holds `PK_2`.
    assert!(
        out.artifacts[VICTIM].contains_key(&2),
        "node {VICTIM} holds no epoch-2 artifact, so nothing could check a σ: {:?}",
        out.artifacts[VICTIM].keys().collect::<Vec<_>>()
    );

    // (3) THE PREMISE OF THE STREAK: the victim's feeder really walked the window,
    // and every height it handed the marshal there is a height the forger did
    // NOT forge. The window heights are partitioned at the marshal seam:
    // delivered ⇒ verified ⇒ not forged.
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

    // (4) THE PROPERTY, and it is arithmetic on the PRIVATE `consecutive_faults`
    // rather than a `> 0`: with `defers == 0` every ingest is either a clean one
    // (it reached the marshal) or a data fault, so `faults = ingests −
    // delivered.len()` is exact — and exact by construction, not by a drop count:
    // `delivered` is a direct push at the marshal seam (`RecordingSink`), where
    // the tee this stood on was a lossy `try_send` whose zero drop count had to
    // be asserted as the arithmetic's precondition; the faults all fall in one
    // unbroken run (the feeder walks the window
    // strictly upward and this victim's only by-height source is the forger), so
    // the streak resets ONLY at the threshold and the rotation count must be
    // `faults / MAX_UPSTREAM_FAULTS` exactly. `> 0` would also hold if every
    // single fault rotated (a broken threshold) or if the streak never reset
    // (one rotation for six faults).
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

    // (5) THE DIRECT WITNESS OF THE KEY: every one of those verify failures was
    // judged WITH `PK_2` resolved, counted on the line that can only increment
    // in that regime.
    assert_eq!(
        f.carry_forward_fails,
        ingested_forged.len() as u64,
        "the victim's verify failures judged under a RESOLVABLE epoch key ({}) are \
         not the forged certificates it took ({:?}) — then the keyed arm is not what \
         this run exercised: {f:?}",
        f.carry_forward_fails,
        ingested_forged
    );

    // The forged upstream is a frontier-plane liar, not a consensus-plane one: the
    // committee is whole and the chain is one chain — including the victim's, whose
    // own engine never left lockstep while its inlet was being lied to.
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

/// (5.0а, Ex-21 stand half — the KEYLESS arm; and what the R-129 fixture RAN
/// INTO) A NON-MEMBER whose EL is held behind the committee, with a live inlet.
///
/// **It is not the plan row's "validator with a lagging EL".** This victim is
/// dropped from `committee[2]` and never returns: it deals no DKG, holds no
/// `PK_2`, signs nothing, and its consensus plane is severed — a node that has
/// LEFT, not one that is catching up. The plan row's role (a MEMBER whose EL fell
/// behind while its inlet kept feeding it) is the partition test below; the two
/// pin different things and neither stands in for the other. What THIS
/// configuration is for is the pair of arms named next: a keyless admission,
/// which only a node without the epoch key can show, and the plane's own gate in
/// front of the inlet, which only a node whose read window is BELOW the
/// committee's epoch can show.
///
/// The victim is dropped from `committee[2]` and its consensus-plane links are
/// cut inside epoch 1 while its `FRONTIER_CHANNEL` links are kept
/// ([`super::stand::CutPlanes::ConsensusOnly`]) — the shape `(4c)`
/// (`a_node_outside_the_tracked_peer_set_keeps_following_through_the_upstream_plane`)
/// already pins as "a node whose only path to the chain is the upstream plane".
///
/// WHY THE CUT AND NOT THE TRACKED SET (5.1). Non-membership no longer holds a
/// node keyless: since П-3 `PK_E` is an artifact any node may ASK a member for
/// over `BEACON_RESOLVER_CHANNEL` (R-121/R-122), and the tracked set is
/// `committee[E-1] ∪ committee[E] ∪ committee[E+1]`, so under
/// `PeerSet::Committee { upstream_link: true }` the victim keeps its consensus
/// links for the whole of epoch 2 — long enough to fetch `PK_2`, after which the
/// committee never changes again and that ONE key carries it to the end of the run
/// (measured: `heights=[160, 160, 160, 159, 159]`). The cut is taken in epoch 1
/// instead, before the epoch-2 mint, and never heals: the victim holds every key
/// up to `PK_1`, so it executes to `last(1)` and stands at the epoch-1 boundary
/// for the whole run, with no path by which to ask for `PK_2`.
/// `re_jump_threshold` is left at the stand default (`u64::MAX`), so the re-jump
/// that would otherwise carry it forward never arms.
///
/// Its inlet is fed the upstream's LIVE FRONTIER
/// (`CertInletSource::Frontier` — production's own inlet input), so the
/// certificates it is handed climb with the committee while its own anchor does
/// not move.
///
/// **ARM 1 — the keyless admission.** A certificate of epoch 2 IS readable at an
/// epoch-1 anchor (the committee module answers every epoch up to
/// `epoch(anchor) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS`), so the multisig verifies,
/// the σ slot is NOT checked because the oracle has no key, and the inlet counts
/// the admission itself (`dpos_cert_vote_only_admissions_total`). That counter is
/// the "before" of Э5 5.2: today the late verdict is invisible, and 5.2 turns it
/// into a `DataFault` on `faults()`. The rotation count beside it is 0, which is
/// the other half of the same statement — a MISSING KEY is not a data fault.
///
/// **ARM 2 — why the inlet's own deferral arm never fires, which is a finding and
/// not a gap in the fixture.** The inlet defers when `Committee::scheme(E)`
/// answers `None`, i.e. when the certificate's epoch is outside this node's
/// committee-read window. On the stand's frontier plane that certificate never
/// ARRIVES: `FrontierHandler::deliver` classifies exactly the same three
/// refusals one layer up (`plane_upstream.rs`, step (4) —
/// `OutOfWindow`/`NotReadable`/`Read`) and step (5) DROPS the answer, counting
/// `dpos_frontier_dropped_total{reason}` and resolving the waiting `fetch_one`
/// to `None`. So the highest certificate this inlet can ever see is the top of
/// its own read window — and that is what the run asserts: the delivered list
/// stops at `last(epoch(anchor) + 2)` while the committee is above it, with the
/// plane's drop counter as the witness. Production has no such gate in front of its
/// inlet (the WS stream hands over whatever the upstream sends,
/// `node/src/cert_inlet.rs`), so the deferral arm is a production regime the
/// stand cannot reach through THIS plane — see the journal §0(9)/§5.
///
/// Falsifier: the victim NOT standing below the epoch-2 boundary, or re-jumping,
/// or holding `PK_2` (then the lag is not held and neither arm is what it says);
/// no keyless admission (then the σ was checked after all); any rotation (a
/// missing key or this node's own lag was counted as a data fault); the delivered
/// list reaching ABOVE the window bound (then `deliver` let an unauthenticatable
/// certificate through and the whole trust argument of the plane is wrong), or
/// stopping BELOW it (then something other than the window stopped the feeder and
/// the bound proves nothing); no plane drop at all; the committee halting.
#[test]
fn a_keyless_admission_is_all_the_plane_lets_an_outrun_inlet_see() {
    use fluentbase_types::staking_protocol::{epoch_at_block, MAX_COMMITTEE_LOOKAHEAD_EPOCHS};
    use metrics_util::debugging::DebuggingRecorder;
    const VICTIM: usize = 4;
    /// Well above `last(epoch(anchor) + 2) = last(3) = 127`, so the committee
    /// really does outrun the victim's read window and the bound below is a bound
    /// on something.
    const TARGET: u64 = 5 * EPOCH_LEN;
    /// Inside epoch 1 and BELOW the epoch-2 mint: the victim holds `PK_1` and
    /// nothing above it.
    const CUT_AT: u64 = EPOCH_LEN + 4;
    /// Longer than the run's virtual deadline — the cut never heals, which is
    /// what "the lag is HELD" means.
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

    // PREMISE: the lag is real and HELD. The cut is the fixture, so it is a
    // premise of its own — without it the victim fetches `PK_2` and follows.
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

    // ARM 1 — the keyless admission, counted by the inlet itself on the line that
    // admits a certificate of a beacon-active epoch whose key it could not get.
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

    // ARM 2 — the bound is the committee-read WINDOW, and the plane is what
    // enforces it.
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
    // PROCESS-WIDE, not per node: `counter_of` sums a `metrics::counter!` family
    // over the whole recorder and `dpos_frontier_dropped_total` carries only a
    // `reason` label (`stand.rs::counter_of`'s own doc, and
    // `plane_upstream.rs`'s increment site), so node 3 — dropped from
    // `committee[2]` by the same schedule, with its upstream links kept — is in
    // this sum too. The conclusion does not rest on the attribution: the BOUND is
    // pinned by the victim's OWN `delivered_top` straddling its own window edge in the
    // two assertions above, and this counter only has to witness that the plane
    // refuses SOMETHING in this run rather than delivering everything. The
    // per-inlet, per-node counterpart of the same gate is the archive test below,
    // where the refusal is counted by the victim's own inlet (`defers`).
    assert!(
        dropped > 0,
        "the plane dropped nothing, so the bound above has no mechanism behind it"
    );
    assert_eq!(
        f.defers, 0,
        "the inlet deferred after all — then the plane's step-(5) drop is not total \
         and the finding in the doc comment is wrong: {f:?}"
    );

    // The committee itself is unaffected by the two parked outsiders.
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

/// (5.0а, the plan row's OWN role — "a validator with a lagging EL and a live
/// inlet") A committee MEMBER that a partition left behind, fed forward by its
/// inlet while it catches up, and still a member at the end of the run.
///
/// **Why this and not the keyless test above.** That node is an EX-member: it
/// leaves `committee[2]` for good, deals nothing, holds no epoch key and never
/// comes back — every property it shows is a property of an outsider. §0.7(а) of
/// the Э5 project and row 5.4 need the opposite node: one that IS in the
/// committee (so it deals its epoch's DKG and its beacon clock matters for its
/// share), whose EL is nevertheless BEHIND (so the marshal's tip is ahead of
/// its own executed tip), with a live inlet. The stand can build exactly that with
/// `Stand::partition`, and with no new `Role` and no new `FakeChain` switch: the
/// cut is physical (it removes the links of BOTH planes, `stand.rs`'s driver
/// loop), so the isolated member stops finalizing while the other three — a
/// quorum of four — carry the chain on; when the cut heals it is behind by the
/// blocks the split cost, and its inlet is one of the two producers that walk it
/// forward.
///
/// **What the lag is NOT.** During the cut the member is fed nothing at all —
/// that is what a physical cut means, its inlet's own pulls included. The window
/// this test is about is the one AFTER the heal, while the member is still behind:
/// the assertion below counts delivered heights strictly between the member's
/// tier-F at heal and the majority's, i.e. certificates it was MISSING at the
/// moment its links came back.
///
/// **What it does not claim.** Not that the inlet is what caught the node up: its
/// own marshal repair is racing the inlet over the same range, and nothing here
/// separates the two. What is claimed is that the inlet stayed live and
/// productive across the whole episode, that it cost the node nothing (no
/// rotation, no deferral, no verify failure under a resolvable key), and that
/// the node was and remained a member. The heal here lands BEFORE the epoch-2
/// deal window opens, so the DKG is not what this run is about — the test below
/// (`a_catching_up_validator_deals_its_first_epoch_on_the_live_frontier`) heals
/// inside the window and is where the beacon clock is under load.
///
/// Falsifier: the partition never firing (no observation to read); no lag at heal
/// (then the cut did not isolate anything); the member not holding its own
/// epoch's artifact at the end (then it is the keyless test's outsider, not a
/// member); an inlet that was never fed, or fed nothing inside the catch-up
/// window; any rotation, deferral, or carry-forward verify failure (an honest
/// donor and this node's own lag must cost none of the three); the node failing
/// to rejoin lockstep, or the committee halting or printing an ERROR.
#[test]
fn a_catching_up_committee_member_is_fed_by_its_inlet_while_its_el_is_behind() {
    const LAGGARD: usize = 3;
    /// Early enough that the cut cannot overlap the epoch-2 DKG (which opens at
    /// `epoch_start(1) = 32`), so the artifact assertion at the end is about
    /// membership and not about a ceremony this fixture happened to interrupt.
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

    // (1) THE CUT HAPPENED AND THE LAG WAS REAL. `heights_at_heal` is the driver's
    // per-node tier-F sample at the moment the links came back.
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

    // (2) IT IS A MEMBER, and the witness is the artifact of its own epoch: only a
    // node inside `committee[E]` deals E's DKG, and nothing in this tree fetches a
    // non-member's artifact (R-121/R-122 — the keyless test asserts the negation of
    // this line for its outsider).
    assert!(
        out.artifacts[LAGGARD].contains_key(&DETERMINISTIC_BOOTSTRAP_EPOCH),
        "node {LAGGARD} holds no epoch-{DETERMINISTIC_BOOTSTRAP_EPOCH} artifact, so it \
         is not the MEMBER this test is about: {:?}",
        out.artifacts[LAGGARD].keys().collect::<Vec<_>>()
    );

    // (3) THE INLET FED IT ACROSS THE CATCH-UP WINDOW: heights it had not executed
    // at heal, and that the majority already had.
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

    // (4) AND IT COST THE NODE NOTHING. An honest donor plus this node's own lag
    // must produce no rotation (a lag is not a data fault, `cert_inlet.rs`'s
    // deferral arm says so in as many words), no deferral (every height it walks is
    // `tier-F + 1`, whose epoch its own anchor can always read), and no verify
    // failure under a resolvable key.
    assert_eq!(
        f.rotations, 0,
        "catching up cost the honest upstream a rotation: {f:?}"
    );
    assert_eq!(f.defers, 0, "a by-height walk deferred: {f:?}");
    assert_eq!(
        f.carry_forward_fails, 0,
        "a certificate failed BLS verify under a resolvable key: {f:?}"
    );

    // (5) IT CAUGHT UP: the predicate above is `min_height >= TARGET` over ALL
    // nodes, and the hashes agree everywhere below it.
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

/// The HELD-LAG fixture over a donor's archive, shared by the two tests below
/// that differ only in what they OBSERVE on it (the inlet's deferral; the
/// blocker slots). One place to change the lag, one place to change the cut.
///
/// The victim. The keyless test above uses the same index for the same node.
const HELD_LAG_VICTIM: usize = 4;
/// The DONOR whose marshal archive the victim's inlet reads directly.
const HELD_LAG_DONOR: usize = 0;
/// The same target as the keyless test: the committee climbs well above the
/// victim's read window, so the walk has heights the victim cannot read.
const HELD_LAG_TARGET: u64 = 5 * EPOCH_LEN;
/// Inside epoch 1 and BELOW the epoch-2 mint — see the keyless test above for
/// why the lag is held by a cut and not by the tracked peer set.
const HELD_LAG_CUT_AT: u64 = EPOCH_LEN + 4;
/// Longer than the run's virtual deadline — the cut never heals.
const HELD_LAG_NEVER: u32 = 4096;

/// Run the held-lag fixture: five live nodes, the victim's inlet over the
/// donor's archive, the victim cut from the consensus plane inside epoch 1 for
/// good, the run ending when the committee reaches [`HELD_LAG_TARGET`].
/// `snapshotter` is ASSIGNED to [`StandConfig::metrics_snapshotter`]
/// unconditionally: `None` means "no snapshotter", not "keep whatever the
/// config had".
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

/// The fixture's PREMISE as assertions: the cut that holds the lag really
/// fired, the victim never executed into the bootstrap epoch, and it holds no
/// key for it — so the donor's later epochs really are outside its read window.
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

/// (5.0а, A-05 — the plan row's FIRST input, and the only one with no `deliver`
/// gate) A donor's marshal archive walked upward by height, which is how the
/// stand finally reaches the inlet's own NON-FAULT DEFERRAL.
///
/// **What was wrong before.** The keyless test above proves that a plane feeder
/// CANNOT reach it: `FrontierHandler::deliver` classifies the same three
/// committee refusals one layer up and its step (5) drops the answer, so the
/// highest certificate a plane feeder is ever handed is the top of this node's
/// own read window. The row's own first input — "`UpstreamFinalized` assembled
/// from ANOTHER node's archive" — has no such gate: `FrontierMarshal::pair_at`
/// on the donor's `MarshalMailbox` is two LOCAL archive reads, and whatever the
/// donor finalized is handed over whether or not this node can read the epoch.
/// That is production's regime too (the WS stream hands over what it sends), and
/// it is the fixture row 5.2 needs for R-129.
///
/// **The three things this run buys, and nothing else claims them.**
///
/// 1. **The deferral arm is REACHED.** `Committee::scheme(E)` answers `None` for
///    an epoch outside this node's window, the inlet counts the skip and leaves
///    `consecutive_faults` untouched — so `defers > 0` WITH `rotations == 0` is
///    the whole statement of `cert_inlet.rs`'s "this node's own lag is not a data
///    fault". It is also the POSITIVE CONTROL for the `defers` counter every
///    other test in this file asserts to be zero.
/// 2. **The gate moved from the plane INTO the inlet.** The clean (delivered)
///    heights still stop at the top of the read window, exactly as in the keyless test —
///    but now the certificates ABOVE it arrive and are refused HERE, by this
///    node's own inlet, and the refusal is counted per node rather than
///    process-wide.
/// 3. **The price of the plane feeder, measured.** This walk is the feeder's own
///    cursor: it climbs by one per pair the donor held, so it never re-ingests a
///    height. The keyless test's `Frontier` feeder re-asks whatever the plane will
///    answer and pays thousands of ingests (each a full BLS verify plus two
///    marshal messages) for the same ~127 distinct heights; the numbers of both
///    runs are in the journal §4.
///
/// The fixture is the keyless test's, unchanged except for the source — which is
/// the point: one knob apart, and the inlet's behaviour is a different regime.
///
/// Falsifier: `defers == 0` (then the archive walk is gated after all, or the
/// victim's window is not below the donor's epoch); any rotation (then the inlet
/// counted its own lag as a data fault); an ingest that took neither the clean nor
/// the deferral arm (then the fault arithmetic of the keyed test does not hold
/// here and one of the two is wrong); a delivered height above the window bound (then
/// the inlet verified a certificate whose committee it cannot read); a
/// non-monotone or repeating walk; the victim executing into epoch 2 or holding
/// its key (then it is not the node this fixture is about).
#[test]
fn a_donors_archive_hands_the_inlet_an_epoch_it_cannot_read_and_it_defers() {
    use fluentbase_types::staking_protocol::{epoch_at_block, MAX_COMMITTEE_LOOKAHEAD_EPOCHS};
    const VICTIM: usize = HELD_LAG_VICTIM;
    const DONOR: usize = HELD_LAG_DONOR;
    const TARGET: u64 = HELD_LAG_TARGET;
    let out = run_held_lag_over_donor_archive(None);

    // PREMISE: the same held lag as the keyless test — the victim's committee
    // anchor stays in epoch 1, which is what puts the donor's later epochs outside
    // its read window — and the cut that holds it really fired.
    assert_the_lag_is_held(&out);

    let f = inlet(&out, VICTIM);
    // (1) THE DEFERRAL ARM, REACHED — and it is not a fault.
    assert!(
        f.defers > 0,
        "the inlet never deferred: every certificate the donor's archive handed it \
         was inside its own read window, so this source is gated after all: {f:?}"
    );
    assert_eq!(
        f.rotations, 0,
        "an unreadable committee — this node's OWN lag — cost the donor a rotation: {f:?}"
    );

    // (2) EVERY ingest took one of exactly two arms: clean (delivered) or
    // deferred. No fault arm is reachable here (the donor is honest and the pair
    // comes from its own archive), and this equality is what says so.
    assert_eq!(
        f.ingests,
        f.delivered.len() as u64 + f.defers,
        "an ingest took neither the clean nor the deferral arm: {f:?}"
    );

    // (3) THE WALK: strictly up, no repeats. The clean prefix is contiguous from 1
    // and the top of it is the top of the read window — the same bound the keyless
    // test reads off the plane, now enforced by the inlet itself.
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

    // (4) THE PRICE. One ingest per height walked, bounded by what the donor could
    // hold: the plane feeder of the keyless test pays thousands for the same
    // distinct heights (journal §4).
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

/// (5.2 заход В — the FRONTIER half of R-129; the marshal half is watched here,
/// not exercised) An epoch OUTSIDE THIS NODE'S COMMITTEE-READ WINDOW costs no
/// peer its channel — and the run proves both halves of that sentence with
/// counters rather than with the absence of a symptom.
///
/// **Read the name: the claim is NARROWED on purpose.** `plane_upstream::deliver`
/// step (5) drops an answer it cannot authenticate for FOUR distinct reasons, and
/// all four are statements about THIS node rather than about the peer:
/// `no_geometry`, `out_of_window`, `not_readable`, `read_failed`
/// (`plane_upstream.rs:140-143`). A test can only pin the property for the arms
/// its fixture actually executes, and this fixture executes exactly ONE of them —
/// `out_of_window`. So that is what the name says and what the coverage
/// assertion below enforces: the set of `reason` labels OBSERVED under
/// `dpos_frontier_dropped_total` in this run, read off the recorder rather than
/// off a list in this file, must equal `COVERED` exactly. A sum over reasons
/// would quietly swallow the zeroes; a hard-coded list would be blind to a label
/// it does not name. If the observed set ever changes — an arm stops firing, or
/// a new one starts — this test goes red and whoever changed it decides what the
/// test now answers for.
///
/// **What R-129 says and which half is here.** The register entry is about the
/// MARSHAL resolver: "`deliver == false` excludes an honest peer forever". On
/// that verdict a resolver runs `commonware_p2p::block!(self.blocker, peer, ..)`
/// and `self.fetcher.block(peer)` back to back (CW
/// `resolver/src/p2p/engine.rs:437-438`). The first is disarmed in our wiring
/// by a [`fluentbase_p2p::NoopBlocker`] in every blocker slot — a decision taken
/// about the simplex BATCHER (`p2p/src/lib.rs`'s "Bug A" rationale) and
/// inherited by the other slots as a side effect. The second is NOT disarmable
/// by any `Blocker` choice: it appends to the fetcher's own `excluded` set
/// (`fetcher.rs:516`), which nothing ever removes from, so the peer is excluded
/// from that resolver's fetches for the life of the engine regardless. The
/// protection we have is therefore accidental AND partial, and this test pins
/// the one rule that is under load here — the frontier plane's.
///
/// **Which slot is EXERCISED, and which is only watched.** The victim is cut
/// from the consensus plane and the frontier plane stays up; step (5) runs
/// somewhere in the process — how many times, and on which node, is a run fact
/// the coverage assertion reads off a recorder that carries no node label. The
/// only per-node observable is the victim's inlet `defers`; the victim's own
/// `upstream[VICTIM].deliveries_decoded` (its `deliver ⇒ true` count, drops and
/// admissions together) is PRINTED, not asserted. The consensus slot
/// ([`BLOCKER_SITE_CONSENSUS`] — the marshal resolver's arms and the simplex
/// batcher) has the spy wired into it and nothing in this fixture pulls its
/// trigger, and that is not an oversight of the fixture: with no scheme for an
/// epoch the marshal refuses BEFORE decoding and answers `true` ("ignoring stale
/// delivery", CW `marshal/core/actor.rs:965-971`), because `EpochSchemeProvider`
/// does not override `Provider::all()` and the trait default is `None`
/// (`cryptography/src/certificate.rs:417-419`). The `verify_delivered ⇒ false`
/// arm that R-129 is about is unreachable from above this layer. Measured, not
/// assumed: blinding `EpochSchemeProvider::scoped` to `None` leaves this test
/// green and only stalls the lagging node (journal
/// `.dpos-study/history/E5-2-V.md` §0(4), mutation 2, beside the other `E5-*`
/// records). So this test does NOT close
/// R-129; it closes the frontier rule and documents the marshal one as
/// unreachable.
///
/// **The counters are PROCESS-WIDE.** `dpos_frontier_dropped_total` and
/// `_rejected_total` go to one recorder for all five nodes and carry no node
/// label (`stand::counter_of`'s own doc). Node 3 leaves the committee on the
/// same schedule and can contribute the same labels, so nothing below
/// attributes a drop to the victim. The per-node half of "this node cannot read
/// the epoch" is the victim's OWN inlet counter, `defers`, asserted first.
///
/// **Why none of this could simply be asserted on the stand as it stood.** Until
/// 5.2 the stand's own blocker slots took `NoopBlocker` too, so "no peer was
/// blocked" was true by construction for ANY code. Five observables carry the
/// run now, deliberately not one, and each is stated with what it proves:
///
/// * [`super::stand::BlockerSpy`] counts `Blocker::block(peer)` without
///   performing it — it names the PEER and the SLOT. Its `sites` half proves
///   that `BlockerSpy::at` ran for both labels on every node and, because `at`
///   is the only constructor of `SpyBlocker` and its fields are private, that
///   the value each slot was HANDED is the spy; it does not prove the builder
///   USED it — a `SpyBlocker` that is constructed and discarded looks the same.
///   For the FRONTIER slot that residual is closed by the per-node counter
///   below; for the consensus slot it stands (journal).
/// * `dpos_frontier_rejected_total{reason}` — the plane's own count of
///   `deliver ⇒ false`, PER REASON, as an ASSUMPTION of this assertion: it is
///   incremented only in `Self::reject`, so a `false` that bypasses `reject`
///   is invisible here. Every peer here is honest, so the OBSERVED label set
///   must be empty, whatever the labels are called.
/// * `upstream[i].deliveries_rejected` — the stand's own per-node count of the
///   FACT `deliver ⇒ false` on the frontier plane (`fakes.rs`,
///   `CountingHandler::deliver` counts the returned `bool`, wrapped
///   unconditionally in `stand.rs::frontier_plane`): blind to how the `false`
///   was produced and to what sits in the blocker slot, so it is the witness
///   that needs neither the spy nor the reason label.
/// * [`Outcome::peers_blocked`] — the resolver-internal exclusion, blind to the
///   `Blocker` choice and present on every resolver engine including the beacon
///   log's and the DKG agreement's, which the spy is never handed to. What its
///   zero proves is bounded: the gauge is written once per loop iteration, so it
///   says "nothing excluded as of the engine's last iteration" (that doc has the
///   line). The expected FAMILIES are asserted by name, not just "some family".
/// * the `block!` macro's own WARN line in [`Outcome::logs`] — the windowless
///   witness of the same resolver line, across every engine, when log capture is
///   live (stated in the output when it is not).
///
/// Falsifier: any `block(peer)` on any node; any `dpos_frontier_rejected_total`
/// label at all; a non-zero `peers_blocked` on any resolver; a `block!` WARN in
/// the logs; `defers == 0` (then the victim never reached "I cannot read this
/// epoch"); an observed drop-reason set other than `COVERED` (then the narrowing
/// in the name is no longer the truth — this catches a reason that STOPS as
/// well as one that STARTS, for LABELS of the `dpos_frontier_dropped_total`
/// family: a new self-inflicted code path that does not count under it is
/// invisible here); a node whose spy was handed to fewer than both
/// slots, a `blocked` vector shorter than the node count, or a missing resolver
/// family (the observables are vacuous again); the victim executing into epoch
/// 2 or holding `PK_2`; the committee halting.
#[test]
fn an_epoch_outside_this_nodes_read_window_costs_no_peer_its_channel() {
    use metrics_util::debugging::DebuggingRecorder;
    use std::collections::BTreeSet;
    const VICTIM: usize = HELD_LAG_VICTIM;
    const N: usize = 5;
    /// Every `reason` under which `plane_upstream::deliver` step (5) may drop an
    /// answer WITHOUT punishing the peer — the whole set, in the source's own
    /// order (`plane_upstream.rs:140-143`). Used for the PRINTED table only, so
    /// the three arms this fixture does not reach are visible as zeroes; the
    /// assertion itself reads the observed labels off the recorder.
    const SELF_INFLICTED: [&str; 4] = [
        "no_geometry",
        "out_of_window",
        "not_readable",
        "read_failed",
    ];
    /// The subset this fixture actually executes, and therefore the only one the
    /// property below is pinned for. Asserted as an exact set against what the
    /// recorder saw.
    const COVERED: [&str; 1] = ["out_of_window"];
    /// The `block!` macro's WARN as the stand's capture renders it
    /// (`capture.rs`: `target: message`), from the resolver's own module.
    const BLOCK_WARN: &str = "commonware_resolver::p2p::engine: invalid data received";

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let out = metrics::with_local_recorder(&recorder, || {
        run_held_lag_over_donor_archive(Some(snapshotter.clone()))
    });

    // PREMISE: the lag is real and HELD, so the epochs the victim is fed are
    // genuinely outside its read window.
    assert_the_lag_is_held(&out);

    // PREMISE: the victim's OWN inlet counted `Committee::scheme(E) == None` on
    // certificates handed to it past its window, and did not call that a data
    // fault. Per node, which the process-wide plane counters below are not.
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

    // (1) THE PROPERTY, by peer and by slot. Deliberately BEFORE the coverage
    // assertion below: "no peer was excluded" is unconditional over the whole
    // run and does not wait on any premise; what the premises buy is the right
    // to read the zero as a statement about the code.
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

    // (2) THE PROPERTY, by reason, off the recorder. `Self::reject` is the ONLY
    // producer of `deliver ⇒ false` and it counts under the arm's own label
    // (`plane_upstream.rs`, `FRONTIER_REJECTED`). Every peer in this run is
    // honest, so the OBSERVED set of rejection labels — whatever they are
    // called, today or later — has to be empty. The printed table beside it is
    // the four self-inflicted arms by name, so the three this fixture does not
    // reach are visible as zeroes rather than summed away.
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

    // (2b) THE PROPERTY, by FACT and by node. `CountingHandler::deliver` counts
    // the `bool` the production consumer returned (`fakes.rs`), on every node's
    // frontier plane (`stand.rs::frontier_plane` wraps unconditionally). It sees
    // a `false` whether or not `Self::reject` produced it, and whether or not
    // the blocker slot held the spy — the one witness of the frontier rule that
    // depends on neither.
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

    // (3) THE PROPERTY, resolver-internal — the half no `Blocker` choice can
    // disarm (`fetcher.block` → append-only `excluded`, see
    // `Outcome::peers_blocked`). Wider than the spy: every resolver engine in
    // the run. The families are asserted BY NAME per node — the three classes
    // the stand labels itself — plus the DKG agreement's, which only dealers
    // run; "some family exists" would let a missing engine hide behind another.
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

    // (4) AND THE SPY IS NOT VACUOUS. Every node handed it to BOTH blocker
    // slots, so the empty list in (1) is a statement about the code and not
    // about a `NoopBlocker` that could never have spoken. What this proves and
    // what it does not is in the docstring. Sorted on both sides: `sites` comes
    // out of a `BTreeSet`, so the literal must not depend on the constants'
    // spelling.
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

    // (5) COVERAGE — what this run ACTUALLY put under load, as the recorder saw
    // it. The observed label set is the assertion, so a reason that stops
    // firing and a reason that starts firing are both red; the printed table
    // over the four known arms is for the reader.
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

    // (6) THE PRICE OF THE REFUSAL (В-3), measured and not fixed. `deliver`'s
    // drop answers the waiting caller NOW and the retry driver is the executor's
    // frozen-tip probe, one ladder step per tick — so the cost is a round trip
    // per probe tick for as long as the lag is held. The bound below is WEAK
    // (journal, accepted residual V-04/D-05/E-03) and is NOT an invariant: the
    // stand's resolver times an active request out at 5 s (`stand.rs`,
    // `frontier_plane`) while the client waits 8 s (`plane_upstream.rs`,
    // `FRONTIER_FETCH_TIMEOUT`), so a timed-out request is re-sent by the
    // resolver (CW `engine.rs`, `pop_active` → `add_retry`) and a later answer
    // that takes step (5) can add a drop with no new `latest_calls` /
    // `finalized_calls`. It does not bite here because the donor answers
    // promptly; it is kept as a sanity rail and a printed number, not as the
    // proof of anything.
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

/// (5.4-А, the mandatory test of the row) A committee MEMBER whose marshal is
/// catching up through its inlet INSIDE its first epoch's deal window deals that
/// epoch before the seal deadline — on the one beacon clock that is left, the
/// marshal's tip.
///
/// **What the tee was for, and what replaces it.** `LiveFrontierTee` fed the
/// `DkgActor` the height of every certificate the inlet verified, one statement
/// BEFORE the marshal was handed the same certificate (`cert_inlet.rs`, the
/// deleted `:781-785` above `verify_block`), so that a still-catching-up
/// early-joiner dealt its first epoch's DKG share at the live frontier instead
/// of K blocks late on its own EL-finalized clock. 5.4-А removes the tee and the
/// `fin + K` poller feeder and leaves the actor ONE clock: the ordering tip the
/// marshal reports (`Update::Tip`, CW `marshal/core/actor.rs:1454-1458`, on
/// every stored finalization above its tip) and `FluentApp::report` publishes
/// on the process-wide watch. The inlet still moves that clock — through the
/// marshal, one call later — and this is the run that says so under the load
/// the tee existed for.
///
/// **The fixture.** Four live nodes, every one in every committee; node 3 runs
/// the production inlet on its own frontier plane. It is cut off (both planes,
/// `Stand::partition`) at height 8 and its links come back when the majority
/// has executed 36 — i.e. INSIDE the epoch-2 deal window: the actor starts
/// epoch 2's ceremony on entering epoch 1 (`epoch_start(1) = 32` on its clock)
/// and seals at `epoch_start(2) − DKG_MARGIN_BLOCKS = 44`. At heal the laggard's
/// own chain stands at 8: its OWN engine cannot open the deal window for it, so
/// the only way its clock reaches 32 before the network's seal at 44 is the
/// certificates that arrive after the heal — its marshal's gap repair and its
/// inlet's by-height walk, both landing in the marshal, whose tip is the clock.
///
/// **What is asserted, and what each assertion is the witness of.**
///
/// 1. The premise: the cut fired, the laggard was BELOW the window at heal and
///    the majority INSIDE it — otherwise the laggard's own chain opened the deal
///    and the run says nothing about catching up.
/// 2. It DEALT, on the artifact's own word: the agreed epoch-2 proposal pins a
///    dealer log at the laggard's seat (`DkgProposal::logs`, `idx` = the
///    dealer's position in `committee[2]`). An artifact alone would not do —
///    a member that missed the seal still ACQUIRES the artifact from its peers
///    (R-121/R-122) — and neither would the chain advancing, since three
///    dealers out of four are a quorum without it.
/// 3. It minted a share off that dealing (`dkg_ceremony_ok_total == 1`) and was
///    never demoted for want of one (`epoch_engine_demoted_no_polynomial_total
///    == 0`) — the consequence the tee's doc named ("deals its first epoch's
///    DKG share before the deal deadline").
/// 4. The clock IS the tip: `dpos_dkg_clock_height <= dpos_ordering_finalized_height`
///    throughout (a clock above the tip is a second feeder, the thing this row
///    removes — Д-5.4Б-3), and equal to it at rest.
/// 5. The inlet fed the catch-up (delivered heights strictly between the
///    laggard's tier-F at heal and the majority's), cost the node nothing, and
///    the node rejoined lockstep; no halt, no ERROR.
///
/// **Falsifier.** M1 of the row's map — an actor that ignores `changed()` — is
/// the direct one: the clock stands at the pre-cut height, the actor never
/// enters epoch 1, never deals, and (2) reds with the laggard's seat missing
/// from the pinned set while the chain goes on without it. M2 — the app
/// publishing on a watch the actor does not hold — reds the same way. A heal
/// landing outside the window reds (1); a dealing that arrived after the
/// peers' seal reds (2); a share that never minted reds (3); a clock fed from
/// anywhere but the marshal reds (4).
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
    /// window with room for the laggard to catch up and deal before the seal.
    const HEAL_ABOVE: u64 = DEAL_OPENS + 4;
    /// Past `epoch_start(2)`, so the artifact, the share and a σ-carrying epoch
    /// are all facts of the run.
    const TARGET: u64 = EPOCH_2_START + EPOCH_LEN;
    let mut cfg = StandConfig::live(N, 1);
    cfg.epoch_len = EPOCH_LEN;
    // Explicit rather than inherited: the laggard has to be IN `committee[2]`
    // for "it dealt" to be about a member.
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

    // (1) THE PREMISE: cut, heal inside the window, laggard below it.
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

    // (2) IT DEALT: the agreed artifact pins a log at the laggard's seat.
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
    // And the other three dealt too: the laggard's dealing was not the only one
    // the seal pinned (a run in which everyone else missed the window would be
    // about something else).
    assert_eq!(
        pinned_seats.len(),
        N,
        "the pinned set is not the whole committee: {pinned_seats:?}"
    );

    // (3) IT MINTED ITS SHARE, and was never demoted for want of one.
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

    // (4) THE CLOCK IS THE TIP.
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

    // (5) THE INLET FED THE CATCH-UP, at no cost, and the node rejoined.
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
