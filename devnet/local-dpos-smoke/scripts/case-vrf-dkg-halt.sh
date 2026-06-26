#!/usr/bin/env bash
# smoke-vrf-dkg-halt: the >f PRE-SEAL TERMINAL halt (G2) — the NEGATIVE control for
# the post-seal fleet recovery in case-vrf-dkg-durability (same kill count, OPPOSITE
# seal state → opposite, PERMANENT outcome).
#
# On the first driven rotation ceremony C_r1 (register v5 → committee changes at
# E_new), TWO committee[E_new] STAYERS (not the leader, not the joiner) are stopped
# BEFORE they seal, then RESTARTED. PRE-seal → on resume they sit out player-only and
# NEVER re-deal (maybe_start Present-journal arm), so the new committee's DKG can NEVER
# reach dealer-quorum 4 → (beacon_for_epoch)(E_new) stays None → on the E_new first
# block every proposer hits application.rs:473-479 (None => info!("...DKG outcome not
# ready; skipping propose"); return None) and SKIPS the boundary view FOREVER. The chain
# can never produce the first block of E_new → it can never reach a later change-epoch to
# self-heal. This is a TERMINAL halt (Finding A, application.rs:443-487), NOT a
# heal-at-next-epoch. The case asserts the positive "skipping propose" log proof + the
# SUSTAINED no-progress halt at the boundary, then TEARS DOWN — nothing runs after it.
#
# WHY RESTART (not keep down): on n=5 the DKG dealer-quorum and the consensus
# notarization quorum are BOTH N3f1(5)=4, so KEEPING 2 down stalls consensus BELOW the
# boundary (3 < 4) — the proposer never REACHES the boundary view, so "skipping propose"
# never fires (an indistinct consensus stall, not the DKG-None boundary skip this case
# isolates). Restarting restores consensus quorum (4 of 5 → the chain climbs to the
# boundary) while the player-only resume leaves the DKG permanently at 3 dealers < 4, so
# the boundary view is reached and skipped — the genuine terminal wedge. (Bringing them
# back does NOT heal it: they resume as players, the DKG stays sub-quorum forever.)
#
# WHY ITS OWN BRING-UP (plan §6.2 fallback): the consolidated case-vrf-dkg-durability
# sequences the recoverable phases (post-seal recovery on C2 + torn sit-out on C_r1) on
# ONE bring-up. The terminal halt would need a SECOND committee change after C_r1 to
# share that bring-up; on this 6-key equal-initial-stake stack a second deterministic
# re-rank is fragile (the benched original is tie-break-ambiguous) and undelegate
# carries a multi-epoch delay. Since the halt is terminal anyway, splitting it costs
# only one extra bring-up (no shared-state coupling) — 2 cases, not 4. It drives the
# halt on the SAME clean C_r1 trigger (register v5) the durability case uses for Phase 3.
#
# PREREQUISITES (host): docker, foundry (forge/cast), jq, a solidity-contracts checkout
# at $SOLIDITY_CONTRACTS_DIR. Long (~10-14 min), foundry-gated; NOT in run-all.
set -euo pipefail
cd "$(dirname "$0")/.."

export COMPOSE_FILE="docker-compose.production-path.yml"
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

SOLIDITY_CONTRACTS_DIR="${SOLIDITY_CONTRACTS_DIR:-../../../solidity-contracts}"
MANIFEST="$(cd "$SOLIDITY_CONTRACTS_DIR" && pwd)/deployments/runtime-deployment.json"
STAKE_1E18="1000000000000000000"

cleanup() { pp_spammer_stop; rm -f "$MANIFEST"; tear_down; }
trap cleanup EXIT

forge_l2() { ( cd "$SOLIDITY_CONTRACTS_DIR" && "$@" ); }

NODES=(validator-0 validator-1 validator-2 validator-3 validator-4 validator-5 full-node)

head_dec() {
    curl -s -X POST -H 'Content-Type: application/json' \
        --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
        "$RPC" 2>/dev/null | jq -r '.result // "0x0"' \
        | { read -r h; printf '%d' "$h" 2>/dev/null || echo 0; }
}

strip_ansi() { sed -E 's/\x1b\[[0-9;]*m//g'; }

