"""`stack/profiles.py` + `stack/static_stack.py` — the static devnet, against `lib.sh`.

The static compose pair is what all 26 smoke cases run on, `bring_up_dpos` is called by 23 of
them and `tear_down` by every one, so this module is the base the whole case layer stands on.
There is no docker here, which means the COMMAND TRANSCRIPT is the only fidelity evidence
available — `test_bring_up_dpos_transcript_is_bash_faithful` is therefore the load-bearing test
in this file and every argv in it is quoted against its `lib.sh` line.

Four things beyond the transcript get their own tests because losing any of them is silent:

  * the extra overlay reaches the recreate and NOTHING else (23 cases inherit that);
  * the anchor is captured BEFORE the stop, and the epoch-0 window is enforced AFTER the flush
    retries (which are what can overshoot it);
  * `_read_dpos_nodes` drops the excluded validator but never the full-node;
  * teardown is best-effort in both halves and passes `--remove-orphans`.
"""

from __future__ import annotations

import pytest

from dpos_harness.core import converge, nodes, proc
from dpos_harness.core.proc import ProcError, Runner
from dpos_harness.stack import dataroot, profiles, static_stack
from dpos_harness.stack.profiles import GeneratedProfile, StaticProfile
from dpos_harness.stack.static_stack import StaticStack

VALS = ["validator-0", "validator-1", "validator-2", "validator-3"]


@pytest.fixture(autouse=True)
def _clean_env(monkeypatch):
    """The profile reads three env vars, and a leaked value from the caller's shell would make
    every expectation below depend on the operator's environment."""
    for k in ("EPOCH_INTERVAL", "DPOS_ACTIVATION_BLOCK", "DPOS_EXTRA_COMPOSE",
              "DPOS_CONVERGE_EXCLUDE", "SIM_DATA_ROOT"):
        monkeypatch.delenv(k, raising=False)


# ══ profiles ════════════════════════════════════════════════════════════════

def test_static_profile_compose_pair():
    """`lib.sh:469` — `-f docker-compose.yml -f docker-compose.dpos.yml`. Phase A is the base
    alone, because bash runs it as a BARE `docker compose up --build -d`."""
    p = StaticProfile()
    assert p.compose_files("base") == ("docker-compose.yml",)
    assert p.compose_files("dpos") == ("docker-compose.yml", "docker-compose.dpos.yml")
    assert p.compose_args("dpos") == ["-f", "docker-compose.yml",
                                      "-f", "docker-compose.dpos.yml"]


def test_static_profile_publishes_no_compose_file_env():
    """`lib.sh` never exports `COMPOSE_FILE`: phase A, the stops, the starts, the logs and the
    teardown are all BARE `docker compose` resolving `docker-compose.yml`, and only the recreate
    names its files. Returning a value here would silently change which project a hundred bare
    read commands resolve."""
    assert StaticProfile().compose_file_env("dpos") is None
    assert StaticProfile().compose_file_env("base") is None


def test_generated_profile_does_publish_one():
    """The other profile drives compose through the ambient env var — that asymmetry is the
    whole reason `compose_file_env` is a question a profile answers rather than a constant."""
    g = GeneratedProfile(6, 4, 8)
    assert g.compose_file_env("base") == "docker-compose.sim.gen.yml"
    assert g.compose_file_env("dpos") == ("docker-compose.sim.gen.yml:"
                                          "docker-compose.sim.dpos.gen.yml")


def test_bringup_derives_its_compose_constants_from_the_profile():
    """`stack/bringup.py`'s module constants are the generated profile's answer, not a second
    copy of the filenames. Asserted so the two cannot drift apart."""
    from dpos_harness.stack import bringup
    g = GeneratedProfile(1, 1, 1)
    assert bringup.BASE_COMPOSE == g.compose_file_env("base")
    assert bringup.DPOS_COMPOSE == g.compose_file_env("dpos")


def test_both_profiles_answer_the_same_interface():
    """The point of §3.4: a caller can hold either one. Checked structurally rather than by
    duck-typing at the call site, which is how the sim's `SimConfig` leak happened."""
    for p in (StaticProfile(), GeneratedProfile(6, 4, 8)):
        assert isinstance(p, profiles.StackProfile)
        assert p.compose_files("dpos") and p.compose_args("dpos") and p.committee()
        p.prepare(Runner(dry=True))          # must not raise; static is a no-op


