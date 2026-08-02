"""The sentinel discipline (§2.4 item 5), pinned across every layer that carries it.

Three coercions in the bash were each a diagnosed false verdict, and all three are the same
shape: a failure mode collapsed onto a value that reads as data. The failure they produce is
the worst kind — the check still passes and no longer checks anything, so nothing goes red.

  1. `check_node` / `check_external` must return `"null|null"` for an unreachable node, never
     "". A refused connection makes curl print NOTHING and exit 7; `jq` on empty stdin emits an
     empty line and exits 0, so bash's `|| echo "null|null"` fallback did NOT fire. Five empty
     strings are trivially identical to each other, so an ALL-NODES-DOWN poll false-passes as
     "converged".
  2. `production` must distinguish `(-1,-1)` "not in committee" from `(-2,-2)` "the getter
     call FAILED". Reading a -2 as `(0,0)` is a false slash verdict: 0 seen out of 0 certs looks
     like absent-but-live and satisfies a below-floor assertion.
  3. `finalized_dec` coerces unreachable to 0, which is SAFE for a `>= target` poll and never
     as a baseline — 0 is indistinguishable from genesis, so `finalized > PRE` against PRE=0
     passes trivially and masks the stalled rejoin it was written to detect. That is why
     `baseline_height` is a separate, fail-loud helper rather than a default argument.

These are tested here rather than each in its own module's file because they are ONE contract
spanning three modules, and the interesting failure is a layer dropping its half of it.
"""

from __future__ import annotations

import pytest

from dpos_harness.core import converge, nodes


# ── 1. an unreachable node is not convergence ────────────────────────────────

def test_unreachable_node_reads_the_sentinel_not_empty():
    """The transport-level guarantee everything below rests on: an empty/garbage RPC response
    becomes the literal `"null|null"`, which is `converge.UNREACHABLE`."""
    assert nodes._num_hash("") == ("null", "null")
    assert nodes._num_hash("not json at all") == ("null", "null")
    assert nodes._num_hash('{"result":null}') == ("null", "null")
    assert converge.UNREACHABLE == "null|null"


def test_all_nodes_down_never_reads_as_converged():
    """THE false pass this whole discipline exists to prevent. Five unreachable nodes agree
    perfectly — every reading is byte-identical — and that must NOT be convergence."""
    readings = [(f"validator-{i}", converge.UNREACHABLE) for i in range(4)]
    readings.append(("full-node@18545", converge.UNREACHABLE))
    assert converge.aligned_reading(readings, None) is None
    assert converge.aligned_reading(readings, 100) is None


def test_empty_readings_never_read_as_converged():
    """A reader that returned NOTHING (every read raised / the service list was empty) is also
    not convergence — the vacuous-`all()` arm."""
    assert converge.aligned_reading([], None) is None
    assert "all nodes unreachable" in converge.divergence_detail([])


def test_one_unreachable_node_breaks_alignment():
    """Healthy nodes and one dark one. Since `_wait_aligned` allows RAGGED heights, non-identity
    is no longer what rejects this — the `"null"` HEAD is, and it is checked per reader. The
    producer stub agrees with every live reader here precisely so nothing else can do the work."""
    readings = [("validator-0@8545", "0x20|0xaa"), ("validator-1", "0x1f|0xbb"),
                ("validator-2", converge.UNREACHABLE)]
    agreeable = lambda h: {32: "0xaa", 31: "0xbb"}.get(h, "null")   # noqa: E731
    assert converge.aligned_reading(readings[:2], None, agreeable) == "0x20|0xaa"
    assert converge.aligned_reading(readings, None, agreeable) is None


def test_genesis_head_never_reads_as_converged():
    """`0x0` is agreement on genesis, not convergence (bash `head != "0x0"`)."""
    readings = [("a", "0x0|0xabc"), ("b", "0x0|0xabc")]
    assert converge.aligned_reading(readings, None) is None


def test_aligned_past_genesis_is_convergence():
    """The positive control — without it every assertion above could pass vacuously."""
    readings = [("a", "0x20|0xaa"), ("b", "0x20|0xaa")]
    assert converge.aligned_reading(readings, None) == "0x20|0xaa"


def test_wait_aligned_on_a_dead_stack_times_out_and_names_it(monkeypatch):
    """End to end through the poll loop: an all-down stack expires rather than returning a
    reading, and the caller is handed the readings so its message can say so."""
    monkeypatch.setattr(converge.time, "sleep", lambda _s: None)
    clock = {"t": 0.0}
    monkeypatch.setattr(converge.time, "monotonic",
                        lambda: clock.__setitem__("t", clock["t"] + 0.6) or clock["t"])
    reader = lambda: [("validator-0@8545", converge.UNREACHABLE),  # noqa: E731
                      ("validator-1", converge.UNREACHABLE)]
    reading, last = converge.wait_aligned(1, "", reader)
    assert reading is None
    assert "validator-1=null|null" in converge.divergence_detail(last)


# ── 2. the two production-credit failure modes stay distinguishable ──────────

_PRODUCED_SIG = "producedAt(uint64,uint32)(uint32)"
_BLOCKS_SIG = "blocksInEpoch(uint64)(uint32)"


