"""test_live_seams.py — GAP-1 (live beacon/metric seam binding) + GAP-2a (result-seam self-check).

The beacon reads scrape a node's :9100 (commonware) / :9200 (reth) metrics through an in-container
curl; these tests record the READ argv (the Runner-record pattern of test_lib_write.py, here via
rpc._run_read) and pin the seam→bash wiring done by Orchestrator._bind_live_seams. The self-check
tests drive both directions (LIVE pass / DEAD abort) with injected node readers, no live topology."""

from __future__ import annotations

import ast
import importlib
import pathlib
from types import SimpleNamespace

import pytest

from dpos_harness.core import nodes, rpc
from dpos_harness.sim.orchestrator import (INERT_SEAMS, LIVE_SEAMS, Orchestrator, SimConfig, SimState)
from dpos_harness.sim.dispatch import Dispatcher
from dpos_harness.chain.writes import ChainError
from dpos_harness.core.proc import Runner
from dpos_harness.tests.conftest import strict_actuators


# ── GAP-1: nodes.py read helpers (exec argv + parse) ─────────────────────────

_COMMONWARE = "dkg_ceremony_ok_total_total 8\nbeacon_seed_active_total_total 5963\n"
_RETH = "reth_dpos_derive_db_timeout_attempts_total 17\n"


def test_beacon_metric_scrapes_9100_and_parses(monkeypatch):
    """sim_beacon_metric: `docker compose exec -T <svc> curl ... http://localhost:9100/metrics`,
    substring family, last field as int."""
    seen = {}

    def fake_run_read(cmd, timeout):
        seen["cmd"] = cmd
        return _COMMONWARE

    monkeypatch.setattr(rpc, "_run_read", fake_run_read)
    assert nodes.beacon_metric("validator-3", "dkg_ceremony_ok") == 8
    line = " ".join(seen["cmd"])
    assert "docker compose exec -T validator-3" in line
    assert "http://localhost:9100/metrics" in line
    assert "9200" not in line                                   # beacon plane ONLY


def test_beacon_metric_absent_family_is_minus_one(monkeypatch):
    monkeypatch.setattr(rpc, "_run_read", lambda cmd, timeout: _COMMONWARE)
    assert nodes.beacon_metric("validator-3", "no_such") == -1


def test_beacon_metric_unreachable_is_minus_one(monkeypatch):
    monkeypatch.setattr(rpc, "_run_read", lambda cmd, timeout: "")   # endpoint down
    assert nodes.beacon_metric("validator-3", "dkg_ceremony_ok") == -1


def test_refill_spare_attempts_scrapes_9200(monkeypatch):
    """_refill_spare_attempts: reth :9200 exporter, reth_dpos_derive_db_timeout_attempts_total."""
    seen = {}

    def fake_run_read(cmd, timeout):
        seen["cmd"] = cmd
        return _RETH

    monkeypatch.setattr(rpc, "_run_read", fake_run_read)
    assert nodes.refill_spare_attempts("validator-7") == 17
    line = " ".join(seen["cmd"])
    assert "docker compose exec -T validator-7" in line
    assert "http://localhost:9200/metrics" in line              # reth plane


def test_refill_spare_attempts_absent_is_zero(monkeypatch):
    monkeypatch.setattr(rpc, "_run_read", lambda cmd, timeout: "")  # lazily-unregistered / down
    assert nodes.refill_spare_attempts("validator-7") == 0


# ── beacon_service_for_idx (sim_beacon_metric's idx→container resolution) ────

def test_service_for_idx_native():
    assert nodes.beacon_service_for_idx(3, val_containers=18, slot_identity={}) == "validator-3"


def test_service_for_idx_retasked_reverse_lookup():
    si = {"validator-5": "20", "validator-6": "21"}
    assert nodes.beacon_service_for_idx(20, val_containers=18, slot_identity=si) == "validator-5"


def test_service_for_idx_unhosted_is_none():
    assert nodes.beacon_service_for_idx(99, val_containers=18, slot_identity={}) is None


# ── GAP-1: Orchestrator._bind_live_seams wiring ──────────────────────────────

def _orch():
    return Orchestrator(cfg=SimConfig(validators=7, initial_committee=7, spares=0,
                                       rotation_slots=0), dry_run=True)