def test_static_committee_is_the_four_compose_validators():
    """`lib.sh:35` — `VALS=(validator-0 … validator-3)`. Fixed by the checked-in compose file."""
    assert list(StaticProfile().committee()) == VALS


def test_extra_overlays_apply_to_dpos_only():
    """Faithfully to bash: phase A is bare, so an overlay is not in effect until the recreate."""
    p = StaticProfile(extra_overlays=("docker-compose.byzantine.yml",))
    assert p.compose_files("base") == ("docker-compose.yml",)
    assert p.compose_files("dpos")[-1] == "docker-compose.byzantine.yml"


@pytest.mark.parametrize("value,want", [
    ("-f docker-compose.byzantine.yml", ("docker-compose.byzantine.yml",)),
    ("-f a.yml -f b.yml", ("a.yml", "b.yml")),
    ("a.yml b.yml", ("a.yml", "b.yml")),          # bare names accepted too
    ("", ()),
    (None, ()),
])
def test_parse_extra_compose(value, want):
    """`case-byzantine.sh:14` exports the bash `-f file.yml` spelling. A Python caller has no
    reason to write the flags, and a silently-dropped overlay is the failure this one mechanism
    exists to prevent — so both spellings parse."""
    assert profiles.parse_extra_compose(value) == want


def test_from_env_reads_the_bash_export(monkeypatch):
    monkeypatch.setenv("DPOS_EXTRA_COMPOSE", "-f docker-compose.byzantine.yml")
    assert StaticProfile.from_env().extra_overlays == ("docker-compose.byzantine.yml",)


def test_explicit_overlays_beat_the_env(monkeypatch):
    monkeypatch.setenv("DPOS_EXTRA_COMPOSE", "-f from-env.yml")
    p = StaticProfile.from_env(extra_overlays=("explicit.yml",))
    assert p.extra_overlays == ("explicit.yml",)


# ── the epoch arithmetic (lib.sh:26-33) ──────────────────────────────────────

def test_default_interval_and_derived_activation():
    """`EPOCH_INTERVAL=32` (lib.sh:26 — NOT the sim's 64) and
    `DPOS_ACTIVATION_BLOCK=2*interval` (lib.sh:33)."""
    p = StaticProfile()
    assert (p.epoch_interval, p.activation_block) == (32, 64)


def test_a_tuned_interval_moves_the_activation_with_it(monkeypatch):
    """ONE derivation rule across compose, here and genesis-bootstrap: a case that raises the
    interval must not have to export the activation block separately, or the two drift and the
    anchor lands outside relative epoch 0."""
    monkeypatch.setenv("EPOCH_INTERVAL", "16")
    p = StaticProfile.from_env()
    assert (p.epoch_interval, p.activation_block) == (16, 32)


def test_an_explicit_activation_block_wins(monkeypatch):
    """`case-vrf-dkg-restart-midwindow.sh:62` pins both. The explicit value must not be
    re-derived from the interval."""
    monkeypatch.setenv("EPOCH_INTERVAL", "16")
    monkeypatch.setenv("DPOS_ACTIVATION_BLOCK", "128")
    p = StaticProfile.from_env()
    assert (p.epoch_interval, p.activation_block) == (16, 128)


def test_epoch_zero_window_is_half_open():
    """`[activation, activation+interval)`. Below the low bound `OriginEpocher` rejects the
    cold-start height; at or above the high bound the cold-start epoch would be >= 1 and re-hit
    the empty-marshal genesis lookup. Both are node behaviour, not margin."""
    assert StaticProfile().epoch_zero_window() == (64, 96)


# ══ the bring-up transcript — the fidelity oracle ══════════════════════════

def _dry_stack(**kw):
    runner = Runner(dry=True)
    return StaticStack(profile=StaticProfile(**kw), runner=runner), runner