def _fake_calls(monkeypatch, committee: str, produced_out: str, blocks_out: str = None):
    """Stub the three `cast call` reads `nodes.production` makes: the committee enumeration (for
    `signer_idx`) and the two liveness counters. `blocks_out` defaults to `produced_out` so a
    test that only cares about the failure mode does not have to spell both."""
    monkeypatch.setattr(nodes, "staking_call",
                        lambda sig, *a, **kw: committee)
    monkeypatch.setattr(
        nodes, "liveness_call",
        lambda sig, *a, **kw: (produced_out if sig == _PRODUCED_SIG
                               else (produced_out if blocks_out is None else blocks_out)))


def test_production_not_in_committee_is_minus_one(monkeypatch):
    """`(-1,-1)` — the address is genuinely absent from the epoch committee. A real answer."""
    _fake_calls(monkeypatch, "[0x00000000000000000000000000000000000000aa]", "5\n")
    assert nodes.production(7, "0x00000000000000000000000000000000000000bb") == (-1, -1)


def test_production_read_failure_is_minus_two(monkeypatch):
    """`(-2,-2)` — the getter call FAILED. NOT the same fact as -1, and above all not `(0,0)`."""
    me = "0x00000000000000000000000000000000000000aa"
    _fake_calls(monkeypatch, f"[{me}]", "")                 # empty answer -> read failure
    assert nodes.production(7, me) == (-2, -2)


@pytest.mark.parametrize("partial", ["", "\n", "garbage\n"])
def test_production_partial_parse_is_also_minus_two(monkeypatch, partial):
    """A garbled answer is a READ failure too. A port that trusted `cast_field(1, …)` blindly
    would report `(0, 0)` — a seated member credited nothing, which is precisely the shape of a
    real verdict-worthy reading and must never be manufactured by a bad read."""
    me = "0x00000000000000000000000000000000000000aa"
    _fake_calls(monkeypatch, f"[{me}]", partial)
    assert nodes.production(7, me) == (-2, -2)


def test_production_real_counters_come_through(monkeypatch):
    """Positive control: well-formed answers are neither sentinel."""
    me = "0x00000000000000000000000000000000000000aa"
    _fake_calls(monkeypatch, f"[{me}]", "  120\n", "  128\n")
    assert nodes.production(7, me) == (120, 128)


def test_the_two_production_sentinels_are_not_the_same_value():
    """Stated as its own assertion because the whole point is that they never merge — and
    because `0` (the wrong collapse) is a third, distinct thing."""
    assert (-1, -1) != (-2, -2) != (0, 0)


# ── 3. the baseline is fail-loud, the poll read is not ───────────────────────

def test_finalized_dec_pinned_coerces_unreachable_to_zero(monkeypatch):
    """Safe for a `>= target` poll: a transient 0 costs one iteration."""
    monkeypatch.setattr(nodes, "check_external", lambda port: converge.UNREACHABLE)
    assert converge.finalized_dec_pinned() == 0


def test_finalized_dec_pinned_reads_only_the_pinned_producer(monkeypatch):
    """It must NOT inherit `nodes.finalized_dec`'s cross-committee fallback. A case that stops
    the pinned node and reads a baseline has to fail loud (bash does); a reader that answered
    with a different node's height would let the case compare two chains' heads."""
    seen = []
    monkeypatch.setattr(nodes, "check_external",
                        lambda port: seen.append(port) or converge.UNREACHABLE)
    monkeypatch.setattr(nodes, "running_services",
                        lambda: pytest.fail("the pinned read enumerated other containers"))
    assert converge.finalized_dec_pinned() == 0
    assert seen == [8545]


def test_baseline_height_refuses_a_zero_baseline(monkeypatch):
    """FAIL LOUD rather than seed 0 — a 0 baseline makes every later `finalized > PRE` pass."""
    monkeypatch.setattr(converge.time, "sleep", lambda _s: None)
    clock = {"t": 0.0}
    monkeypatch.setattr(converge.time, "monotonic",
                        lambda: clock.__setitem__("t", clock["t"] + 0.6) or clock["t"])
    with pytest.raises(converge.ConvergeError) as ei:
        converge.baseline_height(timeout=1, read_fin=lambda: 0)
    assert ei.value.reason_id == "baseline-height"


def test_baseline_height_retries_through_a_blip(monkeypatch):
    """A momentary RPC blip at capture time must cost an iteration, not the baseline."""
    monkeypatch.setattr(converge.time, "sleep", lambda _s: None)
    reads = iter([0, 0, 4711])
    assert converge.baseline_height(timeout=30, read_fin=lambda: next(reads)) == 4711


def test_wait_finalized_ge_tolerates_the_zero_coercion(monkeypatch):
    """The other half of the split: the SAME unreachable→0 is fine here, because the loop just
    polls again. This is why the two helpers exist rather than one."""
    monkeypatch.setattr(converge.time, "sleep", lambda _s: None)
    reads = iter([0, 0, 90, 100])
    assert converge.wait_finalized_ge(100, timeout=30, read_fin=lambda: next(reads)) is True
