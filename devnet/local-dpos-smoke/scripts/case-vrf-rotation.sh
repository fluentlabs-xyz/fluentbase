#!/usr/bin/env bash
# smoke-vrf-rotation: the LIVE per-epoch DKG beacon ROTATES its group key across a
# committee change, and the prev_randao beacon RELIVES from the first block of the
# new epoch (boundary deriver reads PK_E from the in-block beacon_outcome).
#
# This builds on smoke-vrf (which proves the beacon is live at the INITIAL
# committee) by driving a real committee rotation — the exact mechanism from
# smoke-production-path (register an external validator v5 via governance +
# delegate enough BLEND to outrank an initial validator) — and asserting that the
# committee change forces a fresh DKG ceremony whose new PK_E lands on-chain.
#
# It asserts ALL of:
#   1. BASELINE — at the initial committee (epoch E0) the beacon is live:
#      getEpochBeaconKey(E0) is non-empty, prev_randao is non-zero + varies +
#      byte-identical across all nodes (the smoke-vrf window check), and the
#      validators log the assurance=true ACTIVE_LINE.
#   2. TRIGGER — register external validator v5 (governance activate + delegate
#      >1e18 BLEND). committee[N] reads EffBal(N-1) and a delegate is effective in
#      EffBal(E+2) ⇒ v5 enters the committee at E+3 (mirrors smoke-production-path's
#      arithmetic; we compute it, not hardcode it — we wait for the FIRST epoch
#      whose committee set differs from E0's).
#   3. ROTATION — let E_new be that first changed-committee epoch.
#      getEpochBeaconKey(E_new) MUST be non-empty AND != getEpochBeaconKey(E0): the
#      committee change ran a fresh DKG and committed a new group key.
#   4. RELIVE — the beacon is assurance=true from the FIRST block of E_new: the
#      ACTIVE_LINE count keeps GROWING after the boundary and prev_randao is
#      non-zero + node-agreed at and just past the E_new boundary block (the beacon
#      did not stall across the rotation).
#   5. CARRY-FORWARD — pick a STABLE epoch (committee unchanged vs its predecessor;
#      we scan for one rather than guess). Its getEpochBeaconKey is EMPTY *and* the
#      beacon stays live across it (same key carried forward in memory).
#
#      ── CARRY-FORWARD ASSUMPTION (EMPTY, verified against the code) ──────────
#      A stable epoch returns EMPTY bytes (0x) from getEpochBeaconKey, NOT the
#      carried-forward key. Verified:
#        - The DKG ceremony (and thus a committed group key) starts ONLY when the
#          committee actually changes; an unchanged committee "carries the key
#          forward (no ceremony — Phase 5 reuses the prior epoch's BeaconKey)".
#          crates/dpos/consensus/src/beacon/actor.rs:175-185 (maybe_start: `if next
#          == cur { return; // carry-forward }`).
#        - commitEpochBeaconKey fires from the executor ONLY when a boundary block
#          staged a DKG outcome via the beacon_outcomes side-channel
#          (`if let Some((_, group_key)) = self.beacon_outcomes.remove(...)`).
#          crates/node/src/evm.rs:1158-1195. No ceremony ⇒ nothing staged ⇒ no
#          commit for that epoch.
#        - getEpochBeaconKey returns the raw mapping slot, which is empty for any
#          uncommitted epoch (Solidity default), and the doc-comment says exactly
#          "empty if uncommitted or a fallback epoch".
#          solidity-contracts contracts/staking/Staking.sol:741-745 (vendored
#          getter; see devnet/local-dpos-smoke/contracts/Staking.json ABI).
#      So a stable epoch ⇒ getEpochBeaconKey EMPTY while prev_randao stays live.
#      (If a future change made stable epochs commit the carried key instead, this
#      assertion would need to flip to "equals the previous epoch's key".)
#
# This case runs the PRODUCTION-PATH stack (runtime forge deploy + 6 validators),
# because that is the harness that can actually rotate the committee. The beacon
# group key for epoch 0 is committed at genesis-bootstrap on the genesis stack;
# on the production-path stack the staking cluster is deployed at runtime and the
# DPoS-activation epoch (relative E0) gets its key committed at the migration
# boundary the same way smoke-vrf's epoch 0 does.
#
# PREREQUISITES (host): docker, foundry (forge/cast), jq, a solidity-contracts
# checkout at $SOLIDITY_CONTRACTS_DIR.
#
# ⚠️ HARNESS PREREQUISITE (not yet satisfied on this branch — see commit notes):
# the production-path `--dpos` override (docker-compose.production-path.dpos.yml)
# does NOT yet pass `--dpos.beacon-sharing-path` / `--dpos.beacon-share-path`, so
# the production-path validators currently run the beacon KEYLESS (digest
# fallback, assurance=false) until the first live-DKG rotation. genesis-bootstrap
# DOES emit the keys to /runtime/keys/, but two things must be aligned before the
# baseline (always-active) assertions below can pass: (1) wire those flags into
# the .dpos.yml override (mirroring docker-compose.dpos.yml), and (2) ensure the
# genesis Sharing is dealt for the INITIAL committee size (the 6-peer bootstrap
# vs the 5-member initial committee — a size/index mismatch would wedge the
# always-active gate, so this needs an iterative docker run to confirm). The
# ROTATION assertions (3/4) — beacon comes alive at E_new via the live DKG —
# hold regardless; only the baseline (1) and carry-forward-stays-live (5) depend
# on the genesis key being wired+aligned.
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

