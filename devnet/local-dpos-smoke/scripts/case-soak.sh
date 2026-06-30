#!/usr/bin/env bash
# ============================================================================
# case-soak.sh — long-running DPoS production survivability soak (plan
# dpos_production_soak_smoke). ONE unified beacon-ON dynamic profile: a generated
# N-validator production-path stack, seeded-stochastic churn interleaved with
# clean epochs under tx load, a DKG-window-aware safety gate, a continuous
# invariant battery, the marked quorum-loss probe, and a replayable failure
# bundle. Standalone (`make smoke-soak` / `make smoke-soak-quick`), NOT in run-all.
#
# Reuses lib.sh + asserts-fault.sh by FUNCTION NAME (plan D2). The bring-up is a
# soak-specific GENERALIZATION of pp_bring_up_rotation (which is hardcoded to 6
# validators / committee-of-5: `for i in 0 1 2 3 4`, `--peers 6`, literal
# COMPOSE_FILE) — it cannot be N-parameterized by variable override alone, so the
# phases are mirrored here for N (DEVIATION from "pure reuse", flagged in plan §2.2).
# ============================================================================
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=/dev/null
source scripts/lib.sh
# shellcheck source=/dev/null
source scripts/asserts-fault.sh
# shellcheck source=/dev/null
source scripts/soak-prng.sh
# shellcheck source=/dev/null
source scripts/soak-actions.sh
# shellcheck source=/dev/null
source scripts/soak-invariants.sh
# shellcheck source=/dev/null
source scripts/soak-bundle.sh

# ── Knobs (plan §6). SOAK_QUICK overlays the quick column. ───────────────────
SOAK_QUICK="${SOAK_QUICK:-0}"
if [[ "$SOAK_QUICK" == 1 ]]; then
    SOAK_DURATION="${SOAK_DURATION:-5m}"; SOAK_VALIDATORS="${SOAK_VALIDATORS:-4}"
    SOAK_EPOCH_INTERVAL="${SOAK_EPOCH_INTERVAL:-32}"; SOAK_CHURN_PERIOD="${SOAK_CHURN_PERIOD:-20s}"
    SOAK_CHECK_PERIOD="${SOAK_CHECK_PERIOD:-5s}"; SOAK_BYZANTINE=0; SOAK_QUORUM_PROBE=0
else
    SOAK_DURATION="${SOAK_DURATION:-0}"; SOAK_VALIDATORS="${SOAK_VALIDATORS:-10}"
    SOAK_EPOCH_INTERVAL="${SOAK_EPOCH_INTERVAL:-600}"; SOAK_CHURN_PERIOD="${SOAK_CHURN_PERIOD:-90s}"
    SOAK_CHECK_PERIOD="${SOAK_CHECK_PERIOD:-10s}"; SOAK_BYZANTINE="${SOAK_BYZANTINE:-1}"
    SOAK_QUORUM_PROBE="${SOAK_QUORUM_PROBE:-1}"
fi
SOAK_INITIAL_COMMITTEE="${SOAK_INITIAL_COMMITTEE:-4}"
# SELF-HEAL spare pool: extra pre-provisioned validator containers (running --dpos,
# UNREGISTERED) beyond the committee target. When a PERMANENT loss (byzantine tombstone
# — un-recoverable, AlreadySlashedForEquivocation) drops the committee below cap, a spare
# is registered as a FRESH honest replacement (idx SOAK_VALIDATORS+) via the proven
# register path — the container is reused, the on-chain identity is fresh. Container count
# stays FIXED (committee target + spares); only the spare-consumption counter advances.
SOAK_SPARES="${SOAK_SPARES:-3}"
SOAK_VAL_CONTAINERS=$(( SOAK_VALIDATORS + SOAK_SPARES ))   # total validator containers
SOAK_CALM_FRACTION="${SOAK_CALM_FRACTION:-0.4}"   # fraction of epochs with zero churn
SOAK_GROW_LAND_DEADLINE="${SOAK_GROW_LAND_DEADLINE:-8}"   # max epochs a join/refill may take to enter the committee (E+3 land + margin; refills boot a FRESH spare that syncs mid-run) before it's flagged a stalled-growth bug
# SERIALIZE committee-membership changes. The on-CHANGE live DKG for committee[E] is
# dealt by committee[E] during E-1; OVERLAPPING two membership changes (e.g. a growth
# joiner raising the cap AND a byzantine equivocation tombstoning a current member) makes
# committee[E] include members that cannot contribute a valid dealing (the not-yet-ready
# joiner + the byzantine/tombstoned member) → the DKG under-qualifies → NO member gets a
# usable share → all verify-only → beacon DEADLOCK (the chain can't reach a later epoch to
# re-DKG; observed seed=cascadefull1 epoch 6). The consensus correctly refuses to emit a
# beacon from a broken DKG (safety). Real validator-set churn is SERIALIZED governance, so
# a membership change (growth/refill join OR byzantine tombstone) starts only when the
# previous one has fully SETTLED (its tombstone/EffBal landed AND its committee's DKG ran),
# i.e. >= this many epochs after the last one. Keeps every DKG well-formed.
SOAK_MEMBERSHIP_SETTLE="${SOAK_MEMBERSHIP_SETTLE:-4}"
SOAK_BLOCK_INTERVAL=1                              # BLOCK_INTERVAL (consensus 1 blk/s)
SOAK_OUT="${SOAK_OUT:-./soak-out}"
export SOAK_OUT SOAK_LOG_BYTES="${SOAK_LOG_BYTES:-5000000}"

# Seed: derived ONCE + PRINTED so an unseeded run is still replayable (plan §1.1).
SOAK_SEED="${SOAK_SEED:-$(printf '%s' "$(date +%s)-$$" | sha256sum | cut -c1-16)}"
SOAK_PRNG_CTR=0

# D7 alignment: the bash epoch env and the genesis bake share ONE source.
export EPOCH_INTERVAL="$SOAK_EPOCH_INTERVAL" EPOCH_BLOCK_INTERVAL="$SOAK_EPOCH_INTERVAL"
export DPOS_ACTIVATION_BLOCK=$(( 2 * SOAK_EPOCH_INTERVAL ))

# D4: floor from the INITIAL committee, NOT the target (computed ONCE).
INITIAL_F=$(( (SOAK_INITIAL_COMMITTEE - 1) / 3 ))
MIN_COMMITTEE=$(( 3 * INITIAL_F + 1 ))
(( INITIAL_F >= 1 )) || { echo "FAIL: SOAK_INITIAL_COMMITTEE=$SOAK_INITIAL_COMMITTEE < 4 (INITIAL_F must be >=1)"; exit 1; }

# Foundry wrapper (run forge/cast against the sibling contracts repo).
SOLIDITY_CONTRACTS_DIR="${SOLIDITY_CONTRACTS_DIR:-../../../solidity-contracts}"
# DeployStaking writes the manifest via `vm.writeJson`; it MUST land under a path
# foundry's fs_permissions allow (inside the contracts repo's deployments/), like
# the other production-path cases — an out-of-tree path is blocked at runtime.
MANIFEST="$(cd "$SOLIDITY_CONTRACTS_DIR" && pwd)/deployments/runtime-deployment.json"
forge_l2() { ( cd "$SOLIDITY_CONTRACTS_DIR" && "$@" ); }

