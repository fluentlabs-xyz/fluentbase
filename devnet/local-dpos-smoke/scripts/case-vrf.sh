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
#   - every block's prev_randao is BYTE-IDENTICAL across all 4 validators AND the
#     import-follower (full-node) — every deriving node recovered the SAME unique
#     threshold seed, checked at EVERY height in the window (a single node
#     deriving a divergent seed is caught here; the prior single-node /
#     single-height checks could miss it);
#   - the beacon is SUSTAINED AND ACTIVE AT PROBE TIME — each validator's
#     assurance=true count GROWS as the chain advances, so a beacon that fell to
#     the digest fallback after warm-up (frozen count under live blocks) fails,
#     which the static >=MIN_BLOCKS count alone cannot see;
#   - the COMPUTED beacon value lands on-chain — every recently logged
#     `prev_randao=H(seed)` equals the actual mixHash of a finalized block (step
#     1 reads the header, step 2 reads the log; this ties the two together);
#   - PK_epoch is published on-chain and equals the seed-verifying group key.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

# mixHash (prev_randao) of block $1 as seen by RPC $2 (default $RPC), lowercased.
mixhash_at() { cast block "$1" --rpc-url "${2:-$RPC}" --json | jq -r .mixHash | tr 'A-F' 'a-f'; }
# mixHash of block $2 (decimal) as seen INSIDE container service $1 — for the
# validators that expose no host RPC port (same in-container probe as lib.sh's
# _read_tempo_nodes). "null" when the block is absent / RPC unreachable.
mixhash_in() {
    local hexn
    hexn=$(printf '0x%x' "$2")
    docker compose exec -T "$1" curl -s -X POST -H 'Content-Type: application/json' \
        --data "{\"jsonrpc\":\"2.0\",\"method\":\"eth_getBlockByNumber\",\"params\":[\"$hexn\",false],\"id\":1}" \
        http://localhost:8545 2>/dev/null | jq -r '.result.mixHash // "null"' | tr 'A-F' 'a-f'
}
# mixHash of block $2 (decimal) as seen by node service $1 — routes to the host
# RPC for the two services that publish one, else the in-container probe.
mixhash_of() {
    case "$1" in
        validator-0) mixhash_at "$2" ;;
        full-node)   mixhash_at "$2" "http://localhost:18545" ;;
        *)           mixhash_in "$1" "$2" ;;
    esac
}
is_zero_hash() { [[ "$1" =~ ^0x0+$ ]]; }
# Count log lines matching $2 in service $1 (0 on no match — never trips set -e).
log_count() { docker compose logs "$1" 2>/dev/null | grep -c "$2" || true; }

NODES=(validator-0 validator-1 validator-2 validator-3 full-node)
VALIDATORS=(validator-0 validator-1 validator-2 validator-3)

bring_up_dpos
trap tear_down EXIT

fin=$(finalized_dec)
(( fin > 0 )) || { echo "FAIL (smoke-vrf): no finalized block"; exit 1; }

# 1) A window of finalized blocks. At EACH height: prev_randao is non-zero and
#    BYTE-IDENTICAL across all 4 validators + the import-follower (every node
#    recovered the SAME unique threshold seed — the determinism property). ACROSS
#    heights: all distinct (the seed is unique per height → randomness varies).
#    The per-height all-node agreement is the strong check; a single node
#    deriving a divergent seed at one height (which validator-0-only / single-
#    height probes miss) fails here.
WINDOW=8
lo=$(( fin > WINDOW ? fin - WINDOW + 1 : 1 ))
echo "smoke-vrf: DPoS up, finalized=$fin; checking prev_randao over blocks [$lo..$fin] on ${#NODES[@]} nodes"
mixes=()
for ((n = lo; n <= fin; n++)); do
    vals=()
    for svc in "${NODES[@]}"; do
        mh=$(mixhash_of "$svc" "$n")
        if [[ "$mh" == "null" || -z "$mh" ]]; then
            echo "FAIL (smoke-vrf): $svc has no mixHash for finalized block $n (node behind / RPC down)"; exit 1
        fi
        if is_zero_hash "$mh"; then
            echo "FAIL (smoke-vrf): prev_randao is zero at block $n on $svc"; exit 1
        fi
        vals+=("$mh")
    done
    agree=$(printf '%s\n' "${vals[@]}" | sort -u | wc -l)
    if (( agree != 1 )); then
        echo "FAIL (smoke-vrf): nodes disagree on prev_randao at block $n — divergent threshold seed:"
        paste -d' ' <(printf '%s\n' "${NODES[@]}") <(printf '%s\n' "${vals[@]}") | sed 's/^/  /'
        exit 1
    fi
    mixes+=("${vals[0]}")