def _live_shaped(o, confirm=None):
    """rec / hp / disp built the way a LIVE run builds them — Orchestrator._reconcilers (the very
    factory run() calls) plus a real Dispatcher — so every seam starts on its REAL construction
    stub and every object REJECTS an unknown attribute the way production would.

    This replaces the bare `SimpleNamespace()` these tests used to pass as `hp` (and as `rec`).
    A SimpleNamespace accepts ANY attribute write, so a MISSPELLED seam name in the binder wrote a
    brand-new attribute and the test still passed — the binder could bind `hp._confrim` forever."""
    chain = SimpleNamespace(p=Runner(dry=True),
                            owner_addr=lambda idx: f"0xowner{idx}",
                            committee_has=lambda addr, epoch: False)
    rec, hp = o._reconcilers(chain)
    if confirm is not None:
        rec.recovery_confirmed = confirm      # an EXISTING name — a typo here still fails
    disp = Dispatcher(o.state, chain, strict_actuators(), hp, rec)
    return disp, rec, hp


def test_bind_live_seams_wires_families(monkeypatch):
    """rec.beacon_share_delta→dkg_ceremony_ok, rec.beacon_seed_delta→beacon_seed_active,
    hp._beacon→dkg_ceremony_ok, disp.beacon_metric→arbitrary family; all resolve idx→service."""
    calls = []
    monkeypatch.setattr(nodes, "beacon_metric",
                        lambda svc, fam: calls.append((svc, fam)) or 42)
    monkeypatch.setattr(nodes, "refill_spare_attempts",
                        lambda svc: calls.append((svc, "attempts")) or 9)

    o = _orch()
    o.state.container.slot_identity["validator-6"] = "20"   # retasked host for logical idx 20
    disp, rec, hp = _live_shaped(o, confirm=lambda v: True)
    o._bind_live_seams(disp, rec, hp)

    assert rec.beacon_share_delta(3) == 42
    assert ("validator-3", "dkg_ceremony_ok") in calls
    assert rec.beacon_seed_delta(3) == 42
    assert ("validator-3", "beacon_seed_active") in calls
    assert hp._beacon(20) == 42                              # retasked idx → validator-6
    assert ("validator-6", "dkg_ceremony_ok") in calls
    assert hp._confirm == rec.recovery_confirmed             # EL-frontier gate wired (v61 a15)
    assert disp.beacon_metric(4, "dkg_ceremony_fail") == 42
    assert ("validator-4", "dkg_ceremony_fail") in calls
    assert disp.refill_spare_attempts("validator-7") == 9
    assert ("validator-7", "attempts") in calls


def test_bind_live_seams_wires_confirm_gate_not_the_false_stub(monkeypatch):
    """REGRESSION (v61 a15): hp._confirm MUST be wired to rec.recovery_confirmed. Before the fix the
    beacon seam was bound but the confirm seam was forgotten, so hp._confirm kept HostPool's
    construction default `lambda c: False` → sim_warm_ready ALWAYS returned False in a live run →
    bench-promote never fired and the committee could not grow past the native register_activate
    band. Unit tests missed it because they inject confirm= directly; this one drives the real
    _bind_live_seams and asserts the gate is the live reader, not the False stub."""
    monkeypatch.setattr(nodes, "beacon_metric", lambda svc, fam: 0)
    o = _orch()
    # rec/hp built by the live factory — hp._confirm is the real `lambda c: False` stub
    disp, rec, hp = _live_shaped(o, confirm=lambda v: True)
    assert hp._confirm("validator-0") is False               # the pre-bind stub: always False
    o._bind_live_seams(disp, rec, hp)
    assert hp._confirm == rec.recovery_confirmed             # now the live EL-frontier reader
    assert hp._confirm("validator-0") is True                # and it actually answers (not stuck False)


def test_bind_live_seams_leaves_top_stake_leader_inert():
    """top_stake_leader() is INTENTIONALLY INERT in bash (case-soak.sh:555) — the live binding must
    keep returning "" so gate rule 6 stays a tolerated no-op (never a false reject)."""
    o = _orch()
    disp, rec, hp = _live_shaped(o, confirm=lambda v: True)
    o._bind_live_seams(disp, rec, hp)
    assert disp.top_stake_leader() == ""


