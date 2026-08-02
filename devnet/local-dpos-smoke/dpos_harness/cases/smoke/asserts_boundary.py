"""Assertions for the post-jump boundary-seeding cases.

ONE gate, and it is deliberately narrow: **a validator that jumped across an epoch boundary
must be signing again INSIDE the landing epoch, not one epoch later.** Everything else in here
is diagnosis for when that fails.

The negative worth naming, because it is what the pre-fix code did and what a careless case
would call a pass: the victim rejoins, its height realigns, its logs look healthy, and it
promotes — at `landing_epoch + 1`. Rejoin, log-health and promote-count assertions are all
satisfied by that. Two readings separate the two behaviours: the EPOCH on the promote line, and
the executor's own entry-call line naming the boundary it asked to enter on.

═══ WHY THE SEEDING READINGS RUN LAST AND DO NOT GATE ════════════════════════════════════

The driver is fail-fast, so gate ORDER decides which line an operator reads first — and a gate
that can be zero on a healthy cycle placed ahead of the real property reddens runs that proved
it. Seeding is condition-keyed: a member that already holds the boundary pair injects nothing,
logs nothing and moves no counter, and the Rust side pins that skip directly. So the seed line
and the refetch counter are PRINTED after the verdict, not asserted before it. The signals with
no legitimately-zero mode — a seed FAILURE, a both-or-neither abort, a rising refetch-failed
counter, a spawn-defer counter over budget — stay hard gates.

THE DEMOTION COST THE METRICS PLANE ITS ONLY LIVENESS READING, and that is repaired separately.
`evaluate_refetched` was the one gate that demanded a counter go UP; the two that remain both
demand a counter NOT go up, and an empty sample (dead :9100, renamed family) deltas to 0 and
satisfies them for free. The log gates cannot cover for that — they read `docker compose logs`,
not the `:9100` scrape. `vb.evaluate_metrics_plane_live` therefore gates the plane itself: every
`after` sample must have PARSED. A "0" is a live registry and passes.

═══ THE READS ARE TWO SNAPSHOTS OF ONE LOG, NOT TEN GREPS ════════════════════════════════

`ctx.log_count` re-reads the WHOLE container log per call (`nodes.log_count` -> `logs_all`), so a
body that greps six tokens one at a time is reading six DIFFERENT instants of a moving log and
comparing them as though they described one state. `_snapshot` captures the log ONCE before the
disruption and once after, and every count, both parsers and the landing height are derived from
those two texts. That is also what makes the counts honest DELTAS: `compose_stop`/`start` keeps
the same container, so its log persists across the cycle and an absolute count would report a
line the node wrote ten minutes ago as this cycle's evidence.

WHEN the second snapshot is taken is as load-bearing as the fact that there is only one of it:
the rejoin probe answers on the victim's RPC, which runs ahead of the consensus lines every gate
reads. `_await_landing_epoch_promote` holds it until the post-jump sequence has finished writing.

═══ WHY THE RESTART IS PHASE-STEERED ═════════════════════════════════════════════════════

Two of the gates ask what the victim did INSIDE the landing epoch, so both are hostage to where
in its epoch the jump lands. An unsteered landing draws its offset uniformly from 0..interval-1;
at interval 64 that leaves roughly one run in six with under ten blocks to promote AND lead a view
in — which is not a defect and not a pass, it is no measurement. `_wait_restart_window` holds the
restart until the chain sits in the first `RESTART_MAX_OFFSET` blocks of an epoch, so the landing
(boot drift ~7-10 blocks) falls early in one and leaves ~46-57 blocks of room. It costs at most
one epoch of waiting. Both epoch-scoped verdicts still report `epoch_room` on failure and call an
unlucky landing INCONCLUSIVE rather than red, so the steering is belt and the verdict is braces.
"""

from __future__ import annotations

import os

from ...core import topology
from . import asserts_onchain as ao, verdicts_boundary as vb, verdicts_onchain as vo

_say = ao._say


