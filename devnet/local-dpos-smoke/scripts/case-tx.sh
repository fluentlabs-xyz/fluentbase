#!/usr/bin/env bash
# smoke-tx: a value transfer AND a contract call execute + finalize under DPoS.
# A bare value transfer alone does not exercise the EVM execution path (no CALL/
# SSTORE), so the contract call (MockBlendToken.approve) is mandatory — it is what
# closes the brief's "EVM execution path under DPoS is unverified" gap.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

bring_up_dpos
trap tear_down EXIT

KEY=$(docker compose exec -T validator-0 cat /runtime/keys/funded.hex | tr -d '[:space:]')
FROM=$(cast wallet address --private-key "0x$KEY")
DEAD="0x000000000000000000000000000000000000dEaD"
BLEND="0x0000000000000000000000000000000000005207"   # MockBlendToken predeploy
ALLOW=12345

bal_before=$(cast balance "$DEAD" --rpc-url "$RPC")

# 1) value transfer (0.1 ETH — funded account holds 1 ETH; leave headroom for gas).
TXH=$(cast send --private-key "0x$KEY" --rpc-url "$RPC" --chain "$CHAIN_ID" \
        "$DEAD" --value 0.1ether --json | jq -r .transactionHash)

# 2) MANDATORY contract call — exercises EVM CALL + SSTORE.
CTXH=$(cast send --private-key "0x$KEY" --rpc-url "$RPC" --chain "$CHAIN_ID" \
        "$BLEND" "approve(address,uint256)" "$DEAD" "$ALLOW" --json | jq -r .transactionHash)

# 3) both receipts succeeded, and their blocks are finalized.
maxblk=0
for h in "$TXH" "$CTXH"; do
    st=$(cast receipt "$h" --rpc-url "$RPC" --json | jq -r .status)
    [[ "$st" == "0x1" || "$st" == "1" ]] || { echo "FAIL (smoke-tx): receipt $h status=$st"; exit 1; }
    blk=$(cast receipt "$h" --rpc-url "$RPC" --json | jq -r .blockNumber)
    blk=$(printf '%d' "$blk"); (( blk > maxblk )) && maxblk=$blk
done
# wait until both tx blocks are finalized
wait_finalized_ge "$maxblk" 60 || { echo "FAIL (smoke-tx): tx block $maxblk not finalized in time"; exit 1; }

# 4) state changed: recipient balance delta == 0.1 ETH AND allowance slot == ALLOW.
bal_after=$(cast balance "$DEAD" --rpc-url "$RPC")
delta=$(( bal_after - bal_before ))
[[ "$delta" == "100000000000000000" ]] || { echo "FAIL (smoke-tx): balance delta $delta != 0.1 ETH"; exit 1; }
# `cast call ...(uint256)` pretty-prints as "12345 [1.234e4]"; take the bare integer.
allow=$(cast call "$BLEND" "allowance(address,address)(uint256)" "$FROM" "$DEAD" --rpc-url "$RPC" | awk '{print $1}')
[[ "$allow" == "$ALLOW" ]] || { echo "FAIL (smoke-tx): allowance=$allow != $ALLOW (EVM SSTORE not applied)"; exit 1; }

echo "OK (smoke-tx): value transfer + MockBlendToken.approve executed, finalized, and state applied under DPoS"
