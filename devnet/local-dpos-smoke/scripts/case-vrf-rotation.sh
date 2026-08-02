#!/usr/bin/env bash
# smoke-vrf-rotation: the LIVE per-epoch DKG beacon STAYS LIVE + NODE-AGREED across a
# committee change, and the prev_randao beacon RELIVES from the first block of the
# new epoch (boundary deriver reads PK_E from the in-block beacon_outcome).
#
# The threshold beacon has NO on-chain mirror anymore (getEpochBeaconKey /
# commitEpochBeaconKey are REMOVED): the per-block seed rides the consensus cert and
# prev_randao = H(seed). So the rotation is proven NOT by reading a rotated group key
# off-chain, but by asserting the OBSERVABLE beacon output — prev_randao — stays
# non-zero, varying, and byte-IDENTICAL across all honest nodes before, ACROSS, and
# after the committee-change boundary, while the per-member ACTIVE_LINE count keeps
# growing (proving a fresh DKG actually re-dealt and every member, the joiner v5
# included, casts verified seed partials).
#
# This builds on smoke-vrf (which proves the beacon is live at the INITIAL
# committee) by driving a real committee rotation — the exact mechanism from
# smoke-production-path (register an external validator v5 via governance +
# delegate enough BLEND to outrank an initial validator) — and asserting that the
# beacon survives the change LIVE + node-agreed (a fresh DKG re-dealt, no break).
#
# It asserts ALL of:
#   1. BASELINE — the beacon is THRESHOLD-ACTIVE from EPOCH 2 (deterministic
#      bootstrap): committee[2] DKGs during epoch 1 EVEN ON A STABLE committee, so
#      the ACTIVE_LINE fires on the validators from epoch 2 (NOT keyless-until-
#      rotation — that premise is GONE with the genesis-bake removal + epoch-2
#      bootstrap). prev_randao is non-zero + varies + byte-identical across all nodes.
#   2. TRIGGER — register external validator v5 (governance activate + delegate
#      >1e18 BLEND). committee[N] reads EffBal(N-1) and a delegate is effective in
#      EffBal(E+2) ⇒ v5 enters the committee at E+3 (mirrors smoke-production-path's
#      arithmetic; we compute it, not hardcode it — we wait for the FIRST epoch
#      whose committee set differs from the pre-rotation committee).
#   3. ROTATION — let E_new be that first changed-committee epoch. The fresh DKG
#      re-deals: the beacon stays LIVE + node-agreed across the E_new boundary
#      (asserted via prev_randao in step 4, not a rotated on-chain key).
#   4. EARLY-JOIN / RELIVE — v5 PARTICIPATED in committee[E_new]'s DKG during E_new-1
#      FROM ITS FOLLOWER PHASE (the always-on beacon plane deals/receives regardless
#      of the consensus role), HOLDS a CeremonyStore[E_new] entry, and is a FULL
#      beacon SIGNER at E_new — its ACTIVE_LINE count GROWS across the E_new boundary
#      AS A NEW MEMBER and it counts toward the seed quorum. The active-growth probe
#      set INCLUDES v5. prev_randao is non-zero + node-agreed at and just past the
#      E_new boundary (the fresh DKG's PK_E_new verifies the seed on every node).
#      Positive proof: v5 logs the live-DKG ceremony lifecycle ("live DKG: ceremony
#      started" / "PK_epoch + share computed + stored") during E_new-1 WHILE still a
#      cert-follower (before its promotion boundary).
#   5. CARRY-FORWARD — pick a STABLE epoch AFTER E_new (committee unchanged vs its
#      predecessor; we scan for one). The DKG only re-deals on a committee CHANGE, so
#      a stable epoch runs no fresh ceremony — the prior key carries forward in
#      memory and the beacon stays LIVE + node-agreed across it (prev_randao
#      non-zero, varying, byte-identical over a window inside E_s). The carry-forward
#      is an internal detail with no on-chain mirror; the live + agreed beacon across
#      the stable epoch is the full proof.
#
# This case runs the PRODUCTION-PATH stack (runtime forge deploy + 6 validators),
# because that is the harness that can actually rotate the committee.
#
# EPOCH-2 ACTIVATION NOTE: there are NO beacon flags anymore (no genesis bake, no
# --dpos.no-beacon — both REMOVED). Every node, devnet smokes included, goes through
# the SAME always-on live DKG: epoch 1 is seedless and the FIRST key is the
# DETERMINISTIC epoch-2 ceremony's PK_2 (committee[2] DKGs during epoch 1 even on a
# stable committee). So the beacon is THRESHOLD-ACTIVE from epoch 2 on this 5-member
# committee — NOT keyless-until-the-first-rotation. HERE we additionally test
# EARLY-JOIN: when v5 joins committee[E_new] it deals committee[E_new]'s share during
# E_new-1 FROM ITS FOLLOWER PHASE (the always-on beacon plane runs the DkgActor
# regardless of consensus role), so it promotes at E_new with its share already in the
# shared CeremonyStore and is a FULL beacon signer from block 1 of E_new — NOT an
# observer. The active-count-growth probe set therefore INCLUDES v5, and we grep v5's
# logs for the live-DKG ceremony lifecycle during E_new-1 (while it is still a
# cert-follower) as the direct early-join proof.
#
# PREREQUISITES (host): docker, foundry (forge/cast), jq, a solidity-contracts
# checkout at $SOLIDITY_CONTRACTS_DIR.
set -euo pipefail
cd "$(dirname "$0")/.."

