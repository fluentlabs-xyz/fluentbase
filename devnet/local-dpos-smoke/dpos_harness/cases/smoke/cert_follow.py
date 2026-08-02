"""smoke-cert-follow (standalone) — `scripts/case-cert-follow.sh`.

Its own bring-up under the `cert-follow` compose overlay, then the three-phase follower assertion:
subscribe-align, gap back-fill, and the tampered-certificate rejection.

Never chained with another case. Phase 3 runs a WS man-in-the-middle that corrupts every
certificate on validator-0's `consensus` plane; a case sharing the stack afterwards would be
following a feed somebody is deliberately poisoning.
"""

from __future__ import annotations

from . import asserts_follow, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-cert-follow", [asserts_follow.assert_cert_follow], argv,
                      overlays=(asserts_follow.CERT_FOLLOW_OVERLAY,))
