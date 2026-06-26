#!/usr/bin/env bash
# smoke-vrf-dkg-durability: consolidated live-DKG mid-window-restart DURABILITY suite
# on ONE rotation-stack bring-up, sequencing TWO DKG ceremonies, each its own phase:
#
#   Phase 1 — POST-SEAL FLEET RECOVERY (G1) on the epoch-2 bootstrap ceremony C2.
#     TWO committee[2] members (v3, v4 — NOT the leader v0) are stopped POST-seal,
#     which drops the cluster below the CONSENSUS quorum (3 online < n−f = 4) so the
#     chain STALLS while they are down (the timing-robust "recovery required" CONTROL),
#     then restarted. Each downed node RESUMES player-only from its on-disk journal,
#     recovers its OWN epoch-2 share, rejoins consensus → 5 ≥ 4 → the boundary crosses,
#     and the beacon prev_randao is byte-identical across all nodes with no slash. The
#     per-victim recovery is the single-victim midwindow property (asserted per victim);
#     the 2-down stall is what makes the recovery non-trivially required.
#
#   Phase 3 — TORN-JOURNAL SIT-OUT (G5) on the first driven rotation ceremony C_r1.
#     Register v5 → committee changes at E_new (the proven case-vrf-rotation trigger).
#     A committee[E_new] member (v3, NOT the leader) has its on-disk ceremony journal
#     CORRUPTED while stopped; on restart it hits the JournalLoad::Torn arm, SITS OUT
#     gracefully (never re-deals → no self-equivocation), and C_r1 finalizes among the
#     other 4 (= quorum) so the chain stays LIVE. v3 re-derives prev_randao from the
#     cert seed like a verify-only node, byte-identical to the survivors.
#
# The >f PRE-SEAL TERMINAL halt (G2) is the NEGATIVE control for Phase 1 (same kill
# count, opposite seal state → permanent boundary wedge). Driving it needs a SECOND
# committee change after C_r1; on this 6-key equal-initial-stake stack a second
# deterministic re-rank is fragile (the benched original is tie-break-ambiguous) and
# undelegate carries a 7-epoch delay, so per the plan's §6.2 fallback it runs as its
# OWN small bring-up in case-vrf-dkg-halt.sh (2 cases, not 4). This case stays
# RECOVERABLE end-to-end (Phase 1 + Phase 3 both restore the cluster).
#
# Runs the PRODUCTION-PATH / rotation stack (runtime forge deploy + 6 validators) —
# the only harness that can rotate the committee (the genesis stack DKGs exactly once,
# epoch 2, then carries the key forward forever: it structurally cannot host a second
# ceremony). The existing genesis-stack case-vrf-dkg-restart-midwindow stays as the
# single-victim + tuned-liveness-slash baseline; this is the rotation-stack durability
# suite that adds the multi-node + torn coverage it under-tests.
#
# PREREQUISITES (host): docker, foundry (forge/cast), jq, a solidity-contracts checkout
# at $SOLIDITY_CONTRACTS_DIR. Long (~12-18 min), foundry-gated; NOT in run-all (like
# smoke-vrf-rotation / smoke-byzantine-vrf / smoke-production-path).
set -euo pipefail
cd "$(dirname "$0")/.."

# Run the production-path stack (own 6-node compose project). lib.sh's `docker compose`
# helpers inherit this.
export COMPOSE_FILE="docker-compose.production-path.yml"
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

SOLIDITY_CONTRACTS_DIR="${SOLIDITY_CONTRACTS_DIR:-../../../solidity-contracts}"
MANIFEST="$(cd "$SOLIDITY_CONTRACTS_DIR" && pwd)/deployments/runtime-deployment.json"
STAKE_1E18="1000000000000000000"

cleanup() { pp_spammer_stop; rm -f "$MANIFEST"; tear_down; }
trap cleanup EXIT

forge_l2() { ( cd "$SOLIDITY_CONTRACTS_DIR" && "$@" ); }

# All deriving nodes on the production-path stack: 6 validators + full-node.
NODES=(validator-0 validator-1 validator-2 validator-3 validator-4 validator-5 full-node)

# Head (latest/tip) block number on the local RPC (decimal). The no-progress CONTROL
# keys off the HEAD, not finalized: with < quorum online the producer cannot even
# build a notarizable block, so the tip freezes — a robust no-event.
head_dec() {
    curl -s -X POST -H 'Content-Type: application/json' \
        --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
        "$RPC" 2>/dev/null | jq -r '.result // "0x0"' \
        | { read -r h; printf '%d' "$h" 2>/dev/null || echo 0; }
}

# ── §2 DKG durability gates (ANSI-greppable; the case strip_ansi's first) ──────
strip_ansi() { sed -E 's/\x1b\[[0-9;]*m//g'; }   # same as case-byzantine-vrf / midwindow