# Node sets — generated, not hardcoded (plan §2.2 root fix).
# SOAK_VALS = only the actively-running validators (committee target range), NOT the lazy
# spares (idx >= SOAK_VALIDATORS) which stay docker-compose-profile "spare" until a refill
# starts one. So the cold-restart `up`, the convergence reader, and the invariant node set
# all operate on the booted nodes only — spares add zero cold-restart cert-follow load.
SOAK_VALS=(); for ((i=0;i<SOAK_VALIDATORS;i++)); do SOAK_VALS+=("validator-$i"); done
NODES=("${SOAK_VALS[@]}" full-node downstream)   # full-node=L2 cert-follower, downstream=L3 cascade
# Consensus-key pool size for pp_consensus_keys: the soak's pool is N (not the
# production-path 6), so joiners idx 6..N-1 pass run_consensus_keys' `idx < peers`
# assert and get the right (peers-independent) keys. Without this, growth past a
# committee of 6 silently dies on the joiner's setConsensusKeys (empty bls_pub).
export PP_PEERS="$SOAK_VAL_CONTAINERS"
# Scope EVERY `docker compose` (including the reused lib.sh pp_*/check_node
# helpers, which call it BARE) to the soak project via the COMPOSE_FILE env — the
# same convention case-production-path uses. A piecemeal `-f` arg on only SOME
# calls left the bare-helper calls pointing at the default docker-compose.yml
# project (root cause of the silent phase-A exit). Set per-phase in soak_bring_up.
SOAK_BASE_COMPOSE="docker-compose.soak.gen.yml"
SOAK_DPOS_COMPOSE="docker-compose.soak.gen.yml:docker-compose.soak.dpos.gen.yml"
SOAK_PEER_SPOKE="validator-1"

# Generated readers (replace the hardcoded _read_pp_nodes / VALS — plan §2.2).
_read_soak_nodes() {
    local v
    check_external 8545                       # validator-0 (host)
    for v in "${SOAK_VALS[@]:1}"; do check_node docker compose exec -T "$v"; done
    check_external 18545                      # full-node (host)
}

# Orchestrator state.
declare -A RESTORE_AT          # victim -> epoch-deadline to restore
declare -A DISRUPT_KIND        # victim -> action kind (for the right restore)
SOAK_DISRUPTED=""              # space-set of currently-disrupted victims
SOAK_PENDING=""                # space-set "kind@epoch" pending committee changes
SOAK_SHARELESS=0
SOAK_ROUND=0; SOAK_TICK=0
EXPECTED_STALL=0
NEXT_JOINER=$SOAK_INITIAL_COMMITTEE   # next validator index to register/activate
GROW_FIRE_EPOCH=0                      # epoch the last join/refill was fired (landing watchdog)
GROW_LANDED=$(( SOAK_INITIAL_COMMITTEE - 1 ))   # high-water idx confirmed IN the committee (stays landed)
SOAK_TOMBSTONES=0                      # count of PERMANENT losses (byzantine equivocation tombstones); each one earns a spare refill
SETTLE_UNTIL_EPOCH=0                   # epoch until which the last committee-membership change is settling (serialization gate)
PROBE_DONE=0

# register_activate is NOT in this lottery: committee GROWTH is a PLANNED production
# event (governance onboards validators), not a random fault, and the soak's primary
# goal is to actually reach the target committee. It runs on its own deterministic
# track in the churn loop (decoupled from both this lottery and calm/storm), so growth
# fires reliably 4→N instead of depending on a 1/|ACTIONS|×non-calm lottery that left
# earlier runs stuck at the initial size (a false green).
ACTIONS=(graceful_stop_restart sigkill_restart cpu_throttle dkg_midwindow_restart \
         liveness_jail delegate_shift)
(( SOAK_BYZANTINE == 1 )) && ACTIONS+=(byzantine_equivocate byzantine_forge_pk)

# ── teardown / interrupt trap (plan §5) ──────────────────────────────────────
INTERRUPTED=0
on_exit() {
    (( INTERRUPTED == 1 )) && { SOAK_VERDICT="interrupted"; soak_event end "interrupted (SIGINT/SIGTERM)"; }
    # Teardown goes THROUGH the COMPOSE_FILE env (no explicit -f) — the same seam
    # every other call uses. Default to the full dpos scope so an early exit (before
    # the per-phase export) still targets the soak project (down removes by project).
    [[ "${SOAK_KEEP_UP:-0}" == 1 ]] || { COMPOSE_FILE="${COMPOSE_FILE:-$SOAK_DPOS_COMPOSE}" docker compose down -v --remove-orphans 2>/dev/null || true; }
    rm -f docker-compose.soak.byz-*.gen.yml 2>/dev/null || true
}
on_signal() { INTERRUPTED=1; exit 130; }
trap on_exit EXIT
trap on_signal INT TERM

fail_bundle() { soak_event invariant_fail "$2"; soak_bundle_dump "$2" "$1"; exit 1; }

# ── helpers reading live committee state for the gate ────────────────────────
committee_size() { local c; c=$(pp_committee "$1" 2>/dev/null); _count "$c"; }
# 0 (true) iff address $1 is a member of the committee for epoch $2. Checks the
# SPECIFIC joiner (token match, space-bounded), so a join's landing is detected
# even while OTHER validators are transiently jailed — unlike a size comparison.
committee_has() { local a="${1,,}" c; c=" $(pp_committee "$2" 2>/dev/null || true) "; [[ "$c" == *" $a "* ]]; }
top_stake_leader() {
    # Highest-stake committee member (WeightedVRF leader) as validator-IDX — best
    # effort; empty if unavailable. (Used only by the SOFT rule 6.)
    echo ""   # populated from getEpochCommitteeWithStakes if needed; soft rule tolerates "".
}

# Parse a duration like "5m"/"90s"/"0" to seconds (0 = unbounded).
to_secs() { case "$1" in *m) echo $(( ${1%m} * 60 ));; *s) echo "${1%s}";; *) echo "$1";; esac; }

# Address→node map (built ONCE post-bring-up): on-chain committee reads return
# lowercase owner addresses; the gate needs the validator-IDX node names.
declare -A ADDR2IDX
build_addr_map() {
    local i a
    for ((i=0;i<SOAK_VAL_CONTAINERS;i++)); do
        a=$(pp_owner_addr "$i" 2>/dev/null | tr 'A-F' 'a-f')
        [[ -n "$a" ]] && ADDR2IDX[$a]="validator-$i"
    done
}
# Map a committee address space-set (pp_committee output) → validator-IDX set.
_incoming_idx_set() {
    local a out=""
    for a in $1; do a=$(tr 'A-F' 'a-f' <<<"$a"); out+="${ADDR2IDX[$a]:-} "; done
    echo "$out"
}

# Measured bare-sequencer block rate (blk/s), set in soak_bring_up after phase-A
# converge; EVERY wall-timeout derives from it so nothing is sized for the quick
# 32-block config. `secs_for_blocks N [margin]` = wall seconds to produce N blocks
# at that rate, ×margin + a 60s fixed-overhead floor.
BARE_RATE=1
secs_for_blocks() { awk -v n="$1" -v r="${BARE_RATE:-1}" -v m="${2:-1.5}" 'BEGIN{printf "%d", (n/r)*m + 60}'; }

