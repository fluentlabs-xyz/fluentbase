#!/usr/bin/env bash
# smoke-byzantine-vrf: a Byzantine boundary proposer that asserts a FORGED PK_E
# (in OrderBlock.beacon_outcome) at a change-epoch first block must NOT be able to
# finalize it, and the chain must stay live (an honest leader crosses the boundary
# with the real key).
#
# WHY the rotation stack: the forge fires ONLY on a CHANGE-epoch first block
# (BeaconVerify::is_change_epoch_first_block — false unless committee[E] !=
# committee[E-1]). The genesis stack has a STATIC committee → the predicate never
# fires → nothing to forge. So this case runs the PRODUCTION-PATH/rotation harness
# (runtime forge deploy + 6 validators) and drives committee changes (toggle v5 in
# and out of the committee), exactly like case-vrf-rotation.sh, then flags ONE
# Byzantine stayer.
#
# WHY a SINGLE Byzantine (not a quorum): under the realistic N3f1 bound (n=5 ⇒ f=1,
# quorum 4), a forge reaches the Stage-2 certify hook only if a quorum (≥4) of nodes
# notarize it. But flagging 4-of-5 byzantine leaves too few honest share-holders to
# both reject the forge AND propose the real boundary → liveness would stall. There is
# no single-run split that shows BOTH certify-Nullify AND liveness at f=1. So here a
# SINGLE byzantine forges: the 3 honest share-holders REJECT it at the "C" gate at
# verify (their real shares do not lie on the forged poly) → it cannot notarize →
# the forged PK_E never finalizes (SAFETY), and an honest share-holder crosses E_new
# with the real key (LIVENESS). The certify-hook Nullify path itself (which needs a
# colluding byzantine quorum) is proven by the gated UNIT tests in
# crates/dpos/consensus/src/beacon/{outcome.rs,certify.rs} — that is the authoritative
# proof of the certify hook; this docker case proves the realistic-fault end-to-end
# safety + liveness.
#
# NOTE (early-join): under the always-on beacon plane EVERY committee[E] member holds
# a share at E — JOINERS too (a newcomer deals its share from its follower phase in
# E-1), not just stayers. So "byzantine must be a stayer to hold a share to forge" is
# NO LONGER a constraint. We keep the byzantine as a permanent STAYER purely by CHOICE
# (it is the simplest rotation-proof way to give it a forge opportunity at every
# boundary while preserving f=1 and the repeated-boundary reliability fix below). A
# joiner-byzantine variant is possible but adds no new safety/liveness coverage — the
# C gate + certify hook reject the forge regardless of stayer-vs-joiner.
#
# ── WHY REPEATED committee changes (the reliability fix) ────────────────────
# The forge fires in `build_proposal` ONLY when the byzantine node is the LEADER of
# a change-epoch first block (the height whose OrderBlock.beacon_outcome carries the
# new PK_E). Commonware's leader for a view is `permutation_E[(epoch+view) % n]`
# (simplex RoundRobin elector; the permutation is a per-epoch sha256 shuffle keyed by
# the on-chain committee — elector_seed::epoch_leader_seed) — so on ANY ONE
# change-epoch boundary the byzantine view-1-leads it only ~1/n of the time, and when
# an honest node leads view 1 the boundary finalises immediately so the byzantine
# never gets a later view to forge on. A SINGLE committee change (v5 join → ONE E_new
# boundary, then the committee stabilises) therefore fires the forge only ~1/5 of the
# time — that is the bug this case used to hit (validator-2 never led E_new, the poll
# loop exhausted, FAIL).
# The fix drives a SEQUENCE of change-epoch boundaries by toggling v5 in and out of
# the committee: each membership flip is a fresh change-epoch first block, hence a
# fresh ~1/n forge opportunity (the per-epoch reshuffle makes these effectively
# INDEPENDENT draws). Over B boundaries P(byzantine never leads one) ≈ (1−1/n)^B, so
# ~16–18 boundaries drives it to ≈2% — a very-high-probability, BOUNDED budget, and
# the case FAILS LOUD with diagnostics if the forge still never fires. We keep the
# byzantine node permanently rotation-proof (boost its delegated stake) so it stays in
# committee[E] for EVERY E — hence it runs the per-epoch DKG and holds a real share to
# forge at every boundary — while v5 is the only member that enters/leaves. f=1 is
# preserved throughout: exactly ONE node is byzantine; v5's toggling never makes a
# second node byzantine.
#
# This case is built on the case-vrf-rotation.sh bring-up. It differs ONLY in:
#   (a) the --dpos cold-restart applies docker-compose.byzantine-vrf.yml, flagging
#       validator-2 with FLUENT_DPOS_BYZANTINE=forge-beacon-pk;
#   (b) it boosts the byzantine node's stake (rotation-proof) + drives REPEATED
#       committee flips so the byzantine reliably leads a change-epoch boundary;
#   (c) the post-rotation assertions (byzantine forged; forged key never on-chain;
#       honest C-rejection; chain still live).
# Requires the image built with the `dpos-devnet-byzantine` cargo feature (the smoke
# Dockerfile enables it). NEVER in prod.
#
# PREREQUISITES (host): docker, foundry (forge/cast), jq, a solidity-contracts
# checkout at $SOLIDITY_CONTRACTS_DIR.
set -euo pipefail
cd "$(dirname "$0")/.."

