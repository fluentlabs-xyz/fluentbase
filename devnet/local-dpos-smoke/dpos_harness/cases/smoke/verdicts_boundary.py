"""Constants + verdicts for the post-jump boundary-seeding cases.

THE DEFECT THESE GATE. A seated member whose marshal floor is teleported past an epoch
boundary by the EL-sync jump used to spend the REST OF THE LANDING EPOCH as a non-voting
verifier. The engine-spawn gate needs `epocher.last(E-1)`, after a jump that height is below
the floor, and nothing re-fetches below the floor. It self-healed at the next boundary, which
is exactly why it survived: no case asserted that a jumped victim resumes SIGNING.

WHY THE EXISTING CATCH-UP CASE DID NOT CATCH IT. `asserts_onchain.assert_cert_catchup` tunes
its gap BELOW the re-jump threshold on purpose ("no re-jump steals the walk"), and even its
optional deep cycle asserts only rejoin + park + two absent fatals. A validator could sit
silent for a full epoch in every run and no test would redden.

THE ASSERTION THAT MATTERS is therefore the EPOCH ON THE PROMOTE LINE, not its presence:
without the fix the member still promotes — one epoch later. `promoted_epoch ==
epoch_of(landing)` is the whole case; everything else here is diagnosis for when it fails.

═══ WHY THE PROMOTE GATE IS A COUNT MAP AND NOT A SET ════════════════════════════════════

`promoted_in_epoch(whole_log, E)` — presence over the accumulated log — is right only by
accident. `compose_stop`/`start` keeps the SAME container, so the log carries every promotion the
victim logged before the disruption; the gate is safe here ONLY because `boundary_gap` is wider
than two epochs, so the landing epoch cannot be one the victim already promoted for. Narrow the
gap, or reuse these verdicts for the cold-start case (where the victim is down for seconds and
the landing epoch IS the epoch it was already signing), and presence silently reads a pre-restart
line as this cycle's evidence. `promoted_epoch_counts` + `after[E] > before[E]` is the same test
without that dependency, and it is what §9 of the brief means by "deltas, not absolute counts".

═══ THE COUNTER EXPOSITION NAMES ARE NOT THE REGISTERED NAMES ════════════════════════════

`prometheus-client` appends `_total` to a COUNTER's sample name, whatever the registered name
already ends in. `[VERIFIED]` against a captured scrape (`soak-out/bundle-20260701T193521Z/rpc/
metrics-19100.txt`): `beacon_digest_fallback_total` is registered in `beacon/metrics.rs:91` and
the sample line reads

    # HELP beacon_digest_fallback_total …
    beacon_digest_fallback_total_total 0

So a case reading the REGISTERED spelling through `nodes.gauge_val`'s anchored matcher gets ""
back — indistinguishable from a counter that never moved, i.e. a silent green on the metric half
of the case. `counter_sample()` is the one place that suffix is applied; the constants stay the
registered names so they can be grepped against the Rust `ctx.register(…)` literal verbatim
(`tests/test_smoke_boundary_cases.py`).
"""

from __future__ import annotations

import re

from . import verdicts_onchain as vo

#: The two floor-raise sites log distinct success lines, so a case can tell WHICH injection
#: site ran. Both carry a `height` field.
SEED_LOG_COLD = "seeded epoch-boundary block below the marshal floor"
SEED_LOG_REJUMP = "seeded epoch-boundary block below the re-jump floor"

#: `executor.rs` — the re-jump's epoch-entry CALL SITE. It fires exactly once per landing, names
#: the boundary height the call was keyed on, and (unlike the seed line) has no legitimately-absent
#: mode, because the call is condition-keyed on the LANDING and not on whether anything needed
#: seeding. Carries `landing`, `boundary` and `floor`.
#:
#: WHAT IT DOES NOT WITNESS. The line is emitted BEFORE the entry call and outside the seam, so it
#: says the executor asked the epoch state machine to enter on that boundary — a NECESSARY
#: condition for the entry, not evidence that the state machine advanced. A build wired to an
#: inert entry seam prints this line with a correct `boundary=` and enters nothing; the promote
#: and propose gates are what observe the effect.
ENTER_LOG = "entering the landing epoch after a re-jump"

#: Per-height seeding failure (`cert_follow.rs`), carrying `height` + a `reason`. Any hit means
#: the member stayed verify-only for its landing epoch — the pre-fix behaviour, degraded to
#: loudly rather than silently.
SEED_FAIL_LOG = "epoch-boundary block could not be seeded"

