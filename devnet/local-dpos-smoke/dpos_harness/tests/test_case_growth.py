"""Pure-logic tests for the bug-B GROWTH case verdict layer (no docker / no chain).

Covers the NEW helpers case_growth introduces: the dkg_logs idx-out-of-committee
log-signature matcher, the per-height verify-false clustering, and the combined
growth verdict (liveness advance + signature absence). The live bring-up/growth is
exercised by `make case-growth` against the real devnet — NOT here."""

from __future__ import annotations

from dpos_harness.cases.growth import (
    BUGB_IDX_SIGNATURE,
    apply_case_env_defaults,
    cluster_verify_false,
    evaluate_growth_case,
    scan_idx_stall,
)

# The exact bug-B line application.rs::beacon_gate_decision emits on a rejected idx.
_BUGB_LINE = ("WARN dpos::application height=27021 epoch=41 n=5 dkg_logs: an idx is out "
              "of committee[E+1] range — voting false")


def test_scan_idx_stall_finds_the_signature():
    log = ("INFO consensus finalized height=27020\n"
           f"{_BUGB_LINE}\n"
           "INFO consensus verify height=27022 ok\n")
    hits = scan_idx_stall(log)
    assert len(hits) == 1
    assert BUGB_IDX_SIGNATURE in hits[0]
    assert hits[0].startswith("WARN")  # returned stripped


def test_scan_idx_stall_clean_log_is_empty():
    log = ("INFO consensus finalized height=27020\n"
           "INFO beacon dkg_ceremony ok epoch=41\n"
           "INFO consensus verify height=27021 committee ok\n")
    assert scan_idx_stall(log) == []


def test_cluster_verify_false_buckets_by_height():
    # a burst of rejections at ONE height (27021) = the finalize-stall fingerprint.
    log = "\n".join([
        "WARN verify height=27021 voting false reason=dkg_logs_idx",
        "WARN verify height=27021 voting false reason=dkg_logs_idx",
        "WARN verify height=27021 voting false reason=dkg_logs_idx",
        "WARN verify height=27099 voting false reason=bad_signature",
        "INFO verify height=27100 accepted",
    ])
    clusters = cluster_verify_false(log)
    assert clusters[27021] == 3
    assert clusters[27099] == 1
    assert 27100 not in clusters


def test_cluster_verify_false_no_height_buckets_under_minus_one():
    log = "WARN verify voting false (no height field here)\n"
    assert cluster_verify_false(log) == {-1: 1}


def test_evaluate_growth_case_pass():
    logs = {"validator-0": "INFO finalized height=200 ok",
            "validator-1": "INFO beacon dkg ok epoch=6"}
    ok, reason = evaluate_growth_case(fin0=100, fin_now=140, min_advance=32,
                                      per_node_logs=logs)
    assert ok is True
    assert "advanced 40" in reason
    assert "no idx-stall" in reason and "2 validator" in reason


def test_evaluate_growth_case_fails_on_finalize_stall():
    # finalized did not move a full epoch across the growth window => bug B present.
    ok, reason = evaluate_growth_case(fin0=100, fin_now=108, min_advance=32,
                                      per_node_logs={"validator-0": "clean"})
    assert ok is False
    assert "finalize-stall" in reason
    assert "advanced 8" in reason and "fin0=100" in reason


def test_evaluate_growth_case_fails_on_bugb_signature():
    # liveness advanced fine, but a node carries the idx-out-of-committee line.
    logs = {"validator-0": "INFO finalized height=300 ok",
            "validator-2": _BUGB_LINE}
    ok, reason = evaluate_growth_case(fin0=100, fin_now=200, min_advance=32,
                                      per_node_logs=logs)
    assert ok is False
    assert "idx-out-of-committee" in reason
    assert "validator-2" in reason


def test_signature_failure_wins_only_after_liveness_gate():
    # a stall AND a signature: the finalize-stall verdict is reported first (the
    # liveness gate is checked before the log scan) — deterministic ordering.
    logs = {"validator-1": _BUGB_LINE}
    ok, reason = evaluate_growth_case(fin0=100, fin_now=100, min_advance=32,
                                      per_node_logs=logs)
    assert ok is False
    assert "finalize-stall" in reason


def test_apply_case_env_defaults_leaves_room_for_growth(monkeypatch):
    for k in ("SIM_QUICK", "SIM_VALIDATORS", "SIM_INITIAL_COMMITTEE", "SIM_SPARES",
              "SIM_ROTATION_SLOTS", "SIM_EPOCH_INTERVAL", "SIM_NO_CASCADE",
              "SIM_BYZANTINE", "SIM_GEO_LATENCY"):
        monkeypatch.delenv(k, raising=False)
    prof = apply_case_env_defaults()
    # initial_committee < validators is the whole point (room for register_activate to grow).
    assert int(prof["SIM_INITIAL_COMMITTEE"]) < int(prof["SIM_VALIDATORS"])
    # INITIAL_F = (initial_committee - 1)//3 must be >= 1 (the harness floor).
    assert (int(prof["SIM_INITIAL_COMMITTEE"]) - 1) // 3 >= 1
    assert prof["SIM_NO_CASCADE"] == "1" and prof["SIM_BYZANTINE"] == "0"


def test_apply_case_env_defaults_respects_operator_override(monkeypatch):
    monkeypatch.setenv("SIM_VALIDATORS", "9")
    monkeypatch.setenv("SIM_EPOCH_INTERVAL", "48")
    prof = apply_case_env_defaults()
    assert prof["SIM_VALIDATORS"] == "9"      # setdefault must not clobber
    assert prof["SIM_EPOCH_INTERVAL"] == "48"
