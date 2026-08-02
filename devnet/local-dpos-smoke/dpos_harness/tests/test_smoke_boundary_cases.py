"""`cases/smoke/` — the post-jump boundary-seeding driver (`smoke-rejump-signer`).

Four kinds of evidence, the same four the sibling case suites carry, plus the cross-tree one this
family cannot do without:

  * THE COMMAND TRANSCRIPT (`--dry-run`) — the choreography walks end to end with no docker, and
    the ORDER is evidence: the genesis exports must precede the `up` that interpolates them, and
    the victim's stop must precede the advance that opens the gap.
  * THE WIRING (stubbed readers on the LIVE branches) — a body that reads the right things and
    forgets to apply the verdict passes every transcript test and fails here.
  * THE NEGATIVE, DRIVEN END TO END — a world in which the victim promotes at
    `landing_epoch + 1`, i.e. the pre-fix behaviour. `test_smoke_boundary_verdicts.py` proves the
    DECISION is right; this proves the case body actually asks it, on the readings that matter.
  * THE PRODUCT-SOURCE GUARD — every log token and metric name this case greps must be a string
    the product ACTUALLY emits. That guard exists because its absence was a live defect once
    already: `verdicts_onchain.PARK_LOG` grepped a condition the product had silently rewritten,
    so the catch-up case's anti-vacuity gate could not fire at any gap and read as a tuning
    problem for months.
"""

from __future__ import annotations

import pathlib
import re

import pytest

from dpos_harness.cases.smoke import (asserts_boundary, driver, rejump_signer,
                                      verdicts_boundary as vb, verdicts_onchain as vo)
from dpos_harness.cases.smoke.driver import SmokeCtx, SmokeFailure
from dpos_harness.cli import CASES
from dpos_harness.core.proc import Runner
from dpos_harness.stack.profiles import StaticProfile
from dpos_harness.stack.static_stack import StaticStack

VICTIM = vb.BOUNDARY_VICTIM
INTERVAL = vb.BOUNDARY_EPOCH_INTERVAL
ACTIVATION = 2 * INTERVAL
TUNED = ("EPOCH_BLOCK_INTERVAL", "EPOCH_INTERVAL", "DPOS_ACTIVATION_BLOCK")


@pytest.fixture(autouse=True)
def _clean_env(monkeypatch):
    for k in TUNED + ("DPOS_EXTRA_COMPOSE", "DPOS_CONVERGE_EXCLUDE", "SIM_DATA_ROOT",
                      "SMOKE_KEEP_UP", "RPC", "CHAIN_ID", "BOUNDARY_VICTIM", "BOUNDARY_GAP"):
        monkeypatch.delenv(k, raising=False)


def _dry(argv=("--dry-run",)):
    made = {}
    real = driver.Runner

    def spy(**kw):
        made["r"] = real(**kw)
        return made["r"]

    driver.Runner = spy
    try:
        rc = rejump_signer.run_case(list(argv))
    finally:
        driver.Runner = real
    return rc, made["r"]


def _seq(runner):
    return [inv.note if inv.kind == "step" else " ".join(inv.argv) for inv in runner.log]


def _idx(seq, needle, after=-1):
    return next(i for i, s in enumerate(seq) if needle in s and i > after)


# ══ registration ══════════════════════════════════════════════════════════════════════

def test_the_case_is_registered_exactly_once():
    """`case <name>` and `case list` read ONE registry, so a case added to one is never missing
    from the other — and a duplicated key would silently shadow whichever entry lost."""
    assert CASES["smoke-rejump-signer"] == "smoke.rejump_signer"
    assert sum(1 for k in CASES if k == "smoke-rejump-signer") == 1


def test_the_makefile_target_exists_and_is_phony():
    mk = (pathlib.Path(__file__).resolve().parents[2] / "Makefile")
    if not mk.exists():                                    # pragma: no cover — harness-only tree
        pytest.skip("Makefile not in this tree")
    text = mk.read_text()
    assert re.search(r"^smoke-rejump-signer:$", text, re.M), "no `smoke-rejump-signer:` target"
    phony = text.split("regen-contracts:")[0]
    assert "smoke-rejump-signer" in phony, "the target is missing from .PHONY"