#: The both-or-neither abort: one height of the pair fetched, the other did not, so NEITHER is
#: injected (seeding only the terminal would promote the member at exactly the moment the
#: promote VALUE-gate — which reads the epoch's first block — degrades to a no-op).
SEED_PARTIAL_LOG = "epoch-boundary seeding incomplete"

#: The gate itself (`epoch_manager.rs`). One or two of these during ordinary backfill is
#: NORMAL and self-heals on the next derived block; the defect is a defer that persists across
#: the epoch, which is what the promote-epoch assertion actually measures.
DEFER_LOG = "signer spawn deferred"

#: `epoch_manager.rs:853` — the stable in-process Verifier→Signer token. Shared with the
#: rotation family (`verdicts_rotation.PROMOTED_LINE`); duplicated rather than imported so a
#: rotation-side edit cannot silently retarget this case.
PROMOTED_LINE = "promoted to Signer in-process"

#: `application.rs:1210` — `FluentApp::propose` is LEADER-GATED, and a verify-only member has no
#: per-epoch engine at all, so this line cannot be emitted by the state the fix exists to escape.
#: It is the END-TO-END half of the case (the promote line is the mechanism half): brief §3 calls
#: "it proposes again, in the landing epoch" the deliverable. Carries a `height`.
PROPOSE_LINE = "dpos: proposing order block"

#: `docker compose logs` renders the tracing fields ahead of the message. Both orderings appear
#: across services, so the epoch is matched on the LINE rather than positionally.
#:
#: `Epoch` is a `#[derive(Debug)]` NEWTYPE (`commonware_consensus::types::Epoch`) and the product
#: logs it with `?epoch`, so the rendered field is `epoch=Epoch(12)` and NOT `epoch=12` —
#: `[VERIFIED]` against a captured log (`soak-out/bundle-20260629T100526Z/logs/validator-2.log`,
#: ANSI-stripped: `… per-epoch BFT engine started epoch=Epoch(0)`). The bare-`u64` alternative is
#: kept because the beacon lines DO render one and a future `%epoch` would too;
#: `verdicts_onchain.epoch_field_lines`'s `epoch=<N>` pattern matches NEITHER of the two lines
#: these cases read, which is why this case does not reuse it.
_PROMOTED_EPOCH_RE = re.compile(r"epoch[=:]?\s*(?:Epoch\()?(\d+)")

#: `height=<n>`, LEFT-anchored so `to=`/`from=` cannot bleed in and right-bounded so `height=12`
#: does not answer for `height=1234`.
_HEIGHT_RE = re.compile(r"(?:^|[\s\[])height[=:]\s*(\d+)(?!\d)")

#: `boundary=<n>` on the entry line. Same anchoring as `_HEIGHT_RE`, and it matters more here: the
#: entry line also carries `landing=` and `floor=`, and the whole point of the gate is WHICH of
#: those three numbers the entry was keyed on.
_BOUNDARY_RE = re.compile(r"(?:^|[\s\[])boundary[=:]\s*(\d+)(?!\d)")

#: Victim + geometry. Interval 64 is the smallest interval that leaves a usable band between the
#: two jump gates (see `boundary_gap`), which is this case's own reason for it.
#:
#: A LITERAL, deliberately, and it used to alias `vo.CATCHUP_EPOCH_INTERVAL`. The two cases
#: happened to agree on 64 for two unrelated reasons, and the alias meant retuning the catch-up
#: case's re-jump ceiling silently retuned this case's genesis and every budget derived from it.
#: Shared value, not shared requirement — so it is pinned here.
BOUNDARY_EPOCH_INTERVAL = 64
BOUNDARY_VICTIM = vo.CATCHUP_VICTIM

#: Seconds to wait for the victim to rejoin and re-promote after the restart. The walk is
#: `boundary_gap` blocks of derive plus a re-jump round-trip; the catch-up case budgets
#: `gap * 4 + 120` for the advance alone, and this is the same shape on the recovery side.
BOUNDARY_REJOIN_TIMEOUT_S = 300

#: Log tail dumped on a failure path only.
BOUNDARY_LOG_TAIL = vo.CATCHUP_LOG_TAIL

#: The bootstrap-DKG wait before any disruption. Sized for THIS case's 64-block interval — the
#: target is `activation + 2*interval + 8` off a `2*interval` anchor, so it scales with the
#: interval and a budget borrowed from a case on a different interval is the wrong number in
#: whichever direction that case moves. It used to be `vo.CATCHUP_DKG_WAIT_S`, and when the
#: catch-up case went to 128-block epochs that alias would have silently doubled this case's
#: patience for a genuine stall.
BOUNDARY_DKG_WAIT_S = 360

