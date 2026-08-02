#!/usr/bin/env bash
# smoke-cert-catchup (standalone): the DETERMINISTIC, fast gate for the executor
# catch-up PARK fix (task dpos-cert-budget-catchup-recover-stall). It reproduces the
# recover-stall shape on demand instead of waiting for the hour-long stochastic soak
# to drift into it:
#
#   A GRACEFULLY-restarted committee validator falls behind the network and walks the
#   gap by deriving each finalized OrderBlock in turn. While it is >= K (=3,
#   order_block.rs:23) behind the ordering tip, GUARD #2 (executor.rs:2444) cross-checks
#   the committee-attested `result` at h+K BEFORE acking h -- and the marshal backfills
#   bodies OUT OF ORDER inside its repair window, so h+K's BODY can still be absent when
#   h is dispatched. That absent-body arm is the dependency this case exercises.
#     OLD (pre-fix): the executor waited on a cert budget and then self-shut-down
#       "finalization cert unavailable within budget; cannot derive beacon
#        prev_randao without diverging; shutting down height=N"
#       -> the node drops its ack -> marshal Cancels -> "OuterEngine exited cleanly
#       (unexpected)" -> reason=consensus_exit -> never rejoins -> recover-stall.
#     NEW (this fix): the executor PARKS event-driven (no budget, no shutdown),
#       warns once ("guard #2: committee-attested body at h+K not backfilled yet;
#       PARKING derive + hinting peers", executor.rs:1473), sets the `deferred_height`
#       gauge (:1478), hints peers for h+K, and re-derives on the marshal delivery
#       stream + the 8s FCU-heartbeat re-poke (repoke_deferred, executor.rs:1497);
#       if it falls > re_jump_threshold behind, a gap-gated re-jump recovers it.
#
#   THERE IS NO CERT DEPENDENCY LEFT ANYWHERE ON THIS PATH. The pre-B' cert-miss park
#   classes (and their ordered cert prefetch) were DELETED with the witness -- see
#   DPOS_ARCHITECTURE.md, "the one remaining park is guard #2's absent h+K attestation
#   body". That deletion REWROTE the CONDITION half of this very warn!: it used to read
#   "finalization cert not local yet; PARKING derive + hinting peers (...)" and now reads
#   "guard #2: committee-attested body at h+K not backfilled yet; PARKING derive + ...".
#   The ACTION half is byte-identical across both; only the condition changed. Until
#   2026-07-31 this case grepped the OLD condition, so its park gate could not fire at ANY
#   gap. (The old wording is still what HEAD's executor.rs:1133 emits -- the rewrite is in
#   the working tree, which is what docker builds; soak bundles pin the changeover between
#   bundle-20260707T182226Z, old wording, and bundle-20260717T074614Z, new.)
#
# THE GATE (fail loud on any miss, with the captured log tail):
#   1. the restarted victim REJOINS (its finalized height realigns to the hub head);
#   2. its log SHOWS the park path firing (the fresh-park warn! count rises; the
#      `deferred_height` gauge is sampled non-zero when we catch it) -- i.e. the guard-#2
#      park was ACTUALLY exercised. A rejoin that NEVER parked is a FALSE PASS (the
#      fix's path was skipped) -> hard FAIL.
#   3. its log does NOT contain the OLD fatal budget-shutdown ERROR and it did NOT
#      log the consensus_exit ("OuterEngine exited cleanly") signature.
#
# WHY interval=64 + gap=28 (deterministic, prod-faithful, no PRNG/epoch dependence):
#   THE INTERVAL IS THE RE-JUMP CEILING. The effective re-jump gap gate is
#   min(JUMP_THRESHOLD=1024, epochBlockInterval) (dpos.rs:2543 validator / :3237 follower
#   -> 64 here). At the DEFAULT interval=32 the pure-park window (gap < threshold) is too
#   shallow to force a multi-block derive-walk at all; interval=64 opens a 64-block one.
#   The default down-gap (28) sits BELOW the re-jump threshold (64) AND well under a full
#   64-block epoch, so the victim (a) catches up via the PURE derive-walk -- no re-jump
#   steals the walk, exactly the prod failure regime (prod's threshold=1024 >> its observed
#   45-block failure gap, so it parked without ever re-jumping) -- and (b) suffers no
#   on-chain consequence while down: the participation floor that once judged it is
#   deleted, and the successor production-liveness tier ships flag-off.
#
#   ===================================================================================
#   DO NOT RAISE THE INTERVAL TO 128 TO BUY HEADROOM. IT DOES NOT BOOT.
#
#   Tried live on 2026-07-31. The stack never reached this case at all:
#       waiting for the sequencer to finalize >= dposActivationBlock=256 (relative epoch 0)
#         sequencer finalized 256 >= activation 256; proceeding to swap
#       FAIL: DPoS chain did not converge past anchor 0x100
#   ZERO blocks past the anchor. validator-0 logged `dpos: proposing order block
#   height=257` and every view from 4 to 115 answered `proposal failed verification` --
#   including on the PROPOSER'S OWN node -- for the whole 120 s converge budget, at EPOCH 0,
#   before any DKG and before any of this case's logic ran. A structural reject, not a
#   timeout: no budget anywhere makes it pass. Intervals 32 and 64 both reached "DPoS stack
#   live" on the SAME build in the same run (smoke-vrf-dkg-durability, smoke-vrf-dkg-halt),
#   so the interval is the variable. That rejection is a real product finding with its own
#   ticket; it is not this case's to carry. THE GAP IS THE LEVER INSTEAD.
#   ===================================================================================
#
#   THE DEFAULT DELAY IS 3000 AND THE MARGIN AT IT IS 4 BLOCKS. Read GAP=28 <= GAP_CEILING
#   =32 with that attached: effective(28,3000) = 53 against the 64 threshold, and the
#   second-order pass (stall adds backlog, which adds a round, which adds stall) puts it at
#   59 -- inside the ~10-block band this file itself calls marginal. FOUR live bring-ups say
#   it holds: at 1000 ms GAP=28 parked 0 then 1 time; at 3000 ms it parked 2 then 3, with
#   deferred_height peak 305 and NO re-jump on any of them. The measurement wins over the
#   model deliberately -- the model has been wrong in BOTH directions here (it blessed 40 at
#   1000, which lost the walk, and it forbade (28,3000), which works). It is a guard-rail,
#   not an oracle. What reopens it: a re-jump landing at the default, or parks falling back
#   toward zero -- and the answer then is a SMALLER GAP, since the interval cannot go up.
#
#   THE GAP WAS 40 AND 40 WAS NOT ENOUGH. Same config, back to back:
#       run 1: park warns 0 -> 4, re-jump landings 1 -> 1   PASS (deferred peak 309)
#       run 2: park warns 0 -> 0, re-jump landings 1 -> 2   FAIL (the re-jump took the walk)
#   The delay that opens the park window is the same force that grows the victim's gap
#   toward the threshold. Note 40 was NOT over GAP_CEILING (=45 here): it sat 5 blocks
#   under it, and its effective gap 57 sat 7 blocks under the 64-block threshold -- and it
#   lost the walk anyway. So the stall model below is a FLOOR on the real cost, not a bound
#   on it, and a gap needs MARGIN rather than mere compliance. 28 is effective 43: 17 blocks
#   under the ceiling, 21 under the threshold. DELAY_MS is deliberately NOT touched -- it is
#   the half of the pair that was never in doubt, and what makes the park fire at all.
#
#   GAP HAS A CEILING BELOW THE THRESHOLD, and it is DERIVED (GAP_CEILING below), not
#   hand-set. The frontier keeps climbing at 1 blk/s while the victim boots (~7-10 blocks
#   observed between `docker compose start` and the first Update::Tip) AND while its shaped
#   catch-up waits on each repair round trip, so the gap the executor actually faces is
#   GAP + boot + stall. Cross the threshold and `maybe_re_jump` (executor.rs:1765-1790)
#   fires and fast-forwards past the derive-walk -- the walk this case is trying to observe
#   is abandoned and the park gate fails for a reason that has nothing to do with the park.
#
#   DEPTH IS NOT THE LEVER FOR THE PARK EITHER (do not "just raise the gap"). The marshal
#   repairs in a sliding window anchored at the ack pointer, MAX_REPAIR=20 (outer.rs:223),
#   and dispatches up to MAX_PENDING_ACKS=16 bodies ahead of the executor's acks
#   (outer.rs:229). h+K=h+3 therefore sits INSIDE an already-issued batch on a 4-node LAN:
#   a deeper gap just means more blocks with the same body-ahead-of-derive margin. The park
#   fires only where the executor's derive DRAINS the contiguous dispatched prefix before
#   the next repair batch lands -- a real race, but one biased against firing here.
#
# THE LEVER IS RTT ON THE VICTIM (CERT_CATCHUP_DELAY_MS, default 3000). With zero latency
#   the marshal's repair always outruns the derive, so on this stack the park fires at NO
#   gap. Add an RTT and each repair batch pays it while the derive stays local and
#   unaffected: the dispatched prefix advances in <=MAX_REPAIR-block steps per round trip
#   and the derive can reach its edge, which is exactly the guard-#2 condition. This is
#   NOT a crutch -- it is the production condition restored. Real validators are
#   geographically distributed and always pay an RTT; a zero-latency LAN is the
#   unrealistic case. The SOAK already demonstrates the difference empirically: 43 of 323
#   bundles carry a park (bundle-20260724T181031Z/logs/validator-8.log: derived h=25800,
#   parked, re-derived 153 ms later) and its one relevant difference is
#   SOAK_GEO_LATENCY=1, which bakes per-region tc-netem into every validator
#   (gen-soak-compose.sh:146-168). Same tool, same verb, same image (Dockerfile:87 installs
#   iproute2); this case just applies ONE uniform delay to ONE node instead of a
#   five-region matrix to all of them.
#
#   `delay`, NEVER `rate`: bandwidth throttling would also slow the live finalization
#   stream that keeps the ordering tip climbing, which is the wrong side of the race. For
#   the same reason a CPU throttle (asserts-fault's `docker update --cpus`) is REJECTED --
#   it slows the DERIVE, the side that must stay fast, making parks strictly less likely.
#
#   Scoped twice over: to the VICTIM only (nobody else is shaped, so the committee keeps
#   finalizing at full speed) and to the CATCH-UP WINDOW only (applied right after the
#   restart, removed the moment the walk lands). The removal is in `cleanup`, so it fires
#   on the failure and interrupt paths too -- a stack left shaped would poison the deep
#   cycle and every later case on the same bring-up, and would read as a product slowdown.
#
# OPTIONAL deep cycle (CERT_CATCHUP_DEEP=1): a second down/restart with gap >
#   re_jump_threshold to additionally exercise the gap-gated re-jump (design.md
#   §4.3). It tolerates a liveness demotion (a deep gap can leave the victim below the
#   participation floor for a full window) and asserts rejoin + no-fatal + the re-jump
#   landing log; the park there is
#   best-effort (a re-jump can fast-forward past the derive-walk).
#
# Tunables (env): CERT_CATCHUP_VICTIM (default validator-2, a spoke that pins the hub
#   as a trusted-peer -- matches the prod-observed victim), CERT_CATCHUP_GAP (40),
#   CERT_CATCHUP_DELAY_MS (3000), CERT_CATCHUP_DEEP (0/1), CERT_CATCHUP_DEEP_GAP.
#
# Deterministic + fast: a fixed graceful stop/gap/restart, no random churn.
# Heavy (~8-10 min: ~5-6 min to the DKG-safe point at interval=64, then the cycle).
set -euo pipefail
cd "$(dirname "$0")/.."

