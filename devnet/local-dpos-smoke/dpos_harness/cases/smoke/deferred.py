"""smoke-deferred (standalone) — `scripts/case-deferred.sh`.

Its own sequencer→DPoS bring-up, then `assert_deferred` (the K-lag invariant, result-commitment
integrity, and EL-slowed-validator liveness). The body is DESTRUCTIVE — it CPU-throttles a
validator — and restores the throttle before it returns; it lives in `asserts_fault.py`, shared
verbatim with `smoke-fault`, which runs it FIRST because the K-lag invariant wants a pristine
steady state.
"""

from __future__ import annotations

from . import asserts_fault, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-deferred", [asserts_fault.assert_deferred], argv)
