#!/usr/bin/env bash
# smoke-crash-survivor (standalone, Problem A): bring up its own DPoS stack and run
# assert_crash_survivor (SIGKILL a validator ungracefully, let the chain build an EL
# gap, restart it, assert it recovers + realigns instead of wedging on a missing
# block). The body is destructive and lives in asserts-fault.sh, shared with
# case-fault.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
# shellcheck source=asserts-fault.sh
source "$(dirname "$0")/asserts-fault.sh"

bring_up_dpos
trap tear_down EXIT
assert_crash_survivor
