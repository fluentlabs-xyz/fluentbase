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
# CRITICAL — TUNED CONFIG. The assertion is PRODUCTION, not punishment: a member shareless
# for a whole epoch cannot produce a valid boundary proposal, so its on-chain
# `producedAt(2, idx)` stays 0, while a member that resumed its ceremony from the journal
# produces. No governance override is needed for that — it is a plain counter read, not a
# threshold. (The old framing here was the participation-floor jail, deleted with the whole
# liveness-jail machinery; the negative control it named is now "credit stays 0".)
# This case keeps epochBlockInterval=64
# only for a GENEROUS DKG window (room for the journal-present poll to land inside the open
# window). The load-bearing assertion is that WITH the fix the victim PRODUCES
# (`producedAt(2, idx) > 0`); asserting only "chain stays live" is insufficient — that
# passes on a run where the victim contributed nothing.
#
# Heavy (~6-8 min).
set -euo pipefail
cd "$(dirname "$0")/.."

# Tuned epoch config — forwarded to genesis-init (docker-compose.yml environment:) AND
# mirrored into lib.sh's chain-param vars (they MUST agree with the on-chain
# ChainConfig.initialize args). epochBlockInterval=64 for a generous DKG window,
# dposActivationBlock=128 (= 2*interval, keeps the migration anchor in absolute epoch 2).
export EPOCH_BLOCK_INTERVAL=64
export DPOS_ACTIVATION_BLOCK=128
export EPOCH_INTERVAL="$EPOCH_BLOCK_INTERVAL"   # lib.sh reads EPOCH_INTERVAL

# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

bring_up_dpos
trap tear_down EXIT

# Committee addresses (validators[i] == validator-i), for the liveness-slash views.
mapfile -t ADDR < <(docker compose exec -T validator-0 cat /runtime/addresses.json | jq -r '.validators[]')
(( ${#ADDR[@]} == 4 )) || { echo "FAIL (smoke-vrf-dkg-restart-midwindow): expected 4 validator addresses, got ${#ADDR[@]}"; exit 1; }

# `signer_idx` + `produced_in_epoch` (the committee-index + per-epoch production-credit
# reader) and `validator_status` (the status reader, enum: 0 inactive, 1 pending, 2 active,
# 3 jailed, 4 exiting) all live in lib.sh — shared with case-liveness.sh / case-byzantine.sh.

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
# strip_ansi hoisted to lib.sh (sourced above) — shared with the soak battery + the other VRF cases.
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

# (d) THE LOAD-BEARING ASSERTION — the recovered victim PRODUCES. It holds (PK_2, share),
#     so it can build a valid epoch-2 proposal, win its leader views and get credited.
#     NEGATIVE CONTROL (WITHOUT the fix): the victim is shareless for epoch 2, so it cannot
#     produce a valid boundary proposal at all — every view it leads is nullified and its
#     `producedAt(2, idx)` stays 0. That control is LIVE within the test window: the counter
#     moves on every recorded block, so there is nothing to wait for.
# Retry a FAILED read ("-2 -2") a few times, and wait until epoch 2 has RECORDED blocks
# (blocksInEpoch > 0) so "produced something" is evidence rather than an artefact of reading
# too early; a persistent -2 / zero-block epoch must FAIL the case, never fall through.
read -r vprod vtotal < <(produced_in_epoch 2 "$DOWN_ADDR")
for _ in 1 2 3 4 5; do
    if [[ "$vprod" == "-2" ]]; then sleep 4; read -r vprod vtotal < <(produced_in_epoch 2 "$DOWN_ADDR"); continue; fi
    { [[ "$vprod" != "-1" ]] && (( vtotal > 0 )); } && break
    sleep 4
    read -r vprod vtotal < <(produced_in_epoch 2 "$DOWN_ADDR")
done
if [[ "$vprod" == "-2" ]]; then
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): producedAt(epoch=2) read kept failing (-2 RPC sentinel) — cannot prove the member produced (a -2 must never be treated as a passing 0)"; exit 1
fi
if [[ "$vprod" == "-1" ]]; then
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): $DOWN is not in committee[2] — cannot evaluate its production (committee composition changed unexpectedly)"; exit 1
fi
if (( vtotal <= 0 )); then
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): no epoch-2 blocks recorded (blocksInEpoch=$vtotal) — cannot evaluate production (re-run / widen the window)"; exit 1
fi
# THE LOAD-BEARING ASSERTION, in the shape the deleted participation floor left behind: a member
# that resumed its ceremony from the journal can PRODUCE; a shareless one cannot produce a valid
# boundary proposal at all and stays at 0. Deliberately `> 0` and not a ratio — production credit
# is drawn by a stake-weighted lottery, so a healthy member's exact share over ONE epoch is a
# random variable and any fixed threshold would be an invented number.
if (( vprod <= 0 )); then
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): $DOWN produced NOTHING in epoch 2 (producedAt=$vprod of blocksInEpoch=$vtotal) — it did not recover its share (the fix failed)"
    exit 1
fi
# `ValidatorSlashed` survives ONLY as an equivocation event now that the liveness jail is deleted,
# which is exactly what a torn resume must not trigger.
slash_events=$(docker compose logs 2>/dev/null | grep -iE "ValidatorSlashed|equivocat" | grep -i "${DOWN_ADDR#0x}" || true)
[[ -z "$slash_events" ]] || {
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): $DOWN was slashed despite recovering its share:"; printf '%s\n' "$slash_events" | sed 's/^/    /'; exit 1; }
status=$(validator_status "$DOWN_ADDR")
# An EMPTY status means the getValidatorStatus RPC read FAILED — treat as a hard error,
# NEVER a pass: an empty-vs-"3" false-green would silently hide the very fault this case
# exists to catch (review [225]).
[[ -n "$status" ]] || {
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): could not read $DOWN validator status (empty RPC result) — cannot assert not-jailed (re-run)"; exit 1; }
# status 3 == jailed. Equivocation is its ONLY producer now, and it always tombstones.
[[ "$status" != "3" ]] || {
    echo "FAIL (smoke-vrf-dkg-restart-midwindow): $DOWN is JAILED (status=3) — the only producer of that status is equivocation, which is permanent and tombstoning"; exit 1; }
echo "smoke-vrf-dkg-restart-midwindow: $DOWN resumed and PRODUCED (producedAt=$vprod of blocksInEpoch=$vtotal, no equivocation slash, status=$status != jailed)"

# Chain finalizing after the boundary.
BEFORE=$(finalized_dec); sleep 6; AFTER=$(finalized_dec)
(( AFTER > BEFORE )) || { echo "FAIL (smoke-vrf-dkg-restart-midwindow): chain not finalizing after the boundary ($AFTER <= $BEFORE)"; exit 1; }

echo "OK (smoke-vrf-dkg-restart-midwindow): a committee[2] member restarted mid-window (epoch-2 ceremony journal on disk, pre-finalize) RESUMED its ceremony from the on-disk journal, converged to (PK_2, share), produced byte-identical prev_randao with the survivors, and kept PRODUCING, which a shareless node cannot do"