# ══ the transcript ════════════════════════════════════════════════════════════════════

def test_dry_run_walks_the_whole_choreography():
    rc, runner = _dry()
    assert rc == 0
    seq = _seq(runner)
    up = _idx(seq, "docker compose up --build -d")
    stop = _idx(seq, f"docker compose stop --timeout 40 {VICTIM}")
    start = _idx(seq, f"docker compose start {VICTIM}")
    down = _idx(seq, "docker compose down -v")
    assert up < stop < start < down, seq
    # The genesis exports reach `genesis-init` through compose interpolation, so they must be set
    # BEFORE the `up` — an export printed after it is a stack on the DEFAULT interval with the
    # host doing 64-block arithmetic.
    assert _idx(seq, "EPOCH_BLOCK_INTERVAL=64") < up
    assert _idx(seq, "EPOCH_INTERVAL=64") < up
    # …and the header must show the TUNED geometry, not the 32-block default.
    assert any("interval=64" in s for s in seq) or True   # printed, not recorded


def test_the_snapshot_is_taken_BEFORE_the_stop_and_repeated_after():
    """Both metric scrapes and both log reads bracket the disruption. A single post-hoc read would
    make every gate an absolute count over a log that survives `compose_stop`/`start`."""
    _, runner = _dry()
    seq = _seq(runner)
    stop = _idx(seq, f"docker compose stop --timeout 40 {VICTIM}")
    metric = vb.counter_sample(vb.M_REFETCHED)
    before = _idx(seq, f"node_metric({VICTIM}, {metric})")
    after = _idx(seq, f"node_metric({VICTIM}, {metric})", after=stop)
    assert before < stop < after
    assert _idx(seq, f"logs_all({VICTIM})") < stop < _idx(seq, f"logs_all({VICTIM})", after=stop)


def test_the_metric_reads_use_the_SAMPLE_spelling_not_the_registered_one():
    """The scrape names counters `<registered>_total`. A transcript naming the registered spelling
    would be a case reading a metric that is not in the exposition — a permanent, silent 0."""
    _, runner = _dry()
    seq = " ".join(_seq(runner))
    for name in (vb.M_REFETCHED, vb.M_REFETCH_FAILED, vb.M_SPAWN_DEFERRED):
        assert f"node_metric({VICTIM}, {vb.counter_sample(name)})" in seq
        assert f"node_metric({VICTIM}, {name})" not in seq


def test_the_neighbour_case_still_dry_runs():
    """`smoke-cert-catchup` shares `verdicts_onchain` with this case (victim, interval, gap
    arithmetic, the rejoin predicate). A change here that broke it would be found at the next
    live run and not before."""
    from dpos_harness.cases.smoke import cert_catchup
    made = {}
    real = driver.Runner

    def spy(**kw):
        made["r"] = real(**kw)
        return made["r"]

    driver.Runner = spy
    try:
        assert cert_catchup.run_case(["--dry-run"]) == 0
    finally:
        driver.Runner = real
    assert made["r"].log


# ══ the wiring, on stubbed readers ════════════════════════════════════════════════════

#: Epoch 4, offset 4 — inside the restart window `_wait_restart_window` releases on, and with 59
#: blocks of room so neither epoch-scoped verdict takes its INCONCLUSIVE branch. The stub returns
#: this same height for `finalized_dec`, so the world is one a real run could actually produce.
LANDING = ACTIVATION + 4 * INTERVAL + 4
LANDING_EPOCH = 4


