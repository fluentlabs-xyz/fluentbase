#!/usr/bin/env bash
# smoke-vrf-boundary (standalone): bring up its own DPoS stack and run the
# assert_vrf_boundary body (beacon survives the first epoch boundary on a stable
# committee; F1/F2). The assertion is read-only w.r.t. consensus and lives in
# asserts.sh, shared with case-base.sh. Heavy (~3-4 min: it must produce blocks
# past the boundary); NOT in run-all on its own (case-base covers it).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
# shellcheck source=asserts.sh
source "$(dirname "$0")/asserts.sh"

bring_up_dpos
trap tear_down EXIT
assert_vrf_boundary
