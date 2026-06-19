#!/usr/bin/env bash
# Read-only assertion bodies for the DEFAULT-stack smoke cases, extracted so the
# combined `case-base.sh` can run them all on ONE bring-up while each remains a
# standalone `case-<name>.sh` (same function, its own bring-up) for isolated
# debugging. Every function here is READ-ONLY w.r.t. consensus — it queries RPC,
# greps logs, reads on-chain state, and at most deploys a throwaway probe contract;
# none stops/restarts/jails a node, so they cannot contaminate one another and may
# share a single stack.
#
# MUST be sourced AFTER lib.sh (uses its helpers + globals: RPC, CHAIN_ID,
# STAKING_ADDR, EPOCH_INTERVAL, DPOS_ACTIVATION_BLOCK, PREV_FIN, finalized_dec,
# wait_finalized_ge, mixhash_*, assert_beacon_window, wait_nodes_have, ...). The
# caller owns bring_up_dpos + `trap tear_down EXIT`. On failure each function
# `exit 1`s (fail-fast, terminates the whole run); on success it `return`s so a
# combined runner continues to the next assertion.
# shellcheck shell=bash

# smoke-tx: a value transfer AND a contract call execute + finalize under DPoS.
# A bare value transfer alone does not exercise the EVM execution path (no CALL/
# SSTORE), so the contract call (MockBlendToken.approve) is mandatory — it is what
# closes the brief's "EVM execution path under DPoS is unverified" gap.
assert_tx() {
    local KEY FROM DEAD BLEND ALLOW bal_before TXH CTXH maxblk h st blk bal_after delta allow
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
}

