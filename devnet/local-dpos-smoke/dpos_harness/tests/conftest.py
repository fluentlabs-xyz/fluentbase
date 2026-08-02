"""conftest.py — hermeticity guard for the dpos_harness unit suite.

Why this exists (v61 a13, 2026-07-22): running `pytest dpos_harness/tests/` from the smoke directory
TORE DOWN a live production sim stack. Two defects combined:

  (1) COMPOSE_FILE leak. `BringUp._set_compose` (bringup.py:167) writes os.environ["COMPOSE_FILE"]
      = <sim compose> BY DESIGN — real runs need the process-global value so the bare
      `docker compose exec` READ helpers (rpc.py / nodes.py, which inherit only os.environ, no
      Runner env) resolve the right project. Unit tests never isolated os.environ, so an unguarded
      dry `BringUp.run()` leaked the sim COMPOSE_FILE across test files. monkeypatch.setenv guards
      don't help: they snapshot the ALREADY-leaked value and "restore" that, never unsetting it.

  (2) Real teardown `down`. test_launch_gate's `orch.run()` loop tests build a LIVE Runner
      (Orchestrator._runner defaults dry=False) whose clean-exit teardown shells
      `docker compose down -v --remove-orphans` (orchestrator.py:843), inheriting the leaked
      COMPOSE_FILE → destroys project `fluent-dpos-sim`.

This autouse fixture makes the suite PHYSICALLY unable to exec real `docker` (always) or any
network/broadcast `cast`/`forge` (`--rpc-url`, `--broadcast`, `cast send|rpc|publish`), and severs
the COMPOSE_FILE leak at the test boundary. Pure-local `cast`/`forge` encoding stays real. It
touches NO production code (the os.environ write is correct for live runs) and weakens NO assertion:
  - dry-Runner `.log` oracles short-circuit before the exec — untouched;
  - the live-Runner seam tests run `["false"]` / `["sleep","2"]`, which are NOT blocked and still
    really execute (guarded shim delegates non-docker argv to the real subprocess.run);
  - the loop-test teardown `down` is recorded-not-run, and those tests assert on battery/rounds,
    never on the `down` argv.

If a future test legitimately needs a REAL docker/cast/forge, it must opt out explicitly (e.g. an
`@pytest.mark.real_exec` that this fixture would honor) — none does today.
"""

from __future__ import annotations

import os
import subprocess
from unittest.mock import create_autospec

import pytest

_NETWORK_FLAGS = ("--rpc-url", "--broadcast")           # any arg touching a live chain / broadcasting
_CAST_NETWORK_SUBCMDS = {"send", "rpc", "publish"}      # cast subcommands that hit an RPC / submit tx


def _should_block(argv) -> bool:
    """Block argv that would touch LIVE infrastructure from the unit suite:
      - docker            : ALWAYS (the v61-a13 sim-killer; `docker compose down` etc. — never
                            legitimately real in a unit test).
      - cast / forge      : only when they reach the network or broadcast — a `--rpc-url`/`--broadcast`
                            arg, or `cast send|rpc|publish`. Pure-local encoding (`cast calldata`,
                            `cast abi-encode`, `cast keccak`, local `forge`) is SAFE and some tests
                            (e.g. test_lib_write cast-calldata roundtrip) need the real binary.
    """
    try:
        argv = [str(a) for a in argv]
        binary = os.path.basename(argv[0]) if argv else ""
    except (TypeError, IndexError):
        return False
    if binary == "docker":
        return True
    if binary in ("cast", "forge"):
        if any(f in argv for f in _NETWORK_FLAGS):
            return True
        if binary == "cast" and len(argv) > 1 and argv[1] in _CAST_NETWORK_SUBCMDS:
            return True
    return False


def _guarded_run(real_run):
    """Wrap subprocess.run: record-and-block live-infra argv (return a benign rc-0 CompletedProcess
    WITHOUT executing); delegate everything else to the real subprocess.run so coreutils seam tests
    (`false`, `sleep`) and pure-local `cast`/`forge` encoding keep their real semantics."""
    def run(*args, **kwargs):
        argv = args[0] if args else kwargs.get("args")
        if _should_block(argv):
            return subprocess.CompletedProcess(args=argv, returncode=0, stdout="", stderr="")
        return real_run(*args, **kwargs)
    return run


def strict_actuators():
    """The ONE actuator double for the whole suite: `create_autospec(Actuators, instance=True)`.

    Why not a `__getattr__` catch-all (what every actuator double here used to be): a catch-all
    manufactures ANY name asked of it, so 482 green tests coexisted with three names the production
    code called and no object defined — `act_byzantine_restore` (missing outright) plus
    `sim_delegate_shift` / `sim_voluntary_exit` (methods of `Chain`, reached for on `Actuators`).
    The doubles answered all three and the harness silently no-op'd for its entire life.

    An autospec answers ONLY the real surface — `dir(Actuators)`, private helpers `_compose` /
    `_run_ok` included, which `dispatch.py:331,539` need — and raises `AttributeError` on anything
    else. A wiring gap or a rename now fails at the call site, in every test that touches it.

    Consequence for assertions: there is no hand-rolled `.calls` list any more, and `_compose`
    returns a `MagicMock` rather than an argv list, so `self._run_ok(self._compose(...))` no longer
    yields inspectable argv. Assert against the mock's own recorder instead —
    `act.act_byzantine.assert_any_call(...)`, `act.method_calls`, `act._compose.call_args`.
    """
    from dpos_harness.sim import actions
    return create_autospec(actions.Actuators, instance=True)


@pytest.fixture(autouse=True)
def _hermetic_no_real_docker(monkeypatch):
    # (A) block real docker/cast/forge at the shared exec seam. `dpos_harness.core.proc` is now the
    #     ONLY module in the package that imports subprocess (tests/test_exec_seam.py enforces it by
    #     AST), and every spawn in it funnels through `Runner._exec`, so this one module-level patch
    #     covers every exec site there is: Runner.run / run_capture / run_ok / run_checked / read /
    #     pipe_to_exec, the module-level proc.read / read_capture used by every observer, and the
    #     actuator + spammer runners. Coverage got WIDER, not narrower, with that consolidation:
    #     `sim.status` used to clear the screen with `os.system("clear")`, an exec this shim never
    #     saw; it is now an ANSI write with no process at all (core/termio.clear_screen).
    monkeypatch.setattr(subprocess, "run", _guarded_run(subprocess.run))
    # (B) sever the cross-file COMPOSE_FILE leak: snapshot before, restore-or-delete after. The
    #     production write at bringup.py:167 is intentional and stays; we isolate it per test.
    had = "COMPOSE_FILE" in os.environ
    prev = os.environ.get("COMPOSE_FILE")
    try:
        yield
    finally:
        if had:
            os.environ["COMPOSE_FILE"] = prev
        else:
            os.environ.pop("COMPOSE_FILE", None)
