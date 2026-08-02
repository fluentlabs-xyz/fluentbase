"""smoke-vrf-boundary (standalone) — `scripts/case-vrf-boundary.sh`.

Its own bring-up, then `assert_vrf_boundary` (the beacon survives the epoch-2→3 boundary on a
stable committee; F1). Heavy — it must produce blocks past `activation + 3*interval` — and NOT in
run-all on its own, because `smoke-base` covers it.
"""

from __future__ import annotations

from . import asserts, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-vrf-boundary", [asserts.assert_vrf_boundary], argv)
