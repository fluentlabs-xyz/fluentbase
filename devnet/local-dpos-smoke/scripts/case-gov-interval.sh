#!/usr/bin/env bash
# smoke-gov-interval: a governance change to epochBlockInterval (32 → 64) takes effect
# on the next finalized block (the node re-reads it per block) and the chain stays live
# across the new boundary cadence. Config-only NOW-tier case.
#
# PARKED (2026-06-01): descoped from the default suite. `ChainConfig.setEpochBlockInterval`
# is `onlyFromGovernance` — callable ONLY by the FluentGovernance contract (0x5205), not
# an EOA. The direct `cast send` below reverts `OnlyGovernance(0x54348f03)`. A real change
# requires the full OZ-Governor lifecycle (propose → validator castVote → execute) plus
# exposing each validator's L2 voting key from genesis-bootstrap. Re-enable by
# implementing that flow; until then this script is left as the record of intent.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

NEW_INTERVAL=64   # must stay a multiple of WARMUP_DELAY=2

bring_up_dpos
trap tear_down EXIT

GOV=$(docker compose exec -T validator-0 cat /runtime/keys/governance.hex | tr -d '[:space:]')

before=$(chainconfig_call "getEpochBlockInterval()(uint32)" 2>/dev/null || chainconfig_call "epochBlockInterval()(uint32)")
echo "smoke-gov-interval: current interval=$before → setting $NEW_INTERVAL"

cast send --private-key "0x$GOV" --rpc-url "$RPC" --chain "$CHAIN_ID" \
    "$CHAIN_CONFIG_ADDR" "setEpochBlockInterval(uint32)" "$NEW_INTERVAL" >/dev/null

# assert the on-chain value updated, and the chain keeps finalizing afterward.
deadline=$(( $(date +%s) + 60 ))
while (( $(date +%s) < deadline )); do
    now=$(chainconfig_call "getEpochBlockInterval()(uint32)" 2>/dev/null || chainconfig_call "epochBlockInterval()(uint32)")
    [[ "$now" == "$NEW_INTERVAL" ]] && break
    sleep 2
done
now=$(chainconfig_call "getEpochBlockInterval()(uint32)" 2>/dev/null || chainconfig_call "epochBlockInterval()(uint32)")
[[ "$now" == "$NEW_INTERVAL" ]] || { echo "FAIL (smoke-gov-interval): interval=$now != $NEW_INTERVAL"; exit 1; }

pre=$(finalized_dec)
deadline=$(( $(date +%s) + 60 ))
while (( $(date +%s) < deadline )); do (( $(finalized_dec) >= pre + 3 )) && break; sleep 2; done
(( $(finalized_dec) >= pre + 3 )) || { echo "FAIL (smoke-gov-interval): chain stalled after interval change"; exit 1; }

echo "OK (smoke-gov-interval): interval $before→$NEW_INTERVAL applied; chain finalized past $((pre+3))"
