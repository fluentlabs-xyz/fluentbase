"""apply_action state-machine + draw_round forced overrides / empty-pool discipline.

Ports the case-soak.sh apply-dispatch bookkeeping assertions: EVER_FAULTED identity-keying, the
tombstone SEAT-keyed obligation, voluntary_exit release, and the empty-victim-pool → no-victim
rule (never fall back to validator-0)."""

from __future__ import annotations

import ast
import copy
import os
import pathlib

import pytest

from dpos_harness.sim import actions, orchestrator
from dpos_harness.chain.writes import ChainError
from dpos_harness.sim.orchestrator import Orchestrator, SimConfig, SimState
from dpos_harness.sim.rounds import apply_action, draw_round
from dpos_harness.core import proc
from dpos_harness.core.proc import Runner
from dpos_harness.tests.conftest import strict_actuators


class _DryChain:
    """Stub lib_write.Chain — records each chain WRITE, touches no RPC. apply_action writes through
    exactly two arms (delegate_shift / voluntary_exit), both keyed by the SERVED IDENTITY idx."""
    def __init__(self):
        self.writes = []

    def delegate_shift(self, served_idx):
        self.writes.append(("delegate_shift", served_idx))

    def voluntary_exit(self, served_idx):
        self.writes.append(("voluntary_exit", served_idx))
        return True         # Chain.voluntary_exit returns True or RAISES — never False


def _state(**env):
    base = dict(SIM_VALIDATORS="7", SIM_INITIAL_COMMITTEE="7", SIM_SPARES="0",
                SIM_ROTATION_SLOTS="0", SIM_BYZANTINE="1")
    base.update(env)
    os.environ.update(base)
    return SimState(cfg=SimConfig())


def test_transient_fault_marks_disrupted_and_restore_asap():
    st = _state()
    apply_action(st, strict_actuators(), "graceful_stop_restart", "validator-3", cur_epoch=10,
                 chain=_DryChain())
    assert "validator-3" in st.disrupted.split()
    assert st.container.disrupt_kind["validator-3"] == "graceful_stop_restart"
    assert st.container.restore_at["validator-3"] == 10   # restore ASAP (not +down_epochs)
    assert st.identity.ever_faulted["3"] == "graceful_stop_restart"   # identity-keyed ledger


def test_dkg_midwindow_bumps_shareless_and_restore_plus2():
    st = _state()
    apply_action(st, strict_actuators(), "dkg_midwindow_restart", "validator-3", cur_epoch=10,
                 chain=_DryChain())
    assert st.shareless == 1
    assert st.container.restore_at["validator-3"] == 12


def test_byzantine_tombstone_is_seat_keyed_and_permanent():
    st = _state()
    apply_action(st, strict_actuators(), "byzantine_equivocate", "validator-3", cur_epoch=10,
                 chain=_DryChain())
    assert st.tombstones == 1
    assert st.identity.permanently_dead["validator-3"] == 1
    # SEAT-keyed obligation frozen at ident_idx("validator-3") = "3"
    assert st.seat.pending_backfill_by_seat["3"] == 10
    assert st.identity.tombstone_settle_epoch["3"] == 10 + st.cfg.membership_settle
    assert st.settle_until_epoch == 10 + st.cfg.membership_settle


def test_voluntary_exit_releases_identity_to_promotable():
    st = _state(SIM_VOLUNTARY_EXIT="1")
    ch = _DryChain()
    apply_action(st, strict_actuators(), "voluntary_exit", "validator-3", cur_epoch=10, chain=ch)
    # the write goes at the REAL Chain, keyed by the SERVED identity idx (not the container name)
    assert ch.writes == [("voluntary_exit", "3")]
    assert st.voluntary_exits == 1
    assert "3" in st.promotable.split()               # SERVED identity released → re-promotable
    assert "validator-3" not in st.disrupted.split()  # NO mark_disrupted (node stays up)
    assert "validator-3" not in st.identity.permanently_dead  # identity RELEASED, not burned
    assert st.seat.pending_backfill_by_seat["3"] == 10