# Per-node generalisations of the midwindow case's validator-3-hardcoded helpers:
# the beacon dir lives under <datadir>/beacon; `find` under the node's datadir tree so
# a reth datadir-layout change can't silently break the gate (same idiom as midwindow).
dkg_journal_present() {  # <node-idx> <epoch> : true iff a non-empty journal exists
    docker compose exec -T "validator-$1" sh -c \
        "find /runtime/reth-data/v$1 -type f -name 'beacon-dkgjournal-e$2.bin' -size +0c 2>/dev/null | grep -q ." \
        2>/dev/null
}
dkg_share_absent() {     # <node-idx> <epoch> : true iff NO share file yet
    ! docker compose exec -T "validator-$1" sh -c \
        "find /runtime/reth-data/v$1 -type f -name 'beacon-share-e$2.bin' 2>/dev/null | grep -q ." \
        2>/dev/null
}
# True iff a NON-EMPTY share file for <epoch> exists on <node-idx> — read via the SHARED
# /runtime volume from a THROWAWAY genesis-init container, so it works whether the target
# validator is RUNNING or STOPPED (`docker compose exec` only works on a running
# container — exec-on-stopped is a silent no-op, which is why file ops on a stopped victim
# MUST go through `run --rm genesis-init`, NOT exec).
share_present_vol() {    # <node-idx> <epoch>
    docker compose run --rm --no-deps -T --entrypoint sh genesis-init -c \
        "find /runtime/reth-data/v$1 -type f -name 'beacon-share-e$2.bin' -size +0c 2>/dev/null | grep -q ." \
        2>/dev/null
}
# Mutate a file under v<node-idx>'s beacon dir while the validator is STOPPED, via a
# throwaway genesis-init container on the same /runtime volume. $3 = an sh snippet with
# $f bound to the matched file path (empty if none). Used for the torn-journal corruption
# (exec-on-stopped is a no-op, §above). Captures + echoes the snippet's stdout, and ALWAYS
# returns 0 (`|| true`) so a bare call / command-substitution can't trip the caller's
# `set -e` when the inner snippet or `docker compose run` exits non-zero (e.g. an absent
# file's `[ -n "$f" ]` short-circuit) — callers verify success by checking the OUTPUT
# (e.g. the corruption's first-bytes readback), not the exit code.
vol_mutate_beacon() {    # <node-idx> <glob> <sh-snippet-using-$f>
    docker compose run --rm --no-deps -T --entrypoint sh genesis-init -c \
        "f=\$(find /runtime/reth-data/v$1 -type f -name '$2' 2>/dev/null | head -1); $3" \
        2>/dev/null || true
}
# Post-seal gate: the new seal log line is present for <epoch> on <node-idx> (the
# ceremony.rs `seal_dealings` success-arm info!). Brackets the post-seal/pre-finalize
# window when combined with dkg_share_absent. Greps the MESSAGE and the epoch FIELD
# SEPARATELY (order-independent, the robust midwindow idiom) so a tracing field/message
# render-order difference can't silently miss the line.
seal_line_present() {    # <node-idx> <epoch>
    docker compose logs "validator-$1" 2>/dev/null | strip_ansi \
        | grep "live DKG: dealings sealed" | grep -qE "epoch=$2( |,|$)"
}
# Finalized a share for <epoch> on <node-idx> (the positive recovery signal).
share_computed() {       # <node-idx> <epoch>
    docker compose logs "validator-$1" 2>/dev/null | strip_ansi \
        | grep "live DKG: PK_epoch + share computed + stored" | grep -qE "epoch=$2( |,|$)"
}
# Torn sit-out line for <epoch> on <node-idx> (the JournalLoad::Torn arm fired). Message
# + epoch field grepped separately (order-independent).
torn_sitout() {          # <node-idx> <epoch>
    docker compose logs "validator-$1" 2>/dev/null | strip_ansi \
        | grep "live DKG: ceremony journal present but unreadable/torn" | grep -qE "epoch=$2( |,|$)"
}
# Count of "ceremony started" lines for <epoch> on <node-idx> (used to assert NO
# re-deal after a torn restart: a re-deal would log a SECOND "ceremony started").
ceremony_started_count() {  # <node-idx> <epoch>
    docker compose logs "validator-$1" 2>/dev/null | strip_ansi \
        | grep -E "live DKG: ceremony started" | grep -cE "epoch=$2( |,|$)" || true
}
# No-progress control (tempo monitor_blocks / ensure_no_progress idiom): the HEAD does
# not advance by > 0 over $1 s. True iff frozen.
head_frozen_for() {      # <seconds>
    local a b; a=$(head_dec); sleep "$1"; b=$(head_dec); (( b <= a ))
}
# On-chain validator status byte (0 inactive,1 pending,2 active,3 jailed,4 exiting) of
# address $1 — read from the RUNTIME-deployed staking ($STAKING_RT, set by the bring-up),
# NOT lib.sh's genesis STAKING_ADDR (which does not exist on the prod-path stack — reading
# it errors and, under set -e, would kill the script at the `$(...)` assignment). Errors
# coerce to "" so the caller's empty-guard handles a transient RPC failure (never set -e).
pp_validator_status() {  # <address>
    cast call "$STAKING_RT" \
        "getValidatorStatus(address)(address,uint8,uint256,uint32,uint64,uint64,uint64,uint16,uint96)" \
        "$1" --rpc-url "$RPC" 2>/dev/null | sed -n '2p' | tr -d ' ' || true
}

