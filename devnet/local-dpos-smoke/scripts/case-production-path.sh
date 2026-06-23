#!/usr/bin/env bash
# smoke-production-path: the full DPoS production lifecycle on a chain where the
# staking cluster is deployed at RUNTIME via forge (not baked into genesis),
# mirroring prod:
#   plain sequencer → runtime-deploy token+verifier+cluster → bootstrap 5-validator
#   committee → cold-restart ALL validators into unified --dpos
#   (--dpos.follower-upstream) → register the EXTERNAL 6th validator via
#   governance + delegate while its supervisor follows in-process → v5
#   AUTO-promotes at its first committee epoch boundary (no operator action) →
#   committee rotates (displaced validator auto-demotes and keeps following) →
#   eject one validator by liveness → all under a background value-transfer load.
#
# First-of-its-kind end-to-end (live rotation + dynamic join). Per the brief, a
# failure here is a real-bug finding, not a test defect.
#
# PREREQUISITES (host): docker, foundry (forge/cast), jq, a solidity-contracts
# checkout at $SOLIDITY_CONTRACTS_DIR.
set -euo pipefail
cd "$(dirname "$0")/.."

# Run the production-path stack (own 6-node compose project), not the default
# genesis-baked one. lib.sh's `docker compose` helpers inherit this.
export COMPOSE_FILE="docker-compose.production-path.yml"
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

SOLIDITY_CONTRACTS_DIR="${SOLIDITY_CONTRACTS_DIR:-../../../solidity-contracts}"
# DeployStaking's `vm.writeJson` runs under solidity-contracts' foundry.toml,
# whose fs_permissions only allow `./deployments` — so the manifest MUST land
# inside that dir (absolute path, resolved once) rather than the smoke dir.
MANIFEST="$(cd "$SOLIDITY_CONTRACTS_DIR" && pwd)/deployments/runtime-deployment.json"

# Runtime-deployed addresses, populated after the forge deploy.
STAKING_RT="" ; CHAIN_CONFIG_RT="" ; GOV_ADDR="" ; LIVENESS_RT=""
STAKE_1E18="1000000000000000000"

cleanup() { pp_spammer_stop; rm -f "$MANIFEST"; tear_down; }
trap cleanup EXIT

forge_l2() { # forge_l2 <extra forge args...>
    ( cd "$SOLIDITY_CONTRACTS_DIR" && "$@" )
}

# ── phase A: plain 6-node sequencer chain (no staking in genesis) ───────────
echo "== phase A: bare sequencer chain =="
docker compose up --build -d
pp_wait_converge 120 >/dev/null || { echo "FAIL: bare chain did not converge"; docker compose logs --tail=120; exit 1; }
echo "  converged plain chain"

DEPLOYER_KEY="$(pp_owner_key 0)"
DEPLOYER_ADDR="$(pp_owner_addr 0)"

# Dedicated spammer account: a standard BIP44 index-6 key from the same mnemonic,
# unrelated to the genesis validator owner keys (custom derive_32) and to the
# deployer (owner 0). Validators 0-5 all send real deploy/registration txns, so
# the spammer must NOT reuse any of them or its background nonce races theirs
# (`replacement transaction underpriced`). Fund it once from the deployer (native
# gas token) before the load starts — this single tx predates any concurrency.
# Fund only a small slice: the deployer holds exactly 1e18 native and needs most
# of it for the cluster deploys; value-1 sends cost ~5e11 gas each, so 1e15 is
# ~2000 txns of headroom (far beyond the run length).
MNEMONIC="${FLUENT_DPOS_MNEMONIC:-test test test test test test test test test test test junk}"
SPAMMER_KEY="$(cast wallet private-key --mnemonic "$MNEMONIC" --mnemonic-index 6)"
SPAMMER_ADDR="$(cast wallet address --mnemonic "$MNEMONIC" --mnemonic-index 6)"
cast send "$SPAMMER_ADDR" --value 1000000000000000 \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" >/dev/null \
    || { echo "FAIL: fund spammer account"; exit 1; }
pp_spammer_start "$SPAMMER_KEY" "$DEPLOYER_ADDR"
echo "  tx spammer started (pid $PP_SPAMMER_PID, from $SPAMMER_ADDR)"

