#!/usr/bin/env bash
# smoke-peers (standalone): bring up its own DPoS stack and run assert_peers (both
# peer planes connect + a restarted validator re-establishes both + chain advances).
# The body is destructive (restarts a validator) and lives in asserts-fault.sh,
# shared with case-fault.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
# shellcheck source=asserts-fault.sh
source "$(dirname "$0")/asserts-fault.sh"

bring_up_dpos
trap tear_down EXIT
assert_peers