def test_bind_live_seams_unhosted_idx_is_minus_one(monkeypatch):
    monkeypatch.setattr(nodes, "beacon_metric", lambda svc, fam: 5)
    o = _orch()
    disp, rec, hp = _live_shaped(o, confirm=lambda v: True)
    o._bind_live_seams(disp, rec, hp)
    assert rec.beacon_share_delta(999) == -1                 # no container hosts idx 999


# ── the SEAM REGISTRY: completeness, both directions (F4.5) ──────────────────
#
# Two checks that together close the nine-hour wedge. (1) the registry covers every construction
# stub that actually exists, so an EIGHTH seam added to a constructor cannot go unregistered;
# (2) after the binder runs, the only seam left on its stub is the one deliberately left inert, so
# a REGISTERED seam the binder forgets cannot go unbound. Before F4.5 there was only a hand-written
# list of six inside test_bind_live_seams_wires_families that mirrored the binder — a seam missing
# from BOTH failed nowhere, which is exactly how `hp._confirm` was lost.

def _ast_construction_stubs() -> set:
    """DERIVE the construction stubs from the constructors themselves — never hand-listed.

    Two syntactic shapes, both unambiguous in this codebase:
      * a method whose whole body is `return <literal>` (Dispatcher/Reconcilers: -1 / 0 / "");
      * an `__init__` assignment `self.X = <arg> or <lambda>` (HostPool's injected seams).
    Anything matching either shape is a seam by construction: it answers with a constant until
    something rebinds it."""
    def _is_literal(n):
        return (isinstance(n, ast.Constant)
                or (isinstance(n, ast.UnaryOp) and isinstance(n.op, (ast.USub, ast.UAdd))
                    and isinstance(n.operand, ast.Constant)))

    root = pathlib.Path(importlib.import_module("dpos_harness.sim.orchestrator").__file__).parent
    found = set()
    for mod, cls in (("dispatch", "Dispatcher"), ("reconcilers", "Reconcilers"),
                     ("hostpool", "HostPool")):
        tree = ast.parse((root / f"{mod}.py").read_text())
        for node in ast.walk(tree):
            if not (isinstance(node, ast.ClassDef) and node.name == cls):
                continue
            for fn in node.body:
                if not isinstance(fn, ast.FunctionDef):
                    continue
                body = [b for b in fn.body
                        if not (isinstance(b, ast.Expr) and isinstance(b.value, ast.Constant))]
                if len(body) == 1 and isinstance(body[0], ast.Return) and _is_literal(body[0].value):
                    found.add((mod, cls, fn.name))
                if fn.name != "__init__":
                    continue
                for b in ast.walk(fn):
                    if (isinstance(b, ast.Assign) and isinstance(b.value, ast.BoolOp)
                            and isinstance(b.value.op, ast.Or)
                            and any(isinstance(v, ast.Lambda) for v in b.value.values)):
                        tgt = b.targets[0]
                        if isinstance(tgt, ast.Attribute):
                            found.add((mod, cls, tgt.attr))
    return found


def test_live_seam_registry_covers_every_construction_stub():
    """orchestrator.LIVE_SEAMS must name EVERY seam the three constructors stub out. Add a stub and
    forget the registry → this fails, and the residual check below can never see it."""
    derived = _ast_construction_stubs()
    assert derived, "the AST derivation found no construction stub — the derivation itself broke"
    assert derived == {(mod, cls, attr) for _p, mod, cls, attr in LIVE_SEAMS}


def test_bind_live_seams_leaves_only_the_inert_seam_on_its_stub():
    """The completeness assertion `_bind_live_seams` never had. The residual set is DERIVED (snapshot
    each registered seam, run the binder, keep the ones that did not move) — nothing here mirrors
    the binder's body, so forgetting a binding is the only way to change the answer.

    This is the exact geometry of the v61 a15 incident: the binder wired `hp._beacon` and forgot
    `hp._confirm`, sim_warm_ready stayed False for nine hours, the committee could not backfill a
    single vacancy, and no test or log said a word. With this in place that miss leaves `_confirm`
    in the residual set and the suite goes red."""
    o = _orch()
    disp, rec, hp = _live_shaped(o)          # rec.recovery_confirmed left REAL (a bound method)
    owners = {"hp": hp, "rec": rec, "disp": disp}
    before = {(p, attr): getattr(owners[p], attr) for p, _m, _c, attr in LIVE_SEAMS}

    o._bind_live_seams(disp, rec, hp)

    residual = {attr for (p, attr), was in before.items() if getattr(owners[p], attr) == was}
    assert residual == set(INERT_SEAMS)