# interval=64 -> re_jump_threshold = min(1024, 64) = 64; dposActivationBlock derives
# to 2*64 = 128 (lib.sh:33). DO NOT RAISE IT -- 128 does not boot, see the header. Mirror EPOCH_BLOCK_INTERVAL (container genesis-init env,
# docker-compose.yml) into EPOCH_INTERVAL (host-side lib.sh chain math) -- the exact pattern
# case-vrf-dkg-restart-midwindow.sh / case-soak.sh use so the two never drift.
export EPOCH_BLOCK_INTERVAL="${EPOCH_BLOCK_INTERVAL:-64}"
export EPOCH_INTERVAL="$EPOCH_BLOCK_INTERVAL"

# The case's own compose overlay, read by _migrate_to_dpos's `-f`-chain (lib.sh:547) --
# the same seam case-byzantine.sh uses. It carries ONE thing, NET_ADMIN, which `tc qdisc
# add` needs; it shapes nothing on its own. Deliberately NOT an edit to the shared
# docker-compose.yml: this is one case's requirement, not the stack's.
export DPOS_EXTRA_COMPOSE="-f docker-compose.cert-catchup.yml"

# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

# --- tunables --------------------------------------------------------------
VICTIM="${CERT_CATCHUP_VICTIM:-validator-2}"
GAP="${CERT_CATCHUP_GAP:-28}"
DEEP="${CERT_CATCHUP_DEEP:-0}"
DEEP_GAP="${CERT_CATCHUP_DEEP_GAP:-$(( 2 * EPOCH_INTERVAL + EPOCH_INTERVAL / 2 ))}"
REJUMP_THRESHOLD=$(( EPOCH_INTERVAL < 1024 ? EPOCH_INTERVAL : 1024 ))
# THE LEVER (see the header). One-way egress delay added to the victim for its catch-up
# window only -- one round trip per repair batch, while the derive stays local and
# untouched. 3000 ms is MEASURED, and it is what made this case stop flaking: at 1000 ms
# GAP=28 parked on about half its bring-ups (0 parks, then 1 park), at 3000 ms it parked on
# both (2 parks, then 3, deferred_height peak 305, no re-jump either time). The cost is
# stall, which drops GAP_CEILING from 46 to 32 -- so GAP=28 now clears it by 4 blocks, not
# 18. That is thin and it was chosen on evidence; see the header. Named and
# env-overridable in BOTH trees (verdicts_onchain.CATCHUP_NETEM_DELAY_MS) so one live
# iteration can retune it without touching any logic; the two defaults are asserted equal
# in dpos_harness/tests/test_smoke_onchain_cases.py.
DELAY_MS="${CERT_CATCHUP_DELAY_MS:-3000}"
# Every service sits on the single fluent-net bridge (docker-compose.yml:10-15), so the
# interface name is deterministic -- the same assumption gen-soak-compose.sh:152 makes.
NETEM_IFACE='eth0'

