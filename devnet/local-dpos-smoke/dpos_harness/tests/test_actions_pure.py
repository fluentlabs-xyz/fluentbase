"""Pure action helpers — counter-progress, warm-debt belt, DKG barrier, mint/pool economics,
the staking-reader body. Ports the soak-gate-test.sh assertions for the pure seams in
soak-actions.sh."""

from __future__ import annotations

import json
import os

import pytest

from dpos_harness.sim import actions
from dpos_harness.chain.writes import ChainError, staking_reader_json
from dpos_harness.sim.actions import (counter_progress, warm_debt_step, promote_nohost_is_leak,
                             promote_gate_reason, dkg_member_ready,
                             adds_disruption)


# ── _counter_progress: the flat-vs-restart split is load-bearing ──────────────
def test_counter_progress_five_cases():
    assert counter_progress(-1, 5) == "unreadable"        # endpoint down
    assert counter_progress(5, None) == "baseline"        # no usable prev
    assert counter_progress(5, "__none__") == "baseline"
    assert counter_progress(5, -1) == "baseline"          # negative sentinel prev
    assert counter_progress(3, 5) == "restart"            # cur<prev → RE-BASELINE, not a stall
    assert counter_progress(5, 5) == "flat"               # THE stall signal
    assert counter_progress(6, 5) == "progress"


# ── _warm_debt_step: 2-tick belt ──────────────────────────────────────────────
def test_warm_debt_step_belt():
    assert warm_debt_step(held=3, budget=12, prev=0) == ("ok", 0)
    assert warm_debt_step(held=12, budget=12, prev=0) == ("warn", 1)   # first over-tick
    assert warm_debt_step(held=12, budget=12, prev=1) == ("fail", 2)   # second → fail


# ── _promote_nohost_is_leak: leak only WITH a stuck-recyclable tombstone ──────
def test_promote_nohost_leak_needs_stuck_tombstone():
    assert not promote_nohost_is_leak(epochs=20, max_epochs=10, stuck="")     # scarcity → defer
    assert promote_nohost_is_leak(epochs=20, max_epochs=10, stuck="validator-9")  # real leak
    assert not promote_nohost_is_leak(epochs=5, max_epochs=10, stuck="validator-9")  # under cap


# ── _promote_gate_reason: names the FIRST blocking outer gate ─────────────────
def test_promote_gate_reason_precedence():
    assert "UNREADABLE" in promote_gate_reason(read_ok=0, cur=5, settle=0, clean=1, n=6, cap=7)
    assert "settle window" in promote_gate_reason(read_ok=1, cur=3, settle=5, clean=1, n=6, cap=7)
    assert "DIRTY" in promote_gate_reason(read_ok=1, cur=5, settle=0, clean=0, n=6, cap=7)
    assert "at cap" in promote_gate_reason(read_ok=1, cur=5, settle=0, clean=1, n=7, cap=7)
    assert promote_gate_reason(read_ok=1, cur=5, settle=0, clean=1, n=6, cap=7) == ""


# ── _dkg_member_ready: counter-reset safe ─────────────────────────────────────
def test_dkg_member_ready_states():
    assert dkg_member_ready("__none__", 5) == (False, "not-ready")   # no baseline captured
    assert dkg_member_ready(2, -1) == (False, "not-ready")           # unreachable metric
    assert dkg_member_ready(5, 2)[1] == "rebaseline"                 # cur<base → restart
    assert dkg_member_ready(2, 3) == (True, "ready")                 # +1 delta → finalized fresh DKG
    assert dkg_member_ready(2, 2) == (False, "not-ready")            # flat → not qualified yet


# ── _adds_disruption: voluntary_exit / delegate / register add none ───────────
def test_adds_disruption_classification():
    for a in ("graceful_stop_restart", "sigkill_restart", "cpu_throttle",
              "dkg_midwindow_restart", "byzantine_equivocate"):
        assert adds_disruption(a)
    for a in ("voluntary_exit", "delegate_shift", "register_activate"):
        assert not adds_disruption(a)


