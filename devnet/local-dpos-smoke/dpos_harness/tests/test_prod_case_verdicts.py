"""Both directions of every verdict in `verdicts_rotation` — the five production-path CASES.

`test_prod_substrate.py` covers the shared substrate. This covers the drivers, and it exists for
one reason above all others: **this chunk is made of absence assertions.** The halt case proves a
negative over two timeout windows, the durability case proves that a torn member did NOT re-deal
and holds NO share, production-path proves a watchdog line is NOWHERE in a log, and byzantine-vrf
proves a forged key never finalized. Every one of those passes when the reader is DEAD — a wrong
service, a `find` under a moved datadir, an unstripped grep over a log full of ANSI.

So for each of them there is a test where the FORBIDDEN THING IS PRESENT. A verdict that cannot be
made to fail is not a verdict, and on this chunk that is the failure mode rather than a hypothetical
one: `case-byzantine-vrf.sh:540-545` records that exactly this grep once matched nothing because of
the ANSI escapes, and — under `set -euo pipefail` — aborted the whole case with no diagnostic at
all.
"""

from __future__ import annotations

import pathlib
import re

import pytest

from dpos_harness.cases.smoke import verdicts as V
from dpos_harness.cases.smoke import verdicts_onchain as VO
from dpos_harness.cases.smoke import verdicts_fault as VF
from dpos_harness.cases.smoke import verdicts_rotation as VR
from dpos_harness.core.rpc import strip_ansi

A0 = "0x" + "a0" * 20
A1 = "0x" + "a1" * 20
A2 = "0x" + "a2" * 20
A3 = "0x" + "a3" * 20
A4 = "0x" + "a4" * 20
A5 = "0x" + "a5" * 20
#: The bash the production-path port mirrors. DEPTH-SENSITIVE, like `test_prod_cases.py:48`:
#: <smoke>/dpos_harness/tests/<this file> → two dirnames up is the smoke dir.
CASE_PP_SH = pathlib.Path(__file__).resolve().parents[2] / "scripts" / "case-production-path.sh"
CASE_HALT_SH = pathlib.Path(__file__).resolve().parents[2] / "scripts" / "case-vrf-dkg-halt.sh"
#: The consensus crate, absent in a harness-only checkout — the cross-tree tests below SKIP there
#: rather than fail (same shape as `test_smoke_onchain_cases.py:769`).
BEACON_ACTOR_RS = (pathlib.Path(__file__).resolve().parents[4] / "crates" / "dpos" / "consensus"
                   / "src" / "beacon" / "actor.rs")

INITIAL = [A0, A1, A2, A3, A4]
SET5 = " ".join(sorted(INITIAL))
ROTATED = " ".join(sorted([A0, A1, A2, A3, A5]))


# ══ the shared trigger ═════════════════════════════════════════════════════════════════════

def test_the_expected_set_is_sorted_and_lowercased_like_the_on_chain_answer():
    """`pp_committee` normalises the CHAIN's answer; this normalises the OWNER addresses the same
    way, or the two would never compare equal on a chain that returns mixed case in its own order."""
    assert VR.expected_initial_set([A2.upper(), A0, A1]) == " ".join(sorted([A0, A1, A2]))


def test_the_initial_committee_gate_both_ways():
    assert VR.evaluate_initial_committee(SET5, SET5, 3)[0]
    ok, msg = VR.evaluate_initial_committee(ROTATED, SET5, 3)
    assert not ok and "E0=3" in msg and "initial 5" in msg


def test_the_gate_names_the_epoch_when_it_has_one():
    """Two spellings in the bash and the named one is the useful one: "committee != initial 5"
    says nothing about WHEN."""
    assert "E0=" not in VR.evaluate_initial_committee(ROTATED, SET5)[1]
    assert "E0=7" in VR.evaluate_initial_committee(ROTATED, SET5, 7)[1]


@pytest.mark.parametrize("ahead,want", [
    (ROTATED, True),
    ("", False),                       # unreadable is NOT changed
    (SET5, False),                     # unchanged, and the joiner is not in it
    (" ".join(sorted(INITIAL + [A5])), True),
])
def test_the_rotation_scan_predicate(ahead, want):
    assert VR.rotation_reached(ahead, SET5, A5) is want


def test_an_unreadable_committee_never_anchors_the_rotation():
    """An RPC blip reads as "the committee emptied", which DIFFERS from E0's — without the
    non-empty guard the scan would anchor E_new on a failed read."""
    assert VR.rotation_reached("", SET5, A5) is False


