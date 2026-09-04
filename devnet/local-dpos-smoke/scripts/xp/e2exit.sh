#!/usr/bin/env bash
# E2: the NON-byzantine road to the same halt — a validator leaves on its own,
# `undelegate(validator, self_stake)` signed with its own owner key.
# No equivocation, no governance: an ordinary permissionless transaction.
set -euo pipefail
N=${1:-4}; IDX=${2:-3}
RPC="http://localhost:$((28000+N))"
STAKING=0x0000000000000000000000000000000000520011
V=$(docker exec "xp$N-validator-0-1" sh -c 'cat /runtime/addresses.json' | jq -r ".validators[$IDX]")
KEY=0x$(docker exec "xp$N-validator-0-1" cat "/runtime/keys/owner-$IDX.hex")
OWNER=$(cast wallet address --private-key "$KEY")
AMT=$(cast call --rpc-url "$RPC" $STAKING 'getValidatorDelegation(address,address)(uint256)' "$V" "$OWNER" | awk '{print $1}')
echo "validator[$IDX] = $V   owner = $OWNER   self-stake = $AMT"
echo "block before: $(cast block-number --rpc-url "$RPC")"
cast send --rpc-url "$RPC" --private-key "$KEY" --legacy \
  $STAKING 'undelegate(address,uint256)' "$V" "$AMT" 2>&1 | grep -E 'status|blockNumber|transactionHash|Error|error' | head -5
echo "block after: $(cast block-number --rpc-url "$RPC")"
