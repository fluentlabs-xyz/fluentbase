"""`cases/smoke/verdicts.py` — the four assertions' decision logic, driven BOTH WAYS.

WHY THIS FILE CARRIES THE WEIGHT. A live smoke run walks the PASS branch of every check and
nothing else: it runs against a healthy chain, so every failure message in `asserts.sh` is dead
code on a green run. `assert_vrf_boundary` had literally never executed before 2026-07-30 (a
`set -u` bug killed `assert_vrf` first, ee0cf6a9), which is what that looks like in practice. So
the FAIL direction of every verdict is tested here or it is not tested at all.

Each test names the property, not the function — "a reverted receipt fails" rather than
"evaluate_tx_receipts returns False" — because the property is what the port had to keep.
"""

from __future__ import annotations

import pytest

from dpos_harness.cases.smoke import verdicts

ZERO = "0x" + "0" * 64


def _mix(tag: str) -> str:
    return "0x" + tag * 64


# ══ smoke-tx ═══════════════════════════════════════════════════════════════

@pytest.mark.parametrize("status,ok", [
    ("0x1", True), ("1", True),                  # foundry has printed both spellings
    ("0x0", False), ("0", False),
    ("", False), (None, False),                  # an unreadable receipt is NOT a success
])
def test_receipt_status(status, ok):
    assert verdicts.receipt_status_ok(status) is ok


def test_a_reverted_receipt_fails_and_names_the_hash():
    """A reverted `approve` finalizes exactly like a successful one. The status byte is the only
    thing between this case and a green run over a chain whose EVM reverts everything."""
    ok, msg = verdicts.evaluate_tx_receipts([("0xaa", "0x1"), ("0xbb", "0x0")])
    assert not ok and "0xbb" in msg and "status=0x0" in msg


def test_both_receipts_must_be_present_and_ok():
    assert verdicts.evaluate_tx_receipts([("0xaa", "0x1"), ("0xbb", "1")])[0]


@pytest.mark.parametrize("raw,want", [
    ("12345 [1.234e4]", "12345"),                # cast's pretty-print — §2.4 item 9
    ("12345", "12345"),
    ("  22920 [2.292e4]  ", "22920"),
    ("", ""),
])
def test_first_token_strips_the_cast_annotation(raw, want):
    """Dropping this strip re-creates a silent parse corruption: an activation block read as
    `229200 [2.292e5]` once pinned the epoch to 0 and disabled all churn with no error."""
    assert verdicts.first_token(raw) == want


def test_tx_state_passes_only_when_both_halves_applied():
    assert verdicts.evaluate_tx_state(verdicts.TRANSFER_WEI, verdicts.ALLOWANCE)[0]


def test_a_wrong_balance_delta_fails():
    ok, msg = verdicts.evaluate_tx_state(verdicts.TRANSFER_WEI - 1, verdicts.ALLOWANCE)
    assert not ok and "balance delta" in msg


def test_an_unapplied_allowance_fails_even_with_a_correct_transfer():
    """The half that proves the EVM EXECUTED. A value transfer touches no CALL and no SSTORE, so
    a chain that finalizes transfers and executes nothing else passes the delta check alone."""
    ok, msg = verdicts.evaluate_tx_state(verdicts.TRANSFER_WEI, 0)
    assert not ok and "SSTORE" in msg


def test_a_zero_delta_is_not_silently_acceptable():
    """The shape an unreachable-RPC balance read would produce if it were coerced to 0 on both
    sides. It must fail here even though `0 - 0` looks like a consistent reading."""
    assert not verdicts.evaluate_tx_state(0, verdicts.ALLOWANCE)[0]


# ══ smoke-epoch ════════════════════════════════════════════════════════════

@pytest.mark.parametrize("prev,interval,cross,want", [
    (70, 32, 1, 128),      # anchor in epoch 2 -> the start of epoch 4 (crosses 3->4 and 2->3)
    (64, 32, 1, 128),      # exactly on a boundary: still the epoch AFTER the next one
    (95, 32, 1, 128),      # last block of the same epoch -> same target
    (96, 32, 1, 160),      # first block of the next epoch -> one further
    (70, 32, 2, 160),
    (70, 64, 1, 192),
])
def test_epoch_target(prev, interval, cross, want):
    """`((PREV/INTERVAL) + MIN_CROSS + 1) * INTERVAL`. The `+1` is not slack: `PREV/INTERVAL` is
    the epoch the anchor sits IN, so targeting its own start would cross nothing."""
    assert verdicts.epoch_target(prev, interval, cross) == want


def test_epoch_target_refuses_a_zero_interval():
    """A zero interval is the shape a failed `EPOCH_INTERVAL` read takes. Dividing by it must
    raise, not produce a target of 0 that every live chain satisfies."""
    with pytest.raises(ValueError):
        verdicts.epoch_target(70, 0, 1)


