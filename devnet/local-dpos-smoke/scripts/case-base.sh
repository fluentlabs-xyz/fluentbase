#!/usr/bin/env bash
# smoke-base: the READ-ONLY default-stack suite on ONE bring-up.
#
# tx + epoch + vrf + vrf-boundary are all read-only w.r.t. consensus (none stops,
# restarts, or jails a node — at most they send txns or deploy a throwaway probe),
# so they cannot contaminate one another and share a single Tempo→DPoS bring-up
# instead of paying for four. Each assertion still exists as a standalone
# `make smoke-<case>` (same assert_* function in asserts.sh, its own bring-up) for
# isolated debugging.
#
# Order is fail-fast and "increasing sophistication": tx (EVM executes/finalizes)
# → epoch (boundary handoff + 1 blk/s pacing) → vrf (beacon end-to-end) →
# vrf-boundary (beacon across the epoch-1 boundary, by now historical). The first
# failing assertion `exit 1`s the whole run (the trap tears the stack down).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
# shellcheck source=asserts.sh
source "$(dirname "$0")/asserts.sh"

bring_up_dpos
trap tear_down EXIT

assert_tx
assert_epoch
assert_vrf
assert_vrf_boundary

echo "OK (smoke-base): tx + epoch + vrf + vrf-boundary all passed on one bring-up"