# The Byzantine stayer. MUST be a node that stays in committee[E_new] (so it leads
# boundary views and holds a real share to forge from) and MUST NOT be validator-0
# (the host-RPC node we keep honest for greps). We boost its stake below so it is
# permanently rotation-proof (never the member v5 displaces) across every flip.
BYZ_IDX="${BYZ_VRF_IDX:-2}"

# Run the production-path stack (own 6-node compose project), as case-vrf-rotation does.
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

# mixhash_at/in/of, is_zero_hash, log_count are shared in lib.sh (whose mixhash_in
# is GRACEFUL — coerces a down/restarting node's RPC to "null" instead of killing
# the script under set -e, which matters on this churny rotation+forge stack).

# Honest deriving nodes (all EXCEPT the Byzantine stayer): they must agree on
# prev_randao (the real seed each honest node verified and derived). The Byzantine
# node's own reth may diverge while it churns forged boundary views, so it is excluded.
HONEST_NODES=()
for svc in validator-0 validator-1 validator-2 validator-3 validator-4 validator-5 full-node; do
    [[ "$svc" == "validator-$BYZ_IDX" ]] || HONEST_NODES+=("$svc")
done

assert_honest_beacon_window() {
    local lo="$1" hi="$2" label="$3"
    local n svc mh agree
    local mixes=() vals=()
    for ((n = lo; n <= hi; n++)); do
        vals=()
        for svc in "${HONEST_NODES[@]}"; do
            mh=$(mixhash_of "$svc" "$n")
            if [[ "$mh" == "null" || -z "$mh" ]]; then
                echo "FAIL (smoke-byzantine-vrf): $label — honest $svc has no mixHash for block $n (node behind / RPC down)"
                docker compose logs --tail=80 "$svc"; exit 1
            fi
            if is_zero_hash "$mh"; then
                echo "FAIL (smoke-byzantine-vrf): $label — prev_randao is zero at block $n on honest $svc"
                docker compose logs --tail=80 "$svc"; exit 1
            fi
            vals+=("$mh")
        done
        agree=$(printf '%s\n' "${vals[@]}" | sort -u | wc -l)
        if (( agree != 1 )); then
            echo "FAIL (smoke-byzantine-vrf): $label — honest nodes disagree on prev_randao at block $n (the forge corrupted a node's derived seed):"
            paste -d' ' <(printf '%s\n' "${HONEST_NODES[@]}") <(printf '%s\n' "${vals[@]}") | sed 's/^/  /'
            exit 1
        fi
        mixes+=("${vals[0]}")
    done
    echo "  [$label] blocks [$lo..$hi]: non-zero + byte-identical prev_randao across all ${#HONEST_NODES[@]} honest nodes"
}

wait_nodes_have() {
    local block="$1" deadline=$(( SECONDS + ${2:-180} )) svc mh all
    while true; do
        all=1
        for svc in "${HONEST_NODES[@]}"; do
            mh=$(mixhash_of "$svc" "$block")
            if [[ "$mh" == "null" || -z "$mh" ]]; then all=0; break; fi
        done
        (( all == 1 )) && return 0
        if (( SECONDS >= deadline )); then
            echo "  [wait_nodes_have] timeout at block $block on the honest set"
            return 1
        fi
        sleep 2
    done
}

# ════════════════════════════════════════════════════════════════════════════
# Bring up the production-path stack + rotate the committee (register v5). This
# block mirrors case-vrf-rotation.sh phases A..cold-restart..trigger verbatim; the
# ONLY difference is the byzantine-vrf overlay applied at the --dpos cold-restart.
# ════════════════════════════════════════════════════════════════════════════
echo "== phase A: bare sequencer chain =="
docker compose up --build -d
pp_wait_converge 240 >/dev/null || { echo "FAIL (smoke-byzantine-vrf): bare chain did not converge"; docker compose logs --tail=120; exit 1; }
echo "  converged plain chain"

DEPLOYER_KEY="$(pp_owner_key 0)"
DEPLOYER_ADDR="$(pp_owner_addr 0)"

MNEMONIC="${FLUENT_DPOS_MNEMONIC:-test test test test test test test test test test test junk}"
SPAMMER_KEY="$(cast wallet private-key --mnemonic "$MNEMONIC" --mnemonic-index 6)"
SPAMMER_ADDR="$(cast wallet address --mnemonic "$MNEMONIC" --mnemonic-index 6)"
cast send "$SPAMMER_ADDR" --value 1000000000000000 \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" >/dev/null \
    || { echo "FAIL (smoke-byzantine-vrf): fund spammer account"; exit 1; }
