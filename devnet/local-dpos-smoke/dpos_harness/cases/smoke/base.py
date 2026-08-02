"""smoke-base — `scripts/case-base.sh`: the READ-ONLY default-stack suite on ONE bring-up.

tx + epoch + vrf + vrf-boundary are all read-only w.r.t. consensus (none stops, restarts or jails
a node — at most they send transactions or deploy a throwaway probe), so they cannot contaminate
one another and share a single sequencer→DPoS migration instead of paying for four.

THE ORDER IS PART OF THE DESIGN — fail-fast, increasing sophistication:

    tx            the EVM executes and finalizes at all
    epoch         the boundary handoff works, and the pacing is 1 blk/s
    vrf           the beacon drives prev_randao end to end
    vrf-boundary  …and survives an epoch boundary

so the first thing to break is the simplest thing that is broken, and the later, longer
assertions never run against a chain that already failed a cheaper check. Each remains runnable
standalone (`smoke-tx`, `smoke-epoch`, …) with its own bring-up, for isolated debugging.
"""

from __future__ import annotations

from . import asserts, driver

ASSERTIONS = [asserts.assert_tx, asserts.assert_epoch, asserts.assert_vrf,
              asserts.assert_vrf_boundary]


def run_case(argv=None) -> int:
    rc = driver.run("smoke-base", ASSERTIONS, argv)
    if rc == 0 and "--dry-run" not in list(argv or []):
        print("OK (smoke-base): tx + epoch + vrf + vrf-boundary all passed on one bring-up",
              flush=True)
    return rc
