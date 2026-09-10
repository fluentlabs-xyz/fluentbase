"""smoke-byzantine (standalone) — `scripts/case-byzantine.sh`.

Its own bring-up under the `byzantine` compose overlay, which sets
`FLUENT_DPOS_BYZANTINE=equivocate` on validator-3. Requires the image built with the
`dpos-devnet-byzantine` cargo feature (the smoke Dockerfile enables it).

TWO things reach the stack from here and neither is a case-body concern:

  * the OVERLAY, through `driver.run(overlays=…)` — the same `StaticProfile.extra_overlays` seam
    chunk 3 used, honoured at the DPoS recreate (`stack/static_stack.py`). Bash spells it
    `export DPOS_EXTRA_COMPOSE="-f docker-compose.byzantine.yml"`, and that export is still
    honoured for anyone running the bash;
  * `converge_exclude="validator-3"`, because the byzantine node's reth NEVER finalizes. Without
    it the post-swap alignment gate would fail a case whose victim is behaving exactly as
    designed. Bash spells it `export DPOS_CONVERGE_EXCLUDE="validator-3"`.

Standalone because it leaves a JAILED, tombstoned validator behind and SHRINKS the committee for
good: the offender is dropped from the Active set at once, so every later commit on this five-seat
genesis seats the four survivors. There is no restore — the tombstone is permanent and the seat is
not refilled until the population recovers — so any case chained after it would be measuring a
four-signer chain on a four-seat committee sitting exactly on MIN_COMMITTEE_LENGTH.

THE STAND IS FIVE-SEAT FOR THIS CASE. The bash ran four, where the same jail leaves THREE: on
contract `bc42042a` that boundary reverted `CommitteeTooSmall(3,4)` into the node's fail-loud arm
and every honest node died 35 s after the tombstone (R-112, EXPERIMENTS §5.3 E1) while this case
was green because it stopped three blocks after the jail; Э0.2 carried the previous committee
forward instead; and since 2026-09-07 the carry is gone and the revert is back on every epoch.
None of those is what this case is for, so the genesis was sized to leave a legal committee behind
the jail. The below-the-floor path is a separate scenario (`scripts/xp/floor_halt_case.py`).

THE CASE RUNS THROUGH THAT BOUNDARY AND ASSERTS IT (step 4 in `assert_byzantine`): the committee
for the boundary epoch drops the offender, seats four, differs from the previous one, and mints a
DKG ceremony. Roughly two epochs of extra observation (~1.5-2.5 min at the default interval).
"""

from __future__ import annotations

from . import asserts_onchain, driver, verdicts_onchain as vo


def run_case(argv=None) -> int:
    return driver.run("smoke-byzantine", [asserts_onchain.assert_byzantine], argv,
                      converge_exclude=vo.BYZANTINE_EXCLUDE,
                      overlays=(vo.BYZANTINE_OVERLAY,))
