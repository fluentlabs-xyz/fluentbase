"""Dispatcher liveness — warm-stint rotation, pacing, vacancy budgets, slot reuse.

Ports the soak-gate-test.sh dispatcher/selector assertions for the pure decision predicates the
promote/refill/bench-join tracks lean on."""

from __future__ import annotations

from dpos_harness.sim import actions
from dpos_harness.sim.actions import (bench_join_due, slot_reusable, spare_reserved_for_refill,
                             select_promote_candidate, PromoteChoice)


# ── bench-join pacing (phase-3 D5): ELAPSED rounds, not a fixed modulo phase ──
def test_bench_join_due_elapsed_not_modulo():
    assert not bench_join_due(round_=3, last_fire_round=2, gap=3)   # only 1 elapsed
    assert bench_join_due(round_=5, last_fire_round=2, gap=3)       # 3 elapsed → due
    # a missed opportunity stays due (elapsed keeps growing) — no phase to lock onto
    assert bench_join_due(round_=100, last_fire_round=2, gap=3)


# ── spare vacancy budget: a spare at/past the NEXT_JOINER cursor is refill-reserved ──
def test_spare_reserved_for_refill():
    # cursor at 14, spare_end 16: spare 14/15 still refill's; spare 13 already consumed
    assert spare_reserved_for_refill(spare=14, next_joiner=14, spare_end=16)
    assert spare_reserved_for_refill(spare=15, next_joiner=14, spare_end=16)
    assert not spare_reserved_for_refill(spare=13, next_joiner=14, spare_end=16)


def test_spare_reserved_none_once_refill_path_exhausted():
    # cursor reached spare_end → refill path dead → nothing reserved
    assert not spare_reserved_for_refill(spare=15, next_joiner=16, spare_end=16)


def test_rotation_band_never_reserved_at_steady_state():
    """v61 a18 regression: a ROTATION-band container ([spare_end, val_containers)) is NEVER a
    refill target — refill's cursor only walks the SPARE band [validators, spare_end). At the
    steady-state full committee (next_joiner == validators == 10, spare_end 12, val_containers 14),
    rotation slots 12/13 satisfied the old unbounded `spare >= next_joiner` and read as reserved,
    starving bench-join of a host and wedging ALL committee rotation. They must read as FREE."""
    validators, spare_end = 10, 12                 # rotation band = {12, 13}
    for nj in range(validators, spare_end + 1):     # every cursor position the refill path can hold
        assert not spare_reserved_for_refill(spare=12, next_joiner=nj, spare_end=spare_end)
        assert not spare_reserved_for_refill(spare=13, next_joiner=nj, spare_end=spare_end)
    # and the SPARE band is still reserved at/past the cursor (fix did not over-free)
    assert spare_reserved_for_refill(spare=10, next_joiner=10, spare_end=spare_end)
    assert spare_reserved_for_refill(spare=11, next_joiner=10, spare_end=spare_end)
    assert not spare_reserved_for_refill(spare=10, next_joiner=11, spare_end=spare_end)  # consumed


# ── _slot_reusable: the SEATED-GUARD-FIRST fix (bundle-20260720T170507Z) ──────
def test_slot_reusable_seated_never_evicted_even_on_native_idx():
    # consumed spare: served == native AND seated → must NOT be reusable (the incident)
    assert not slot_reusable(served=8, native=8, seated=True, promotable="",
                             promote_pending="", refill_pending="")


def test_slot_reusable_native_and_not_seated_is_idle():
    assert slot_reusable(served=8, native=8, seated=False, promotable="",
                         promote_pending="", refill_pending="")


def test_slot_reusable_retasked_but_promotable_is_needed():
    assert not slot_reusable(served=16, native=8, seated=False, promotable="16",
                             promote_pending="", refill_pending="")


def test_slot_reusable_retasked_and_free_is_reclaimable():
    assert slot_reusable(served=16, native=8, seated=False, promotable="17",
                         promote_pending="", refill_pending="")


def test_slot_reusable_in_flight_landings_block_reuse():
    assert not slot_reusable(served=16, native=8, seated=False, promotable="",
                             promote_pending="16", refill_pending="")
    assert not slot_reusable(served=16, native=8, seated=False, promotable="",
                             promote_pending="", refill_pending="16")