# --- the gap's DERIVED ceiling (mirrors verdicts_onchain.catchup_gap_ceiling) ------------
# The gap the executor faces is never the gap this case asked for:
#
#     effective = GAP + boot + stall
#
# Wall-clock converts to gap one for one -- the committee produces a block a second, so a
# second the victim spends waiting is a block further behind. Two things make it wait, and
# both cost exactly one added RTT (DELAY_MS) each:
#   * FETCH -- the marshal drains the backlog MAX_REPAIR=20 bodies per repair round
#     (outer.rs:223), so a backlog of B costs ceil(B/20) round trips;
#   * PARKS -- each guard-#2 park waits on one further body, i.e. one further round trip.
#     The measured run produced 4 parks against 3 fetch rounds, so parks price at rounds+1.
# Hence stall = ceil((2*rounds + 1) * DELAY_MS / 1000) blocks, and the ceiling is the
# largest gap that still leaves effective < threshold.
#
# ROUNDS COME FROM THE ACTUAL BACKLOG (GAP + boot), not from a full-threshold one. An
# earlier revision priced them at the threshold, calling it "conservative". It was not
# safer, just a different unmeasured number, and it FORBADE VALID CONFIGURATIONS -- it
# priced (GAP 28, delay 3000) at a stall of 27 and refused the run, where the backlog model
# prices it at 15 and the effective gap at 53, eleven blocks clear of 64. The measurement
# settles it: at GAP 40 / delay 1000 the observed effective gap was 57 = 40 + boot 10 +
# stall 7, and ceil(50/20)=3 rounds gives exactly 7 while ceil(64/20)=4 rounds gives 9.
#
# At interval=64: ceiling is 46 at delay 1000 and 32 at delay 3000. The literal 52 this
# used to be is refused by both -- effective(52,1000) = 71, past the threshold.
#
# MORE PERMISSIVE, and where: this is a FIRST-ORDER fixed point. The stall itself adds
# backlog, which can add a round, which adds stall. At (28, 3000) the second pass is
# ceil(53/20)=3 rounds -> stall 21 -> effective 59: still under 64, but 6 blocks of margin
# rather than 11. Read anything landing within ~10 of the threshold as MARGINAL.
BOOT_BLOCKS=10                     # measured `docker compose start` -> first Update::Tip
MAX_REPAIR=20                      # outer.rs:223
# effective(gap) = gap + boot + ceil((2*ceil((gap+boot)/MAX_REPAIR) + 1) * DELAY_MS / 1000)
_catchup_effective_gap() {
    local b r stall
    b=$(( $1 + BOOT_BLOCKS ))
    r=$(( (b + MAX_REPAIR - 1) / MAX_REPAIR ))
    stall=$(( ((2 * r + 1) * DELAY_MS + 999) / 1000 ))
    echo $(( b + stall ))
}
# Scan for the largest gap still under the threshold; `effective` is monotone in the gap
# (rounds only grow), so the scan is exact. Mirrors verdicts_onchain.catchup_gap_ceiling.
GAP_CEILING=0
for (( _g = 1; _g < REJUMP_THRESHOLD; _g++ )); do
    if (( $(_catchup_effective_gap "$_g") < REJUMP_THRESHOLD )); then GAP_CEILING=$_g; fi