def test_bring_up_dpos_transcript_is_bash_faithful():
    """ARGV FOR ARGV against `lib.sh`, in order. This is the only fidelity evidence there is
    without docker, so it carries the weight of the whole chunk.

        lib.sh:493  docker compose up --build -d
        lib.sh:436  docker compose stop --timeout 40 "${VALS[@]}"
        lib.sh:469  docker compose -f docker-compose.yml -f docker-compose.dpos.yml \
                        ${DPOS_EXTRA_COMPOSE:-} up -d --force-recreate "${VALS[@]}"

    Three commands, because that IS the happy path: the activation wait, the flush barrier and
    both converges are READS, and `tear_down` is the case's own `trap`, not part of bring-up.
    """
    stack, runner = _dry_stack()
    stack.bring_up_dpos()
    assert runner.argvs() == [
        ["docker", "compose", "up", "--build", "-d"],
        ["docker", "compose", "stop", "--timeout", "40", *VALS],
        ["docker", "compose", "-f", "docker-compose.yml", "-f", "docker-compose.dpos.yml",
         "up", "-d", "--force-recreate", *VALS],
    ]


def test_the_overlay_lands_in_the_recreate_and_nowhere_else():
    """`_migrate_to_dpos` is the SINGLE honouring point of the per-case overlay, and 23 cases
    inherit it through `bring_up_dpos` without knowing it exists. If it leaked into phase A the
    overlay's services would boot before DPoS; if it were missing from the recreate,
    `case-byzantine` would silently run an honest validator-3 and pass."""
    stack, runner = _dry_stack(extra_overlays=("docker-compose.byzantine.yml",))
    stack.bring_up_dpos()
    argvs = runner.argvs()
    assert argvs[0] == ["docker", "compose", "up", "--build", "-d"]     # phase A stays bare
    assert argvs[-1] == [
        "docker", "compose", "-f", "docker-compose.yml", "-f", "docker-compose.dpos.yml",
        "-f", "docker-compose.byzantine.yml", "up", "-d", "--force-recreate", *VALS]
    assert sum("docker-compose.byzantine.yml" in a for a in argvs) == 1


def test_bring_up_never_exports_compose_file(monkeypatch):
    """The static stack must leave `COMPOSE_FILE` alone — every bare read in `core/rpc.py`
    inherits it, and setting it here would repoint them at a different project than `lib.sh`
    does. (`bringup.BringUp._set_compose` deliberately DOES set it, for the other profile.)"""
    import os
    stack, runner = _dry_stack()
    stack.bring_up_dpos()
    assert "COMPOSE_FILE" not in os.environ
    assert "COMPOSE_FILE" not in runner.env


def test_migrate_returns_and_stores_the_anchor():
    """`PREV_FIN` is the hex height half of the anchor reading and is the floor
    `wait_dpos_converge` polls against."""
    stack, _ = _dry_stack()
    assert stack.prev_fin == "null"          # lib.sh:408 — before any migration
    assert stack.migrate_to_dpos() == "0x0"  # dry anchor
    assert stack.prev_fin == "0x0"


# ══ the flush gate ═════════════════════════════════════════════════════════

def _live_stack(monkeypatch, profile=None):
    """A StaticStack on its LIVE read/converge branches, with a Runner that records writes and
    executes nothing.

    The two halves are separated deliberately: the Runner stays dry (so every write is
    transcribed and no docker is spawned) while `_dry()` is forced False (so the reads, the
    barriers and the fail-loud paths take their real branches). Flipping `p.dry` instead would
    make the recorder spawn processes, which is the one thing no unit test may do."""
    runner = Runner(dry=True)
    stack = StaticStack(profile=profile or StaticProfile(), runner=runner)
    monkeypatch.setattr(runner, "_exec",
                        lambda *a, **k: pytest.fail("the recorder executed a command"))
    monkeypatch.setattr(stack, "_dry", lambda: False)
    monkeypatch.setattr(static_stack.converge.time, "sleep", lambda _s: None)
    return stack, runner


def test_flush_gate_captures_the_anchor_before_each_stop(monkeypatch):
    """Ordering that matters: a flush-retry brings the sequencer back UP and the chain advances,
    so the anchor must be the head of the attempt that actually stopped cleanly. Reading it once
    before the loop would pin a stale anchor that is no longer on disk."""
    stack, runner = _live_stack(monkeypatch)
    heads = iter(["0x40|0xaa", "0x50|0xbb"])
    order = []
    monkeypatch.setattr(nodes, "check_external",
                        lambda port: order.append("read") or next(heads))
    flushed = iter([False, True])            # attempt 1 fails the barrier, attempt 2 succeeds
    monkeypatch.setattr(stack, "shutdown_flushed",
                        lambda v: order.append(f"flush:{v}") or next(flushed)
                        if v == "validator-0" else True)
    monkeypatch.setattr(stack, "wait_converge", lambda *a, **k: order.append("converge"))
    monkeypatch.setattr(stack, "_wait_activation", lambda: None)
    monkeypatch.setattr(stack, "_assert_anchor_in_epoch_zero", lambda: None)
    stack.migrate_to_dpos()
    assert stack.prev_fin == "0x50", "the anchor was not re-read for the retry"
    assert order.index("read") < order.index("flush:validator-0")
    assert "converge" in order, "the flush retry did not reconverge the sequencer"
    starts = [a for a in runner.argvs() if a[:3] == ["docker", "compose", "start"]]
    assert starts == [["docker", "compose", "start", *VALS]]     # lib.sh:446


