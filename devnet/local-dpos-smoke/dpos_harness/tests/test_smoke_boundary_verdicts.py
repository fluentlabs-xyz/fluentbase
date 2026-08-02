"""`verdicts_boundary` — the PURE decision layer of `smoke-rejump-signer`, both directions.

Same contract as the four sibling verdict suites: a live run only ever walks the PASS side
(`SmokeCtx.check` raises on the FAIL side and a `--dry-run` never evaluates a verdict at all), so
this file is the only place the failure branches can be driven. Every gate below is exercised
BOTH ways.

THE ONE TEST THAT MATTERS is `test_a_promote_line_naming_landing_epoch_PLUS_ONE_fails`. Without
the Rust fix the victim still promotes — one epoch later — so a case that asserted presence, or a
count, or a set difference would go green on the exact behaviour it exists to catch. That test is
the anti-vacuity proof for the whole case, and if it ever passes on `landing_epoch + 1` the case
has stopped testing anything.
"""

from __future__ import annotations

from dpos_harness.cases.smoke import verdicts_boundary as vb, verdicts_onchain as vo
from dpos_harness.core import rpc

INTERVAL = 64
ACTIVATION = 128
VICTIM = "validator-2"


# ══ the geometry ══════════════════════════════════════════════════════════════════════

def test_epoch_arithmetic_matches_OriginEpocher():
    """`epocher.rs`'s own `anchor_is_relative_epoch_zero` test, replayed: origin 64, length 32 =>
    the anchor at 64 is epoch 0, `last(0)` is 95 and `first(1)` is 96. The harness must agree with
    the product here or every epoch-scoped gate is off by one epoch."""
    assert vb.epoch_of(64, 32, 64) == 0
    assert vb.epoch_last(0, 32, 64) == 95
    assert vb.epoch_of(96, 32, 64) == 1
    assert vb.epoch_last(1, 32, 64) == 127
    # …and at the tuned geometry the case actually runs on.
    assert vb.epoch_of(ACTIVATION, INTERVAL, ACTIVATION) == 0
    assert vb.epoch_of(ACTIVATION + INTERVAL - 1, INTERVAL, ACTIVATION) == 0
    assert vb.epoch_of(ACTIVATION + INTERVAL, INTERVAL, ACTIVATION) == 1


def test_offset_and_room_are_complementary():
    """`offset + room == interval - 1` everywhere, which is what makes the restart steering and
    the INCONCLUSIVE branch two views of the same number."""
    for h in range(ACTIVATION, ACTIVATION + 3 * INTERVAL):
        off = vb.epoch_offset(h, INTERVAL, ACTIVATION)
        room = vb.epoch_room(h, INTERVAL, ACTIVATION)
        assert off + room == INTERVAL - 1, h
    assert vb.epoch_room(ACTIVATION, INTERVAL, ACTIVATION) == INTERVAL - 1
    assert vb.epoch_room(ACTIVATION + INTERVAL - 1, INTERVAL, ACTIVATION) == 0


def test_the_steering_window_leaves_more_room_than_the_inconclusive_floor():
    """The two constants are a pair: steering the restart into the first `RESTART_MAX_OFFSET`
    blocks is only worth doing if the resulting landing clears `MIN_EPOCH_ROOM` even after the
    ~7-10 blocks of victim boot drift. If this ever fails, the steering has stopped buying the
    thing it costs an epoch of waiting for."""
    worst_boot = 10
    landing = ACTIVATION + vb.RESTART_MAX_OFFSET + worst_boot
    assert vb.epoch_room(landing, INTERVAL, ACTIVATION) > vb.MIN_EPOCH_ROOM


def test_boundary_gap_sits_strictly_between_the_two_jump_gates():
    """THE PREMISE OF THE WHOLE CASE. Below `min(1024, interval)` neither path fires (that is what
    the catch-up case does on purpose); above 1024 the COLD-START path fires instead and the case
    would be measuring the other injection site. 160 at interval 64 is inside (64, 1024)."""
    gap = vb.boundary_gap(INTERVAL)
    assert vo.rejump_threshold(INTERVAL) < gap < vo.JUMP_THRESHOLD
    assert gap == 160
    # …and it is wider than two epochs, so the buried terminal necessarily falls inside the
    # victim's downtime — which is why a landing is always an epoch the node has to be told about.
    assert gap > 2 * INTERVAL


# ══ the parsers, against the VERIFIED renderings ══════════════════════════════════════