# ============================================================================
# Bring-up (N-generalized mirror of pp_bring_up_rotation).
# ============================================================================
soak_bring_up() {
    local L="smoke-soak" i ck bls_pub bls_pop peer addr HEAD ACT ANCHOR

    echo "== generate compose for N=$SOAK_VAL_CONTAINERS (committee target=$SOAK_VALIDATORS + ${SOAK_SPARES} lazy spares) + L2 cert-follower + L3 cascade =="
    bash scripts/gen-soak-compose.sh "$SOAK_VAL_CONTAINERS" "$SOAK_VALIDATORS"
    # Phase A: scope EVERY docker compose (incl. the bare-calling lib.sh helpers)
    # to the generated base via COMPOSE_FILE — else pp_* read the wrong project.
    export COMPOSE_FILE="$SOAK_BASE_COMPOSE"

    echo "== assert byzantine flag parses (feature image) BEFORE scheduling byz =="
    if (( SOAK_BYZANTINE == 1 )); then
        docker compose run --rm --no-deps -T --entrypoint /usr/local/bin/fluent \
            genesis-init node --dpos.byzantine equivocate --help >/dev/null 2>&1 \
            || { echo "FAIL ($L): --dpos.byzantine does not parse — image lacks dpos-devnet-byzantine; rebuild or SOAK_BYZANTINE=0"; exit 1; }
    fi

    echo "== phase A: bare sequencer chain (N=$SOAK_VAL_CONTAINERS) =="
    docker compose up --build -d
    # Converge timeout scales with the container count (staggered boot: more nodes →
    # longer), not the epoch interval (phase A only needs the first few blocks aligned).
    local converge_wait=$(( 180 + SOAK_VAL_CONTAINERS * 20 ))
    _wait_aligned "$converge_wait" "" _read_soak_nodes >/dev/null \
        || { echo "FAIL ($L): bare chain did not converge in ${converge_wait}s"; docker compose logs --tail=120; exit 1; }

    # Measure the REAL bare block rate; every downstream wall-timeout derives from it
    # (+ the epoch geometry) so it SCALES. The activation block is 2×SOAK_EPOCH_INTERVAL,
    # so a constant tuned for the 32-block quick config is ~half what the 600-block
    # config needs (the cause of the full-run activation-timeout failure).
    local _r0 _r1
    _r0=$(finalized_dec); sleep 15; _r1=$(finalized_dec)
    BARE_RATE=$(awk -v a="$_r0" -v b="$_r1" 'BEGIN{d=(b-a)/15.0; if(d<0.2)d=0.2; printf "%.3f", d}')
    echo "  measured bare block rate: ${BARE_RATE} blk/s"

    # NOTE: TOKEN is intentionally NOT local — it must survive the function for the
    # loop's register/delegate actions (a `local`+`export` would vanish on return).
    # STAKING_RT/CHAIN_CONFIG_RT/GOV_ADDR/LIVENESS_RT are likewise globals (below).
    local DEPLOYER_KEY DEPLOYER_ADDR VERIFIER MNEMONIC SPAMMER_KEY SPAMMER_ADDR
    DEPLOYER_KEY="$(pp_owner_key 0)"; DEPLOYER_ADDR="$(pp_owner_addr 0)"
    MNEMONIC="${FLUENT_DPOS_MNEMONIC:-test test test test test test test test test test test junk}"
    SPAMMER_KEY="$(cast wallet private-key --mnemonic "$MNEMONIC" --mnemonic-index 6)"
    SPAMMER_ADDR="$(cast wallet address --mnemonic "$MNEMONIC" --mnemonic-index 6)"
    cast send "$SPAMMER_ADDR" --value 1000000000000000 --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" >/dev/null
    pp_spammer_start "$SPAMMER_KEY" "$DEPLOYER_ADDR"

    echo "== runtime deploy: token + BLS verifier + staking cluster =="
    TOKEN=$(forge_l2 forge create --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --json \
        "contracts/staking/mocks/MockBlendToken.sol:MockBlendToken" | jq -r '.deployedTo')
    VERIFIER=$(forge_l2 forge create --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --json \
        "contracts/libraries/BLS12381Verifier.sol:BLS12381Verifier" | jq -r '.deployedTo')
    [[ "$TOKEN" == 0x* && "$VERIFIER" == 0x* ]] || { echo "FAIL ($L): token/verifier deploy"; exit 1; }
    # NO pre-deploy BLEND transfer. The deployer's pre-DeployStaking prefix is exactly
    # fund-spammer(0)/token(1)/verifier(2), so DeployStaking's internal CREATEs land at the
    # fixed nonces the genesis-init prediction assumes (`--staking-reader-create-nonces=7,13,17`,
    # gen-soak-compose.sh — the transfer-free prefix is one nonce earlier than production-path's
    # 8,14,18). ALL joiners are funded POST-deploy (below); keeping the pre-deploy nonce
    # sequence uninterrupted removes the brittle "exactly one tx at nonce 3" balancing act.
    # The fail-loud staking-reader↔manifest assertion below still guards any residual drift.

    # Pre-register ONLY the initial committee (v0..v$((SOAK_INITIAL_COMMITTEE-1))) as the deploy's
    # initialValidators, with activeValidatorsLength == the initial committee size. The joiners
    # SOAK_INITIAL_COMMITTEE..N-1 are deliberately LEFT OUT so they start NotFound on-chain and
    # soak_register_activate can register them FRESH at runtime (register→Pending→activate→Active
    # →delegate, the proven production-path add-validator flow) + raise the cap by 1 each, so the
    # committee SIZE actually grows 4→5→6… This is the real production "governance adds a validator"
    # scenario. The shared scripts/config/local-dpos-smoke/l2.json hardcodes the production-path
    # 5-of-5 set; DeployStaking's INITIAL_VALIDATORS/INITIAL_STAKES/ACTIVE_VALIDATORS_LENGTH env
    # overrides (config-fallback, same pattern as INITIAL_OWNER) narrow it without forking the file.
    # The addresses are read from the genesis-derived addresses.json (pp_owner_addr) so they match
    # the on-chain committee reads exactly. v0 gets 5e18 (top-stake leader, rule 6 / host RPC); the
    # rest 1e18. The deployer (v0) funds every initial stake via DeployStaking's approve+transfer.
    local iv="" ist="" j
    for ((j=0;j<SOAK_INITIAL_COMMITTEE;j++)); do
        iv+="$(pp_owner_addr "$j"),"
        if (( j == 0 )); then ist+="5000000000000000000,"; else ist+="1000000000000000000,"; fi
    done
    iv="${iv%,}"; ist="${ist%,}"
    NETWORK=local-dpos-smoke/l2 DEPLOYER="$DEPLOYER_ADDR" INITIAL_OWNER="$DEPLOYER_ADDR" \
      STAKING_TOKEN="$TOKEN" OUTPUT_PATH="$MANIFEST" \
      INITIAL_VALIDATORS="$iv" INITIAL_STAKES="$ist" ACTIVE_VALIDATORS_LENGTH="$SOAK_INITIAL_COMMITTEE" \
      forge_l2 forge script scripts/deploy/DeployStaking.s.sol:DeployStaking \
        --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" --broadcast --skip-simulation \
      || { echo "FAIL ($L): DeployStaking"; exit 1; }
    STAKING_RT=$(jq -r '.staking' "$MANIFEST"); CHAIN_CONFIG_RT=$(jq -r '.chain_config' "$MANIFEST")
    GOV_ADDR=$(jq -r '.governance' "$MANIFEST"); LIVENESS_RT=$(jq -r '.liveness_slashing' "$MANIFEST")
    export STAKING_RT CHAIN_CONFIG_RT GOV_ADDR LIVENESS_RT TOKEN

    # Fail LOUD if the pre-written staking-reader.json (predicted CREATE addresses)
    # does not match the actual deploy — otherwise the --dpos cold-start dies with an
    # opaque "evm read call reverted" at a codeless predicted ChainConfig. Mirrors
    # pp_bring_up_rotation's manifest assertion.
    local PRE pair k want got
    PRE=$(pp_runtime_cat staking-reader.json)
    for pair in "staking_address:$STAKING_RT" "chain_config_address:$CHAIN_CONFIG_RT" "liveness_slashing_address:$LIVENESS_RT"; do
        k=${pair%%:*}; want=$(tr 'A-F' 'a-f' <<<"${pair#*:}"); got=$(jq -r ".$k" <<<"$PRE" | tr 'A-F' 'a-f')
        [[ "$got" == "$want" ]] || { echo "FAIL ($L): staking-reader $k=$got != deployed $want (deployer nonce drift — realign --staking-reader-create-nonces with DeployStaking's CREATE order)"; exit 1; }
    done

    # Fund ALL joiners AND spares (post-deploy; deployer nonces no longer affect the
    # prediction). idx SOAK_INITIAL_COMMITTEE..VAL_CONTAINERS-1 = growth joiners (4..9)
    # PLUS the self-heal spares (10..12) — spares must be pre-funded so a refill can
    # register them instantly when a tombstone frees a committee slot.
    for ((i=SOAK_INITIAL_COMMITTEE;i<SOAK_VAL_CONTAINERS;i++)); do
        pp_token_transfer "$TOKEN" "$(pp_owner_addr "$i")" "100000000000000000000"
    done

    # Governance voter set scales to the initial committee (lib.sh pp_gov_action).
    export PP_GOV_VOTERS="$SOAK_INITIAL_COMMITTEE"

    echo "== gov: setBlsVerifier + setEpochBlockInterval=$SOAK_EPOCH_INTERVAL + jail length (D7) =="
    pp_gov_action "$CHAIN_CONFIG_RT" "$(cast calldata 'setBlsVerifier(address)' "$VERIFIER")" "setBlsVerifier"
    pp_gov_action "$CHAIN_CONFIG_RT" "$(cast calldata 'setEpochBlockInterval(uint32)' "$SOAK_EPOCH_INTERVAL")" "setEpochBlockInterval"
    # P5: short jail length so the un-jail→re-activate cycle completes in-run.
    pp_gov_action "$CHAIN_CONFIG_RT" "$(cast calldata 'setValidatorJailEpochLength(uint32)' 4)" "setValidatorJailEpochLength" || true

    echo "== setConsensusKeys for the INITIAL committee v0..v$((SOAK_INITIAL_COMMITTEE-1)) =="
    for ((i=0;i<SOAK_INITIAL_COMMITTEE;i++)); do
        ck=$(pp_consensus_keys "$i")
        bls_pub=$(jq -r '.blsPubkeyUncompressed' <<<"$ck"); bls_pop=$(jq -r '.blsPoPUncompressed' <<<"$ck")
        peer=$(jq -r '.peerPubkey' <<<"$ck"); addr=$(jq -r '.validatorAddress' <<<"$ck")
        cast send "$STAKING_RT" "setConsensusKeys(address,bytes,bytes,bytes32)" \
            "$addr" "$bls_pub" "$bls_pop" "$peer" --rpc-url "$RPC" --private-key "$(pp_owner_key "$i")" >/dev/null
    done

    HEAD=$(printf '%d' "$(check_external 8545 | cut -d'|' -f1)")
    # Activation = 2 epochs ahead (one full interval of lead for setup to finish before
    # the sequencer clean-halts; proven by production-path). Kept at +2 (not reduced to
    # +1): for an all-day run the one-time pre-DPoS wait is negligible and +2 is safe at
    # ANY interval, whereas +1 risks too-little setup lead for small intervals.
    ACT=$(( ((HEAD / SOAK_EPOCH_INTERVAL) + 2) * SOAK_EPOCH_INTERVAL ))
    pp_gov_action "$CHAIN_CONFIG_RT" "$(cast calldata 'setDposActivationBlock(uint64)' "$ACT")" "setDposActivationBlock"

    # DERIVED activation wait: (blocks remaining to ACT) / measured rate × margin — the
    # class-fix. At SOAK_EPOCH_INTERVAL=600, ACT=1200 ⇒ ~1800s, NOT the old hardcoded 600s.
    local act_wait; act_wait=$(secs_for_blocks "$(( ACT - HEAD ))" 1.5)
    echo "== wait clean-halt at activation $ACT (up to ${act_wait}s @ ${BARE_RATE} blk/s), then --dpos cold-restart of v0..v$((SOAK_VALIDATORS-1)) =="
    wait_finalized_ge "$ACT" "$act_wait" \
        || { echo "FAIL ($L): sequencer did not reach activation $ACT in ${act_wait}s (head=$(finalized_dec), rate=${BARE_RATE})"; exit 1; }
    _wait_aligned "$converge_wait" "" _read_soak_nodes >/dev/null \
        || { echo "FAIL ($L): not aligned at activation in ${converge_wait}s"; exit 1; }
    ANCHOR=$(check_external 8545 | cut -d'|' -f1)
    # The spares (idx >= SOAK_VALIDATORS) booted in phase A as bare followers, so each now
    # holds a reth datadir synced PAST the runtime staking deploy + activation. STOP them
    # before the --dpos cold-restart so they add ZERO cold-restart cert-follow load (13
    # simultaneous cert-followers froze the L2 executor; committee+L2 alone converges). Each
    # is RESUMED on-demand at refill from its phase-A datadir → a clean bare→dpos cold-start
    # (dpos.rs:742 reads a CODE-FUL ChainConfig past the deploy, not the genesis codeless one
    # that crashed a fresh-from-genesis spare with an ABI buffer-overrun).
    local s
    for ((s=SOAK_VALIDATORS;s<SOAK_VAL_CONTAINERS;s++)); do
        docker compose stop "validator-$s" >/dev/null 2>&1 || true
    done
    # Phase B: add the --dpos overlay to the scope for every subsequent compose call.
    export COMPOSE_FILE="$SOAK_DPOS_COMPOSE"
    docker compose up -d --force-recreate "${SOAK_VALS[@]}" full-node \
        || { echo "FAIL ($L): cold-restart into --dpos"; exit 1; }
    # Cold-restart converge: boot N nodes under --dpos + first DKG + finalize past anchor
    # (boot/N-bound + DKG headroom; not interval-bound — early epochs use the agreed fallback).
    local cold_wait=$(( converge_wait + 90 ))
    _wait_aligned "$cold_wait" "$ANCHOR" _read_soak_nodes >/dev/null \
        || { echo "FAIL ($L): DPoS chain did not converge past anchor $ANCHOR in ${cold_wait}s"; docker compose logs --tail=200; exit 1; }
    # ── L3 cascade: validators → L2 (full-node cert-follower) → L3 (downstream),
    # where L3's ONLY peer is L2 (multi-tier). Captures L2's enode, pins L3 to it, asserts
    # L3 finalizes THROUGH L2 with no validator access. ──
    soak_start_cascade_l3 "$ANCHOR" \
        || { echo "FAIL ($L): L3 cascade did not finalize through L2"; docker compose logs --tail=120 downstream full-node; exit 1; }
    echo "  DPoS soak chain live past anchor $ANCHOR (committee=$SOAK_INITIAL_COMMITTEE, target=$SOAK_VALIDATORS, +${SOAK_SPARES} spares; L2 cert-follower + L3 cascade synced)"
}