# ════════════════════════════════════════════════════════════════════════════
# Bring up the rotation stack (shared helper) → 5-validator committee (v0..v4),
# v5 registered-but-not-yet-committee, beacon threshold-active from the epoch-2
# bootstrap ceremony. Sets ACT, EPOCH_LEN, STAKING_RT, etc.; epoch_first_block().
# ════════════════════════════════════════════════════════════════════════════
PP_ROT_LABEL="smoke-vrf-dkg-durability"
pp_bring_up_rotation

E0=$(pp_current_epoch)
GOT0=$(pp_committee "$E0")
EXPECT0=$(for i in 0 1 2 3 4; do pp_owner_addr "$i"; done | tr 'A-F' 'a-f' | sort | paste -sd' ' -)
[[ "$GOT0" == "$EXPECT0" ]] || { echo "FAIL (smoke-vrf-dkg-durability): committee(E0=$E0) != initial 5 (got [$GOT0] want [$EXPECT0])"; exit 1; }
echo "  bring-up done: committee(epoch $E0) == initial 5; beacon active from the epoch-2 bootstrap"

# ════════════════════════════════════════════════════════════════════════════
# PHASE 1 — POST-SEAL FLEET RECOVERY on C2 (epoch-2 bootstrap), kill 2-of-5  [G1]
#   v3 + v4 (committee[2], not the leader v0) are gated POST-seal (the seal line is
#   present — so their Reveals were disseminated and the DKG is RECOVERABLE, not
#   terminal), then STOPPED so 3 < consensus quorum 4 → chain STALLS (the control), then
#   STARTED → each reloads its OWN epoch-2 share from disk, the chain RESUMES and crosses
#   the boundary (the 2-down rejoin), beacon byte-identical, no slash.
#
#   WHY POST-SEAL (seal line), not the wide journal window: a PRE-seal 2-kill is the
#   TERMINAL halt (the killed dealers never re-deal → the epoch-2 DKG can't reach
#   dealer-quorum 4 → the boundary view is skipped forever — that is case-vrf-dkg-halt).
#   POST-seal, the victims already broadcast their Reveals AND finalized their share
#   (all-in), so the survivors hold a quorum of logs and each victim already persisted
#   `beacon-share-e2.bin` — recoverable. The seal line is MONOTONE (never un-fires) so the
#   gate is wide-open from seal to the boundary — no narrow seal→finalize race (run1's
#   `seal && share-absent` simultaneous gate missed because the all-in finalize writes the
#   share ~1 block after seal).
#
#   RECOVERY MECHANISM = the DURABLE SHARE FILE, not the journal-resume. Because the kill
#   lands POST-finalize (the all-in seal+finalize is ~1 block), the share is already on
#   disk; on restart `build_beacon_plane::load_all` reloads it into the CeremonyStore so
#   the victim rejoins the seed quorum with the EXACT share (§8.11.1). The actor does NOT
#   re-run `maybe_start` for an already-past epoch, so there is no post-restart resume line
#   for epoch 2 — that PRE-finalize journal-resume path is the genesis midwindow case's
#   coverage; here the share's PRESENCE + the byte-identical beacon are the recovery proof.
# ════════════════════════════════════════════════════════════════════════════
echo "== PHASE 1: post-seal fleet recovery on the epoch-2 ceremony (kill v3+v4 post-seal) =="
EPOCH2_START=$(epoch_first_block 2)
P1_BOUNDARY_PROBE=$(( EPOCH2_START + 6 ))