#: A real promote line, escapes and all, lifted verbatim from a captured devnet log
#: (`soak-out/bundle-20260629T100526Z/logs/validator-2.log`). `Epoch` is a `#[derive(Debug)]`
#: newtype logged with `?epoch`, so the field renders `epoch=Epoch(0)` and NOT `epoch=0` — and the
#: node writes SGR escapes INSIDE the `key=value` pair, which is why every reader in the package
#: strips first.
RAW_PROMOTE = ("validator-2-1  | \x1b[2m2026-06-29T10:01:54.604244Z\x1b[0m \x1b[32m INFO\x1b[0m "
               "\x1b[2mfluentbase_consensus::epoch_manager\x1b[0m\x1b[2m:\x1b[0m promoted to "
               "Signer in-process: per-epoch BFT engine started \x1b[3mepoch\x1b[0m\x1b[2m="
               "\x1b[0mEpoch(12)")


def test_the_promote_parser_reads_the_Epoch_DEBUG_rendering():
    """`epoch=Epoch(12)`, on an ANSI-STRIPPED line. Both halves are load-bearing: an unstripped
    line reads `epoch<ESC>=<ESC>Epoch(12)` and matches nothing, and
    `verdicts_onchain.epoch_field_lines`'s bare-`u64` `epoch=<N>` pattern — written for the beacon
    lines — does not match this rendering at all, which is why this case does not reuse it."""
    stripped = rpc.strip_ansi(RAW_PROMOTE)
    assert "epoch=Epoch(12)" in stripped
    assert vb.promoted_epochs(stripped) == [12]
    # The module path `fluentbase_consensus::epoch_manager` also contains "epoch" and sits to the
    # LEFT of the real field — the parser must not stop there.
    assert "epoch_manager" in stripped
    # And the raw line must NOT parse, so a reader that forgot to strip fails loudly rather than
    # reporting "never promoted".
    assert vb.promoted_epochs(RAW_PROMOTE) == []


def test_promote_counts_are_per_epoch_and_count_REPEATS():
    """A node that restarts inside an epoch it was already a signer for logs the line twice, and
    the gate is `after[E] > before[E]` — so the map has to count, not collect."""
    log = "\n".join([rpc.strip_ansi(RAW_PROMOTE)] * 3 +
                    [f"INFO {vb.PROMOTED_LINE}: per-epoch BFT engine started epoch=Epoch(13)"])
    assert vb.promoted_epoch_counts(log) == {12: 3, 13: 1}
    assert vb.promoted_epoch_counts("") == {}


def test_landing_heights_read_to_NOT_from():
    """`cold_start_jump.rs:874-879` logs `from=<anchor> to=<landing> floor=<landing-K>` on ONE
    line. Reading `from` would hand every gate the epoch the victim LEFT — an epoch it genuinely
    did promote for, so the case would pass on the pre-fix behaviour."""
    log = (f"INFO cold-start jump: {vo.REJUMP_LOG} from=1200 to=1360 floor=1357\n"
           f"INFO cold-start jump: {vo.REJUMP_LOG} from=1360 to=1500 floor=1497\n")
    assert vb.landing_heights(log) == [1360, 1500]
    # LAST wins at the call site: a case that ran two cycles must read the current one.
    assert vb.landing_heights(log)[-1] == 1500
    assert vb.landing_heights("INFO something else to=99") == []


def test_height_parsing_is_bounded_on_both_sides():
    """`height=12` must not answer for `height=1234`, and a `to=`/`floor=` on the same line must
    not bleed into the height."""
    log = (f"INFO {vb.PROPOSE_LINE} height=1234\n"
           f"INFO {vb.PROPOSE_LINE} height=12\n"
           f"INFO {vb.SEED_LOG_REJUMP} so this member can spawn height=1215 floor=1245\n"
           f"INFO {vb.SEED_LOG_COLD} so this member can spawn height=1216\n")
    assert vb.propose_heights(log) == [1234, 12]
    assert vb.seed_heights(log) == [1215, 1216]


def test_enter_boundaries_reads_boundary_and_not_landing_or_floor():
    """The entry line carries all three numbers, and which one the parser picks IS the gate: an
    entry keyed on the marshal floor rather than the landing names a boundary one whole epoch
    early, and reading `landing=` or `floor=` here would hide exactly that."""
    log = (f"INFO {vb.ENTER_LOG} landing=1360 boundary=1255 floor=1357\n"
           f"INFO {vb.ENTER_LOG} landing=1500 boundary=1319 floor=1497\n"
           "INFO something else boundary=99\n")
    assert vb.enter_boundaries(log) == [1255, 1319]
    assert vb.enter_boundaries("") == []


