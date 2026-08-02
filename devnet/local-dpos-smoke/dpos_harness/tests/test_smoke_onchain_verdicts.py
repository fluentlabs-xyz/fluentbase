"""`cases/smoke/verdicts_onchain.py` — every decision of the four ON-CHAIN cases, BOTH ways.

The three sibling verdict files each make one argument for why this layer is tested apart from
the bodies. This one makes two, and the second is specific to the cluster:

  * A live run only ever walks the PASS side. Every FAIL branch here is a state a green devnet
    cannot produce, and several of them need a Byzantine build, a corrupted journal, or an
    unpatched binary to produce at all.
  * FIVE of these decisions are NEGATIVES — they pass when something did NOT happen — and two of
    them (`evaluate_no_old_fatal`, `evaluate_no_consensus_exit`) pass when a COUNT DID NOT MOVE.
    That is the one shape that also passes when the reader is silent, when the log is empty, and
    when the window under test never opened. So for every absence-gate below there is a test in
    which the forbidden line IS present, and `evaluate_park_exercised` — the positive control that
    makes the other two mean anything — is driven both ways too.

The participation sentinels get the most coverage of anything here, because collapsing `-1` or
`-2` onto `0` is a false verdict in BOTH directions: it makes `smoke-liveness` pass on a chain it
could not read, and it makes `smoke-vrf-dkg-restart-midwindow` fail a node that recovered.
"""

from __future__ import annotations

import re

import pytest

from dpos_harness.cases.smoke import verdicts_onchain as vo


def _chain(h):
    """A stand-in hub for the same-height fork check: the canonical chain's block at height N
    hashes to `0x<N>`; above 1000 the hub does not hold the block and `blockhash_at` renders it
    `"null"`.

    The two rejoin verdicts take this injected — the default shells out to `cast block`, which
    the suite's conftest blocks, and a unit test that comes out right only because the fixture
    swallowed a subprocess is testing the fixture."""
    return f"0x{h}" if 1 <= h <= 1000 else "null"


# ══ the participation sentinels ════════════════════════════════════════════

@pytest.mark.parametrize("seen,want", [
    (-2, vo.STATE_FAILED),
    (-1, vo.STATE_ABSENT),
    (0, vo.STATE_OK),
    (7, vo.STATE_OK),
    ("-2", vo.STATE_FAILED),
    ("", vo.STATE_FAILED),
    (None, vo.STATE_FAILED),
])
def test_credit_state_keeps_the_three_answers_apart(seen, want):
    """`0` is a REAL counter and must never be confused with either sentinel — that is the whole
    contract. An unparseable reading is treated as read-failed, which is the conservative arm: it
    retries and, if it persists, fails loud."""
    assert vo.credit_state(seen) == want


def test_a_failed_read_never_satisfies_the_liveness_comparison():
    """THE FALSE PASS `smoke-liveness` exists to avoid. `-2 < 10` is true as arithmetic, so a
    getter that stopped answering would make "the victim signed fewer certs than the hub" pass
    trivially — on a chain the harness could not read at all."""
    assert vo.production_hit(-2, 10) is False
    ok, msg = vo.evaluate_production(4, -2, -2, 10, 10)
    assert ok is False and "victim getter read FAILED" in msg


def test_a_not_in_committee_victim_never_satisfies_the_liveness_comparison():
    assert vo.production_hit(-1, 10) is False
    ok, msg = vo.evaluate_production(4, -1, -1, 10, 10)
    assert ok is False and "victim address is NOT in this epoch's committee" in msg


def test_a_failed_HUB_read_is_named_as_the_hub():
    """Two addresses are read per poll and they call for different next steps: a broken hub read
    is an RPC problem, a broken victim read may be a committee problem."""
    ok, msg = vo.evaluate_production(4, 3, 10, -2, -2)
    assert ok is False and "hub getter read FAILED" in msg


def test_production_hit_needs_the_hub_to_have_seen_anything():
    """Early in a window both counters are 0 and `0 < 0` is false. The `rseen > 0` clause makes
    that an explicit "wait for the hub to accumulate evidence" rather than a comparison that
    happens not to be true yet."""
    assert vo.production_hit(0, 0) is False
    assert vo.production_hit(3, 10) is True
    assert vo.production_hit(10, 10) is False, "equal is not lagging"


def test_evaluate_production_passes_when_the_victim_lags():
    assert vo.evaluate_production(4, 3, 10, 10, 10) == (True, "")


# ══ smoke-liveness ═════════════════════════════════════════════════════════

def test_validator_address_guard():
    assert vo.evaluate_validator_addresses(["a", "b", "c", "d"]) == (True, "")
    ok, msg = vo.evaluate_validator_addresses(["a", "b"])
    assert ok is False and "expected 4 validator addresses, got 2" in msg
    assert vo.evaluate_validator_addresses([])[0] is False
    assert vo.evaluate_validator_addresses(None)[0] is False


def test_cycle_gaps_match_the_bash_spectrum():
    """`case-liveness.sh:127-130` at the default 32-block interval: 97 (three boundaries), 33
    (one), 5 (none), 33 (one, on a node that just rejoined). The four are the case."""
    assert [vo.cycle_gap(32, e, x) for _, e, x in vo.LIVENESS_CYCLES] == [97, 33, 5, 33]
    assert [i for i, _, _ in vo.LIVENESS_CYCLES] == [3, 2, 1, 3]


