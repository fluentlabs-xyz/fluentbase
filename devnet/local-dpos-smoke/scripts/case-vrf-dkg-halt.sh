#!/usr/bin/env bash
# smoke-vrf-dkg-halt: the >f PRE-SEAL TERMINAL halt (G2) — the NEGATIVE control for
# the post-seal fleet recovery in case-vrf-dkg-durability (same kill count, OPPOSITE
# seal state → opposite, PERMANENT outcome).
#
# On the first driven rotation ceremony C_r1 (register v5 → committee changes at
# E_new), TWO committee[E_new] STAYERS (not the leader, not the joiner) are stopped
# BEFORE they seal, have their E_new ceremony JOURNALS TORN on disk, and are restarted.
# Each comes back into maybe_start's JournalLoad::Torn arm and SITS OUT E_new permanently,
# so the new committee's DKG can NEVER reach dealer-quorum 4 → (beacon_for_epoch)(E_new)
# stays None → on the E_new first block every proposer hits application.rs:473-479
# (None => info!("...DKG outcome not ready; skipping propose"); return None) and SKIPS the
# boundary view FOREVER. The chain can never produce the first block of E_new → it can
# never reach a later change-epoch to self-heal. This is a TERMINAL halt (Finding A,
# application.rs:443-487), NOT a heal-at-next-epoch. The case asserts the Torn sit-out on
# BOTH victims, the positive "skipping propose" log proof and the SUSTAINED no-progress
# halt at the boundary, then TEARS DOWN — nothing runs after it.
#
# WHY THE JOURNALS ARE TORN AND NOT MERELY THE NODES KILLED. This case used to just stop
# the two victims and start them again, on the premise that a PRE-seal restart resumes
# player-only. THAT PREMISE IS GONE: maybe_start's Present-journal arm re-derives the
# seeded dealer whenever `last_height < epoch_start(target) − DKG_MARGIN_BLOCKS`
# (beacon/actor.rs:1046-1054), so a victim restarted before the seal deadline SEALS
# NORMALLY, the ceremony finalizes and the boundary CROSSES. The case failed for exactly
# that reason — a harness defect, not a product bug. Only the Torn arm
# (beacon/actor.rs:1056-1069) sits out UNCONDITIONALLY, and `torn_warned` (:998) makes it
# permanent for the epoch. THE TEAR IS STILL NOT SUFFICIENT ON ITS OWN: a victim that had
# already SEALED has broadcast its dealer log and the survivors finalize without it, so
# the tear must land before the SEAL DEADLINE — which is what the pre-seal gate + rail
# below enforce.
#
# WHY RESTART (not keep down): on n=5 the DKG dealer-quorum and the consensus
# notarization quorum are BOTH N3f1(5)=4, so KEEPING 2 down stalls consensus BELOW the
# boundary (3 < 4) — the proposer never REACHES the boundary view, so "skipping propose"
# never fires (an indistinct consensus stall, not the DKG-None boundary skip this case
# isolates). Restarting restores consensus quorum (4 of 5 → the chain climbs to the
# boundary) while the TORN sit-out leaves the DKG permanently at 3 dealers < 4, so the
# boundary view is reached and skipped — the genuine terminal wedge. (Bringing them back
# does NOT heal it: torn_warned is permanent for E_new, the DKG stays sub-quorum forever.)
#
# ANTI-VACUITY: a chain frozen for an unrelated reason must NOT read as a pass. So the
# halt is only claimed after the mechanism is PROVEN — the corruption readback (`od` first
# 4 bytes == ffffffff, fails loud on a silent file-op no-op), the Torn-arm line on BOTH
# victims with NO re-deal, and no committee member holding an E_new share at all. The
# freeze at the boundary edge is then interpreted as the halt of an already-proven
# shareless committee.
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

