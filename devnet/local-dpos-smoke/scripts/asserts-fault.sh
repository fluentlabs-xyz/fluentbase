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
#   1. K-lag invariant: eth "finalized" trails "latest" by K in steady state
#      (EAGER derive, bundle-20260716T173148Z: the finalized derive tracks the
#      delivered tip with NO +1 — the old F6 h+1-witness lookahead is gone),
#      transiently K+1 while a derive is in flight; never less than K — less =
#      result-finality overclaim. The consensus namespace agrees
#      (latestFinalized.height − latestResultFinalized == K, transiently K+1;
#      latestResultFinalized == eth finalized).
#   2. result-commitment integrity: the ordering artifact at height N+K
#      carries `result` == the derived EVM block hash at N. Decoded from the
#      consensus_getFinalization wire bytes at the fixed codec offset
#      (parent 32 + height 8 + proposal_view 8 + timestamp 8 + fee_recipient 20
#      + gas_limit 8 = byte 84; OrderBlock field order is part of the wire
#      format, and the offset is derived from that field list at :138).
#   3. EL-slowed validator: CPU-throttling one validator must not stall the
#      chain (verify budget → nullify, BFT f=1 holds); after unthrottle the
#      victim catches back up to the live tip.
# Run FIRST in the chain: its K-lag invariant wants a pristine steady state
# (before any node has been restarted/stopped by a later case).
assert_deferred() {
    local K RPC_URL base saw_exact saw_safe_ahead safe latest final lag cgap cons cons_fin cons_res eth_final delta
    local N artifact wire committed_result derived_hash victim cid pre_throttle during deadline vfin
    local wire_fields wf wire_result_off wire_result_len wire_min_len drift wire_lc derived_lc before
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

    # ── 1. K-lag invariant — EAGER-derive re-baseline (bundle-20260716T173148Z, deliberate
    # product change): the finalized derive no longer waits for the h+1 witness lookahead (old
    # F6), so ordering_finalized — and the safe/finalized tags riding it — tracks the delivered
    # tip with NO +1: the steady eth lag is K, transiently K+1 while a derive is in flight.
    # NOTE: under eager derive lag==K sustained is the healthy CAUGHT-UP state, no longer a
    # spec-dead signal (the soak's spec-head-lag now discriminates via the derive-PATH telemetry
    # instead). 6 samples: lag < K in any sample = overclaim (hard fail, unchanged); accept
    # K..K+2 (in-flight derive + a 1-block FCU straddle) and require safe−finalized == K at
    # least once (liveness half: the derive gap must not drift wide of the eager steady state).
    saw_exact=0
    # safe rides the derive tip (== the delivered marshal tip under eager derive), STRICTLY
    # ahead of where finalized sits (safe − K): require at least one sample with
    # latest−safe < K (pre-split safe == finalized == latest−K would never satisfy it).
    saw_safe_ahead=0
    for _ in 1 2 3 4 5 6; do
        latest=$(block_number_of latest)
        final=$(block_number_of finalized)
        safe=$(block_number_of safe)
        lag=$(( latest - final ))
        (( lag >= K )) || { echo "FAIL (smoke-deferred): finalized overclaims — lag=$lag < K=$K (latest=$latest finalized=$final)"; exit 1; }
        (( lag <= K + 2 )) || { echo "FAIL (smoke-deferred): finality lag drifted — lag=$lag > K+2 (latest=$latest finalized=$final; eager-derive budget is K, +1 in-flight derive, +1 straddle)"; exit 1; }
        # WITNESS ON safe−finalized, NOT on lag: the executor writes safe+finalized in ONE
        # forkchoice with finalized = ordering_tip − K, so safe−finalized == K IS the derive
        # statement. `latest` is the SPECULATIVE head (advanced at notarization, not rolled back
        # when correctly_speculated), so lag = K + (latest−safe) — and the band below calls
        # latest−safe ∈ {0,1,2} healthy. `lag == K` would demand a ZERO speculative lead: the
        # soak's spec-head-lag "speculation is dead" signature. Do not move it back.
        (( safe - final == K )) && saw_exact=1
        # Ancestry: finalized ⊆ safe ⊆ head(latest).
        (( safe >= final )) || { echo "FAIL (smoke-deferred): safe below finalized — safe=$safe < finalized=$final (ancestry finalized ⊆ safe violated)"; exit 1; }
        (( safe <= latest )) || { echo "FAIL (smoke-deferred): safe above latest — safe=$safe > latest=$latest (ancestry safe ⊆ head violated)"; exit 1; }
        # Steady-state tracking: safe ≈ latest under eager derive (the derive tip tracks the
        # delivered tip; the head may speculate ahead) — allow ≤2 for the in-flight derive +
        # the ~1 blk/s straddle.
        (( latest - safe <= 2 )) || { echo "FAIL (smoke-deferred): safe not tracking the derive tip — latest−safe=$(( latest - safe )) > 2 (latest=$latest safe=$safe; eager-derive budget is 0 plus in-flight + straddle)"; exit 1; }
        (( latest - safe < K )) && saw_safe_ahead=1
        sleep 2
    done
    (( saw_exact == 1 )) || { echo "FAIL (smoke-deferred): derive gap never sampled at exactly K=$K — safe−finalized (the derive-pipeline gap, free of the speculative head lead) never reached the eager-derive steady state in 6 samples (derive pipeline lagging)"; exit 1; }
    (( saw_safe_ahead == 1 )) || { echo "FAIL (smoke-deferred): safe never sampled ahead of the finalized tier — latest−safe never < K=$K (the safe/finalized split did not take effect)"; exit 1; }
    echo "  K-lag (eth): latest − finalized within $K..$((K + 2)) and safe − finalized == $K (eager-derive steady state) held across 6 samples"
    echo "  safe-tier: finalized ≤ safe ≤ latest, latest−safe < $K held"

    # Consensus namespace must tell the same story as the eth tags. One snapshot
    # is two RPCs apart from the eth read, so allow ±1 skew on the cross-check.
    # EAGER derive (bundle-20260716T173148Z): latestFinalized (marshal tip) −
    # latestResultFinalized (derive tip − K, derive tip == delivered tip) == K in steady
    # state, K+1 while a derive is in flight — poll for the exact K (an atomic snapshot,
    # so anything outside {K, K+1} is a hard tier disagreement).
    local _cs_ok=0
    for _ in 1 2 3 4 5 6; do
        cons=$(rpc consensus_getLatest "[]")
        cons_fin=$(jq -r '.result.latestFinalized.height' <<<"$cons")
        cons_res=$(jq -r '.result.latestResultFinalized' <<<"$cons")
        [[ "$cons_fin" != "null" && "$cons_res" != "null" ]] || { echo "FAIL (smoke-deferred): consensus_getLatest incomplete: $cons"; exit 1; }
        cgap=$(( cons_fin - cons_res ))
        (( cgap == K || cgap == K + 1 )) || { echo "FAIL (smoke-deferred): consensus tiers disagree — latestFinalized=$cons_fin latestResultFinalized=$cons_res cgap=$cgap (want K=$K, transiently K+1 — eager derive)"; exit 1; }
        if (( cgap == K )); then _cs_ok=1; break; fi
        sleep 1
    done
    (( _cs_ok == 1 )) || { echo "FAIL (smoke-deferred): consensus cgap never sampled at exactly K=$K — the result tier is durably a block behind the eager-derive steady state (derive pipeline lagging)"; exit 1; }
    eth_final=$(block_number_of finalized)
    delta=$(( eth_final - cons_res )); (( delta < 0 )) && delta=$(( -delta ))
    (( delta <= 1 )) || { echo "FAIL (smoke-deferred): eth finalized=$eth_final vs latestResultFinalized=$cons_res (skew > 1)"; exit 1; }
    echo "  K-lag (consensus): latestFinalized=$cons_fin − latestResultFinalized=$cons_res == $K (eager derive), matches eth finalized=$eth_final"

    # ── 2. result-commitment integrity ──────────────────────────────────────────
    # The artifact at N+K commits the derived hash of N in its `result` field.
    N=$cons_res
    artifact=$(rpc consensus_getFinalization "[{\"height\":$(( N + K ))}]")
    wire=$(jq -r '.result.block' <<<"$artifact"); wire=${wire#0x}
    [[ -n "$wire" && "$wire" != "null" ]] || { echo "FAIL (smoke-deferred): no ordering artifact at $((N + K)): $artifact"; exit 1; }
    # The OrderBlock codec's FIXED header, as <field>:<byte-width>, mirroring
    # `crates/dpos/consensus/src/order_block.rs` `OrderBlock::write` field-for-field up to and
    # including `result`. Everything after `result` is variable-length (extra_data, RLP txs, the
    # optional beacon fields), so this prefix is the only part with fixed offsets — which is also
    # why the wire's TOTAL length cannot be pinned, only this prefix's. Add a field here when
    # `write` grows one and the slice + the guard both follow.
    wire_fields=(parent:32 height:8 proposal_view:8 timestamp:8 fee_recipient:20 gas_limit:8 result:32)
    wire_result_off=0; wire_result_len=0; wire_min_len=0 # hex chars: 2 per byte
    for wf in "${wire_fields[@]}"; do
        if [[ ${wf%%:*} == result ]]; then
            wire_result_off=$wire_min_len
            wire_result_len=$(( 2 * ${wf##*:} ))
        fi
        wire_min_len=$(( wire_min_len + 2 * ${wf##*:} ))
    done
    # The whole fixed header must be present before any of it can be sliced. This guard does NOT
    # detect a field inserted BEFORE `result` — that makes the wire LONGER, so it passes while the
    # slice lands on a neighbouring field. The mismatch branch below is the drift detector.
    (( ${#wire} >= wire_min_len )) || { echo "FAIL (smoke-deferred): artifact wire too short (${#wire} hex chars, need >= $wire_min_len) — OrderBlock codec layout changed?"; exit 1; }
    committed_result=${wire:wire_result_off:wire_result_len}
    derived_hash=$(rpc eth_getBlockByNumber "[\"$(printf '0x%x' "$N")\",false]" | jq -r '.result.hash'); derived_hash=${derived_hash#0x}
    if [[ "${committed_result,,}" != "${derived_hash,,}" ]]; then
        # Re-locate the hash: if it is in the wire at ANOTHER offset the codec moved and the chain
        # is fine — report that, not a fabricated safety violation.
        drift=""; wire_lc=${wire,,}; derived_lc=${derived_hash,,}
        if [[ -n "$derived_lc" && "$wire_lc" == *"$derived_lc"* ]]; then
            before=${wire_lc%%"$derived_lc"*}
            drift=" — LAYOUT CHANGED: the derived hash sits at hex offset ${#before}, not $wire_result_off; a field was added to OrderBlock::write ahead of \`result\` and wire_fields did not follow"
        fi
        echo "FAIL (smoke-deferred): result commitment mismatch at h=$((N + K)) — artifact result=$committed_result, derived hash($N)=$derived_hash$drift"; exit 1
    fi
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
    local DOWN SURVIVORS NODES down_at gap_target a_lo a_hi catchup_deadline dh n d v miss EPOCH2_PROBE
    DOWN=validator-3
    SURVIVORS=(validator-0 validator-1 validator-2 full-node)

    # The beacon is THRESHOLD-ACTIVE from EPOCH 2 (deterministic bootstrap). The fault
    # window + the downed-node share reload must exercise a seeded epoch, so wait until
    # finalized is inside epoch >= 2 before stopping the victim.
    EPOCH2_PROBE=$(( DPOS_ACTIVATION_BLOCK + 2 * EPOCH_INTERVAL + 8 ))
    echo "smoke-vrf-fault: beacon active from epoch 2; waiting for finalized >= $EPOCH2_PROBE before the fault"
    wait_finalized_ge "$EPOCH2_PROBE" 300 >/dev/null || {
        echo "FAIL (smoke-vrf-fault): chain did not reach the epoch-2 window ($EPOCH2_PROBE) before the fault"
        docker compose logs --tail=120 validator-0
        exit 1
    }
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

# smoke-vrf-dkg-liveness (DKG-liveness negative edge — NO reshare): a committee
# member taken OFFLINE during its DKG window misses the ceremony, is SHARELESS for
# that epoch, and SITS OUT seed voting while the chain stays live on the remaining
# n−f share-holder quorum. On the genesis stack (4 validators, f=1, the deterministic
# epoch-2 ceremony is the activation DKG), we stop validator-3 BEFORE the epoch-2 DKG
# window opens (epoch_start(2) − DKG_MARGIN_BLOCKS) and keep it down across the
# epoch-2 boundary; the 3 survivors (n−f=3 quorum) carry committee[2]'s DKG and seed
# the beacon from epoch 2. Assert:
#   1. the chain reaches epoch 2 and finalizes WHILE validator-3 is down (the beacon
#      went live on n−f survivors);
#   2. validator-3, after restart, logs NO "share computed + stored" for epoch 2 (it
#      was excluded by Joint-Feldman QUAL → shareless for epoch 2 → it sits out);
#   3. the chain stays live after validator-3 rejoins (it re-derives prev_randao from
#      the cert seed like any verify-only node, and rejoins the SEED quorum only at
#      the NEXT DKG it attends — covered in the rotation stack, see the note below).
# NOTE: the "rejoins the seed quorum at the next DKG" leg requires a committee CHANGE
# (recurring DKG) — that is the rotation stack's domain (case-vrf-rotation.sh proves
# early-join for a joiner); here we prove the SIT-OUT + chain-liveness half on the
# stable genesis stack. NO reshare.
# DKG_MARGIN_BLOCKS = 20 (consensus/beacon/actor.rs; 10→16→20 for the AM5 fetch-before-finalize
# schedule — the shared SOAK_DKG_MARGIN_BLOCKS default below) — the window is
# [epoch_start(2) − 20, epoch_start(2)).
assert_vrf_dkg_liveness() {
    local DOWN SURVIVORS NODES EPOCH2_START WINDOW_OPEN BOUNDARY_PROBE PRE share_lines
    : "${SOAK_DKG_MARGIN_BLOCKS:=20}"   # single source (env-overridable); mirrors soak-invariants.sh's default
    DOWN=validator-3
    SURVIVORS=(validator-0 validator-1 validator-2 full-node)
    EPOCH2_START=$(( DPOS_ACTIVATION_BLOCK + 2 * EPOCH_INTERVAL ))
    WINDOW_OPEN=$(( EPOCH2_START - SOAK_DKG_MARGIN_BLOCKS ))   # DKG_MARGIN_BLOCKS (default 20)
    BOUNDARY_PROBE=$(( EPOCH2_START + 6 ))

    # Stop the victim BEFORE the epoch-2 DKG window opens, so it misses committee[2]'s
    # ceremony entirely. epoch 1 is seedless, so taking it down now does not affect a
    # live beacon (none yet) — it only excludes it from the epoch-2 deal.
    echo "smoke-vrf-dkg-liveness: bringing $DOWN down BEFORE the epoch-2 DKG window opens (block < $WINDOW_OPEN)"
    if (( $(finalized_dec) >= WINDOW_OPEN )); then
        echo "FAIL (smoke-vrf-dkg-liveness): chain already at/past the epoch-2 DKG window ($WINDOW_OPEN) — cannot stop the victim before its ceremony (raise the activation gap or run earlier)"
        exit 1
    fi
    docker compose stop "$DOWN" >/dev/null
    PRE=$(finalized_dec)
    # shellcheck disable=SC2034
    NODES=("${SURVIVORS[@]}")

    # 1) The chain crosses the epoch-2 boundary and finalizes on the n−f=3 survivors —
    #    committee[2]'s DKG completed without the offline member, beacon live from
    #    epoch 2.
    echo "smoke-vrf-dkg-liveness: waiting for finalized >= $BOUNDARY_PROBE with $DOWN down (n−f=3 quorum must seed epoch 2)"
    wait_finalized_ge "$BOUNDARY_PROBE" 400 >/dev/null || {
        echo "FAIL (smoke-vrf-dkg-liveness): chain did not reach the epoch-2 boundary with $DOWN down (survivors below n−f quorum / DKG could not complete shorthanded)"
        docker compose logs --tail=120 validator-0
        exit 1
    }
    wait_nodes_have "$BOUNDARY_PROBE" 120 || { echo "FAIL (smoke-vrf-dkg-liveness): survivors did not all reach $BOUNDARY_PROBE"; exit 1; }
    assert_beacon_window "$EPOCH2_START" "$BOUNDARY_PROBE" "dkg-liveness-epoch2"
    echo "smoke-vrf-dkg-liveness: beacon went LIVE at epoch 2 on the 3 survivors while $DOWN was offline during its DKG window"

    # 2) Restart the victim. It missed epoch 2's ceremony → it holds NO epoch-2 share.
    #    Assert it logs NO "share computed + stored" for epoch 2 (it sits out the seed
    #    quorum for epoch 2; it re-derives prev_randao from the cert like a verify-only
    #    node and stays live).
    echo "smoke-vrf-dkg-liveness: restarting $DOWN — it must NOT hold an epoch-2 share (excluded by QUAL)"
    docker compose start "$DOWN" >/dev/null
    # Let it catch up to the boundary so its logs for epoch 2 are complete.
    local catchup_deadline dh
    catchup_deadline=$(( SECONDS + 150 ))
    while :; do
        dh=$(mixhash_in "$DOWN" "$BOUNDARY_PROBE")
        [[ "$dh" != "null" && -n "$dh" ]] && break
        (( SECONDS < catchup_deadline )) || { echo "FAIL (smoke-vrf-dkg-liveness): $DOWN did not catch up to $BOUNDARY_PROBE after restart"; docker compose logs --tail=120 "$DOWN"; exit 1; }
        sleep 2
    done
    # The actor logs "live DKG: PK_epoch + share computed + stored" with epoch=<E> ONLY
    # for an epoch it actually finalized a share for. A member excluded from epoch 2's
    # QUAL produces no such line for epoch 2.
    share_lines=$(docker compose logs "$DOWN" 2>/dev/null \
        | grep "live DKG: PK_epoch + share computed + stored" | grep -E "epoch=2( |,|$)" || true)
    if [[ -n "$share_lines" ]]; then
        echo "FAIL (smoke-vrf-dkg-liveness): $DOWN logged an epoch-2 share despite being offline during the epoch-2 DKG window — it should be SHARELESS for epoch 2 (QUAL exclusion):"
        printf '%s\n' "$share_lines" | sed 's/^/    /'
        exit 1
    fi
    echo "smoke-vrf-dkg-liveness: $DOWN holds NO epoch-2 share (correctly excluded by QUAL — it sits out the epoch-2 seed quorum, NO reshare)"

    # 3) The chain stays live after the victim rejoins (it derives prev_randao from the
    #    cert seed like a verify-only node — byte-identical to the survivors).
    local v hi miss
    hi=$(finalized_dec)
    miss=()
    for ((v = EPOCH2_START; v <= BOUNDARY_PROBE; v++)); do
        dh=$(mixhash_in "$DOWN" "$v"); sv=$(mixhash_at "$v")
        [[ "$dh" == "null" || -z "$dh" ]] && { miss+=("$v=missing-on-$DOWN"); continue; }
        [[ "$dh" == "$sv" ]] || miss+=("$v: $DOWN=$dh != validator-0=$sv")
    done
    (( ${#miss[@]} == 0 )) || { echo "FAIL (smoke-vrf-dkg-liveness): restarted $DOWN derived divergent prev_randao (did not recover the cert seed as a verify-only node):"; printf '  %s\n' "${miss[@]}"; exit 1; }
    BEFORE=$hi; sleep 6; AFTER=$(finalized_dec)
    (( AFTER > BEFORE )) || { echo "FAIL (smoke-vrf-dkg-liveness): chain not finalizing after $DOWN rejoined ($AFTER <= $BEFORE)"; exit 1; }
    echo "smoke-vrf-dkg-liveness: chain stayed live after $DOWN rejoined ($BEFORE → $AFTER); $DOWN re-derived epoch-2 prev_randao from the cert seed byte-identically"

    echo "OK (smoke-vrf-dkg-liveness): a member offline during its epoch-2 DKG window was excluded from the ceremony (shareless, NO reshare) and SAT OUT the epoch-2 seed quorum, while the chain finalized on the n−f=3 survivors and the rejoined member re-derived prev_randao from the cert seed"
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
        # SAME-HEIGHT identity, not tip identity (de2de13c / 3f47bb91). The victim is
        # backfilling an EL gap on a chain producing a block a second and the two reads
        # are two separate round-trips, so "still one block behind" is the NORMAL shape
        # of a healthy recovery — demanding the two tips coincide made the 600s budget a
        # lottery. The fork half is what the compare existed for and is kept: v0's block
        # at the victim's OWN height must be the victim's block, so a victim at height h
        # on a different chain still fails, and the "null|null" pre-test still stops two
        # unreachable nodes (trivially equal) from reading as recovery — which is the exact
        # wedge shape this case was written for. `_aligned_now` owns that rule; a private
        # copy of it here is how it drifts.
        #
        # THE FLOOR IS `HEAD_WHILE_DOWN`, and running with no floor made this check VACUOUS.
        # Same-height identity alone is satisfied by a victim sitting on its own persisted tail:
        # a live run printed `realigned at 0x41(=65) … (v0=0x50(=80))` and PASSED — fifteen
        # blocks of the EL gap this case deliberately builds, never backfilled. `HEAD_WHILE_DOWN`
        # is the one height the chain is PROVEN to have reached during the cycle (it is read
        # right after the gap wait and the `>= PRE + 3` check below fails the case otherwise).
        # `PRE + 12` is NOT usable — that wait is `|| true`, a target and not a fact — and
        # v0's live reading is not usable either, since `_aligned_now` applies the floor to
        # EVERY reading including the producer's. The floor is honoured only as HEX.
        if _aligned_now "$(printf '0x%x' "$HEAD_WHILE_DOWN")" "$v0" "$v3" >/dev/null; then
            echo "OK (smoke-crash-survivor): validator-3 recovered from crash and realigned at $v3 (v0=$v0, floor=head-while-down=$HEAD_WHILE_DOWN)"
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
    echo "FAIL (smoke-crash-survivor): validator-3 did not realign after crash+restart (v0=$(check_external 8545) v3=$v3_final floor=head-while-down=$HEAD_WHILE_DOWN)"
    echo "  (Problem A: crash survivor wedged on a missing EL block — needs 2b FCU-driven recovery)"
    docker compose logs --tail=80 validator-3
    exit 1
}

# smoke-full-restart: stop ALL 4 validators (verify each persisted, exit 0), restart them, and
# assert the network reconverges from the persisted finalized head — i.e. DPoS
# cold-restart from disk works for the whole set, not just the migration anchor.
# Run LAST in the chain: it is the most invasive (stops the entire validator set).
assert_full_restart() {
    local pre v floor got last labels i detail
    pre=$(baseline_height)
    echo "smoke-full-restart: stopping all 4 validators at finalized=$pre"
    docker compose stop --timeout 40 "${VALS[@]}"
    for v in "${VALS[@]}"; do
        shutdown_flushed "$v" || { echo "FAIL (smoke-full-restart): $v did not exit cleanly (code 0) on shutdown"; exit 1; }
    done
    echo "  all persisted (exit 0); restarting"
    docker compose start "${VALS[@]}"

    # reconverge: all 5 align finalized STRICTLY PAST pre (resume from the persisted head AND
    # produce at least one block on top of it).
    #
    # The reading set is EXACTLY `_read_sequencer_nodes` (v0 + validators 1-3 + full-node),
    # so this reuses `_wait_aligned` outright instead of hand-rolling a fifth copy of the
    # cross-node compare. The old five-way byte-identity was the same race the sweep is
    # closing: the fleet is only coincident at the instant it comes back, and one second
    # later the five tips are ragged again for entirely healthy reasons. Same-height
    # identity keeps the half that mattered — a validator that came back on a DIFFERENT
    # chain still fails — and the "null"/"0x0" guards inside `_aligned_now` still reject
    # an all-down poll and a fleet that came back at genesis.
    #
    # `> pre`, NOT `>= pre` — and this is the half the same-height rewrite lost. Byte
    # identity used to make `>= pre` mean more than it says: five EQUAL readings at the
    # persisted head can only be the instant of resumption, so the case was in practice
    # watching the fleet come back together. Ragged heights break that equivalence — one
    # wedged node parked exactly at `pre` while the other four climb satisfies `>= pre` and
    # reads as reconvergence — and both trees duly passed at EXACTLY the floor
    # (`all 5 reconverged at 0x41 (>= pre=65)`), proving only that everyone came back on the
    # same persisted tail. With the floor at `pre` (and `_aligned_now`'s floor being strict)
    # every node must be at `pre + 1` or better: the chain produced at least one block after
    # the restart and all five saw it. `baseline_height` fails loud below 1, so the floor is
    # never negative. What is still NOT asserted is SUSTAINED liveness — one block is all
    # this demands, and nothing runs after it.
    floor=$(printf '0x%x' "$pre")
    if got=$(_wait_aligned 120 "$floor" _read_sequencer_nodes); then
        # Print EVERY node's height, not just the producer's: there is no victim/hub split
        # here, so the auditable evidence is the whole ragged set against the floor. This is a
        # post-align re-read (the aligned sweep is inside `_wait_aligned`), which is why it is
        # labelled as one — it is the diagnostic, and the verdict above is the assertion.
        mapfile -t last < <(_read_sequencer_nodes)
        labels=(validator-0 validator-1 validator-2 validator-3 full-node)
        detail=""
        for i in "${!labels[@]}"; do detail+="${detail:+, }${labels[$i]}=${last[$i]:-<no reading>}"; done
        echo "OK (smoke-full-restart): all 5 reconverged at $got (> pre=$pre) after full stop/start [post-align sweep: $detail]"
        return 0
    fi
    echo "FAIL (smoke-full-restart): network did not reconverge after full restart (> pre=$pre)"
    mapfile -t last < <(_read_sequencer_nodes)
    labels=(validator-0 validator-1 validator-2 validator-3 full-node)
    for i in "${!labels[@]}"; do echo "  ${labels[$i]}: ${last[$i]:-<no reading>}"; done
    docker compose logs --tail=200
    exit 1
}
