"""The SAFETY SWEEP — `ChainBattery.check_safety_subset` and its wiring into `driver.run`.

THE DEFECT THIS CLOSES. `checks/battery.py` holds ~30 invariant detectors and not one of the
twenty cases in `cli.SUITE` ever built one. Its only callers were `sim/orchestrator.py` (the soak,
which is outside the suite) and `sim/shadow.py`, which writes a log line and returns 0
unconditionally. So a chain that finalized two different blocks under a gate case went green as
long as that case's own assertions held.

TWO KINDS OF EVIDENCE, and the second is the one that matters most here:

  * THE SUBSET decides correctly — an armed id fails, an unarmed one is reported and stepped over,
    and a stand that cannot answer produces no verdict at all;
  * THE WIRING — a failing subset reaches `RC_FAIL` through `driver.run`, and every path that must
    NOT be able to redden a case (dry run, kill switch, a read-side exception, an unreadable
    address file) really cannot.

The second set exists because the first cannot see it: a detector that returns False into a caller
that ignores it is exactly the shape of the defect being fixed.
"""

from __future__ import annotations

import pytest

from dpos_harness.cases.smoke import driver
from dpos_harness.checks import battery as B
from dpos_harness.checks.battery import ChainBattery, Ctx
from dpos_harness.core.exit_codes import RC_FAIL, RC_PASS
from dpos_harness.stack.static_stack import StaticStack

ADDRS = ["0xa1", "0xb2", "0xc3", "0xd4"]
FIN = 100


def _bat(**seams):
    """A 4-node committee whose every read AGREES, with the seams named by `seams` overridden.

    Nothing here touches a daemon: every impure seam of the battery is replaced, which is the same
    discipline `test_battery.py` uses. `SIM_FIN_WALK_MAX` matches the driver's sweep window so the
    walk covers the same heights the real sweep would."""
    ctx = Ctx(SIM_TICK=0, SIM_ROUND=0, SIM_CUR_F=1,
              SIM_CUR_COMMITTEE=" ".join(ADDRS),
              ADDR2IDX={a: f"validator-{i}" for i, a in enumerate(ADDRS)},
              SIM_NO_CASCADE=1)
    bat = ChainBattery(ctx=ctx)
    bat.SIM_FIN_WALK_MAX = driver.SAFETY_SWEEP_FIN_WALK
    bat._finalized_dec = lambda: FIN
    bat._node_metrics = lambda svc: ""            # no metrics -> safety-halt has nothing to judge
    bat._node_fin_in = lambda svc: FIN
    bat._fork_sleep = lambda: None
    bat._mixhash_of = lambda svc, b: f"0x{b:064x}"          # distinct per height, equal per node
    bat._blockhash_of = lambda svc, b: f"0xbb{b:062x}"
    bat._node_roots_of = lambda svc, h: f"0x{h:064x} 0x{h:064x}"
    for name, fn in seams.items():
        setattr(bat, name, fn)
    return bat


# ══ the subset decides ═════════════════════════════════════════════════════

def test_a_healthy_stand_holds():
    bat = _bat()
    assert bat.check_safety_subset() is True
    assert bat.inv_fail_id == ""


def test_result_divergence_FAILS():
    """The two-sided safety witness the whole sweep exists for: one node's executed result at
    fin−K disagrees and stays disagreeing across the re-read."""
    bat = _bat(_node_roots_of=lambda svc, h: ("0xbad 0xbad" if svc == "validator-3"
                                              else f"0x{h:064x} 0x{h:064x}"))
    assert bat.check_safety_subset() is False
    assert bat.inv_fail_id == "result-divergence"
    assert "validator-3" in bat.inv_fail_msg


def test_fork_detected_FAILS():
    """A finalized-hash disagreement inside the beacon window, persisting across the re-read."""
    bat = _bat(_blockhash_of=lambda svc, b: ("0xdead" if svc == "validator-3"
                                             else f"0xbb{b:062x}"))
    assert bat.check_safety_subset() is False
    assert bat.inv_fail_id == "fork-detected"


def test_double_finalization_FAILS():
    """The same disagreement placed BELOW the beacon window ([fin−6, fin−1] = [94, 99]) so the
    fork detector cannot claim it first — this is the finalized-chain walk's own verdict, and it
    is only reachable because the sweep walks `SAFETY_SWEEP_FIN_WALK` heights back."""
    def hashes(svc, b):
        if b == 90 and svc == "validator-3":
            return "0xdead"
        return f"0xbb{b:062x}"

    bat = _bat(_blockhash_of=hashes)
    assert bat.check_safety_subset() is False
    assert bat.inv_fail_id == "double-finalization"
    assert " 90 " in bat.inv_fail_msg