# ══ the metrics ═══════════════════════════════════════════════════════════════════════
#
# `sync_metrics.rs:159,165` and `beacon/metrics.rs:129` — the REGISTERED names. See the module
# header for why these are not what the scrape calls them.

M_REFETCHED = "dpos_jump_boundary_refetched_total"
M_REFETCH_FAILED = "dpos_jump_boundary_refetch_failed_total"
M_SPAWN_DEFERRED = "epoch_engine_spawn_deferred_total"

#: `prometheus-client`'s counter-sample suffix.
COUNTER_SAMPLE_SUFFIX = "_total"

#: The `epoch_engine_spawn_deferred_total` BELT (brief §4). A defer or two while the marshal
#: backfills is NORMAL and self-heals on the next derived block; the DEFECT is a defer that
#: persists across the epoch, and the pre-fix behaviour re-fires the gate on every executor
#: `spawn_unblocked` edge for the whole landing epoch — tens of increments, not a handful. The
#: budget therefore sits BETWEEN the two regimes rather than at zero, so an ordinary backfill
#: defer does not redden a healthy run (brief §9's last bullet).
SPAWN_DEFER_BUDGET = 8

# ══ the epoch geometry the gates depend on ════════════════════════════════════════════

#: The floor `epoch_room` must clear for a missing promotion to be read as THE DEFECT rather than
#: as an unlucky landing. Seeding is synchronous (both sites inject BEFORE raising the floor) and
#: the spawn gate is re-poked on the next derived block, so a promotion follows the landing within
#: a couple of blocks; ten is comfortably above that and far below a 64-block epoch.
MIN_EPOCH_ROOM = 10

#: THE STEERING KNOB, and the reason the two epoch-scoped gates are not a coin flip.
#:
#: Both gates ask what the victim did INSIDE the landing epoch, so both are hostage to where in
#: its epoch the jump happens to land. Landing 3 blocks before the terminal is not a defect and
#: not a pass — it is no measurement at all, and at interval 64 an unsteered landing draws its
#: offset uniformly from 0..63, so roughly one run in six would have under ten blocks to work in.
#: The case therefore holds the restart until the chain sits in the FIRST `RESTART_MAX_OFFSET`
#: blocks of an epoch. Victim boot drift is ~7-10 blocks (measured on this stack), so the landing
#: then falls ~7-18 blocks into an epoch and leaves ~46-57 blocks of room. Costs at most one epoch
#: of extra waiting and buys the difference between a gate and a lottery.
RESTART_MAX_OFFSET = 8
#: 1 s poll against a 1 blk/s chain: a 2 s cadence can step straight over an 8-block window.
RESTART_POLL_S = 1

#: How long the propose gate waits for the victim to actually LEAD a view once it has promoted.
#: The real bound is the landing epoch itself (a propose in a later epoch is not evidence), so the
#: caller derives `epoch_room + PROPOSE_SLACK_S`; this is only the floor for a landing that is
#: already near the terminal, where the verdict's own room diagnostic is what the operator reads.
PROPOSE_SLACK_S = 15
PROPOSE_MIN_TIMEOUT_S = 20
PROPOSE_POLL_S = 2

#: How long the `after` snapshot is held for the victim's landing-epoch PROMOTION to appear.
#:
#: THE REJOIN PROBE ANSWERS ON A DIFFERENT PLANE FROM THE GATE. `catchup_rejoined` reads the
#: victim's RPC, and the victim's height clears the floor the moment the jump's EL backfill
#: finishes — which is BEFORE the consensus side logs the landing, seeds the boundary pair and
#: promotes. A one-shot `after` read on that edge samples a log the promote line has not been
#: written to yet, and the promote gate (unlike the propose gate) had no poll of its own.
#:
#: `[VERIFIED]` on the 2026-07-31 run: landing logged at 20:29:39.097, seed at .105, promote for
#: the landing epoch at .421 — the snapshot caught the first two and missed the third by 0.3 s,
#: and the case reported THE PRE-FIX SIGNATURE against a node that was already signing. The
#: same commit and config passed the run before it. The window is the gap between the two
#: planes, not a rare draw, so it is closed with a wait rather than a bigger margin.
#:
#: IT CANNOT MANUFACTURE A GREEN. The wait is for a promotion NAMING THE LANDING EPOCH; a member
#: that promotes one epoch late — the pre-fix behaviour — never writes that line at all, so it
#: burns the whole budget and the gate then fails on an honest reading. The budget stays well
#: under one epoch's room (~46-57 blocks ≈ s, by `RESTART_MAX_OFFSET` steering) so a genuine
#: failure still leaves the landing epoch's diagnostics meaningful.
PROMOTE_WAIT_S = 30
PROMOTE_POLL_S = 2


