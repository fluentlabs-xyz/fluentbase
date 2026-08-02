"""smoke-tx (standalone) — `scripts/case-tx.sh`.

Its own sequencer→DPoS bring-up, then the `assert_tx` body. The assertion is read-only w.r.t.
consensus and is shared verbatim with `smoke-base`, which runs it on a shared stack.
"""

from __future__ import annotations

from . import asserts, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-tx", [asserts.assert_tx], argv)