def test_safety_halt_FAILS():
    """A node that PARKED refusing the chain. It is first in the order, so it also proves the
    order is the one `check_invariants` uses."""
    bat = _bat(_node_metrics=lambda svc: "dpos_sync_degraded{reason=\"result_divergence\"} 1")
    assert bat.check_safety_subset() is False
    assert bat.inv_fail_id == "safety-halt"


def test_every_armed_id_is_reachable_from_the_subset():
    """The armed set and the detectors that can produce it must not drift apart: an id nobody can
    emit is a check that looks armed and is not."""
    assert B.ARMED_SAFETY_IDS == {"safety-halt", "fork-detected", "result-divergence",
                                  "double-finalization"}
    assert len(ChainBattery._SAFETY_SUBSET) == 4


# ══ the subset SKIPS what it cannot judge ══════════════════════════════════

def test_an_unarmed_verdict_is_REPORTED_and_STEPPED_OVER():
    """`beacon-stuck` is a real verdict of an armed DETECTOR and is not an armed ID. It must not
    fail the run, it must not stop the two detectors after it, and it must not vanish."""
    bat = _bat(_mixhash_of=lambda svc, b: "0x" + "11" * 32)      # identical across the window
    assert bat.check_safety_subset() is True
    assert bat.inv_fail_id == ""
    skips = [n for k, n in bat.events if k == "safety_subset_skip"]
    assert len(skips) == 1 and "beacon-stuck" in skips[0]


def test_an_UNREADABLE_committee_produces_no_verdict_at_all():
    """Every cross-node read fails. The three cross-node detectors sit below their f+1 responder
    floors and the starvation belt needs consecutive EVALUATED ticks, which one sweep can never
    supply — so the sweep holds. This is the flakiness boundary: an absent node is a node that did
    not answer, not a node that disagreed."""
    bat = _bat(_node_fin_in=lambda svc: -1,
               _mixhash_of=lambda svc, b: "null",
               _blockhash_of=lambda svc, b: "null",
               _node_roots_of=lambda svc, h: "null null")
    assert bat.check_safety_subset() is True
    assert bat.inv_fail_id == ""


def test_an_EMPTY_committee_holds():
    """The shadow's shape, and the shape of a stand whose address file did not answer."""
    bat = _bat()
    bat.ctx.SIM_CUR_COMMITTEE = ""
    bat.ctx.ADDR2IDX = {}
    assert bat.check_safety_subset() is True


def test_a_DEAD_chain_holds():
    """fin=0 short-circuits all three cross-node detectors by their own guards. A chain that is
    not finalizing is a LIVENESS statement, and liveness is the cases' own business."""
    bat = _bat(_finalized_dec=lambda: 0)
    assert bat.check_safety_subset() is True


def test_no_belt_can_arm_on_a_single_sweep():
    """The structural reason the subset is safe: every belt in the battery needs at least two
    evaluated ticks, and a sweep gives it one."""
    bat = _bat()
    for knob in ("SIM_DET_STARVE_TICKS", "SIM_STALL_TICKS", "SIM_COVERAGE_MIN_TICKS",
                 "SIM_PARK_TICKS"):
        assert getattr(bat, knob) >= 2, knob


# ══ the wiring: a failing sweep reddens the case ═══════════════════════════

def _stub_stack(monkeypatch, calls):
    monkeypatch.setattr(StaticStack, "bring_up_dpos",
                        lambda self: calls.append("bring-up") or "0x40")
    monkeypatch.setattr(StaticStack, "tear_down", lambda self: calls.append("teardown"))


def _stub_addrs(monkeypatch, addrs=ADDRS):
    monkeypatch.setattr(driver.SmokeCtx, "runtime_addresses",
                        lambda self, dry_value=None: list(addrs))


def _stub_subset(monkeypatch, ok, fail_id="result-divergence"):
    def check(self):
        self.inv_fail_id = "" if ok else fail_id
        self.inv_fail_msg = "" if ok else "nodes disagree at height 97"
        return ok

    monkeypatch.setattr(driver.battery.ChainBattery, "check_safety_subset", check)


def test_a_failing_subset_FAILS_the_case(monkeypatch, capsys):
    """THE POINT OF THE WHOLE CHANGE. Every assertion the case owns passes; the sweep does not,
    and the case comes back RC_FAIL — which `cli._case_all` ranks above everything else."""
    calls = []
    _stub_stack(monkeypatch, calls)
    _stub_addrs(monkeypatch)
    _stub_subset(monkeypatch, ok=False)
    assert driver.run("smoke-x", [lambda ctx: None]) == RC_FAIL
    out = capsys.readouterr().out
    assert "SAFETY INVARIANT" in out and "result-divergence" in out
    assert calls == ["bring-up", "teardown"]        # and the stand is still torn down