@pytest.mark.parametrize("comm,ok", [
    ("[0xaa, 0xbb]", True),
    ("[]", False),          # the on-chain empty-array rendering
    ("", False),            # an unreadable getter
    ("   ", False),
])
def test_committee_must_be_non_empty(comm, ok):
    """A boundary crossed with an EMPTY committee is a boundary that handed off to nobody, and
    the height check alone cannot see it."""
    assert verdicts.evaluate_committee(comm, 2)[0] is ok


@pytest.mark.parametrize("delta,ok", [
    (60, True), (45, True), (66, True),
    (44, False),            # too slow — view timeouts / a stalling chain
    (67, False), (350, False),   # too fast — the unpaced chain did ~350 blocks/min
    (0, False),             # frozen
])
def test_pacing_is_bounded_on_both_sides(delta, ok):
    """The UPPER bound is the half people delete. Without it a pacing REGRESSION (the unpaced
    chain) passes, and the 1 blk/s target is not asserted at all."""
    assert verdicts.evaluate_pacing(delta)[0] is ok


# ══ the beacon window ══════════════════════════════════════════════════════

NODES = ["validator-0", "validator-1", "validator-2", "validator-3", "full-node"]


def _rows(*per_height):
    return [(100 + i, list(v)) for i, v in enumerate(per_height)]


def test_a_good_window_passes_and_returns_its_values():
    rows = _rows([_mix("a")] * 5, [_mix("b")] * 5, [_mix("c")] * 5)
    ok, msg, mixes = verdicts.evaluate_beacon_window(NODES, rows)
    assert ok and msg == "" and mixes == [_mix("a"), _mix("b"), _mix("c")]


def test_a_missing_block_fails_rather_than_being_skipped():
    """`"null"` is a node that does not have the block. Treating it as "nothing to compare" would
    silently shrink the window to the nodes that answered."""
    rows = _rows([_mix("a")] * 4 + ["null"])
    ok, msg, _ = verdicts.evaluate_beacon_window(NODES, rows)
    assert not ok and "full-node has no mixHash" in msg


def test_an_empty_reading_fails_too():
    rows = _rows([_mix("a")] * 4 + [""])
    assert not verdicts.evaluate_beacon_window(NODES, rows)[0]


def test_a_zero_prev_randao_fails_and_names_the_node():
    """`0x00…0` is the `order.digest()` fallback / a stalled beacon — the exact thing this case
    exists to detect, and it is perfectly node-agreed."""
    rows = _rows([_mix("a")] * 2 + [ZERO] + [_mix("a")] * 2)
    ok, msg, _ = verdicts.evaluate_beacon_window(NODES, rows)
    assert not ok and "zero at block 100 on validator-2" in msg


def test_one_divergent_node_at_one_height_fails():
    """The safety failure. A validator-0-only probe cannot see it, and neither can a single-height
    one — which is why the compare is per-height across ALL five nodes."""
    rows = _rows([_mix("a")] * 5, [_mix("b")] * 4 + [_mix("d")])
    ok, msg, _ = verdicts.evaluate_beacon_window(NODES, rows)
    assert not ok and "disagree on prev_randao at block 101" in msg
    assert _mix("d") in msg and "full-node" in msg


def test_a_stuck_beacon_fails_the_variance_check():
    """A constant beacon is non-zero AND byte-identical across every node at every height — it
    passes every other check in this function. Only the distinctness check catches it."""
    rows = _rows([_mix("a")] * 5, [_mix("a")] * 5, [_mix("a")] * 5)
    ok, msg, _ = verdicts.evaluate_beacon_window(NODES, rows)
    assert not ok and "not varying" in msg and "only 1 distinct" in msg


def test_a_single_repeat_anywhere_in_the_window_fails():
    rows = _rows([_mix("a")] * 5, [_mix("b")] * 5, [_mix("a")] * 5)
    assert not verdicts.evaluate_beacon_window(NODES, rows)[0]


def test_the_label_reaches_the_message():
    """`assert_beacon_window` is called from three cases; without the label a failure does not say
    WHICH window broke."""
    rows = _rows([ZERO] * 5)
    _, msg, _ = verdicts.evaluate_beacon_window(NODES, rows, "epoch-boundary-160")
    assert msg.startswith("epoch-boundary-160 — ")