def test_voluntary_exit_no_state_change_when_gov_fails():
    """A gov round-trip that never landed opens NO window. Chain.voluntary_exit returns True or
    RAISES (there is no False return) — so the ChainError IS the failure signal, and it must be
    caught in apply_action, reported as a churn diagnostic, and treated as landed=False."""
    st = _state(SIM_VOLUNTARY_EXIT="1")

    class _FailChain(_DryChain):
        def voluntary_exit(self, served_idx):
            raise ChainError("gov-not-active", f"proposal not Active for: exit-{served_idx}")

    seen = []
    apply_action(st, strict_actuators(), "voluntary_exit", "validator-3", cur_epoch=10,
                 chain=_FailChain(), event=lambda kind, msg: seen.append((kind, msg)))
    assert st.voluntary_exits == 0
    assert st.promotable == ""
    assert "3" not in st.seat.pending_backfill_by_seat   # no phantom settle window / obligation
    assert st.settle_until_epoch == 0
    assert "validator-3" not in st.container.disrupt_kind
    assert seen and seen[0][0] == "churn" and "chain-write deferred" in seen[0][1]


def test_delegate_shift_writes_at_the_chain_and_registers_pending():
    st = _state()
    ch = _DryChain()
    apply_action(st, strict_actuators(), "delegate_shift", "validator-3", cur_epoch=10, chain=ch)
    assert ch.writes == [("delegate_shift", "3")]        # SERVED idx, not the container name
    assert "delegate@13" in st.pending.split()


def test_delegate_shift_registers_no_pending_when_the_write_fails():
    st = _state()

    class _FailShift(_DryChain):
        def delegate_shift(self, served_idx):
            raise ChainError("send", f"delegate reverted for v{served_idx}")

    apply_action(st, strict_actuators(), "delegate_shift", "validator-3", cur_epoch=10, chain=_FailShift())
    assert st.pending == ""     # a shift that never landed leaves no rotation expectation


def test_reborn_victim_resolves_to_served_identity():
    st = _state()
    st.container.slot_identity["validator-6"] = "16"   # reborn container serves minted idx 16
    apply_action(st, strict_actuators(), "byzantine_equivocate", "validator-6", cur_epoch=5,
                 chain=_DryChain())
    assert st.identity.ever_faulted["16"] == "byzantine_equivocate"   # SERVED idx, not native 6
    assert "16" in st.seat.pending_backfill_by_seat


# ── draw_round: empty victim pool → NO victim (never validator-0 fallback) ────
def test_empty_victim_pool_yields_no_victim_but_consumes_draws():
    st = _state()
    st.cur_committee = ""   # empty committee → empty victim pool
    d = draw_round(st, st.cfg.actions_pool(), [], cur_epoch=5, calm_permille=400)
    assert d.victim == ""                 # NOT validator-0
    assert st.prng.ctr == 4               # all four draws consumed (replay determinism preserved)


def test_act_byzantine_merges_live_ambient_compose_file(monkeypatch, tmp_path):
    """v61.11 regression: act_byzantine must base its merged COMPOSE_FILE on the LIVE ambient
    os.environ value (bringup stamps it at phase B), NOT the construction-time self.env snapshot
    (still the empty startup sentinel). The old code appended ':overlay' onto that '' → a
    leading-colon COMPOSE_FILE that dropped the base sim files, so `up --force-recreate` was a
    silent no-op and the victim never received FLUENT_DPOS_BYZANTINE → byzantine-slash-not-landed.
    """
    monkeypatch.chdir(tmp_path)
    # self.env is the stale snapshot: COMPOSE_FILE captured before bring-up (empty), like real life.
    act = actions.Actuators(env={"COMPOSE_FILE": ""}, dry_run=False)
    # bring-up has since stamped the real phase-B file list into the process-global os.environ.
    base = "docker-compose.sim.gen.yml:docker-compose.sim.dpos.gen.yml"
    monkeypatch.setenv("COMPOSE_FILE", base)

    seen = {}

    def fake_run(cmd, **kw):
        seen["argv"] = list(cmd)
        seen["compose_file"] = kw["env"].get("COMPOSE_FILE")
        class _R:  # noqa: D401
            returncode = 0
            stdout = stderr = ""
        return _R()

    # Patched at the SEAM (proc.subprocess), not at `actions` — actions no longer imports
    # subprocess at all, which is the contract test_exec_seam enforces.
    monkeypatch.setattr(proc.subprocess, "run", fake_run)
    act.act_byzantine("validator-4", "equivocate")

    overlay = "docker-compose.sim.byz-validator-4.gen.yml"
    assert seen["argv"] == ["docker", "compose", "up", "-d", "--no-deps",
                            "--force-recreate", "validator-4"]
    assert seen["compose_file"] == f"{base}:{overlay}"          # base preserved, overlay appended
    assert not seen["compose_file"].startswith(":")             # no leading-colon no-op
    assert os.path.exists(overlay)                              # overlay actually written
    import pathlib
    assert 'FLUENT_DPOS_BYZANTINE: "equivocate"' in pathlib.Path(overlay).read_text()


