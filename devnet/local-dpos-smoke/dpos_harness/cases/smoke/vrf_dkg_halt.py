"""smoke-vrf-dkg-halt — `scripts/case-vrf-dkg-halt.sh`.

The >f PRE-SEAL TERMINAL halt (G2) — the NEGATIVE CONTROL for `smoke-vrf-dkg-durability`'s
post-seal fleet recovery. Same kill count, OPPOSITE seal state, opposite and PERMANENT outcome.

THIS CASE PASSES BY TIMEOUT, and that is the whole design (§2.4 item 3). Its assertion is an
ABSENCE: after two committee stayers are killed pre-seal and restarted, the chain must climb to the
change-epoch boundary edge and then NOT MOVE — twice, over a 30 s window and again over a 15 s one
— while no member ever finalizes a share for the new epoch. A shorter window cannot tell a chain
that has not produced a block YET from one that never will, so shortening either one leaves the
case green and testing nothing. Both budgets are copied from the bash verbatim
(`verdicts_rotation.FREEZE_WINDOW_S` / `REFREEZE_WINDOW_S`), and `tests/test_prod_case_verdicts.py`
drives every one of the absences with the forbidden evidence present.

WHY THE VICTIMS ARE RESTARTED rather than kept down: on n=5 the DKG dealer-quorum and the consensus
notarization quorum are BOTH 4. Keeping two down stalls CONSENSUS below the boundary, so the
proposer never reaches the boundary view and the `skipping propose` line never fires — an
indistinct consensus stall, not the DKG-None boundary skip this case isolates. Restarting restores
consensus quorum so the chain CLIMBS to the boundary, while the two nodes resume PLAYER-ONLY and
never re-deal, leaving the DKG permanently at 3 dealers < 4. Bringing them back does not heal it.

WHY ITS OWN BRING-UP (plan §6.2 fallback): the durability case sequences its two RECOVERABLE phases
on one bring-up. Sharing that bring-up would need a SECOND committee change after the first, and on
this six-key equal-initial-stake stack a second deterministic re-rank is fragile (the benched
original is tie-break ambiguous) and `undelegate` carries a multi-epoch delay. Since the halt is
terminal anyway, splitting it costs exactly one extra bring-up and no shared state — two cases, not
four. It drives the halt on the SAME clean trigger the durability case uses.

Long (~10-14 min), foundry-gated; NOT in run-all.
"""

from __future__ import annotations

from . import asserts_prod_dkg, prod


def run_case(argv=None) -> int:
    return prod.run("smoke-vrf-dkg-halt", [asserts_prod_dkg.assert_vrf_dkg_halt], argv)
