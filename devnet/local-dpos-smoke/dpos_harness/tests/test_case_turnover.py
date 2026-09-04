"""turnover — the pure verdict layer of the zero-overlap boundary case.

The premise is scored SEPARATELY and FIRST, and these tests are what pin that:
a run in which three of four seats flip must fail on the premise, not pass on
the conclusion. Without that, the recorded decay of delegated weight in a
minimal devnet set would let the case go green over a boundary that still had
overlap — i.e. over the case that already worked.
"""

import pathlib

import pytest

from dpos_harness.cases import turnover
from dpos_harness.cases.turnover import (
    BOUNDARY_METRICS,
    CONSTANT_BASE_METRIC,
    SPAWN_DEFER_METRIC,
    evaluate_turnover_case,
    evaluate_turnover_premise,
)

OUT = {f"0xout{i}" for i in range(4)}
IN = {f"0xin{i}" for i in range(4)}


def _c(**kw):
    """One node's reading of BOUNDARY_METRICS, defaulting every family to 0."""
    vals = {fam: 0 for fam in BOUNDARY_METRICS}
    vals.update(kw)
    return {"validator-0": vals}


def _committee(addrs):
    return " ".join(sorted(addrs))


def test_a_full_flip_satisfies_the_premise():
    ok, why = evaluate_turnover_premise(_committee(OUT), _committee(IN), IN, OUT)
    assert ok, why


def test_three_of_four_fails_on_the_premise_not_on_the_conclusion():
    """The deliberate breakage the plan requires. One incumbent survives, so the
    boundary has overlap — and the case must say so instead of scoring liveness."""
    survivor = sorted(OUT)[0]
    after = (IN - {sorted(IN)[0]}) | {survivor}
    premise = evaluate_turnover_premise(_committee(OUT), _committee(after), IN, OUT)
    assert not premise[0]
    assert "overlap" in premise[1]

    # And the full verdict must inherit that, over readings that would otherwise
    # be a clean pass: the chain advanced and neither counter moved.
    ok, why = evaluate_turnover_case(premise, 100, 200, 32, _c(), _c())
    assert not ok and why == premise[1]


def test_a_partial_arrival_fails_on_the_premise():
    after = set(sorted(IN)[:3])
    ok, why = evaluate_turnover_premise(_committee(OUT), _committee(after), IN, OUT)
    assert not ok and "partial" in why


def test_an_unreadable_committee_is_not_scored():
    ok, why = evaluate_turnover_premise("", _committee(IN), IN, OUT)
    assert not ok and "unreadable" in why


def test_a_flat_chain_across_the_boundary_is_the_halt_under_test():
    premise = evaluate_turnover_premise(_committee(OUT), _committee(IN), IN, OUT)
    ok, why = evaluate_turnover_case(premise, 100, 105, 32, _c(), _c())
    assert not ok and "did not advance" in why


def test_a_spawn_defer_refuses_the_run_even_when_the_chain_advanced():
    """A chain that advanced while an incoming member sat verify-only was carried by
    the remaining quorum, not by the transport. At a four-seat committee one deferring
    member still leaves a quorum of three, so liveness alone cannot see this — which is
    why the assert stays at zero rather than being relaxed."""
    premise = evaluate_turnover_premise(_committee(OUT), _committee(IN), IN, OUT)
    ok, why = evaluate_turnover_case(premise, 100, 200, 32, _c(),
                                     _c(**{SPAWN_DEFER_METRIC: 1}))
    assert not ok
    assert SPAWN_DEFER_METRIC in why and "verify-only" in why


def test_a_constant_base_election_refuses_the_run_even_when_nothing_deferred():
    """The silent downgrade, and the reason the two families are scored separately: a
    member that elected on the predictable base never deferred and never skipped, so
    the defer counter says nothing about it."""
    premise = evaluate_turnover_premise(_committee(OUT), _committee(IN), IN, OUT)
    ok, why = evaluate_turnover_case(premise, 100, 200, 32, _c(),
                                     _c(**{CONSTANT_BASE_METRIC: 3}))
    assert not ok
    assert CONSTANT_BASE_METRIC in why and "predictable" in why
    assert SPAWN_DEFER_METRIC not in why, (
        "the two defects must not share a failure line — they send an operator to "
        "different halves of the system")


def test_an_unreadable_metric_is_never_scored_as_zero():
    premise = evaluate_turnover_premise(_committee(OUT), _committee(IN), IN, OUT)
    unread = {"validator-0": {fam: -1 for fam in BOUNDARY_METRICS}}
    ok, why = evaluate_turnover_case(premise, 100, 200, 32, unread, unread)
    assert not ok and "unreadable" in why


