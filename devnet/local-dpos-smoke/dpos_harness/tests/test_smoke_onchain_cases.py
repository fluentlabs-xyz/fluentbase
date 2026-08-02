"""`cases/smoke/` — the four ported ON-CHAIN drivers, against `scripts/case-liveness.sh`,
`case-byzantine.sh`, `case-cert-catchup.sh` and `case-vrf-dkg-restart-midwindow.sh`.

The same four kinds of evidence as the three sibling files, plus one this chunk introduces:

  * THE COMMAND TRANSCRIPT (`--dry-run`) — argv for argv against the bash, IN ORDER. Two of these
    cases have 30+ reads and four stop/start pairs, so the comparison there is over the SHAPE and
    the COUNTS (which services, in which order, how many of each) rather than one long literal
    list, which would pin nothing a reader could check against the bash.
  * THE WIRING (stubbed readers on the LIVE branches) — a body that reads the right things and
    forgets to apply the verdict passes every transcript test and fails here.
  * THE NEGATIVES, DRIVEN — a world in which the forbidden thing HAPPENS. For `cert-catchup`
    that means BOTH absence-greps driven with the line present, and the park gate driven with a
    victim that rejoined without parking. `test_smoke_onchain_verdicts.py` proves the decisions
    are right; these prove the bodies ask them, on the readings that matter.
  * THE GENESIS EXPORTS — new here. Two cases tune the chain through `os.environ`, which reaches
    genesis-init via compose interpolation. The export has to land BEFORE the bring-up `up`, the
    PROFILE has to see the same value (or the case runs 64-block epochs with 32-block
    arithmetic), and the environment has to be back to normal afterwards.
"""

from __future__ import annotations

import os
import pathlib
import re

import pytest

from dpos_harness.cases.smoke import (asserts_onchain, byzantine, cert_catchup, driver, liveness,
                                      verdicts_onchain as vo, vrf_dkg_restart_midwindow as mw)
from dpos_harness.cases.smoke.driver import SmokeCtx, SmokeFailure
from dpos_harness.core.proc import Runner
from dpos_harness.stack.profiles import StaticProfile
from dpos_harness.stack.static_stack import StaticStack

VALS = ["validator-0", "validator-1", "validator-2", "validator-3"]
UP = ["docker", "compose", "up", "--build", "-d"]
STOP = ["docker", "compose", "stop", "--timeout", "40", *VALS]
DOWN = ["docker", "compose", "down", "-v", "--remove-orphans"]
CAT_ADDRS = ["docker", "compose", "exec", "-T", "validator-0", "cat", "/runtime/addresses.json"]
#: `driver.SmokeCtx.runtime_addresses`'s canned answer, in `validators[i] == validator-i` order.
DRY_ADDRS = ["0x" + f"{0xa0 + i:02x}" * 20 for i in range(4)]

TUNED = ("EPOCH_BLOCK_INTERVAL", "EPOCH_INTERVAL", "DPOS_ACTIVATION_BLOCK")


@pytest.fixture(autouse=True)
def _clean_env(monkeypatch):
    for k in TUNED + ("DPOS_EXTRA_COMPOSE", "DPOS_CONVERGE_EXCLUDE", "SIM_DATA_ROOT",
                      "SMOKE_KEEP_UP", "RPC", "CHAIN_ID", "CERT_CATCHUP_VICTIM",
                      "CERT_CATCHUP_GAP", "CERT_CATCHUP_DEEP", "CERT_CATCHUP_DEEP_GAP",
                      "CERT_CATCHUP_DELAY_MS"):
        monkeypatch.delenv(k, raising=False)


def _dry(case_module, argv=("--dry-run",)):
    made = {}
    real = driver.Runner

    def spy(**kw):
        made["r"] = real(**kw)
        return made["r"]

    driver.Runner = spy
    try:
        rc = case_module.run_case(list(argv))
    finally:
        driver.Runner = real
    return rc, made["r"]


def _cmds(runner):
    """The real commands, with the in-process `<step>` markers dropped — the argv oracle."""
    return [inv.argv for inv in runner.log if inv.kind != "step"]


def _seq(runner):
    """The whole transcript in order: a command joined, a `<step>` as its detail."""
    return [inv.note if inv.kind == "step" else " ".join(inv.argv) for inv in runner.log]


def _idx(seq, needle, after=-1):
    return next(i for i, s in enumerate(seq) if needle in s and i > after)


def _recreate(*overlays):
    files = ["-f", "docker-compose.yml", "-f", "docker-compose.dpos.yml"]
    for o in overlays:
        files += ["-f", o]
    return ["docker", "compose", *files, "up", "-d", "--force-recreate", *VALS]


def _live_ctx(monkeypatch, interval=32, activation=64, overlay=None, **stubs):
    """A `SmokeCtx` on its LIVE branches with every reader stubbed.

    Same construction as the three sibling files: the Runner stays dry (writes are recorded,
    nothing spawns) while `dry` reports False, so `check` raises and `poll` really loops. The
    CLOCK is fast-forwarded rather than the constants shrunk — `cert-catchup`'s gap waits and
    `vrf-dkg-restart-midwindow`'s 400 s journal poll ARE the assertion, and a test that shortened
    them would leave somebody free to shorten them for real."""
    import time as _time
    clock = [0.0]

    def _tick():
        clock[0] += 3600.0
        return clock[0]

    monkeypatch.setattr(_time, "monotonic", _tick)
    monkeypatch.setattr(_time, "sleep", lambda _s: None)

    runner = Runner(dry=True)
    monkeypatch.setattr(runner, "_exec",
                        lambda *a, **k: pytest.fail("the recorder executed a command"))
    profile = StaticProfile(extra_overlays=((overlay,) if overlay else ()),
                            epoch_interval=interval, activation_block=activation)
    stack = StaticStack(profile=profile, runner=runner)
    stack.prev_fin = "0x50"
    ctx = SmokeCtx(stack)
    monkeypatch.setattr(SmokeCtx, "dry", property(lambda self: False))
    for name, value in stubs.items():
        monkeypatch.setattr(ctx, name, value)
    return ctx, runner


# ══ the command transcripts ════════════════════════════════════════════════

def test_liveness_transcript_is_bash_faithful_in_shape_and_counts():
    """`case-liveness.sh:30,49,88` × four cycles. The literal list would be 54 entries of which
    50 are reads, so what is pinned is the SHAPE: the address read once, then exactly four
    stop/start PAIRS, on the victims and in the order `:127-130` names — v3 deep, v2 single
    boundary, v1 within-epoch, v3 again (the immediate double-rejoin).

    The last cycle re-killing v3 is the one that catches stale catch-up state carried across
    restarts, so a port that deduplicated the victim list would delete it."""
    rc, r = _dry(liveness)
    assert rc == 0
    cmds = _cmds(r)
    assert cmds[0] == UP and cmds[1] == STOP and cmds[2] == _recreate()
    assert cmds[3] == CAT_ADDRS
    lifecycle = [c for c in cmds[4:-1] if c[:3] == ["docker", "compose", "stop"]
                 or c[:3] == ["docker", "compose", "start"]]
    assert lifecycle == [
        ["docker", "compose", "stop", "--timeout", "40", "validator-3"],
        ["docker", "compose", "start", "validator-3"],
        ["docker", "compose", "stop", "--timeout", "40", "validator-2"],
        ["docker", "compose", "start", "validator-2"],
        ["docker", "compose", "stop", "--timeout", "40", "validator-1"],
        ["docker", "compose", "start", "validator-1"],
        ["docker", "compose", "stop", "--timeout", "40", "validator-3"],
        ["docker", "compose", "start", "validator-3"],
    ]
    assert cmds[-1] == DOWN


def test_liveness_waits_for_the_bootstrap_DKG_before_ANY_disruption():
    """`case-liveness.sh:114-117`. A victim stopped before its epoch-2 ceremony is permanently
    shareless on rejoin (no reshare in v1) and cannot re-promote to signer, so a case that
    disrupted first would be exercising an out-of-scope path and calling it a rejoin failure.

    The target is `activation + 2*interval + 8` — 136 on the default 32-block stack."""
    seq = _seq(_dry(liveness)[1])
    dkg = _idx(seq, "wait_finalized_ge(136, 180s)")
    first_stop = _idx(seq, "stop --timeout 40 validator-3")
    assert dkg < first_stop


def test_liveness_reads_the_production_credit_while_the_victim_is_DOWN_and_rejoins_after():
    """The order IS the assertion. `producedAt` is a per-epoch counter: reading it after the
    restart would sample a window in which the victim is signing again, and the lag would have
    closed."""
    seq = _seq(_dry(liveness)[1])
    stop = _idx(seq, "stop --timeout 40 validator-3")
    advance = _idx(seq, "wait_finalized_ge(97, 240s)", stop)
    part = _idx(seq, "production(2, " + DRY_ADDRS[3], advance)
    start = _idx(seq, "start validator-3", part)
    peers = _idx(seq, "peer_count(validator-3)", start)
    assert stop < advance < part < start < peers


def test_liveness_snapshots_the_promote_LEDGER_BEFORE_the_stop_and_re_reads_it_after_the_start():
    """THE DELTA SHAPE, pinned in the transcript. `docker compose stop`/`start` keeps the SAME
    container, so its log carries the promotion the victim did on its way into the epoch it is
    about to be killed in. A port that grepped the log ONCE, after the restart, would be satisfied
    by that dead node's line and the gate would be vacuous — the same trap `cert-catchup`'s four
    counters are read twice for.

    And the second read is AFTER the rejoin poll, not merged into it: the promote can only follow
    the landing, so a single fused poll would spend the height budget on a signing question that
    cannot yet have an answer."""
    seq = _seq(_dry(liveness)[1])
    before = _idx(seq, "logs_all(validator-3)")
    stop = _idx(seq, "stop --timeout 40 validator-3", before)
    start = _idx(seq, "start validator-3", stop)
    peers = _idx(seq, "peer_count(validator-3)", start)
    after = _idx(seq, "logs_all(validator-3)", peers)
    assert before < stop < start < peers < after
    # One snapshot + one poll sample per cycle, on the cycle's own victim — v3 twice (cycles 1
    # and 4), v2 and v1 once each.
    assert len([s for s in seq if s.startswith("logs_all(")]) == 8
    assert len([s for s in seq if s == "logs_all(validator-3)"]) == 4


