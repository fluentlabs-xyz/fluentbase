"""smoke-cert-cascade (standalone) — `scripts/case-cert-cascade.sh`.

Its own bring-up under the `cert-cascade` compose overlay, then the L1 trust root, the
follower-serves-follower cascade, and the fail-closed rejection of an unverifiable checkpoint.

Standalone because it DEPLOYS two MockRollup contracts on the devnet mid-case and passes their
addresses to compose through the environment. The contracts are harmless leftovers, but the
addresses are per-run, so a second case sharing the stack would inherit an environment naming
contracts it knows nothing about.
"""

from __future__ import annotations

from . import asserts_follow, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-cert-cascade", [asserts_follow.assert_cert_cascade], argv,
                      overlays=(asserts_follow.CERT_CASCADE_OVERLAY,))