# ── log helpers (verbatim from case-vrf.sh) ─────────────────────────────────
# mixHash (prev_randao) of block $1 as seen by RPC $2 (default $RPC), lowercased.
mixhash_at() { cast block "$1" --rpc-url "${2:-$RPC}" --json | jq -r .mixHash | tr 'A-F' 'a-f'; }
# mixHash of block $2 (decimal) as seen INSIDE container service $1.
mixhash_in() {
    local hexn
    hexn=$(printf '0x%x' "$2")
    docker compose exec -T "$1" curl -s -X POST -H 'Content-Type: application/json' \
        --data "{\"jsonrpc\":\"2.0\",\"method\":\"eth_getBlockByNumber\",\"params\":[\"$hexn\",false],\"id\":1}" \
        http://localhost:8545 2>/dev/null | jq -r '.result.mixHash // "null"' | tr 'A-F' 'a-f'
}
# mixHash of block $2 (decimal) as seen by node service $1 — routes to the host RPC
# for the two services that publish one, else the in-container probe. The
# production-path stack exposes validator-0 on 8545 and full-node on 18545.
mixhash_of() {
    case "$1" in
        validator-0) mixhash_at "$2" ;;
        full-node)   mixhash_at "$2" "http://localhost:18545" ;;
        *)           mixhash_in "$1" "$2" ;;
    esac
}
is_zero_hash() { [[ "$1" =~ ^0x0+$ ]]; }
# Count log lines matching $2 in service $1 (0 on no match — never trips set -e).
log_count() { docker compose logs "$1" 2>/dev/null | grep -c "$2" || true; }

# On-chain beacon group key for epoch $1 (lowercased, whitespace-stripped). Empty
# string when uncommitted (the getter returns 0x → we normalise that to "").
# Reader: getEpochBeaconKey(uint64)(bytes) — the only on-chain reader (smoke-vrf
# step 4 / Staking.sol:743). Reads against the runtime-deployed STAKING_RT.
beacon_key() {
    local k
    k=$(cast call "$STAKING_RT" "getEpochBeaconKey(uint64)(bytes)" "$1" --rpc-url "$RPC" 2>/dev/null \
        | tr -d '[:space:]' | tr 'A-F' 'a-f')
    [[ "$k" == "0x" || "$k" == "0x0" ]] && k=""
    printf '%s' "$k"
}
beacon_empty() { [[ -z "$1" ]]; }

# All deriving nodes on the production-path stack: 6 validators + full-node.
NODES=(validator-0 validator-1 validator-2 validator-3 validator-4 validator-5 full-node)
VALIDATORS=(validator-0 validator-1 validator-2 validator-3 validator-4 validator-5)
ACTIVE_LINE="beacon: threshold prev_randao active"
MIN_BLOCKS=5