def test_liveness_polls_the_signing_gate_for_a_WHOLE_EPOCH_plus_slack():
    """The budget is derived, not chosen: a re-jumped member that misses boundary seeding promotes
    at the NEXT boundary, at most one epoch away, and one epoch is `interval` seconds at 1 blk/s.
    A test that let it be shortened would let the next operator buy a green with it."""
    seq = _seq(_dry(liveness)[1])
    assert vo.liveness_signing_budget(32) == 92
    assert len([s for s in seq if "signing_again (<= 92s)" in s]) == 4


def test_liveness_reads_the_hub_counters_too_and_re_reads_the_epoch_each_poll():
    """Both addresses per poll: the assertion is RELATIVE (`victim < hub`), so a case that read
    only the victim would be asserting an absolute count nobody chose. And `currentEpoch()` is
    re-read every iteration — the window can roll over mid-poll, and reading counters of an epoch
    that has just been superseded is how a correct victim reads as absent."""
    seq = _seq(_dry(liveness)[1])
    epoch = _idx(seq, "staking_call(currentEpoch()(uint64))")
    vic = _idx(seq, "production(2, " + DRY_ADDRS[3], epoch)
    hub = _idx(seq, "production(2, " + DRY_ADDRS[0], vic)
    assert epoch < vic < hub
    assert len([s for s in seq if s.startswith("staking_call(currentEpoch")]) == 4


def test_byzantine_transcript_is_bash_faithful():
    """`case-byzantine.sh:14,15,22,26,53`, argv for argv. Everything else is a READ recorded as a
    marker. Note what IS a command: only the address `cat` — the status poll, the baseline and
    the liveness wait all go through `core/nodes` / `core/converge`."""
    rc, r = _dry(byzantine)
    assert rc == 0
    assert _cmds(r) == [UP, STOP, _recreate(vo.BYZANTINE_OVERLAY), CAT_ADDRS, DOWN]


def test_byzantine_carries_its_overlay_on_the_recreate_and_excludes_v3_from_converge():
    """The overlay reaches the chain through `_migrate_to_dpos`'s recreate — the single honouring
    point 23 cases inherit. And validator-3's reth NEVER finalizes, so without the exclusion the
    post-swap alignment gate would fail a case whose victim is behaving exactly as designed."""
    made = {}
    real = driver.StaticStack

    def spy(**kw):
        made["stack"] = real(**kw)
        return made["stack"]

    driver.StaticStack = spy
    try:
        byzantine.run_case(["--dry-run"])
    finally:
        driver.StaticStack = real
    assert made["stack"].converge_exclude == "validator-3"
    assert made["stack"].profile.extra_overlays == (vo.BYZANTINE_OVERLAY,)


def test_byzantine_asserts_liveness_AFTER_the_jail_not_before():
    """TWO assertions, and the second only means something after the first. A post-jail
    committee-drop epoch-boundary wedge is a real DPoS failure class, and it can only be observed
    once the committee has actually changed — which happens AT the jail."""
    seq = _seq(_dry(byzantine)[1])
    status = _idx(seq, "validator_status(")
    baseline = _idx(seq, "baseline_height()", status)
    advance = _idx(seq, "wait_finalized_ge(3, 90s)", baseline)
    assert status < baseline < advance


def _netem(verb, service="validator-2", tail=()):
    return ["docker", "compose", "exec", "-T", service,
            "tc", "qdisc", verb, "dev", vo.CATCHUP_NETEM_IFACE, "root", *tail]


NETEM_ADD = _netem("add", tail=("netem", "delay", f"{vo.CATCHUP_NETEM_DELAY_MS}ms"))
NETEM_DEL = _netem("del")


def test_cert_catchup_transcript_is_bash_faithful_in_shape_and_counts():
    """`case-cert-catchup.sh:135,154`. The PRE-FLIGHT unshape, one graceful stop, one start, one
    netem add/del PAIR, and the four log counters read TWICE each — once before the stop, once
    after the rejoin. The delta shape is the whole reason the counts are eight and not four."""
    rc, r = _dry(cert_catchup)
    assert rc == 0
    assert _cmds(r) == [
        UP, STOP, _recreate(vo.CATCHUP_OVERLAY),
        NETEM_DEL,
        ["docker", "compose", "stop", "--timeout", "40", "validator-2"],
        ["docker", "compose", "start", "validator-2"],
        NETEM_ADD,
        NETEM_DEL,
        DOWN,
    ]
    seq = _seq(r)
    for pattern in (vo.PARK_LOG, vo.REJUMP_LOG, vo.OLD_FATAL, vo.EXIT_LOG):
        hits = [s for s in seq if f"log_count(validator-2, '{pattern}')" in s]
        assert len(hits) == 2, pattern


def test_cert_catchup_shapes_the_VICTIM_AND_NOBODY_ELSE():
    """The delay is the lever that makes the park fire, and it belongs to ONE node.

    Shaping a second validator would slow the committee — the side of the race that must stay
    fast — and the case would then be measuring a degraded chain rather than a lagging victim.
    So: every `tc` command in the run names the victim, and there is exactly one add — the two
    others are the pre-flight clear and the post-walk clear, and a `del` shapes nothing."""
    cmds = _cmds(_dry(cert_catchup)[1])
    tc = [c for c in cmds if "tc" in c]
    assert [c[4] for c in tc] == ["validator-2"] * 3, tc
    assert len([c for c in tc if "add" in c]) == 1
    # …and the delay is a DELAY, never a `rate`: throttling bandwidth would also slow the live
    # finalization stream that keeps the ordering tip climbing, which is the wrong side.
    assert "netem" in NETEM_ADD and "delay" in NETEM_ADD
    assert not any("rate" in c for c in tc)


def test_cert_catchup_shapes_only_the_CATCH_UP_WINDOW():
    """After the restart (there is nothing to shape while the victim is down) and removed before
    the park counters are read (a delay left on would slow the live chain the rest of the case
    measures, and would carry into the optional deep cycle)."""
    seq = _seq(_dry(cert_catchup)[1])
    start = _idx(seq, "start validator-2")
    add = _idx(seq, " ".join(NETEM_ADD), start)
    gauge = _idx(seq, "deferred_gauge(validator-2)", add)
    delete = _idx(seq, " ".join(NETEM_DEL), gauge)
    park1 = _idx(seq, f"log_count(validator-2, '{vo.PARK_LOG}')", delete)
    assert start < add < gauge < delete < park1


def test_cert_catchup_UNSHAPES_the_victim_BEFORE_any_other_command_touches_it():
    """THE SELF-HEALING PRE-FLIGHT. The `finally` and bash's EXIT trap only cover an unwind of
    THIS process; neither fires on a SIGKILL of the parent, which is how a run ended on
    2026-07-30 and left validator-2 carrying a 1 s delay. netem lives in the container's netns,
    so a container that survives into the next bring-up keeps its qdisc — and every later case on
    that stack then measures a chain that is quietly degraded and reads as a slow product.

    So the case does not TRUST that it was cleaned up: it clears first, unconditionally, on every
    run. "First" is the assertion — a clear issued after the DKG wait or after the stop would let
    a stale delay shape the very window it is meant to leave alone."""
    cmds = _cmds(_dry(cert_catchup)[1])
    # Commands addressed at the victim ALONE — the whole-stack bring-up lines name all four.
    victim_cmds = [c for c in cmds if "validator-2" in c and "validator-0" not in c]
    assert victim_cmds[0] == NETEM_DEL, victim_cmds[:3]
    # …and it is a `del`, not a re-apply: the pre-flight restores the UNSHAPED condition.
    assert "add" not in victim_cmds[0]


def test_an_operator_override_of_the_netem_delay_is_honoured(monkeypatch):
    """`${CERT_CATCHUP_DELAY_MS:-300}`. 300 ms is a STARTING POINT, not a proven value — it is
    the number one live iteration retunes if the park still does not fire — so the override has to
    reach the actual `tc` command, not merely the failure message."""
    monkeypatch.setenv("CERT_CATCHUP_DELAY_MS", "800")
    tc = [c for c in _cmds(_dry(cert_catchup)[1]) if "tc" in c and "add" in c]
    assert tc == [_netem("add", tail=("netem", "delay", "800ms"))]


def test_cert_catchup_REMOVES_the_qdisc_on_the_FAILURE_path_too(monkeypatch):
    """THE ONE THAT MATTERS. A stack left shaped is not a stopped node, it is a slow one: every
    later assertion — and, on a kept-up stack, every later case — would measure a chain that is
    quietly degraded, and the slowdown would read as the product's rather than as this case's.

    Driven through the rejoin failure, which is the branch a real red run takes."""
    ctx, runner = _live_ctx(monkeypatch, interval=CC_INTERVAL, activation=CC_ACTIVATION,
                            **_cc_world(check_node=lambda s, dry_value="": "0x90|0xbb"))
    with pytest.raises(SmokeFailure):
        asserts_onchain.assert_cert_catchup(ctx)
    cmds = [inv.argv for inv in runner.log if inv.kind != "step"]
    assert NETEM_ADD in cmds
    # The clear that matters is the one AFTER the apply — `cmds` opens with the pre-flight clear,
    # which proves nothing about the failure path.
    add = cmds.index(NETEM_ADD)
    assert NETEM_DEL in cmds[add:], cmds[add:]


