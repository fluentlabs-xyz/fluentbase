"""turnover — the pure verdict layer of the zero-overlap boundary case.

The premise is scored SEPARATELY and FIRST, and these tests are what pin that:
a run in which three of four seats flip must fail on the premise, not pass on
the conclusion. Without that, the recorded decay of delegated weight in a
minimal devnet set would let the case go green over a boundary that still had
overlap — i.e. over the case that already worked.
"""

from dpos_harness.cases.turnover import (
    evaluate_turnover_case,
    evaluate_turnover_premise,
)

OUT = {f"0xout{i}" for i in range(4)}
IN = {f"0xin{i}" for i in range(4)}


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
    # be a clean pass: the chain advanced and nobody skipped a view.
    ok, why = evaluate_turnover_case(premise, 100, 200, 32, {"validator-0": 0},
                                     {"validator-0": 0})
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
    ok, why = evaluate_turnover_case(premise, 100, 105, 32, {"validator-0": 0},
                                     {"validator-0": 0})
    assert not ok and "did not advance" in why


def test_a_boundary_skip_refuses_the_run_even_when_the_chain_advanced():
    """A chain that advanced but skipped a view was carried by a retry, not by
    the transport — the assert stays at zero rather than being relaxed."""
    premise = evaluate_turnover_premise(_committee(OUT), _committee(IN), IN, OUT)
    ok, why = evaluate_turnover_case(premise, 100, 200, 32, {"validator-0": 0},
                                     {"validator-0": 1})
    assert not ok and "boundary_skip" in why


def test_an_unreadable_metric_is_never_scored_as_zero():
    premise = evaluate_turnover_premise(_committee(OUT), _committee(IN), IN, OUT)
    ok, why = evaluate_turnover_case(premise, 100, 200, 32, {"validator-0": -1},
                                     {"validator-0": -1})
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