# ── deploy MockBlendToken + BLS verifier (forge create, host side) ──────────
echo "== runtime deploy: token + BLS verifier =="
TOKEN=$(forge_l2 forge create --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --json \
    "contracts/staking/mocks/MockBlendToken.sol:MockBlendToken" | jq -r '.deployedTo')
[[ "$TOKEN" == 0x* ]] || { echo "FAIL: MockBlendToken deploy"; exit 1; }
VERIFIER=$(forge_l2 forge create --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --json \
    "contracts/libraries/BLS12381Verifier.sol:BLS12381Verifier" | jq -r '.deployedTo')
[[ "$VERIFIER" == 0x* ]] || { echo "FAIL: BLS12381Verifier deploy"; exit 1; }
echo "  token=$TOKEN verifier=$VERIFIER"

# Deployer (v0) holds all BLEND; fund the external joiner (v5) owner so it can
# registerValidator + delegate later (1e18 register + 2e18 delegate + headroom).
pp_token_transfer "$TOKEN" "$(pp_owner_addr 5)" "10000000000000000000"

# ── deploy the staking cluster (production forge script) ────────────────────
echo "== runtime deploy: staking cluster (DeployStaking) =="
NETWORK=local-dpos-smoke/l2 DEPLOYER="$DEPLOYER_ADDR" INITIAL_OWNER="$DEPLOYER_ADDR" \
  STAKING_TOKEN="$TOKEN" OUTPUT_PATH="$MANIFEST" \
  forge_l2 forge script scripts/deploy/DeployStaking.s.sol:DeployStaking \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --skip-simulation \
  || { echo "FAIL: DeployStaking (EIP-170? see prereqs)"; exit 1; }

STAKING_RT=$(jq -r '.staking' "$MANIFEST")
CHAIN_CONFIG_RT=$(jq -r '.chain_config' "$MANIFEST")
GOV_ADDR=$(jq -r '.governance' "$MANIFEST")
LIVENESS_RT=$(jq -r '.liveness_slashing' "$MANIFEST")
for v in STAKING_RT CHAIN_CONFIG_RT GOV_ADDR LIVENESS_RT; do
    [[ "${!v}" == 0x* ]] || { echo "FAIL: manifest missing $v"; cat "$MANIFEST"; exit 1; }
done
echo "  staking=$STAKING_RT chainConfig=$CHAIN_CONFIG_RT gov=$GOV_ADDR liveness=$LIVENESS_RT"

# ── wire BLS verifier via governance BEFORE setConsensusKeys ────────────────
# setConsensusKeys verifies the PoP against ChainConfig.getBlsVerifier(); calling
# it before the verifier is wired reverts BlsVerifierNotConfigured.
echo "== governance: setBlsVerifier (MUST precede setConsensusKeys) =="
pp_gov_action "$CHAIN_CONFIG_RT" \
    "$(cast calldata 'setBlsVerifier(address)' "$VERIFIER")" \
    "setBlsVerifier" || { echo "FAIL: gov setBlsVerifier"; exit 1; }

# ── setConsensusKeys for the initial committee (v0..v4) ─────────────────────
echo "== setConsensusKeys for committee v0..v4 =="
for i in 0 1 2 3 4; do
    ck=$(pp_consensus_keys "$i")
    bls_pub=$(jq -r '.blsPubkeyUncompressed' <<<"$ck")
    bls_pop=$(jq -r '.blsPoPUncompressed' <<<"$ck")
    peer=$(jq -r '.peerPubkey' <<<"$ck")
    addr=$(jq -r '.validatorAddress' <<<"$ck")
    cast send "$STAKING_RT" "setConsensusKeys(address,bytes,bytes,bytes32)" \
        "$addr" "$bls_pub" "$bls_pop" "$peer" \
        --rpc-url "$RPC" --private-key "$(pp_owner_key "$i")" >/dev/null \
        || { echo "FAIL: setConsensusKeys v$i"; exit 1; }
done
echo "  consensus keys set for 5 validators"

# ── set the DPoS activation block via governance (64-aligned, ahead) ────────
HEAD=$(printf '%d' "$(check_external 8545 | cut -d'|' -f1)")
ACT=$(( ((HEAD / 64) + 2) * 64 ))   # next-but-one 64-aligned boundary, ample lead
echo "== governance: setDposActivationBlock=$ACT (head=$HEAD) =="
pp_gov_action "$CHAIN_CONFIG_RT" \
    "$(cast calldata 'setDposActivationBlock(uint64)' "$ACT")" \
    "setDposActivationBlock" || { echo "FAIL: gov setDposActivationBlock"; exit 1; }