pp_spammer_start "$SPAMMER_KEY" "$DEPLOYER_ADDR"
echo "  tx spammer started (pid $PP_SPAMMER_PID, from $SPAMMER_ADDR)"

echo "== runtime deploy: token + BLS verifier =="
TOKEN=$(forge_l2 forge create --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --json \
    "contracts/staking/mocks/MockBlendToken.sol:MockBlendToken" | jq -r '.deployedTo')
[[ "$TOKEN" == 0x* ]] || { echo "FAIL (smoke-byzantine-vrf): MockBlendToken deploy"; exit 1; }
VERIFIER=$(forge_l2 forge create --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --json \
    "contracts/libraries/BLS12381Verifier.sol:BLS12381Verifier" | jq -r '.deployedTo')
[[ "$VERIFIER" == 0x* ]] || { echo "FAIL (smoke-byzantine-vrf): BLS12381Verifier deploy"; exit 1; }
echo "  token=$TOKEN verifier=$VERIFIER"

pp_token_transfer "$TOKEN" "$(pp_owner_addr 5)" "10000000000000000000"
# The Byzantine node's owner also needs BLEND on hand to self-delegate (its stake
# boost below makes it permanently rotation-proof across the repeated flips).
pp_token_transfer "$TOKEN" "$(pp_owner_addr "$BYZ_IDX")" "10000000000000000000"
# A DEDICATED delegator (BIP44 index 7 — distinct from the spammer at 6 and from the
# custom-derived validator owner keys) drives the v5 in/out toggles. Using a
# third-party delegator (NOT v5's own owner) sidesteps the OwnerSelfStakeBelowMinimum
# guard on undelegate (StakingEconomics.sol::_undelegateFrom) so v5 can be toggled out
# cleanly, and its nonce never races the gov/deploy/spammer keys (it issues only the
# sequential toggle txns in the loop below). Fund it with native gas + BLEND.
TOGGLE_KEY="$(cast wallet private-key --mnemonic "$MNEMONIC" --mnemonic-index 7)"
TOGGLE_ADDR="$(cast wallet address --mnemonic "$MNEMONIC" --mnemonic-index 7)"
cast send "$TOGGLE_ADDR" --value 1000000000000000 \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" >/dev/null \
    || { echo "FAIL (smoke-byzantine-vrf): fund v5-toggle delegator"; exit 1; }
# Each delegate-IN pulls V5_IN_AMOUNT (2e18) of fresh BLEND; undelegate parks it in a
# withdrawal queue (not re-spendable without an explicit claim), so fund enough to
# cover all MAX_FLIPS in-flips without needing to claim back: 2e18 × 16 + headroom.
pp_token_transfer "$TOKEN" "$TOGGLE_ADDR" "40000000000000000000"

echo "== runtime deploy: staking cluster (DeployStaking) =="
NETWORK=local-dpos-smoke/l2 DEPLOYER="$DEPLOYER_ADDR" INITIAL_OWNER="$DEPLOYER_ADDR" \
  STAKING_TOKEN="$TOKEN" OUTPUT_PATH="$MANIFEST" \
  forge_l2 forge script scripts/deploy/DeployStaking.s.sol:DeployStaking \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --skip-simulation \
  || { echo "FAIL (smoke-byzantine-vrf): DeployStaking (EIP-170? see prereqs)"; exit 1; }

STAKING_RT=$(jq -r '.staking' "$MANIFEST")
CHAIN_CONFIG_RT=$(jq -r '.chain_config' "$MANIFEST")
GOV_ADDR=$(jq -r '.governance' "$MANIFEST")
LIVENESS_RT=$(jq -r '.liveness_slashing' "$MANIFEST")
for v in STAKING_RT CHAIN_CONFIG_RT GOV_ADDR LIVENESS_RT; do
    [[ "${!v}" == 0x* ]] || { echo "FAIL (smoke-byzantine-vrf): manifest missing $v"; cat "$MANIFEST"; exit 1; }
done
echo "  staking=$STAKING_RT chainConfig=$CHAIN_CONFIG_RT gov=$GOV_ADDR liveness=$LIVENESS_RT"

echo "== governance: setBlsVerifier (MUST precede setConsensusKeys) =="
pp_gov_action "$CHAIN_CONFIG_RT" \
    "$(cast calldata 'setBlsVerifier(address)' "$VERIFIER")" \
    "setBlsVerifier" || { echo "FAIL (smoke-byzantine-vrf): gov setBlsVerifier"; exit 1; }

echo "== setConsensusKeys for committee v0..v4 =="
for i in 0 1 2 3 4; do
    ck=$(pp_consensus_keys "$i")
    cast send "$STAKING_RT" "setConsensusKeys(address,bytes,bytes,bytes32)" \
        "$(jq -r '.validatorAddress' <<<"$ck")" "$(jq -r '.blsPubkeyUncompressed' <<<"$ck")" \
        "$(jq -r '.blsPoPUncompressed' <<<"$ck")" "$(jq -r '.peerPubkey' <<<"$ck")" \
        --rpc-url "$RPC" --private-key "$(pp_owner_key "$i")" >/dev/null \
        || { echo "FAIL (smoke-byzantine-vrf): setConsensusKeys v$i"; exit 1; }
