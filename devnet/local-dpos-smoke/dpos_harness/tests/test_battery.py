"""Battery pure-logic + belt-state-machine tests (the ~65% portable gate sites).

Bash-idiom assertions (assoc-array mis-parse, subshell side-effect survival,
array-length syntax, `(( )) &&` set-e tails) are obsolete-by-construction in
Python and are NOT ported — see the report for the count and rationale."""

from __future__ import annotations

import pytest

from dpos_harness.checks import battery as B
from dpos_harness.checks.battery import Battery, ChainBattery, Ctx, SimCtx


# ── settle_gate_open ─────────────────────────────────────────────────────────
@pytest.mark.parametrize("cur,settle,cap,expected", [
    (10, 5, 12, True),    # past settle -> run
    (5, 10, 12, False),   # blinded within window
    (5, 20, 12, True),    # stuck-far (15 > cap) -> re-arm
    (5, 17, 12, False),   # exactly at cap boundary (12) -> still blinded
    (5, 18, 12, True),    # 13 > cap -> re-arm
])
def test_settle_gate_open(cur, settle, cap, expected):
    assert B.settle_gate_open(cur, settle, cap) is expected


# ── counter_progress (the flat/restart split is load-bearing) ────────────────
@pytest.mark.parametrize("cur,prev,expected", [
    (-1, 5, "unreadable"),
    (5, None, "baseline"),
    (5, "__none__", "baseline"),
    (5, -1, "baseline"),
    (3, 5, "restart"),
    (5, 5, "flat"),
    (6, 5, "progress"),
])
def test_counter_progress(cur, prev, expected):
    assert B.counter_progress(cur, prev) == expected


# ── node panic / down verdicts ───────────────────────────────────────────────
def test_node_panic_verdict():
    assert B.node_panic_verdict("validator-3", "") is None
    v = B.node_panic_verdict("validator-3", "thread panicked at storage.rs:43")
    assert v[0] == "node-panic" and "validator-3 PANICKED" in v[1]


@pytest.mark.parametrize("state,expected_id", [
    ("running", None), ("exited", "node-down"), ("dead", "node-down"),
])
def test_node_down_verdict(state, expected_id):
    v = B.node_down_verdict("validator-10", state, "1", "executor fatal")
    if expected_id is None:
        assert v is None
    else:
        assert v[0] == expected_id


# ── node-down exclusion at r0, built through the REAL to_ctx data flow (v61.10) ──
def _r0_state():
    """SimState exactly as run() holds it post-bringup, post-rotation-seed, pre-churn: full
    profile (validators=10, spare_end=12, val_containers=14) with the rotation slots seeded to
    their OWN native idx as STRINGS (orchestrator.py:857-858) — the shape that tripped the bug."""
    import os
    for k in ("SIM_QUICK", "SIM_VALIDATORS", "SIM_INITIAL_COMMITTEE", "SIM_IDENTITY_POOL",
              "SIM_SPARES", "SIM_ROTATION_SLOTS"):
        os.environ.pop(k, None)
    from dpos_harness.sim.orchestrator import SimState, SimConfig
    st = SimState(cfg=SimConfig())
    assert (st.cfg.validators, st.cfg.spare_end, st.cfg.val_containers) == (10, 12, 14)
    for rs in range(st.cfg.spare_end, st.cfg.val_containers):
        st.container.slot_identity.setdefault(f"validator-{rs}", str(rs))
    return st


def test_node_down_excludes_stopped_spare_and_rotation_band_at_r0():
    """The bug the CODE-parity verdict missed: at r0 the stop-spares step deliberately stopped
    validator-10..13 (exit 0). Through the real to_ctx mapping, node-down must EXCLUDE all four —
    spares 10-11 (never-activated) AND rotation 12-13 (idle, serving own idx). Before the int-
    normalization the "12" != 12 (str vs int) mismatch made 12/13 look re-tasked → node-down fired."""
    from dpos_harness.sim import state_ctx
    bat = Battery(*state_ctx.to_ctx(_r0_state()))
    for s in range(10, 14):
        assert bat._node_down_excluded(f"validator-{s}", "0") is True, f"validator-{s} NOT excluded at r0"