# Gate: BOTH v3 AND v4 have SEALED epoch 2 (the new seal-line product log) AND still
# have a journal on disk (pre-boundary). Seal-line monotone → wide window, no race.
echo "  waiting for v3 AND v4 to SEAL epoch 2 (seal line present + journal on disk) — the recoverable post-seal window"
p1_deadline=$(( SECONDS + 600 ))
until seal_line_present 3 2 && dkg_journal_present 3 2 && seal_line_present 4 2 && dkg_journal_present 4 2; do
    (( SECONDS < p1_deadline )) || {
        echo "FAIL (smoke-vrf-dkg-durability): v3/v4 never both SEALED epoch 2 within the deadline"
        for i in 3 4; do
            echo "  [v$i] seal=$(seal_line_present "$i" 2 && echo yes || echo no) journal=$(dkg_journal_present "$i" 2 && echo yes || echo no)"
            docker compose logs --tail=60 "validator-$i" | strip_ansi | grep -iE 'DKG|ceremony|seal' | tail -20 | sed 's/^/    /' || true
        done
        exit 1; }
    # Safety rail: the epoch-2 ceremony journal lives until the past-boundary sweep, so
    # a present journal can be killed anytime up to the boundary; but if the chain has
    # already crossed the boundary the journal is about to be swept — fail rather than
    # kill late.
    NOW=$(finalized_dec)
    if (( NOW >= EPOCH2_START )); then
        echo "FAIL (smoke-vrf-dkg-durability): chain reached the epoch-2 boundary ($EPOCH2_START) before v3+v4 were both gated post-seal (re-run)"
        exit 1
    fi
    sleep 1
done
echo "  v3 and v4 have both SEALED epoch 2 (recoverable post-seal kill window)"

# CONTROL — the consensus-quorum stall. STOP both so only 3 < quorum 4 are online: the
# producer cannot notarize, so the HEAD must FREEZE. A timing-robust no-event proof the
# down nodes are required. (POST-seal → recoverable; the chain resumes on restart.)
PRE_STALL=$(baseline_height)
echo "  stopping v3 + v4 (3 of 5 online < consensus quorum 4 → chain must STALL); pre-stall finalized=$PRE_STALL"
docker compose stop validator-3 validator-4 >/dev/null
if ! head_frozen_for 12; then
    echo "FAIL (smoke-vrf-dkg-durability): CONTROL — head did NOT freeze with 2 of 5 down (only 3 online, < consensus quorum 4): it advanced. Either the quorum is not n−f=4 here or a down node is still notarizing — the 'recovery required' control is broken"
    docker compose logs --tail=80 validator-0; exit 1
fi
echo "  CONTROL ok — head frozen while 2 of 5 down (consensus quorum 4 not met)"

# Restart both → 5 ≥ quorum 4 → chain RESUMES. Each victim recovers its OWN epoch-2
# share from disk: it sealed + finalized BEFORE the kill (post-seal, all-in), so its
# `beacon-share-e2.bin` was persisted; on restart `build_beacon_plane::load_all` reloads
# it into the CeremonyStore (the DURABLE restart mirror, §8.11.1) so the node rejoins the
# seed quorum with the EXACT same share — it does NOT re-derive a wrong key. (The
# PRE-finalize journal-RESUME path is what the genesis midwindow case covers; here the
# kill is post-finalize, so the durable SHARE FILE — not the journal — is the recovery
# mechanism. The actor does not re-run maybe_start for an already-past epoch, so there is
# no post-restart resume line for epoch 2; the share's PRESENCE + the byte-identical
# beacon are the recovery proof.)
echo "  starting v3 + v4 — they must reload their epoch-2 share from disk and the chain must resume"
docker compose start validator-3 validator-4 >/dev/null

# THE 2-DOWN REJOIN (plan §6.3): the chain must RESUME past the stall and cross the
# epoch-2 boundary once both are back (5 ≥ quorum 4). A failure here would be a real
# 2-simultaneous-down rejoin wedge (existing fault cases only down ONE) — capture it.
wait_finalized_ge "$P1_BOUNDARY_PROBE" 400 >/dev/null || {
    echo "FAIL (smoke-vrf-dkg-durability): chain did not resume + cross the epoch-2 boundary ($P1_BOUNDARY_PROBE) after restarting v3+v4 (a 2-simultaneous-down rejoin wedge — capture before papering over)"
    for i in 0 3 4; do echo "  [v$i tail]:"; docker compose logs --tail=60 "validator-$i" | tail -40 | sed 's/^/    /'; done
    exit 1; }
(( $(finalized_dec) > PRE_STALL )) || { echo "FAIL (smoke-vrf-dkg-durability): finalized did not advance past the pre-stall height $PRE_STALL after restart"; exit 1; }
echo "  2-DOWN REJOIN ok — chain RESUMED and crossed the epoch-2 boundary (finalized now $(finalized_dec) > pre-stall $PRE_STALL); no 2-down wedge"