def test_membership_is_space_delimited_and_never_a_substring():
    """`[[ " $SET " == *" $addr "* ]]`. Without the padding a prefix of a real address matches, and
    on 40-hex addresses that is rare enough to pass for months and wrong forever."""
    short = A0[:20]
    assert VR.committee_has(SET5, A0)
    assert not VR.committee_has(SET5, short)
    assert not VR.committee_has(SET5, A5)


def test_the_rotation_found_and_real_rotation_verdicts_both_ways():
    assert VR.evaluate_rotation_found(4)[0]
    assert not VR.evaluate_rotation_found(None)[0]
    assert VR.evaluate_real_rotation(ROTATED, SET5, 4)[0]
    ok, msg = VR.evaluate_real_rotation(SET5, SET5, 4)
    assert not ok and "not actually a rotation" in msg


# ══ case-production-path ═══════════════════════════════════════════════════════════════════

def test_plane_native_rejects_a_ws_upstream():
    ok, msg = VR.evaluate_plane_native(
        "fluent node --dpos --dpos.follower-upstream=ws://172.20.0.10:8546")
    assert not ok and "plane de-hub regressed" in msg


def test_plane_native_accepts_a_committee_validator():
    assert VR.evaluate_plane_native("/usr/local/bin/fluent node --chain=x --dpos --http")[0]


def test_an_unreadable_cmdline_fails_on_the_FIRST_check_not_the_second():
    """The order is bash's and it matters: an EMPTY string trivially passes "does not contain
    follower-upstream", so reversing the two would report "the plane de-hub regressed" for a
    container whose cmdline could not be read at all."""
    ok, msg = VR.evaluate_plane_native("")
    assert not ok and "could not read v0 live cmdline" in msg


def test_the_join_committee_needs_the_joiner_AND_the_size():
    assert VR.evaluate_join_committee(ROTATED, A5, 4, 5)[0]
    assert not VR.evaluate_join_committee(SET5, A5, 4, 5)[0]
    grown = " ".join(sorted(INITIAL + [A5]))
    ok, msg = VR.evaluate_join_committee(grown, A5, 4, 5)
    assert not ok and "committee size != 5" in msg


def test_the_displaced_member_is_the_one_missing_from_the_rotated_set():
    assert VR.displaced_idx(INITIAL, ROTATED) == 4
    assert VR.displaced_idx(INITIAL, SET5) is None


def test_a_rotation_that_displaced_the_host_rpc_node_is_refused():
    """v0 is the node every reading in the case goes through. bash calls displacing it "tie-break
    drift" and FAILS rather than adapting."""
    assert VR.evaluate_displaced(4)[0]
    assert not VR.evaluate_displaced(None)[0]
    ok, msg = VR.evaluate_displaced(0)
    assert not ok and "harness RPC" in msg


@pytest.mark.parametrize("pre,post,ok", [
    ("0x100", "0x101", True),
    ("0x101", "0x101", False),
    ("null", "0x101", False),
    ("0x100", "null", False),
    ("", "0x101", False),
])
def test_the_demoted_validator_must_keep_following(pre, post, ok):
    """The `null` sentinel is a FAILURE and not a comparison: a node whose RPC is gone is not a
    node that is following, and `printf '%d' null` would have aborted the bash."""
    assert VR.evaluate_still_following(pre, post, 4)[0] is ok

def _bash_arith(text: str, var: str) -> str:
    """The right-hand side of a `VAR=$(( … ))` assignment in the bash case, as a Python-evaluable
    expression (bash and Python agree on `+`/`-` over ints)."""
    m = re.search(rf"^{var}=\$\(\( (.+?) \)\)\s*$", text, re.M)
    assert m, f"{var}=$(( … )) is gone from {CASE_PP_SH.name}"
    return m.group(1)

def test_the_watchdog_absence_fails_when_the_line_IS_present():
    """`grep -q "NOT in the current committee"` over v5's ENTIRE log. The pass side is an absence,
    so the only direction that distinguishes a working grep from a dead one is this one."""
    assert VR.evaluate_watchdog_silent([])[0]
    ok, msg = VR.evaluate_watchdog_silent(["WARN validator NOT in the current committee epoch=4"])
    assert not ok and "committee watchdog" in msg


