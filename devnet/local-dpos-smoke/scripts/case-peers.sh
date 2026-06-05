#!/usr/bin/env bash
# smoke-peers: both peer planes connect, and a restarted node re-establishes both.
#   - commonware consensus plane: discovery connects each validator to its
#     committee peers (observed via the devnet metrics endpoint, Metrics::encode
#     over --dpos.metrics-port on host :19100). Tracked peer set == on-chain
#     committee (Addendum B), so a healthy node converges to committee_size-1.
#   - reth devp2p plane (EL transport for block sync/catch-up): each spoke pins
#     validator-0's enode (--trusted-peers), so net_peerCount > 0. Regression
#     guard: under --dpos the override must keep reth peering wired (else rejoin
#     EL-sync breaks — see dpos_rejoin_el_sync_devp2p).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

bring_up_dpos
trap tear_down EXIT

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
        echo "OK (smoke-peers): commonware connected=$EXPECT + validator-1 reth peers>0 + chain advanced past $PRE after restart"; exit 0; }
    sleep 3
done
echo "FAIL (smoke-peers): after validator-1 restart connected=$(connected_count) (want $EXPECT), reth peers=$(peer_count validator-1) (want >0), finalized=$(finalized_dec) (want > $PRE)"
exit 1
