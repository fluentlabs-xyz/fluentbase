"""test_state_ctx.py — the SimState → (battery.Ctx, battery.SimCtx) adapter. Pins the field
mapping the two halves agreed on, asserts no NAME drift (every battery-read global has a
SimState home), and pins WHICH half each field lands in (P4 item 4.4)."""

from dpos_harness.sim.orchestrator import SimConfig, SimState
from dpos_harness.sim import state_ctx


def _state():
    s = SimState(cfg=SimConfig(validators=14, initial_committee=7, spares=2))
    s.tick = 11
    s.round = 22
    s.cur_epoch = 33
    s.cur_f = 2
    s.cur_committee = "0xaa 0xbb"
    s.disrupted = "validator-3"
    s.settle_until_epoch = 40
    s.next_joiner = 8
    s.grow_landed = 6
    s.promote_pending = "9"
    s.address.addr2idx = {"0xaa": "validator-0"}
    s.identity.permanently_dead = {"validator-5": 1}
    s.identity.ever_faulted = {5: "byzantine_equivocate"}
    s.container.slot_identity = {"validator-12": "20"}
    return s


def test_adapter_maps_every_field():
    s = _state()
    ctx, sim = state_ctx.to_ctx(s)
    assert ctx.SIM_TICK == 11
    assert ctx.SIM_ROUND == 22
    assert ctx.SIM_CUR_EPOCH == 33
    assert ctx.SIM_CUR_F == 2
    assert ctx.SIM_CUR_COMMITTEE == "0xaa 0xbb"
    assert ctx.SIM_DISRUPTED == "validator-3"
    assert ctx.SETTLE_UNTIL_EPOCH == 40
    assert sim.NEXT_JOINER == 8
    assert sim.GROW_LANDED == 6
    assert sim.PROMOTE_PENDING == "9"
    assert ctx.ADDR2IDX == {"0xaa": "validator-0"}
    assert ctx.PERMANENTLY_DEAD == {"validator-5": 1}
    assert sim.EVER_FAULTED == {5: "byzantine_equivocate"}
    # SLOT_IDENTITY values are INT-normalized: the source stores string idx (bash assoc-array
    # semantics), but the battery's idle-rotation predicate compares against an int idx, so the
    # adapter converts here (v61.10: "20" != 20 str/int mismatch fired node-down on idle slots).
    assert sim.SLOT_IDENTITY == {"validator-12": 20}


def test_adapter_maps_cfg_derived():
    s = _state()
    ctx, sim = state_ctx.to_ctx(s)
    assert ctx.MIN_COMMITTEE == s.cfg.min_committee
    assert ctx.SIM_NO_CASCADE == s.cfg.no_cascade
    # the identity-pool banding is sim bookkeeping: only _node_down_excluded reads it
    assert sim.SIM_VALIDATORS == 14
    assert sim.SIM_SPARE_END == s.cfg.spare_end
    assert sim.SIM_VAL_CONTAINERS == s.cfg.val_containers


def test_overrides_win():
    s = _state()
    ctx, _sim = state_ctx.to_ctx(s, STAKING_RT="0xDEPLOYED", SIM_PEER_SPOKE="validator-2")
    assert ctx.STAKING_RT == "0xDEPLOYED"
    assert ctx.SIM_PEER_SPOKE == "validator-2"


def test_refill_stall_verdict_is_the_only_documented_drift():
    """REFILL_STALL_VERDICT is battery-internal (no SimState home) — it keeps its SimCtx default."""
    _ctx, sim = state_ctx.to_ctx(_state())
    assert sim.REFILL_STALL_VERDICT == "climbing"


def test_the_two_halves_are_disjoint_and_complete():
    """The adapter must not be able to put a field in the wrong half: both context classes reject
    an unknown keyword, so this is really a pin on the split not silently drifting back together."""
    from dpos_harness.checks.battery import Ctx, SimCtx
    gen, simf = set(vars(Ctx())), set(vars(SimCtx()))
    assert gen & simf == set(), "a field is declared in BOTH contexts"
    assert len(gen) == 21 and len(simf) == 13, (len(gen), len(simf))
    import pytest
    with pytest.raises(TypeError):
        Ctx(EVER_FAULTED={})                  # sim bookkeeping cannot enter the generic context
    with pytest.raises(TypeError):
        SimCtx(SIM_CUR_EPOCH=1)              # nor a chain fact the sim context
