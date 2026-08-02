"""Unit tests for the PURE layer of case_seed_continuity (no docker, no RPC).

The live driver is not exercised here; `conftest.py` blocks every docker argv outright.
What IS exercised is everything the verdict depends on: the offline fallback predictor, the
two-INFO-log parser, the nullified-gap detector, and the verdict trichotomy.
"""

from __future__ import annotations

import hashlib
from bisect import bisect_right

import pytest

from dpos_harness.cases.seed_continuity import (
    LEADER_DOMAIN,
    build_cum,
    evaluate_seed_continuity_case,
    fallback_leader_index,
    parse_view_leader_map,
    successors_of_nullified,
)


# ── the offline predictor ──────────────────────────────────────────────────────

def _pks(n):
    """n distinct 32-byte peer pubkeys, deliberately NOT in ascending order of index."""
    return [bytes([255 - i]) + bytes(31) for i in range(n)]


def test_predictor_matches_an_independent_recomputation():
    pks = sorted(_pks(7))
    cum, total = build_cum([1] * 7)
    for view in (1, 2, 17, 4096):
        seed = hashlib.sha256(b"".join([(3).to_bytes(8, "big")] + pks)).digest()
        rand = hashlib.sha256(LEADER_DOMAIN + seed + view.to_bytes(8, "big")).digest()
        want = bisect_right(cum, int.from_bytes(rand, "big") % total)
        assert fallback_leader_index(3, view, pks, cum, total) == want


def test_predictor_is_epoch_and_view_separated():
    pks = sorted(_pks(7))
    cum, total = build_cum([1] * 7)
    a = [fallback_leader_index(3, v, pks, cum, total) for v in range(1, 40)]
    b = [fallback_leader_index(4, v, pks, cum, total) for v in range(1, 40)]
    assert a != b, "changing the epoch must change the schedule"
    assert len(set(a)) > 1, "a fixed epoch must still rotate leaders across views"


def test_predictor_index_is_always_in_range():
    pks = sorted(_pks(7))
    cum, total = build_cum([1] * 7)
    for v in range(1, 500):
        assert 0 <= fallback_leader_index(9, v, pks, cum, total) < 7


# The devnet's deterministic 7-peer committee, and the compacted 50e18 self-delegation.
# Mirrors `weighted_vrf.rs::xlang_conformance` EXACTLY — if that Rust test changes, this
# fixture must change with it, or the live case predicts against a stale elector.
_XLANG_PKS = [
    "2e71978f382869ff2f2ac15424a86125610cccafb8629ca9f72c5aa5e5a9fefe",
    "0bd49e62f8033187d06ef14ef76ac78c26d3dc640613bf86cf7267a949cd9c50",
    "a6b3db1592dfaed7e0aebe01f2f1df8f71c06abd558df30f6a55b0159afee225",
    "2a23be9412ba671627da659cfee2bb01db7b81dbed2d595407e8102c62940b75",
    "0f89339953580de411151a06a1d8bbc8030b77e467e3d3670f7c6bfdf2be63e3",
    "fac42278ce587337d76a08cdcb21fed8e27dfba5d55bca0bbf6d1842fba7c999",
    "596918e015ca3b4b2bc2482dc36a58398423cc6f0c89b9d018ae8c928e73977c",
]
_XLANG_STAKE = 5_000_000_000
_XLANG_VECTOR = [
    (2, 1, 5), (2, 2, 2), (2, 3, 5), (2, 4, 1),
    (2, 5, 0), (2, 6, 4), (2, 7, 5), (2, 8, 1),
    (5, 1, 2), (5, 2, 5), (5, 3, 6), (5, 4, 1),
    (5, 5, 6), (5, 6, 3), (5, 7, 0), (5, 8, 3),
]


def test_predictor_matches_the_rust_cross_language_vector():
    """The one test that would have caught the first live run's defect at desk time."""
    pks = sorted(bytes.fromhex(h) for h in _XLANG_PKS)
    cum, total = build_cum([_XLANG_STAKE] * len(pks))
    for epoch, view, want in _XLANG_VECTOR:
        assert fallback_leader_index(epoch, view, pks, cum, total) == want, (
            f"diverged from weighted_vrf.rs at epoch {epoch} view {view}")


def test_weight_magnitude_changes_the_winner_not_just_the_distribution():
    """`rand % total` is magnitude-sensitive: normalising a uniform committee to 1s
    elects someone ELSE. This is the defect the first live run hit."""
    pks = sorted(bytes.fromhex(h) for h in _XLANG_PKS)
    ones = build_cum([1] * len(pks))
    real = build_cum([_XLANG_STAKE] * len(pks))
    assert (fallback_leader_index(2, 1, pks, *ones)
            != fallback_leader_index(2, 1, pks, *real))


def test_build_cum_matches_the_rust_all_zero_guard():
    assert build_cum([0, 0, 0]) == ([1, 2, 3], 3)
    assert build_cum([5, 0, 5]) == ([5, 5, 10], 10)


def test_stake_weight_biases_selection():
    pks = sorted(_pks(3))
    cum, total = build_cum([98, 1, 1])
    hits = [fallback_leader_index(1, v, pks, cum, total) for v in range(1, 300)]
    assert hits.count(0) > hits.count(1) + hits.count(2)


# ── the log parser ─────────────────────────────────────────────────────────────

# Pinned against the derived Debug of `Round { epoch: Epoch(u64), view: View(u64) }`.
# If a live run disagrees with this rendering, THIS fixture is what to re-pin.
def _derive_line(height, epoch, view):
    return (f"2026-07-29T10:00:00.000Z  INFO dpos::derive_seed: derive-seed: block derived "
            f"height={height} seed_round=Round {{ epoch: Epoch({epoch}), view: View({view}) }} "
            f"prev_randao=0xabc evm_hash=0xdef beacon_active=Some(true)")