# Run the production-path stack (own 6-node compose project), as case-production-path
# does. lib.sh's `docker compose` helpers inherit this.
export COMPOSE_FILE="docker-compose.production-path.yml"
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

SOLIDITY_CONTRACTS_DIR="${SOLIDITY_CONTRACTS_DIR:-../../../solidity-contracts}"
MANIFEST="$(cd "$SOLIDITY_CONTRACTS_DIR" && pwd)/deployments/runtime-deployment.json"

STAKING_RT="" ; CHAIN_CONFIG_RT="" ; GOV_ADDR="" ; LIVENESS_RT=""
STAKE_1E18="1000000000000000000"

cleanup() { pp_spammer_stop; rm -f "$MANIFEST"; tear_down; }
trap cleanup EXIT

forge_l2() { ( cd "$SOLIDITY_CONTRACTS_DIR" && "$@" ); }

# mixhash_at/in/of, is_zero_hash, log_count, assert_beacon_window,
# wait_nodes_have are now shared in lib.sh.

# Head (latest/tip) block number on the local RPC. The ACTIVE_LINE fires at the
# SPECULATIVE TIP (notarize-time derive under deferred execution), so a growth
# probe must wait for the HEAD to advance — finalized can catch up to a burst-
# ahead tip WITHOUT the tip producing new blocks (false "frozen"); see case-vrf.sh
# 2b for the same idiom.
head_dec() {
    curl -s -X POST -H 'Content-Type: application/json' \
        --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
        "$RPC" 2>/dev/null | jq -r '.result // "0x0"' \
        | { read -r h; printf '%d' "$h" 2>/dev/null || echo 0; }
}

# All deriving nodes on the production-path stack: 6 validators + full-node.
NODES=(validator-0 validator-1 validator-2 validator-3 validator-4 validator-5 full-node)
ACTIVE_LINE="beacon: threshold prev_randao active"

# assert_beacon_window + wait_nodes_have are now shared in lib.sh.

# ════════════════════════════════════════════════════════════════════════════
# Bring up the production-path DPoS stack with the initial 5-validator committee.
# The bring-up (phases A..cold-restart: forge-deploy token+verifier+cluster,
# setConsensusKeys, set activation, clean-halt, --dpos cold-restart) is the
# shared `pp_bring_up_rotation` helper in lib.sh — identical to the durability
# case. It sets DEPLOYER_KEY/ADDR, TOKEN, VERIFIER, STAKING_RT, CHAIN_CONFIG_RT,
# GOV_ADDR, LIVENESS_RT, ACT, ANCHOR, EPOCH_LEN, defines epoch_first_block(), and
# starts the tx spammer. The only difference between the two cases is what they
# assert afterwards (beacon rotation here, not the join/eject lifecycle).
# ════════════════════════════════════════════════════════════════════════════
PP_ROT_LABEL="smoke-vrf-rotation"
pp_bring_up_rotation