# smoke-epoch: the chain crosses >= EPOCH_MIN_CROSS epoch boundary(ies) after the
# DPoS swap. This is the permanent regression guard for the epoch-boundary handoff
# deadlock — the canonical smoke's "head > anchor" threshold can pass entirely
# inside epoch 0 and would hide a boundary regression.
assert_epoch() {
    local EPOCH_MIN_CROSS PREV_DEC TARGET deadline v0 v1 v2 v3 fn head hd cur comm r0 r1 delta
    EPOCH_MIN_CROSS="${EPOCH_MIN_CROSS:-1}"

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
                # 1 blk/s pacing assert: 60s of chain time must finalize ~60
                # blocks. Lower bound 45 tolerates view timeouts/jitter; upper
                # bound 66 catches a pacing regression (the unpaced chain did
                # ~350 blocks/min).
                r0=$(finalized_dec); sleep 60; r1=$(finalized_dec)
                delta=$(( r1 - r0 ))
                (( delta >= 45 && delta <= 66 )) || {
                    echo "FAIL (smoke-epoch): block rate off target: $delta blocks in 60s (want 45..66)"; exit 1; }
                echo "OK (smoke-epoch): all 5 aligned finalized=$hd >= $TARGET (epoch $cur), committee non-empty, pacing $delta blk/60s"
                return 0
            fi
        fi
        sleep 2
    done
    echo "FAIL (smoke-epoch): did not reach finalized >= $TARGET within 220s"
    docker compose logs --tail=200
    exit 1
}

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
assert_vrf() {
    # mixhash_at/in/of, is_zero_hash, log_count, beacon_key_at, assert_beacon_window,
    # wait_nodes_have are shared in lib.sh.

    local NODES VALIDATORS fin WINDOW lo mixes vals n svc mh agree distinct
    NODES=(validator-0 validator-1 validator-2 validator-3 full-node)
    VALIDATORS=(validator-0 validator-1 validator-2 validator-3)

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
    local MIN_BLOCKS ACTIVE_LINE before c v
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
    local head0 hdeadline after
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
    local raw logged fin3 chk_lo missing onc
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
    local pk_file pk_chain
    pk_file=$(docker compose exec -T validator-0 cat /runtime/keys/beacon-pk.hex | tr -d '[:space:]')
    pk_chain=$(cast call "$STAKING_ADDR" "getEpochBeaconKey(uint64)(bytes)" 0 --rpc-url "$RPC" | tr -d '[:space:]')
    if [[ -z "$pk_file" || "$pk_chain" != "0x$pk_file" ]]; then
        echo "FAIL (smoke-vrf): on-chain getEpochBeaconKey(0)=$pk_chain != published PK_epoch 0x$pk_file"
        exit 1
    fi
    echo "smoke-vrf: PK_epoch on-chain (getEpochBeaconKey(0)) matches the seed-verifying group key"

    # 5) C1/C2 (+H2) — the EVM-visible `block.prevrandao` EQUALS the header mixHash, i.e.
    #    the beacon value H(seed) reached EVM EXECUTION, not just the header. Deploy a
    #    probe whose snapshot() records `block.prevrandao` at its own block and emits it;
    #    compare to that block's header mixHash. The snapshot tx ALSO makes a tx-bearing
    #    block (H2 — the beacon is exercised under real load, not only empty blocks).
    local KEY PROBE_BC PROBE RCPT snap_block evm_pr hdr_pr
    KEY=$(docker compose exec -T validator-0 cat /runtime/keys/funded.hex | tr -d '[:space:]')
    PROBE_BC=$(python3 -c "import json;print(json.load(open('contracts/PrevRandaoProbe.json'))['bytecode']['object'])")
    PROBE=$(cast send --private-key "0x$KEY" --rpc-url "$RPC" --create "$PROBE_BC" --json \
        | python3 -c "import sys,json;print(json.load(sys.stdin)['contractAddress'])")
    echo "smoke-vrf: PrevRandaoProbe deployed at $PROBE"
    RCPT=$(cast send --private-key "0x$KEY" --rpc-url "$RPC" "$PROBE" "snapshot()" --json)
    snap_block=$(echo "$RCPT" | python3 -c "import sys,json;print(int(json.load(sys.stdin)['blockNumber'],16))")
    # Snapshot(uint256 indexed blockNumber, uint256 prevrandao): `prevrandao` is the only
    # NON-indexed arg → it is the log `data` (0x + 64 hex). Compare to the header mixHash.
    evm_pr=$(echo "$RCPT" | python3 -c "import sys,json;print(json.load(sys.stdin)['logs'][0]['data'])" | tr 'A-F' 'a-f')
    hdr_pr=$(mixhash_at "$snap_block")
    if [[ "$evm_pr" != "$hdr_pr" ]]; then
        echo "FAIL (smoke-vrf): EVM block.prevrandao ($evm_pr) != header mixHash ($hdr_pr) at probe block $snap_block — the beacon value did not reach EVM execution"
        exit 1
    fi
    echo "smoke-vrf: C1/C2 — EVM block.prevrandao == header mixHash at tx-bearing block $snap_block (beacon H(seed) reached EVM execution)"

    # 6) D1 — on a beacon-active chain `beacon_digest_fallback_total` must NOT grow while
    #    the chain advances (every block gets a VERIFIED seed; growth ⇒ a block fell to
    #    order.digest()), and `beacon_seed_active_total` MUST grow (metric wired + live).
    #    Substring grep tolerates the OpenMetrics `_total` suffix convention; on a miss,
    #    print the available beacon_/dkg_ series so the real metric names surface.
    local fb0 sa0 h0 hd fb1 sa1
    metric() { curl -s "http://localhost:19100/metrics" 2>/dev/null | grep "$1" | grep -v '^#' | awk '{print $NF}' | head -1; }
    fb0=$(metric beacon_digest_fallback); sa0=$(metric beacon_seed_active)
    if [[ -z "$fb0" || -z "$sa0" ]]; then
        echo "FAIL (smoke-vrf): D1 — beacon metrics absent on :19100 (digest_fallback='$fb0' seed_active='$sa0'). beacon_/dkg_ series present:"
        curl -s "http://localhost:19100/metrics" 2>/dev/null | grep -iE "beacon|dkg" | grep -v '^#' | head -20 | sed 's/^/  /'
        exit 1
    fi
    h0=$(head_dec); hd=$(( SECONDS + 30 ))
    while (( $(head_dec) < h0 + 3 )); do (( SECONDS < hd )) || break; sleep 1; done
    fb1=$(metric beacon_digest_fallback); sa1=$(metric beacon_seed_active); fb1=${fb1:-0}; sa1=${sa1:-0}
    if awk "BEGIN{exit !($fb1 > $fb0)}"; then
        echo "FAIL (smoke-vrf): D1 — beacon_digest_fallback grew $fb0 → $fb1 on a beacon-active chain (a block fell to order.digest())"
        exit 1
    fi
    if ! awk "BEGIN{exit !($sa1 > $sa0)}"; then
        echo "FAIL (smoke-vrf): D1 — beacon_seed_active did not grow ($sa0 → $sa1) — beacon stalled / metric not incrementing"
        exit 1
    fi
    echo "smoke-vrf: D1 — beacon_digest_fallback flat ($fb0) while beacon_seed_active grew ($sa0 → $sa1) over 3 blocks"

    # E1 (cert-follower mixHash == validators) is already covered above: the import
    # follower `full-node` is in NODES, and step 1 asserts byte-identical prev_randao
    # across ALL nodes (incl full-node) at every height.

    echo "OK (smoke-vrf): threshold-beacon prev_randao active+sustained+still-growing on all 4 validators (>=$MIN_BLOCKS blocks), non-zero and ${#mixes[@]}/${#mixes[@]} distinct and byte-identical across all ${#NODES[@]} nodes at every height in [$lo..$fin], recent logged H(seed) values match on-chain mixHashes, PK_epoch published+matched on L2 (deterministic seed)"
}

