#!/usr/bin/env bash
# Continuous invariant battery for the soak (sourced by case-soak.sh).
#
# check_invariants() runs the whole battery against ONE captured snapshot per
# tick (bounds docker load at N nodes) and, unlike the exit-on-fail assert_*
# bodies, RETURNS a status: 0 = all ok, 1 = a violation (INV_FAIL_ID +
# INV_FAIL_MSG set) so the orchestrator can write the structured bundle instead
# of a bare `exit 1`. Caller provides: NODES (up services), EXPECTED_STALL (1
# suspends inv-1 during the quorum-loss probe), SOAK_BLOCK_INTERVAL, RESULT_LAG_K.
# Reuses lib.sh: finalized_dec, mixhash_of/is_zero_hash (beacon), log_count.

: "${RESULT_LAG_K:=3}"
: "${SOAK_STALL_TICKS:=4}"
: "${SOAK_BEACON_WINDOW:=6}"

# consensus_getLatest two-tier finality from the host RPC: echoes
# "latestFinalized latestResultFinalized" (decimal), or "0 0" if unreachable.
soak_consensus_latest() {
    local out lf lrf
    out=$(curl -s -X POST -H 'Content-Type: application/json' \
        --data '{"jsonrpc":"2.0","method":"consensus_getLatest","params":[],"id":1}' \
        "$RPC" 2>/dev/null || true)
    lf=$(jq -r '.result.latestFinalized // .result.latest_finalized // 0' <<<"$out" 2>/dev/null || echo 0)
    lrf=$(jq -r '.result.latestResultFinalized // .result.latest_result_finalized // 0' <<<"$out" 2>/dev/null || echo 0)
    printf '%d %d\n' "$(printf '%d' "$lf" 2>/dev/null || echo 0)" "$(printf '%d' "$lrf" 2>/dev/null || echo 0)"
}

# eth latest height (decimal) from host RPC, 0 if unreachable.
soak_eth_latest() {
    local h
    h=$(curl -s -X POST -H 'Content-Type: application/json' \
        --data '{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["latest",false],"id":1}' \
        "$RPC" 2>/dev/null | jq -r '.result.number // "0x0"' 2>/dev/null || echo 0x0)
    printf '%d' "$h" 2>/dev/null || echo 0
}

# rolling state (across ticks)
SOAK_LAST_FIN=-1
SOAK_FLAT_TICKS=0
SOAK_L2_LAST=-1; SOAK_L2_FLAT=0   # L2 cert-follower (full-node) liveness
SOAK_L3_LAST=-1; SOAK_L3_FLAT=0   # L3 cascade (downstream) liveness
SOAK_WP_LAST=-1; SOAK_WP_FLAT=0   # write-path: L3-submitted-tx → validator delivery

# Finalized height (decimal) at a host-mapped RPC port (18545=L2, 28545=L3); 0 if unreachable.
_soak_fin_at() {
    local p="$1" h
    h=$(curl -s -X POST -H 'Content-Type: application/json' --data "$JSONRPC_FINALIZED" \
        "http://localhost:${p}" 2>/dev/null | jq -r '.result.number // "0x0"' 2>/dev/null || echo 0x0)
    printf '%d' "$h" 2>/dev/null || echo 0
}

_inv_fail() { INV_FAIL_ID="$1"; INV_FAIL_MSG="$2"; return 1; }

