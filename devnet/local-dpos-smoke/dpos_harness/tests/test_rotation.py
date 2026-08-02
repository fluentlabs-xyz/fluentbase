"""test_rotation.py — the chain-driven ROTATION phase (v61 a20). Once growth fills the cap the
dispatcher latches into phase-2, where a departure is JUST a stake/status op (the on-chain
stake-weighted top-k re-selects the committee) and the departed container is rekeyed under a fresh
pool key. These tests pin: (1) the new lib_write chain ops issue the exact cast/gov lines; (2)
each permanent departure issues ONLY a stake/status op — NEVER register_activate / bench_promote /
setActiveValidatorsLength; (3) transient/liveness stay reversible (no rekey); (4) the rotation
reconciler rebirths a departed container under a fresh key once it has left the committee, and
emits a diagnostic-only rotation-stall when a departure never lands.
"""

import pytest

from dpos_harness.chain.writes import Chain, ChainError
from dpos_harness.core.proc import Runner
from dpos_harness.sim.orchestrator import SimConfig, SimState
from dpos_harness.sim.dispatch import Dispatcher
from dpos_harness.sim.reconcilers import Reconcilers
from dpos_harness.tests.conftest import strict_actuators


# ── lib_write chain-op oracle (dry Runner records argv) ──────────────────────
def _chain(**reads):
    r = Runner(dry=True)
    r.reads = {
        "docker compose exec": "aa" * 32,
        "cast wallet": "0x000000000000000000000000000000000000dEaD",
        "cast nonce": "0",
        # getValidatorStatus → "<addr> <statusByte>"; byte 2 = Pending (register_setkeys asserts ==2)
        "cast call": "0x000000000000000000000000000000000000dEaD 2",
        "cast balance": "1000000000000000000",
        "cast gas-price": "1000000000",
    }
    r.reads.update(reads)
    c = Chain(runner=r, RPC="http://localhost:8545", STAKING_RT="0xSTAKE",
              CHAIN_CONFIG_RT="0xCFG", GOV_ADDR="0xGOV", TOKEN="0xTOKEN", CHAIN_ID="2026",
              PP_PEERS="6")
    return c, r


def _find(r, *needles):
    for a in r.log:
        line = " ".join(str(x) for x in a.argv)
        if all(n in line for n in needles):
            return a.argv
    return None


def test_undelegate_sends_undelegate():
    """sim_undelegate: cast send <STAKING> undelegate(address,uint256) <addr> <amt> from own key."""
    c, r = _chain()
    c.undelegate(5, "2000000000000000000")
    assert _find(r, "cast", "send", "undelegate(address,uint256)", "2000000000000000000")


def test_stake_delegate_approves_then_delegates():
    """sim_stake_delegate (вытеснен): approve(TOKEN) then delegate(STAKING) the evict amount."""
    c, r = _chain()
    c.stake_delegate(8, "10000000000000000000")
    assert _find(r, "cast", "send", "approve(address,uint256)(bool)", "10000000000000000000")
    assert _find(r, "cast", "send", "delegate(address,uint256)", "10000000000000000000")


def test_remove_validator_uses_governance():
    """sim_remove_validator: removeValidator(address) via the gov propose→vote→execute flow."""
    c, r = _chain()
    c.remove_validator(6)
    assert _find(r, "cast", "calldata", "removeValidator(address)")
    assert _find(r, "cast", "send", "0xGOV", "propose(address[],uint256[],bytes[],string)(uint256)")


def test_activate_bench_registers_activates_delegates_low_no_cap_raise():
    """sim_activate_bench: register→activateValidator(gov)→delegate LOW, and NEVER a cap raise
    (setActiveValidatorsLength is growth-only)."""
    c, r = _chain()
    c.activate_bench(12, "1000000000000000000")
    assert _find(r, "cast", "calldata", "activateValidator(address)")
    assert _find(r, "cast", "send", "delegate(address,uint256)", "1000000000000000000")
    assert _find(r, "cast", "calldata", "setActiveValidatorsLength(uint32)") is None


def test_fund_blend_tops_up_deficit_from_owner0():
    """sim_fund_blend: a member below the target gets a BLEND transfer of the DEFICIT from owner-0
    (balance reads 0 in the dry Runner → full-amount top-up)."""
    c, r = _chain()
    c.fund_blend(3, "60000000000000000000")
    assert _find(r, "cast", "send", "0xTOKEN", "transfer(address,uint256)(bool)",
                 "60000000000000000000")


def test_self_delegate_to_delegates_the_deficit_from_own_key():
    """sim_self_delegate_to: a member at stake 0 (dry read) self-delegates the full target —
    approve(TOKEN) then delegate(STAKING) of target-0."""
    c, r = _chain()
    c.self_delegate_to(5, "50000000000000000000")
    assert _find(r, "cast", "send", "approve(address,uint256)(bool)", "50000000000000000000")
    assert _find(r, "cast", "send", "delegate(address,uint256)", "50000000000000000000")


def test_self_delegate_to_is_noop_when_already_at_target():
    """No wasted tx when the member's CURRENT stake already meets the target (getValidatorStatus
    field 3 = 99e18 ≥ 50e18 target → delta ≤ 0 → no delegate)."""
    c, r = _chain(**{"cast call":
                     "0x000000000000000000000000000000000000dEaD 1 99000000000000000000 0 0 0 0 0"})
    c.self_delegate_to(5, "50000000000000000000")
    assert _find(r, "cast", "send", "delegate(address,uint256)", "50000000000000000000") is None


