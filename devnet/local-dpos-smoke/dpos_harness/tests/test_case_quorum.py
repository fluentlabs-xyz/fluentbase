"""Pure-logic tests for the quorum-loss SAFETY case verdict layer (no docker / no chain).

Covers evaluate_quorum_case (the stall / restart / recovery trichotomy) and the fast profile.
The live bring-up/stop/restore is exercised by `make case-quorum` against the real devnet — the
verdict function is where the safety logic lives, so it is what gets unit coverage here."""

from __future__ import annotations

from dpos_harness.cases.quorum import apply_case_env_defaults, evaluate_quorum_case


# ── the PASS path: stalled flat, all victims back, finalized resumed ──────────
def test_pass_stall_then_recover():
    ok, reason = evaluate_quorum_case(need=3, plateau_lo=2460, plateau_hi=2460,
                                      victims_down=[], fin_recovered=2475)
    assert ok
    assert "RECOVERED" in reason and "safety threshold holds" in reason


def test_pass_allows_settle_drain_below_lo():
    """A plateau_hi BELOW plateau_lo (an in-flight <=K finalization completing during the drain,
    never a NEW quorum) is still a valid stall — only ADVANCE past plateau_lo fails the stall."""
    ok, _ = evaluate_quorum_case(need=3, plateau_lo=2460, plateau_hi=2460,
                                 victims_down=[], fin_recovered=2500)
    assert ok


# ── the NO-STALL failure: quorum was never actually lost ─────────────────────
def test_fail_no_stall_when_finalized_advances():
    ok, reason = evaluate_quorum_case(need=3, plateau_lo=2460, plateau_hi=2468,
                                      victims_down=[], fin_recovered=2480)
    assert not ok
    assert "NO STALL" in reason and "safety threshold did not hold" in reason


# ── the RESTART failure: a stopped victim never came back (the old probe's blind spot) ──
def test_fail_victim_never_restarts():
    ok, reason = evaluate_quorum_case(need=3, plateau_lo=2460, plateau_hi=2460,
                                      victims_down=["validator-10"], fin_recovered=2475)
    assert not ok
    assert "RESTART FAILURE" in reason and "validator-10" in reason


def test_restart_failure_takes_precedence_over_recovery():
    """Even if finalized happened to climb (the other victims restored quorum), a straggler that is
    still down must FAIL — this is exactly the false-'recovered' the removed in-sim probe gave."""
    ok, reason = evaluate_quorum_case(need=3, plateau_lo=2460, plateau_hi=2460,
                                      victims_down=["validator-9", "validator-10"],
                                      fin_recovered=2490)
    assert not ok and "RESTART FAILURE" in reason


# ── the RECOVERY failure: all victims up but finalization did not resume ──────
def test_fail_no_recovery_after_restore():
    ok, reason = evaluate_quorum_case(need=3, plateau_lo=2460, plateau_hi=2460,
                                      victims_down=[], fin_recovered=2460)
    assert not ok
    assert "RECOVERY FAILURE" in reason


# ── profile ──────────────────────────────────────────────────────────────────
def test_profile_yields_fplus1_over_one(monkeypatch):
    for k in ("SIM_VALIDATORS", "SIM_INITIAL_COMMITTEE", "SIM_GEO_LATENCY", "SIM_BYZANTINE"):
        monkeypatch.delenv(k, raising=False)
    prof = apply_case_env_defaults()
    n = int(prof["SIM_INITIAL_COMMITTEE"])
    f = (n - 1) // 3
    assert f >= 1 and f + 1 < n          # a real f+1 drop with survivors left
    assert prof["SIM_GEO_LATENCY"] == "0" and prof["SIM_BYZANTINE"] == "0"


def test_profile_respects_operator_override(monkeypatch):
    monkeypatch.setenv("SIM_INITIAL_COMMITTEE", "10")
    prof = apply_case_env_defaults()
    assert prof["SIM_INITIAL_COMMITTEE"] == "10"
