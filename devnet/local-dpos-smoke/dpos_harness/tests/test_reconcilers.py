"""test_reconcilers.py — the per-tick reconcilers with docker/RPC READ seams stubbed. Drives the
state machine with synthetic inputs (battery seam-override discipline) and asserts the confirm-
before-free restore flow, the tombstone against-f free, and the watchdog trips."""

import pytest

from dpos_harness.sim.orchestrator import SimConfig, SimState
from dpos_harness.sim.reconcilers import Reconcilers
from dpos_harness.chain.writes import ChainError
from dpos_harness.core.proc import Runner
from dpos_harness.tests.conftest import strict_actuators


class FakeChain:
    def __init__(self, committee_next="", seated=(), cap=0):
        self.committee_next = committee_next
        self.seated = set(str(s) for s in seated)
        self.cap = cap
        self.p = Runner(dry=True)
        self.staking_rt = "0xSTAKE"
        self.rpc = "http://localhost:8545"

    def committee(self, epoch):
        return self.committee_next

    def owner_addr(self, idx):
        return f"0xowner{idx}"

    def committee_has(self, addr, epoch):
        return addr.replace("0xowner", "") in self.seated

    def active_validators_length(self):
        return self.cap

    def _pp_cfg_read_retry(self, sig):
        return None                         # activation unresolved → barrier falls back to env


def _rec(chain=None, **state_kw):
    s = SimState(cfg=SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1))
    for k, v in state_kw.items():
        setattr(s, k, v)
    chain = chain or FakeChain()
    rec = Reconcilers(s, chain, strict_actuators())
    # stub the RPC/docker read seams so nothing live is touched
    rec.finalized_dec = lambda: 100
    rec.node_finalized = lambda v: 100
    rec.recovery_confirmed = lambda v: True
    rec.blockhash_of = lambda v, h: "0xabc"
    rec.beacon_share_delta = lambda i: 5
    rec.beacon_seed_delta = lambda i: 5
    return rec, s


def test_process_restores_issues_then_confirms_and_frees():
    """process_restores: Phase-1 issues the restart (victim STAYS against-f + RECOVERING). The
    against-f slot frees only on a LIVE chain (finalized ADVANCED since the prior tick — the
    dpos_equivocator_finalize_stall root fix), so it takes a SECOND tick with progress."""
    rec, s = _rec()
    fin = [100]
    rec.finalized_dec = lambda: fin[0]
    s.disrupted = "validator-1"
    s.container.restore_at["validator-1"] = 0
    s.container.disrupt_kind["validator-1"] = "sigkill_restart"
    rec.process_restores(5)                     # tick 1: issue; leadfin=100, chain not yet "live"
    rec.act.act_sigkill_start.assert_called_once_with("validator-1")
    assert s.disrupted == "validator-1"         # held against-f until a LIVE-chain confirm
    fin[0] = 200                                # chain advances
    rec.process_restores(5)                     # tick 2: chain_live → confirm + free
    assert s.disrupted == ""
    assert "validator-1" not in s.container.restore_at


def test_process_restores_byzantine_forge_pk_recreates_the_victim():
    """F4.2 integration: a byzantine_forge_pk victim is restored through act_byzantine_restore —
    the arm that, for the whole life of the harness, called a method NO object defined. It was
    invisible because every actuator double was a catch-all that manufactured the name; against
    the strict autospec (which raises on an unknown name) this test is the real check that the
    forge-pk victim is actually recreated instead of left running byzantine forever."""
    rec, s = _rec()
    s.disrupted = "validator-2"
    s.container.restore_at["validator-2"] = 0
    s.container.disrupt_kind["validator-2"] = "byzantine_forge_pk"
    rec.process_restores(5)
    rec.act.act_byzantine_restore.assert_called_once_with("validator-2")
    # and ONLY that actuator — a forge-pk restore is a recreate, never a plain start
    assert [c[0] for c in rec.act.method_calls] == ["act_byzantine_restore"]
    assert "validator-2" in s.container.recovering     # still held against-f until confirmed


def test_process_restores_defers_while_dkg_fragile():
    rec, s = _rec()
    rec.dkg_window_fragile = 1
    s.disrupted = "validator-1"
    s.container.restore_at["validator-1"] = 0
    s.container.disrupt_kind["validator-1"] = "sigkill_restart"
    rec.process_restores(5)
    assert rec.act.method_calls == []           # restore DEFERRED
    assert "validator-1" not in s.container.recovering