done
echo "  consensus keys set for 5 validators"

HEAD=$(printf '%d' "$(check_external 8545 | cut -d'|' -f1)")
ACT=$(( ((HEAD / 64) + 2) * 64 ))
echo "== governance: setDposActivationBlock=$ACT (head=$HEAD) =="
pp_gov_action "$CHAIN_CONFIG_RT" \
    "$(cast calldata 'setDposActivationBlock(uint64)' "$ACT")" \
    "setDposActivationBlock" || { echo "FAIL (smoke-byzantine-vrf): gov setDposActivationBlock"; exit 1; }

echo "== wait for sequencer to clean-halt at activation block $ACT =="
wait_finalized_ge "$ACT" 400 || {
    echo "FAIL (smoke-byzantine-vrf): sequencer did not reach activation block $ACT (head=$(finalized_dec))"
    docker compose logs validator-0 --tail=80; exit 1
}
pp_wait_converge 180 >/dev/null \
    || { echo "FAIL (smoke-byzantine-vrf): followers did not align at the activation block"; docker compose logs --tail=120; exit 1; }
echo "  all nodes aligned at $ACT; proceeding to --dpos cold-restart (with the byzantine-vrf overlay)"

echo "== cold-restart: all validators into unified --dpos + BYZANTINE overlay on validator-$BYZ_IDX =="
ANCHOR=$(check_external 8545 | cut -d'|' -f1)
# Apply the byzantine-vrf overlay ON TOP of the production-path .dpos.yml so
# validator-$BYZ_IDX boots with FLUENT_DPOS_BYZANTINE=forge-beacon-pk.
export COMPOSE_FILE="docker-compose.production-path.yml:docker-compose.production-path.dpos.yml:docker-compose.byzantine-vrf.yml"
docker compose up -d --force-recreate "${PP_VALS[@]}" full-node \
    || { echo "FAIL (smoke-byzantine-vrf): cold-restart into --dpos (a validator exited)"; docker compose logs validator-0 --tail=80; exit 1; }
pp_wait_converge 180 "$ANCHOR" >/dev/null \
    || { echo "FAIL (smoke-byzantine-vrf): DPoS chain did not converge past anchor $ANCHOR"; docker compose logs --tail=200; exit 1; }
echo "  DPoS chain live past anchor $ANCHOR (one byzantine stayer present)"

EPOCH_LEN=$(printf '%d' "$(pp_chainconfig_call 'getEpochBlockInterval()(uint32)')")
(( EPOCH_LEN > 0 )) || { echo "FAIL (smoke-byzantine-vrf): getEpochBlockInterval()=0"; exit 1; }
epoch_first_block() { echo $(( ACT + $1 * EPOCH_LEN )); }

# ── stake setup: a clean v5 swing member + a rotation-proof byzantine + a settled
#    "out" committee so every flip is a CRISP, tie-break-free membership change ─────
# Initial stakes (l2.json): v0=5e18 (rotation-proof sequencer), v1..v4=1e18 each.
# We need: (i) the byzantine permanently in the top-5 (so it always runs the DKG and
# can forge at every boundary); (ii) v5 the ONLY member that enters/leaves, with NO
# tie-break ambiguity about whether v5 is in or out.
#
# v5's owner self-stake floor is the 1e18 minimum (can't go lower), so v5-OUT would
# TIE the other 1e18 validators (v1/v3/v4) at 1e18 — a tie-break gamble over which of
# the four 1e18 nodes fills the bottom committee slot. To make v5 the CLEAN swing, we
# lift the three non-byzantine 1e18 validators above v5's floor (+1e18 each — the
# minStakingAmount is 1e18, so a smaller bump would revert AmountTooLow):
#   v5 OUT  → v5 EffBal 1e18  < {v1,v3,v4}=2e18 ⇒ v5 is strictly the lowest ⇒ excluded.
#   v5 IN   → v5 EffBal 3e18  > 2e18            ⇒ v5 in, the lowest 2e18 node out.
# Either way exactly ONE membership swap (v5 ↔ a 2e18 node) ⇒ a genuine change-epoch
# boundary, and the byzantine (boosted to 4e18) is never the one swapped.
echo "== stake setup: boost byzantine (rotation-proof) + lift the v5-OUT committee floor =="
BYZ_KEY="$(pp_owner_key "$BYZ_IDX")" ; BYZ_ADDR="$(pp_owner_addr "$BYZ_IDX")"
byzl=$(tr 'A-F' 'a-f' <<<"$BYZ_ADDR")
# Byzantine: +3e18 self-delegate ⇒ 4e18, above the 2e18 floor tier and above v5's max 3e18.
cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "3000000000000000000" \
    --rpc-url "$RPC" --private-key "$BYZ_KEY" >/dev/null
