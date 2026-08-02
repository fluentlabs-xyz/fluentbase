"""smoke-vrf-dkg-restart-midwindow (standalone) — `scripts/case-vrf-dkg-restart-midwindow.sh`.

TUNED GENESIS, forwarded to genesis-init through the compose file's environment interpolation AND
mirrored into the host-side chain math (they MUST agree with the on-chain `ChainConfig.initialize`
arguments):

    epochBlockInterval  64   a GENEROUS DKG window — room for the journal-present poll to land
                             inside the open window on any host
    dposActivationBlock 128  = 2 * interval, which keeps the migration anchor in absolute epoch 2

There was a third tuned knob, `felonyThreshold=1`, whose only job was to make the windowed
participation slash escalate to a JAIL so the case had a consequence to assert. That whole tier
is deleted. The load-bearing assertion is now PRODUCTION — `producedAt(2, idx) > 0` — which is a
strictly better signal: it is a direct positive observation that the resumed member did the work,
where the old one only observed that a punishment failed to arrive. Asserting merely "the chain
stays live" would still pass on a run where the victim contributed nothing.

Standalone and heavy (~6-8 min).

═══ SUBSUMPTION: NOT SUBSUMED ════════════════════════════════════════════════════════════

`Makefile:202` describes `smoke-vrf-dkg-durability` phase 1 as subsuming "the single-victim
midwindow baseline". Read against the two sources that is not so, and `case-vrf-dkg-durability.sh`
itself says so in three places (`:34`, `:190`, `:239`): its phase-1 kill lands POST-FINALIZE, so
the recovery mechanism there is the DURABLE SHARE FILE reloaded by `build_beacon_plane::load_all`,
the actor never re-runs `maybe_start` for an already-past epoch, and **no resume line is emitted
at all**. The journal-RESUME path — `Player::resume` from `beacon-dkgjournal-e2.bin`, the
resolver re-fetch of missing peer logs, the pre-seal `dealing_closed()` finalize gate — is
reachable only from a PRE-finalize restart, which is what the share-ABSENT half of this case's
gate exists to guarantee. Nor does durability phase 1 assert that its recovered member went on to
PRODUCE. The two cases overlap on "a restarted member ends up with a consistent share" and share
nothing else.
"""

from __future__ import annotations

from . import asserts_onchain, driver, verdicts_onchain as vo


def run_case(argv=None) -> int:
    return driver.run(
        "smoke-vrf-dkg-restart-midwindow",
        [asserts_onchain.assert_vrf_dkg_restart_midwindow], argv,
        exports={"EPOCH_BLOCK_INTERVAL": vo.MIDWINDOW_EPOCH_INTERVAL,
                 "DPOS_ACTIVATION_BLOCK": vo.MIDWINDOW_ACTIVATION_BLOCK,
                 "EPOCH_INTERVAL": vo.MIDWINDOW_EPOCH_INTERVAL})