# ── assert: pre-written staking-reader.json == deployed manifest ────────────
# genesis-init predicted the CREATE addresses from deployer nonces (see the
# --staking-reader-create-nonces comment in docker-compose.production-path.yml);
# any drift in the driver tx order or the linked-library set lands here.
echo "== assert pre-written staking-reader.json matches the deploy manifest =="
PRE=$(docker compose exec -T validator-0 cat /runtime/staking-reader.json)
for pair in "staking_address:$STAKING_RT" \
            "chain_config_address:$CHAIN_CONFIG_RT" \
            "liveness_slashing_address:$LIVENESS_RT"; do
    k=${pair%%:*} want=$(tr 'A-F' 'a-f' <<<"${pair#*:}")
    got=$(jq -r ".$k" <<<"$PRE" | tr 'A-F' 'a-f')
    [[ "$got" == "$want" ]] || { echo "FAIL: pre-written $k=$got != deployed $want \
(deployer nonce drift — update --staking-reader-create-nonces in \
docker-compose.production-path.yml)"; exit 1; }
done
echo "  pre-written config matches manifest ✓"

# ── wait for the SEQUENCER (validator-0) to clean-halt at $ACT ───────────────
# No restart: every node has carried --dpos.staking-config since first boot, so
# validator-0's dynamic activation gate (per-tick re-read, launcher.rs) picks up
# setDposActivationBlock on-chain and halts Tempo production at exactly $ACT.
# DPoS refuses to anchor unless reth's head == dposActivationBlock exactly (no
# orphaned Tempo tail past it — dpos.rs:523-531). The followers ride the
# uninterrupted WS stream to the same height.
echo "== wait for sequencer (validator-0) to clean-halt at activation block $ACT =="
wait_finalized_ge "$ACT" 200 || {
    echo "FAIL: sequencer did not reach activation block $ACT (head=$(finalized_dec))"
    docker compose logs validator-0 --tail=80; exit 1
}
echo "  sequencer holds block $ACT"

# All 7 nodes (followers + full-node) must align at the halted head before the
# --dpos cold-restart — dpos.rs:523-531 requires head == anchor on every
# validator, and a lagging follower would race ANCHOR_WAIT.
pp_wait_converge 90 >/dev/null \
    || { echo "FAIL: followers did not align at the activation block"; docker compose logs --tail=120; exit 1; }
echo "  all nodes aligned at $ACT; proceeding to --dpos cold-restart"

# ── cold-restart all 6 validators into unified --dpos ───────────────────────
# v0-4 are in committee(0) → their supervisors enter the signer phase directly
# (legacy FreshMigration discriminator); v5 is not → follower substrate.
echo "== cold-restart: all validators into unified --dpos =="
ANCHOR=$(check_external 8545 | cut -d'|' -f1)
export COMPOSE_FILE="docker-compose.production-path.yml:docker-compose.production-path.dpos.yml"
docker compose up -d --force-recreate "${PP_VALS[@]}" \
    || { echo "FAIL: cold-restart into --dpos (a validator exited)"; docker compose logs validator-0 --tail=80; exit 1; }
pp_wait_converge 180 "$ANCHOR" >/dev/null \
    || { echo "FAIL: DPoS chain did not converge past anchor $ANCHOR"; docker compose logs --tail=200; exit 1; }
echo "  DPoS chain live past anchor $ANCHOR"

# ── assert: committee == our initial 5 (v0..v4) ─────────────────────────────
echo "  [cm-diag] head=$(check_external 8545 | cut -d'|' -f1) epoch=$(pp_current_epoch)"
echo "  [cm-diag] nextEpochToCommit=$(pp_staking_call 'nextEpochToCommit()(uint64)' 2>&1 | awk '{print $1}') committeeSelectionEpoch=$(pp_staking_call 'committeeSelectionEpoch()(uint64)' 2>&1 | awk '{print $1}')"
for _e in 0 1 2 3; do echo "  [cm-diag] committee($_e)=[$(pp_committee "$_e")]"; done
E=$(pp_current_epoch)
GOT=$(pp_committee "$E")
EXPECT=$(for i in 0 1 2 3 4; do pp_owner_addr "$i"; done | tr 'A-F' 'a-f' | sort | paste -sd' ' -)
[[ "$GOT" == "$EXPECT" ]] || { echo "FAIL: committee != initial 5 (got [$GOT] want [$EXPECT])"; exit 1; }
echo "  committee(epoch $E) == initial 5 ✓"