# ════════════════════════════════════════════════════════════════════════════
# 1) BASELINE — the deterministic epoch-2 bootstrap brings the beacon
#    THRESHOLD-ACTIVE from epoch 2 even on the STABLE initial committee:
#    prev_randao is non-zero + varying + node-agreed, and the ACTIVE_LINE fires on
#    the validators from epoch 2. (No genesis bake, no --dpos.no-beacon — see the
#    EPOCH-2 ACTIVATION NOTE.)
# ════════════════════════════════════════════════════════════════════════════
echo "== 1) BASELINE: chain live + epoch-2 beacon active on the initial committee =="
E0=$(pp_current_epoch)
GOT0=$(pp_committee "$E0")
EXPECT0=$(for i in 0 1 2 3 4; do pp_owner_addr "$i"; done | tr 'A-F' 'a-f' | sort | paste -sd' ' -)
[[ "$GOT0" == "$EXPECT0" ]] || { echo "FAIL (smoke-vrf-rotation): committee(E0=$E0) != initial 5 (got [$GOT0] want [$EXPECT0])"; exit 1; }
echo "  committee(epoch $E0) == initial 5"

# Epoch-2 activation: wait for the chain to enter epoch >= 2 — committee[2] DKGed
# during epoch 1 even on the stable committee, so the beacon is threshold-active from
# epoch 2 (the prev_randao window below is the observable proof).
EPOCH2_BOUNDARY=$(epoch_first_block 2)
wait_finalized_ge "$(( EPOCH2_BOUNDARY + 4 ))" 600 \
    || { echo "FAIL (smoke-vrf-rotation): chain did not reach epoch 2 ($EPOCH2_BOUNDARY) — cannot observe the deterministic epoch-2 activation"; docker compose logs validator-0 --tail=80; exit 1; }

# prev_randao live over a window inside epoch >= 2 — non-zero, varying, node-agreed
# (now the threshold path, not the digest fallback).
fin=$(finalized_dec)
(( fin > 0 )) || { echo "FAIL (smoke-vrf-rotation): no finalized block"; exit 1; }
WINDOW=8
lo=$(( fin > WINDOW ? fin - WINDOW + 1 : 1 ))
(( lo >= EPOCH2_BOUNDARY )) || lo=$EPOCH2_BOUNDARY
echo "  checking baseline prev_randao window [$lo..$fin] (inside epoch >= 2)"
assert_beacon_window "$lo" "$fin" "baseline epoch-2-active"
echo "  baseline beacon THRESHOLD-ACTIVE from epoch 2 on the stable committee"

# ════════════════════════════════════════════════════════════════════════════
# 2) TRIGGER — register external validator v5 (governance + delegate). The
#    committee change is what forces a fresh DKG → a rotated beacon key.
#    Mechanism + arithmetic copied from case-production-path.sh: a delegate is
#    effective in EffBal(E+2); committee[N] reads EffBal(N-1) ⇒ entry at E+3.
# ════════════════════════════════════════════════════════════════════════════
echo "== 2) TRIGGER: register external validator v5 to rotate the committee =="
REG_FLOOR=$(check_external 8545 | cut -d'|' -f1)
V5_KEY="$(pp_owner_key 5)" ; V5_ADDR="$(pp_owner_addr 5)"
v5l=$(tr 'A-F' 'a-f' <<<"$V5_ADDR")
cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "$STAKE_1E18" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null
cast send "$STAKING_RT" "registerValidator(address,uint16,uint256)" "$V5_ADDR" 0 "$STAKE_1E18" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL (smoke-vrf-rotation): registerValidator v5"; exit 1; }
ck=$(pp_consensus_keys 5)
cast send "$STAKING_RT" "setConsensusKeys(address,bytes,bytes,bytes32)" \
    "$(jq -r '.validatorAddress' <<<"$ck")" "$(jq -r '.blsPubkeyUncompressed' <<<"$ck")" \
    "$(jq -r '.blsPoPUncompressed' <<<"$ck")" "$(jq -r '.peerPubkey' <<<"$ck")" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL (smoke-vrf-rotation): setConsensusKeys v5"; exit 1; }
