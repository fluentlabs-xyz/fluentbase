"""converge.py — the convergence / wait-for-height polls (`lib.sh` cluster A).

Python port of the poll loops every case and both bring-ups stand on: `_wait_aligned`
(lib.sh:172-205), `wait_converge` (:218), `wait_finalized_ge` (:181-188),
`wait_follower_align` (:213-227) and `baseline_height` (:402-411). The per-node READS they
poll already live in `core/nodes.py` (`check_node` / `check_external` / `finalized_dec`);
this module is the loop and the verdict, nothing else — it opens no socket of its own.

═══ THE SENTINELS ARE THE CONTRACT ══════════════════════════════════════════════════════

Every guard here exists because a specific coercion produced a check that still passed and
no longer checked anything. Do not "simplify" any of them:

  * `check_node` / `check_external` return the literal `"null|null"` for an unreachable
    node — NEVER "". Bash learned this the hard way: a refused connection makes curl print
    nothing and exit 7, `jq` on empty stdin emits an empty line and exits 0, so the
    `|| echo "null|null"` fallback did NOT fire and the reading slipped past the
    `!= "null"` guard. An ALL-NODES-DOWN poll then false-passes as "converged", because
    five empty strings are trivially identical to each other. `_aligned_reading` therefore
    rejects the head `"null"` explicitly and `UNREACHABLE` is named here so a caller can
    test for it rather than for falsiness.
  * The head is also rejected at `"0x0"` — genesis is not convergence.
  * A `floor` is honoured only when it is HEX (bash `[[ "$floor" == 0x* ]] || floor=""`),
    and the head must be NUMERICALLY GREATER than it. Not merely different: a head that
    differs from the floor could be a REGRESSED chain, which is the thing the floor is
    there to catch.
  * `finalized_dec_pinned` coerces an unreachable RPC to 0, which is safe for a `>= target`
    poll (a transient 0 costs one iteration) and NEVER safe as a baseline — 0 is
    indistinguishable from genesis and a "finalized > PRE" assertion against PRE=0 passes
    trivially, masking exactly the stalled rejoin it was written to detect. That is why
    `baseline_height` exists as a separate, FAIL-LOUD helper instead of a default argument.

═══ WHY `finalized_dec_pinned` IS NOT `nodes.finalized_dec` ═════════════════════════════

`nodes.finalized_dec` is the SIM's chain-wide liveness read: when the pinned host RPC is
0/unreachable it falls back to the max finalized across the other running committee
containers, because one node dying must not zero a whole-chain liveness signal.

The case layer needs the bash semantics instead — `finalized_dec()` is
`check_external 8545` and nothing else (lib.sh:273). The difference is not cosmetic: a case
that stops the pinned node and then reads a baseline must FAIL LOUD (bash does), whereas the
sim reader would happily answer with a different node's height and let the case continue
comparing two different chains' heads. The two are kept as two functions, named for what
they do, rather than merged into one that is subtly wrong for one caller.
"""

from __future__ import annotations

import time

from . import nodes, topology

#: What `check_node` / `check_external` return for a node that did not answer. A poll that
#: treats this as data is the all-nodes-down false pass.
UNREACHABLE = "null|null"

#: The head values that are never convergence: an unreachable node, genesis, and NO READING AT
#: ALL. The empty string is in the tuple because `aligned_reading` is now the rule the case-layer
#: rejoin verdicts delegate to, and one of them (`crash_survivor_realigned`) was reached with `""`
#: on both sides — two empty readings are byte-identical, so the short-circuit would accept them
#: as agreement. Bash rejects `""` a step later, on `printf '%d' ""`; naming it here makes the two
#: trees reject it in the same place and for the same stated reason.
NON_HEADS = ("null", "0x0", "")

#: `_wait_aligned` poll cadence (lib.sh:202 `sleep 1`).
ALIGN_POLL_S = 1
#: `wait_follower_align` poll cadence (lib.sh:224 `sleep 2`) — deliberately slower than the
#: alignment poll: an above-floor iteration issues two host round-trips, not one (the
#: follower's reading, then the producer's block hash at that height).
FOLLOWER_POLL_S = 2
#: `baseline_height` budget (lib.sh:381 `+ 30`). Not a knob: the chain is already live past
#: the anchor at every call site, so a real height is always available within seconds and a
#: longer wait would only delay a loud failure.
BASELINE_TIMEOUT_S = 30


class ConvergeError(Exception):
    """Fail-loud translation of the bash convergence aborts (`echo FAIL …; exit 1`).

    Carries `(reason_id, message)` — the same attribute shape as `chain.writes.ChainError`,
    so a case can handle both with one `except (ConvergeError, ChainError)`. It is a distinct
    class only because `core` may not import `chain` (the layering contract, enforced by
    tests/test_layering.py); if that ever became legal these should merge.
    """

    def __init__(self, reason_id: str, message: str):
        self.reason_id = reason_id
        self.message = message
        super().__init__(f"{reason_id}: {message}")


