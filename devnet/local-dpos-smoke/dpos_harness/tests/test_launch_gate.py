"""test_launch_gate.py — the launch-gate review findings (bash→Python sim-harness port).

Covers the fixes that don't have a natural home in an existing test module: build_addr_map (F2),
the fail-bundle helper + the persistent-battery loop (F1/F3/F4/F5/F6), the spammer pool (F11),
the proc lifecycle/capture seam (F12), the Chain.send transient-retry reachability (F13), the
sender wallet-new/funder fixes (F14), the refill convergence recompute (F15), and the
self-partition deferral charge (F16). The reconciler-side findings (F7/F8/F9) live in
test_reconcilers.py; the shareless-honesty split (F19) lives in test_dispatch_round.py."""

import os

import pytest

from dpos_harness.sim.orchestrator import Orchestrator, SimConfig, SimState
from dpos_harness.sim.reconcilers import Reconcilers
from dpos_harness.tests.conftest import strict_actuators


# ── F2: build_addr_map (string-keyed OWNER_ADDR_CACHE) ────────────────────────
class _AddrChain:
    def __init__(self):
        self.calls = 0

    def owner_addr(self, idx):
        self.calls += 1
        return f"0xADDR{idx}"                      # mixed-case → build_addr_map lowercases


def _state():
    return SimState(cfg=SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1))


def test_build_addr_map_native_and_string_keyed_cache():
    from dpos_harness.chain.writes import build_addr_map
    s = _state()
    s.max_minted_idx = 2
    ch = _AddrChain()
    build_addr_map(s, ch)
    assert s.address.addr2idx == {
        "0xaddr0": "validator-0", "0xaddr1": "validator-1", "0xaddr2": "validator-2"}
    # OWNER_ADDR_CACHE keyed by str(idx) — the bash-faithful domain (F2)
    assert s.address.owner_addr_cache == {"0": "0xaddr0", "1": "0xaddr1", "2": "0xaddr2"}
    # a rebuild is memoized — no second owner_addr spawn per identity
    ch.calls = 0
    build_addr_map(s, ch)
    assert ch.calls == 0


def test_build_addr_map_reborn_slot_maps_bench_idx():
    from dpos_harness.chain.writes import build_addr_map
    s = _state()                                  # val_containers = 4+1+1 = 6
    s.max_minted_idx = 6
    s.container.slot_identity["validator-5"] = "6"   # reborn as bench idx 6 (str = production domain)
    build_addr_map(s, _AddrChain())
    assert s.address.addr2idx["0xaddr6"] == "validator-5"   # hosted bench idx → its live slot
    assert "validator-5" not in s.address.addr2idx.values() or \
        s.address.addr2idx.get("0xaddr5") is None           # native idx 5 unmapped (re-tasked)


# ── F5: fail_bundle helper (event + bundle + teardown) ────────────────────────
def test_fail_bundle_emits_event_bundle_and_tears_down():
    orch = Orchestrator()
    calls = []
    tore_down = {"v": False}

    class FakeELog:
        def sim_event(self, kind, note):
            calls.append(("event", kind))

        def bundle_dump(self, msg, inv_id):
            calls.append(("bundle", inv_id))

    orch._fail_bundle(FakeELog(), "committee-undersized", "boom",
                      lambda: tore_down.__setitem__("v", True))
    assert ("event", "invariant_fail") in calls
    assert ("bundle", "committee-undersized") in calls
    assert tore_down["v"] is True


# ── F1/F3/F4/F6: the persistent-battery live loop ─────────────────────────────
class _LoopChain:
    def __init__(self, committee="0xa 0xb 0xc 0xd 0xe 0xf 0xg", empty_in_loop=False):
        self.committee_str = committee
        self.empty_in_loop = empty_in_loop
        self.reads = 0

    def current_epoch(self):
        return 5

    def owner_addr(self, idx):
        # build_addr_map (run() startup, case-soak.sh:2133) resolves each identity's owner addr;
        # map idx→the idx-th committee token so ADDR2IDX comes out populated + consistent.
        parts = "0xa 0xb 0xc 0xd 0xe 0xf 0xg".split()
        return parts[int(idx)] if int(idx) < len(parts) else ""

    def committee(self, e):
        self.reads += 1
        # the first read is the STARTUP self-check (must be readable); later reads are the loop's
        # per-tick read, which can go UNREADABLE to exercise the F6 vacuous-tick path.
        if self.empty_in_loop and self.reads > 1:
            return ""
        return "0xa 0xb 0xc 0xd 0xe 0xf 0xg" if self.empty_in_loop else self.committee_str