# Per-victim recovery: each victim still HOLDS its epoch-2 share after the restart (the
# durable reload). Read the share file via the volume (the validator is running again, so
# either exec or the volume reader works; the volume reader is uniform). A share that
# vanished (or a node that re-derived a divergent key) would FAIL the beacon window below.
for i in 3 4; do
    p1_share_deadline=$(( SECONDS + 60 ))
    while (( SECONDS < p1_share_deadline )); do share_present_vol "$i" 2 && break; sleep 2; done
    share_present_vol "$i" 2 || {
        echo "FAIL (smoke-vrf-dkg-durability): v$i has NO epoch-2 share file after the restart — the durable share reload did not recover it (it would be shareless → liveness-slashed)"
        docker compose logs --tail=120 "validator-$i" | strip_ansi | grep -iE 'DKG|share|beacon|load' | tail -30 | sed 's/^/    /'
        exit 1; }
    echo "  v$i RECOVERED — holds its epoch-2 share after the 2-down restart (durable reload)"
done

# Beacon byte-identical across ALL nodes over an epoch-2 window — the recovered shares
# are CONSISTENT (the victims rejoined the seed quorum with the SAME share; varying,
# non-zero, node-agreed). This is the load-bearing correctness check: a victim that
# reloaded a WRONG/missing share would diverge here.
wait_nodes_have "$P1_BOUNDARY_PROBE" 180 || { echo "FAIL (smoke-vrf-dkg-durability): not all nodes reached the epoch-2 window block $P1_BOUNDARY_PROBE after recovery"; exit 1; }
assert_beacon_window "$EPOCH2_START" "$P1_BOUNDARY_PROBE" "phase1 recovered epoch-2"

# No liveness slash on either victim — best-effort on the runtime-deployed default
# config (the hard control is the stall-then-resume + recovery legs above; the rotation
# stack does not carry the tuned EPOCH_BLOCK_INTERVAL>missThreshold geometry). A
# ValidatorSlashed for a victim, or a jailed status, FAILs.
slash_events=$(docker compose logs 2>/dev/null | strip_ansi | grep -iE "ValidatorSlashed|LivenessSlashDispatched" || true)
for i in 3 4; do
    addr=$(pp_owner_addr "$i")
    hit=$(grep -i "${addr#0x}" <<<"$slash_events" || true)
    [[ -z "$hit" ]] || { echo "FAIL (smoke-vrf-dkg-durability): v$i was liveness-slashed despite recovering its share:"; printf '%s\n' "$hit" | sed 's/^/    /'; exit 1; }
    st=$(pp_validator_status "$addr")
    [[ -n "$st" ]] || { echo "FAIL (smoke-vrf-dkg-durability): could not read v$i validator status (empty RPC result) — cannot assert not-jailed (re-run)"; exit 1; }
    [[ "$st" != "3" ]] || { echo "FAIL (smoke-vrf-dkg-durability): v$i is JAILED (status=3) after the post-seal restart"; exit 1; }
done
echo "  PHASE 1 OK — 2-of-5 post-seal kill: chain stalled (3 < quorum 4), both victims recovered their epoch-2 share, beacon byte-identical, no slash"

# ════════════════════════════════════════════════════════════════════════════
# PHASE 3 — TORN-JOURNAL SIT-OUT on C_r1 (first driven rotation), 1 node  [G5]
#   Register v5 → committee changes at E_new. A committee[E_new] member (v3, not the
#   leader) has its E_new ceremony journal corrupted on disk → JournalLoad::Torn → it
#   SITS OUT gracefully (no re-deal, no self-equivocation), C_r1 finalizes among the
#   other 4 = quorum, chain stays LIVE, v3 re-derives prev_randao from the cert.
# ════════════════════════════════════════════════════════════════════════════
echo "== PHASE 3: torn-journal sit-out on the first driven rotation (corrupt v3's journal) =="

# Cluster healthy after Phase 1's recovery? (case-fault.sh hand-off pattern.)
pp_wait_converge 120 >/dev/null || { echo "FAIL (smoke-vrf-dkg-durability): cluster not healthy/converged after Phase 1 — cannot hand off to Phase 3"; docker compose logs --tail=120; exit 1; }

# TRIGGER — register external v5 to rotate the committee (verbatim case-vrf-rotation
# mechanism: register + setConsensusKeys + governance activate + delegate). committee[N]
# reads EffBal(N-1); a delegate is effective in EffBal(E+2) ⇒ entry at E+3 (we SCAN for
# the first changed committee, never hardcode).
echo "  TRIGGER: register external validator v5 to drive C_r1"
REG_FLOOR=$(check_external 8545 | cut -d'|' -f1)
V5_KEY="$(pp_owner_key 5)" ; V5_ADDR="$(pp_owner_addr 5)"
v5l=$(tr 'A-F' 'a-f' <<<"$V5_ADDR")
cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "$STAKE_1E18" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null
cast send "$STAKING_RT" "registerValidator(address,uint16,uint256)" "$V5_ADDR" 0 "$STAKE_1E18" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL (smoke-vrf-dkg-durability): registerValidator v5"; exit 1; }
ck=$(pp_consensus_keys 5)
cast send "$STAKING_RT" "setConsensusKeys(address,bytes,bytes,bytes32)" \
    "$(jq -r '.validatorAddress' <<<"$ck")" "$(jq -r '.blsPubkeyUncompressed' <<<"$ck")" \
    "$(jq -r '.blsPoPUncompressed' <<<"$ck")" "$(jq -r '.peerPubkey' <<<"$ck")" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL (smoke-vrf-dkg-durability): setConsensusKeys v5"; exit 1; }