def test_the_netem_removal_cannot_MASK_the_real_failure():
    """`run_ok`, not `run_checked`, and the asymmetry with the apply is the point. The removal is
    also the cleanup path — the qdisc may never have been added, the container may already be
    gone — and a raise there would replace the real failure with a cleanup error. The APPLY is the
    opposite: fail-loud, or the case runs unshaped and reports a gap problem it does not have."""
    import inspect

    def channel(fn):
        """The `self.p.<method>` the body actually calls — the docstrings name both."""
        return [ln.split("self.p.")[1].split("(")[0]
                for ln in inspect.getsource(fn).splitlines() if "self.p." in ln]

    assert channel(driver.SmokeCtx.netem_delay) == ["run_checked"]
    assert channel(driver.SmokeCtx.netem_clear) == ["run_ok"]


def test_cert_catchup_snapshots_the_log_counts_BEFORE_the_stop():
    """The container is the SAME across `stop`/`start`, so its log persists. A "before" snapshot
    taken after the restart would include whatever the restart produced, and every one of the
    four deltas would read as zero — three absence gates that pass and one park gate that fails
    for the wrong reason."""
    seq = _seq(_dry(cert_catchup)[1])
    first = _idx(seq, f"log_count(validator-2, '{vo.OLD_FATAL}')")
    stop = _idx(seq, "stop --timeout 40 validator-2", first)
    start = _idx(seq, "start validator-2", stop)
    second = _idx(seq, f"log_count(validator-2, '{vo.OLD_FATAL}')", start)
    assert first < stop < start < second


def test_cert_catchup_samples_the_deferred_gauge_INSIDE_the_rejoin_poll():
    """The park is transient — the gauge clears the instant the cert lands — so a sample taken
    after the rejoin poll returns would read 0 on a victim that parked all through the walk."""
    seq = _seq(_dry(cert_catchup)[1])
    start = _idx(seq, "start validator-2")
    gauge = _idx(seq, "deferred_gauge(validator-2)", start)
    head = _idx(seq, "check_node(validator-2)", gauge)
    assert start < gauge < head


def test_cert_catchup_runs_the_deep_cycle_only_when_asked(monkeypatch):
    """`CERT_CATCHUP_DEEP=1` (`:224`). The deep gap CROSSES the re-jump threshold, so it is a
    different assertion (the gap-gated re-jump) and it tolerates a liveness demotion — which is
    why it is opt-in rather than always-on."""
    assert len([c for c in _cmds(_dry(cert_catchup)[1])
                if c[:3] == ["docker", "compose", "start"]]) == 1
    monkeypatch.setenv("CERT_CATCHUP_DEEP", "1")
    seq = _seq(_dry(cert_catchup)[1])
    assert len([s for s in seq if "start validator-2" in s]) == 2
    # 2*64 + 64/2 = 160, above the 64-block re-jump threshold; timeout 160*4+120.
    assert vo.deep_gap(CC_INTERVAL) == 160 > vo.rejump_threshold(CC_INTERVAL)
    assert any("wait_finalized_ge(160, 760s)" in s for s in seq)


def test_midwindow_transcript_is_bash_faithful():
    """`case-vrf-dkg-restart-midwindow.sh:73,93,106,141`, argv for argv. The two `find` probes are
    the case's own commands — their whole answer is the EXIT CODE — and the restart is a
    `restart`, not a stop/start pair: bash uses `docker compose restart`, which keeps the same
    container, hence the same datadir and the same accumulated log the resume grep reads."""
    rc, r = _dry(mw)
    assert rc == 0
    assert _cmds(r) == [
        UP, STOP, _recreate(), CAT_ADDRS,
        ["docker", "compose", "exec", "-T", "validator-3", "sh", "-c", vo.journal_probe(3, 2)],
        ["docker", "compose", "exec", "-T", "validator-3", "sh", "-c", vo.share_probe(3, 2)],
        ["docker", "compose", "restart", "validator-3"],
        DOWN,
    ]


def test_midwindow_gates_the_restart_on_journal_PRESENT_and_share_ABSENT():
    """BOTH halves, before the restart. The journal alone is not the gate: a restart that lands
    post-finalize hits `maybe_start`'s store-hit early-return and never logs a resume — the
    original false-RED (`:98-104`). And the restart must come after both, or the case is
    restarting a node in a state it did not verify."""
    seq = _seq(_dry(mw)[1])
    journal = _idx(seq, "beacon-dkgjournal-e2.bin")
    share = _idx(seq, "beacon-share-e2.bin", journal)
    restart = _idx(seq, "restart validator-3", share)
    assert journal < share < restart


def test_midwindow_reads_the_resume_log_AFTER_the_restart_and_the_boundary():
    """The resume line can only be emitted by the RESTARTED process. Reading before the boundary
    crossing would sample a ceremony that has not had time to converge, and the case would report
    a broken resume on a node that was still working."""
    seq = _seq(_dry(mw)[1])
    restart = _idx(seq, "restart validator-3")
    boundary = _idx(seq, "wait_finalized_ge(262, 400s)", restart)
    logs = _idx(seq, "logs_all(validator-3)", boundary)
    assert restart < boundary < logs


def test_midwindow_compares_the_whole_epoch2_window_node_against_node():
    """`:192-196` — every height from the boundary block to the probe, read from the VICTIM
    in-container and from the reference. Seven heights, two reads each: a single-height compare
    would pass on a victim that derived one block correctly and diverged on the rest."""
    seq = _seq(_dry(mw)[1])
    epoch2_start = vo.MIDWINDOW_ACTIVATION_BLOCK + 2 * vo.MIDWINDOW_EPOCH_INTERVAL
    heights = list(range(epoch2_start, epoch2_start + vo.BOUNDARY_PROBE_OFFSET + 1))
    assert len(heights) == 7
    for h in heights:
        assert f"mixhash_in(validator-3, {h})" in seq
        assert f"mixhash_at({h})" in seq


def test_midwindow_reads_the_slash_log_across_the_WHOLE_project():
    """`:242` greps `docker compose logs` with NO service. It has to: the slash is dispatched by
    whichever validator observed the miss, so scoping it to the victim would search the one node
    that cannot have logged it."""
    assert "logs_all_project()" in _seq(_dry(mw)[1])


def test_midwindow_asserts_production_AND_the_slash_log_AND_the_status():
    """Three independent witnesses of one property, in order. The counter says the victim
    produced, the log says nothing slashed it, and the status says it is not jailed — a port that
    kept only the first would pass on a chain that punished it anyway."""
    seq = _seq(_dry(mw)[1])
    prod = _idx(seq, f"production(2, {DRY_ADDRS[3]})")
    log = _idx(seq, "logs_all_project()", prod)
    status = _idx(seq, f"validator_status({DRY_ADDRS[3]})", log)
    assert prod < log < status


# ══ the genesis exports ════════════════════════════════════════════════════

def test_the_tuned_exports_land_before_the_bring_up_that_consumes_them():
    """`docker-compose.yml:51,57,58` interpolates these into genesis-init's environment, so an
    export that arrived after `docker compose up --build -d` would build a DEFAULT chain and the
    case would then measure tuned epochs while asserting against the default 32."""
    for module, want in ((cert_catchup, ["EPOCH_BLOCK_INTERVAL=64", "EPOCH_INTERVAL=64"]),
                         (mw, ["EPOCH_BLOCK_INTERVAL=64", "DPOS_ACTIVATION_BLOCK=128",
                               "EPOCH_INTERVAL=64"])):
        seq = _seq(_dry(module)[1])
        exports = [s for s in seq if "=" in s and s.split("=")[0] in TUNED]
        assert exports == want, module
        assert _idx(seq, "up --build -d") > seq.index(exports[-1])


def test_the_tuned_interval_reaches_the_PROFILE_not_only_compose():
    """The other half. `StaticProfile` reads `EPOCH_INTERVAL` / `DPOS_ACTIVATION_BLOCK` off the
    environment, so an export that reached compose but not the profile would give the case a
    tuned stack and 32-block epoch arithmetic — every derived target off by a factor of two, and
    nothing red.

    The witness is an interval-derived target read off each case's own transcript: cert-catchup's
    bootstrap-DKG block (`activation + 2*interval + 8` = 264, the interval read three times over,
    since activation is itself `2*interval`) and midwindow's boundary target (262). A profile
    still on the default 32 cannot produce either."""
    for module, target in ((cert_catchup, 264), (mw, 262)):
        seq = _seq(_dry(module)[1])
        assert any("dposActivationBlock" not in s for s in seq)
        assert any(f"wait_finalized_ge({target}," in s for s in seq), module


def test_a_tuned_case_leaves_the_environment_as_it_found_it():
    """bash cannot do this — its process exits — and nothing observes the variable there either,
    so it is invisible. What it buys is that a tuned genesis cannot leak into whatever runs next
    in the same interpreter, this suite included."""
    before = {k: os.environ.get(k) for k in TUNED}
    _dry(mw)
    assert {k: os.environ.get(k) for k in TUNED} == before


def test_an_operator_override_of_the_catchup_interval_is_honoured(monkeypatch):
    """`${EPOCH_BLOCK_INTERVAL:-64}` (`:65`). The override wins, and `EPOCH_INTERVAL` follows it
    unconditionally (`:66`) so the container's genesis and the host-side chain math can never
    drift apart — which is the exact reason bash mirrors them."""
    monkeypatch.setenv("EPOCH_BLOCK_INTERVAL", "96")
    seq = _seq(_dry(cert_catchup)[1])
    assert [s for s in seq if s.split("=")[0] in TUNED] == ["EPOCH_BLOCK_INTERVAL=96",
                                                            "EPOCH_INTERVAL=96"]