def test_node_down_fires_on_stopped_committee_member_at_r0():
    """Canary: a committee/growth idx (validator-3, idx < validators) that exits at r0 is NOT a
    deliberate stop — node-down MUST still fire (exclusion returns False)."""
    from dpos_harness.sim import state_ctx
    bat = Battery(*state_ctx.to_ctx(_r0_state()))
    assert bat._node_down_excluded("validator-3", "0") is False


def test_node_down_expects_up_retasked_rotation_slot():
    """The int-normalization must NOT over-exclude: a rotation slot re-tasked to a FOREIGN bench
    idx (SLOT_IDENTITY['validator-12']='16') is hosting a live mint → expected up (not excluded)."""
    from dpos_harness.sim import state_ctx
    st = _r0_state()
    st.container.slot_identity["validator-12"] = "16"   # hostpool re-task stores a numeric string
    bat = Battery(*state_ctx.to_ctx(st))
    assert bat._node_down_excluded("validator-12", "0") is False


# ── wrongful_slash_verdict ───────────────────────────────────────────────────
@pytest.mark.parametrize("jail,expected,mapped,cls", [
    (0, 0, 0, "clear"),          # not jailed
    (1, 1, 1, "clear"),          # jailed but expected (harness-faulted)
    (1, 0, 1, "wrongful"),       # jailed, unexpected, mapped identity
    (1, 0, 0, "unknown"),        # jailed, unexpected, unmapped
])
def test_wrongful_slash_verdict(jail, expected, mapped, cls):
    assert B.wrongful_slash_verdict(jail, expected, mapped) == cls


# ── dkg qual / iff verdicts ──────────────────────────────────────────────────
@pytest.mark.parametrize("tok,state", [("1", 1), ("0", 0), ("", -1), ("garbage", -1)])
def test_dkg_qual_state(tok, state):
    assert B.dkg_qual_state(tok) == state
    assert B.dkg_qual_is_set(tok) is (tok == "1")


@pytest.mark.parametrize("changed,qs,verdict", [
    (1, 0, "fail"),    # committee changed with EMPTY qual = shareless open (v40)
    (1, 1, "ok"),      # qualified change landed
    (0, 1, "watch"),   # qual set, committee unchanged = benign
    (0, 0, "ok"),      # deferral / no-change
    (1, -1, "watch"),  # unreadable -> not judged
    (0, -1, "watch"),
])
def test_dkg_iff_verdict(changed, qs, verdict):
    assert B.dkg_iff_verdict(changed, qs)[0] == verdict


@pytest.mark.parametrize("verdict,qs,gate,kind", [
    ("hold", -1, "pass", "hold"),
    ("boundary", 0, "pass", "deferred"),
    ("boundary", 1, "pass", "qualified"),
    ("cap", -1, "fail", "unknown"),
])
def test_dkg_barrier_qual_gate(verdict, qs, gate, kind):
    assert B.dkg_barrier_qual_gate(verdict, qs) == (gate, kind)


# ── backfill / grow budgets ──────────────────────────────────────────────────
def test_backfill_overdue():
    # budget = settle(5)+deadline(8) = 13; obligation@29 -> overdue past 29+13=42
    overdue, budget = B.backfill_overdue(43, 29, 0, 5, 8)
    assert budget == 13 and overdue is True
    assert B.backfill_overdue(42, 29, 0, 5, 8)[0] is False
    # pause credit extends the actionable clock
    assert B.backfill_overdue(43, 29, 2, 5, 8)[0] is False


def test_grow_land_overdue_and_ncap_only():
    assert B.grow_land_overdue(20, 5, 8) is True         # 15 > 8
    assert B.grow_land_overdue(20, 5, 8, 8) is False     # credit 8 -> 7 <= 8
    assert B.cp_ncap_only(1, 4, 5, 10, 12) is True
    assert B.cp_ncap_only(0, 4, 5, 10, 12) is False


# ── finalize-stall belt ──────────────────────────────────────────────────────
def test_finalize_stall_belt():
    bat = Battery(Ctx(EXPECTED_STALL=0))
    # advancing chain never trips
    for fin in (10, 11, 12):
        assert bat._inv_finalize_stall(fin) is True
    assert bat.SIM_FLAT_TICKS == 0
    # flat for SIM_STALL_TICKS (default 4) -> finalize-stall
    for _ in range(bat.SIM_STALL_TICKS - 1):
        assert bat._inv_finalize_stall(12) is True
    assert bat._inv_finalize_stall(12) is False
    assert bat.inv_fail_id == "finalize-stall"


