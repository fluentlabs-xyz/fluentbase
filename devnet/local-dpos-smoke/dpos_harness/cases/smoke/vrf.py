"""smoke-vrf (standalone) — `scripts/case-vrf.sh`.

Its own bring-up, then `assert_vrf` (the threshold beacon end to end; C1/C2/D1/E1). The assertion
is read-only w.r.t. consensus — it deploys a throwaway probe and sends one transaction to it.

This is the ONE of the five wrappers that honours `SMOKE_KEEP_UP` (`case-vrf.sh:14`), because it
is the one people debug interactively. The other four tear down unconditionally.
"""

from __future__ import annotations

from . import asserts, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-vrf", [asserts.assert_vrf], argv, honours_keep_up=True)