def test_act_byzantine_falls_back_to_self_env_when_ambient_unset(monkeypatch, tmp_path):
    """When os.environ has no COMPOSE_FILE (e.g. an operator harness that only seeded self.env),
    fall back to self.env — and never emit a leading-colon (base-dropping) value."""
    monkeypatch.chdir(tmp_path)
    monkeypatch.delenv("COMPOSE_FILE", raising=False)
    act = actions.Actuators(env={"COMPOSE_FILE": "base.yml"}, dry_run=False)
    seen = {}

    def fake_run(cmd, **kw):
        seen["compose_file"] = kw["env"].get("COMPOSE_FILE")
        class _R:
            returncode = 0
            stdout = stderr = ""
        return _R()

    monkeypatch.setattr(proc.subprocess, "run", fake_run)
    act.act_byzantine("validator-4", "equivocate")
    assert seen["compose_file"] == "base.yml:docker-compose.sim.byz-validator-4.gen.yml"


def test_forced_byzantine_override_precedes_voluntary_exit():
    st = _state(SIM_VOLUNTARY_EXIT="1", SIM_FORCE_BYZANTINE_EPOCH="3",
                SIM_FORCE_VOLUNTARY_EXIT_EPOCH="3")
    # committee of validator-2..6 mapped; growth complete so overrides are eligible
    addrs = [f"0x{i:040x}" for i in range(7)]
    st.cur_committee = " ".join(addrs)
    st.address.addr2idx = {a: f"validator-{i}" for i, a in enumerate(addrs)}
    st.next_joiner = st.cfg.validators
    st.grow_landed = st.cfg.validators - 1
    vpool = [f"validator-{i}" for i in range(2, 7)]
    d = draw_round(st, st.cfg.actions_pool(), vpool, cur_epoch=5, calm_permille=400)
    assert d.forced == "byzantine" and d.action == "byzantine_equivocate"
    assert d.is_calm == 0   # forced override beats the calm bit


# ── the two graceful-stop barriers (F5, lib.sh:66-138) ────────────────────────
#
# Neither existed in the port: act_graceful_stop ran a bare `compose stop`, and
# act_dkg_midwindow_restart's docstring CLAIMED a shutdown_flushed barrier while running
# stop→start with none. These pin the three things that carry the value — the cursor is captured
# BEFORE the stop, the flush barrier reads container METADATA (never logs), and the tripwire never
# trips on absence.

_ACK = "application did not acknowledge block"


def _stopped_act(events=None, dry_run=False):
    return actions.Actuators(env={"COMPOSE_FILE": "x.yml"}, dry_run=dry_run, events=events)


def test_shutdown_flushed_polls_inspect_never_logs(monkeypatch):
    """The barrier must read container METADATA. It WAS a `docker compose logs | grep` and that
    raced: under heavy daemon load the log read lagged past the poll window and false-reported
    'flush incomplete' for a node that exited 0 in <1 s (diagnosed 2026-06-05). Also pins `ps -aq`
    — `-q` is running-only and would resolve nothing for a container that has just stopped."""
    act = _stopped_act()
    argv = []

    def fake_run(cmd, timeout=90):
        argv.append(cmd)
        return "deadbeefcafe\n" if "ps" in cmd else "false 0\n"

    monkeypatch.setattr(act, "_run", fake_run)
    assert act._shutdown_flushed("validator-3") is True
    assert argv[0] == ["docker", "compose", "ps", "-aq", "validator-3"]
    assert argv[1] == ["docker", "inspect", "deadbeefcafe", "--format",
                       "{{.State.Running}} {{.State.ExitCode}}"]
    assert not any("logs" in c for c in argv), "the barrier went back to log-grepping"