def test_the_in_process_promotion_must_be_observed():
    assert VR.evaluate_promoted_in_process(1)[0]
    ok, msg = VR.evaluate_promoted_in_process(0)
    assert not ok and "joined some other way" in msg


def test_still_finalizing_is_strict():
    assert VR.evaluate_still_finalizing(10, 11)[0]
    assert not VR.evaluate_still_finalizing(10, 10)[0]
    assert not VR.evaluate_still_finalizing(10, 9)[0]


# ══ case-vrf-rotation ══════════════════════════════════════════════════════════════════════

def test_the_probe_set_is_the_full_committee_with_validator_0_first():
    members = VR.probe_members(INITIAL + [A5], ROTATED, lambda i: f"validator-{i}")
    assert members[0] == "validator-0"
    assert set(members) == {"validator-0", "validator-1", "validator-2", "validator-3",
                            "validator-5"}


def test_the_probe_set_must_contain_the_joiner():
    """Under the always-on beacon plane the joiner holds a share at E_new too — it dealt one from
    its follower phase. A probe set without it would delete the early-join half of the case."""
    ok, _ = VR.evaluate_probe_members(["validator-0", "validator-5"], "validator-5")
    assert ok
    ok, msg = VR.evaluate_probe_members(["validator-0"], "validator-5")
    assert not ok and "the rotation did not bring v5 in" in msg
    ok, msg = VR.evaluate_probe_members([], "validator-5")
    assert not ok and "could not map" in msg


def test_the_early_join_lifecycle_grep_both_ways():
    logs = ("noise\nlive DKG: ceremony started epoch=4\nmore\n"
            "live DKG: PK_epoch + share computed + stored epoch=4\n")
    hits = VR.dkg_lifecycle_lines(logs)
    assert len(hits) == 2 and VR.evaluate_early_join(hits)[0]
    ok, msg = VR.evaluate_early_join(VR.dkg_lifecycle_lines("nothing here\n"))
    assert not ok and "beacon OBSERVER" in msg


def test_the_lifecycle_grep_keeps_only_the_last_four():
    logs = "\n".join(f"live DKG: ceremony started epoch={i}" for i in range(10))
    assert len(VR.dkg_lifecycle_lines(logs)) == 4


def test_active_growth_names_the_ROLE_in_the_failure():
    """A frozen count on a STAYER means the beacon did not relive; on the JOINER it means a
    shareless observer. Two different bugs behind one number, so the message says which."""
    assert VR.evaluate_member_active_growth(3, 4, "validator-1", "validator-5")[0]
    ok, msg = VR.evaluate_member_active_growth(4, 4, "validator-1", "validator-5")
    assert not ok and "share-holder" in msg
    ok, msg = VR.evaluate_member_active_growth(4, 3, "validator-5", "validator-5")
    assert not ok and "EARLY-JOIN newcomer" in msg


@pytest.mark.parametrize("lo,epoch_len,want", [(100, 64, 104), (100, 5, 104), (100, 4, 103)])
def test_the_carry_window_never_reaches_into_the_next_epoch(lo, epoch_len, want):
    assert VR.carry_window_hi(lo, epoch_len) == want


def test_a_stable_epoch_needs_two_READABLE_equal_committees():
    """Two unreadable committees are both "" and compare EQUAL — without the non-empty half an RPC
    outage would present as the stable epoch the case is looking for."""
    assert VR.is_stable_epoch(SET5, SET5)
    assert not VR.is_stable_epoch(SET5, ROTATED)
    assert not VR.is_stable_epoch("", "")


def test_the_stable_epoch_verdict_both_ways():
    assert VR.evaluate_stable_epoch(6, 5, 8)[0]
    ok, msg = VR.evaluate_stable_epoch(None, 5, 8)
    assert not ok and "cannot test carry-forward" in msg


def test_the_stall_discriminator_says_which_of_the_two_it_is():
    assert "just slow" in VR.stall_verdict(100, 105)
    assert "FROZEN" in VR.stall_verdict(100, 100)


# ══ the shared torn-journal recipe ═════════════════════════════════════════════════════════

