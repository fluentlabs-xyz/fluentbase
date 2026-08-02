"""smoke-production-path — `scripts/case-production-path.sh`.

The full DPoS production lifecycle on a chain where the staking cluster is deployed at RUNTIME via
forge rather than baked into genesis, mirroring prod: plain sequencer → runtime-deploy
token+verifier+cluster → bootstrap a five-validator committee → cold-restart every validator into
unified `--dpos` → register an EXTERNAL sixth validator through governance while its supervisor
follows in-process → it AUTO-promotes at its first committee epoch boundary with no operator action
→ the committee rotates and the displaced validator auto-demotes and keeps following → one member
is ejected by liveness — all under a background value-transfer load.

First-of-its-kind end-to-end (live rotation plus dynamic join). Per the brief, a failure here is a
real-bug finding, not a test defect.

PREREQUISITES (host): docker, foundry (forge/cast), a solidity-contracts checkout at
`$SOLIDITY_CONTRACTS_DIR`.

═══ TWO DELIBERATE DELTAS FROM THE BASH ═══════════════════════════════════════════════════

This case's bash builds its bring-up INLINE (`:45-182`) rather than calling `pp_bring_up_rotation`,
and the two differ in four places. Routing it through the shared `RotationBringUp` — which is what
chunk 5a ported, phase for phase, from `pp_bring_up_rotation` — adopts the shared values:

  * THREE CONVERGE BUDGETS LENGTHEN: phase A 120→240 s, the activation-block wait 200→400 s, the
    post-activation alignment 90→180 s. All three are strictly MORE generous, so nothing that
    passed can now fail; they are bring-up patience, not one of the three timeout-based negative
    proofs (§2.4 item 3), which are untouched.
  * THE FULL NODE IS RECREATED at the cold restart. `pp_bring_up_rotation:1349` passes
    `"${PP_VALS[@]}" full-node` and this case's inline copy passes only `"${PP_VALS[@]}"`, which
    leaves the full-node running its PHASE-A command while the validators switch consensus —
    `docker-compose.production-path.dpos.yml:150-165` is what gives it `--cert-follow`. The case
    then requires all seven nodes to align past the anchor, so including it is what that assertion
    needs; the omission is the outlier among the five.

Both are recorded here rather than in a commit message because a later reader diffing the
transcript against the bash will find exactly these four lines and nothing else.
"""

from __future__ import annotations

from . import asserts_prod, prod


def run_case(argv=None) -> int:
    return prod.run("smoke-production-path", [asserts_prod.assert_production_path], argv)