def test_shutdown_flushed_false_on_empty_cid_and_on_a_still_running_container(monkeypatch):
    """Two negative arms. Both are non-fatal by contract (bash `|| true`) — the caller only loses
    proof of a clean exit — but the barrier must still return False rather than True-by-default."""
    act = _stopped_act()
    monkeypatch.setattr(actions.time, "sleep", lambda _s: None)
    monkeypatch.setattr(act, "_run", lambda cmd, timeout=90: "")
    assert act._shutdown_flushed("validator-3") is False        # no container id at all

    polls = []

    def fake_run(cmd, timeout=90):
        polls.append(cmd)
        return "cafe\n" if "ps" in cmd else "true 0\n"          # still running, exit code stale

    monkeypatch.setattr(act, "_run", fake_run)
    assert act._shutdown_flushed("validator-3") is False
    assert len(polls) == 11, "expected the cid read + exactly 10 bounded polls"


def test_shutdown_flushed_short_circuits_under_dry_run():
    act = _stopped_act(dry_run=True)
    act._run = lambda cmd, timeout=90: pytest.fail("dry-run touched docker")
    assert act._shutdown_flushed("validator-3") is True


def test_marshal_ack_tripwire_trips_on_the_ack_line():
    """A positive detection is a PRODUCT-safety witness: the marshal died on a graceful, unhalted
    stop, so an Exact ack was dropped. It raises (the fail-loud translation of bash's `_die`) and
    is deliberately NOT routed through fatal_or_diag — nothing may silence it via a set edit."""
    from dpos_harness.core.policy import DEMOTED_INVARIANTS
    events = []
    act = _stopped_act(events=events)
    act._logs_since = lambda v, since: f"some line\n{_ACK} 4711\nmore\n"
    with pytest.raises(ChainError) as e:
        act._marshal_ack_tripwire("validator-3", "2026-07-30T00:00:00Z")
    assert e.value.reason_id == "marshal-ack-tripwire"
    assert "validator-3" in e.value.message
    assert events == [("diag", e.value.message)]
    assert "marshal-ack-tripwire" not in DEMOTED_INVARIANTS


@pytest.mark.parametrize("marker", ["SafetyHalt", "subsystem failed",
                                    "subsystem exited cleanly (unexpected)"])
def test_marshal_ack_tripwire_scoped_out_on_non_graceful_paths(marker):
    """The ack line IS legitimate when the stop was not graceful — an AbortAll subsystem exit
    (outer.rs aborts the executor one statement before the marshal, so a multi-threaded runtime can
    poll the marshal in the gap) or a SafetyHalt. Each marker alone must scope the trip out."""
    events = []
    act = _stopped_act(events=events)
    act._logs_since = lambda v, since: f"{_ACK}\n{marker} whatever\n"
    act._marshal_ack_tripwire("validator-3", "2026-07-30T00:00:00Z")   # no raise
    assert events == []


def test_marshal_ack_tripwire_is_clean_on_an_unreadable_log(monkeypatch):
    """It must NEVER trip on absence. nodes.logs_since yields "" for an unreadable log and for a
    genuinely empty slice alike; both end clean — but only after the empty-slice retries."""
    act = _stopped_act(events=[])
    monkeypatch.setattr(actions.time, "sleep", lambda _s: None)
    reads = []
    act._logs_since = lambda v, since: (reads.append(since), "")[1]
    act._marshal_ack_tripwire("validator-3", "2026-07-30T00:00:00Z")   # no raise
    assert len(reads) == 5, "an unreadable slice must be re-read, not accepted at once"


def test_marshal_ack_tripwire_retries_while_the_slice_is_empty(monkeypatch):
    """A graceful stop ALWAYS logs something, so an empty slice means a lagging docker-logs read,
    not a clean one. Accepting the first empty read would silently blind the tripwire under exactly
    the daemon load that makes a marshal death likely."""
    act = _stopped_act(events=[])
    monkeypatch.setattr(actions.time, "sleep", lambda _s: None)
    slices = ["", "", f"{_ACK}\n"]
    act._logs_since = lambda v, since: slices.pop(0)
    with pytest.raises(ChainError):
        act._marshal_ack_tripwire("validator-3", "2026-07-30T00:00:00Z")
    assert slices == []


def test_marshal_ack_tripwire_short_circuits_under_dry_run():
    act = _stopped_act(dry_run=True)
    act._logs_since = lambda v, since: pytest.fail("dry-run read a log")
    act._marshal_ack_tripwire("validator-3", "2026-07-30T00:00:00Z")


def test_log_cursor_is_utc_rfc3339_to_the_second():
    """`date -u +%Y-%m-%dT%H:%M:%SZ`. The format is load-bearing: docker compares the `--since`
    cursor against DAEMON-side host timestamps."""
    import re as _re
    assert _re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", actions.Actuators._log_cursor())