# ── the shipping lottery pool ─────────────────────────────────────────────────
@pytest.mark.parametrize("byz,vex,want", [
    (0, 0, ["graceful_stop_restart", "sigkill_restart", "cpu_throttle",
            "dkg_midwindow_restart", "delegate_shift"]),
    (1, 0, ["graceful_stop_restart", "sigkill_restart", "cpu_throttle",
            "dkg_midwindow_restart", "delegate_shift", "byzantine_equivocate"]),
    (1, 1, ["graceful_stop_restart", "sigkill_restart", "cpu_throttle",
            "dkg_midwindow_restart", "delegate_shift", "byzantine_equivocate",
            "voluntary_exit"]),
])
def test_actions_pool_composition(monkeypatch, byz, vex, want):
    """The pool the sim actually draws from, in ORDER — the guard `test_replay` stopped being
    when its bash pool was frozen. `byzantine_forge_pk` must not come back: it actuates a mode the
    node `bail!`s on, so its presence turns a fault-injection draw into a container kill.

    Goes red when a name is added, removed or reordered without the author saying so, and when a
    seed's modulus changes underneath a replayable run."""
    from dpos_harness.sim.orchestrator import SimConfig
    monkeypatch.delenv("SIM_ACTIONS", raising=False)
    monkeypatch.setenv("SIM_BYZANTINE", str(byz))
    monkeypatch.setenv("SIM_VOLUNTARY_EXIT", str(vex))
    assert SimConfig().actions_pool() == want


def test_every_pool_action_has_an_apply_arm(monkeypatch):
    """A drawn action with no arm in `apply_action` is a lottery slot that can only log a skip —
    the shape `liveness_jail`'s deletion removed. Red the day a name is added to the pool and not
    to the dispatch (or vice versa)."""
    from dpos_harness.sim import rounds
    from dpos_harness.sim.orchestrator import SimConfig
    monkeypatch.delenv("SIM_ACTIONS", raising=False)
    monkeypatch.setenv("SIM_BYZANTINE", "1")
    monkeypatch.setenv("SIM_VOLUNTARY_EXIT", "1")
    pool = SimConfig().actions_pool()
    assert set(pool) == set(rounds.APPLY_ACTIONS), (
        f"pool {sorted(pool)} != apply arms {sorted(rounds.APPLY_ACTIONS)}")


# ── staking_reader_json: ONE field, lowercased, and no way to omit it ────────
def test_staking_reader_json_is_one_lowercased_field():
    """The node's whole `--dpos.staking-config`. Three fields collapsed to one because the
    config's worst failure mode was an OMITTED one: it fell back to a codeless address, and an
    EVM call to a codeless account returns Success, so the per-block system call became a
    silent no-op. Lowercased to match what `genesis-bootstrap` writes — bring-up re-writes this
    file over the bootstrap's copy, and the two must produce the same bytes."""
    out = json.loads(staking_reader_json("0xAABBccDD00000000000000000000000000520011"))
    assert out == {"staking_address": "0xaabbccdd00000000000000000000000000520011"}


@pytest.mark.parametrize("bad", ["", "   ", "not-an-address", "520011"])
def test_staking_reader_json_refuses_a_non_address(bad):
    """A refusal, not a best-effort write. The node PARSES whatever is in this file and then
    reads at it; a blank or malformed address gets past the parser and reports nothing, which is
    the failure this whole collapse exists to remove."""
    with pytest.raises(ChainError):
        staking_reader_json(bad)