def test_every_ported_case_is_registered_in_the_cli():
    from dpos_harness.cli import CASES
    for name, mod in (("smoke-liveness", "smoke.liveness"),
                      ("smoke-byzantine", "smoke.byzantine"),
                      ("smoke-cert-catchup", "smoke.cert_catchup"),
                      ("smoke-vrf-dkg-restart-midwindow", "smoke.vrf_dkg_restart_midwindow")):
        assert CASES[name] == mod


def test_liveness_stays_OUT_of_the_fault_aggregate():
    """Its jail is UNRECOVERABLE — the cycles can push the participation counter to a jail, which
    permanently shrinks the committee — so unlike the five `smoke-fault` bodies it does not
    restore what it broke and must stay an isolated stack. `fault.py` records the same exclusion
    from the other side."""
    from dpos_harness.cases.smoke import fault
    assert asserts_onchain.assert_liveness not in fault.ASSERTIONS
    assert asserts_onchain.assert_byzantine not in fault.ASSERTIONS


# ══ the wiring: liveness ═══════════════════════════════════════════════════

#: A promote line as the node writes it — `?epoch` is a Debug newtype, so `Epoch(N)`.
def _promote(epoch):
    return (f"validator-x  | INFO promoted to Signer in-process: per-epoch BFT engine started "
            f"epoch=Epoch({epoch})\n")


#: The pre-stop log ALREADY carries a promotion: the victim was a working signer right up to the
#: moment it was killed. Epoch 1 is below every cycle's floor epoch (4, 5, 7, 11 — see `_lv_world`)
#: so it can never satisfy the gate on its own, which is the point of having it here at all.
LV_LOG_BEFORE = _promote(1)
#: …and the healthy post-restart log adds a promotion for the live epoch.
LV_LOG_AFTER = LV_LOG_BEFORE + _promote(99)


def _lv_world(after_log=LV_LOG_AFTER, **over):
    # One baseline per cycle. The four cycles' gaps are 97/33/5/33 (interval 32), so the rejoin
    # floors are `pre + gap` = 197, 233, 305, 433 — which is why the readings sit at 500 rather
    # than at the old 100: a victim BELOW the floor is no longer a rejoin, and that is the point.
    # Those floors are epochs 4, 5, 7 and 11 (`(h - 64) // 32`), and THAT is what the signing gate
    # measures a promotion against.
    heights = iter([100, 200, 300, 400, 500, 600, 700, 800])
    # Which side of the restart the log reader is on. Flipped by `peer_count`, which only the
    # rejoin probe calls (i.e. only after `compose_start`), and reset by `baseline_height`, which
    # opens each cycle — so the before/after split holds however many times the poll iterates.
    phase = {"restarted": False}

    def _baseline(dry_value=0):
        phase["restarted"] = False
        return next(heights, 800)

    def _peers(svc, dry_value=1):
        phase["restarted"] = True
        return 2

    def _logs(svc, dry_value=""):
        return after_log if phase["restarted"] else LV_LOG_BEFORE

    world = dict(
        runtime_addresses=lambda dry_value=None: list(DRY_ADDRS),
        wait_finalized_ge=lambda *a, **k: True,
        baseline_height=_baseline,
        logs_all=_logs,
        finalized_dec=lambda dry_value=0: 900,
        staking_call=lambda sig, *a, **k: "4",
        production=lambda epoch, addr, dry_value=None: (
            (10, 10) if addr == DRY_ADDRS[0] else (3, 10)),
        check_external=lambda port, dry_value="": "0x1f4|0xaa",
        check_node=lambda svc, dry_value="": "0x1f4|0xaa",
        # The hub's chain, for the same-height fork half of the rejoin gate: block 500 is 0xaa
        # and the hub holds nothing else. Stubbed EXPLICITLY — left unstubbed it would reach the
        # real `cast block`, which the suite's conftest blocks into a `"null"`, and a fork test
        # that only passes because the fixture swallowed a subprocess is testing the fixture.
        blockhash_at=lambda block, dry_value="null": ("0xaa" if int(block) == 0x1f4 else "null"),
        peer_count=_peers,
    )
    world.update(over)
    return world


def test_liveness_passes_on_a_healthy_world(monkeypatch, capsys):
    ctx, _ = _live_ctx(monkeypatch, **_lv_world())
    asserts_onchain.assert_liveness(ctx)
    out = capsys.readouterr().out
    assert "OK (smoke-liveness)" in out
    assert out.count("on-chain production credit correct") == 4
    assert out.count("rejoined at") == 4
    assert out.count("is SIGNING again") == 4


def test_liveness_FAILS_when_the_victim_is_HEIGHT_ALIGNED_but_never_promotes_again(monkeypatch):
    """THE 2026-08-01 DEFECT, driven through the body. Every height reading is perfect — the
    victim is on the hub's chain, past `pre + gap`, with peers — and it logs no new promotion. It
    is verify-only: no per-epoch BFT engine, no proposals, no votes. Before this gate the case
    marched on, stopped a SECOND validator, and 2 live signers of 4 against a quorum of 3 stalled
    the chain at `finalized=241, pre=241` — a correct BFT stall reported as a product bug."""
    ctx, _ = _live_ctx(monkeypatch, **_lv_world(after_log=LV_LOG_BEFORE))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_liveness(ctx)
    assert "HEIGHT-ALIGNED but NOT SIGNING" in e.value.message
    assert "no 'promoted to Signer in-process' line appeared at all" in e.value.message
    # Cycle 1's floor is 197 = epoch 4, and the message has to name it: an operator reading this
    # needs to know WHICH epoch the member failed to promote for.
    assert "epoch >= 4" in e.value.message


def test_liveness_FAILS_when_the_victim_only_promotes_for_a_STALE_epoch(monkeypatch):
    """The false pass a bare log-delta would take. A restarted victim boots on its persisted tail
    and reconciles the epoch it was KILLED in first, so it really can log the promote line — for
    an epoch the committee left three boundaries ago. The engine is real and useless: nobody is
    voting on that epoch.

    This is why the gate carries an epoch FLOOR and not just a delta, and it is exactly the shape
    a re-jumped member produces (`epoch_manager.rs:734-749` defers the LIVE epoch's spawn for the
    missing `E-1` boundary block, while the stale one has its boundary on disk)."""
    ctx, _ = _live_ctx(monkeypatch, **_lv_world(after_log=LV_LOG_BEFORE + _promote(1)))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_liveness(ctx)
    assert "HEIGHT-ALIGNED but NOT SIGNING" in e.value.message
    assert "it promoted only for epoch(s) [1] — BELOW the floor" in e.value.message


def test_the_two_liveness_rejoin_failures_do_not_read_alike(monkeypatch):
    """THE POINT OF SPLITTING THEM, and it cost a live run to learn. "Never came back" sends the
    operator to the victim's own catch-up; "came back but is not signing" sends them to the
    engine-spawn gate. One message for both is how `finalized=241, pre=241` was read as a chain
    that would not advance, when the chain was correct and the committee was two signers short."""
    ctx, _ = _live_ctx(monkeypatch, **_lv_world(
        check_node=lambda svc, dry_value="": "0x87|0xbehind",
        blockhash_at=lambda block, dry_value="null": ("0xaa" if int(block) == 0x1f4
                                                      else "0xbehind" if int(block) == 0x87
                                                      else "null")))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_liveness(ctx)
    height_half = e.value.message
    ctx2, _ = _live_ctx(monkeypatch, **_lv_world(after_log=LV_LOG_BEFORE))
    with pytest.raises(SmokeFailure) as e2:
        asserts_onchain.assert_liveness(ctx2)
    signing_half = e2.value.message

    assert "did not rejoin" in height_half and "did not rejoin" not in signing_half
    assert "NOT SIGNING" in signing_half and "NOT SIGNING" not in height_half
    # …and each one names its own next step, so neither sends an operator at the other's cause.
    assert "check its reth devp2p peers" in height_half
    assert "engine-spawn gate" in signing_half


def test_liveness_FAILS_when_the_victim_produced_AS_MUCH_as_the_hub(monkeypatch):
    """THE CORE NEGATIVE, driven through the body. A victim that is DOWN and still reads as
    fully producing means the production record never reached the chain — which is exactly what
    `recordProduction` is supposed to credit and what this case exists to prove it does."""
    ctx, _ = _live_ctx(monkeypatch,
                       **_lv_world(production=lambda e, a, dry_value=None: (10, 10)))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_liveness(ctx)
    assert "production credit wrong" in e.value.message


def test_liveness_FAILS_on_a_PERSISTENT_getter_failure_rather_than_crediting_it(monkeypatch):
    """§2.4 item 5, driven. `-2 < 10` is true as arithmetic, so a body that passed the raw
    numbers to a bare comparison would report a perfect bitmap on a chain it could not read."""
    ctx, _ = _live_ctx(monkeypatch,
                       **_lv_world(production=lambda e, a, dry_value=None: (-2, -2)))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_liveness(ctx)
    assert "getter read FAILED" in e.value.message


def test_liveness_FAILS_when_the_victim_is_not_in_committee(monkeypatch):
    ctx, _ = _live_ctx(monkeypatch, **_lv_world(
        production=lambda e, a, dry_value=None: ((10, 10) if a == DRY_ADDRS[0] else (-1, -1))))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_liveness(ctx)
    assert "NOT in this epoch's committee" in e.value.message


def test_liveness_FAILS_when_the_victim_rejoins_below_the_gap_the_chain_covered(monkeypatch):
    """THE REGRESSION, driven through the body. Cycle 1: pre=100, gap=97, so the chain provably
    reached 197 while the victim was down. A victim back on the hub's own chain at 135 is not a
    rejoin — and calling it one is what let cycle 2 stop a SECOND validator while the first was
    still ~114 blocks behind: 2 of 4 signers, a correct BFT stall, a failed case."""
    ctx, _ = _live_ctx(monkeypatch, **_lv_world(
        check_node=lambda svc, dry_value="": "0x87|0xbehind",
        blockhash_at=lambda block, dry_value="null": ("0xaa" if int(block) == 0x1f4
                                                      else "0xbehind" if int(block) == 0x87
                                                      else "null")))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_liveness(ctx)
    assert "did not rejoin" in e.value.message
    assert "floor=pre+gap=197" in e.value.message


