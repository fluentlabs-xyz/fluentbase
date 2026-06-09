#!/usr/bin/env bash
# Shared helpers for the DPoS smoke regression cases. Sourced by assert-smoke.sh
# (phase1/phase2 happy-path) and every scripts/case-*.sh. Extracted so each case
# is self-contained and the RPC/convergence/migration logic lives in one place.
#
# Callers must `cd "$(dirname "$0")/.."` (the local-dpos-smoke dir) before sourcing,
# and run under `set -euo pipefail`.

JSONRPC_FINALIZED='{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["finalized",false],"id":1}'

# On-chain predeploy addresses (genesis-bootstrap canonical) + chain params.
STAKING_ADDR="0x0000000000000000000000000000000000005201"
CHAIN_CONFIG_ADDR="0x0000000000000000000000000000000000005202"
# LivenessSlashing is pinned to the fixed executor predeploy
# (PRECOMPILE_LIVENESS_SLASHING), NOT the 0x...520N scheme — see
# genesis-bootstrap/src/bootstrap.rs LIVENESS_SLASHING_ADDR.
LIVENESS_SLASHING_ADDR="0x0000000000000000000000000000000000520020"
CHAIN_ID=2026
RPC="http://localhost:8545"
EPOCH_INTERVAL="${EPOCH_INTERVAL:-32}"   # mirrors genesis-bootstrap epochBlockInterval
# MUST equal genesis-bootstrap's ChainConfig.dposActivationBlock (bootstrap.rs).
# DPoS epoch numbering rebases to 0 here; the migration anchor must land in
# relative epoch 0 = [activation, activation+EPOCH_INTERVAL). Aligned, future.
DPOS_ACTIVATION_BLOCK="${DPOS_ACTIVATION_BLOCK:-64}"

VALS=(validator-0 validator-1 validator-2 validator-3)

# --- RPC probes (verbatim from the original assert-smoke.sh) ---------------

check_node() {
    # A refused connection (node not up / restarting) makes curl print NOTHING
    # and exit 7; `jq` on empty stdin emits an empty line and exits 0, so the
    # trailing `|| echo "null|null"` would NOT fire — the function would return
    # "" and slip past the `!= "null"` / `!= "0x0"` convergence guards (an
    # all-nodes-down poll could then false-pass as "converged"). Capture the
    # output and coerce an empty result to the "null|null" sentinel explicitly.
    local cmd_prefix=("$@") out
    out=$("${cmd_prefix[@]}" curl -s -X POST -H 'Content-Type: application/json' \
        --data "$JSONRPC_FINALIZED" http://localhost:8545 2>/dev/null \
        | jq -r '(.result.number // "null") + "|" + (.result.hash // "null")' 2>/dev/null) || true
    [[ -n "$out" ]] && printf '%s\n' "$out" || echo "null|null"
}
check_external() {
    # Same empty-stdin caveat as check_node — coerce "" → "null|null".
    local port="$1" out
    out=$(curl -s -X POST -H 'Content-Type: application/json' \
        --data "$JSONRPC_FINALIZED" "http://localhost:${port}" 2>/dev/null \
        | jq -r '(.result.number // "null") + "|" + (.result.hash // "null")' 2>/dev/null) || true
    [[ -n "$out" ]] && printf '%s\n' "$out" || echo "null|null"
}

# True iff the validator's most recent graceful shutdown persisted cleanly. A
# graceful `docker compose stop` lets reth's own `on_graceful_shutdown` arm
# (crates/node/builder/src/launch/engine.rs) Terminate the engine and persist the
# in-memory tail to MDBX before the container exits 0 — no external flush watcher.
shutdown_flushed() {
    # A graceful exit (code 0) ⇒ reth's native graceful-shutdown persisted the
    # in-memory tail; a SIGKILL on the 40s stop-timeout exits 137. Check the
    # container's exit code via `docker inspect` (container metadata, not logs): the
    # previous `docker compose logs | grep` raced — under heavy docker-daemon load in
    # the suite the log read lagged past the poll window and false-reported
    # "flush incomplete" even though the node exited 0 in <1s (diagnosed 2026-06-05;
    # migration-flush-diag.txt: exit=0/flush_complete=yes/<1s, nominal and under load).
    local cid i running ec
    cid=$(docker compose ps -aq "$1" 2>/dev/null)
    [[ -z "$cid" ]] && return 1
    for i in $(seq 1 10); do
        read -r running ec < <(docker inspect "$cid" \
            --format '{{.State.Running}} {{.State.ExitCode}}' 2>/dev/null)
        [[ "$running" == "false" && "$ec" == "0" ]] && return 0
        sleep 1
    done
    return 1
}