# ── mint/pool economics: SIM_MINT_FUNDABLE + the identity-pool arithmetic ─────
def test_mint_fundable_arithmetic():
    # SIM_MINT_FUNDABLE = (GENESIS - RESERVE - FLOOR) / MINT_WEI (case-soak.sh:179)
    os.environ.update(SIM_MINT_ETH_WEI="10000000000000000",
                      SIM_GENESIS_ETH_WEI="1000000000000000000",
                      SIM_MINT_ETH_RESERVE_WEI="100000000000000000",
                      SIM_MINT_ETH_FLOOR_WEI="50000000000000000")
    genesis = int(os.environ["SIM_GENESIS_ETH_WEI"])
    reserve = int(os.environ["SIM_MINT_ETH_RESERVE_WEI"])
    floor = int(os.environ["SIM_MINT_ETH_FLOOR_WEI"])
    mint_wei = int(os.environ["SIM_MINT_ETH_WEI"])
    fundable = (genesis - reserve - floor) // mint_wei
    assert fundable == 85   # 0.85 ETH / 0.01 ETH = 85 mints from owner-0's genesis grant


def test_config_derived_pool_arithmetic():
    os.environ.update(SIM_VALIDATORS="14", SIM_SPARES="2", SIM_ROTATION_SLOTS="2",
                      SIM_INITIAL_COMMITTEE="7", SIM_IDENTITY_POOL="0")
    from dpos_harness.sim.orchestrator import SimConfig
    cfg = SimConfig()
    assert cfg.val_containers == 18          # 14 + 2 + 2
    assert cfg.spare_end == 16               # 14 + 2 (first idx PAST the spare band)
    assert cfg.identity_pool == 24           # val_containers + 6 (full-profile bench, case-soak.sh:141)
    assert cfg.initial_f == 2 and cfg.min_committee == 7   # (7-1)/3=2, 3*2+1=7


def _clear_profile_env():
    for k in ("SIM_QUICK", "SIM_VALIDATORS", "SIM_INITIAL_COMMITTEE", "SIM_IDENTITY_POOL",
              "SIM_SPARES", "SIM_ROTATION_SLOTS"):
        os.environ.pop(k, None)


def test_config_full_profile_defaults():
    """Clean-env FULL profile (case-soak.sh:64,79,141): validators=10 (NOT 14 — that flat
    default was the bash VAL_CONTAINERS value, and drove the phantom validator-14..17), so
    val_containers=14 → compose defines validator-0..13; identity_pool=val_containers+6=20."""
    _clear_profile_env()
    from dpos_harness.sim.orchestrator import SimConfig
    cfg = SimConfig()
    assert cfg.validators == 10
    assert cfg.initial_committee == 7
    assert cfg.spare_end == 12
    assert cfg.val_containers == 14          # 10 + 2 + 2 — NOT 18
    assert cfg.identity_pool == 20           # val_containers + 6 (bench beyond containers)


def test_config_quick_profile_defaults():
    """Clean-env QUICK profile (case-soak.sh:59,62,139): validators=4, initial_committee=4,
    identity_pool=val_containers+2."""
    _clear_profile_env()
    os.environ["SIM_QUICK"] = "1"
    try:
        from dpos_harness.sim.orchestrator import SimConfig
        cfg = SimConfig()
        assert cfg.validators == 4
        assert cfg.initial_committee == 4
        assert cfg.spare_end == 6
        assert cfg.val_containers == 8       # 4 + 2 + 2
        assert cfg.identity_pool == 10       # val_containers + 2
    finally:
        os.environ.pop("SIM_QUICK", None)


# ── enqueue out-value contract (no subshell write-loss dodge needed) ──────────
def test_enqueue_returns_seat_value():
    os.environ.update(SIM_VALIDATORS="7", SIM_INITIAL_COMMITTEE="7", SIM_SPARES="0",
                      SIM_ROTATION_SLOTS="0")
    from dpos_harness.sim.orchestrator import SimState, SimConfig
    st = SimState(cfg=SimConfig())
    st.container.slot_identity["validator-5"] = "16"   # reborn container serves minted idx 16
    seat = actions.enqueue_backfill_obligation(st, "validator-5", cur_epoch=7)
    assert seat == "16"   # ident_idx resolves the SERVED idx, not the native container idx