def test_flush_gate_fails_loud_after_four_attempts(monkeypatch):
    """`lib.sh:434,449-451` — four attempts, then a fail-loud that TEARS DOWN. Without the
    teardown the raise would leak a running devnet, because Python has no `trap … EXIT`."""
    stack, _ = _live_stack(monkeypatch)
    monkeypatch.setattr(nodes, "check_external", lambda port: "0x40|0xaa")
    monkeypatch.setattr(stack, "shutdown_flushed", lambda v: False)
    monkeypatch.setattr(stack, "wait_converge", lambda *a, **k: True)
    torn = []
    monkeypatch.setattr(stack, "tear_down", lambda: torn.append(1))
    with pytest.raises(converge.ConvergeError) as ei:
        stack._flush_gate()
    assert ei.value.reason_id == "flush-gate"
    assert torn == [1], "a failed flush gate left the stack running"


def test_flush_gate_stops_all_four_in_one_command(monkeypatch):
    """`lib.sh:436` stops the whole committee in a single `docker compose stop`. Stopping them
    one at a time would let the still-running nodes finalize past the anchor the stopped ones
    just persisted."""
    stack, runner = _live_stack(monkeypatch)
    monkeypatch.setattr(nodes, "check_external", lambda port: "0x40|0xaa")
    monkeypatch.setattr(stack, "shutdown_flushed", lambda v: True)
    stack._flush_gate()
    stops = [a for a in runner.argvs() if a[:3] == ["docker", "compose", "stop"]]
    assert stops == [["docker", "compose", "stop", "--timeout", "40", *VALS]]


# ══ the epoch-0 anchor window ══════════════════════════════════════════════

@pytest.mark.parametrize("anchor_hex,ok", [
    ("0x40", True),    # 64 == activation, the inclusive low bound
    ("0x5f", True),    # 95, the last block of relative epoch 0
    ("0x3f", False),   # 63, below activation: OriginEpocher rejects the cold-start height
    ("0x60", False),   # 96 == activation+interval: cold-start epoch >= 1
])
def test_anchor_window_is_enforced_after_the_flush(monkeypatch, anchor_hex, ok):
    """The upper bound is checked AFTER the flush retries because they are what overshoots it: a
    retry brings the sequencer back up and it keeps finalizing. Rejecting here turns an opaque
    in-node abort into an actionable harness message."""
    stack, _ = _live_stack(monkeypatch)
    stack.prev_fin = anchor_hex
    torn = []
    monkeypatch.setattr(stack, "tear_down", lambda: torn.append(1))
    if ok:
        stack._assert_anchor_in_epoch_zero()
        assert torn == []
    else:
        with pytest.raises(converge.ConvergeError) as ei:
            stack._assert_anchor_in_epoch_zero()
        assert ei.value.reason_id == "anchor-window"
        assert "EPOCH_INTERVAL" in ei.value.message, "the message must be actionable"
        assert torn == [1]


# ══ the converge readers ═══════════════════════════════════════════════════

@pytest.fixture
def _labelled_reads(monkeypatch):
    monkeypatch.setattr(nodes, "check_external", lambda port: f"0x5|0xh@{port}")
    monkeypatch.setattr(nodes, "check_node", lambda svc: f"0x5|0xh@{svc}")


def test_read_sequencer_nodes_shape(_labelled_reads):
    """`lib.sh:167-173`: validator-0 over host 8545, validator-1..3 by `compose exec`, the
    full-node over host 18545 — in that order. The host-vs-exec split is a topology fact
    (`HOST_RPC_PORTS`), not a preference."""
    stack, _ = _dry_stack()
    assert [lbl for lbl, _ in stack.read_sequencer_nodes()] == [
        "validator-0@8545", "validator-1", "validator-2", "validator-3", "full-node@18545"]