pp_gov_action "$STAKING_RT" \
    "$(cast calldata 'activateValidator(address)' "$V5_ADDR")" \
    "activateValidator-v5" || { echo "FAIL (smoke-vrf-dkg-durability): gov activateValidator v5"; exit 1; }
cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "2000000000000000000" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null
cast send "$STAKING_RT" "delegate(address,uint256)" "$V5_ADDR" "2000000000000000000" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL (smoke-vrf-dkg-durability): delegate v5"; exit 1; }
echo "  v5 registered + activated + delegated"
pp_wait_converge 180 "$REG_FLOOR" >/dev/null \
    || { echo "FAIL (smoke-vrf-dkg-durability): nodes lost alignment during v5 registration"; docker compose logs validator-5 --tail=80; exit 1; }

# Scan for E_new = the FIRST ahead-committed committee that differs from E0's and
# includes v5 (case-vrf-rotation:294-313). We want v3 (the torn victim) to be a
# committee[E_new] STAYER — assert it after the scan.
echo "  waiting for the committee to change (E_new ~ E0+3 by EffBal arithmetic; scanned, not hardcoded)"
E_new=""
_deadline=$(( $(date +%s) + 900 ))
while (( $(date +%s) < _deadline )); do
    E=$(pp_current_epoch)
    AHEAD=$(pp_committee $((E + 1)))
    if [[ -n "$AHEAD" && " $AHEAD " == *" $v5l "* && "$AHEAD" != "$GOT0" ]]; then
        E_new=$((E + 1)); break
    fi
    sleep 2
done
[[ -n "$E_new" ]] || { echo "FAIL (smoke-vrf-dkg-durability): C_r1 committee never changed (v5 never entered an ahead-committed committee within 900s)"; docker compose logs validator-5 --tail=80; exit 1; }
GOT_NEW=$(pp_committee "$E_new")
[[ "$GOT_NEW" != "$GOT0" ]] || { echo "FAIL (smoke-vrf-dkg-durability): committee(E_new=$E_new) equals E0's — C_r1 is not a real rotation"; exit 1; }
echo "  C_r1: committee changed at E_new=$E_new (E0=$E0): [$GOT_NEW] (was [$GOT0])"

# The torn victim must be a committee[E_new] member (so it RUNS the C_r1 ceremony) and
# NOT the leader v0. Try v3 first (matches the midwindow baseline); if the equal-stake
# tie-break benched v3, pick another non-leader stayer.
V3_ADDR=$(tr 'A-F' 'a-f' <<<"$(pp_owner_addr 3)")
TORN=""
if [[ " $GOT_NEW " == *" $V3_ADDR "* ]]; then
    TORN=3
else
    for i in 1 2 4; do
        al=$(tr 'A-F' 'a-f' <<<"$(pp_owner_addr "$i")")
        [[ " $GOT_NEW " == *" $al "* ]] && { TORN=$i; break; }
    done
fi
[[ -n "$TORN" ]] || { echo "FAIL (smoke-vrf-dkg-durability): no non-leader committee[E_new] stayer available as the torn victim (committee=[$GOT_NEW])"; exit 1; }
echo "  torn victim = validator-$TORN (committee[E_new] member, not the leader)"

# Gate: v$TORN's C_r1 ceremony has STARTED (journal present) but NOT finalized (share
# absent). The journal is written at deal-start and lives the whole deal→boundary span.
echo "  waiting for v$TORN's E_new=$E_new ceremony journal (present) AND share (absent) — the open window"
P3_BOUNDARY=$(epoch_first_block "$E_new")
p3_deadline=$(( SECONDS + 600 ))
until dkg_journal_present "$TORN" "$E_new" && dkg_share_absent "$TORN" "$E_new"; do
    (( SECONDS < p3_deadline )) || {
        echo "FAIL (smoke-vrf-dkg-durability): v$TORN's E_new=$E_new DKG journal never appeared pre-finalize (the C_r1 ceremony never started, or finalized before the poll)"
        docker compose logs --tail=120 "validator-$TORN" | strip_ansi | grep -iE 'DKG|ceremony|journal' | tail -30 | sed 's/^/    /' || true
        exit 1; }
    NOW=$(finalized_dec)
    if (( NOW >= P3_BOUNDARY )); then
        echo "FAIL (smoke-vrf-dkg-durability): chain reached the E_new boundary ($P3_BOUNDARY) before v$TORN's journal was observed (window missed — re-run)"
        exit 1
    fi
    sleep 1