def test_proposals_in_epoch_scopes_by_height():
    heights = [ACTIVATION + 5, ACTIVATION + INTERVAL + 5, ACTIVATION + INTERVAL + 9]
    assert vb.proposals_in_epoch(heights, 0, INTERVAL, ACTIVATION) == 1
    assert vb.proposals_in_epoch(heights, 1, INTERVAL, ACTIVATION) == 2
    assert vb.proposals_in_epoch(heights, 2, INTERVAL, ACTIVATION) == 0


def test_count_lines_is_a_plain_substring_count():
    log = f"a {vb.SEED_PARTIAL_LOG} b\nunrelated\n{vb.SEED_PARTIAL_LOG}\n"
    assert vb.count_lines(log, vb.SEED_PARTIAL_LOG) == 2
    assert vb.count_lines("", vb.SEED_PARTIAL_LOG) == 0


# ══ the counter-name trap ═════════════════════════════════════════════════════════════

def test_counter_sample_appends_the_SECOND_total():
    """`prometheus-client` appends `_total` to a counter's SAMPLE name whatever the registered
    name already ends in — verified against a captured scrape, where
    `beacon_digest_fallback_total` (the registered literal) is exposed as
    `beacon_digest_fallback_total_total`. Reading the registered spelling through
    `nodes.gauge_val`'s ANCHORED matcher returns "", which is indistinguishable from a counter
    that never moved: a silent green on the metric half of the case."""
    assert vb.counter_sample(vb.M_REFETCHED) == "dpos_jump_boundary_refetched_total_total"
    assert vb.counter_sample(vb.M_SPAWN_DEFERRED) == "epoch_engine_spawn_deferred_total_total"

    from dpos_harness.core import nodes
    scrape = ("# HELP dpos_jump_boundary_refetched_total heights fetched.\n"
              "# TYPE dpos_jump_boundary_refetched_total counter\n"
              "dpos_jump_boundary_refetched_total_total 2\n")
    assert nodes.gauge_val(scrape, vb.counter_sample(vb.M_REFETCHED)) == "2"
    assert nodes.gauge_val(scrape, vb.M_REFETCHED) == "", (
        "the REGISTERED name must not match the sample line — if it ever does, "
        "counter_sample() has become a lie and the metric gates would double-count the suffix")


def test_an_ABSENT_metric_sample_SATISFIES_the_did_not_rise_gates():
    """"" is what `rpc.metrics_get_exec` returns for both an unregistered family (a binary without
    the metric) and an unreachable scrape. It maps to 0, which is the safe direction on the "did it
    move" diagnostics and the WRONG one on the two gates that remain: both ask that a counter did
    NOT rise, and an empty sample passes them without evidence. That is the hole
    `evaluate_metrics_plane_live` exists to close, and this test pins that it is a real hole rather
    than a hypothetical — if these two ever start failing on "", the plane gate has become
    redundant and should be re-argued, not deleted."""
    assert vb.counter_delta("", "") == 0
    assert vb.counter_delta("", "3") == 3
    assert vb.counter_delta("1", "") == -1
    assert vb.counter_delta(None, "2") == 2
    assert vb.counter_delta("0", "nonsense") == 0
    assert vb.evaluate_refetched("", "", VICTIM)[0] is False
    assert vb.evaluate_no_refetch_failure("", "", VICTIM)[0] is True
    assert vb.evaluate_spawn_defer_bounded("", "", VICTIM, 2)[0] is True