def counter_sample(name: str) -> str:
    """The registered counter name -> the name its SAMPLE line carries in the scrape.

    One place, because the alternative is every reader remembering the double suffix and one of
    them not doing so — and that failure mode is a metric that reads as a flat 0 forever."""
    return name + COUNTER_SAMPLE_SUFFIX


def counter_delta(before, after) -> int:
    """`after - before` over two metric SAMPLES, either of which may be `""`.

    `rpc.metrics_get_exec` returns "" both for a metric that is not in the registry and for a
    scrape that did not answer, and an absent sample reads as 0 here DELIBERATELY: it is the
    right direction on the "did it move" diagnostics, which then report not-moved rather than
    work.

    IT IS THE WRONG DIRECTION ON THE "DID NOT RISE" GATES, and nothing here saves them. A dead
    :9100 registry makes every sample "", every delta 0, and both `evaluate_no_refetch_failure`
    and `evaluate_spawn_defer_bounded` pass for free. The log-line gates do NOT cover that: they
    read `docker compose logs` (`core/nodes.py`), a wholly different plane from the
    `docker compose exec … curl :9100` scrape (`core/rpc.py`), so a run can be green on every log
    gate while the metrics plane answered nothing at all. `evaluate_metrics_plane_live` is what
    closes that, by asserting the samples PARSED rather than that they moved."""
    def num(v):
        tok = str(v if v is not None else "").strip()
        try:
            return int(float(tok))
        except ValueError:
            return 0
    return num(after) - num(before)


def boundary_gap(interval) -> int:
    """The down-gap that reaches the STEADY-STATE re-jump and NOT the cold-start jump.

    Two gates, two thresholds, and a case that confuses them tests nothing:

      * cold-start (pre-engine, at boot): FIXED `JUMP_THRESHOLD` = 1024;
      * steady-state re-jump (running executor): `min(1024, epochBlockInterval)`.

    So the gap must sit ABOVE `min(1024, interval)` and BELOW 1024. At interval 64 that band is
    (64, 1024). Reusing `vo.deep_gap` — `2*interval + interval/2`, i.e. 160 — keeps this case's
    gap identical to the catch-up case's optional deep cycle, which is the same choreography;
    the difference is entirely in what gets asserted afterwards.

    Consequence worth stating because it is counter-intuitive: a `compose_stop`/`start` DOES
    reach the steady-state path. At boot the cold-start gate sees 160 < 1024 and declines, then
    the running executor sees 160 > 64 and re-jumps. The restart is not what selects the path —
    the gap size is.
    """
    return vo.deep_gap(interval)


def epoch_of(height, interval, activation) -> int:
    """Relative epoch of an absolute height (`OriginEpocher`: `(h - origin) / length`)."""
    return (int(height) - int(activation)) // int(interval)


def epoch_last(epoch, interval, activation) -> int:
    """The TERMINAL height of a relative epoch — `first + length - 1` (`OriginEpocher::bounds`).
    This is the height the engine-spawn gate needs for `epoch + 1`, i.e. the one the fix seeds."""
    return int(activation) + (int(epoch) + 1) * int(interval) - 1


def epoch_offset(height, interval, activation) -> int:
    """How far INTO its epoch `height` sits (0 = the epoch's first block)."""
    return max(0, int(height) - int(activation)) % int(interval)


def epoch_room(height, interval, activation) -> int:
    """How many blocks of `height`'s epoch are still to come.

    The one environmental quantity that can make a CORRECT fix miss both epoch-scoped gates: a
    jump that lands three blocks before the terminal leaves no room to promote and lead a view
    inside the landing epoch, whatever the fix does. `RESTART_MAX_OFFSET` steers the landing away
    from that band, and every failure message prints this number so an unlucky landing is never
    reported as the defect."""
    return epoch_last(epoch_of(height, interval, activation), interval, activation) - int(height)


def count_lines(log_text: str, token: str) -> int:
    """`ctx.log_count`, but over an already-captured log.

    Both snapshots read the WHOLE log ONCE and count every token against that one text —
    `nodes.log_count` re-runs `docker compose logs` per call, so six tokens grepped one at a time
    describe six different instants of a moving log and are then compared as one state. Same ANSI
    contract: `nodes.logs_all` strips unconditionally (§2.4 item 2), and without the strip the
    node's own `key=value` pairs render as `height<ESC>=<ESC>1234` so every parser below reads
    nothing — a SILENT reader, which on a delta gate is indistinguishable from a clean run."""
    return sum(1 for line in (log_text or "").splitlines() if token in line)