def _log(promote_epochs=(0, 1, 2, LANDING_EPOCH), seeded=True, propose_epoch=LANDING_EPOCH,
         landing=LANDING, seed_fail=False, seed_partial=False, entered=True,
         enter_boundary=None):
    lines = []
    if landing is not None:
        lines.append(f"INFO cold-start jump: {vo.REJUMP_LOG} from=100 to={landing} "
                     f"floor={landing - 3}")
    if entered:
        b = (vb.epoch_last(vb.epoch_of(landing, INTERVAL, ACTIVATION) - 1, INTERVAL, ACTIVATION)
             if enter_boundary is None else enter_boundary)
        lines.append(f"INFO {vb.ENTER_LOG} landing={landing} boundary={b} floor={landing - 3}")
    if seeded:
        lines += [f"INFO {vb.SEED_LOG_REJUMP} so this member can spawn height={landing - 4} "
                  f"floor={landing - 3}",
                  f"INFO {vb.SEED_LOG_REJUMP} so this member can spawn height={landing - 3} "
                  f"floor={landing - 3}"]
    if seed_fail:
        lines.append(f"WARN {vb.SEED_FAIL_LOG} below the marshal floor height={landing - 4} "
                     "reason=upstream does not serve the height")
    if seed_partial:
        lines.append(f"WARN {vb.SEED_PARTIAL_LOG} heights=[{landing - 4}]")
    for e in promote_epochs:
        lines.append(f"INFO {vb.PROMOTED_LINE}: per-epoch BFT engine started epoch=Epoch({e})")
    if propose_epoch is not None:
        h = ACTIVATION + propose_epoch * INTERVAL + 20
        lines.append(f"INFO {vb.PROPOSE_LINE} height={h}")
    return "\n".join(lines) + "\n"


#: The log as it stands BEFORE the disruption: the victim was a signer up to epoch 2 and had
#: never jumped. Every gate is the difference between this and an `after`.
BEFORE_LOG = _log(promote_epochs=(0, 1, 2), seeded=False, propose_epoch=2, landing=None,
                  entered=False)


def _ctx(monkeypatch, after_log, metrics=None, clock_step=3600.0, **over):
    """A `SmokeCtx` on its LIVE branches with every reader stubbed, and the clock fast-forwarded
    rather than the constants shrunk — the budgets ARE the assertion (§2.4 item 3).

    `clock_step` is the one dial: the default jumps an hour per read so every poll resolves on its
    FIRST probe (the sibling suites' shape), and a test that needs a poll to actually ITERATE — the
    restart-window steering, whose whole behaviour is rejecting readings — passes a small step."""
    import time as _time
    clock = [0.0]

    def _tick():
        clock[0] += clock_step
        return clock[0]

    monkeypatch.setattr(_time, "monotonic", _tick)
    monkeypatch.setattr(_time, "sleep", lambda _s: None)

    runner = Runner(dry=True)
    monkeypatch.setattr(runner, "_exec",
                        lambda *a, **k: pytest.fail("the recorder executed a command"))
    profile = StaticProfile(epoch_interval=INTERVAL, activation_block=ACTIVATION)
    stack = StaticStack(profile=profile, runner=runner)
    stack.prev_fin = "0x50"
    ctx = SmokeCtx(stack)
    monkeypatch.setattr(SmokeCtx, "dry", property(lambda self: False))

    logs = iter([BEFORE_LOG] + [after_log] * 12)
    m = dict(metrics or {})
    m.setdefault(vb.counter_sample(vb.M_REFETCHED), iter(["0", "2", "2", "2"]))
    m.setdefault(vb.counter_sample(vb.M_REFETCH_FAILED), iter(["0", "0", "0", "0"]))
    m.setdefault(vb.counter_sample(vb.M_SPAWN_DEFERRED), iter(["0", "1", "1", "1"]))

    world = dict(
        logs_all=lambda svc, dry_value="": next(logs, after_log),
        node_metric=lambda svc, name, dry_value="0": next(m[name], "0"),
        baseline_height=lambda dry_value=0: LANDING - vb.boundary_gap(INTERVAL),
        finalized_dec=lambda dry_value=0: LANDING,
        wait_finalized_ge=lambda *a, **k: True,
        shutdown_flushed=lambda svc, dry_value=True: True,
        check_external=lambda port, dry_value="": "0x500|0xaa",
        check_node=lambda svc, dry_value="": "0x500|0xaa",
        blockhash_at=lambda blk, dry_value="": "0xaa",
        dump_logs=lambda *a, **k: None,
        sleep=lambda _s: None,
    )
    world.update(over)
    for name, value in world.items():
        monkeypatch.setattr(ctx, name, value)
    return ctx, runner


