"""smoke-vrf-dkg-live-heal (standalone) — was `smoke-vrf-dkg-liveness`.

Its own bring-up, then `assert_vrf_dkg_live_heal`. A committee member taken OFFLINE for the whole
of its epoch-2 DKG window misses the ceremony — and then OBTAINS the epoch key inside epoch 2
anyway: it pulls the epoch's agreed artifact over the beacon resolver, recomputes its share from
the dealers' public reveals, leaves vote-only certificate admission, and PRODUCES in the epoch it
was elected for (FLU-1166).

THE NAME CHANGED BECAUSE THE VERDICT DID. As `smoke-vrf-dkg-liveness` this case asserted the
opposite — that the member sat the epoch out — and "liveness" meant the CHAIN's, on the n-f
survivors. That sit-out was not a property, it was a hole: nothing fetched the agreed artifact for
the LIVE epoch, because the epoch manager's repair sweep excludes `epoch >= frontier` by design,
so a demoted member waited for a fetch that was never issued. The chain-liveness half is still
asserted (legs 1 and 6); it is no longer the point.

ONE VICTIM, NEVER TWO. `max_reveals = f` over the player set, and at n=4 that is 1. A second
absentee makes every honest dealer withhold its ENTIRE log on `TooManyReveals`, the ceremony fails
outright, and the case measures a failed DKG instead of a heal. The assertion body says so at
length; this is the pointer for whoever edits the compose topology instead.

TUNED GENESIS, for the same reason `smoke-vrf-dkg-restart-midwindow` runs one, and mirrored into
the host-side chain math (they MUST agree with the on-chain `ChainConfig.initialize` arguments):

    epochBlockInterval  64   epoch 2 is 64 blocks (~64 s at 1 blk/s) instead of 32, which is what
                             gives the restarted member room to catch up, pull, heal, promote AND
                             be elected at least once before the epoch ends. `producedAt(2, ...)`
                             is the load-bearing leg and it is a stake-weighted lottery: on a
                             32-block epoch the recovery eats most of the window and the leg would
                             be flaky for a reason that has nothing to do with the product. It
                             also buys the headroom the deal-window guard needs — the migration
                             anchor may land anywhere in epoch 0, and the DEAL phase opens one
                             interval later. Measured live: anchor 128, victim stopped at ~145,
                             deal window open at 189.
    dposActivationBlock 128  = 2 * interval, which keeps the migration anchor in absolute epoch 2

Both spellings are exported. `EPOCH_BLOCK_INTERVAL` is what the compose file interpolates into
genesis-init; `EPOCH_INTERVAL` is what the host profile reads for its epoch arithmetic. Setting
one and not the other gives the case a 64-block stack and 32-block math, silently.

THE VICTIM MUST BE STOPPED BEFORE the epoch-2 DKG ceremony's DEAL phase opens, which is
`epoch_start(1) - K` and NOT `epoch_start(2) - DKG_MARGIN_BLOCKS`: `maybe_start(now + 1)` runs on
every height tick, so the ceremony starts the moment the actor's clock enters epoch 1 and the
whole of epoch 1 is its deal phase; the margin is only where that phase CLOSES. Guarding on the
margin let a run stop the victim mid-phase, with every dealing already received and ACKED — which
passes every other assertion in the case while testing the wrong path. The assertion fails loud
rather than waiting, and the case gates a second time on the victim's own log
(`live DKG: ceremony started` on the restarted process, i.e. `JournalLoad::NoFile`) so a lost race
against a moving chain cannot go unnoticed. Heavy (~8-10 min).

NOT part of `smoke-fault`: like the bash it replaces, it is a standalone stack. Its whole premise
is a window that opens once, near bring-up, and a chained run would have consumed it.
"""

from __future__ import annotations

from . import asserts_fault, driver

#: The tuned geometry, in one place so the two spellings cannot disagree. Mirrors
#: `verdicts_onchain.MIDWINDOW_*`, which the restart-midwindow case exports for the same reason.
EPOCH_INTERVAL = 64
ACTIVATION_BLOCK = 128


def run_case(argv=None) -> int:
    return driver.run(
        "smoke-vrf-dkg-live-heal", [asserts_fault.assert_vrf_dkg_live_heal], argv,
        exports={"EPOCH_BLOCK_INTERVAL": EPOCH_INTERVAL,
                 "DPOS_ACTIVATION_BLOCK": ACTIVATION_BLOCK,
                 "EPOCH_INTERVAL": EPOCH_INTERVAL})