def _snapshot(ctx, victim: str) -> dict:
    """One log read + one metrics scrape, reduced to everything the gates compare against.

    Taken BEFORE the disruption and again after it; every verdict below is the difference of two
    of these.

    THE METRIC SAMPLES CAN BE EMPTY, AND EMPTY IS NOT ZERO. `node_metric` returns "" for an
    unregistered family, for a misspelled one and for a scrape that did not answer at all, and
    `vb.counter_delta` maps "" to 0 — so every remaining metric gate is a "did not rise" gate that
    an empty sample satisfies. Both spellings of the trap are live: the exposition adds its own
    `_total` on top of the registered name (hence `vb.counter_sample`, and reading the REGISTERED
    spelling through `nodes.gauge_val`'s anchored matcher returns ""), and a dead :9100 returns ""
    for everything. `vb.evaluate_metrics_plane_live` gates the `after` samples on having PARSED,
    which is the only reading that separates a live registry from a silent one.
    """
    log = ctx.logs_all(victim)
    return {
        "seed_rejump": vb.count_lines(log, vb.SEED_LOG_REJUMP),
        "seed_fail": vb.count_lines(log, vb.SEED_FAIL_LOG),
        "seed_partial": vb.count_lines(log, vb.SEED_PARTIAL_LOG),
        "rejump": vb.count_lines(log, vo.REJUMP_LOG),
        "landings": vb.landing_heights(log),
        "entered": vb.enter_boundaries(log),
        "seed_heights": vb.seed_heights(log),
        "promoted": vb.promoted_epoch_counts(log),
        "proposed": vb.propose_heights(log),
        "m_refetched": ctx.node_metric(victim, vb.counter_sample(vb.M_REFETCHED)),
        "m_refetch_failed": ctx.node_metric(victim, vb.counter_sample(vb.M_REFETCH_FAILED)),
        "m_spawn_deferred": ctx.node_metric(victim, vb.counter_sample(vb.M_SPAWN_DEFERRED)),
    }


def _wait_restart_window(ctx, case: str, victim: str, floor: int, timeout: int) -> None:
    """Hold the restart until the chain is past `floor` AND sitting early in an epoch.

    The gap gate and the phase gate are ONE condition, not two waits: releasing at `floor` and
    then sleeping to the next epoch start would leave the victim down for an unbounded extra
    stretch, and releasing on phase alone would restart it before the gap reached the re-jump
    threshold. See the module header for why the phase half exists at all.
    """
    def ready():
        h = ctx.finalized_dec()
        if h < int(floor):
            return False
        if vb.epoch_offset(h, ctx.interval, ctx.activation_block) > vb.RESTART_MAX_OFFSET:
            return False
        return h

    hit = ctx.poll(ready, timeout, poll_s=vb.RESTART_POLL_S, dry_value=int(floor))
    ctx.check(case, bool(hit),
              lambda: (f"chain did not reach finalized >= {floor} inside the first "
                       f"{vb.RESTART_MAX_OFFSET} blocks of an epoch within {timeout}s "
                       f"(finalized={ctx.finalized_dec()}, interval={ctx.interval}, "
                       f"activation={ctx.activation_block})"),
              on_fail=lambda: ctx.dump_logs(vb.BOUNDARY_LOG_TAIL, topology.PINNED_RPC_HOST))
    _say(ctx, f"  committee advanced to {hit} with {victim} down "
              f"(epoch offset {vb.epoch_offset(hit, ctx.interval, ctx.activation_block)} "
              f"<= {vb.RESTART_MAX_OFFSET} — the landing gets a full epoch to sign in)")


def _await_landing_epoch_promote(ctx, victim: str, base: dict) -> None:
    """Hold the `after` snapshot until the victim has logged its LANDING-EPOCH promotion.

    WHY THE SNAPSHOT NEEDS A WAIT AT ALL. `_await_rejoin` answers on the victim's RPC, and its
    height clears the floor as soon as the jump's EL backfill finishes — a second or more BEFORE
    the consensus side logs the landing, seeds the boundary pair and promotes. Snapshotting on
    that edge reads a log the promote line has not reached yet, and every gate below is a delta
    against that one read. See `verdicts_boundary.PROMOTE_WAIT_S` for the run that proved it.

    It is a WAIT and not a retry loop around the verdict: the landing and seed lines sit in the
    same few hundred milliseconds and are read from the same snapshot, so a poll that only
    covered the promote gate would leave the two gates ahead of it exposed to the same race.
    On expiry the snapshot is taken anyway and the verdicts speak — the wait never decides
    anything, it only decides WHEN the one honest reading is taken.
    """
    def promoted():
        log = ctx.logs_all(victim)
        landings = vb.landing_heights(log)
        if not landings:
            return False
        epoch = vb.epoch_of(landings[-1], ctx.interval, ctx.activation_block)
        return (vb.promoted_epoch_counts(log).get(epoch, 0)
                > int(base["promoted"].get(epoch, 0)))

    ctx.poll(promoted, vb.PROMOTE_WAIT_S, poll_s=vb.PROMOTE_POLL_S, dry_value=True)