def _propose_line(height):
    return (f"2026-07-29T10:00:00.000Z  INFO fluentbase_consensus::application: "
            f"dpos: proposing order block height={height}")


def test_parser_joins_height_view_and_proposer():
    logs = {
        "validator-0": "\n".join([_derive_line(11, 4, 7), _propose_line(10)]),
        "validator-2": "\n".join([_derive_line(11, 4, 7), _derive_line(12, 4, 9)]),
    }
    view_of, epoch_of, proposer_of = parse_view_leader_map(logs)
    # `seed_round` is the block's OWN round — the seed it consumes for its own derive, NOT
    # rule PIN's parent witness. An off-by-one here mispredicts every leader by one view.
    assert view_of[11] == 7 and epoch_of[11] == 4
    assert view_of[12] == 9
    assert proposer_of[10] == "validator-0"


def test_parser_ignores_unrelated_lines_and_genesis():
    logs = {"validator-1": "\n".join([
        "INFO something else entirely height=5",
        _derive_line(0, 0, 0),
        _propose_line(3),
    ])}
    view_of, _, proposer_of = parse_view_leader_map(logs)
    assert view_of == {0: 0}
    assert proposer_of == {3: "validator-1"}


def test_parser_is_idempotent_across_duplicate_reporters():
    """Every node logs derive-seed for every block; the map must not depend on how many did."""
    one = {"validator-0": _derive_line(11, 4, 7)}
    many = {f"validator-{i}": _derive_line(11, 4, 7) for i in range(5)}
    assert parse_view_leader_map(one)[0] == parse_view_leader_map(many)[0]


# ── the nullified-gap detector ─────────────────────────────────────────────────

def test_successors_of_nullified_finds_gaps_within_an_epoch():
    view_of = {10: 1, 11: 2, 12: 5, 13: 6}
    epoch_of = dict.fromkeys(view_of, 4)
    # views 3,4 produced nothing → block 12 (view 5) is the successor of a nullified view
    assert successors_of_nullified(view_of, epoch_of) == [12]


def test_successors_of_nullified_does_not_cross_epoch_boundaries():
    view_of = {10: 8, 11: 1}
    epoch_of = {10: 4, 11: 5}
    assert successors_of_nullified(view_of, epoch_of) == []


def test_successors_of_nullified_is_empty_on_a_perfect_run():
    view_of = {h: h - 9 for h in range(10, 20)}
    assert successors_of_nullified(view_of, dict.fromkeys(view_of, 2)) == []


# ── the verdict ────────────────────────────────────────────────────────────────

_OK_CONTROLS = [(1, 3, 3), (2, 5, 5), (3, 0, 0)]


def _samples(n_mismatch, n_total, base_epoch=1):
    out = []
    for i in range(n_total):
        pred = 2
        obs = 4 if i < n_mismatch else 2
        out.append((base_epoch, 10 + i, pred, obs))
    return out


#: A broken predictor: mismatches everywhere, and would otherwise read as a green run.
_BAD_CONTROLS = [(1, 3, 3), (2, 5, 6), (3, 0, 0)]

#: `(controls, samples, fin_delta, bad_share_nodes) -> (exit_code, reason substring)`.
#: A `None` substring means the original assertion read the code only.
_VERDICTS = [
    (_OK_CONTROLS, _samples(20, 24), 50, [], 0, "BRANCH-INDEPENDENT",
     "pass_when_enough_leaders_differ_from_the_fallback"),
    # The pre-change signature: mismatch_count == 0 with certainty.
    (_OK_CONTROLS, _samples(0, 30), 50, [], 1, "FALLBACK STILL IN USE",
     "fail_when_every_leader_matches_the_fallback"),
    # The threshold, driven from both sides: 4 mismatches still fails, 5 is the pass.
    (_OK_CONTROLS, _samples(4, 30), 50, [], 1, None,
     "fail_is_not_triggered_just_below_the_threshold_by_accident__below"),
    (_OK_CONTROLS, _samples(5, 30), 50, [], 0, None,
     "fail_is_not_triggered_just_below_the_threshold_by_accident__at"),
    (_BAD_CONTROLS, _samples(30, 30), 50, [], 2, "positive control FAILED",
     "inconclusive_when_the_positive_control_fails"),
    ([(1, 3, 3)], _samples(30, 30), 50, [], 2, "positive control",
     "inconclusive_when_too_few_controls"),
    # A stop that never took effect must not pass vacuously.
    (_OK_CONTROLS, _samples(3, 3), 50, [], 2, "successor-of-nullified",
     "inconclusive_when_the_sample_is_too_thin"),
    (_OK_CONTROLS, _samples(20, 24), 50, ["validator-3"], 1, "SHARE-GATE FIRED",
     "fail_when_the_share_gate_fired"),
    (_OK_CONTROLS, _samples(20, 24), 0, [], 1, "NO PROGRESS",
     "fail_when_finalized_did_not_advance"),
    # A demoted member explains the thin sample; report the cause, not the symptom.
    ([], [], 50, ["validator-2"], 1, "SHARE-GATE FIRED",
     "share_gate_outranks_a_thin_sample"),
]


@pytest.mark.parametrize("controls,samples,fin_delta,bad_shares,want_code,want_reason",
                         [r[:6] for r in _VERDICTS], ids=[r[6] for r in _VERDICTS])
def test_the_seed_continuity_verdict(controls, samples, fin_delta, bad_shares,
                                     want_code, want_reason):
    code, reason = evaluate_seed_continuity_case(controls, samples, fin_delta, bad_shares)
    assert code == want_code, reason
    if want_reason is not None:
        assert want_reason in reason
