#!/usr/bin/env bash
# smoke-byzantine: validator-3 equivocates (double-signs Notarizes via commonware's
# Conflicter, behind the devnet-byzantine build feature + FLUENT_DPOS_BYZANTINE env).
# The honest quorum (3/4) keeps finalizing, and the offending validator is slashed
# on-chain → jailed (ValidatorStatus.Jail == 3). There is no public tombstoned()
# getter, so we assert via getValidatorStatus (Addendum D).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

export DPOS_EXTRA_COMPOSE="-f docker-compose.byzantine.yml"
export DPOS_CONVERGE_EXCLUDE="validator-3"   # byzantine node's reth never finalizes

bring_up_dpos          # honest quorum converges past the anchor (validator-3 excluded)
trap tear_down EXIT

VIC=$(docker compose exec -T validator-0 jq -r '.validators[3]' /runtime/addresses.json | tr -d '[:space:]')
echo "smoke-byzantine: validator-3 ($VIC) equivocating; honest quorum live (anchor=$PREV_FIN)"

# Poll for the on-chain equivocation slash → Jail (status field, index 1 of the tuple).
deadline=$(( $(date +%s) + 200 ))
status=""
while (( $(date +%s) < deadline )); do
    status=$(staking_call \
        "getValidatorStatus(address)(address,uint8,uint256,uint32,uint64,uint64,uint64,uint16,uint96)" \
        "$VIC" 2>/dev/null | sed -n '2p' | tr -d ' ' || true)
    if [[ "$status" == "3" ]]; then
        echo "OK (smoke-byzantine): validator-3 jailed (status=Jail) by equivocation slashing; honest chain continued"
        exit 0
    fi
    sleep 3
done
echo "FAIL (smoke-byzantine): validator-3 not jailed within 200s (getValidatorStatus.status=$status)"
docker compose logs --tail=200 validator-0 validator-1
exit 1
