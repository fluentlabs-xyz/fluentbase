"""test_dispatch_round.py — the full per-round churn dispatch chain (case-soak.sh main loop 2249-
3118). Drives Dispatcher.run_round with a stubbed Chain + beacon/reconciler seams and asserts each
track fires in the right ORDER under the right state (growth/refill/promote/bench-mint/serialize/
lottery), and that the gate decision matches the ported gate-matrix semantics.

Where the bash side cannot be subprocess-driven for these decisions (they are pure state-machine
choices, not shell-outs), the assertion is pinned against the ported gate-matrix / track-precedence
semantics — analysis §3.3: the pure-logic keepers ARE the equivalence proof for the round flow."""

import pytest

from dpos_harness.sim.orchestrator import SimConfig, SimState
from dpos_harness.sim.dispatch import Dispatcher
from dpos_harness.chain.writes import ChainError
from dpos_harness.core.proc import Runner
from dpos_harness.tests.conftest import strict_actuators


class FakeChain:
    def __init__(self, seated=(), committee_next=""):
        self.seated = set(str(s) for s in seated)
        self.committee_next = committee_next
        self.p = Runner(dry=True)
        self.staking_rt = "0xSTAKE"
        self.rpc = "http://localhost:8545"
        self.calls = []

    def owner_addr(self, idx):
        return f"0xowner{idx}"

    def committee_has(self, addr, epoch):
        return addr.replace("0xowner", "") in self.seated

    def committee(self, epoch):
        return self.committee_next

    def active_validators_length(self):
        return len(self.seated)

    def _pp_cfg_read_retry(self, sig):
        return None

    def register_activate(self, idx, raise_cap, voter_idx=None):
        self.calls.append(("register_activate", idx, raise_cap))

    def register_only(self, idx):
        self.calls.append(("register_only", idx))

    def bench_promote(self, idx, voter_idx=None):
        self.calls.append(("bench_promote", idx))

    def mint_identity(self, idx, wei):
        self.calls.append(("mint_identity", idx))
        return 0


class FakeHP:
    def __init__(self, host_avail=True):
        self._ha = host_avail
        self.warmed = []

    def select_promote_candidate(self):
        from dpos_harness.sim import actions
        return actions.select_promote_candidate("", lambda i: False, lambda i: False)

    def warm_identity(self, idx):
        self.warmed.append(idx)
        return True

    def promote_host_avail(self, idx):
        return self._ha

    def add_source_avail(self):
        return 1

    def recyclable_containers(self):
        return []


class FakeRec:
    dkg_window_fragile = 0
    dkg_barrier_notready = ""

    def finalized_dec(self):
        return 100

    def node_finalized(self, v):
        return 100

    def recovery_confirmed(self, v):
        return True

    def beacon_share_delta(self, idx):
        return 5

    def _dkg_qual_state(self, epoch):
        return -1


def _disp(chain=None, hp=None, **state_kw):
    cfg = SimConfig(validators=7, initial_committee=7, spares=2, rotation_slots=2)
    s = SimState(cfg=cfg)
    for k, v in state_kw.items():
        setattr(s, k, v)
    chain = chain or FakeChain(seated=range(7))
    d = Dispatcher(s, chain, strict_actuators(), hp or FakeHP(), FakeRec())
    d.beacon_share_delta = lambda i: 5
    d.beacon_metric = lambda i, f: 0
    d.top_stake_leader = lambda: ""
    d.refill_spare_attempts = lambda v: 0
    return d, s, chain


# ── track precedence + firing ────────────────────────────────────────────────

def test_growth_track_fires_below_target():
    """GROWTH: next_joiner < validators, settled, clean, no promote in flight → register_activate
    WITH cap-raise (raise_cap=1), NEXT_JOINER advances (case-soak.sh:2522-2534)."""
    d, s, chain = _disp(next_joiner=5, grow_landed=4, cur_committee="0xowner0 0xowner1")
    s.settle_until_epoch = 0
    s.next_growth_epoch = 0
    v = d.run_round(5, 6)
    assert ("register_activate", 5, 1) in chain.calls
    assert s.next_joiner == 6 and v == "applied"


def test_refill_resume_then_activate():
    """REFILL mechanism B: RESUME the spare first (skipped), then once frontier-confirmed
    register_activate WITHOUT cap-raise (raise_cap=0) (case-soak.sh:2535-2627)."""
    chain = FakeChain(seated=range(6))               # n=6 < cap 7 → a hole
    d, s, _ = _disp(chain=chain, next_joiner=7, grow_landed=6, tombstones=1,
                    cur_committee=" ".join(f"0xowner{i}" for i in range(6)))
    s.settle_until_epoch = 0
    v1 = d.run_round(5, 6)
    assert v1 == "skipped" and s.refill_resumed == "validator-7"
    v2 = d.run_round(5, 6)
    assert ("register_activate", 7, 0) in chain.calls
    assert v2 == "applied" and s.refill_pending == "7"


def test_serialize_settle_blocks_fault_lottery():
    """SERIALIZE (case-soak.sh:2837): inside the settle window the fault lottery is suppressed."""
    d, s, chain = _disp(next_joiner=7, grow_landed=6,
                        cur_committee=" ".join(f"0xowner{i}" for i in range(7)))
    s.settle_until_epoch = 100
    assert d.run_round(5, 7) == "skipped"
    assert chain.calls == []


