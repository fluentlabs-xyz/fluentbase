#!/usr/bin/env bash
# Shared helpers for the DPoS smoke regression cases. Sourced by assert-smoke.sh
# (phase1/phase2 happy-path) and every scripts/case-*.sh. Extracted so each case
# is self-contained and the RPC/convergence/migration logic lives in one place.
#
# Callers must `cd "$(dirname "$0")/.."` (the local-dpos-smoke dir) before sourcing,
# and run under `set -euo pipefail`.

# The JSON-RPC-over-curl transport layer (rpc_post_url/_exec, block-field getters, the
# quarantined cast parse) lives in rpc.sh; sourced relative to THIS file so it resolves
# regardless of the caller's cwd. Every case script gets it transitively via lib.sh.
# shellcheck source=rpc.sh
source "$(dirname "${BASH_SOURCE[0]}")/rpc.sh"

JSONRPC_FINALIZED="$(rpc_body eth_getBlockByNumber '["finalized",false]')"

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
    # FLK-2 bounds (outer `timeout` + curl --max-time/--connect-timeout) and the empty→`{}`
    # coercion now live in rpc_post_exec; jq on `{}` yields "null|null", the same
    # unreachable-node sentinel the convergence guards expect.
    local out
    out=$(rpc_post_exec "$JSONRPC_FINALIZED" "$@" \
        | jq -r '(.result.number // "null") + "|" + (.result.hash // "null")' 2>/dev/null) || true
    [[ -n "$out" ]] && printf '%s\n' "$out" || echo "null|null"
}
check_external() {
    # Same empty-stdin caveat as check_node — rpc_post_url coerces "" → "{}" → "null|null".
    local port="$1" out
    out=$(rpc_post_url "http://localhost:${port}" "$JSONRPC_FINALIZED" \
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

# Marshal ack-death tripwire, SCOPED to graceful, unhalted stops. The commonware
# marshal dies (its run() returns; the node degrades to a zombie that serves no
# blocks/certs) the moment any `Exact` ack is dropped — the line it logs first is
# "application did not acknowledge block". On a GRACEFUL stop that line must be
# ABSENT: commonware's teardown drops the executor/marshal unpolled, so the
# marshal never observes the cancellation. The line IS legitimate on
# NON-graceful paths, which this tripwire scopes out:
#   1. AbortAll (a subsystem exit): outer.rs aborts the executor one statement
#      before the marshal, so a multi-threaded runtime can poll the marshal in
#      the gap — discriminated by the supervisor's own exit lines, "subsystem
#      failed" (a crash) / "subsystem exited cleanly (unexpected)" (a clean
#      exit; also AbortAll, logged at warn with no "failed" line).
#   2. A SafetyHalt (result divergence / el_invalid / l1_fork) — the soak's
#      COV-1 dpos_sync_degraded scrape hard-fails the run on any UNEXPECTED
#      halt, so within the soak a graceful stop is unhalted by construction;
#      a case that DELIBERATELY provokes a halt is discriminated post-mortem
#      by the executor's own "SafetyHalt" log line. (Since the ack-retention
#      fix a halt no longer emits the ack line at all — the executor parks
#      holding every ack — so this scope-out only shields pre-fix binaries and
#      the deliberate-halt cases from a false trip.)
# EVENT CORRELATION: `since` ($2) is a UTC RFC3339 cursor the caller captures
# IMMEDIATELY BEFORE issuing the stop — both the tripwire grep and the
# scope-out grep run only over the log slice from that cursor onward
# (`docker logs --since`, daemon-side host timestamps, so the host cursor is
# comparable), so an unrelated halt/crash line hours earlier in the container
# log cannot mask a genuine ack-drop during THIS stop. Docker-logs read-race
# belt (the 2026-06-05 flush-race lesson): the slice is re-read a few bounded
# times while it comes back EMPTY (a graceful stop always logs something), so
# a lagging log read is not mistaken for a clean slice. Without a cursor it
# degrades to a whole-log `--tail` check (weaker: pre-existing lines both trip
# and scope out).
# Returns 0 (clean or scoped-out) / 1 (tripped; the caller fail-louds).
marshal_ack_tripwire() {
    local v="$1" since="${2:-}" logs i
    for i in 1 2 3 4 5; do
        if [[ -n "$since" ]]; then
            logs=$(docker compose logs --no-color --since "$since" "$v" 2>/dev/null) || return 0
        else
            logs=$(docker compose logs --no-color --tail 20000 "$v" 2>/dev/null) || return 0
        fi
        [[ -n "$logs" ]] && break
        sleep 1
    done
    if ! grep -q "application did not acknowledge block" <<<"$logs"; then
        return 0
    fi
    if grep -Eq "SafetyHalt|subsystem failed|subsystem exited cleanly \(unexpected\)" <<<"$logs"; then
        return 0
    fi
    echo "TRIPWIRE ($v): marshal logged 'application did not acknowledge block' on a graceful," \
         "unhalted stop — an Exact ack was dropped somewhere (marshal-death bug)" >&2
    return 1
}

# THE alignment rule, in ONE place — the per-poll verdict over a set of readings.
# `_wait_aligned` below is this function plus a deadline; every OTHER loop that has to
# do something else on the same iteration (sample a gauge, probe the peer count) calls
# THIS directly rather than growing another copy of the comparison. A fifth hand-rolled
# copy of "are these nodes on the same chain" is itself the bug this rule exists to fix,
# which is why the split mirrors converge.py's `aligned_reading` / `wait_aligned`.
#
# Usage: _aligned_now <floor_hex|""> <reading> [<reading>...]   — PINNED PRODUCER FIRST.
# Echoes the producer's reading and returns 0 when the set IS convergence, else 1.
#
# Every reading's head must be non-empty, non-null and non-genesis and — when a hex
# `floor` is given — numerically GREATER than the floor (a head merely *different* from
# the floor could be a regressed chain). Cross-node agreement is checked at EACH READER'S
# OWN HEIGHT: the producer's block at that height must be that reader's block.
#
# SAME-HEIGHT identity, NOT tip identity — the rule de2de13c established for
# wait_follower_align, applied to its sibling. This used to demand one byte-identical
# "height|hash" from every reader, i.e. that up to SEVEN independently-advancing tips
# coincide across a read sweep that is not even close to atomic: v0 by host curl, then
# five sequential `docker compose exec` round-trips, then the full-node by host curl,
# several seconds end to end on a 1 block/s chain. A single node steadily one block
# behind — which is normal, not a fault — fails EVERY poll to the deadline. That is
# what smoke-production-path hit: validator-5, the sole node on the WS follower
# upstream, sat exactly one block behind in 26 of 26 samples while the other six
# agreed and the chain climbed past 383, healthy.
#
# The FORK check is what the compare existed for, and it is kept in full: a node at
# height h on a DIFFERENT chain still fails, because the producer's block at h is not
# its block. Only the demand that the tips coincide is dropped. `blockhash_at` coerces
# a block the producer does not hold (or an unreachable producer) to "null", which
# never matches a real hash, so a ragged set never passes by accident.
#
# All-identical readings short-circuit before any producer read: when every reader
# reports the same height AND the same hash they already agree with each other, so the
# extra `cast block` per distinct height would only re-confirm it. That is also why a
# single-reading reader (`_read_cascade_node` — the L3 downstream, a DIFFERENT chain
# from $RPC) keeps its old semantics exactly: one reading is trivially self-identical,
# so it is floor-checked and never compared against the L2 producer.
_aligned_now() {
    local floor="$1"; shift
    (( $# > 0 )) || return 1
    local identical=1 r head hash h_dec p_hash
    local floor_dec=0
    # A floor is honoured only when HEX; "null"/empty/decimal → floor 0, i.e. just "past genesis".
    if [[ "$floor" == 0x* ]]; then floor_dec=$(printf '%d' "$floor"); fi
    for r in "$@"; do [[ "$r" == "$1" ]] || identical=0; done
    local -A producer_hash=()
    for r in "$@"; do
        head="${r%%|*}"; hash="${r##*|}"
        [[ -n "$head" && "$head" != "null" && "$head" != "0x0" ]] || return 1
        h_dec=$(printf '%d' "$head" 2>/dev/null) || return 1
        (( h_dec > floor_dec )) || return 1
        if (( identical == 0 )); then
            p_hash="${producer_hash[$h_dec]:-}"
            if [[ -z "$p_hash" ]]; then
                p_hash=$(blockhash_at "$h_dec"); producer_hash[$h_dec]="$p_hash"
            fi
            [[ "$p_hash" != "null" && "${p_hash,,}" == "${hash,,}" ]] || return 1
        fi
    done
    echo "$1"
    return 0
}

# Core alignment poll shared by every converge helper: `reader` is a function
# emitting one "height|hash" reading per line (the pinned producer FIRST); each poll
# is handed to `_aligned_now`, which owns the rule. Echoes the producer's reading.
# Usage: _wait_aligned <timeout_s> <floor_hex|""> <reader-fn>
_wait_aligned() {
    local deadline=$(( $(date +%s) + $1 )) floor="$2" reader="$3"
    local readings out
    while (( $(date +%s) < deadline )); do
        mapfile -t readings < <("$reader")
        if (( ${#readings[@]} > 0 )) && out=$(_aligned_now "$floor" "${readings[@]}"); then
            echo "$out"; return 0
        fi
        sleep 1
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

# Wait (default 90s) until all 5 nodes are finalized > 0 and on the producer's chain
# at their own heights; echo the producer's "height|hash".
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

# Wait (default 240s) until the follower RPC on host port $1 is finalized strictly
# above the decimal floor $2 AND validator-0 agrees on the block AT THAT HEIGHT
# (same hash). Echoes the follower's aligned reading. Shared by the cert-follow/
# cascade cases — alignment semantics must not drift between them.
#
# SAME-HEIGHT identity, NOT tip identity. This used to require the follower's whole
# "height|hash" reading to equal validator-0's, which is a condition about two
# separate, non-atomic RPC round-trips against two independently-advancing tips: it
# can only hold when both happen to sit on the same height across two different
# wall-clock instants. At ~1 block/s a perfectly healthy follower one block behind
# fails EVERY poll until the deadline, and that is what it did — smoke-cert-follow
# phase 2 (restart into pure live-tail) and smoke-cert-cascade tier-1 (L1-checkpoint
# replay) both failed on an idle host with followers hundreds of blocks past the
# floor and no WARN/ERROR in their logs. The three call sites that passed were the
# three taken right after a cold-start jump LANDS on v0's finalized, i.e. the only
# ones with a coinciding instant. It was never correct, it was lucky.
#
# The hash leg is kept because the fork check is the point: v0's block at the
# follower's own height must be the follower's block, so a follower at height h on a
# DIFFERENT chain still fails. What is dropped is only the demand that two moving
# tips coincide. The "null" pre-test stays — an unreachable follower reads the
# "null|null" sentinel and must never count as agreement — and blockhash_at coerces
# a block v0 does not hold (or an unreachable v0) to "null", which never matches.
wait_follower_align() {
    local port="$1" floor="$2" deadline=$(( $(date +%s) + ${3:-240} )) f f_h f_hash h_dec v0_hash
    while (( $(date +%s) < deadline )); do
        f=$(check_external "$port"); f_h="${f%%|*}"; f_hash="${f##*|}"
        if [[ "$f_h" != "null" ]] && (( $(printf '%d' "$f_h") > floor )); then
            h_dec=$(printf '%d' "$f_h")
            v0_hash=$(blockhash_at "$h_dec")
            if [[ "$v0_hash" != "null" && "${v0_hash,,}" == "${f_hash,,}" ]]; then
                echo "$f"; return 0
            fi
        fi
        sleep 2
    done
    return 1
}

# $1 = RPC url → the 128-hex enode pubkey of that node (via admin_nodeInfo). The
# embedded IP in admin_nodeInfo is unreliable inside docker, so callers rebuild
# the enode with the node's fixed compose IP + devp2p port. Shared by the cascade
# cases (case-soak / case-tx-cascade).
_enode_pubkey() {
    cast rpc --rpc-url "$1" admin_nodeInfo 2>/dev/null \
        | jq -r '.enode // empty' | sed -E 's#^enode://([0-9a-fA-F]+)@.*#\1#'
}

# --- cast read wrappers (host → validator-0 RPC) ---------------------------

staking_call()     { cast call "$STAKING_ADDR"           "$@" --rpc-url "$RPC"; }
chainconfig_call() { cast call "$CHAIN_CONFIG_ADDR"      "$@" --rpc-url "$RPC"; }
liveness_call()    { cast call "$LIVENESS_SLASHING_ADDR" "$@" --rpc-url "$RPC"; }

# On-chain validator status byte of address $1 (0 inactive, 1 pending, 2 active,
# 3 jailed, 4 exiting) — `getValidatorStatus`'s second tuple field. Shared by every
# case that reads the status (case-byzantine, the DKG-restart case) so the 6-field ABI
# tuple has ONE definition; a Staking struct-shape change updates it once. The tuple lost
# `slashesCount` and `jailedBefore` with the liveness jail, and a STALE signature here does
# not error — cast decodes garbage or truncates — so it must never be copied per-case.
validator_status() {
    local out
    out=$(staking_call \
        "getValidatorStatus(address)(address,uint8,uint256,uint64,uint64,uint16)" \
        "$1" 2>/dev/null)
    cast_field 2 "$out"   # 2nd tuple field (status byte); CQ-M2 parse quarantined in rpc.sh
}

# 0-based peer-pubkey-sorted committee index of address $1 in epoch $2, or -1 if
# absent. The index space `ProductionLiveness.producedAt` is keyed by — the same order the
# consensus committee BiMap uses.
signer_idx() {
    local addr="${1,,}" epoch="$2" comm
    comm=$(staking_call "getEpochCommittee(uint64)(address[])" "$epoch")
    grep -oE '0x[0-9a-fA-F]{40}' <<<"$comm" | nl -ba -v0 \
        | awk -v v="$addr" 'tolower($2)==v{print $1; f=1} END{if(!f) print -1}'
}

# Blocks PRODUCED by address $2 in epoch $1, echoed as "<produced> <blocksInEpoch>":
# <produced> = blocks the member led and got finalized, <blocksInEpoch> = blocks recorded for the
# epoch (`ProductionLiveness.producedAt(uint64,uint32)` / `.blocksInEpoch(uint64)`).
#
# This REPLACES the retired `participation()` bitmap reader. The difference is not cosmetic: the
# bitmap was proposer-asserted and never crypto-checked, and its first-quorum-tail exclusion
# wrongly credited a fully-live geo-distant member with a small fraction of certs — the fairness
# defect the whole change exists to remove. `producedAt` counts a 2-byte record that EVERY honest
# voter checked against the consensus-supplied round leader, so it is exact.
#
# Counters PERSIST after the epoch closes (never cleared), so a read is valid before or after it.
# Sentinels (BOTH fields set together): "-1 -1" = not in committee; "-2 -2" = the getter call
# FAILED (RPC error / empty / partial parse) — a -2 must be retried, NEVER treated as real 0s.
produced_in_epoch() {
    local epoch="$1" idx out produced total
    idx=$(signer_idx "$2" "$epoch")
    [[ "$idx" == "-1" ]] && { echo "-1 -1"; return; }
    out=$(liveness_call "producedAt(uint64,uint32)(uint32)" "$epoch" "$idx" 2>/dev/null) || true
    produced=$(cast_field 1 "$out")
    out=$(liveness_call "blocksInEpoch(uint64)(uint32)" "$epoch" 2>/dev/null) || true
    total=$(cast_field 1 "$out")
    [[ "$produced" =~ ^[0-9]+$ && "$total" =~ ^[0-9]+$ ]] || { echo "-2 -2"; return; }
    printf '%s %s\n' "$produced" "$total"
}

# Decimal finalized height of the host-exposed producer (validator-0). NOTE: an
# unreachable RPC coerces to 0 here (same as genesis) — fine for the `>= target`
# polling loops (a transient 0 just costs an iteration), but a 0 must NOT be
# accepted as a PRE/baseline (see baseline_height).
finalized_dec() { printf '%d' "$(check_external 8545 | cut -d'|' -f1)" 2>/dev/null || echo 0; }

# --- beacon / prev_randao smoke helpers (shared by the VRF cases) ----------
# Hoisted from case-vrf.sh / case-vrf-rotation.sh so the VRF / fault / boundary
# cases share ONE copy. Callers define a `NODES` array (the services to compare).

# mixHash (prev_randao) / blockHash getters. These are thin field-specific wrappers over the
# generic _block_field_{at,in,of} in rpc.sh (CQ-H4 collapsed the 6 near-identical copies into
# one field-parameterized family): `_at` reads the host `cast block`, `_in` the in-container
# eth_getBlockByNumber, `_of` routes validator-0/full-node→host else→in-container. Same GRACEFUL
# "null" coercion so a not-yet-synced block / unreachable RPC never trips set -e + pipefail; the
# `_of` fork detector (soak-invariants.sh G1) reads a DIRECT block hash, not the prev_randao proxy.
mixhash_at()   { _block_field_at "$1" mixHash "${2:-$RPC}"; }
mixhash_in()   { _block_field_in "$1" "$2" mixHash; }
mixhash_of()   { _block_field_of "$1" "$2" mixHash; }
blockhash_at() { _block_field_at "$1" hash "${2:-$RPC}"; }
blockhash_in() { _block_field_in "$1" "$2" hash; }
blockhash_of() { _block_field_of "$1" "$2" hash; }
is_zero_hash() { [[ "$1" =~ ^0x0+$ ]]; }
# Count log lines matching $2 in service $1 (0 on no match — never trips set -e).
log_count() { docker compose logs "$1" 2>/dev/null | grep -c "$2" || true; }

# Strip SGR/ANSI escape sequences from a log stream (stdin → stdout). MANDATORY before ANY
# machine-parse (grep -F / awk / `${var#key=}` param-expansion) of container logs: the fluent node
# emits APPLICATION-EMBEDDED ANSI around every structured key=value pair, so a single field renders as
# `height` ESC[0m ESC[2m `=` ESC[0m `2295` — the `=` is split off from both `height` and its value, and
# a `grep -F "height=2295"` / `"${line#*seed_round=}"` extraction can NEVER match. `docker compose logs
# --no-color` does NOT help: it suppresses only compose's OWN prefix coloring, not the escapes the
# application wrote into the log text. Four VRF/byzantine case scripts each carried a private copy of
# this (retired to point here); the soak battery's dpos::derive_seed readers (_inv_seed_round /
# _spec_derive_paths) were vacuous for want of it (the v57 run-killer — the starvation belt correctly
# caught the silent reader). Byte-identical to the retired copies.
strip_ansi() { sed -E 's/\x1b\[[0-9;]*m//g'; }

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
    hx=$(rpc_post_exec "$(rpc_body net_peerCount)" docker compose exec -T "$svc" \
        | jq -r '.result // empty')
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
    #
    # HAZARD, for whoever next raises EPOCH_BLOCK_INTERVAL in a case script: the 180s below is a
    # CONSTANT and the thing it waits for is NOT. DPOS_ACTIVATION_BLOCK derives to
    # 2 * EPOCH_INTERVAL (:33) and the sequencer produces ~1 blk/s from block 0, so this wait
    # costs roughly 2 * EPOCH_INTERVAL seconds: ~64s at the default 32, ~128s at the tuned 64
    # (already only 40% under the budget), and ~256s at 128 -- which does NOT fit. It passes today
    # by luck of the block rate, not by design.
    #
    # Left as a constant ON PURPOSE. Deriving it (e.g. max(180, 2*activation)) would widen the
    # timeout for every case in the suite, changing how long a genuinely stuck bring-up takes to
    # report -- a shared-helper behaviour change paid by twenty green cases to serve a tuned one.
    # A case that needs a bigger activation should carry its own wait, not retune this.
    # (Interval 128 is separately known not to boot at all -- see case-cert-catchup.sh's header.)
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
        sleep 1
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

# Wait (default 120s) until every honest node is finalized strictly past PREV_FIN and
# on the producer's chain at its own height. Normally requires all 5; if
# $DPOS_CONVERGE_EXCLUDE names a validator, that node is dropped from the set and the
# honest quorum must satisfy it instead.
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

tear_down() { docker compose down -v --remove-orphans 2>/dev/null || true; soak_data_root_wipe || true; }

# Wipe the relocated SOAK_DATA_ROOT bind target. `docker compose down -v` removes the
# named-volume METADATA but never touches a bind mount's host contents, so when the soak
# relocates /runtime onto a bind (SOAK_DATA_ROOT set, gen-soak-compose.sh) we must clear
# $SOAK_DATA_ROOT/runtime explicitly to preserve the fresh-state-each-run semantics the
# named-volume down -v gave. reth writes the tree as ROOT, so a host-side `rm -rf` by the
# invoking user hits Permission-denied on the root-owned subdirs — the clear runs as root
# inside a throwaway busybox container instead. CONTENTS only (keep the dir so the bind
# mount stays valid). No-op when SOAK_DATA_ROOT is unset. Belt-and-braces foot-gun guard
# (mirrors gen-soak-compose.sh) so this never targets `/`.
soak_data_root_wipe() {
    local root="${SOAK_DATA_ROOT:-}"
    [[ -n "$root" ]] || return 0
    root="${root%/}"
    case "$root" in
        ""|"/") echo "soak_data_root_wipe: refusing unsafe SOAK_DATA_ROOT='$SOAK_DATA_ROOT'" >&2; return 1;;
        /?*) : ;;
        *) echo "soak_data_root_wipe: SOAK_DATA_ROOT must be an absolute path (got '$SOAK_DATA_ROOT')" >&2; return 1;;
    esac
    local dev="$root/runtime"
    mkdir -p "$dev" 2>/dev/null || true
    # FAIL-LOUD (was silently `|| true` + `exit 0`): a wipe that did NOT actually clear the tree
    # leaves a leaked reth datadir → owner-0 is already past nonce 0 at the next deploy → the
    # staking-address prediction drifts and the run dies opaquely at bring-up (v54). Clear the
    # CONTENTS as root inside a throwaway busybox (reth writes root-owned files the host user
    # can't rm), then VERIFY the dir is empty IN THE SAME container and exit nonzero otherwise;
    # a docker-run failure (daemon down / image pull) also surfaces as a nonzero rc, never a
    # false "wiped". The caller (bring-up preflight) fail-louds on the nonzero return.
    local out rc
    if out=$(docker run --rm -v "$dev:/wipe" busybox sh -c \
        'rm -rf /wipe/* /wipe/.[!.]* /wipe/..?* 2>/dev/null; n=$(ls -A /wipe 2>/dev/null | wc -l); [ "$n" = 0 ] || { echo "remaining=$n"; exit 3; }' 2>&1); then
        rc=0
    else
        rc=$?
    fi
    if (( rc != 0 )); then
        echo "soak_data_root_wipe: FAILED to clear $dev (rc=$rc ${out:-}) — a leaked reth datadir would shift owner-0's deploy nonce and drift the staking-reader prediction; refusing to proceed" >&2
        return 1
    fi
    return 0
}

# RAM-mode purge-on-fail (bundle-20260717T-hostfreeze). When a run FAILS with a /dev/shm data root,
# the tmpfs pages the reth trees occupy are NOT released by teardown alone: `docker compose down -v`
# drops the named volume, but the bind target's files are host-owned and stay resident until something
# unlinks them. So a failed RAM soak keeps holding gigabytes of RAM. This releases the whole subtree
# with a HOST-side `rm -rf` (deliberately NOT the container-mount wipe soak_data_root_wipe uses: under
# the very memory exhaustion that triggers this, `docker run busybox` may fail to start). The unlink is
# safe while nodes still hold the files open — POSIX reclaims the tmpfs pages when the last fd closes
# (the on_exit `down -v` moments later). Called from fail_bundle AFTER bundle collection completes
# (the bundle reads docker logs under /var/lib/docker, untouched by this).
# HARD-GUARDED: fires ONLY when SOAK_DATA_ROOT resolves STRICTLY under /dev/shm/ (a non-empty component
# past it) — never /dev/shm itself, never `/`, never a non-tmpfs path, never a `..`-traversing path.
# No-op (returns 0, silent) for a disk-backed or unset root — a clean/disk teardown keeps using the
# container wipe above. Only the soak calls this; inert elsewhere.
soak_ram_root_purge() {
    local root="${SOAK_DATA_ROOT:-}"
    [[ -n "$root" ]] || return 0
    root="${root%/}"
    [[ "$root" == *".."* ]] && { echo "soak_ram_root_purge: refusing '..'-traversing SOAK_DATA_ROOT='$SOAK_DATA_ROOT'" >&2; return 0; }
    case "$root" in
        /dev/shm/?*) : ;;                        # strictly under /dev/shm/ with a real component
        *) return 0 ;;                           # /dev/shm itself, /, or any non-tmpfs path → not ours to purge
    esac
    echo "soak_ram_root_purge: releasing RAM data root $root (post-fail tmpfs purge — host-side rm)" >&2
    rm -rf -- "$root" 2>/dev/null || true
}

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
# Committee size is env-overridable (default 5 = byte-identical to before). The
# n=6 byzantine repro (case-byzantine-n6.sh) sets PP_COMMITTEE_SIZE=6.
PP_COMMITTEE_SIZE="${PP_COMMITTEE_SIZE:-5}"

# Read a file out of the runtime docker volume (validator-0 mounts /runtime).
pp_runtime_cat() { timeout 15 docker compose exec -T validator-0 cat "/runtime/$1" 2>/dev/null; }  # FLK-2: bound the exec (pp_owner_addr → this runs per-tick via committee_has; a wedged docker daemon must not hang the whole tick)
# Inverse of pp_runtime_cat: write stdin to a file in the shared runtime volume (validator-0
# mounts /runtime). Used to regenerate staking-reader.json from the DeployStaking manifest's
# ACTUAL deployed addresses (derive-don't-predict — kills the create-nonce prediction fragility).
pp_runtime_write() { timeout 15 docker compose exec -T validator-0 sh -c "cat > /runtime/$1"; }

# Mutate (or read back) a file under v<node-idx>'s beacon dir from a THROWAWAY genesis-init
# container on the shared /runtime volume. $3 = an sh snippet with $f bound to the matched file
# path (empty if none).
#
# THE TRANSPORT IS LOAD-BEARING, not a style choice: `docker compose exec` only works on a RUNNING
# container and exec-on-STOPPED SUCCEEDS while doing nothing. Every file op against a victim that
# is currently down MUST come through here — a silent no-op leaves the case measuring a node it
# never touched.
#
# Captures + echoes the snippet's stdout, and ALWAYS returns 0 (`|| true`) so a bare call or a
# command substitution cannot trip the caller's `set -e` when the inner snippet exits non-zero
# (e.g. an absent file's `[ -n "$f" ]` short-circuit). Callers verify success by checking the
# OUTPUT (the corruption's first-bytes readback), never the exit code.
#
# Shared by case-vrf-dkg-durability (one torn journal) and case-vrf-dkg-halt (two) — the tear
# recipe lives in one place so the two cases cannot drift.
vol_mutate_beacon() {    # <node-idx> <glob> <sh-snippet-using-$f>
    docker compose run --rm --no-deps -T --entrypoint sh genesis-init -c \
        "f=\$(find /runtime/reth-data/v$1 -type f -name '$2' 2>/dev/null | head -1); $3" \
        2>/dev/null || true
}

# Validator <idx> l2 owner key (0x-prefixed) and address, from the bare export.
pp_owner_key()  { printf '0x%s' "$(pp_runtime_cat "keys/owner-$1.hex")"; }
# Owner ADDRESS of validator <idx>, resolved from the DURABLE per-idx private key
# keys/owner-N.hex (written for EVERY idx — genesis `write` for 0..POOL-1, `write-keys`
# for a minted idx >= POOL — output.rs), derived on the host with `cast wallet address`.
# The key files survive the shared-file wholesale rewrites that a stray pool-scoped
# genesis re-run performs, so a minted idx no longer resolves to null when addresses.json
# is reverted to the pool prefix (geo-soak v53 r572). Falls back to the legacy
# jq-on-addresses.json lookup ONLY if the key file is absent (or cast can't derive) — the
# pre-baked/native path stays byte-identical (same address either way). Lowercased to
# match addresses.json's non-checksummed `{:#x}` format.
pp_owner_addr() {
    local key addr
    key=$(pp_runtime_cat "keys/owner-$1.hex" 2>/dev/null)
    if [[ -n "$key" ]]; then
        addr=$(cast wallet address --private-key "0x$key" 2>/dev/null || true)
        if [[ "$addr" == 0x* ]]; then printf '%s\n' "${addr,,}"; return 0; fi
    fi
    pp_runtime_cat addresses.json | jq -r ".validators[$1]"
}

# Emit validator <idx> consensus-key material JSON (validatorAddress,
# blsPubkeyUncompressed, blsPoPUncompressed, peerPubkey, ownerKey) by re-running
# the bootstrap binary's consensus-keys subcommand inside the shared image.
# `--peers` is the POOL SIZE: keys::derive() is peers-INDEPENDENT (derive_32 hashes
# only seed|role|idx — keys.rs), so the byte output for a given idx is identical at
# any peer count; but run_consensus_keys ASSERTS `idx < peers` (main.rs), so a soak
# growing past the production-path 6 (joiners idx 6..N-1) MUST pass the larger pool
# size or the assert aborts and growth silently breaks. PP_PEERS defaults to 6, so
# every existing case (which never sets it) is byte-for-byte unchanged.
pp_consensus_keys() {
    docker compose run --rm --no-deps -T \
        --entrypoint /usr/local/bin/genesis-bootstrap genesis-init \
        consensus-keys --idx "$1" --peers "${PP_PEERS:-6}" --chain-id "$CHAIN_ID" 2>/dev/null
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

# Wait (default 90s) until all 6 validators + full-node are finalized > floor and on the
# producer's chain at their own heights.
# $1 = timeout, $2 = floor hex to require strictly past (or "" for >0).
pp_wait_converge() { _wait_aligned "${1:-90}" "${2:-}" _read_pp_nodes; }

# cast read wrappers against the runtime-deployed addresses (set post-deploy).
pp_staking_call()     { cast call "$STAKING_RT"      "$@" --rpc-url "$RPC"; }
pp_chainconfig_call() { cast call "$CHAIN_CONFIG_RT" "$@" --rpc-url "$RPC"; }

# Validator status BYTE (ValidatorStatus: NotFound=0,Active=1,Pending=2,Jail=3) read
# from the RUNTIME staking contract (STAKING_RT). The production-path/soak deploy
# staking at runtime, so the lib.sh `validator_status` (staking_call→the hardcoded
# 0x..5201 PREDEPLOY in STAKING_ADDR) reads the WRONG, codeless address there and
# always yields "". Use THIS for any runtime-deploy lifecycle assertion.
pp_validator_status() {
    local out
    out=$(pp_staking_call "getValidatorStatus(address)(address,uint8,uint256,uint64,uint64,uint16)" "$1" 2>/dev/null)
    cast_field 2 "$out"   # 2nd tuple field (status byte); CQ-M2 parse quarantined in rpc.sh
}

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

# getDkgQual(epoch) — the authoritative on-chain DKG qualification record for an epoch's committee
# (qualify-before-commit, design Phase D). NON-EMPTY bytes ⇒ the incoming committee's DKG QUALIFIED
# by H_qual and committee[epoch] = candidate opened WITH shares; EMPTY bytes ("0x") ⇒ the change
# DEFERRED — the incoming DKG did not qualify, so the incumbent committee[epoch-1] was re-committed
# (committee[epoch] == committee[epoch-1]) and the change retries at a later boundary. Read from the
# RUNTIME staking contract (STAKING_RT), same address family as pp_committee. Echoes the raw hex
# ("0x…", "0x" for empty), or "" on an UNREADABLE read — the pure seams treat "" as UNKNOWN (a
# three-valued state via _dkg_qual_state), never as empty=deferred, so an RPC blip is not mistaken
# for a deferral. Normalizes cast's bool decode to a THREE-valued token: `1` (qualified — the
# candidate DKG met threshold), `0` (deferred — incumbent re-committed), `""` (unreadable/garbled —
# the caller treats it as UNKNOWN via _dkg_qual_state, never as deferred). `|| true`: a
# codeless/reverting call under the caller's pipefail must yield "" the verdict can report, not
# abort the script.
pp_dkg_qual() {
    local out; out=$(pp_staking_call "getDkgQual(uint64)(bool)" "$1" 2>/dev/null | tr -d '[:space:]' || true)
    case "$out" in
        true)  echo 1 ;;
        false) echo 0 ;;
        *)     echo "" ;;
    esac
}

# Retry a ChainConfig read up to 3× (short sleep). Separates a TRANSIENT RPC failure
# (cast exits non-zero → retry, then the caller falls back to its memoized last-good /
# env value) from a legitimate 0 return (echoed immediately, no wasted retries). M4.
_pp_cfg_read_retry() {
    local out i
    for ((i=0;i<3;i++)); do
        if out=$(pp_chainconfig_call "$1" 2>/dev/null); then
            # `cast call …(uintN)` PRETTY-PRINTS any value >= 10000 as "<dec> [<sci>]"
            # (e.g. getDposActivationBlock >= 10000 → "22920 [2.292e4]"). `printf '%d'`
            # on that emits the bare int BUT exits non-zero on the trailing " [..]", so
            # the `|| echo 0` then APPENDS "0" → "229200" (a bogus >head activation that
            # pins pp_current_epoch to 0 forever, silently disabling all soak churn).
            # Strip to the first whitespace token first (house idiom: asserts.sh awk '{print $1}').
            printf '%d' "${out%% *}" 2>/dev/null || echo 0
            return 0
        fi
        sleep 1
    done
    return 1
}

# Decimal current relative DPoS epoch from the host RPC head. The epoch interval + DPoS
# activation block are on-chain CONSTANTS after bring-up, so MEMOIZE the last good read
# and reuse it on an RPC blip (M4): a transient double-read failure must NOT default the
# interval to 64 / activation to 0 (both INFLATE cur_epoch), which false-trips the soak
# growth-landing watchdog (fail_bundle growth). Fail-safe, not fail-loud in the loop.
_PP_INTERVAL_CACHE=""; _PP_ACT_CACHE=""
pp_current_epoch() {
    local head act interval
    head=$(printf '%d' "$(check_external 8545 | cut -d'|' -f1)")
    if interval=$(_pp_cfg_read_retry 'getEpochBlockInterval()(uint32)'); then
        _PP_INTERVAL_CACHE="$interval"
    else
        interval="${_PP_INTERVAL_CACHE:-${EPOCH_INTERVAL:-64}}"   # never the raw 64 in the soak (EPOCH_INTERVAL is exported = SOAK_EPOCH_INTERVAL)
    fi
    if act=$(_pp_cfg_read_retry 'getDposActivationBlock()(uint64)'); then
        _PP_ACT_CACHE="$act"
    else
        act="${_PP_ACT_CACHE:-${DPOS_ACTIVATION_BLOCK:-0}}"
    fi
    (( interval == 0 )) && { echo 0; return; }
    (( head < act )) && { echo 0; return; }
    echo $(( (head - act) / interval ))
}

# seam: current tip block height (decimal) on the primary RPC — the `blocks`-domain cursor for
# the chain-paced primitive + soak_status_assert. Gate-test overrides it to drive them without a
# live chain. "" (non-numeric) on an unreachable RPC → the primitive reads it as read_fail.
_soak_block_number() { cast block-number --rpc-url "$RPC" 2>/dev/null || echo ""; }

# seam: wall seconds since the chain TIP block's timestamp (rough "head age" — a healthy 1 blk/s
# chain reads ~0-2s, a stalled/congested chain climbs). The chain-frozen escape of the chain-paced
# primitive + the mint fund-fail classifier read it. Echoes "?" when the tip timestamp is
# unreadable. gate-test overrides this seam to drive them without a live chain.
_soak_head_age_s() {
    local ts now
    ts=$(cast block latest --field timestamp --rpc-url "$RPC" 2>/dev/null)
    [[ "$ts" =~ ^[0-9]+$ ]] || { echo "?"; return 0; }
    now=$(date +%s)
    echo $(( now - ts ))
}

# ── Family 4: one chain-paced deadline primitive ────────────────────────────
# A pure, resumable verdict machine (chain_paced_step) plus a thin blocking wrapper
# (wait_chain_paced) that together replace the hand-rolled wall-clock-vs-{epoch,chain,
# progress} conversions. The CALLER evaluates its own <met> condition and owns
# attribution; the primitive owns the domain-cursor read, the entry-cursor latch, the
# budget arithmetic, the chain-frozen escape and the shared attribution emitter — the
# part that was re-implemented site-by-site. Lives in lib.sh (the universal base) so the
# lib-only cases (production-path / vrf-*) that drive pp_wait_epochs / pp_gov_action can
# reach it too.
#
# Seams (gate-test overrides): the domain cursor is read via _soak_block_number (blocks),
# pp_current_epoch (epochs) or _soak_now (wall); the `ticks` cursor is an internal per-key
# counter the caller resets on its OWN progress signal (_cp_reset). The chain-frozen escape
# reads _soak_head_age_s (blocks/epochs domains only). All CP_* state is pre-declared for
# set -u; every predicate branch ends in an explicit return (never a bare `(( ))`-`&&` tail,
# the set-e killer); the machine mutates CP_* in the CURRENT shell — NEVER call it via $(...).
declare -gA CP_START CP_TICK        # per-key entry cursor + internal tick counter (set-u safe via ${..:-})
CP_VERDICT="" CP_DETAIL=""          # last step's verdict + a cursor-detail crumb for attribution

# Read the domain cursor into stdout; "" on an unreadable read (blocks/epochs → read_fail). ALWAYS
# returns 0 — a bare `[[ ]] &&` tail would return non-zero on an empty read and (under the caller's
# set -e) abort at `cur=$(_cp_cursor ...)`, the exact set-e-&&-tail gotcha this primitive documents.
_cp_cursor() {
    local v=""
    case "$1" in
        blocks) v=$(_soak_block_number) ;;
        epochs) v=$(pp_current_epoch) ;;
        wall)   v=$(_soak_now) ;;
        ticks)  printf '%s' "${CP_TICK[$2]:-0}"; return 0 ;;
    esac
    if [[ "$v" =~ ^[0-9]+$ ]]; then printf '%s' "$v"; fi
    return 0
}