def test_the_metrics_plane_gate_separates_an_EMPTY_sample_from_a_ZERO_one():
    """THE ANTI-VACUITY TWIN of the plane gate, and the whole distinction it carries.

    `""` is "the registry said nothing" and must redden; `"0"` is "the registry said zero" and
    must not — a cycle whose boundary pair was already local legitimately leaves every one of
    these counters flat, and a gate that reddened on a live zero would be the demoted
    `evaluate_refetched` back under another name."""
    live = {vb.counter_sample(vb.M_REFETCHED): "0",
            vb.counter_sample(vb.M_REFETCH_FAILED): "0",
            vb.counter_sample(vb.M_SPAWN_DEFERRED): "3"}
    assert vb.evaluate_metrics_plane_live(live, VICTIM) == (True, "")

    dead = dict(live, **{vb.counter_sample(vb.M_REFETCHED): ""})
    ok, msg = vb.evaluate_metrics_plane_live(dead, VICTIM)
    assert not ok
    assert vb.counter_sample(vb.M_REFETCHED) in msg, (
        "the diagnostic must name the SAMPLE spelling an operator would grep for")
    assert "an EMPTY sample, not a zero" in msg

    # A whitespace-only sample and a `None` are the same nothing.
    assert vb.evaluate_metrics_plane_live({"x": "  "}, VICTIM)[0] is False
    assert vb.evaluate_metrics_plane_live({"x": None}, VICTIM)[0] is False
    # …and so is an empty map: a gate that passes when it was handed nothing to check is the
    # failure mode it was written for.
    assert vb.evaluate_metrics_plane_live({}, VICTIM)[0] is False


# ══ the verdicts ══════════════════════════════════════════════════════════════════════

def test_evaluate_landed_both_ways():
    ok, msg = vb.evaluate_landed([1360], VICTIM, 64)
    assert ok and msg == ""
    ok, msg = vb.evaluate_landed([], VICTIM, 64)
    assert not ok
    assert "never logged a jump landing" in msg and "RAISE BOUNDARY_GAP" in msg


def test_evaluate_seeded_both_ways():
    """A DIAGNOSTIC now, so its zero side must not read as a verdict.

    Seeding is condition-keyed and a member that already holds the boundary pair logs nothing, so
    `asserts_boundary` prints this instead of asserting it. The message must say so — a zero
    reported as "the pre-fix behaviour" is what made the case red runs in which the victim entered
    the landing epoch and signed."""
    ok, msg, n = vb.evaluate_seeded(0, 2, VICTIM, "re-jump", [1215, 1216])
    assert ok and n == 2
    ok, msg, n = vb.evaluate_seeded(2, 2, VICTIM, "re-jump", [])
    assert not ok and n == 0
    assert "Expected when the boundary pair was already local" in msg
    assert "pre-fix" not in msg


def test_evaluate_refetched_both_ways():
    """Same demotion, same reason: the counter is bumped once per height actually injected, so a
    cycle whose pair was already local leaves it flat."""
    assert vb.evaluate_refetched("0", "2", VICTIM)[:2][0] is True
    ok, msg, _ = vb.evaluate_refetched("2", "2", VICTIM)
    assert not ok
    assert "Expected when the pair was already local" in msg
    assert vb.counter_sample(vb.M_REFETCHED) in msg, (
        "the diagnostic must name the SAMPLE spelling — an operator who greps the registered "
        "name on a live registry finds nothing and concludes the metric is broken")


def test_evaluate_no_refetch_failure_both_ways():
    assert vb.evaluate_no_refetch_failure("0", "0", VICTIM) == (True, "")
    ok, msg = vb.evaluate_no_refetch_failure("0", "1", VICTIM)
    assert not ok
    assert "stays verify-only" in msg and vb.SEED_FAIL_LOG in msg


# --- THE ANTI-VACUITY GROUP -----------------------------------------------------------

LANDING = ACTIVATION + 2 * INTERVAL + 10          # epoch 2, 53 blocks of room
LANDING_EPOCH = 2


#: The boundary the entry MUST name for `LANDING`: the terminal of the epoch before the landing's,
#: which is the height the engine-spawn gate reads.
LANDING_BOUNDARY = vb.epoch_last(LANDING_EPOCH - 1, INTERVAL, ACTIVATION)


def test_evaluate_entered_landing_epoch_both_ways():
    """The call-site witness, and the reason it replaces what the demoted seed/refetch gates
    reached for: the entry CALL is condition-keyed on the LANDING, so unlike them it has no cycle
    in which a healthy node legitimately logs nothing. Its absence is conclusive; its presence is
    a necessary condition only, because the line is written before the call (see `vb.ENTER_LOG`)."""
    ok, msg, got = vb.evaluate_entered_landing_epoch([], [LANDING_BOUNDARY], LANDING, INTERVAL,
                                                     ACTIVATION, VICTIM)
    assert ok and msg == "" and got == LANDING_BOUNDARY

    ok, msg, got = vb.evaluate_entered_landing_epoch([], [], LANDING, INTERVAL, ACTIVATION, VICTIM)
    assert not ok and got is None
    assert vb.ENTER_LOG in msg and "THE PRE-FIX BEHAVIOUR" in msg

    # A pre-existing entry from an earlier cycle is not this cycle's evidence — the container log
    # survives `compose_stop`/`start`, so the gate is a delta like every other one here.
    ok, _, _ = vb.evaluate_entered_landing_epoch([LANDING_BOUNDARY], [LANDING_BOUNDARY], LANDING,
                                                 INTERVAL, ACTIVATION, VICTIM)
    assert not ok
    assert vb.evaluate_entered_landing_epoch([LANDING_BOUNDARY],
                                             [LANDING_BOUNDARY, LANDING_BOUNDARY], LANDING,
                                             INTERVAL, ACTIVATION, VICTIM)[0] is True