cast send "$STAKING_RT" "delegate(address,uint256)" "$BYZ_ADDR" "3000000000000000000" \
    --rpc-url "$RPC" --private-key "$BYZ_KEY" >/dev/null \
    || { echo "FAIL (smoke-byzantine-vrf): self-delegate to byzantine validator-$BYZ_IDX"; exit 1; }
echo "  validator-$BYZ_IDX boosted (+3e18 ⇒ 4e18) — permanently in the committee top-5"
# Lift the other 1e18 validators (all initial v0..v4 except v0=5e18 and the byzantine)
# +1e18 each (= minStakingAmount; a smaller bump reverts) so the v5-OUT committee floor
# (2e18) is strictly above v5's 1e18 self-stake.
FLOOR_BUMP="1000000000000000000"   # 1e18 (== minStakingAmount)
for i in 1 2 3 4; do
    [[ "$i" == "$BYZ_IDX" ]] && continue
    fk="$(pp_owner_key "$i")"; fa="$(pp_owner_addr "$i")"
    cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "$FLOOR_BUMP" \
        --rpc-url "$RPC" --private-key "$fk" >/dev/null
    cast send "$STAKING_RT" "delegate(address,uint256)" "$fa" "$FLOOR_BUMP" \
        --rpc-url "$RPC" --private-key "$fk" >/dev/null \
        || { echo "FAIL (smoke-byzantine-vrf): floor-bump validator-$i"; exit 1; }
done
echo "  v1/v3/v4 lifted +1e18 each (v5-OUT committee floor = 2e18 > v5 floor 1e18)"

# ── register the external validator v5 (the member we toggle in and out) ────
echo "== register external validator v5 (the committee toggle target) =="
REG_FLOOR=$(check_external 8545 | cut -d'|' -f1)
V5_KEY="$(pp_owner_key 5)" ; V5_ADDR="$(pp_owner_addr 5)"
v5l=$(tr 'A-F' 'a-f' <<<"$V5_ADDR")
E0=$(pp_current_epoch)
GOT0=$(pp_committee "$E0")
cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "$STAKE_1E18" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null
cast send "$STAKING_RT" "registerValidator(address,uint16,uint256)" "$V5_ADDR" 0 "$STAKE_1E18" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL (smoke-byzantine-vrf): registerValidator v5"; exit 1; }
ck=$(pp_consensus_keys 5)
cast send "$STAKING_RT" "setConsensusKeys(address,bytes,bytes,bytes32)" \
    "$(jq -r '.validatorAddress' <<<"$ck")" "$(jq -r '.blsPubkeyUncompressed' <<<"$ck")" \
    "$(jq -r '.blsPoPUncompressed' <<<"$ck")" "$(jq -r '.peerPubkey' <<<"$ck")" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL (smoke-byzantine-vrf): setConsensusKeys v5"; exit 1; }
pp_gov_action "$STAKING_RT" \
    "$(cast calldata 'activateValidator(address)' "$V5_ADDR")" \
    "activateValidator-v5" || { echo "FAIL (smoke-byzantine-vrf): gov activateValidator v5"; exit 1; }
echo "  v5 registered + activated (not yet delegated into the committee)"

pp_wait_converge 180 "$REG_FLOOR" >/dev/null \
    || { echo "FAIL (smoke-byzantine-vrf): nodes lost alignment during registration"; docker compose logs "validator-$BYZ_IDX" --tail=80; exit 1; }

# v5 enters the committee when its EffBal outranks the v5-OUT floor (2e18). The
# toggle delegator adds/removes V5_IN_AMOUNT on top of v5's 1e18 owner self-stake:
#   IN  → EffBal 1e18 + 2e18 = 3e18  > 2e18 ⇒ v5 in (a 2e18 node out).
#   OUT → EffBal 1e18                < 2e18 ⇒ v5 out.
# A delegate change at epoch e is effective at EffBal(e+2) ⇒ surfaces at committee[e+3];
# undelegate is effective at EffBal(e+1) ⇒ surfaces at committee[e+2]
# (StakingEconomics.sol::_delegateTo/_undelegateFrom; WARMUP_DELAY=2,
# DPOS_ARCHITECTURE.md §6 snapshot chain). The loop self-syncs by polling the ACTUAL
# committee, so the exact lag does not matter.
V5_IN_AMOUNT="2000000000000000000"   # +2e18 via the dedicated delegator (TOGGLE_KEY)

# Approve a generous BLEND allowance once (from the dedicated delegator) so each
# toggle's delegate needs no re-approve.
cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "40000000000000000000" \
    --rpc-url "$RPC" --private-key "$TOGGLE_KEY" >/dev/null