def test_finalize_stall_fin0_resets_belt_not_baseline():
    bat = Battery(Ctx(EXPECTED_STALL=0))
    bat._inv_finalize_stall(50)
    for _ in range(2):
        bat._inv_finalize_stall(50)   # accumulate flat ticks
    assert bat.SIM_FLAT_TICKS == 2
    bat._inv_finalize_stall(0)        # FLK-5 blip -> reset belt, keep baseline
    assert bat.SIM_FLAT_TICKS == 0
    assert bat.SIM_LAST_FIN == 50    # baseline not poisoned by the 0


def test_expected_stall_suspends_finalize_stall():
    bat = Battery(Ctx(EXPECTED_STALL=1))
    for _ in range(10):
        assert bat._inv_finalize_stall(12) is True
    assert bat.SIM_FLAT_TICKS == 0


# ── k-lag exactness ──────────────────────────────────────────────────────────
def test_klag_underflow():
    bat = Battery(Ctx())
    # latest-fin = 2 < K=3 -> immediate SAFETY fail
    assert bat._inv_klag(fin=100, latest=102, lat=0, lrf=0) is False
    assert bat.inv_fail_id == "k-lag-underflow"


def test_klag_healthy_band():
    bat = Battery(Ctx())
    # gap == K, cgap == K -> holds, no belt
    assert bat._inv_klag(fin=100, latest=103, lat=100, lrf=97) is True


def test_klag_consensus_overclaim():
    bat = Battery(Ctx())
    # cgap = lat-lrf = 2 < K -> unreachable-by-construction SAFETY fail (no belt)
    assert bat._inv_klag(fin=100, latest=103, lat=100, lrf=98) is False
    assert bat.inv_fail_id == "k-lag-consensus-overclaim"


def test_klag_catchup_overclaim_belt():
    bat = Battery(Ctx())
    # latest < fin (finalized ahead of eth head) persisting past belt -> SAFETY fail
    for _ in range(bat.SIM_STALL_TICKS - 1):
        assert bat._inv_klag(fin=100, latest=98, lat=0, lrf=0) is True
    assert bat._inv_klag(fin=100, latest=98, lat=0, lrf=0) is False
    assert bat.inv_fail_id == "k-lag-catchup-overclaim"


# ── detector-starvation belt ─────────────────────────────────────────────────
def test_detector_sampled_belt():
    # detector-starved is NOT demoted (F3, 2026-07-29): it guards the BATTERY's own validity —
    # a starved detector reports GREEN while sampling too few nodes to prove anything — so
    # sustained starvation FAILS the run instead of filing a diagnostic.
    bat = Battery(Ctx())
    # sufficient samples -> clears
    assert bat._detector_sampled("d", "chan", 3, 2) is True
    # under floor for SIM_DET_STARVE_TICKS consecutive evaluated ticks -> belt trips
    for _ in range(bat.SIM_DET_STARVE_TICKS - 1):
        assert bat._detector_sampled("d", "chan", 0, 2) is True
    assert bat._detector_sampled("d", "chan", 0, 2) is False
    assert bat.inv_fail_id == "detector-starved"
    assert not any(k == "diag" for k, _n in bat.events)


def test_inv_fail_diag_for_still_demoted_id():
    # the demoted-diag path, driven through an id that IS still demoted (member-stall was
    # UN-demoted 2026-07-30, F6): record one diag, report HOLDING (True), leave inv_fail_id clear
    # so the run continues.
    bat = Battery(Ctx())
    assert bat._inv_fail("committee-vacancy", "seat vacancy never backfilled") is True
    assert bat._inv_fail("committee-vacancy", "seat vacancy never backfilled") is True  # latched
    assert bat.inv_fail_id == ""
    diags = [n for k, n in bat.events if k == "diag" and "committee-vacancy" in n]
    assert len(diags) == 1


