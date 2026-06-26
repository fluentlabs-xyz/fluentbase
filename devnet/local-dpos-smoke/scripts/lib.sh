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
# Default DERIVES from EPOCH_INTERVAL (== bootstrap's `2 * epochBlockInterval`), so a
# tuned interval stays aligned without exporting DPOS_ACTIVATION_BLOCK separately —
# one derivation rule across compose (empty → bootstrap derives), lib.sh, and bootstrap.
DPOS_ACTIVATION_BLOCK="${DPOS_ACTIVATION_BLOCK:-$((2 * EPOCH_INTERVAL))}"

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

# Core alignment poll shared by every converge helper: `reader` is a function
# emitting one "height|hash" reading per line; all readings must be identical,
# the head non-null/non-genesis, and — when a hex `floor` is given —
# numerically GREATER than the floor (a head merely *different* from the floor
# could be a regressed chain). Echoes the aligned reading.
# Usage: _wait_aligned <timeout_s> <floor_hex|""> <reader-fn>
_wait_aligned() {
    local deadline=$(( $(date +%s) + $1 )) floor="$2" reader="$3"
    local readings aligned r head
    [[ "$floor" == 0x* ]] || floor=""   # "null"/empty → no floor check
    while (( $(date +%s) < deadline )); do
        mapfile -t readings < <("$reader")
        if (( ${#readings[@]} > 0 )); then
            aligned=1
            for r in "${readings[@]}"; do [[ "$r" == "${readings[0]}" ]] || aligned=0; done
            head="${readings[0]%%|*}"
            if [[ "$aligned" == 1 && "$head" != "null" && "$head" != "0x0" ]] \
               && { [[ -z "$floor" ]] || (( $(printf '%d' "$head") > $(printf '%d' "$floor") )); }; then
                echo "${readings[0]}"; return 0
            fi
        fi
        sleep 2
    done
    return 1
}

# One "height|hash" reading per line: the 4 genesis validators + full-node.
_read_sequencer_nodes() {
    check_external 8545
    check_node docker compose exec -T validator-1
    check_node docker compose exec -T validator-2
    check_node docker compose exec -T validator-3
    check_external 18545
}

# Wait (default 90s) for all 5 nodes to align finalized > 0; echo "height|hash".
wait_converge() { _wait_aligned "${1:-90}" "" _read_sequencer_nodes; }

# Poll (default 60s) until validator-0's finalized height reaches `target`
# (decimal). Shared by every "wait for block N" loop in the case scripts —
# see finalized_dec's note on the unreachable-RPC→0 coercion.
wait_finalized_ge() {
    local target="$1" deadline=$(( $(date +%s) + ${2:-60} ))
    while (( $(date +%s) < deadline )); do
        (( $(finalized_dec) >= target )) && return 0
        sleep 1
    done
    return 1
}

# Wait (default 240s) until the follower RPC on host port $1 reports the SAME
# "height|hash" as validator-0 with height strictly above the decimal floor
# $2. Echoes the aligned reading. Shared by the cert-follow/cascade cases —
# alignment semantics must not drift between them.
wait_follower_align() {
    local port="$1" floor="$2" deadline=$(( $(date +%s) + ${3:-240} )) v0 f f_h
    while (( $(date +%s) < deadline )); do
        v0=$(check_external 8545); f=$(check_external "$port")
        f_h="${f%%|*}"
        if [[ "$f_h" != "null" && "$f" == "$v0" ]] && (( $(printf '%d' "$f_h") > floor )); then
            echo "$f"; return 0
        fi
        sleep 2
    done
    return 1
}

# --- cast read wrappers (host → validator-0 RPC) ---------------------------

staking_call()     { cast call "$STAKING_ADDR"           "$@" --rpc-url "$RPC"; }
chainconfig_call() { cast call "$CHAIN_CONFIG_ADDR"      "$@" --rpc-url "$RPC"; }
liveness_call()    { cast call "$LIVENESS_SLASHING_ADDR" "$@" --rpc-url "$RPC"; }

# On-chain validator status byte of address $1 (0 inactive, 1 pending, 2 active,
# 3 jailed, 4 exiting) — `getValidatorStatus`'s second tuple field. Shared by every
# case that reads the jail/active status (case-byzantine, the DKG-restart case) so the
# 9-field ABI tuple has ONE definition; a Staking struct-shape change updates it once.
validator_status() {
    staking_call \
        "getValidatorStatus(address)(address,uint8,uint256,uint32,uint64,uint64,uint64,uint16,uint96)" \
        "$1" 2>/dev/null | sed -n '2p' | tr -d ' '
}

# 0-based peer-pubkey-sorted committee index of address $1 in epoch $2, or -1 if
# absent. Shared by every liveness-slash case (case-liveness, the DKG-restart case).
signer_idx() {
    local addr="${1,,}" epoch="$2" comm
    comm=$(staking_call "getEpochCommittee(uint64)(address[])" "$epoch")
    grep -oE '0x[0-9a-fA-F]{40}' <<<"$comm" | nl -ba -v0 \
        | awk -v v="$addr" 'tolower($2)==v{print $1; f=1} END{if(!f) print -1}'
}

# On-chain consecutive-miss count of address $2 in epoch $1. Sentinels: -1 = not in
# committee; -2 = getter call FAILED (RPC error / empty output) — a -2 must be
# retried, NEVER treated as a real 0 (a 0 would wrongly satisfy a "stayed at 0" /
# "below threshold" assertion).
misscount() {
    local epoch="$1" idx out
    idx=$(signer_idx "$2" "$epoch")
    [[ "$idx" == "-1" ]] && { echo -1; return; }
    out=$(liveness_call "missCount(uint64,uint32)(uint32)" "$epoch" "$idx" 2>/dev/null) || true
    [[ -n "$out" ]] && printf '%s\n' "$out" || echo -2
}

# Decimal finalized height of the host-exposed producer (validator-0). NOTE: an
# unreachable RPC coerces to 0 here (same as genesis) — fine for the `>= target`
# polling loops (a transient 0 just costs an iteration), but a 0 must NOT be
# accepted as a PRE/baseline (see baseline_height).
finalized_dec() { printf '%d' "$(check_external 8545 | cut -d'|' -f1)" 2>/dev/null || echo 0; }

# --- beacon / prev_randao smoke helpers (shared by the VRF cases) ----------
# Hoisted from case-vrf.sh / case-vrf-rotation.sh so the VRF / fault / boundary
# cases share ONE copy. Callers define a `NODES` array (the services to compare).

# mixHash (prev_randao) of block $1 as seen by RPC $2 (default $RPC), lowercased.
# GRACEFUL: a not-yet-synced block makes `cast block` exit non-zero — under
# set -e + pipefail that would silently kill the script mid-window instead of the
# intended "node behind" FAIL. Coerce any failure / missing field to "null" so the
# callers (assert_beacon_window / wait_nodes_have) handle a lagging node cleanly.
mixhash_at() {
    { cast block "$1" --rpc-url "${2:-$RPC}" --json 2>/dev/null || echo '{}'; } \
        | jq -r '.mixHash // "null"' 2>/dev/null | tr 'A-F' 'a-f'
}
# mixHash of block $2 (decimal) as seen INSIDE container service $1 (for the
# validators that expose no host RPC port). "null" when absent / RPC unreachable.
mixhash_in() {
    local hexn
    hexn=$(printf '0x%x' "$2")
    # GRACEFUL: a stopped/restarting container or a not-yet-up RPC makes
    # `docker compose exec … curl` exit non-zero (curl 7 = connect failed) — under
    # set -e + pipefail that kills the script instead of yielding the intended
    # "null" (node behind). Coerce any failure to {} so callers handle it cleanly.
    { docker compose exec -T "$1" curl -s -X POST -H 'Content-Type: application/json' \
        --data "{\"jsonrpc\":\"2.0\",\"method\":\"eth_getBlockByNumber\",\"params\":[\"$hexn\",false],\"id\":1}" \
        http://localhost:8545 2>/dev/null || echo '{}'; } \
        | jq -r '.result.mixHash // "null"' 2>/dev/null | tr 'A-F' 'a-f'
}
# mixHash of block $2 (decimal) as seen by node service $1 — routes to the host
# RPC for the two services that publish one (validator-0 → 8545, full-node →
# 18545), else the in-container probe.
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

# Assert prev_randao is non-zero AND byte-identical across all $NODES at every block
# in [$1..$2] (decimal), AND all distinct across heights (real, varying randomness).
# $3 = a label for FAIL messages. Echoes a one-line summary on success.
assert_beacon_window() {
    local lo="$1" hi="$2" label="$3"
    local n svc mh agree distinct
    local mixes=() vals=()
    for ((n = lo; n <= hi; n++)); do
        vals=()
        for svc in "${NODES[@]}"; do
            mh=$(mixhash_of "$svc" "$n")
            if [[ "$mh" == "null" || -z "$mh" ]]; then
                echo "FAIL (beacon-window): $label — $svc has no mixHash for block $n (node behind / RPC down)"
                docker compose logs --tail=80 "$svc"; exit 1
            fi
            if is_zero_hash "$mh"; then
                echo "FAIL (beacon-window): $label — prev_randao is zero at block $n on $svc (beacon stalled / fell to digest)"
                docker compose logs --tail=80 "$svc"; exit 1
            fi
            vals+=("$mh")
        done
        agree=$(printf '%s\n' "${vals[@]}" | sort -u | wc -l)
        if (( agree != 1 )); then
            echo "FAIL (beacon-window): $label — nodes disagree on prev_randao at block $n (divergent seed):"
            paste -d' ' <(printf '%s\n' "${NODES[@]}") <(printf '%s\n' "${vals[@]}") | sed 's/^/  /'
            exit 1
        fi
        mixes+=("${vals[0]}")
    done
    distinct=$(printf '%s\n' "${mixes[@]}" | sort -u | wc -l)
    if (( distinct != ${#mixes[@]} )); then
        echo "FAIL (beacon-window): $label — prev_randao not varying over [$lo..$hi]: ${#mixes[@]} blocks but only $distinct distinct (stuck randomness)"
        printf '  %s\n' "${mixes[@]}"
        exit 1
    fi
    echo "  [$label] blocks [$lo..$hi]: ${#mixes[@]}/${#mixes[@]} distinct non-zero prev_randao, byte-identical across all ${#NODES[@]} nodes"
}

# Wait until EVERY $NODES service has block $1 (decimal) available, up to $2 s
# (default 120). Followers lag the validators by a few blocks, so a cross-node
# window right at a fresh boundary must wait for them first.
wait_nodes_have() {
    local block="$1" deadline=$(( SECONDS + ${2:-120} )) svc mh all
    while true; do
        all=1
        for svc in "${NODES[@]}"; do
            mh=$(mixhash_of "$svc" "$block")
            if [[ "$mh" == "null" || -z "$mh" ]]; then all=0; break; fi
        done
        (( all == 1 )) && return 0
        if (( SECONDS >= deadline )); then
            echo "  [wait_nodes_have] timeout at block $block — per-node status:"
            for svc in "${NODES[@]}"; do
                mh=$(mixhash_of "$svc" "$block")
                if [[ "$mh" == "null" || -z "$mh" ]]; then
                    echo "    $svc: MISSING block $block"
                else
                    echo "    $svc: has block $block"
                fi
            done
            return 1
        fi
        sleep 1
    done
}

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

# --- migration (sequencer → DPoS) ------------------------------------------

# Graceful-stop the 4 validators, verify each persisted via reth's native
# graceful shutdown — exit 0 (retry up to 4×, bringing
# the sequencer back up to let a laggard re-obtain the anchor), then cold-restart them
# under --dpos with the migration anchor. Sets the global PREV_FIN (anchor height,
# hex). Honours $DPOS_EXTRA_COMPOSE (extra `-f file.yml` args, e.g. byzantine).
PREV_FIN="null"
_migrate_to_dpos() {
    local anchor flush_ok=0 attempt v all_flushed
    # R-contract migration: wait until the sequencer finalizes past the (genesis-baked)
    # dposActivationBlock so the swap anchor lands in relative epoch 0
    # ([activation, activation+EPOCH_INTERVAL)). Below activation OriginEpocher
    # rejects the cold-start height; at/above activation+interval the cold-start
    # epoch would be >= 1 and re-hit the empty-marshal genesis lookup (bug #2).
    echo "waiting for the sequencer to finalize >= dposActivationBlock=$DPOS_ACTIVATION_BLOCK (relative epoch 0)"
    local _wstart _fin_hex _fin_dec
    _wstart=$(date +%s)
    while :; do
        _fin_hex=$(check_external 8545 | cut -d'|' -f1)
        if [[ "$_fin_hex" != "null" ]]; then
            _fin_dec=$(printf '%d' "$_fin_hex")
            if (( _fin_dec >= DPOS_ACTIVATION_BLOCK )); then
                echo "  sequencer finalized $_fin_dec >= activation $DPOS_ACTIVATION_BLOCK; proceeding to swap"
                break
            fi
        fi
        if (( $(date +%s) - _wstart > 180 )); then
            echo "FAIL: sequencer did not reach dposActivationBlock=$DPOS_ACTIVATION_BLOCK within 180s"
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
        echo "  bringing the sequencer network back up to reconverge before retry"
        docker compose start "${VALS[@]}"
        wait_converge 90 >/dev/null || { echo "FAIL: sequencer did not reconverge during flush-retry"; docker compose logs --tail=120; tear_down; exit 1; }
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
    # (relative epoch 0). A flush-retry that reconverged the sequencer past the window
    # would trip that fail-loud — reject it here with an actionable message.
    if (( anchor_dec < DPOS_ACTIVATION_BLOCK || anchor_dec >= DPOS_ACTIVATION_BLOCK + EPOCH_INTERVAL )); then
        echo "FAIL: sequencer finalized $anchor_dec outside relative epoch 0 \
[$DPOS_ACTIVATION_BLOCK, $((DPOS_ACTIVATION_BLOCK + EPOCH_INTERVAL))) — a flush-retry likely \
advanced the sequencer past the window. Re-run, or widen the window (raise EPOCH_INTERVAL)."
        tear_down; exit 1
    fi
    # No migration-anchor flags: nodes read dposActivationBlock from the contract
    # and the restart-vs-fresh state from the consensus-archive discriminator.
    # shellcheck disable=SC2086
    docker compose -f docker-compose.yml -f docker-compose.dpos.yml ${DPOS_EXTRA_COMPOSE:-} \
        up -d --force-recreate "${VALS[@]}"
}

# Honest-set reader for wait_dpos_converge: drops $DPOS_CONVERGE_EXCLUDE (e.g.
# a byzantine validator whose reth never finalizes) from the alignment set.
_read_dpos_nodes() {
    local excl="${DPOS_CONVERGE_EXCLUDE:-}"
    [[ "$excl" == "validator-0" ]] || check_external 8545
    [[ "$excl" == "validator-1" ]] || check_node docker compose exec -T validator-1
    [[ "$excl" == "validator-2" ]] || check_node docker compose exec -T validator-2
    [[ "$excl" == "validator-3" ]] || check_node docker compose exec -T validator-3
    check_external 18545
}

# Wait (default 120s) until the honest nodes align finalized strictly past PREV_FIN.
# Normally requires all 5; if $DPOS_CONVERGE_EXCLUDE names a validator, that node
# is dropped from the alignment check and the honest quorum must align instead.
wait_dpos_converge() { _wait_aligned "${1:-120}" "$PREV_FIN" _read_dpos_nodes; }

# One-shot: bring up the full sequencer-era stack, converge, migrate to DPoS, and wait
# until the DPoS chain is live past the anchor. Leaves the stack UP. Used by every
# case-*.sh so each is self-contained.
bring_up_dpos() {
    docker compose up --build -d
    wait_converge 90 >/dev/null || { echo "FAIL: phase1 sequencer did not converge"; docker compose logs --tail=120; tear_down; exit 1; }
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

# Production-path reader: all 6 validators + full-node.
_read_pp_nodes() {
    check_external 8545
    check_node docker compose exec -T validator-1
    check_node docker compose exec -T validator-2
    check_node docker compose exec -T validator-3
    check_node docker compose exec -T validator-4
    check_node docker compose exec -T validator-5
    check_external 18545
}

# Wait (default 90s) until all 6 validators + full-node align finalized > floor.
# $1 = timeout, $2 = floor hex to require strictly past (or "" for >0).
pp_wait_converge() { _wait_aligned "${1:-90}" "${2:-}" _read_pp_nodes; }

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
# the 5-validator voting supply). Votes go out --async, so the 10-block
# votingPeriod (l2.json) only needs to fit their inclusion, not 5 receipt waits.
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
    # --async (no receipt wait): five sequential receipt-awaited sends cost
    # ~1-2 blocks EACH at 1 blk/s and can overrun the 10-block voting window
    # (votingPeriod, l2.json) — the Succeeded poll below is the real
    # synchronization point, and the five voters are distinct keys (no nonce
    # races between them).
    for i in 0 1 2 3 4; do
        cast send "$GOV_ADDR" "castVote(uint256,uint8)(uint256)" "$pid" 1 \
            --rpc-url "$RPC" --private-key "$(pp_owner_key "$i")" --async >/dev/null
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
