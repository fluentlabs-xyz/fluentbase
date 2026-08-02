"""load sender pure seams — port of the soak-gate-test.sh §20 assertions.

Pins the arithmetic/lifecycle seams of load-heavy.sh: calibration, burn-iters, nonce-error
classification, the backpressure window, acquisition/backoff/identity/pidfile, the status-line
grep contract, and the marker gate."""

from __future__ import annotations

import pytest

from dpos_harness.stack import sender


# ── calibration + burn-iters ──────────────────────────────────────────────────
def test_calibrate():
    tps, ivl = sender.lh_calibrate(30_000_000, 0.6, 4, 3_000_000)
    assert tps == 6.0                       # 30e6 * 0.6 / 3e6
    assert ivl == int(4 * 1000 / 6.0)       # 666 ms per sender


def test_calibrate_rejects_nonpositive():
    with pytest.raises(ValueError):
        sender.lh_calibrate(0, 0.6, 4, 3_000_000)


def test_burn_iters_headroom_and_floor():
    # fill 90% of the tx gas budget minus base, over per-iter
    assert sender.lh_burn_iters(3_000_000, 50_000, 22_300) == int((0.9 * 3_000_000 - 50_000) / 22_300)
    assert sender.lh_burn_iters(1, 999_999, 1) == 1   # never below 1


# ── burner bytecode wiring (v61 a12: zero-load regression) ────────────────────
def test_burner_bytecode_env_override(monkeypatch):
    monkeypatch.setenv("LOAD_BURNER_BYTECODE", "0xdeadbeef")
    assert sender.lh_burner_bytecode() == "0xdeadbeef"


def test_burner_bytecode_loads_vendored_fixture(monkeypatch):
    """With no env override, the deploy bytecode is loaded from contracts/GasBurner.json — the port
    used to default to a bare '0x' and abort burn mode on a zero-load run."""
    monkeypatch.delenv("LOAD_BURNER_BYTECODE", raising=False)
    bc = sender.lh_burner_bytecode()
    assert bc.startswith("0x") and len(bc) > 100   # real deploy bytecode, not the empty '0x'


def test_burner_bytecode_missing_env_is_not_bare_0x(monkeypatch):
    """The regression guard: an unset LOAD_BURNER_BYTECODE must NOT collapse to '0x' (that is what
    silently produced the no-contractAddress deploy failure)."""
    monkeypatch.delenv("LOAD_BURNER_BYTECODE", raising=False)
    assert sender.lh_burner_bytecode() != "0x"


# ── nonce-error classification ────────────────────────────────────────────────
@pytest.mark.parametrize("msg", ["nonce too low", "nonce too high", "invalid nonce",
                                 "already known", "replacement transaction underpriced"])
def test_is_nonce_error_true(msg):
    assert sender.lh_is_nonce_error(msg)


def test_is_nonce_error_false():
    assert not sender.lh_is_nonce_error("connection refused")
    assert not sender.lh_is_nonce_error("execution reverted")


# ── backpressure window (the wedge-2 fix) ─────────────────────────────────────
def test_inflight_gate_send_and_skip():
    assert sender.lh_inflight_gate(5, 3, 4) == (5, "send")    # inflight 2 < 4
    assert sender.lh_inflight_gate(7, 3, 4) == (7, "skip")    # inflight 4 >= 4 → skip, no bump


def test_inflight_gate_reconciles_next_to_mined():
    # external reality wins: local next can never trail the mined head
    assert sender.lh_inflight_gate(2, 5, 4) == (5, "send")


# ── acquisition verdict + backoff ─────────────────────────────────────────────
def test_acquire_verdict():
    assert sender.lh_acquire_verdict(10, 1800) == "retry"
    assert sender.lh_acquire_verdict(1800, 1800) == "fail"


def test_restart_backoff_exponential_capped():
    assert sender.lh_restart_backoff(0) == 0
    assert sender.lh_restart_backoff(1) == 2      # base 2 * 2^0
    assert sender.lh_restart_backoff(2) == 4
    assert sender.lh_restart_backoff(3) == 8
    assert sender.lh_restart_backoff(100) == 60   # capped at MAX


# ── clean-stop identity + pidfile parse ───────────────────────────────────────
def test_proc_identity_matches():
    # now - etime == recorded start (within tol) → ours
    assert sender.lh_proc_identity_matches(1000, now=1100, etimes=100, tol=2)
    # recycled PID: start time far off → NOT ours
    assert not sender.lh_proc_identity_matches(1000, now=1100, etimes=50, tol=2)


def test_pidfile_parse():
    assert sender.lh_pidfile_parse("123 123 1700000000") == (123, 123, 1700000000)
    for bad in ("", "123 456", "123 456 789 junk", "abc 1 2"):
        with pytest.raises(ValueError):
            sender.lh_pidfile_parse(bad)


# ── the status-line grep contract (must never drift from soak-status.sh / status.py) ──
def test_status_line_grep_contract():
    import re
    line = sender.lh_status_line("burn", 42, 5, 48, basefee=7, maxfee=5000, sample="0x1", head=2295)
    assert re.search(r"sent=[0-9]+ inflight=[0-9]+/[0-9]+", line)
    assert "mode=burn" in line and "sent=42 inflight=5/48" in line


# ── the DeployStaking marker gate (funding-safety) ────────────────────────────
def test_log_has_marker(tmp_path):
    log = tmp_path / "sim.log"
    log.write_text("bringing up...\nno marker yet\n")
    assert not sender.lh_log_has_marker(str(log))
    log.write_text("  [sim r1 churn] fin=10 epoch=0 f=2 started\n")
    assert sender.lh_log_has_marker(str(log))
    assert not sender.lh_log_has_marker(str(tmp_path / "nope.log"))


# ── pidfile path resolution + stop on a missing pidfile ───────────────────────
def test_stop_no_pidfile(tmp_path):
    assert sender.stop(str(tmp_path / "absent.pid")) == 0


def test_stop_malformed_pidfile(tmp_path):
    pf = tmp_path / "load-heavy.pid"
    pf.write_text("garbage not numbers\n")
    assert sender.stop(str(pf)) == 0
    assert not pf.exists()   # malformed pidfile is removed