done
if (( GAP < 0 || GAP > GAP_CEILING )); then
    echo "FAIL (smoke-cert-catchup): CERT_CATCHUP_GAP=$GAP is above the derived ceiling \
$GAP_CEILING at interval=$EPOCH_INTERVAL / delay=${DELAY_MS}ms (boot=$BOOT_BLOCKS, \
effective=$(_catchup_effective_gap "$GAP")): the effective gap would cross re_jump_threshold=$REJUMP_THRESHOLD, so \
maybe_re_jump would fast-forward past the derive-walk and the park could not fire. Raise \
EPOCH_BLOCK_INTERVAL, not the gap."
    exit 1
fi

# --- greppable log signatures (verbatim from the source -- see anchors) ----
# PARK_LOG is the ACTION half of the guard-#2 warn! (executor.rs:1473-1477), which the
# Rust source splits across two lines with a `\` continuation:
#   "guard #2: committee-attested body at h+K not backfilled yet; \
#   PARKING derive + hinting peers (event-driven re-poke, no give-up timer)"
# The two-word action token is deliberately the shortest unique slice of it, and that is
# not a style preference: the condition clause is the half that ALREADY got reworded out
# from under this case (see the header), while the action half survived the rewrite
# byte-identical. It is grep-unique in crates/ and that uniqueness is ASSERTED against the
# product source in dpos_harness/tests/test_smoke_onchain_cases.py -- the guard that was
# missing while this case grepped a wording the product had stopped emitting.
PARK_LOG='PARKING derive'                           # guard-#2 fresh-park warn! (executor.rs:1473)
REJUMP_LOG='EL-sync fast-forwarded the anchor'      # re-jump/cold-start landing info! (cold_start_jump.rs:874)
OLD_FATAL='cannot derive beacon prev_randao'        # DELETED pre-fix ERROR (gone from src); regression tripwire
EXIT_LOG='OuterEngine exited cleanly'               # the consensus_exit signature (node/dpos.rs:539)

