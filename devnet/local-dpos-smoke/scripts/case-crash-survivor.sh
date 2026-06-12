#!/usr/bin/env bash
# smoke-crash-survivor (Problem A): a validator is CRASHED ungracefully
# (SIGKILL, no persistence flush) mid-operation, the chain advances while it is
# down (building an EL gap), then it is restarted. Assert it recovers its EL and
# realigns to the honest finalized head instead of wedging on a missing block.
# Contrast with smoke-liveness, which uses a graceful `stop` (flushed shutdown).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

bring_up_dpos
trap tear_down EXIT

PRE=$(baseline_height)
# Use the raw container id + raw docker kill/start so the crash+restart is surgical
# (no `docker compose start` dependency re-run of genesis-init, which races on the
# ungraceful path and made the restart flaky).
VIC_CID=$(docker compose ps -q validator-3)
[[ -n "$VIC_CID" ]] || { echo "FAIL: could not resolve validator-3 container id"; tear_down; exit 1; }
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
        exit 0
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
