"""smoke-epoch (standalone) — `scripts/case-epoch.sh`.

Its own bring-up, then `assert_epoch` (>= EPOCH_MIN_CROSS boundaries crossed + 1 blk/s pacing).
`EPOCH_MIN_CROSS` is read inside the assertion, so `make smoke-epoch`'s `EPOCH_MIN_CROSS=1`
export keeps working unchanged.
"""

from __future__ import annotations

from . import asserts, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-epoch", [asserts.assert_epoch], argv)