def seed_heights(log_text: str) -> list[int]:
    """Every `height` a boundary-seed success line named, from EITHER floor-raise site.

    Both sites are read because the pair is what matters, not which site injected it: the fix
    seeds `b` and `b + 1` together (both-or-neither), so a healthy cycle reports two heights and
    a single height would mean that guard was bypassed."""
    out = []
    for line in (log_text or "").splitlines():
        if SEED_LOG_COLD not in line and SEED_LOG_REJUMP not in line:
            continue
        m = _HEIGHT_RE.search(line)
        if m:
            out.append(int(m.group(1)))
    return out


def enter_boundaries(log_text: str) -> list[int]:
    """Every `boundary=` height an epoch-entry line named, in log order.

    The VALUE and not the count, because the count alone cannot tell a correct entry from one that
    named the wrong epoch: `terminal_at_or_below(floor)` and `terminal_at_or_below(landing)` are
    the same height for most landings and a whole epoch apart at two geometries — a landing within
    K of an epoch start (the floor falls into the previous epoch) and a landing that IS its own
    epoch's terminal (the landing-keyed answer is the landing itself). An entry keyed on the floor
    would fire, log, and enter the epoch BEFORE the one the node is in."""
    out = []
    for line in (log_text or "").splitlines():
        if ENTER_LOG not in line:
            continue
        m = _BOUNDARY_RE.search(line)
        if m:
            out.append(int(m.group(1)))
    return out


def promoted_epochs(log_text: str) -> list[int]:
    """Every epoch number appearing on a promote line, in log order.

    Parsed from the line rather than counted, because the count alone cannot distinguish the
    fixed behaviour from the broken one: an unfixed node promotes too, just one epoch later.
    """
    out = []
    for line in log_text.splitlines():
        if PROMOTED_LINE not in line:
            continue
        m = _PROMOTED_EPOCH_RE.search(line)
        if m:
            out.append(int(m.group(1)))
    return out


def promoted_epoch_counts(log_text: str) -> dict:
    """`{epoch: how many promote lines named it}` — the delta-capable form of `promoted_epochs`.

    A COUNT and not a set membership: a node that restarts inside an epoch it was already a
    signer for logs the line a SECOND time, and a set difference would cancel exactly the
    observation the gate is after. See the module header."""
    out = {}
    for e in promoted_epochs(log_text):
        out[e] = out.get(e, 0) + 1
    return out


def propose_heights(log_text: str) -> list[int]:
    """Every `height` a `dpos: proposing order block` line named, in log order."""
    out = []
    for line in log_text.splitlines():
        if PROPOSE_LINE not in line:
            continue
        m = _HEIGHT_RE.search(line)
        if m:
            out.append(int(m.group(1)))
    return out


def proposals_in_epoch(heights, epoch, interval, activation) -> int:
    """How many of `heights` fall inside `epoch`."""
    e = int(epoch)
    return sum(1 for h in heights if epoch_of(h, interval, activation) == e)


#: `cold_start_jump.rs:874-879` logs `from`, `to` and `floor` alongside the landing message, so
#: the landing HEIGHT is readable from the victim's own log.
_LANDING_TO_RE = re.compile(r"\bto[=:]\s*(\d+)")


def landing_heights(log_text: str) -> list[int]:
    """Every jump-landing height in the victim's log, in order.

    Read from the log rather than inferred from the chain clock ON PURPOSE. The alternative —
    "epoch of the finalized head at the moment the victim rejoined" — makes the case's verdict
    depend on how long the rejoin poll happened to take: at interval 64 an epoch is ~64 s, so a
    slow poll would drift into the NEXT epoch, where a broken node also promotes, and the case
    would pass vacuously. The landing height is a fact about the run, not about the observer.
    """
    out = []
    for line in log_text.splitlines():
        if vo.REJUMP_LOG not in line:
            continue
        m = _LANDING_TO_RE.search(line)
        if m:
            out.append(int(m.group(1)))
    return out


# ══ the verdicts ══════════════════════════════════════════════════════════════════════
#
# `(ok: bool, message: str)` — empty message on success, the operator diagnostic on failure, the
# same convention as `verdicts_onchain`. They live here rather than inline in `asserts_boundary`
# for the reason that whole split exists: a FAIL branch can only be driven from a unit test, and
# a live run only ever walks the PASS side (`driver.SmokeCtx.check` is a no-op under `--dry-run`).
# `tests/test_smoke_boundary_verdicts.py` drives every one of them through BOTH outcomes.