def test_inv_fail_diag_latch_is_per_entity():
    """F6b: the latch keys on (fail_id, entity), not fail_id alone. Keyed on the id alone a demoted
    detector emitted exactly ONE line per run — the first offender's message with its counter
    frozen in — so a second offending address was invisible. A caller with no entity keeps the
    one-line-per-run behaviour (correct: its condition is about the committee as a whole)."""
    bat = Battery(Ctx())
    bat._inv_fail("committee-stranger", "0xaaa seated stranger, 3 ticks", entity="0xaaa")
    bat._inv_fail("committee-stranger", "0xaaa seated stranger, 9 ticks", entity="0xaaa")
    bat._inv_fail("committee-stranger", "0xbbb seated stranger, 3 ticks", entity="0xbbb")
    diags = [n for k, n in bat.events if k == "diag" and "committee-stranger" in n]
    assert len(diags) == 2                       # one per address, not one per run, not per tick
    assert any("0xaaa" in n for n in diags) and any("0xbbb" in n for n in diags)
    # entity-less caller: still exactly one line per run
    bat._inv_fail("committee-vacancy", "size 5 < MIN_COMMITTEE, 3 epochs")
    bat._inv_fail("committee-vacancy", "size 5 < MIN_COMMITTEE, 9 epochs")
    assert len([n for k, n in bat.events if k == "diag" and "committee-vacancy" in n]) == 1


def test_undemoted_ids_fail_the_run():
    """F6 (2026-07-30): these six ids were demoted with zero recorded justification and zero
    false-fires in 171 bundles. They are back to failing the run — member-stall/deferred-park
    because they are the ONLY per-member liveness checks (finalize-stall needs EVERY node frozen),
    the rest because none of them is seat bookkeeping. A silent re-demotion breaks this."""
    from dpos_harness.core.policy import DEMOTED_INVARIANTS
    for fid in ("member-stall", "deferred-park", "peer-plane", "committee-unhosted",
                "graceful-stop-hole", "warm-debt-stall"):
        assert fid not in DEMOTED_INVARIANTS
        bat = Battery(Ctx())
        assert bat._inv_fail(fid, "boom") is False
        assert bat.inv_fail_id == fid
        assert not any(k == "diag" for k, _n in bat.events)


def test_member_stall_fails_the_run_through_its_own_detector():
    """The un-demotion driven through the REAL detector, not just _inv_fail. This is the single-node
    liveness hole: finalize-stall needs EVERY node frozen (nodes.finalized_dec takes the max across
    the other running containers), so a wedged member is invisible to it. validator-1 proves
    progress, then freezes while the chain (and validator-2) advance."""
    a2i = {"0xa": "validator-0", "0xb": "validator-1", "0xc": "validator-2"}
    ctx = Ctx(EXPECTED_STALL=0, SIM_CUR_EPOCH=10, SETTLE_UNTIL_EPOCH=0,
              SIM_CUR_COMMITTEE="0xa 0xb 0xc", ADDR2IDX=a2i)
    bat = Battery(ctx)
    fin = 100
    bat.SIM_NODE_FIN = {"validator-1": 100, "validator-2": 100}
    assert bat._inv_member_liveness(fin) is True            # baseline
    fin += 1
    bat.SIM_NODE_FIN = {"validator-1": 101, "validator-2": 101}
    assert bat._inv_member_liveness(fin) is True            # both ARMED by proven progress
    verdict = True
    for _ in range(bat.SIM_STALL_TICKS):                   # v1 WEDGED, chain + v2 keep moving
        fin += 1
        bat.SIM_NODE_FIN = {"validator-1": 101, "validator-2": fin}
        verdict = bat._inv_member_liveness(fin)
    assert verdict is False
    assert bat.inv_fail_id == "member-stall" and "validator-1" in bat.inv_fail_msg


def test_deferred_park_fails_the_run_through_its_own_detector():
    """member-stall's executor-side twin (the soak7 park signature): deferred_height flat while the
    chain runs. Un-demoted 2026-07-30 — the same hole, on the execution side."""
    bat = Battery(Ctx(EXPECTED_STALL=0))
    bat.SIM_SCRAPE_NODES = ["validator-1"]
    bat.SIM_SCRAPE_TEXT = {"validator-1": "dpos_deferred_height 4242"}
    bat._gauge_val = lambda text, suffix: "4242"            # flat forever
    verdict = True
    for _ in range(bat.SIM_PARK_TICKS + 2):
        verdict = bat._inv_deferred_park()
    assert verdict is False
    assert bat.inv_fail_id == "deferred-park" and "PARKED" in bat.inv_fail_msg


