#!/usr/bin/env bash
# smoke-vrf: the threshold randomness beacon drives prev_randao under DPoS.
#
# Proves the live VRF path end to end:
#   - validators produce blocks whose prev_randao = H(threshold seed), where the
#     seed is VERIFIED against the bootstrapped PK_epoch — the "beacon active"
#     log line fires ONLY on assurance=true (a verified threshold seed), never on
#     the digest fallback (derive.rs::resolve_prev_randao);
#   - prev_randao (mixHash) is non-zero AND VARIES per block (real randomness —
#     a stuck/constant value would still converge, so convergence alone is not
#     enough; this catches it);
#   - the beacon is SUSTAINED (verified seed across many blocks, not a one-off);
#   - a non-validator FOLLOWER (full-node) carries the same non-zero prev_randao
#     (the beacon randomness propagates through import, not just to signers);
#   - all nodes converge — every deriving node recovered the SAME unique
#     threshold seed (else derived hashes diverge from the committee-attested
#     `result` and the chain stalls): a determinism check.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

# mixHash (prev_randao) of block $1 as seen by RPC $2 (default $RPC).
mixhash_at() { cast block "$1" --rpc-url "${2:-$RPC}" --json | jq -r .mixHash; }
is_zero_hash() { [[ "$1" =~ ^0x0+$ ]]; }
# Count log lines matching $2 in service $1 (0 on no match — never trips set -e).
log_count() { docker compose logs "$1" 2>/dev/null | grep -c "$2" || true; }

bring_up_dpos
trap tear_down EXIT

fin=$(finalized_dec)
(( fin > 0 )) || { echo "FAIL (smoke-vrf): no finalized block"; exit 1; }

# 1) A window of finalized blocks: every prev_randao is non-zero AND they are all
#    distinct (the threshold seed is unique per height → randomness varies).
WINDOW=6
lo=$(( fin > WINDOW ? fin - WINDOW + 1 : 1 ))
echo "smoke-vrf: DPoS up, finalized=$fin; checking prev_randao over blocks [$lo..$fin]"
mixes=()
for ((n = lo; n <= fin; n++)); do
    mh=$(mixhash_at "$n")
    if is_zero_hash "$mh"; then
        echo "FAIL (smoke-vrf): prev_randao is zero at finalized block $n"; exit 1
    fi
    mixes+=("$mh")
done
distinct=$(printf '%s\n' "${mixes[@]}" | sort -u | wc -l)
if (( distinct != ${#mixes[@]} )); then
    echo "FAIL (smoke-vrf): prev_randao not varying — ${#mixes[@]} blocks [$lo..$fin] but only $distinct distinct (stuck randomness)"
    printf '  %s\n' "${mixes[@]}"
    exit 1
fi
echo "smoke-vrf: ${#mixes[@]} blocks, ${#mixes[@]} distinct non-zero prev_randao — varies as expected"

# 2) Each validator logged threshold-verified prev_randao (assurance=true) — the
#    beacon path against PK_epoch, NOT the digest fallback — and SUSTAINED across
#    at least MIN_BLOCKS blocks (not a one-off / intermittent).
MIN_BLOCKS=5
for v in validator-0 validator-1 validator-2 validator-3; do
    c=$(log_count "$v" "beacon: threshold prev_randao active")
    c=${c:-0}
    if (( c < MIN_BLOCKS )); then
        echo "FAIL (smoke-vrf): $v logged threshold prev_randao only $c times (< $MIN_BLOCKS) — beacon inactive/intermittent/fell back to digest"
        docker compose logs --tail=80 "$v"
        exit 1
    fi
    echo "smoke-vrf: $v — threshold prev_randao active x$c"
done

# 3) The follower (full-node, host port 18545) carries the SAME prev_randao at a
#    finalized height it has imported — beacon randomness reaches non-signers.
#    Probe the LATEST finalized height (post-migration → a real beacon block),
#    NOT lo (which may predate the DPoS anchor and carry Tempo-era randomness).
#    NOTE: step 1's non-zero+distinct check is a liveness/sanity guard only — the
#    digest fallback is also non-zero and per-height-distinct, so the
#    threshold-beacon-vs-fallback discriminator is step 2's assurance=true log.
FN_RPC="http://localhost:18545"
probe=$fin
fn_mix=$(mixhash_at "$probe" "$FN_RPC")
v0_mix=$(mixhash_at "$probe" "$RPC")
if is_zero_hash "$fn_mix" || [[ "$fn_mix" != "$v0_mix" ]]; then
    echo "FAIL (smoke-vrf): follower prev_randao at block $probe = $fn_mix != validator-0 $v0_mix (beacon randomness not propagated to follower)"
    exit 1
fi

# 4) PK_epoch is PUBLISHED ON-CHAIN (commitEpochBeaconKey at genesis) and equals
#    the group key recovered seeds verify against — the trustless source the STF
#    reads (research #2). Compare on-chain getEpochBeaconKey(0) to beacon-pk.hex.
pk_file=$(docker compose exec -T validator-0 cat /runtime/keys/beacon-pk.hex | tr -d '[:space:]')
pk_chain=$(cast call "$STAKING_ADDR" "getEpochBeaconKey(uint64)(bytes)" 0 --rpc-url "$RPC" | tr -d '[:space:]')
if [[ -z "$pk_file" || "$pk_chain" != "0x$pk_file" ]]; then
    echo "FAIL (smoke-vrf): on-chain getEpochBeaconKey(0)=$pk_chain != published PK_epoch 0x$pk_file"
    exit 1
fi
echo "smoke-vrf: PK_epoch on-chain (getEpochBeaconKey(0)) matches the seed-verifying group key"

echo "OK (smoke-vrf): threshold-beacon prev_randao active+sustained on all 4 validators (>=$MIN_BLOCKS blocks), non-zero and ${#mixes[@]}/${#mixes[@]} distinct across [$lo..$fin], follower agrees at block $probe, PK_epoch published+matched on L2, all nodes converged (deterministic seed)"
