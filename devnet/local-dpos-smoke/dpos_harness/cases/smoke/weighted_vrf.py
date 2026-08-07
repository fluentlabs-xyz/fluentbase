"""smoke-weighted-vrf (standalone) — the port of `scripts/case-weighted-vrf.sh`.

STANDALONE BECAUSE THE SKEW IS A DIFFERENT GENESIS, not a different phase. `HEAVY_STAKE_MULT`
travels through `docker-compose.yml` into `genesis-init`'s environment and is consumed at
`genesis-bootstrap/src/bootstrap.rs:426-430,460-473`, where it multiplies validator-0's entry in
`initialStakes` before `Staking.initialize`. That is a different chain from the one every other
smoke case brings up, so this cannot ride `smoke-base`'s stack the way `smoke-tx` and `smoke-epoch`
do — it needs its own bring-up with its own `exports`, the shape `cert_catchup.py` uses.

A TUNED EPOCH INTERVAL, and the tuning IS the sample size. The case scores one epoch, and one of
its three conditions is that every LIGHT validator produced at least one block — at the default
interval of 32 that condition fails by chance about one run in six. See
`verdicts_onchain.WEIGHTED_EPOCH_INTERVAL` for the arithmetic and for why 128, which would settle
it, is not available. `EPOCH_INTERVAL` is mirrored alongside `EPOCH_BLOCK_INTERVAL` for the reason
`cert_catchup` mirrors them: the container's genesis-init env and the host-side epoch arithmetic
must agree, and a case that tuned only one would compute boundaries for a chain it is not running.

WHAT THE NAME PROMISES AND WHAT THE CASE DELIVERS are not the same thing, deliberately. It proves
STAKE-WEIGHTED election. It does not prove the VRF: `randomness_bytes` weights identically on the
seed arm and the fallback arm and `elect` feeds both into one CDF
(`crates/dpos/consensus/src/weighted_vrf.rs:136-154,190-198`), so a completely dead beacon leaves
the leader distribution unchanged. The case asserts beacon liveness separately so the name is not
a lie about the chain it ran on.
"""

from __future__ import annotations

import os

from . import asserts_onchain, driver, verdicts_onchain as vo


def run_case(argv=None) -> int:
    mult = os.environ.get("HEAVY_STAKE_MULT", str(vo.HEAVY_STAKE_MULT))
    interval = os.environ.get("EPOCH_BLOCK_INTERVAL", str(vo.WEIGHTED_EPOCH_INTERVAL))
    return driver.run("smoke-weighted-vrf", [asserts_onchain.assert_weighted_vrf], argv,
                      exports={"HEAVY_STAKE_MULT": mult,
                               "EPOCH_BLOCK_INTERVAL": interval, "EPOCH_INTERVAL": interval})