def test_peer_plane_fails_the_run_through_its_own_detector():
    """One bundle in 171 and it read as a TRUE positive: devp2p plane down on a seated member,
    which consensus liveness does not reveal until that node falls behind. Un-demoted 2026-07-30."""
    a2i = {"0xa": "validator-0", "0xb": "validator-1"}
    bat = Battery(Ctx(SIM_CUR_COMMITTEE="0xa 0xb", ADDR2IDX=a2i, SIM_TICK=0))
    bat._peer_count = lambda svc: 0
    verdict = True
    for _ in range(4):
        verdict = bat._inv_peer_plane()                     # SIM_TICK 0 % SIM_PEER_EVERY == 0
    assert verdict is False
    assert bat.inv_fail_id == "peer-plane" and "validator-1" in bat.inv_fail_msg


def test_deleted_ids_are_gone_from_the_demotion_set():
    """The three bash-era holdovers with no Python call site at all (verified by grep: the string
    "growth" survives only as dispatch/orchestrator last_growth_kind STATE and the case_growth
    module, never as a fail id). Deleted rather than un-demoted — there is nothing to un-demote."""
    from dpos_harness.core.policy import DEMOTED_INVARIANTS
    for fid in ("growth", "promote-shareless", "refill-shareless"):
        assert fid not in DEMOTED_INVARIANTS


def test_detector_sampled_announces_once():
    bat = Battery(Ctx())
    bat._detector_sampled("d", "chan", 0, 2)
    bat._detector_sampled("d", "chan", 0, 2)
    undersampled = [e for e in bat.events if e[0] == "detector_undersampled"]
    assert len(undersampled) == 1  # one event per starvation run, not per tick


# ── block-rate soft reporter ─────────────────────────────────────────────────
def test_block_rate_slow_warn():
    bat = Battery(Ctx(EXPECTED_STALL=0))
    clock = {"t": 1000}
    bat._now = lambda: clock["t"]
    bat._inv_block_rate(100)          # baseline
    # advance 1 block / 10s = 0.1 blk/s < warn(0.5), twice -> warn
    clock["t"] += 10
    bat._inv_block_rate(101)
    clock["t"] += 10
    bat._inv_block_rate(102)
    warns = [e for e in bat.events if e[0] == "warn" and "block-rate" in e[1]]
    assert warns


# ── committee membership tiers ───────────────────────────────────────────────
def test_membership_tier1_undersized():
    # committee-undersized is DEMOTED (liveness-first): records a diag, holds (True), never fatal.
    # f_run=2 -> BFT floor 3*2+1=7; a size-4 committee is below it
    ctx = Ctx(SIM_CUR_F=2, SIM_CUR_COMMITTEE="0xa 0xb 0xc 0xd", MIN_COMMITTEE=7)
    bat = Battery(ctx)
    for _ in range(bat.SIM_MEMBERSHIP_BELT - 1):
        assert bat._inv_committee_membership() is True
    assert bat._inv_committee_membership() is True   # demoted: holds, not fatal
    assert bat.inv_fail_id == ""
    assert any(k == "diag" and "committee-undersized" in n for k, n in bat.events)


def test_membership_tier2_vacancy_watch_not_fail():
    # f_run=1 -> BFT floor 4; MIN_COMMITTEE 7; size 5 = seat vacancy (>=4, <7)
    addrs = " ".join(f"0x{i:040x}" for i in range(5))
    a2i = {f"0x{i:040x}": f"validator-{i}" for i in range(5)}
    ctx = Ctx(SIM_CUR_F=1, SIM_CUR_COMMITTEE=addrs, ADDR2IDX=a2i,
              MIN_COMMITTEE=7, SIM_CUR_EPOCH=10)
    bat = Battery(ctx)
    assert bat._inv_committee_membership() is True  # within budget -> watch
    assert any(e[0] == "warn" and "seat vacancy" in e[1] for e in bat.events)


def test_membership_stranger_first_tick_warns():
    # a seated address resolving in NEITHER ADDR2IDX nor OWNER_ADDR_CACHE -> WARN
    # (unmapped), not yet a fail
    ctx = Ctx(SIM_CUR_F=1, SIM_CUR_COMMITTEE="0xstranger 0xa 0xb 0xc",
              ADDR2IDX={"0xa": "validator-1", "0xb": "validator-2", "0xc": "validator-3"},
              MIN_COMMITTEE=0)
    bat = Battery(ctx)
    assert bat._inv_committee_membership() is True
    assert any(e[0] == "warn" and "UNMAPPED" in e[1] and "no OWNER_ADDR_CACHE match" in e[1]
               for e in bat.events)