done
distinct=$(printf '%s\n' "${mixes[@]}" | sort -u | wc -l)
if (( distinct != ${#mixes[@]} )); then
    echo "FAIL (smoke-vrf): prev_randao not varying — ${#mixes[@]} blocks [$lo..$fin] but only $distinct distinct (stuck randomness)"
    printf '  %s\n' "${mixes[@]}"
    exit 1
fi
echo "smoke-vrf: ${#mixes[@]} blocks, ${#mixes[@]} distinct non-zero prev_randao, all ${#NODES[@]} nodes byte-identical at every height"

# 2) Each validator logged threshold-verified prev_randao (assurance=true) — the
#    beacon path against PK_epoch, NOT the digest fallback — SUSTAINED across at
#    least MIN_BLOCKS blocks AND still ACTIVE now (the count GROWS as the chain
#    advances). A beacon that logged its MIN_BLOCKS during warm-up then silently
#    fell to the digest fallback would keep a frozen count under live blocks —
#    invisible to a static threshold, caught by the growth check.
MIN_BLOCKS=5
ACTIVE_LINE="beacon: threshold prev_randao active"
declare -A before
for v in "${VALIDATORS[@]}"; do
    c=$(log_count "$v" "$ACTIVE_LINE")
    c=${c:-0}
    if (( c < MIN_BLOCKS )); then
        echo "FAIL (smoke-vrf): $v logged threshold prev_randao only $c times (< $MIN_BLOCKS) — beacon inactive/intermittent/fell back to digest"
        docker compose logs --tail=80 "$v"
        exit 1
    fi
    before[$v]=$c
    echo "smoke-vrf: $v — threshold prev_randao active x$c"
done

# 2b) Let the chain advance a few blocks (~1 blk/s) and require every validator's
#     active-count to GROW — proves the beacon is live at probe time, not frozen
#     post-warm-up on the digest fallback. The ACTIVE_LINE fires at the SPECULATIVE
#     TIP (notarize-time derive under deferred execution), so growth must track the
#     HEAD, not finalized: right after the Tempo→DPoS migration the tip bursts ahead
#     and finalized can catch up to it WITHOUT the tip producing new blocks — which
#     froze the count under the old finalized-based wait (false "beacon stopped").
head_dec() {
    curl -s -X POST -H 'Content-Type: application/json' \
        --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
        "$RPC" 2>/dev/null | jq -r '.result // "0x0"' \
        | { read -r h; printf '%d' "$h" 2>/dev/null || echo 0; }
}
head0=$(head_dec)
hdeadline=$(( SECONDS + 30 ))
while (( $(head_dec) < head0 + 3 )); do
    if (( SECONDS >= hdeadline )); then
        echo "FAIL (smoke-vrf): head did not advance >= 3 blocks past $head0 within 30s — cannot observe a sustained beacon"
        exit 1
    fi
    sleep 1
done
for v in "${VALIDATORS[@]}"; do
    after=$(log_count "$v" "$ACTIVE_LINE"); after=${after:-0}
    if (( after <= ${before[$v]} )); then
        echo "FAIL (smoke-vrf): $v active-count frozen at $after while the chain advanced — beacon stopped (fell back to digest)"
        docker compose logs --tail=80 "$v"
        exit 1
    fi
    echo "smoke-vrf: $v — active-count grew ${before[$v]} → $after (beacon live now)"
done

# 3) The COMPUTED beacon value reaches the chain: each CONFIRMED finalized
#    block's on-chain mixHash must appear among the prev_randao=H(seed) values
#    validator-0 logged on the assurance path. Step 1 reads the header, step 2
#    reads the log; this ties them together. We anchor on finalized blocks (never
#    rolled back) — the reverse check ("every logged value is on-chain") would
#    false-fail, since the active log fires on SPECULATIVE notarization derives
#    whose bleeding-edge / nullified rounds legitimately never canonicalize.
#    Extract the value from each active line as the only 32-byte hex on it (the
#    `round` field is not 64-hex) — format-agnostic across text `prev_randao=0x..`
#    and JSON `"prev_randao":"0x.."`. `|| true`: a non-matching grep must surface
#    as a labelled FAIL, not a silent `set -o pipefail` death.
raw=$(docker compose logs validator-0 2>/dev/null || true)
logged=$(printf '%s\n' "$raw" | grep "$ACTIVE_LINE" \
    | grep -oE '0x[0-9a-fA-F]{64}' | tr 'A-F' 'a-f' | sort -u || true)
if [[ -z "$logged" ]]; then
    echo "FAIL (smoke-vrf): no prev_randao value parsed from validator-0 '$ACTIVE_LINE' logs"
    echo "  --- sample raw active lines (diagnostic) ---"
    printf '%s\n' "$raw" | grep "$ACTIVE_LINE" | head -2 | sed 's/^/  /'
    printf '%s\n' "$raw" | grep "$ACTIVE_LINE" | head -1 | od -c | head -8 | sed 's/^/  /'
    exit 1
fi
# Anchor on the most recent finalized blocks: many blocks past activation, so
# genuine beacon blocks (NOT the pre-DPoS Tempo prefix in step 1's window, whose
# mixHash is the digest fallback and was never logged as a beacon value).
fin3=$(finalized_dec)
chk_lo=$(( fin3 > 4 ? fin3 - 3 : 1 ))
missing=()
for ((n = chk_lo; n <= fin3; n++)); do
    onc=$(mixhash_at "$n")
    grep -qxF "$onc" <<<"$logged" || missing+=("$n=$onc")
done
if (( ${#missing[@]} > 0 )); then
    echo "FAIL (smoke-vrf): finalized block mixHash(es) never logged by validator-0 as a threshold beacon value — header value is not the deriver's H(seed):"
    printf '  %s\n' "${missing[@]}"
    exit 1
fi
echo "smoke-vrf: all $((fin3 - chk_lo + 1)) recent finalized mixHashes [$chk_lo..$fin3] were logged as threshold-active beacon values (header == deriver H(seed))"

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

echo "OK (smoke-vrf): threshold-beacon prev_randao active+sustained+still-growing on all 4 validators (>=$MIN_BLOCKS blocks), non-zero and ${#mixes[@]}/${#mixes[@]} distinct and byte-identical across all ${#NODES[@]} nodes at every height in [$lo..$fin], recent logged H(seed) values match on-chain mixHashes, PK_epoch published+matched on L2 (deterministic seed)"
