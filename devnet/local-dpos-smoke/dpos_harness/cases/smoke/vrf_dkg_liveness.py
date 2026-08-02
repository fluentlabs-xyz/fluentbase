"""smoke-vrf-dkg-liveness (standalone) — `scripts/case-vrf-dkg-liveness.sh`.

Its own bring-up, then `assert_vrf_dkg_liveness` — the DKG-liveness NEGATIVE edge (NO reshare). A
committee member taken OFFLINE during its epoch-2 DKG window misses the ceremony, is shareless for
that epoch, and SITS OUT the seed quorum while the chain stays live on the remaining n−f
survivors; on restart it re-derives prev_randao from the cert seed.

THE VICTIM MUST BE STOPPED BEFORE the epoch-2 DKG window opens (`epoch_start(2) −
DKG_MARGIN_BLOCKS`), which is why the assertion stops it immediately after bring-up — the DPoS
swap lands near the activation block, well below the window. The assertion fails loud rather than
waiting if the chain is already past it, because a victim stopped after the window may already
hold a share and the case would then be measuring something else. Heavy (~4-5 min).

NOT part of `smoke-fault`: like the bash, it is a standalone stack. Its whole premise is a window
that opens once, near bring-up, and a chained run would have consumed it.
"""

from __future__ import annotations

from . import asserts_fault, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-vrf-dkg-liveness", [asserts_fault.assert_vrf_dkg_liveness], argv)