def evaluate_landed(landings, victim, threshold):
    """THE PREMISE, and the anti-vacuity gate for the gap.

    Everything below is measured against `epoch_of(landing)`, so a case that silently took "no
    landing" would compare against a landing that never happened. It is also the only thing
    standing between this case and a green run in which the victim simply walked the backlog: a
    victim that never jumped never raises a floor, never buries a boundary, and satisfies every
    "no failure" negative below perfectly."""
    if landings:
        return True, ""
    return False, (f"{victim} never logged a jump landing ({vo.REJUMP_LOG!r} with a parsable "
                   f"`to=<height>`) — the gap did not reach the re-jump gate ({threshold}), so "
                   "nothing below would mean anything. Either the victim closed the gap on the "
                   "ordinary derive-walk before its first Update::Tip (RAISE BOUNDARY_GAP), or "
                   "the landing line's `to=` field moved and this case can no longer locate the "
                   "landing epoch.")


def evaluate_entered_landing_epoch(before, after, landing, interval, activation, victim):
    """THE CALL-SITE WITNESS. The executor asked to enter on the boundary the landing implies.

    `before`/`after` are `enter_boundaries(log)` over the two snapshots, so the gate is a delta on
    a container log that survives `compose_stop`/`start` — the same shape as the promote gate and
    for the same reason.

    A NECESSARY CONDITION AND NOT A PROOF: the line is written at the call site, before the entry
    and outside the seam (see `ENTER_LOG`), so its absence is conclusive and its presence is not.
    What it buys over the demoted seed/refetch gates is that it cannot be legitimately absent —
    seeding is condition-keyed on whether the pair was already local, this call is condition-keyed
    on the LANDING. The effect is gated downstream, by promote and propose.

    THE OFF-BY-ONE HALF. The `boundary=` must name `terminal_at_or_below(landing)`: an entry keyed
    on the marshal FLOOR instead of the landing names the terminal an epoch earlier and enters the
    epoch BEFORE the one the node is in, which every other gate here except the promote one reads
    as green. That terminal is `epoch_last(E - 1)` for an ordinary landing and the LANDING ITSELF
    when the landing is its own epoch's terminal — there the product enters `E + 1` directly
    (`epoch_transition::apply_at` sets `is_boundary` and tracks `epoch_e + 1`), no block of `E`
    remains to sign, and demanding `epoch_last(E - 1)` would red a correct run. That case is
    exactly `epoch_room == 0`, where the promote gate's `MIN_EPOCH_ROOM` INCONCLUSIVE branch is
    the verdict the run deserves — so this gate must not pre-empt it."""
    e = epoch_of(landing, interval, activation)
    terminal = epoch_last(e, interval, activation)
    want = terminal if int(landing) == terminal else epoch_last(e - 1, interval, activation)
    prior = list(before or [])
    seen = list(after or [])
    fresh = seen[len(prior):]
    if not fresh:
        return False, (
            f"{victim} never logged {ENTER_LOG!r} after the jump (entries unchanged at "
            f"{len(prior)}). The floor rose past an epoch boundary and the epoch state machine "
            f"was never told, so the landing epoch {e} is entered only at the NEXT boundary and "
            f"the member stays verify-only (no proposals, no votes) until then — THE PRE-FIX "
            f"BEHAVIOUR. landing={landing}, expected boundary={want}."), None
    got = fresh[-1]
    if got == want:
        return True, "", got
    return False, (
        f"{victim} asked to enter on boundary={got}, but the landing implies boundary={want} "
        f"(landing={landing}, interval={interval}, activation={activation}) — the call named the "
        f"WRONG epoch. The two heights differ when the landing sits within K of an epoch start "
        f"and when the landing is itself an epoch terminal; either is where a call keyed on the "
        f"marshal FLOOR instead of the landing goes one whole epoch backwards."), got


def evaluate_seeded(before, after, victim, site, heights):
    """DIAGNOSTIC, not a gate: the seed line fired and the boundary pair was injected.

    Printed rather than asserted because seeding is condition-keyed and a member that already
    holds the pair does no work and logs nothing (`executor::reseed_forward_skips_present_boundary`
    pins that skip). A zero here is therefore a legitimate cycle, and asserting it ahead of the
    promote/propose gates in a fail-fast driver reddened runs in which the victim entered the
    landing epoch and signed. `evaluate_entered_landing_epoch` is the gate that has no such mode.

    The FAILURE signals around it — `SEED_FAIL_LOG`, `SEED_PARTIAL_LOG`, `M_REFETCH_FAILED` — stay
    hard gates: a rise in any of them means something genuinely broke."""
    n = int(after) - int(before)
    if n > 0:
        return True, "", n
    return False, (f"{victim} logged no {site} boundary-seed line (count unchanged at {before}). "
                   "Expected when the boundary pair was already local — the injection is skipped "
                   "and nothing is written. If the entry gate above also failed, read this as the "
                   f"floor having risen past a boundary with NOTHING injected below it. Heights "
                   f"seen: {heights or 'none'}."), 0