@pytest.mark.parametrize("fin,window,floor,want", [
    (136, 8, 128, 129),    # the ordinary case: the last 8 finalized blocks
    (130, 8, 128, 128),    # the window would reach below epoch-2 start -> clamped up
    (128, 8, 128, 128),
    (5, 8, 1, 1),          # fin <= window: start at block 1, as bash does
])
def test_window_lo_clamps_to_the_beacon_active_floor(fin, window, floor, want):
    """The floor is the half that matters. Blocks below `epoch_start(2)` carry the DIGEST fallback
    mixHash, which is neither zero nor node-divergent — it sails through every check in
    `evaluate_beacon_window` and turns the beacon assertion into a no-op."""
    assert verdicts.window_lo(fin, window, floor) == want


# ══ smoke-vrf ══════════════════════════════════════════════════════════════

def test_active_counts_pass_at_the_floor():
    counts = {f"validator-{i}": verdicts.MIN_ACTIVE_BLOCKS for i in range(4)}
    assert verdicts.evaluate_active_counts(counts)[0]


def test_a_validator_below_the_floor_fails_and_is_named():
    """Named so the caller can dump THAT node's log tail — bash's `docker compose logs --tail=80
    "$v"`. A message without the node is a message that costs a second run to act on."""
    counts = {"validator-0": 9, "validator-1": 2, "validator-2": 9, "validator-3": 9}
    ok, msg, culprit = verdicts.evaluate_active_counts(counts)
    assert not ok and culprit == "validator-1" and "only 2 times" in msg


def test_zero_active_lines_fail():
    assert not verdicts.evaluate_active_counts({"validator-0": 0})[0]


def test_active_growth_requires_a_strict_increase():
    before = {"validator-0": 5, "validator-1": 5}
    assert verdicts.evaluate_active_growth(before, {"validator-0": 6, "validator-1": 7})[0]


def test_a_frozen_active_count_fails_even_though_it_clears_the_floor():
    """The whole point of step 2b. A beacon that logged its five lines during warm-up and then
    dropped to the digest fallback keeps a FROZEN count — it satisfies the static `>= MIN_BLOCKS`
    floor for the rest of the run, so only growth can see it."""
    before = {"validator-0": 50, "validator-1": 50}
    ok, msg, culprit = verdicts.evaluate_active_growth(before, {"validator-0": 51,
                                                               "validator-1": 50})
    assert not ok and culprit == "validator-1" and "frozen at 50" in msg


def test_a_node_missing_from_the_after_read_fails_rather_than_being_ignored():
    """An absent second reading is `0`, which cannot be > the first — so a node that stopped
    answering fails instead of quietly leaving the growth set."""
    assert not verdicts.evaluate_active_growth({"validator-3": 5}, {})[0]


ACTIVE = verdicts.ACTIVE_LINE


def test_logged_randaos_are_parsed_lowercased_and_deduped():
    text = "\n".join([
        f"INFO {ACTIVE} round=Round{{1,2}} prev_randao={_mix('A')}",
        f"INFO {ACTIVE} round=Round{{1,3}} prev_randao={_mix('a')}",
        f"INFO {ACTIVE} round=Round{{1,4}} prev_randao={_mix('b')}",
        f"INFO something else prev_randao={_mix('c')}",
    ])
    assert verdicts.parse_logged_randaos(text) == [_mix("a"), _mix("b")]


def test_the_round_field_is_not_mistaken_for_a_value():
    """Extraction is "the only 32-byte hex on the line" — the `round` field is not 64 hex, which
    is what makes this format-agnostic across the text and JSON renderings."""
    text = f'{{"msg":"{ACTIVE}","round":12345,"prev_randao":"{_mix("e")}"}}'
    assert verdicts.parse_logged_randaos(text) == [_mix("e")]


def test_no_parsed_values_is_a_failure_not_an_empty_pass():
    """Without this gate the cross-check below iterates an empty set and reports every block as
    missing — or, written the other way round, reports success over no data at all."""
    assert not verdicts.evaluate_logged_randaos_present([])[0]
    assert verdicts.evaluate_logged_randaos_present([_mix("a")])[0]


def test_an_ansi_wrapped_line_still_parses_once_stripped():
    """§2.4 item 2. The node writes SGR escapes INSIDE its key=value pairs; `--no-color` does not
    remove them. The reader strips before calling this, and the point of the test is that the
    stripped form is what the parser is written against."""
    from dpos_harness.core.rpc import strip_ansi
    raw = f"\x1b[2mINFO\x1b[0m {ACTIVE} \x1b[2mprev_randao\x1b[0m=\x1b[1m{_mix('f')}\x1b[0m"
    assert verdicts.parse_logged_randaos(strip_ansi(raw)) == [_mix("f")]


@pytest.mark.parametrize("fin,want", [(136, 133), (5, 2), (4, 1), (1, 1)])
def test_logged_check_lo_anchors_on_the_recent_tail(fin, want):
    """Deliberately NOT step 1's window: that one starts at `epoch_start(2)` and can include the
    pre-DPoS sequencer prefix, whose mixHash is the digest fallback and was never logged."""
    assert verdicts.logged_check_lo(fin) == want