def test_only_ONE_of_the_four_cycles_stays_under_the_re_jump_gate():
    """FLAGGED, NOT RETUNED — and pinned so the next reader is contradicted by a test rather than
    by a live run. The gate is `min(1024, interval)` = the interval itself, so cycle 1 (`3*I+1`,
    deliberately deep) AND cycles 2 and 4 (`I+1`, over by ONE BLOCK) all re-jump; only the
    within-epoch cycle walks. "Single epoch-boundary cross" names a boundary, not the jump gate,
    and the two happen to be the same number."""
    assert [vo.cycle_walks(32, e, x) for _, e, x in vo.LIVENESS_CYCLES] == [False, False,
                                                                           True, False]
    # …and it is the gap-vs-interval relation, not the number 32: at a 64-block interval the same
    # three cycles are still over it, by the same construction.
    assert [vo.cycle_walks(64, e, x) for _, e, x in vo.LIVENESS_CYCLES] == [False, False,
                                                                           True, False]
    assert vo.rejump_threshold(32) == 32 and vo.rejump_threshold(2048) == 1024


# ── the SIGNING half ───────────────────────────────────────────────────────

#: The line as the node renders it. `?epoch` is a `#[derive(Debug)]` newtype, so `Epoch(N)`; the
#: bare-`u64` form is here because a future `%epoch` would emit it and the reader accepts both.
PROMOTED = ("2026-08-01T00:00:00Z  INFO promoted to Signer in-process: per-epoch BFT engine "
            "started epoch=Epoch({e})")
PROMOTED_BARE = "INFO promoted to Signer in-process: per-epoch BFT engine started epoch={e}"


def test_the_promote_reader_finds_the_epoch_in_BOTH_renderings_and_counts_repeats():
    log = "\n".join([PROMOTED.format(e=4), PROMOTED_BARE.format(e=4), PROMOTED.format(e=5),
                     "INFO signer spawn deferred — E-1 boundary block not yet in marshal"])
    assert vo.promoted_epoch_counts(log) == {4: 2, 5: 1}
    assert vo.defer_count(log) == 1


def test_the_promote_reader_is_not_fooled_by_the_words_per_epoch_in_the_message():
    """The message itself contains `per-epoch BFT engine started`. A pattern with an OPTIONAL
    separator would take that `epoch`, walk forward to the first number on the line, and report
    an epoch nobody logged — silently, and in the direction that PASSES the gate."""
    assert vo.promoted_epoch_counts("promoted to Signer in-process: per-epoch BFT engine "
                                    "started 12345") == {}


def test_an_UNSTRIPPED_log_reads_as_no_promotion_at_all():
    """WHY `ctx.logs_all` STRIPS, stated as a test. The node writes SGR escapes INSIDE its
    `key=value` pairs, so the raw line carries `epoch<ESC>=<ESC>Epoch(4)` and the epoch never
    parses. On a gate that WAITS for a line to appear, a silent reader is indistinguishable from a
    member that never promoted — it would fail every healthy run, loudly and for the wrong
    reason."""
    raw = ("INFO promoted to Signer in-process: per-epoch BFT engine started "
           "epoch\x1b[0m\x1b[2m=\x1b[0mEpoch(4)")
    assert vo.promoted_epoch_counts(raw) == {}
    assert vo.promoted_epoch_counts(re.sub(r"\x1b\[[0-9;]*m", "", raw)) == {4: 1}


#: `(before, after, min_epoch) -> signing?`, one row per decision.
_LIVENESS_SIGNING = [
    ({1: 1}, {1: 1, 5: 1}, 4, True,
     "a_promotion_for_the_live_epoch_is_the_gate_passing"),
    ({1: 1}, {1: 1, 4: 1}, 4, True,
     "the_floor_epoch_ITSELF_counts__the_chain_only_provably_reached_it"),
    # THE DEFECT. Height says rejoined, the log says the engine never spawned.
    ({1: 1}, {1: 1}, 4, False,
     "height_aligned_with_no_new_promotion_is_NOT_signing"),
    # THE FALSE PASS A BARE DELTA WOULD TAKE: a restarted victim reconciles its persisted tail
    # first and can promote for the epoch it was killed in, three boundaries behind the committee.
    ({1: 1}, {1: 2}, 4, False,
     "a_promotion_for_a_STALE_epoch_is_not_signing_on_the_live_one"),
    # THE FALSE PASS AN ABSOLUTE GREP WOULD TAKE: the container log persists across
    # `compose_stop`/`start`, so the pre-stop promotion for the live epoch is still in there.
    ({5: 1}, {5: 1}, 4, False,
     "the_PRE_STOP_promotion_survives_the_restart_and_must_not_count"),
    ({5: 1}, {5: 2}, 4, True,
     "…but_promoting_for_that_same_epoch_AGAIN_after_the_restart_does"),
    ({}, {}, 0, False, "an_empty_log_is_never_a_promotion__not_even_at_floor_zero"),
]


@pytest.mark.parametrize("before,after,floor,expected",
                         [r[:4] for r in _LIVENESS_SIGNING],
                         ids=[r[4] for r in _LIVENESS_SIGNING])
def test_liveness_signing(before, after, floor, expected):
    assert vo.liveness_signing(before, after, floor) is expected


def test_the_signing_failure_tells_the_two_stale_shapes_apart():
    """Never promoted at all vs promoted for the wrong epoch are different faults with different
    next steps, so they do not share a sentence."""
    ok, never = vo.evaluate_liveness_signing({1: 1}, {1: 1}, 4, "validator-3", "", 197)
    assert ok is False
    assert "no 'promoted to Signer in-process' line appeared at all" in never
    assert "reached the floor pre+gap=197" in never

    ok, stale = vo.evaluate_liveness_signing({1: 1}, {1: 2}, 4, "validator-3", "")
    assert ok is False and "promoted only for epoch(s) [1] — BELOW the floor" in stale
    assert vo.evaluate_liveness_signing({1: 1}, {1: 1, 4: 1}, 4, "validator-3", "") == (True, "")