# ── dispatch / reconciler fakes ──────────────────────────────────────────────
class FakeChain:
    def __init__(self, seated=(), committee_next=None, status_by_addr=None, stake_by_addr=None):
        self.seated = set(f"0xowner{i}" for i in seated)
        self._next = committee_next
        self.p = Runner(dry=True)
        self.staking_rt = "0xSTAKE"
        self.rpc = "http://localhost:8545"
        self.calls = []
        self.status_by_addr = dict(status_by_addr or {})   # addr -> status byte; default "1" (Active)
        self.stake_by_addr = dict(stake_by_addr or {})     # addr -> int stake; default 0

    def owner_addr(self, idx):
        return f"0xowner{idx}"

    def committee(self, epoch):
        if self._next is not None:
            return self._next
        return " ".join(sorted(self.seated))

    def committee_has(self, addr, epoch):
        return addr in self.seated

    def active_validators_length(self):
        return len(self.seated)

    def validator_status(self, addr):
        return self.status_by_addr.get(addr, "1")          # Active unless overridden

    def validator_stake(self, addr):
        return int(self.stake_by_addr.get(addr, 0))

    # rotation ops (record)
    def undelegate(self, idx, amount, key=None):
        self.calls.append(("undelegate", str(idx), str(amount)))

    def stake_delegate(self, idx, amount, key=None):
        self.calls.append(("stake_delegate", str(idx), str(amount)))

    def remove_validator(self, idx, voter_idx=None):
        self.calls.append(("remove_validator", str(idx)))

    def activate_bench(self, idx, stake, voter_idx=None):
        self.calls.append(("activate_bench", str(idx), str(stake)))

    def fund_blend(self, idx, amount):
        self.calls.append(("fund_blend", str(idx), str(amount)))

    def self_delegate_to(self, idx, target):
        self.calls.append(("self_delegate_to", str(idx), str(target)))

    def mint_identity(self, idx, mint_eth_wei):
        self.calls.append(("mint_identity", str(idx)))
        return 0                                           # 0 = minted (rc 2 would DEFER)

    # growth ops (must NOT be called in the rotation phase)
    def register_activate(self, idx, raise_cap, voter_idx=None):
        self.calls.append(("register_activate", str(idx)))

    def bench_promote(self, idx, voter_idx=None):
        self.calls.append(("bench_promote", str(idx)))

    def register_only(self, idx):
        self.calls.append(("register_only", str(idx)))


class FakeHP:
    def __init__(self):
        self.rebirths = []
        self.retasks = []

    def rebirth_identity(self, slot, idx):
        self.rebirths.append((slot, str(idx)))
        return True

    def retask_slot(self, container, idx):
        self.retasks.append((container, str(idx)))


class FakeRec:
    dkg_window_fragile = 0
    dkg_barrier_notready = ""

    def recovery_confirmed(self, v):
        return True

    def beacon_share_delta(self, idx):
        return 5


def _disp(chain=None, hp=None, act=None, **state_kw):
    cfg = SimConfig(validators=7, initial_committee=7, spares=2, rotation_slots=2)
    s = SimState(cfg=cfg)
    for k, v in state_kw.items():
        setattr(s, k, v)
    chain = chain or FakeChain(seated=range(7))
    d = Dispatcher(s, chain, act or strict_actuators(), hp or FakeHP(), FakeRec())
    d.top_stake_leader = lambda: ""
    d.beacon_share_delta = lambda i: 5
    return d, s, chain


_GROWN = dict(next_joiner=7, grow_landed=6,
              cur_committee=" ".join(f"0xowner{i}" for i in range(7)))


def _no_growth_calls(chain):
    kinds = {c[0] for c in chain.calls}
    return not (kinds & {"register_activate", "bench_promote", "register_only",
                         "set_active_validators_length"})


# ── the latch ────────────────────────────────────────────────────────────────
def test_growth_complete_latches_rotation_phase():
    """At full cap with growth landed, run_round flips rotation_phase and routes to phase-2."""
    d, s, _ = _disp(bench_provisioned=1, **_GROWN)   # skip provisioning noise
    s.settle_until_epoch = 0
    assert s.rotation_phase == 0
    d.run_round(5, 7)
    assert s.rotation_phase == 1


def test_below_cap_stays_growth_phase():
    """Below cap the latch stays off — the growth-phase machinery still owns the round."""
    d, s, _ = _disp(next_joiner=6, grow_landed=5,
                    cur_committee=" ".join(f"0xowner{i}" for i in range(6)))
    d.run_round(5, 6)
    assert s.rotation_phase == 0


# ── the three permanent departures: stake/status op + rekey, no micro-management ─
def test_rotation_voluntary_undelegates_and_marks_rekey():
    d, s, chain = _disp(rotation_phase=1, bench_provisioned=1, **_GROWN)
    d._rotation_apply(20, "voluntary_exit", "validator-3")
    assert ("undelegate", "3", s.cfg.voluntary_undelegate_wei) in chain.calls
    assert "validator-3" in s.rekey_pending and s.rekey_pending["validator-3"]["cause"] == "voluntary"
    assert not s.seat.pending_backfill_by_seat          # NO obligation enqueued
    assert _no_growth_calls(chain)