# Wait (default 90s) for all 5 nodes to align finalized > 0; echo "height|hash".
wait_converge() {
    local deadline=$(( $(date +%s) + ${1:-90} )) v0 v1 v2 v3 fn hn
    while [[ $(date +%s) -lt $deadline ]]; do
        v0=$(check_external 8545); v1=$(check_node docker compose exec -T validator-1)
        v2=$(check_node docker compose exec -T validator-2); v3=$(check_node docker compose exec -T validator-3)
        fn=$(check_external 18545); hn="${v0%%|*}"
        if [[ "$hn" != "null" && "$hn" != "0x0" \
              && "$v0" == "$v1" && "$v1" == "$v2" && "$v2" == "$v3" && "$v3" == "$fn" ]]; then
            echo "$v0"; return 0
        fi
        sleep 2
    done
    return 1
}

# --- cast read wrappers (host → validator-0 RPC) ---------------------------

staking_call()     { cast call "$STAKING_ADDR"           "$@" --rpc-url "$RPC"; }
chainconfig_call() { cast call "$CHAIN_CONFIG_ADDR"      "$@" --rpc-url "$RPC"; }
liveness_call()    { cast call "$LIVENESS_SLASHING_ADDR" "$@" --rpc-url "$RPC"; }

# Decimal finalized height of the host-exposed producer (validator-0). NOTE: an
# unreachable RPC coerces to 0 here (same as genesis) — fine for the `>= target`
# polling loops (a transient 0 just costs an iteration), but a 0 must NOT be
# accepted as a PRE/baseline (see baseline_height).
finalized_dec() { printf '%d' "$(check_external 8545 | cut -d'|' -f1)" 2>/dev/null || echo 0; }

# Settled finalized height for a PRE/baseline capture. Unlike finalized_dec, which
# maps "RPC unreachable" to 0 indistinguishably from genesis, this retries until
# validator-0 returns a real height > 0 so a momentary RPC blip at capture time
# cannot seed a 0 baseline that makes a later "finalized > PRE" assertion pass
# trivially (masking a stalled rejoin/restart). The chain is already live past the
# anchor at every call site, so a real height is always available; fail loud after
# ~30s rather than return a bogus baseline.
baseline_height() {
    local deadline=$(( $(date +%s) + 30 )) h
    while (( $(date +%s) < deadline )); do
        h=$(finalized_dec)
        (( h > 0 )) && { printf '%d' "$h"; return 0; }
        sleep 1
    done
    echo "FAIL: baseline_height could not read a finalized height > 0 from validator-0 within 30s" >&2
    return 1
}