class _CountBattery:
    instances = 0
    checks = 0

    def __init__(self, events=None):
        _CountBattery.instances += 1
        self.events = events if events is not None else []
        self.ctx = None
        self.sim = None
        self.inv_fail_id = ""
        self.inv_fail_msg = ""

    def bind(self, ctx, sim):
        """Same shape as Battery.bind — BOTH halves or neither. A stub that only accepted the
        chain half would let the loop bind half the context and stay green."""
        self.ctx = ctx
        self.sim = sim

    def check_invariants(self):
        _CountBattery.checks += 1
        return True


# The RECONCILERS surface, taken off a real instance (so instance attributes like
# `epoch_interval`, which the run-config record reads, count too). Resolved once at import — the
# loop tests monkeypatch battery.Battery / dispatch.Dispatcher, and F3 proved that a module-level
# patch means importing "the real class" from inside a helper hands you the patch instead.
_REC_SURFACE = frozenset(dir(Reconcilers(SimState(cfg=SimConfig()), None, None)))


class _RecStub:
    """A no-op Reconcilers double for the run()-loop tests — SURFACE-CHECKED, not a catch-all.

    It stands in for `Reconcilers`, NOT for `Actuators`, so an autospec of `Actuators` would be the
    wrong replacement (F4.4). What it must not keep is the blanket `__getattr__` that manufactured
    ANY name: run() calls twelve methods on `rec`, and a rename on the production side would have
    gone on quietly answering here. Names outside the real Reconcilers surface now raise."""

    def __init__(self):
        self.events = []
        self.committee_read_seen = []
        self.epoch_interval = 64          # read by the run-config record (orchestrator.py:1074)

    def __getattr__(self, name):
        if name not in _REC_SURFACE:
            raise AttributeError(
                f"the run loop reached for rec.{name!r}, which reconcilers.Reconcilers does not "
                "define — a wiring gap the old catch-all double would have swallowed")
        return lambda *a, **k: None


class _DispStub:
    last = None

    def __init__(self, state, *a, **k):
        self.rounds = 0
        self.state = state
        _DispStub.last = self

    def run_round(self, cur, n):
        self.rounds += 1
        self.state.round += 1          # mimic the SINGLE real increment (dispatch.run_round, NEW-2)


class _ELogStub:
    def __init__(self, *a, **k):
        self.events = []

    def sim_event(self, kind, note):
        self.events.append((kind, note))

    def bundle_dump(self, *a, **k):
        pass


def _wire_loop(monkeypatch, orch, committee="0xa 0xb 0xc 0xd 0xe 0xf 0xg", chain=None):
    import dpos_harness.stack.bringup as BU
    import dpos_harness.sim.dispatch as DP
    import dpos_harness.core.events as BD
    import dpos_harness.checks.battery as BAT
    import dpos_harness.sim.orchestrator as O

    class FakeBringUp:
        def __init__(self, cfg, runner):
            import types
            self.spammers = types.SimpleNamespace(stop=lambda: None)

        def run(self):
            return True

    _CountBattery.instances = 0
    _CountBattery.checks = 0
    _DispStub.last = None
    monkeypatch.setattr(BU, "BringUp", FakeBringUp)
    monkeypatch.setattr(DP, "Dispatcher", _DispStub)
    monkeypatch.setattr(BAT, "Battery", _CountBattery)
    monkeypatch.setattr(BD, "EventLog", _ELogStub)
    monkeypatch.setattr(O.time, "sleep", lambda s: None)
    chain = chain or _LoopChain(committee)
    elog_holder = {}
    orig_elog = _ELogStub

    class _CapELog(orig_elog):
        def __init__(self, *a, **k):
            super().__init__(*a, **k)
            elog_holder["e"] = self
    monkeypatch.setattr(BD, "EventLog", _CapELog)
    monkeypatch.setattr(orch, "_build_chain", lambda r, b: chain)
    monkeypatch.setattr(orch, "_reconcilers", lambda c: (_RecStub(), object()))
    monkeypatch.setattr(orch, "_bind_live_seams", lambda *a: None)
    monkeypatch.setattr(orch, "self_check", lambda: True)
    monkeypatch.setattr(orch, "selfcheck_result_seams_or_abort", lambda **k: None)
    return chain, elog_holder


