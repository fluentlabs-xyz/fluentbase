//! Э5 5.0а — the stand's `CertInlet`: the second producer into a node's marshal.
//!
//! What these tests can see that no other stand test can: the inlet's own verify
//! gate (`CertInlet::ingest` → `Committee::scheme(E)` → the certificate verify
//! under the epoch's SEED ORACLE), its non-fault deferral, its data-fault
//! rotation, and the live-frontier tee. The frontier plane's own `deliver` cannot
//! stand in for any of it: it builds its verifier WITHOUT an oracle
//! (`plane_upstream.rs::verifier_for` → `build_verifier(.., None)`), so a
//! certificate whose σ slot has been swapped under an intact multisig passes it
//! and reaches the caller — the inlet is where the σ is first judged, and before
//! 5.0а the stand had no inlet at all (`git grep CertInlet testbed/` was empty).
//!
//! Three of the seven tests are fed by the node's own frontier PLANE and four
//! are not, and the split is load-bearing rather than incidental: the plane's
//! `deliver` refuses a certificate whose epoch this node cannot read, so the
//! inlet's own deferral is reachable ONLY through
//! `CertInletSource::PeerArchive` — another node's archive, read directly. Two
//! tee wirings run beside that split, and they answer different questions: the
//! LIST of teed heights (`TeeWiring::Observed`) or the ORDER of the tick against
//! the marshal (`TeeWiring::Production`). `TeeWiring`'s own doc carries the proof
//! that no single wiring gives both.
//!
//! The NEGATIVE CONTROL of the whole file is `StandConfig::cert_inlet = None`:
//! every other test in the crate runs it, no node spawns an inlet, and none of
//! their numbers move. Every test here also states the positive half —
//! `only_these_ran_inlets` — so "the inlet ran where it was asked and nowhere
//! else" is an assertion in each run rather than a property of the green suite.

use super::stand::{
    CertInletCfg, CertInletFacts, CertInletSource, Committees, Outcome, Progress, Stand,
    StandConfig, TeeWiring,
};
use crate::beacon::testing::DETERMINISTIC_BOOTSTRAP_EPOCH;
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