def test_dkg_barrier_hold_skips_all_churn():
    """DKG-barrier HOLD (case-soak.sh:2385): all churn skipped while the window is fragile."""
    d, s, chain = _disp(cur_committee="0xowner0")
    d.rec.dkg_window_fragile = 1
    assert d.run_round(5, 7) == "skipped"
    assert chain.calls == []


def test_heal_before_harm_suppresses_lottery_when_backfill_open_below_cap():
    """HEAL-BEFORE-HARM (v61 a17): an OPEN backfill obligation on a BELOW-CAP committee suppresses
    the fault lottery so the vacated seat heals to full before more faults land. Without this the
    ~60% fault rate re-dirties the committee faster than transients clear, permanently gating the
    promote track ('committee DIRTY') → the seat never backfills and the committee bleeds to the
    min-committee floor instead of holding the cap."""
    d, s, chain = _disp(next_joiner=7, grow_landed=6,
                        cur_committee=" ".join(f"0xowner{i}" for i in range(7)))
    s.settle_until_epoch = 0
    s.seat.pending_backfill_by_seat = {"5": 3}         # an open backfill obligation (seat 5 vacated)
    v = d.run_round(5, 6)                               # n=6 < cap 7 → below cap
    assert v == "skipped"
    assert any("heal-before-harm" in msg for _, msg in d.events)
    assert chain.calls == []                           # no NEW fault drawn onto the healing committee


def test_no_heal_suppression_when_committee_at_cap():
    """The heal-before-harm skip is scoped to BELOW-cap: an obligation while the committee is already
    at cap (seat covered) must NOT suppress churn — otherwise a stale obligation would freeze the
    lottery forever."""
    d, s, chain = _disp(next_joiner=7, grow_landed=6,
                        cur_committee=" ".join(f"0xowner{i}" for i in range(7)))
    s.settle_until_epoch = 0
    s.seat.pending_backfill_by_seat = {"5": 3}
    d.run_round(5, 7)                                  # n=7 == cap → NOT below cap
    assert not any("heal-before-harm" in msg for _, msg in d.events)


def test_growth_incomplete_suppresses_lottery():
    """GROWTH-COMPLETE gate (v61 a19): a below-cap committee whose growth is already ISSUED
    (next_joiner == validators) but not yet fully landed, with NO backfill obligation, must suppress
    the fault lottery. Churn may not interleave with incomplete growth: a mid-growth departure sets
    bench_promote's promote_pending, which then deadlocks the cap-raise growth step (`not
    promote_pending`) → the on-chain cap freezes below `validators` and the backfill can never seat.
    Distinct from heal-before-harm (that path needs an OPEN obligation; this is the pre-growth,
    no-obligation window)."""
    chain = FakeChain(seated=range(6))                 # n=6 < cap 7
    d, s, _ = _disp(chain=chain, next_joiner=7, grow_landed=6,   # every growth step ISSUED, none pending
                    cur_committee=" ".join(f"0xowner{i}" for i in range(6)))
    s.settle_until_epoch = 0
    s.next_bench_joiner = 10 ** 9                        # bench supply idle → bench-join track won't fire
    v = d.run_round(5, 6)                               # n=6 < validators 7
    assert v == "skipped"
    assert any("growth-incomplete" in msg for _, msg in d.events)
    assert chain.calls == []                            # no fault drawn onto a still-growing committee


def test_promote_landing_hook_clears_pending():
    """PROMOTE landing hook (case-soak.sh:2268-2279): seated + qualified signer ⇒ PROMOTE_PENDING
    clears + oldest obligation clears. This is a GROWTH-phase mechanism (v61 a20: promote_pending
    only exists while growing to cap), so drive it BELOW cap (n=6 < validators 7) — at full cap the
    dispatcher latches into the chain-driven rotation phase where there is no promote track."""
    chain = FakeChain(seated=[20])
    d, s, _ = _disp(chain=chain, promote_pending="20", promote_share_base="0",
                    cur_committee="0xowner20")
    s.seat.pending_backfill_by_seat = {3: 2}
    d.beacon_share_delta = lambda i: 5               # delta 5-0 >= 1 → qualified
    d.run_round(5, 6)                                # n=6 < cap → growth phase → landing hook runs
    assert s.promote_pending == "" and 3 not in s.seat.pending_backfill_by_seat


def test_growth_landing_high_water():
    """GROW_LANDED is a HIGH-WATER latch: once the joiner is seen seated it locks (case-soak.sh:2249)."""
    chain = FakeChain(seated=[6])
    d, s, _ = _disp(chain=chain, next_joiner=7, grow_landed=5, last_growth_kind="growth",
                    cur_committee="0xowner6")
    s.settle_until_epoch = 0
    d.run_round(5, 7)
    assert s.grow_landed == 6


def test_warm_stint_budget_expires_and_resets():
    d, s, chain = _disp()
    d.promote_warm_max_rounds = 2
    d._warm_stint_charge(20)
    assert not d._warm_stint_expired(20)     # 1 < 2
    d._warm_stint_charge(20)
    assert d._warm_stint_expired(20)         # 2 >= 2 → expire + reset
    assert d.warm_stint[20] == 0