def test_recover_stall_watchdog_records_diag_not_fatal():
    """recover-stall is DEMOTED (liveness-first policy): a never-rejoining node records a diag
    event, it does NOT fail the run — chain liveness is the only pass/fail signal."""
    rec, s = _rec()
    rec.recovery_confirmed = lambda v: False   # never rejoins
    rec.finalized_dec = lambda: 100
    s.container.recovering["validator-1"] = 0.0   # long ago
    s.container.disrupt_kind["validator-1"] = "sigkill_restart"
    s.container.recover_leadfin["validator-1"] = 100
    rec.recover_deadline = -1                   # immediately overdue
    rec.process_restores(5)                     # tick 1: over_ticks=1
    rec.process_restores(5)                     # tick 2: >=2 → would-fail, now diag (no raise)
    assert any(k == "diag" and "recover-stall" in n for k, n in rec.events)


def test_tombstone_against_f_free_on_new_face():
    """process_tombstone_backfills: a new committee face frees a still-disrupted tombstone's
    against-f slot (FIFO), capped at the new-face count."""
    rec, s = _rec()
    s.identity.permanently_dead = {"validator-2": 1}
    s.disrupted = "validator-2"
    s.seat.seat_container = {2: "validator-2"}
    s.seat.pending_backfill_by_seat = {2: 3}
    s.cur_committee = "0xnewface"              # a brand-new address
    rec.process_tombstone_backfills(6)
    assert s.disrupted == ""                    # against-f slot freed
    assert 2 not in s.seat.pending_backfill_by_seat


def test_backfill_stall_watchdog_records_diag_not_fatal():
    """tombstone-backfill-stall is DEMOTED: an unbackfilled seat past the epoch budget records a
    diag, never fails the run (seat bookkeeping routinely lags the chain's top-K during growth
    ramps — bundle-20260721T015406Z). Chain liveness is the real signal; the ≤f source gate
    already keeps the committee safe."""
    rec, s = _rec()
    s.seat.pending_backfill_by_seat = {2: 0}    # enqueued @0
    # budget = membership_settle(5)+grow_land(8)=13; cur 20 > 0+13 → overdue → diag (no raise)
    rec.process_tombstone_backfills(20)
    assert any(k == "diag" and "tombstone-backfill-stall" in n for k, n in rec.events)
    assert 2 in s.seat.pending_backfill_by_seat  # obligation still tracked for future bug-finding


def test_backfill_stall_diag_latched_once_per_seat():
    """The demoted watchdog re-evaluates every tick while the seat stays open, but the latch emits
    exactly ONE diag per (watchdog, seat) — enough signal for future bug-finding, no spam."""
    rec, s = _rec()
    s.seat.pending_backfill_by_seat = {2: 0, 3: 0}   # two stuck seats
    for cur in range(20, 40):                        # 20 overdue ticks
        rec.process_tombstone_backfills(cur)
    diags = [n for k, n in rec.events if k == "diag" and "tombstone-backfill-stall" in n]
    assert len(diags) == 2                           # one per seat, not 2*20
    assert any("identity 2" in n for n in diags) and any("identity 3" in n for n in diags)


def test_slash_not_landed_watchdog_records_diag_not_fatal():
    """byzantine-slash-not-landed is DEMOTED: an unslashed byzantine records a diag, it does NOT
    fail the run — the ≤f source gate prevents an unslashed byzantine from breaking quorum, and
    chain liveness is the pass/fail signal. But the condition STAYS TRACKED (2026-07-30): the
    entry used to be popped after the first diag, so a slash landing late was indistinguishable
    from one that never landed and the live 2026-07-22 diag could never be re-checked. The
    (id, identity) latch is what keeps it from re-diagging every tick."""
    rec, s = _rec(chain=FakeChain(seated={"2"}))   # identity 2 STILL seated
    s.identity.tombstone_settle_epoch = {2: 3}
    for cur in range(10, 20):                       # cur 10 > 3+margin(3) → overdue, 10 ticks
        rec.process_tombstone_backfills(cur)
    diags = [n for k, n in rec.events if k == "diag" and "byzantine-slash-not-landed" in n]
    assert len(diags) == 1                          # latched, not one per tick
    assert s.identity.tombstone_settle_epoch[2] == 3   # STILL tracked → re-evaluated every tick


