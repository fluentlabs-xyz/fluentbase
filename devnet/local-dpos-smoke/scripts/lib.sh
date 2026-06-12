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

# ============================================================================
# Production-path (runtime-deploy) helpers — used by case-production-path.sh.
#
# This scenario runs its OWN compose project (6 validators, bare genesis): the
# case script `export COMPOSE_FILE=...` before sourcing nothing extra, so every
# `docker compose` above targets the right stack. The staking cluster is
# deployed at RUNTIME via forge (not genesis); addresses are discovered from the
# DeployStaking manifest and threaded into staking-reader.json before the --dpos
# cold-restart. Foundry (forge/cast) is a host prerequisite, as in case-tx.sh.
#
# Requires (set by the case script): SOLIDITY_CONTRACTS_DIR, GOV_ADDR/STAKING_RT/
# CHAIN_CONFIG_RT/LIVENESS_RT (populated after deploy).
# ============================================================================

PP_VALS=(validator-0 validator-1 validator-2 validator-3 validator-4 validator-5)
PP_COMMITTEE_SIZE=5

# Read a file out of the runtime docker volume (validator-0 mounts /runtime).
pp_runtime_cat() { docker compose exec -T validator-0 cat "/runtime/$1" 2>/dev/null; }

# Validator <idx> l2 owner key (0x-prefixed) and address, from the bare export.
pp_owner_key()  { printf '0x%s' "$(pp_runtime_cat "keys/owner-$1.hex")"; }
pp_owner_addr() { pp_runtime_cat addresses.json | jq -r ".validators[$1]"; }

# Emit validator <idx> consensus-key material JSON (validatorAddress,
# blsPubkeyUncompressed, blsPoPUncompressed, peerPubkey, ownerKey) by re-running
# the bootstrap binary's consensus-keys subcommand inside the shared image.
pp_consensus_keys() {
    docker compose run --rm --no-deps -T \
        --entrypoint /usr/local/bin/genesis-bootstrap genesis-init \
        consensus-keys --idx "$1" --peers 6 --chain-id "$CHAIN_ID" 2>/dev/null
}

# Wait (default 90s) until all 6 validators + full-node align finalized > floor.
# $1 = timeout, $2 = floor hex to require strictly past (or "" for >0).
pp_wait_converge() {
    local deadline=$(( $(date +%s) + ${1:-90} )) floor="${2:-}" r readings head aligned
    while [[ $(date +%s) -lt $deadline ]]; do
        readings=(
            "$(check_external 8545)"
            "$(check_node docker compose exec -T validator-1)"
            "$(check_node docker compose exec -T validator-2)"
            "$(check_node docker compose exec -T validator-3)"
            "$(check_node docker compose exec -T validator-4)"
            "$(check_node docker compose exec -T validator-5)"
            "$(check_external 18545)"
        )
        aligned=1
        for r in "${readings[@]}"; do [[ "$r" == "${readings[0]}" ]] || aligned=0; done
        head="${readings[0]%%|*}"
        if [[ "$aligned" == 1 && "$head" != "null" && "$head" != "0x0" ]] \
           && { [[ -z "$floor" ]] || [[ "$head" != "$floor" ]]; }; then
            echo "${readings[0]}"; return 0
        fi
        sleep 2
    done
    return 1
}

# cast read wrappers against the runtime-deployed addresses (set post-deploy).
pp_staking_call()     { cast call "$STAKING_RT"      "$@" --rpc-url "$RPC"; }
pp_chainconfig_call() { cast call "$CHAIN_CONFIG_RT" "$@" --rpc-url "$RPC"; }

# Committee member set at epoch $1 as a sorted, lowercased, space-joined string
# (so set membership can be compared regardless of on-chain ordering).
pp_committee() {
    # Trailing `|| true`: an empty committee makes `grep .` exit non-zero, which
    # under the caller's `set -o pipefail` would abort the whole script silently
    # instead of yielding an empty string the assertion can report.
    pp_staking_call "getEpochCommittee(uint64)(address[])" "$1" 2>/dev/null \
        | tr -d '[]' | tr ',' '\n' | sed 's/^ *//;s/ *$//' \
        | tr 'A-F' 'a-f' | grep . | sort | paste -sd' ' - || true
}

# Decimal current relative DPoS epoch from the host RPC head.
pp_current_epoch() {
    local head act interval
    head=$(printf '%d' "$(check_external 8545 | cut -d'|' -f1)")
    act=$(printf '%d' "$(pp_chainconfig_call 'getDposActivationBlock()(uint64)' 2>/dev/null || echo 0)")
    interval=$(printf '%d' "$(pp_chainconfig_call 'getEpochBlockInterval()(uint32)' 2>/dev/null || echo 64)")
    (( interval == 0 )) && { echo 0; return; }
    (( head < act )) && { echo 0; return; }
    echo $(( (head - act) / interval ))
}