done
echo "  v$TORN's E_new=$E_new ceremony journal is on disk, pre-finalize"

# Stop v$TORN, CORRUPT its E_new journal to JournalLoad::Torn, restart. Recipe
# (share_state.rs:454-482): overwrite the first record's 4-byte big-endian length
# prefix with 0xffffffff (4294967295 ≫ file size) → load_journal hits "truncated record
# body" → first record never decodes → out.is_empty() → Torn (NOT NoFile: the file stays
# non-empty). A NoFile (0-byte) node would instead re-deal → log a second "ceremony
# started", which the no-re-deal assertion below catches. The mutation runs via a
# throwaway genesis-init container on the shared /runtime volume — `docker compose exec`
# only works on a RUNNING container (exec-on-stopped is a silent no-op), so a stopped
# victim's files MUST be touched through `run --rm genesis-init`. OCTAL escapes (\377 =
# 0xff): genesis-init's /bin/sh supports POSIX \ooo octal (printf \xHH hex is a
# bash/coreutils-only extension) — the over-large length prefix is 4 bytes of 0xff.
echo "  stopping v$TORN, corrupting its E_new=$E_new journal to Torn (via the volume), restarting"
docker compose stop "validator-$TORN" >/dev/null
# Defensive: if the share finalized in the gate→stop gap, delete it so the restart
# reaches load_journal (Torn) instead of the store-hit early-return (actor.rs:597). The
# snippets end `; true` so an absent file (the `[ -n "$f" ]` short-circuit) does NOT make
# the inner sh exit non-zero — which under `set -e` would silently kill the script.
vol_mutate_beacon "$TORN" "beacon-share-e$E_new.bin" '[ -n "$f" ] && rm -f "$f"; true' || true
vol_mutate_beacon "$TORN" "beacon-dkgjournal-e$E_new.bin" \
    '[ -n "$f" ] && printf "\377\377\377\377" | dd of="$f" bs=1 count=4 conv=notrunc 2>/dev/null; true' \
    || true
# Verify the corruption actually landed (the first 4 bytes are now 0xff) — a silent
# no-op (e.g. exec-on-stopped, the bug this replaces) must FAIL LOUD, not false-green.
firstbytes=$(vol_mutate_beacon "$TORN" "beacon-dkgjournal-e$E_new.bin" \
    'od -An -tx1 -N4 "$f" 2>/dev/null | tr -d " \n"; true')
[[ "$firstbytes" == "ffffffff" ]] || {
    echo "FAIL (smoke-vrf-dkg-durability): v$TORN's E_new journal corruption did NOT land (first 4 bytes='$firstbytes', want 'ffffffff') — the Torn arm would not fire (a silent file-op no-op)"
    exit 1; }
echo "  corruption verified: v$TORN's E_new journal first 4 bytes = 0xffffffff (will load as Torn)"
docker compose start "validator-$TORN" >/dev/null

# Chain MUST stay LIVE — quorum 4 survives one Torn sit-out (exactly like one kill).
P3_PROBE=$(( P3_BOUNDARY + 6 ))
wait_finalized_ge "$P3_PROBE" 400 >/dev/null || {
    echo "FAIL (smoke-vrf-dkg-durability): CONTROL — chain did NOT stay live across the E_new boundary with one Torn sit-out (4 survivors should be a quorum)"
    docker compose logs --tail=120 validator-0; exit 1; }
wait_nodes_have "$P3_PROBE" 180 || { echo "FAIL (smoke-vrf-dkg-durability): not all nodes reached the E_new window block $P3_PROBE"; exit 1; }
echo "  chain stayed LIVE across the E_new boundary on the 4 survivors"

# v$TORN fired the Torn arm, did NOT re-deal, and is shareless for E_new.
torn_deadline=$(( SECONDS + 120 ))
while (( SECONDS < torn_deadline )); do torn_sitout "$TORN" "$E_new" && break; sleep 3; done
torn_sitout "$TORN" "$E_new" || {
    echo "FAIL (smoke-vrf-dkg-durability): v$TORN did NOT log the Torn sit-out for E_new=$E_new — the corruption did not land in the Torn arm (NoFile? a wrong recipe would re-deal)"
    docker compose logs --tail=160 "validator-$TORN" | strip_ansi | grep -iE 'DKG|torn|journal|ceremony' | tail -40 | sed 's/^/    /'
    exit 1; }