def test_membership_stranger_belt():
    # a seated address that resolves nowhere, held past the shared belt -> committee-stranger
    ctx = Ctx(SIM_CUR_F=1, SIM_CUR_COMMITTEE="0xstranger 0xa 0xb 0xc",
              ADDR2IDX={"0xa": "validator-1", "0xb": "validator-2", "0xc": "validator-3"},
              MIN_COMMITTEE=0)
    bat = Battery(ctx)
    for _ in range(bat.SIM_MEMBERSHIP_BELT - 1):
        assert bat._inv_committee_membership() is True
    assert bat._inv_committee_membership() is True   # demoted: holds, not fatal
    assert bat.inv_fail_id == ""
    assert any(k == "diag" and "committee-stranger" in n for k, n in bat.events)


def _unhosted_ctx():
    # 0xdead is registered (OWNER_ADDR_CACHE idx 7) but NOT hosted (ADDR2IDX miss) —
    # a consumed-spare eviction, bundle-20260720T170507Z. OWNER_ADDR_CACHE is SIM bookkeeping, so
    # it comes in through the second context, not the first.
    return (Ctx(SIM_CUR_F=1, SIM_CUR_COMMITTEE="0xdead 0xa 0xb 0xc",
                ADDR2IDX={"0xa": "validator-1", "0xb": "validator-2", "0xc": "validator-3"},
                MIN_COMMITTEE=0),
            SimCtx(OWNER_ADDR_CACHE={7: "0xdead"}))


def test_membership_unhosted_first_tick_warns():
    # ADDR2IDX miss but OWNER_ADDR_CACHE HIT -> WARN (registered but unhosted), not a stranger
    bat = Battery(*_unhosted_ctx())
    assert bat._inv_committee_membership() is True
    assert any(e[0] == "warn" and "UNHOSTED" in e[1] and "OWNER_ADDR_CACHE hit" in e[1]
               for e in bat.events)
    assert not any("UNMAPPED" in e[1] for e in bat.events)  # not misreported as a stranger


def test_membership_unhosted_belt():
    # sustained ADDR2IDX-miss + OWNER_ADDR_CACHE-hit past the shared belt -> committee-unhosted.
    # UN-DEMOTED 2026-07-30 (F6): a live committee seat with no running host FAILS the run. It was
    # demoted alongside committee-stranger — the very false positive this branch exists to fix.
    bat = Battery(*_unhosted_ctx())
    for _ in range(bat.SIM_MEMBERSHIP_BELT - 1):
        assert bat._inv_committee_membership() is True
    assert bat._inv_committee_membership() is False
    assert bat.inv_fail_id == "committee-unhosted"
    assert "idx 7" in bat.inv_fail_msg and "UNHOSTED" in bat.inv_fail_msg
    assert not any(k == "diag" for k, _n in bat.events)


# ── the exclusion predicates, which now go through the ONE core.setops.in_set ────
def test_excluded_reads_the_declared_perturbation_sets_whole_word():
    """`_excluded` is the gate on every cross-node sample, and it is the battery's only use of
    the shared whole-word membership primitive. It had a private copy until the dedup pass, and
    NOTHING covered it: breaking the merged helper left the battery green. Both halves here —
    the disrupted space-set and the permanently-dead map — plus the substring canary that is the
    whole reason the primitive is whole-word (validator-1 must survive validator-10)."""
    bat = ChainBattery(Ctx(SIM_DISRUPTED="validator-10 validator-4",
                           PERMANENTLY_DEAD={"validator-7": 1, "validator-8": 0}))
    # `_excluded` returns the truthy MAP VALUE for a permanently-dead container, not a bool
    assert bat._excluded("validator-4")
    assert bat._excluded("validator-10")
    assert not bat._excluded("validator-1")          # substring of validator-10, NOT excluded
    assert bat._excluded("validator-7")               # permanently dead
    assert not bat._excluded("validator-8")           # present but falsy -> not excluded
    assert not bat._excluded("validator-2")


def test_node_down_excludes_a_declared_disrupted_committee_member():
    """The same primitive on the sim-coupled side: a container the consumer declared disrupted is
    never a node-down, while its substring neighbour still is."""
    bat = Battery(Ctx(SIM_DISRUPTED="validator-1"), SimCtx(SIM_VALIDATORS=10, SIM_SPARE_END=12,
                                                            SIM_VAL_CONTAINERS=14))
    assert bat._node_down_excluded("validator-1", "0") is True
    assert bat._node_down_excluded("validator-13", "0") is True    # idle rotation slot
    assert bat._node_down_excluded("validator-3", "0") is False