def test_an_entry_naming_the_WRONG_epoch_fails():
    """THE OFF-BY-ONE GUARD, and the second reason this gate carries a value and not a count.

    `terminal_at_or_below(floor)` and `terminal_at_or_below(landing)` coincide for most landings
    and are a whole epoch apart at two geometries: a landing within K of an epoch start, and a
    landing that is itself an epoch terminal (the case below this one). An entry keyed on the
    floor therefore enters the epoch BEFORE the one the node is in — it fires, it logs, and every
    count-based reading of it is green."""
    one_epoch_early = vb.epoch_last(LANDING_EPOCH - 2, INTERVAL, ACTIVATION)
    assert one_epoch_early != LANDING_BOUNDARY
    ok, msg, got = vb.evaluate_entered_landing_epoch([], [one_epoch_early], LANDING, INTERVAL,
                                                     ACTIVATION, VICTIM)
    assert not ok, (
        "an entry on the epoch below the landing's MUST fail — that is the floor-keyed "
        "implementation this gate exists to catch")
    assert got == one_epoch_early
    assert "WRONG epoch" in msg
    assert f"boundary={LANDING_BOUNDARY}" in msg and f"boundary={one_epoch_early}" in msg


def test_a_landing_ON_a_terminal_expects_the_LANDING_itself_as_the_boundary():
    """The one geometry where `epoch_last(E - 1)` is the wrong expectation.

    When the jump lands exactly on an epoch terminal, `terminal_at_or_below(landing)` IS the
    landing, and the product enters `E + 1` off it — no block of `E` is left to sign, so nothing
    is skipped. Demanding `epoch_last(E - 1)` there reds a correct run, and it reds it at
    `epoch_room == 0`, i.e. precisely where the promote gate's INCONCLUSIVE hedge is the honest
    answer — so the wrong expectation does not merely mislabel the run, it pre-empts the label
    that was designed for it."""
    terminal = vb.epoch_last(LANDING_EPOCH, INTERVAL, ACTIVATION)
    assert vb.epoch_room(terminal, INTERVAL, ACTIVATION) == 0

    ok, msg, got = vb.evaluate_entered_landing_epoch([], [terminal], terminal, INTERVAL,
                                                     ACTIVATION, VICTIM)
    assert ok and msg == "" and got == terminal
    # …and the ordinary expectation is still refused there — that is the floor-keyed entry.
    assert vb.evaluate_entered_landing_epoch([], [LANDING_BOUNDARY], terminal, INTERVAL,
                                             ACTIVATION, VICTIM)[0] is False

    # The hedge this unblocks: with the entry gate satisfied, a terminal landing reaches the
    # promote gate and is reported INCONCLUSIVE rather than as the defect.
    ok, msg, _ = vb.evaluate_promoted_in_landing_epoch({}, {}, terminal, INTERVAL, ACTIVATION,
                                                       VICTIM)
    assert not ok and "INCONCLUSIVE" in msg


def test_a_promote_in_the_landing_epoch_passes():
    ok, msg, e = vb.evaluate_promoted_in_landing_epoch({0: 1, 1: 1}, {0: 1, 1: 1, 2: 1},
                                                       LANDING, INTERVAL, ACTIVATION, VICTIM)
    assert ok and msg == "" and e == LANDING_EPOCH