# smoke-vrf-boundary: the threshold beacon survives an EPOCH BOUNDARY on a STABLE
# committee. Crosses the first relative-epoch boundary (activation + interval) and
# asserts:
#   F1 — prev_randao stays threshold-active, byte-identical across all nodes, and
#        varying ACROSS the boundary (the per-epoch engine rebuild + Phase-1
#        full-enter carry-forward read at the boundary edge);
#   F2 — the new (uncommitted, stable-committee) epoch CARRIES the key forward
#        on-chain: getEpochBeaconKey(1) == getEpochBeaconKey(0).
assert_vrf_boundary() {
    local NODES BOUNDARY TARGET lo hi pk0 pk1
    NODES=(validator-0 validator-1 validator-2 validator-3 full-node)

    # Epoch geometry (lib.sh, mirrors genesis-bootstrap): DPoS activates at
    # DPOS_ACTIVATION_BLOCK; epochs are EPOCH_INTERVAL blocks. Relative epoch 1 begins
    # at activation + interval — the first boundary to cross on a stable committee.
    BOUNDARY=$(( DPOS_ACTIVATION_BLOCK + EPOCH_INTERVAL ))
    TARGET=$(( BOUNDARY + 8 ))
    echo "smoke-vrf-boundary: waiting for finalized >= $TARGET (epoch-1 boundary at block $BOUNDARY)"
    wait_finalized_ge "$TARGET" 180 >/dev/null || {
        echo "FAIL (smoke-vrf-boundary): chain did not reach finalized $TARGET"
        docker compose logs --tail=120 validator-0
        exit 1
    }

    # F1: beacon stays active + node-agreed + varying ACROSS the boundary. Wait for the
    # followers to have the top of the window first (they lag the validators a few blocks).
    lo=$(( BOUNDARY - 6 ))
    hi=$(( BOUNDARY + 6 ))
    wait_nodes_have "$hi" 120 || {
        echo "FAIL (smoke-vrf-boundary): not all nodes reached block $hi"
        exit 1
    }
    assert_beacon_window "$lo" "$hi" "epoch-boundary-$BOUNDARY"
    echo "smoke-vrf-boundary: F1 — beacon active + byte-identical across the epoch-1 boundary (block $BOUNDARY)"

    # F2: a STABLE committee commits no fresh PK_E at the boundary, so on-chain
    # getEpochBeaconKey CARRIES FORWARD — epoch 1 reads the same key as epoch 0.
    pk0=$(beacon_key_at 0)
    pk1=$(beacon_key_at 1)
    if [[ -z "$pk0" || "$pk0" == "0x" ]]; then
        echo "FAIL (smoke-vrf-boundary): getEpochBeaconKey(0) is empty ('$pk0') — genesis beacon key not committed"
        exit 1
    fi
    if [[ "$pk1" != "$pk0" ]]; then
        echo "FAIL (smoke-vrf-boundary): F2 — getEpochBeaconKey(1)='$pk1' != getEpochBeaconKey(0)='$pk0' (a stable committee must carry the key forward)"
        exit 1
    fi
    echo "smoke-vrf-boundary: F2 — getEpochBeaconKey(1) == getEpochBeaconKey(0) ($pk0) — carry-forward on a stable committee"

    echo "OK (smoke-vrf-boundary): threshold beacon active + node-agreed + varying across the epoch-1 boundary (block $BOUNDARY); on-chain key carries forward for the stable committee"
}
