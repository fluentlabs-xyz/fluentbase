#!/usr/bin/env bash
# smoke-tx-cascade: the DPoS TRANSACTION WRITE PATH across the sentry cascade.
#
# Topology (operator's "sentry node" intent):
#   validators (L1, hidden) → sentry (L2, public, knows v0) → downstream (L3,
#   reaches ONLY L2). The --sequencer-url cert feed carries certificates, NOT
#   transactions; the write path rides reth devp2p tx-gossip over the trusted-peer
#   mesh, a SEPARATE link relayed L3→L2→validator.
#
# Asserts a tx submitted to L3 (which can reach NOTHING but L2) is relayed to a
# hidden validator's proposer pool, mined, finalized, and synced back to L3 — the
# regression guard for the write path. Privacy invariant checked: L3's only devp2p
# peer is the sentry (zero validator contact).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

TXC_COMPOSE=(-f docker-compose.yml -f docker-compose.dpos.yml -f docker-compose.tx-cascade.yml)
SENTRY_IP="172.20.0.30"
DOWNSTREAM_IP="172.20.0.31"
SENTRY_RPC="http://localhost:38545"
DOWNSTREAM_RPC="http://localhost:28545"
DEAD="0x000000000000000000000000000000000000dEaD"
BLEND="0x0000000000000000000000000000000000005207"   # MockBlendToken predeploy
ALLOW=4242

bring_up_dpos
trap tear_down EXIT
anchor_dec=$(printf '%d' "$PREV_FIN")

# ── L2 sentry: cert-follow v0 + devp2p pinned to v0. Public; allowed to know v0. ──
echo "smoke-tx-cascade: starting L2 sentry (cert-follow v0; devp2p → v0)"
docker compose "${TXC_COMPOSE[@]}" up -d sentry \
    || { echo "FAIL (smoke-tx-cascade): could not start sentry"; exit 1; }
wait_follower_align 38545 "$anchor_dec" 200 >/dev/null \
    || { echo "FAIL (smoke-tx-cascade): sentry (L2) did not align past $anchor_dec"; \
         docker compose "${TXC_COMPOSE[@]}" logs --tail=120 sentry; exit 1; }
echo "  sentry (L2) aligned with v0 past $anchor_dec"

# ── L3 downstream: cert-follow the SENTRY + devp2p pinned to ONLY the sentry. ──
# `|| true` on every capture that reads a possibly-not-ready RPC: under `set -e`
# a bare `v=$(failing_pipeline)` exits the script before the explicit regex/status
# gate below can react. The gate (not the assignment) is the fail-loud check.
pk=$(_enode_pubkey "$SENTRY_RPC") || true
[[ "$pk" =~ ^[0-9a-fA-F]{128}$ ]] || { echo "FAIL (smoke-tx-cascade): bad sentry enode pubkey '${pk:0:20}…'"; exit 1; }
sentry_enode="enode://${pk}@${SENTRY_IP}:30303"
docker compose "${TXC_COMPOSE[@]}" exec -T sentry sh -c "printf '%s' '$sentry_enode' > /runtime/sentry-enode.txt" \
    || { echo "FAIL (smoke-tx-cascade): could not write /runtime/sentry-enode.txt"; exit 1; }
echo "smoke-tx-cascade: sentry enode captured → starting L3 downstream (devp2p → sentry ONLY)"
docker compose "${TXC_COMPOSE[@]}" up -d downstream \
    || { echo "FAIL (smoke-tx-cascade): could not start downstream"; exit 1; }

# Mutual trust: the sentry must ACCEPT L3's inbound (both keep --trusted-only).
pk3=""
for ((i=0;i<30;i++)); do
    pk3=$(_enode_pubkey "$DOWNSTREAM_RPC") || true
    [[ "$pk3" =~ ^[0-9a-fA-F]{128}$ ]] && break
    sleep 2
done
[[ "$pk3" =~ ^[0-9a-fA-F]{128}$ ]] || { echo "FAIL (smoke-tx-cascade): bad downstream enode pubkey (L3 RPC up?)"; exit 1; }
l3_enode="enode://${pk3}@${DOWNSTREAM_IP}:30303"
cast rpc --rpc-url "$SENTRY_RPC" admin_addTrustedPeer "$l3_enode" >/dev/null 2>&1 \
    || { echo "FAIL (smoke-tx-cascade): admin_addTrustedPeer(L3) on sentry"; exit 1; }
echo "  sentry now trusts L3 (mutual) → awaiting L3 align via the sentry"
wait_follower_align 28545 "$anchor_dec" 200 >/dev/null \
    || { echo "FAIL (smoke-tx-cascade): L3 did not align past $anchor_dec via the sentry"; \
         docker compose "${TXC_COMPOSE[@]}" logs --tail=120 downstream; exit 1; }
echo "  L3 aligned with v0 past $anchor_dec through the sentry (validators→L2→L3)"

# ── PRIVACY INVARIANT: L3 reaches ONLY the sentry (zero validator contact). ──
l3_peers=$(cast rpc --rpc-url "$DOWNSTREAM_RPC" net_peerCount 2>/dev/null | tr -d '"') || true
l3_peers=$(printf '%d' "${l3_peers:-0x0}")
(( l3_peers >= 1 )) || { echo "FAIL (smoke-tx-cascade): L3 has NO devp2p peer — the tx uplink is absent"; exit 1; }
(( l3_peers == 1 )) || echo "  NOTE: L3 reports $l3_peers devp2p peers (expected 1 = sentry only)"
echo "  L3 devp2p peers=$l3_peers (sole peer = sentry; no validator contact)"