def test_a_promote_line_naming_landing_epoch_PLUS_ONE_fails():
    """THE TEST THIS WHOLE CASE EXISTS FOR.

    The pre-fix behaviour is not "no promote" — it is a promote one epoch late. Every weaker
    formulation of the gate (presence, `promote_count > 0`, "some new epoch appeared") is
    SATISFIED by this world, and a case built on one of them would go green against the bug it
    was written to catch. This drives exactly that world and requires a red, with the pre-fix
    signature named in the message so an operator is sent to the floor-raise sites and not to
    the chain."""
    before = {0: 1, 1: 1}
    after = {0: 1, 1: 1, 3: 1}                     # promoted at LANDING_EPOCH + 1, not at 2
    ok, msg, e = vb.evaluate_promoted_in_landing_epoch(before, after, LANDING, INTERVAL,
                                                       ACTIVATION, VICTIM)
    assert not ok, "a promote at landing_epoch+1 MUST fail — that is the pre-fix behaviour"
    assert e == LANDING_EPOCH
    assert "THE PRE-FIX SIGNATURE" in msg
    assert "promoted in epoch 3 instead" in msg
    assert "below the marshal floor" in msg
    # …and it is NOT reported as an unlucky landing: there were 53 blocks left in epoch 2.
    assert "INCONCLUSIVE" not in msg


def test_a_PRE_EXISTING_promote_for_the_landing_epoch_does_not_satisfy_the_gate():
    """The delta property, and the reason the gate is a count map rather than a set membership.

    A victim that already promoted for this epoch before the disruption leaves that line in the
    container's log (`compose_stop`/`start` keeps the container). Presence-over-the-whole-log
    would read it as this cycle's evidence; `after[E] > before[E]` does not."""
    before = {2: 1}
    after = {2: 1}
    ok, msg, _ = vb.evaluate_promoted_in_landing_epoch(before, after, LANDING, INTERVAL,
                                                       ACTIVATION, VICTIM)
    assert not ok
    assert "count 1 -> 1" in msg
    # …and the same world with a SECOND line for that epoch is the pass.
    assert vb.evaluate_promoted_in_landing_epoch({2: 1}, {2: 2}, LANDING, INTERVAL,
                                                 ACTIVATION, VICTIM)[0] is True


def test_a_landing_with_no_room_left_is_INCONCLUSIVE_not_a_verdict():
    """A jump that lands three blocks before the terminal gives a correct fix no chance to promote
    inside the landing epoch. That is not the defect and must not be reported as one — but it is
    still a RED, because a silent pass there is exactly the vacuity this case exists to avoid."""
    tight = ACTIVATION + 3 * INTERVAL - 2          # 1 block of room
    ok, msg, _ = vb.evaluate_promoted_in_landing_epoch({}, {}, tight, INTERVAL, ACTIVATION, VICTIM)
    assert not ok
    assert "INCONCLUSIVE" in msg and "RE-RUN" in msg
    assert f"MIN_EPOCH_ROOM={vb.MIN_EPOCH_ROOM}" in msg


def test_evaluate_proposed_in_landing_epoch_both_ways():
    ok, msg, n = vb.evaluate_proposed_in_landing_epoch(0, 3, LANDING, INTERVAL, ACTIVATION, VICTIM)
    assert ok and n == 3
    ok, msg, n = vb.evaluate_proposed_in_landing_epoch(0, 0, LANDING, INTERVAL, ACTIVATION, VICTIM)
    assert not ok and n == 0
    assert "never led a view" in msg and "verify-only" in msg
    # A pre-existing proposal in that epoch is not this cycle's evidence either.
    assert vb.evaluate_proposed_in_landing_epoch(4, 4, LANDING, INTERVAL, ACTIVATION,
                                                 VICTIM)[0] is False
    # …and a landing with no room is INCONCLUSIVE here too (one leader rotation needs blocks).
    tight = ACTIVATION + 3 * INTERVAL - 2
    assert "INCONCLUSIVE" in vb.evaluate_proposed_in_landing_epoch(0, 0, tight, INTERVAL,
                                                                   ACTIVATION, VICTIM)[1]


def test_evaluate_spawn_defer_bounded_is_a_budget_not_a_zero():
    """Brief §9's last bullet: one defer during ordinary backfill is normal and self-heals. The
    defect is a defer that persists across the epoch, which the pre-fix code re-fires on every
    executor re-poke — tens, not a handful."""
    assert vb.evaluate_spawn_defer_bounded("0", "1", VICTIM, 2)[0] is True
    assert vb.evaluate_spawn_defer_bounded("0", str(vb.SPAWN_DEFER_BUDGET), VICTIM, 2)[0] is True
    ok, msg, n = vb.evaluate_spawn_defer_bounded("0", str(vb.SPAWN_DEFER_BUDGET + 1), VICTIM, 2)
    assert not ok and n == vb.SPAWN_DEFER_BUDGET + 1
    assert "kept deferring" in msg and vb.DEFER_LOG in msg