def test_the_signing_failure_reports_the_DEFER_count_as_the_next_place_to_look():
    """The one line that says WHICH of the eight spawn conditions is holding. It is a diagnostic
    and never the gate: one or two defers during ordinary backfill are normal and self-heal, so
    "no defers" is neither necessary nor sufficient for "is signing"."""
    logs = "\n".join(["INFO signer spawn deferred — E-1 boundary block not yet in marshal"] * 3)
    _, msg = vo.evaluate_liveness_signing({}, {}, 4, "validator-3", logs)
    assert "'signer spawn deferred' 3 time(s)" in msg
    assert "epoch_manager.rs:734-749" in msg
    # …and it says out loud that the budget is not the lever, because that is the reflex it has
    # to survive: this gate fails by TIMING OUT, which always looks like a budget.
    assert "NOT a budget to raise blindly" in msg


def test_the_signing_budget_is_one_whole_epoch_plus_slack():
    """Derived from the worst HONEST wait: a re-jumped member that misses boundary seeding
    promotes at the next epoch boundary, at most `interval` blocks = seconds away at 1 blk/s."""
    assert vo.liveness_signing_budget(32) == 32 + vo.LIVENESS_SIGNING_SLACK_S
    assert vo.liveness_signing_budget(64) == 64 + vo.LIVENESS_SIGNING_SLACK_S


def test_the_height_failure_names_the_victims_own_catch_up_and_nothing_else():
    msg = vo.liveness_not_rejoined_message("validator-3", 0, "0x87|0xb", "0xf9|0xa", 197)
    assert "did not rejoin" in msg and "floor=pre+gap=197" in msg
    assert "the SIGNING half was never reached" in msg
    assert "NOT SIGNING" not in msg


def test_the_epoch_floor_is_the_one_the_chain_provably_reached():
    """`epoch_of(pre + gap)`, the same anchor the HEIGHT floor uses — step (1) hard-asserts
    `wait_finalized_ge(pre + gap)` before the victim is restarted at all."""
    # Cycle 1 on the default stack: pre=100, gap=97, activation=64, interval=32.
    assert vo.epoch_of(197, 32, 64) == 4
    assert vo.epoch_of(64, 32, 64) == 0 and vo.epoch_of(95, 32, 64) == 0
    assert vo.epoch_of(96, 32, 64) == 1


#: `(hub, victim, peers, floor) -> rejoined?`, one row per decision, id per original assertion.
_LIVENESS_REJOIN = [
    # THE GATE HAS THREE HALVES now. The peer count is not a diagnostic: the epoch walk rides the
    # consensus plane but the BLOCKS arrive over devp2p, so a victim matching the hub with zero
    # peers is reporting a head it did not sync.
    ("0x64|0x100", "0x64|0x100", 2, 90, True,
     "liveness_rejoin_needs_BOTH_alignment_and_a_reth_peer__with_a_peer"),
    ("0x64|0x100", "0x64|0x100", 0, 90, False,
     "liveness_rejoin_needs_BOTH_alignment_and_a_reth_peer__zero_peers"),
    # THE FIRST REGRESSION, and this was the tightest budget in the suite: 120s for a victim that
    # in cycle 1 has to walk three epoch boundaries. A rejoining node is BEHIND the hub for most
    # of that walk by construction, so demanding one byte-identical `"height|hash"` from two
    # non-atomic reads of two moving tips made the pass a coincidence. Ragged-but-above-the-floor
    # still passes.
    ("0x64|0x100", "0x63|0x99", 3, 90, True,
     "a_victim_still_a_few_blocks_behind_the_hub_has_rejoined__one_block"),
    ("0x64|0x100", "0x5a|0x90", 1, 80, True,
     "a_victim_still_a_few_blocks_behind_the_hub_has_rejoined__ten_blocks"),
    # The fork half, at both shapes: same height as the hub on a different block, and a ragged
    # height on a different block. A height-only compare — the naive way to allow ragged heights
    # — would call the second one rejoined.
    ("0x64|0x100", "0x64|0xDEAD", 3, 90, False,
     "a_victim_on_a_different_chain_is_not_a_rejoin__same_height"),
    ("0x64|0x100", "0x63|0xDEAD", 3, 90, False,
     "a_victim_on_a_different_chain_is_not_a_rejoin__ragged_height"),
    # A height the hub does not hold reads `"null"`, which never matches a real hash — a retry,
    # not an agreement.
    ("0x64|0x100", "0x7d1|0x2001", 3, 90, False,
     "a_victim_ahead_of_a_hub_that_cannot_confirm_it_is_not_a_rejoin"),
    # §2.4 item 5. `null|null` on both sides is byte-identical, so without the explicit head
    # guard an all-down poll would report a perfect rejoin. It has to survive the move to
    # same-height identity, where an unreachable reading would otherwise be just another ragged
    # one.
    ("null|null", "null|null", 4, 90, False,
     "two_unreachable_nodes_never_read_as_a_rejoin__both_sides"),
    ("0x64|0x100", "null|null", 4, 90, False,
     "two_unreachable_nodes_never_read_as_a_rejoin__victim_only"),
    # An unparseable peer count is not a peer. Coercing it to a number would be inventing data
    # on the one read that is not a chain height.
    ("0x64|0x100", "0x64|0x100", "", 90, False,
     "liveness_rejoin_tolerates_an_unreadable_peer_count__empty"),
    ("0x64|0x100", "0x64|0x100", None, 90, False,
     "liveness_rejoin_tolerates_an_unreadable_peer_count__none"),
    # ── THE SECOND REGRESSION: the floor. ────────────────────────────────────────────────────
    # A LIVE RUN, cycle 1: the victim came back on the hub's own chain at 135 while the hub was
    # at 249 and the gate said "rejoined". Cycle 2 then stopped a second validator with the first
    # still ~114 blocks behind — 2 of 4 signers, a correct BFT stall, a failed case. Same-height
    # identity is satisfied by any node on the right chain, no matter how far back; only the
    # floor rejects this, and the floor has to be what the CHAIN reached (`pre + gap` = 197),
    # never the victim's own pre-stop height.
    ("0xf9|0x249", "0x87|0x135", 3, 197, False,
     "a_victim_on_the_right_chain_but_114_blocks_behind_the_hub_has_NOT_rejoined"),
    # The same reading, judged against the OLD floor — the victim's `pre`, which it clears on its
    # persisted tail the moment it restarts. This row IS the mutation witness: it is the exact
    # input the case saw, and it passes iff the floor is put back the way it was.
    ("0xf9|0x249", "0x87|0x135", 3, 100, True,
     "the_old_pre_floor_is_what_let_that_victim_through__mutation_witness"),
    # The floor applies PER READER, so a HUB below it is not a rejoin either — the victim cannot
    # be past a height the chain has not got back to.
    ("0x87|0x135", "0x87|0x135", 3, 197, False,
     "the_floor_is_checked_on_the_hub_too_not_only_the_victim"),
]