def test_the_liveness_OK_line_prints_the_hub_height_too(monkeypatch, capsys):
    """bash printed `(v0=…)` on this line all along; the port printed only the victim, and that
    asymmetry is what let a victim 114 blocks behind read as a rejoin without anyone noticing."""
    ctx, _ = _live_ctx(monkeypatch, **_lv_world())
    asserts_onchain.assert_liveness(ctx)
    out = capsys.readouterr().out
    assert out.count("v0=0x1f4|0xaa") == 4
    assert "floor=pre+gap=197" in out and "floor=pre+gap=433" in out


def test_liveness_FAILS_when_the_victim_rejoins_with_no_reth_peer(monkeypatch):
    """The half of the rejoin gate that is about the TRANSPORT. A victim matching the hub with
    zero peers is reporting a head it did not sync."""
    ctx, _ = _live_ctx(monkeypatch, **_lv_world(peer_count=lambda svc, dry_value=1: 0))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_liveness(ctx)
    assert "did not rejoin" in e.value.message


def test_liveness_FAILS_LOUD_on_a_short_address_list(monkeypatch):
    """Otherwise `ADDR[3]` is missing and the case index-errors three assertions later, inside
    the participation read, where it would look like a getter problem."""
    ctx, _ = _live_ctx(monkeypatch,
                       **_lv_world(runtime_addresses=lambda dry_value=None: DRY_ADDRS[:2]))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_liveness(ctx)
    assert "expected 4 validator addresses" in e.value.message


def test_liveness_FAILS_when_the_chain_stops_advancing_with_one_node_down(monkeypatch):
    """BFT f=1 must hold with 3 of 4 up. If it does not, everything after is measuring a dead
    chain and the bitmap would legitimately show nobody signing."""
    ctx, _ = _live_ctx(monkeypatch, **_lv_world(wait_finalized_ge=lambda *a, **k: False))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_liveness(ctx)
    assert "bootstrap DKG" in e.value.message or "did not advance" in e.value.message


# ══ the wiring: byzantine ══════════════════════════════════════════════════

def _byz_world(**over):
    world = dict(
        runtime_addresses=lambda dry_value=None: list(DRY_ADDRS),
        validator_status=lambda addr, dry_value="": "3",
        baseline_height=lambda dry_value=0: 500,
        wait_finalized_ge=lambda *a, **k: True,
        finalized_dec=lambda dry_value=0: 520,
        logs_tail=lambda *s, **k: "",
        dump_logs=lambda *a, **k: None,
    )
    world.update(over)
    return world


def test_byzantine_passes_on_a_healthy_world(monkeypatch, capsys):
    ctx, _ = _live_ctx(monkeypatch, overlay=vo.BYZANTINE_OVERLAY, **_byz_world())
    asserts_onchain.assert_byzantine(ctx)
    out = capsys.readouterr().out
    assert "jailed (status=Jail) by equivocation slashing" in out
    assert "OK (smoke-byzantine)" in out and "advanced past 500 after the jail" in out


def test_byzantine_FAILS_when_the_equivocator_is_never_jailed(monkeypatch):
    """The slasher path is the point of the case. A never-jailed equivocator means the evidence
    never reached the chain, or the decoder is unconfigured, or the byzantine build is not in the
    image at all — and the diagnostic dump names the equivocator FIRST for exactly that reason."""
    ctx, _ = _live_ctx(monkeypatch, overlay=vo.BYZANTINE_OVERLAY,
                       **_byz_world(validator_status=lambda a, dry_value="": "2"))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_byzantine(ctx)
    assert "not jailed within 200s" in e.value.message


def test_byzantine_FAILS_when_the_chain_STALLS_after_the_jail(monkeypatch):
    """THE SECOND ASSERTION, driven. Jailing the equivocator and then wedging at the next epoch
    boundary is a real DPoS failure class — and it passes the first assertion perfectly."""
    ctx, _ = _live_ctx(monkeypatch, overlay=vo.BYZANTINE_OVERLAY,
                       **_byz_world(wait_finalized_ge=lambda *a, **k: False))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_byzantine(ctx)
    assert "chain stalled after jail" in e.value.message


def test_byzantine_dumps_the_EQUIVOCATOR_markers_first_on_a_failure(monkeypatch, capsys):
    """`:40-46`. A slasher that saw nothing and an equivocator that never equivocated look
    identical from validator-0's side, so the first question the dump answers is whether
    validator-3 took the Equivocate path at all."""
    def logs(*svcs, **k):
        return ("INFO BYZANTINE equivocating now" if svcs[0] == "validator-3"
                else "INFO slash evidence submitted")

    ctx, _ = _live_ctx(monkeypatch, overlay=vo.BYZANTINE_OVERLAY,
                       **_byz_world(validator_status=lambda a, dry_value="": "2", logs_tail=logs))
    with pytest.raises(SmokeFailure):
        asserts_onchain.assert_byzantine(ctx)
    out = capsys.readouterr().out
    eq = out.index("validator-3 (equivocator) byzantine markers")
    sl = out.index("validator-0/1 slasher / conflict markers")
    assert eq < sl
    assert "BYZANTINE equivocating now" in out and "slash evidence submitted" in out


# ══ the wiring: cert-catchup ═══════════════════════════════════════════════

#: The case's own geometry, which every cert-catchup world below is built on. A `_live_ctx`
#: pinned to the OLD interval would exercise the body against a threshold and a gap ceiling the
#: case no longer runs on, and the ceiling the failure message quotes would not be the constant.
CC_INTERVAL = vo.CATCHUP_EPOCH_INTERVAL
CC_ACTIVATION = 2 * CC_INTERVAL
#: A head high enough to clear the DEEP cycle's floor (`pre + deep_gap(interval)`).
CC_HEAD_DEC = 500
CC_HEAD = hex(CC_HEAD_DEC)


def _cc_world(park=(2, 5), fatal=(0, 0), exits=(0, 0), rejump=(0, 0), **over):
    """A cert-catchup world keyed by the four log counters as `(before, after)` pairs. The FIRST
    read of each pattern returns `before` and every later one `after` — which is what makes the
    delta drivable in both directions from a test."""
    counts = {vo.PARK_LOG: list(park), vo.REJUMP_LOG: list(rejump),
              vo.OLD_FATAL: list(fatal), vo.EXIT_LOG: list(exits)}
    seen = {}

    def log_count(svc, pattern, dry_value=0):
        seen[pattern] = seen.get(pattern, 0) + 1
        pair = counts[pattern]
        return pair[0] if seen[pattern] == 1 else pair[1]

    world = dict(
        log_count=log_count,
        baseline_height=lambda dry_value=0: 300,
        wait_finalized_ge=lambda *a, **k: True,
        finalized_dec=lambda dry_value=0: 360,
        shutdown_flushed=lambda svc, dry_value=True: True,
        deferred_gauge=lambda svc, dry_value="na": "812",
        # 500, not the old 360: the rejoin floor is `pre + gap` and the deep cycle's gap is
        # `deep_gap(64)` = 160, so a world whose readings sit below `pre + 160` = 460 is a victim
        # that never caught up.
        check_external=lambda port, dry_value="": f"{CC_HEAD}|0xaa",
        check_node=lambda svc, dry_value="": f"{CC_HEAD}|0xaa",
        # The hub's chain for the fork half of the rejoin gate — see `_lv_world` on why this is
        # stubbed rather than left to reach a conftest-blocked `cast block`.
        blockhash_at=lambda block, dry_value="null": ("0xaa" if int(block) == CC_HEAD_DEC
                                                      else "null"),
        dump_logs=lambda *a, **k: None,
    )
    world.update(over)
    return world


def test_cert_catchup_passes_on_a_healthy_world(monkeypatch, capsys):
    ctx, _ = _live_ctx(monkeypatch, interval=CC_INTERVAL, activation=CC_ACTIVATION, **_cc_world())
    asserts_onchain.assert_cert_catchup(ctx)
    out = capsys.readouterr().out
    assert "GATE PASS: guard-#2 park EXERCISED" in out and "parked 3 fresh" in out
    assert "deferred_height peak sampled: 812" in out
    assert "OK (smoke-cert-catchup)" in out


def test_cert_catchup_FAILS_when_the_OLD_FATAL_line_IS_present(monkeypatch):
    """THE ABSENCE-GREP, DRIVEN THROUGH THE BODY. A live green run can never walk this branch —
    producing it needs an unpatched binary — so this is the only place it is ever executed."""
    ctx, _ = _live_ctx(monkeypatch, interval=CC_INTERVAL, activation=CC_ACTIVATION,
                       **_cc_world(fatal=(1, 2)))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_cert_catchup(ctx)
    assert "budget-shutdown ERROR reappeared" in e.value.message


def test_cert_catchup_FAILS_when_the_CONSENSUS_EXIT_line_IS_present(monkeypatch):
    """The second absence-grep, driven separately: the two are different regressions and a body
    that checked only one would miss a break that skipped the ERROR and still exited."""
    ctx, _ = _live_ctx(monkeypatch, interval=CC_INTERVAL, activation=CC_ACTIVATION,
                       **_cc_world(exits=(0, 1)))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_cert_catchup(ctx)
    assert "shut down instead of parking" in e.value.message