def hex_floor(floor) -> int:
    """A floor as a decimal int, or None when there is no floor to check.

    Bash: `[[ "$floor" == 0x* ]] || floor=""` — so `"null"` (the PREV_FIN sentinel before a
    migration has run), `""` and a decimal string all mean "no floor". Keeping that exact
    rule matters: `PREV_FIN` starts life as the string `"null"` (lib.sh:408), and coercing
    that to 0 would turn `wait_dpos_converge`'s floor check into `head > 0`, which every
    live chain satisfies — the anchor guard would be gone and nobody would see it go.
    """
    if isinstance(floor, str) and floor.startswith("0x"):
        return nodes.hex_to_dec(floor)
    return None


def aligned_reading(readings, floor_dec, producer_hash_at=None):
    """The per-poll verdict of `_wait_aligned`: the pinned producer's `"height|hash"` when this
    set of readings IS convergence, else None.

    `readings` is a list of `(label, "height|hash")` pairs — one per node, the PINNED PRODUCER
    FIRST. Labels exist only for the timeout diagnostic; the verdict reads the readings.

    SAME-HEIGHT identity, NOT tip identity — de2de13c's rule for `wait_follower_align`, applied
    to its sibling. This used to require every reading BYTE-IDENTICAL, i.e. that up to SEVEN
    independently-advancing tips coincide across a read sweep that is nowhere near atomic (the
    producer by host curl, then five sequential `docker compose exec` round-trips, then the
    full-node by host curl — seconds end to end on a 1 block/s chain). One node steadily a
    block behind, which is normal and not a fault, then fails EVERY poll to the deadline.
    smoke-production-path hit exactly that: validator-5, the only node on the WS follower
    upstream, was one block behind in 26 of 26 samples while the other six agreed and the chain
    climbed past 383, healthy.

    The FORK check is what the compare existed for and it is kept whole: each reader's hash is
    checked against the producer's block AT THAT READER'S OWN HEIGHT, so a node at height h on a
    different chain still fails. `blockhash_at` yields `"null"` for a block the producer does
    not hold (or an unreachable producer), which never matches a real hash.

    Conditions, in order: at least one reading; EVERY head neither `"null"` (unreachable) nor
    `"0x0"` (genesis); EVERY head strictly greater than the floor, when one was given (the bash
    original floor-checked only `readings[0]`, which identity made equivalent — with ragged
    heights allowed it must be per-reader); and cross-node agreement.

    Agreement short-circuits when all readings are byte-identical: nodes reporting the same
    height AND the same hash already agree with each other, so the extra producer read would
    only re-confirm it. That also keeps a SINGLE-reading reader (`_read_cascade_node` — the L3
    downstream, a different chain from the producer) on its old semantics exactly: one reading
    is trivially self-identical, so it is floor-checked and never compared against the L2
    producer.

    `producer_hash_at` is `h_dec -> hash`, defaulting to `nodes.blockhash_at`. It is injectable
    so a test can drive the ragged path without a subprocess, NOT so a caller can point the fork
    check at some other chain.

    THIS IS THE ONLY COPY OF THE RULE. The four case-layer rejoin/reconverge verdicts —
    `verdicts_fault.crash_survivor_realigned` / `.full_restart_reconverged` and
    `verdicts_onchain.catchup_rejoined` / `.liveness_rejoined` — each carried their own
    byte-identity compare and each was the same race; they now delegate here, passing their own
    floor and their own two-or-five readings. A predicate that re-derives "are these nodes on the
    same chain" locally is a regression even when it looks correct on the day it is written.
    """
    if not readings:
        return None
    first = readings[0][1]
    identical = all(r == first for _, r in readings)
    producer_hash_at = producer_hash_at or nodes.blockhash_at
    memo = {}
    for _, r in readings:
        head, _, node_hash = (r or "").partition("|")
        if head in NON_HEADS:
            return None
        head_dec = nodes.hex_to_dec(head)
        if floor_dec is not None and head_dec <= floor_dec:
            return None
        if identical:
            continue
        if head_dec not in memo:
            memo[head_dec] = producer_hash_at(head_dec)
        p_hash = memo[head_dec] or "null"
        if p_hash == "null" or p_hash.lower() != node_hash.lower():
            return None
    return first


def wait_aligned(timeout, floor, reader, poll_s=ALIGN_POLL_S, producer_hash_at=None):
    """Poll `reader` until every node is past genesis, past the `floor` (when hex) and on the
    producer's chain at its own height. Faithful port of `_wait_aligned` (lib.sh:172-205).

    Returns `(reading, last_readings)` — `reading` is the producer's `"height|hash"` on success
    and None on expiry. `last_readings` is always the most recent read, so the CALLER owns
    the fail-loud message and can name every node's last value (bash dumps the divergent set
    at its call sites). Sleeps only BETWEEN unaligned reads, never after a success.
    """
    floor_dec = hex_floor(floor)
    deadline = time.monotonic() + timeout
    last = []
    while time.monotonic() < deadline:
        last = list(reader())
        hit = aligned_reading(last, floor_dec, producer_hash_at)
        if hit is not None:
            return hit, last
        time.sleep(poll_s)
    return None, last