def test_rotation_byzantine_marks_rekey_no_obligation():
    act = strict_actuators()
    d, s, chain = _disp(act=act, rotation_phase=1, bench_provisioned=1, **_GROWN)
    d._rotation_apply(20, "byzantine_equivocate", "validator-4")
    act.act_byzantine.assert_called_once_with("validator-4", "equivocate")
    assert s.identity.permanently_dead.get("validator-4") == 1
    assert s.rekey_pending["validator-4"]["cause"] == "byzantine"
    assert not s.seat.pending_backfill_by_seat          # the a18 obligation machine is NOT used
    assert _no_growth_calls(chain)


def test_rotation_eviction_boosts_bench_and_snapshots_no_drawn_victim_rekey():
    """v61 a22 FIX-3: stake-eviction boosts a warm bench above the (uniform) committee and RECORDS a
    committee snapshot in EVICT_PENDING. It does NOT pre-mark the drawn victim for rekey — under
    uniform stakes the chain drops the weakest incumbent, not the drawn one; process_evictions
    resolves the emergent victim on landing."""
    d, s, chain = _disp(rotation_phase=1, bench_provisioned=1, **_GROWN)
    d._rotation_apply(20, "delegate_shift", "validator-2")
    evicts = [c for c in chain.calls if c[0] == "stake_delegate"]
    assert evicts and evicts[0][2] == s.cfg.evict_stake_wei
    bench = int(evicts[0][1])
    assert bench in range(7, 11)                          # boosted a bench-band idx
    assert "validator-2" not in s.rekey_pending          # NO drawn-victim rekey (the a21 bug)
    ep = s.evict_pending[str(bench)]                      # snapshot recorded for the reconciler
    assert int(ep["bench_idx"]) == bench and ep["epoch"] == 20
    assert set(ep["snapshot"]) == {f"0xowner{i}" for i in range(7)}
    assert _no_growth_calls(chain)


# ── EVER_FAULTED: the rotation phase stamps its own inline arms ──────────────
# Regression bundle-20260730T071018Z: ever_faulted had a single writer, apply_action — which
# _rotation_apply only reaches on its `else` arm. A rotation-phase equivocation was therefore
# slashed correctly by the product and then failed the run on wrongful-slash, because the victim
# identity was never in the expectation set. test_apply.py:133 covered the OTHER path, which is
# exactly why this shipped uncaught.
def test_rotation_byzantine_stamps_ever_faulted():
    d, s, _ = _disp(rotation_phase=1, bench_provisioned=1, **_GROWN)
    d._rotation_apply(20, "byzantine_equivocate", "validator-4")
    assert s.identity.ever_faulted[s.ident_idx("validator-4")] == "byzantine_equivocate"


def test_rotation_voluntary_stamps_ever_faulted():
    d, s, _ = _disp(rotation_phase=1, bench_provisioned=1, **_GROWN)
    d._rotation_apply(20, "voluntary_exit", "validator-3")
    assert s.identity.ever_faulted[s.ident_idx("validator-3")] == "voluntary_exit"


def test_rotation_voluntary_stamps_ever_faulted_even_when_the_undelegate_defers():
    """ORDERING: the stamp precedes the arms, so a deferred chain write (which abandons the
    departure for this round and returns early) still leaves the identity in the expectation set —
    an attempted fault is one the identity may be jailed for before the retry lands."""
    class _FailUndelegate(FakeChain):
        def undelegate(self, idx, amount, key=None):
            raise ChainError("send", f"undelegate reverted for v{idx}")

    d, s, chain = _disp(chain=_FailUndelegate(seated=range(7)),
                        rotation_phase=1, bench_provisioned=1, **_GROWN)
    d._rotation_apply(20, "voluntary_exit", "validator-3")
    assert s.voluntary_exits == 0                          # the departure itself was abandoned
    assert "validator-3" not in s.rekey_pending
    assert s.identity.ever_faulted[s.ident_idx("validator-3")] == "voluntary_exit"


def test_rotation_eviction_stamps_no_ever_faulted():
    """An out-ranked incumbent is NOT faulted: it is returned to the bench unslashed, never jailed.
    Stamping it would hand wrongful-slash a false expectation for an honest identity, so the drawn
    delegate_shift victim stays out of the ledger (and the emergent one never enters it either)."""
    d, s, _ = _disp(rotation_phase=1, bench_provisioned=1, **_GROWN)
    d._rotation_apply(20, "delegate_shift", "validator-2")
    assert s.identity.ever_faulted == {}


def test_rotation_transient_stamps_ever_faulted_via_apply_action():
    """The `else` arm keeps EXACTLY ONE writer — apply_action's own stamp, not a second one here."""
    d, s, _ = _disp(rotation_phase=1, bench_provisioned=1, **_GROWN)
    d._rotation_apply(20, "cpu_throttle", "validator-5")
    assert s.identity.ever_faulted[s.ident_idx("validator-5")] == "cpu_throttle"


# ── FIX-1: activate_bench idempotency (real Chain oracle, dispatch on status byte) ──
def test_activate_bench_noop_when_already_active():
    """status 1 (Active) → fully provisioned already: NO register, NO activate, NO re-delegate."""
    c, r = _chain(**{"cast call":
                     "0x000000000000000000000000000000000000dEaD 1 5000000000000000000 0 0 0 0 0"})
    c.activate_bench(12, "4000000000000000000")
    assert _find(r, "cast", "send", "registerValidator") is None
    assert _find(r, "cast", "calldata", "activateValidator(address)") is None
    assert _find(r, "cast", "send", "delegate(address,uint256)") is None