def test_cert_catchup_FAILS_when_the_victim_rejoined_without_ever_PARKING(monkeypatch):
    """THE FALSE PASS THE GATE EXISTS FOR. Both absence-greps are clean here — the path simply
    never ran — and the victim rejoined perfectly. Without this gate the case would be green
    having exercised nothing at all."""
    ctx, _ = _live_ctx(monkeypatch, interval=CC_INTERVAL, activation=CC_ACTIVATION,
                       **_cc_world(park=(4, 4), deferred_gauge=lambda s, dry_value="na": "na"))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_cert_catchup(ctx)
    assert "guard-#2 park NOT exercised" in e.value.message
    assert "FALSE PASS" in e.value.message
    # The remedy the message names must be the one that can actually work. "Deepen the gap" is
    # the wrong lever twice over (see `vo.evaluate_park_exercised`), and this pins that the
    # message says so rather than sending the next operator back to CERT_CATCHUP_GAP.
    assert "DO NOT just raise CERT_CATCHUP_GAP" in e.value.message
    assert f"~{vo.CATCHUP_GAP_CEILING}" in e.value.message


#: `scripts/case-cert-catchup.sh` and `crates/`, reached from this file. Both are absent in a
#: harness-only checkout, so the tests below SKIP rather than fail there (same shape as
#: `test_smoke_fault_verdicts.py::test_the_field_list_matches_the_rust_codec`).
CASE_CC_SH = pathlib.Path(__file__).resolve().parents[2] / "scripts" / "case-cert-catchup.sh"
CASE_LV_SH = pathlib.Path(__file__).resolve().parents[2] / "scripts" / "case-liveness.sh"
CRATES_DIR = pathlib.Path(__file__).resolve().parents[4] / "crates"
EPOCH_MANAGER_RS = CRATES_DIR / "dpos" / "consensus" / "src" / "epoch_manager.rs"


def _park_log_hits():
    """Every line under `crates/` that CONTAINS `vo.PARK_LOG`, as `(path, lineno, text)`."""
    hits = []
    for src in sorted(CRATES_DIR.rglob("*.rs")):
        try:
            text = src.read_text(encoding="utf-8", errors="replace")
        except OSError:                                     # pragma: no cover — unreadable file
            continue
        if vo.PARK_LOG not in text:
            continue
        hits.extend((src, n, line) for n, line in enumerate(text.splitlines(), 1)
                    if vo.PARK_LOG in line)
    return hits


def test_PARK_LOG_is_a_string_the_product_ACTUALLY_EMITS():
    """THE GUARD THAT WAS MISSING, and the reason this case was red in both trees.

    `PARK_LOG` used to be `finalization cert not local yet` — the CONDITION half of this same
    `warn!` before the cert-miss park class was replaced by guard #2's absent-body arm, which
    rewrote the condition (`guard #2: committee-attested body at h+K not backfilled yet`) and left
    the action half byte-identical. Nothing in the harness noticed: the case kept grepping the old
    condition, `park1 > park0` could not become true at ANY value of `CERT_CATCHUP_GAP`, and the
    anti-vacuity gate that fired instead ("park path NOT exercised") read as a too-shallow gap.
    Only a cross-tree assertion can catch that class, so this is it: the string the case greps must
    be one the product emits, exactly once, from a logging macro rather than a comment.

    Exactly ONCE matters as much as "at least once": two emitters would make `park1 - park0` a
    count of two different events, and the gate's "$n fresh deferred block(s)" a lie."""
    if not CRATES_DIR.is_dir():
        pytest.skip(f"consensus crate not in this tree ({CRATES_DIR})")
    hits = _park_log_hits()
    assert len(hits) == 1, (
        f"{vo.PARK_LOG!r} must appear exactly once under crates/, found "
        f"{[(str(p), n) for p, n, _ in hits]}")
    src, lineno, line = hits[0]
    assert src.name == "executor.rs", f"the guard-#2 park moved to {src}"
    assert not line.strip().startswith("//"), (
        f"{src}:{lineno} is a COMMENT — a comment is not an emitted line, and grepping for it "
        "would make the park gate unfalsifiable again")

    #: …and it must be the guard-#2 `warn!`, not some unrelated future park. Walk back to the
    #: opening macro: the message is split over two source lines by a `\` continuation, so the
    #: condition half sits ABOVE the action token this case greps for.
    body = src.read_text(encoding="utf-8", errors="replace").splitlines()
    window = body[max(0, lineno - 8):lineno]
    assert any("warn!(" in w for w in window), (
        f"{src}:{lineno} is not inside a warn! — the park must stay operator-visible")
    assert any("guard #2: committee-attested body at h+K not backfilled yet" in w
               for w in window), (
        f"{src}:{lineno} no longer carries the guard-#2 condition — the case would be counting "
        "a different park")


def test_the_SIGNER_PROMOTED_LINE_is_a_string_the_product_ACTUALLY_EMITS_ONCE():
    """The same cross-tree guard `PARK_LOG` needed, for the string the SIGNING gate depends on.

    Exactly ONE emitter matters as much as "at least one": the gate is a per-epoch DELTA, so a
    second emitter would make it count two different events. And it has to be the `spawn_engine`
    success arm — every path that leaves the member verify-only returns BEFORE it, which is the
    entire reason this line means "is signing" rather than "tried to sign"."""
    if not EPOCH_MANAGER_RS.exists():
        pytest.skip(f"consensus crate not in this tree ({EPOCH_MANAGER_RS})")
    body = EPOCH_MANAGER_RS.read_text(encoding="utf-8", errors="replace").splitlines()
    hits = [(n, line) for n, line in enumerate(body, 1) if vo.SIGNER_PROMOTED_LINE in line]
    assert len(hits) == 1, f"{vo.SIGNER_PROMOTED_LINE!r} is emitted {len(hits)} times: {hits}"
    lineno, line = hits[0]
    assert not line.strip().startswith("//"), (
        f"epoch_manager.rs:{lineno} is a COMMENT — grepping a comment makes the gate "
        "unfalsifiable")
    window = body[max(0, lineno - 6):lineno]
    assert any("info!(" in w for w in window), "the promotion must stay operator-visible"
    assert any("?epoch" in w for w in window), (
        f"epoch_manager.rs:{lineno} no longer carries `?epoch` — the gate's EPOCH FLOOR would "
        "have nothing to read, and a promotion for a stale epoch would pass as a live one")
    # …and it is guarded by the spawn RETURNING TRUE, not merely attempted.
    assert any("spawn_engine(" in w for w in body[max(0, lineno - 12):lineno])
    # The defer line the failure message points at is real too — it is a diagnostic, so one or
    # more is fine, but zero would mean the message sends the operator after a string that does
    # not exist.
    assert any(vo.SIGNER_DEFER_LINE in w for w in body)


def test_both_trees_run_the_SAME_signing_gate():
    """bash and the port hold the promote token, the epoch extraction and the budget in separate
    literals, so the drift class `test_both_trees_grep_the_SAME_park_string` exists for applies
    here too — and worse, because this gate fails by TIMING OUT: a tree grepping a stale token
    reports "height-aligned but not signing" on a perfectly healthy member."""
    if not CASE_LV_SH.exists():
        pytest.skip(f"bash case not in this tree ({CASE_LV_SH})")
    text = CASE_LV_SH.read_text()
    assert f'/{vo.SIGNER_PROMOTED_LINE}/' in text, (
        f"case-liveness.sh does not select on {vo.SIGNER_PROMOTED_LINE!r}")
    assert f'grep -c "{vo.SIGNER_DEFER_LINE}"' in text
    # The EPOCH extraction, which both trees spell in their own dialect over the same rendering.
    assert r"epoch[=:][[:space:]]*(Epoch\()?([0-9]+)" in text
    # The budget: `interval + slack` on both sides, and the slack is the same number.
    assert f"EPOCH_INTERVAL + {vo.LIVENESS_SIGNING_SLACK_S}" in text
    assert vo.liveness_signing_budget(32) == 32 + vo.LIVENESS_SIGNING_SLACK_S
    # The FLOOR: bash derives it from the same `(h - activation) / interval`.
    assert "($1 - DPOS_ACTIVATION_BLOCK) / EPOCH_INTERVAL" in text
    # …and the ANSI strip, without which both readers are silent on a live log.
    assert "| strip_ansi" in text


def test_both_trees_grep_the_SAME_park_string():
    """The bash case and the port must not drift apart on the one string that decides the gate.

    They are separate trees with separate literals, so a fix applied to one and not the other
    reproduces the original defect in half the runs."""
    if not CASE_CC_SH.exists():
        pytest.skip(f"bash case not in this tree ({CASE_CC_SH})")
    m = re.search(r"^PARK_LOG='([^']*)'", CASE_CC_SH.read_text(), re.M)
    assert m, "PARK_LOG assignment not found in case-cert-catchup.sh"
    assert m.group(1) == vo.PARK_LOG


def test_both_trees_use_the_SAME_INTERVAL_GAP_and_DELAY():
    """THE THREE NUMBERS THE PARK GATE DEPENDS ON, and the two trees hold each in its own literal.

    They are not independent. The delay is what makes the park fire at all; the gap is what the
    victim must walk; the interval is the re-jump ceiling both of those have to stay under
    (`min(1024, epochBlockInterval)`). A live iteration that retunes one tree and not the other
    leaves the bash case and the port measuring DIFFERENT races while both call themselves
    smoke-cert-catchup — the same drift class `test_both_trees_grep_the_SAME_park_string` exists
    for, and the interval is the one that silently moves the ceiling rather than the measurement.

    The INTERFACE too: every service sits on the single `fluent-net` bridge, so `eth0` is
    deterministic — but only as long as both trees say `eth0`."""
    if not CASE_CC_SH.exists():
        pytest.skip(f"bash case not in this tree ({CASE_CC_SH})")
    text = CASE_CC_SH.read_text()
    m = re.search(r'^export EPOCH_BLOCK_INTERVAL="\$\{EPOCH_BLOCK_INTERVAL:-(\d+)\}"', text, re.M)
    assert m, "EPOCH_BLOCK_INTERVAL export not found in case-cert-catchup.sh"
    assert int(m.group(1)) == vo.CATCHUP_EPOCH_INTERVAL
    # …and bash MIRRORS it into the host-side chain math rather than defaulting separately.
    assert 'export EPOCH_INTERVAL="$EPOCH_BLOCK_INTERVAL"' in text
    m = re.search(r'^GAP="\$\{CERT_CATCHUP_GAP:-(\d+)\}"', text, re.M)
    assert m, "GAP assignment not found in case-cert-catchup.sh"
    assert int(m.group(1)) == vo.CATCHUP_GAP
    m = re.search(r'^DELAY_MS="\$\{CERT_CATCHUP_DELAY_MS:-(\d+)\}"', text, re.M)
    assert m, "DELAY_MS assignment not found in case-cert-catchup.sh"
    assert int(m.group(1)) == vo.CATCHUP_NETEM_DELAY_MS
    m = re.search(r"^NETEM_IFACE='([^']*)'", text, re.M)
    assert m, "NETEM_IFACE assignment not found in case-cert-catchup.sh"
    assert m.group(1) == vo.CATCHUP_NETEM_IFACE


