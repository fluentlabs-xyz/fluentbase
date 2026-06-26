#!/usr/bin/env bash
# smoke-vrf-dkg-restart-midwindow (standalone): a committee[2] member RESTARTS while
# its epoch-2 DKG window is STILL OPEN — its ceremony journal is on disk (dealt) but
# not yet finalized/evicted. Unlike smoke-vrf-dkg-liveness (which stops the victim
# BEFORE the window opens so it never starts its ceremony), this stops it AFTER it has
# dealt — so its partial ceremony progress is lost from memory and can only be
# recovered from the on-disk ceremony journal + the DKG-log recovery resolver
# (dpos_dkg_midwindow_restart_durability). With the fix the restarted node RESUMES
# player-only from `beacon-dkgjournal-e2.bin` (Player::resume — never re-dealing),
# re-fetches any missing peer logs via the `commonware_resolver::p2p` engine on
# BEACON_RESOLVER_CHANNEL, and converges to (PK_2, share) before the boundary; WITHOUT
# the fix it stays shareless for epoch 2, abstains from every seeded vote, and is
# LIVENESS-SLASHED.
#
# ITERATION-3 COVERAGE (no new case needed): the journal-keyed restart below can land
# EITHER after the victim's own seal (resume re-broadcasts its OwnSeal log) OR before it
# (a PRE-SEAL crash: full Player.view, own log absent + never broadcast). The pre-seal
# path is the [1] regression — `Player::finalize` recovers the share purely as a player
# from `view` + the n−f survivors, so the finalize gate is `dealing_closed()` NOT
# `own_log_recorded(me)` (gating on the own log would leave a pre-seal victim shareless →
# the slash this case asserts is AVOIDED). The deterministic pre-seal finalize, the
# capture-then-commit finalize-Err ([2]), the undecode→retry deliver ([967]), the
# cancel-dead-fetches retain ([804]), and the cold-miss negative cache ([914]) are
# unit-pinned in `actor.rs::clock_tests` (`pre_seal_player_only_finalizes_without_own_log`
# et al.); this docker case is the end-to-end guard that the recovery path as a whole
# still converges + avoids the slash.
#
# RESTART TIMING — keyed off the ON-DISK JOURNAL, never an EL-finalized height. The
# DKG actor's clock leads EL-finalized by K=3 blocks and seals/finalizes on its own
# clock, so a height of `seal_deadline+1` in EL-finalized terms is already past the
# actor's seal — a height-keyed restart there reloads the persisted share and SKIPS
# resume entirely (maybe_start's store-hit early-return), the original false-red. The
# journal `beacon-dkgjournal-e2.bin` is written at DEAL-START and is NOT evicted at
# finalize (eviction is the past-boundary sweep), so it lives the WHOLE deal→boundary
# span — polling for it lands the restart squarely inside the open window on any host,
# independent of when the victim finalizes (no fast-host false-negative).
#
# CRITICAL — TUNED CONFIG. The DEFAULT devnet config (epochBlockInterval=32 <
# missThreshold=50, per-(epoch,signerIdx) consecutive-miss counter that resets each
# boundary) is STRUCTURALLY INCAPABLE of firing the liveness slash — exactly why the
# existing smoke-vrf-dkg-liveness is green despite the gap. This case raises
# epochBlockInterval to 64 (> missThreshold 50) so a whole-epoch-shareless victim
# accrues > 50 consecutive misses and IS slashed, and lowers felonyThreshold to 1 so
# that slash also escalates to a JAIL — the negative control the asserts reference.
# The load-bearing assertion is that WITH the fix the slash is AVOIDED (missCount stays
# at 0, no ValidatorSlashed, status != Jail); asserting only "chain stays live" is
# insufficient (that passes even on the default config that can't slash).
#
# Heavy (~6-8 min).
set -euo pipefail
cd "$(dirname "$0")/.."

# Tuned liveness-slash config — forwarded to genesis-init (docker-compose.yml
# environment:) AND mirrored into lib.sh's chain-param vars (they MUST agree with the
# on-chain ChainConfig.initialize args). epochBlockInterval=64, dposActivationBlock=128
# (= 2*interval, keeps the migration anchor in absolute epoch 2), felonyThreshold=1.
# ChainConfig.initialize enforces misdemeanorThreshold <= felonyThreshold (both > 0), so
# MISDEMEANOR_THRESHOLD must also drop to 1 — otherwise its default (100) > felony (1)
# reverts genesis-init.
export EPOCH_BLOCK_INTERVAL=64
export DPOS_ACTIVATION_BLOCK=128
export FELONY_THRESHOLD=1
export MISDEMEANOR_THRESHOLD=1
export EPOCH_INTERVAL="$EPOCH_BLOCK_INTERVAL"   # lib.sh reads EPOCH_INTERVAL

# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

bring_up_dpos
trap tear_down EXIT

# Committee addresses (validators[i] == validator-i), for the liveness-slash views.
mapfile -t ADDR < <(docker compose exec -T validator-0 cat /runtime/addresses.json | jq -r '.validators[]')
(( ${#ADDR[@]} == 4 )) || { echo "FAIL (smoke-vrf-dkg-restart-midwindow): expected 4 validator addresses, got ${#ADDR[@]}"; exit 1; }

# `signer_idx` + `misscount` (the liveness-slash committee-index + consecutive-miss
# readers) and `validator_status` (the jail-status reader, status enum: 0 inactive,
# 1 pending, 2 active, 3 jailed, 4 exiting) all live in lib.sh — shared with
# case-liveness.sh / case-byzantine.sh.

DOWN=validator-3
DOWN_ADDR="${ADDR[3]}"
EPOCH2_START=$(( DPOS_ACTIVATION_BLOCK + 2 * EPOCH_INTERVAL ))
BOUNDARY_PROBE=$(( EPOCH2_START + 6 ))

# Is the epoch-2 ceremony journal present on the victim's disk RIGHT NOW? The
# journal `beacon-dkgjournal-e2.bin` lives under `<datadir>/beacon/` (the beacon
# dir is `data_dir()/beacon`); `find` under the victim's datadir tree rather than
# hardcoding the path so a future reth datadir-layout change can't silently break
# the gate. Read through validator-3's mount of the shared `runtime` volume.
# Returns 0 (true) iff a non-empty epoch-2 journal file exists.
dkg_journal_present() {
    docker compose exec -T "$DOWN" sh -c \
        'find /runtime/reth-data/v3 -type f -name "beacon-dkgjournal-e2.bin" -size +0c 2>/dev/null | grep -q .' \
        2>/dev/null
}

# Is the epoch-2 SHARE file ABSENT on the victim's disk RIGHT NOW? The share
# `beacon-share-e2.bin` is persisted at FINALIZE. Gating the restart on it being absent
# (in addition to the journal being present) guarantees the victim is restarted
# pre-finalize, so on restart `maybe_start` takes the `store` MISS path and actually runs
# `resume_from_journal` (the asserted "resumed" log line) — a victim that already finalized
# would hit the `store.contains_key(2)` early-return and never log a resume (false-RED,
# review [164]). Returns 0 (true) iff NO epoch-2 share file exists.
dkg_share_absent() {
    ! docker compose exec -T "$DOWN" sh -c \
        'find /runtime/reth-data/v3 -type f -name "beacon-share-e2.bin" 2>/dev/null | grep -q .' \
        2>/dev/null
}

# 1) Restart the victim in the GENUINE mid-window state — while its epoch-2
#    ceremony journal is still on disk (dealt, not yet evicted). Keying off the
#    on-disk journal (not an EL-finalized height) is what makes this correct: the DKG
#    actor's clock leads EL-finalized by K=3 blocks, so a height of `seal_deadline+1`
#    in EL-finalized terms is ALREADY past the actor's seal on its own clock.
#    DETERMINISTIC WINDOW: the journal is written at DEAL-START (the self-dealing
#    `ReceivedDealing` is journaled the instant the ceremony starts) and is NOT
#    evicted at finalize — eviction moved to the past-boundary sweep — so the file
#    lives for the WHOLE deal→boundary span (~tens of blocks here) regardless of when
#    the victim finalizes (even an all-in fast-finalize keeps the journal until the
#    boundary). The 1 s poll therefore reliably lands inside the window on any host
#    (no false-negative from a victim that finalized + evicted between two polls).
echo "smoke-vrf-dkg-restart-midwindow: waiting for $DOWN's epoch-2 DKG journal (present) AND share (absent) — the genuine pre-finalize mid-window so resume provably runs"
journal_deadline=$(( SECONDS + 400 ))
until dkg_journal_present && dkg_share_absent; do
    (( SECONDS < journal_deadline )) || {
        echo "FAIL (smoke-vrf-dkg-restart-midwindow): $DOWN's epoch-2 DKG journal never appeared on disk (the ceremony never started — the journal now lives deal-start→boundary, so a present journal cannot be missed by the poll)"
        docker compose logs --tail=160 "$DOWN" | grep -iE 'DKG|ceremony|journal' | tail -40
        exit 1; }
    # The host RPC must not already be at/past the boundary (would mean the window
    # closed): the journal poll is the primary gate, this is the safety rail.
    NOW=$(finalized_dec)
    if (( NOW >= EPOCH2_START )); then
        echo "FAIL (smoke-vrf-dkg-restart-midwindow): chain reached the epoch-2 boundary ($EPOCH2_START) before $DOWN's journal was observed — the open window was missed (re-run)"
        exit 1
    fi
    sleep 1
done
NOW=$(finalized_dec)
echo "smoke-vrf-dkg-restart-midwindow: restarting $DOWN mid-window at finalized=$NOW (epoch-2 ceremony journal on disk, not yet finalized)"
docker compose restart "$DOWN" >/dev/null

# 2) The chain crosses the epoch-2 boundary; the restarted victim RESUMES its ceremony
#    from the on-disk journal (+ pull) and converges to a share BEFORE the boundary.
NODES=("${VALS[@]}" full-node)
wait_finalized_ge "$BOUNDARY_PROBE" 400 >/dev/null || {
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): chain did not reach the epoch-2 boundary after the mid-window restart"
    docker compose logs --tail=120 "$DOWN"; exit 1; }
wait_nodes_have "$BOUNDARY_PROBE" 180 || { echo "FAIL (smoke-vrf-dkg-restart-midwindow): nodes did not all reach $BOUNDARY_PROBE"; exit 1; }

# (a) THE RECOVERY ASSERTION — the OPPOSITE of smoke-vrf-dkg-liveness's shareless
#     assertion: the restarted victim RESUMED its ceremony from the on-disk journal
#     and converged to an epoch-2 share. We anchor on the POST-RESTART "ceremony
#     resumed from journal" log (actor.rs `resume_from_journal`), NOT the "share
#     computed + stored" line: a fast host can finalize + emit "share computed" DURING
#     the open window BEFORE the restart, so a full-log grep for it is a false green
#     even against a broken resume. The resume log is emitted ONLY by the restarted
#     process, so its presence proves the recovery path actually ran. (The chain-since
#     `docker compose restart` is the restarted incarnation's log; the resume line
#     cannot pre-date the restart.)
# `fluent`'s tracing logs carry ANSI colour escapes even over a pipe, so the raw
# `docker compose logs` line reads `epoch<ESC>=<ESC>2` — a literal `epoch=2` match
# silently never fires (the false-red that masks a genuine resume). Strip the escapes
# before any field-grep (same helper as case-byzantine-vrf.sh).
strip_ansi() { sed -E 's/\x1b\[[0-9;]*m//g'; }
deadline=$(( SECONDS + 120 ))
resume_lines=""
share_lines=""
while (( SECONDS < deadline )); do
    resume_lines=$(docker compose logs "$DOWN" 2>/dev/null | strip_ansi \
        | grep "live DKG: ceremony resumed from journal" | grep -E "epoch=2( |,|$)" || true)
    share_lines=$(docker compose logs "$DOWN" 2>/dev/null | strip_ansi \
        | grep "live DKG: PK_epoch + share computed + stored" | grep -E "epoch=2( |,|$)" || true)
    [[ -n "$resume_lines" && -n "$share_lines" ]] && break
    sleep 3
done
[[ -n "$resume_lines" ]] || {
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): $DOWN did NOT log a post-restart 'ceremony resumed from journal' for epoch 2 — the journal+resume path never ran (this is the bug the fix closes):"
    docker compose logs --tail=160 "$DOWN" | grep -iE 'DKG|resume|journal' | tail -40
    exit 1; }
[[ -n "$share_lines" ]] || {
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): $DOWN resumed from journal but did NOT converge to an epoch-2 share — resume started but did not finalize (resolver fetch or settle gate did not complete):"
    docker compose logs --tail=160 "$DOWN" | grep -iE 'DKG|resume|journal' | tail -40
    exit 1; }