def test_activate_bench_pending_skips_register_but_activates():
    """status 2 (Pending, e.g. a gov-defeated activate) → SKIP register, retry activate + stake."""
    c, r = _chain()   # default read → status byte 2 (Pending)
    c.activate_bench(11, "4000000000000000000")
    assert _find(r, "cast", "send", "registerValidator") is None    # NOT re-registered
    assert _find(r, "cast", "calldata", "activateValidator(address)")
    assert _find(r, "cast", "send", "delegate(address,uint256)", "4000000000000000000")


def test_activate_bench_notfound_full_register_path():
    """status 0 (NotFound) → the full path STARTS with register (register_setkeys). The static dry
    read can't flip 0→2 for its post-register assert, so we assert register was ATTEMPTED (the
    NotFound branch was taken, unlike the Active/Pending short-circuits)."""
    c, r = _chain(**{"cast call": "0x000000000000000000000000000000000000dEaD 0"})
    with pytest.raises(ChainError):
        c.activate_bench(13, "4000000000000000000")
    assert _find(r, "cast", "send", "registerValidator(address,uint16,uint256)")


def test_rotation_transient_stays_reversible_no_rekey():
    act = strict_actuators()
    d, s, chain = _disp(act=act, rotation_phase=1, bench_provisioned=1, **_GROWN)
    d._rotation_apply(20, "cpu_throttle", "validator-5")
    act.act_cpu_throttle.assert_called_once_with("validator-5")
    assert "validator-5" not in s.rekey_pending          # transient → same identity, no rekey
    assert "validator-5" in s.disrupted.split()          # held against-f, reversible
    assert _no_growth_calls(chain)


def test_provision_warm_bench_stratifies_two_bands_no_cap_raise():
    """v61 a23 BELT: provisioning is paced ONE step per round (provision_cursor). Driving the belt to
    completion, every committee member (0..validators-1) is funded + self-delegated UP to the Hi
    band; every warm bench (validators..val_containers) is activated at the Lo band (bench_lo total =
    1e18 register + this delegate). One step advances the cursor; latch on the final step. NO growth
    op (register_activate / bench_promote / setActiveValidatorsLength) is ever issued."""
    d, s, chain = _disp(rotation_phase=1, **_GROWN)
    # each committee-stratify step must fund+self-delegate exactly ONE member, in cursor order
    d.provision_warm_bench(5)
    assert sorted(int(c[1]) for c in chain.calls if c[0] == "fund_blend") == [0]   # one member/round
    # drive the belt to completion (validators + benches + 1 latch step)
    for _ in range(s.cfg.val_containers + 2):
        if s.bench_provisioned:
            break
        d.provision_warm_bench(5)
    # committee → Hi (fund + self-delegate every member, once each)
    funded = sorted(int(c[1]) for c in chain.calls if c[0] == "fund_blend")
    self_deleg = sorted(int(c[1]) for c in chain.calls if c[0] == "self_delegate_to")
    assert funded == [0, 1, 2, 3, 4, 5, 6]
    assert self_deleg == [0, 1, 2, 3, 4, 5, 6]
    assert all(c[2] == s.cfg.committee_hi_wei for c in chain.calls if c[0] == "self_delegate_to")
    assert all(c[2] == s.cfg.member_fund_wei for c in chain.calls if c[0] == "fund_blend")
    # bench → Lo (delegate = bench_lo - 1e18 register)
    activated = sorted(int(c[1]) for c in chain.calls if c[0] == "activate_bench")
    assert activated == [7, 8, 9, 10]                    # validators..val_containers
    bench_delegate = str(int(s.cfg.bench_lo_wei) - 10 ** 18)
    assert all(c[2] == bench_delegate for c in chain.calls if c[0] == "activate_bench")
    # band invariant holds for the configured defaults
    assert int(s.cfg.bench_lo_wei) < int(s.cfg.committee_hi_wei)
    assert _no_growth_calls(chain)
    assert s.bench_provisioned == 1


# ── the rotation reconciler: rekey once departed, stall when overdue ──────────
def _rec(chain, hp, **state_kw):
    cfg = SimConfig(validators=7, initial_committee=7, spares=2, rotation_slots=2)
    s = SimState(cfg=cfg)
    for k, v in state_kw.items():
        setattr(s, k, v)
    rec = Reconcilers(s, chain, strict_actuators(), hostpool=hp, events=[])
    return rec, s


def test_process_rotations_rekeys_when_departed_left_committee():
    # idx-4 departed and is NO LONGER seated (committee = 0..3,5,6 + a bench) → rekey validator-4.
    chain = FakeChain(seated=[0, 1, 2, 3, 5, 6])
    hp = FakeHP()
    rec, s = _rec(chain, hp, rotation_phase=1)
    s.next_rekey_idx = s.cfg.val_containers               # 11
    s.rekey_pending = {"validator-4": {"epoch": 20, "cause": "byzantine", "idx": "4"}}
    s.identity.permanently_dead["validator-4"] = 1
    s.disrupted = "validator-4"
    rec.process_rotations(25)
    assert hp.rebirths == [("validator-4", "11")]         # reborn under the fresh pool key
    bench_delegate = str(int(s.cfg.bench_lo_wei) - 10 ** 18)   # rejoin at the Lo band, not the legacy tier
    assert ("activate_bench", "11", bench_delegate) in chain.calls
    assert s.next_rekey_idx == 12
    assert "validator-4" not in s.rekey_pending
    assert "validator-4" not in s.identity.permanently_dead