def _barrier_order(act, calls, logs="reth: shutting down gracefully\n"):
    """Wire every seam of the stop path to a recorder, so the assertion is on the ORDER. The
    default slice is non-empty and benign — what a real graceful stop produces — so the tripwire
    reads once and stays clean."""
    act._node_finalized = lambda v: 5
    act._log_cursor = lambda: (calls.append("cursor"), "2026-07-30T00:00:00Z")[1]
    act._run_ok = lambda cmd: (calls.append(" ".join(cmd)), True)[1]
    act._shutdown_flushed = lambda v: (calls.append("flushed"), True)[1]
    act._logs_since = lambda v, since: (calls.append(f"tripwire<-{since}"), logs)[1]


def test_act_graceful_stop_captures_the_cursor_before_the_stop():
    """THE ordering assertion. A cursor taken after the stop would start the slice mid-shutdown and
    could miss the ack line entirely; one taken far earlier would let an unrelated halt hours ago
    both trip and scope out. Ownership of the sequence lives INSIDE the act_* method precisely so
    no caller can drift it — the bash sites had to repeat all five steps by hand."""
    calls = []
    act = _stopped_act()
    _barrier_order(act, calls)
    act.act_graceful_stop("validator-3")
    assert calls == ["cursor",
                     "docker compose stop --timeout 40 validator-3",
                     "flushed",
                     "tripwire<-2026-07-30T00:00:00Z"]


def test_act_dkg_midwindow_restart_restarts_only_after_both_barriers():
    """Same ordering, plus the restart LAST. This action's whole point is a COMPLETED persist so
    the victim resumes its share from the on-disk journal; restarting before the exit is observed
    is a truncated CeremonyStore → share-less resume → warm-debt hang."""
    calls = []
    act = _stopped_act()
    _barrier_order(act, calls)
    act.act_dkg_midwindow_restart("validator-3")
    assert calls == ["cursor",
                     "docker compose stop --timeout 40 validator-3",
                     "flushed",
                     "tripwire<-2026-07-30T00:00:00Z",
                     "docker compose start validator-3"]


def test_act_graceful_stop_notes_a_barrier_that_never_saw_a_clean_exit():
    """bash `shutdown_flushed || true` discarded this entirely. Non-fatal here too — but recorded,
    because a stop with no observed exit 0 is the pre-condition for the truncated-persist class."""
    events = []
    act = _stopped_act(events=events)
    calls = []
    _barrier_order(act, calls)
    act._shutdown_flushed = lambda v: False
    act.act_graceful_stop("validator-3")
    assert [k for k, _ in events] == ["churn"]
    assert "validator-3" in events[0][1]


# ── the actuator SURFACE conformance pair (F4.4) ──────────────────────────────
#
# Both of these exist because the catch-all doubles hid three names for the whole life of the
# harness. The first is static (every `act.X` in production resolves on Actuators), the second is
# dynamic (the harness's own probe actually runs). Neither could ever fail while the doubles
# manufactured names on demand.

def _production_modules():
    """Every non-test module in the package, DERIVED by walking it.

    The scan used to be pointed at a hand-written tuple of the four modules that then held an
    `Actuators` reference. P3 moved `apply_action` into `sim/rounds.py` — which the tuple did not
    name — and five actuator names silently fell out of the scan while the test stayed green.
    That is the exact failure shape this check exists to catch, so the module list is now derived,
    never listed: a module added or moved anywhere under the package is scanned the day it lands."""
    pkg = pathlib.Path(orchestrator.__file__).parent.parent
    for p in sorted(pkg.rglob("*.py")):
        rel = p.relative_to(pkg)
        if rel.parts[0] == "tests" or "__pycache__" in rel.parts:
            continue
        yield p


def _actuator_attr_names_by_module() -> dict:
    """{module path relative to the package: {attribute names reached on `act`}}.

    DERIVED by AST over the WHOLE package — `act.X`, `self.act.X`, `rec.act.X` — never
    hand-listed. A hand-copied mirror is the same failure shape this whole check exists to remove
    (the `_bind_live_seams` lesson, plan §F3 decision 1): it stops matching silently the first
    time somebody adds an arm or moves a caller."""
    pkg = pathlib.Path(orchestrator.__file__).parent.parent
    out = {}
    for path in _production_modules():
        names = set()
        for node in ast.walk(ast.parse(path.read_text())):
            if not isinstance(node, ast.Attribute):
                continue
            base = node.value
            holder = (base.id if isinstance(base, ast.Name)
                      else base.attr if isinstance(base, ast.Attribute) else "")
            if holder == "act":
                names.add(node.attr)
        if names:
            out[str(path.relative_to(pkg))] = names
    return out


