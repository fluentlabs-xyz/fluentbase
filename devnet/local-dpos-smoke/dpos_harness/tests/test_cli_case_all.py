"""The gate aggregate: `python -m dpos_harness case all`.

Driven against FAKE cases — a stubbed `subprocess.run` returning scripted codes — because the real
ones each bring up a docker stack, and what is under test here is the accumulator, the banner and
the exit code, none of which care what a case actually did.

The two defects these tests exist for both produced a GREEN run over a broken one, which is the
class of lie the whole task removes: an interrupt that vanished into `ALL SMOKE CASES PASSED`, and
an INCONCLUSIVE folded into a pass.
"""

from __future__ import annotations

import types

import pytest

from dpos_harness import cli
from dpos_harness.cases.smoke import base, fault
from dpos_harness.core import proc
from dpos_harness.core.exit_codes import (RC_ERROR, RC_FAIL, RC_INCONCLUSIVE, RC_PASS,
                                          RC_USAGE)


def _run_all(monkeypatch, codes, names=None, dry=False, env=None):
    """Run `_case_all` over `codes` (case name -> rc, or a list matched positionally).

    Returns `(rc, stdout)`. Every `subprocess.run` is intercepted: a case invocation answers from
    `codes`, and anything else is recorded so a test can assert on the inter-case cleanup.
    """
    monkeypatch.setenv("SMOKE_CASES", " ".join(names) if names else "")
    for k, v in (env or {}).items():
        monkeypatch.setenv(k, v)
    issued = []

    def fake_stream(argv, **kw):
        issued.append(list(argv))
        return codes[argv[argv.index("case") + 1]]

    def fake_exec(self, argv, overlay, timeout, cwd, **kw):
        issued.append(list(argv))
        return proc.RunResult(argv=list(argv), rc=0)

    monkeypatch.setattr(proc, "run_streaming", fake_stream)
    monkeypatch.setattr(proc.Runner, "_exec", fake_exec)
    rc = cli._case_all(types.SimpleNamespace(dry_run=dry))
    return rc, issued


# ══ the SUITE's composition ════════════════════════════════════════════════

def test_the_suite_is_the_aggregates_and_never_their_constituents(monkeypatch):
    """THE EXCLUSION, PINNED AGAINST DRIFT. Nine registered cases are dropped from the suite
    because `smoke-base` and `smoke-fault` already run them — and not "the same body", the SAME
    FUNCTION OBJECT. Anything weaker than object identity would rot the first time an aggregate's
    list changed, and the cost of being wrong is a check the gate silently stops running.

    `smoke-vrf` additionally passes `honours_keep_up=True`; that is a teardown flag on the driver,
    not a difference in what is asserted."""
    aggregated = set(base.ASSERTIONS) | set(fault.ASSERTIONS)
    excluded = sorted(set(cli.CASES) - set(cli.SUITE))
    assert len(excluded) == 9

    import importlib
    for name in excluded:
        mod = importlib.import_module(f"dpos_harness.cases.{cli.CASES[name]}")
        seen = {}

        def spy(case, assertions, argv=None, **kw):
            seen["fns"] = list(assertions)
            return 0

        monkeypatch.setattr(mod.driver, "run", spy)
        mod.run_case([])
        assert seen["fns"], name
        assert set(seen["fns"]) <= aggregated, f"{name} runs something no aggregate does"


def test_every_suite_entry_is_a_registered_case():
    assert not [n for n in cli.SUITE if n not in cli.CASES]
    assert len(cli.SUITE) == 20


def test_the_production_path_substrate_runs_LAST():
    """The four prod cases need foundry and a contracts checkout and are the long ones; putting
    them first would spend the run's budget before anything cheap had a chance to go red."""
    prod = ["smoke-production-path", "smoke-vrf-rotation", "smoke-vrf-dkg-halt",
            "smoke-vrf-dkg-durability"]
    assert cli.SUITE[-len(prod):] == prod


# ══ the accumulator ════════════════════════════════════════════════════════

def test_all_green_passes(monkeypatch, capsys):
    rc, _ = _run_all(monkeypatch, {"smoke-tx": RC_PASS, "smoke-vrf": RC_PASS},
                     names=["smoke-tx", "smoke-vrf"])
    assert rc == RC_PASS
    assert "ALL SMOKE CASES PASSED" in capsys.readouterr().out


def test_a_failure_is_STICKY_across_later_green_cases(monkeypatch, capsys):
    """The suite reports the WORST outcome, not the last one."""
    rc, _ = _run_all(monkeypatch, {"smoke-tx": RC_PASS, "smoke-vrf": RC_FAIL,
                                   "smoke-epoch": RC_PASS},
                     names=["smoke-tx", "smoke-vrf", "smoke-epoch"])
    out = capsys.readouterr().out
    assert rc == RC_FAIL
    assert "SOME SMOKE CASES FAILED" in out and "ALL SMOKE CASES PASSED" not in out