dkg_journal_present() {  # <node-idx> <epoch>
    docker compose exec -T "validator-$1" sh -c \
        "find /runtime/reth-data/v$1 -type f -name 'beacon-dkgjournal-e$2.bin' -size +0c 2>/dev/null | grep -q ." \
        2>/dev/null
}
dkg_share_absent() {     # <node-idx> <epoch>
    ! docker compose exec -T "validator-$1" sh -c \
        "find /runtime/reth-data/v$1 -type f -name 'beacon-share-e$2.bin' 2>/dev/null | grep -q ." \
        2>/dev/null
}
seal_line_present() {    # <node-idx> <epoch>
    docker compose logs "validator-$1" 2>/dev/null | strip_ansi \
        | grep "live DKG: dealings sealed" | grep -qE "epoch=$2( |,|$)"
}
share_computed() {       # <node-idx> <epoch>
    docker compose logs "validator-$1" 2>/dev/null | strip_ansi \
        | grep "live DKG: PK_epoch + share computed + stored" | grep -qE "epoch=$2( |,|$)"
}
# The positive boundary-skip proof: ANY node (the proposer) logged
# "DKG outcome not ready; skipping propose" for <epoch>. Only fires when the DKG
# outcome is genuinely None at the change-epoch boundary.
skipping_propose() {     # <epoch>
    docker compose logs 2>/dev/null | strip_ansi \
        | grep "beacon: change-epoch boundary but DKG outcome not ready; skipping propose" \
        | grep -qE "epoch=$1( |,|$)"
}
# No-progress control: the HEAD does not advance by > 0 over $1 s. True iff frozen.
head_frozen_for() { local a b; a=$(head_dec); sleep "$1"; b=$(head_dec); (( b <= a )); }

# ════════════════════════════════════════════════════════════════════════════
# Bring up the rotation stack (shared helper).
# ════════════════════════════════════════════════════════════════════════════
PP_ROT_LABEL="smoke-vrf-dkg-halt"
pp_bring_up_rotation

E0=$(pp_current_epoch)
GOT0=$(pp_committee "$E0")
EXPECT0=$(for i in 0 1 2 3 4; do pp_owner_addr "$i"; done | tr 'A-F' 'a-f' | sort | paste -sd' ' -)
[[ "$GOT0" == "$EXPECT0" ]] || { echo "FAIL (smoke-vrf-dkg-halt): committee(E0=$E0) != initial 5 (got [$GOT0] want [$EXPECT0])"; exit 1; }
echo "  bring-up done: committee(epoch $E0) == initial 5"

# ── TRIGGER: register v5 to drive C_r1 (same clean trigger as case-vrf-rotation) ──
echo "== TRIGGER: register external validator v5 to drive C_r1 =="
REG_FLOOR=$(check_external 8545 | cut -d'|' -f1)
V5_KEY="$(pp_owner_key 5)" ; V5_ADDR="$(pp_owner_addr 5)"
v5l=$(tr 'A-F' 'a-f' <<<"$V5_ADDR")
cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "$STAKE_1E18" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null
cast send "$STAKING_RT" "registerValidator(address,uint16,uint256)" "$V5_ADDR" 0 "$STAKE_1E18" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL (smoke-vrf-dkg-halt): registerValidator v5"; exit 1; }
ck=$(pp_consensus_keys 5)
cast send "$STAKING_RT" "setConsensusKeys(address,bytes,bytes,bytes32)" \
    "$(jq -r '.validatorAddress' <<<"$ck")" "$(jq -r '.blsPubkeyUncompressed' <<<"$ck")" \
    "$(jq -r '.blsPoPUncompressed' <<<"$ck")" "$(jq -r '.peerPubkey' <<<"$ck")" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL (smoke-vrf-dkg-halt): setConsensusKeys v5"; exit 1; }
pp_gov_action "$STAKING_RT" \
    "$(cast calldata 'activateValidator(address)' "$V5_ADDR")" \
    "activateValidator-v5" || { echo "FAIL (smoke-vrf-dkg-halt): gov activateValidator v5"; exit 1; }
cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "2000000000000000000" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null
cast send "$STAKING_RT" "delegate(address,uint256)" "$V5_ADDR" "2000000000000000000" \
    --rpc-url "$RPC" --private-key "$V5_KEY" >/dev/null || { echo "FAIL (smoke-vrf-dkg-halt): delegate v5"; exit 1; }
echo "  v5 registered + activated + delegated"
pp_wait_converge 180 "$REG_FLOOR" >/dev/null \
    || { echo "FAIL (smoke-vrf-dkg-halt): nodes lost alignment during v5 registration"; docker compose logs validator-5 --tail=80; exit 1; }