def test_process_rotations_defers_while_departed_still_seated():
    # idx-4 still seated (not yet re-selected out) and within horizon → no rekey, no stall.
    chain = FakeChain(seated=range(7))
    hp = FakeHP()
    rec, s = _rec(chain, hp, rotation_phase=1)
    s.rekey_pending = {"validator-4": {"epoch": 24, "cause": "voluntary", "idx": "4"}}
    rec.process_rotations(25)                              # 25-24=1 <= confirm horizon
    assert hp.rebirths == []
    assert "validator-4" in s.rekey_pending
    assert not any("rotation-stall" in str(e) for e in rec.events)


def test_process_rotations_emits_stall_when_overdue():
    chain = FakeChain(seated=range(7))                    # idx-4 STILL seated way past the horizon
    hp = FakeHP()
    rec, s = _rec(chain, hp, rotation_phase=1)
    s.rekey_pending = {"validator-4": {"epoch": 10, "cause": "voluntary", "idx": "4"}}
    rec.process_rotations(10 + s.cfg.rotation_confirm_epochs + 1)
    assert any("rotation-stall" in str(e) for e in rec.events)
    assert hp.rebirths == []                              # diagnostic only — no rekey forced


# ── a27: byzantine departure fallback — force removeValidator for a starved equivocator ──
def test_process_rotations_forces_removal_for_starved_byzantine():
    """v61 a27: a byzantine victim that never won a proposal slot to equivocate stays seated (the
    slasher saw no evidence). Past the horizon, force removeValidator (latched) so the rekey proceeds."""
    chain = FakeChain(seated=range(7))                    # idx-4 STILL seated past the horizon
    hp = FakeHP()
    rec, s = _rec(chain, hp, rotation_phase=1)
    s.rekey_pending = {"validator-4": {"epoch": 10, "cause": "byzantine", "idx": "4"}}
    rec.process_rotations(10 + s.cfg.rotation_confirm_epochs + 1)
    assert ("remove_validator", "4") in chain.calls
    assert s.rekey_pending["validator-4"].get("force_removed") is not None
    assert hp.rebirths == []                              # still seated → not yet rekeyed


def test_process_rotations_forced_removal_is_idempotent():
    """The gov removeValidator round is issued exactly once (force_removed latch)."""
    chain = FakeChain(seated=range(7))
    hp = FakeHP()
    rec, s = _rec(chain, hp, rotation_phase=1)
    s.rekey_pending = {"validator-4": {"epoch": 10, "cause": "byzantine", "idx": "4"}}
    rec.process_rotations(10 + s.cfg.rotation_confirm_epochs + 1)
    rec.process_rotations(10 + s.cfg.rotation_confirm_epochs + 2)
    assert sum(1 for c in chain.calls if c[0] == "remove_validator") == 1


def test_process_rotations_voluntary_stall_not_force_removed():
    """The force-removal fallback is byzantine-ONLY; a voluntary stall stays a plain diagnostic."""
    chain = FakeChain(seated=range(7))
    hp = FakeHP()
    rec, s = _rec(chain, hp, rotation_phase=1)
    s.rekey_pending = {"validator-4": {"epoch": 10, "cause": "voluntary", "idx": "4"}}
    rec.process_rotations(10 + s.cfg.rotation_confirm_epochs + 1)
    assert not any(c[0] == "remove_validator" for c in chain.calls)
    assert any("rotation-stall" in str(e) for e in rec.events)


def test_process_rotations_rekeys_after_forced_removal():
    """Once forced removeValidator lands and the identity leaves the committee, the normal byzantine
    rekey fires (consumes a fresh key — byzantine is the permanent consumer)."""
    chain = FakeChain(seated=range(7))                    # idx-4 seated
    hp = FakeHP()
    rec, s = _rec(chain, hp, rotation_phase=1)
    s.next_rekey_idx = s.cfg.val_containers               # 11
    s.rekey_pending = {"validator-4": {"epoch": 10, "cause": "byzantine", "idx": "4"}}
    rec.process_rotations(10 + s.cfg.rotation_confirm_epochs + 1)   # force removeValidator (still seated)
    chain.seated = {f"0xowner{i}" for i in [0, 1, 2, 3, 5, 6]}      # idx-4 now dropped by the chain
    rec.process_rotations(10 + s.cfg.rotation_confirm_epochs + 2)
    assert hp.rebirths == [("validator-4", "11")]         # now rekeyed under a fresh pool key
    assert s.next_rekey_idx == 12
    assert "validator-4" not in s.rekey_pending


# ── a25: voluntary exits RECYCLE in place (no fresh key); byzantine still rekeys ──
def test_process_rotations_returns_voluntary_departure_under_own_key():
    """v61 a25: a VOLUNTARY departure that has left the committee is RETURNED to the warm bench under
    its OWN key (self-delegate up to Lo) — NOT rekeyed. No rebirth, and the fresh-key supply cursor
    is NOT advanced (voluntary recycles in place; only byzantine consumes a fresh key)."""
    chain = FakeChain(seated=[0, 1, 2, 3, 5, 6])          # idx-4 departed (not seated)
    hp = FakeHP()
    rec, s = _rec(chain, hp, rotation_phase=1)
    s.next_rekey_idx = s.cfg.val_containers               # 11 — must stay put
    s.rekey_pending = {"validator-4": {"epoch": 20, "cause": "voluntary", "idx": "4"}}
    s.container.disrupt_kind["validator-4"] = "voluntary_exit"
    rec.process_rotations(25)
    # re-funded to the member floor (undelegate parks stake in unbonding, not spendable balance) so a
    # future Hi-boost is affordable, THEN restaked to Lo under its own key
    assert ("fund_blend", "4", s.cfg.member_fund_wei) in chain.calls
    assert ("self_delegate_to", "4", s.cfg.bench_lo_wei) in chain.calls   # back to Lo, own key
    assert hp.rebirths == []                              # NO rebirth (same identity)
    assert s.next_rekey_idx == s.cfg.val_containers       # fresh-key supply UNTOUCHED
    assert "validator-4" not in s.rekey_pending           # handled
    assert "validator-4" not in s.container.disrupt_kind  # cleaned up