def test_TORN_LINE_is_a_string_the_product_ACTUALLY_EMITS_from_the_Torn_arm():
    """THE ANTI-VACUITY GUARD for both DKG cases, and for the halt case it is the difference
    between a proof and a coincidence.

    The halt case's whole claim is "the committee is SHARELESS and therefore the boundary cannot be
    crossed". It establishes shareless by grepping this line on both victims. If the product
    renames the warn, the grep goes dead — and a dead grep on this case does not merely weaken it:
    the sit-out assertion would fail loud, which is the correct direction, but only as long as the
    string is checked against the product rather than against itself.

    So: the string must appear under a `tracing::warn!` in `maybe_start`'s `JournalLoad::Torn`
    arm — the ONLY arm that sits out unconditionally (`Present` re-derives the dealer pre-deadline,
    `NoFile` re-deals), and the one `torn_warned` makes permanent for the epoch."""
    if not BEACON_ACTOR_RS.exists():
        pytest.skip(f"consensus crate not in this tree ({BEACON_ACTOR_RS})")
    body = BEACON_ACTOR_RS.read_text(encoding="utf-8", errors="replace").splitlines()
    hits = [(n, ln) for n, ln in enumerate(body, 1) if VR.TORN_LINE in ln]
    assert len(hits) == 1, f"{VR.TORN_LINE!r} must appear exactly once in actor.rs, found {hits}"
    lineno, line = hits[0]
    assert not line.strip().startswith("//"), (
        f"actor.rs:{lineno} is a COMMENT — grepping a comment makes the sit-out unfalsifiable")
    window = body[max(0, lineno - 8):lineno]
    assert any("tracing::warn!(" in w for w in window), (
        f"actor.rs:{lineno} is not inside a warn! — the sit-out must stay operator-visible")
    assert any("JournalLoad::Torn" in w for w in window), (
        f"actor.rs:{lineno} left the JournalLoad::Torn arm — the cases would be grepping a "
        "different (possibly recoverable) outcome")


def test_CEREMONY_STARTED_LINE_is_a_string_the_product_ACTUALLY_EMITS():
    """The other half of the sit-out proof: a SECOND one of these is a re-deal. A dead grep here
    reads as zero starts, which `evaluate_no_re_deal` accepts — so this one passes vacuously if the
    string drifts, and only a cross-tree check catches it."""
    if not BEACON_ACTOR_RS.exists():
        pytest.skip(f"consensus crate not in this tree ({BEACON_ACTOR_RS})")
    text = BEACON_ACTOR_RS.read_text(encoding="utf-8", errors="replace")
    emitters = [ln for ln in text.splitlines()
                if VR.CEREMONY_STARTED_LINE in ln and not ln.strip().startswith("//")]
    assert len(emitters) == 1, f"{VR.CEREMONY_STARTED_LINE!r} emitters: {emitters}"
    assert "tracing::info!" in emitters[0]


def test_the_bash_halt_case_tears_with_the_SAME_magic_and_the_SAME_torn_grep():
    """Two trees, two literals, one claim. A recipe fixed in one and not the other reproduces the
    original defect in half the runs — the drift class
    `test_smoke_onchain_cases.py::test_both_trees_grep_the_SAME_park_string` exists for."""
    if not CASE_HALT_SH.exists():
        pytest.skip(f"bash case not in this tree ({CASE_HALT_SH})")
    text = CASE_HALT_SH.read_text()
    assert VR.TORN_LINE in text
    assert VR.CEREMONY_STARTED_LINE in text
    assert VR.TORN_MAGIC in text
    # OCTAL, never hex: genesis-init's /bin/sh has POSIX `\\ooo` and would write the literal
    # characters for `\\xHH` — a corruption that lands as Present/NoFile rather than Torn.
    assert r'printf "\377\377\377\377"' in text
    # And the share must be removed too, or `store.contains_key` short-circuits before the journal
    # is ever read (`actor.rs:982-989`) and the torn journal is never even opened.
    assert re.search(r'vol_mutate_beacon "\$i" "beacon-share-e\$e\.bin" .*rm -f', text)
    # Both victims, not one: on n=5 a single sit-out leaves 4 dealers = quorum and the ceremony
    # finalizes, which is the DURABILITY case's outcome.
    assert 'for i in "$K0" "$K1"; do tear_journal_to_torn "$i" "$E_new"; done' in text


# ══ case-vrf-dkg-halt ══════════════════════════════════════════════════════════════════════

def test_the_kill_set_is_committee_stayers_from_v1_to_v4_in_index_order():
    kills = VR.kill_candidates(INITIAL + [A5], ROTATED)
    assert kills == [1, 2]
    assert VR.evaluate_kill_set(kills, ROTATED)[0]