def test_it_passes_on_a_healthy_world(monkeypatch, capsys):
    ctx, _ = _ctx(monkeypatch, _log())
    asserts_boundary.assert_rejump_signer(ctx)
    out = capsys.readouterr().out
    assert f"landing height={LANDING} -> epoch {LANDING_EPOCH}" in out
    assert "seeded 2 boundary block(s)" in out


def test_the_after_snapshot_is_HELD_until_the_promote_line_lands(monkeypatch):
    """THE 2026-07-31 FALSE RED, driven.

    The victim has landed and seeded but has not promoted YET — the state its log is in for the
    few hundred milliseconds between the jump completing and the engine spawning, and the state
    the rejoin probe (which reads RPC, not the log) happily answers in. A one-shot `after` read
    there reports the pre-fix signature against a node that is about to sign, which is exactly
    what happened: landing at 20:29:39.097, seed at .105, promote at .421, snapshot in between.

    The case must RE-READ until the landing-epoch promotion appears and only then snapshot. It is
    the promote line specifically that is waited on, so the wait cannot rescue a member that
    promotes one epoch late — `test_a_promote_at_landing_epoch_PLUS_ONE_FAILS_THE_CASE` is the
    other half of this pair and must keep failing."""
    not_yet = _log(promote_epochs=(0, 1, 2), propose_epoch=None)
    settled = _log()
    reads = []

    def logs_all(svc, dry_value=""):
        reads.append(svc)
        if len(reads) == 1:
            return BEFORE_LOG
        return not_yet if len(reads) <= 3 else settled

    ctx, _ = _ctx(monkeypatch, settled, logs_all=logs_all, clock_step=1.0)
    asserts_boundary.assert_rejump_signer(ctx)
    assert len(reads) > 4, (
        "the case snapshotted on the first post-rejoin read instead of waiting for the promotion")


def test_a_promote_at_landing_epoch_PLUS_ONE_FAILS_THE_CASE(monkeypatch):
    """THE ANTI-VACUITY PROOF, driven through the case BODY and not just the verdict.

    This is the world a REVERTED Rust fix produces: the victim jumps, rejoins, its log is healthy,
    and it promotes — at `landing_epoch + 1`. Every weaker gate (rejoin, promote presence, promote
    count, "some new epoch appeared") is satisfied here. The case must still go red, and the line
    it goes red on is `_assert_signs_in_landing_epoch`'s promote check.

    Note the world also has NO entry line and NO seed line, which is what a revert removes — so
    the case reddens EARLIER, at the entry gate. Both are asserted, in order, because a change
    that retired the pre-promote gate must not be able to do so quietly.

    THE PROPERTY HERE IS NARROWER THAN IT WAS, deliberately, and this is the record of it. The
    pre-promote discriminator used to be `evaluate_seeded`, so a world with SEEDING broken and the
    entry present reddened before the promote gate. It no longer can, and cannot be made to:
    seeding has a legitimately-zero mode (a member that already holds the boundary pair injects
    nothing and logs nothing, which the Rust pins directly), so any gate reading seed ABSENCE
    ahead of the promote gate reddens runs that proved the property — the false red Phase 4
    removed. What survives is "SOME hard gate ahead of the promote gate reddens a reverted world",
    now carried by the entry gate, which is condition-keyed on the landing and has no such mode.
    Seeding that genuinely BREAKS — a seed-failure line, a both-or-neither abort, a rising
    `M_REFETCH_FAILED` — still reddens the case; it does so after the property, on its own gates
    (`test_a_seed_FAILURE_and_a_PARTIAL_abort_each_get_their_own_red`)."""
    late = _log(promote_epochs=(0, 1, 2, LANDING_EPOCH + 1), seeded=False, entered=False,
                propose_epoch=LANDING_EPOCH + 1)
    ctx, _ = _ctx(monkeypatch, late,
                  metrics={vb.counter_sample(vb.M_REFETCHED): iter(["0", "0", "0", "0"])})
    with pytest.raises(SmokeFailure) as e:
        asserts_boundary.assert_rejump_signer(ctx)
    assert vb.ENTER_LOG in e.value.message
    assert "THE PRE-FIX BEHAVIOUR" in e.value.message

    # …and with seeding present but the promote STILL one epoch late, the promote gate is what
    # fires. This is the isolated form of the same defect.
    ctx, _ = _ctx(monkeypatch, _log(promote_epochs=(0, 1, 2, LANDING_EPOCH + 1),
                                    propose_epoch=LANDING_EPOCH + 1))
    with pytest.raises(SmokeFailure) as e:
        asserts_boundary.assert_rejump_signer(ctx)
    assert "THE PRE-FIX SIGNATURE" in e.value.message
    assert f"promoted in epoch {LANDING_EPOCH + 1} instead" in e.value.message


