#!/usr/bin/env bash
# DESTRUCTIVE fault-injection assertion bodies for the DEFAULT-stack smoke cases,
# extracted so the combined `case-fault.sh` can chain them on ONE bring-up while
# each remains a standalone `case-<name>.sh` (same function, its own bring-up) for
# isolated debugging.
#
# Unlike asserts.sh (read-only), every function here MUTATES the stack — it
# restarts / SIGKILLs / CPU-throttles / full-stops nodes — BUT each one RESTORES
# the stack to a healthy, realigned state before it returns (its terminal
# assertion is the recovery check). So they can be chained least-invasive-first
# with fail-fast: a case only hands off to the next if its own recovery passed.
# `case-liveness` is deliberately NOT here — its kill/rejoin cycles can push the
# miss-count to a JAIL, which permanently shrinks the committee (unrecoverable);
# it must stay an isolated stack.
#
# MUST be sourced AFTER lib.sh (uses bring_up's globals + helpers: VALS,
# finalized_dec, baseline_height, check_external/check_node, peer_count,
# staking_call, shutdown_flushed, assert_beacon_window, wait_nodes_have,
# wait_finalized_ge, mixhash_*, ...). The caller owns bring_up_dpos +
# `trap tear_down EXIT`. On failure each function `exit 1`s (fail-fast, terminates
# the whole run + the trap tears the stack down); on success it `return`s so a
# combined runner continues to the next, more-invasive case.
# shellcheck shell=bash

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
# Run FIRST in the chain: its K-lag invariant wants a pristine steady state
# (before any node has been restarted/stopped by a later case).
assert_deferred() {
    local K RPC_URL base saw_exact latest final lag cons cons_fin cons_res eth_final delta
    local N artifact wire committed_result derived_hash victim cid pre_throttle during deadline vfin
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
            return 0
        }
        sleep 3
    done
    echo "FAIL (smoke-deferred): $victim did not rejoin after unthrottle (victim=$(check_node docker compose exec -T "$victim"), v0=$(check_external 8545))"
    exit 1
}