# Reset a key's entry cursor (ticks-domain progress signal / a fresh window). The NEXT
# chain_paced_step re-latches the entry cursor at the current cursor value.
_cp_reset() { local k="${1:?_cp_reset: key required}"; unset "CP_START[$k]"; }

# chain_paced_step <key> <met> <domain> <budget_n> [frozen_head_age_s=120]
#   <key>   unique site name; state lives in CP_START[<key>] / CP_TICK[<key>].
#   <met>   0|1 — the caller evaluates its own condition and passes the result.
#   <domain> blocks | epochs | ticks | wall ;  <budget_n> budget in that domain.
#   <frozen_head_age_s> chain-frozen escape threshold (blocks/epochs only); 0 disables it
#                       (hold forever — e.g. the dkg-barrier, whose stall is finalize-stall's).
# Echo-free; sets CP_VERDICT = met | waiting | budget | frozen | read_fail  and returns 0.
chain_paced_step() {
    local key="${1:?chain_paced_step: key required}" met="${2:?chain_paced_step: met required}" \
          domain="${3:?chain_paced_step: domain required}" budget="${4:?chain_paced_step: budget required}" \
          froz="${5:-120}"
    CP_VERDICT="" CP_DETAIL=""
    if [[ -n "${SOAK_DRYRUN:-}" ]]; then CP_VERDICT=waiting; return 0; fi   # selfcheck flush: no RPC, no verdict
    # ticks: advance the internal per-key counter each call (progress resets via _cp_reset).
    if [[ "$domain" == ticks ]]; then CP_TICK[$key]=$(( ${CP_TICK[$key]:-0} + 1 )); fi
    local cur; cur=$(_cp_cursor "$domain" "$key")
    if [[ -z "$cur" ]]; then CP_VERDICT=read_fail; return 0; fi   # RPC/read problem — NEVER "the thing didn't happen"
    if [[ -z "${CP_START[$key]:-}" ]]; then CP_START[$key]="$cur"; fi   # latch the entry cursor on first measurable call
    local adv=$(( cur - ${CP_START[$key]} )); if (( adv < 0 )); then adv=0; fi
    CP_DETAIL="domain=$domain budget=$budget entry=${CP_START[$key]} cursor=$cur advanced=$adv"
    if (( met == 1 )); then CP_VERDICT=met; _cp_reset "$key"; unset "CP_TICK[$key]"; return 0; fi
    if (( adv >= budget )); then CP_VERDICT=budget; return 0; fi   # the honest failure: the chain made the agreed progress, condition still unmet
    # chain-frozen escape (blocks/epochs only; froz=0 disables). Attribution DEFERS to the
    # finalize-stall / node-down invariants that own a frozen chain.
    if (( froz > 0 )) && [[ "$domain" == blocks || "$domain" == epochs ]]; then
        local age; age=$(_soak_head_age_s)
        if [[ "$age" =~ ^[0-9]+$ ]] && (( age >= froz )); then
            CP_VERDICT=frozen; CP_DETAIL="$CP_DETAIL head_age=${age}s"; return 0
        fi
    fi
    CP_VERDICT=waiting; return 0
}