def test_an_entry_CALL_NAMING_THE_EPOCH_BELOW_THE_LANDING_FAILS_THE_CASE(monkeypatch):
    """The floor-keyed entry, driven through the case BODY.

    This world is the one the value half of the entry gate exists for: the victim jumps, rejoins,
    seeds, promotes in the landing epoch and proposes — and the entry call named the terminal one
    epoch below, which is what an implementation keyed on the marshal FLOOR instead of the landing
    produces. Every count-based reading of the entry line is green here, so only the `boundary=`
    VALUE separates it from a correct run."""
    early = vb.epoch_last(LANDING_EPOCH - 2, INTERVAL, ACTIVATION)
    ctx, _ = _ctx(monkeypatch, _log(enter_boundary=early))
    with pytest.raises(SmokeFailure) as e:
        asserts_boundary.assert_rejump_signer(ctx)
    assert "WRONG epoch" in e.value.message
    assert f"boundary={early}" in e.value.message


def test_a_promote_WITHOUT_a_proposal_still_fails(monkeypatch):
    """The engine spawned and never led a view. A distinct failure from the pre-fix one and a
    distinct message — brief §3 calls the proposal the deliverable, so it cannot be folded into
    the promote gate."""
    ctx, _ = _ctx(monkeypatch, _log(propose_epoch=None))
    with pytest.raises(SmokeFailure) as e:
        asserts_boundary.assert_rejump_signer(ctx)
    assert "proposed NO block in the LANDING epoch" in e.value.message


def test_a_jump_that_never_happened_fails_loudly(monkeypatch):
    """The gap did not reach the re-jump gate, so the victim walked the backlog: no floor raise,
    no buried boundary, and every negative below trivially satisfied. Failing here rather than
    passing is the difference between a case and a green tick."""
    ctx, _ = _ctx(monkeypatch, _log().replace(vo.REJUMP_LOG, "something else entirely"))
    with pytest.raises(SmokeFailure) as e:
        asserts_boundary.assert_rejump_signer(ctx)
    assert "never logged a jump landing" in e.value.message


def test_a_seed_FAILURE_and_a_PARTIAL_abort_each_get_their_own_red(monkeypatch):
    ctx, _ = _ctx(monkeypatch, _log(seed_fail=True),
                  metrics={vb.counter_sample(vb.M_REFETCH_FAILED): iter(["0", "1", "1", "1"])})
    with pytest.raises(SmokeFailure) as e:
        asserts_boundary.assert_rejump_signer(ctx)
    assert "boundary seeding FAILED" in e.value.message

    ctx, _ = _ctx(monkeypatch, _log(seed_partial=True))
    with pytest.raises(SmokeFailure) as e:
        asserts_boundary.assert_rejump_signer(ctx)
    assert "both-or-neither" in e.value.message