def evaluate_refetched(before, after, victim):
    """DIAGNOSTIC, not a gate: the refetch counter moved, on a different plane from the log.

    Same legitimately-zero mode as `evaluate_seeded` — the counter is bumped once per height
    actually injected, so a cycle whose pair was already local leaves it flat. An ABSENT sample
    reads as 0 too (the scrape names counters `<registered>_total`, hence `counter_sample`), which
    is why this cannot be a gate on its own even when the mode does not apply."""
    n = counter_delta(before, after)
    if n > 0:
        return True, "", n
    return False, (f"{victim}'s {M_REFETCHED} did not move ({before or 'absent'} -> "
                   f"{after or 'absent'}): no epoch-boundary block was injected through the cert "
                   f"upstream. Expected when the pair was already local. An ABSENT sample reads "
                   f"the same — the scrape names counters {counter_sample(M_REFETCHED)!r} "
                   "(prometheus-client appends its own _total), so an empty reading is a binary "
                   "without the metric or an unreachable :9100 registry."), 0


def evaluate_metrics_plane_live(samples, victim):
    """THE METRICS-PLANE LIVENESS GATE: every sample PARSED, not that any of them moved.

    Both remaining metric gates are "did not rise" gates, and `counter_delta` maps an empty sample
    to 0, so a victim whose :9100 registry never answered satisfies both for free — a green run
    with two blind gates. Nothing else in the case notices: the log gates read `docker compose
    logs`, a different plane from the `docker compose exec … curl :9100` scrape, and they stay
    fully readable while the scrape is dead.

    "Parsed" is `!= ""` and nothing stronger, because that is the only thing the samples share. A
    legitimately-zero counter — a cycle whose boundary pair was already local — reads "0", which
    is a LIVE registry and must not redden; the demoted `evaluate_refetched` is what asks whether
    a counter moved, and it was the only positive-direction read in the case, which is why its
    demotion is what left the other two unfalsifiable."""
    read = dict(samples or {})
    absent = sorted(name for name, value in read.items()
                    if str(value if value is not None else "").strip() == "")
    if read and not absent:
        return True, ""
    if not read:
        return False, (f"the metrics-plane gate for {victim} was handed NO samples — it would pass "
                       "on an empty map, which is the vacuity it exists to prevent.")
    return False, (
        f"{victim}'s metrics registry answered nothing for {absent} — an EMPTY sample, not a zero. "
        f"Every gate below reading those counters is a 'did not rise' gate and an empty sample "
        f"deltas to 0, so they would all pass blind. Either the :9100 scrape did not answer (check "
        f"the container and the port), or the binary no longer registers the family under that "
        f"name — note the scrape spells counters '<registered>_total' on top of whatever the "
        f"registered name already ends in.")


def evaluate_no_refetch_failure(before, after, victim):
    """The seeding did not FAIL for any height.

    Distinct from "the fix never ran" and given its own gate because the consequence is identical
    to the pre-fix behaviour while the cause is not: the member stays verify-only, but because an
    upstream would not serve the height / served a different one / served a cert whose payload
    does not match the block digest / could not be checked against a readable committee. Those are
    the four `reason=` values on `SEED_FAIL_LOG`, and they point at four different places."""
    n = counter_delta(before, after)
    if n <= 0:
        return True, ""
    return False, (f"{victim}'s {M_REFETCH_FAILED} rose by {n} — boundary seeding FAILED and the "
                   "member stays verify-only (no proposals, no votes) for its landing epoch. Grep "
                   f"its log for {SEED_FAIL_LOG!r} and read the `reason=` field.")


