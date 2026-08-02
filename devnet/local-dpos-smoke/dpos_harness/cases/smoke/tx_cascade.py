"""smoke-tx-cascade (standalone) — `scripts/case-tx-cascade.sh`.

Its own bring-up under the `tx-cascade` compose overlay, then the transaction write path across
the L1→L2→L3 sentry cascade.

Standalone because it MUTATES a running node's peer policy: it calls `admin_addTrustedPeer` on
the sentry so the sentry accepts L3's inbound connection. Nothing undoes that, and the next case
on a shared stack would be measuring a topology this one widened.
"""

from __future__ import annotations

from . import asserts_follow, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-tx-cascade", [asserts_follow.assert_tx_cascade], argv,
                      overlays=(asserts_follow.TX_CASCADE_OVERLAY,))