# ── warm-stint rotation (phase-3 D1): budget the kind=warm sub-path ───────────
def test_warm_stint_charge_and_expire():
    import os
    os.environ["SIM_PROMOTE_WARM_MAX_ROUNDS"] = "3"
    tries = {}

    def charge(idx):
        tries[idx] = tries.get(idx, 0) + 1

    def expired(idx):
        cap = int(os.environ["SIM_PROMOTE_WARM_MAX_ROUNDS"])
        if tries.get(idx, 0) >= cap:
            tries[idx] = 0   # RESET the stint (the caller rotates to tail in the same breath)
            return True
        return False

    idx = "16"
    for _ in range(2):
        charge(idx)
        assert not expired(idx)
    charge(idx)                 # 3rd
    assert expired(idx)         # budget spent → rotate + yield
    assert tries[idx] == 0      # reset for a fresh stint (slow candidate keeps earning stints)


# ── select_promote_candidate: ready wins; else first hostable; else block reason ──
def test_select_ready_wins_over_shadowing_head():
    # head "16" not ready but hostable; "17" ready → ready wins (v56 shadowing fix)
    choice = select_promote_candidate("16 17",
                                      warm_ready=lambda i: i == "17",
                                      host_avail=lambda i: True)
    assert choice.ok and choice.cand == "17" and choice.kind == "ready"


def test_select_first_hostable_when_none_ready():
    choice = select_promote_candidate("16 17",
                                      warm_ready=lambda i: False,
                                      host_avail=lambda i: i == "17")
    assert choice.ok and choice.cand == "17" and choice.kind == "warm"


def test_select_empty_pool_block_reason():
    choice = select_promote_candidate("", warm_ready=lambda i: False, host_avail=lambda i: False)
    assert not choice.ok and "PROMOTABLE is EMPTY" in choice.block_reason


def test_select_no_host_block_reason():
    choice = select_promote_candidate("16 17",
                                      warm_ready=lambda i: False, host_avail=lambda i: False)
    assert not choice.ok and "none is hostable" in choice.block_reason


# ── seat/backfill obligation domain discipline (the reviewed-hardest part) ────
def _fresh_state():
    import os
    os.environ.update(SIM_VALIDATORS="7", SIM_INITIAL_COMMITTEE="7", SIM_SPARES="0",
                      SIM_ROTATION_SLOTS="0")
    from dpos_harness.sim.orchestrator import SimState, SimConfig
    return SimState(cfg=SimConfig())


def test_enqueue_freezes_seat_and_no_overwrite_keeps_older_stamp():
    st = _fresh_state()
    seat = actions.enqueue_backfill_obligation(st, "validator-3", cur_epoch=10)
    assert seat == "3"
    assert st.seat.pending_backfill_by_seat["3"] == 10
    assert st.seat.seat_container["3"] == "validator-3"
    # re-enqueue the SAME seat while open → keep the OLDER stamp (never refresh a live deadline)
    actions.enqueue_backfill_obligation(st, "validator-3", cur_epoch=25)
    assert st.seat.pending_backfill_by_seat["3"] == 10


def test_clear_oldest_is_fifo_min_by_epoch():
    st = _fresh_state()
    st.container.slot_identity["validator-3"] = "3"
    st.container.slot_identity["validator-4"] = "4"
    actions.enqueue_backfill_obligation(st, "validator-4", cur_epoch=20)
    actions.enqueue_backfill_obligation(st, "validator-3", cur_epoch=10)
    actions.clear_oldest_pending_backfill(st)
    # the older (epoch 10, seat "3") is cleared first
    assert "3" not in st.seat.pending_backfill_by_seat
    assert "4" in st.seat.pending_backfill_by_seat


def test_recycle_severs_seat_container_bridge():
    # a re-tasked container must not let an OLD seat reach the NEW occupant's state
    st = _fresh_state()
    actions.enqueue_backfill_obligation(st, "validator-8", cur_epoch=5)
    # simulate sim_recycle_container severing every seat->container bridge naming validator-8
    for seat, c in list(st.seat.seat_container.items()):
        if c == "validator-8":
            del st.seat.seat_container[seat]
    # release now frees nothing from the container domain (bridge gone) — no cross-domain write
    actions.release_covered_seat(st, "8")
    assert "8" not in st.seat.pending_backfill_by_seat