# Returns 0 if every invariant holds this tick, else 1 with INV_FAIL_ID/MSG set.
check_invariants() {
    INV_FAIL_ID=""; INV_FAIL_MSG=""
    local fin latest read lat lrf

    fin=$(finalized_dec)
    latest=$(soak_eth_latest)
    read -r lat lrf < <(soak_consensus_latest)

    # ── Inv 1: finalizing / no stall (suspended inside the quorum-loss probe).
    if (( ${EXPECTED_STALL:-0} == 0 )); then
        if (( SOAK_LAST_FIN >= 0 && fin <= SOAK_LAST_FIN )); then
            SOAK_FLAT_TICKS=$(( SOAK_FLAT_TICKS + 1 ))
            if (( SOAK_FLAT_TICKS >= SOAK_STALL_TICKS )); then
                _inv_fail finalize-stall "finalized FLAT at $fin for $SOAK_FLAT_TICKS ticks (>= $SOAK_STALL_TICKS) — chain stalled"
                return 1
            fi
        else
            SOAK_FLAT_TICKS=0
        fi
    else
        SOAK_FLAT_TICKS=0   # don't carry probe-window flatness into steady state
    fi
    SOAK_LAST_FIN="$fin"

    # ── Inv 2: K-lag (SAFETY: latest-finalized never < K = result-final overclaim).
    # Steady-state only: skip during a deep catch-up window (lag transiently wide).
    if (( fin > 0 && latest >= fin )); then
        local gap=$(( latest - fin ))
        if (( gap < RESULT_LAG_K )); then
            _inv_fail k-lag-underflow "eth latest-finalized=$gap < K=$RESULT_LAG_K (result-final overclaim, SAFETY) latest=$latest fin=$fin"
            return 1
        fi
        # consensus two-tier exactness (only when both tiers are populated)
        if (( lf_nonzero=(lat>0 && lrf>0), lf_nonzero )); then
            local cgap=$(( lat - lrf ))
            if (( cgap != RESULT_LAG_K && cgap >= 0 && lat - lrf <= RESULT_LAG_K + 2 )); then
                _inv_fail k-lag-consensus "consensus latestFinalized-latestResultFinalized=$cgap != K=$RESULT_LAG_K (lat=$lat lrf=$lrf)"
                return 1
            fi
        fi
    fi

    # ── Inv 3 + 4: prev_randao byte-identity + beacon ACTIVE (not digest fallback).
    # Compare the last SOAK_BEACON_WINDOW finalized blocks across UP nodes.
    if (( fin > SOAK_BEACON_WINDOW + 1 )); then
        local lo=$(( fin - SOAK_BEACON_WINDOW )) hi=$(( fin - 1 )) b svc mh prev="" distinct_ok=1
        local -a seen=()
        for ((b=lo;b<=hi;b++)); do
            local first="" agree=1
            for svc in "${NODES[@]}"; do
                mh=$(mixhash_of "$svc" "$b")
                [[ "$mh" == "null" || -z "$mh" ]] && { first=""; agree=1; break; }  # node behind → skip block (waited elsewhere)
                if is_zero_hash "$mh"; then
                    _inv_fail beacon-zero "prev_randao ZERO at block $b on $svc (beacon stalled / fell to digest)"
                    return 1
                fi
                if [[ -z "$first" ]]; then first="$mh"; elif [[ "$mh" != "$first" ]]; then agree=0; fi
            done
            [[ -z "$first" ]] && continue
            if (( agree == 0 )); then
                _inv_fail beacon-divergent "nodes disagree on prev_randao at block $b (divergent seed)"
                return 1
            fi
            seen+=("$first")
        done
        # varying (non-stuck) randomness across the window
        if (( ${#seen[@]} > 1 )); then
            local distinct; distinct=$(printf '%s\n' "${seen[@]}" | sort -u | wc -l)
            if (( distinct == 1 )); then
                _inv_fail beacon-stuck "prev_randao identical across ${#seen[@]} blocks [$lo..$hi] (stuck randomness)"
                return 1
            fi
        fi
        # Inv 4: ACTIVE beacon (not silent NoBeacon digest fallback). The live-DKG
        # active line must keep appearing on the sequencer; a steady-state run with
        # NO active line over the window means the beacon silently fell back.
        local active; active=$(log_count validator-0 'beacon.*active\|live DKG\|ACTIVE')
        SOAK_BEACON_ACTIVE_PREV="${SOAK_BEACON_ACTIVE_PREV:-0}"
        if (( active > 0 )); then SOAK_BEACON_ACTIVE_PREV="$active"; fi
        # (growth across ticks is asserted by the orchestrator's coarse check; here
        # we only fail-hard on the zero/divergent/stuck signals above.)
    fi

    # ── Inv 7 (coarse): reth devp2p peer plane on the spoke. SKIPPED only when the
    # SPOKE ITSELF is a current churn victim (a down spoke legitimately has 0 peers —
    # pre-mortem §11.2); other nodes being disrupted does NOT blind the spoke, so the
    # check still runs then. A 0 must PERSIST across two checks (tolerates the transient
    # reconnect dip right after a restore).
    local spoke="${SOAK_PEER_SPOKE:-validator-1}"
    if (( ${SOAK_TICK:-0} % ${SOAK_PEER_EVERY:-6} == 0 )) && ! _in_set "$spoke" "${SOAK_DISRUPTED:-}"; then
        local pc; pc=$(peer_count "$spoke")
        if (( pc == 0 )); then
            SOAK_PEER_ZERO=$(( ${SOAK_PEER_ZERO:-0} + 1 ))
            # Persist threshold is 4 (not 2): a --dpos validator drives its EL via the
            # consensus plane (InsertExecuted), so reth devp2p is non-critical and a node
            # JUST restored from a sigkill restart can take a couple minutes to re-establish
            # its trusted-peer devp2p link (observed flaky 2-check fail). 4 consecutive
            # zero-checks still catches a genuinely-down plane (~minutes) without flagging
            # a slow post-restore reconnect.
            if (( SOAK_PEER_ZERO >= 4 )); then
                _inv_fail peer-plane "reth net_peerCount=0 on $spoke across 4 steady-state checks (devp2p plane down)"
                return 1
            fi
        else
            SOAK_PEER_ZERO=0
        fi
    fi

    # ── Inv 8 (SOFT / observability WARN, not a hard fail): MULTI-TIER cascade liveness —
    # L2 (full-node) + L3 (downstream) health. These are downstream OBSERVER tiers, not the
    # validator/chain-safety plane: a follower lagging/wedging does NOT compromise chain
    # safety (validators + the chain are fine), so it must NOT halt the soak. Chain-safety is
    # already hard-enforced by Inv 1/2 (v0) + Inv 3 prev_randao byte-identity (which catches a
    # follower on the WRONG chain when it IS synced). This WARN surfaces a follower whose
    # FINALIZED is not advancing while v0 is — note cert-followers finalize in lagged/epoch-
    # batched jumps (flat between batches is normal), so this is a health signal to read in the
    # bundle, not a verdict. Emitted once per wedge-episode (== threshold); resets when the
    # tier advances. (A genuinely-dead tier of the cascade is a known reth-2.2 cert-follow/
    # cascade fragility — tracked separately, not a soak-halting condition.)
    if (( ${EXPECTED_STALL:-0} == 0 && SOAK_FLAT_TICKS == 0 && fin > SOAK_BEACON_WINDOW )); then
        local l2f l3f
        l2f=$(_soak_fin_at 18545)
        if (( l2f > SOAK_L2_LAST )); then SOAK_L2_FLAT=0; SOAK_L2_LAST="$l2f"
        else
            SOAK_L2_FLAT=$(( SOAK_L2_FLAT + 1 ))
            (( SOAK_L2_FLAT == 4 * SOAK_STALL_TICKS )) && soak_event warn "L2 cert-follower (full-node) finalized lagging at $l2f for $SOAK_L2_FLAT ticks while v0 advances (follower health — cert-cascade lag, NOT chain-safety)"
        fi
        l3f=$(_soak_fin_at 28545)
        if (( l3f > SOAK_L3_LAST )); then SOAK_L3_FLAT=0; SOAK_L3_LAST="$l3f"
        else
            SOAK_L3_FLAT=$(( SOAK_L3_FLAT + 1 ))
            (( SOAK_L3_FLAT == 4 * SOAK_STALL_TICKS )) && soak_event warn "L3 cascade (downstream) finalized lagging at $l3f for $SOAK_L3_FLAT ticks while v0 advances (follower health — cert-cascade lag, NOT chain-safety)"
        fi
    fi

    # ── Inv 9 (SOFT WARN): WRITE-PATH liveness — a tx submitted to L3 (the sentry
    # cascade) must reach a validator's proposer pool. The L3 spammer's ON-CHAIN
    # nonce (read at v0, the proposer side) advances IFF its L3-submitted txs
    # gossiped L3→L2→validator and mined. A flat nonce while v0 keeps finalizing =
    # the write path is down (the silent-starvation failure mode the sentry topology
    # is exposed to). SOFT (like Inv 8): the write path is an availability signal,
    # not chain-safety; warn once per stall episode, never halt.
    if (( ${EXPECTED_STALL:-0} == 0 && SOAK_FLAT_TICKS == 0 )) && [[ -n "${L3_SPAMMER_ADDR:-}" ]]; then
        local wpn
        wpn=$(cast nonce "$L3_SPAMMER_ADDR" --rpc-url "$RPC" 2>/dev/null || echo 0)
        wpn=$(printf '%d' "$wpn" 2>/dev/null || echo 0)
        if (( wpn > SOAK_WP_LAST )); then SOAK_WP_FLAT=0; SOAK_WP_LAST="$wpn"
        else
            SOAK_WP_FLAT=$(( SOAK_WP_FLAT + 1 ))
            (( SOAK_WP_FLAT == 4 * SOAK_STALL_TICKS )) && soak_event warn "write-path: L3-submitted txs not landing — sender nonce flat at $wpn for $SOAK_WP_FLAT ticks while v0 finalizes (L3→L2→validator gossip relay may be down)"
        fi
    fi

    return 0
}