def test_both_trees_derive_the_SAME_gap_ceiling():
    """The ceiling is the bound that decides whether the derive-walk survives to park, and bash
    computes it with its own `$(( ))` arithmetic and its own scan. Reimplemented arithmetic
    drifts, so the bash source is evaluated here and compared against `vo.catchup_gap_ceiling`
    across six geometries — including (64, 3000), the delay experiment the guard must not
    forbid — so a formula that happens to agree on one input cannot pass."""
    if not CASE_CC_SH.exists():
        pytest.skip(f"bash case not in this tree ({CASE_CC_SH})")
    import subprocess
    text = CASE_CC_SH.read_text()
    for name in ("BOOT_BLOCKS=10", "MAX_REPAIR=20"):
        assert name in text, name
    # The effective-gap helper plus the scan that solves it for the ceiling, lifted verbatim.
    start = text.index("_catchup_effective_gap() {")
    expr = text[start:text.index("\ndone\n", start) + len("\ndone\n")]
    assert "GAP_CEILING=0" in expr and "REJUMP_THRESHOLD" in expr
    for interval, delay in ((vo.CATCHUP_EPOCH_INTERVAL, vo.CATCHUP_NETEM_DELAY_MS), (64, 3000),
                            (128, 1000), (64, 0), (64, 300), (64, 10 ** 6)):
        script = (f"REJUMP_THRESHOLD={min(interval, vo.JUMP_THRESHOLD)}; DELAY_MS={delay}; "
                  f"BOOT_BLOCKS={vo.CATCHUP_BOOT_BLOCKS}; MAX_REPAIR={vo.CATCHUP_MAX_REPAIR}\n"
                  f"{expr}\necho $GAP_CEILING")
        out = subprocess.run(["bash", "-c", script], capture_output=True, text=True, check=True)
        assert int(out.stdout) == vo.catchup_gap_ceiling(interval, delay), (interval, delay)


def test_the_bash_case_UNSHAPES_the_victim_BEFORE_the_DKG_wait():
    """The bash half of the self-healing pre-flight. bash's EXIT trap does not fire on a SIGKILL
    of the parent, so the guarantee has to be made at the START of the next run, not at the end of
    the last one — and before the DKG wait, which is the first thing that measures the chain."""
    if not CASE_CC_SH.exists():
        pytest.skip(f"bash case not in this tree ({CASE_CC_SH})")
    text = CASE_CC_SH.read_text()
    preflight = text.index('netem_clear "$VICTIM"\necho "netem pre-flight')
    assert text.index("trap cleanup EXIT") < preflight
    assert preflight < text.index("establishing bootstrap DKG")
    # …and before the first thing that stops or shapes the victim.
    assert preflight < text.index('docker compose stop --timeout 40 "$VICTIM"')
    assert preflight < text.index('netem_apply "$VICTIM"')


def test_both_trees_layer_the_SAME_NET_ADMIN_overlay():
    """`tc qdisc add` needs `NET_ADMIN`, and without it the case dies six minutes in. bash reaches
    the overlay through `DPOS_EXTRA_COMPOSE`, the port through `driver.run(overlays=…)` — two
    spellings of one file, which this pins, plus the file actually granting the capability.

    The capability is on ALL FOUR validators on purpose: `CERT_CATCHUP_VICTIM` is
    operator-overridable in both trees, and it is inert without the case's own `tc` exec."""
    if not CASE_CC_SH.exists():
        pytest.skip(f"bash case not in this tree ({CASE_CC_SH})")
    m = re.search(r'^export DPOS_EXTRA_COMPOSE="-f ([^"]*)"', CASE_CC_SH.read_text(), re.M)
    assert m, "DPOS_EXTRA_COMPOSE export not found in case-cert-catchup.sh"
    assert m.group(1) == vo.CATCHUP_OVERLAY

    overlay = CASE_CC_SH.resolve().parents[1] / vo.CATCHUP_OVERLAY
    assert overlay.exists(), f"{overlay} is named by both trees but is not in the tree"
    import yaml
    services = yaml.safe_load(overlay.read_text())["services"]
    assert sorted(services) == VALS
    for name, body in services.items():
        assert body["cap_add"] == ["NET_ADMIN"], name
        # NOTHING ELSE. The overlay grants a capability; the SHAPING is the case's explicit `tc`
        # exec, which targets one service for one window. An overlay that also baked in a netem
        # prologue would shape from bring-up and there would be no window to speak of.
        assert list(body) == ["cap_add"], name


def test_cert_catchup_FAILS_when_the_graceful_stop_did_not_FLUSH(monkeypatch):
    """The fix targets a GRACEFUL restart. An unflushed victim cold re-syncs instead of catching
    up, so the park window never opens and the case would then fail at the park gate with a
    message pointing at the gap rather than at the stop."""
    ctx, _ = _live_ctx(monkeypatch, interval=CC_INTERVAL, activation=CC_ACTIVATION,
                       **_cc_world(shutdown_flushed=lambda s, dry_value=True: False))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_cert_catchup(ctx)
    assert "did not flush on graceful stop" in e.value.message


def test_cert_catchup_FAILS_when_the_victim_never_realigns(monkeypatch):
    ctx, _ = _live_ctx(monkeypatch, interval=CC_INTERVAL, activation=CC_ACTIVATION,
                       **_cc_world(check_node=lambda s, dry_value="": "0x90|0xbb"))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_cert_catchup(ctx)
    assert "did not rejoin within 240s" in e.value.message


def test_cert_catchup_FAILS_when_the_victim_is_past_pre_but_short_of_the_gap(monkeypatch):
    """The floor is `pre + gap`, not `pre`. pre=300 and gap=28, so the chain reached 328 while
    the victim was down; a victim at 320 is on the hub's own chain and past its own pre-stop
    height, and is still short of the gap. Under the old `pre` floor that is a rejoin."""
    ctx, _ = _live_ctx(monkeypatch, interval=CC_INTERVAL, activation=CC_ACTIVATION, **_cc_world(
        check_node=lambda s, dry_value="": "0x140|0xshort",
        blockhash_at=lambda block, dry_value="null": ("0xaa" if int(block) == CC_HEAD_DEC
                                                      else "0xshort" if int(block) == 0x140
                                                      else "null")))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_cert_catchup(ctx)
    assert "did not rejoin within 240s" in e.value.message
    assert "floor=pre+gap=328" in e.value.message


def test_the_cert_catchup_OK_line_prints_the_hub_height_too(monkeypatch, capsys):
    ctx, _ = _live_ctx(monkeypatch, interval=CC_INTERVAL, activation=CC_ACTIVATION, **_cc_world())
    asserts_onchain.assert_cert_catchup(ctx)
    out = capsys.readouterr().out
    assert f"rejoined at {CC_HEAD}|0xaa (v0={CC_HEAD}|0xaa, floor=pre+gap=328)" in out


def test_every_log_gate_in_this_chunk_reads_through_the_ANSI_STRIPPING_reader(monkeypatch):
    """THE LOOP CLOSED ON THE READER, not just on the grep — and stated precisely, because the
    two families of pattern in this chunk do NOT depend on the strip equally:

      * cert-catchup's four counters are MESSAGE substrings with no `=` in them
        (`PARKING derive`, `OuterEngine exited cleanly`, …). tracing colours the level and the field
        keys/values, not the message body, so those match with or without the strip. For them the
        strip is a reader GUARANTEE, not the deciding factor.
      * midwindow's two greps carry a FIELD (`epoch=2`), and there the strip is the whole thing:
        the node writes `epoch<ESC>=<ESC>2`, so an unstripped read matches nothing and a genuine
        resume reads as absent. That direction is driven in
        `test_smoke_onchain_verdicts.py::test_an_ANSI_SPLIT_field_does_not_match…`.

    What this pins is the property both rest on: the readers the case layer uses return text with
    NO escapes in it, so no gate downstream has to know which family its pattern belongs to.
    """
    from dpos_harness.core import nodes, rpc
    raw = (f"2026-07-30T00:00:00Z \x1b[31mERROR\x1b[0m {vo.OLD_FATAL} "
           "height\x1b[0m=\x1b[2m2295\x1b[0m\n")
    monkeypatch.setattr(rpc, "_run_read", lambda *a, **k: raw)
    out = nodes.logs_all("validator-2")
    assert "\x1b" not in out, "the case layer must never see an escape"
    assert "height=2295" in out and "height=2295" not in raw
    assert nodes.log_count("validator-2", vo.OLD_FATAL) == 1
    # The two case-layer spellings that build their own argv strip too (`driver.SmokeCtx`).
    ctx, _ = _live_ctx(monkeypatch)
    monkeypatch.setattr(driver.proc, "read", lambda *a, **k: raw)
    assert "\x1b" not in ctx.logs_all_project()
    assert "\x1b" not in ctx.logs_tail("validator-3", tail=600)