# Exactly ONE "ceremony started" for E_new (the original, pre-corruption run): a Torn
# resume must NOT re-deal (no SECOND start), and a wrong-recipe NoFile would re-deal.
sc=$(ceremony_started_count "$TORN" "$E_new")
(( sc <= 1 )) || {
    echo "FAIL (smoke-vrf-dkg-durability): v$TORN logged $sc 'ceremony started' for E_new=$E_new — it RE-DEALT after the torn restart (self-equivocation risk; the corruption fell to NoFile, not Torn)"
    docker compose logs --tail=160 "validator-$TORN" | strip_ansi | grep "live DKG: ceremony started" | tail -10 | sed 's/^/    /'
    exit 1; }
share_computed "$TORN" "$E_new" && {
    echo "FAIL (smoke-vrf-dkg-durability): v$TORN computed an E_new=$E_new share despite sitting out torn — it should be SHARELESS for E_new"
    exit 1; }
echo "  v$TORN sat out gracefully — Torn arm fired, NO re-deal ($sc 'ceremony started'), shareless for E_new"

# C_r1 finalized among the OTHER 4 committee members (each computed its E_new share).
finalized_members=0
for i in 0 1 2 3 4 5; do
    [[ "$i" == "$TORN" ]] && continue
    al=$(tr 'A-F' 'a-f' <<<"$(pp_owner_addr "$i")")
    [[ " $GOT_NEW " == *" $al "* ]] || continue
    if share_computed "$i" "$E_new"; then
        finalized_members=$(( finalized_members + 1 ))
    else
        echo "FAIL (smoke-vrf-dkg-durability): committee[E_new] member v$i did NOT compute its E_new=$E_new share — C_r1 did not finalize among the survivors"
        docker compose logs --tail=120 "validator-$i" | strip_ansi | grep -iE 'DKG|share|ceremony' | tail -20 | sed 's/^/    /'
        exit 1
    fi
done
(( finalized_members >= 4 )) || { echo "FAIL (smoke-vrf-dkg-durability): only $finalized_members of the non-torn committee[E_new] members finalized E_new (want >= 4 = quorum)"; exit 1; }
echo "  C_r1 finalized among $finalized_members survivors (>= quorum 4)"

# v$TORN re-derives prev_randao from the cert seed like a verify-only node —
# byte-identical to the survivors over the E_new window. (assert_beacon_window compares
# ALL nodes incl v$TORN.)
assert_beacon_window "$P3_BOUNDARY" "$P3_PROBE" "phase3 torn-sitout E$E_new"

# GRACEFUL sit-out: no panic, no equivocation evidence for v$TORN.
panic=$(docker compose logs "validator-$TORN" 2>/dev/null | strip_ansi | grep -iE "panic|thread '.*' panicked" || true)
[[ -z "$panic" ]] || { echo "FAIL (smoke-vrf-dkg-durability): v$TORN PANICKED on the torn journal (should sit out gracefully):"; printf '%s\n' "$panic" | tail -10 | sed 's/^/    /'; exit 1; }
v_torn_addr=$(pp_owner_addr "$TORN")
equiv=$(docker compose logs 2>/dev/null | strip_ansi | grep -iE "ValidatorSlashed|equivocat" | grep -i "${v_torn_addr#0x}" || true)
[[ -z "$equiv" ]] || { echo "FAIL (smoke-vrf-dkg-durability): v$TORN produced equivocation/slash evidence (a torn resume must NOT re-deal):"; printf '%s\n' "$equiv" | sed 's/^/    /'; exit 1; }
echo "  PHASE 3 OK — torn-journal sit-out: v$TORN sat out gracefully (no panic, no re-deal, no equivocation), C_r1 finalized on the 4, chain stayed live, prev_randao byte-identical"

# ── chain still finalizing under the background tx load ───────────────────────
BEFORE=$(baseline_height); sleep 6; AFTER=$(finalized_dec)
(( AFTER > BEFORE )) || { echo "FAIL (smoke-vrf-dkg-durability): chain not finalizing under tx load at the end ($AFTER <= $BEFORE)"; exit 1; }

echo "OK (smoke-vrf-dkg-durability): consolidated live-DKG durability suite on ONE rotation-stack bring-up — \
PHASE 1 (C2 epoch-2): 2-of-5 POST-seal kill (v3+v4) STALLED the chain (3 online < consensus quorum 4), both victims RESUMED player-only from journal (own_log_recorded=true) and recovered their epoch-2 share, beacon byte-identical across all ${#NODES[@]} nodes, no slash; \
PHASE 3 (C_r1 E$E_new): registering v5 rotated the committee, a torn journal on validator-$TORN hit the Torn arm and SAT OUT (no re-deal, shareless), C_r1 finalized among the 4 survivors, chain stayed LIVE, and validator-$TORN re-derived byte-identical prev_randao from the cert seed. \
(The >f pre-seal TERMINAL halt G2 — Phase 1's negative control — runs as case-vrf-dkg-halt.sh per the plan's §6.2 fallback: a clean second deterministic committee change is not drivable on this 6-key equal-stake stack.)"
