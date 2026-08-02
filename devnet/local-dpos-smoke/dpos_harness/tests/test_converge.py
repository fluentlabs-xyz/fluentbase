"""`core/converge.py` — the cluster-A poll loops, against their bash originals.

The sentinel half of this contract is in `test_sentinels.py`; here it is the LOOP shape: the
floor rule, the cadence, what sleeps when, and the SAME-HEIGHT identity test both `_wait_aligned`
and `wait_follower_align` make. Each of those is a line of `lib.sh` and each has a way of being
lost in a port that still looks right — a floor read as `>=` instead of `>`, a floor checked on
one reader instead of all of them, a node compared on height alone with no hash, a `"null"` floor
coerced to 0, or the fork check quietly restored to a tip-against-tip compare.
"""

from __future__ import annotations

import pytest

from dpos_harness.core import converge, nodes


@pytest.fixture(autouse=True)
def _no_wall_clock(monkeypatch):
    """Neutralise sleep and advance a fake monotonic clock 0.4s per read, so every loop below
    runs its real body without wall time. Records the sleeps for cadence assertions."""
    slept = []
    clock = {"t": 0.0}

    def mono():
        clock["t"] += 0.4
        return clock["t"]

    monkeypatch.setattr(converge.time, "monotonic", mono)
    monkeypatch.setattr(converge.time, "sleep", lambda s: slept.append(s))
    return slept


# ── the floor rule ───────────────────────────────────────────────────────────

def test_hex_floor_only_honours_a_hex_floor():
    """Bash: `[[ "$floor" == 0x* ]] || floor=""`. Everything else means NO floor."""
    assert converge.hex_floor("0x10") == 16
    assert converge.hex_floor("") is None
    assert converge.hex_floor(None) is None
    assert converge.hex_floor(100) is None          # a decimal int is not the bash spelling


def test_the_null_prev_fin_means_no_floor_not_zero():
    """`PREV_FIN` starts as the literal string `"null"` (lib.sh:408). Coercing it to 0 would
    turn the anchor guard into `head > 0`, which every live chain satisfies — the guard would be
    gone and nothing would say so."""
    assert converge.hex_floor("null") is None
    readings = [("a", "0x1|0xaa"), ("b", "0x1|0xaa")]
    assert converge.aligned_reading(readings, converge.hex_floor("null")) == "0x1|0xaa"


def test_the_floor_is_strict_not_inclusive():
    """A head EQUAL to the floor is not past it. bash `(( head > floor ))`."""
    readings = [("a", "0x10|0xaa"), ("b", "0x10|0xaa")]
    assert converge.aligned_reading(readings, 16) is None      # == floor
    assert converge.aligned_reading(readings, 15) == "0x10|0xaa"


def test_a_regressed_head_is_rejected_not_merely_a_different_one():
    """This is WHY the floor is numeric rather than an inequality against the string: a head
    that merely DIFFERS from the anchor could be a regressed chain, which is exactly what the
    anchor floor is there to catch."""
    readings = [("a", "0x0a|0xaa"), ("b", "0x0a|0xaa")]        # 10, below the anchor
    assert converge.aligned_reading(readings, 32) is None


# ── same-height identity (the `_wait_aligned` fork check) ────────────────────

def _chain_hash(h_dec):
    """A stand-in producer: the canonical chain's block at height N hashes to `0x<N>`. Blocks
    above 1000 are ones the producer does not hold, which `blockhash_at` renders as `"null"`."""
    return f"0x{h_dec}" if 1 <= h_dec <= 1000 else "null"


def test_ragged_heights_with_matching_hashes_are_convergence():
    """The smoke-production-path shape: seven readers, one of them (the sole node on the WS
    follower upstream) exactly one block behind, all on the same chain. Read serially over
    several seconds on a 1 blk/s chain, this set NEVER coincides — and it is healthy. Demanding
    one identical tip from seven moving tips is a race, not a check."""
    readings = [("validator-0", "0x17f|0x383")] + \
               [(f"validator-{i}", "0x17f|0x383") for i in range(1, 5)] + \
               [("validator-5", "0x17e|0x382"),          # one behind, same chain
                ("full-node", "0x17f|0x383")]
    assert converge.aligned_reading(readings, 256, _chain_hash) == "0x17f|0x383"


#: The second reader that must break a two-node set at floor 256, one row per original test.
_NOT_CONVERGENCE = [
    # The half the compare existed for. v5 is one behind AND on a different block there, so the
    # producer's block at v5's own height is not v5's block.
    (("validator-5", "0x17e|0xDEAD"), "a_reader_forked_at_its_own_height_still_fails"),
    # Same height, different block — the classic fork. Byte-identity no longer holds, so the
    # short-circuit does not fire and the producer read catches it.
    (("validator-5", "0x17f|0xDEAD"), "a_reader_forked_at_the_SAME_height_still_fails"),
    # `"null|null"` in a RAGGED set must still be rejected on the head, not silently skipped —
    # the all-nodes-down false pass is the reason `NON_HEADS` exists.
    (("validator-5", converge.UNREACHABLE), "an_unreachable_reader_never_counts_as_agreement"),
    # A reader AHEAD of the producer reads a height `blockhash_at` renders as `"null"`. That is
    # a retry, not an agreement — "null" must never compare equal to a real hash.
    (("full-node", "0x7d1|0x2001"), "a_block_the_producer_does_not_hold_never_matches"),
]