def test_replenish_key_supply_noop_when_headroom_sufficient():
    """a27 top-up: with >= min_key_headroom ready-to-consume keys available, nothing is minted."""
    chain = FakeChain(seated=range(7))
    rec, s = _rec(chain, FakeHP(), rotation_phase=1, bench_provisioned=1)
    s.next_rekey_idx = s.cfg.val_containers               # available = full band >= min_headroom
    assert s.cfg.identity_pool - s.next_rekey_idx >= rec.min_key_headroom
    pool0 = s.cfg.identity_pool
    rec.replenish_key_supply(30)
    assert not any(c[0] == "mint_identity" for c in chain.calls)
    assert s.cfg.identity_pool == pool0


def test_replenish_key_supply_mints_one_when_headroom_below_min():
    """a27 top-up: when available headroom drops below min_key_headroom, mint ONE new pool identity
    (belt-paced, one per tick)."""
    chain = FakeChain(seated=range(7))
    rec, s = _rec(chain, FakeHP(), rotation_phase=1, bench_provisioned=1)
    pool0 = s.cfg.identity_pool
    s.next_rekey_idx = pool0 - 1                          # available = 1 < min_headroom (3)
    rec.replenish_key_supply(30)
    assert ("mint_identity", str(pool0)) in chain.calls   # minted the next idx beyond the pool
    assert s.cfg.identity_pool == pool0 + 1
    assert s.max_minted_idx == pool0                       # identity_pool - 1


def test_replenish_key_supply_maintains_fixed_headroom_not_proportional():
    """a27 FIX: the reserve is a FIXED headroom, NOT proportional to cumulative usage. Even with the
    band fully drained, repeated ticks converge identity_pool to exactly next_rekey_idx +
    min_key_headroom and then STOP — proving the pool does not grow with cumulative spent."""
    chain = FakeChain(seated=range(7))
    rec, s = _rec(chain, FakeHP(), rotation_phase=1, bench_provisioned=1)
    s.next_rekey_idx = s.cfg.identity_pool               # available = 0 (fully drained)
    fixed = s.next_rekey_idx
    for _ in range(10):                                   # belt: one mint/tick until headroom restored
        rec.replenish_key_supply(30)
    assert s.cfg.identity_pool == fixed + rec.min_key_headroom      # converged to the FIXED floor
    assert sum(1 for c in chain.calls if c[0] == "mint_identity") == rec.min_key_headroom  # not more


def test_replenish_key_supply_gated_off_before_stratified():
    """a25 top-up runs only once rotation is armed (bench_provisioned)."""
    chain = FakeChain(seated=range(7))
    rec, s = _rec(chain, FakeHP(), rotation_phase=1, bench_provisioned=0)
    s.next_rekey_idx = s.cfg.identity_pool                # fully spent
    rec.replenish_key_supply(30)
    assert not any(c[0] == "mint_identity" for c in chain.calls)


# ── FIX-2: a rekey rebuilds ADDR2IDX so the fresh identity is visible ─────────
class _HPRetask(FakeHP):
    """retask_slot that also mutates SLOT_IDENTITY (as the real HostPool does) so build_addr_map
    can resolve the re-tasked container."""
    def __init__(self, state):
        super().__init__()
        self.state = state

    def retask_slot(self, container, idx):
        super().retask_slot(container, idx)
        self.state.container.slot_identity[container] = str(idx)


def test_process_rotations_rebuilds_addr2idx_after_rekey():
    chain = FakeChain(seated=[0, 1, 2, 3, 5, 6])          # idx-4 departed (not seated)
    cfg = SimConfig(validators=7, initial_committee=7, spares=2, rotation_slots=2)
    s = SimState(cfg=cfg)
    s.rotation_phase = 1
    s.next_rekey_idx = s.cfg.val_containers               # 11
    hp = _HPRetask(s)
    rec = Reconcilers(s, chain, strict_actuators(), hostpool=hp, events=[])
    s.rekey_pending = {"validator-4": {"epoch": 20, "cause": "byzantine", "idx": "4"}}
    s.identity.permanently_dead["validator-4"] = 1
    rec.process_rotations(25)
    # fresh idx 11 is now served by validator-4's container → its owner-addr resolves in ADDR2IDX
    assert chain.owner_addr(11).lower() in s.address.addr2idx
    assert s.address.addr2idx[chain.owner_addr(11).lower()] == "validator-4"


