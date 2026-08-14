"""smoke-byzantine-vrf — `scripts/case-byzantine-vrf.sh`.

A Byzantine boundary proposer that asserts a FORGED PK_E (in `OrderBlock.beacon_outcome`) at a
change-epoch first block must NOT be able to finalize it, and the chain must stay live — an honest
leader crosses the boundary with the real key.

WHY THE ROTATION STACK: the forge fires ONLY on a CHANGE-epoch first block
(`BeaconVerify::is_change_epoch_first_block`, false unless committee[E] != committee[E-1]). The
genesis stack has a STATIC committee, so the predicate never fires and there is nothing to forge.

Requires the image built with the `dpos-devnet-byzantine` cargo feature (the smoke Dockerfile
enables it). NEVER in prod. Long (~8-10 min), foundry-gated; NOT in run-all.

═══ THE BRING-UP FINDING ══════════════════════════════════════════════════════════════════

The bash builds its own bring-up inline (`:163-297`) rather than calling `pp_bring_up_rotation`,
and the plan flags that as something to check before routing it through the shared one. Read line
by line, the two differ in exactly TWO places, both load-bearing:

  1. **A third compose file at the cold restart** — `docker-compose.byzantine-vrf.yml`, which puts
     `FLUENT_DPOS_BYZANTINE=forge-beacon-pk` on one validator. LOAD-BEARING, and it is the whole
     case. Expressed as `ProductionPathProfile.extra_overlays`, which appends it to the DPoS pair
     and leaves phase A on the bare file, exactly as the bash `export COMPOSE_FILE` sequence does.
  2. **Five extra deployer-funded transfers, after the staking module exists** — LOAD-BEARING in
     their ORDERING rather than their content. Their ORIGINAL justification was a create-nonce
     one — an earlier transfer would shift the deploy's CREATE addresses off the prediction in
     `staking-reader.json` — and that is gone with the prediction. The position survives for a
     reason that justification hid: they move BLEND and they act through a `Chain`, so they
     cannot run before the token has been deployed and the module installed, and they must still
     precede the first governance write. Expressed as `RotationBringUp`'s `post_manifest` seam,
     which fires exactly there.

The bash's THIRD difference — the staking-reader assert hoisted ahead of `setBlsVerifier` — is
gone with both of the steps it sat between. There is no create-nonce prediction to assert against
and no `setBlsVerifier`; the verifier rides `initialize`.

Everything else — the phase-A converge budget, the spammer, the two `forge create`s, the BLEND to
the joiner, the runtime-upgrade delivery, `initialize` seeding v0..v4, the two governance config
calls, the activation-block governance call, the clean-halt wait, the recreate argv including
`full-node`, the converge past the anchor and the `getEpochBlockInterval` read — is identical,
budget for budget.

So the case does NOT get a bring-up of its own here. Two seams is a far smaller surface than a
second copy of the bring-up, which is where the bug density in this family actually lives
(`plan §11`).

═══ WHY REPEATED COMMITTEE FLIPS ══════════════════════════════════════════════════════════

The forge fires only when the byzantine LEADS a change-epoch first block, and on such a block
leader election takes the stake-weighted view-1 fallback — so its probability equals its committee
stake share. It is boosted to ~72%, and the case still drives a SEQUENCE of boundaries by toggling
the joiner in and out for the residual margin: over B boundaries P(never leads) ≈ (1−0.72)^B, so
five flips put it at ≈0.17%. f=1 is preserved throughout — exactly ONE node is byzantine, and the
joiner's toggling never makes a second one.
"""

from __future__ import annotations

from . import asserts_byzantine_vrf as B
from . import prod

#: `case-byzantine-vrf.sh:288` — the overlay layered ON TOP of the production-path DPoS pair.
BYZANTINE_OVERLAY = "docker-compose.byzantine-vrf.yml"


def run_case(argv=None) -> int:
    drive = B.ByzantineDrive()
    return prod.run("smoke-byzantine-vrf", [drive.assertion], argv,
                    overlays=(BYZANTINE_OVERLAY,), post_manifest=drive.post_manifest)