@pytest.mark.parametrize("second", [r[0] for r in _NOT_CONVERGENCE],
                         ids=[r[1] for r in _NOT_CONVERGENCE])
def test_a_two_node_set_is_not_convergence(second):
    readings = [("validator-0", "0x17f|0x383"), second]
    assert converge.aligned_reading(readings, 256, _chain_hash) is None


def test_an_empty_reading_is_not_a_head():
    """`""` joined `"null"` and `"0x0"` when the case-layer rejoin verdicts started delegating
    here: two empty readings are byte-identical, so the short-circuit would have accepted "no
    answer at all, twice" as agreement — with no floor to catch it, which is exactly the shape
    `crash_survivor_realigned` is called with."""
    assert converge.aligned_reading([("a", ""), ("b", "")], None, _chain_hash) is None
    assert converge.aligned_reading([("a", "0x17f|0x383"), ("b", "")], None,
                                    _chain_hash) is None


def test_the_floor_is_checked_per_reader_not_only_on_the_first():
    """Bash floor-checked `readings[0]` alone, which byte-identity made equivalent to checking
    all of them. Ragged heights break that equivalence: a laggard still AT or BELOW the anchor
    has not passed it, and the anchor floor exists to catch exactly that."""
    readings = [("validator-0", "0x101|0x257"), ("validator-5", "0x100|0x256")]
    assert converge.aligned_reading(readings, 256, _chain_hash) is None     # v5 == floor
    assert converge.aligned_reading(readings, 255, _chain_hash) == "0x101|0x257"


def test_byte_identical_readings_never_read_the_producer():
    """The old happy path costs nothing new: readers reporting the same height AND the same hash
    already agree with each other, so the extra `cast block` per distinct height would only
    re-confirm it. This is also what keeps a SINGLE-reading reader (`_read_cascade_node` — the L3
    downstream, a DIFFERENT chain from the producer) on its old semantics exactly."""
    def boom(_h):
        raise AssertionError("producer read on an already-identical set")

    identical = [("a", "0x20|0xaa"), ("b", "0x20|0xaa"), ("c", "0x20|0xaa")]
    assert converge.aligned_reading(identical, 16, boom) == "0x20|0xaa"
    l3_only = [("l3", "0x20|0xL3")]
    assert converge.aligned_reading(l3_only, 16, boom) == "0x20|0xL3"


# ── wait_aligned ─────────────────────────────────────────────────────────────

def test_wait_aligned_returns_on_the_first_aligned_poll(_no_wall_clock):
    reader = lambda: [("a", "0x20|0xaa"), ("b", "0x20|0xaa")]  # noqa: E731
    reading, last = converge.wait_aligned(30, "0x10", reader)
    assert reading == "0x20|0xaa"
    assert _no_wall_clock == [], "slept after a successful poll"


def test_wait_aligned_sleeps_only_between_unaligned_polls(_no_wall_clock):
    """b is FORKED (its hash is not the producer's block at b's own height), so the poll must
    keep going; the third read is the one that agrees."""
    seq = iter([
        [("a", "0x20|0xaa"), ("b", "0x1f|0xFORK")],
        [("a", "0x20|0xaa"), ("b", "0x1f|0xFORK")],
        [("a", "0x21|0xcc"), ("b", "0x21|0xcc")],   # identical → no producer read at all
    ])
    reading, _ = converge.wait_aligned(60, "0x10", lambda: next(seq),
                                       producer_hash_at=_chain_hash)
    assert reading == "0x21|0xcc"
    assert _no_wall_clock == [converge.ALIGN_POLL_S] * 2


def test_wait_aligned_returns_the_last_readings_on_expiry(_no_wall_clock):
    """The caller owns the fail-loud message, so the loop must hand back what it last saw —
    otherwise a timeout can only say "did not converge" and the laggard stays invisible."""
    reader = lambda: [("validator-0@8545", "0x20|0xaa"),        # noqa: E731
                      ("validator-3", "0x1f|0xbb")]
    reading, last = converge.wait_aligned(1, "0x10", reader,
                                          producer_hash_at=lambda h: "null")
    assert reading is None
    detail = converge.divergence_detail(last)
    assert "validator-0@8545=0x20|0xaa" in detail and "validator-3=0x1f|0xbb" in detail


def test_wait_aligned_consumes_a_generator_reader(_no_wall_clock):
    """`wait_aligned` materialises each read (`list(reader())`) so a generator-based reader is
    safe to index and to re-read for the diagnostic. Without that the timeout message would be
    empty for exactly the readers that stream."""
    def reader():
        yield ("a", "0x20|0xaa")
        yield ("b", "0x20|0xaa")
    reading, last = converge.wait_aligned(30, "", reader)
    assert reading == "0x20|0xaa" and len(last) == 2


# ── wait_finalized_ge ────────────────────────────────────────────────────────