toggle_v5() {
    # $1 = "in" | "out". Issues the delegation change (from the dedicated TOGGLE
    # delegator, NOT v5's owner — so undelegate never trips OwnerSelfStakeBelowMinimum)
    # that flips v5's committee membership.
    if [[ "$1" == "in" ]]; then
        cast send "$STAKING_RT" "delegate(address,uint256)" "$V5_ADDR" "$V5_IN_AMOUNT" \
            --rpc-url "$RPC" --private-key "$TOGGLE_KEY" >/dev/null \
            || { echo "FAIL (smoke-byzantine-vrf): delegate-in v5"; return 1; }
    else
        cast send "$STAKING_RT" "undelegate(address,uint256)" "$V5_ADDR" "$V5_IN_AMOUNT" \
            --rpc-url "$RPC" --private-key "$TOGGLE_KEY" >/dev/null \
            || { echo "FAIL (smoke-byzantine-vrf): undelegate-out v5"; return 1; }
    fi
}

# ════════════════════════════════════════════════════════════════════════════
# 1) The Byzantine node FORGES at a change-epoch boundary. The forge fires in
#    build_proposal ONLY when the byzantine LEADS the boundary view, and commonware's
#    view-1 leader is `permutation[(epoch+view) % n]` — so on ANY ONE boundary the
#    byzantine leads it only ~1/n of the time (this is the bug: a single rotation
#    gives ONE boundary, hit ~1/5). We therefore drive a SEQUENCE of change-epoch
#    boundaries by PIPELINING v5 in/out toggles — one toggle op roughly per epoch, so
#    the membership flips surface at CONSECUTIVE epochs (a delegate at epoch e
#    surfaces at committee[e+3], so toggles at e,e+1,e+2,… cascade to boundaries at
#    e+3,e+4,e+5,…). Each boundary is a fresh ~1/n forge opportunity (the elector
#    permutation reshuffles per epoch, so the draws are effectively independent); over
#    B boundaries P(byzantine never leads) ≈ (1−1/n)^B, so ~16–18 boundaries ⇒ ≈2%.
#    The byzantine is kept permanently in the committee (stake boost), so it runs the
#    DKG and holds a share to forge at EVERY boundary. We poll the forge-count
#    throughout and stop as soon as it fires; fail loud (with diagnostics) if a
#    generous toggle/wall-clock budget is exhausted. f=1 is preserved: exactly ONE
#    node forges; v5's toggling never makes a second node byzantine.
# ════════════════════════════════════════════════════════════════════════════
echo "== 1) pipeline committee flips until the byzantine forges a PK_E at a boundary =="
FORGE_LINE="BYZANTINE: proposing forged PK_E at boundary"
MAX_TOGGLES="${BYZ_VRF_MAX_TOGGLES:-18}"        # toggle ops to issue (one per epoch);
                                                # ≈18 ~1/n boundaries ⇒ ≈2% miss — the
                                                # byzantine usually forges well before
                                                # all 18 are issued.
# Wall-clock budget for the whole drive (toggles + drain). Generous because the 6-node
# stack runs well below 1 blk/s under load and a flip surfaces ~3 epochs after its
# toggle (WARMUP_DELAY). Tunable via env if the parent's docker run is slower/faster.
DRIVE_DEADLINE=$(( $(date +%s) + 60 * ${BYZ_VRF_BUDGET_MIN:-20} ))
E_new=""                                        # FIRST change-epoch (v5 first enters)
forged_count=0
want_in=1
prev_seen=""                                    # last committee set we observed
changes_seen=0                                  # change-epoch boundaries we counted
for ((t = 1; t <= MAX_TOGGLES; t++)); do
    (( $(date +%s) < DRIVE_DEADLINE )) || { echo "  [drive budget] wall-clock exhausted after $((t - 1)) toggles"; break; }
    forged_count=$(log_count "validator-$BYZ_IDX" "$FORGE_LINE"); forged_count=${forged_count:-0}
    (( forged_count >= 1 )) && break

    desc=$([[ "$want_in" == 1 ]] && echo "IN" || echo "OUT")
    echo "  [toggle $t/$MAX_TOGGLES] v5 $desc"
    toggle_v5 "$([[ "$want_in" == 1 ]] && echo in || echo out)" \
        || { docker compose logs "validator-$BYZ_IDX" --tail=60; exit 1; }
    want_in=$(( 1 - want_in ))

    # Advance ~one epoch before the next toggle (so flips surface at consecutive
    # epochs), checking the forge-count frequently meanwhile. Within this window, also
    # observe the committee timeline: each epoch whose committee SET differs from the
    # previous is a change-epoch boundary (a fresh forge opportunity), and the first
    # one that ADDS v5 is recorded as E_new (the byzantine MUST be a stayer there).
    epoch_window_start=$(pp_current_epoch)
    wstop=$(( $(date +%s) + 60 * 3 ))           # at most ~3 min per toggle's epoch wait
    while (( $(date +%s) < wstop && $(date +%s) < DRIVE_DEADLINE )); do
        forged_count=$(log_count "validator-$BYZ_IDX" "$FORGE_LINE"); forged_count=${forged_count:-0}
        (( forged_count >= 1 )) && break
        nowE=$(pp_current_epoch)
        cur_set=$(pp_committee "$nowE")
        if [[ -n "$cur_set" && "$cur_set" != "$prev_seen" ]]; then
            if [[ -n "$prev_seen" ]]; then
                changes_seen=$(( changes_seen + 1 ))
                echo "    committee changed at epoch $nowE (#$changes_seen): [$cur_set]"
            fi
            # First change that brings v5 IN → anchor E_new; assert byzantine stays.
            if [[ -z "$E_new" && " $cur_set " == *" $v5l "* && "$cur_set" != "$GOT0" ]]; then
                E_new="$nowE"
                echo "    → E_new=$E_new (v5 first entered the committee; E0=$E0)"
                if [[ " $cur_set " != *" $byzl "* ]]; then
                    echo "FAIL (smoke-byzantine-vrf): the Byzantine node validator-$BYZ_IDX ($byzl) is NOT in committee[E_new=$E_new] \
despite the rotation-proof stake boost — raise the byzantine boost above v5's V5_IN_AMOUNT."
                    echo "  committee[E_new=$E_new]: $cur_set"
                    exit 1
                fi
            fi
            prev_seen="$cur_set"
        fi
        (( nowE >= epoch_window_start + 1 )) && break   # advanced one epoch → next toggle
        sleep 3
    done