@pytest.mark.parametrize("hub,victim,peers,floor,expected",
                         [r[:5] for r in _LIVENESS_REJOIN],
                         ids=[r[5] for r in _LIVENESS_REJOIN])
def test_liveness_rejoined(hub, victim, peers, floor, expected):
    assert vo.liveness_rejoined(hub, victim, peers, floor, _chain) is expected


# ══ smoke-byzantine ════════════════════════════════════════════════════════

def test_jail_is_asserted_on_the_status_byte():
    assert vo.evaluate_jailed("3") == (True, "")
    assert vo.evaluate_jailed(" 3 ") == (True, "")
    ok, msg = vo.evaluate_jailed("2")
    assert ok is False and "status=2" in msg


def test_an_EMPTY_status_fails_and_says_the_read_failed():
    """An empty answer is a failed RPC, not "not jailed". It fails either way, and the message is
    what stops an operator debugging the slasher when the RPC was the thing that did not answer."""
    ok, msg = vo.evaluate_jailed("")
    assert ok is False and "read EMPTY" in msg
    assert vo.evaluate_jailed(None)[0] is False


def test_post_jail_liveness_is_a_second_assertion():
    """THE NEGATIVE half of `smoke-byzantine`. A post-jail committee-drop epoch-boundary wedge is
    a real DPoS failure class, and the pre-jail converge structurally cannot see it."""
    assert vo.evaluate_post_jail_liveness(True, 500) == (True, "")
    ok, msg = vo.evaluate_post_jail_liveness(False, 500)
    assert ok is False and "chain stalled after jail" in msg and "500" in msg


def test_marker_grep_is_case_insensitive_and_substring():
    """`grep -iE "BYZANTINE|equivocat|…"`. `equivocat` is a stem on purpose — it has to match
    both `equivocation` and `equivocating`."""
    logs = "INFO node is Equivocating now\nINFO nothing to see\nWARN cannot sign: no local share"
    hit = vo.grep_markers(logs, vo.BYZ_EQUIVOCATOR_MARKERS)
    assert "Equivocating" in hit and "cannot sign" in hit
    assert "nothing to see" not in hit
    assert vo.grep_markers("", vo.BYZ_SLASHER_MARKERS) == ""


# ══ smoke-cert-catchup: the two negatives, driven both ways ════════════════

def test_the_old_fatal_gate_passes_only_when_the_count_did_not_move():
    assert vo.evaluate_no_old_fatal(4, 4, "validator-2", "park") == (True, "")


def test_the_old_fatal_gate_FAILS_when_the_line_IS_present():
    """THE ABSENCE-GREP, DRIVEN. A live green run can never walk this branch: producing it needs
    an unpatched binary. The delta shape is what makes it possible at all — the container's log
    persists across the stop/start, so history is not a regression."""
    ok, msg = vo.evaluate_no_old_fatal(4, 5, "validator-2", "park")
    assert ok is False
    assert vo.OLD_FATAL in msg and "the fix regressed" in msg


def test_the_consensus_exit_gate_is_a_SEPARATE_regression():
    """Not redundant with the one above: the old fatal is the ERROR the executor logged on the
    way out, this is the line the node logs when the ack is dropped and the marshal cancels. A
    regression that skipped the ERROR and still exited passes the first and fails this."""
    assert vo.evaluate_no_consensus_exit(0, 0, "validator-2", "park") == (True, "")
    ok, msg = vo.evaluate_no_consensus_exit(0, 1, "validator-2", "park")
    assert ok is False
    assert vo.EXIT_LOG in msg and "instead of parking" in msg