def test_loop_uses_one_persistent_battery_and_churns(monkeypatch):
    os.environ.update({"SIM_DURATION": "0", "SIM_STOP_ROUND": "2",
                       "SIM_CHURN_PERIOD": "0s", "SIM_CHECK_PERIOD": "0s"})
    try:
        orch = Orchestrator(SimConfig(validators=7, initial_committee=7,
                                       spares=0, rotation_slots=0, stop_round=2))
        import dpos_harness.sim.orchestrator as O
        monkeypatch.setattr(O.time, "time", lambda: 0.0)   # constant clock → duration never trips
        _wire_loop(monkeypatch, orch)
        rc = orch.run()
        assert rc == 0
        assert _CountBattery.instances == 1                # F1: ONE battery for the whole run
        assert _CountBattery.checks >= 2                   # F1/F3: battery runs every tick
        assert _DispStub.last.rounds == 2                  # F3: churn cadence fired per round
        assert orch.state.round == 2                        # NEW-2: round advances ONCE per churn
        assert orch.state.committee_read_ok == 1           # F6: readable committee
    finally:
        for k in ("SIM_DURATION", "SIM_STOP_ROUND", "SIM_CHURN_PERIOD", "SIM_CHECK_PERIOD"):
            os.environ.pop(k, None)


def test_loop_duration_run_end(monkeypatch):
    os.environ.update({"SIM_DURATION": "1s", "SIM_CHURN_PERIOD": "0s", "SIM_CHECK_PERIOD": "0s"})
    try:
        orch = Orchestrator(SimConfig(validators=7, initial_committee=7,
                                       spares=0, rotation_slots=0))
        import dpos_harness.sim.orchestrator as O
        clock = iter([0.0, 5.0, 5.0, 5.0])                 # start=0, first now=5 >= dur → break (F4)
        monkeypatch.setattr(O.time, "time", lambda: next(clock))
        _chain, elog_holder = _wire_loop(monkeypatch, orch)
        rc = orch.run()
        assert rc == 0
        assert ("end", "duration reached") in elog_holder["e"].events   # F4
    finally:
        for k in ("SIM_DURATION", "SIM_CHURN_PERIOD", "SIM_CHECK_PERIOD"):
            os.environ.pop(k, None)


def test_loop_marks_unreadable_committee_vacuous(monkeypatch):
    os.environ.update({"SIM_DURATION": "0", "SIM_STOP_ROUND": "1",
                       "SIM_CHURN_PERIOD": "0s", "SIM_CHECK_PERIOD": "0s"})
    try:
        orch = Orchestrator(SimConfig(validators=7, initial_committee=7,
                                       spares=0, rotation_slots=0, stop_round=1))
        import dpos_harness.sim.orchestrator as O
        monkeypatch.setattr(O.time, "time", lambda: 0.0)
        _chain, elog_holder = _wire_loop(monkeypatch, orch,
                                         chain=_LoopChain(empty_in_loop=True))  # UNREADABLE in-loop
        orch.run()
        assert orch.state.committee_read_ok == 0           # F6: visibility, not papered over
        assert orch.state.battery_coverage == 0            # vacuous tick claims no membership
        assert any(k == "committee_read_fail" for k, _ in elog_holder["e"].events)
    finally:
        for k in ("SIM_DURATION", "SIM_STOP_ROUND", "SIM_CHURN_PERIOD", "SIM_CHECK_PERIOD"):
            os.environ.pop(k, None)


# ── F11: SpammerPool ──────────────────────────────────────────────────────────
def test_spammer_pool_dry_is_noop():
    from dpos_harness.stack.bringup import SpammerPool
    sp = SpammerPool(dry=True)
    sp.start("0xkey", "0xto", "http://localhost:8545")
    assert sp.threads == []                        # dry never spawns a thread


def test_spammer_pool_live_starts_and_stops(monkeypatch):
    import dpos_harness.stack.bringup as BU
    from dpos_harness.core import proc
    monkeypatch.setattr(proc.subprocess, "run", lambda *a, **k: None)
    sp = BU.SpammerPool(dry=False)
    sp.start("0xkey", "0xto", "http://localhost:8545")
    assert len(sp.threads) == 1
    sp.stop()
    assert sp.threads == []


