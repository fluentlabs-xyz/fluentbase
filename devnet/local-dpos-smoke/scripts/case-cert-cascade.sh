#!/usr/bin/env bash
# smoke-cert-cascade: the lean cert-follower's serving side + L1 trust root.
#   (1) L1-checkpoint align — MockRollup (same selectors as the real Rollup)
#       is deployed ON the devnet L2 and fed a real finalized hash; a tier-1
#       follower with --cert-follow.l1-rpc-url verifies the checkpoint against
#       its own synced chain and finalized-aligns with v0.
#   (2) cascade — a tier-2 follower whose ONLY upstream is tier 1's
#       window-backed `consensus` WS aligns too (followers serve followers).
#   (3) bogus-checkpoint reject — a follower pointed at a MockRollup whose
#       checkpoint hash is NOT in the chain must refuse to follow (fail-closed
#       trust root: zero finalized progress).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

CC_COMPOSE=(-f docker-compose.yml -f docker-compose.dpos.yml -f docker-compose.cert-cascade.yml)
T1_PORT=28545   # tier-1 follower reth RPC
T2_PORT=48545   # tier-2 follower reth RPC
BOGUS_PORT=58545

bring_up_dpos
trap tear_down EXIT

anchor=$(finalized_dec)
echo "smoke-cert-cascade: DPoS converged; anchor finalized=$anchor"

KEY=$(docker compose exec -T validator-0 cat /runtime/keys/funded.hex | tr -d '[:space:]')

# ── MockRollup deploy + checkpoint push ──────────────────────────────────────
BYTECODE=$(python3 -c "import json; print(json.load(open('contracts/MockRollup.json'))['bytecode']['object'])")
MOCK_ROLLUP_ADDR=$(cast send --private-key "0x$KEY" --rpc-url "$RPC" --create "$BYTECODE" --json | python3 -c "import sys,json; print(json.load(sys.stdin)['contractAddress'])")
echo "smoke-cert-cascade: MockRollup deployed at $MOCK_ROLLUP_ADDR"

fin_hex=$(check_external 8545 | cut -d'|' -f1)
fin_hash=$(check_external 8545 | cut -d'|' -f2)
set_block=$(cast send --private-key "0x$KEY" --rpc-url "$RPC" "$MOCK_ROLLUP_ADDR" \
    "setCheckpoint(uint256,bytes32)" 1 "$fin_hash" --json \
    | python3 -c "import sys,json; print(int(json.load(sys.stdin)['blockNumber'],16))")
echo "smoke-cert-cascade: checkpoint pushed in block $set_block (batch 1 → finalized $fin_hex $fin_hash)"
# The follower reads the Rollup at the FINALIZED tag (two-tier lag = K blocks):
# wait until the setCheckpoint block is finalized before starting it.
wait_finalized_ge "$set_block" || { echo "FAIL: setCheckpoint block never finalized"; exit 1; }

export MOCK_ROLLUP_ADDR

# ── Phase 1: tier-1 follower with the L1 trust root ─────────────────────────
docker compose "${CC_COMPOSE[@]}" up -d cert-follower-l1

align_target=$(( anchor + EPOCH_INTERVAL ))
aligned=$(wait_follower_align "$T1_PORT" "$align_target" 240) || {
    echo "FAIL (smoke-cert-cascade): tier-1 did not align with v0 past $align_target"
    docker compose "${CC_COMPOSE[@]}" logs --tail=200 cert-follower-l1
    exit 1
}
# NB: `logs | grep -q` under pipefail false-fails on grep's early pipe close
# (docker gets SIGPIPE -> 141) — capture to a file first.
docker compose "${CC_COMPOSE[@]}" logs --no-color cert-follower-l1 > /tmp/cc-t1.log 2>&1 || true
grep -q "L1 Rollup checkpoint verified" /tmp/cc-t1.log \
    || { echo "FAIL (smoke-cert-cascade): tier-1 aligned but the L1 checkpoint assert never ran"; exit 1; }
echo "OK (phase 1 L1-checkpoint align): tier-1 verified the Rollup checkpoint and aligned at $aligned"

# ── Phase 2: tier-2 follower fed ONLY by tier 1 ──────────────────────────────
docker compose "${CC_COMPOSE[@]}" up -d cert-follower-tier2

t2_target=$(( $(printf '%d' "${aligned%%|*}") + EPOCH_INTERVAL ))
t2_aligned=$(wait_follower_align "$T2_PORT" "$t2_target" 240) || {
    echo "FAIL (smoke-cert-cascade): tier-2 did not align via the tier-1 window past $t2_target"
    docker compose "${CC_COMPOSE[@]}" logs --tail=200 cert-follower-tier2 cert-follower-l1
    exit 1
}
echo "OK (phase 2 cascade): tier-2 aligned with v0 at $t2_aligned through tier 1"

# ── Phase 3: bogus checkpoint must fail closed ───────────────────────────────
# A second MockRollup whose checkpoint hash exists nowhere in the chain.
BOGUS_ROLLUP_ADDR=$(cast send --private-key "0x$KEY" --rpc-url "$RPC" --create "$BYTECODE" --json | python3 -c "import sys,json; print(json.load(sys.stdin)['contractAddress'])")
bogus_block=$(cast send --private-key "0x$KEY" --rpc-url "$RPC" "$BOGUS_ROLLUP_ADDR" \
    "setCheckpoint(uint256,bytes32)" 1 \
    0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef --json \
    | python3 -c "import sys,json; print(int(json.load(sys.stdin)['blockNumber'],16))")
wait_finalized_ge "$bogus_block" || { echo "FAIL: bogus setCheckpoint block never finalized"; exit 1; }
export BOGUS_ROLLUP_ADDR
docker compose "${CC_COMPOSE[@]}" up -d cert-follower-l1-bogus

deadline=$(( $(date +%s) + 240 ))
rejected=""
while (( $(date +%s) < deadline )); do
    docker compose "${CC_COMPOSE[@]}" logs --no-color cert-follower-l1-bogus > /tmp/cc-bogus.log 2>&1 || true
    if grep -q "NOT in the local chain" /tmp/cc-bogus.log; then
        rejected=1; break
    fi
    # The node may have already exited after the refusal.
    state=$(docker compose "${CC_COMPOSE[@]}" ps --format '{{.State}}' cert-follower-l1-bogus 2>/dev/null || true)
    [[ "$state" == "exited" ]] && { rejected=1; break; }
    sleep 3
done
if [[ -z "$rejected" ]]; then
    echo "FAIL (smoke-cert-cascade): bogus-checkpoint follower did not refuse"
    docker compose "${CC_COMPOSE[@]}" logs --tail=120 cert-follower-l1-bogus
    exit 1
fi
b=$(check_external "$BOGUS_PORT" 2>/dev/null || echo "null|null")
b_h="${b%%|*}"
if [[ "$b_h" != "null" ]] && (( $(printf '%d' "$b_h") > anchor )); then
    echo "FAIL (smoke-cert-cascade): bogus-checkpoint follower made finalized progress ($b)"
    exit 1
fi
echo "OK (phase 3 bogus-checkpoint reject): follower refused the unverifiable trust root (finalized=$b_h)"

echo "OK (smoke-cert-cascade): L1 trust root + follower cascade + fail-closed reject all verified"