# L3 cascade bring-up: capture the L2 cert-follower's (full-node) reth enode, pin L3 to
# it, start L3, and assert it finalizes THROUGH L2 (its sole peer — no validator access).
_read_cascade_node() { check_external 28545; }   # L3 downstream reth RPC (host)
# _enode_pubkey is shared from lib.sh.
soak_start_cascade_l3() {
    local anchor="${1:?soak_start_cascade_l3: anchor required}" pk2 pk3 l2_enode l3_enode l3_wait i
    # admin_nodeInfo's embedded IP is unreliable inside docker — keep only the 128-hex
    # pubkey and rebuild each enode with the node's fixed compose IP + default devp2p 30303.
    # 1. capture L2's enode, pin it for L3's --trusted-peers (shared /runtime volume).
    pk2=$(_enode_pubkey http://localhost:18545)
    [[ "$pk2" =~ ^[0-9a-fA-F]{128}$ ]] || { echo "FAIL: L3 — bad L2 enode pubkey '${pk2:0:20}…'"; return 1; }
    l2_enode="enode://${pk2}@172.20.0.250:30303"
    docker compose exec -T full-node sh -c "printf '%s' '$l2_enode' > /runtime/l2-enode.txt" \
        || { echo "FAIL: L3 — could not write /runtime/l2-enode.txt"; return 1; }
    echo "  L3 cascade: L2 enode captured → starting downstream"
    docker compose up -d downstream || { echo "FAIL: L3 — docker compose up downstream"; return 1; }
    # 2. capture L3's enode (once its RPC is up) and make L2 ACCEPT L3 via
    #    admin_addTrustedPeer — mutual trust WITHOUT either tier dropping --trusted-only
    #    (L2 needs it for its own v0 backfill; dropping it froze L2's EL at the boundary).
    for ((i=0;i<30;i++)); do
        pk3=$(_enode_pubkey http://localhost:28545); [[ "$pk3" =~ ^[0-9a-fA-F]{128}$ ]] && break; sleep 2
    done
    [[ "$pk3" =~ ^[0-9a-fA-F]{128}$ ]] || { echo "FAIL: L3 — bad L3 enode pubkey '${pk3:0:20}…' (downstream RPC up?)"; return 1; }
    l3_enode="enode://${pk3}@172.20.0.251:30303"
    cast rpc --rpc-url http://localhost:18545 admin_addTrustedPeer "$l3_enode" >/dev/null 2>&1 \
        || { echo "FAIL: L3 — admin_addTrustedPeer(L3) on L2"; return 1; }
    echo "  L3 cascade: L2 now trusts L3 (mutual) → awaiting L3 finalize via L2"
    # 3. L3 must finalize THROUGH L2 (its only peer — consensus via L2 WS, EL via L2 devp2p).
    l3_wait=$(( 150 + SOAK_VAL_CONTAINERS * 15 ))
    _wait_aligned "$l3_wait" "$anchor" _read_cascade_node >/dev/null \
        || { echo "FAIL: L3 cascade did not finalize past $anchor via L2 in ${l3_wait}s"; return 1; }
    echo "  L3 cascade synced past anchor $anchor through L2 (validators→L2→L3)"
    # Route a tx stream THROUGH the sentry cascade (the real client path): submit
    # to L3's RPC (28545); the write-path gossips L3→L2→validator into the proposer
    # pool. Distinct key (mnemonic index 7) so it never nonce-races the v0 spammer
    # (index 6). Implicit inclusion = its ON-CHAIN (v0) nonce advances — asserted by
    # the write-path-liveness invariant (Inv 9). Fund it via v0; L3 picks up the
    # balance once it syncs the funding block (the spammer loop retries until then).
    local _mnem="${FLUENT_DPOS_MNEMONIC:-test test test test test test test test test test test junk}"
    L3_SPAMMER_KEY="$(cast wallet private-key --mnemonic "$_mnem" --mnemonic-index 7)"
    L3_SPAMMER_ADDR="$(cast wallet address --mnemonic "$_mnem" --mnemonic-index 7)"
    export L3_SPAMMER_ADDR
    cast send "$L3_SPAMMER_ADDR" --value 100000000000000 --rpc-url "$RPC" \
        --private-key "$(pp_owner_key 0)" >/dev/null 2>&1 || true
    pp_spammer_start "$L3_SPAMMER_KEY" "$(pp_owner_addr 0)" "http://localhost:28545"
    echo "  L3 write-path spammer started: tx → L3:28545 → L2 → validator proposer (sender $L3_SPAMMER_ADDR)"
    return 0
}

# ── disruption bookkeeping ───────────────────────────────────────────────────
mark_disrupted()   { SOAK_DISRUPTED="$SOAK_DISRUPTED $1"; SOAK_DISRUPTED="$(echo $SOAK_DISRUPTED)"; }
unmark_disrupted() { SOAK_DISRUPTED="$(echo "${SOAK_DISRUPTED// $1/ }" | sed "s/^$1 //;s/ $1\$//;s/^$1\$//" )"; SOAK_DISRUPTED="$(echo $SOAK_DISRUPTED)"; }
add_pending()      { SOAK_PENDING="$(echo "$SOAK_PENDING $1")"; }

# restore any disruptions whose epoch deadline has passed
process_restores() {
    local cur="$1" v
    for v in "${!RESTORE_AT[@]}"; do
        (( cur >= RESTORE_AT[$v] )) || continue
        case "${DISRUPT_KIND[$v]}" in
            graceful_stop_restart|dkg_midwindow_restart) act_graceful_start "$v" ;;
            sigkill_restart)                              act_sigkill_start "$v" ;;
            cpu_throttle)                                 act_cpu_restore "$v" ;;
            liveness_jail)                               soak_unjail_reactivate "$v" ;;   # P5 un-jail → re-activate
            byzantine_forge_pk)                          act_byzantine_restore "$v" ;;     # recreate honest (drop byz overlay)
        esac
        SOAK_VERDICT="recover"; SOAK_APPLIED="restore"; soak_event recover "restored $v (${DISRUPT_KIND[$v]})"
        unmark_disrupted "$v"; unset 'RESTORE_AT[$v]' 'DISRUPT_KIND[$v]'
    done
}

