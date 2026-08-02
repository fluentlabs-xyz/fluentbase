"""smoke-vrf-fault (standalone) — `scripts/case-vrf-fault.sh`.

Its own bring-up, then `assert_vrf_fault` (A1/B3/B4: the beacon survives an f=1 fault on the n−f
survivors; the downed node restarts, reloads its share, and catches up the gap with verified
prev_randao). The body is DESTRUCTIVE — it stops and restarts a validator — and lives in
`asserts_fault.py`, shared with `smoke-fault`. Heavy (~4-5 min).
"""

from __future__ import annotations

from . import asserts_fault, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-vrf-fault", [asserts_fault.assert_vrf_fault], argv)