def evaluate_promoted_in_landing_epoch(before_counts, after_counts, landing, interval, activation,
                                       victim):
    """THE CASE. The victim promoted to Signer FOR THE LANDING EPOCH, not for the one after it.

    `after[E] > before[E]`, per epoch — see the module header for why neither a presence test nor
    a set difference will do.

    WITHOUT THE FIX THIS IS THE LOAD-BEARING RED, and it is the ONLY assertion in the suite that
    separates the two behaviours: the promote line still appears, naming `E + 1`. That branch gets
    its own message because it is the pre-fix SIGNATURE and not a generic miss — an operator
    seeing it should look at the two floor-raise sites, not at the chain."""
    e = epoch_of(landing, interval, activation)
    before = int(before_counts.get(e, 0))
    after = int(after_counts.get(e, 0))
    if after > before:
        return True, "", e
    room = epoch_room(landing, interval, activation)
    fresh = sorted(k for k, v in after_counts.items() if v > int(before_counts.get(k, 0)))
    seen = f"epochs promoted after the jump: {fresh or 'none'}"
    if e + 1 in fresh:
        return False, (
            f"{victim} did NOT promote in the LANDING epoch {e} (count {before} -> {after}) and "
            f"promoted in epoch {e + 1} instead — THE PRE-FIX SIGNATURE. The member spent the "
            f"whole of epoch {e} verify-only (no proposals, no votes) because the E-1 boundary "
            f"block sat below the marshal floor, where repair starts at floor+1 and nothing ever "
            f"re-fetches it. landing={landing}, room-left-in-epoch={room}. {seen}"), e
    if room < MIN_EPOCH_ROOM:
        return False, (
            f"INCONCLUSIVE, not a verdict — RE-RUN. The jump landed at {landing}, only {room} "
            f"block(s) before the epoch-{e} terminal (< MIN_EPOCH_ROOM={MIN_EPOCH_ROOM}), so "
            f"{victim} had no room to promote inside it whatever the fix does. Steering "
            f"(RESTART_MAX_OFFSET={RESTART_MAX_OFFSET}) should have prevented this: check the "
            f"restart-window poll. {seen}"), e
    return False, (f"{victim} did NOT promote in the LANDING epoch {e} (count {before} -> "
                   f"{after}) despite {room} blocks left in it, and the read was held "
                   f"{PROMOTE_WAIT_S}s for the line, so this is not the rejoin/promote read "
                   f"race. landing={landing}. {seen}. Next: grep the victim for {DEFER_LOG!r} "
                   f"(its `boundary=` field names the height the gate wanted — compare it with "
                   f"the seeded heights above) and for the two other promote-gate demotions, "
                   "'share-gate' and 'promote value-gate'."), e


def evaluate_proposed_in_landing_epoch(before, after, landing, interval, activation, victim):
    """…and the promotion was REAL: it led a view and proposed a block inside that epoch.

    Brief §3 calls this the deliverable, and it is the only assertion that observes the DEFECT
    directly rather than its mechanism — "verify-only, no proposals, no votes" is a statement
    about proposals, and `FluentApp::propose` is leader-gated, so a member in that state cannot
    emit this line at all. A member that promoted and then could not propose is a different (and
    still red) failure, which is why it is not folded into the promote gate."""
    n = int(after) - int(before)
    e = epoch_of(landing, interval, activation)
    if n > 0:
        return True, "", n
    room = epoch_room(landing, interval, activation)
    if room < MIN_EPOCH_ROOM:
        return False, (
            f"INCONCLUSIVE, not a verdict — RE-RUN. {victim} proposed no block in the landing "
            f"epoch {e}, but the jump landed at {landing} with only {room} block(s) of that epoch "
            f"left, which is fewer than one leader rotation is worth."), 0
    return False, (f"{victim} proposed NO block in the LANDING epoch {e} (propose lines in that "
                   f"epoch: {before} -> {after}; landing={landing}, room-left={room}). It "
                   "promoted but never led a view — verify-only in effect, which is the state "
                   "this case exists to rule out."), 0


def evaluate_spawn_defer_bounded(before, after, victim, epoch, budget=SPAWN_DEFER_BUDGET):
    """THE BELT (brief §4): the engine-spawn gate did not keep deferring after the seed landed.

    Deliberately a BUDGET and not zero — brief §9's last bullet. A defer during ordinary backfill
    is normal and self-heals on the next derived block; the defect is a defer that PERSISTS across
    the epoch, which the pre-fix behaviour re-fires on every executor re-poke for the rest of the
    landing epoch."""
    n = counter_delta(before, after)
    if n <= int(budget):
        return True, "", n
    return False, (f"{victim}'s {M_SPAWN_DEFERRED} rose by {n} (> budget {budget}) across landing "
                   f"epoch {epoch}: the engine-spawn gate kept deferring for a missing E-1 "
                   f"boundary block, i.e. seeding did not cover the height it needed. Grep "
                   f"{DEFER_LOG!r} and read its `boundary=` field."), n