# ── FIX-3: process_evictions resolves the EMERGENT victim, undelegates it, rekeys it ─
def test_process_evictions_recycles_the_member_that_actually_left():
    """v61 a27: a stake-eviction victim is NON-malicious (out-ranked, never slashed) → RECYCLED under
    its own key like a voluntary exit (undelegate below the tier, then re-fund + self-delegate to Lo),
    NOT rekeyed. No rebirth, no fresh-key consumed."""
    # apply-time committee snapshot = idx 0..6; after landing idx-3 is gone and bench idx-8 entered.
    chain = FakeChain(seated=[0, 1, 2, 4, 5, 6, 8])
    hp = FakeHP()
    rec, s = _rec(chain, hp, rotation_phase=1)
    s.next_rekey_idx = s.cfg.val_containers               # 11 — must stay put
    snapshot = [f"0xowner{i}" for i in range(7)]
    s.evict_pending = {"8": {"epoch": 20, "bench_idx": 8, "snapshot": snapshot,
                             "addr_idx": {a: f"validator-{i}" for i, a in enumerate(snapshot)}}}
    rec.process_evictions(25)
    # the emergent victim (idx-3) is undelegated below the tier, THEN returned to the Lo bench
    assert ("undelegate", "3", s.cfg.voluntary_undelegate_wei) in chain.calls
    assert ("fund_blend", "3", s.cfg.member_fund_wei) in chain.calls
    assert ("self_delegate_to", "3", s.cfg.bench_lo_wei) in chain.calls
    assert hp.rebirths == []                              # NO rebirth (recycled, not rekeyed)
    assert s.next_rekey_idx == s.cfg.val_containers       # fresh-key supply UNTOUCHED
    assert "8" not in s.evict_pending                     # resolved


def test_process_evictions_stall_when_no_member_left():
    # bench idx-8 entered but the snapshot committee is fully intact (nobody dropped) past horizon.
    chain = FakeChain(seated=[0, 1, 2, 3, 4, 5, 6, 8])
    hp = FakeHP()
    rec, s = _rec(chain, hp, rotation_phase=1)
    snapshot = [f"0xowner{i}" for i in range(7)]
    s.evict_pending = {"8": {"epoch": 10, "bench_idx": 8, "snapshot": snapshot,
                             "addr_idx": {a: f"validator-{i}" for i, a in enumerate(snapshot)}}}
    rec.process_evictions(10 + s.cfg.rotation_confirm_epochs + 1)
    assert any("rotation-stall" in str(e) for e in rec.events)
    assert hp.rebirths == []


# ── a24: bench_join disabled, committee-Hi maintenance, eviction Active-guard ──

def test_bench_join_track_disabled_in_growth():
    """v61 a24: the OLD bench_join/mint supply track is disabled — even with every firing condition
    met, _growth_refill_promote_bench falls through (returns None) and never mints or advances
    next_bench_joiner. The warm bench comes from provision_warm_bench; fresh keys from rekey."""
    d, s, chain = _disp(next_joiner=7, grow_landed=6)     # growth done, rotation_phase=0
    s.next_bench_joiner = s.cfg.val_containers            # a valid mint cursor (< max_mint_idx)
    s.last_bench_join_round = 0
    s.round = 1000                                        # bench_join_due: round - last >= gap
    s.promotable = ""                                     # count 0 < mint_buffer
    before = s.next_bench_joiner
    v = d._growth_refill_promote_bench(5, s.cfg.validators, 1, 0)
    assert v is None                                      # bench-join elif disabled → falls through
    assert s.next_bench_joiner == before                 # cursor NOT advanced (no mint)
    assert not any(c[0] in ("mint_identity", "register_only") for c in chain.calls)


def test_maintain_committee_hi_boosts_below_hi_seated_only():
    """v61 a24: each tick, a SEATED committee member below the Hi band is self-delegated UP to
    committee_hi_wei (a just-promoted Lo bench); a member already at/above Hi is a no-op; a
    non-seated identity is untouched. Keeps the committee uniform Hi so the bench stays cleanly below."""
    hi = int(SimConfig().committee_hi_wei)
    # idx-2 seated & LOW (needs boost); idx-3 seated & already Hi (no-op); idx-9 NOT seated
    chain = FakeChain(seated=[2, 3],
                      stake_by_addr={"0xowner2": 5 * 10**18, "0xowner3": hi, "0xowner9": 5 * 10**18})
    rec, s = _rec(chain, FakeHP(), rotation_phase=1, bench_provisioned=1)
    s.address.addr2idx = {"0xowner2": "validator-2", "0xowner3": "validator-3",
                          "0xowner9": "validator-9"}
    rec.maintain_committee_hi(50)
    deleg = [c for c in chain.calls if c[0] == "self_delegate_to"]
    assert deleg == [("self_delegate_to", "2", str(hi))]  # only the below-Hi SEATED member


def test_maintain_committee_hi_noop_before_stratified():
    """v61 a24 FIX: gated on bench_provisioned, not rotation_phase. During the belt (rotation_phase on
    but bench_provisioned still off) the belt OWNS the stakes — maintain_committee_hi must NOT run, or
    it spams 0-balance defers for genesis members the belt has not funded yet."""
    hi = int(SimConfig().committee_hi_wei)
    chain = FakeChain(seated=[2], stake_by_addr={"0xowner2": 5 * 10**18})
    rec, s = _rec(chain, FakeHP(), rotation_phase=1, bench_provisioned=0)
    s.address.addr2idx = {"0xowner2": "validator-2"}
    rec.maintain_committee_hi(50)
    assert not any(c[0] == "self_delegate_to" for c in chain.calls)


