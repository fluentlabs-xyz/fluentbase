#!/usr/bin/env bash
# smoke-deferred: pins the deferred-execution (F-type) observables that the
# convergence-based cases cannot see — they only require cross-node EQUALITY,
# so a uniform finality overclaim (e.g. finalized == latest on every node)
# keeps all of them green:
#   1. K-lag invariant: eth "finalized" trails "latest" by exactly K in steady
#      state (never less — less = result-finality overclaim), and the
#      consensus namespace agrees (latestFinalized.height −
#      latestResultFinalized == K, latestResultFinalized == eth finalized).
#   2. result-commitment integrity: the ordering artifact at height N+K
#      carries `result` == the derived EVM block hash at N. Decoded from the
#      consensus_getFinalization wire bytes at the fixed codec offset
#      (parent 32 + height 8 + timestamp 8 + fee_recipient 20 + gas_limit 8
#      = byte 76; OrderBlock field order is part of the wire format).
#   3. EL-slowed validator: CPU-throttling one validator must not stall the
#      chain (verify budget → nullify, BFT f=1 holds); after unthrottle the
#      victim catches back up to the live tip.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

K="${RESULT_LAG_K:-3}" # mirrors fluentbase_consensus::K
RPC_URL="http://localhost:8545"

rpc() { # rpc <method> <params-json>
    curl -s -X POST -H 'Content-Type: application/json' \
        --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":$2}" \
        "$RPC_URL"
}

block_number_of() { # block_number_of <tag> → decimal height
    printf '%d' "$(rpc eth_getBlockByNumber "[\"$1\",false]" | jq -r '.result.number')"
}

bring_up_dpos
trap tear_down EXIT

# Steady state: let the chain move well past the anchor + result window so the
# pre-K ramp (finalized clamped to the anchor) cannot skew the lag samples.
base=$(baseline_height)
wait_finalized_ge $(( base + K + 5 )) 90 || { echo "FAIL (smoke-deferred): chain did not reach steady state past $((base + K + 5))"; exit 1; }

# ── 1. K-lag invariant ──────────────────────────────────────────────────────
# 6 samples: lag < K in any sample = overclaim (hard fail); the chain advances
# ~1 block/s so a sample can straddle an FCU — accept K..K+1 — but require the
# exact K at least once (liveness half: the lag must not drift wide).
saw_exact=0
for _ in 1 2 3 4 5 6; do
    latest=$(block_number_of latest)
    final=$(block_number_of finalized)
    lag=$(( latest - final ))
    (( lag >= K )) || { echo "FAIL (smoke-deferred): finalized overclaims — lag=$lag < K=$K (latest=$latest finalized=$final)"; exit 1; }
    (( lag <= K + 1 )) || { echo "FAIL (smoke-deferred): finality lag drifted — lag=$lag > K+1 (latest=$latest finalized=$final)"; exit 1; }
    (( lag == K )) && saw_exact=1
    sleep 2
done
(( saw_exact == 1 )) || { echo "FAIL (smoke-deferred): lag never sampled at exactly K=$K"; exit 1; }
echo "  K-lag (eth): latest − finalized == $K held across 6 samples"

# Consensus namespace must tell the same story as the eth tags. One snapshot
# is two RPCs apart from the eth read, so allow ±1 skew on the cross-check but
# require the namespace-internal arithmetic to be exact.
cons=$(rpc consensus_getLatest "[]")
cons_fin=$(jq -r '.result.latestFinalized.height' <<<"$cons")
cons_res=$(jq -r '.result.latestResultFinalized' <<<"$cons")
[[ "$cons_fin" != "null" && "$cons_res" != "null" ]] || { echo "FAIL (smoke-deferred): consensus_getLatest incomplete: $cons"; exit 1; }
(( cons_fin - cons_res == K )) || { echo "FAIL (smoke-deferred): consensus tiers disagree — latestFinalized=$cons_fin latestResultFinalized=$cons_res (want gap $K)"; exit 1; }
eth_final=$(block_number_of finalized)
delta=$(( eth_final - cons_res )); (( delta < 0 )) && delta=$(( -delta ))
(( delta <= 1 )) || { echo "FAIL (smoke-deferred): eth finalized=$eth_final vs latestResultFinalized=$cons_res (skew > 1)"; exit 1; }
echo "  K-lag (consensus): latestFinalized=$cons_fin − latestResultFinalized=$cons_res == $K, matches eth finalized=$eth_final"

# ── 2. result-commitment integrity ──────────────────────────────────────────
# The artifact at N+K commits the derived hash of N in its `result` field.
N=$cons_res
artifact=$(rpc consensus_getFinalization "[{\"height\":$(( N + K ))}]")
wire=$(jq -r '.result.block' <<<"$artifact"); wire=${wire#0x}
[[ -n "$wire" && "$wire" != "null" ]] || { echo "FAIL (smoke-deferred): no ordering artifact at $((N + K)): $artifact"; exit 1; }
# fixed-offset slice into the OrderBlock codec (layout documented above) —
# guard the length so codec drift fails loudly instead of slicing garbage.
(( ${#wire} >= 216 )) || { echo "FAIL (smoke-deferred): artifact wire too short (${#wire} hex chars) — OrderBlock codec layout changed?"; exit 1; }
committed_result=${wire:152:64}
derived_hash=$(rpc eth_getBlockByNumber "[\"$(printf '0x%x' "$N")\",false]" | jq -r '.result.hash'); derived_hash=${derived_hash#0x}
[[ "${committed_result,,}" == "${derived_hash,,}" ]] || { echo "FAIL (smoke-deferred): result commitment mismatch at h=$((N + K)) — artifact result=$committed_result, derived hash($N)=$derived_hash"; exit 1; }
echo "  result commitment: artifact($((N + K))).result == eth hash($N) == 0x${derived_hash:0:16}…"

# ── 3. EL-slowed validator ──────────────────────────────────────────────────
# Throttle validator-1's CPU hard for ~1.5 epochs: its verify gate starts
# timing out (EL backpressure → nullify) but BFT f=1 must keep the chain
# finalizing. Afterwards the victim must rejoin the live tip.
victim=validator-1
cid=$(docker compose ps -q "$victim")
[[ -n "$cid" ]] || { echo "FAIL (smoke-deferred): no container for $victim"; exit 1; }
pre_throttle=$(finalized_dec)
echo "  throttling $victim to 0.15 cpu (pre=$pre_throttle)"
docker update --cpus "0.15" "$cid" >/dev/null
sleep 45
during=$(finalized_dec)
docker update --cpus "4" "$cid" >/dev/null
(( during >= pre_throttle + 20 )) || { echo "FAIL (smoke-deferred): chain stalled under one slowed EL — finalized $pre_throttle → $during in 45s (want +20)"; exit 1; }
echo "  chain stayed live under throttle: finalized $pre_throttle → $during"

# Rejoin: the victim's own finalized view must reach the network tip observed
# at unthrottle time (and keep moving with it).
deadline=$(( $(date +%s) + 180 ))
while (( $(date +%s) < deadline )); do
    vfin=$(check_node docker compose exec -T "$victim" | cut -d'|' -f1)
    [[ "$vfin" != "null" ]] && (( $(printf '%d' "$vfin") >= during )) && {
        echo "OK (smoke-deferred): K-lag invariant + result commitment + EL-slowed liveness (victim rejoined at $vfin >= $during)"
        exit 0
    }
    sleep 3
done
echo "FAIL (smoke-deferred): $victim did not rejoin after unthrottle (victim=$(check_node docker compose exec -T "$victim"), v0=$(check_external 8545))"
exit 1