done

# DRAIN: the last few toggles' committee flips surface ~3 epochs AFTER they were
# issued (WARMUP_DELAY), so boundaries are still in-flight when the toggle loop ends.
# Keep polling the forge-count (and the committee timeline) until the in-flight
# boundaries have surfaced or the wall-clock budget is exhausted — otherwise a forge
# the byzantine is about to lead would read as a false FAIL.
if (( forged_count < 1 )); then
    echo "  [drain] toggles issued; waiting for in-flight change boundaries to surface…"
    while (( $(date +%s) < DRIVE_DEADLINE )); do
        forged_count=$(log_count "validator-$BYZ_IDX" "$FORGE_LINE"); forged_count=${forged_count:-0}
        (( forged_count >= 1 )) && break
        nowE=$(pp_current_epoch)
        cur_set=$(pp_committee "$nowE")
        if [[ -n "$cur_set" && "$cur_set" != "$prev_seen" ]]; then
            [[ -n "$prev_seen" ]] && { changes_seen=$(( changes_seen + 1 )); echo "    committee changed at epoch $nowE (#$changes_seen): [$cur_set]"; }
            if [[ -z "$E_new" && " $cur_set " == *" $v5l "* && "$cur_set" != "$GOT0" ]]; then
                E_new="$nowE"; echo "    → E_new=$E_new (v5 first entered the committee; E0=$E0)"
                [[ " $cur_set " == *" $byzl "* ]] || { echo "FAIL (smoke-byzantine-vrf): byzantine validator-$BYZ_IDX ($byzl) NOT in committee[E_new=$E_new] (boost too small)"; exit 1; }
            fi
            prev_seen="$cur_set"
        fi
        sleep 4
    done
fi

if (( forged_count < 1 )); then
    echo "FAIL (smoke-byzantine-vrf): validator-$BYZ_IDX never logged '$FORGE_LINE' across $changes_seen change-epoch boundaries — \
the forge never fired. Possible causes: it never led ANY change-epoch boundary (raise BYZ_VRF_MAX_TOGGLES / BYZ_VRF_BUDGET_MIN), \
too few committee changes actually surfaced (EffBal/warmup timing — see the toggle log above), \
the image was NOT built with --features dpos-devnet-byzantine, or the byzantine fell out of the committee (boost too small)."
    echo "  --- byzantine validator-$BYZ_IDX recent log (byzantine/beacon/boundary/change-epoch) ---"
    docker compose logs "validator-$BYZ_IDX" --tail=300 2>/dev/null | grep -iE "byzantine|beacon|boundary|change-epoch|is_change_epoch" | tail -50
    exit 1
fi
echo "  validator-$BYZ_IDX forged a PK_E at a change-epoch boundary x$forged_count (across $changes_seen observed committee changes)"

# Pin the SAFETY/relive assertions to the boundary the byzantine ACTUALLY forged at
# (its forge warn! carries the structured `epoch=<E>` field). Use the LAST such epoch
# (the most recent forged boundary, certain to be finalised by the time we assert).
# Fall back to E_new (the first observed v5-entry change boundary, where the byzantine
# is also a stayer) if the field can't be parsed — both are change-epoch boundaries
# where an honest leader commits the real key, so either is a valid safety anchor.
FORGE_EPOCH=$(docker compose logs "validator-$BYZ_IDX" 2>/dev/null \
    | grep "$FORGE_LINE" | grep -oE 'epoch=[0-9]+' | tail -1 | cut -d= -f2)
[[ "$FORGE_EPOCH" =~ ^[0-9]+$ ]] || FORGE_EPOCH="$E_new"
[[ "$FORGE_EPOCH" =~ ^[0-9]+$ ]] || { echo "FAIL (smoke-byzantine-vrf): forge fired but could not determine the forged epoch \
(neither the log's epoch= field nor a recorded v5-entry change boundary) — observability gap; raise the committee-poll cadence"; exit 1; }
echo "  forged boundary epoch = $FORGE_EPOCH (anchoring the SAFETY window there)"

