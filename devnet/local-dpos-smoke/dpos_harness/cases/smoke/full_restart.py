"""smoke-full-restart (standalone) — `scripts/case-full-restart.sh`.

Its own bring-up, then `assert_full_restart`: stop ALL validators, verify each persisted (exit 0),
restart them, and assert the network reconverges from the persisted finalized head. The body is
DESTRUCTIVE — it stops the whole validator set — and lives in `asserts_fault.py`, shared with
`smoke-fault`, which runs it LAST because it is the most invasive thing in the file.
"""

from __future__ import annotations

from . import asserts_fault, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-full-restart", [asserts_fault.assert_full_restart], argv)