def test_maintain_committee_hi_keeps_anchors_above_eviction_reach():
    """v61 a26: pinned anchors (validator-0/1) are held at anchor_stake_wei (> bench_lo+evict_stake=
    60e18) so the emergent-victim eviction never makes one the weakest incumbent — the chain can
    never drop/rekey/restart the host-RPC + spec-derive node. Non-anchors stay at committee_hi; the
    anchor is funded to clear the higher band."""
    chain = FakeChain(seated=[0, 2],
                      stake_by_addr={"0xowner0": int(SimConfig().committee_hi_wei),
                                     "0xowner2": 5 * 10**18})
    rec, s = _rec(chain, FakeHP(), rotation_phase=1, bench_provisioned=1)
    s.address.addr2idx = {"0xowner0": "validator-0", "0xowner2": "validator-2"}
    rec.maintain_committee_hi(50)
    deleg = {c[1]: c[2] for c in chain.calls if c[0] == "self_delegate_to"}
    assert deleg.get("0") == s.cfg.anchor_stake_wei             # anchor → anchor band (above eviction)
    assert deleg.get("2") == s.cfg.committee_hi_wei             # non-anchor → Hi
    assert ("fund_blend", "0", s.cfg.anchor_fund_wei) in chain.calls   # anchor funded for the band
    assert int(s.cfg.anchor_stake_wei) > int(s.cfg.bench_lo_wei) + int(s.cfg.evict_stake_wei)


def test_process_evictions_reseat_anchor_not_rekey():
    """v61 a26: if the emergent eviction victim is an anchor, it is RE-SEATED under its own key
    (fund + self-delegate to the anchor band), NOT rekeyed — the container is never rebirthed."""
    chain = FakeChain(seated=[1, 2, 3, 4, 5, 6, 8])         # anchor idx-0 left snapshot; bench-8 entered
    hp = FakeHP()
    rec, s = _rec(chain, hp, rotation_phase=1)
    s.next_rekey_idx = s.cfg.val_containers
    snapshot = [f"0xowner{i}" for i in range(7)]
    s.evict_pending = {"8": {"epoch": 20, "bench_idx": 8, "snapshot": snapshot,
                             "addr_idx": {a: f"validator-{i}" for i, a in enumerate(snapshot)}}}
    rec.process_evictions(25)
    assert hp.rebirths == []                                 # anchor NOT rebirthed/restarted
    assert ("fund_blend", "0", s.cfg.anchor_fund_wei) in chain.calls
    assert ("self_delegate_to", "0", s.cfg.anchor_stake_wei) in chain.calls
    assert not any(c[0] == "undelegate" for c in chain.calls)   # anchor NOT undelegated either
    assert "8" not in s.evict_pending                        # resolved


def test_maintain_committee_hi_skips_departing_victim():
    """v61 a24 FIX (critical): a below-Hi seated member marked rekey_pending is a DEPARTING victim
    (byzantine/voluntary) that undelegated below the tier ON PURPOSE. maintain_committee_hi must SKIP
    it — re-boosting it back to Hi re-delegates the freed stake and cancels the rotation (permanent
    rekey stall). idx-2 departing (skip); idx-4 a genuine below-Hi promoted bench (boost)."""
    hi = int(SimConfig().committee_hi_wei)
    chain = FakeChain(seated=[2, 4],
                      stake_by_addr={"0xowner2": 4 * 10**18, "0xowner4": 5 * 10**18})
    rec, s = _rec(chain, FakeHP(), rotation_phase=1, bench_provisioned=1)
    s.address.addr2idx = {"0xowner2": "validator-2", "0xowner4": "validator-4"}
    s.rekey_pending = {"validator-2": {"epoch": 20, "cause": "voluntary", "idx": "2"}}
    rec.maintain_committee_hi(50)
    deleg = [c for c in chain.calls if c[0] == "self_delegate_to"]
    assert deleg == [("self_delegate_to", "4", str(hi))]  # departing validator-2 NOT re-boosted


def test_pick_bench_for_evict_skips_non_active():
    """v61 a24: _pick_bench_for_evict skips a non-Active (Pending/Jail) bench and returns the first
    ACTIVE, non-seated one — only an Active staked bench can be boosted into the top-k."""
    # bench band = validators..val_containers = 7..10 (cfg validators=7, val_containers=11)
    chain = FakeChain(seated=range(7),
                      status_by_addr={"0xowner7": "2", "0xowner8": "1"})   # 7 Pending, 8 Active
    d, s, _ = _disp(chain=chain, rotation_phase=1)
    s.cur_epoch = 5
    assert d._pick_bench_for_evict("validator-99") == "8"  # skips Pending 7 → Active 8


# ── catch symmetry: _rotation_chain_write only defers on a ChainError ────────
def test_rotation_chain_write_defers_on_chain_error():
    d, _s, _c = _disp(rotation_phase=1)
    d.events = []
    def boom():
        raise ChainError("send-reverted", "disableValidator reverted")
    assert d._rotation_chain_write(boom, "voluntary_exit validator-4") is False
    assert any("rotation chain-write deferred" in n for _k, n in d.events)


def test_rotation_chain_write_propagates_a_harness_bug():
    """Follow-up 3 (2026-07-30): this caught bare `Exception`, so a TypeError/AttributeError in the
    rotation write path was swallowed as "chain-write deferred" — a broken path would log a churn
    line every round and the run would still end green. Matches orchestrator._apply_chain_write,
    which catches ChainError only. Do NOT loosen either back."""
    d, _s, _c = _disp(rotation_phase=1)
    d.events = []
    def bug():
        raise AttributeError("Chain has no attribute 'undelagate'")
    with pytest.raises(AttributeError):
        d._rotation_chain_write(bug, "voluntary_exit validator-4")
    assert d.events == []