# _cp_sleep [s] — overridable inter-poll pause for the blocking wrapper (gate-test → no-op).
_cp_sleep() { sleep "${1:-2}"; }

# _cp_attribute <key> <verdict> <detail> — one soak_event + one FAIL line naming the
# waited-for condition, the domain+budget, the entry/exit cursors and (frozen) the head-age
# with the explicit defer-to-finalize-stall/node-down note. Sets CP_FAIL_MSG so the caller can
# reuse the exact text in its own _die / fail_bundle line.
CP_FAIL_MSG=""
_cp_attribute() {
    local key="$1" verdict="$2" detail="$3" note
    case "$verdict" in
        budget)    note="chain-paced '$key': condition never met while the chain advanced the agreed budget ($detail)" ;;
        frozen)    note="chain-paced '$key': chain FROZEN ($detail) — deferring to the finalize-stall / node-down invariants that own a stalled chain" ;;
        read_fail) note="chain-paced '$key': domain cursor UNREADABLE ($detail) — an RPC/read failure, NOT a missed condition" ;;
        *)         note="chain-paced '$key': $verdict ($detail)" ;;
    esac
    CP_FAIL_MSG="$note"
    # soak_event lives in soak-bundle.sh (case-soak only); the lib-only cases (production-path/vrf-*)
    # that drive pp_wait_epochs / pp_gov_action have no soak_event — fall back to a plain stderr line so
    # a budget/frozen verdict there is still surfaced (and never aborts under set -e on a missing command).
    if declare -F soak_event >/dev/null 2>&1; then soak_event WARN "$note"; else echo "WARN (chain-paced): $note" >&2; fi
}