# ── register the EXTERNAL 6th validator (v5) via governance + delegate ──────
# v5 runs the same unified `--dpos` as everyone: its supervisor keeps it on
# the in-process cert-follow substrate (key not in any committee yet) while
# the registration lands on-chain — no operator choreography at any point.
echo "== register external validator v5 (unified follower substrate meanwhile) =="
REG_FLOOR=$(check_external 8545 | cut -d'|' -f1)
V5_KEY="$(pp_owner_key 5)" ; V5_ADDR="$(pp_owner_addr 5)"
cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "$STAKE_1E18" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null
cast send "$STAKING_RT" "registerValidator(address,uint16,uint256)" "$V5_ADDR" 0 "$STAKE_1E18" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL: registerValidator v5"; exit 1; }
ck=$(pp_consensus_keys 5)
cast send "$STAKING_RT" "setConsensusKeys(address,bytes,bytes,bytes32)" \
    "$(jq -r '.validatorAddress' <<<"$ck")" "$(jq -r '.blsPubkeyUncompressed' <<<"$ck")" \
    "$(jq -r '.blsPoPUncompressed' <<<"$ck")" "$(jq -r '.peerPubkey' <<<"$ck")" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL: setConsensusKeys v5"; exit 1; }
pp_gov_action "$STAKING_RT" \
    "$(cast calldata 'activateValidator(address)' "$V5_ADDR")" \
    "activateValidator-v5" || { echo "FAIL: gov activateValidator v5"; exit 1; }
# Delegate >1e18 so v5 EffBal outranks an initial validator. Effective in
# EffBal(E+2); committee[N] reads EffBal(N-1) ⇒ committee entry at E+3.
cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "2000000000000000000" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null
cast send "$STAKING_RT" "delegate(address,uint256)" "$V5_ADDR" "2000000000000000000" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL: delegate v5"; exit 1; }
echo "  v5 registered + activated + delegated"

# v5's RPC must keep tracking v0 through the registration load — the unified
# follower substrate is load-bearing for the join.
pp_wait_converge 90 "$REG_FLOOR" >/dev/null \
    || { echo "FAIL: nodes (v5 on follower substrate) lost alignment during registration"; docker compose logs validator-5 --tail=80; exit 1; }
echo "  v5 follower substrate tracked the chain through registration ✓"

# ── AUTO-promotion: no operator action — just observe ───────────────────────
# committee[N] is committed one boundary early; once v5 shows up in the
# ahead-committed set, its own supervisor must stop the follower lap at the
# boundary−1 and promote in-process.
echo "== wait for v5 in the ahead-committed committee (auto-promotion) =="
v5l=$(tr 'A-F' 'a-f' <<<"$V5_ADDR")
JOIN_E=""
_deadline=$(( $(date +%s) + 300 ))
while (( $(date +%s) < _deadline )); do
    E=$(pp_current_epoch)
    [[ " $(pp_committee $((E + 1))) " == *" $v5l "* ]] && { JOIN_E=$((E + 1)); break; }
    sleep 2
done
[[ -n "$JOIN_E" ]] || { echo "FAIL: v5 never appeared in an ahead-committed committee"; exit 1; }
echo "  v5 in committee(epoch $JOIN_E), current epoch $((JOIN_E - 1)); expecting in-process promotion"

