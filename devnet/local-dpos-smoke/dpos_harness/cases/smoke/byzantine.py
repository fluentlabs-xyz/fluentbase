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

Standalone because it leaves a JAILED validator and a permanently-shrunk committee behind — there
is no restore, and any case chained after it would be measuring a 3-node network.
"""

from __future__ import annotations

from . import asserts_onchain, driver, verdicts_onchain as vo


def run_case(argv=None) -> int:
    return driver.run("smoke-byzantine", [asserts_onchain.assert_byzantine], argv,
                      converge_exclude=vo.BYZANTINE_EXCLUDE,
                      overlays=(vo.BYZANTINE_OVERLAY,))