# smoke-peers: both peer planes connect, and a restarted node re-establishes both.
#   - commonware consensus plane: discovery connects each validator to its
#     committee peers (observed via the devnet metrics endpoint, Metrics::encode
#     over --dpos.metrics-port on host :19100). Tracked peer set == on-chain
#     committee (Addendum B), so a healthy node converges to committee_size-1.
#   - reth devp2p plane (EL transport for block sync/catch-up): each spoke pins
#     validator-0's enode (--trusted-peers), so net_peerCount > 0. Regression
#     guard: under --dpos the override must keep reth peering wired (else rejoin
#     EL-sync breaks — see dpos_rejoin_el_sync_devp2p).
assert_peers() {
    local METRICS CSIZE EXPECT deadline c rp PRE
    METRICS="http://localhost:19100/"

    committee_size() {
        local cur comm
        cur=$(staking_call "currentEpoch()(uint256)")
        comm=$(staking_call "getEpochCommittee(uint64)(address[])" "$cur")
        # cast prints address[] as [0x.., 0x..]; count the addresses.
        grep -oE '0x[0-9a-fA-F]{40}' <<<"$comm" | wc -l | tr -d ' '
    }

    # Connected committee peers as seen by validator-0. The commonware p2p
    # `tracker_*` gauges are NOT in `Metrics::encode()` output; the observable
    # per-peer series is keyed by the peer's consensus pubkey. Count the distinct
    # peers validator-0 exchanges broadcasts with. `{ grep || true; }` keeps the
    # pipeline alive under `set -o pipefail` when there is no match yet (early boot).
    connected_count() {
        curl -s "$METRICS" \
            | { grep -oE 'outer_engine_buffered_peer_total\{sequencer="[0-9a-f]+"\}' || true; } \
            | sort -u | wc -l | tr -d ' '
    }

    CSIZE=$(committee_size)
    EXPECT=$(( CSIZE - 1 ))
    echo "smoke-peers: committee_size=$CSIZE → expect connected=$EXPECT on validator-0"

    # Poll for discovery to settle to committee_size-1 connected peers.
    deadline=$(( $(date +%s) + 60 ))
    while (( $(date +%s) < deadline )); do
        [[ "$(connected_count)" == "$EXPECT" ]] && break
        sleep 2
    done
    c=$(connected_count)
    [[ "$c" == "$EXPECT" ]] || { echo "FAIL (smoke-peers): connected=$c != $EXPECT"; curl -s "$METRICS" | grep -E 'buffered_peer_total|peer_performance' || true; exit 1; }
    echo "  initial: connected=$c (== committee_size-1)"

    # reth devp2p plane: a spoke must hold a live reth peer (its trusted-peers enode
    # to validator-0). Poll briefly — devp2p handshake can lag commonware discovery.
    deadline=$(( $(date +%s) + 60 ))
    while (( $(date +%s) < deadline )); do (( $(peer_count validator-1) > 0 )) && break; sleep 2; done
    rp=$(peer_count validator-1)
    (( rp > 0 )) || { echo "FAIL (smoke-peers): validator-1 reth devp2p net_peerCount=$rp (want > 0 — --dpos peering not wired)"; exit 1; }
    echo "  initial: validator-1 reth devp2p peers=$rp (> 0)"

    # Reconnect: restart validator-1; assert validator-0 re-establishes the commonware
    # peer set, validator-1 re-establishes its reth devp2p peer, AND the chain
    # finalizes past the restart point (the restarted node rejoins and contributes).
    PRE=$(baseline_height)
    docker compose restart validator-1
    deadline=$(( $(date +%s) + 120 ))
    while (( $(date +%s) < deadline )); do
        [[ "$(connected_count)" == "$EXPECT" ]] && (( $(peer_count validator-1) > 0 )) && (( $(finalized_dec) > PRE )) && {
            echo "OK (smoke-peers): commonware connected=$EXPECT + validator-1 reth peers>0 + chain advanced past $PRE after restart"; return 0; }
        sleep 3
    done
    echo "FAIL (smoke-peers): after validator-1 restart connected=$(connected_count) (want $EXPECT), reth peers=$(peer_count validator-1) (want >0), finalized=$(finalized_dec) (want > $PRE)"
    exit 1
}

