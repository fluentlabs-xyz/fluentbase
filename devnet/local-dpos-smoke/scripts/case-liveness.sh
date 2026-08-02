#!/usr/bin/env bash
# smoke-liveness: with 4 validators (f=1, quorum 3) the network keeps finalizing
# while ONE is offline, the offline validator's on-chain PRODUCTION count stays flat
# while the always-up hub's rises (proving the 2-byte production record reaches the
# chain and `recordProduction` credits it to the right member), and the validator
# re-syncs/rejoins on restart over reth devp2p.
#
# It rotates the victim across the three non-hub validators [v3, v2, v1], one at a
# time (f=1 ⇒ never two down at once; validator-0 is the hub whose enode the spokes
# pin as --trusted-peers, so it stays up). A full liveness slash is NOT awaited —
# rising miss-count + continued liveness + clean rejoin is the signal.
#
# Rejoin mechanism under test (NOT reth pipeline-backfill — that earlier framing was
# wrong): a validator down across ≥1 epoch boundary resumes in its stale epoch while
# the committee is ahead; it catches up on the CONSENSUS plane — the vote backup
# channel detects ahead-epoch votes → hints the marshal → the marshal walks the
# finalized tip forward boundary-by-boundary → each crossed epoch soft-enters its
# committee scheme (no engine) until the live epoch full-enters. The cycles below
# span the catch-up spectrum so the per-epoch walk is exercised at multiple depths.
#
# …and only the within-epoch cycle really walks it — see the cycle list at the bottom
# of this file. The other three gap past the steady-state re-jump gate and TELEPORT
# instead, which is why every cycle ends on a SIGNING assertion (step 4) and not on a
# height: a re-jumped member is height-aligned long before it holds a per-epoch BFT
# engine, and this case's whole premise is that the previous victim is a working
# signer again before the next one is stopped.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

bring_up_dpos
trap tear_down EXIT