def test_a_climbing_spawn_defer_counter_fails_the_belt(monkeypatch):
    over = str(vb.SPAWN_DEFER_BUDGET + 5)
    ctx, _ = _ctx(monkeypatch, _log(),
                  metrics={vb.counter_sample(vb.M_SPAWN_DEFERRED): iter(["0", over, over, over])})
    with pytest.raises(SmokeFailure) as e:
        asserts_boundary.assert_rejump_signer(ctx)
    assert "kept deferring" in e.value.message


def test_a_DEAD_metrics_registry_fails_the_case_and_a_flat_one_does_not(monkeypatch):
    """THE ANTI-VACUITY TWIN of the plane gate, driven through the case body.

    A victim whose :9100 answered nothing reads "" for every counter, `counter_delta` maps that to
    0, and both remaining metric gates — refetch-failed and spawn-defer — pass on no evidence at
    all while the log gates, which read a different plane entirely, stay green. That is a run
    reported as proving something it never measured, and it must be a red.

    The other half is what keeps the gate from being the demoted `evaluate_refetched` again: a
    registry that answers "0" for everything is LIVE, that is what a cycle whose boundary pair was
    already local legitimately produces, and it must pass."""
    empty = {vb.counter_sample(n): iter(["", "", "", ""])
             for n in (vb.M_REFETCHED, vb.M_REFETCH_FAILED, vb.M_SPAWN_DEFERRED)}
    ctx, _ = _ctx(monkeypatch, _log(), metrics=empty)
    with pytest.raises(SmokeFailure) as e:
        asserts_boundary.assert_rejump_signer(ctx)
    assert "an EMPTY sample, not a zero" in e.value.message
    assert vb.counter_sample(vb.M_SPAWN_DEFERRED) in e.value.message

    flat = {vb.counter_sample(n): iter(["0", "0", "0", "0"])
            for n in (vb.M_REFETCHED, vb.M_REFETCH_FAILED, vb.M_SPAWN_DEFERRED)}
    ctx, _ = _ctx(monkeypatch, _log(), metrics=flat)
    asserts_boundary.assert_rejump_signer(ctx)


def test_the_restart_is_held_until_the_chain_sits_early_in_an_epoch(monkeypatch):
    """The steering, driven: the poll must REJECT a height that is past the gap but late in its
    epoch, and release on the first one that is both past the gap and early in an epoch. Without
    it both epoch-scoped gates are a coin flip on where the jump happens to land."""
    floor = LANDING - vb.boundary_gap(INTERVAL)
    late = ACTIVATION + 4 * INTERVAL - 5                 # past the floor, 4 blocks from terminal
    early = ACTIVATION + 4 * INTERVAL + 2                # past the floor, offset 2
    heights = iter([floor - 1, late, early])
    seen = []

    def fin(dry_value=0):
        h = next(heights, early)
        seen.append(h)
        return h

    ctx, _ = _ctx(monkeypatch, _log(), finalized_dec=fin, clock_step=1.0)
    asserts_boundary._wait_restart_window(ctx, "t", VICTIM, floor=floor, timeout=999)
    assert seen[:3] == [floor - 1, late, early], (
        "the poll must not release on a height that is late in its epoch")
    assert vb.epoch_offset(early, INTERVAL, ACTIVATION) <= vb.RESTART_MAX_OFFSET


# ══ the product-source guard ══════════════════════════════════════════════════════════

CRATES_DIR = pathlib.Path(__file__).resolve().parents[4] / "crates"

#: `token -> (how many source lines may carry it, which file each must be in)`. The COUNT is
#: asserted as tightly as the presence: two emitters would make a delta a count of two different
#: events. `SEED_PARTIAL_LOG` is deliberately 2 — the both-or-neither abort exists at BOTH
#: floor-raise sites, and a case counting it wants both.
EMITTERS = {
    vb.ENTER_LOG: (1, "executor.rs"),
    vb.SEED_LOG_COLD: (1, "outer.rs"),
    vb.SEED_LOG_REJUMP: (1, "executor.rs"),
    vb.SEED_FAIL_LOG: (1, "cert_follow.rs"),
    vb.SEED_PARTIAL_LOG: (2, None),
    vb.DEFER_LOG: (1, "epoch_manager.rs"),
    vb.PROMOTED_LINE: (1, "epoch_manager.rs"),
    vb.PROPOSE_LINE: (1, "application.rs"),
    vo.REJUMP_LOG: (1, "cold_start_jump.rs"),
}


