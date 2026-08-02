#!/usr/bin/env bash
# smoke-vrf-fault (standalone): bring up its own DPoS stack and run assert_vrf_fault
# (beacon survives an f=1 fault on n−f survivors; the downed node restarts, reloads
# its share, and catches up the gap with verified prev_randao; A1/B3/B4). The body
# is destructive (stops + restarts a validator) and lives in asserts-fault.sh,
# shared with case-fault.sh. Heavy (~4-5 min).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
# shellcheck source=asserts-fault.sh
source "$(dirname "$0")/asserts-fault.sh"

bring_up_dpos
trap tear_down EXIT
assert_vrf_fault