# wait_chain_paced <key> <cond_fn> <budget_domain> <budget_n> [frozen_head_age_s] [poll_s=2]
# Loops chain_paced_step over the caller's <cond_fn> (a function returning 0 when met); returns
# 0 on `met`; on budget/frozen/read_fail emits the shared attribution and returns 1 (the caller
# decides _die vs fail_bundle). A transient read_fail is tolerated (kept polling) — only a
# PERSISTENT one past the same budget/frozen guard as any wait terminates it, so a single RPC
# blip never fails a blocking wait. MUST run in the current shell (mutates CP_*).
wait_chain_paced() {
    local key="${1:?wait_chain_paced: key required}" cond="${2:?wait_chain_paced: cond_fn required}" \
          domain="${3:?wait_chain_paced: domain required}" budget="${4:?wait_chain_paced: budget required}" \
          froz="${5:-120}" poll="${6:-2}"
    [[ -n "${SOAK_DRYRUN:-}" ]] && return 0
    _cp_reset "$key"; unset "CP_TICK[$key]"
    local met
    while :; do
        if "$cond"; then met=1; else met=0; fi
        chain_paced_step "$key" "$met" "$domain" "$budget" "$froz"
        case "$CP_VERDICT" in
            met)             return 0 ;;
            budget|frozen)   _cp_attribute "$key" "$CP_VERDICT" "$CP_DETAIL"; return 1 ;;
            read_fail)       : ;;   # transient read blip — keep polling; a real stall trips the frozen/budget guard once reads recover
        esac
        _cp_sleep "$poll"
    done
}

