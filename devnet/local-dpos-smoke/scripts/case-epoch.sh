#!/usr/bin/env bash
# smoke-epoch (standalone): bring up its own DPoS stack and run the assert_epoch
# body (chain crosses >= EPOCH_MIN_CROSS boundaries + 1 blk/s pacing). The
# assertion is read-only w.r.t. consensus and lives in asserts.sh, shared with
# case-base.sh. Respects EPOCH_MIN_CROSS (default 1).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
# shellcheck source=asserts.sh
source "$(dirname "$0")/asserts.sh"

bring_up_dpos          # sets PREV_FIN (anchor, hex); chain already past the anchor
trap tear_down EXIT
assert_epoch