# ── GAP-2a: sim_selfcheck_result_seams (pass / abort, both directions) ──────

def _orch_with_committee():
    o = Orchestrator(cfg=SimConfig(validators=7, initial_committee=7, spares=0,
                                    rotation_slots=0), dry_run=True)
    addrs = [f"0xowner{i}" for i in range(7)]
    o.state.cur_committee = " ".join(addrs)
    o.state.address.addr2idx = {a: f"validator-{i}" for i, a in enumerate(addrs)}
    o.state.cur_f = 2                                          # f+1 = 3 floor
    return o


def test_selfcheck_too_early_refuses():
    o = _orch_with_committee()
    ok, msg = o.selfcheck_result_seams(2)                     # fin 2 <= K=3
    assert not ok and "<= K=" in msg


def test_selfcheck_live_when_floor_met():
    o = _orch_with_committee()
    ok, msg = o.selfcheck_result_seams(
        100,
        node_fin_in=lambda ss: 90,
        node_roots_of=lambda ss, h: "0xstate 0xreceipts")
    assert ok and "LIVE" in msg


def test_selfcheck_dead_when_seams_return_null():
    o = _orch_with_committee()
    ok, msg = o.selfcheck_result_seams(
        100,
        node_fin_in=lambda ss: -1,
        node_roots_of=lambda ss, h: "null null")             # the dead-seam sentinel
    assert not ok and "DEAD" in msg


def test_selfcheck_skips_disrupted_and_permdead():
    """A disrupted / tombstoned member is excluded from the tried set — with 2 of 7 excluded and
    v0 always counting, exactly f+1=3 of the remaining 5 must answer to pass."""
    o = _orch_with_committee()
    o.state.disrupted = "validator-3"
    o.state.identity.permanently_dead = {"validator-4": 1}
    answered = {"validator-0", "validator-2", "validator-5"}
    ok, msg = o.selfcheck_result_seams(
        100,
        node_fin_in=lambda ss: 90 if ss in answered else -1,
        node_roots_of=lambda ss, h: "0xa 0xb" if ss in answered else "null null")
    # validator-3/4 never probed; v0 counts (fin=fin); +v2,v5 = 3 >= floor 3 → LIVE
    assert ok


def test_selfcheck_or_abort_passes_on_second_attempt():
    o = _orch_with_committee()
    attempts = {"n": 0}

    def flaky(_fin):
        attempts["n"] += 1
        return (attempts["n"] >= 2, "brownout" if attempts["n"] < 2 else "ok")

    slept = []
    o.selfcheck_result_seams_or_abort(finalized_dec=lambda: 100, selfcheck=flaky,
                                      sleep=slept.append)
    assert attempts["n"] == 2 and slept == [10]              # one 10s retry, then pass


def test_selfcheck_or_abort_raises_after_three_dead():
    o = _orch_with_committee()
    slept = []
    with pytest.raises(ChainError) as e:
        o.selfcheck_result_seams_or_abort(finalized_dec=lambda: 100,
                                          selfcheck=lambda _f: (False, "seams DEAD"),
                                          sleep=slept.append)
    assert e.value.reason_id == "selfcheck-seams-dead"
    assert slept == [10, 10]                                  # 3 attempts → 2 sleeps


# ── the 0-of-0 message-honesty split: empty candidate list vs dead seams ─────

def test_selfcheck_resolution_empty_names_harness_not_dead_seams():
    """An UNPOPULATED ADDR2IDX (build_addr_map skipped) resolves 0 committee addrs → 0-of-0. That
    is a HARNESS resolution failure, NOT the on-chain seams being dead: the message must name the
    resolution layer and must NOT claim the seams are DEAD (the v61.5 mis-attribution)."""
    o = _orch_with_committee()
    o.state.address.addr2idx = {}                             # build_addr_map never ran
    ok, msg = o.selfcheck_result_seams(
        100,
        node_fin_in=lambda ss: 90,                           # seams WOULD answer if we asked
        node_roots_of=lambda ss, h: "0xa 0xb")
    assert not ok
    assert "candidate list EMPTY" in msg and "RESOLUTION" in msg
    assert "DEAD" not in msg                                  # must NOT blame the seams
    assert o._seam_err_id == "selfcheck-resolution-empty"


