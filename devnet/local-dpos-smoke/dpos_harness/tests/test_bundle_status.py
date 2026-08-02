"""bundle event-log format + status log-scraping. Ports the soak-bundle.sh event-line contract
and the soak-status.sh section-scrape assertions (the pieces that need no live chain)."""

from __future__ import annotations

import ast
import inspect
import json
import os

from dpos_harness.sim import status
from dpos_harness.core import events as events_mod
from dpos_harness.core.events import EventContext, EventLog
from dpos_harness.sim.orchestrator import SimConfig, SimState


def _state():
    os.environ.update(SIM_VALIDATORS="7", SIM_INITIAL_COMMITTEE="7", SIM_SPARES="0",
                      SIM_ROTATION_SLOTS="0", SIM_SEED="testseed")
    st = SimState(cfg=SimConfig())
    st.round = 5
    st.cur_epoch = 12
    st.cur_f = 2
    st.cur_committee = "0xaaa 0xbbb"
    st.disrupted = "validator-3"
    return st


def test_sim_event_writes_jsonl_and_heartbeat(tmp_path, capsys, monkeypatch):
    st = _state()
    monkeypatch.setattr("dpos_harness.core.events.EventLog._finalized", lambda self: 2295)
    el = EventLog(context=st.event_context, out_dir=str(tmp_path))
    el.sim_event("churn", "APPLIED graceful_stop_restart validator-3")
    # JSONL record carries the seed on every line + structured context
    rec = json.loads((tmp_path / "events.jsonl").read_text().strip())
    assert rec["seed"] == "testseed" and rec["kind"] == "churn"
    assert rec["round"] == 5 and rec["epoch"] == 12 and rec["f"] == 2
    assert rec["committee"] == ["0xaaa", "0xbbb"] and rec["disrupted"] == ["validator-3"]
    # PINNED heartbeat line (soak-status.sh / status.py + the load marker grep pin the format)
    out = capsys.readouterr().out
    assert "[sim r5 churn] fin=2295 epoch=12 f=2 APPLIED graceful_stop_restart validator-3" in out


def test_event_context_fields_are_exactly_what_the_log_renders():
    """The EventContext contract, DERIVED ON BOTH SIDES: every `ctx.<name>` events.py renders
    must be a declared field, and every declared field must be rendered somewhere. The event log
    used to duck-type these off SimState with `getattr(s, name, default)`, which meant a renamed
    state attribute produced a bundle full of zeroes instead of an error — the failure mode this
    replaced. A hand-written mirror list here would reproduce it, so both sides are derived."""
    tree = ast.parse(inspect.getsource(events_mod))
    rendered = {n.attr for n in ast.walk(tree)
                if isinstance(n, ast.Attribute)
                and isinstance(n.value, ast.Name) and n.value.id == "ctx"}
    declared = set(EventContext.__dataclass_fields__)
    assert rendered, "AST found no ctx reads — the derivation broke, not the contract"
    assert rendered == declared, (
        f"events.py renders {sorted(rendered - declared)} that EventContext does not declare; "
        f"EventContext declares {sorted(declared - rendered)} that nothing renders")


def test_event_context_is_reread_per_event_not_snapshotted(tmp_path, monkeypatch):
    """`context` is a CALLABLE for a reason: round/epoch/f move every round. A snapshot taken at
    construction would pin every line in the run to round 0 — and since the heartbeat is what
    `status` and the load blaster's marker grep read, a frozen round is a silently wrong log."""
    st = _state()
    monkeypatch.setattr("dpos_harness.core.events.EventLog._finalized", lambda self: 10)
    el = EventLog(context=st.event_context, out_dir=str(tmp_path))
    el.sim_event("churn", "first")
    st.round, st.cur_epoch, st.cur_f = 6, 13, 3
    el.sim_event("churn", "second")
    recs = [json.loads(l) for l in (tmp_path / "events.jsonl").read_text().splitlines()]
    assert [(r["round"], r["epoch"], r["f"]) for r in recs] == [(5, 12, 2), (6, 13, 3)]