def test_a_holding_subset_leaves_the_case_GREEN(monkeypatch):
    calls = []
    _stub_stack(monkeypatch, calls)
    _stub_addrs(monkeypatch)
    _stub_subset(monkeypatch, ok=True)
    assert driver.run("smoke-x", [lambda ctx: None]) == RC_PASS


def test_the_sweep_runs_AFTER_the_case_assertions(monkeypatch):
    """It judges the stand the case left behind, not the one it was handed."""
    order = []
    _stub_stack(monkeypatch, [])
    _stub_addrs(monkeypatch)

    def check(self):
        order.append("sweep")
        return True

    monkeypatch.setattr(driver.battery.ChainBattery, "check_safety_subset", check)
    driver.run("smoke-x", [lambda ctx: order.append("assertion")])
    assert order == ["assertion", "sweep"]


def test_a_case_that_already_FAILED_never_reaches_the_sweep(monkeypatch):
    """Fail-fast is unchanged: the case's own verdict is the one reported, and the sweep does not
    get to overwrite it with a reading taken from a chain that already failed."""
    _stub_stack(monkeypatch, [])
    _stub_addrs(monkeypatch)
    _stub_subset(monkeypatch, ok=False, fail_id="double-finalization")

    def boom(ctx):
        raise driver.SmokeFailure("smoke-x", "the case's own verdict")

    assert driver.run("smoke-x", [boom]) == RC_FAIL


# ══ the wiring: what must NOT be able to redden a case ═════════════════════

def test_the_kill_switch_takes_the_sweep_out_of_the_picture(monkeypatch):
    """One variable, because an operator bisecting a red gate must be able to remove the sweep
    without editing the harness."""
    monkeypatch.setenv(driver.SAFETY_SWEEP_ENV, "0")
    _stub_stack(monkeypatch, [])
    monkeypatch.setattr(driver.SmokeCtx, "runtime_addresses",
                        lambda self, dry_value=None: pytest.fail("the sweep read the stand"))
    _stub_subset(monkeypatch, ok=False)
    assert driver.run("smoke-x", [lambda ctx: None]) == RC_PASS


def test_a_DRY_run_records_the_sweep_and_evaluates_NOTHING(monkeypatch):
    """A dry run's readings are canned, so a verdict computed over them is meaningless in both
    directions — the same contract `SmokeCtx.check` keeps. The step proves the WIRING and nothing
    else, which is all a dry run has ever proved."""
    _stub_stack(monkeypatch, [])
    _stub_subset(monkeypatch, ok=False)
    monkeypatch.setattr(driver.SmokeCtx, "runtime_addresses",
                        lambda self, dry_value=None: pytest.fail("the dry sweep read the stand"))
    assert driver.run("smoke-x", [lambda ctx: None], argv=["--dry-run"]) == RC_PASS


def test_an_UNREADABLE_address_file_skips_the_sweep(monkeypatch, capsys):
    """A cross-node safety check needs at least two nodes to compare. Nothing to say is not a
    violation — but it is PRINTED, because silence is indistinguishable from a clean sweep."""
    _stub_stack(monkeypatch, [])
    _stub_addrs(monkeypatch, addrs=[])
    _stub_subset(monkeypatch, ok=False)
    assert driver.run("smoke-x", [lambda ctx: None]) == RC_PASS
    assert "safety sweep SKIPPED" in capsys.readouterr().out


def test_a_READ_SIDE_EXCEPTION_is_reported_not_asserted(monkeypatch, capsys):
    """The sweep is a net under the case, not a second case: an unreachable daemon at teardown
    time must not be able to turn a green run red."""
    _stub_stack(monkeypatch, [])
    _stub_addrs(monkeypatch)

    def boom(self):
        raise RuntimeError("docker daemon went away")

    monkeypatch.setattr(driver.battery.ChainBattery, "check_safety_subset", boom)
    assert driver.run("smoke-x", [lambda ctx: None]) == RC_PASS
    assert "safety sweep could not run" in capsys.readouterr().out


def test_the_sweep_never_reaches_for_simulation_state():
    """It builds a `ChainBattery`, the class with no `sim` attribute, so a detector that started
    reading churn bookkeeping would raise here rather than working by accident."""
    import inspect
    src = inspect.getsource(driver.safety_sweep)
    assert "battery.ChainBattery(" in src and "battery.Battery(" not in src
    assert not hasattr(ChainBattery(), "sim")