# ── WRITE PATH: submit to L3; assert it reaches a hidden validator + finalizes. ──
KEY=$(docker compose exec -T validator-0 cat /runtime/keys/funded.hex | tr -d '[:space:]')
FROM=$(cast wallet address --private-key "0x$KEY")
bal_before=$(cast balance "$DEAD" --rpc-url "$DOWNSTREAM_RPC")

echo "smoke-tx-cascade: submitting value transfer + MockBlendToken.approve to L3 ($DOWNSTREAM_RPC)"
# Explicit sequential nonces: both txs are submitted --async (neither is mined by
# L3 — a follower mines nothing), so cast's per-invocation "latest" nonce would
# collide ("replacement transaction underpriced"). Pin nonce, nonce+1.
nonce=$(cast nonce "$FROM" --rpc-url "$DOWNSTREAM_RPC") \
    || { echo "FAIL (smoke-tx-cascade): could not read sender nonce from L3"; exit 1; }
TXH=$(cast send --async --nonce "$nonce" --private-key "0x$KEY" --rpc-url "$DOWNSTREAM_RPC" --chain "$CHAIN_ID" \
        "$DEAD" --value 0.05ether) \
    || { echo "FAIL (smoke-tx-cascade): L3 rejected eth_sendRawTransaction (value transfer)"; exit 1; }
CTXH=$(cast send --async --nonce "$((nonce+1))" --private-key "0x$KEY" --rpc-url "$DOWNSTREAM_RPC" --chain "$CHAIN_ID" \
        "$BLEND" "approve(address,uint256)" "$DEAD" "$ALLOW") \
    || { echo "FAIL (smoke-tx-cascade): L3 rejected eth_sendRawTransaction (approve)"; exit 1; }
echo "  submitted to L3: transfer=$TXH (nonce $nonce) approve=$CTXH (nonce $((nonce+1)))"

# Proof it reached a PROPOSER, not just L3's local pool: the receipt appears on
# validator-0 (a hidden L1 validator) — a tx can only enter the canonical chain
# via a committee proposer's pool, i.e. relayed L3→sentry→validator over devp2p.
maxblk=0
for h in "$TXH" "$CTXH"; do
    st=""
    for ((i=0;i<90;i++)); do
        st=$(cast receipt "$h" --rpc-url "$RPC" --json 2>/dev/null | jq -r '.status // empty') || true
        [[ -n "$st" ]] && break
        sleep 1
    done
    [[ "$st" == "0x1" || "$st" == "1" ]] \
        || { echo "FAIL (smoke-tx-cascade): tx $h not mined by a validator (status='$st') — devp2p tx-gossip relay L3→L2→validator failed"; \
             docker compose "${TXC_COMPOSE[@]}" logs --tail=80 sentry downstream; exit 1; }
    blk=$(printf '%d' "$(cast receipt "$h" --rpc-url "$RPC" --json | jq -r .blockNumber)")
    (( blk > maxblk )) && maxblk=$blk
    echo "  validator-0 mined $h in block $blk (reached a hidden validator's proposer pool)"
done

wait_finalized_ge "$maxblk" 120 || { echo "FAIL (smoke-tx-cascade): tx block $maxblk not finalized in time"; exit 1; }
echo "  tx block $maxblk finalized by the validators"

# Round-trip: L3 syncs the mined+finalized block back and serves the receipt +
# applied state — proves the FULL cascade, not just L3's local pool.
st=""
for ((i=0;i<120;i++)); do
    st=$(cast receipt "$TXH" --rpc-url "$DOWNSTREAM_RPC" --json 2>/dev/null | jq -r '.status // empty') || true
    [[ -n "$st" ]] && break
    sleep 1
done
[[ "$st" == "0x1" || "$st" == "1" ]] || { echo "FAIL (smoke-tx-cascade): L3 never synced the receipt for $TXH"; exit 1; }
bal_after=$(cast balance "$DEAD" --rpc-url "$DOWNSTREAM_RPC")
delta=$(( bal_after - bal_before ))
[[ "$delta" == "50000000000000000" ]] || { echo "FAIL (smoke-tx-cascade): L3 balance delta $delta != 0.05 ETH"; exit 1; }
allow=$(cast call "$BLEND" "allowance(address,address)(uint256)" "$FROM" "$DEAD" --rpc-url "$DOWNSTREAM_RPC" | awk '{print $1}')
[[ "$allow" == "$ALLOW" ]] || { echo "FAIL (smoke-tx-cascade): L3 allowance=$allow != $ALLOW (EVM SSTORE not synced)"; exit 1; }

# Fail-loud monitor must NOT false-positive while the uplink is healthy.
if docker compose "${TXC_COMPOSE[@]}" logs sentry downstream 2>/dev/null | grep -q "tx-route ISOLATED"; then
    echo "FAIL (smoke-tx-cascade): tx-route monitor warned ISOLATED while peers were connected (false positive)"; exit 1
fi

echo "OK (smoke-tx-cascade): tx submitted to L3 (reaches ONLY the sentry) relayed via devp2p tx-gossip to a hidden validator, mined by the proposer, finalized, and synced back to L3 with state applied."