# ── the apply dispatch (gate already said safe). EXTRACTED from the loop so it is
# dry-runnable through the REAL dispatch (selfcheck_dispatch) under set -u — a
# missing positional/var in ANY arm aborts loudly at startup, not hours in. Reads
# globals: action, victim, cur_epoch, down_epochs. (Committee growth is NOT here —
# it runs on its own deterministic track in the main loop, not via this lottery.)
apply_action() {
    SOAK_VERDICT="applied"; SOAK_APPLIED="$action"
    APPLIED_MSG="APPLIED $action $victim (restore@~epoch $(( cur_epoch + down_epochs )))"
    case "$action" in
        graceful_stop_restart) act_graceful_stop "$victim"; mark_disrupted "$victim"; DISRUPT_KIND[$victim]=$action; RESTORE_AT[$victim]=$(( cur_epoch + down_epochs )) ;;
        sigkill_restart)       act_sigkill_stop "$victim"; mark_disrupted "$victim"; DISRUPT_KIND[$victim]=$action; RESTORE_AT[$victim]=$(( cur_epoch + down_epochs )) ;;
        cpu_throttle)          act_cpu_throttle "$victim"; mark_disrupted "$victim"; DISRUPT_KIND[$victim]=$action; RESTORE_AT[$victim]=$(( cur_epoch + down_epochs )) ;;
        dkg_midwindow_restart) act_dkg_midwindow_restart "$victim"; SOAK_SHARELESS=$(( SOAK_SHARELESS + 1 )) ;;
        liveness_jail)         act_liveness_jail_begin "$victim"; mark_disrupted "$victim"; DISRUPT_KIND[$victim]=$action; add_pending "jail@$(( cur_epoch + 2 ))"; RESTORE_AT[$victim]=$(( cur_epoch + 7 )) ;;  # drop@E+2, un-jail@~E+7 (jailEpochLength=4)

        delegate_shift)        soak_delegate_shift "$victim"; add_pending "delegate@$(( cur_epoch + 3 ))" ;;
        byzantine_equivocate)  export DPOS_CONVERGE_EXCLUDE="$victim"; act_byzantine "$victim" equivocate; mark_disrupted "$victim"; DISRUPT_KIND[$victim]=$action; SOAK_TOMBSTONES=$(( SOAK_TOMBSTONES + 1 )); SETTLE_UNTIL_EPOCH=$(( cur_epoch + SOAK_MEMBERSHIP_SETTLE )) ;;  # PERMANENT loss (membership change) → earns a spare refill + opens a settle window
        byzantine_forge_pk)    act_byzantine "$victim" forge-beacon-pk; mark_disrupted "$victim"; DISRUPT_KIND[$victim]=$action; RESTORE_AT[$victim]=$(( cur_epoch + 1 )) ;;
    esac
    [[ -n "$APPLIED_MSG" ]] && soak_event churn "$APPLIED_MSG"
}