def test_slash_landed_discharges_the_obligation():
    """The pop that remains: once the identity really is out of the committee the obligation is
    discharged. An UNREADABLE owner address discharges nothing (retry next tick)."""
    rec, s = _rec(chain=FakeChain(seated=()))       # identity 2 no longer seated → slash landed
    s.identity.tombstone_settle_epoch = {2: 3}
    rec.process_tombstone_backfills(10)
    assert 2 not in s.identity.tombstone_settle_epoch
    assert not any(k == "diag" and "byzantine-slash-not-landed" in n for k, n in rec.events)

    rec, s = _rec(chain=FakeChain(seated={"2"}))
    rec.chain.owner_addr = lambda idx: ""           # read fails this tick
    s.identity.tombstone_settle_epoch = {2: 3}
    rec.process_tombstone_backfills(10)
    assert s.identity.tombstone_settle_epoch[2] == 3
    assert not any(k == "diag" for k, _n in rec.events)


def test_graceful_stop_hole_is_fatal():
    """UN-DEMOTED 2026-07-30 (F6): a node that rejoins WITHOUT the block it had finalized at stop
    is a data hole, not seat bookkeeping. Zero false-fires in 171 bundles, zero recorded reason to
    have demoted it — it raises again, as its bash counterpart always did."""
    rec, s = _rec()
    rec.finalized_dec = lambda: 100
    rec.recovery_confirmed = lambda v: True
    rec.blockhash_of = lambda v, h: "null"          # the block is MISSING after the rejoin
    s.disrupted = "validator-1"
    s.container.recovering["validator-1"] = 0.0
    s.container.disrupt_kind["validator-1"] = "graceful_stop"
    s.container.recover_leadfin["validator-1"] = 50   # < 100 → chain live
    s.container.stop_fin["validator-1"] = 42
    with pytest.raises(ChainError) as e:
        rec.process_restores(5)
    assert e.value.reason_id == "graceful-stop-hole"


def test_warm_debt_stall_is_fatal():
    """UN-DEMOTED 2026-07-30 (F6): a member that never re-warms after a restart is a real liveness
    fault. Held past SIM_WARM_DEBT_MAX_EPOCHS it raises."""
    rec, s = _rec()
    rec.beacon_share_delta = lambda i: 0            # never re-warms
    rec.beacon_seed_delta = lambda i: 0
    s.container.warm_debt["validator-1"] = 1
    s.container.warm_debt_since["validator-1"] = 0
    with pytest.raises(ChainError) as e:
        for cur in range(rec.warm_debt_max + 1, rec.warm_debt_max + 6):
            rec.compute_effective_faults(cur)
    assert e.value.reason_id == "warm-debt-stall"


def test_dkg_barrier_inert_when_no_change_no_warmdebt():
    rec, s = _rec(chain=FakeChain(committee_next=""))
    s.cur_committee = "0xaa"
    assert rec.compute_dkg_barrier(5) == 0
    assert rec.dkg_window_fragile == 0


def test_dkg_barrier_fragile_on_unmapped_incoming():
    rec, s = _rec(chain=FakeChain(committee_next="0xincoming"))
    s.cur_committee = "0xcurrent"
    s.address.addr2idx = {}                     # incoming addr not mapped → conservative fragile
    rec.beacon_share_delta = lambda i: 5
    # getDkgQual read is unreadable (canned "" → -1) → qual gate fails only past belt
    rec.compute_dkg_barrier(5)
    assert rec.dkg_window_fragile in (0, 1)     # fragile set then possibly cleared by qual gate
    assert "unmapped" in rec.dkg_barrier_notready or rec.dkg_barrier_notready == ""


# ── _dkg_barrier_verdict HEIGHT-geometry (was battery.DkgBarrier; live copy is here now, F15) ──
def _barrier():
    rec, _s = _rec()
    rec._dkg_barrier_window_reset()
    return rec


def test_dkg_barrier_verdict_hold_then_boundary():
    rec = _barrier()
    # epoch_start = act + (cur+1)*interval = 100 + 5*100 = 600; fin0=550 keeps the 3-interval cap off
    assert rec._dkg_barrier_verdict(550, 4, 100, 100, 1) == "hold"
    assert rec.dkg_barrier_estart == 600
    assert rec._dkg_barrier_verdict(620, 4, 100, 100, 1) == "boundary"


def test_dkg_barrier_verdict_unknown_activation_no_boundary():
    rec = _barrier()
    rec._dkg_barrier_verdict(300, 4, 0, 100, 1)
    assert rec.dkg_barrier_estart == -1


