#!/usr/bin/env bash
# smoke-vrf (standalone): bring up its own DPoS stack and run the assert_vrf body
# (threshold randomness beacon drives prev_randao end-to-end; C1/C2/D1/E1). The
# assertion is read-only w.r.t. consensus (it only deploys a throwaway probe) and
# lives in asserts.sh, shared with case-base.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
# shellcheck source=asserts.sh
source "$(dirname "$0")/asserts.sh"

bring_up_dpos
trap '[ -n "${SMOKE_KEEP_UP:-}" ] || tear_down' EXIT
assert_vrf