def test_every_recent_header_must_have_been_logged():
    logged = [_mix("a"), _mix("b")]
    assert verdicts.evaluate_logged_vs_onchain(logged, [(10, _mix("a")), (11, _mix("b"))])[0]


def test_an_unlogged_header_value_fails_and_names_the_height():
    """The header carries a value the deriver never computed on the assurance path."""
    ok, msg = verdicts.evaluate_logged_vs_onchain([_mix("a")],
                                                 [(10, _mix("a")), (11, _mix("z"))])
    assert not ok and f"11={_mix('z')}" in msg


def test_the_check_runs_header_to_log_and_not_the_reverse():
    """Direction matters. A logged value with no on-chain block is CORRECT — the active line fires
    on speculative notarization derives whose nullified rounds legitimately never canonicalize —
    so the reverse check would false-fail on a healthy chain."""
    extra = [_mix("a"), _mix("speculative"[0] * 1)]
    assert verdicts.evaluate_logged_vs_onchain(extra + [_mix("q")], [(10, _mix("a"))])[0]


def test_evm_prevrandao_must_equal_the_header():
    assert verdicts.evaluate_evm_prevrandao(_mix("a"), _mix("a"), 100)[0]
    assert verdicts.evaluate_evm_prevrandao(_mix("A"), _mix("a"), 100)[0]   # case-insensitive
    ok, msg = verdicts.evaluate_evm_prevrandao(_mix("a"), _mix("b"), 100)
    assert not ok and "did not reach EVM execution" in msg


def test_an_absent_beacon_metric_fails_before_any_delta_is_taken():
    """"" -> "" is not growth, and without this gate the delta check would silently pass on a node
    exporting no beacon metrics at all — the shape a metric RENAME takes."""
    assert not verdicts.evaluate_beacon_metrics_present("", "7")[0]
    assert not verdicts.evaluate_beacon_metrics_present("0", "")[0]
    assert not verdicts.evaluate_beacon_metrics_present(None, None)[0]
    assert verdicts.evaluate_beacon_metrics_present("0", "7")[0]


def test_a_zero_fallback_counter_is_present_not_absent():
    """`0` is the HEALTHY value of `beacon_digest_fallback`. A truthiness test here would call the
    healthy chain "metrics absent"."""
    assert verdicts.evaluate_beacon_metrics_present("0", "0")[0]


def test_beacon_metric_deltas():
    assert verdicts.evaluate_beacon_metrics_delta("0", "7", "0", "9")[0]


def test_a_growing_digest_fallback_fails():
    """Any growth means a block fell through to `order.digest()` on a chain that is supposed to
    have a verified seed for every block."""
    ok, msg = verdicts.evaluate_beacon_metrics_delta("0", "7", "1", "9")
    assert not ok and "beacon_digest_fallback grew" in msg


def test_a_flat_seed_active_fails():
    """Flat means the beacon stalled, or the metric is not wired — indistinguishable from here and
    both worth failing."""
    ok, msg = verdicts.evaluate_beacon_metrics_delta("0", "7", "0", "7")
    assert not ok and "did not grow" in msg


def test_a_vanished_second_reading_reads_as_zero_and_fails_the_growth_half():
    """bash's `${sa1:-0}`. The first pair is known present (gated above), so a missing SECOND
    reading is a node that stopped answering — 0 cannot exceed 7, so it fails."""
    assert not verdicts.evaluate_beacon_metrics_delta("0", "7", "", "")[0]


# ══ epoch geometry ═════════════════════════════════════════════════════════

def test_the_beacon_active_floor_is_epoch_2():
    """Not a tunable and not margin: epoch 1 is seedless, committee[2] DKGs during epoch 1, and
    PK_2 commits at the epoch-2 boundary. Sampling below this measures the digest fallback."""
    assert verdicts.beacon_active_epoch_start(64, 32) == 128


def test_the_probed_boundary_is_2_to_3():
    """0->1 and 1->2 are keyless / the bootstrap COMMIT. 2->3 is the first stable CARRY-FORWARD
    boundary, which is what the case claims to test."""
    assert verdicts.boundary_block(64, 32) == 160
    assert verdicts.boundary_block(128, 64) == 320


def test_the_boundary_window_straddles_the_boundary():
    """It has to straddle it: the property is continuity ACROSS the edge, so a window that starts
    after the boundary proves only that the new epoch works."""
    b = verdicts.boundary_block(64, 32)
    lo, hi = b - verdicts.BOUNDARY_HALF_WINDOW, b + verdicts.BOUNDARY_HALF_WINDOW
    assert lo < b < hi and (hi - lo) == 2 * verdicts.BOUNDARY_HALF_WINDOW