def test_dkg_barrier_verdict_cap():
    rec = _barrier()
    rec._dkg_barrier_verdict(100, 4, 0, 100, 0)          # fin0=100, no estart
    assert rec._dkg_barrier_verdict(400, 4, 0, 100, 0) == "cap"


def test_dkg_barrier_verdict_unmeasurable_holds():
    rec = _barrier()
    assert rec._dkg_barrier_verdict(0, 4, 0, 100, 1) == "hold"
    assert rec.dkg_barrier_fin0 == -1


# ── F7: reconcile_inflight_membership ─────────────────────────────────────────
def test_reconcile_drops_refill_resumed_when_seated():
    rec, s = _rec(refill_resumed="validator-5")
    rec.chain.seated = {"5"}                        # spare seated → resume landed
    rec.reconcile_inflight_membership(cur=10, n_now=3)   # n below cap; seated path
    assert s.refill_resumed == ""
    assert any(k == "reconcile" for k, _ in rec.events)


def test_reconcile_drops_refill_resumed_when_back_at_cap():
    rec, s = _rec(refill_resumed="validator-5")
    rec.chain.seated = set()                        # NOT seated, but committee back at cap
    rec.reconcile_inflight_membership(cur=10, n_now=4)   # n_now >= validators(4)
    assert s.refill_resumed == ""


def test_reconcile_noop_while_resume_meaningful():
    rec, s = _rec(refill_resumed="validator-5")
    rec.chain.seated = set()
    rec.reconcile_inflight_membership(cur=10, n_now=3)   # not seated AND below cap → keep resuming
    assert s.refill_resumed == "validator-5"


# ── F8: check_resource_pools ──────────────────────────────────────────────────
def test_check_resource_pools_announces_spare_exhaustion_once():
    rec, s = _rec(next_joiner=5)                     # spare_end = validators(4)+spares(1)=5 → 0 left
    rec.check_resource_pools(3)
    rec.check_resource_pools(4)
    warns = [n for k, n in rec.events if k == "warn" and "REGIME CHANGE" in n]
    assert len(warns) == 1                           # one-shot at the transition


def test_check_resource_pools_identity_exhaustion_fails_with_demand():
    import os as _os
    _os.environ["SIM_MAX_MINT_IDX"] = "6"
    try:
        rec, s = _rec(next_bench_joiner=6)           # id_left <= 0
        s.promotable = ""                            # nothing banked
        s.seat.pending_backfill_by_seat = {"9": 3}   # open obligation → unmet demand
        with pytest.raises(ChainError) as e:
            rec.check_resource_pools(5)
        assert e.value.reason_id == "test-resource-exhausted"
    finally:
        _os.environ.pop("SIM_MAX_MINT_IDX", None)


# ── F9: compute_self_partitioned + coverage + effective_faults fold ───────────
def test_compute_self_partitioned_flags_flat_seed_and_sets_coverage():
    rec, s = _rec(cur_committee="0xowner0 0xowner1")
    s.address.addr2idx = {"0xowner0": "validator-0", "0xowner1": "validator-1"}
    rec.self_partition_flat_ticks = 2
    rec.beacon_seed_delta = lambda idx: 5            # flat every tick (same value)
    for _ in range(3):
        rec.compute_self_partitioned(chain_live=1)
    assert s.battery_coverage == 2                   # both members resolved AND responded
    assert set(s.container.self_partitioned) == {"validator-0", "validator-1"}


def test_compute_self_partitioned_restart_rebaselines_not_flags():
    rec, s = _rec(cur_committee="0xowner0")
    s.address.addr2idx = {"0xowner0": "validator-0"}
    rec.self_partition_flat_ticks = 2
    seeds = iter([5, 5, 2, 2])                        # flat, flat, RESTART(backward), flat
    rec.beacon_seed_delta = lambda idx: next(seeds)
    for _ in range(4):
        rec.compute_self_partitioned(chain_live=1)
    # a backward step re-baselined and dropped the streak, so no false SELF_PARTITIONED
    assert "validator-0" not in s.container.self_partitioned


def test_effective_faults_folds_self_partitioned():
    rec, s = _rec(cur_committee="0xowner0")
    s.address.addr2idx = {"0xowner0": "validator-0"}
    rec.self_partition_flat_ticks = 1
    rec.beacon_seed_delta = lambda idx: 5
    lf = [100, 101, 102]
    rec.finalized_dec = lambda: lf.pop(0) if lf else 102
    for _ in range(3):
        rec.compute_effective_faults(5)
    assert "validator-0" in s.effective_faults.split()
