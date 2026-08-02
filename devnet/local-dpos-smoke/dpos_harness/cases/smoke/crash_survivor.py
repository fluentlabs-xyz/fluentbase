"""smoke-crash-survivor (standalone, Problem A) — `scripts/case-crash-survivor.sh`.

Its own bring-up, then `assert_crash_survivor`: SIGKILL a validator ungracefully, let the chain
build an EL gap, restart it, and assert it recovers and REALIGNS rather than wedging on a missing
block. The body is DESTRUCTIVE and lives in `asserts_fault.py`, shared with `smoke-fault`.
"""

from __future__ import annotations

from . import asserts_fault, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-crash-survivor", [asserts_fault.assert_crash_survivor], argv)