# ── F12: proc lifecycle seam ──────────────────────────────────────────────────
def test_run_checked_raises_on_failure():
    from dpos_harness.core.proc import Runner, ProcError
    r = Runner()
    res = r.run_capture(["false"])
    assert res.rc != 0 and not res.ok
    with pytest.raises(ProcError):
        r.run_checked(["false"])


def test_run_checked_dry_is_ok():
    from dpos_harness.core.proc import Runner
    r = Runner(dry=True)
    assert r.run_checked(["docker", "compose", "up", "--build"]).ok


def test_run_capture_timeout_sentinel():
    from dpos_harness.core.proc import Runner
    r = Runner()
    res = r.run_capture(["sleep", "2"], timeout=0.15)
    assert res.timed_out and res.stderr == "<timed-out>" and not res.ok


# ── F13: Chain.send transient retry reachability ──────────────────────────────
class _QueueRunner:
    def __init__(self, captures):
        self.dry = False
        self._q = list(captures)
        self.capture_calls = 0

    def run(self, argv, **k):
        return ""                                  # empty sender/nonce → skip the nonce short-circuit

    def run_capture(self, argv, **k):
        self.capture_calls += 1
        return self._q.pop(0)


def test_chain_send_retries_on_transient_then_succeeds(monkeypatch):
    from dpos_harness.chain.writes import Chain
    from dpos_harness.core.proc import RunResult
    import dpos_harness.chain.writes as LW
    monkeypatch.setattr(LW.time, "sleep", lambda s: None)
    transient = RunResult(argv=["cast"], stderr="nonce too low", rc=1)
    ok = RunResult(argv=["cast"], stdout='{"status":"0x1"}', rc=0)
    runner = _QueueRunner([transient, ok])
    chain = Chain(runner=runner, RPC="http://localhost:8545")
    chain.send("lbl", "0xto", "sig()", key="0xk")  # must NOT raise — retry reaches the 2nd send
    assert runner.capture_calls == 2


def test_chain_send_retries_on_timeout(monkeypatch):
    from dpos_harness.chain.writes import Chain
    from dpos_harness.core.proc import RunResult
    import dpos_harness.chain.writes as LW
    monkeypatch.setattr(LW.time, "sleep", lambda s: None)
    timed = RunResult(argv=["cast"], stderr="<timed-out>", rc=124, timed_out=True)
    ok = RunResult(argv=["cast"], stdout='{"status":"1"}', rc=0)
    runner = _QueueRunner([timed, ok])
    chain = Chain(runner=runner, RPC="http://localhost:8545")
    chain.send("lbl", "0xto", "sig()", key="0xk")
    assert runner.capture_calls == 2


def test_chain_send_raises_after_max_transients(monkeypatch):
    from dpos_harness.chain.writes import Chain, ChainError
    from dpos_harness.core.proc import RunResult
    import dpos_harness.chain.writes as LW
    monkeypatch.setattr(LW.time, "sleep", lambda s: None)
    t = lambda: RunResult(argv=["cast"], stderr="already known", rc=1)
    runner = _QueueRunner([t(), t(), t()])
    chain = Chain(runner=runner, RPC="http://localhost:8545")
    with pytest.raises(ChainError):
        chain.send("lbl", "0xto", "sig()", key="0xk")


# ── F14: sender wallet-new array + funder fallback ────────────────────────────
def test_new_sender_keys_parses_json_array():
    from dpos_harness.stack import sender
    os.environ["LOAD_SENDERS"] = "1"
    try:
        s = sender.Sender()
        s.p.run = lambda argv, note="": '[{"address":"0xA","private_key":"0xK"}]'
        assert s._new_sender_keys() == [("0xA", "0xK")]
    finally:
        os.environ.pop("LOAD_SENDERS", None)


def test_resolve_funder_once_reads_funded_hex():
    """The funder key lives on the ONE container that mounts /runtime. Pinned as argv against
    the literal `validator-0`: the mount is a topology fact, and reading funded.hex off a
    container without the volume returns "" — which the acquire loop reads as "not up yet" and
    burns the whole LOAD_ACQUIRE_TIMEOUT on before aborting the run."""
    from dpos_harness.stack import sender
    s = sender.Sender()
    s.funder_key = ""
    s.p.dry = False
    seen = []

    def run(argv, note=""):
        seen.append(list(argv))
        return "abc123" if "cat" in argv else ""

    s.p.run = run
    assert s._resolve_funder_once() == "0xabc123"
    assert seen == [["docker", "compose", "exec", "-T", "validator-0",
                     "cat", "/runtime/keys/funded.hex"]]