# Committee addresses (validators[i] == validator-i). jq is host-side; the file
# is read in-container (runtime image is curl-only). NB: no `tr -d [:space:]` —
# it would strip the per-line newlines too, collapsing all four into ADDR[0].
mapfile -t ADDR < <(docker compose exec -T validator-0 cat /runtime/addresses.json | jq -r '.validators[]')
(( ${#ADDR[@]} == 4 )) || { echo "FAIL (smoke-liveness): expected 4 validator addresses, got ${#ADDR[@]}: ${ADDR[*]}"; exit 1; }
V0_ADDR="${ADDR[0]}"

# `signer_idx` + `participation` (the committee-index + windowed seen/certs readers) live
# in lib.sh — shared with case-vrf-dkg-restart-midwindow.sh. A "-2 -2" from participation
# is a FAILED read (retry), never a valid "0 0": a failed read collapsing to 0 would make
# the `vic_seen < hub_seen` assertion easier to satisfy and hide a getter regression.

# How many times $1 logged the Verifier→Signer promotion for an epoch >= $2.
#
# `promoted to Signer in-process: per-epoch BFT engine started` (epoch_manager.rs:838-841) is
# emitted at the ONE `spawn_engine` call site (:831) and ONLY when the spawn returned true — every
# path that leaves the member verify-only returns from the same match arm BEFORE it: the share
# gate (:718), the E-1 boundary-block defer (:734), the promote VALUE gate (:768) and the promote
# SHARE gate (:797). That is what makes it the positive control: it cannot be logged by a member
# that is not signing.
#
# strip_ansi IS MANDATORY and is the reason this is not `log_count`. The node writes SGR escapes
# INSIDE its key=value pairs, so a raw line renders `epoch<ESC>=<ESC>Epoch(4)` and the epoch
# extraction below matches NOTHING — a silent reader, which on a gate that waits for a line to
# APPEAR is indistinguishable from a member that never promoted.
#
# `?epoch` is a Debug newtype, hence `Epoch(N)`; the bare-u64 form is accepted too because a
# future `%epoch` would emit it. The EPOCH FLOOR is not decoration: a restarted victim boots on
# its persisted tail and reconciles the epoch it was KILLED in first, so it can log this line for
# an epoch the committee left three boundaries ago. That promotion is real and useless.
#
# ONE sed for both the token and the epoch, and the whole pipeline is `|| true`d: this file runs
# under `set -o pipefail`, so a `grep` that matched nothing (exit 1) mid-pipeline would abort the
# CASE — with an error that looks nothing like "the victim has not promoted yet", which is a
# perfectly ordinary reading here. `awk` prints `0` for the empty stream either way.
promoted_ge() {
    { docker compose logs "$1" 2>/dev/null | strip_ansi \
        | sed -nE '/promoted to Signer in-process/ s/.*epoch[=:][[:space:]]*(Epoch\()?([0-9]+).*/\2/p' \
        | awk -v m="$2" '$1 >= m { n++ } END { print n+0 }'; } || true
}

# Relative epoch of an absolute height (OriginEpocher: (h - origin) / length).
epoch_of() { echo $(( ($1 - DPOS_ACTIVATION_BLOCK) / EPOCH_INTERVAL )); }

# One kill/rejoin cycle. $1 = victim index (1..3), $2 = min blocks to advance while down. A gap
# ABOVE EPOCH_INTERVAL is above the steady-state re-jump gate, so the victim teleports to the
# frontier instead of walking it — which is why step 4 asks whether it can SIGN, not only whether
# it reached the height.
liveness_cycle() {
    # NB: separate `local` statements — a single `local a=$1 b=validator-$a`
    # expands all RHS before assigning, so `$a` would be unbound under `set -u`.
    local n="$1" gap="$2"
    local svc="validator-$n" vic="${ADDR[$n]}"
    local pre cur deadline min_epoch pro0
    pre=$(baseline_height)
    echo "── cycle: victim=$svc ($vic), target gap >= $gap blocks (pre=$pre) ──"

    # The victim's Verifier→Signer ledger BEFORE the stop, at the floor epoch this cycle will
    # judge it against. `docker compose stop`/`start` keeps the SAME container and its log
    # persists across the cycle, so only a DELTA against this snapshot is evidence about the node
    # that came back — an absolute grep is satisfied by the promotion the victim did on its way
    # into the epoch it is about to be killed in.
    min_epoch=$(epoch_of $(( pre + gap )))
    pro0=$(promoted_ge "$svc" "$min_epoch")

    docker compose stop --timeout 40 "$svc"

    # 1) network keeps finalizing with 1/4 down (BFT f=1 holds) and advances the
    #    required gap (cycle 1: past a full epoch → reth pipeline-backfill rejoin).
    #    240s: cycle 1 waits 3*EPOCH_INTERVAL+1 = 97 blocks; at 1 blk/s with the
    #    victim's leader views timing out (1750ms) until skip_timeout mutes them,
    #    that's ~100-115s of chain time.
    wait_finalized_ge $(( pre + gap )) 240 || {
        echo "FAIL (smoke-liveness): chain did not advance $gap blocks with $svc down (finalized=$(finalized_dec), pre=$pre)"; exit 1; }
    echo "  chain finalized past $((pre+gap)) with $svc down (BFT f=1 holds)"

    # 2) the production record reaches the chain and is credited to the RIGHT member: in
    #    the current epoch the offline victim is credited strictly FEWER blocks than the
    #    always-up hub (v0 is never killed, so it keeps winning leader views and producing).
    #
    #    This is the successor to the retired participation-bitmap assertion, and it is a
    #    strictly better signal. The bitmap was proposer-asserted and never crypto-checked,
    #    and commonware stops verifying a view's votes at first quorum — so a fully-live but
    #    geo-distant member was credited a small fraction of the certs it actually signed,
    #    which is the fairness defect this whole change removes. `producedAt` counts a 2-byte
    #    record that every honest voter checked against the consensus-supplied round leader.
    #
    #    `<` (not `== 0`) on purpose: the victim may hold credit from blocks it produced
    #    BEFORE it was stopped, since the counter is per-epoch and never cleared.
    deadline=$(( $(date +%s) + 90 ))
    local vprod vtotal rprod rtotal
    vprod=-1; vtotal=-1; rprod=0; rtotal=0
    while (( $(date +%s) < deadline )); do
        cur=$(staking_call "currentEpoch()(uint64)")
        read -r vprod vtotal < <(produced_in_epoch "$cur" "$vic")
        read -r rprod rtotal < <(produced_in_epoch "$cur" "$V0_ADDR")
        # A failed getter read (-2) on either value must trigger a retry, never
        # satisfy or weaken the assertion.
        [[ "$vprod" == "-2" || "$rprod" == "-2" ]] && { sleep 2; continue; }
        { [[ "$vprod" != "-1" && "$rprod" != "-1" ]] && (( rprod > 0 && vprod < rprod )); } && break
        sleep 2
    done
    { [[ "$vprod" != "-1" && "$vprod" != "-2" && "$rprod" != "-1" && "$rprod" != "-2" ]] \
        && (( rprod > 0 && vprod < rprod )); } || {
        echo "FAIL (smoke-liveness): production credit wrong (epoch=$cur victim produced=$vprod hub produced=$rprod blocksInEpoch=$rtotal)"; exit 1; }
    echo "  on-chain production credit correct: producedAt(epoch=$cur, $svc)=$vprod < hub=$rprod (blocksInEpoch=$rtotal)"

    # 3) rejoin: restart and assert the victim realigns to the hub's finalized
    #    head AND has a live reth devp2p peer (the EL transport that did the sync).
    docker compose start "$svc"
    deadline=$(( $(date +%s) + 120 ))
    local tick=0 v0 vn pc aligned=0
    while (( $(date +%s) < deadline )); do
        v0=$(check_external 8545); vn=$(check_node docker compose exec -T "$svc")
        pc=$(peer_count "$svc")
        # Both halves of the gate, and the peer count is the mechanism under test (the epoch
        # walk rides the consensus plane but the BLOCKS arrive over reth devp2p, so a victim
        # matching the hub with zero peers is reporting a head it did not sync).
        #
        # The alignment half is now SAME-HEIGHT identity via `_aligned_now`, not tip-vs-tip.
        # This was the tightest budget of the sweep — 120s for a victim that has to walk up
        # to three epoch boundaries — and a rejoining node is behind the hub for most of that
        # walk by construction, so requiring the two tips to coincide made the pass a
        # coincidence. The fork check is kept at the victim's own height and the "null|null"
        # guard still stops two unreachable nodes from reading as a rejoin.
        #
        # THE FLOOR IS `pre + gap`, and it is not optional. The `v0 == vn` equality that was
        # removed was quietly doing a SECOND job: it could not be satisfied until the victim had
        # really caught up to the hub, which is what stopped the NEXT cycle from taking a second
        # validator down while this one was still far behind. Same-height identity has no such
        # property, and a live run proved it: cycle 1's victim "rejoined" at 135 with the hub at
        # 249, cycle 2 then stopped its own victim, and 2 of 4 signers is a correct BFT stall.
        # `pre + gap` is what the CHAIN PROVABLY REACHED this cycle — step (1) above hard-fails
        # unless `wait_finalized_ge $((pre+gap))` returns — whereas the victim's own `pre` is
        # cleared the instant it comes back on its persisted tail. `_aligned_now`'s floor is
        # strict and applies per reader, and it only honours a HEX floor, hence the printf.
        if _aligned_now "$(printf '0x%x' $(( pre + gap )))" "$v0" "$vn" >/dev/null && (( pc > 0 )); then
            echo "  OK: $svc rejoined at $vn with reth peers=$pc (v0=$v0, floor=pre+gap=$((pre+gap)))"
            aligned=1
            break
        fi
        if (( tick % 7 == 0 )); then echo "    t+$((tick*2))s: $svc peers=$pc $svc=$vn v0=$v0"; fi
        tick=$((tick+1)); sleep 2
    done
    if (( aligned == 0 )); then
        echo "FAIL (smoke-liveness): $svc did not rejoin (peers=$(peer_count "$svc"), $svc=$(check_node docker compose exec -T "$svc"), v0=$(check_external 8545), floor=pre+gap=$((pre+gap))) — it never came back on the hub's chain above the floor, so the SIGNING half below was never reached. This is the victim's own catch-up: check its reth devp2p peers and its consensus-plane walk, not the committee."
        exit 1
    fi

    # 4) …AND IT IS SIGNING AGAIN. Height is not participation, and this half is not optional.
    #
    # Three of the four cycles gap PAST the steady-state re-jump gate — that gate is
    # `min(JUMP_THRESHOLD=1024, epochBlockInterval)` (dpos.rs:2556), i.e. the INTERVAL itself, so
    # `3*I+1` and both `I+1` cycles are over it and only the within-epoch cycle walks. `re_jump`
    # is live for these validators: it is `upstream.as_ref().map(...)` (dpos.rs:2557) and
    # node/dpos.rs:1683 sets `upstream = Some(ValidatorUpstream::Plane(...))` unconditionally
    # under plane-native sync, with no `--dpos.follower-upstream` needed (and the compose overlay
    # passes none).
    #
    # A re-jumped member catches up by HEIGHT — devp2p serves it the blocks, so the gate above
    # goes green — while its per-epoch BFT engine never spawned: the jump teleported the marshal
    # floor past the `E-1` boundary block the spawn gate needs (epoch_manager.rs:734-749). It is
    # verify-only, no proposals and no votes. On 2026-08-01 that let cycle 2 stop a SECOND
    # validator while cycle 1's victim still could not sign: 2 live signers of 4 against a quorum
    # of 3, `finalized=241, pre=241`, a correct BFT stall reported as a product bug.
    #
    # THE SIGNAL IS THE PROMOTE LINE, NOT A PROPOSAL. A member that IS signing can still skip its
    # first proposals (application.rs:911, "parent-seed witness not in SeedStore; skipping
    # propose"), so gating on "the victim produced a block" would trade this false PASS for a
    # false FAIL. The budget is one full epoch (the worst honest wait: a re-jumped member that
    # misses boundary seeding promotes at the NEXT boundary, at most `EPOCH_INTERVAL` blocks =
    # seconds away at 1 blk/s) plus 60s of slack — do NOT raise it to buy a green, a member that
    # needs longer than a whole epoch to sign again IS the finding.
    local sdeadline=$(( $(date +%s) + EPOCH_INTERVAL + 60 )) pro1=0 defers=0
    while (( $(date +%s) < sdeadline )); do
        pro1=$(promoted_ge "$svc" "$min_epoch")
        if (( pro1 > pro0 )); then
            echo "  OK: $svc is SIGNING again — $((pro1 - pro0)) new Signer promotion(s) for epoch >= $min_epoch (floor epoch of pre+gap=$((pre+gap)))"
            return 0
        fi
        sleep 5
    done
    defers=$(docker compose logs "$svc" 2>/dev/null | strip_ansi | grep -c "signer spawn deferred" || true)
    echo "FAIL (smoke-liveness): $svc is HEIGHT-ALIGNED but NOT SIGNING (it reached the floor pre+gap=$((pre+gap)) at $vn, and logged no new 'promoted to Signer in-process' for epoch >= $min_epoch: $pro0 -> $pro1). It holds no per-epoch BFT engine — verify-only, NO proposals and NO votes. Stopping the next validator now would leave 2 live signers of 4 against a quorum of 3, which stalls the chain and reads as a product bug; that is exactly the run this gate exists to stop."
    echo "  $svc logged 'signer spawn deferred' $defers time(s) — if that is rising, the engine-spawn gate is waiting on the E-1 boundary block (epoch_manager.rs:734-749) and the place to look is boundary seeding after the re-jump; if it is flat, read the victim's log for the share-gate / promote VALUE-gate / promote SHARE-gate lines instead."
    docker compose logs --tail=120 "$svc"
    exit 1
}

# Establish the bootstrap DKG BEFORE any disruption. `bring_up_dpos` returns at the
# migration anchor (relative epoch 0), but the bootstrap live-DKG for
# DETERMINISTIC_BOOTSTRAP_EPOCH=2 only deals during epoch 1 and finalizes just
# before epoch_start(2) = activation + 2*EPOCH_INTERVAL. A victim stopped before
# that never attends ANY ceremony, so it is permanently shareless on rejoin (no
# reshare in v1) and cannot re-promote to signer — out of v1 scope. Advancing past
# epoch 2 first makes every validator (incl. the cycle-1 victim) deal + PERSIST its
# share, so the deep-rejoin exercises the SUPPORTED path: a member that attended its
# bootstrap DKG, restarts, reloads its on-disk share, and re-promotes by
# carry-forward on the stable committee.
echo "establishing bootstrap DKG: advancing past epoch 2 (shares persisted) before disruption"
wait_finalized_ge $(( DPOS_ACTIVATION_BLOCK + 2 * EPOCH_INTERVAL + 8 )) 180 || {
    echo "FAIL (smoke-liveness): chain did not reach epoch 2 (bootstrap DKG) before disruption (finalized=$(finalized_dec))"; exit 1; }
echo "  bootstrap DKG epoch 2 finalized; all validators hold a persisted share"

# Catch-up spectrum (one at a time, each fully rejoins AND is signing again before the
# next):
#   cycle 1 (v3): DEEP — down across ~3 epoch boundaries; exercises the per-epoch
#                 soft-enter walk over several boundaries (the real rejoin stress).
#   cycle 2 (v2): SINGLE epoch-boundary cross (gap just over one epoch).
#   cycle 3 (v1): WITHIN-epoch gap (no boundary cross — full-enter, no soft-enter walk).
#   cycle 4 (v3): IMMEDIATE double-rejoin — re-kill a just-rejoined node across a
#                 boundary, to catch stale catch-up state carried across restarts
#                 (highest_entered_epoch / highest_observed_epoch).
#
# WHICH OF THEM ACTUALLY WALK, because three of the four names promise a walk they do
# not get. The steady-state re-jump gate is min(JUMP_THRESHOLD=1024, epochBlockInterval)
# (dpos.rs:2556) — the INTERVAL itself — so a cycle re-jumps whenever its gap exceeds one
# epoch: cycle 1 is over by 2*I+1 (DELIBERATE, "deep" is the deep path), cycles 2 and 4
# are over by ONE BLOCK, and cycle 3 is the only walker. "Single epoch-boundary cross" is
# a statement about BOUNDARIES, not about the jump gate; the two happen to be the same
# number and `I+1` lands on the far side of it.
#
# FLAGGED, NOT RETUNED. Trimming the `+1` would not buy the walk back — boot drift alone
# (the victim keeps falling behind while it restarts) puts the effective gap over the
# gate — so a walking version of cycles 2/4 would need a gap well UNDER the interval,
# which is cycle 3's job already. What the crossing costs is exactly what step 4 of
# `liveness_cycle` now measures. The Python port pins the same four answers in
# `verdicts_onchain.cycle_walks`.
liveness_cycle 3 $(( 3 * EPOCH_INTERVAL + 1 ))
liveness_cycle 2 $(( EPOCH_INTERVAL + 1 ))
liveness_cycle 1 5
liveness_cycle 3 $(( EPOCH_INTERVAL + 1 ))

echo "OK (smoke-liveness): [v3 deep, v2 single-boundary, v1 within-epoch, v3 double-rejoin] all held liveness while down, recorded on-chain miss-count, and rejoined the live tip"