# reth devp2p peer count (decimal) as seen INSIDE a validator container — the EL
# transport plane (distinct from commonware consensus connectivity). 0 if the RPC
# is not answering yet / no peers. Usage: peer_count validator-3
peer_count() {
    local svc="$1" hx
    hx=$(docker compose exec -T "$svc" curl -s -X POST -H 'content-type: application/json' \
        --data '{"jsonrpc":"2.0","method":"net_peerCount","params":[],"id":1}' \
        http://localhost:8545 2>/dev/null | jq -r '.result // empty')
    [[ "$hx" == 0x* ]] && printf '%d' "$hx" || echo 0
}

# --- migration (Tempo → DPoS) ----------------------------------------------

# Graceful-stop the 4 validators, verify each persisted via reth's native
# graceful shutdown — exit 0 (retry up to 4×, bringing
# Tempo back up to let a laggard re-obtain the anchor), then cold-restart them
# under --dpos with the migration anchor. Sets the global PREV_FIN (anchor height,
# hex). Honours $DPOS_EXTRA_COMPOSE (extra `-f file.yml` args, e.g. byzantine).
PREV_FIN="null"
_migrate_to_dpos() {
    local anchor flush_ok=0 attempt v all_flushed
    # R-contract migration: wait until Tempo finalizes past the (genesis-baked)
    # dposActivationBlock so the swap anchor lands in relative epoch 0
    # ([activation, activation+EPOCH_INTERVAL)). Below activation OriginEpocher
    # rejects the cold-start height; at/above activation+interval the cold-start
    # epoch would be >= 1 and re-hit the empty-marshal genesis lookup (bug #2).
    echo "waiting for Tempo to finalize >= dposActivationBlock=$DPOS_ACTIVATION_BLOCK (relative epoch 0)"
    local _wstart _fin_hex _fin_dec
    _wstart=$(date +%s)
    while :; do
        _fin_hex=$(check_external 8545 | cut -d'|' -f1)
        if [[ "$_fin_hex" != "null" ]]; then
            _fin_dec=$(printf '%d' "$_fin_hex")
            if (( _fin_dec >= DPOS_ACTIVATION_BLOCK )); then
                echo "  Tempo finalized $_fin_dec >= activation $DPOS_ACTIVATION_BLOCK; proceeding to swap"
                break
            fi
        fi
        if (( $(date +%s) - _wstart > 180 )); then
            echo "FAIL: Tempo did not reach dposActivationBlock=$DPOS_ACTIVATION_BLOCK within 180s"
            docker compose logs --tail=120; tear_down; exit 1
        fi
        sleep 3
    done
    for attempt in 1 2 3 4; do
        anchor=$(check_external 8545)
        docker compose stop --timeout 40 "${VALS[@]}"
        all_flushed=1
        for v in "${VALS[@]}"; do
            shutdown_flushed "$v" || { all_flushed=0; echo "  flush incomplete: $v (attempt $attempt)"; }
        done
        if [[ "$all_flushed" == 1 ]]; then
            echo "all validators flushed persistence on shutdown (attempt $attempt)"; flush_ok=1; break
        fi
        [[ "$attempt" == 4 ]] && break
        echo "  bringing Tempo network back up to reconverge before retry"
        docker compose start "${VALS[@]}"
        wait_converge 90 >/dev/null || { echo "FAIL: Tempo did not reconverge during flush-retry"; docker compose logs --tail=120; tear_down; exit 1; }
    done
    if [[ "$flush_ok" != 1 ]]; then
        echo "FAIL: validators did not flush after 4 attempts — anchor block would be missing"; tear_down; exit 1
    fi
    PREV_FIN=$(printf '%s' "$anchor" | cut -d'|' -f1)
    local anchor_dec
    anchor_dec=$(printf '%d' "$PREV_FIN")
    # Nodes self-discover the anchor from ChainConfig.dposActivationBlock and
    # anchor the consensus genesis there; the fresh-migration cold-start fails
    # loud unless reth's finalized tip sits in [activation, activation+EPOCH_INTERVAL)
    # (relative epoch 0). A flush-retry that reconverged Tempo past the window
    # would trip that fail-loud — reject it here with an actionable message.
    if (( anchor_dec < DPOS_ACTIVATION_BLOCK || anchor_dec >= DPOS_ACTIVATION_BLOCK + EPOCH_INTERVAL )); then
        echo "FAIL: Tempo finalized $anchor_dec outside relative epoch 0 \
[$DPOS_ACTIVATION_BLOCK, $((DPOS_ACTIVATION_BLOCK + EPOCH_INTERVAL))) — a flush-retry likely \
advanced Tempo past the window. Re-run, or widen the window (raise EPOCH_INTERVAL)."
        tear_down; exit 1
    fi
    # No migration-anchor flags: nodes read dposActivationBlock from the contract
    # and the restart-vs-fresh state from the consensus-archive discriminator.
    # shellcheck disable=SC2086
    docker compose -f docker-compose.yml -f docker-compose.dpos.yml ${DPOS_EXTRA_COMPOSE:-} \
        up -d --force-recreate "${VALS[@]}"
}

# Wait (default 120s) until the honest nodes align finalized strictly past PREV_FIN.
# Normally requires all 5; if $DPOS_CONVERGE_EXCLUDE names a validator (e.g. a
# byzantine one whose consensus is replaced and whose reth never finalizes), that
# node is dropped from the alignment check and the honest quorum must align instead.
wait_dpos_converge() {
    local deadline=$(( $(date +%s) + ${1:-120} )) v0 v1 v2 v3 fn head
    local excl="${DPOS_CONVERGE_EXCLUDE:-}"
    while [[ $(date +%s) -lt $deadline ]]; do
        v0=$(check_external 8545); v1=$(check_node docker compose exec -T validator-1)
        v2=$(check_node docker compose exec -T validator-2); v3=$(check_node docker compose exec -T validator-3)
        fn=$(check_external 18545); head="${v0%%|*}"
        # collect the honest set's readings, excluding $excl
        local readings=()
        [[ "$excl" != "validator-0" ]] && readings+=("$v0")
        [[ "$excl" != "validator-1" ]] && readings+=("$v1")
        [[ "$excl" != "validator-2" ]] && readings+=("$v2")
        [[ "$excl" != "validator-3" ]] && readings+=("$v3")
        readings+=("$fn")
        local aligned=1 r
        for r in "${readings[@]}"; do [[ "$r" == "${readings[0]}" ]] || aligned=0; done
        head="${readings[0]%%|*}"
        if [[ "$aligned" == 1 && "$head" != "null" && "$head" != "0x0" && "$head" != "$PREV_FIN" ]]; then
            echo "${readings[0]}"; return 0
        fi
        sleep 2
    done
    return 1
}

# One-shot: bring up the full Tempo stack, converge, migrate to DPoS, and wait
# until the DPoS chain is live past the anchor. Leaves the stack UP. Used by every
# case-*.sh so each is self-contained.
bring_up_dpos() {
    docker compose up --build -d
    wait_converge 90 >/dev/null || { echo "FAIL: phase1 Tempo did not converge"; docker compose logs --tail=120; tear_down; exit 1; }
    _migrate_to_dpos
    wait_dpos_converge 120 >/dev/null || { echo "FAIL: DPoS chain did not converge past anchor $PREV_FIN"; docker compose logs --tail=200; tear_down; exit 1; }
    echo "DPoS stack live (anchor=$PREV_FIN)"
}

tear_down() { docker compose down -v --remove-orphans 2>/dev/null || true; }