def test_a_FAIL_outranks_an_ERROR_and_an_INCONCLUSIVE(monkeypatch):
    """Severity is an explicit ranking, not the code values: `RC_INCONCLUSIVE` is 4 and a `max()`
    over the raw numbers would put an unevaluated property above a real failure."""
    rc, _ = _run_all(monkeypatch, {"smoke-tx": RC_INCONCLUSIVE, "smoke-vrf": RC_ERROR,
                                   "smoke-epoch": RC_FAIL},
                     names=["smoke-tx", "smoke-vrf", "smoke-epoch"])
    assert rc == RC_FAIL
    assert cli._RC_RANK[RC_FAIL] > cli._RC_RANK[RC_ERROR] > cli._RC_RANK[RC_INCONCLUSIVE]


def test_an_ERROR_outranks_an_INCONCLUSIVE(monkeypatch):
    rc, _ = _run_all(monkeypatch, {"smoke-tx": RC_INCONCLUSIVE, "smoke-vrf": RC_ERROR},
                     names=["smoke-tx", "smoke-vrf"])
    assert rc == RC_ERROR


def test_a_LONE_INCONCLUSIVE_is_not_a_pass(monkeypatch, capsys):
    """THE SECOND DEFECT. A case returns INCONCLUSIVE exactly where a reading failed and the
    property was never evaluated — `seed-continuity` does it when the beacon metrics do not
    answer. Folding that into `RC_PASS` prints "ALL SMOKE CASES PASSED" with exit 0 over a table
    that says INCONCLUSIVE one line above, which is the same silent lie this suite removes."""
    rc, _ = _run_all(monkeypatch, {"smoke-tx": RC_PASS, "smoke-vrf": RC_INCONCLUSIVE},
                     names=["smoke-tx", "smoke-vrf"])
    out = capsys.readouterr().out
    assert rc == RC_INCONCLUSIVE and rc != 0
    assert "SOME SMOKE CASES COULD NOT BE JUDGED" in out
    assert "ALL SMOKE CASES PASSED" not in out and "FAILED" not in out


# ══ interruption ═══════════════════════════════════════════════════════════

def test_an_INTERRUPT_after_green_cases_is_not_a_pass(monkeypatch, capsys):
    """THE FIRST DEFECT. `break` sat between reading the code and folding it in, so the interrupt
    left `worst` at PASS and the run printed "ALL SMOKE CASES PASSED" with exit 0 while the table
    showed INTERRUPTED and a column of MISSING."""
    rc, _ = _run_all(monkeypatch, {"smoke-tx": RC_PASS, "smoke-vrf": 130, "smoke-epoch": RC_PASS},
                     names=["smoke-tx", "smoke-vrf", "smoke-epoch"])
    out = capsys.readouterr().out
    assert rc == 130
    assert "SMOKE RUN INTERRUPTED" in out
    assert "ALL SMOKE CASES PASSED" not in out


def test_an_INTERRUPT_does_not_MASK_an_earlier_failure(monkeypatch, capsys):
    """The other direction, and why 130 is a flag rather than a rank: a Ctrl-C must not overwrite a
    genuine red from a case that already ran."""
    rc, _ = _run_all(monkeypatch, {"smoke-tx": RC_FAIL, "smoke-vrf": 130, "smoke-epoch": RC_PASS},
                     names=["smoke-tx", "smoke-vrf", "smoke-epoch"])
    out = capsys.readouterr().out
    assert rc == RC_FAIL
    assert "SMOKE RUN INTERRUPTED" in out and "SOME SMOKE CASES FAILED" in out


def test_130_is_not_in_the_severity_ranking_at_all():
    """If it were, `.get(rc, ERROR)` would fold an interrupt in as an ERROR."""
    assert 130 not in cli._RC_RANK


def test_cases_after_an_INTERRUPT_are_reported_MISSING_not_assumed(monkeypatch, capsys):
    _run_all(monkeypatch, {"smoke-tx": RC_PASS, "smoke-vrf": 130, "smoke-epoch": RC_PASS},
             names=["smoke-tx", "smoke-vrf", "smoke-epoch"])
    out = capsys.readouterr().out
    assert "smoke-epoch" in out and "MISSING" in out
    assert "smoke-vrf" in out and "INTERRUPTED" in out


def test_the_run_STOPS_at_the_interrupt(monkeypatch):
    _, issued = _run_all(monkeypatch, {"smoke-tx": RC_PASS, "smoke-vrf": 130,
                                       "smoke-epoch": RC_PASS},
                         names=["smoke-tx", "smoke-vrf", "smoke-epoch"])
    assert not any("smoke-epoch" in a for a in issued)


