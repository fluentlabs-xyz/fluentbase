"""smoke-peers (standalone) — `scripts/case-peers.sh`.

Its own bring-up, then `assert_peers` (both peer planes connect, a restarted validator
re-establishes both, and the chain advances past the restart). The body is DESTRUCTIVE — it
gracefully restarts one validator — and the restart is itself the restore; it lives in
`asserts_fault.py`, shared with `smoke-fault`.
"""

from __future__ import annotations

from . import asserts_fault, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-peers", [asserts_fault.assert_peers], argv)
