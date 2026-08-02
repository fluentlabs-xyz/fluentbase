"""smoke-rejump-signer (standalone) — the post-jump boundary-seeding gate.

WHAT IT GATES. A seated validator that falls far enough behind to re-jump must be SIGNING
again inside the landing epoch. Before the boundary-seeding fix it was not: the engine-spawn
gate needs the previous epoch's terminal block, a jump puts that height below the marshal
floor, and nothing re-fetches below the floor — so the member sat verify-only (no proposals,
no votes) until the next epoch boundary healed it by accident.

WHY THIS CASE HAD TO EXIST SEPARATELY. `smoke-cert-catchup` runs the same stop/advance/restart
choreography, but it deliberately tunes its gap BELOW the re-jump threshold so the re-jump does
not steal the derive-walk it measures, and even its optional deep cycle asserts only rejoin,
the park path and two absent fatals. A validator could be silent for a full epoch in every run
and nothing would redden.

TUNED GENESIS, and the tuning is the assertion. `EPOCH_BLOCK_INTERVAL=64` mirrored into
`EPOCH_INTERVAL`, exactly as the catch-up case does it, because the container's genesis env and
the host-side epoch arithmetic must not drift. The gap then sits in the band between the two
jump gates — see `verdicts_boundary.boundary_gap`, which carries the full argument.
"""

from __future__ import annotations

import os

from . import asserts_boundary, driver, verdicts_boundary as vb


def run_case(argv=None) -> int:
    interval = os.environ.get("EPOCH_BLOCK_INTERVAL", str(vb.BOUNDARY_EPOCH_INTERVAL))
    return driver.run("smoke-rejump-signer", [asserts_boundary.assert_rejump_signer], argv,
                      exports={"EPOCH_BLOCK_INTERVAL": interval, "EPOCH_INTERVAL": interval})
