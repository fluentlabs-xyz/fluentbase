"""counters.py — the monotonic-counter progress verdict.

One pure function with two consumers in different layers: `checks/battery.py` (the beacon
re-baseline arm) and `sim/actions.py` (the recovery-confirm / seed-progress arms). Both used to
carry their own copy and the copies had already drifted — the sim's coerced `cur` with `int()`,
the battery's did not — which is exactly how a shared verdict stops being shared. It lives in
`core` because both consumers sit above it; the sim's copy is gone, not mirrored.
"""

from __future__ import annotations


def counter_progress(cur, prev):
    """_counter_progress: classify one monotonic-counter step from a RESTARTABLE process:
    unreadable (cur<0) | baseline (no usable prev) | restart (cur<prev, RE-BASELINE) |
    flat (cur==prev, THE stall signal) | progress. The flat-vs-restart split is the whole
    point; collapsing them fabricated a stall out of a routine container recreate. PURE."""
    cur = int(cur)
    if cur < 0:
        return "unreadable"
    if prev is None or prev == "" or prev == "__none__":
        return "baseline"
    prev = int(prev)
    if prev < 0:
        return "baseline"
    if cur < prev:
        return "restart"
    if cur == prev:
        return "flat"
    return "progress"
