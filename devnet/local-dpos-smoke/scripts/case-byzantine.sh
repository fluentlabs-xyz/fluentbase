#!/usr/bin/env bash
# smoke-byzantine: validator-3 equivocates (double-signs Notarize/Finalize votes via
# the vendored VoteEquivocator, behind the dpos-devnet-byzantine build feature +
# FLUENT_DPOS_BYZANTINE=equivocate env). The honest quorum (3/4, f=1) keeps
# finalizing, and the offending validator is slashed on-chain → jailed
# (ValidatorStatus.Jail == 3). There is no public tombstoned() getter, so we assert
# via getValidatorStatus (Addendum D). After the jail we re-check that the honest
# chain KEEPS advancing (a post-jail / committee-drop wedge is a real DPoS class).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

export DPOS_EXTRA_COMPOSE="-f docker-compose.byzantine.yml"
export DPOS_CONVERGE_EXCLUDE="validator-3"   # byzantine node's reth never finalizes

bring_up_dpos          # honest quorum converges past the anchor (validator-3 excluded)
trap tear_down EXIT

# `cat` in the container, `jq` on the host (the validator image has no jq — running
# jq in-container is a 127). Mirrors case-liveness.sh / lib.sh::pp_owner_addr.
VIC=$(docker compose exec -T validator-0 cat /runtime/addresses.json | jq -r '.validators[3]' | tr -d '[:space:]')
echo "smoke-byzantine: validator-3 ($VIC) equivocating; honest quorum live (anchor=$PREV_FIN)"

# Poll for the on-chain equivocation slash → Jail (status field, index 1 of the tuple).
deadline=$(( $(date +%s) + 200 ))
status=""
while (( $(date +%s) < deadline )); do
    status=$(staking_call \
        "getValidatorStatus(address)(address,uint8,uint256,uint32,uint64,uint64,uint64,uint16,uint96)" \
        "$VIC" 2>/dev/null | sed -n '2p' | tr -d ' ' || true)
    if [[ "$status" == "3" ]]; then
        echo "smoke-byzantine: validator-3 jailed (status=Jail) by equivocation slashing"
        break
    fi
    sleep 3
done
if [[ "$status" != "3" ]]; then
    echo "FAIL (smoke-byzantine): validator-3 not jailed within 200s (getValidatorStatus.status=$status)"
    # Dump the EQUIVOCATOR first (it is the diagnostic-relevant node): did it take
    # the Equivocate path, can its scheme sign, is it broadcasting conflicting votes?
    echo "--- validator-3 (equivocator) byzantine markers ---"
    docker compose logs --tail=600 validator-3 2>&1 \
        | grep -iE "BYZANTINE|equivocat|cannot sign|no local share|decode vote|broadcast" || true
    echo "--- validator-0/1 slasher / conflict markers ---"
    docker compose logs --tail=600 validator-0 validator-1 2>&1 \
        | grep -iE "slash|conflict|equivocat|evidence" || true
    echo "--- validator-0 tail ---"; docker compose logs --tail=120 validator-0
    exit 1
fi

# Liveness after the jail: the honest 3-of-4 quorum must KEEP finalizing once the
# equivocator is dropped (guards against a post-jail / committee-drop epoch-boundary
# wedge — a real DPoS failure class, not covered by the pre-jail converge).
post_jail=$(baseline_height)
if wait_finalized_ge "$(( post_jail + 3 ))" 90; then
    echo "OK (smoke-byzantine): honest chain advanced past $post_jail after the jail (now $(finalized_dec))"
    exit 0
fi
echo "FAIL (smoke-byzantine): chain stalled after jail (finalized stuck at ~$post_jail)"
docker compose logs --tail=200 validator-0 validator-1 validator-3
exit 1