def test_a_victim_that_never_parked_FAILS_the_gate_even_with_both_negatives_clean():
    """THE POSITIVE CONTROL, and the reason the two negatives above are not the case. A path that
    never ran logs neither forbidden line, so both absence gates pass perfectly — and the run
    would be green having exercised nothing."""
    assert vo.evaluate_no_old_fatal(0, 0, "v", "park")[0] is True
    assert vo.evaluate_no_consensus_exit(0, 0, "v", "park")[0] is True
    ok, msg, parked = vo.evaluate_park_exercised(0, 0, 0, "validator-2", "park", 64)
    assert ok is False and parked == 0
    assert "NOT exercised" in msg and "FALSE PASS" in msg
    assert "re_jump_threshold=64" in msg, "the remedy names the bound the gap must stay under"


def test_the_park_gate_passes_and_reports_how_many_blocks_parked():
    ok, msg, parked = vo.evaluate_park_exercised(4, 7, 812, "validator-2", "park", 64)
    assert ok is True and msg == "" and parked == 3


def test_rejump_threshold_and_deep_gap():
    """`min(interval, 1024)` — at the case's 64-block interval the pure-park window is 64, and
    the 28-block gap sits below it so the derive-walk is not stolen by a re-jump."""
    assert vo.rejump_threshold(64) == 64
    assert vo.rejump_threshold(128) == 128
    assert vo.rejump_threshold(4096) == vo.JUMP_THRESHOLD
    assert vo.CATCHUP_GAP < vo.rejump_threshold(vo.CATCHUP_EPOCH_INTERVAL)
    #: The gap's real bound is the CEILING, not the threshold. A future "just deepen the gap" edit
    #: that sails past it loses the derive-walk to `maybe_re_jump` and fails the park gate for a
    #: reason unrelated to the park.
    assert (vo.CATCHUP_GAP <= vo.CATCHUP_GAP_CEILING
            < vo.rejump_threshold(vo.CATCHUP_EPOCH_INTERVAL))
    assert vo.deep_gap(64) == 160, "2*interval + interval/2, integer division as bash's is"
    assert vo.deep_gap(64) > vo.rejump_threshold(64), "the deep cycle must CROSS the threshold"


def test_the_effective_gap_is_the_gap_PLUS_boot_PLUS_stall():
    """What `maybe_re_jump` sees is never what the case asked for. At the configuration that was
    measured to park — 40 blocks of gap, 1000 ms of added RTT — the model reproduces the observed
    ~57, which is also why 40 had to go: 57 is past the 64-block threshold's ceiling."""
    #: THE ANCHOR OF THE WHOLE MODEL, and the only number in it that was measured rather than
    #: derived: gap 40 + boot 10 + stall 7. It is what forced `rounds` to be priced on the ACTUAL
    #: backlog (ceil(50/20) = 3 → stall 7) instead of a full-threshold one (ceil(64/20) = 4 →
    #: stall 9). Any change to the model has to keep reproducing it.
    assert vo.effective_catchup_gap(40, 1000) == 57
    #: The gap the case now runs, and the headroom it buys.
    assert vo.effective_catchup_gap(vo.CATCHUP_GAP, 1000) == 43
    #: The default delay, three times the original RTT: still clear of the threshold, and the
    #: configuration that made the case stop flaking (4/4 live bring-ups parked, 0/4 re-jumped).
    assert vo.effective_catchup_gap(vo.CATCHUP_GAP, 3000) == 53 < vo.rejump_threshold(64)
    #: The stall is what the old ceiling forgot, and it is the whole difference: with no delay the
    #: victim only pays its boot drift.
    assert vo.effective_catchup_gap(40, 0) == 50
    #: Monotone in both inputs — a deeper gap and a longer delay each cost more.
    assert vo.effective_catchup_gap(60, 1000) > vo.effective_catchup_gap(40, 1000)
    assert vo.effective_catchup_gap(40, 2000) > vo.effective_catchup_gap(40, 1000)


