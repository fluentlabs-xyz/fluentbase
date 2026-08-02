#!/usr/bin/env bash
# rpc.sh — ONE place for the JSON-RPC-over-curl incantation, its FLK-2 timeouts, and the
# empty-response coercion. Sourced by lib.sh (hence by every case script), so the whole
# suite shares one transport. This collapses the copy-pasted curl incantation (CQ-M1, ~10×)
# and the per-field block getters (CQ-H4, mixhash/blockhash ×6), and quarantines the fragile
# human-readable `cast` parses (CQ-M2) behind cast_field.
#
# Transport rules preserved VERBATIM from the originals (do not "simplify" these away):
#   - host reads add `--max-time 10 --connect-timeout 3` (FLK-2: an up-but-CPU-starved node
#     accepts the socket then never answers; without a bound the whole tick hangs);
#   - in-container reads add an OUTER `timeout` around the whole `docker compose exec` (an
#     unresponsive docker daemon/container can hang the exec itself); curl 10s < timeout 15s
#     so curl's own bound normally fires first (clean sentinel), the outer timeout is the backstop;
#   - an empty/failed response is coerced to `{}` here so a caller piping to jq under
#     set -e + pipefail always gets a clean JSON object, never a mid-window abort.

RPC_MAX_TIME="${RPC_MAX_TIME:-10}"
RPC_CONNECT_TIMEOUT="${RPC_CONNECT_TIMEOUT:-3}"
RPC_EXEC_TIMEOUT="${RPC_EXEC_TIMEOUT:-15}"

# Build a JSON-RPC request body. $1 = method; $2 = raw-JSON params array (default []).
rpc_body() { printf '{"jsonrpc":"2.0","method":"%s","params":%s,"id":1}' "$1" "${2:-[]}"; }

# POST a JSON-RPC body ($2) to a host URL ($1). Echoes the raw response JSON, `{}` if empty/failed.
rpc_post_url() {
    local url="$1" body="$2" out
    out=$(curl -s --max-time "$RPC_MAX_TIME" --connect-timeout "$RPC_CONNECT_TIMEOUT" \
        -X POST -H 'Content-Type: application/json' --data "$body" "$url" 2>/dev/null) || true
    [[ -n "$out" ]] && printf '%s' "$out" || printf '{}'
}

# POST a JSON-RPC body ($1) to a node's in-container RPC (:8545) through a command prefix
# ($@ after the body, e.g. `docker compose exec -T validator-2`). Outer-bounded by `timeout`.
# Echoes the raw response JSON, `{}` if empty/failed.
rpc_post_exec() {
    local body="$1"; shift
    local out
    out=$(timeout "$RPC_EXEC_TIMEOUT" "$@" curl -s --max-time "$RPC_MAX_TIME" --connect-timeout "$RPC_CONNECT_TIMEOUT" \
        -X POST -H 'Content-Type: application/json' --data "$body" http://localhost:8545 2>/dev/null) || true
    [[ -n "$out" ]] && printf '%s' "$out" || printf '{}'
}

# GET a plain text endpoint (Prometheus /metrics) at a host URL. "" if unreachable.
metrics_get_url() {
    curl -s --max-time "$RPC_MAX_TIME" --connect-timeout "$RPC_CONNECT_TIMEOUT" "$1" 2>/dev/null || true
}
# GET a plain text endpoint ($1, e.g. http://localhost:9100/metrics) via a command prefix
# ($@ after the url). Outer-bounded like every exec read. "" if unreachable.
metrics_get_exec() {
    local url="$1"; shift
    timeout "$RPC_EXEC_TIMEOUT" "$@" curl -s --max-time "$RPC_MAX_TIME" --connect-timeout "$RPC_CONNECT_TIMEOUT" "$url" 2>/dev/null || true
}

# --- block-field getters (CQ-H4) -------------------------------------------
# GRACEFUL everywhere: a not-yet-synced block / unreachable RPC yields "null" (never trips
# set -e + pipefail), so callers handle a lagging node cleanly. All results lowercased.

# Field $2 of block $1 via the HOST `cast block` (validator-0/full-node publish a host RPC);
# $3 overrides the RPC url (default $RPC). Note: `cast block --json` returns the block object
# at top level, so the jq path is `.<field>` (no `.result` wrapper).
_block_field_at() {
    { cast block "$1" --rpc-url "${3:-$RPC}" --json 2>/dev/null || echo '{}'; } \
        | jq -r ".$2 // \"null\"" 2>/dev/null | tr 'A-F' 'a-f'
}
# Field $3 of block $2 (decimal) as seen INSIDE container service $1 (validators with no host
# RPC port), via an in-container eth_getBlockByNumber. jq path is `.result.<field>`.
_block_field_in() {
    local hexn; hexn=$(printf '0x%x' "$2")
    rpc_post_exec "$(rpc_body eth_getBlockByNumber "[\"$hexn\",false]")" docker compose exec -T "$1" \
        | jq -r ".result.$3 // \"null\"" 2>/dev/null | tr 'A-F' 'a-f'
}
# Field $3 of block $2 as seen by node service $1 — routes to the host RPC for the two services
# that publish one (validator-0 → $RPC, full-node → 18545), else the in-container probe.
_block_field_of() {
    case "$1" in
        validator-0) _block_field_at "$2" "$3" ;;
        full-node)   _block_field_at "$2" "$3" "http://localhost:18545" ;;
        *)           _block_field_in "$1" "$2" "$3" ;;
    esac
}

# --- fragile human-readable `cast` parse, QUARANTINED (CQ-M2) ---------------
# `cast call ... (typeN)` prints one value per line, right-justified with leading spaces; a
# multi-field tuple prints one field per line after a leading blank. Historically parsed inline
# with `sed -n '2p' | tr -d ' '` / `awk '{print $1}'`, which fail-OPEN to ""/0 on any foundry
# output-format change (one such break shipped a 229200 bug). One definition here so a format
# change is fixed in ONE place. (Single-scalar reads elsewhere use the robust `${out%% *}` /
# `awk '{print $1}'` idiom in place — it was the MULTI-FIELD tuple parse that shipped the bug.)
#   cast_field <line-no> <cast-output>   → the <line-no>'th line, whitespace-stripped.
cast_field() { sed -n "${1}p" <<<"$2" | tr -d ' '; }