def test_resolve_funder_once_ps_fallback_targets_the_runtime_mount_container():
    """The docker-ps fallback (compose exec unavailable) must pick the /runtime-mounting
    container out of the full container list — matching on `validator-0`, not the first name
    it sees. Picking validator-3's container reads a path that is not mounted there."""
    from dpos_harness.stack import sender
    s = sender.Sender()
    s.funder_key = ""
    s.p.dry = False
    seen = []

    def run(argv, note=""):
        seen.append(list(argv))
        if argv[:2] == ["docker", "ps"]:
            return "sim-validator-3-1\nsim-validator-0-1\nsim-full-node-1\n"
        if argv[:2] == ["docker", "exec"]:
            return "abc123"
        return ""                                   # compose exec: unavailable

    s.p.run = run
    assert s._resolve_funder_once() == "0xabc123"
    assert seen[-1] == ["docker", "exec", "sim-validator-0-1",
                        "cat", "/runtime/keys/funded.hex"]


def test_resolve_funder_once_returns_empty_on_miss_no_raise():
    """OP-1: a missing funder is NOT a t=0 abort — the single attempt returns "" and the acquire
    loop retries; the loud abort is reserved for LOAD_ACQUIRE_TIMEOUT exhaustion."""
    from dpos_harness.stack import sender
    s = sender.Sender()
    s.funder_key = ""
    s.p.dry = False
    s.p.run = lambda argv, note="": ""             # no funded.hex anywhere
    assert s._resolve_funder_once() == ""          # no SystemExit at t=0


def test_acquire_waits_then_proceeds_when_funder_appears(monkeypatch):
    """OP-1: funder key absent for the first N probe ticks, then materializes → the sender
    ACQUIRES (no abort). Clock + sleep stubbed so no wall time passes."""
    from dpos_harness.stack import sender
    s = sender.Sender()
    s.p.dry = False
    s.wait_marker_log = ""                          # marker gate off for this test
    s.acquire_interval = 15
    s.acquire_timeout = 1800
    clock = {"t": 0.0}
    monkeypatch.setattr(sender.time, "time", lambda: clock["t"])
    monkeypatch.setattr(s._stop, "wait", lambda d: clock.__setitem__("t", clock["t"] + d) or False)
    monkeypatch.setattr(s, "_rpc_ready", lambda: True)
    ticks = {"n": 0}

    def funder():
        ticks["n"] += 1
        return "0xFEED" if ticks["n"] >= 3 else ""   # appears on the 3rd probe
    monkeypatch.setattr(s, "_resolve_funder_once", funder)
    assert s._acquire() is True
    assert s.funder_key == "0xFEED"
    assert ticks["n"] == 3                            # retried until it appeared, never aborted


def test_acquire_loud_abort_only_at_timeout(monkeypatch):
    """OP-1: funder NEVER appears → the loud zero-load abort fires AT LOAD_ACQUIRE_TIMEOUT
    exhaustion (clock stubbed), not at t=0."""
    from dpos_harness.stack import sender
    s = sender.Sender()
    s.p.dry = False
    s.wait_marker_log = ""
    s.acquire_interval = 15
    s.acquire_timeout = 30
    clock = {"t": 0.0}
    monkeypatch.setattr(sender.time, "time", lambda: clock["t"])
    monkeypatch.setattr(s._stop, "wait", lambda d: clock.__setitem__("t", clock["t"] + d) or False)
    monkeypatch.setattr(s, "_rpc_ready", lambda: True)
    monkeypatch.setattr(s, "_resolve_funder_once", lambda: "")   # never materializes
    with pytest.raises(SystemExit) as e:
        s._acquire()
    assert "acquisition timed out" in str(e.value)   # abort at exhaustion, not preflight