def test_read_dpos_nodes_drops_only_the_excluded_validator(_labelled_reads):
    """`lib.sh:475-482`. `case-byzantine` excludes validator-3, whose reth never finalizes by
    design; requiring all five would fail a case behaving exactly as intended."""
    stack = StaticStack(profile=StaticProfile(), runner=Runner(dry=True),
                        converge_exclude="validator-3")
    assert [lbl for lbl, _ in stack.read_dpos_nodes()] == [
        "validator-0@8545", "validator-1", "validator-2", "full-node@18545"]


def test_read_dpos_nodes_can_exclude_the_pinned_host(_labelled_reads):
    stack = StaticStack(profile=StaticProfile(), runner=Runner(dry=True),
                        converge_exclude="validator-0")
    assert [lbl for lbl, _ in stack.read_dpos_nodes()] == [
        "validator-1", "validator-2", "validator-3", "full-node@18545"]


def test_the_full_node_is_never_excluded(_labelled_reads):
    """It follows the honest quorum's certs, so it is the check that the cascade tier saw the
    same chain — dropping it would leave the assertion committee-only."""
    stack = StaticStack(profile=StaticProfile(), runner=Runner(dry=True),
                        converge_exclude="full-node")
    assert ("full-node@18545", "0x5|0xh@18545") in stack.read_dpos_nodes()


def test_converge_exclude_defaults_from_the_bash_export(monkeypatch):
    monkeypatch.setenv("DPOS_CONVERGE_EXCLUDE", "validator-2")
    assert StaticStack(runner=Runner(dry=True)).converge_exclude == "validator-2"


def test_wait_dpos_converge_uses_the_anchor_as_its_floor(monkeypatch):
    """The post-swap gate must require finalized STRICTLY PAST the anchor — an aligned head AT
    the anchor is the pre-swap chain, and accepting it would pass a migration that never
    happened."""
    stack, _ = _live_stack(monkeypatch)
    stack.prev_fin = "0x40"
    seen = []
    monkeypatch.setattr(static_stack.converge, "wait_aligned",
                        lambda t, floor, reader, **k: (seen.append((t, floor)), ("x", []))[1])
    stack.wait_dpos_converge()
    assert seen == [(static_stack.DPOS_CONVERGE_S, "0x40")]


def test_a_failed_converge_tears_down_and_raises(monkeypatch):
    """bash `echo FAIL; docker compose logs --tail=N; tear_down; exit 1`. The `exit` fired the
    case's `trap tear_down EXIT`; a Python `raise` only unwinds, so the teardown is explicit."""
    stack, _ = _live_stack(monkeypatch)
    monkeypatch.setattr(static_stack.converge, "wait_aligned",
                        lambda *a, **k: (None, [("validator-3", "null|null")]))
    calls = []
    monkeypatch.setattr(stack, "dump_logs", lambda tail, *s: calls.append(("logs", tail)))
    monkeypatch.setattr(stack, "tear_down", lambda: calls.append(("teardown",)))
    with pytest.raises(converge.ConvergeError):
        stack.wait_converge()
    assert calls == [("logs", static_stack.LOG_TAIL_SEQUENCER), ("teardown",)]


def test_the_post_swap_failure_dumps_more_history(monkeypatch):
    """`lib.sh:496` uses `--tail=200` where the phase-A failure uses 120. Flattening them would
    quietly shorten the diagnostic on the harder failure."""
    stack, _ = _live_stack(monkeypatch)
    stack.prev_fin = "0x40"
    monkeypatch.setattr(static_stack.converge, "wait_aligned", lambda *a, **k: (None, []))
    tails = []
    monkeypatch.setattr(stack, "dump_logs", lambda tail, *s: tails.append(tail))
    monkeypatch.setattr(stack, "tear_down", lambda: None)
    with pytest.raises(converge.ConvergeError):
        stack.wait_dpos_converge()
    assert tails == [static_stack.LOG_TAIL_DPOS]


# ══ the log dump helper ═══════════════════════════════════════════════════

