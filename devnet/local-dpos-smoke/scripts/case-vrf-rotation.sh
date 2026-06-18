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
#   1. BASELINE — at the initial committee (epoch E0) the chain is live and the
#      beacon is in its KEYLESS pre-rotation state: getEpochBeaconKey(E0) is EMPTY,
#      prev_randao is non-zero + varies + byte-identical across all nodes
#      (digest-fallback determinism), and the ACTIVE_LINE has NOT yet fired
#      (assurance=false is EXPECTED here — see the keyless-baseline note below).
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
#   5. CARRY-FORWARD — pick a STABLE epoch AFTER E_new (committee unchanged vs its
#      predecessor; we scan for one). The DKG only re-deals + commits on a committee
#      CHANGE, so a stable epoch has NO own commit — but the staking contract's
#      getEpochBeaconKey does CARRY-FORWARD ON READ (option A), returning the
#      most-recent committed key, so a stable epoch returns the SAME key as E_new
#      (non-empty), AND the beacon stays live across it.
#
#      ── CARRY-FORWARD MODEL (option A — carry-forward on read) ───────────────
#        - commitEpochBeaconKey(uint64 epoch, bytes) fires ONLY on a committee
#          CHANGE (the boundary block stages the DKG outcome via beacon_outcomes;
#          crates/node/src/evm.rs); stable epochs commit nothing.
#        - getEpochBeaconKey(E) returns the active key = the most-recent committed
#          key with committed-epoch <= E (Staking.sol _activeBeaconKey), so a
#          stable epoch E_s > E_new returns E_new's PK (carried forward), and the
#          deriver/STF read the active key directly.
#        - Contract change tracked in solidity-contracts task
#          beacon_key_sparse_epoch_commit.
#      So a stable epoch ⇒ getEpochBeaconKey == E_new's key, prev_randao stays live.
#
# This case runs the PRODUCTION-PATH stack (runtime forge deploy + 6 validators),
# because that is the harness that can actually rotate the committee.
#
# KEYLESS-BASELINE NOTE (by design — do NOT "fix" by wiring genesis keys): the
# production-path `--dpos` override does NOT pass --dpos.beacon-sharing-path /
# --dpos.beacon-share-path, so the validators run the beacon KEYLESS until the
# first live-DKG rotation. This is intentional: genesis-bootstrap deals the beacon
# for --peers=6 aligned to the 6-peer sorted order, but the initial committee is 5
# — wiring that key would WEDGE the always-active gate on an index mismatch (a
# member's Participant index in the 5-member BiMap != its 6-deal index). The
# always-active-from-genesis property is covered by smoke-vrf (genesis stack,
# --peers=4 == committee=4). HERE we test the LIVE DKG: a committee change runs a
# fresh ceremony among the stayers, commits PK_E_new on-chain, and the beacon
# comes ALIVE at E_new (all nodes derive prev_randao by verifying the cert seed vs
# the committed PK — even v5/full-node, which hold no share). v5 (the joiner) is a
# cert-follower during E_new-1, so it MISSES E_new's ceremony (not on BEACON_CHANNEL)
# → it is a beacon-observer at E_new (no share); the 4 stayers (n=5 ⇒ quorum 4)
# carry the DKG + the seed quorum. So the active-count-growth probe targets a
# STAYER; node-agreement spans all nodes.
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

# ── log helpers (verbatim from case-vrf.sh) ─────────────────────────────────
# mixHash (prev_randao) of block $1 as seen by RPC $2 (default $RPC), lowercased.
# GRACEFUL: a not-yet-synced block makes `cast block` exit non-zero — under
# set -e + pipefail that would silently kill the script mid-window instead of the
# intended "node behind" FAIL. Coerce any failure / missing field to "null" so the
# callers (assert_beacon_window / wait_nodes_have) handle a lagging node cleanly.
mixhash_at() {
    { cast block "$1" --rpc-url "${2:-$RPC}" --json 2>/dev/null || echo '{}'; } \
        | jq -r '.mixHash // "null"' 2>/dev/null | tr 'A-F' 'a-f'
}
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
ACTIVE_LINE="beacon: threshold prev_randao active"

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