def test_a_committee_without_two_non_leader_stayers_fails_loud():
    only_v0_and_joiner = " ".join(sorted([A0, A5]))
    kills = VR.kill_candidates(INITIAL + [A5], only_v0_and_joiner)
    ok, msg = VR.evaluate_kill_set(kills, only_v0_and_joiner)
    assert not ok and "could not find 2" in msg


@pytest.mark.parametrize("journal,sealed,absent,want", [
    (True, False, True, True),      # the pre-seal window
    (False, False, True, False),    # the ceremony never started
    (True, True, True, False),      # ALREADY SEALED — this is the recoverable case, not terminal
    (True, False, False, False),    # already finalized
])
def test_the_preseal_window_needs_all_three(journal, sealed, absent, want):
    assert VR.preseal_ok(journal, sealed, absent) is want


def test_a_post_seal_kill_is_refused_by_the_gate():
    """The seal-absent half is what makes the halt TERMINAL. Post-seal the survivors hold the
    disseminated Reveals, the DKG finalizes and the boundary crosses — the OPPOSITE outcome from
    the same kill count, which is `case-vrf-dkg-durability` phase 1."""
    assert not VR.preseal_ok(journal_present=True, sealed=True, share_absent=True)


def test_the_window_missed_rail_both_ways():
    assert VR.evaluate_window_not_missed(100, 200, "E_new=4")[0]
    ok, msg = VR.evaluate_window_not_missed(200, 200, "E_new=4")
    assert not ok and "window missed" in msg


def test_the_seal_deadline_is_one_margin_below_the_boundary():
    """`epoch_start − DKG_MARGIN_BLOCKS` (beacon/actor.rs:83), from the one named constant."""
    assert VR.seal_deadline(200) == 200 - VF.DKG_MARGIN_BLOCKS
    assert VR.seal_deadline(200, margin=16) == 184


def test_the_preseal_rail_closes_at_the_seal_deadline_not_the_boundary():
    """THE off-by-one-margin this rail exists for. Between the seal deadline and the boundary the
    ceremony has already sealed, so a kill there lets the survivors finalize on the disseminated
    Reveals and RECOVER — the durability case's outcome, and the one thing the halt case is the
    negative control for. The old rail (`< boundary`) accepted that whole 20-block window."""
    deadline = VR.seal_deadline(200)
    assert VR.evaluate_seal_window_not_missed(deadline - 1, 200, "E_new=4")[0]
    ok, msg = VR.evaluate_seal_window_not_missed(deadline, 200, "E_new=4")
    assert not ok and "SEAL DEADLINE" in msg and "POST-seal and RECOVERABLE" in msg
    # the block the OLD rail would have waved through
    assert not VR.evaluate_seal_window_not_missed(199, 200, "E_new=4")[0]
    assert VR.evaluate_window_not_missed(199, 200, "E_new=4")[0]


def test_the_climb_is_the_discriminator():
    assert VR.evaluate_climbed(191, 191)[0]
    ok, msg = VR.evaluate_climbed(150, 191)
    assert not ok and "may not have rejoined consensus" in msg


@pytest.mark.parametrize("a,b,frozen", [(100, 100, True), (100, 99, True), (100, 101, False)])
def test_frozen_accepts_a_head_that_went_backwards(a, b, frozen):
    """`(( b <= a ))`, not `==`. A re-org or a restarted RPC serving an older tip is not progress
    either, and bash counts it as frozen."""
    assert VR.head_frozen(a, b) is frozen


def test_the_terminal_halt_fails_when_the_boundary_CROSSED():
    """THE REGRESSION LINE. Everything before it has already proven the committee is shareless for
    E_new — both victims logged the Torn sit-out, neither re-dealt, nobody computed a share — so an
    ADVANCING head here is a chain that crossed a change-epoch boundary with an unfinished DKG key.
    The message has to say that and not "a kill slipped post-seal", because with the tear recipe a
    slipped kill is no longer the plausible cause."""
    assert VR.evaluate_terminal_halt(True)[0]
    ok, msg = VR.evaluate_terminal_halt(False, 191, 195)
    assert not ok
    assert "CROSSED" in msg and "SHARELESS" in msg
    assert "application.rs:473-479" in msg and "REGRESSED" in msg


def test_the_halt_must_be_PERMANENT():
    assert VR.evaluate_permanent_halt(True)[0]
    ok, msg = VR.evaluate_permanent_halt(False)
    assert not ok and "must never self-heal" in msg