def test_event_log_works_with_no_sim_state_at_all(tmp_path, capsys, monkeypatch):
    """The layering claim: `core/events` is usable by a caller that has never heard of
    SimState — a smoke case supplies a plain EventContext and gets a correctly attributed
    line. Before this, such a caller passed nothing and logged seed="" round=0 silently."""
    monkeypatch.setattr("dpos_harness.core.events.EventLog._finalized", lambda self: 77)
    ctx = EventContext(seed="case-cert-follow", round=2, epoch=9, f=1)
    el = EventLog(context=lambda: ctx, out_dir=str(tmp_path))
    el.sim_event("churn", "upstream cert rejected")
    assert "[sim r2 churn] fin=77 epoch=9 f=1 upstream cert rejected" in capsys.readouterr().out
    assert json.loads((tmp_path / "events.jsonl").read_text().strip())["seed"] == "case-cert-follow"


def test_bundle_reports_absent_services_instead_of_inventing_them(tmp_path, monkeypatch):
    """`_services` used to fall back to a HARDCODED 14 validators + full-node whenever
    `docker compose ps` came back empty. That put a container list into the failure bundle that
    was read from nothing and true of no run but a 14-container one: at any other committee size
    it named containers that never existed and omitted ones that did. The fallback is gone; the
    gap is now stated. A wrong container count in a bundle is worse than an absent one."""
    st = _state()
    monkeypatch.setattr("dpos_harness.core.events.EventLog._finalized", lambda self: 2295)
    monkeypatch.chdir(tmp_path)
    out = tmp_path / "out"
    out.mkdir()
    el = EventLog(context=st.event_context, out_dir=str(out))
    # NO stub on _services: the hermetic fixture already answers `docker compose ps` with an
    # empty stdout, so this drives the REAL enumeration down its empty path. Stubbing it out
    # would leave the fallback that used to live inside it untested — and did.
    assert el._services() == []
    d = el.bundle_dump("docker is down", "x")
    logs = out / os.path.basename(d) / "logs"
    assert [p.name for p in logs.iterdir()] == ["_MISSED_SERVICES.txt"]
    missed = (logs / "_MISSED_SERVICES.txt").read_text()
    assert "enumerated nothing" in missed
    assert "validator-" not in missed and "full-node" not in missed   # nothing invented


def test_sim_event_noop_under_dry_run(tmp_path):
    st = _state()
    el = EventLog(context=st.event_context, out_dir=str(tmp_path), dry_run=True)
    el.sim_event("churn", "should not write")
    assert not (tmp_path / "events.jsonl").exists()


def test_bundle_summary_lists_demoted_trips(tmp_path, monkeypatch):
    """F6c: bundle.py used to reference NO demoted id — summary.txt named only the un-demoted
    failing id, so a demoted trip was invisible unless another detector failed first, and then only
    as a buried "kind":"diag" line in a run-spanning, copied-whole events.jsonl. The trips are now
    accounted IN-PROCESS and listed in the summary, with the count = distinct entities (the diag
    latch is per (id, entity), so a 2 means two seats tripped it)."""
    st = _state()
    monkeypatch.setattr("dpos_harness.core.events.EventLog._finalized", lambda self: 2295)
    el = EventLog(context=st.event_context, out_dir=str(tmp_path))
    el.sim_event("diag", "[tombstone-backfill-stall] seat vacated by identity 2 never backfilled")
    el.sim_event("diag", "[tombstone-backfill-stall] seat vacated by identity 3 never backfilled")
    el.sim_event("diag", "[bench] promote no-host leak: validator-9")
    el.sim_event("churn", "APPLIED sigkill_restart validator-3")   # not a trip
    d = el.bundle_dump("the chain stopped finalizing", "finalize-stall")
    summary = (tmp_path / os.path.basename(d) / "summary.txt").read_text()
    assert "broken invariant : finalize-stall" in summary           # unchanged
    assert "DEMOTED TRIPS   : 2 demoted invariant(s) fired this run" in summary
    assert "tombstone-backfill-stall x2" in summary and "identity 2" in summary
    assert "bench x1" in summary


def test_bundle_summary_says_none_when_no_demoted_trip(tmp_path, monkeypatch):
    st = _state()
    monkeypatch.setattr("dpos_harness.core.events.EventLog._finalized", lambda self: 2295)
    el = EventLog(context=st.event_context, out_dir=str(tmp_path))
    el.sim_event("churn", "APPLIED sigkill_restart validator-3")
    d = el.bundle_dump("the chain stopped finalizing", "finalize-stall")
    summary = (tmp_path / os.path.basename(d) / "summary.txt").read_text()
    assert "DEMOTED TRIPS   : none this run" in summary


