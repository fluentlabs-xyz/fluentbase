#!/usr/bin/env bash
# Event log + structured failure bundle for the soak (sourced by case-soak.sh).
#
# soak_event KIND MSG  — append one JSON object to $SOAK_OUT/events.jsonl AND
#   mirror a human line to stdout. The seed is on EVERY line (a truncated log is
#   still attributable). Context (round/epoch/committee/f/disrupted/pending) is
#   read from the orchestrator's globals — set them before calling.
# soak_bundle_dump REASON INV_ID — write $SOAK_OUT/bundle-<ts>/ with everything
#   needed to diagnose + replay (plan §5), print summary.txt, return.

: "${SOAK_OUT:=./soak-out}"
: "${SOAK_LOG_BYTES:=5000000}"

soak_event() {
    # No-op under the startup self-check dry-run: the dispatch dry-call (selfcheck_dispatch)
    # routes register_activate/probe arms through here, but a dry-run must touch no RPC/log.
    [[ -n "${SOAK_DRYRUN:-}" ]] && return 0
    local kind="$1" note="${2:-}"
    mkdir -p "$SOAK_OUT"
    local fin epoch
    fin=$(finalized_dec 2>/dev/null || echo 0)
    epoch="${SOAK_CUR_EPOCH:-0}"
    # committee/disrupted/pending as JSON arrays from the orchestrator's space-sets
    local comm_json disr_json pend_json
    comm_json=$(printf '%s\n' ${SOAK_CUR_COMMITTEE:-} | jq -R . | jq -cs 'map(select(length>0))')
    disr_json=$(printf '%s\n' ${SOAK_DISRUPTED:-}     | jq -R . | jq -cs 'map(select(length>0))')
    pend_json=$(printf '%s\n' ${SOAK_PENDING:-}       | jq -R . | jq -cs 'map(select(length>0))')
    jq -nc \
        --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
        --argjson round "${SOAK_ROUND:-0}" \
        --arg seed "$SOAK_SEED" \
        --arg kind "$kind" \
        --argjson fin "${fin:-0}" \
        --argjson epoch "${epoch:-0}" \
        --argjson f "${SOAK_CUR_F:-0}" \
        --argjson committee "$comm_json" \
        --argjson disrupted "$disr_json" \
        --argjson pending "$pend_json" \
        --arg intended "${SOAK_INTENDED:-}" \
        --arg verdict "${SOAK_VERDICT:-}" \
        --arg applied "${SOAK_APPLIED:-}" \
        --arg note "$note" \
        '{ts:$ts,round:$round,seed:$seed,kind:$kind,intended:$intended,verdict:$verdict,
          applied_action:$applied,finalized_block:$fin,epoch:$epoch,committee:$committee,
          f:$f,disrupted:$disrupted,pending:$pending,note:$note}' \
        >>"$SOAK_OUT/events.jsonl"
    printf '  [soak r%s %s] fin=%s epoch=%s f=%s %s\n' \
        "${SOAK_ROUND:-0}" "$kind" "${fin:-0}" "$epoch" "${SOAK_CUR_F:-0}" "$note"
}

# Structured bundle on the first invariant_fail / trap. Best-effort per artifact
# (a down node's logs/RPC may be unavailable — that absence is itself a signal).
soak_bundle_dump() {
    local reason="$1" inv_id="${2:-unknown}"
    local ts dir svc
    ts=$(date -u +%Y%m%dT%H%M%SZ)
    dir="$SOAK_OUT/bundle-$ts"
    mkdir -p "$dir/logs" "$dir/rpc" "$dir/compose"

    local fin epoch
    fin=$(finalized_dec 2>/dev/null || echo 0)
    epoch="${SOAK_CUR_EPOCH:-0}"

    {
        echo "SOAK FAILURE BUNDLE"
        echo "broken invariant : $inv_id"
        echo "reason           : $reason"
        echo "seed             : $SOAK_SEED"
        echo "round            : ${SOAK_ROUND:-0}"
        echo "finalized_block  : $fin"
        echo "epoch            : $epoch"
        echo "committee        : ${SOAK_CUR_COMMITTEE:-}"
        echo "f                : ${SOAK_CUR_F:-0}"
        echo "disrupted        : ${SOAK_DISRUPTED:-}"
        echo "pending          : ${SOAK_PENDING:-}"
        echo
        echo "REPLAY (reproduces the INTENT schedule; applied churn may differ — plan §1.2):"
        echo "  SOAK_SEED=$SOAK_SEED SOAK_VALIDATORS=${SOAK_VALIDATORS:-} SOAK_INITIAL_COMMITTEE=${SOAK_INITIAL_COMMITTEE:-} SOAK_EPOCH_INTERVAL=${SOAK_EPOCH_INTERVAL:-} make smoke-soak"
        echo "  (SOAK_STOP_ROUND=$(( ${SOAK_ROUND:-1} - 1 )) to halt just before this round)"
    } >"$dir/summary.txt"

    [[ -f "$SOAK_OUT/events.jsonl" ]] && cp "$SOAK_OUT/events.jsonl" "$dir/events.jsonl"

    # All per-node logs (tail-bounded). Scope via COMPOSE_FILE (set by the orchestrator).
    for svc in "${SOAK_VALS[@]}" full-node; do
        docker compose logs "$svc" 2>/dev/null \
            | tail -c "$SOAK_LOG_BYTES" >"$dir/logs/$svc.log" 2>/dev/null || true
    done

    # RPC snapshots (host RPC + on-chain reads). Best-effort.
    soak_consensus_latest >"$dir/rpc/consensus_getLatest.txt" 2>/dev/null || true
    curl -s -X POST -H 'Content-Type: application/json' \
        --data '{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["finalized",false],"id":1}' \
        "$RPC" >"$dir/rpc/eth_finalized.json" 2>/dev/null || true
    curl -s -X POST -H 'Content-Type: application/json' \
        --data '{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["latest",false],"id":1}' \
        "$RPC" >"$dir/rpc/eth_latest.json" 2>/dev/null || true
    {
        local e
        for e in $(( epoch - 1 )) "$epoch" $(( epoch + 1 )); do
            (( e < 0 )) && continue
            echo "epoch $e committee: $(pp_committee "$e" 2>/dev/null || true)"
            echo "epoch $e withStakes: $(pp_staking_call 'getEpochCommitteeWithStakes(uint64)(address[],(bytes,bytes32,uint64)[],uint256[])' "$e" 2>/dev/null || true)"
        done
    } >"$dir/rpc/committees.txt" 2>/dev/null || true
    # per-victim status + misscount
    {
        local v addr
        for v in ${SOAK_DISRUPTED:-}; do
            addr=$(pp_owner_addr "${v##*-}" 2>/dev/null || true)
            echo "$v ($addr): status=$(validator_status "$addr" 2>/dev/null || true) miss=$(misscount "$epoch" "$addr" 2>/dev/null || true)"
        done
    } >"$dir/rpc/victims.txt" 2>/dev/null || true
    curl -s "http://localhost:19100/metrics" >"$dir/rpc/metrics-19100.txt" 2>/dev/null || true

    # Topology: generated compose + the runtime config the bring-up wrote.
    cp docker-compose.soak.gen.yml docker-compose.soak.dpos.gen.yml "$dir/compose/" 2>/dev/null || true
    for f in staking-reader.json addresses.json peers.json; do
        pp_runtime_cat "$f" >"$dir/compose/$f" 2>/dev/null || true
    done

    echo "================ SOAK FAILURE ================"
    cat "$dir/summary.txt"
    echo "bundle: $dir"
    echo "=============================================="
}
