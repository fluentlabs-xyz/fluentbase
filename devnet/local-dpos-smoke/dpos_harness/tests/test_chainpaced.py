"""test_chainpaced.py — the chain-paced deadline primitive (lib.sh chain_paced_step /
wait_chain_paced) + the dkg-barrier HEIGHT verdict (soak-invariants.sh _dkg_barrier_verdict).
Drives them with stubbed cursors (the gate-test seam-override discipline)."""

from dpos_harness.core.chainpaced import ChainPaced
from dpos_harness.sim.orchestrator import SimConfig, SimState
from dpos_harness.sim.reconcilers import Reconcilers
from dpos_harness.core.proc import Runner


# ── chain_paced_step / wait_chain_paced ──────────────────────────────────────

def _cp(blocks, head_age=0):
    it = iter(blocks)
    return ChainPaced(block_number=lambda: next(it, blocks[-1]),
                      head_age_s=lambda: head_age, sleep=lambda s: None)


def test_step_met_returns_met_and_resets():
    cp = _cp([100, 101])
    assert cp.step("k", met=1, domain="blocks", budget=3) == "met"
    assert "k" not in cp.cp_start


def test_step_budget_exhausted():
    """Entry latched at 100; once the cursor advances >= budget with met=0 → 'budget' (the honest
    failure: the chain made the agreed progress, condition still unmet)."""
    cp = _cp([100, 101, 102, 103])
    cp.step("k", 0, "blocks", 3)     # latch 100
    cp.step("k", 0, "blocks", 3)     # 101 adv 1
    cp.step("k", 0, "blocks", 3)     # 102 adv 2
    assert cp.step("k", 0, "blocks", 3) == "budget"   # 103 adv 3 >= budget


def test_step_frozen_escape():
    """A head unmoving past the frozen threshold (blocks/epochs domain) → 'frozen' (deferred to
    finalize-stall / node-down)."""
    cp = _cp([100, 100], head_age=200)
    cp.step("k", 0, "blocks", 99)
    assert cp.step("k", 0, "blocks", 99) == "frozen"


def test_step_read_fail_on_unreadable_cursor():
    cp = ChainPaced(block_number=lambda: None, sleep=lambda s: None)
    assert cp.step("k", 0, "blocks", 3) == "read_fail"


def test_wait_returns_true_on_met():
    calls = iter([False, False, True])
    cp = _cp([100, 101, 102])
    assert cp.wait("k", lambda: next(calls), "blocks", 10) is True


def test_wait_returns_false_on_budget_with_attribution():
    cp = _cp([100, 101, 102, 103, 104])
    ok = cp.wait("k", lambda: False, "blocks", 2)
    assert ok is False
    assert "budget" in cp.fail_msg


def test_dry_short_circuits():
    cp = ChainPaced(dry=True)
    assert cp.step("k", 0, "blocks", 3) == "waiting"
    assert cp.wait("k", lambda: False, "blocks", 1) is True


# ── _dkg_barrier_verdict HEIGHT geometry (replaces the simplified 2-tick form) ─

def _rec():
    s = SimState(cfg=SimConfig(validators=7, initial_committee=7))

    class C:
        p = Runner(dry=True)
        staking_rt = "0xS"
        rpc = "http://x"

        def _pp_cfg_read_retry(self, sig):
            return None
    rec = Reconcilers(s, C(), None)
    rec.epoch_interval = 64
    return rec


def test_barrier_verdict_hold_below_boundary():
    rec = _rec()
    # first measurable call pins fin0=1000, estart = act(500) + (cur+1=6)*64 = 884? act 500 →
    # 500+6*64=884 < fin 1000 → would be boundary; use act so estart > fin to stay hold.
    v = rec._dkg_barrier_verdict(fin=1000, cur=5, act=1000, interval=64, imm=1)
    assert v == "hold"
    assert rec.dkg_barrier_fin0 == 1000
    assert rec.dkg_barrier_estart == 1000 + 6 * 64        # act + (cur+1)*interval


def test_barrier_verdict_boundary_crossed():
    rec = _rec()
    # act=716 → estart = 716 + (cur+1=6)*64 = 1100, inside (fin0=1000, fin0+3*interval=1192).
    v0 = rec._dkg_barrier_verdict(fin=1000, cur=5, act=716, interval=64, imm=1)  # < estart → hold
    assert v0 == "hold" and rec.dkg_barrier_estart == 1100
    v = rec._dkg_barrier_verdict(fin=1150, cur=5, act=716, interval=64, imm=1)   # >= estart, adv<3i
    assert v == "boundary"


def test_barrier_verdict_cap_three_intervals():
    rec = _rec()
    rec._dkg_barrier_verdict(fin=1000, cur=5, act=0, interval=64, imm=0)   # fin0=1000, no estart
    v = rec._dkg_barrier_verdict(fin=1000 + 3 * 64, cur=5, act=0, interval=64, imm=0)
    assert v == "cap"                                       # advanced >= 3*interval


def test_barrier_verdict_unmeasurable_fin_holds():
    rec = _rec()
    assert rec._dkg_barrier_verdict(fin=0, cur=5, act=100, interval=64, imm=1) == "hold"
    assert rec.dkg_barrier_fin0 == -1                       # never pinned on an RPC blip