# strip_ansi hoisted to lib.sh (sourced above) — shared with the soak battery + the other VRF cases.

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
# The JournalLoad::Torn arm's product warn (beacon/actor.rs:1060-1067) — the SIT-OUT proof.
# Message + epoch field grepped SEPARATELY (order-independent: tracing renders fields in an
# order this case does not control).
torn_sitout() {          # <node-idx> <epoch>
    docker compose logs "validator-$1" 2>/dev/null | strip_ansi \
        | grep "live DKG: ceremony journal present but unreadable/torn" | grep -qE "epoch=$2( |,|$)"
}
# Count of "ceremony started" (beacon/actor.rs:1133) for <epoch> on <node-idx>. A SECOND one
# after the torn restart means the node RE-DEALT (self-equivocation — and a re-dealt victim is
# a dealer back in the count, which would let the ceremony finalize).
ceremony_started_count() {  # <node-idx> <epoch>
    docker compose logs "validator-$1" 2>/dev/null | strip_ansi \
        | grep -E "live DKG: ceremony started" | grep -cE "epoch=$2( |,|$)" || true
}
# TEAR v<idx>'s <epoch> ceremony journal so the restart lands in JournalLoad::Torn.
#
# RECIPE (share_state.rs:470-516): load_journal returns Torn iff the file is NON-EMPTY and
# nothing decoded. Overwriting the first record's 4-byte big-endian length prefix with
# 0xffffffff makes `after_len.len() < len` at :495 — "truncated record body" — so the loop
# BREAKS before decode_record ever runs (state-agnostic: plaintext and encrypted framing tear
# identically) and `out` is empty while the file is not. Truncating to 0 bytes is the WRONG
# recipe: :483 maps an empty file to NoFile and the node RE-DEALS.
#
# The SHARE FILE is deleted FIRST: maybe_start returns at actor.rs:982-989 on
# store.contains_key(&target), BEFORE load_journal — a finalized share short-circuits the whole
# thing and the torn journal is never read.
#
# All three ops go through vol_mutate_beacon (lib.sh) — a THROWAWAY container on the shared
# volume — because `docker compose exec` on a STOPPED container SUCCEEDS and does nothing. The
# snippets end `; true` so an absent file's `[ -n "$f" ]` short-circuit cannot kill the script
# under `set -e`. OCTAL escapes (\377): genesis-init's /bin/sh has POSIX \ooo, not \xHH.
tear_journal_to_torn() {  # <node-idx> <epoch>
    local i="$1" e="$2" fb
    vol_mutate_beacon "$i" "beacon-share-e$e.bin" '[ -n "$f" ] && rm -f "$f"; true' >/dev/null || true
    vol_mutate_beacon "$i" "beacon-dkgjournal-e$e.bin" \
        '[ -n "$f" ] && printf "\377\377\377\377" | dd of="$f" bs=1 count=4 conv=notrunc 2>/dev/null; true' \
        >/dev/null || true
    # Verify the corruption ACTUALLY landed — a silent no-op must FAIL LOUD, not false-green:
    # without it the restart takes the normal resume path, the Torn arm never fires, and every
    # later assertion measures a node that was never torn (the case would be vacuous).
    fb=$(vol_mutate_beacon "$i" "beacon-dkgjournal-e$e.bin" \
        'od -An -tx1 -N4 "$f" 2>/dev/null | tr -d " \n"; true')
    [[ "$fb" == "ffffffff" ]] || {
        echo "FAIL (smoke-vrf-dkg-halt): v$i's E$e journal corruption did NOT land (first 4 bytes='$fb', want 'ffffffff') — the Torn arm would not fire (a silent file-op no-op), so the halt this case asserts would be unattributable"
        exit 1; }
    echo "  v$i: E$e journal torn (first 4 bytes = 0xffffffff → JournalLoad::Torn), share removed"
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
# as the pre-seal victims. (Stopping v0 would remove the host RPC; v5 is the joiner, not
# a stayer.) TWO is what puts the DKG out of reach: dealer-quorum on n=5 is N3f1(5)=4, so
# two sit-outs leave 3 < 4. They come straight back, so consensus quorum (also 4) is
# restored and the chain still climbs to the boundary.
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
# (seal line absent) AND NOT finalized (share absent) — the PRE-seal window.
#
# TEARING THE JOURNAL IS NECESSARY BUT NOT SUFFICIENT, and this gate is the "sufficient"
# half. A victim that had already SEALED has broadcast its dealer log, so the survivors
# hold enough logs to finalize no matter what its own journal says afterwards — the tear
# would sit out a node whose contribution is already in flight and the boundary would
# cross. Only a tear taken before the seal deadline actually removes a dealer from the
# count. The journal-present half is load-bearing the other way: with no journal on disk
# there is nothing to tear, load_journal answers NoFile (share_state.rs:483) and the
# restarted node DEALS FRESH — the opposite of a sit-out.
echo "== waiting for v$K0 AND v$K1 to be PRE-seal on E_new=$E_new (journal present, NOT sealed, share absent) =="
P_BOUNDARY=$(epoch_first_block "$E_new")
# The pre-seal window closes at the SEAL DEADLINE, not at the boundary. The ceremony
# seals at `epoch_start(E_new) − DKG_MARGIN_BLOCKS` (beacon/actor.rs:83), so the last
# 20 blocks before the boundary are already POST-seal: a victim torn there has ALREADY
# broadcast its dealer log, the survivors finalize on the disseminated Reveals and the
# boundary CROSSES — precisely the outcome this case exists to exclude, and it would be
# accepted silently. Same env-overridable
# knob and default as asserts-fault.sh:379 / soak-invariants.sh:2230 (the tree's single
# source for the seal margin), so an override moves every consumer together.
: "${SOAK_DKG_MARGIN_BLOCKS:=20}"
SEAL_DEADLINE=$(( P_BOUNDARY - SOAK_DKG_MARGIN_BLOCKS ))
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
    if (( NOW >= SEAL_DEADLINE )); then
        echo "FAIL (smoke-vrf-dkg-halt): chain reached the E_new SEAL DEADLINE ($SEAL_DEADLINE = boundary $P_BOUNDARY − DKG_MARGIN_BLOCKS $SOAK_DKG_MARGIN_BLOCKS) before v$K0+v$K1 were both gated pre-seal — a tear from here is POST-seal: the victims' dealer logs are already broadcast, the survivors finalize without them and the boundary CROSSES, which is the opposite of what this case asserts (window missed — re-run)"
        exit 1
    fi
    sleep 1
done
# Both gated PRE-seal in this iteration. Capture the floor to assert the freeze against.
PRE_HALT=$(baseline_height)
echo "  v$K0 and v$K1 are both PRE-seal for E_new=$E_new (the tear window closes at the seal deadline $SEAL_DEADLINE); pre-halt finalized=$PRE_HALT"

# Stop both PRE-seal, TEAR their journals, then RESTART them — and THIS is the
# load-bearing distinction from a plain consensus-quorum stall. On n=5 the DKG
# dealer-quorum and the consensus notarization quorum are BOTH N3f1(5)=4, so simply
# KEEPING 2 down stalls consensus BELOW the boundary (3 < 4) → the proposer never even
# REACHES the boundary view → the "skipping propose" log never fires (an indistinct stall,
# not the DKG-None boundary skip). To isolate the boundary-skip mechanism the nodes are
# RESTARTED: consensus quorum is restored (4 of 5 online → the chain CLIMBS to the
# boundary), while the TORN journals leave them sitting out E_new permanently, so the new
# committee's DKG is stuck at 3 dealer logs < dealer-quorum 4 → (beacon_for_epoch)(E_new)
# stays None → every proposer hits application.rs:473-479 and SKIPS the boundary view
# FOREVER (the chain cannot produce E_new's first block → it can never reach a later
# change-epoch to self-heal). This is the TERMINAL halt; bringing the nodes back does NOT
# help (Finding A, §5).
#
# The STOP is only the window in which the files can be touched — the sit-out comes from
# the TEAR, not from the downtime (a plain stop/start would re-derive the dealer and seal,
# see the header). BOTH journals are torn while BOTH nodes are down: restarting v$K0
# before v$K1's journal was torn would give it time to reach the seal deadline and seal,
# putting a fourth dealer log back in the count.
echo "== stopping v$K0 + v$K1 PRE-seal (the window in which their journals can be torn) =="
docker compose stop "validator-$K0" "validator-$K1" >/dev/null
echo "== tearing both E_new=$E_new ceremony journals to JournalLoad::Torn (via the volume) =="
for i in "$K0" "$K1"; do tear_journal_to_torn "$i" "$E_new"; done
sleep 3
echo "== restarting v$K0 + v$K1 — consensus quorum restored, but they SIT OUT E_new (Torn) and never re-deal =="
docker compose start "validator-$K0" "validator-$K1" >/dev/null

# THE MECHANISM, asserted BEFORE anything is measured. Without this the case would claim a
# shareless-committee halt from a chain that might simply have lost a peer — a freeze with
# an unrelated cause would read as a pass. Fail here (fast, with the right diagnosis)
# rather than 600 s later at the climb or, worse, green at a freeze that had another cause.
echo "== asserting BOTH victims hit the JournalLoad::Torn arm for E_new=$E_new (the sit-out mechanism) =="
for i in "$K0" "$K1"; do
    torn_deadline=$(( SECONDS + 120 ))
    while (( SECONDS < torn_deadline )); do torn_sitout "$i" "$E_new" && break; sleep 3; done
    torn_sitout "$i" "$E_new" || {
        echo "FAIL (smoke-vrf-dkg-halt): v$i did NOT log the Torn sit-out for E_new=$E_new — the tear did not land in the Torn arm (NoFile? a wrong recipe RE-DEALS and the ceremony finalizes), so this case cannot attribute any halt to a shareless committee"
        docker compose logs --tail=160 "validator-$i" | strip_ansi | grep -iE 'DKG|torn|journal|ceremony' | tail -40 | sed 's/^/    /'
        exit 1; }
    sc=$(ceremony_started_count "$i" "$E_new")
    (( sc <= 1 )) || {
        echo "FAIL (smoke-vrf-dkg-halt): v$i logged $sc 'ceremony started' for E_new=$E_new — it RE-DEALT after the torn restart (the corruption fell to NoFile, not Torn), so it is a dealer again and the DKG is NOT below quorum"
        docker compose logs --tail=160 "validator-$i" | strip_ansi | grep "live DKG: ceremony started" | tail -10 | sed 's/^/    /'
        exit 1; }
    echo "  v$i sat out on the Torn arm (no re-deal, $sc 'ceremony started')"
done
echo "  both victims are sitting out E_new=$E_new — the C_r1 ceremony can reach at most 3 dealer logs < dealer-quorum 4"

# The chain now has 4 of 5 online → it CLIMBS to the boundary, where the DKG-None skip
# wedges it. Wait for the proposer to reach + skip the boundary view: the positive
# "DKG outcome not ready; skipping propose" log for E_new is the proof the halt is the
# DKG-None boundary skip (NOT a consensus stall — which would freeze BELOW the boundary
# and never log this). If a tear had slipped to POST-seal, the survivors' disseminated
# Reveals would let the DKG finalize, the boundary would cross, and this line would NOT
# fire → fails loud, not green.
# Wait for the chain to CLIMB to the boundary EDGE (head == boundary−1). The climb
# itself — from the tear point (PRE_HALT≈$PRE_HALT) all the way up to boundary−1 — is the
# discriminator: only a chain whose consensus quorum was RESTORED (the 2 restarted nodes
# rejoined → 4 of 5 online) can climb here; a genuine consensus stall would freeze AT the
# tear point and never reach the boundary edge. Generous budget for the 2-down resync +
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

# ══ THE REGRESSION LINE ══════════════════════════════════════════════════════════════
# TERMINAL HALT — the HEAD freezes AT the boundary edge and STAYS frozen (≥30 s, well past
# several 1 blk/s intervals). With beacon=None every proposer declines E_new's first block
# (application.rs:473-479), so the chain cannot cross the boundary — a permanent option-A
# halt.
#
# THIS IS THE LINE THE CASE FAILS ON IF THE HALT BEHAVIOUR REGRESSES. By the time it runs,
# the case has already PROVEN that both victims sat out on the Torn arm and did not
# re-deal, so the committee is shareless for E_new. A head that advances through this
# window is therefore a chain that CROSSED a change-epoch boundary with an unfinished DKG
# key — exactly the product failure this case exists to catch. (The
# finalized-never-crossed check further down is the same claim measured on finalized and
# is the second line to fire.)
HALT_HEAD=$(head_dec)
echo "  asserting the SUSTAINED no-progress halt at the boundary edge (head frozen ≥ 30 s from $HALT_HEAD)"
if ! head_frozen_for 30; then
    h0=$(head_dec); sleep 5; h1=$(head_dec)
    echo "FAIL (smoke-vrf-dkg-halt): TERMINAL control — head did NOT stay frozen at the boundary edge (head $h0 → $h1) — the chain CROSSED the E_new=$E_new boundary even though the committee is provably SHARELESS for E_new (both victims logged the Torn sit-out, neither re-dealt). That is a change-epoch boundary crossed with an unfinished DKG key: the halt gate at application.rs:473-479 has REGRESSED"
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
        echo "FAIL (smoke-vrf-dkg-halt): validator-$i computed an E_new=$E_new share — the C_r1 ceremony FINALIZED, so the two tears did not actually remove two dealers (tear too late / a re-deal) — not a terminal halt"
        exit 1
    fi
done
echo "  no committee member finalized an E_new share (the DKG stayed below dealer-quorum 4)"

# Clean halt, not a crash — no panic in any node.
panic=$(docker compose logs 2>/dev/null | strip_ansi | grep -iE "panic|thread '.*' panicked" || true)
[[ -z "$panic" ]] || { echo "FAIL (smoke-vrf-dkg-halt): a node PANICKED — the halt must be a clean option-A stall, not a crash:"; printf '%s\n' "$panic" | tail -10 | sed 's/^/    /'; exit 1; }

# Re-confirm the halt is still sustained AND below the boundary — it is permanent (the
# boundary view is skipped forever even with 4 of 5 online; torn_warned is permanent for
# E_new, so the two victims never re-join the ceremony and the DKG never reaches
# dealer-quorum).
if ! head_frozen_for 15; then
    echo "FAIL (smoke-vrf-dkg-halt): the halt was NOT permanent — the head advanced after the freeze window (a terminal >f pre-seal halt must never self-heal)"
    exit 1
fi
# The SECOND regression line: measured on FINALIZED and on the boundary height itself, so a
# crossing that happened while the tip was momentarily flat still fails here.
(( $(finalized_dec) < P_BOUNDARY )) || { echo "FAIL (smoke-vrf-dkg-halt): finalized=$(finalized_dec) reached the E_new boundary $P_BOUNDARY — the chain FINALIZED E_new's first block with a shareless committee (no member holds an E_new share), so the change-epoch halt did NOT hold"; exit 1; }
echo "  halt is permanent — head still frozen, finalized $(finalized_dec) never crossed the E_new boundary $P_BOUNDARY"

echo "OK (smoke-vrf-dkg-halt): >f PRE-SEAL TERMINAL halt — on C_r1 (E$E_new), TEARING the E$E_new ceremony journals of 2 committee stayers (v$K0+v$K1) PRE-seal (verified on disk: first 4 bytes = 0xffffffff) and restarting them restored CONSENSUS quorum (4 of 5 → the chain climbed to the boundary) while BOTH sat out on the JournalLoad::Torn arm with NO re-deal, leaving the DKG permanently below DEALER-quorum 4 → (beacon_for_epoch)(E$E_new)=None → the proposer logged 'DKG outcome not ready; skipping propose' and SKIPPED the boundary view → the chain FROZE at the boundary (head $HALT_HEAD, finalized never crossed $P_BOUNDARY) and stayed frozen, a clean permanent option-A halt (no panic, no E_new share). NEGATIVE control for case-vrf-dkg-durability's ONE-torn sit-out: same recipe, two victims instead of one → dealer-quorum unreachable → opposite (permanent) outcome. NOTE: a plain stop/start does NOT sit a member out — maybe_start's Present-journal arm re-derives the seeded dealer while last_height < epoch_start − DKG_MARGIN_BLOCKS (beacon/actor.rs:1046-1054) and the victim seals normally; only the Torn arm sits out. And on n=5 the DKG dealer-quorum and the consensus quorum are BOTH 4, so the nodes must be RESTARTED (not kept down) — keeping them down stalls consensus BELOW the boundary and the 'skipping propose' log never fires; restarting isolates the DKG-None boundary-skip as the wedge. Tearing down."
