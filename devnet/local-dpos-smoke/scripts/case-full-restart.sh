#!/usr/bin/env bash
# smoke-full-restart (standalone): bring up its own DPoS stack and run
# assert_full_restart (stop ALL validators, verify each persisted/exit-0, restart,
# assert the network reconverges from the persisted finalized head). The body is
# destructive (stops the whole validator set) and lives in asserts-fault.sh, shared
# with case-fault.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
# shellcheck source=asserts-fault.sh
source "$(dirname "$0")/asserts-fault.sh"

bring_up_dpos
trap tear_down EXIT
assert_full_restart