echo "smoke-vrf-dkg-restart-midwindow: $DOWN RESUMED from journal and recovered its epoch-2 share:"
printf '%s\n' "$resume_lines" | sed 's/^/    /'
printf '%s\n' "$share_lines" | sed 's/^/    /'

# (b) chain stayed live + (c) the victim's epoch-2 prev_randao is byte-identical to the
#     survivors (it participated as a real share-holder, not a verify-only re-deriver).
miss=()
for ((v = EPOCH2_START; v <= BOUNDARY_PROBE; v++)); do
    dh=$(mixhash_in "$DOWN" "$v"); sv=$(mixhash_at "$v")
    [[ "$dh" == "null" || -z "$dh" ]] && { miss+=("$v=missing-on-$DOWN"); continue; }
    [[ "$dh" == "$sv" ]] || miss+=("$v: $DOWN=$dh != validator-0=$sv")
done
(( ${#miss[@]} == 0 )) || { echo "FAIL (smoke-vrf-dkg-restart-midwindow): $DOWN epoch-2 prev_randao diverged:"; printf '  %s\n' "${miss[@]}"; exit 1; }

# (d) THE LOAD-BEARING ASSERTION — the slash is AVOIDED by the fix. The victim's
#     consecutive-miss counter in epoch 2 stays at 0 (it produced valid seeded votes
#     once it recovered), no ValidatorSlashed event fires for it, and it is NOT jailed.
#     NEGATIVE CONTROL (what would happen WITHOUT the fix, on THIS tuned config): the
#     victim would be shareless for epoch 2 → abstain from every seeded Notarize/Finalize
#     → a 0 bit in every cert → its missCount(epoch=2, idx) would climb to missThreshold
#     (50, < epochBlockInterval 64) → Staking.slash → with felonyThreshold=1 → JAIL. The
#     env-tuned config is what makes that path reachable at all (the default config
#     epoch 32 < missThreshold 50 can never fire it — the smoke-vrf-dkg-liveness blind
#     spot). Re-run this case against an unpatched binary to SEE the slash/jail.
cur_epoch=$(staking_call "currentEpoch()(uint64)" 2>/dev/null || staking_call "currentEpoch()(uint256)")
# Retry a FAILED read (-2) a few times; a persistent -2 must FAIL the case, never fall
# through — a -2 in the `vmc >= miss_threshold` arithmetic below is < 50 and would
# WRONGLY satisfy the slash-avoidance assertion ([8]). Only a real numeric count is
# acceptable evidence that the slash was avoided.
vmc=$(misscount 2 "$DOWN_ADDR")
for _ in 1 2 3; do
    [[ "$vmc" == "-2" ]] || break
    sleep 4
    vmc=$(misscount 2 "$DOWN_ADDR")
done
if [[ "$vmc" == "-2" ]]; then
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): missCount(epoch=2) read kept failing (-2 RPC sentinel) — cannot prove the slash was avoided (a -2 must never be treated as a passing 0)"; exit 1
fi
if [[ "$vmc" == "-1" ]]; then
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): $DOWN is not in committee[2] — cannot evaluate the liveness slash (committee composition changed unexpectedly)"; exit 1
fi
# A recovered signer accrues at most a few transient view-change misses, never the
# whole-epoch run that reaches missThreshold; assert it stayed well under threshold.
miss_threshold=$(chainconfig_call "getMissThreshold()(uint32)" 2>/dev/null || echo 50)
if (( vmc >= miss_threshold )); then
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): $DOWN missCount(epoch=2)=$vmc reached missThreshold=$miss_threshold — it was liveness-slashed, i.e. it did NOT recover its share (the fix failed)"
    exit 1