def test_selfcheck_or_abort_resolution_empty_distinct_error_id():
    """selfcheck_result_seams_or_abort over the REAL (unpopulated-ADDR2IDX) self-check raises with
    the distinct 'selfcheck-resolution-empty' id, never the generic dead-seams id."""
    o = _orch_with_committee()
    o.state.address.addr2idx = {}
    with pytest.raises(ChainError) as e:
        o.selfcheck_result_seams_or_abort(finalized_dec=lambda: 100, sleep=lambda _s: None)
    assert e.value.reason_id == "selfcheck-resolution-empty"


def test_selfcheck_still_dead_when_addr_map_populated_but_seams_null():
    """The split must not swallow a genuine dead-seam: with ADDR2IDX populated (nodes resolve) but
    the readers returning null, it stays DEAD, not resolution-empty."""
    o = _orch_with_committee()
    ok, msg = o.selfcheck_result_seams(
        100, node_fin_in=lambda ss: -1, node_roots_of=lambda ss, h: "null null")
    assert not ok and "DEAD" in msg
    assert o._seam_err_id == "selfcheck-seams-dead"


# ── run() ordering: build_addr_map (case-soak.sh:2133) BEFORE the result-seam self-check ─────

def test_run_populates_addr_map_before_result_selfcheck(monkeypatch, tmp_path):
    """Orchestrator.run() must build ADDR2IDX from the live chain the moment bring-up returns and
    BEFORE selfcheck_result_seams_or_abort — so the self-check receives a NON-EMPTY mapping (bash
    ordering: sim_bring_up → build_addr_map → self-check). Stub the impure seams; assert the call
    order and that ADDR2IDX was populated from the committee read at self-check time."""
    from dpos_harness.stack import bringup as bringup_mod
    from dpos_harness.sim import dispatch as dispatch_mod
    from dpos_harness.checks import battery as battery_mod
    from dpos_harness.core.proc import Runner

    monkeypatch.setenv("SIM_OUT", str(tmp_path))
    monkeypatch.setenv("SIM_KEEP_UP", "1")                   # teardown must not touch docker

    o = Orchestrator(cfg=SimConfig(validators=3, initial_committee=3, spares=0,
                                    rotation_slots=0), dry_run=True)
    order = []
    addrs = [f"0xowner{i}" for i in range(3)]

    class FakeChain:
        p = Runner(dry=True)
        def current_epoch(self):
            return 5
        def committee(self, e):
            return " ".join(addrs)
        def owner_addr(self, idx):
            return addrs[int(idx)] if int(idx) < len(addrs) else ""

    monkeypatch.setattr(bringup_mod.BringUp, "run", lambda self: order.append("bringup") or True)
    monkeypatch.setattr(o, "_runner", lambda *a, **k: Runner(dry=True))
    monkeypatch.setattr(o, "_build_chain", lambda runner, bu: FakeChain())
    monkeypatch.setattr(o, "_reconcilers",
                        lambda chain: (SimpleNamespace(events=None), SimpleNamespace()))
    monkeypatch.setattr(o, "_bind_live_seams", lambda disp, rec, hp: None)
    monkeypatch.setattr(o, "self_check", lambda: order.append("self_check") or True)
    monkeypatch.setattr(dispatch_mod, "Dispatcher", lambda *a, **k: SimpleNamespace())
    monkeypatch.setattr(battery_mod, "Battery", lambda *a, **k: SimpleNamespace())

    seen_map = {}

    def fake_or_abort(**kw):
        order.append("selfcheck")
        seen_map["addr2idx"] = dict(o.state.address.addr2idx)
        raise ChainError("stop-test", "halt the test before the loop")

    monkeypatch.setattr(o, "selfcheck_result_seams_or_abort", fake_or_abort)

    rc = o.run()
    assert rc == 1                                            # the sentinel ChainError → bundle path
    assert order.index("bringup") < order.index("selfcheck")
    # ADDR2IDX was populated (build_addr_map ran) with the committee owner-addrs BEFORE the self-check.
    assert seen_map["addr2idx"] == {a: f"validator-{i}" for i, a in enumerate(addrs)}