pp_gov_action "$STAKING_RT" \
    "$(cast calldata 'activateValidator(address)' "$V5_ADDR")" \
    "activateValidator-v5" || { echo "FAIL (smoke-vrf-rotation): gov activateValidator v5"; exit 1; }
cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "2000000000000000000" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null
cast send "$STAKING_RT" "delegate(address,uint256)" "$V5_ADDR" "2000000000000000000" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL (smoke-vrf-rotation): delegate v5"; exit 1; }
echo "  v5 registered + activated + delegated"

pp_wait_converge 180 "$REG_FLOOR" >/dev/null \
    || { echo "FAIL (smoke-vrf-rotation): nodes lost alignment during registration"; docker compose logs validator-5 --tail=80; exit 1; }
echo "  v5 follower substrate tracked the chain through registration"

# Wait for the FIRST epoch whose committee set differs from E0's — that is E_new.
# We do NOT hardcode E0+3; we scan the ahead-committed committees for the first
# change (the production-path arithmetic predicts E0+3, but we assert against the
# real on-chain committee so a re-tuned EffBal timeline does not silently skew us).
echo "== wait for the committee to actually change (expect ~E0+3 by EffBal arithmetic) =="
E_new=""
_deadline=$(( $(date +%s) + 900 ))
while (( $(date +%s) < _deadline )); do
    E=$(pp_current_epoch)
    # committee[E+1] is committed one boundary ahead; inspect it for v5.
    AHEAD=$(pp_committee $((E + 1)))
    if [[ -n "$AHEAD" && " $AHEAD " == *" $v5l "* && "$AHEAD" != "$GOT0" ]]; then
        E_new=$((E + 1)); break
    fi
    sleep 2
done
[[ -n "$E_new" ]] || { echo "FAIL (smoke-vrf-rotation): committee never changed (v5 never entered an ahead-committed committee within 900s)"; docker compose logs validator-5 --tail=80; exit 1; }
GOT_NEW=$(pp_committee "$E_new")
[[ "$GOT_NEW" != "$GOT0" ]] || { echo "FAIL (smoke-vrf-rotation): committee(E_new=$E_new) equals E0's — not actually a rotation"; exit 1; }
echo "  committee changed at E_new=$E_new (E0=$E0): [$GOT_NEW] (was [$GOT0])"

# EARLY-JOIN: ALL of committee[E_new] holds a share at E_new — the stayers ran
# E_new's DKG alongside their signer engine, AND the joiner v5 ran it from its
# FOLLOWER phase (the always-on beacon plane deals/receives regardless of consensus
# role). So the active-growth probe set is the FULL committee[E_new] INCLUDING v5
# (not just the stayers). Map each committee member address back to its validator
# container.
PROBE_MEMBERS=()
for i in 0 1 2 3 4 5; do
    al=$(tr 'A-F' 'a-f' <<<"$(pp_owner_addr "$i")")
    if [[ " $GOT_NEW " == *" $al "* ]]; then
        PROBE_MEMBERS+=("validator-$i")
    fi