def test_the_dkg_none_discriminator_fails_when_a_member_DOES_hold_a_share():
    """The absence, driven with the share present. A survivor holding an E_new share would mean
    the ceremony finalized, i.e. an ordinary stall rather than a DKG-None boundary skip."""
    assert VR.evaluate_no_share_computed(3, False, 4)[0]
    ok, msg = VR.evaluate_no_share_computed(3, True, 4)
    assert not ok and "not a terminal halt" in msg


def test_finalized_must_never_cross_the_boundary():
    """The SECOND regression line, measured on finalized rather than the tip and on the boundary
    height itself — so a crossing that happened while the tip was momentarily flat still fails."""
    assert VR.evaluate_below_boundary(190, 192)[0]
    ok, msg = VR.evaluate_below_boundary(192, 192)
    assert not ok and "shareless committee" in msg and "did NOT hold" in msg
    assert not VR.evaluate_below_boundary(500, 192)[0]


def test_the_panic_sweep_finds_both_spellings_and_passes_on_a_clean_log():
    """The absence, driven with the forbidden line present — in BOTH of bash's alternations."""
    assert VR.panic_lines("all fine\nINFO block sealed\n") == []
    assert VR.panic_lines("ERROR PANIC at foo") != []
    assert VR.panic_lines("thread 'main' panicked at src/lib.rs:1") != []
    assert not VR.evaluate_no_panic(VR.panic_lines("thread 'x' panicked at y"))[0]
    assert VR.evaluate_no_panic([])[0]


# ══ case-vrf-dkg-durability ════════════════════════════════════════════════════════════════

def test_the_torn_victim_prefers_v3_and_falls_back_to_another_stayer():
    assert VR.torn_victim(INITIAL + [A5], ROTATED) == 3
    without_v3 = " ".join(sorted([A0, A1, A2, A4, A5]))
    assert VR.torn_victim(INITIAL + [A5], without_v3) == 1
    only_leader = " ".join(sorted([A0, A5]))
    assert VR.torn_victim(INITIAL + [A5], only_leader) is None
    assert not VR.evaluate_torn_victim(None, only_leader)[0]


def test_the_seal_gate_and_the_control_stall_both_ways():
    assert VR.evaluate_seal_gate(True)[0]
    assert not VR.evaluate_seal_gate(False)[0]
    assert VR.evaluate_control_stall(True)[0]
    ok, msg = VR.evaluate_control_stall(False)
    assert not ok and "recovery required" in msg


def test_the_control_fails_when_the_chain_KEPT_GOING_with_two_of_five_down():
    """The no-event proof, driven with the event present. If the head advances with 2 of 5 down
    the quorum is not what the case assumes, and the recovery afterwards proves nothing."""
    ok, msg = VR.evaluate_control_stall(False)
    assert not ok and "did NOT freeze" in msg


def test_the_rejoin_and_the_resume_verdicts_both_ways():
    assert VR.evaluate_rejoin(True, 262)[0]
    ok, msg = VR.evaluate_rejoin(False, 262)
    assert not ok and "2-simultaneous-down rejoin wedge" in msg
    assert VR.evaluate_resumed_past_stall(300, 250)[0]
    assert not VR.evaluate_resumed_past_stall(250, 250)[0]


def test_a_victim_that_came_back_shareless_fails():
    assert VR.evaluate_share_recovered(True, 3)[0]
    ok, msg = VR.evaluate_share_recovered(False, 3)
    assert not ok and "durable share reload" in msg


def test_the_slash_absence_fails_when_a_slash_names_the_victim():
    """The absence, driven with the forbidden event present — through the SAME `slash_hits` the
    midwindow case uses, so the markers cannot drift between the two."""
    logs = f"WARN ValidatorSlashed validator={A3[2:]} epoch=4"
    hits = VO.slash_hits(logs, A3)
    assert hits and not VR.evaluate_not_slashed(hits, 3)[0]
    assert VR.evaluate_not_slashed(VO.slash_hits(logs, A4), 4)[0]


def test_an_empty_status_read_is_a_hard_error_not_a_not_jailed():
    assert VR.evaluate_status_readable("1", 3)[0]
    ok, msg = VR.evaluate_status_readable("", 3)
    assert not ok and "cannot assert not-jailed" in msg
    ok, msg = VR.evaluate_status_readable("3", 3)
    assert not ok and "JAILED" in msg