# ══ the process boundary and the cleanup ═══════════════════════════════════

def test_each_case_runs_as_its_OWN_SUBPROCESS(monkeypatch):
    """Not caution. `core/nodes.py` binds `RPC` at IMPORT time (and `core/rpc.py`,
    `core/events.py`, `stack/sender.py` bind their timeouts the same way), so in one interpreter
    the first case to import them freezes those values for every later one — and
    `stack/bringup.py` leaves `DPOS_ACTIVATION_BLOCK` behind, which `StaticProfile` reads at CALL
    time. A process boundary closes both."""
    import sys
    _, issued = _run_all(monkeypatch, {"smoke-tx": RC_PASS}, names=["smoke-tx"])
    case_argv = next(a for a in issued if "case" in a)
    assert case_argv[:4] == [sys.executable, "-m", "dpos_harness", "case"]


def test_the_defensive_cleanup_runs_BETWEEN_cases_AND_COVERS_EVERY_PROJECT(monkeypatch):
    """There is no preflight `down -v` before a STATIC bring-up, so a leftover from the previous
    case would be inherited rather than reaped.

    PER PROJECT, and this is the half a live run found missing. Every compose ROOT pins its own
    `name:`, so the suite spans four docker projects — the static cases in `fluent-dpos-smoke`,
    the three sim cases in `fluent-dpos-sim`, the five production-path cases in
    `fluent-dpos-prod-path`. A bare `docker compose down` resolves `docker-compose.yml` and reaps
    only the first; `-v` and `--remove-orphans` widen what is removed WITHIN a project, never
    across projects. All three live roots publish host 8545/8546, so the miss is not cosmetic: it
    is the next case failing to bind its ports."""
    from dpos_harness.core import topology
    _, issued = _run_all(monkeypatch, {"smoke-tx": RC_PASS, "smoke-vrf": RC_PASS},
                         names=["smoke-tx", "smoke-vrf"])
    downs = [a for a in issued if a[:2] == ["docker", "compose"]]
    assert all("-v" in a and "--remove-orphans" in a for a in downs)
    assert len(downs) == 2 * len(topology.DOCKER_PROJECTS)
    for project in topology.DOCKER_PROJECTS:
        scoped = [a for a in downs if a[2:4] == ["-p", project]]
        assert len(scoped) == 2, f"{project} not cleaned after every case"


def test_a_DRY_aggregate_forwards_the_flag_and_issues_NO_docker(monkeypatch):
    """The cleanup is the one command the aggregate owns itself, so a dry run that still issued it
    would touch a real daemon while claiming to be a rehearsal."""
    _, issued = _run_all(monkeypatch, {"smoke-tx": RC_PASS, "smoke-vrf": RC_PASS},
                         names=["smoke-tx", "smoke-vrf"], dry=True)
    assert not [a for a in issued if a and a[0] == "docker"]
    assert all("--dry-run" in a for a in issued if "case" in a)


# ══ the override and the guards ════════════════════════════════════════════

def test_SMOKE_CASES_overrides_the_suite(monkeypatch):
    """bash carried the same override at `run-all.sh:21`."""
    _, issued = _run_all(monkeypatch, {"smoke-tx": RC_PASS}, names=["smoke-tx"])
    ran = [a[a.index("case") + 1] for a in issued if "case" in a]
    assert ran == ["smoke-tx"]


def test_an_unknown_case_in_the_override_is_a_USAGE_error_and_runs_NOTHING(monkeypatch):
    rc, issued = _run_all(monkeypatch, {}, names=["smoke-tx", "not-a-case"])
    assert rc == RC_USAGE
    assert not issued


def test_an_unrecognised_child_code_is_treated_as_an_ERROR(monkeypatch):
    """A case that died on a signal or a bare `sys.exit(7)` is infrastructure, not a verdict —
    and it must not be silently ranked below a real one."""
    rc, _ = _run_all(monkeypatch, {"smoke-tx": 7}, names=["smoke-tx"])
    assert rc == 7
    assert cli._RC_RANK.get(7, cli._RC_RANK[RC_ERROR]) == cli._RC_RANK[RC_ERROR]


@pytest.mark.parametrize("code,label", [(RC_PASS, "PASS"), (RC_FAIL, "FAIL"),
                                        (RC_USAGE, "USAGE"), (RC_ERROR, "ERROR"),
                                        (RC_INCONCLUSIVE, "INCONCLUSIVE"), (130, "INTERRUPTED")])
def test_every_code_has_a_name_in_the_table(code, label):
    assert cli._RC_NAME[code] == label