# Scrape the victim's `deferred_height` gauge from its in-container metrics endpoint
# (--dpos.metrics-port=9100; curl-in-container, mirroring peer_count()). Echoes the
# integer value, or "na" if the endpoint/metric is not answering (container mid-
# restart). The park is transient -- the gauge clears the instant the h+K BODY lands and
# repoke_deferred re-derives -- so this is best-effort corroboration; the persistent
# park warn! is the authority.
deferred_gauge() {
    # `|| true` so a mid-restart docker-exec/curl failure does not trip pipefail+set -e
    # at the caller's `g=$(deferred_gauge ...)` assignment; awk then emits "na".
    local out
    out=$(docker compose exec -T "$1" curl -s http://localhost:9100 2>/dev/null || true)
    printf '%s\n' "$out" | awk '$1 ~ /(^|_)deferred_height$/{v=$2; f=1} END{if(f) print v; else print "na"}'
}

# --- the victim's catch-up latency (THE LEVER -- see the header) ------------
# Non-empty while a qdisc is live, holding the service it is live on. `cleanup` reads it,
# so the removal is UNCONDITIONAL: it fires on the success path, on every `exit 1` failure
# branch, and on an interrupt. Leaving a validator shaped would poison the optional deep
# cycle on the same stack and read as a product slowdown rather than as this case's doing.
NETEM_SHAPED=""

# Add ONE uniform egress delay on the victim. FAIL-LOUD by design (no `|| true`): it runs
# under `set -e`, so a missing NET_ADMIN or a missing `tc` aborts the case here instead of
# letting it run unshaped and then fail the park gate with a message pointing at the gap --
# the same "loud, not silently unshaped" rule gen-soak-compose.sh:144 states for the soak.
netem_apply() {
    local svc="$1" ms="$2"
    docker compose exec -T "$svc" tc qdisc add dev "$NETEM_IFACE" root netem delay "${ms}ms"
    NETEM_SHAPED="$svc"
}

# Remove it. Best-effort and idempotent (`|| true`): it is also the cleanup path, where the
# container may already be gone, the qdisc may never have been added, or the shell may be
# unwinding from an earlier failure -- none of which should mask the real error.
netem_clear() {
    local svc="${1:-$NETEM_SHAPED}"
    [[ -n "$svc" ]] || return 0
    docker compose exec -T "$svc" tc qdisc del dev "$NETEM_IFACE" root >/dev/null 2>&1 || true
    NETEM_SHAPED=""
}

bring_up_dpos
# Unshape BEFORE tearing down -- same composed-cleanup idiom as case-vrf-rotation.sh:88 /
# case-byzantine-vrf.sh:96. `tear_down` would destroy the containers anyway, but the trap
# is not only about teardown: an interrupted run with SMOKE_KEEP_UP-style manual poking, or
# a future case that reuses this bring-up, must not inherit a throttled validator.
cleanup() { netem_clear; tear_down; }
trap cleanup EXIT

# (0) PRE-FLIGHT: UNSHAPE THE VICTIM BEFORE TOUCHING IT AT ALL.
#
# Unconditional, failure-ignoring, and FIRST -- before the DKG wait, before any stop, on every
# run whether or not this one ever shapes anything. `cleanup` above covers the paths where THIS
# shell unwinds; it does NOT fire when the process is SIGKILLed, which is how a real run ended on
# 2026-07-30 and left validator-2 carrying a 1 s delay. netem lives in the container's netns, so
# a container that survives into the next bring-up keeps its qdisc, and a stack left shaped
# poisons every later case on it and reads as a product slowdown rather than as this case's
# doing. Self-healing beats a cleanup path that cannot be guaranteed to run: start by ASSERTING
# the condition the case needs instead of trusting that it was restored.
netem_clear "$VICTIM"
echo "netem pre-flight: cleared any stale qdisc on $VICTIM ($NETEM_IFACE)"

# Establish the bootstrap DKG BEFORE any disruption (mirrors case-liveness): a victim
# stopped before its epoch-2 ceremony is permanently shareless on rejoin (no reshare
# in v1) and cannot re-promote to signer. Advancing past epoch 2 makes every
# validator deal + PERSIST its share, so the restart exercises the SUPPORTED path (a
# member that attended its DKG, restarts, reloads its on-disk share, and catches up).
#
# THE BUDGET IS SIZED FOR interval=64 AND DOES NOT TRAVEL: activation is 2*EPOCH_INTERVAL
# (lib.sh:33) and this waits 2*EPOCH_INTERVAL past it, so the wait covers ~136 blocks past the
# 128-block anchor at 1 blk/s and 360 s is 2.6x headroom. Anyone changing EPOCH_BLOCK_INTERVAL
# must move this with it -- read the header's DO-NOT-RAISE block first.
EPOCH2_END=$(( DPOS_ACTIVATION_BLOCK + 2 * EPOCH_INTERVAL + 8 ))
echo "establishing bootstrap DKG: advancing past epoch 2 (finalized >= $EPOCH2_END) before disruption"
if ! wait_finalized_ge "$EPOCH2_END" 360; then
    echo "FAIL (smoke-cert-catchup): chain did not reach epoch 2 (block $EPOCH2_END) before disruption (finalized=$(finalized_dec))"
    docker compose logs --tail=120 validator-0
    exit 1
fi
echo "  bootstrap DKG epoch 2 finalized; $VICTIM holds a persisted share"

# One graceful-stop -> advance-gap -> restart -> rejoin cycle.
#   $1 gap (min blocks to advance while down)  $2 label  $3 require_park (0|1)
# Log counts are snapshotted BEFORE the stop so the assertions measure ONLY what the
# restart produced -- `docker compose stop`/`start` keeps the SAME container, so its
# log + datadir persist across the cycle (a genuine graceful restart, not a recreate).
catchup_cycle() {
    local gap="$1" label="$2" require_park="$3"
    local pre head park0 rejump0 fatal0 exit0 park1 rejump1 fatal1 exit1
    local deadline tick v0 vn g max_gauge=0 ok=0

    park0=$(log_count "$VICTIM" "$PARK_LOG")
    rejump0=$(log_count "$VICTIM" "$REJUMP_LOG")
    fatal0=$(log_count "$VICTIM" "$OLD_FATAL")
    exit0=$(log_count "$VICTIM" "$EXIT_LOG")

    pre=$(baseline_height)
    echo "── [$label] victim=$VICTIM  down-gap>=$gap  re_jump_threshold=$REJUMP_THRESHOLD  (pre=$pre, park_baseline=$park0) ──"

    # (2) GRACEFUL stop. The fix targets a graceful restart (flushed tail): verify
    #     reth's native graceful shutdown persisted (container exit 0) so the restart
    #     RESUMES from the persisted tail and must catch up (not a cold re-sync).
    docker compose stop --timeout 40 "$VICTIM"
    if ! shutdown_flushed "$VICTIM"; then
        echo "FAIL (smoke-cert-catchup): [$label] $VICTIM did not flush on graceful stop (exit != 0) -- not the graceful-restart scenario the fix targets"
        docker compose logs --tail=80 "$VICTIM"
        exit 1
    fi
    echo "  $VICTIM graceful-stopped + flushed (persisted tail)"

    # (3) advance the live committee (BFT f=1 holds with 3/4 up) by >= gap blocks so
    #     the restarted victim must catch up across a deep-enough gap to hit the park.
    if ! wait_finalized_ge $(( pre + gap )) $(( gap * 4 + 120 )); then
        echo "FAIL (smoke-cert-catchup): [$label] chain did not advance $gap blocks with $VICTIM down (finalized=$(finalized_dec), pre=$pre)"
        docker compose logs --tail=120 validator-0
        exit 1
    fi
    head=$(finalized_dec)
    echo "  committee advanced to $head with $VICTIM down (gap=$(( head - pre )) >= $gap)"

    # (4) graceful restart (same container -> same datadir + accumulated log).
    docker compose start "$VICTIM"

    # (4a) THE LEVER: shape the VICTIM ONLY, for the catch-up window ONLY. Applied here,
    #      after the restart, because the delay must bite on the BACKLOG WALK and nothing
    #      else -- while the victim was down it had no traffic to shape, and once it has
    #      rejoined the delay would only slow the live chain the case still has to measure.
    #      Nobody else is touched: the committee keeps finalizing at full LAN speed, which
    #      is the side of the race that must stay fast. Removed at (5a) on the success
    #      path and by `cleanup` on every other path.
    netem_apply "$VICTIM" "$DELAY_MS"
    echo "  netem: +${DELAY_MS}ms egress delay on $VICTIM ($NETEM_IFACE) for the catch-up window"

    # (5) REJOIN: poll until the victim realigns to the hub's finalized head; WHILE
    #     polling, sample deferred_height (the park is transient, so sample fast to
    #     catch it non-zero -- corroborates the persistent park warn!).
    deadline=$(( $(date +%s) + 240 )); tick=0
    while (( $(date +%s) < deadline )); do
        g=$(deferred_gauge "$VICTIM")
        if [[ "$g" =~ ^[0-9]+$ ]] && (( g > max_gauge )); then max_gauge=$g; fi
        v0=$(check_external 8545); vn=$(check_node docker compose exec -T "$VICTIM")
        # A floor is what makes this a CATCH-UP check rather than an agreement check, and it is
        # `pre + gap`, not `pre`. `pre` is the victim's OWN pre-stop height — it clears that on
        # its persisted tail the moment it restarts, proving nothing about catching up. What the
        # chain PROVABLY reached this cycle is `pre + gap`: step (3) above fails the case unless
        # `wait_finalized_ge $((pre+gap))` returns. `_aligned_now`'s floor is strict and applies
        # per reader (the hub clears it trivially), and it only honours a HEX floor.
        # What is DROPPED is the tip-vs-tip equality: the victim is walking a deep gap on a
        # chain producing a block a second, so it reaches the hub's chain long before it
        # reaches the hub's tip, and demanding both readings coincide made the 240s budget a
        # race. The fork half survives at the victim's OWN height (see `_aligned_now`).
        if _aligned_now "$(printf '0x%x' $(( pre + gap )))" "$v0" "$vn" >/dev/null; then
            ok=1; break
        fi
        if (( tick % 6 == 0 )); then echo "    t+$((tick*2))s: $VICTIM=$vn v0=$v0 deferred_height~$g"; fi
        tick=$((tick+1)); sleep 2
    done
    if (( ok != 1 )); then
        echo "FAIL (smoke-cert-catchup): [$label] $VICTIM did not rejoin within 240s ($VICTIM=$(check_node docker compose exec -T "$VICTIM"), v0=$(check_external 8545), floor=pre+gap=$((pre+gap)))"
        docker compose logs --tail=200 "$VICTIM"
        exit 1
    fi
    echo "  OK: $VICTIM rejoined at $vn (v0=$v0, floor=pre+gap=$((pre+gap)))"

    # (5a) The walk is done -- unshape immediately. The deep cycle runs a SECOND
    #      stop/gap/restart on this same stack, so a delay carried out of cycle 1 would
    #      silently change cycle 2's conditions. The failure branches above never reach
    #      this line; `cleanup`'s netem_clear covers them.
    netem_clear "$VICTIM"
    echo "  netem: cleared on $VICTIM (catch-up window closed)"

    park1=$(log_count "$VICTIM" "$PARK_LOG")
    rejump1=$(log_count "$VICTIM" "$REJUMP_LOG")
    fatal1=$(log_count "$VICTIM" "$OLD_FATAL")
    exit1=$(log_count "$VICTIM" "$EXIT_LOG")

    # (5b) regression guards: NO old fatal budget-shutdown, NO consensus_exit.
    if (( fatal1 > fatal0 )); then
        echo "FAIL (smoke-cert-catchup): [$label] the OLD fatal budget-shutdown ERROR reappeared on $VICTIM ('$OLD_FATAL') -- the fix regressed"
        docker compose logs --tail=200 "$VICTIM"
        exit 1
    fi
    if (( exit1 > exit0 )); then
        echo "FAIL (smoke-cert-catchup): [$label] $VICTIM logged the consensus_exit signature ('$EXIT_LOG') -- the executor shut down instead of parking"
        docker compose logs --tail=200 "$VICTIM"
        exit 1
    fi

    echo "  [$label] park warns: $park0 -> $park1 | re-jump landings: $rejump0 -> $rejump1 | deferred_height peak sampled: $max_gauge | old-fatal/consensus-exit: none"

    # (5c) THE GATE: the guard-#2 park must have been ACTUALLY exercised.
    if (( require_park == 1 )); then
        if (( park1 > park0 )); then
            echo "  GATE PASS: guard-#2 park EXERCISED -- $VICTIM parked $(( park1 - park0 )) fresh deferred block(s) on an absent h+K body, recovered event-driven, and rejoined"
        else
            echo "FAIL (smoke-cert-catchup): [$label] guard-#2 park NOT exercised (park warns unchanged: $park0 == $park1; deferred_height peak sampled=$max_gauge, netem delay=${DELAY_MS}ms on $VICTIM). The victim rejoined without ever parking. DO NOT just raise CERT_CATCHUP_GAP: the park needs the executor's derive to DRAIN the marshal's contiguous dispatched prefix (MAX_REPAIR=20 / MAX_PENDING_ACKS=16, outer.rs:223,229), and a deeper gap does not change that ratio -- it only risks crossing re_jump_threshold=$REJUMP_THRESHOLD (with boot drift and catch-up stall priced in, the practical ceiling is ~$GAP_CEILING) and letting maybe_re_jump fast-forward past the walk entirely. The lever is SLOWING THE VICTIM'S BODY FETCH relative to its derive, and it is already applied: RAISE CERT_CATCHUP_DELAY_MS (now ${DELAY_MS}) -- more RTT per repair batch, derive untouched. A green that never triggered the path is a FALSE PASS."
            docker compose logs --tail=200 "$VICTIM"
            exit 1
        fi
    else
        # deep cycle: the gap-gated re-jump is the target; park is best-effort.
        if (( rejump1 > rejump0 )); then
            echo "  [$label] gap-gated re-jump EXERCISED -- $VICTIM re-jumped $(( rejump1 - rejump0 )) time(s) past the deep gap (design.md §4.3), park warns $park0 -> $park1"
        else
            echo "  [$label] NOTE: re-jump landing not observed (the victim rejoined via the derive-walk before the gap crossed re_jump_threshold=$REJUMP_THRESHOLD); park warns $park0 -> $park1"
        fi
    fi
}

# CYCLE 1 (MANDATORY GATE): pure PARK. gap <= GAP_CEILING < re_jump_threshold(64) AND well
# under a full 64-block window (absent ≈44% of it leaves seen above the 15% participation
# floor) -> the derive-walk catch-up parks on blocks whose h+K body has not backfilled yet,
# with no re-jump stealing the walk and no liveness slash. This is the fix's core path.
catchup_cycle "$GAP" "park" 1

# CYCLE 2 (OPTIONAL, CERT_CATCHUP_DEEP=1): gap > re_jump_threshold to also exercise
# the gap-gated re-jump. Tolerates a liveness demotion (a deep gap can drop the victim
# below the participation floor for a full window).
if [[ "$DEEP" == 1 ]]; then
    catchup_cycle "$DEEP_GAP" "rejump-deep" 0
fi

REJUMP_NOTE=""
if [[ "$DEEP" == 1 ]]; then REJUMP_NOTE=" + gap-gated re-jump"; fi
echo "OK (smoke-cert-catchup): $VICTIM parked on deferred blocks whose h+K attestation body had not backfilled during graceful-restart catch-up, recovered event-driven (delivery/heartbeat re-poke${REJUMP_NOTE}), and rejoined the live tip -- NO budget-shutdown ERROR, NO consensus_exit"
