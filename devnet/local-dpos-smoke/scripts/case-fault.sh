#!/usr/bin/env bash
# smoke-fault: the recoverable DESTRUCTIVE default-stack suite on ONE bring-up.
#
# deferred + peers + vrf-fault + crash-survivor + full-restart each MUTATE the
# stack (CPU-throttle / restart / stop / SIGKILL nodes) but each RESTORES it to a
# healthy, realigned state before returning. So they chain on a single Tempo→DPoS
# bring-up instead of paying for five, with fail-fast: a case hands off to the next
# only if its own recovery assertion passed. Each remains a standalone
# `make smoke-<case>` (same assert_* function in asserts-fault.sh, its own
# bring-up) for isolated debugging when one fails here.
#
# Order = least → most invasive (and K-lag-sensitive first):
#   1. deferred       — needs a pristine steady state for the K-lag invariant, so
#                       it runs before any node has been disturbed.
#   2. peers          — graceful restart of one validator.
#   3. vrf-fault      — stop + restart one validator (beacon survives n−f).
#   4. crash-survivor — ungraceful SIGKILL + recovery of one validator.
#   5. full-restart   — stop + restart the ENTIRE validator set (most invasive).
#
# `case-liveness` is deliberately EXCLUDED: its kill/rejoin cycles can JAIL a
# validator, permanently shrinking the committee (unrecoverable) — it stays an
# isolated stack. Heavy (~25-30 min on one stack).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
# shellcheck source=asserts-fault.sh
source "$(dirname "$0")/asserts-fault.sh"

bring_up_dpos
trap tear_down EXIT

assert_deferred
assert_peers
assert_vrf_fault
assert_crash_survivor
assert_full_restart

echo "OK (smoke-fault): deferred + peers + vrf-fault + crash-survivor + full-restart all passed on one bring-up"