def test_the_gap_ceiling_is_DERIVED_and_the_old_hand_set_52_FAILS_it():
    """THE CONSTANT THAT WAS WRONG. 52 was derived as `GAP + boot` with NO stall term, so it
    declared legal a gap whose effective size (71) was past the 64-block threshold the case then
    ran on — which is the run-2 failure exactly: park warns 0 → 0, re-jump landings 1 → 2.

    `catchup_gap_ceiling` prices the stall, and at that same interval it answers 46, i.e. it
    REJECTS 52. It is also a function of the interval and the delay rather than a literal, so the
    bound moves with the geometry instead of being re-derived by hand every time one changes."""
    assert vo.catchup_gap_ceiling(64, 1000) == 46
    assert vo.effective_catchup_gap(52, 1000) == 71 > vo.rejump_threshold(64)
    assert 52 > vo.catchup_gap_ceiling(64, 1000), "the old constant must not survive the formula"

    #: THE SECOND CONSTANT, AND THE HONEST READING OF IT. `CATCHUP_GAP` was 40. 40 is NOT over
    #: this ceiling — it is 6 blocks under it, and `effective_catchup_gap(40)` = 57 is 7 blocks
    #: under the threshold. It still failed live: park warns 0 → 0, re-jump landings 1 → 2, on a
    #: re-run of the very config that had parked four times. So the model UNDER-prices the real
    #: stall, and a gap that merely fits under the ceiling is not safe — it has to fit with room.
    #: That is what the cut to 28 buys, and pinning the two margins is what stops the next person
    #: reading "40 <= 46" as permission. (Both are delay-1000 numbers; see the default below.)
    #: ── THE DEFAULT, AND THE MARGIN AT IT ────────────────────────────────────────────────────
    #: `CATCHUP_GAP_CEILING` follows the DEFAULT delay, which is 3000. The headroom there is 4
    #: blocks, not the 18 that 1000 ms gave, and `effective(28, 3000)` = 53 sits 11 under the
    #: threshold on the first pass and 59/5 on the second — inside the band the model calls
    #: marginal. Pinned as 4 on purpose: it was chosen on four green live bring-ups, not by
    #: rounding, and anyone widening this test should read `CATCHUP_GAP`'s note first.
    assert vo.CATCHUP_NETEM_DELAY_MS == 3000
    assert vo.CATCHUP_GAP_CEILING == vo.catchup_gap_ceiling(vo.CATCHUP_EPOCH_INTERVAL) == 32
    assert vo.CATCHUP_GAP_CEILING - vo.CATCHUP_GAP == 4
    assert vo.effective_catchup_gap(vo.CATCHUP_GAP) == 53 < vo.rejump_threshold(64)

    #: ── THE DELAY-1000 HISTORY, PINNED EXPLICITLY ────────────────────────────────────────────
    #: These are the MEASURED numbers the model is anchored on and they must not drift with the
    #: default. Every one of them names its delay rather than inheriting it.
    assert vo.catchup_gap_ceiling(64, 1000) == 46
    assert vo.catchup_gap_ceiling(64, 1000) - 40 == 6, "the margin run 2 spent"
    assert vo.rejump_threshold(64) - vo.effective_catchup_gap(40, 1000) == 7
    assert vo.catchup_gap_ceiling(64, 1000) - vo.CATCHUP_GAP == 18

    #: …AND THE GUARD MUST NOT FORBID THE DEFAULT. The threshold-priced `rounds` this function
    #: used to carry put (28, 3000) at a stall of 27 and a ceiling of 27, so the case refused to
    #: start at its own gap — the configuration that is now the default.
    assert vo.catchup_gap_ceiling(64, 3000) == 32 >= vo.CATCHUP_GAP

    #: Raising the interval WOULD lift the ceiling — that is the obvious alternative to shrinking
    #: the gap, and it is why `CATCHUP_EPOCH_INTERVAL` carries a do-not-raise warning: 128 lifts
    #: this to 104 and does not boot.
    assert vo.catchup_gap_ceiling(128, 1000) == 104

    #: A longer delay BUYS parks and COSTS ceiling — the two levers pull against each other, which
    #: is why the delay is not the answer to a gap that no longer fits.
    assert vo.catchup_gap_ceiling(64, 2000) < vo.catchup_gap_ceiling(64, 1000)
    #: …and an unshaped victim pays only its boot drift: the largest gap under 64 with zero stall
    #: is 53 (`53 + 10 = 63`).
    assert vo.catchup_gap_ceiling(64, 0) == 53
    #: Never negative, however absurd the delay — the case's own guard reports the gap, not a
    #: nonsense bound.
    assert vo.catchup_gap_ceiling(64, 10 ** 6) == 0


def test_catchup_rejoin_requires_progress_past_the_floor():
    """A floor is what makes this a CATCH-UP check. A victim that came back and sat on its
    persisted tail would eventually match a stalled hub, and the premise of the case is that the
    hub moved `gap` blocks while the victim was down."""
    assert vo.catchup_rejoined("0x100|0x256", "0x100|0x256", 200) is True
    assert vo.catchup_rejoined("0x100|0x256", "0x100|0x256", 256) is False
    assert vo.catchup_rejoined("null|null", "null|null", 0) is False


#: `(hub, victim, floor) -> rejoined?` against the injected hub, one row per original assertion.
#: The three-argument shape (the DEFAULT `producer_hash_at`) stays in its own test above.
#: The floor is `pre + gap`, not `pre` — see the two rows at the bottom.
_CATCHUP_REJOIN = [
    # Byte-identity used to make "the hub cleared the floor" equivalent to "the victim did".
    # Ragged heights break that equivalence, and the victim's own progress IS the assertion —
    # the hub never stopped moving.
    ("0x110|0x272", "0x100|0x256", 255, True,
     "the_catchup_floor_applies_to_the_victim_not_only_the_hub__victim_past_floor"),
    ("0x110|0x272", "0x100|0x256", 256, False,
     "the_catchup_floor_applies_to_the_victim_not_only_the_hub__victim_at_floor"),
    # THE REGRESSION. The victim is walking a gap of over 2*EPOCH_INTERVAL on a chain producing
    # a block a second, so it joins the hub's CHAIN long before it reaches the hub's TIP. The old
    # equality leg required the two moving tips to coincide inside a 240s budget.
    ("0x120|0x288", "0x101|0x257", 256, True,
     "a_victim_partway_through_the_gap_has_rejoined_the_chain"),
    # The fork half, at both shapes. The floor leg alone would bless either of these: both
    # victims are past it, and both are on the wrong chain.
    ("0x120|0x288", "0x120|0xDEAD", 256, False,
     "a_victim_that_caught_up_onto_a_different_chain_still_fails__same_height"),
    ("0x120|0x288", "0x101|0xDEAD", 256, False,
     "a_victim_that_caught_up_onto_a_different_chain_still_fails__ragged_height"),
    # A height the hub does not hold reads `"null"`, and one unreachable side must be rejected
    # on its head rather than skipped as another ragged reading.
    ("0x120|0x288", "0x7d1|0x2001", 256, False,
     "a_catchup_reading_the_hub_cannot_confirm_fails__victim_ahead_of_the_hub"),
    ("0x120|0x288", "null|null", 256, False,
     "a_catchup_reading_the_hub_cannot_confirm_fails__victim_unreachable"),
    # ── THE FLOOR IS `pre + gap`, NOT `pre`. ─────────────────────────────────────────────────
    # pre=256, gap=40, so the chain provably reached 296 while the victim was down. A victim at
    # 260 is on the right chain and past its OWN pre-stop height — the old floor — while still 60
    # blocks short of where the hub was when it restarted. The second row is the mutation
    # witness: the same reading passes iff the floor is put back to `pre`.
    ("0x180|0x384", "0x104|0x260", 296, False,
     "a_victim_four_blocks_into_a_forty_block_gap_has_not_caught_up"),
    ("0x180|0x384", "0x104|0x260", 256, True,
     "the_old_pre_floor_is_what_let_that_victim_through__mutation_witness"),
]