def _assert_landed_and_entered(ctx, case: str, victim: str, base: dict, after: dict) -> int:
    """The jump happened AND the executor asked the epoch state machine to enter the landing epoch.

    Returns the landing height. Two gates and nothing else, because these two are the only ones
    ahead of the promote/propose gates that cannot be zero on a healthy cycle: the landing is the
    case's premise, and the entry CALL is condition-keyed on the landing rather than on whether
    anything needed seeding. The call is not the effect — the log line is written at the call site
    (see `vb.ENTER_LOG`), so this is a necessary condition and the promote/propose gates below are
    what observe that the state machine actually advanced. Everything the seeding did or did not
    do is diagnosis, and it is printed AFTER the property has been proved (`_report_seeding`).
    """
    def dump():
        ctx.dump_logs(vb.BOUNDARY_LOG_TAIL, victim)

    landings = after["landings"]
    ctx.check(case, *vb.evaluate_landed(landings, victim, vo.rejump_threshold(ctx.interval)),
              on_fail=dump)
    landing = landings[-1] if landings else ctx.finalized_dec()

    ok, msg, boundary = vb.evaluate_entered_landing_epoch(base["entered"], after["entered"],
                                                          landing, ctx.interval,
                                                          ctx.activation_block, victim)
    ctx.check(case, ok, msg, on_fail=dump)
    _say(ctx, f"  asked to enter the landing epoch on boundary={boundary} "
              f"(entry calls seen after the jump: {after['entered'] or 'none'})")
    return landing


def _report_seeding(ctx, case: str, victim: str, base: dict, after: dict) -> None:
    """What the seeding did, and the four ways it can genuinely BREAK.

    The split is between signals with a legitimately-zero mode and signals without one. The seed
    line and the refetch counter have one — a member that already holds the boundary pair injects
    nothing and logs nothing — so they are PRINTED. A seed failure, a both-or-neither abort, a
    rising `M_REFETCH_FAILED` and a spawn-defer counter over budget have no such mode, so they
    stay hard gates. They run last because a fail-fast driver would otherwise let a diagnosis
    pre-empt the verdict it exists to explain.

    The re-jump site is the only one this family COUNTS — the `seeded N` line is a delta on the
    re-jump seed log alone. That is a choice of what to report, NOT a claim that the cold-start
    injection is out of reach: it is gated on `marshal_floor.is_some()` (`outer.rs:766`) and the
    builder always passes `Some(...)` (`consensus/dpos.rs:2792-2793`), so it can fire at this gap
    too. Nothing here turns on the distinction — the seed HEIGHTS are read from both sites
    (`vb.seed_heights`), because which site injected a height is not what any gate asks.
    """
    def dump():
        ctx.dump_logs(vb.BOUNDARY_LOG_TAIL, victim)

    _, msg, seeded = vb.evaluate_seeded(base["seed_rejump"], after["seed_rejump"], victim,
                                        "re-jump", after["seed_heights"])
    _say(ctx, f"  re-jump site seeded {seeded} boundary block(s) at "
              f"{after['seed_heights'] or 'n/a'} below the new floor")
    if msg:
        _say(ctx, f"    {msg}")

    _, msg, n = vb.evaluate_refetched(base["m_refetched"], after["m_refetched"], victim)
    _say(ctx, f"  {vb.M_REFETCHED} +{n}, {vb.M_REFETCH_FAILED} +"
              f"{vb.counter_delta(base['m_refetch_failed'], after['m_refetch_failed'])}")
    if msg:
        _say(ctx, f"    {msg}")

    ctx.check(case, *vb.evaluate_no_refetch_failure(base["m_refetch_failed"],
                                                    after["m_refetch_failed"], victim),
              on_fail=dump)
    ctx.check(case, after["seed_fail"] == base["seed_fail"],
              f"{victim} logged a boundary-seed FAILURE ({vb.SEED_FAIL_LOG!r}) — it stays "
              "verify-only (no proposals, no votes) for the landing epoch, which is the pre-fix "
              "behaviour reached by a different route; read the `reason=` field",
              on_fail=dump)
    ctx.check(case, after["seed_partial"] == base["seed_partial"],
              f"{victim} aborted seeding both-or-neither ({vb.SEED_PARTIAL_LOG!r}): one height of "
              "the boundary pair was fetched and the other was not, so NEITHER was injected — "
              "seeding the terminal alone would promote the member at exactly the moment the "
              "promote VALUE-gate (which reads the epoch's first block) degrades to a no-op",
              on_fail=dump)