/// (5.0а, the clean path and the TEE) An inlet on a healthy committee member is a
/// second producer into the marshal its own BFT engine already drives, and it
/// changes nothing: the node stays in lockstep, nothing halts, no ERROR line
/// appears. What the run BUYS is the tee and the clean-ingest bookkeeping, which
/// no other stand configuration can show.
///
/// Three things are pinned here and nowhere else in the crate:
///
/// 1. **Every attempt was a clean ingest.** `ingests` counts certificates handed
///    to `CertInlet::ingest`; the tee fires on the LAST line of the clean path
///    only (`cert_inlet.rs`, after `observe_certificate` and
///    after the three fault arms have returned), so `tee_heights.len() ==
///    ingests` says the verify gate passed every time. With `rotations == 0` and
///    `defers == 0` beside it, all three fault arms and the deferral arm are
///    witnessed UNTAKEN on an honest run — which is what makes their being
///    TAKEN, in the two tests below, mean something.
/// 2. **The tee is monotone and contiguous from 1 — and that is a pin on a RATIO
///    OF SPEEDS, not on a mechanism.** The by-height feeder re-reads `tier-F + 1`
///    on every iteration, so nothing in the code forbids it asking the same
///    height twice: that is exactly what happens when the executor has not
///    advanced tier-F between two iterations, and it is what the `Frontier`
///    source below does thousands of times. The equality holds here because the
///    fetch of `tip + 1` effectively waits for the chain to grow (71 ingests over
///    71.9 s of virtual time at one block per second), so what it pins is the
///    ratio — the executor keeps up with the inlet — plus the absence of GAPS,
///    which would mean a certificate the inlet verified for a height it never
///    asked for. One seed (`live(4, 1)`), byte-identical over three runs; a
///    slower executor could legitimately repeat a height without the property
///    itself changing, so the failure message names the shape and not the count.
///
///    The tee's own tick is wired `TeeWiring::Observed` here, which is what makes
///    the LIST readable at all — and, by the same token, says nothing about WHEN
///    the DKG clock moved relative to the marshal's Tip (see `TeeWiring`, and the
///    production-wiring twin at the bottom of this file).
/// 3. **The heights reach the `DkgActor`.** The tee's own channel is drained by
///    the inlet task and forwarded into the REAL `dkg_height_tx` — the channel
///    whose receiver is `ValidatorInputs::heights` — so `dpos_dkg_height_drops_total
///    == 0` says nothing was lost on the forward and `dpos_dkg_clock_height >=
///    max(tee_heights)` says the actor's clamp (`beacon/actor.rs::on_height`) saw
///    at least that height. NOT exclusive: `FluentApp::report(Update::Tip)` feeds
///    the same channel on a healthy node, so this is "the forward is not lossy",
///    not "the tee is the only feeder" (see the journal §4).
///
/// Falsifier: `tee_heights.len() != ingests` (a fault or a deferral happened on
/// an honest run); a non-contiguous or non-monotone tee; a drop on the forward;
/// a dkg clock below the highest teed height; any rotation or deferral; the
/// stand losing lockstep, halting, or printing an ERROR — a second producer into
/// the marshal is not allowed to cost any of those.
#[test]
fn an_inlet_on_a_healthy_member_verifies_every_height_it_is_fed_and_tees_it() {
    const TARGET: u64 = 72;
    let mut cfg = StandConfig::live(4, 1);
    cfg.epoch_len = EPOCH_LEN;
    cfg.cert_inlet = Some(CertInletCfg {
        nodes: vec![3],
        source: CertInletSource::NextAboveTier,
        tee: TeeWiring::Observed,
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
        f.tee_heights.len() as u64,
        f.ingests,
        "an honest run took a fault or a deferral arm: {f:?}"
    );
    assert_eq!(f.rotations, 0, "an honest upstream cost a rotation: {f:?}");
    assert_eq!(f.defers, 0, "an honest run deferred a certificate: {f:?}");

    // (2) The tee is the by-height walk, exactly: 1, 2, 3, … with no gap and no
    // repeat.
    let expected: Vec<u64> = (1..=f.ingests).collect();
    assert_eq!(
        f.tee_heights, expected,
        "the tee is not the contiguous walk the feeder asked for: {f:?}"
    );

    // (3) The forward into the beacon plane's own height channel lost nothing and
    // the actor's clock saw it.
    assert_eq!(
        out.metric(3, "dpos_dkg_height_drops_total"),
        Some(0.0),
        "a teed height was dropped on the forward into dkg_height_tx"
    );
    let teed_top = *f.tee_heights.last().expect("non-empty");
    let dkg_clock = out
        .metric(3, "dpos_dkg_clock_height")
        .expect("the DkgActor gauges its clock on a Beacon::Live node");
    assert!(
        dkg_clock >= teed_top as f64,
        "the DkgActor's clock ({dkg_clock}) never reached the highest teed height ({teed_top})"
    );

    // The second producer is harmless.
    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[]);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    only_these_ran_inlets(&out, &[3]);
    eprintln!(
        "(5.0а/clean) heights={:?} ingests={} tee=[{}..{}] rotations={} defers={} \
         dkg_clock={dkg_clock} virtual={:?}",
        out.heights,
        f.ingests,
        f.tee_heights[0],
        teed_top,
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
/// not served to this victim); a dropped forward into the DKG height channel
/// (which would make a clean ingest look like a fault and the arithmetic above a
/// coincidence); the honest committee losing lockstep, halting, or printing an
/// ERROR line.
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
        tee: TeeWiring::Observed,
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
    // and every height it teed there is a height the forger did NOT forge. The
    // window heights are partitioned by the tee: teed ⇒ verified ⇒ not forged.
    let f = inlet(&out, VICTIM);
    let teed_in_window: Vec<u64> = f
        .tee_heights
        .iter()
        .copied()
        .filter(|h| FORGE_WINDOW.contains(h))
        .collect();
    let ingested_forged: Vec<u64> = byz
        .forged_heights
        .iter()
        .copied()
        .filter(|h| !teed_in_window.contains(h))
        .collect();
    assert!(
        ingested_forged.len() >= crate::cert_inlet::MAX_UPSTREAM_FAULTS as usize,
        "the victim did not take {} forged certificates in a run — forged={:?} \
         teed_in_window={teed_in_window:?}: {f:?}",
        crate::cert_inlet::MAX_UPSTREAM_FAULTS,
        byz.forged_heights
    );

    // (4) THE PROPERTY, and it is arithmetic on the PRIVATE `consecutive_faults`
    // rather than a `> 0`: with `defers == 0` every ingest is either a clean one
    // (it teed) or a data fault, so `faults = ingests − tee_heights.len()` is
    // exact; the faults all fall in one unbroken run (the feeder walks the window
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
    let faults = f.ingests - f.tee_heights.len() as u64;
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

    // (5) THE PREMISE OF THE ARITHMETIC, and the direct witness of the KEY. The
    // step above reads `faults = ingests − tee_heights.len()`, which is exact only
    // while no clean ingest was miscounted as a fault — and the one way that could
    // happen is a tee `try_send` that failed (`cert_inlet.rs:712-717` counts a
    // failed send as a drop and the height never reaches the list). Zero drops is
    // therefore the arithmetic's own precondition, stated.
    assert_eq!(
        out.metric(VICTIM, "dpos_dkg_height_drops_total"),
        Some(0.0),
        "a teed height was lost on the forward, so `ingests − tee.len()` is not the \
         fault count: {f:?}"
    );
    // And the key: every one of those verify failures was judged WITH `PK_2`
    // resolved, counted on the line that can only increment in that regime.
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
         teed_in_window={teed_in_window:?} ingests={} rotations={} defers={} \
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
/// its own read window — and that is what the run asserts: the tee stops at
/// `last(epoch(anchor) + 2)` while the committee is above it, with the plane's
/// drop counter as the witness. Production has no such gate in front of its
/// inlet (the WS stream hands over whatever the upstream sends,
/// `node/src/cert_inlet.rs`), so the deferral arm is a production regime the
/// stand cannot reach through THIS plane — see the journal §0(9)/§5.
///
/// Falsifier: the victim NOT standing below the epoch-2 boundary, or re-jumping,
/// or holding `PK_2` (then the lag is not held and neither arm is what it says);
/// no keyless admission (then the σ was checked after all); any rotation (a
/// missing key or this node's own lag was counted as a data fault); the tee
/// reaching ABOVE the window bound (then `deliver` let an unauthenticatable
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
            tee: TeeWiring::Observed,
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
    let teed_top = *f
        .tee_heights
        .last()
        .expect("the inlet verified at least one certificate");
    assert!(
        teed_top <= window_top,
        "the inlet verified height {teed_top}, above the top of its own \
         committee-read window ({window_top}) — `deliver` let an unauthenticatable \
         certificate through"
    );
    assert!(
        teed_top > (anchor_epoch + MAX_COMMITTEE_LOOKAHEAD_EPOCHS) * EPOCH_LEN - 1,
        "the inlet stopped at {teed_top}, below the top window epoch — the window \
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
    // pinned by the victim's OWN `teed_top` straddling its own window edge in the
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
         defers={} teed_top={teed_top} window_top={window_top} plane_dropped={dropped} \
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
/// committee (so it deals its epoch's DKG and the tee's clock matters for its
/// share), whose EL is nevertheless BEHIND (so the tee is ahead of its own
/// executed tip), with a live inlet. The stand can build exactly that with
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
/// the assertion below counts teed heights strictly between the member's tier-F
/// at heal and the majority's, i.e. certificates it was MISSING at the moment its
/// links came back.
///
/// **What it does not claim.** Not that the inlet is what caught the node up: its
/// own marshal repair is racing the inlet over the same range, and nothing here
/// separates the two (that separation is 5.4's "with the tee / without it"
/// comparison). What is claimed is that the inlet stayed live and productive
/// across the whole episode, that it cost the node nothing (no rotation, no
/// deferral, no verify failure under a resolvable key), and that the node was and
/// remained a member.
///
/// Falsifier: the partition never firing (no observation to read); no lag at heal
/// (then the cut did not isolate anything); the member not holding its own
/// epoch's artifact at the end (then it is the keyless test's outsider, not a
/// member); an inlet that was never fed, or fed nothing inside the catch-up
/// window; any rotation, deferral, or carry-forward verify failure (an honest
/// donor and this node's own lag must cost none of the three); a drop on the tee's
/// forward; the node failing to rejoin lockstep, or the committee halting or
/// printing an ERROR.
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
        // `Observed`: the point of the run is WHICH heights the inlet fed the
        // node while it was behind, and that is the list. The ORDER of the tee
        // against the marshal — the other half of §0.7(а) — needs the production
        // wiring and is the test below.
        tee: TeeWiring::Observed,
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
        .tee_heights
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
    assert_eq!(
        out.metric(LAGGARD, "dpos_dkg_height_drops_total"),
        Some(0.0),
        "a teed height was dropped on the forward into dkg_height_tx"
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
/// 2. **The gate moved from the plane INTO the inlet.** The clean (teed) heights
///    still stop at the top of the read window, exactly as in the keyless test —
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
/// here and one of the two is wrong); a teed height above the window bound (then
/// the inlet verified a certificate whose committee it cannot read); a
/// non-monotone or repeating walk; the victim executing into epoch 2 or holding
/// its key (then it is not the node this fixture is about).
#[test]
fn a_donors_archive_hands_the_inlet_an_epoch_it_cannot_read_and_it_defers() {
    use fluentbase_types::staking_protocol::{epoch_at_block, MAX_COMMITTEE_LOOKAHEAD_EPOCHS};
    const VICTIM: usize = 4;
    const DONOR: usize = 0;
    /// The same target as the keyless test: the committee climbs well above the
    /// victim's read window, so the walk has heights the victim cannot read.
    const TARGET: u64 = 5 * EPOCH_LEN;
    /// Inside epoch 1 and BELOW the epoch-2 mint — see the keyless test above for
    /// why the lag is held by a cut and not by the tracked peer set.
    const CUT_AT: u64 = EPOCH_LEN + 4;
    /// Longer than the run's virtual deadline — the cut never heals.
    const NEVER: u32 = 4096;
    let mut cfg = StandConfig::live(5, 1);
    cfg.epoch_len = EPOCH_LEN;
    cfg.committees = drop_the_last_two_from_epoch_two();
    cfg.cert_inlet = Some(CertInletCfg {
        nodes: vec![VICTIM],
        source: CertInletSource::PeerArchive { from: DONOR },
        tee: TeeWiring::Observed,
    });
    let mut stand = Stand::new(cfg);
    stand
        .partition(&[0, 1, 2, 3], &[VICTIM])
        .after_height(CUT_AT)
        .consensus_only()
        .for_views(NEVER);
    let out = stand.run_until(
        move |p| p.min_height_of(&[0, 1, 2]) >= TARGET,
        Duration::from_secs(400),
    );
    assert!(
        !out.timed_out,
        "the committee did not reach {TARGET}: {:?}",
        out.heights
    );

    // PREMISE: the same held lag as the keyless test — the victim's committee
    // anchor stays in epoch 1, which is what puts the donor's later epochs outside
    // its read window — and the cut that holds it really fired.
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

    // (2) EVERY ingest took one of exactly two arms: clean (teed) or deferred. No
    // fault arm is reachable here (the donor is honest and the pair comes from its
    // own archive), and this equality is what says so.
    assert_eq!(
        f.ingests,
        f.tee_heights.len() as u64 + f.defers,
        "an ingest took neither the clean nor the deferral arm: {f:?}"
    );

    // (3) THE WALK: strictly up, no repeats. The clean prefix is contiguous from 1
    // and the top of it is the top of the read window — the same bound the keyless
    // test reads off the plane, now enforced by the inlet itself.
    assert!(
        f.tee_heights.windows(2).all(|w| w[0] < w[1]),
        "the archive walk repeated or went backwards: {f:?}"
    );
    let anchor_epoch = epoch_at_block(
        out.heights[VICTIM],
        super::fakes::DPOS_ACTIVATION_BLOCK,
        EPOCH_LEN,
    )
    .expect("the victim executed at least one block");
    let window_top = (anchor_epoch + MAX_COMMITTEE_LOOKAHEAD_EPOCHS + 1) * EPOCH_LEN - 1;
    let teed_top = *f
        .tee_heights
        .last()
        .expect("the inlet verified at least one certificate");
    assert!(
        teed_top <= window_top,
        "the inlet verified height {teed_top}, above the top of its own \
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
        "(5.0а/peer-archive defer) heights={:?} ingests={} teed={} teed_top={teed_top} \
         window_top={window_top} defers={} rotations={} virtual={:?}",
        out.heights,
        f.ingests,
        f.tee_heights.len(),
        f.defers,
        f.rotations,
        out.virtual_elapsed
    );
}

/// (5.0а, A-03 — the wiring row 5.4 has to measure on) The SAME clean-member
/// fixture as the first test, with the tee handed the node's REAL `dkg_height_tx`
/// instead of a stand channel.
///
/// **Why it exists as a test and not as a comment.** Under `TeeWiring::Observed`
/// the stand reads the tee's heights out of its own channel and forwards them on
/// AFTER `ingest` returns — that is, after `verify_block` and
/// `report_finalization` have already driven the marshal. Production does the
/// opposite: the tee's `try_send` is the last clean-path statement BEFORE those
/// two calls, so the height is queued for the `DkgActor` while the marshal has
/// not seen the certificate yet. That ORDER is the subject of Э5 §0.7(а) ("the
/// tee fires before `store_finalization`, the marshal's Tip after") and of row
/// 5.4's "with the tee / without it" comparison, and no drain can reproduce it:
/// `ingest` has no await point between the tee and the marshal call, so nothing
/// else in that task can run in between (`TeeWiring`'s doc carries the anchors).
/// This test pins that the production wiring RUNS — the inlet with the real
/// sender, on a member of a live committee, losing nothing — which is the fixture
/// 5.4 extends with a lagging EL and a clock comparison.
///
/// What it can and cannot see. It CANNOT see the list of teed heights: a
/// `tokio::sync::mpsc` channel has one receiver and the beacon actor owns it, so
/// `tee_heights` is empty by construction and the top teed height is read as
/// `ingests` instead — legitimate here only because the twin test above pins
/// `tee == (1..=ingests)` on this very fixture. It also cannot claim the clock
/// moved BECAUSE of the tee: on a healthy node `FluentApp::report(Update::Tip)`
/// feeds the same channel. What it does claim is that nothing was dropped on the
/// production path and that the clock is at least as high as the certificates the
/// inlet verified.
///
/// Falsifier: a non-empty `tee_heights` (then the wiring is not the production
/// one and this test is the first one again); an inlet that was never fed; any
/// rotation or deferral; a single dropped height (under this wiring a drop means
/// the beacon actor's channel was genuinely FULL, which is the counter's
/// documented meaning and a real finding); a DKG clock below the number of
/// certificates the inlet verified; the stand losing lockstep, halting or
/// printing an ERROR.
#[test]
fn the_production_tee_wiring_feeds_the_dkg_clock_with_no_drain_of_ours() {
    /// Short on purpose: this run is about the wiring, and the same fixture's
    /// contiguity and lockstep properties are pinned at length by the first test.
    const TARGET: u64 = 24;
    let mut cfg = StandConfig::live(4, 1);
    cfg.epoch_len = EPOCH_LEN;
    cfg.cert_inlet = Some(CertInletCfg {
        nodes: vec![3],
        source: CertInletSource::NextAboveTier,
        tee: TeeWiring::Production,
    });
    let out = Stand::new(cfg).run_until(reached(TARGET), Duration::from_secs(200));
    assert!(!out.timed_out, "heights {:?}", out.heights);

    let f = inlet(&out, 3);
    assert!(f.ingests > 0, "the inlet was never fed: {f:?}");
    assert!(
        f.tee_heights.is_empty(),
        "the stand recorded teed heights under the PRODUCTION wiring, where the \
         beacon actor owns the only receiver: {f:?}"
    );
    assert_eq!(f.rotations, 0, "an honest upstream cost a rotation: {f:?}");
    assert_eq!(f.defers, 0, "an honest run deferred a certificate: {f:?}");
    assert_eq!(
        f.carry_forward_fails, 0,
        "a certificate failed BLS verify under a resolvable key: {f:?}"
    );

    // The tee's own `try_send` into the real channel lost nothing, and the actor's
    // clamp saw at least the certificates the inlet verified. `ingests` is the top
    // teed height here BECAUSE the twin test pins `tee == (1..=ingests)` on this
    // fixture; without that this number would mean nothing.
    assert_eq!(
        out.metric(3, "dpos_dkg_height_drops_total"),
        Some(0.0),
        "the beacon actor's height channel was full — under this wiring that is the \
         counter's documented meaning and a real finding"
    );
    let dkg_clock = out
        .metric(3, "dpos_dkg_clock_height")
        .expect("the DkgActor gauges its clock on a Beacon::Live node");
    assert!(
        dkg_clock >= f.ingests as f64,
        "the DkgActor's clock ({dkg_clock}) never reached the {} certificates the \
         inlet verified",
        f.ingests
    );

    assert_eq!(out.diverged, None);
    assert!(out.halted.is_empty(), "{:?}", out.halted);
    out.assert_lockstep_except(&[]);
    assert!(out.errors().is_empty(), "{:?}", out.errors());
    only_these_ran_inlets(&out, &[3]);
    eprintln!(
        "(5.0а/production tee) heights={:?} ingests={} tee_heights={} \
         dkg_clock={dkg_clock} virtual={:?}",
        out.heights,
        f.ingests,
        f.tee_heights.len(),
        out.virtual_elapsed
    );
}

/// (5.0а, A-08 — the configuration that would turn a counter into a liar) A
/// cert-inlet asked for on a node whose beacon never DRAINS the DKG height
/// channel is refused at the fixture, loudly.
///
/// The tee's `try_send` into a channel whose receiver was dropped answers
/// `Err(Closed)`, and `LiveFrontierTee` counts every failed send as
/// `dpos_dkg_height_drops_total` — whose documented meaning is the other one
/// ("the channel was FULL"). Two stand configurations drop that receiver:
/// `Beacon::Static` (it is never handed to anyone) and `Role::AbsentBeacon`
/// (`absent()` takes no inputs). On either, EVERY clean ingest would tick the
/// counter, so the number every other test in this file reads as "nothing was
/// lost on the forward" would mean nothing at all. The refusal is at the node
/// build, because a fixture mistake has to be reported where it was made.
///
/// Falsifier: a run that reaches the assertion inside the stand instead of
/// panicking at the build (then the combination is live and the drop counter is
/// no longer a witness anywhere).
#[test]
#[should_panic(expected = "never drains the DKG height channel")]
fn a_cert_inlet_is_refused_on_a_node_whose_beacon_drops_the_height_channel() {
    let mut cfg = StandConfig::honest(4, 1);
    cfg.epoch_len = EPOCH_LEN;
    cfg.cert_inlet = Some(CertInletCfg {
        nodes: vec![3],
        source: CertInletSource::NextAboveTier,
        tee: TeeWiring::Observed,
    });
    Stand::new(cfg).run_until(reached(4), Duration::from_secs(60));
}