@pytest.mark.parametrize("hub,victim,floor,expected",
                         [r[:4] for r in _CATCHUP_REJOIN],
                         ids=[r[4] for r in _CATCHUP_REJOIN])
def test_catchup_rejoined(hub, victim, floor, expected):
    assert vo.catchup_rejoined(hub, victim, floor, _chain) is expected


def test_the_gauge_peak_ignores_the_na_sentinel():
    """`"na"` is a mid-restart unreadable endpoint, and a 0 there is a REAL value meaning nothing
    parked. Coercing the sentinel would make the two indistinguishable in the one number the case
    prints alongside its verdict."""
    assert vo.gauge_peak(0, "na") == 0
    assert vo.gauge_peak(5, "na") == 5
    assert vo.gauge_peak(5, "812") == 812
    assert vo.gauge_peak(812, "3") == 812
    assert vo.gauge_peak(0, "") == 0


def test_the_deep_cycle_only_NOTES_a_missing_rejump():
    """Best-effort by design: a re-jump can fast-forward past the derive-walk, so a gate here
    would make the optional cycle flaky on a fast host."""
    assert "EXERCISED" in vo.rejump_note(0, 1, "v", "rejump-deep", 64)
    assert "NOTE: re-jump landing not observed" in vo.rejump_note(2, 2, "v", "rejump-deep", 64)


# ══ smoke-vrf-dkg-restart-midwindow ════════════════════════════════════════

def test_the_epoch_field_grep_is_two_greps_and_the_field_is_right_anchored():
    """`grep "<message>" | grep -E "epoch=2( |,|$)"`. Both halves, and the anchor: without it
    `epoch=2` also matches `epoch=20`, and the case would credit a resume from the wrong
    ceremony."""
    logs = "\n".join([
        f"INFO {vo.RESUME_LINE} epoch=2 me=3",
        f"INFO {vo.RESUME_LINE} epoch=20 me=3",
        f"INFO {vo.RESUME_LINE} epoch=3, me=3",
        "INFO something else epoch=2",
    ])
    hits = vo.epoch_field_lines(logs, vo.RESUME_LINE, 2)
    assert hits == [f"INFO {vo.RESUME_LINE} epoch=2 me=3"]


@pytest.mark.parametrize("tail", [" x", ",", ""])
def test_the_epoch_field_accepts_the_three_terminators_bash_accepts(tail):
    line = f"INFO {vo.SHARE_LINE} epoch=2{tail}"
    assert vo.epoch_field_lines(line, vo.SHARE_LINE, 2) == [line]


def test_an_ANSI_SPLIT_field_does_not_match_which_is_why_the_reader_strips():
    """§2.4 item 2, made concrete. The node writes escapes INSIDE the pair, so a RAW log line
    reads `epoch<ESC>=<ESC>2` and this grep — correctly — finds nothing. `ctx.logs_all` strips
    before this ever sees the text; a reader that forgot to would present a silent miss as a
    genuine "the resume never ran"."""
    raw = f"INFO {vo.RESUME_LINE} epoch\x1b[0m=\x1b[2m2 me=3"
    assert vo.epoch_field_lines(raw, vo.RESUME_LINE, 2) == []


def test_resume_and_share_are_two_distinct_failures():
    """Started-but-never-finished is a different bug from never-started (a resolver fetch or the
    settle gate did not complete), and it gets its own message."""
    ok, msg = vo.evaluate_resumed([], "validator-3")
    assert ok is False and "the journal+resume path never ran" in msg
    assert vo.evaluate_resumed(["a line"], "validator-3") == (True, "")
    ok, msg = vo.evaluate_share_computed([], "validator-3")
    assert ok is False and "did not finalize" in msg
    assert vo.evaluate_share_computed(["a line"], "validator-3") == (True, "")


def test_prev_randao_window_treats_a_MISSING_block_as_a_failure():
    """A block the victim cannot serve is a height it never derived — not a height to skip."""
    ok, msg = vo.evaluate_prev_randao_window([(256, "null", "0xaa")], "validator-3")
    assert ok is False and "256=missing-on-validator-3" in msg
    assert vo.evaluate_prev_randao_window([(256, "", "0xaa")], "validator-3")[0] is False


def test_prev_randao_window_FAILS_on_divergence_and_names_the_height():
    ok, msg = vo.evaluate_prev_randao_window(
        [(256, "0xaa", "0xaa"), (257, "0xbb", "0xcc")], "validator-3")
    assert ok is False and "257: validator-3=0xbb != validator-0=0xcc" in msg


def test_prev_randao_window_passes_when_every_height_agrees():
    rows = [(h, "0xaa", "0xaa") for h in range(256, 263)]
    assert vo.evaluate_prev_randao_window(rows, "validator-3") == (True, "")


