"""smoke-liveness (standalone) — `scripts/case-liveness.sh`.

Four kill/rejoin cycles across three different validators on one bring-up, spanning the catch-up
spectrum: a deep multi-boundary gap, a single boundary cross, a within-epoch gap that crosses
none, and an immediate re-kill of the node that just rejoined.

STANDALONE, and deliberately NOT in the `smoke-fault` aggregate. Every other destructive case in
this tree restores the stack before it returns; this one's jail is UNRECOVERABLE — the cycles can
push the participation counter below the floor often enough to jail a validator, which
permanently shrinks the committee. `fault.py`'s header records the same exclusion from the other
side, and the two must not drift apart.
"""

from __future__ import annotations

from . import asserts_onchain, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-liveness", [asserts_onchain.assert_liveness], argv)