fi
slash_events=$(docker compose logs 2>/dev/null | grep -iE "ValidatorSlashed|LivenessSlashDispatched" | grep -i "${DOWN_ADDR#0x}" || true)
[[ -z "$slash_events" ]] || {
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): $DOWN was liveness-slashed despite recovering its share:"; printf '%s\n' "$slash_events" | sed 's/^/    /'; exit 1; }
status=$(validator_status "$DOWN_ADDR")
# An EMPTY status means the getValidatorStatus RPC read FAILED — treat as a hard error,
# NEVER a pass: an empty-vs-"3" false-green would silently hide the very liveness slash
# this case exists to catch (review [225]).
[[ -n "$status" ]] || {
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): could not read $DOWN validator status (empty RPC result) — cannot assert not-jailed (re-run)"; exit 1; }
# status 3 == jailed
[[ "$status" != "3" ]] || {
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): $DOWN is JAILED (status=3) — it was slashed to felony despite recovering (felonyThreshold=$FELONY_THRESHOLD)"; exit 1; }
echo "smoke-vrf-dkg-restart-midwindow: $DOWN AVOIDED the liveness slash (missCount(epoch=2)=$vmc < threshold=$miss_threshold, no ValidatorSlashed, status=$status != jailed)"

# Chain finalizing after the boundary.
BEFORE=$(finalized_dec); sleep 6; AFTER=$(finalized_dec)
(( AFTER > BEFORE )) || { echo "FAIL (smoke-vrf-dkg-restart-midwindow): chain not finalizing after the boundary ($AFTER <= $BEFORE)"; exit 1; }

echo "OK (smoke-vrf-dkg-restart-midwindow): a committee[2] member restarted mid-window (epoch-2 ceremony journal on disk, pre-finalize) RESUMED its ceremony from the on-disk journal, converged to (PK_2, share), produced byte-identical prev_randao with the survivors, and AVOIDED the liveness slash + jail that a shareless node would suffer on this tuned config"