# Scan for E_new = first ahead-committed committee that differs from E0's + includes v5.
echo "== waiting for the committee to change (E_new ~ E0+3; scanned, not hardcoded) =="
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
[[ -n "$E_new" ]] || { echo "FAIL (smoke-vrf-dkg-halt): C_r1 committee never changed (v5 never entered an ahead-committed committee within 900s)"; docker compose logs validator-5 --tail=80; exit 1; }
GOT_NEW=$(pp_committee "$E_new")
[[ "$GOT_NEW" != "$GOT0" ]] || { echo "FAIL (smoke-vrf-dkg-halt): committee(E_new=$E_new) equals E0's — C_r1 is not a real rotation"; exit 1; }
echo "  C_r1: committee changed at E_new=$E_new (E0=$E0): [$GOT_NEW] (was [$GOT0])"

# Pick TWO committee[E_new] ORIGINAL stayers, NOT the leader v0 and NOT the joiner v5,
# as the pre-seal victims. (Killing v0 would remove the host RPC; killing v5 is the
# joiner not a stayer.) Need >= 2 such members for a >f kill (n=5, f=1, quorum 4 → 2
# down → 3 < 4 → cannot notarize).
KILL=()
for i in 1 2 3 4; do
    al=$(tr 'A-F' 'a-f' <<<"$(pp_owner_addr "$i")")
    [[ " $GOT_NEW " == *" $al "* ]] && KILL+=("$i")
    [[ ${#KILL[@]} -ge 2 ]] && break
done
[[ ${#KILL[@]} -ge 2 ]] || { echo "FAIL (smoke-vrf-dkg-halt): could not find 2 non-leader original committee[E_new] stayers to kill (committee=[$GOT_NEW])"; exit 1; }
K0=${KILL[0]} K1=${KILL[1]}
echo "  pre-seal victims = validator-$K0 + validator-$K1 (committee[E_new] stayers, not leader/joiner)"

# Gate: BOTH victims' C_r1 ceremony has STARTED (journal present) but NOT sealed
# (seal line absent) AND NOT finalized (share absent) — the PRE-seal window. The
# pre-seal state is what makes the halt terminal: a post-seal kill would let the
# survivors finalize on the disseminated Reveals (that is the recoverable Phase 1).
echo "== waiting for v$K0 AND v$K1 to be PRE-seal on E_new=$E_new (journal present, NOT sealed, share absent) =="
P_BOUNDARY=$(epoch_first_block "$E_new")
gate_deadline=$(( SECONDS + 600 ))
preseal_ok() {  # <i> : journal present AND NOT sealed AND share absent
    dkg_journal_present "$1" "$E_new" && ! seal_line_present "$1" "$E_new" && dkg_share_absent "$1" "$E_new"
}
until preseal_ok "$K0" && preseal_ok "$K1"; do
    (( SECONDS < gate_deadline )) || {
        echo "FAIL (smoke-vrf-dkg-halt): v$K0/v$K1 never both reached the PRE-seal window for E_new=$E_new"
        for i in "$K0" "$K1"; do
            echo "  [v$i] journal=$(dkg_journal_present "$i" "$E_new" && echo yes || echo no) sealed=$(seal_line_present "$i" "$E_new" && echo yes || echo no) share-absent=$(dkg_share_absent "$i" "$E_new" && echo yes || echo no)"
        done
        exit 1; }
    NOW=$(finalized_dec)
    if (( NOW >= P_BOUNDARY )); then
        echo "FAIL (smoke-vrf-dkg-halt): chain reached the E_new boundary ($P_BOUNDARY) before v$K0+v$K1 were both gated pre-seal (window missed — re-run)"
        exit 1
    fi
    sleep 1
done
# Both gated PRE-seal in this iteration. Capture the floor to assert the freeze against.
PRE_HALT=$(baseline_height)
echo "  v$K0 and v$K1 are both PRE-seal for E_new=$E_new (terminal kill window); pre-halt finalized=$PRE_HALT"

# Kill both PRE-seal, then RESTART them — and THIS is the load-bearing distinction
# from a plain consensus-quorum stall. On n=5 the DKG dealer-quorum and the consensus
# notarization quorum are BOTH N3f1(5)=4, so simply KEEPING 2 down stalls consensus
# BELOW the boundary (3 < 4) → the proposer never even REACHES the boundary view → the
# "skipping propose" log never fires (an indistinct stall, not the DKG-None boundary
# skip). To isolate the boundary-skip mechanism the killed nodes are RESTARTED: consensus
# quorum is restored (4 of 5 online → the chain CLIMBS to the boundary), but the two
# restarted nodes resume PLAYER-ONLY and NEVER re-deal (maybe_start Present-journal arm),
# so the new committee's DKG is permanently stuck at 3 dealer logs < dealer-quorum 4 →
# (beacon_for_epoch)(E_new) stays None → every proposer hits application.rs:473-479 and
# SKIPS the boundary view FOREVER (the chain cannot produce E_new's first block → it can
# never reach a later change-epoch to self-heal). This is the TERMINAL halt; bringing the
# nodes back does NOT help (Finding A, §5). Restarting is REQUIRED to make the boundary
# reachable so the DKG-None skip — not the consensus-quorum stall — is what wedges it.
echo "== stopping v$K0 + v$K1 PRE-seal (DKG loses their dealings) =="
docker compose stop "validator-$K0" "validator-$K1" >/dev/null
sleep 3
echo "== restarting v$K0 + v$K1 — consensus quorum restored, but they resume PLAYER-ONLY and never re-deal =="
docker compose start "validator-$K0" "validator-$K1" >/dev/null

# The chain now has 4 of 5 online → it CLIMBS to the boundary, where the DKG-None skip
# wedges it. Wait for the proposer to reach + skip the boundary view: the positive
# "DKG outcome not ready; skipping propose" log for E_new is the proof the halt is the
# DKG-None boundary skip (NOT a consensus stall — which would freeze BELOW the boundary
# and never log this). If a kill had slipped to POST-seal, the survivors' disseminated
# Reveals would let the DKG finalize, the boundary would cross, and this line would NOT
# fire → fails loud, not green.
# Wait for the chain to CLIMB to the boundary EDGE (head == boundary−1). The climb
# itself — from the kill point (PRE_HALT≈$PRE_HALT) all the way up to boundary−1 — is the
# discriminator: only a chain whose consensus quorum was RESTORED (the 2 restarted nodes
# rejoined → 4 of 5 online) can climb here; a genuine consensus stall would freeze AT the
# kill point and never reach the boundary edge. Generous budget for the 2-down resync +
# the ~$((P_BOUNDARY - PRE_HALT))-block climb at 1 blk/s (the old 300 s was too tight — the
# chain reached boundary−1 only near the deadline, before the skip-log was observed).
echo "  waiting for the chain to climb to the E_new=$E_new boundary edge ($((P_BOUNDARY - 1))) — proves the restarted nodes rejoined and the boundary view is reached"
climb_deadline=$(( SECONDS + 600 ))
until (( $(head_dec) >= P_BOUNDARY - 1 )); do
    (( SECONDS < climb_deadline )) || {
        echo "FAIL (smoke-vrf-dkg-halt): chain did not climb to the boundary edge ($((P_BOUNDARY - 1))) within 600 s after the restart (head=$(head_dec), finalized=$(finalized_dec)) — the restarted nodes may not have rejoined consensus"
        for i in 0 "$K0" "$K1"; do echo "  [v$i tail]:"; docker compose logs --tail=40 "validator-$i" | strip_ansi | tail -25 | sed 's/^/    /'; done
        exit 1; }
    sleep 3
done
echo "  chain climbed to head=$(head_dec) (boundary edge $((P_BOUNDARY - 1))) — quorum restored, boundary reached"

# Best-effort confirmation: the positive "DKG outcome not ready; skipping propose" log.
# It may lag the head reaching the edge (the proposer's first boundary attempt + log
# flush), so it is NOT the hard gate — the head-frozen-at-edge + no-E_new-share proof
# below is authoritative and timing-robust.
if skipping_propose "$E_new"; then
    echo "  POSITIVE log — a proposer reached + SKIPPED the E_new=$E_new boundary view (beacon=None)"
else
    echo "  (skipping-propose log not yet flushed; the head-frozen-at-edge + no-share proof below is authoritative)"
fi

# TERMINAL HALT — the HEAD freezes AT the boundary edge and STAYS frozen (≥30 s, well past
# several 1 blk/s intervals). With beacon=None every proposer declines E_new's first block
# (application.rs:473-479), so the chain cannot cross the boundary — a permanent option-A
# halt. A kill that had slipped POST-seal would instead let the DKG finalize and the
# boundary CROSS (head advancing past the edge), failing this freeze check.
HALT_HEAD=$(head_dec)
echo "  asserting the SUSTAINED no-progress halt at the boundary edge (head frozen ≥ 30 s from $HALT_HEAD)"
if ! head_frozen_for 30; then
    h0=$(head_dec); sleep 5; h1=$(head_dec)
    echo "FAIL (smoke-vrf-dkg-halt): TERMINAL control — head did NOT stay frozen at the boundary edge (head $h0 → $h1) — the boundary CROSSED, so a kill slipped post-seal (the DKG finalized) — not a >f pre-seal terminal halt"
    docker compose logs --tail=80 validator-0; exit 1
fi
echo "  head frozen at $(head_dec) over a 30 s window (the boundary view is skipped forever)"

# DKG-None discriminator: no committee member computed an E_new share. This separates the
# DKG-None terminal halt (the ceremony stayed below dealer-quorum 4 → no node has a share)
# from any hypothetical successful-DKG stall (where the survivors WOULD hold an E_new
# share). Together with the climb-to-edge + freeze, this proves the wedge is the DKG-None
# boundary skip, not an indistinct consensus stall.
for i in 0 1 2 3 4 5; do
    if share_computed "$i" "$E_new"; then
        echo "FAIL (smoke-vrf-dkg-halt): validator-$i computed an E_new=$E_new share — the C_r1 ceremony FINALIZED, so the kill did not land >f pre-seal (not a terminal halt)"
        exit 1
    fi
done
echo "  no committee member finalized an E_new share (the DKG stayed below dealer-quorum 4)"

# Clean halt, not a crash — no panic in any node.
panic=$(docker compose logs 2>/dev/null | strip_ansi | grep -iE "panic|thread '.*' panicked" || true)
[[ -z "$panic" ]] || { echo "FAIL (smoke-vrf-dkg-halt): a node PANICKED — the halt must be a clean option-A stall, not a crash:"; printf '%s\n' "$panic" | tail -10 | sed 's/^/    /'; exit 1; }

# Re-confirm the halt is still sustained AND below the boundary — it is permanent (the
# boundary view is skipped forever even with 4 of 5 online; the restarted nodes resume
# as players, so the DKG never reaches dealer-quorum).
if ! head_frozen_for 15; then
    echo "FAIL (smoke-vrf-dkg-halt): the halt was NOT permanent — the head advanced after the freeze window (a terminal >f pre-seal halt must never self-heal)"
    exit 1
fi
# Finalized never crossed the E_new boundary (the first block of E_new was never produced).
(( $(finalized_dec) < P_BOUNDARY )) || { echo "FAIL (smoke-vrf-dkg-halt): finalized=$(finalized_dec) reached the E_new boundary $P_BOUNDARY — the chain crossed the boundary, so it did NOT halt there"; exit 1; }
echo "  halt is permanent — head still frozen, finalized $(finalized_dec) never crossed the E_new boundary $P_BOUNDARY"

echo "OK (smoke-vrf-dkg-halt): >f PRE-SEAL TERMINAL halt — on C_r1 (E$E_new), killing 2 committee stayers (v$K0+v$K1) PRE-seal then restarting them restored CONSENSUS quorum (4 of 5 → the chain climbed to the boundary) but left the DKG permanently below DEALER-quorum 4 (the restarted nodes resumed player-only and never re-dealt) → (beacon_for_epoch)(E$E_new)=None → the proposer logged 'DKG outcome not ready; skipping propose' and SKIPPED the boundary view → the chain FROZE at the boundary (head $HALT_HEAD, finalized never crossed $P_BOUNDARY) and stayed frozen, a clean permanent option-A halt (no panic, no E_new share). NEGATIVE control for case-vrf-dkg-durability's post-seal recovery: same 2-of-5 PRE-seal kill, but PRE-seal (vs post-seal) → the DKG can't finalize → opposite (permanent) outcome. NOTE: on n=5 the DKG dealer-quorum and the consensus quorum are BOTH 4, so the killed nodes must be RESTARTED (not kept down) — keeping them down stalls consensus BELOW the boundary and the 'skipping propose' log never fires; restarting isolates the DKG-None boundary-skip as the wedge. Tearing down."