def test_bundle_copies_reborn_and_byz_overlays(tmp_path, monkeypatch):
    """The bundle must capture the per-run generated overlays, not just the base compose files
    (soak-bundle.sh:154-160). `docker-compose.sim.reborn-<slot>.gen.yml` is the ONLY record of
    which on-chain identity a re-tasked slot serves, and `docker-compose.sim.byz-<victim>.gen.yml`
    the only record of who was equivocating; both are unlinked on exit by
    _sweep_generated_overlays, so an uncaptured overlay is lost evidence. All 29 bundles from
    2026-07-22/23 have zero reborn files — that gap forced a byzantine-slash-not-landed diagnosis
    to infer the container→identity mapping instead of reading it."""
    st = _state()
    monkeypatch.setattr("dpos_harness.core.events.EventLog._finalized", lambda self: 2295)
    work = tmp_path / "smoke"
    work.mkdir()
    (work / "docker-compose.sim.gen.yml").write_text("services: {}\n")
    (work / "docker-compose.sim.reborn-validator-13.gen.yml").write_text(
        'services:\n  validator-13:\n    environment:\n      IDENTITY_IDX: "15"\n')
    (work / "docker-compose.sim.byz-validator-7.gen.yml").write_text(
        'services:\n  validator-7:\n    environment:\n      FLUENT_DPOS_BYZANTINE: "equivocate"\n')
    monkeypatch.chdir(work)                      # bundle_dump reads the overlays from the CWD
    out = tmp_path / "out"
    out.mkdir()
    d = EventLog(context=st.event_context, out_dir=str(out)).bundle_dump(
        "byzantine slash never landed", "byzantine-slash-not-landed")
    compose = out / os.path.basename(d) / "compose"
    assert (compose / "docker-compose.sim.gen.yml").is_file()          # base still copied
    assert 'IDENTITY_IDX: "15"' in (
        compose / "docker-compose.sim.reborn-validator-13.gen.yml").read_text()
    assert 'FLUENT_DPOS_BYZANTINE: "equivocate"' in (
        compose / "docker-compose.sim.byz-validator-7.gen.yml").read_text()


def test_status_scrapes_failed_run(tmp_path):
    log = tmp_path / "geo-sim.log"
    log.write_text(
        "  [sim r10 churn] fin=27000 epoch=222 f=2 APPLIED sigkill_restart validator-5\n"
        "  [sim r651 invariant_fail] fin=27020 epoch=222 f=2 finalize-stall\n"
        "SIM FAILURE BUNDLE\n"
        "broken invariant : finalize-stall\n"
        "reason           : the chain stopped finalizing\n"
        "seed             : abc123\n"
        "bundle: ./sim-out/bundle-x\n"
    )
    out = status.render(str(log), "test", pid=None)
    assert "ENDED - FAILED" in out
    assert "finalize-stall" in out
    assert "1 invariant failure(s)" in out
    assert "round 651" in out and "epoch 222" in out


def test_status_scrapes_passed_run(tmp_path):
    log = tmp_path / "geo-sim.log"
    log.write_text("  [sim r5 churn] fin=100 epoch=3 f=2 ok\n"
                   "sim finished cleanly (seed x, 5 rounds)\n")
    out = status.render(str(log), "test", pid=None)
    assert "ENDED - PASSED" in out
    assert "0 invariant failures" in out


def test_status_blaster_sent_inflight_grep(tmp_path):
    """status must surface the sender's sent=/inflight= heartbeat — the cross-module grep
    contract."""
    log = tmp_path / "geo-sim.log"
    log.write_text("  [sim r1 churn] fin=1 epoch=0 f=2 x\n")
    lh = tmp_path / "load-heavy.log"
    lh.write_text("load-heavy[status]: mode=burn sent=42 inflight=5/48 basefee=7/max=5000 "
                  "sample_receipt(last sender-0)=0x1 head=2295\n")
    out = status.render(str(log), "test", pid=None)
    assert "sent=42 inflight=5/48" in out