def _actuator_attr_names() -> set:
    return set().union(*_actuator_attr_names_by_module().values())


def test_every_actuator_name_production_uses_exists_on_actuators():
    """SURFACE ASSERTION: nothing production calls on `act` may be missing from `Actuators`.

    This failed before F4.1-4.3 on three names — `act_byzantine_restore` (defined nowhere) and
    `sim_delegate_shift` / `sim_voluntary_exit` (methods of `Chain`, reached for on the
    actuator behind a `getattr`/`hasattr` guard, so they silently no-op'd). It fails again the
    moment somebody renames a method on one side only."""
    used = _actuator_attr_names()
    missing = sorted(used - set(dir(actions.Actuators)))
    assert missing == [], (f"production reaches for {missing} on the actuator, but actions."
                           "Actuators does not define it")


def test_every_actuator_arm_is_reached_by_production():
    """ANTI-VACUITY, and the reverse half of the surface contract: every `act_*` arm defined on
    `Actuators` must be reached somewhere in production.

    The old guard was `assert "act_byzantine" in used` — the presence of ONE lucky name. When the
    P3 move narrowed the scan from 18 references to 11, `dispatch.py` still happened to supply
    `act_byzantine`, so the guard passed while five arms went unscanned. A SET equality over the
    whole `act_*` surface cannot be satisfied by a narrowed scan: drop a module from the scan and
    the arms only it reaches show up here as unreached. It also catches the opposite defect — an
    arm added to `Actuators` and never wired to a caller, which is a dead actuator."""
    used = _actuator_attr_names()
    defined = {a for a in dir(actions.Actuators) if a.startswith("act_")}
    unreached = sorted(defined - used)
    assert unreached == [], (
        f"{unreached} are defined on actions.Actuators but no production module reaches them on "
        f"`act`. Either they are dead, or the AST scan has narrowed again — it currently sees "
        f"{ {m: len(n) for m, n in sorted(_actuator_attr_names_by_module().items())} }")


def _state_fingerprint(st):
    """A comparable snapshot of SimState. Every field compares by value (the sub-dataclasses
    generate __eq__) EXCEPT `prng`, a plain class whose default identity __eq__ would report a
    deep copy as different; its whole mutable state is the (seed, ctr) stream position."""
    import dataclasses
    snap = {f.name: copy.deepcopy(getattr(st, f.name))
            for f in dataclasses.fields(st) if f.name != "prng"}
    snap["prng"] = (st.prng.seed, st.prng.ctr)
    return snap


def test_self_check_walks_every_action_against_a_real_dry_actuators_and_chain():
    """Orchestrator.self_check() is the harness's OWN conformance probe (case-soak.sh:2077): it
    dry-runs the whole apply dispatch over every action in the pool against a REAL
    `Actuators(dry_run=True)` + a REAL dry `Chain`. No test had ever invoked it — it is
    monkeypatched away in both loop-driving tests (test_launch_gate.py:187, test_live_seams.py:316),
    so a missing actuator/chain method would have raised at sim startup and nowhere else.

    Asserted here: it returns True, it leaves the live state untouched (the deep-copy rollback that
    replaces bash's subshell isolation), and both chain-writing arms REALLY execute rather than
    silently no-op — the dry Runner must carry the delegate approve/delegate pair and the
    voluntary-exit gov round-trip."""
    o = Orchestrator(cfg=SimConfig(validators=7, initial_committee=7, spares=0, rotation_slots=0),
                     dry_run=True)
    captured = Runner(dry=True)
    o._runner = lambda dry=False, echo=False: captured

    before = _state_fingerprint(o.state)
    assert o.self_check() is True
    assert _state_fingerprint(o.state) == before, \
        "self_check mutated the live state — the deep-copy rollback broke"

    notes = {inv.note for inv in captured.log}
    assert {"delegate-approve", "delegate"} <= notes          # the delegate_shift arm really ran
    assert {"gov-propose", "gov-vote", "gov-execute"} <= notes  # the voluntary_exit arm really ran