def test_the_corruption_readback_must_be_exactly_the_magic():
    """A silent file-op no-op — `docker compose exec` on a STOPPED container — must FAIL LOUD.
    Without the corruption the restart takes the normal resume path and every later assertion in
    the phase measures a node that was never torn."""
    assert VR.evaluate_corruption_landed("ffffffff", 3)[0]
    ok, msg = VR.evaluate_corruption_landed("", 3)
    assert not ok and "did NOT land" in msg
    assert not VR.evaluate_corruption_landed("00000000", 3)[0]


def test_the_torn_sitout_and_liveness_verdicts_both_ways():
    assert VR.evaluate_chain_live_across_torn(True)[0]
    assert not VR.evaluate_chain_live_across_torn(False)[0]
    assert VR.evaluate_torn_sitout(True, 3, 4)[0]
    ok, msg = VR.evaluate_torn_sitout(False, 3, 4)
    assert not ok and "NoFile" in msg


def test_a_torn_resume_that_RE_DEALT_fails():
    """The forbidden thing present: a SECOND "ceremony started" is a re-deal, which is
    self-equivocation. Zero is accepted because the journal gate already excluded it."""
    assert VR.evaluate_no_re_deal(1, 3, 4)[0]
    assert VR.evaluate_no_re_deal(0, 3, 4)[0]
    ok, msg = VR.evaluate_no_re_deal(2, 3, 4)
    assert not ok and "RE-DEALT" in msg


def test_the_torn_victim_must_be_SHARELESS():
    assert VR.evaluate_shareless(False, 3, 4)[0]
    ok, msg = VR.evaluate_shareless(True, 3, 4)
    assert not ok and "despite sitting out torn" in msg


def test_the_survivors_must_each_hold_a_share_and_reach_quorum():
    assert VR.evaluate_member_finalized(True, 1, 4)[0]
    assert not VR.evaluate_member_finalized(False, 1, 4)[0]
    assert VR.evaluate_finalized_quorum(4)[0]
    ok, msg = VR.evaluate_finalized_quorum(3)
    assert not ok and "want >= 4" in msg


def test_the_equivocation_absence_fails_when_evidence_names_the_victim():
    """The on-chain witness for the log-side no-re-deal check, driven with the evidence present.
    Both markers, and both are `-i` in bash."""
    for marker in VR.EQUIV_MARKERS:
        logs = f"WARN {marker}ion detected who={A3[2:]}"
        assert VR.equiv_hits(logs, A3), marker
        assert not VR.evaluate_no_equivocation(VR.equiv_hits(logs, A3), 3)[0]
    assert VR.evaluate_no_equivocation(VR.equiv_hits("all quiet", A3), 3)[0]


def test_the_equivocation_grep_needs_BOTH_a_marker_AND_the_address():
    assert not VR.equiv_hits(f"WARN something who={A3[2:]}", A3)
    assert not VR.equiv_hits("WARN equivocation who=" + A4[2:], A3)


# ══ case-byzantine-vrf ═════════════════════════════════════════════════════════════════════

def test_the_honest_set_excludes_the_byzantine_node():
    nodes = ("validator-0", "validator-1", "validator-2", "full-node")
    assert VR.honest_nodes(nodes, "validator-2") == ("validator-0", "validator-1", "full-node")


@pytest.mark.parametrize("desc,ahead,want", [
    ("IN", ROTATED, True),
    ("IN", SET5, False),
    ("OUT", SET5, True),
    ("OUT", ROTATED, False),
    ("IN", "", False),      # unreadable is not a landed flip in either direction
    ("OUT", "", False),
])
def test_a_flip_lands_only_when_the_ahead_committed_set_agrees(desc, ahead, want):
    assert VR.toggle_landed(desc, ahead, A5) is want


def test_a_byzantine_that_fell_out_of_the_committee_is_named_as_such():
    """It can only forge at a boundary it LEADS, and only as a member. bash names the FIX — raise
    the boost — rather than reporting "the forge never fired" forty minutes later."""
    assert VR.evaluate_byz_in_committee(ROTATED, A2, 2, 5)[0]
    without_byz = " ".join(sorted([A0, A1, A3, A4, A5]))
    ok, msg = VR.evaluate_byz_in_committee(without_byz, A2, 2, 5)
    assert not ok and "raise the byzantine boost" in msg