# Assert prev_randao is non-zero AND byte-identical across all NODES at every block
# in [lo..hi] (decimal), AND all distinct across heights (real, varying randomness).
# $3 = a label for FAIL messages. Echoes nothing on success.
assert_beacon_window() {
    local lo="$1" hi="$2" label="$3"
    local n svc mh agree distinct
    local mixes=() vals=()
    for ((n = lo; n <= hi; n++)); do
        vals=()
        for svc in "${NODES[@]}"; do
            mh=$(mixhash_of "$svc" "$n")
            if [[ "$mh" == "null" || -z "$mh" ]]; then
                echo "FAIL (smoke-vrf-rotation): $label — $svc has no mixHash for block $n (node behind / RPC down)"
                docker compose logs --tail=80 "$svc"; exit 1
            fi
            if is_zero_hash "$mh"; then
                echo "FAIL (smoke-vrf-rotation): $label — prev_randao is zero at block $n on $svc (beacon stalled / fell to digest)"
                docker compose logs --tail=80 "$svc"; exit 1
            fi
            vals+=("$mh")
        done
        agree=$(printf '%s\n' "${vals[@]}" | sort -u | wc -l)
        if (( agree != 1 )); then
            echo "FAIL (smoke-vrf-rotation): $label — nodes disagree on prev_randao at block $n (divergent seed):"
            paste -d' ' <(printf '%s\n' "${NODES[@]}") <(printf '%s\n' "${vals[@]}") | sed 's/^/  /'
            exit 1
        fi
        mixes+=("${vals[0]}")
    done
    distinct=$(printf '%s\n' "${mixes[@]}" | sort -u | wc -l)
    if (( distinct != ${#mixes[@]} )); then
        echo "FAIL (smoke-vrf-rotation): $label — prev_randao not varying over [$lo..$hi]: ${#mixes[@]} blocks but only $distinct distinct (stuck randomness)"
        printf '  %s\n' "${mixes[@]}"
        exit 1
    fi
    echo "  [$label] blocks [$lo..$hi]: ${#mixes[@]}/${#mixes[@]} distinct non-zero prev_randao, byte-identical across all ${#NODES[@]} nodes"
}

# Snapshot each validator's ACTIVE_LINE count into the named assoc array $1.
snapshot_active_counts() {
    local -n _dst="$1"
    local v c
    for v in "${VALIDATORS[@]}"; do
        c=$(log_count "$v" "$ACTIVE_LINE"); c=${c:-0}
        _dst[$v]=$c
    done
}

# ════════════════════════════════════════════════════════════════════════════
# Bring up the production-path DPoS stack with the initial 5-validator committee.
# This block mirrors case-production-path.sh phases A..cold-restart verbatim
# (deploy token+verifier+cluster via forge, setConsensusKeys, set activation,
# clean-halt, --dpos cold-restart). The only difference from that case is what we
# assert afterwards (beacon rotation, not the join/eject lifecycle).
# ════════════════════════════════════════════════════════════════════════════
echo "== phase A: bare sequencer chain =="
docker compose up --build -d
pp_wait_converge 120 >/dev/null || { echo "FAIL (smoke-vrf-rotation): bare chain did not converge"; docker compose logs --tail=120; exit 1; }
echo "  converged plain chain"

DEPLOYER_KEY="$(pp_owner_key 0)"
DEPLOYER_ADDR="$(pp_owner_addr 0)"

MNEMONIC="${FLUENT_DPOS_MNEMONIC:-test test test test test test test test test test test junk}"
SPAMMER_KEY="$(cast wallet private-key --mnemonic "$MNEMONIC" --mnemonic-index 6)"
SPAMMER_ADDR="$(cast wallet address --mnemonic "$MNEMONIC" --mnemonic-index 6)"
cast send "$SPAMMER_ADDR" --value 1000000000000000 \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" >/dev/null \
    || { echo "FAIL (smoke-vrf-rotation): fund spammer account"; exit 1; }
pp_spammer_start "$SPAMMER_KEY" "$DEPLOYER_ADDR"
echo "  tx spammer started (pid $PP_SPAMMER_PID, from $SPAMMER_ADDR)"

echo "== runtime deploy: token + BLS verifier =="
TOKEN=$(forge_l2 forge create --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --json \
    "contracts/staking/mocks/MockBlendToken.sol:MockBlendToken" | jq -r '.deployedTo')
[[ "$TOKEN" == 0x* ]] || { echo "FAIL (smoke-vrf-rotation): MockBlendToken deploy"; exit 1; }
VERIFIER=$(forge_l2 forge create --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --json \
    "contracts/libraries/BLS12381Verifier.sol:BLS12381Verifier" | jq -r '.deployedTo')
[[ "$VERIFIER" == 0x* ]] || { echo "FAIL (smoke-vrf-rotation): BLS12381Verifier deploy"; exit 1; }
echo "  token=$TOKEN verifier=$VERIFIER"

pp_token_transfer "$TOKEN" "$(pp_owner_addr 5)" "10000000000000000000"

echo "== runtime deploy: staking cluster (DeployStaking) =="
NETWORK=local-dpos-smoke/l2 DEPLOYER="$DEPLOYER_ADDR" INITIAL_OWNER="$DEPLOYER_ADDR" \
  STAKING_TOKEN="$TOKEN" OUTPUT_PATH="$MANIFEST" \
  forge_l2 forge script scripts/deploy/DeployStaking.s.sol:DeployStaking \
    --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --skip-simulation \
  || { echo "FAIL (smoke-vrf-rotation): DeployStaking (EIP-170? see prereqs)"; exit 1; }

STAKING_RT=$(jq -r '.staking' "$MANIFEST")
CHAIN_CONFIG_RT=$(jq -r '.chain_config' "$MANIFEST")
GOV_ADDR=$(jq -r '.governance' "$MANIFEST")
LIVENESS_RT=$(jq -r '.liveness_slashing' "$MANIFEST")
for v in STAKING_RT CHAIN_CONFIG_RT GOV_ADDR LIVENESS_RT; do
    [[ "${!v}" == 0x* ]] || { echo "FAIL (smoke-vrf-rotation): manifest missing $v"; cat "$MANIFEST"; exit 1; }
done
echo "  staking=$STAKING_RT chainConfig=$CHAIN_CONFIG_RT gov=$GOV_ADDR liveness=$LIVENESS_RT"

echo "== governance: setBlsVerifier (MUST precede setConsensusKeys) =="
pp_gov_action "$CHAIN_CONFIG_RT" \
    "$(cast calldata 'setBlsVerifier(address)' "$VERIFIER")" \
    "setBlsVerifier" || { echo "FAIL (smoke-vrf-rotation): gov setBlsVerifier"; exit 1; }

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
        || { echo "FAIL (smoke-vrf-rotation): setConsensusKeys v$i"; exit 1; }
done
echo "  consensus keys set for 5 validators"

HEAD=$(printf '%d' "$(check_external 8545 | cut -d'|' -f1)")
ACT=$(( ((HEAD / 64) + 2) * 64 ))
echo "== governance: setDposActivationBlock=$ACT (head=$HEAD) =="
pp_gov_action "$CHAIN_CONFIG_RT" \
    "$(cast calldata 'setDposActivationBlock(uint64)' "$ACT")" \
    "setDposActivationBlock" || { echo "FAIL (smoke-vrf-rotation): gov setDposActivationBlock"; exit 1; }

echo "== assert pre-written staking-reader.json matches the deploy manifest =="
PRE=$(docker compose exec -T validator-0 cat /runtime/staking-reader.json)
for pair in "staking_address:$STAKING_RT" \
            "chain_config_address:$CHAIN_CONFIG_RT" \
            "liveness_slashing_address:$LIVENESS_RT"; do
    k=${pair%%:*} want=$(tr 'A-F' 'a-f' <<<"${pair#*:}")
    got=$(jq -r ".$k" <<<"$PRE" | tr 'A-F' 'a-f')
    [[ "$got" == "$want" ]] || { echo "FAIL (smoke-vrf-rotation): pre-written $k=$got != deployed $want (deployer nonce drift — update --staking-reader-create-nonces)"; exit 1; }
done
echo "  pre-written config matches manifest"

echo "== wait for sequencer (validator-0) to clean-halt at activation block $ACT =="
wait_finalized_ge "$ACT" 200 || {
    echo "FAIL (smoke-vrf-rotation): sequencer did not reach activation block $ACT (head=$(finalized_dec))"
    docker compose logs validator-0 --tail=80; exit 1
}
pp_wait_converge 90 >/dev/null \
    || { echo "FAIL (smoke-vrf-rotation): followers did not align at the activation block"; docker compose logs --tail=120; exit 1; }
echo "  all nodes aligned at $ACT; proceeding to --dpos cold-restart"

echo "== cold-restart: all validators into unified --dpos =="
ANCHOR=$(check_external 8545 | cut -d'|' -f1)
export COMPOSE_FILE="docker-compose.production-path.yml:docker-compose.production-path.dpos.yml"
docker compose up -d --force-recreate "${PP_VALS[@]}" \
    || { echo "FAIL (smoke-vrf-rotation): cold-restart into --dpos (a validator exited)"; docker compose logs validator-0 --tail=80; exit 1; }
pp_wait_converge 180 "$ANCHOR" >/dev/null \
    || { echo "FAIL (smoke-vrf-rotation): DPoS chain did not converge past anchor $ANCHOR"; docker compose logs --tail=200; exit 1; }
echo "  DPoS chain live past anchor $ANCHOR"

# Epoch length (blocks) — read once, used to map epoch numbers to boundary blocks.
EPOCH_LEN=$(printf '%d' "$(pp_chainconfig_call 'getEpochBlockInterval()(uint32)')")
(( EPOCH_LEN > 0 )) || { echo "FAIL (smoke-vrf-rotation): getEpochBlockInterval()=0"; exit 1; }
# Decimal block height of the FIRST block of relative epoch $1.
epoch_first_block() { echo $(( ACT + $1 * EPOCH_LEN )); }

# ════════════════════════════════════════════════════════════════════════════
# 1) BASELINE — initial committee E0 has a live, verified beacon.
# ════════════════════════════════════════════════════════════════════════════
echo "== 1) BASELINE: beacon live at the initial committee =="
E0=$(pp_current_epoch)
GOT0=$(pp_committee "$E0")
EXPECT0=$(for i in 0 1 2 3 4; do pp_owner_addr "$i"; done | tr 'A-F' 'a-f' | sort | paste -sd' ' -)
[[ "$GOT0" == "$EXPECT0" ]] || { echo "FAIL (smoke-vrf-rotation): committee(E0=$E0) != initial 5 (got [$GOT0] want [$EXPECT0])"; exit 1; }
echo "  committee(epoch $E0) == initial 5"

# E0's beacon key must be committed (non-empty): the relative-epoch-0 key is
# committed at the migration boundary (the same on-chain reader smoke-vrf step 4
# checks for genesis epoch 0).
KEY_E0=$(beacon_key "$E0")
if beacon_empty "$KEY_E0"; then
    echo "FAIL (smoke-vrf-rotation): getEpochBeaconKey(E0=$E0) is EMPTY — initial committee has no committed PK_E (beacon never bootstrapped)"
    docker compose logs validator-0 --tail=80; exit 1
fi
echo "  getEpochBeaconKey(E0=$E0) committed (non-empty)"

# prev_randao live over a window inside E0 — non-zero, varying, node-agreed.
fin=$(finalized_dec)
(( fin > 0 )) || { echo "FAIL (smoke-vrf-rotation): no finalized block"; exit 1; }
WINDOW=8
lo=$(( fin > WINDOW ? fin - WINDOW + 1 : 1 ))
echo "  checking baseline prev_randao window [$lo..$fin]"
assert_beacon_window "$lo" "$fin" "baseline E$E0"

# Validators logged the assurance=true ACTIVE_LINE (the beacon path against PK_E,
# not the digest fallback) at least MIN_BLOCKS times.
for v in "${VALIDATORS[@]}"; do
    c=$(log_count "$v" "$ACTIVE_LINE"); c=${c:-0}
    if (( c < MIN_BLOCKS )); then
        echo "FAIL (smoke-vrf-rotation): $v logged threshold prev_randao only $c times (< $MIN_BLOCKS) — beacon inactive at baseline"
        docker compose logs --tail=80 "$v"; exit 1
    fi
    echo "  $v — threshold prev_randao active x$c (baseline)"
done

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

pp_wait_converge 90 "$REG_FLOOR" >/dev/null \
    || { echo "FAIL (smoke-vrf-rotation): nodes lost alignment during registration"; docker compose logs validator-5 --tail=80; exit 1; }
echo "  v5 follower substrate tracked the chain through registration"

# Wait for the FIRST epoch whose committee set differs from E0's — that is E_new.
# We do NOT hardcode E0+3; we scan the ahead-committed committees for the first
# change (the production-path arithmetic predicts E0+3, but we assert against the
# real on-chain committee so a re-tuned EffBal timeline does not silently skew us).
echo "== wait for the committee to actually change (expect ~E0+3 by EffBal arithmetic) =="
E_new=""
_deadline=$(( $(date +%s) + 360 ))
while (( $(date +%s) < _deadline )); do
    E=$(pp_current_epoch)
    # committee[E+1] is committed one boundary ahead; inspect it for v5.
    AHEAD=$(pp_committee $((E + 1)))
    if [[ -n "$AHEAD" && " $AHEAD " == *" $v5l "* && "$AHEAD" != "$GOT0" ]]; then
        E_new=$((E + 1)); break
    fi
    sleep 2
done
[[ -n "$E_new" ]] || { echo "FAIL (smoke-vrf-rotation): committee never changed (v5 never entered an ahead-committed committee within 360s)"; docker compose logs validator-5 --tail=80; exit 1; }
GOT_NEW=$(pp_committee "$E_new")
[[ "$GOT_NEW" != "$GOT0" ]] || { echo "FAIL (smoke-vrf-rotation): committee(E_new=$E_new) equals E0's — not actually a rotation"; exit 1; }
echo "  committee changed at E_new=$E_new (E0=$E0): [$GOT_NEW] (was [$GOT0])"

# ════════════════════════════════════════════════════════════════════════════
# 3) ROTATION — the fresh DKG committed a NEW group key for E_new.
# ════════════════════════════════════════════════════════════════════════════
echo "== 3) ROTATION: getEpochBeaconKey(E_new) non-empty AND != getEpochBeaconKey(E0) =="
# E_new's key may be committed exactly at its boundary; wait until the chain
# crosses the E_new boundary so the commitEpochBeaconKey system call has landed.
E_NEW_BOUNDARY=$(epoch_first_block "$E_new")
wait_finalized_ge "$E_NEW_BOUNDARY" 240 \
    || { echo "FAIL (smoke-vrf-rotation): chain did not reach E_new boundary block $E_NEW_BOUNDARY (head=$(finalized_dec))"; docker compose logs validator-0 --tail=80; exit 1; }
KEY_NEW=""
for _ in $(seq 1 30); do
    KEY_NEW=$(beacon_key "$E_new")
    beacon_empty "$KEY_NEW" || break
    sleep 2
done
if beacon_empty "$KEY_NEW"; then
    echo "FAIL (smoke-vrf-rotation): getEpochBeaconKey(E_new=$E_new) is EMPTY after the committee changed — the fresh DKG did not commit a key (beacon did not re-deal)"
    echo "  --- 'live DKG' lines on the new committee member v5 ---"
    docker compose logs validator-5 2>/dev/null | grep "live DKG" | tail -20 || true
    docker compose logs validator-0 --tail=80
    exit 1
fi
if [[ "$KEY_NEW" == "$KEY_E0" ]]; then
    echo "FAIL (smoke-vrf-rotation): getEpochBeaconKey(E_new=$E_new) == getEpochBeaconKey(E0=$E0) — committee changed but the group key did NOT rotate (stale DKG / carried forward through a change)"
    echo "  E0  key: $KEY_E0"
    echo "  Enew key: $KEY_NEW"
    exit 1
fi
echo "  getEpochBeaconKey(E_new=$E_new) is non-empty and ROTATED away from E0's key"

# ════════════════════════════════════════════════════════════════════════════
# 4) RELIVE — the beacon is assurance=true from the FIRST block of E_new: the
#    boundary deriver reads PK_E from the in-block beacon_outcome, so prev_randao
#    is non-zero + agreed at and just past the boundary, and the ACTIVE_LINE
#    count keeps GROWING (the beacon did not stall across the rotation).
# ════════════════════════════════════════════════════════════════════════════
echo "== 4) RELIVE: beacon live from the first block of E_new =="
declare -A before_relive
snapshot_active_counts before_relive

# Advance a few blocks past the E_new boundary so we have a window strictly inside
# the new epoch to check (and so the growth check below has room).
RELIVE_HI=$(( E_NEW_BOUNDARY + 4 ))
wait_finalized_ge "$RELIVE_HI" 60 \
    || { echo "FAIL (smoke-vrf-rotation): chain did not advance past the E_new boundary ($RELIVE_HI) — cannot observe a sustained post-rotation beacon"; exit 1; }
# Window: the boundary block itself + the next few blocks of E_new.
assert_beacon_window "$E_NEW_BOUNDARY" "$RELIVE_HI" "relive E$E_new"

for v in "${VALIDATORS[@]}"; do
    after=$(log_count "$v" "$ACTIVE_LINE"); after=${after:-0}
    if (( after <= ${before_relive[$v]} )); then
        echo "FAIL (smoke-vrf-rotation): $v active-count frozen at $after across the E_new boundary — beacon stalled at the rotation (fell back to digest)"
        docker compose logs --tail=80 "$v"; exit 1
    fi
    echo "  $v — active-count grew ${before_relive[$v]} → $after across the rotation (beacon relive)"
done

# ════════════════════════════════════════════════════════════════════════════
# 5) CARRY-FORWARD — a STABLE epoch (committee unchanged vs its predecessor) has
#    an EMPTY on-chain beacon key (no commit when nothing rotated) yet the beacon
#    stays live across it (the prior key carried forward in memory).
#    Assumption: EMPTY (see header). Reader: getEpochBeaconKey(uint64)(bytes).
# ════════════════════════════════════════════════════════════════════════════
echo "== 5) CARRY-FORWARD: stable epoch has EMPTY beacon key but a live beacon =="
# Find a stable epoch: committee(E_s) == committee(E_s - 1), with E_s already
# fully elapsed (so its window of blocks exists). Scan [1 .. E_new-1] for the
# first such epoch (e.g. E0+1, which is before any rotation). E_s>=1 so a
# predecessor committee exists; E_s != E_new (which by construction changed).
E_s=""
cur_now=$(pp_current_epoch)
for ((e = 1; e < E_new && e < cur_now; e++)); do
    c_e=$(pp_committee "$e")
    c_p=$(pp_committee $((e - 1)))
    # both must be readable (non-empty) and equal → unchanged committee.
    if [[ -n "$c_e" && "$c_e" == "$c_p" ]]; then
        E_s="$e"; break
    fi
done
[[ -n "$E_s" ]] || { echo "FAIL (smoke-vrf-rotation): could not find a stable (unchanged-committee) epoch in [1..$((E_new - 1))] — cannot test carry-forward"; exit 1; }
echo "  stable epoch E_s=$E_s (committee == committee($((E_s - 1))))"

# 5a) Its on-chain beacon key is EMPTY (no commit on a no-change epoch).
KEY_S=$(beacon_key "$E_s")
if ! beacon_empty "$KEY_S"; then
    echo "FAIL (smoke-vrf-rotation): getEpochBeaconKey(stable E_s=$E_s) is NON-empty ($KEY_S) — a key was committed for an unchanged committee, violating the carry-forward (no-commit) assumption (see header). If the contract now commits the carried key on stable epochs, flip this assertion to '== getEpochBeaconKey(E_s-1)'."
    exit 1
fi
echo "  getEpochBeaconKey(stable E_s=$E_s) is EMPTY (no on-chain commit — key carried forward)"

# 5b) The beacon stays LIVE across E_s: prev_randao non-zero + node-agreed over a
#     window inside E_s (the carried-forward key still verifies the seed).
S_LO=$(epoch_first_block "$E_s")
S_HI=$(( S_LO + (EPOCH_LEN > 5 ? 4 : EPOCH_LEN - 1) ))
echo "  checking live-across-stable prev_randao window [$S_LO..$S_HI]"
assert_beacon_window "$S_LO" "$S_HI" "carry-forward E$E_s"

# ── assert the background tx load kept finalizing throughout ─────────────────
BEFORE=$(baseline_height)
sleep 6
AFTER=$(finalized_dec)
(( AFTER > BEFORE )) || { echo "FAIL (smoke-vrf-rotation): chain not finalizing under tx load ($AFTER <= $BEFORE)"; exit 1; }
echo "  chain still finalizing under tx load ($BEFORE → $AFTER)"

echo "OK (smoke-vrf-rotation): live per-epoch DKG beacon ROTATED across a committee change — \
baseline beacon live at initial committee E$E0 (getEpochBeaconKey non-empty, prev_randao non-zero+varying+node-agreed, assurance log active); \
registering v5 changed the committee at E$E_new; getEpochBeaconKey(E$E_new) is non-empty and != getEpochBeaconKey(E$E0) (fresh DKG re-dealt); \
beacon relived from the first block of E$E_new (prev_randao non-zero+agreed at the boundary, ACTIVE_LINE counts still growing); \
stable epoch E$E_s carried the key forward (getEpochBeaconKey EMPTY, no commit) while prev_randao stayed live across it"
