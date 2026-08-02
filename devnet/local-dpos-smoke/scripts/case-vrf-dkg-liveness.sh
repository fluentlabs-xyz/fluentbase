#!/usr/bin/env bash
# smoke-vrf-dkg-liveness (standalone): bring up its own DPoS stack and run
# assert_vrf_dkg_liveness — the DKG-liveness NEGATIVE edge (NO reshare). A committee
# member taken OFFLINE during its epoch-2 DKG window misses the ceremony, is shareless
# for that epoch, and SITS OUT the seed quorum while the chain stays live on the
# remaining n−f survivors; on restart it re-derives prev_randao from the cert seed.
#
# The victim MUST be stopped BEFORE the epoch-2 DKG window opens
# (epoch_start(2) − DKG_MARGIN_BLOCKS), so the assertion stops it immediately after
# bring-up (the DPoS swap lands near the activation block, well below the window).
# Heavy (~4-5 min).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
# shellcheck source=asserts-fault.sh
source "$(dirname "$0")/asserts-fault.sh"

bring_up_dpos
trap tear_down EXIT
assert_vrf_dkg_liveness