# ── dkg pinned-idx / finalize-deferred (2026-07-30 handoff metrics) ──────────
_DKG_METRICS = ("dpos_dkg_pinned_idx_out_of_range_total_total {pinned}\n"
                "dpos_dkg_finalize_deferred_total_total {deferred}\n")


def _dkg_bat(**per_node):
    """A battery whose shared per-tick scrape is pre-populated (the commonware :9100 text the
    real _scrape_metrics builds); per_node maps service -> (pinned, deferred)."""
    bat = Battery(Ctx())
    bat.SIM_SCRAPE_NODES = list(per_node)
    bat.SIM_SCRAPE_TEXT = {n: _DKG_METRICS.format(pinned=p, deferred=d)
                            for n, (p, d) in per_node.items()}
    return bat


def test_dkg_pinned_idx_holds_at_zero():
    bat = _dkg_bat(**{"validator-0": (0, 0), "validator-3": (0, 0)})
    assert bat._inv_dkg_pinned_idx() is True
    assert bat.inv_fail_id == ""
    assert bat.SIM_DET_SAMPLES["dkg-pinned-idx"] == 2


def test_dkg_pinned_idx_non_zero_fails_the_run():
    """The 2026-07-21 idx-stall class resurfacing — a hard fail, not a report."""
    bat = _dkg_bat(**{"validator-0": (0, 0), "validator-3": (1, 0)})
    assert bat._inv_dkg_pinned_idx() is False
    assert bat.inv_fail_id == "dkg-pinned-idx-out-of-range"
    assert "validator-3" in bat.inv_fail_msg and "2026-07-21" in bat.inv_fail_msg


def test_dkg_pinned_idx_fail_id_is_not_demoted():
    from dpos_harness.core.policy import DEMOTED_INVARIANTS
    assert "dkg-pinned-idx-out-of-range" not in DEMOTED_INVARIANTS


def test_dkg_pinned_idx_unreadable_metric_reports_instead_of_holding_silently():
    """A tick where NO scraped node exposes the family must SAY SO (undersampled event) and
    hard-fail on sustained starvation — silence from an unreadable detector is not health."""
    bat = Battery(Ctx())
    bat.SIM_SCRAPE_NODES = ["validator-0"]
    bat.SIM_SCRAPE_TEXT = {"validator-0": "beacon_seed_active_total_total 5963\n"}
    assert bat._inv_dkg_pinned_idx() is True                    # first tick: report, don't fail
    assert bat.SIM_DET_SAMPLES["dkg-pinned-idx"] == 0
    assert any(k == "detector_undersampled" and "dkg-pinned-idx" in n for k, n in bat.events)
    verdict = True
    for _ in range(bat.SIM_DET_STARVE_TICKS):
        verdict = bat._inv_dkg_pinned_idx()
    assert verdict is False and bat.inv_fail_id == "detector-starved"


def test_dkg_finalize_deferred_is_report_only():
    """SOFT: a rise is a signal to read the logs, never a verdict (no baseline yet)."""
    bat = _dkg_bat(**{"validator-0": (0, 0)})
    assert bat._inv_dkg_finalize_deferred() is True             # baseline sighting, no event
    assert bat.events == []
    bat.SIM_SCRAPE_TEXT["validator-0"] = _DKG_METRICS.format(pinned=0, deferred=2)
    assert bat._inv_dkg_finalize_deferred() is True
    assert bat.inv_fail_id == ""
    kind, note = bat.events[-1]
    assert kind == "warn" and "rose 0 -> 2" in note and "validator-0" in note


def test_dkg_finalize_deferred_rebaselines_on_a_node_restart():
    """A restarted node's counter drops below the last value — re-baseline, never a false rise."""
    bat = _dkg_bat(**{"validator-0": (0, 5)})
    bat._inv_dkg_finalize_deferred()
    bat.SIM_SCRAPE_TEXT["validator-0"] = _DKG_METRICS.format(pinned=0, deferred=0)
    bat._inv_dkg_finalize_deferred()
    assert bat.events == [] and bat.SIM_DKG_DEFERRED_LAST["validator-0"] == 0