# Wait until EVERY node has block $1 (decimal) available, up to $2 s (default 120).
# The followers (full-node + a freshly-promoted v5) lag the validators by a few
# blocks, so a strict cross-node window right at a fresh boundary must wait for
# them to catch up first — otherwise assert_beacon_window trips "node behind".
wait_nodes_have() {
    local block="$1" deadline=$(( SECONDS + ${2:-120} )) svc mh all
    while true; do
        all=1
        for svc in "${NODES[@]}"; do
            mh=$(mixhash_of "$svc" "$block")
            if [[ "$mh" == "null" || -z "$mh" ]]; then all=0; break; fi
        done
        (( all == 1 )) && return 0
        if (( SECONDS >= deadline )); then
            echo "  [wait_nodes_have] timeout at block $block — per-node status:"
            for svc in "${NODES[@]}"; do
                mh=$(mixhash_of "$svc" "$block")
                if [[ "$mh" == "null" || -z "$mh" ]]; then
                    echo "    $svc: MISSING block $block — last 50 log lines (beacon/epoch/seed/error):"
                    docker compose logs --tail=400 "$svc" 2>/dev/null \
                        | grep -iE "beacon|epoch|seed|prev_randao|share|verif|notariz|finaliz|nullif|error|panic|syncing|stuck" \
                        | tail -50 | sed 's/^/      /' || true
                else
                    echo "    $svc: has block $block"
                fi
            done
            return 1
        fi
        sleep 2
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
pp_wait_converge 240 >/dev/null || { echo "FAIL (smoke-vrf-rotation): bare chain did not converge"; docker compose logs --tail=120; exit 1; }
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
wait_finalized_ge "$ACT" 400 || {
    echo "FAIL (smoke-vrf-rotation): sequencer did not reach activation block $ACT (head=$(finalized_dec))"
    docker compose logs validator-0 --tail=80; exit 1
}
pp_wait_converge 180 >/dev/null \
    || { echo "FAIL (smoke-vrf-rotation): followers did not align at the activation block"; docker compose logs --tail=120; exit 1; }
echo "  all nodes aligned at $ACT; proceeding to --dpos cold-restart"

echo "== cold-restart: all validators into unified --dpos (+ full-node into --cert-follow) =="
ANCHOR=$(check_external 8545 | cut -d'|' -f1)
export COMPOSE_FILE="docker-compose.production-path.yml:docker-compose.production-path.dpos.yml"
# full-node is recreated too: the .dpos.yml overlay switches it from the
# pre-DPoS trust-follow block relay to a CERT-FOLLOWER. A relay can't reproduce
# the beacon boundary block's commitEpochBeaconKey (PK_E lives in the consensus
# OrderBlock, not the executed block), so it diverges at the rotation; a
# cert-follower derives + verifies the seed and survives.
docker compose up -d --force-recreate "${PP_VALS[@]}" full-node \
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
# 1) BASELINE — initial committee E0, KEYLESS beacon (pre-rotation). The chain is
#    live and prev_randao is non-zero + node-agreed (digest fallback), but no
#    beacon key is committed yet and the ACTIVE_LINE has not fired. See the
#    KEYLESS-BASELINE NOTE in the header for WHY (genesis key not wired on the
#    production-path stack — by design; the live DKG brings the beacon alive at
#    the first rotation).
# ════════════════════════════════════════════════════════════════════════════
echo "== 1) BASELINE: chain live + KEYLESS beacon at the initial committee =="
E0=$(pp_current_epoch)
GOT0=$(pp_committee "$E0")
EXPECT0=$(for i in 0 1 2 3 4; do pp_owner_addr "$i"; done | tr 'A-F' 'a-f' | sort | paste -sd' ' -)
[[ "$GOT0" == "$EXPECT0" ]] || { echo "FAIL (smoke-vrf-rotation): committee(E0=$E0) != initial 5 (got [$GOT0] want [$EXPECT0])"; exit 1; }
echo "  committee(epoch $E0) == initial 5"

# E0's beacon key is EMPTY — the production-path baseline is keyless (no genesis
# key wired; the live DKG commits a key only at the first committee change). Stays
# in scope for step 3's rotation comparison (KEY_E0 == "" ⇒ any committed KEY_NEW
# trivially differs).
KEY_E0=$(beacon_key "$E0")
if ! beacon_empty "$KEY_E0"; then
    echo "FAIL (smoke-vrf-rotation): getEpochBeaconKey(E0=$E0) is NON-empty ($KEY_E0) — the production-path baseline is expected KEYLESS. If the .dpos.yml override now wires a genesis key, flip this baseline back to beacon-active (and the genesis Sharing must be dealt for the 5-member committee, not 6 — see header)."
    exit 1
fi
echo "  getEpochBeaconKey(E0=$E0) is EMPTY (keyless production-path baseline — expected)"

# prev_randao live over a window inside E0 — non-zero, varying, node-agreed. On the
# keyless baseline this is the digest fallback (still deterministic + agreed across
# nodes), which is all this assertion requires; ACTIVE_LINE is NOT required here.
fin=$(finalized_dec)
(( fin > 0 )) || { echo "FAIL (smoke-vrf-rotation): no finalized block"; exit 1; }
WINDOW=8
lo=$(( fin > WINDOW ? fin - WINDOW + 1 : 1 ))
echo "  checking baseline prev_randao window [$lo..$fin]"
assert_beacon_window "$lo" "$fin" "baseline E$E0"
echo "  baseline beacon keyless (ACTIVE_LINE not required at E0; goes live at E_new)"

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

# Stayers = members in BOTH committee[E0] and committee[E_new]: they were on the
# BEACON_CHANNEL during E_new-1 and so dealt+hold shares from E_new's DKG, logging
# the ACTIVE_LINE first. v5 (the joiner) was a cert-follower during E_new-1, missed
# the ceremony, and is a beacon-OBSERVER at E_new (no share) — so the active-count
# growth probe (step 4) MUST target a stayer, not v5. n=5 ⇒ quorum 4 ⇒ the 4
# stayers carry the DKG + the seed quorum on their own.
PROBE_STAYERS=()
for i in 0 1 2 3 4; do
    al=$(tr 'A-F' 'a-f' <<<"$(pp_owner_addr "$i")")
    if [[ " $GOT0 " == *" $al "* && " $GOT_NEW " == *" $al "* ]]; then
        PROBE_STAYERS+=("validator-$i")
    fi
done
[[ ${#PROBE_STAYERS[@]} -ge 1 ]] || { echo "FAIL (smoke-vrf-rotation): no stayer between committee[E0] and committee[E_new] — no beacon-share holder to probe"; exit 1; }
# Prefer validator-0 (host-mapped 8545) first if it is a stayer.
if printf '%s\n' "${PROBE_STAYERS[@]}" | grep -qx validator-0; then
    PROBE_STAYERS=(validator-0 $(printf '%s\n' "${PROBE_STAYERS[@]}" | grep -vx validator-0))
fi
echo "  stayers (DKG-share holders; active-growth probe set): ${PROBE_STAYERS[*]}"

# ════════════════════════════════════════════════════════════════════════════
# 3) ROTATION — the fresh DKG committed a NEW group key for E_new.
# ════════════════════════════════════════════════════════════════════════════
echo "== 3) ROTATION: getEpochBeaconKey(E_new) non-empty AND != getEpochBeaconKey(E0) =="
# E_new's key may be committed exactly at its boundary; wait until the chain
# crosses the E_new boundary so the commitEpochBeaconKey system call has landed.
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
KEY_NEW=""
for _ in $(seq 1 30); do
    KEY_NEW=$(beacon_key "$E_new")
    beacon_empty "$KEY_NEW" || break
    sleep 2
done
if beacon_empty "$KEY_NEW"; then
    echo "FAIL (smoke-vrf-rotation): getEpochBeaconKey(E_new=$E_new) is EMPTY after the committee changed — the fresh DKG did not commit a key (beacon did not re-deal)"
    echo "  --- 'live DKG' lines on the STAYERS (eval / ceremony started / computed?) ---"
    for s in "${PROBE_STAYERS[@]}"; do
        echo "  [$s]:"; docker compose logs "$s" 2>/dev/null | grep "live DKG" | tail -8 | sed 's/^/    /' || true
    done
    docker compose logs validator-0 --tail=60
    exit 1
fi
# With the keyless baseline KEY_E0 is "", so a non-empty KEY_NEW trivially differs;
# the -n guard documents that the real assertion is the non-empty check above and
# only fires if a (non-empty) E0 key somehow equals E_new's.
if [[ -n "$KEY_E0" && "$KEY_NEW" == "$KEY_E0" ]]; then
    echo "FAIL (smoke-vrf-rotation): getEpochBeaconKey(E_new=$E_new) == getEpochBeaconKey(E0=$E0) — committee changed but the group key did NOT rotate"
    echo "  E0  key: $KEY_E0"
    echo "  Enew key: $KEY_NEW"
    exit 1
fi
echo "  getEpochBeaconKey(E_new=$E_new) is non-empty (first live DKG) and != E0's key"

# ════════════════════════════════════════════════════════════════════════════
# 4) RELIVE — the beacon is assurance=true from the FIRST block of E_new: the
#    boundary deriver reads PK_E from the in-block beacon_outcome, so prev_randao
#    is non-zero + agreed at and just past the boundary, and the ACTIVE_LINE
#    count keeps GROWING (the beacon did not stall across the rotation).
# ════════════════════════════════════════════════════════════════════════════
echo "== 4) RELIVE: beacon live from the first block of E_new =="
# Snapshot ACTIVE_LINE counts on the STAYERS only (the share holders that log
# assurance=true; v5 is a beacon-observer at E_new and may not log active yet).
declare -A before_relive
for v in "${PROBE_STAYERS[@]}"; do
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
for v in "${PROBE_STAYERS[@]}"; do
    after=$(log_count "$v" "$ACTIVE_LINE"); after=${after:-0}
    if (( after <= ${before_relive[$v]} )); then
        echo "FAIL (smoke-vrf-rotation): $v (stayer/share-holder) active-count frozen at $after across the E_new boundary — beacon did not relive (fell back to digest)"
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
echo "== 5) CARRY-FORWARD: stable epoch carries E_new's key forward (live beacon) =="
# The stable epoch MUST be AFTER E_new: a stable epoch in [1, E_new) is keyless
# (digest fallback, not threshold-live) on the production-path baseline, so testing
# carry-forward there proves nothing about the beacon carrying its GROUP KEY
# forward. The first stable epoch after the rotation is E_new+1: its committee
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

# 5a) CARRY-FORWARD ON READ (Staking option A): the stable epoch has NO own
#     commit (the DKG only re-deals + commits on a committee CHANGE), but
#     getEpochBeaconKey carries the most-recent committed key forward, so it
#     returns the SAME key as E_new (non-empty) — NOT empty. This is the on-chain
#     source of truth for "active PK at epoch E_s".
KEY_S=$(beacon_key "$E_s")
if beacon_empty "$KEY_S"; then
    echo "FAIL (smoke-vrf-rotation): getEpochBeaconKey(stable E_s=$E_s) is EMPTY — carry-forward-on-read should return the carried key (E_new's PK). Contract regression (option A) or no commit landed."
    exit 1
fi
if [[ "$KEY_S" != "$KEY_NEW" ]]; then
    echo "FAIL (smoke-vrf-rotation): getEpochBeaconKey(stable E_s=$E_s)=$KEY_S != carried key from E_new=$KEY_NEW — carry-forward returned the wrong key"
    exit 1
fi
echo "  getEpochBeaconKey(stable E_s=$E_s) carries E_new's key forward (== $KEY_NEW)"

# 5b) The beacon stays LIVE across E_s: prev_randao non-zero + node-agreed over a
#     window inside E_s (the carried-forward key still verifies the seed).
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

echo "OK (smoke-vrf-rotation): live per-epoch DKG beacon ROTATED across a committee change — \
baseline chain live at initial committee E$E0 with KEYLESS beacon (getEpochBeaconKey EMPTY, prev_randao non-zero+varying+node-agreed via digest fallback, ACTIVE_LINE not yet firing); \
registering v5 changed the committee at E$E_new; getEpochBeaconKey(E$E_new) non-empty (first live DKG) and != getEpochBeaconKey(E$E0); \
beacon CAME ALIVE from the first block of E$E_new (prev_randao threshold-verified, byte-identical across all ${#NODES[@]} nodes incl v5+full-node, ACTIVE_LINE growing on stayers ${PROBE_STAYERS[*]}); \
stable epoch E$E_s > E_new carried E_new's key forward (getEpochBeaconKey == E_new, carry-forward on read) while the threshold beacon stayed live"
