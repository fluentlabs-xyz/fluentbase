#!/usr/bin/env bash
# smoke-liveness: with 4 validators (f=1, quorum 3) the network keeps finalizing
# while ONE is offline, the offline validator's on-chain liveness miss-counter
# rises (proving the signing bitmap reaches the chain and processBitmap marks it
# absent), and the validator re-syncs/rejoins on restart over reth devp2p.
#
# It rotates the victim across the three non-hub validators [v3, v2, v1], one at a
# time (f=1 ⇒ never two down at once; validator-0 is the hub whose enode the spokes
# pin as --trusted-peers, so it stays up). A full liveness slash is NOT awaited —
# rising miss-count + continued liveness + clean rejoin is the signal.
#
# Rejoin mechanism under test (NOT reth pipeline-backfill — that earlier framing was
# wrong): a validator down across ≥1 epoch boundary resumes in its stale epoch while
# the committee is ahead; it catches up on the CONSENSUS plane — the vote backup
# channel detects ahead-epoch votes → hints the marshal → the marshal walks the
# finalized tip forward boundary-by-boundary → each crossed epoch soft-enters its
# committee scheme (no engine) until the live epoch full-enters. The cycles below
# span the catch-up spectrum so the per-epoch walk is exercised at multiple depths.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

bring_up_dpos
trap tear_down EXIT