def test_one_absent_family_is_enough_to_lose_the_node_as_a_witness():
    """An eagerly-registered family that did not render is a broken scrape, not a
    healthy zero — so a node that answered for only ONE of the two proves neither, and
    with no other node reading, the run is unverified rather than green."""
    premise = evaluate_turnover_premise(_committee(OUT), _committee(IN), IN, OUT)
    half = {"validator-0": {SPAWN_DEFER_METRIC: 0, CONSTANT_BASE_METRIC: -1}}
    ok, why = evaluate_turnover_case(premise, 100, 200, 32, half, half)
    assert not ok and "unreadable" in why



def test_the_challenger_stake_dominates_the_largest_incumbent():
    """A flat figure is a claim about a stake table the case never read. A live
    run at 20e18 flat left exactly one incumbent seated — selection uses a strict
    `>`, so one genesis validator above the flat figure is all it takes, and the
    result reads like a partial flip, i.e. like the defect under test."""
    from dpos_harness.cases.turnover import CHALLENGER_STAKE_FLOOR, challenger_stake

    assert challenger_stake([]) == CHALLENGER_STAKE_FLOOR
    assert challenger_stake([0, 0]) == CHALLENGER_STAKE_FLOOR
    big = 40 * 10**18
    assert challenger_stake([3 * 10**18, big]) > big


# ══ the product-source guard ══════════════════════════════════════════════════════

CRATES_DIR = pathlib.Path(__file__).resolve().parents[4] / "crates"


@pytest.mark.parametrize("metric", BOUNDARY_METRICS)
def test_the_conclusion_metrics_are_families_the_product_ACTUALLY_REGISTERS(metric):
    """THE REASON THIS TEST EXISTS, stated plainly because it is a bug that already happened.

    This case's conclusion assert used to read `dpos_parent_seed_boundary_skip_total`. FLU-1204
    deleted that family. Nothing went red: `beacon_metric_value` returns -1 for an absent family,
    `skip_count` mapped -1 to 0 as a healthy "never incremented", and the assert compared 0 to 0
    on every node and passed. A case that brought up eight containers, flipped a whole committee
    and verified NOTHING.

    So the two replacements are pinned to the `ctx.register("<name>", …)` literal in the Rust,
    once each. A rename or a deletion reds HERE, in a suite that runs in four seconds, instead of
    turning a live case green while it measures nothing.

    The REGISTERED spelling, not the scrape spelling: prometheus-client appends its own `_total`
    to a counter sample, which is why the readers go through `nodes.beacon_metric_value`'s
    substring parse rather than an anchored match."""
    if not CRATES_DIR.is_dir():
        pytest.skip(f"consensus crate not in this tree ({CRATES_DIR})")
    needle = f'"{metric}"'
    hits = [(str(src), n) for src in sorted(CRATES_DIR.rglob("*.rs"))
            for n, line in enumerate(src.read_text(encoding="utf-8", errors="replace")
                                     .splitlines(), 1)
            if needle in line]
    assert len(hits) == 1, f"{metric!r} must be registered exactly once, found {hits}"


def test_the_retired_witness_family_is_gone_from_the_product_and_from_this_case():
    """The other half of the same guard, and it is not symmetric with the one above.

    Pinning the NEW names catches a future rename. It does not catch the failure that actually
    occurred, which was the OLD name surviving in the harness after the product dropped it. So
    the retired family is asserted absent on BOTH sides: no emitter under `crates/`, and no
    reader in this module. If it ever comes back, that is a decision someone must make
    deliberately rather than inherit."""
    if not CRATES_DIR.is_dir():
        pytest.skip(f"consensus crate not in this tree ({CRATES_DIR})")
    retired = "dpos_parent_seed_boundary_skip_total"
    emitters = [str(src) for src in sorted(CRATES_DIR.rglob("*.rs"))
                if retired in src.read_text(encoding="utf-8", errors="replace")]
    assert emitters == [], f"{retired!r} is emitted again by {emitters} — decide, do not inherit"
    src = pathlib.Path(turnover.__file__).read_text(encoding="utf-8")
    # COMMENT lines are exempt, and deliberately: the module header explains what the retired
    # family was and why it went, which is the note that stops the next reader re-adding it.
    # A CODE line is the thing being forbidden.
    code = [(n, ln) for n, ln in enumerate(src.splitlines(), 1)
            if retired in ln and not ln.lstrip().startswith("#")]
    assert code == [], (
        f"{retired!r} is back in turnover.py CODE at {[n for n, _ in code]} while nothing emits "
        "it — that read sums to zero on every node and passes the conclusion assert "
        "unconditionally")