# Block until the host RPC head crosses the next epoch boundary $1 times. EPOCH-paced (Family 4
# straggler — retires the old 60·(n+2)s wall guess, which mis-scaled under a degraded block rate):
# the wait now follows the chain and only fails if it FREEZES (head-age escape).
_pp_wait_epochs_reached() { (( $(pp_current_epoch) >= _cp_wep_start + _cp_wep_n )); }
pp_wait_epochs() {
    local _cp_wep_n="$1" _cp_wep_start
    _cp_wep_start=$(pp_current_epoch)
    wait_chain_paced "epochs_${_cp_wep_n}" _pp_wait_epochs_reached epochs "$_cp_wep_n"
}

# Send BLEND <amount> from the deployer (v0) to <to>. $1=token $2=to $3=amount.
pp_token_transfer() {
    cast send "$1" "transfer(address,uint256)(bool)" "$2" "$3" \
        --rpc-url "$RPC" --private-key "$(pp_owner_key 0)" >/dev/null
}

# Idempotent BLEND funding for the mint path. A TRANSIENT cast tx-confirmation timeout must NOT
# fail_bundle a night-long soak: finalization briefly slows during churn/catch-up, so cast's confirm
# window can lapse even though the chain is healthy and the tx will land. A naive re-send is unsafe
# (transfer is not idempotent), so re-check the recipient's balance FIRST — a fresh minted idx starts
# at 0 BLEND, so any non-zero balance == the transfer already landed (even if a prior attempt's receipt
# timed out) → skip. Only re-send while still 0. Bounded (4×). Worst case (a still-pending first send
# re-sent) merely over-funds, which is harmless (the idx needs >= the stake floor; extra BLEND is inert).
pp_ensure_blend() {
    local token="${1:?pp_ensure_blend: token}" to="${2:?pp_ensure_blend: to}" amount="${3:?pp_ensure_blend: amount}" i bal
    for i in 1 2 3 4; do
        bal=$(cast call "$token" "balanceOf(address)(uint256)" "$to" --rpc-url "$RPC" 2>/dev/null | awk '{print $1}' || echo 0)
        [[ "$bal" =~ ^[1-9][0-9]*$ ]] && return 0   # non-zero => already funded (fresh idx starts at 0)
        pp_token_transfer "$token" "$to" "$amount" 2>/dev/null || true   # best-effort; a timed-out send may still land — the next balance check catches it
        sleep 3
    done
    bal=$(cast call "$token" "balanceOf(address)(uint256)" "$to" --rpc-url "$RPC" 2>/dev/null | awk '{print $1}')
    [[ "$bal" =~ ^[1-9][0-9]*$ ]]
}