# smoke-vrf-fault: the threshold beacon under FAULT + RESTART + deep catch-up.
#   A1 — f=1 validator DOWN: the beacon SURVIVES (the n−f seed quorum of survivors
#        still recovers the threshold seed); prev_randao stays threshold-active and
#        byte-identical on the survivors while the node is down.
#   B3/B4 — the downed validator RESTARTS, reloads its share, and CATCHES UP: every
#        gap block it missed is re-obtained with the cert-recovered threshold seed
#        (ASSURANCE), NOT the order.digest() fallback — its mixHash on the gap blocks
#        is byte-identical to a validator that never went down (a fork / fallback
#        would diverge). Folds item I (keyless restart) + the executor catch-up
#        seed-availability invariant.
assert_vrf_fault() {
    local DOWN SURVIVORS NODES down_at gap_target a_lo a_hi catchup_deadline dh n d v miss
    DOWN=validator-3
    SURVIVORS=(validator-0 validator-1 validator-2 full-node)

    (( $(finalized_dec) > 0 )) || { echo "FAIL (smoke-vrf-fault): no finalized block"; exit 1; }

    # A1: take ONE validator down (f=1). With 4 validators (f=1) the seed quorum is
    # n−f=3, so the 3 survivors still recover the threshold seed and the beacon stays
    # live. NODES = survivors (the downed node serves no RPC).
    echo "smoke-vrf-fault: stopping $DOWN (f=1 fault) — the beacon must stay live on the survivors"
    docker compose stop "$DOWN" >/dev/null
    down_at=$(finalized_dec)
    gap_target=$(( down_at + 10 ))
    # NODES is read by lib.sh's assert_beacon_window + wait_nodes_have via bash
    # dynamic scope — shellcheck can't trace that cross-function use.
    # shellcheck disable=SC2034
    NODES=("${SURVIVORS[@]}")
    wait_finalized_ge "$gap_target" 150 >/dev/null || {
        echo "FAIL (smoke-vrf-fault): A1 — chain stalled with $DOWN down (survivors below n−f quorum?)"
        docker compose logs --tail=120 validator-0
        exit 1
    }
    a_lo=$(( down_at + 2 ))
    a_hi=$(( gap_target - 1 ))
    wait_nodes_have "$a_hi" 90 || { echo "FAIL (smoke-vrf-fault): A1 — survivors did not all reach block $a_hi"; exit 1; }
    assert_beacon_window "$a_lo" "$a_hi" "f=1-down"
    echo "smoke-vrf-fault: A1 — beacon survived the f=1 fault, active + byte-identical on survivors over [$a_lo..$a_hi]"

    # B3/B4: restart $DOWN. It reloads its beacon share, rejoins, and catches up. Every
    # gap block must come back with the SAME threshold prev_randao the survivors have —
    # i.e. the executor recovered the seed from the cert (assurance), not the fallback.
    echo "smoke-vrf-fault: restarting $DOWN — it must catch up the gap with verified prev_randao"
    docker compose start "$DOWN" >/dev/null
    catchup_deadline=$(( SECONDS + 150 ))
    while :; do
        dh=$(mixhash_in "$DOWN" "$a_hi")
        [[ "$dh" != "null" && -n "$dh" ]] && break
        (( SECONDS < catchup_deadline )) || {
            echo "FAIL (smoke-vrf-fault): B4 — $DOWN did not catch up to block $a_hi within the deadline"
            docker compose logs --tail=120 "$DOWN"
            exit 1
        }
        sleep 2
    done
    miss=()
    for ((n = a_lo; n <= a_hi; n++)); do
        d=$(mixhash_in "$DOWN" "$n")
        v=$(mixhash_at "$n")
        if [[ "$d" == "null" || -z "$d" ]]; then miss+=("$n=missing-on-$DOWN"); continue; fi
        [[ "$d" == "$v" ]] || miss+=("$n: $DOWN=$d != validator-0=$v")
    done
    if (( ${#miss[@]} > 0 )); then
        echo "FAIL (smoke-vrf-fault): B4 — restarted $DOWN derived divergent prev_randao on gap blocks (fell to fallback / forked instead of recovering the cert seed):"
        printf '  %s\n' "${miss[@]}"
        exit 1
    fi
    echo "smoke-vrf-fault: B3/B4 — $DOWN restarted, caught up, and re-obtained the gap [$a_lo..$a_hi] with the byte-identical threshold prev_randao (assurance, not fallback)"

    echo "OK (smoke-vrf-fault): beacon survived the f=1 fault; the downed validator restarted, reloaded its share, and caught up the gap with verified threshold prev_randao"
}

# smoke-crash-survivor (Problem A): a validator is CRASHED ungracefully
# (SIGKILL, no persistence flush) mid-operation, the chain advances while it is
# down (building an EL gap), then it is restarted. Assert it recovers its EL and
# realigns to the honest finalized head instead of wedging on a missing block.
# Contrast with smoke-liveness, which uses a graceful `stop` (flushed shutdown).
assert_crash_survivor() {
    local PRE VIC_CID GAP_TARGET HEAD_WHILE_DOWN deadline tick PC v0 v3 pc v3_final
    PRE=$(baseline_height)
    # Use the raw container id + raw docker kill/start so the crash+restart is surgical
    # (no `docker compose start` dependency re-run of genesis-init, which races on the
    # ungraceful path and made the restart flaky).
    VIC_CID=$(docker compose ps -q validator-3)
    [[ -n "$VIC_CID" ]] || { echo "FAIL: could not resolve validator-3 container id"; exit 1; }
    echo "smoke-crash-survivor: SIGKILL validator-3 ($VIC_CID) at finalized=$PRE (ungraceful, no flush)"
    docker kill "$VIC_CID"   # raw SIGKILL — simulates a crash, bypasses compose deps

    # Chain keeps finalizing (quorum 3/4); let it advance to build an EL gap the
    # crashed node will have to backfill on restart.
    GAP_TARGET=$(( PRE + 12 ))
    wait_finalized_ge "$GAP_TARGET" 90 || true   # soft target; the hard assert is PRE+3 below
    HEAD_WHILE_DOWN=$(finalized_dec)
    (( HEAD_WHILE_DOWN >= PRE + 3 )) || { echo "FAIL: chain stalled with 1/4 crashed (finalized=$HEAD_WHILE_DOWN, pre=$PRE)"; docker compose logs --tail=120; exit 1; }
    echo "  chain advanced to $HEAD_WHILE_DOWN with validator-3 crashed (gap ~$((HEAD_WHILE_DOWN - PRE)) blocks)"

    # Restart the crashed node; assert it recovers + realigns (no permanent wedge).
    echo "  restarting crashed validator-3 ..."
    docker start "$VIC_CID"
    # Decisive diagnostic: long deadline (10 min) + periodic peer probe to learn whether
    # the post-ungraceful-crash connected_peers=0 is PERMANENT or just slow to re-peer.
    deadline=$(( $(date +%s) + 600 ))
    tick=0
    PC='{"jsonrpc":"2.0","method":"net_peerCount","params":[],"id":1}'
    while (( $(date +%s) < deadline )); do
        v0=$(check_external 8545); v3=$(check_node docker compose exec -T validator-3)
        if [[ "$v0" == "$v3" && "${v0%%|*}" != "null" ]]; then
            echo "OK (smoke-crash-survivor): validator-3 recovered from crash and realigned at $v3"
            return 0
        fi
        if (( tick % 10 == 0 )); then
            pc=$(docker compose exec -T validator-3 curl -s -X POST -H 'content-type: application/json' --data "$PC" http://localhost:8545 2>/dev/null | grep -oE '0x[0-9a-f]+' | tail -1) || true
            echo "  t+$((tick*3))s: v3 peers=${pc:-?} v3=$v3 v0=$v0"
        fi
        tick=$((tick+1))
        sleep 3
    done
    v3_final=$(check_node docker compose exec -T validator-3)
    echo "FAIL (smoke-crash-survivor): validator-3 did not realign after crash+restart (v0=$(check_external 8545) v3=$v3_final)"
    echo "  (Problem A: crash survivor wedged on a missing EL block — needs 2b FCU-driven recovery)"
    docker compose logs --tail=80 validator-3
    exit 1
}

# smoke-full-restart: stop ALL 4 validators (verify each persisted, exit 0), restart them, and
# assert the network reconverges from the persisted finalized head — i.e. DPoS
# cold-restart from disk works for the whole set, not just the migration anchor.
# Run LAST in the chain: it is the most invasive (stops the entire validator set).
assert_full_restart() {
    local pre v deadline v0 v1 v2 v3 fn head
    pre=$(baseline_height)
    echo "smoke-full-restart: stopping all 4 validators at finalized=$pre"
    docker compose stop --timeout 40 "${VALS[@]}"
    for v in "${VALS[@]}"; do
        shutdown_flushed "$v" || { echo "FAIL (smoke-full-restart): $v did not exit cleanly (code 0) on shutdown"; exit 1; }
    done
    echo "  all persisted (exit 0); restarting"
    docker compose start "${VALS[@]}"

    # reconverge: all 5 align finalized at >= pre (resume from persisted head).
    deadline=$(( $(date +%s) + 120 ))
    while (( $(date +%s) < deadline )); do
        v0=$(check_external 8545); v1=$(check_node docker compose exec -T validator-1)
        v2=$(check_node docker compose exec -T validator-2); v3=$(check_node docker compose exec -T validator-3)
        fn=$(check_external 18545); head="${v0%%|*}"
        if [[ "$head" != "null" && "$head" != "0x0" \
              && "$v0" == "$v1" && "$v1" == "$v2" && "$v2" == "$v3" && "$v3" == "$fn" ]] \
           && (( $(printf '%d' "$head") >= pre )); then
            echo "OK (smoke-full-restart): all 5 reconverged at $v0 (>= pre=$pre) after full stop/start"
            return 0
        fi
        sleep 2
    done
    echo "FAIL (smoke-full-restart): network did not reconverge after full restart"
    docker compose logs --tail=200
    exit 1
}