def _assert_signs_in_landing_epoch(ctx, case: str, victim: str, base: dict, after: dict,
                                   landing: int) -> None:
    """THE GATE. Promote must name the epoch the jump landed in — and the member must then
    actually lead a view inside it.

    Two assertions and not one: the promote line is the MECHANISM (the per-epoch engine spawned),
    the propose line is the EFFECT (it is signing). The pre-fix behaviour fails both; a fix that
    spawned an engine that never led would fail only the second.
    """
    def dump():
        ctx.dump_logs(vb.BOUNDARY_LOG_TAIL, victim)

    epoch = vb.epoch_of(landing, ctx.interval, ctx.activation_block)
    room = vb.epoch_room(landing, ctx.interval, ctx.activation_block)
    _say(ctx, f"  landing height={landing} -> epoch {epoch} (room left {room}); promote epochs "
              f"seen after the jump: {sorted(after['promoted'])}")

    ok, msg, _ = vb.evaluate_promoted_in_landing_epoch(base["promoted"], after["promoted"],
                                                       landing, ctx.interval,
                                                       ctx.activation_block, victim)
    ctx.check(case, ok, msg, on_fail=dump)

    # The propose gate polls: promoting and LEADING a view are separated by however long the VRF
    # takes to hand this member a slot. Bounded by the landing epoch itself — a proposal in a
    # later epoch is not evidence — so a stalled victim fails here rather than passing late.
    before_n = vb.proposals_in_epoch(base["proposed"], epoch, ctx.interval, ctx.activation_block)
    budget = max(vb.PROPOSE_MIN_TIMEOUT_S, room + vb.PROPOSE_SLACK_S)
    seen = {"n": before_n}

    def proposed():
        seen["n"] = vb.proposals_in_epoch(vb.propose_heights(ctx.logs_all(victim)), epoch,
                                          ctx.interval, ctx.activation_block)
        return seen["n"] > before_n

    ctx.poll(proposed, budget, poll_s=vb.PROPOSE_POLL_S, dry_value=True)
    ok, msg, n = vb.evaluate_proposed_in_landing_epoch(before_n, seen["n"], landing, ctx.interval,
                                                       ctx.activation_block, victim)
    ctx.check(case, ok, msg, on_fail=dump)
    _say(ctx, f"  {victim} proposed {n} block(s) in epoch {epoch} after re-promoting")

    # The metrics plane answered at all, gated ONCE for all three counters and placed here: after
    # the property has been decided on the log plane, and before the first gate that reads a
    # counter. Every metric gate left in this case is a "did not rise" gate, so a dead :9100 makes
    # them unfalsifiable rather than red (see `vb.evaluate_metrics_plane_live`).
    ctx.check(case, *vb.evaluate_metrics_plane_live(
        {vb.counter_sample(vb.M_REFETCHED): after["m_refetched"],
         vb.counter_sample(vb.M_REFETCH_FAILED): after["m_refetch_failed"],
         vb.counter_sample(vb.M_SPAWN_DEFERRED): after["m_spawn_deferred"]}, victim),
        on_fail=dump)

    ok, msg, n = vb.evaluate_spawn_defer_bounded(base["m_spawn_deferred"],
                                                 after["m_spawn_deferred"], victim, epoch)
    ctx.check(case, ok, msg, on_fail=dump)
    _say(ctx, f"  {vb.M_SPAWN_DEFERRED} +{n} (budget {vb.SPAWN_DEFER_BUDGET})")