# ── assert v5 enters consensus at its first committee boundary ──────────────
# Convergence strictly past the boundary block with v5 in the reading set
# proves the supervisor promoted at the boundary and v5 follows as a signer.
EPOCH_LEN=$(printf '%d' "$(pp_chainconfig_call 'getEpochBlockInterval()(uint32)')")
JOIN_BOUNDARY=$(( ACT + JOIN_E * EPOCH_LEN ))
# The deadline must cover the FULL external-join flow, which the live-clock DKG fix
# makes actually happen: v5 finishes Phase-1 catch-up, committee[E] deals its
# epoch-E share (a multi-block ceremony that only seals near the E-1/E boundary),
# v5 promotes at the boundary, then all 7 nodes align past it — ~2-3 min from JOIN
# detection (which fires early, in epoch E-1). 240s was calibrated for the pre-fix
# path (v5 never promoted) and is too tight for the real ceremony + join.
pp_wait_converge 420 "$(printf '0x%x' "$JOIN_BOUNDARY")" >/dev/null \
    || { echo "FAIL: v5 did not enter consensus past its committee boundary (block $JOIN_BOUNDARY)"
         echo "--- v5 commonware peers (buffered_peer_total series) ---"
         { docker compose exec -T validator-5 sh -c 'wget -qO- http://localhost:9100 2>/dev/null' \
             | { grep -m20 -E "buffered_peer_total|peers_blocked" || true; }; } || true
         echo "--- v0 commonware peers ---"
         { curl -s http://localhost:19100 | { grep -m20 -E "buffered_peer_total|peers_blocked" || true; }; } || true
         V5D=$(mktemp); docker compose logs validator-5 >"$V5D" 2>&1 || true
         echo "--- v5 log (filtered) ---"
         { grep -vE "Block added to canonical|Regular root task|Forkchoice updated|Canonical chain committed|Received forkchoice|Status connected" "$V5D" || true; } | tail -60
         V0D=$(mktemp); docker compose logs validator-0 >"$V0D" 2>&1 || true
         echo "--- v0 log (v5-related) ---"
         { grep -iE "discovery|handshake|fac42278|dial|listener" "$V0D" || true; } | tail -40
         rm -f "$V5D" "$V0D"
         exit 1; }
echo "  v5 entered consensus: all 6 validators + full-node aligned past block $JOIN_BOUNDARY ✓"

# The promotion must be the in-process Verifier→Signer transition (not a restart):
# logs go through a file — `docker logs | grep -q` SIGPIPEs under pipefail.
V5_LOG=$(mktemp)
docker compose logs validator-5 >"$V5_LOG" 2>&1 || true
grep -q "promoted to Signer in-process" "$V5_LOG" \
    || { echo "FAIL: v5 log has no in-process promotion (joined some other way?)"; rm -f "$V5_LOG"; exit 1; }
rm -f "$V5_LOG"
echo "  v5 promoted in-process (Verifier→Signer) ✓"

E=$(pp_current_epoch)
GOT=$(pp_committee "$E")
[[ " $GOT " == *" $v5l "* ]] || { echo "FAIL: v5 not in committee(epoch $E): [$GOT]"; exit 1; }
[[ "$(wc -w <<<"$GOT")" == "$PP_COMMITTEE_SIZE" ]] || { echo "FAIL: committee size != $PP_COMMITTEE_SIZE"; exit 1; }
echo "  committee rotated: v5 entered top-5, lowest dropped ✓"

# ── DEMOTION: the validator v5 displaced must keep following ─────────────────
# The displaced one demotes to verify-only in-process (rotation-out →
# reconcile_roles drops its signer engine) and keeps following the chain via its
# cert-inlet (its `--dpos.follower-upstream` is set), instead of wedging as a
# silent verifier. Identify it as the initial-5 member missing from the rotated
# committee.
DISPLACED_IDX=""
for i in 0 1 2 3 4; do
    a=$(tr 'A-F' 'a-f' <<<"$(pp_owner_addr "$i")")
    [[ " $GOT " != *" $a "* ]] && { DISPLACED_IDX="$i"; break; }
done
[[ -n "$DISPLACED_IDX" ]] || { echo "FAIL: could not identify the displaced validator"; exit 1; }
[[ "$DISPLACED_IDX" != 0 ]] || { echo "FAIL: rotation displaced validator-0 (harness RPC) — tie-break drift"; exit 1; }
DEM_PRE=$(check_node docker compose exec -T "validator-$DISPLACED_IDX" | cut -d'|' -f1)
sleep 8
DEM_POST=$(check_node docker compose exec -T "validator-$DISPLACED_IDX" | cut -d'|' -f1)
{ [[ "$DEM_PRE" != "null" && "$DEM_POST" != "null" ]] \
    && (( $(printf '%d' "$DEM_POST") > $(printf '%d' "$DEM_PRE") )); } \
    || { echo "FAIL: displaced validator-$DISPLACED_IDX stopped following after demotion ($DEM_PRE → $DEM_POST)"; docker compose logs "validator-$DISPLACED_IDX" --tail=80; exit 1; }
echo "  displaced validator-$DISPLACED_IDX demoted in-process and keeps following ($DEM_PRE → $DEM_POST) ✓"

# ── liveness ejection: stop one committee validator for ≥50 blocks/1 epoch ──
# Stop at an epoch START: _missCounter resets each boundary, MISS_THRESHOLD=50 in
# a 64-block epoch ⇒ only the first 14 blocks of an epoch leave room for 50 misses.
# Pick a current committee member that is NOT the sequencer.
echo "== liveness ejection =="
VICTIM_ADDR=""
for w in $GOT; do [[ "$w" != "$(tr 'A-F' 'a-f' <<<"$(pp_owner_addr 0)")" ]] && { VICTIM_ADDR="$w"; break; }; done
VICTIM_IDX=""
for i in 1 2 3 4 5; do [[ "$(tr 'A-F' 'a-f' <<<"$(pp_owner_addr "$i")")" == "$VICTIM_ADDR" ]] && { VICTIM_IDX="$i"; break; }; done
[[ -n "$VICTIM_IDX" ]] || { echo "FAIL: could not pick a non-sequencer committee victim"; exit 1; }
# Sync the stop to an epoch start so all 50 misses fit before the counter resets.
START_E=$(pp_current_epoch)
pp_wait_epochs 1 >/dev/null
echo "  stopping validator-$VICTIM_IDX ($VICTIM_ADDR) at start of epoch $((START_E + 1))"
docker compose stop "validator-$VICTIM_IDX"

# Assert jailed at the 50-miss threshold (status 3 = Jail), immediate.
JAIL_OK=0
for _ in $(seq 1 40); do
    st=$(pp_staking_call "getValidatorStatus(address)(address,uint8,uint256,uint32,uint64,uint64)" "$VICTIM_ADDR" 2>/dev/null | sed -n '2p')
    [[ "$st" == 3 ]] && { JAIL_OK=1; break; }
    sleep 2
done
(( JAIL_OK == 1 )) || { echo "FAIL: validator-$VICTIM_IDX not jailed (status=$st)"; exit 1; }
echo "  validator-$VICTIM_IDX jailed at 50-miss threshold ✓"

# Committee[N] is committed at the boundary entering epoch N-1, and the jail
# lands MID-epoch (50 misses ≈ 50 s into it) — so the first committee selected
# post-jail is committee[E+2]: wait TWO boundaries, not one. (One boundary only
# ever passed on the unpaced ~5.8 blk/s chain, where the 2 s jail-detect poll
# lagged several epochs.) validatorJailEpochLength=4 keeps E+2 inside the jail.
pp_wait_epochs 2 >/dev/null
E=$(pp_current_epoch)
GOT=$(pp_committee "$E")
[[ " $GOT " != *" $VICTIM_ADDR "* ]] || { echo "FAIL: jailed validator still in committee(epoch $E): [$GOT]"; exit 1; }
echo "  ejected validator gone from getEpochCommittee(epoch $E) ✓"

# In unified mode v5 NEVER runs the legacy silent-verifier wait, so the
# committee watchdog WARN must be absent from its ENTIRE log — any occurrence
# means it fell back to the wedge the supervisor exists to eliminate. Logs go
# through a file: `docker logs | grep -q` SIGPIPEs under pipefail.
V5_LOG=$(mktemp)
docker compose logs validator-5 >"$V5_LOG" 2>&1 || true
if grep -q "NOT in the current committee" "$V5_LOG"; then
    echo "FAIL: v5 hit the committee watchdog (legacy verifier wedge):"
    grep "NOT in the current committee" "$V5_LOG"
    rm -f "$V5_LOG"; exit 1
fi
rm -f "$V5_LOG"
echo "  v5 watchdog silent for the whole run (unified mode) ✓"

# ── assert the background tx load kept finalizing across every transition ───
BEFORE=$(baseline_height)
sleep 6
AFTER=$(finalized_dec)
(( AFTER > BEFORE )) || { echo "FAIL: chain not finalizing under tx load ($AFTER <= $BEFORE)"; exit 1; }
echo "  chain still finalizing under tx load ($BEFORE → $AFTER) ✓"

echo "PASS (smoke-production-path)"