# Send ETH (wei $2) from owner-0 to $1 (mint-on-demand): a fresh minted idx >= POOL has ZERO on-chain
# ETH (genesis funds only the pre-baked pool) so it needs gas to sign its own register/setkeys/stake.
# owner-0's finite genesis ETH is the TRUE ceiling of unbounded minting, so PRE-CHECK its balance and
# WARN + return non-zero (before the send — the caller fail-louds) when it dips below
# SOAK_MINT_ETH_FLOOR_WEI (~0.05 ETH). owner-0 starts at 1 ETH (1e18 < int64 max) and only decreases,
# so the wei arithmetic never overflows bash's 64-bit ints. SOAK_DRYRUN → early-return (soak_selfcheck
# flush; no cast). Only soak calls this, so the DRYRUN guard is inert for other cases.
#
# bundle-20260716T203448Z: the old single non-zero return CONFLATED three distinct failures that the
# caller then all mislabeled "budget exhausted". Return DISTINCT codes so the caller can attribute
# honestly:
#   rc 1  genuine pre-check floor  — bal < SOAK_MINT_ETH_FLOOR_WEI; PP_FUND_ETH_BAL carries the balance.
#   rc 2  send/confirm failure     — the cast send erred or produced no success receipt (stalled/congested
#                                    chain); PP_FUND_ETH_ERR carries cast's last error line.
#   rc 3  balance-read failure     — a FAILED `cast balance` read must NOT collapse to bal=0 (which would
#                                    fake a floor hit); retry once, then classify as a transient read
#                                    failure, NOT a floor.
# The funding send is fee-hardened: it bids eth_gasPrice×3 (matching the blaster's bid, load-heavy.sh
# lh_gas_price) with a 1-gwei floor so a mint tx isn't gas-starved under sustained load, and it confirms
# via the --json receipt `.status` (mirrors soak_send, soak-actions.sh) instead of a bare default wait.
SOAK_MINT_ETH_FLOOR_WEI="${SOAK_MINT_ETH_FLOOR_WEI:-50000000000000000}"
SOAK_MINT_GAS_FLOOR_WEI="${SOAK_MINT_GAS_FLOOR_WEI:-1000000000}"   # 1 gwei — sane bid floor when eth_gasPrice reads 0
PP_FUND_ETH_BAL="" PP_FUND_ETH_ERR=""
pp_fund_eth() {
    local to="${1:?pp_fund_eth: recipient required}" wei="${2:?pp_fund_eth: wei required}"
    [[ -n "${SOAK_DRYRUN:-}" ]] && return 0
    PP_FUND_ETH_BAL="" PP_FUND_ETH_ERR=""
    local owner bal attempt
    owner=$(pp_owner_addr 0)
    # Balance read with ONE retry. A failed read yields empty/non-numeric — do NOT coerce to 0 (that
    # would masquerade as a floor hit, bundle-20260716T203448Z case 3). Only a numeric read proceeds.
    bal=""
    for attempt in 1 2; do
        bal=$(cast balance "$owner" --rpc-url "$RPC" 2>/dev/null)
        [[ "$bal" =~ ^[0-9]+$ ]] && break
        bal=""
        (( attempt < 2 )) && sleep 2
    done
    if [[ -z "$bal" ]]; then
        echo "WARN (pp_fund_eth): owner-0 balance read FAILED after retry (RPC unreachable/garbled) — NOT treating as floor" >&2
        return 3
    fi
    # Genuine pre-check floor: surface the balance (PP_FUND_ETH_BAL) so the caller reports it honestly.
    if (( ${#bal} <= 18 )) && (( bal < SOAK_MINT_ETH_FLOOR_WEI )); then
        PP_FUND_ETH_BAL="$bal"
        echo "WARN (pp_fund_eth): owner-0 ETH balance ${bal} wei below the ${SOAK_MINT_ETH_FLOOR_WEI} wei floor — mint budget exhausted" >&2
        return 1
    fi
    # Fee-hardened, receipt-confirmed send (bundle-20260716T203448Z case 2).
    local gp bid out st
    gp=$(cast gas-price --rpc-url "$RPC" 2>/dev/null); [[ "$gp" =~ ^[0-9]+$ ]] || gp=0
    bid=$(( gp * 3 )); (( bid < SOAK_MINT_GAS_FLOOR_WEI )) && bid="$SOAK_MINT_GAS_FLOOR_WEI"
    if out=$(cast send --json --rpc-url "$RPC" --gas-price "$bid" \
                 --private-key "$(pp_owner_key 0)" --value "$wei" "$to" 2>&1); then
        st=$(jq -r 'if type=="object" then (.status // .receipt.status // "") else "" end' <<<"$out" 2>/dev/null)
        [[ "$st" == "0x1" || "$st" == "1" ]] && return 0
    fi
    PP_FUND_ETH_ERR="$(printf '%s' "$out" | tail -1)"
    echo "WARN (pp_fund_eth): funding send to ${to} FAILED/unconfirmed: ${PP_FUND_ETH_ERR}" >&2
    return 2
}

# OZ Governor ProposalState names (state(uint256) return byte) for honest failure labels.
_pp_gov_state_name() {
    local names=(Pending Active Canceled Defeated Succeeded Queued Expired Executed)
    [[ "$1" =~ ^[0-7]$ ]] && echo "${names[$1]}" || echo "Unknown"
}
# seams (gate-test overrides): inter-poll pause + wall clock for the frozen-chain escape.
_pp_gov_sleep() { sleep 2; }
_pp_gov_now()   { date +%s; }

# Wait until proposal $1 is Succeeded(4), BLOCK-PACED (bundle-20260716T232341Z — the 6th wall-clock-
# vs-chain instance in this harness). The voting window is BLOCK-denominated (votingPeriod, l2.json),
# so the deadline must be measured in observed chain blocks, not seconds: read the proposal's voting
# end block ONCE (proposalDeadline — the OZ Governor getter on the governance predeploy) and poll
# state each ~2s WITHOUT giving up until the head has advanced PAST end+5 (margin for the state
# transition to surface). Under a blaster-degraded rate the wait simply follows the crawling window.
# Two loud exits remain:
#   - frozen-chain escape: the head not advancing for >=120s is a chain stall, owned by the liveness
#     invariants — fail naming the head age, never spin forever;
#   - genuine terminal non-Succeeded (Canceled/Defeated/Expired, or any non-4 past end+margin): fail
#     with the ACTUAL state name, not a timeout.
# $2 = desc (failure labels). Uses GOV_ADDR/RPC from the caller's scope (same as pp_gov_action).
pp_gov_wait_succeeded() {
    local pid="$1" desc="$2"
    local end state head last_head=-1 last_move now
    end=$(cast call "$GOV_ADDR" "proposalDeadline(uint256)(uint256)" "$pid" --rpc-url "$RPC" 2>/dev/null | awk '{print $1}')
    [[ "$end" =~ ^[0-9]+$ ]] || end=""   # unreadable deadline → rely on the terminal-state + frozen escapes below
    last_move=$(_pp_gov_now)
    while :; do
        state=$(cast call "$GOV_ADDR" "state(uint256)(uint8)" "$pid" --rpc-url "$RPC" 2>/dev/null | awk '{print $1}')
        [[ "$state" == 4 ]] && return 0
        # Terminal non-Succeeded — the vote genuinely failed; name the state, not a timeout.
        if [[ "$state" == 2 || "$state" == 3 || "$state" == 6 ]]; then
            echo "FAIL pp_gov_action: proposal $(_pp_gov_state_name "$state") (state=$state) for: $desc"
            return 1
        fi
        head=$(cast block-number --rpc-url "$RPC" 2>/dev/null); [[ "$head" =~ ^[0-9]+$ ]] || head=-1
        now=$(_pp_gov_now)
        if (( head >= 0 && head != last_head )); then last_head=$head; last_move=$now; fi
        # Past the voting end block + margin and STILL not Succeeded/terminal — fail with the state name.
        if [[ -n "$end" ]] && (( head >= 0 && head > end + 5 )); then
            echo "FAIL pp_gov_action: proposal $(_pp_gov_state_name "$state") (state=$state) past voting end block $end (head=$head, margin 5) for: $desc"
            return 1
        fi
        # Frozen-chain escape: the stall itself belongs to the finalize-stall/node-down invariants.
        if (( now - last_move >= 120 )); then
            echo "FAIL pp_gov_action: chain FROZEN waiting out the voting window (head=$last_head unmoved for $(( now - last_move ))s; voting end block=${end:-unreadable}) for: $desc"
            return 1
        fi
        _pp_gov_sleep
    done
}

# cond_fn for the pp_gov_action Active-poll: reads state(pid)==Active(1), updating the enclosing
# `state` (bash dynamic scope) so the caller's fail message names the last observed state.
_pp_gov_active() {
    state=$(cast call "$GOV_ADDR" "state(uint256)(uint8)" "$pid" --rpc-url "$RPC" 2>/dev/null | awk '{print $1}')
    [[ "$state" == 1 ]]
}

# pp_gov_action execute-confirm knobs (Family 4; bundle-20260717T-govexec). The final `execute` tx is
# confirmed by the sender's nonce advancing (no fee flags → cast auto-estimates ~2·baseFee+tip, which
# ALREADY out-bids the blaster cap — this is NOT a fee problem). SOAK_GOV_CONFIRM_BLOCKS bounds the wait
# by CHAIN PROGRESS (blocks mined on the same RPC) instead of a fixed wall deadline, so a healthy tx
# that merely mines slowly under blaster-degraded throughput is not false-failed; SOAK_GOV_CONFIRM_FROZEN_S
# is the genuine-stall backstop (chain producing zero blocks that long IS a real stall, owned by the
# finalize-stall / node-down invariants). 20 blocks ≈ 20s at the 1 blk/s target, but ~100s at the ~0.2
# blk/s degraded rate that tripped the old 30s deadline — generous enough for slow mining, still short
# enough to catch a genuinely dropped/underpriced tx (an out-bidding tx lands within a few blocks).
SOAK_GOV_CONFIRM_BLOCKS="${SOAK_GOV_CONFIRM_BLOCKS:-20}"
SOAK_GOV_CONFIRM_FROZEN_S="${SOAK_GOV_CONFIRM_FROZEN_S:-120}"
# cond_fn for the execute-confirm wait: 0 once the sender's nonce advanced past its pre-send value (the
# execute tx mined). Updates the enclosing `ex_now` (bash dynamic scope) so the fail message names the
# last observed nonce. A transient unreadable nonce is simply "not advanced" (keep polling) — the
# primitive's frozen/read_fail guards own a persistent stall, not this predicate.
_pp_gov_exec_confirmed() {
    ex_now=$(cast nonce --rpc-url "$RPC" "$ex_sender" 2>/dev/null || echo "")
    [[ "$ex_now" =~ ^[0-9]+$ ]] && (( ex_now > ex_pre ))
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
    # votingDelay=0 → Active within a block. BLOCK-paced (Family 4 straggler — retires the fixed
    # 15×1s wall poll): wait until state==Active, or the chain advances 3 blocks (genuine miss) /
    # freezes. The cond_fn updates the enclosing `state` via bash dynamic scope for the fail message.
    if ! wait_chain_paced "gov_active_$pid" _pp_gov_active blocks 3; then
        echo "FAIL pp_gov_action: proposal not Active (state=$state) for: $desc"; return 1
    fi
    # --async (no receipt wait): five sequential receipt-awaited sends cost
    # ~1-2 blocks EACH at 1 blk/s and can overrun the 10-block voting window
    # (votingPeriod, l2.json) — the Succeeded poll below is the real
    # synchronization point, and the five voters are distinct keys (no nonce
    # races between them).
    # Voter set scales past the hardcoded 5 for the soak's growing committee:
    # vote the first ceil(2/3·voters)+1 owner keys (>= the governance
    # super-majority). PP_GOV_VOTERS defaults to 5, so every existing case (which
    # never sets it) draws exactly `0 1 2 3 4` — byte-identical to before.
    local voters="${PP_GOV_VOTERS:-5}" need j
    if [[ "${PP_GOV_VOTE_ALL:-0}" == 1 ]]; then
        # FluentGovernance quorum is STAKE-WEIGHTED (= 2/3 of the total delegated stake
        # across the active set; FluentGovernance.quorum/_votingSupply). With uneven
        # stake the equal-weight ceil(2/3·count)+1 heuristic below can fall short of
        # 2/3 of the POWER, so a caller over a grown/uneven committee opts into voting
        # EVERY listed validator-owner. All votes are "for", so this only ever ADDS
        # for-power — it can never flip a previously-passing proposal. (Default-off:
        # every existing case keeps the byte-identical ceil(2/3·voters)+1 path.)
        need="$voters"
    else
        need=$(( (2 * voters + 2) / 3 + 1 ))   # ceil(2/3·voters)+1 (equal-weight)
    fi
    (( need > voters )) && need="$voters"
    (( need < 1 )) && need=1
    # PP_GOV_VOTER_IDX (explicit space-list of owner idx) takes precedence over the
    # 0..need-1 prefix. The soak's churn callers pass the CURRENT committee's idx (the
    # live stake-holders): the fixed prefix under-votes once the original pool tombstones
    # and committee power migrates to MINTED identities (idx >= POOL, not in the prefix) →
    # for-power < 2/3 stake quorum → proposal Defeated. Unset/empty → the prefix path,
    # byte-identical for every non-soak caller.
    local voter_list
    if [[ -n "${PP_GOV_VOTER_IDX:-}" ]]; then
        voter_list="$PP_GOV_VOTER_IDX"
    else
        voter_list=""
        for (( j = 0; j < need; j++ )); do voter_list="$voter_list $j"; done
    fi
    for j in $voter_list; do
        cast send "$GOV_ADDR" "castVote(uint256,uint8)(uint256)" "$pid" 1 \
            --rpc-url "$RPC" --private-key "$(pp_owner_key "$j")" --async >/dev/null
    done
    # Wait out the voting period until Succeeded (4), then execute — BLOCK-PACED, not wall-clock
    # (bundle-20260716T232341Z; 6th wall-clock-vs-chain instance). The old fixed 30×1s poll measured
    # the block-denominated voting window in SECONDS: under blaster-degraded rate the window crawled,
    # the attempts ran out while voting was still ACTIVE (state=1), and a healthy-but-slow chain was
    # reported as a governance failure ("proposal not Succeeded (state=1)" at r75/ep10).
    pp_gov_wait_succeeded "$pid" "$desc" || return 1
    # Gate the final `execute` on MINED confirmation the same way soak_send does
    # (soak-actions.sh:590-629). A bare fire-and-forget `cast send >/dev/null` (a) SWALLOWS a
    # genuine on-chain revert and (b) returns BEFORE the tx lands, so the caller's immediate
    # single pp_validator_status read races an unmined execute → the phantom "status=2 after
    # activateValidator (want 1=Active)" even though the activate DID land later (bundle-
    # 20260716T030527Z). Capture the receipt, assert .status==0x1; on a no-receipt transient
    # confirm the tx actually mined via the sender's nonce advancing; fail-loud (non-zero, reason
    # surfaced) on a real revert. NOTE: soak_send lives in soak-actions.sh, which the non-soak
    # pp_gov_action callers (case-production-path, case-vrf-*) do NOT source, so this small
    # confirmation block is DUPLICATED here rather than shared — keep the two idioms in sync.
    local ex_args=( "$GOV_ADDR" "execute(address[],uint256[],bytes[],bytes32)(uint256)" \
        "[$target]" "[0]" "[$calldata]" "$desc_hash" \
        --rpc-url "$RPC" --private-key "$(pp_owner_key 0)" )
    local ex_out ex_st ex_sender ex_pre ex_now
    ex_sender=$(pp_owner_addr 0)
    ex_pre=$(cast nonce --rpc-url "$RPC" "$ex_sender" 2>/dev/null || echo "")
    if ex_out=$(cast send --json "${ex_args[@]}" 2>&1); then
        ex_st=$(jq -r 'if type=="object" then (.status // .receipt.status // "") else "" end' <<<"$ex_out" 2>/dev/null)
        if [[ "$ex_st" != "0x1" && "$ex_st" != "1" ]]; then
            local reason; reason=$(cast call --rpc-url "$RPC" "${ex_args[@]}" 2>&1 | tail -1)
            echo "FAIL pp_gov_action: execute REVERTED (status=$ex_st) for: $desc: $reason"; return 1
        fi
    else
        # No receipt (RPC hiccup / cast timeout): the tx may still be pending-then-mined. Confirm via the
        # sender's nonce advancing past its pre-send value. BLOCK-paced (Family 4; bundle-20260717T-govexec)
        # — the old fixed 30s wall deadline false-FAILED an out-bidding execute that simply mined slowly
        # under blaster-degraded throughput ("execute not confirmed, sender nonce stuck at N"). Bound the
        # wait by chain progress: SOAK_GOV_CONFIRM_BLOCKS blocks mined without the nonce advancing = a
        # genuinely dropped/underpriced tx; a chain frozen for SOAK_GOV_CONFIRM_FROZEN_S = a real stall
        # (deferred to the finalize-stall / node-down invariants). A stalled chain is the ONE case that
        # still times out — the block bound never makes this unconditionally patient.
        if [[ -n "$ex_sender" && "$ex_pre" =~ ^[0-9]+$ ]]; then
            ex_now="$ex_pre"
            if ! wait_chain_paced gov_exec_confirm _pp_gov_exec_confirmed blocks \
                    "$SOAK_GOV_CONFIRM_BLOCKS" "$SOAK_GOV_CONFIRM_FROZEN_S"; then
                echo "FAIL pp_gov_action: execute not confirmed (no receipt, sender nonce stuck at $ex_pre, last read $ex_now) for: $desc — ${CP_FAIL_MSG:-chain-paced budget/stall}"; return 1
            fi
        else
            echo "FAIL pp_gov_action: execute send failed for: $desc: $(printf '%s' "$ex_out" | tail -1)"; return 1
        fi
    fi
}

# Governance kill switch for the production-liveness tier
# (`ChainConfig.setProductionLivenessDisabled`). $1 = true|false. true DISABLES the tier: the
# recorder keeps counting and publishing, but no verdict is evaluated, no exclusion is stamped and
# none EXPIRES — the verdict state is frozen, deliberately unlike the retired participation jail,
# whose cursor advanced anyway so a window finalized while disabled was permanently unjudged.
# Fresh deploys ship the tier OFF (seeded `true`), so the ENABLE call is the load-bearing one here.
# Drives the full propose->vote->execute governance flow via pp_gov_action; requires
# CHAIN_CONFIG_RT/GOV_ADDR populated (post-deploy).
set_production_liveness_disabled() {
    local disabled="$1"
    [[ "$disabled" == "true" || "$disabled" == "false" ]] || {
        echo "FAIL set_production_liveness_disabled: expected true|false, got '$disabled'"; return 1; }
    pp_gov_action "$CHAIN_CONFIG_RT" \
        "$(cast calldata 'setProductionLivenessDisabled(bool)' "$disabled")" \
        "setProductionLivenessDisabled($disabled)"
}

# Background value-transfer spammer (PID stored in PP_SPAMMER_PID). Sends 1 wei
# v0→v1 every ~2s; asserts inclusion implicitly via nonce progression. The case
# script checks the chain keeps finalizing across transitions; this just keeps
# user tx pressure on the mempool throughout.
PP_SPAMMER_PID=""
PP_SPAMMER_PIDS=()
pp_spammer_start() {
    # $1 = funded sender key, $2 = recipient addr, $3 = RPC (default $RPC). The
    # sender MUST be an account that issues no other txns during the run, else its
    # nonce races the real deploy/registration txns (`replacement transaction
    # underpriced`). A NON-default $3 (an L3/L2 follower RPC) routes tx pressure
    # THROUGH the sentry cascade — submit to the follower, which gossips
    # L3→L2→validator into the proposer pool. `timeout` bounds a transient
    # follower wedge (the loop retries instead of hanging); sync `cast send`
    # self-paces the nonce (waits for the mined+synced receipt before the next).
    local from_key="$1" to_addr="$2" rpc="${3:-$RPC}"
    (
        while :; do
            timeout 90 cast send "$to_addr" --value 1 \
                --rpc-url "$rpc" --private-key "$from_key" >/dev/null 2>&1 || true
            sleep 2
        done
    ) &
    PP_SPAMMER_PID=$!
    PP_SPAMMER_PIDS+=("$!")
}
pp_spammer_stop() {
    local p
    for p in "${PP_SPAMMER_PIDS[@]}"; do [[ -n "$p" ]] && kill "$p" 2>/dev/null || true; done
    PP_SPAMMER_PIDS=()
    PP_SPAMMER_PID=""
}

# Bring up the production-path / rotation stack with the initial 5-validator
# committee, identically for every rotation-stack case (case-vrf-rotation +
# case-vrf-dkg-durability). Mirrors case-production-path.sh phases A..cold-restart
# verbatim: bare sequencer chain → fund + start the tx spammer → forge-deploy
# token + BLS verifier + the staking cluster → setBlsVerifier → setConsensusKeys
# v0..v4 → setDposActivationBlock → clean-halt at activation → --dpos cold-restart
# of v0..v5 + full-node → converge past the anchor.
#
# Caller contract (set BEFORE calling):
#   - COMPOSE_FILE=docker-compose.production-path.yml (exported, before sourcing)
#   - SOLIDITY_CONTRACTS_DIR, MANIFEST (deploy-output path), forge_l2() wrapper
#   - PP_ROT_LABEL — FAIL-message label (e.g. "smoke-vrf-rotation")
# Exports (read by the caller afterwards): DEPLOYER_KEY, DEPLOYER_ADDR, TOKEN,
#   VERIFIER, STAKING_RT, CHAIN_CONFIG_RT, GOV_ADDR, LIVENESS_RT, ACT, ANCHOR,
#   EPOCH_LEN; defines epoch_first_block(); starts the spammer (PP_SPAMMER_PID).
# No behavior change vs the former inline block — a pure extraction so both cases
# own ONE forge/governance bring-up.
pp_bring_up_rotation() {
    local L="${PP_ROT_LABEL:?pp_bring_up_rotation: set PP_ROT_LABEL}"
    local MNEMONIC SPAMMER_KEY SPAMMER_ADDR ck bls_pub bls_pop peer addr HEAD PRE k want got pair v i

    echo "== phase A: bare sequencer chain =="
    docker compose up --build -d
    pp_wait_converge 240 >/dev/null || { echo "FAIL ($L): bare chain did not converge"; docker compose logs --tail=120; exit 1; }
    echo "  converged plain chain"

    DEPLOYER_KEY="$(pp_owner_key 0)"
    DEPLOYER_ADDR="$(pp_owner_addr 0)"

    MNEMONIC="${FLUENT_DPOS_MNEMONIC:-test test test test test test test test test test test junk}"
    SPAMMER_KEY="$(cast wallet private-key --mnemonic "$MNEMONIC" --mnemonic-index 6)"
    SPAMMER_ADDR="$(cast wallet address --mnemonic "$MNEMONIC" --mnemonic-index 6)"
    cast send "$SPAMMER_ADDR" --value 1000000000000000 \
        --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" >/dev/null \
        || { echo "FAIL ($L): fund spammer account"; exit 1; }
    pp_spammer_start "$SPAMMER_KEY" "$DEPLOYER_ADDR"
    echo "  tx spammer started (pid $PP_SPAMMER_PID, from $SPAMMER_ADDR)"

    echo "== runtime deploy: token + BLS verifier =="
    TOKEN=$(forge_l2 forge create --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --json \
        "contracts/staking/mocks/MockBlendToken.sol:MockBlendToken" | jq -r '.deployedTo')
    [[ "$TOKEN" == 0x* ]] || { echo "FAIL ($L): MockBlendToken deploy"; exit 1; }
    VERIFIER=$(forge_l2 forge create --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --json \
        "contracts/libraries/BLS12381Verifier.sol:BLS12381Verifier" | jq -r '.deployedTo')
    [[ "$VERIFIER" == 0x* ]] || { echo "FAIL ($L): BLS12381Verifier deploy"; exit 1; }
    echo "  token=$TOKEN verifier=$VERIFIER"

    pp_token_transfer "$TOKEN" "$(pp_owner_addr 5)" "10000000000000000000"

    echo "== runtime deploy: staking cluster (DeployStaking) =="
    NETWORK=local-dpos-smoke/l2 DEPLOYER="$DEPLOYER_ADDR" INITIAL_OWNER="$DEPLOYER_ADDR" \
      STAKING_TOKEN="$TOKEN" OUTPUT_PATH="$MANIFEST" \
      forge_l2 forge script scripts/deploy/DeployStaking.s.sol:DeployStaking \
        --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --skip-simulation \
      || { echo "FAIL ($L): DeployStaking (EIP-170? see prereqs)"; exit 1; }

    STAKING_RT=$(jq -r '.staking' "$MANIFEST")
    CHAIN_CONFIG_RT=$(jq -r '.chain_config' "$MANIFEST")
    GOV_ADDR=$(jq -r '.governance' "$MANIFEST")
    LIVENESS_RT=$(jq -r '.liveness_slashing' "$MANIFEST")
    for v in STAKING_RT CHAIN_CONFIG_RT GOV_ADDR LIVENESS_RT; do
        [[ "${!v}" == 0x* ]] || { echo "FAIL ($L): manifest missing $v"; cat "$MANIFEST"; exit 1; }
    done
    echo "  staking=$STAKING_RT chainConfig=$CHAIN_CONFIG_RT gov=$GOV_ADDR liveness=$LIVENESS_RT"

    echo "== governance: setBlsVerifier (MUST precede setConsensusKeys) =="
    pp_gov_action "$CHAIN_CONFIG_RT" \
        "$(cast calldata 'setBlsVerifier(address)' "$VERIFIER")" \
        "setBlsVerifier" || { echo "FAIL ($L): gov setBlsVerifier"; exit 1; }

    echo "== setConsensusKeys for committee v0..v$((PP_COMMITTEE_SIZE - 1)) =="
    for i in $(seq 0 $((PP_COMMITTEE_SIZE - 1))); do
        ck=$(pp_consensus_keys "$i")
        bls_pub=$(jq -r '.blsPubkeyUncompressed' <<<"$ck")
        bls_pop=$(jq -r '.blsPoPUncompressed' <<<"$ck")
        peer=$(jq -r '.peerPubkey' <<<"$ck")
        addr=$(jq -r '.validatorAddress' <<<"$ck")
        cast send "$STAKING_RT" "setConsensusKeys(address,bytes,bytes,bytes32)" \
            "$addr" "$bls_pub" "$bls_pop" "$peer" \
            --rpc-url "$RPC" --private-key "$(pp_owner_key "$i")" >/dev/null \
            || { echo "FAIL ($L): setConsensusKeys v$i"; exit 1; }
    done
    echo "  consensus keys set for $PP_COMMITTEE_SIZE validators"

    HEAD=$(printf '%d' "$(check_external 8545 | cut -d'|' -f1)")
    ACT=$(( ((HEAD / 64) + 2) * 64 ))
    echo "== governance: setDposActivationBlock=$ACT (head=$HEAD) =="
    pp_gov_action "$CHAIN_CONFIG_RT" \
        "$(cast calldata 'setDposActivationBlock(uint64)' "$ACT")" \
        "setDposActivationBlock" || { echo "FAIL ($L): gov setDposActivationBlock"; exit 1; }

    echo "== assert pre-written staking-reader.json matches the deploy manifest =="
    PRE=$(docker compose exec -T validator-0 cat /runtime/staking-reader.json)
    for pair in "staking_address:$STAKING_RT" \
                "chain_config_address:$CHAIN_CONFIG_RT" \
                "liveness_slashing_address:$LIVENESS_RT"; do
        k=${pair%%:*} want=$(tr 'A-F' 'a-f' <<<"${pair#*:}")
        got=$(jq -r ".$k" <<<"$PRE" | tr 'A-F' 'a-f')
        [[ "$got" == "$want" ]] || { echo "FAIL ($L): pre-written $k=$got != deployed $want (deployer nonce drift — update --staking-reader-create-nonces)"; exit 1; }
    done
    echo "  pre-written config matches manifest"

    echo "== wait for sequencer (validator-0) to clean-halt at activation block $ACT =="
    wait_finalized_ge "$ACT" 400 || {
        echo "FAIL ($L): sequencer did not reach activation block $ACT (head=$(finalized_dec))"
        docker compose logs validator-0 --tail=80; exit 1
    }
    pp_wait_converge 180 >/dev/null \
        || { echo "FAIL ($L): followers did not align at the activation block"; docker compose logs --tail=120; exit 1; }
    echo "  all nodes aligned at $ACT; proceeding to --dpos cold-restart"

    echo "== cold-restart: all validators into unified --dpos (+ full-node into --cert-follow) =="
    ANCHOR=$(check_external 8545 | cut -d'|' -f1)
    export COMPOSE_FILE="docker-compose.production-path.yml:docker-compose.production-path.dpos.yml"
    docker compose up -d --force-recreate "${PP_VALS[@]}" full-node \
        || { echo "FAIL ($L): cold-restart into --dpos (a validator exited)"; docker compose logs validator-0 --tail=80; exit 1; }
    pp_wait_converge 180 "$ANCHOR" >/dev/null \
        || { echo "FAIL ($L): DPoS chain did not converge past anchor $ANCHOR"; docker compose logs --tail=200; exit 1; }
    echo "  DPoS chain live past anchor $ANCHOR"

    EPOCH_LEN=$(printf '%d' "$(pp_chainconfig_call 'getEpochBlockInterval()(uint32)')")
    (( EPOCH_LEN > 0 )) || { echo "FAIL ($L): getEpochBlockInterval()=0"; exit 1; }
}

# Decimal block height of the FIRST block of relative epoch $1 (ACT + $1*EPOCH_LEN).
# Defined here (not inside pp_bring_up_rotation) so it survives the helper's local
# scope; reads the globals ACT + EPOCH_LEN that the bring-up set.
epoch_first_block() { echo $(( ACT + $1 * EPOCH_LEN )); }