# Block until the host RPC head crosses the next epoch boundary $1 times.
pp_wait_epochs() {
    local n="$1" start now
    start=$(pp_current_epoch)
    local deadline=$(( $(date +%s) + 60 * (n + 2) ))
    while (( $(date +%s) < deadline )); do
        now=$(pp_current_epoch)
        (( now >= start + n )) && return 0
        sleep 2
    done
    return 1
}

# Send BLEND <amount> from the deployer (v0) to <to>. $1=token $2=to $3=amount.
pp_token_transfer() {
    cast send "$1" "transfer(address,uint256)(bool)" "$2" "$3" \
        --rpc-url "$RPC" --private-key "$(pp_owner_key 0)" >/dev/null
}

# Drive one onlyFromGovernance action through propose → castVote(For) → execute.
# $1=target $2=calldata(0x..) $3=description. v0 proposes; v0-v4 vote For (≥2/3 of
# the 5-validator voting supply). votingPeriod=10 (l2.json) gives the 5 sequential
# castVote sends room to land before the deadline.
pp_gov_action() {
    local target="$1" calldata="$2" desc="$3"
    local desc_hash pid i state
    desc_hash=$(cast keccak "$desc")
    # `cast call …(uint256)` pretty-prints large numbers as "<dec> [<sci>]"; the
    # ` [9.86e76]` suffix must be stripped or it fails to re-parse as a uint256
    # argument to state()/castVote()/execute() (silent parser error → empty state).
    pid=$(cast call "$GOV_ADDR" \
        "hashProposal(address[],uint256[],bytes[],bytes32)(uint256)" \
        "[$target]" "[0]" "[$calldata]" "$desc_hash" --rpc-url "$RPC" | awk '{print $1}')
    cast send "$GOV_ADDR" "propose(address[],uint256[],bytes[],string)(uint256)" \
        "[$target]" "[0]" "[$calldata]" "$desc" \
        --rpc-url "$RPC" --private-key "$(pp_owner_key 0)" >/dev/null
    # votingDelay=0 → Active at the next block. Poll state (1 = Active).
    for i in $(seq 1 15); do
        state=$(cast call "$GOV_ADDR" "state(uint256)(uint8)" "$pid" --rpc-url "$RPC" 2>/dev/null | awk '{print $1}')
        [[ "$state" == 1 ]] && break
        sleep 1
    done
    [[ "$state" == 1 ]] || { echo "FAIL pp_gov_action: proposal not Active (state=$state) for: $desc"; return 1; }
    for i in 0 1 2 3 4; do
        cast send "$GOV_ADDR" "castVote(uint256,uint8)(uint256)" "$pid" 1 \
            --rpc-url "$RPC" --private-key "$(pp_owner_key "$i")" >/dev/null
    done
    # Wait out the voting period until Succeeded (4), then execute.
    for i in $(seq 1 30); do
        state=$(cast call "$GOV_ADDR" "state(uint256)(uint8)" "$pid" --rpc-url "$RPC" 2>/dev/null | awk '{print $1}')
        [[ "$state" == 4 ]] && break
        sleep 1
    done
    [[ "$state" == 4 ]] || { echo "FAIL pp_gov_action: proposal not Succeeded (state=$state) for: $desc"; return 1; }
    cast send "$GOV_ADDR" "execute(address[],uint256[],bytes[],bytes32)(uint256)" \
        "[$target]" "[0]" "[$calldata]" "$desc_hash" \
        --rpc-url "$RPC" --private-key "$(pp_owner_key 0)" >/dev/null
}

# Background value-transfer spammer (PID stored in PP_SPAMMER_PID). Sends 1 wei
# v0→v1 every ~2s; asserts inclusion implicitly via nonce progression. The case
# script checks the chain keeps finalizing across transitions; this just keeps
# user tx pressure on the mempool throughout.
PP_SPAMMER_PID=""
pp_spammer_start() {
    # $1 = funded sender key, $2 = recipient addr. The sender MUST be an account
    # that issues no other txns during the run, else its nonce races the real
    # deploy/registration txns (`replacement transaction underpriced`).
    local from_key="$1" to_addr="$2"
    (
        while :; do
            cast send "$to_addr" --value 1 \
                --rpc-url "$RPC" --private-key "$from_key" >/dev/null 2>&1 || true
            sleep 2
        done
    ) &
    PP_SPAMMER_PID=$!
}
pp_spammer_stop() { [[ -n "$PP_SPAMMER_PID" ]] && kill "$PP_SPAMMER_PID" 2>/dev/null || true; }