def test_dump_logs_prints_the_captured_output(monkeypatch, capsys):
    """`dump_logs` must SHOW what it captured, matching `SmokeCtx.overlay_dump_logs`
    (cases/smoke/driver.py) — a dump that runs `docker compose logs` and then discards the
    output via `run_ok` looks like a diagnostic on a fail path and is not one; the operator
    would see nothing where bash's `docker compose logs --tail=N` writes straight to the
    terminal. Asserting only that the argv was issued would not catch that regression, so this
    checks the captured text actually reaches stdout."""
    stack, runner = _live_stack(monkeypatch)
    runner.reads["docker compose logs"] = "line one from validator-0\nline two"
    stack.dump_logs(120, "validator-0")
    out = capsys.readouterr().out
    assert "line one from validator-0" in out
    assert "line two" in out


def test_dump_logs_is_a_noop_under_dry(monkeypatch, capsys):
    """The dry-run transcript is about the WRITE commands; a dump under dry must neither issue
    the command nor print anything, so it does not pollute `--dry-run-bringup` output."""
    stack, runner = _dry_stack()
    stack.dump_logs(120, "validator-0")
    assert capsys.readouterr().out == ""
    assert runner.argvs() == []


# ══ the activation wait ════════════════════════════════════════════════════

def test_activation_wait_polls_to_the_activation_block(monkeypatch):
    stack, _ = _live_stack(monkeypatch)
    seen = []
    monkeypatch.setattr(static_stack.converge, "wait_finalized_ge",
                        lambda target, timeout: seen.append((target, timeout)) or True)
    stack._wait_activation()
    assert seen == [(64, static_stack.ACTIVATION_WAIT_S)]


def test_activation_wait_fails_loud_and_tears_down(monkeypatch):
    """`lib.sh:428-431`. A sequencer that never reaches the activation block cannot produce a
    valid anchor, so continuing would cold-start every node into a fail-loud."""
    stack, _ = _live_stack(monkeypatch)
    monkeypatch.setattr(static_stack.converge, "wait_finalized_ge", lambda *a, **k: False)
    torn = []
    monkeypatch.setattr(stack, "tear_down", lambda: torn.append(1))
    monkeypatch.setattr(stack, "dump_logs", lambda *a: None)
    with pytest.raises(converge.ConvergeError) as ei:
        stack._wait_activation()
    assert ei.value.reason_id == "activation-wait" and torn == [1]


# ══ teardown ═══════════════════════════════════════════════════════════════

def test_tear_down_argv_and_remove_orphans():
    """`lib.sh:500`. `--remove-orphans` is load-bearing, not hygiene: the `down` is BARE, so
    every container a per-case overlay added (cert-follower, sentry, the mitm proxy) is an orphan
    of this project. Without the flag they survive and the next case inherits them."""
    stack, runner = _dry_stack()
    stack.tear_down()
    assert runner.argvs()[0] == ["docker", "compose", "down", "-v", "--remove-orphans"]


def test_tear_down_is_best_effort_in_both_halves(monkeypatch):
    """bash `2>/dev/null || true` and `soak_data_root_wipe || true`. A teardown must never turn
    a passing case into a failing one — so a dead daemon and a failed wipe both stay silent."""
    runner = Runner(dry=True)
    stack = StaticStack(profile=StaticProfile(), runner=runner)
    stack.p.dry = False
    monkeypatch.setattr(runner, "run_ok", lambda *a, **k: False)
    monkeypatch.setattr(dataroot, "data_root_wipe", lambda r: False)
    stack.tear_down()                       # no raise


def test_tear_down_wipes_the_data_root(monkeypatch):
    """`down -v` drops named-volume metadata but never a BIND mount's host contents, so a
    relocated `/runtime` survives teardown and the next run inherits a populated reth datadir —
    which shifts owner-0's deploy nonce and drifts the staking-address prediction (v54)."""
    stack, _ = _dry_stack()
    seen = []
    monkeypatch.setattr(dataroot, "data_root_wipe", lambda r: seen.append(r) or True)
    stack.tear_down()
    assert seen == [stack.p]


# ══ the data-root wipes ════════════════════════════════════════════════════

def test_data_root_wipe_is_a_noop_when_unset():
    runner = Runner(dry=True)
    assert dataroot.data_root_wipe(runner) is True
    assert runner.argvs() == [], "an unset data root issued a docker command"


@pytest.mark.parametrize("root", ["relative/path", "", "  "])
def test_data_root_wipe_refuses_a_non_absolute_root(monkeypatch, root):
    """The foot-gun guard: this runs `rm -rf` as root inside a container, so it must never be
    pointed at a path that resolves somewhere surprising."""
    monkeypatch.setenv("SIM_DATA_ROOT", root)
    runner = Runner(dry=True)
    assert dataroot.data_root_wipe(runner) is True
    assert runner.argvs() == []


