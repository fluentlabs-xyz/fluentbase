#!/usr/bin/env bash
# smoke-epoch: the chain crosses >= EPOCH_MIN_CROSS epoch boundary(ies) after the
# DPoS swap. This is the permanent regression guard for the epoch-boundary handoff
# deadlock — the canonical smoke's "head > anchor" threshold can pass entirely
# inside epoch 0 and would hide a boundary regression.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

EPOCH_MIN_CROSS="${EPOCH_MIN_CROSS:-1}"

bring_up_dpos          # sets PREV_FIN (anchor, hex); chain already past the anchor
trap tear_down EXIT

PREV_DEC=$(printf '%d' "$PREV_FIN")
TARGET=$(( ((PREV_DEC / EPOCH_INTERVAL) + EPOCH_MIN_CROSS + 1) * EPOCH_INTERVAL ))
echo "smoke-epoch: anchor=$PREV_DEC; require finalized >= $TARGET (cross $EPOCH_MIN_CROSS boundary(ies), interval=$EPOCH_INTERVAL)"

deadline=$(( $(date +%s) + 220 ))
while (( $(date +%s) < deadline )); do
    v0=$(check_external 8545); v1=$(check_node docker compose exec -T validator-1)
    v2=$(check_node docker compose exec -T validator-2); v3=$(check_node docker compose exec -T validator-3)
    fn=$(check_external 18545); head="${v0%%|*}"
    if [[ "$head" != "null" && "$head" != "0x0" \
          && "$v0" == "$v1" && "$v1" == "$v2" && "$v2" == "$v3" && "$v3" == "$fn" ]]; then
        hd=$(printf '%d' "$head")
        if (( hd >= TARGET )); then
            cur=$(staking_call "currentEpoch()(uint256)")
            comm=$(staking_call "getEpochCommittee(uint64)(address[])" "$cur")
            [[ -n "$comm" && "$comm" != "[]" ]] || { echo "FAIL (smoke-epoch): getEpochCommittee($cur) empty"; exit 1; }
            echo "OK (smoke-epoch): all 5 aligned finalized=$hd >= $TARGET (epoch $cur), committee non-empty"
            exit 0
        fi
    fi
    sleep 2
done
echo "FAIL (smoke-epoch): did not reach finalized >= $TARGET within 220s"
docker compose logs --tail=200
exit 1
