"""smoke-fault — `scripts/case-fault.sh`: the recoverable DESTRUCTIVE suite on ONE bring-up.

deferred + peers + vrf-fault + crash-survivor + full-restart each MUTATE the stack — CPU-throttle,
restart, stop, SIGKILL — but each RESTORES it to a healthy, realigned state before returning, and
its terminal assertion IS that recovery check. So they chain on a single sequencer→DPoS bring-up
instead of paying for five, with fail-fast: a case hands off to the next only because its own
recovery passed. Each stays runnable standalone (`smoke-deferred`, `smoke-peers`, …) with its own
bring-up, for isolated debugging when one of them fails here.

THE ORDER IS LEAST → MOST INVASIVE, and it is not a preference:

    deferred        needs a PRISTINE steady state for the K-lag invariant, so it runs before any
                    node has been restarted or stopped. Moving it later measures a chain that a
                    previous case disturbed.
    peers           a graceful restart of one validator.
    vrf-fault       stop + restart one validator (the beacon survives on n−f).
    crash-survivor  an ungraceful SIGKILL + recovery of one validator.
    full-restart    stop + restart the ENTIRE validator set. Nothing can follow it.

`smoke-liveness` is deliberately EXCLUDED. Its kill/rejoin cycles can push the miss-count to a
JAIL, which permanently shrinks the committee and is unrecoverable — it must stay an isolated
stack. `smoke-vrf-dkg-liveness` is excluded for a different reason: it needs a DKG window that
opens once, near bring-up, and by the time a chain of five cases reached it the window is gone.

Heavy (~25-30 min on one stack).
"""

from __future__ import annotations

from . import asserts_fault, driver

ASSERTIONS = [asserts_fault.assert_deferred, asserts_fault.assert_peers,
              asserts_fault.assert_vrf_fault, asserts_fault.assert_crash_survivor,
              asserts_fault.assert_full_restart]


def run_case(argv=None) -> int:
    rc = driver.run("smoke-fault", ASSERTIONS, argv)
    if rc == 0 and "--dry-run" not in list(argv or []):
        print("OK (smoke-fault): deferred + peers + vrf-fault + crash-survivor + full-restart "
              "all passed on one bring-up", flush=True)
    return rc