def test_cert_catchup_deep_cycle_does_not_require_a_park(monkeypatch, capsys):
    """The optional cycle's target is the gap-gated re-jump, and a re-jump can fast-forward past
    the derive-walk. Requiring a park there would make it flaky on a fast host — while the
    MANDATORY cycle still requires one, which is what keeps the case honest."""
    monkeypatch.setenv("CERT_CATCHUP_DEEP", "1")
    parks = iter([2, 5, 5, 5])
    world = _cc_world()
    world["log_count"] = lambda svc, p, dry_value=0: (next(parks, 5) if p == vo.PARK_LOG else 0)
    ctx, _ = _live_ctx(monkeypatch, interval=CC_INTERVAL, activation=CC_ACTIVATION, **world)
    asserts_onchain.assert_cert_catchup(ctx)
    out = capsys.readouterr().out
    assert "re-jump landing not observed" in out
    assert "gap-gated re-jump" in out


# ══ the wiring: vrf-dkg-restart-midwindow ══════════════════════════════════

MW_LOGS = (f"INFO {vo.RESUME_LINE} epoch=2 me=3\n"
           f"INFO {vo.SHARE_LINE} epoch=2 me=3\n")


def _mw_world(**over):
    world = dict(
        runtime_addresses=lambda dry_value=None: list(DRY_ADDRS),
        exec_test=lambda svc, snippet, dry_value=False: "dkgjournal" in snippet,
        finalized_dec=lambda dry_value=0: 200,
        wait_finalized_ge=lambda *a, **k: True,
        mixhash_of=lambda svc, blk, dry_value="": "0xaa",
        mixhash_in=lambda svc, blk, dry_value="": "0xaa",
        mixhash_at=lambda blk, dry_value="": "0xaa",
        logs_all=lambda svc, dry_value="": MW_LOGS,
        logs_all_project=lambda dry_value="": "INFO all good",
        production=lambda e, a, dry_value=None: (99, 100),
        chainconfig_call=lambda sig, *a, **k: "1500",
        validator_status=lambda a, dry_value="": "2",
        sleep=lambda _s: None,
        dump_logs=lambda *a, **k: None,
    )
    world.update(over)
    return world


def _mw_ctx(monkeypatch, **over):
    return _live_ctx(monkeypatch, interval=vo.MIDWINDOW_EPOCH_INTERVAL,
                     activation=vo.MIDWINDOW_ACTIVATION_BLOCK, **_mw_world(**over))


def test_midwindow_passes_on_a_healthy_world(monkeypatch, capsys):
    fins = iter([200, 200, 400, 406])
    ctx, _ = _mw_ctx(monkeypatch, finalized_dec=lambda dry_value=0: next(fins, 406))
    asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    out = capsys.readouterr().out
    assert "RESUMED from journal and recovered its epoch-2 share" in out
    assert "kept PRODUCING, which a shareless node cannot do" in out
    assert "OK (smoke-vrf-dkg-restart-midwindow)" in out


def test_midwindow_FAILS_when_the_resume_line_NEVER_APPEARS(monkeypatch):
    """THE CORE NEGATIVE. Without the fix the restarted node never resumes; the whole-log grep for
    "share computed" would still be green (a fast host emits it before the restart), which is why
    the assertion is anchored on the resume line instead."""
    fins = iter([200, 200, 400, 406])
    ctx, _ = _mw_ctx(monkeypatch, finalized_dec=lambda dry_value=0: next(fins, 406),
                     logs_all=lambda svc, dry_value="": f"INFO {vo.SHARE_LINE} epoch=2\n")
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    assert "did NOT log a post-restart 'ceremony resumed from journal'" in e.value.message


def test_midwindow_FAILS_when_the_resume_STARTED_but_never_converged(monkeypatch):
    fins = iter([200, 200, 400, 406])
    ctx, _ = _mw_ctx(monkeypatch, finalized_dec=lambda dry_value=0: next(fins, 406),
                     logs_all=lambda svc, dry_value="": f"INFO {vo.RESUME_LINE} epoch=2\n")
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    assert "did not finalize" in e.value.message


def test_midwindow_is_NOT_fooled_by_an_ANSI_SPLIT_resume_line(monkeypatch):
    """§2.4 item 2 at the one place in this case where a missed match is a FALSE RED. `logs_all`
    strips before the body ever greps; this drives the un-stripped shape through the grep to show
    what would happen if a reader stopped stripping — a genuine resume reported as absent."""
    fins = iter([200, 200, 400, 406])
    raw = f"INFO {vo.RESUME_LINE} epoch\x1b[0m=\x1b[2m2\nINFO {vo.SHARE_LINE} epoch=2\n"
    ctx, _ = _mw_ctx(monkeypatch, finalized_dec=lambda dry_value=0: next(fins, 406),
                     logs_all=lambda svc, dry_value="": raw)
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    assert "did NOT log a post-restart" in e.value.message


def test_midwindow_FAILS_when_the_victim_prev_randao_DIVERGES(monkeypatch):
    """The share-holder assertion. A verify-only re-deriver also produces a mixHash; only a node
    holding the share produces the SAME one over every block of the window."""
    fins = iter([200, 200, 400, 406])
    ctx, _ = _mw_ctx(monkeypatch, finalized_dec=lambda dry_value=0: next(fins, 406),
                     mixhash_in=lambda svc, blk, dry_value="": "0xbb")
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    assert "prev_randao diverged" in e.value.message


def test_midwindow_FAILS_when_the_victim_is_BELOW_the_floor(monkeypatch):
    """THE LOAD-BEARING ASSERTION, driven with the negative control's readings: a shareless victim
    cannot produce a valid boundary proposal at all, so its epoch-2 credit stays 0.

    The participation FLOOR this used to assert went with the liveness jail; what survives is the
    property that actually mattered, and `> 0` is the one a shareless node cannot fake."""
    fins = iter([200, 200, 400, 406])
    ctx, _ = _mw_ctx(monkeypatch, finalized_dec=lambda dry_value=0: next(fins, 406),
                     production=lambda e, a, dry_value=None: (0, 100))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    assert "produced NOTHING in epoch 2" in e.value.message


def test_midwindow_FAILS_on_a_PERSISTENT_getter_failure_rather_than_reading_it_as_zero(monkeypatch):
    """A -2 read as 0 would be below every positive floor, i.e. it would FAIL a node that
    recovered perfectly — the sentinel discipline in its other direction."""
    fins = iter([200, 200, 400, 406])
    ctx, _ = _mw_ctx(monkeypatch, finalized_dec=lambda dry_value=0: next(fins, 406),
                     production=lambda e, a, dry_value=None: (-2, -2))
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    assert "-2 RPC sentinel" in e.value.message


def test_midwindow_FAILS_when_a_slash_event_names_the_victim(monkeypatch):
    fins = iter([200, 200, 400, 406])
    bare = DRY_ADDRS[3][2:]
    ctx, _ = _mw_ctx(monkeypatch, finalized_dec=lambda dry_value=0: next(fins, 406),
                     logs_all_project=lambda dry_value="": f"WARN ValidatorSlashed who={bare}")
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    assert "was slashed despite recovering" in e.value.message


def test_midwindow_FAILS_when_the_victim_is_JAILED(monkeypatch):
    fins = iter([200, 200, 400, 406])
    ctx, _ = _mw_ctx(monkeypatch, finalized_dec=lambda dry_value=0: next(fins, 406),
                     validator_status=lambda a, dry_value="": "3")
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    assert "is JAILED" in e.value.message


def test_midwindow_FAILS_LOUD_on_an_EMPTY_status_read(monkeypatch):
    """review [225]: an empty-vs-"3" false-green would hide the very liveness slash the case
    exists to catch."""
    fins = iter([200, 200, 400, 406])
    ctx, _ = _mw_ctx(monkeypatch, finalized_dec=lambda dry_value=0: next(fins, 406),
                     validator_status=lambda a, dry_value="": "")
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    assert "empty RPC result" in e.value.message


def test_midwindow_FAILS_with_the_WINDOW_MISSED_message_when_the_boundary_passed(monkeypatch):
    """The two poll outcomes are distinct failures. A chain already past the boundary means the
    window CLOSED — re-run — and reporting it as "the journal never appeared" would send an
    operator looking for a ceremony that ran perfectly."""
    ctx, _ = _mw_ctx(monkeypatch,
                     exec_test=lambda svc, snippet, dry_value=False: False,
                     finalized_dec=lambda dry_value=0: 999)
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    assert "the open window was missed" in e.value.message


def test_midwindow_FAILS_with_the_JOURNAL_message_when_the_ceremony_never_started(monkeypatch):
    ctx, _ = _mw_ctx(monkeypatch,
                     exec_test=lambda svc, snippet, dry_value=False: False,
                     finalized_dec=lambda dry_value=0: 10)
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    assert "DKG journal never appeared on disk" in e.value.message


def test_midwindow_does_NOT_restart_while_the_share_is_already_on_disk(monkeypatch):
    """The share-ABSENT half of the gate. A restart that lands post-finalize hits `maybe_start`'s
    store-hit early-return and never logs a resume — the original false-RED. So a world where BOTH
    files exist must never reach the restart."""
    ctx, runner = _mw_ctx(monkeypatch,
                          exec_test=lambda svc, snippet, dry_value=False: True,
                          finalized_dec=lambda dry_value=0: 10)
    with pytest.raises(SmokeFailure):
        asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    assert not any("restart" in " ".join(inv.argv) for inv in runner.log)


def test_midwindow_FAILS_when_the_chain_stops_finalizing_after_the_boundary(monkeypatch):
    ctx, _ = _mw_ctx(monkeypatch, finalized_dec=lambda dry_value=0: 400)
    with pytest.raises(SmokeFailure) as e:
        asserts_onchain.assert_vrf_dkg_restart_midwindow(ctx)
    assert "chain not finalizing after the boundary" in e.value.message