done
[[ ${#PROBE_MEMBERS[@]} -ge 1 ]] || { echo "FAIL (smoke-vrf-rotation): could not map committee[E_new] to any validator container"; exit 1; }
# v5 (the joiner) MUST be in committee[E_new] (that is the rotation we drove) and so
# in the probe set — the early-join assertion targets it specifically.
printf '%s\n' "${PROBE_MEMBERS[@]}" | grep -qx validator-5 \
    || { echo "FAIL (smoke-vrf-rotation): validator-5 (the joiner) is not in committee[E_new] — the rotation did not bring v5 in"; exit 1; }
# Prefer validator-0 (host-mapped 8545) first if it is a member.
if printf '%s\n' "${PROBE_MEMBERS[@]}" | grep -qx validator-0; then
    PROBE_MEMBERS=(validator-0 $(printf '%s\n' "${PROBE_MEMBERS[@]}" | grep -vx validator-0))
fi
echo "  committee[E_new] share-holders (active-growth probe set, incl. joiner v5): ${PROBE_MEMBERS[*]}"

# POSITIVE EARLY-JOIN PROOF: v5 ran committee[E_new]'s DKG during E_new-1 WHILE it
# was still a cert-follower (before its promotion boundary). Grep its logs for the
# live-DKG ceremony lifecycle (actor.rs: "ceremony started" / "computed + stored").
# This is the direct proof the former follower dealt/received in E_new's ceremony —
# i.e. it holds a share at E_new, not merely the on-chain group key.
v5_dkg=$(docker compose logs validator-5 2>/dev/null \
    | grep -E "live DKG: ceremony started|live DKG: PK_epoch \+ share computed \+ stored" | tail -4 || true)
if [[ -z "$v5_dkg" ]]; then
    echo "FAIL (smoke-vrf-rotation): EARLY-JOIN — validator-5 logged NO live-DKG ceremony lifecycle during E_new-1; it did NOT deal/receive its committee[E_new] share from its follower phase (would be a beacon OBSERVER, not an early-join signer)"
    docker compose logs validator-5 2>/dev/null | grep "live DKG" | tail -10 | sed 's/^/    /' || true
    exit 1
fi
echo "  EARLY-JOIN proof — validator-5 ran committee[E_new]'s DKG from its follower phase:"
printf '%s\n' "$v5_dkg" | sed 's/^/    /'

# ════════════════════════════════════════════════════════════════════════════
# 3) ROTATION — the fresh DKG re-deals for E_new. There is no on-chain group-key
#    mirror anymore, so the rotation is proven by the OBSERVABLE beacon staying live
#    + node-agreed across the E_new boundary (step 4 prev_randao) PLUS the fresh-DKG
#    ceremony actually running (step 4 ACTIVE_LINE growth + v5's logged lifecycle).
#    Here we only WAIT for the chain to cross the E_new boundary so step 4 has a
#    window strictly inside the rotated epoch.
# ════════════════════════════════════════════════════════════════════════════
echo "== 3) ROTATION: cross the E_new boundary (fresh DKG re-deals; proven via prev_randao + ACTIVE_LINE) =="
E_NEW_BOUNDARY=$(epoch_first_block "$E_new")
# Generous budget: the 6-node production-path stack runs well below 1 blk/s under
# the spammer load (and the DKG batch-verify) — the climb from the discovery point
# (~epoch E_new-1) to the boundary is ~EPOCH_LEN blocks. On timeout, distinguish a
# STALL (head frozen) from merely-slow by sampling head twice.
if ! wait_finalized_ge "$E_NEW_BOUNDARY" 900; then
    h1=$(finalized_dec); sleep 10; h2=$(finalized_dec)
    echo "FAIL (smoke-vrf-rotation): chain did not reach E_new boundary block $E_NEW_BOUNDARY in 900s (finalized $h1 → $h2 over 10s — $([[ "$h2" -gt "$h1" ]] && echo 'advancing, just slow: raise the budget' || echo 'FROZEN: real stall at/before the rotation'))"
    echo "  --- live-DKG ceremony lifecycle on the stayers (started/computed?) ---"
    for s in validator-0 validator-2 validator-3 validator-4; do
        echo "  [$s]:"; docker compose logs "$s" 2>/dev/null | grep "live DKG" | tail -6 | sed 's/^/    /' || true
    done
    docker compose logs validator-0 --tail=80; exit 1
fi
echo "  crossed the E_new=$E_new boundary (block $E_NEW_BOUNDARY) — asserting the live + agreed beacon next"

# ════════════════════════════════════════════════════════════════════════════
# 4) EARLY-JOIN / RELIVE — the beacon is assurance=true from the FIRST block of
#    E_new on the FULL committee[E_new] INCLUDING the joiner v5 (early-join): the
#    boundary deriver reads PK_E from the in-block beacon_outcome, prev_randao is
#    non-zero + agreed at and just past the boundary, and the ACTIVE_LINE count
#    keeps GROWING on every member — v5 included, as a NEW signer that holds its
#    share, not as an observer.
# ════════════════════════════════════════════════════════════════════════════
echo "== 4) EARLY-JOIN: beacon live from the first block of E_new on ALL members (incl v5) =="
# Snapshot ACTIVE_LINE counts on EVERY committee[E_new] member (stayers AND v5) —
# under early-join v5 holds its share and logs assurance=true like the stayers.
declare -A before_relive
for v in "${PROBE_MEMBERS[@]}"; do
    c=$(log_count "$v" "$ACTIVE_LINE"); before_relive[$v]=${c:-0}
done

# Advance a few blocks past the E_new boundary so we have a window strictly inside
# the new epoch to check (and so the growth check below has room).
RELIVE_HI=$(( E_NEW_BOUNDARY + 4 ))
wait_finalized_ge "$RELIVE_HI" 180 \
    || { echo "FAIL (smoke-vrf-rotation): chain did not advance past the E_new boundary ($RELIVE_HI) — cannot observe a sustained post-rotation beacon"; exit 1; }
# Followers (full-node + freshly-promoted v5) lag the validators at the fresh
# boundary — wait for ALL nodes to have the window before the strict cross-node
# check, else assert_beacon_window trips "node behind" on a node still catching up.
wait_nodes_have "$RELIVE_HI" 180 \
    || { echo "FAIL (smoke-vrf-rotation): not all nodes reached the relive window block $RELIVE_HI (a follower never caught up)"; exit 1; }
# Window: the boundary block itself + the next few blocks of E_new — checked across
# ALL nodes (incl v5 + full-node): they all DERIVE prev_randao by verifying the
# cert seed vs the committed PK_E_new, so it is byte-identical everywhere even
# though only the stayers hold shares.
assert_beacon_window "$E_NEW_BOUNDARY" "$RELIVE_HI" "relive E$E_new"

# Active-count growth must track the HEAD (the speculative tip where ACTIVE_LINE
# fires), not finalized — same reason as case-vrf.sh 2b.
head0=$(head_dec)
hdeadline=$(( SECONDS + 90 ))
while (( $(head_dec) < head0 + 3 )); do
    if (( SECONDS >= hdeadline )); then
        echo "FAIL (smoke-vrf-rotation): head did not advance >= 3 past $head0 within 90s — cannot observe a sustained post-rotation beacon"
        exit 1
    fi
    sleep 1
done
for v in "${PROBE_MEMBERS[@]}"; do
    after=$(log_count "$v" "$ACTIVE_LINE"); after=${after:-0}
    if (( after <= ${before_relive[$v]} )); then
        m="share-holder"; [[ "$v" == validator-5 ]] && m="EARLY-JOIN newcomer"
        echo "FAIL (smoke-vrf-rotation): $v ($m) active-count frozen at $after across the E_new boundary — it is NOT casting verified seed partials as a committee[E_new] member (beacon did not relive / it is a shareless observer)"
        docker compose logs --tail=80 "$v"; exit 1
    fi
    echo "  $v — active-count grew ${before_relive[$v]} → $after across the rotation ($([[ "$v" == validator-5 ]] && echo 'EARLY-JOIN: v5 votes as a new member' || echo 'beacon relive'))"
done

# ════════════════════════════════════════════════════════════════════════════
# 5) CARRY-FORWARD — a STABLE epoch (committee unchanged vs its predecessor) runs
#    NO fresh DKG ceremony (the DKG only re-deals on a committee CHANGE), yet the
#    beacon stays LIVE + node-agreed across it: the prior key carries forward in
#    memory and still verifies the seed. There is no on-chain key to read; the
#    proof is the prev_randao window inside E_s (non-zero, varying, node-agreed).
# ════════════════════════════════════════════════════════════════════════════
echo "== 5) CARRY-FORWARD: stable epoch carries E_new's key forward (live beacon) =="
# The stable epoch we pick is AFTER E_new so the carry-forward we assert is E_new's
# ROTATED key (not the epoch-2 bootstrap key). A stable epoch in [2, E_new) already
# carries the epoch-2 key forward, but E_new's key is the one whose carry-forward we
# want to pin here. The first stable epoch after the rotation is E_new+1: its committee
# equals E_new's (no committee change → no own DKG commit). The committee may keep
# OSCILLATING afterwards (a joiner whose delegated weight does not durably hold its
# slot is bumped back out a couple of epochs later — a stake-dynamics artifact of
# the minimal devnet set, NOT a beacon property), so we scan from E_new+1 upward
# for the FIRST stable epoch rather than assuming the set has permanently settled.
# The chain must have ENTERED at least epoch E_new+2 so that E_new+1 is excluded
# from the `e < cur_now` guard below (which skips the current, not-yet-finalized
# epoch's committee).
WAIT_BOUNDARY=$(epoch_first_block "$(( E_new + 2 ))")
wait_finalized_ge "$(( WAIT_BOUNDARY + (EPOCH_LEN > 5 ? 4 : EPOCH_LEN - 1) ))" 900 \
    || { echo "FAIL (smoke-vrf-rotation): chain did not elapse into epoch $(( E_new + 2 )) (head=$(finalized_dec))"; docker compose logs validator-0 --tail=80; exit 1; }
E_s=""
cur_now=$(pp_current_epoch)
for ((e = E_new + 1; e < cur_now; e++)); do
    c_e=$(pp_committee "$e")
    c_p=$(pp_committee $((e - 1)))
    # both readable (non-empty) and equal → unchanged committee (carry-forward).
    if [[ -n "$c_e" && "$c_e" == "$c_p" ]]; then
        E_s="$e"; break
    fi
done
[[ -n "$E_s" ]] || { echo "FAIL (smoke-vrf-rotation): no stable (unchanged-committee) epoch found in [$((E_new + 1)), $((cur_now - 1))] — cannot test carry-forward"; exit 1; }
echo "  stable epoch E_s=$E_s (committee == committee($((E_s - 1)))=E_new), AFTER E_new=$E_new"

# The beacon stays LIVE across E_s: prev_randao non-zero + varying + node-agreed over
# a window inside E_s (E_new's key carried forward in memory still verifies the seed).
S_LO=$(epoch_first_block "$E_s")
S_HI=$(( S_LO + (EPOCH_LEN > 5 ? 4 : EPOCH_LEN - 1) ))
echo "  checking live-across-stable prev_randao window [$S_LO..$S_HI]"
wait_finalized_ge "$S_HI" 300 \
    || { echo "FAIL (smoke-vrf-rotation): chain did not reach the carry-forward window block $S_HI"; exit 1; }
wait_nodes_have "$S_HI" 180 \
    || { echo "FAIL (smoke-vrf-rotation): not all nodes reached the carry-forward window block $S_HI"; exit 1; }
assert_beacon_window "$S_LO" "$S_HI" "carry-forward E$E_s"

# ── assert the background tx load kept finalizing throughout ─────────────────
BEFORE=$(baseline_height)
sleep 6
AFTER=$(finalized_dec)
(( AFTER > BEFORE )) || { echo "FAIL (smoke-vrf-rotation): chain not finalizing under tx load ($AFTER <= $BEFORE)"; exit 1; }
echo "  chain still finalizing under tx load ($BEFORE → $AFTER)"

echo "OK (smoke-vrf-rotation): live per-epoch DKG beacon STAYED LIVE + NODE-AGREED across a committee change with EARLY-JOIN — \
baseline beacon THRESHOLD-ACTIVE from epoch 2 on the stable initial committee E$E0 (deterministic epoch-2 live DKG, prev_randao non-zero+varying+node-agreed); \
registering v5 changed the committee at E$E_new and the fresh DKG re-dealt; \
v5 EARLY-JOINED — it ran committee[E$E_new]'s DKG from its FOLLOWER phase during E_new-1 (logged 'live DKG: ceremony started/computed') and is a FULL beacon signer from block 1 of E$E_new (ACTIVE_LINE growing on the full committee ${PROBE_MEMBERS[*]} incl v5, prev_randao non-zero + byte-identical across all ${#NODES[@]} nodes at and past the E_new boundary); \
stable epoch E$E_s > E_new carried the key forward in memory (no fresh ceremony) while the threshold beacon stayed live + node-agreed (prev_randao window)"