def test_the_forge_verdict_enumerates_the_four_causes():
    assert VR.evaluate_forged(2, 2, 5)[0]
    ok, msg = VR.evaluate_forged(0, 2, 5)
    assert not ok
    for cause in ("BYZ_VRF_MAX_TOGGLES", "EffBal/warmup", "dpos-devnet-byzantine", "boost too "):
        assert cause in msg, cause


def test_the_forge_epoch_is_the_LAST_one_and_needs_an_ANSI_STRIPPED_log():
    """The strip is not the reader's discretion. The tracing renderer wraps the `=` in colour
    escapes, so a raw log yields NOTHING — and in bash the unguarded `$( … | grep … )` then exited
    non-zero and aborted the case with no diagnostic at all (`case-byzantine-vrf.sh:540-545`)."""
    clean = (f"WARN {VR.FORGE_LINE} epoch=3\n"
             f"WARN {VR.FORGE_LINE} epoch=7\n")
    assert VR.forge_epoch(clean) == "7"
    raw = f"WARN {VR.FORGE_LINE} epoch\x1b[0m=\x1b[0m7"
    assert VR.forge_epoch(raw, fallback=None) == ""
    assert VR.forge_epoch(strip_ansi(raw)) == "7"


def test_the_forge_epoch_falls_back_to_E_new():
    assert VR.forge_epoch("nothing", fallback=5) == "5"
    assert VR.evaluate_forge_epoch(VR.forge_epoch("nothing", fallback=5))[0]
    ok, msg = VR.evaluate_forge_epoch(VR.forge_epoch("nothing", fallback=None))
    assert not ok and "observability gap" in msg


def test_a_line_that_is_not_a_forge_line_never_contributes_an_epoch():
    assert VR.forge_epoch("INFO some other line epoch=9", fallback=None) == ""


def test_the_c_gate_rejection_is_the_POSITIVE_half_of_the_safety_proof():
    """Without it, a run where the byzantine simply never proposed to validator-0 would look
    identical to a run where the gate worked."""
    assert VR.evaluate_c_gate_rejected(3)[0]
    ok, msg = VR.evaluate_c_gate_rejected(0)
    assert not ok and "did not reject the forged boundary at verify" in msg


def test_the_byzantine_liveness_message_differs_from_the_shared_one():
    """Same shape, different diagnosis: here a frozen chain means a byzantine stayer wedged
    liveness, which is the second half of what the case claims."""
    assert VR.evaluate_byz_liveness(10, 11)[0]
    ok, msg = VR.evaluate_byz_liveness(10, 10)
    assert not ok and "byzantine stayer wedged liveness" in msg
    assert "byzantine" not in VR.evaluate_still_finalizing(10, 10)[1]


# ══ the two shared-verdict extensions ══════════════════════════════════════════════════════

def _rows(values):
    return [(100 + i, list(v)) for i, v in enumerate(values)]


def test_the_honest_window_drops_ONLY_the_distinctness_check():
    """`case-byzantine-vrf.sh:112` omits the variance check that `lib.sh:332` runs. A STUCK beacon
    must therefore pass the honest window and FAIL the shared one — and every other property must
    behave identically in both."""
    stuck = _rows([("0xaa", "0xaa"), ("0xaa", "0xaa")])
    names = ["validator-0", "validator-1"]
    assert not V.evaluate_beacon_window(names, stuck, "x")[0]
    assert V.evaluate_beacon_window(names, stuck, "x", require_distinct=False)[0]


@pytest.mark.parametrize("rows,fragment", [
    (_rows([("null", "0xaa")]), "has no mixHash"),
    (_rows([("0x0000", "0x0000")]), "prev_randao is zero"),
    (_rows([("0xaa", "0xbb")]), "disagree on prev_randao"),
])
def test_the_other_three_properties_still_fire_in_the_honest_window(rows, fragment):
    ok, msg, _ = V.evaluate_beacon_window(["validator-0", "validator-1"], rows, "x",
                                          require_distinct=False)
    assert not ok and fragment in msg


def test_slash_hits_keeps_its_default_markers_when_none_are_given():
    """The parameterisation must not have changed the default: `verdicts_onchain`'s own callers
    pass no markers, and a default that had drifted would silently narrow chunk 4's grep."""
    logs = f"WARN ValidatorSlashed who={A3[2:]}"
    assert VO.slash_hits(logs, A3)
    assert not VO.slash_hits(logs, A3, markers=("nosuchmarker",))