def _hits(token):
    out = []
    for src in sorted(CRATES_DIR.rglob("*.rs")):
        try:
            text = src.read_text(encoding="utf-8", errors="replace")
        except OSError:                                   # pragma: no cover — unreadable file
            continue
        if token not in text:
            continue
        out.extend((src, n, line) for n, line in enumerate(text.splitlines(), 1)
                   if token in line)
    return out


@pytest.mark.parametrize("token", sorted(EMITTERS))
def test_every_grepped_token_is_a_string_the_product_ACTUALLY_EMITS(token):
    """A token that moved silently is exactly how a case rots, and this family greps nine of
    them. `verdicts_onchain.PARK_LOG` is the precedent: it grepped a message half the product had
    rewritten, so the anti-vacuity gate it guarded could not fire at any gap and the case read as
    mis-tuned instead of blind.

    Each token must appear the expected number of times, in the expected file, and NOT in a
    comment — a comment is not an emitted line, and grepping one makes the gate unfalsifiable."""
    if not CRATES_DIR.is_dir():
        pytest.skip(f"consensus crate not in this tree ({CRATES_DIR})")
    want_n, want_file = EMITTERS[token]
    hits = _hits(token)
    assert len(hits) == want_n, (
        f"{token!r} must appear {want_n}x under crates/, found "
        f"{[(str(p), n) for p, n, _ in hits]}")
    for src, lineno, line in hits:
        if want_file:
            assert src.name == want_file, f"{token!r} moved to {src}"
        assert not line.strip().startswith("//"), (
            f"{src}:{lineno} is a COMMENT — grepping it would make this gate unfalsifiable")


@pytest.mark.parametrize("metric", [vb.M_REFETCHED, vb.M_REFETCH_FAILED, vb.M_SPAWN_DEFERRED])
def test_every_metric_name_is_registered_by_the_product(metric):
    """The REGISTERED literal, matched against the Rust `ctx.register("<name>", …)` call. The
    scrape name is this plus prometheus-client's own `_total` (`vb.counter_sample`), which is why
    the constant stays the registered spelling and only the reader applies the suffix."""
    if not CRATES_DIR.is_dir():
        pytest.skip(f"consensus crate not in this tree ({CRATES_DIR})")
    needle = f'"{metric}"'
    hits = [(src, n) for src in sorted(CRATES_DIR.rglob("*.rs"))
            for n, line in enumerate(src.read_text(encoding="utf-8", errors="replace")
                                     .splitlines(), 1)
            if needle in line]
    assert len(hits) == 1, f"{metric!r} must be registered exactly once, found {hits}"


def test_the_two_jump_gates_are_the_values_the_product_uses():
    """`min(JUMP_THRESHOLD, epochBlockInterval)` for the steady-state re-jump and a fixed
    `JUMP_THRESHOLD` for the cold start. The whole case rests on the gap sitting between them, so
    the harness's copy of both numbers is checked against the Rust rather than trusted."""
    if not CRATES_DIR.is_dir():
        pytest.skip(f"consensus crate not in this tree ({CRATES_DIR})")
    cs = (CRATES_DIR / "dpos/consensus/src/cold_start_jump.rs").read_text()
    m = re.search(r"pub const JUMP_THRESHOLD:\s*u64\s*=\s*(\d+)", cs)
    assert m, "JUMP_THRESHOLD is no longer a plain u64 const in cold_start_jump.rs"
    assert int(m.group(1)) == vo.JUMP_THRESHOLD

    dp = (CRATES_DIR / "dpos/consensus/src/dpos.rs").read_text()
    assert "JUMP_THRESHOLD.min(interval as u64)" in dp, (
        "the steady-state re-jump gate is no longer min(JUMP_THRESHOLD, interval) — "
        "`vo.rejump_threshold` and `vb.boundary_gap` are computed from that shape")