# ── NEW-1: reborn identity fault resolves via cache (str key domain) ──────────
def test_reborn_fault_resolves_expected_not_wrongful():
    """End-to-end: a rotation slot reborn to serve an INT-flavored minted idx (the production
    dispatch→hostpool path) → a fault stamped through ident_idx lands in EVER_FAULTED under the
    STR key, build_addr_map caches the owner addr under the SAME str key, and the wrongful-slash
    detector resolves the on-chain jail to EXPECTED (idx in EVER_FAULTED) — never a spurious
    wrongful-slash for a legitimately-faulted rebirth (NEW-1)."""
    from dpos_harness.sim.rounds import apply_action
    from dpos_harness.sim.hostpool import HostPool
    from dpos_harness.chain.writes import build_addr_map
    from dpos_harness.checks.battery import Battery
    from dpos_harness.sim import state_ctx

    class _Chain16:
        def owner_addr(self, idx):
            return f"0xOWNER{idx}"

        def committee_has(self, addr, epoch):
            return False

    s = _state()                                        # val_containers = 4+1+1 = 6
    chain = _Chain16()
    hp = HostPool(s, chain, None)
    hp.retask_slot("validator-5", 16)                   # INT input from the production path
    assert s.container.slot_identity["validator-5"] == "16"   # stored as STR (NEW-1)

    apply_action(s, strict_actuators(), "sigkill_restart", "validator-5", cur_epoch=5, chain=chain)
    assert s.identity.ever_faulted == {"16": "sigkill_restart"}   # str key via ident_idx

    s.max_minted_idx = 16
    build_addr_map(s, chain)
    assert s.address.owner_addr_cache["16"] == "0xowner16"        # cached under the SAME str key

    ctx, sim = state_ctx.to_ctx(s)
    sim.EVER_MEMBERS = {"0xowner16": 1}                 # sim bookkeeping half
    ctx.SIM_TICK = 0                                   # % SIM_WRONGSLASH_EVERY == 0 → detector runs
    bat = Battery(ctx=ctx, sim=sim)
    bat._pp_validator_status = lambda addr: 3           # on-chain JAIL status byte
    for _ in range(bat.SIM_WRONGSLASH_BELT + 2):
        assert bat._inv_wrongful_slash() is True        # EXPECTED jail → never fails
    assert bat.inv_fail_id == ""                         # no wrongful-slash for the reborn identity


def test_funder_key_leading_zero_preserved():
    """LOW: a funder key with a leading zero nibble after 0x must be preserved (removeprefix, not
    lstrip which would eat the 0)."""
    from dpos_harness.stack import sender
    s = sender.Sender()
    s.funder_key = ""
    s.p.dry = False
    s.p.run = lambda argv, note="": "0x0abc" if "cat" in argv else ""
    assert s._resolve_funder_once() == "0x0abc"         # NOT "0xabc"


# ── OP-2: unbuffered / per-line-flushed operator output ───────────────────────
def test_event_emission_flushes_to_pipe_before_exit(tmp_path):
    """OP-2: an operator redirecting stdout to a file/pipe must see each event AT ONCE — Python
    block-buffers a non-tty by default, which left the sim log 0 bytes for minutes. Capture a real
    PIPE (not a tty), emit ONE event in a child that then blocks, and assert the line arrives before
    the child exits (proves the sim_event print flushes per line, not on a buffer boundary)."""
    import os
    import select
    import subprocess
    import sys

    repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    child = (
        "import os,sys,time\n"
        "from dpos_harness.core.events import EventLog\n"
        "e=EventLog(out_dir=os.environ['SIM_OUT'])\n"
        "e._finalized=lambda:0\n"                      # no RPC in the child
        "e.sim_event('boot','hello-line')\n"
        "time.sleep(30)\n"
    )
    env = dict(os.environ, SIM_OUT=str(tmp_path))
    proc = subprocess.Popen([sys.executable, "-c", child], stdout=subprocess.PIPE,
                            cwd=repo, env=env, text=True)
    try:
        ready, _, _ = select.select([proc.stdout], [], [], 10)
        assert ready, "no stdout within 10s — the event print was block-buffered (OP-2 regressed)"
        line = proc.stdout.readline()
        assert "hello-line" in line                    # arrived WHILE the child is still alive
        # events.jsonl is flushed+fsync'd per event too (external tooling tails it live)
        assert (tmp_path / "events.jsonl").read_text().strip().endswith("}")
    finally:
        proc.kill()
        proc.wait(timeout=5)


def test_cli_enable_line_buffering_is_safe():
    """OP-2 chokepoint: enable_line_buffering never raises on a non-reconfigurable stream (pytest
    capture) — it degrades to a no-op. It moved from `cli` to `core.termio` so `shadow` could
    stop importing the entrypoint module to reach it."""
    from dpos_harness.core.termio import enable_line_buffering
    enable_line_buffering()                             # must not raise under pytest capture
