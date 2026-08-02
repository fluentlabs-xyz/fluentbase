"""beacon.py — the two shared VRF helpers `lib.sh` hoisted out of the case scripts.

`wait_nodes_have` (lib.sh:348-371) and `assert_beacon_window` (lib.sh:310-343) were pulled up so
the VRF, fault and boundary cases share ONE copy of the cross-node prev_randao compare instead of
three that drift. They live here rather than in `core/` for the same reason: they are case-layer
assertions over the node set, not transport.

The verdict itself is in `verdicts.evaluate_beacon_window`; this module is the READING half — it
decides which heights and which nodes, and hands the readings over.
"""

from __future__ import annotations

from . import verdicts
from .driver import BEACON_NODES

#: `lib.sh:349` — how long the followers get to catch up to the top of a window.
NODES_HAVE_TIMEOUT_S = 120
#: `lib.sh:320,323` — the tail depth of the per-node diagnostic on a window failure.
WINDOW_FAIL_LOG_TAIL = 80


def wait_nodes_have(ctx, block, timeout=NODES_HAVE_TIMEOUT_S, nodes=BEACON_NODES) -> bool:
    """Wait until EVERY node in `nodes` can serve `block` (lib.sh:348-371).

    Not politeness — a correctness gate. The import follower syncs the validator chain over devp2p
    and lags the validators' finalized tip by a few blocks, so a cross-node compare issued the
    instant the tip moves races the follower's catch-up and reports "follower has no mixHash for
    block N" for a follower that is merely three seconds behind. On expiry this prints WHICH node
    is missing the block, so a genuinely stuck follower fails loud instead of being waited on.
    """
    def have_all():
        return all(_has(ctx, svc, block) for svc in nodes)

    if ctx.poll(have_all, timeout):
        return True
    print(f"  [wait_nodes_have] timeout at block {block} — per-node status:", flush=True)
    for svc in nodes:
        state = "has" if _has(ctx, svc, block) else "MISSING"
        print(f"    {svc}: {state} block {block}", flush=True)
    return False


def _has(ctx, service: str, block) -> bool:
    mh = ctx.mixhash_of(service, block)
    return bool(mh) and mh != "null"


def read_beacon_window(ctx, lo, hi, nodes=BEACON_NODES):
    """[(height, [mixhash per node])] over the inclusive height range, in `nodes` order."""
    return [(n, [ctx.mixhash_of(svc, n) for svc in nodes]) for n in range(int(lo), int(hi) + 1)]


def assert_beacon_window(ctx, case: str, lo, hi, label: str, nodes=BEACON_NODES):
    """`assert_beacon_window` (lib.sh:310-343) — read the window and apply the verdict.

    Raises `SmokeFailure` on a bad window, after dumping the offending node's log tail the way
    bash does. Returns the per-height agreed values on success."""
    rows = read_beacon_window(ctx, lo, hi, nodes)
    ok, msg, mixes = verdicts.evaluate_beacon_window(list(nodes), rows, label)
    ctx.check(case, ok, f"(beacon-window) {msg}",
              on_fail=lambda: _dump_offender(ctx, msg, nodes))
    if not ctx.dry:
        print(f"  [{label}] blocks [{lo}..{hi}]: {len(mixes)}/{len(mixes)} distinct non-zero "
              f"prev_randao, byte-identical across all {len(nodes)} nodes", flush=True)
    return mixes


def _dump_offender(ctx, msg: str, nodes) -> None:
    """bash dumps `--tail=80` of the node named in the message for the two per-node failures (a
    missing block, a zero mixHash) and nothing for the two window-wide ones (disagreement, stuck
    randomness), whose diagnostic is the printed table. Same split here, derived from the message
    rather than duplicated as a flag."""
    for svc in nodes:
        if f"{svc} has no mixHash" in msg or f"on {svc}" in msg:
            ctx.dump_logs(WINDOW_FAIL_LOG_TAIL, svc)
            return