def assert_rejump_signer(ctx) -> None:
    """STEADY-STATE site (`executor::reseed_forward`): a running validator falls behind far
    enough to re-jump, and must be signing again inside the landing epoch.

    THE GAP SIZE IS THE PATH SELECTOR, not the restart. Two gates with two thresholds:
    cold-start (at boot) is a FIXED 1024; the steady-state re-jump (running executor) is
    `min(1024, interval)`. `vb.boundary_gap` sits between them — at interval 64 that is 160 —
    so at boot the cold-start gate declines (160 < 1024) and the running executor then
    re-jumps (160 > 64). A `compose_stop`/`start` therefore reaches the STEADY-STATE path, and
    a naive "small gap + restart" reaches NEITHER.

    THE GAP'S OTHER CONSEQUENCE, stated because it is deliberate and looks like a bug in the
    logs: 160 blocks is two and a half epochs, so the victim produces nothing across two full
    epochs. That used to take the liveness slash, survivable only because the genesis kept the
    felony ladder out of reach. Both are deleted. The successor production-liveness tier WOULD
    fail a member that produced zero against a positive due share, but it ships flag-off and no
    case arms it — so today the gap has no on-chain consequence at all, and the victim is still
    seated and still in the committee when it returns.
    """
    case = "smoke-rejump-signer"
    victim = os.environ.get("BOUNDARY_VICTIM", vb.BOUNDARY_VICTIM)
    gap = int(os.environ.get("BOUNDARY_GAP", vb.boundary_gap(ctx.interval)))

    ao._wait_bootstrap_dkg(ctx, case, vb.BOUNDARY_DKG_WAIT_S, tail=vb.BOUNDARY_LOG_TAIL)

    base = _snapshot(ctx, victim)
    pre = ctx.baseline_height()
    _say(ctx, f"── victim={victim} down-gap>={gap} "
              f"(re-jump gate={vo.rejump_threshold(ctx.interval)}, cold-start gate="
              f"{vo.JUMP_THRESHOLD}, pre={pre}) ──")

    ctx.compose_stop(victim, timeout=vo.CATCHUP_STOP_TIMEOUT_S, note=f"boundary-stop {victim}")
    ctx.check(case, ctx.shutdown_flushed(victim),
              f"{victim} did not flush on graceful stop (exit != 0)",
              on_fail=lambda: ctx.dump_logs(vo.CATCHUP_FLUSH_TAIL, victim))

    # `gap * 4 + 120` is the catch-up case's advance budget; the phase steering can hold the
    # release for up to one more interval, so the interval is added rather than absorbed.
    _wait_restart_window(ctx, case, victim, floor=pre + gap,
                         timeout=gap * 4 + 120 + ctx.interval * 2)

    ctx.compose_start(victim, note=f"boundary-start {victim}")
    _await_rejoin(ctx, case, victim, floor=pre + gap)
    _await_landing_epoch_promote(ctx, victim, base)

    after = _snapshot(ctx, victim)
    landing = _assert_landed_and_entered(ctx, case, victim, base, after)
    _assert_signs_in_landing_epoch(ctx, case, victim, base, after, landing)
    _report_seeding(ctx, case, victim, base, after)


def _await_rejoin(ctx, case: str, victim: str, floor: int) -> None:
    def probe():
        v0 = ctx.check_external(topology.HOST_RPC_PORT)
        vn = ctx.check_node(victim)
        return (vn, v0) if vo.catchup_rejoined(v0, vn, floor, ctx.blockhash_at) else False

    landed = ctx.poll(probe, vb.BOUNDARY_REJOIN_TIMEOUT_S, poll_s=vo.CATCHUP_REJOIN_POLL_S,
                      dry_value=("0x0|0x0", "0x0|0x0"))
    ctx.check(case, bool(landed),
              lambda: (f"{victim} did not rejoin within {vb.BOUNDARY_REJOIN_TIMEOUT_S}s "
                       f"({victim}={ctx.check_node(victim)})"),
              on_fail=lambda: ctx.dump_logs(vb.BOUNDARY_LOG_TAIL, victim))