def divergence_detail(readings) -> str:
    """`label=reading, …` for a timeout message, or a loud note when NOTHING answered.

    The empty case is called out rather than rendered as an empty string: "no readings" and
    "all five nodes agreed" look identical in a log that only prints the joined list, and the
    first is the all-nodes-down shape this module's sentinels exist to keep visible.
    """
    if not readings:
        return "no readings (all nodes unreachable)"
    return ", ".join(f"{label}={r}" for label, r in readings)


def finalized_dec_pinned() -> int:
    """Decimal finalized height of the host-exposed producer, and ONLY of it — bash
    `finalized_dec` (lib.sh:273) is `check_external 8545 | cut -d'|' -f1` through `printf '%d'`.

    An unreachable RPC coerces to 0 (same value as genesis). Fine for the `>= target` polls;
    never acceptable as a PRE/baseline — see `baseline_height` and the module header.
    """
    head = nodes.check_external(topology.HOST_RPC_PORT).split("|", 1)[0]
    return 0 if head == "null" else nodes.hex_to_dec(head)


def wait_finalized_ge(target, timeout=60, read_fin=None, poll_s=ALIGN_POLL_S) -> bool:
    """Poll until the pinned producer's finalized height reaches `target` (decimal).
    Bash `wait_finalized_ge` (lib.sh:181-188); True on success, False on expiry.

    `read_fin` defaults to `finalized_dec_pinned` — the bash reader. It is injectable so a
    test can drive the loop, NOT so a caller can quietly swap in the sim's fallback reader.
    """
    read_fin = read_fin or finalized_dec_pinned
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if read_fin() >= target:
            return True
        time.sleep(poll_s)
    return False


def wait_follower_align(port, floor_dec, timeout=240, poll_s=FOLLOWER_POLL_S,
                        producer_port=None):
    """Poll until the follower RPC on host `port` is finalized strictly above the decimal
    `floor_dec` AND the producer agrees on the block AT THAT HEIGHT (same hash). Bash
    `wait_follower_align` (lib.sh:213-227). Returns the follower's aligned reading, or None
    on expiry.

    SAME-HEIGHT identity, NOT tip identity. This used to require the follower's whole
    `"height|hash"` reading to equal the producer's, which is a condition about two separate,
    non-atomic RPC round-trips against two independently-advancing tips: it holds only when
    both happen to sit on the same height across two different wall-clock instants. At ~1
    block/s a perfectly healthy follower one block behind fails EVERY poll to the deadline,
    and did — smoke-cert-follow phase 2 (restart into pure live-tail) and smoke-cert-cascade
    tier-1 (L1-checkpoint replay) failed on an idle host with followers hundreds of blocks
    past the floor and clean logs. The call sites that passed were the ones taken right after
    a cold-start jump LANDS on the producer's finalized — the only ones with a coinciding
    instant. The compare was never correct, it was lucky.

    The hash leg stays because the FORK check is the point: the producer's block at the
    follower's own height must be the follower's block, so a follower at height h on a
    different chain still fails. Only the demand that two moving tips coincide is dropped.
    The `f_head != "null"` pre-test stays for the same reason bash keeps it — an unreachable
    follower reads the `"null|null"` sentinel and must never count as agreement — and
    `blockhash_at` yields `"null"` for a block the producer does not hold (or an unreachable
    producer), which never matches a real hash.
    """
    producer_port = topology.HOST_RPC_PORT if producer_port is None else producer_port
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        f = nodes.check_external(port)
        f_head, _, f_hash = f.partition("|")
        if f_head != "null":
            f_dec = nodes.hex_to_dec(f_head)
            if f_dec > floor_dec:
                producer_hash = nodes.blockhash_at(f_dec, topology.host_url(producer_port))
                if producer_hash != "null" and producer_hash.lower() == f_hash.lower():
                    return f
        time.sleep(poll_s)
    return None


def baseline_height(timeout=BASELINE_TIMEOUT_S, read_fin=None, poll_s=ALIGN_POLL_S) -> int:
    """A settled finalized height for a PRE/baseline capture. Bash `baseline_height`
    (lib.sh:380-389) — retry until the producer returns a real height > 0, then FAIL LOUD.

    This is not `finalized_dec_pinned` with a retry bolted on; it is the other half of a
    deliberate split. `finalized_dec_pinned` maps "RPC unreachable" to 0 indistinguishably
    from genesis, so a momentary blip at capture time would seed a 0 baseline and make every
    later "finalized > PRE" assertion pass trivially — masking a stalled rejoin/restart. The
    chain is already live past the anchor at every call site, so a real height is always
    available; raising after ~30s is strictly better than returning a bogus baseline.
    """
    read_fin = read_fin or finalized_dec_pinned
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        h = read_fin()
        if h > 0:
            return h
        time.sleep(poll_s)
    raise ConvergeError(
        "baseline-height",
        f"could not read a finalized height > 0 from the producer within {timeout}s — "
        "refusing to seed a 0 baseline (it would make a later 'finalized > PRE' assertion "
        "pass trivially)")