def test_data_root_wipe_argv_verifies_in_the_same_container(monkeypatch):
    """The verify must run in the SAME container as the removal — a partially-failed wipe would
    otherwise report success, and a docker-run failure (daemon down, image pull) has to surface
    as a nonzero rc rather than a false "wiped"."""
    monkeypatch.setenv("SIM_DATA_ROOT", "/mnt/fast/dpos/")
    runner = Runner(dry=True)
    dataroot.data_root_wipe(runner)
    argv = runner.argvs()[0]
    assert argv[:6] == ["docker", "run", "--rm", "-v", "/mnt/fast/dpos/runtime:/wipe", "busybox"]
    assert "rm -rf /wipe/*" in argv[-1] and "exit 3" in argv[-1]


def test_bringup_and_teardown_issue_the_same_wipe(monkeypatch):
    """One definition, two consumers. The generated bring-up's preflight and the static
    teardown must not drift into two differently-guarded `rm -rf`s."""
    from dpos_harness.stack.bringup import BringUp
    from dpos_harness.sim.orchestrator import SimConfig
    monkeypatch.setenv("SIM_DATA_ROOT", "/mnt/fast/dpos")
    r1, r2 = Runner(dry=True), Runner(dry=True)
    BringUp(SimConfig(validators=4, initial_committee=3).stack_spec(), r1)._data_root_wipe()
    StaticStack(profile=StaticProfile(), runner=r2).tear_down()
    wipes = [a for a in r2.argvs() if a[:2] == ["docker", "run"]]
    assert r1.argvs() == wipes


@pytest.mark.parametrize("root,fires", [
    ("/dev/shm/dpos-sim", True),
    ("/dev/shm", False),          # the tmpfs mount point itself
    ("/dev/shm/", False),         # trailing slash, no component past it
    ("/mnt/fast/dpos", False),    # disk-backed: the container wipe handles it
    ("/", False),
    ("/dev/shm/../etc", False),   # `..`-traversing
    ("", False),
])
def test_ram_root_purge_hard_guard(monkeypatch, root, fires):
    """A HOST-side `rm -rf`, deliberately not the container wipe: under the memory exhaustion
    that triggers this, `docker run busybox` may fail to start. So the guard has to be tight —
    strictly under `/dev/shm/` with a non-empty component past it, and nothing else."""
    removed = []
    monkeypatch.setattr(dataroot.shutil, "rmtree",
                        lambda p, ignore_errors=False: removed.append(p))
    assert dataroot.ram_root_purge(root) is fires
    assert removed == ([root.rstrip("/")] if fires else [])


# ══ the exec seam ══════════════════════════════════════════════════════════

def test_the_barrier_reads_use_the_ambient_channel(monkeypatch):
    """A flush barrier polls `docker inspect` up to ten times per node. Those are READS and must
    stay off the write transcript — `--dry-run-static` is a fixed, diffable command sequence and
    a poll count that varies with daemon load would destroy it."""
    reads = []
    monkeypatch.setattr(proc, "read", lambda argv, **k: reads.append(argv) or "")
    stack = StaticStack(profile=StaticProfile(), runner=Runner(dry=True))
    stack.p.dry = False
    assert stack.shutdown_flushed("validator-1") is False
    assert reads == [["docker", "compose", "ps", "-aq", "validator-1"]]
    assert stack.p.argvs() == [], "a barrier read entered the write transcript"


def test_a_failed_lifecycle_command_raises(monkeypatch):
    """`docker compose up --build -d` runs under `set -e` in bash, so a failure ABORTS the case.
    `run_checked` is the translation — a swallowed False here would let a case run its
    assertions against a stack that never came up."""
    runner = Runner(dry=True)
    stack = StaticStack(profile=StaticProfile(), runner=runner)
    stack.p.dry = False
    monkeypatch.setattr(runner, "_exec",
                        lambda *a, **k: pytest.fail("should not reach exec"))
    monkeypatch.setattr(runner, "run_capture",
                        lambda argv, **k: proc.RunResult(argv=list(argv), rc=1,
                                                         stderr="no such image"))
    with pytest.raises(ProcError):
        stack.bring_up_dpos()