# Startup set-u flush of the REAL dispatch + restore/probe paths (the MECHANISM the
# task asks for): dry-run apply_action for EVERY action through the actual case, plus
# process_restores for every DISRUPT_KIND, in SUBSHELLS so the bookkeeping mutations
# (SOAK_DISRUPTED / RESTORE_AT / NEXT_JOINER) are discarded. An unbound var in any arm
# aborts the subshell → caught here → exit 1 BEFORE the multi-hour loop. act_*/soak_*/
# soak_event all no-op under SOAK_DRYRUN, so this touches no docker/RPC and is <1s.
selfcheck_dispatch() {
    local a kind
    for a in "${ACTIONS[@]}"; do
        ( SOAK_DRYRUN=1; action="$a"; victim="validator-1"; cur_epoch=5; down_epochs=2
          NEXT_JOINER="$SOAK_INITIAL_COMMITTEE"; apply_action ) \
            || { echo "FAIL: selfcheck_dispatch — apply_action '$a' aborted (set -u / unbound / missing arg)"; exit 1; }
    done
    for kind in graceful_stop_restart sigkill_restart cpu_throttle dkg_midwindow_restart liveness_jail byzantine_forge_pk; do
        ( SOAK_DRYRUN=1; declare -A RESTORE_AT=([validator-1]=0) DISRUPT_KIND=([validator-1]="$kind")
          process_restores 99 ) \
            || { echo "FAIL: selfcheck_dispatch — process_restores '$kind' aborted (set -u / unbound)"; exit 1; }
    done
    echo "  self-check: real dispatch + restore paths pass the set-u dry-run"
}

# ============================================================================
# Main
# ============================================================================
mkdir -p "$SOAK_OUT"; : >"$SOAK_OUT/events.jsonl"
echo "soak seed: $SOAK_SEED   (replay: SOAK_SEED=$SOAK_SEED ...)"
SOAK_VERDICT="start"; soak_event start "N=$SOAK_VALIDATORS init=$SOAK_INITIAL_COMMITTEE epoch=$SOAK_EPOCH_INTERVAL MIN_COMMITTEE=$MIN_COMMITTEE byz=$SOAK_BYZANTINE probe=$SOAK_QUORUM_PROBE"

soak_bring_up
build_addr_map
soak_selfcheck      # FLUSH per-action set-u / arg-contract / same-line-local traps
selfcheck_dispatch  # FLUSH the REAL apply dispatch + restore arms (set -u, subshell-isolated)

DUR_SECS=$(to_secs "$SOAK_DURATION"); START=$(date +%s)
LAST_CHURN=0; CHURN_SECS=$(to_secs "$SOAK_CHURN_PERIOD"); CHECK_SECS=$(to_secs "$SOAK_CHECK_PERIOD")
calm_permille=$(awk -v f="$SOAK_CALM_FRACTION" 'BEGIN{printf "%d", f*1000}')