def test_wait_finalized_ge_is_inclusive(_no_wall_clock):
    """`>= target`, not `>` — bash `(( $(finalized_dec) >= target ))`."""
    assert converge.wait_finalized_ge(100, timeout=30, read_fin=lambda: 100) is True


def test_wait_finalized_ge_expires_false(_no_wall_clock):
    assert converge.wait_finalized_ge(100, timeout=1, read_fin=lambda: 99) is False


def test_wait_finalized_ge_defaults_to_the_pinned_reader(monkeypatch, _no_wall_clock):
    """The reader is injectable for tests, NOT so a caller can quietly substitute the sim's
    cross-committee fallback. The default must be the pinned bash read."""
    monkeypatch.setattr(nodes, "check_external", lambda port: "0x64|0xaa")   # 100
    assert converge.wait_finalized_ge(100, timeout=30) is True


# ── wait_follower_align ──────────────────────────────────────────────────────

def _follower_world(monkeypatch, follower, producer_chain):
    """Stub the two reads the poll makes: the FOLLOWER's `"height|hash"` reading (a string, or a
    per-call iterator), and the PRODUCER's block hash at a DECIMAL height — `{height: hash}`, a
    height the producer does not hold reading `"null"` exactly as `blockhash_at` coerces it."""
    monkeypatch.setattr(nodes, "check_external",
                        lambda port: next(follower) if hasattr(follower, "__next__") else follower)
    monkeypatch.setattr(nodes, "blockhash_at",
                        lambda block, rpc_url=None: producer_chain.get(int(block), "null"))


def test_follower_align_needs_the_hash_to_match_too(monkeypatch, _no_wall_clock):
    """The FORK check, and the reason the hash is read at all. A follower at height 32 on a
    different block than the producer's 32 is not aligned — a height-only compare would say it
    was."""
    _follower_world(monkeypatch, "0x20|0xbb", {32: "0xaa"})
    assert converge.wait_follower_align(18545, 0, timeout=1) is None


def test_follower_align_succeeds_when_the_producer_agrees_at_that_height(monkeypatch,
                                                                        _no_wall_clock):
    _follower_world(monkeypatch, "0x20|0xaa", {32: "0xaa"})
    assert converge.wait_follower_align(18545, 15, timeout=30) == "0x20|0xaa"


def test_follower_align_passes_while_the_two_tips_differ(monkeypatch, _no_wall_clock):
    """THE REGRESSION. Nothing here requires the two tips to be level: the producer is at 40 and
    the follower at 32, on the producer's own block 32, and that is alignment. The old compare
    demanded the two READINGS be byte-equal, so a healthy follower a few blocks behind in a live
    tail failed every poll to the deadline (smoke-cert-follow phase 2, smoke-cert-cascade
    tier-1). It also holds the other way round — a follower AHEAD of the producer's finalized,
    on a block the producer has canonical, is aligned."""
    _follower_world(monkeypatch, "0x20|0xaa", {32: "0xaa", 40: "0xdd"})     # producer tip 40
    assert converge.wait_follower_align(18545, 15, timeout=30) == "0x20|0xaa"
    _follower_world(monkeypatch, "0x28|0xdd", {32: "0xaa", 40: "0xdd"})     # follower AHEAD
    assert converge.wait_follower_align(18545, 15, timeout=30) == "0x28|0xdd"


def test_follower_align_rejects_a_head_at_or_below_the_floor(monkeypatch, _no_wall_clock):
    """The floor here is a DECIMAL int (bash `(( ... > floor ))` on an unquoted arg), and it is
    strict."""
    _follower_world(monkeypatch, "0x20|0xaa", {32: "0xaa"})
    assert converge.wait_follower_align(18545, 32, timeout=1) is None    # 0x20 == 32


def test_follower_align_never_passes_on_a_dead_follower(monkeypatch, _no_wall_clock):
    """An unreachable follower reads `"null|null"` and is rejected on the head sentinel, before a
    height is computed at all. Bash keeps that pre-test rather than leaning on
    `hex_to_dec("null") == 0` happening to lose to the floor: the sentinel is the contract, and a
    node that did not answer must never reach the hash compare."""
    _follower_world(monkeypatch, converge.UNREACHABLE, {0: "null"})
    assert converge.wait_follower_align(18545, 0, timeout=1) is None


def test_follower_align_fails_when_the_producer_cannot_be_read(monkeypatch, _no_wall_clock):
    """The producer side has its own unreachable coercion: `blockhash_at` yields `"null"` for a
    block it does not hold AND for an RPC that did not answer. Neither may read as agreement."""
    _follower_world(monkeypatch, "0x20|0xaa", {})
    assert converge.wait_follower_align(18545, 0, timeout=1) is None


def test_follower_align_polls_at_its_own_slower_cadence(monkeypatch, _no_wall_clock):
    """2s, not 1s (lib.sh:224) — an above-floor iteration issues two host round-trips."""
    _follower_world(monkeypatch, converge.UNREACHABLE, {})
    converge.wait_follower_align(18545, 0, timeout=1)
    assert _no_wall_clock and set(_no_wall_clock) == {converge.FOLLOWER_POLL_S}