# ════════════════════════════════════════════════════════════════════════════
# 2) SAFETY: the forged PK_E never finalized. The beacon has no on-chain mirror,
#    so the seed rides the consensus cert and prev_randao = H(seed). The safety
#    proof: at and just past the forged boundary, ALL honest nodes derive a
#    non-zero, byte-IDENTICAL prev_randao. If a forged seed had finalized, the
#    honest set's seeds would fail-verify → a digest-fallback / zero / DIVERGENT
#    mixHash, so this window MUST fail on a finalized forge — it is meaningful.
# ════════════════════════════════════════════════════════════════════════════
echo "== 2) SAFETY: the forged PK_E never finalized (honest nodes agree on prev_randao) =="
FORGE_BOUNDARY=$(epoch_first_block "$FORGE_EPOCH")
wait_finalized_ge "$FORGE_BOUNDARY" 900 \
    || { echo "FAIL (smoke-byzantine-vrf): chain did not reach the forged-boundary block $FORGE_BOUNDARY (epoch $FORGE_EPOCH)"; docker compose logs validator-0 --tail=80; exit 1; }
# The honest nodes derive prev_randao at and just past the forged boundary and AGREE —
# i.e. every honest node verified the SAME real threshold seed (a forged seed that had
# finalized would fail-verify → a digest-fallback/zero/divergent mixHash on the honest set).
RELIVE_HI=$(( FORGE_BOUNDARY + 4 ))
wait_finalized_ge "$RELIVE_HI" 180 \
    || { echo "FAIL (smoke-byzantine-vrf): chain did not advance past the forged boundary ($RELIVE_HI)"; exit 1; }
wait_nodes_have "$RELIVE_HI" 180 \
    || { echo "FAIL (smoke-byzantine-vrf): not all honest nodes reached the window block $RELIVE_HI"; exit 1; }
assert_honest_beacon_window "$FORGE_BOUNDARY" "$RELIVE_HI" "post-forge E$FORGE_EPOCH"
echo "  honest nodes derive a non-zero, byte-identical prev_randao across the forged boundary — the forge corrupted neither the finalized seed nor the derived randomness"

# 3) The honest share-holders REJECTED the forged boundary at the "C" gate at verify
#    (the f=1 safety mechanism: the forge could not pass C → could not notarize →
#    never reached certify). Grep an honest share-holder (validator-0, a stayer).
echo "== 3) honest C-gate rejected the forged boundary (the f=1 safety mechanism) =="
CFAIL_LINE="C share-on-poly FAILED for asserted outcome"
cfail=$(log_count validator-0 "$CFAIL_LINE"); cfail=${cfail:-0}
if (( cfail < 1 )); then
    echo "FAIL (smoke-byzantine-vrf): validator-0 (honest share-holder) never logged '$CFAIL_LINE' — it did not reject the forged boundary at verify (did the byzantine never get to propose to it, or is the forge not differing from the real key?)"
    docker compose logs validator-0 --tail=120 | grep -iE "C share-on-poly|beacon|boundary" | tail -40
    exit 1
fi
echo "  validator-0 rejected the forged boundary at the C gate x$cfail (forged PK_E could not notarize → never reached certify)"

# 4) LIVENESS: the chain crossed the change boundaries (an honest leader carried the
#    real boundary each time) and keeps finalizing under tx load.
echo "== 4) LIVENESS: the chain crossed the change boundaries and keeps finalizing =="
BEFORE=$(baseline_height)
sleep 6
AFTER=$(finalized_dec)
(( AFTER > BEFORE )) || { echo "FAIL (smoke-byzantine-vrf): chain not finalizing past the boundary under tx load ($AFTER <= $BEFORE) — a byzantine stayer wedged liveness"; docker compose logs --tail=120; exit 1; }
echo "  chain still finalizing under tx load ($BEFORE → $AFTER)"

echo "OK (smoke-byzantine-vrf): a Byzantine stayer (validator-$BYZ_IDX) FORGED a PK_E at a change-epoch boundary (forged epoch E$FORGE_EPOCH, x$forged_count, reached by toggling v5 in/out of the committee — repeated change-epoch boundaries — until the byzantine led one); \
the honest share-holders REJECTED it at the C gate (x$cfail) so it could not notarize → the forged PK_E NEVER finalized; \
all ${#HONEST_NODES[@]} honest nodes derive a non-zero, byte-identical prev_randao across the forged boundary (the SAME real threshold seed — a finalized forge would have diverged the honest set); \
the chain crossed the boundary via an honest leader and kept finalizing under tx load ($BEFORE → $AFTER). \
SAFETY (forged PK_E never finalized → honest prev_randao agreed) + LIVENESS (honest beacon crossed + sustained). \
The certify-hook Nullify path is proven by the gated unit tests (beacon/outcome.rs, beacon/certify.rs)."