while :; do
    now=$(date +%s)
    (( DUR_SECS > 0 && now - START >= DUR_SECS )) && { SOAK_VERDICT="end"; soak_event end "duration reached"; break; }

    cur_epoch=$(pp_current_epoch); SOAK_CUR_EPOCH="$cur_epoch"
    n=$(committee_size "$cur_epoch"); (( n == 0 )) && n="$SOAK_INITIAL_COMMITTEE"
    SOAK_CUR_F=$(( (n - 1) / 3 ))
    SOAK_CUR_COMMITTEE="$(pp_committee "$cur_epoch" 2>/dev/null || true)"

    process_restores "$cur_epoch"

    # ── invariant battery at the check cadence ──
    SOAK_TICK=$(( SOAK_TICK + 1 ))
    if check_invariants; then
        SOAK_VERDICT="ok"; SOAK_INTENDED=""; SOAK_APPLIED=""
        (( SOAK_TICK % 6 == 0 )) && soak_event invariant_ok "battery green (n=$n f=$SOAK_CUR_F up=$(_count "$SOAK_DISRUPTED") disrupted)"
    else
        fail_bundle "$INV_FAIL_ID" "$INV_FAIL_MSG"
    fi

    # ── churn cadence ──
    if (( now - LAST_CHURN >= CHURN_SECS )); then
        LAST_CHURN="$now"; SOAK_ROUND=$(( SOAK_ROUND + 1 ))
        [[ -n "${SOAK_STOP_ROUND:-}" ]] && (( SOAK_ROUND > SOAK_STOP_ROUND )) && { SOAK_VERDICT="end"; soak_event end "SOAK_STOP_ROUND reached"; break; }

        # ── deterministic committee GROWTH + self-heal REFILL track (own cadence, BEFORE
        # the fault lottery, INDEPENDENT of calm/storm): onboarding/replacing a validator is a
        # PLANNED governance event, not a fault. ONE at a time — fire the next only once the
        # PREVIOUS landed (HIGH-WATER GROW_LANDED: once seen in the committee it STAYS landed,
        # so a later jail/kill of a just-joined validator can't reopen its watchdog). A
        # join/refill that NEVER lands within the deadline fail_bundles (no silent under-
        # strength). GROWTH (NEXT_JOINER<target) registers WITH a cap-raise (committee 4→
        # target). REFILL (NEXT_JOINER>=target, once a PERMANENT loss freed a slot:
        # refills_done < SOAK_TOMBSTONES, spares remain) starts a LAZY spare container + registers
        # it WITHOUT a cap-raise → committee returns to cap. ──
        if (( NEXT_JOINER - 1 > GROW_LANDED )) \
           && committee_has "$(pp_owner_addr $(( NEXT_JOINER - 1 )))" "$cur_epoch"; then
            GROW_LANDED=$(( NEXT_JOINER - 1 ))   # high-water: confirmed in committee, locked landed
        fi
        # A membership change must run with a CLEAN committee (no CURRENT member disrupted) so
        # its on-change DKG qualifies; paired with the settle-window fault suppression below,
        # this guarantees exactly ONE perturbation (the joiner itself) in any DKG window. A
        # tombstoned ex-member that already LEFT the committee does NOT block (it isn't a dealer).
        membership_clean=1
        for _d in $SOAK_DISRUPTED; do
            committee_has "$(pp_owner_addr "${_d##*-}")" "$cur_epoch" && { membership_clean=0; break; }
        done
        if (( NEXT_JOINER > SOAK_INITIAL_COMMITTEE && NEXT_JOINER - 1 > GROW_LANDED )); then
            # the last fired join/refill has not yet landed → watchdog it, then fall through.
            if (( cur_epoch - GROW_FIRE_EPOCH > SOAK_GROW_LAND_DEADLINE )); then
                fail_bundle growth "validator-$(( NEXT_JOINER - 1 )) did not enter the committee within $SOAK_GROW_LAND_DEADLINE epochs of activation (fired@$GROW_FIRE_EPOCH now@$cur_epoch) — growth/refill stalled"
            fi
            :   # explicit success so set -e never trips on the inner-if status
        elif (( NEXT_JOINER < SOAK_VALIDATORS && cur_epoch >= SETTLE_UNTIL_EPOCH && membership_clean == 1 )); then
            # GROWTH: previous landed (or first), below target, no OTHER membership change still
            # settling, AND a clean committee (serialize: one membership change per CLEAN DKG
            # window) → register WITH cap-raise.
            SOAK_INTENDED="register_activate:validator-$NEXT_JOINER"
            soak_register_activate "$NEXT_JOINER" 1 \
                || fail_bundle growth "register_activate FAILED for validator-$NEXT_JOINER — committee growth broken"
            GROW_FIRE_EPOCH="$cur_epoch"; SETTLE_UNTIL_EPOCH=$(( cur_epoch + SOAK_MEMBERSHIP_SETTLE )); add_pending "register@$(( cur_epoch + 3 ))"
            SOAK_VERDICT="applied"; SOAK_APPLIED="register_activate"
            soak_event churn "APPLIED register_activate validator-$NEXT_JOINER (committee $n->$((n+1)) lands @~epoch $(( cur_epoch + 3 )))"
            NEXT_JOINER=$(( NEXT_JOINER + 1 ))
            continue
        elif (( NEXT_JOINER < SOAK_VAL_CONTAINERS && NEXT_JOINER - SOAK_VALIDATORS < SOAK_TOMBSTONES && cur_epoch >= SETTLE_UNTIL_EPOCH && membership_clean == 1 )); then
            # SELF-HEAL REFILL: a permanent loss (tombstone) freed a slot, a spare remains, the
            # triggering tombstone has fully SETTLED, AND a clean committee (serialize: the refill
            # join must not overlap any other committee-transition DKG window). RESUME the spare
            # (stopped before cold-restart, datadir synced past the staking deploy) under --dpos:
            # --no-deps so its linear depends_on chain can't pull a stopped sibling, --force-
            # recreate so the bare→--dpos command swap applies. Its datadir is past the deploy
            # → clean bare→dpos cold-start (no genesis codeless-ChainConfig crash). Then register
            # WITHOUT a cap-raise → committee→cap.
            SOAK_INTENDED="refill:validator-$NEXT_JOINER"
            docker compose up -d --no-deps --force-recreate "validator-$NEXT_JOINER" \
                || fail_bundle growth "refill: could not resume spare validator-$NEXT_JOINER"
            soak_register_activate "$NEXT_JOINER" 0 \
                || fail_bundle growth "refill register_activate FAILED for spare validator-$NEXT_JOINER — self-heal broken"
            GROW_FIRE_EPOCH="$cur_epoch"; SETTLE_UNTIL_EPOCH=$(( cur_epoch + SOAK_MEMBERSHIP_SETTLE )); add_pending "refill@$(( cur_epoch + 3 ))"
            SOAK_VERDICT="applied"; SOAK_APPLIED="refill"
            soak_event churn "APPLIED refill validator-$NEXT_JOINER (replaces a tombstoned member; committee→cap @~epoch $(( cur_epoch + 3 )))"
            NEXT_JOINER=$(( NEXT_JOINER + 1 ))
            continue
        fi

        # ── SERIALIZE: protect the settling membership change's on-CHANGE DKG. During a settle
        # window (a growth join / refill / byzantine tombstone settling), suppress the ENTIRE
        # fault lottery — ANY member disruption (dkg_midwindow_restart, stop, throttle, jail,
        # byzantine) concurrent with a committee-membership change perturbs that change's DKG and
        # can push the qualifying dealer set below quorum → 0 usable shares → all verify-only →
        # beacon DEADLOCK (observed: dkg_midwindow_restart during a growth join, seed capstone2
        # epoch 5; byzantine during growth, cascadefull1). rule 4c gated only byzantine; THIS
        # gates EVERY fault, so the on-change DKG always sees a clean, undisturbed committee.
        if (( cur_epoch < SETTLE_UNTIL_EPOCH )); then
            SOAK_VERDICT="skipped"; SOAK_APPLIED=""
            soak_event churn "settle-window until epoch $SETTLE_UNTIL_EPOCH — no fault churn (membership-change DKG protected)"
            continue
        fi

        # FIXED 4-draw order, all consumed unconditionally (plan §1.1). The PRNG
        # writes SOAK_RAND and is called DIRECTLY — NEVER `$(next_u32)`, which would
        # lose the counter increment in the command-substitution subshell.
        next_u32;                      local_delay=$SOAK_RAND
        next_mod "${#ACTIONS[@]}";     aidx=$SOAK_RAND
        next_mod "$SOAK_VALIDATORS";   vidx=$SOAK_RAND
        next_u32;                      aparam=$SOAK_RAND
        action="${ACTIONS[$aidx]}"; victim="validator-$vidx"
        SOAK_INTENDED="$action:$victim"

        # calm/storm: epoch calm bit derived from (seed,epoch) — deterministic.
        is_calm=0
        if (( cur_epoch <= 2 )); then is_calm=1
        else
            cb=$(( $(printf '%d' "0x$(printf '%s' "$SOAK_SEED:calm:$cur_epoch" | sha256sum | cut -c1-8)") % 1000 ))
            (( cb < calm_permille )) && is_calm=1
        fi

        # DEBUG self-heal trigger (off unless SOAK_FORCE_BYZANTINE_EPOCH set): deterministically
        # force a byzantine_equivocate on a live committee member at/after that epoch (post-growth),
        # until SOAK_FORCE_BYZANTINE_COUNT tombstones have applied — so the tombstone→refill
        # self-heal is EXERCISED without waiting for the rare organic byzantine draw. Still fully
        # gated (rule-4b floor, rule-4c serialize); overrides the calm bit so it actually fires.
        if [[ -n "${SOAK_FORCE_BYZANTINE_EPOCH:-}" ]] \
           && (( cur_epoch >= SOAK_FORCE_BYZANTINE_EPOCH && NEXT_JOINER >= SOAK_VALIDATORS \
                 && SOAK_TOMBSTONES < ${SOAK_FORCE_BYZANTINE_COUNT:-1} )); then
            _fbv=""
            for _fba in $SOAK_CUR_COMMITTEE; do
                _fbi="${ADDR2IDX[$_fba]:-}"; [[ -z "$_fbi" || "$_fbi" == validator-0 ]] && continue
                _in_set "$_fbi" "$SOAK_DISRUPTED" && continue; _fbv="$_fbi"; break
            done
            [[ -n "$_fbv" ]] && { action="byzantine_equivocate"; victim="$_fbv"; is_calm=0; SOAK_INTENDED="$action:$victim (FORCED self-heal test)"; }
        fi

        # ── quorum-loss probe scheduling (full runs, once, in TRUE STEADY STATE) ──
        # The probe drops f+1 of the live committee to force a quorum-loss STALL, then
        # restores and asserts RECOVERY. It MUST fire only when the committee is fully
        # settled, because a stalled chain that ALSO carries a pending perturbation does
        # not resume — the first attempt fired while a join was still settling (cap-raise
        # pending at E+3, beyond the cur+1 GA_CHANGE_IMMINENT horizon) and the restored
        # chain never recovered (false RECOVERY-FAILURE). Steady state =
        #   (a) growth COMPLETE: all joins fired (NEXT_JOINER==N) AND the last LANDED
        #       (NEXT_JOINER-1==GROW_LANDED) ⇒ no cap-raise in flight; AND
        #   (b) NO disrupted validator is a CURRENT committee member (transient faults
        #       drained). A tombstoned ex-member that already LEFT the committee does NOT
        #       block (else one equivocation would suppress the probe for the whole run).
        probe_ready=0
        if (( SOAK_QUORUM_PROBE == 1 && PROBE_DONE == 0 )) \
           && (( NEXT_JOINER == SOAK_VALIDATORS && NEXT_JOINER - 1 == GROW_LANDED )); then
            probe_ready=1
            for _d in $SOAK_DISRUPTED; do
                committee_has "$(pp_owner_addr "${_d##*-}")" "$cur_epoch" && { probe_ready=0; break; }
            done
        fi
        if (( probe_ready == 1 )); then
            GA_ACTION="quorum_probe"; GA_CHANGE_IMMINENT=0
            nxt="$(pp_committee "$((cur_epoch+1))" 2>/dev/null || true)"
            [[ -n "$nxt" && "$nxt" != "$SOAK_CUR_COMMITTEE" ]] && GA_CHANGE_IMMINENT=1
            if gate_accept; then
                mapfile -t pv < <(printf '%s\n' "${SOAK_VALS[@]:1}" | head -n $(( SOAK_CUR_F + 1 )))
                if run_quorum_probe "$n" "${pv[@]}"; then PROBE_DONE=1
                else fail_bundle quorum-probe "probe did not stall-then-recover safely"; fi
                continue
            fi
        fi

        if (( is_calm == 1 )); then
            SOAK_VERDICT="skipped"; SOAK_APPLIED=""; soak_event churn "calm epoch $cur_epoch — no churn (intended $SOAK_INTENDED)"
            continue
        fi

        # ── populate the PURE gate inputs from live state ──
        nxt="$(pp_committee "$((cur_epoch+1))" 2>/dev/null || true)"
        GA_ACTION="$action"; GA_VICTIM="$victim"; GA_N="$n"
        GA_DISRUPTED="$SOAK_DISRUPTED"; GA_INCOMING="$(_incoming_idx_set "$nxt")"
        GA_CHANGE_IMMINENT=0; [[ -n "$nxt" && "$nxt" != "$SOAK_CUR_COMMITTEE" ]] && GA_CHANGE_IMMINENT=1
        GA_SHARELESS="$SOAK_SHARELESS"; GA_MIN_COMMITTEE="$MIN_COMMITTEE"
        # A refill is "in flight" iff a join has been fired but not yet landed in the
        # committee (NEXT_JOINER > live n). Precise + self-clearing — unlike the old
        # append-only SOAK_PENDING *register@* marker, which once set stayed set for
        # the whole run and would let rule 4 jail below MIN_COMMITTEE indefinitely.
        GA_REFILL=0; (( NEXT_JOINER > n )) && GA_REFILL=1
        GA_PENDING_MIN="$n"   # conservative: projected shrink bounded by gate rule 4/5 admission
        GA_LEADER="$(top_stake_leader)"; GA_ROUND_OTHERS=$(_count "$SOAK_DISRUPTED")
        # 1 iff a committee-membership change (growth/refill join OR a prior byzantine
        # tombstone) is still SETTLING — gate rule 4c rejects byzantine_equivocate then, to
        # serialize membership changes (one per DKG window; see SOAK_MEMBERSHIP_SETTLE).
        GA_MEMBERSHIP_SETTLING=0; (( cur_epoch < SETTLE_UNTIL_EPOCH )) && GA_MEMBERSHIP_SETTLING=1

        if ! gate_accept; then
            SOAK_VERDICT="skipped"; SOAK_APPLIED=""
            soak_event churn "GATE SKIP $action $victim — $GA_REASON"
            continue
        fi

        # ── apply (gate said safe) — through the SAME apply_action the self-check dry-ran ──
        down_epochs=$(( 1 + aparam % 3 ))   # restore depth: within-epoch..deep
        apply_action
    fi

    sleep "$CHECK_SECS"
done

echo "soak finished cleanly (seed $SOAK_SEED, $SOAK_ROUND rounds)"