def test_the_production_retry_waits_for_an_epoch_with_recorded_blocks():
    """`blocksInEpoch > 0` is load-bearing: an epoch with nothing recorded credits everyone 0, so
    "produced something" would be false for a healthy node purely because the read was early."""
    assert vo.credit_ready(10, 10) is True
    assert vo.credit_ready(0, 0) is False
    assert vo.credit_ready(-2, 10) is False
    assert vo.credit_ready(-1, 10) is False


def test_the_three_production_failure_branches_stay_three():
    """Three messages and that is not verbosity: a persistent -2 means the harness could not read
    the chain, a -1 means the committee is not what the case assumed, and blocksInEpoch==0 means
    it looked too early. Three different next steps."""
    ok, msg = vo.evaluate_production_readable(-2, -2, "validator-3")
    assert ok is False and "-2 RPC sentinel" in msg and "never be treated as a passing 0" in msg
    ok, msg = vo.evaluate_production_readable(-1, -1, "validator-3")
    assert ok is False and "not in committee[2]" in msg
    ok, msg = vo.evaluate_production_readable(5, 0, "validator-3")
    assert ok is False and "no epoch-2 blocks recorded" in msg
    assert vo.evaluate_production_readable(9, 10, "validator-3") == (True, "")


def test_produced_something_is_the_load_bearing_assertion_both_ways():
    """The negative control is what gives it teeth: a shareless victim cannot build a valid
    boundary proposal at all, so its credit stays 0 and this fires. Deliberately `> 0` and not a
    ratio — production credit is drawn by a stake-weighted lottery, so a healthy member's exact
    share over ONE epoch is a random variable and any fixed threshold would be invented."""
    ok, msg = vo.evaluate_produced_something(0, "validator-3")
    assert ok is False and "produced NOTHING" in msg and "the fix failed" in msg
    assert vo.evaluate_produced_something(7, "validator-3") == (True, "")
    ok, _ = vo.evaluate_produced_something("garbled", "validator-3")
    assert ok is False, "an unparseable read must not read as produced"


def test_slash_hits_needs_BOTH_a_marker_AND_the_victim_address():
    """Dropping the address filter fails on a slash against some other node; dropping the marker
    filter matches any line that mentions the address."""
    addr = "0xAbCdEf0123456789abcdef0123456789ABCDEF01"
    bare = addr[2:].lower()
    logs = "\n".join([
        f"INFO ValidatorSlashed validator={bare} amount=1",
        f"INFO some unrelated line about {bare}",
        "INFO ValidatorSlashed validator=deadbeef amount=1",
    ])
    hits = vo.slash_hits(logs, addr)
    assert len(hits) == 1 and "ValidatorSlashed" in hits[0] and bare in hits[0]


def test_every_slash_marker_is_looked_for():
    addr = "0x" + "ab" * 20
    for marker in vo.SLASH_MARKERS:
        assert vo.slash_hits(f"WARN {marker} who={'ab' * 20}", addr), marker


def test_no_slash_both_ways():
    assert vo.evaluate_no_slash([], "validator-3") == (True, "")
    ok, msg = vo.evaluate_no_slash(["WARN ValidatorSlashed …"], "validator-3")
    assert ok is False and "slashed despite recovering" in msg


def test_an_EMPTY_status_is_never_read_as_not_jailed():
    """review [225]: an empty-vs-"3" false-green would silently hide the very fault the case
    exists to catch. `""` must FAIL here, not fall into the `!= "3"` success arm."""
    ok, msg = vo.evaluate_not_jailed("", "validator-3")
    assert ok is False and "empty RPC result" in msg
    ok, msg = vo.evaluate_not_jailed(vo.STATUS_JAIL, "validator-3")
    assert ok is False and "is JAILED" in msg and "equivocation" in msg
    assert vo.evaluate_not_jailed("2", "validator-3") == (True, "")


def test_still_finalizing_needs_strict_growth():
    assert vo.evaluate_still_finalizing(100, 106) == (True, "")
    ok, msg = vo.evaluate_still_finalizing(100, 100)
    assert ok is False and "chain not finalizing after the boundary" in msg


def test_the_on_disk_probes_are_the_bash_find_lines():
    """The journal probe carries `-size +0c` (an empty file is not a journal); the share probe
    does NOT, matching bash — the question there is existence, and a victim that began writing
    the share has finalized."""
    j = vo.journal_probe(3, 2)
    s = vo.share_probe(3, 2)
    assert "/runtime/reth-data/v3" in j and 'beacon-dkgjournal-e2.bin' in j
    assert "-size +0c" in j and j.endswith("| grep -q .")
    assert "/runtime/reth-data/v3" in s and 'beacon-share-e2.bin' in s
    assert "-size +0c" not in s and s.endswith("| grep -q .")


def test_the_tuned_genesis_constants_match_the_bash_exports():
    """`case-vrf-dkg-restart-midwindow.sh` and `case-cert-catchup.sh:65`. The activation block is
    `2 * interval`, which is what keeps the migration anchor in absolute epoch 2. There were once
    three tuned knobs; the third went with the participation-floor jail that consumed it."""
    assert vo.MIDWINDOW_EPOCH_INTERVAL == 64
    assert vo.MIDWINDOW_ACTIVATION_BLOCK == 2 * vo.MIDWINDOW_EPOCH_INTERVAL
    #: cert-catchup's interval is NOT the midwindow one and must not be aliased to it: they agree
    #: on 64 for unrelated reasons. Raising it to 128 was tried live and the stack does not boot.
    assert vo.CATCHUP_EPOCH_INTERVAL == 64
