#!/usr/bin/env bash
# smoke-deferred (standalone): bring up its own DPoS stack and run assert_deferred
# (K-lag invariant + result-commitment integrity + EL-slowed-validator liveness).
# The body is destructive (CPU-throttles a validator, then restores) and lives in
# asserts-fault.sh, shared with case-fault.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
# shellcheck source=asserts-fault.sh
source "$(dirname "$0")/asserts-fault.sh"

bring_up_dpos
trap tear_down EXIT
assert_deferred