# Committee addresses (validators[i] == validator-i). jq is host-side; the file
# is read in-container (runtime image is curl-only). NB: no `tr -d [:space:]` —
# it would strip the per-line newlines too, collapsing all four into ADDR[0].
mapfile -t ADDR < <(docker compose exec -T validator-0 cat /runtime/addresses.json | jq -r '.validators[]')
(( ${#ADDR[@]} == 4 )) || { echo "FAIL (smoke-liveness): expected 4 validator addresses, got ${#ADDR[@]}: ${ADDR[*]}"; exit 1; }
V0_ADDR="${ADDR[0]}"

# 0-based committee index of address $1 in epoch $2 (peer-pubkey-sorted
# getEpochCommittee order), or -1 if absent.
signer_idx() {
    local addr="${1,,}" epoch="$2" comm
    comm=$(staking_call "getEpochCommittee(uint64)(address[])" "$epoch")
    grep -oE '0x[0-9a-fA-F]{40}' <<<"$comm" | nl -ba -v0 \
        | awk -v v="$addr" 'tolower($2)==v{print $1; f=1} END{if(!f) print -1}'
}

# On-chain miss-count of address $2 in epoch $1. Sentinels: -1 = not in
# committee; -2 = getter call FAILED (RPC error / empty output). Distinguishing
# -2 from a real 0 matters for the reference (hub) read: a failed read collapsing
# to 0 would make the `vmc > refmc` assertion easier to satisfy and would hide a
# regression that breaks the missCount getter. Callers must treat -2 as "retry",
# never as a valid count.
misscount() {
    local epoch="$1" idx out
    idx=$(signer_idx "$2" "$epoch")
    [[ "$idx" == "-1" ]] && { echo -1; return; }
    out=$(liveness_call "missCount(uint64,uint32)(uint32)" "$epoch" "$idx" 2>/dev/null) || true
    [[ -n "$out" ]] && printf '%s\n' "$out" || echo -2
}

# One kill/rejoin cycle. $1 = victim index (1..3), $2 = min blocks to advance
# while down (> EPOCH_INTERVAL forces the pipeline-backfill rejoin path).
liveness_cycle() {
    # NB: separate `local` statements — a single `local a=$1 b=validator-$a`
    # expands all RHS before assigning, so `$a` would be unbound under `set -u`.
    local n="$1" gap="$2"
    local svc="validator-$n" vic="${ADDR[$n]}"
    local pre cur vmc refmc deadline
    pre=$(baseline_height)
    echo "── cycle: victim=$svc ($vic), target gap >= $gap blocks (pre=$pre) ──"
    docker compose stop --timeout 40 "$svc"

    # 1) network keeps finalizing with 1/4 down (BFT f=1 holds) and advances the
    #    required gap (cycle 1: past a full epoch → reth pipeline-backfill rejoin).
    #    240s: cycle 1 waits 3*EPOCH_INTERVAL+1 = 97 blocks; at 1 blk/s with the
    #    victim's leader views timing out (1750ms) until skip_timeout mutes them,
    #    that's ~100-115s of chain time.
    wait_finalized_ge $(( pre + gap )) 240 || {
        echo "FAIL (smoke-liveness): chain did not advance $gap blocks with $svc down (finalized=$(finalized_dec), pre=$pre)"; exit 1; }
    echo "  chain finalized past $((pre+gap)) with $svc down (BFT f=1 holds)"

    # 2) the signing bitmap reaches the chain and is correct: the offline victim's
    #    miss-count rises in the current epoch and exceeds the always-up hub's
    #    (v0 is never killed → its miss-count stays minimal). > (not == 0) tolerates
    #    transient view-change misses on present nodes.
    deadline=$(( $(date +%s) + 90 ))
    vmc=-1; refmc=0
    while (( $(date +%s) < deadline )); do
        cur=$(staking_call "currentEpoch()(uint64)")
        vmc=$(misscount "$cur" "$vic"); refmc=$(misscount "$cur" "$V0_ADDR")
        # A failed getter read (-2) on either value must trigger a retry, never
        # satisfy or weaken the assertion.
        [[ "$vmc" == "-2" || "$refmc" == "-2" ]] && { sleep 2; continue; }
        [[ "$vmc" != "-1" ]] && (( vmc > 0 )) && (( vmc > refmc )) && break
        sleep 2
    done
    { [[ "$vmc" != "-1" && "$vmc" != "-2" && "$refmc" != "-2" ]] && (( vmc > 0 && vmc > refmc )); } || {
        echo "FAIL (smoke-liveness): bitmap/miss-count wrong (epoch=$cur victim=$vmc hub=$refmc)"; exit 1; }
    echo "  on-chain bitmap correct: missCount(epoch=$cur, $svc)=$vmc > hub=$refmc (>0)"

    # 3) rejoin: restart and assert the victim realigns to the hub's finalized
    #    head AND has a live reth devp2p peer (the EL transport that did the sync).
    docker compose start "$svc"
    deadline=$(( $(date +%s) + 120 ))
    local tick=0 v0 vn pc
    while (( $(date +%s) < deadline )); do
        v0=$(check_external 8545); vn=$(check_node docker compose exec -T "$svc")
        pc=$(peer_count "$svc")
        if [[ "$v0" == "$vn" && "${v0%%|*}" != "null" ]] && (( pc > 0 )); then
            echo "  OK: $svc rejoined at $vn with reth peers=$pc"; return 0
        fi
        (( tick % 7 == 0 )) && echo "    t+$((tick*2))s: $svc peers=$pc $svc=$vn v0=$v0"
        tick=$((tick+1)); sleep 2
    done
    echo "FAIL (smoke-liveness): $svc did not rejoin (peers=$(peer_count "$svc"), $svc=$(check_node docker compose exec -T "$svc"), v0=$(check_external 8545))"
    exit 1
}

# Catch-up spectrum (one at a time, each fully rejoins before the next):
#   cycle 1 (v3): DEEP — down across ~3 epoch boundaries; exercises the per-epoch
#                 soft-enter walk over several boundaries (the real rejoin stress).
#   cycle 2 (v2): SINGLE epoch-boundary cross (gap just over one epoch).
#   cycle 3 (v1): WITHIN-epoch gap (no boundary cross — full-enter, no soft-enter walk).
#   cycle 4 (v3): IMMEDIATE double-rejoin — re-kill a just-rejoined node across a
#                 boundary, to catch stale catch-up state carried across restarts
#                 (highest_entered_epoch / highest_observed_epoch).
liveness_cycle 3 $(( 3 * EPOCH_INTERVAL + 1 ))
liveness_cycle 2 $(( EPOCH_INTERVAL + 1 ))
liveness_cycle 1 5
liveness_cycle 3 $(( EPOCH_INTERVAL + 1 ))

echo "OK (smoke-liveness): [v3 deep, v2 single-boundary, v1 within-epoch, v3 double-rejoin] all held liveness while down, recorded on-chain miss-count, and rejoined the live tip"
