"""`cases/smoke/` — the seven ported DESTRUCTIVE drivers, against `scripts/case-*.sh` +
`asserts-fault.sh`.

Same two kinds of evidence as `test_smoke_cases.py`, with one addition that only matters here:

  * THE COMMAND TRANSCRIPT (`--dry-run`) — argv for argv against the bash. For these seven the
    ORDER carries as much of the meaning as the argv does. A restore issued before the reading it
    is supposed to bracket is a different test that still passes, and a transcript that only
    checked "the restore is present somewhere" would not see it.
  * THE WIRING (stubbed readers, LIVE branches) — the bodies run for real against scripted
    readings, so a body that reads the right things and forgets to apply the verdict fails here.
  * THE RESTORE UNDER FAILURE — that a body which fails its verdict, or which hits an unexpected
    exception, still puts back what it broke. A dry run cannot show this (it never fails a
    verdict) and a live run only walks the passing path.
"""

from __future__ import annotations

import time

import pytest

from dpos_harness.cases.smoke import (asserts_fault, crash_survivor, deferred, driver, fault,
                                      full_restart, peers, verdicts, verdicts_fault as vf,
                                      vrf_dkg_live_heal, vrf_fault)
from dpos_harness.cases.smoke.driver import SmokeCtx, SmokeFailure
from dpos_harness.core import proc, topology
from dpos_harness.core.proc import Runner
from dpos_harness.stack.profiles import StaticProfile
from dpos_harness.stack.static_stack import StaticStack

VALS = ["validator-0", "validator-1", "validator-2", "validator-3"]
UP = ["docker", "compose", "up", "--build", "-d"]
STOP = ["docker", "compose", "stop", "--timeout", "40", *VALS]
RECREATE = ["docker", "compose", "-f", "docker-compose.yml", "-f", "docker-compose.dpos.yml",
            "up", "-d", "--force-recreate", *VALS]
DOWN = ["docker", "compose", "down", "-v", "--remove-orphans"]

#: `Runner` answers a dry `docker compose ps -q` with `driver.SmokeCtx.compose_ps_q`'s canned id.
CID = "dry-container-id"

K = vf.RESULT_LAG_K


@pytest.fixture(autouse=True)
def _clean_env(monkeypatch):
    """The case layer reads the environment for the profile and for two knobs. A value leaked
    from the operator's shell would make every expectation below depend on where the suite ran."""
    for k in ("EPOCH_INTERVAL", "DPOS_ACTIVATION_BLOCK", "DPOS_EXTRA_COMPOSE",
              "DPOS_CONVERGE_EXCLUDE", "SIM_DATA_ROOT", "SMOKE_KEEP_UP", "RPC", "CHAIN_ID",
              vf.DKG_MARGIN_ENV):
        monkeypatch.delenv(k, raising=False)


def _dry(case_module, argv=("--dry-run",)):
    """Run a case in dry mode. Returns its `Runner`, whose `.log` IS the transcript."""
    made = {}
    real = driver.Runner

    def spy(**kw):
        made["r"] = real(**kw)
        return made["r"]

    driver.Runner = spy
    try:
        rc = case_module.run_case(list(argv))
    finally:
        driver.Runner = real
    return rc, made["r"]


def _cmds(runner):
    """The real commands, with the in-process `<step>` markers dropped. This is the argv oracle
    against the bash; the markers are read by `_flat` below, where their ORDER is the point."""
    return [inv.argv for inv in runner.log if inv.kind != "step"]


def _flat(runner):
    """The WHOLE transcript as one ordered sequence of strings — a command as its first three
    argv tokens, a `<step>` as its detail (`mixhash_of(validator-1, 138)`, `45s`).

    Ordering assertions read this rather than the argv list, because for these seven cases the
    interleaving of reads and mutations is half the meaning: `docker update` before the reading
    it brackets is a different test from `docker update` after it, and both produce the same set
    of commands."""
    out = []
    for inv in runner.log:
        out.append(inv.note if inv.kind == "step" else " ".join(inv.argv[:3]))
    return out


def _steps(runner, prefix=""):
    """The `<step>` details, optionally filtered by prefix — the delegated READS, whose argv
    belongs to the transport layer and which are therefore recorded as markers."""
    return [inv.note for inv in runner.log
            if inv.kind == "step" and inv.note.startswith(prefix)]


# ══ the command transcripts ════════════════════════════════════════════════

def test_deferred_transcript_is_bash_faithful():
    """`asserts-fault.sh:178-185` + `case-deferred.sh`. The only commands the case issues are the
    container-id read and the two `docker update`s — everything else is an RPC read, which goes
    out over urllib and is recorded as a marker (`core/rpc.py`'s PORT-NOTE)."""
    rc, r = _dry(deferred)
    assert rc == 0
    assert _cmds(r) == [
        UP, STOP, RECREATE,
        ["docker", "compose", "ps", "-q", "validator-1"],
        ["docker", "update", "--cpus", vf.THROTTLE_CPUS, CID],
        ["docker", "update", "--cpus", vf.RESTORE_CPUS, CID],
        DOWN,
    ]


def test_the_throttle_brackets_the_measurement_and_is_restored_before_the_verdict():
    """`asserts-fault.sh:180-186`, and the order IS the test.

    throttle -> sleep -> read -> RESTORE -> judge. Reading before the throttle would measure an
    unthrottled chain; judging before the restore would leave a validator pinned at 0.15 CPU on
    every failing run, and in the chained `smoke-fault` would hand the next four assertions a
    chain that is quietly degraded rather than one that is broken."""
    _, r = _dry(deferred)
    labels = _flat(r)
    throttle = labels.index("docker update --cpus")
    restore = len(labels) - 1 - labels[::-1].index("docker update --cpus")
    assert throttle < restore
    assert labels[throttle:restore + 1] == ["docker update --cpus",
                                            f"{vf.THROTTLE_WINDOW_S}s", "finalized_dec()",
                                            "docker update --cpus"]


def test_deferred_reads_the_three_eth_tags_and_both_consensus_methods():
    """`asserts-fault.sh:81-83,117,135,142` — `latest`/`finalized`/`safe`, `consensus_getLatest`,
    `consensus_getFinalization`, and the derived block's hash."""
    made = []
    ctx = SmokeCtx(StaticStack(profile=StaticProfile(), runner=Runner(dry=True)))
    ctx.p.step = lambda label, detail: made.append(detail)
    for tag in ("latest", "finalized", "safe"):
        ctx.json_rpc("eth_getBlockByNumber", [tag, False])
    ctx.json_rpc("consensus_getLatest", [])
    ctx.json_rpc("consensus_getFinalization", [{"height": 140}])
    assert made == ['json_rpc(eth_getBlockByNumber, ["latest", false])',
                    'json_rpc(eth_getBlockByNumber, ["finalized", false])',
                    'json_rpc(eth_getBlockByNumber, ["safe", false])',
                    "json_rpc(consensus_getLatest, [])",
                    'json_rpc(consensus_getFinalization, [{"height": 140}])']


def test_the_k_lag_loop_is_a_sample_count_not_a_deadline():
    """`asserts-fault.sh:80` — six readings. A `poll`-shaped rewrite would turn the count into a
    wall deadline and make "the lag never once sat at exactly K" depend on RPC latency."""
    made = []
    ctx = SmokeCtx(StaticStack(profile=StaticProfile(), runner=Runner(dry=True)))
    ctx.p.step = lambda label, detail: made.append((label, detail))
    ctx.repeat(vf.LAG_SAMPLES, lambda i: i, sleep_s=vf.LAG_SAMPLE_SLEEP_S, label="k-lag")
    assert made == [("sample", "k-lag (x6, 2s apart)")]


def test_peers_transcript_restarts_one_validator_and_nothing_else():
    """`asserts-fault.sh:262` + `case-peers.sh`. A GRACEFUL `docker compose restart`, which is
    what makes this the least invasive of the five and lets it run second in the chain."""
    rc, r = _dry(peers)
    assert rc == 0
    assert _cmds(r) == [UP, STOP, RECREATE,
                            ["docker", "compose", "restart", "validator-1"], DOWN]


def test_peers_reads_the_registry_at_the_bare_root():
    """`asserts-fault.sh:215` — `METRICS="http://localhost:19100/"`, NOT the `/metrics` path
    `asserts.sh:344` uses. Two different URLs in two different bash files, kept distinct."""
    assert driver.PEERS_METRICS_URL == "http://localhost:19100/"
    assert driver.CONSENSUS_METRICS_URL == "http://localhost:19100/metrics"
    made = {}
    ctx = SmokeCtx(StaticStack(profile=StaticProfile(), runner=Runner(dry=True)))
    ctx.p.step = lambda label, detail: made.setdefault("d", detail)
    ctx.peers_metrics()
    assert made["d"] == "peers_metrics(http://localhost:19100/)"


def test_vrf_fault_stops_the_victim_and_starts_it_again():
    """`asserts-fault.sh:304,299` + `case-vrf-fault.sh`. The stop is GRACEFUL (`docker compose
    stop`), unlike crash-survivor's — the victim is expected to have flushed and to reload its
    beacon share on restart, which is half of what B3/B4 assert."""
    rc, r = _dry(vrf_fault)
    assert rc == 0
    assert _cmds(r) == [UP, STOP, RECREATE,
                            ["docker", "compose", "stop", "validator-3"],
                            ["docker", "compose", "start", "validator-3"], DOWN]


def test_vrf_fault_compares_the_survivors_and_leaves_the_victim_out():
    """`asserts-fault.sh:286,283` — `NODES` is reassigned to the SURVIVORS before the window, and
    `assert_beacon_window`/`wait_nodes_have` read it through bash's dynamic scope. Comparing the
    stopped node would report every block as missing on a node that is correctly down."""
    _, r = _dry(vrf_fault)
    window = _steps(r, "mixhash_of(")
    assert window, "the beacon window read nothing"
    assert not [d for d in window if "validator-3" in d]
    assert {d.split("(")[1].split(",")[0] for d in window} == {
        "validator-0", "validator-1", "validator-2", "full-node"}


def test_vrf_fault_reads_the_victims_gap_blocks_in_container():
    """`asserts-fault.sh:340` uses `mixhash_in`, not `mixhash_of`. The point is the VICTIM's own
    view of the gap; `mixhash_of` would route a validator-0 victim to the host RPC and end up
    comparing the reference chain with itself."""
    _, r = _dry(vrf_fault)
    gap = _steps(r, "mixhash_in(")
    assert gap and all(d.startswith("mixhash_in(validator-3,") for d in gap)


def test_crash_survivor_uses_the_raw_container_channel_not_compose():
    """`asserts-fault.sh:468-483` + `case-crash-survivor.sh`.

    `docker kill` / `docker start` on the resolved container id, deliberately NOT `docker compose
    kill|start`: compose re-runs the service's dependencies, and re-running `genesis-init` races
    the ungraceful path and made the restart flaky. A `docker compose stop` here would also flush
    the node, which deletes the case."""
    rc, r = _dry(crash_survivor)
    assert rc == 0
    assert _cmds(r) == [UP, STOP, RECREATE,
                            ["docker", "compose", "ps", "-q", "validator-3"],
                            ["docker", "kill", CID], ["docker", "start", CID], DOWN]


def test_crash_survivor_resolves_the_id_before_it_kills():
    """The id read must precede the kill: `docker compose ps -q` is RUNNING-only, so resolving it
    after the SIGKILL would answer nothing and the restart would have no target."""
    flat = _flat(_dry(crash_survivor)[1])
    assert flat.index("docker compose ps") < flat.index(f"docker kill {CID}")


def test_full_restart_stops_the_whole_committee_then_starts_it():
    """`asserts-fault.sh:517-522` + `case-full-restart.sh`. `--timeout 40` is reth's
    `on_graceful_shutdown` budget; a SIGKILL at the ceiling exits 137 and the flush check sees
    it."""
    rc, r = _dry(full_restart)
    assert rc == 0
    assert _cmds(r) == [
        UP, STOP, RECREATE,
        ["docker", "compose", "stop", "--timeout", str(vf.FULL_RESTART_STOP_TIMEOUT_S), *VALS],
        ["docker", "compose", "start", *VALS], DOWN]


def test_full_restart_checks_every_validator_flushed_between_the_stop_and_the_start():
    """`asserts-fault.sh:518-522`. The barrier must run while the containers are STOPPED — that
    is the only moment `docker inspect` can report an exit code — and it must cover all four,
    because one un-flushed validator is exactly the case this is written to catch."""
    _, r = _dry(full_restart)
    flat = _flat(r)
    stop = flat.index("docker compose stop", 1 + flat.index("docker compose stop"))
    start = flat.index("docker compose start")
    assert stop < start
    assert flat[stop + 1:start] == [f"shutdown_flushed({v})" for v in VALS]


def test_live_heal_stops_the_victim_before_anything_else_reads_the_chain():
    """THE TIMING IS THE ASSERTION: the victim must be down before its epoch-2 DKG window opens,
    so the only read that may precede the stop is the window guard itself. The victim's on-chain
    address is resolved AFTER the stop for exactly this reason — it costs a `docker compose exec`
    and nothing may widen the gap between reading the height and taking the node down."""
    rc, r = _dry(vrf_dkg_live_heal)
    assert rc == 0
    flat = _flat(r)
    stop = flat.index("docker compose stop", 1 + flat.index("docker compose stop"))
    before = [x for x in flat[stop - 1:stop]]
    assert before == ["finalized_dec()"], f"reads before the victim was stopped: {flat[:stop]}"
    assert _cmds(r) == [UP, STOP, RECREATE,
                        ["docker", "compose", "stop", "validator-3"],
                        ["docker", "compose", "exec", "-T", "validator-0", "cat",
                         "/runtime/addresses.json"],
                        ["docker", "compose", "start", "validator-3"], DOWN]


def test_live_heal_exports_both_spellings_of_the_tuned_interval():
    """`EPOCH_BLOCK_INTERVAL` is what the compose file interpolates into genesis-init;
    `EPOCH_INTERVAL` is what the host profile reads for its epoch arithmetic. Exporting one and
    not the other gives the case a 64-block stack and 32-block math, silently — the trap
    `smoke-vrf-dkg-restart-midwindow` records and the only reason the value appears twice."""
    _, r = _dry(vrf_dkg_live_heal)
    flat = _flat(r)
    n = vrf_dkg_live_heal.EPOCH_INTERVAL
    assert f"EPOCH_BLOCK_INTERVAL={n}" in flat and f"EPOCH_INTERVAL={n}" in flat
    assert f"DPOS_ACTIVATION_BLOCK={vrf_dkg_live_heal.ACTIVATION_BLOCK}" in flat


def test_live_heal_reads_the_recovery_log_after_the_victim_caught_up():
    """The log is only complete for epoch 2 once the restarted node has caught up to the boundary
    AND the off-tick reconstruction has had time to run. Grepping earlier would find no share line
    for a node that is merely still replaying, and would report the absence as a failed pull."""
    _, r = _dry(vrf_dkg_live_heal)
    flat = _flat(r)
    start = flat.index("docker compose start")
    log = flat.index("logs_all(validator-3)")
    catchup = flat.index(f"has_it (<= {vf.DKG_CATCHUP_S}s)")
    recovered = flat.index(f"recovered (<= {vf.DKG_HEAL_S}s)")
    assert start < catchup < recovered <= log


def test_live_heal_reads_production_only_after_epoch_2_has_ended():
    """`producedAt` is a per-epoch counter and the load-bearing leg: it climbs for as long as the
    epoch runs, so a sample taken right after the member is seated reports the slots it has won SO
    FAR. A live run read 0 at that instant and 10 of 64 once the epoch finished, which is how this
    case first went red. The wait must target the END of epoch 2 — `epoch_start(3)` plus the
    K-block execution lag, because the counter for height h is written when h EXECUTES — and must
    precede the read."""
    _, r = _dry(vrf_dkg_live_heal)
    flat = _flat(r)
    recovered = flat.index(f"recovered (<= {vf.DKG_HEAL_S}s)")
    prod = flat.index("producedAt(epoch=2) (<= 5, 4s apart)")
    epoch3 = vrf_dkg_live_heal.ACTIVATION_BLOCK + 3 * vrf_dkg_live_heal.EPOCH_INTERVAL
    wait = flat.index(f"wait_finalized_ge({epoch3 + vf.RESULT_LAG_K}, {vf.DKG_EPOCH_END_S}s)")
    assert recovered < wait < prod


def test_fault_runs_all_five_on_exactly_one_bring_up():
    """`case-fault.sh` — each body restores the stack, so they share ONE migration instead of
    paying for five. A second bring-up here means the sharing is gone."""
    rc, r = _dry(fault)
    assert rc == 0
    cmds = _cmds(r)
    assert cmds.count(UP) == 1 and cmds.count(RECREATE) == 1 and cmds.count(DOWN) == 1
    assert cmds[0] == UP and cmds[-1] == DOWN


def test_fault_orders_the_five_least_to_most_invasive():
    """`case-fault.sh:12-18`. deferred FIRST because its K-lag invariant wants a pristine steady
    state; full-restart LAST because nothing can follow a stop of the entire validator set."""
    assert fault.ASSERTIONS == [asserts_fault.assert_deferred, asserts_fault.assert_peers,
                                asserts_fault.assert_vrf_fault,
                                asserts_fault.assert_crash_survivor,
                                asserts_fault.assert_full_restart]


def test_fault_excludes_liveness_and_dkg_liveness():
    """`case-fault.sh:20-22` — `smoke-liveness` can JAIL a validator, which permanently shrinks
    the committee and is unrecoverable. `smoke-vrf-dkg-live-heal` needs a DKG window that opens
    once near bring-up, which five chained cases would have consumed."""
    names = [f.__name__ for f in fault.ASSERTIONS]
    assert "assert_vrf_dkg_live_heal" not in names
    assert not any("liveness" in n for n in names)


def test_every_destructive_body_restores_the_stack_in_the_transcript():
    """The chunk's whole soundness argument, checked mechanically: for each of the five chained
    bodies, the transcript pairs each disruption with its restore. A body that returned with a
    node still down would leave the NEXT body measuring a degraded chain — and reporting green."""
    _, r = _dry(fault)
    cmds = _cmds(r)
    pairs = [(["docker", "update", "--cpus", vf.THROTTLE_CPUS, CID],
              ["docker", "update", "--cpus", vf.RESTORE_CPUS, CID]),
             (["docker", "compose", "stop", "validator-3"],
              ["docker", "compose", "start", "validator-3"]),
             (["docker", "kill", CID], ["docker", "start", CID]),
             (["docker", "compose", "stop", "--timeout",
               str(vf.FULL_RESTART_STOP_TIMEOUT_S), *VALS],
              ["docker", "compose", "start", *VALS])]
    for broke, fixed in pairs:
        assert broke in cmds and fixed in cmds, f"{broke} has no restore in the transcript"
        assert cmds.index(broke) < cmds.index(fixed)


def test_a_dry_run_never_spawns_a_process(monkeypatch):
    """The point of the transcript is that it can be produced anywhere, against no chain — which
    matters more for the destructive seven than for the read-only five."""
    monkeypatch.setattr(proc.subprocess, "run",
                        lambda *a, **k: pytest.fail("the dry run executed a command"))
    for mod in (deferred, peers, vrf_fault, crash_survivor, full_restart, vrf_dkg_live_heal,
                fault):
        assert mod.run_case(["--dry-run"]) == 0


def test_an_unknown_argument_is_refused():
    assert deferred.run_case(["--turbo"]) == 2


def test_every_fault_case_is_registered_in_the_cli():
    """`case <name>` and `case list` read one registry, and the Makefile targets resolve through
    it — a case added to one is never missing from the other."""
    from dpos_harness import cli
    for name, mod in [("smoke-fault", "smoke.fault"), ("smoke-deferred", "smoke.deferred"),
                      ("smoke-peers", "smoke.peers"),
                      ("smoke-crash-survivor", "smoke.crash_survivor"),
                      ("smoke-full-restart", "smoke.full_restart"),
                      ("smoke-vrf-fault", "smoke.vrf_fault"),
                      ("smoke-vrf-dkg-live-heal", "smoke.vrf_dkg_live_heal")]:
        assert cli.CASES[name] == mod


def test_no_destructive_wrapper_opts_into_keep_up(monkeypatch):
    """`SMOKE_KEEP_UP` is honoured by exactly one bash wrapper (`case-vrf.sh:14`), and none of
    the seven here is it. Leaving a BROKEN stack up is worse than leaving a healthy one up: the
    next case's bring-up would inherit a throttled or half-stopped devnet."""
    seen = {}
    monkeypatch.setattr(driver, "run",
                        lambda case, a, argv=None, converge_exclude=None, honours_keep_up=False,
                        overlays=None, exports=None:
                        seen.__setitem__(case, honours_keep_up) or 0)
    for mod in (deferred, peers, vrf_fault, crash_survivor, full_restart, vrf_dkg_live_heal,
                fault):
        mod.run_case([])
    assert set(seen.values()) == {False}
    assert len(seen) == 7


# ══ the wiring: assertion bodies on their LIVE branches ════════════════════

def _live_ctx(monkeypatch, **stubs):
    """A `SmokeCtx` on its LIVE branches with every reader stubbed.

    Identical construction to `test_smoke_cases.py::_live_ctx`: the Runner stays dry (writes are
    recorded, nothing spawns) while `dry` reports False, so `check` raises and `poll` really
    loops. The CLOCK is fast-forwarded rather than the constants being shrunk — three cases in
    this tree pass BY timeout, and a test that shortened the real budgets would leave somebody
    free to shorten them for real."""
    import time as _time
    clock = [0.0]

    def _tick():
        clock[0] += 3600.0
        return clock[0]

    monkeypatch.setattr(_time, "monotonic", _tick)
    monkeypatch.setattr(_time, "sleep", lambda _s: None)

    runner = Runner(dry=True)
    monkeypatch.setattr(runner, "_exec",
                        lambda *a, **k: pytest.fail("the recorder executed a command"))
    stack = StaticStack(profile=StaticProfile(), runner=runner)
    stack.prev_fin = "0x50"
    ctx = SmokeCtx(stack)
    monkeypatch.setattr(SmokeCtx, "dry", property(lambda self: False))
    for name, value in stubs.items():
        monkeypatch.setattr(ctx, name, value)
    return ctx, runner


def _seq(*values):
    """A reader that answers a scripted sequence, repeating its last value."""
    box = list(values)

    def read(*_a, **_kw):
        return box.pop(0) if len(box) > 1 else box[0]
    return read


# ── deferred ───────────────────────────────────────────────────────────────

GOOD_WIRE = "0x" + "ab" * (vf.WIRE_RESULT_OFFSET // 2) + "cd" * 32 + "ef" * 32
GOOD_HASH = "0x" + "cd" * 32


def _json_rpc(latest=140, final=140 - K, safe=140, cons=(140, 140 - K),
              wire=GOOD_WIRE, block_hash=GOOD_HASH):
    tags = {"latest": latest, "finalized": final, "safe": safe}

    def call(method, params=None, dry_value=None):
        params = list(params or [])
        if method == "eth_getBlockByNumber":
            tag = params[0]
            if tag in tags:
                return {"result": {"number": hex(tags[tag])}}
            return {"result": {"hash": block_hash}}
        if method == "consensus_getLatest":
            return {"result": {"latestFinalized": {"height": cons[0]},
                               "latestResultFinalized": cons[1]}}
        if method == "consensus_getFinalization":
            return {"result": {"block": wire}}
        return {}
    return call


def _deferred_world(monkeypatch, **over):
    world = dict(
        baseline_height=lambda **kw: 100,
        wait_finalized_ge=lambda target, timeout: True,
        json_rpc=_json_rpc(),
        compose_ps_q=lambda service, **kw: CID,
        finalized_dec=_seq(100, 100 + vf.THROTTLE_MIN_GROWTH),
        check_node=lambda service, **kw: f"{hex(100 + vf.THROTTLE_MIN_GROWTH)}|0xh",
        check_external=lambda port, **kw: f"{hex(100 + vf.THROTTLE_MIN_GROWTH)}|0xh",
        sleep=lambda s: None,
        dump_logs=lambda *a, **kw: None,
    )
    world.update(over)
    return _live_ctx(monkeypatch, **world)


def test_assert_deferred_passes_on_a_healthy_two_tier_chain(monkeypatch):
    ctx, _ = _deferred_world(monkeypatch)
    asserts_fault.assert_deferred(ctx)


def test_assert_deferred_fails_on_a_finality_overclaim(monkeypatch):
    """The safety half. Every convergence-based case in the tree passes over a UNIFORM overclaim,
    because they only require the nodes to agree with each other."""
    ctx, _ = _deferred_world(monkeypatch, json_rpc=_json_rpc(latest=140, final=139))
    with pytest.raises(SmokeFailure, match="overclaims"):
        asserts_fault.assert_deferred(ctx)


def test_assert_deferred_fails_when_the_consensus_tiers_disagree(monkeypatch):
    ctx, _ = _deferred_world(monkeypatch, json_rpc=_json_rpc(cons=(140, 120)))
    with pytest.raises(SmokeFailure, match="tiers disagree"):
        asserts_fault.assert_deferred(ctx)


def test_assert_deferred_fails_when_the_artifact_commits_a_different_result(monkeypatch):
    ctx, _ = _deferred_world(monkeypatch,
                             json_rpc=_json_rpc(block_hash="0x" + "99" * 32))
    with pytest.raises(SmokeFailure, match="result commitment mismatch"):
        asserts_fault.assert_deferred(ctx)


def test_assert_deferred_fails_loud_when_a_tag_read_returns_nothing(monkeypatch):
    """Sentinel discipline. Bash aborts under `set -e` on `printf '%d' null`; Python would turn
    an unreachable tag into a 0 and compute a lag that is arithmetic fiction — and a fiction that
    lands inside the accepted band about as often as outside it."""
    ctx, _ = _deferred_world(monkeypatch, json_rpc=lambda m, p=None, dry_value=None: {})
    with pytest.raises(SmokeFailure, match="refusing to read an unreachable tag"):
        asserts_fault.assert_deferred(ctx)


def test_assert_deferred_fails_when_the_chain_stalls_under_the_throttle(monkeypatch):
    ctx, _ = _deferred_world(monkeypatch, finalized_dec=_seq(100, 105))
    with pytest.raises(SmokeFailure, match="chain stalled under one slowed EL"):
        asserts_fault.assert_deferred(ctx)


def test_the_throttle_is_restored_even_when_the_liveness_verdict_fails(monkeypatch):
    """THE failure direction a live run can never show. bash restores at :152 and judges at :153,
    so a stalled chain still leaves the victim unthrottled; a port that judged first would pin a
    validator at 0.15 CPU on exactly the runs where somebody is about to debug the stack."""
    ctx, runner = _deferred_world(monkeypatch, finalized_dec=_seq(100, 105))
    with pytest.raises(SmokeFailure):
        asserts_fault.assert_deferred(ctx)
    assert ["docker", "update", "--cpus", vf.RESTORE_CPUS, CID] in runner.argvs()


def test_the_throttle_is_restored_when_the_measurement_itself_raises(monkeypatch):
    """The `finally`. `docker update --cpus` is container CONFIG and outlives a Python exception,
    so an unexpected error between the throttle and the restore would leave the stack degraded
    rather than broken — which is the harder failure to notice."""
    def boom(**kw):
        raise RuntimeError("RPC exploded mid-window")

    ctx, runner = _deferred_world(monkeypatch, finalized_dec=_seq(100, 0))
    monkeypatch.setattr(ctx, "sleep", lambda s: boom())
    with pytest.raises(RuntimeError, match="exploded"):
        asserts_fault.assert_deferred(ctx)
    assert ["docker", "update", "--cpus", vf.RESTORE_CPUS, CID] in runner.argvs()


def test_assert_deferred_fails_when_the_victim_never_rejoins(monkeypatch):
    """A validator that comes back serving RPC from BEFORE the throttle is the stuck catch-up
    this half exists to catch — it answers everything and has recovered nothing."""
    ctx, _ = _deferred_world(monkeypatch, check_node=lambda service, **kw: "0x1|0xh")
    with pytest.raises(SmokeFailure, match="did not rejoin after unthrottle"):
        asserts_fault.assert_deferred(ctx)


def test_assert_deferred_fails_when_the_victim_container_is_gone(monkeypatch):
    ctx, _ = _deferred_world(monkeypatch, compose_ps_q=lambda service, **kw: "")
    with pytest.raises(SmokeFailure, match="no container for validator-1"):
        asserts_fault.assert_deferred(ctx)


# ── peers ──────────────────────────────────────────────────────────────────

FOUR_ADDRS = ", ".join("0x" + str(i) * 40 for i in range(1, 5))


def _directory(peers) -> str:
    """The commonware `p2p_network_tracker_directory_connected` family — peer pubkey → the
    epoch-millis instant that connection became active."""
    return "\n".join(f'p2p_network_tracker_directory_connected{{peer="{pk}"}} {ts}'
                     for pk, ts in peers.items())


#: validator-0's three committee peers, all connected long before the case started.
BASE_DIRECTORY = {f"{i:02x}": 1000 + i for i in range(3)}
#: after the restart: one socket came back, stamped ABOVE every baseline connect instant.
REJOINED_DIRECTORY = dict(BASE_DIRECTORY, **{"01": 2000})


def _restarted(runner) -> bool:
    return ["docker", "compose", "restart", topology.validator(vf.PEERS_VICTIM_IDX)] \
        in runner.argvs()


def _peers_world(monkeypatch, **over):
    """The peers world MODELS THE RESTART: the metrics reader answers the baseline directory
    until the victim's `docker compose restart` has actually been issued, and a rejoined one
    afterwards. A constant reading would make the reconnect leg unfalsifiable in the test the
    same way the cumulative broadcast family made it unfalsifiable on a live chain."""
    world = dict(
        staking_call=lambda sig, *a, **kw: ("2" if "currentEpoch" in sig
                                            else f"[{FOUR_ADDRS}]"),
        peer_count=lambda service, **kw: 2,
        baseline_height=lambda **kw: 100,
        finalized_dec=lambda **kw: 105,
        dump_logs=lambda *a, **kw: None,
    )
    world.update(over)
    ctx, runner = _live_ctx(monkeypatch, **world)
    if "peers_metrics" not in over:
        monkeypatch.setattr(ctx, "peers_metrics", lambda **kw: _directory(
            REJOINED_DIRECTORY if _restarted(runner) else BASE_DIRECTORY))
    return ctx, runner


def test_assert_peers_passes_when_both_planes_are_up(monkeypatch):
    ctx, _ = _peers_world(monkeypatch)
    asserts_fault.assert_peers(ctx)


def test_assert_peers_derives_the_expectation_from_the_on_chain_committee(monkeypatch):
    """`asserts-fault.sh:236-237` — `committee_size - 1`, read from the chain rather than
    hardcoded, so a committee of a different size does not silently pass or silently fail."""
    ctx, _ = _peers_world(monkeypatch,
                          staking_call=lambda sig, *a, **kw: ("2" if "currentEpoch" in sig
                                                              else f"[{FOUR_ADDRS}, 0x"
                                                                   + "9" * 40 + "]"))
    with pytest.raises(SmokeFailure, match="connected=3 != 4"):
        asserts_fault.assert_peers(ctx)


def test_assert_peers_fails_when_a_committee_peer_is_missing(monkeypatch):
    ctx, _ = _peers_world(monkeypatch, peers_metrics=_seq(
        _directory({f"{i:02x}": 1000 + i for i in range(2)})))
    with pytest.raises(SmokeFailure, match="connected=2 != 3"):
        asserts_fault.assert_peers(ctx)


def test_assert_peers_fails_when_the_victim_never_rejoins_the_consensus_plane(monkeypatch):
    """THE F1 REGRESSION GUARD, at case level. The restarted validator's reth devp2p peering
    comes back and the chain keeps finalizing without it (f=1), but its commonware socket never
    returns: the directory stays exactly as the baseline scrape saw it.

    Under the old reading — a distinct-series count over the CUMULATIVE
    `outer_engine_buffered_peer_total` family, whose series are never pruned — this case passed,
    because the victim's series was still there from before the restart. Consensus-plane
    discovery is the property `smoke-peers` exists to pin, so that was a false GREEN over the
    whole subject of the case."""
    ctx, _ = _peers_world(monkeypatch,
                          peers_metrics=lambda **kw: _directory(BASE_DIRECTORY))
    with pytest.raises(SmokeFailure, match="fresh consensus connection=False"):
        asserts_fault.assert_peers(ctx)


def test_assert_peers_fails_on_a_stale_directory_entry_for_the_dead_socket(monkeypatch):
    """The count leg alone cannot see this: validator-0 still carries an entry for a peer whose
    process is gone, so `connected == committee_size-1` holds over a socket that died. Only the
    connect TIMESTAMP separates "the peer is back" from "the record is stale"."""
    stale = dict(BASE_DIRECTORY, **{"01": BASE_DIRECTORY["01"]})
    ctx, _ = _peers_world(monkeypatch, peers_metrics=lambda **kw: _directory(stale))
    with pytest.raises(SmokeFailure, match="fresh consensus connection=False"):
        asserts_fault.assert_peers(ctx)


def test_assert_peers_fails_when_the_devp2p_plane_is_not_wired(monkeypatch):
    """The `dpos_rejoin_el_sync_devp2p` regression guard: commonware discovery is perfectly
    healthy and reth has no peer at all."""
    ctx, _ = _peers_world(monkeypatch, peer_count=lambda service, **kw: 0)
    with pytest.raises(SmokeFailure, match="peering not wired"):
        asserts_fault.assert_peers(ctx)


def test_assert_peers_fails_when_the_chain_does_not_advance_past_the_restart(monkeypatch):
    """Both peer planes reconnect and the node contributes nothing. Without the chain-advance leg
    this would be a socket test wearing a rejoin test's name."""
    ctx, _ = _peers_world(monkeypatch, finalized_dec=lambda **kw: 100)
    with pytest.raises(SmokeFailure, match="after validator-1 restart"):
        asserts_fault.assert_peers(ctx)


def test_assert_peers_restarts_the_victim_before_measuring_the_reconnect(monkeypatch):
    """The baseline is captured BEFORE the restart, so `finalized > PRE` measures blocks produced
    after it. Capturing afterwards would compare the chain against itself."""
    ctx, runner = _peers_world(monkeypatch)
    asserts_fault.assert_peers(ctx)
    assert ["docker", "compose", "restart", "validator-1"] in runner.argvs()


# ── vrf-fault ──────────────────────────────────────────────────────────────

def _mix(n) -> str:
    return "0x%064x" % int(n)


#: The epoch the fault window runs in on this world's geometry — `epoch_of(a_hi)`, which is what
#: `assert_vrf_fault` scopes its share witness to.
VRF_FAULT_SHARE_EPOCH = 2


def _reload_log(promote=VRF_FAULT_SHARE_EPOCH, gate=None, boot=True, pre_stop_promote=False):
    """The restarted victim's log, assembled leg by leg.

    `pre_stop_promote` writes a promotion BEFORE the boot marker — the shape the process that was
    STOPPED left behind. It must not satisfy the witness: the claim is about the process that came
    back, and its own promotion is the only thing that says the share survived the restart."""
    out = []
    if pre_stop_promote:
        out.append(f"INFO {vf.PROMOTE_LINE} " + vf.PIN_EPOCH_FMT.format(VRF_FAULT_SHARE_EPOCH))
    if boot:
        out.append(f"INFO {vf.ACTOR_STARTED_LINE} epocher=(test)")
    if gate is not None:
        out.append(f"INFO {vf.SHARE_GATE_LINE} reason=NoUsableShare "
                   + vf.PIN_EPOCH_FMT.format(gate))
    if promote is not None:
        out.append(f"INFO {vf.PROMOTE_LINE} " + vf.PIN_EPOCH_FMT.format(promote))
    return "\n".join(out)


def _vrf_fault_world(monkeypatch, **over):
    world = dict(
        wait_finalized_ge=lambda target, timeout: True,
        finalized_dec=lambda **kw: 140,
        mixhash_of=lambda svc, block, **kw: _mix(block),
        mixhash_in=lambda svc, block, **kw: _mix(block),
        mixhash_at=lambda block, **kw: _mix(block),
        logs_all=lambda service, **kw: _reload_log(),
        dump_logs=lambda *a, **kw: None,
    )
    world.update(over)
    return _live_ctx(monkeypatch, **world)


def test_assert_vrf_fault_passes_when_the_beacon_survives_and_the_victim_catches_up(monkeypatch):
    ctx, _ = _vrf_fault_world(monkeypatch)
    asserts_fault.assert_vrf_fault(ctx)


def test_assert_vrf_fault_fails_when_the_chain_stalls_with_one_validator_down(monkeypatch):
    """A1 — n−f=3 must still finalize. A stall here means the seed quorum did not hold."""
    ctx, _ = _vrf_fault_world(monkeypatch, wait_finalized_ge=_seq(True, False))
    with pytest.raises(SmokeFailure, match="A1 — chain stalled"):
        asserts_fault.assert_vrf_fault(ctx)


def test_assert_vrf_fault_fails_when_the_survivors_beacon_diverges(monkeypatch):
    """The safety property while the fault is open: one survivor deriving a different threshold
    seed. The import follower is in the compared set, so this covers it too."""
    ctx, _ = _vrf_fault_world(monkeypatch, mixhash_of=lambda svc, block, **kw: (
        "0x" + "ff" * 32 if svc == "full-node" and int(block) == 145 else _mix(block)))
    with pytest.raises(SmokeFailure, match="disagree on prev_randao at block 145"):
        asserts_fault.assert_vrf_fault(ctx)


def test_assert_vrf_fault_fails_when_the_restarted_victim_never_catches_up(monkeypatch):
    ctx, _ = _vrf_fault_world(monkeypatch, mixhash_in=lambda svc, block, **kw: "null")
    with pytest.raises(SmokeFailure, match="B4 — validator-3 did not catch up"):
        asserts_fault.assert_vrf_fault(ctx)


def test_assert_vrf_fault_fails_when_the_victim_fell_back_to_the_digest(monkeypatch):
    """B3/B4 — the whole point. The victim came back, caught up, serves every gap block, and
    derived a DIFFERENT prev_randao: it used `order.digest()` instead of recovering the cert
    seed, or it forked. A "did it catch up" check alone reports this as success."""
    ctx, _ = _vrf_fault_world(monkeypatch, mixhash_in=lambda svc, block, **kw: (
        "0x" + "ff" * 32 if int(block) == 145 else _mix(block)))
    with pytest.raises(SmokeFailure, match="fell to fallback"):
        asserts_fault.assert_vrf_fault(ctx)


def test_assert_vrf_fault_fails_when_the_victim_came_back_shareless(monkeypatch):
    """THE F2 REGRESSION GUARD. Every other post-restart leg is IDENTICAL to the passing world:
    the victim answers RPC, catches up, and serves every gap block with the same prev_randao as
    the four survivors. It just came back without its DKG share and said so.

    That combination is not a contrived fixture — `prev_randao` rides in the CERTIFICATE
    (`crates/node/src/derive.rs`), so a shareless node re-derives the whole gap byte-identically.
    Before the share witness this case's OK line claimed "reloaded its share" over exactly this
    run and passed."""
    ctx, _ = _vrf_fault_world(monkeypatch,
                              logs_all=lambda service, **kw: _reload_log(
                                  promote=None, gate=VRF_FAULT_SHARE_EPOCH))
    with pytest.raises(SmokeFailure, match="WITHOUT a usable epoch-2 DKG share"):
        asserts_fault.assert_vrf_fault(ctx)


def test_assert_vrf_fault_fails_when_the_victim_is_never_re_seated_as_a_signer(monkeypatch):
    """The other arm: no share-gate line either, because the reconciler never got as far as the
    `Role::Signer` decision. An absence-only witness would be green here — a node that logged
    nothing at all satisfies "no share-gate lines"."""
    ctx, _ = _vrf_fault_world(monkeypatch,
                              logs_all=lambda service, **kw: _reload_log(promote=None))
    with pytest.raises(SmokeFailure, match="never logged .* for an epoch >= 2"):
        asserts_fault.assert_vrf_fault(ctx)


def test_assert_vrf_fault_ignores_the_promotion_the_stopped_process_wrote(monkeypatch):
    """The victim's log still carries everything the PRE-STOP process wrote, and that process was
    a perfectly healthy signer. Reading it would witness the share the restart was supposed to
    test the reload of."""
    ctx, _ = _vrf_fault_world(monkeypatch,
                              logs_all=lambda service, **kw: _reload_log(
                                  promote=None, pre_stop_promote=True))
    with pytest.raises(SmokeFailure, match="never logged .* for an epoch >= 2"):
        asserts_fault.assert_vrf_fault(ctx)


def test_assert_vrf_fault_accepts_a_re_seat_at_a_later_epoch(monkeypatch):
    """The catch-up can run past the epoch the fault window was in, and the reconcile that
    re-seats the victim then names the epoch it landed in. A witness pinned to epoch 2 exactly
    would be a false RED on correct behaviour."""
    ctx, _ = _vrf_fault_world(monkeypatch,
                              logs_all=lambda service, **kw: _reload_log(promote=4))
    asserts_fault.assert_vrf_fault(ctx)


# ── vrf-dkg-live-heal ──────────────────────────────────────────────────────

def _heal_log(fresh=True, heal=None, road=vf.SHARE_LINE, promote=True,
              pre_stop_ceremony=False):
    """The victim's log, assembled leg by leg so each test can remove exactly one.

    `pre_stop_ceremony` writes a `ceremony started` BEFORE the boot marker — the shape a victim
    stopped mid-deal-phase leaves behind, and the one the setup gate has to reject.

    `heal` is the `(want, dealers)` of the heal-DETECT line, the setup gate's SECOND witness: the
    demote-heal road reaches the share with no ceremony at all, so `fresh=False, heal=(n, n)` is a
    correct run and not a failure — see `vf.evaluate_victim_held_nothing`."""
    out = []
    if pre_stop_ceremony:
        out.append(f"INFO {vf.CEREMONY_STARTED_LINE} epoch=2")
    out.append(f"INFO {vf.ACTOR_STARTED_LINE} epocher=(test)")
    if fresh:
        out.append(f"INFO {vf.CEREMONY_STARTED_LINE} epoch=2")
    if heal is not None:
        out.append(f"INFO live DKG: demoted committee member detected \u2014 "
                   f"{vf.HEAL_START_LINE} epoch=2 want={heal[0]} dealers={heal[1]}")
    if road:
        out.append(f"INFO {road} epoch=2 height=257")
    if promote:
        out.append(f"INFO {vf.PROMOTE_LINE} " + vf.PIN_EPOCH_FMT.format(2))
    return "\n".join(out) or "INFO nothing of interest"


def _dkg_world(monkeypatch, **over):
    # THE TUNED GEOMETRY, set here and not left on the profile default. `_live_ctx` builds a
    # `StaticProfile` off the environment, while the case's exports are applied by `driver.run`,
    # which these wiring tests bypass — so without this the body would be walked on 32/64 epoch
    # arithmetic while the real case runs on 64/128, and every height below would be describing a
    # chain the case never sees.
    monkeypatch.setenv("EPOCH_INTERVAL", str(vrf_dkg_live_heal.EPOCH_INTERVAL))
    monkeypatch.setenv("DPOS_ACTIVATION_BLOCK", str(vrf_dkg_live_heal.ACTIVATION_BLOCK))
    world = dict(
        finalized_dec=lambda **kw: 100,
        wait_finalized_ge=lambda target, timeout: True,
        runtime_addresses=lambda **kw: ["0x" + f"{0xa0 + i:02x}" * 20 for i in range(4)],
        mixhash_of=lambda svc, block, **kw: _mix(block),
        mixhash_in=lambda svc, block, **kw: _mix(block),
        mixhash_at=lambda block, **kw: _mix(block),
        logs_all=lambda svc, **kw: _heal_log(),
        node_metric=lambda svc, name, **kw: "3",
        # LEG 5's WITNESS IS A COUNTER NOW (FLU-1202 deleted the pin transition it used to grep).
        # Two families off ONE scrape: the seeds this node verified against PK_2, and the ones it
        # could not because it held no key. The keyless count is non-zero in the healthy world on
        # purpose — the victim WAS seed-blind for part of epoch 2, that is the whole scenario, and
        # a fixture that zeroed it would be describing a node that never needed to heal.
        node_metrics_text=lambda svc, **kw: (
            f"{vf.SEED_VERIFY_OK_SAMPLE} 12\n{vf.SEED_VERIFY_NO_KEY_SAMPLE} 4\n"),
        production=lambda epoch, addr, **kw: (10, 64),
        sleep=lambda s: None,
        dump_logs=lambda *a, **kw: None,
    )
    world.update(over)
    return _live_ctx(monkeypatch, **world)


#: The case's `finalized_dec` readings, IN ORDER: the deal-window guard, the shorthanded pacing
#: pair, the height at which the victim was seated, and the two that bracket the post-rejoin
#: liveness check. Under the tuned 64/128 geometry epoch 2 is [256, 320) and the DEAL window opens
#: at 189, so a guard reading of 145 is in-window and a seating at 272 leaves 48 blocks of the
#: epoch for the member to be elected in. The guard, the seating and the pacing delta are the
#: numbers live runs actually produced.
#:
#: The ORDER is the case's own and changing it here to make a test pass would hide a reordering in
#: the body: pacing rides the SHORTHANDED window (victim down) because there is no room for a 60 s
#: window after the victim is seated 16 blocks into a 64-block epoch.
def _fin(guard=145, pace=None, seated=272, before=274, after=280):
    """`finalized_dec` for a passing run, with a pacing pair inside the band."""
    lo = guard + 5
    hi = lo + (verdicts.PACING_MIN_BLOCKS + 5 if pace is None else pace)
    return _seq(guard, lo, hi, seated, before, after)


def test_assert_vrf_dkg_live_heal_passes_when_the_member_recovers_inside_the_epoch(monkeypatch):
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin())
    asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_assert_vrf_dkg_live_heal_passes_on_EITHER_road_to_the_share(monkeypatch):
    """Both were observed live, minutes apart on the same geometry: the live ceremony's
    finalize-over-pinned when it beats the past-boundary sweep, the demote-heal's recompute when
    it does not. Pinning one made a legitimate outcome red."""
    for marker, _road in vf.SHARE_ROADS:
        ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(),
                            logs_all=lambda svc, m=marker, **kw: _heal_log(road=m))
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_assert_vrf_dkg_live_heal_refuses_to_run_past_the_DEAL_window(monkeypatch):
    """The victim would already be journaling dealings, and the case would be watching a member
    that had taken part. Fail loud rather than measure something else."""
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=lambda **kw: 200)
    with pytest.raises(SmokeFailure, match="already at/past the epoch-2 DKG DEAL window"):
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_assert_vrf_dkg_live_heal_passes_on_the_demote_heal_road_with_no_ceremony(monkeypatch):
    """THE RUN THE OLD GATE FAILED THREE TIMES, end to end through the case body. The victim came
    back holding nothing, the demote-heal fetched every pinned dealer's log (`want == dealers`)
    and recomputed — `start_fresh` never ran, so there is no `ceremony started` to read, and the
    old single-witness gate called that a journal."""
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(),
                        logs_all=lambda svc, **kw: _heal_log(fresh=False, heal=(4, 4),
                                                             road=vf.HEAL_LINE))
    asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_assert_vrf_dkg_live_heal_fails_when_the_victim_came_back_with_a_journal(monkeypatch):
    """THE SETUP GATE. `want < dealers` at the heal means the journal already held some dealer
    logs — the victim had received and ACKED those dealings before it went down, so it rebuilds
    from its own records and their dealers reveal nothing. Every other leg here stays green on
    that run, which is why the gate exists."""
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(),
                        logs_all=lambda svc, **kw: _heal_log(fresh=False, heal=(1, 4)))
    with pytest.raises(SmokeFailure, match="came back holding a journal"):
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_the_setup_gate_ignores_a_ceremony_start_from_BEFORE_the_restart(monkeypatch):
    """`docker compose logs` returns the whole container log, pre-stop lines included. A
    `ceremony started` from the process that was stopped means the OPPOSITE of what this gate
    witnesses, so the slice at the last boot marker is load-bearing, not tidiness: counted, this
    log passes; sliced, it has no witness at all and says so."""
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(),
                        logs_all=lambda svc, **kw: _heal_log(fresh=False,
                                                             pre_stop_ceremony=True))
    with pytest.raises(SmokeFailure, match="took NEITHER road"):
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_assert_vrf_dkg_live_heal_fails_when_the_share_never_arrives(monkeypatch):
    """THE INVERSION ITSELF. Under the old behaviour this log — absent, and nothing after — was
    the PASSING one; it is now the failure the case exists to catch."""
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(),
                        logs_all=lambda svc, **kw: _heal_log(road=None))
    with pytest.raises(SmokeFailure, match="did not recover its epoch-2 share"):
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_assert_vrf_dkg_live_heal_fails_when_the_artifact_pull_never_landed(monkeypatch):
    """The mechanism, named. A zero here says the live-epoch pull did not happen, which is the
    exact shape of the FLU-1166 regression. Verified live to move on BOTH roads."""
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(),
                        node_metric=lambda svc, name, **kw: "0")
    with pytest.raises(SmokeFailure, match="never obtained the LIVE epoch's agreed artifact"):
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_assert_vrf_dkg_live_heal_fails_when_the_pull_counter_is_unread(monkeypatch):
    """An UNREAD counter is not a zero and not a pass: `node_metric` answers "" for both an absent
    family and a dead scrape, and coercing that to 0 would make the one assertion that names the
    fix satisfiable by an endpoint nobody served."""
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(),
                        node_metric=lambda svc, name, **kw: "")
    with pytest.raises(SmokeFailure, match="could not be read off validator-3"):
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_assert_vrf_dkg_live_heal_fails_when_the_key_never_reached_VERIFICATION(monkeypatch):
    """A member can hold the share and still verify certificates seed-blind — the share signs a
    partial, the group key checks an assembled seed, and they are resolved by different rungs. So
    it gets its own reading, and after FLU-1202 that reading is a counter rather than the deleted
    `epoch scheme upgraded to PINNED` line.

    The world here is the sharp one: every OTHER leg passes — fresh journal, artifact pulled,
    share recovered, member seated — and only the verification counter is flat. That is the state
    the leg exists to catch, and nothing else in the case would notice it."""
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(),
                        node_metrics_text=lambda svc, **kw: (
                            f"{vf.SEED_VERIFY_OK_SAMPLE} 0\n{vf.SEED_VERIFY_NO_KEY_SAMPLE} 40\n"))
    with pytest.raises(SmokeFailure, match="verified ZERO seeds"):
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_assert_vrf_dkg_live_heal_fails_when_the_seed_verify_counter_is_UNREAD(monkeypatch):
    """An unscraped registry is not a zero and not a pass — the leg's positive edge is only worth
    anything if the endpoint answered."""
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(),
                        node_metrics_text=lambda svc, **kw: "")
    with pytest.raises(SmokeFailure, match="could not be read off"):
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_assert_vrf_dkg_live_heal_fails_when_the_member_is_never_seated(monkeypatch):
    """…and a share that lands with no edge to wake is a share nobody votes with. Without this the
    production leg below would fail for a reason that is not about the key."""
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(),
                        logs_all=lambda svc, **kw: _heal_log(promote=False))
    with pytest.raises(SmokeFailure, match="never seated as a signer"):
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_assert_vrf_dkg_live_heal_fails_when_the_seated_member_produced_nothing(monkeypatch):
    """THE LOAD-BEARING LEG. Every reading above can be green on a node that recovered its share
    and never won a slot; only production observes the whole chain of consequences."""
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(),
                        production=lambda epoch, addr, **kw: (0, 64))
    with pytest.raises(SmokeFailure, match="produced NOTHING in epoch 2"):
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_the_production_credit_is_read_only_after_epoch_2_has_ended(monkeypatch):
    """WHY THIS CASE FIRST WENT RED. `producedAt(2, idx)` climbs for as long as epoch 2 runs, so a
    read taken seconds after the member is seated reports the slots it has won SO FAR — on a live
    run that was 0 at height ~260 and 10 of 64 by the time the epoch finished. The wait must
    therefore reach the END of the epoch, plus the K-block deferred-execution lag (the counter for
    height h is written when h EXECUTES, so epoch 2's last block is credited K heights later), and
    must precede the counter read.

    It targets the epoch-3 boundary and not a count of the victim's own blocks, because that is
    the only floor at which the counter is FINAL — anything earlier samples a prefix of the epoch
    and calls the remainder a lottery loss."""
    seen = []
    ctx, _ = _dkg_world(
        monkeypatch, finalized_dec=_fin(),
        wait_finalized_ge=lambda target, timeout: seen.append(target) or True,
        production=lambda epoch, addr, **kw: (seen.append("read") or (10, 64)))
    asserts_fault.assert_vrf_dkg_live_heal(ctx)
    epoch3 = vrf_dkg_live_heal.ACTIVATION_BLOCK + 3 * vrf_dkg_live_heal.EPOCH_INTERVAL
    floor = epoch3 + vf.RESULT_LAG_K
    assert floor in seen and seen.index(floor) < seen.index("read")


def test_the_pacing_window_rides_the_SHORTHANDED_span_and_not_the_recovery(monkeypatch):
    """WHERE the 60 s window sits is a property of the case, not a detail. The victim is seated
    ~16 blocks into a 64-block epoch 2 and the production leg waits out the rest of it, so a window
    opened after the recovery would be measuring the epoch-2→3 boundary rather than the chain. So
    the window brackets the n-f=3 span with the victim DOWN, which is where the fault this case
    injects actually is."""
    _, r = _dry(vrf_dkg_live_heal)
    flat = _flat(r)
    # The SECOND `docker compose stop` is the victim's; the first is the migration's
    # graceful-stop of the whole committee.
    stop = flat.index("docker compose stop", 1 + flat.index("docker compose stop"))
    pace = flat.index(f"{verdicts.PACING_WINDOW_S}s")
    restart = flat.index("docker compose start")
    assert stop < pace < restart


def test_assert_vrf_dkg_live_heal_fails_when_the_chain_stalls_after_the_rejoin(monkeypatch):
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(before=274, after=274))
    with pytest.raises(SmokeFailure, match="not finalizing after validator-3 rejoined"):
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_assert_vrf_dkg_live_heal_fails_when_the_rejoined_member_derives_a_different_seed(
        monkeypatch):
    """It re-derives prev_randao from the CERT seed. A divergence means it fell to the digest
    fallback instead — the thing the recovered share is supposed to make unnecessary."""
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(),
                        mixhash_in=lambda svc, block, **kw: (
                            "0x" + "ff" * 32 if int(block) == 258 else _mix(block)))
    with pytest.raises(SmokeFailure, match="did not recover the cert seed"):
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


def test_assert_vrf_dkg_live_heal_fails_when_the_block_rate_falls_out_of_band(monkeypatch):
    """The pacing instrument, measured on the fleet that has just re-admitted a recovered signer.
    `evaluate_still_finalizing` above passes on ONE block in six seconds; the historical
    regression was 26-27 blk/60s, which is exactly that shape."""
    ctx, _ = _dkg_world(monkeypatch, finalized_dec=_fin(pace=27))
    with pytest.raises(SmokeFailure, match="block rate off target: 27 blocks"):
        asserts_fault.assert_vrf_dkg_live_heal(ctx)


# ── crash-survivor ─────────────────────────────────────────────────────────

def _crash_world(monkeypatch, **over):
    world = dict(
        baseline_height=lambda **kw: 100,
        compose_ps_q=lambda service, **kw: CID,
        wait_finalized_ge=lambda target, timeout: True,
        # 112 is `head` — the height the chain was MEASURED at while the victim was down — and it
        # is now the realign floor, so the recovery readings have to be strictly past it. A world
        # in which everyone reads exactly `head` is the vacuous pass this case shipped with.
        finalized_dec=lambda **kw: 112,
        check_external=lambda port, **kw: "0x71|0xaa",
        check_node=lambda service, **kw: "0x71|0xaa",
        # The producer's chain, for the same-height fork half of the realign gate: block 113 is
        # 0xaa and the producer holds nothing else. Stubbed EXPLICITLY — left unstubbed it would
        # reach the real `cast block`, which the suite's conftest blocks into a `"null"`, and a
        # fork test that only passes because the fixture swallowed a subprocess tests the fixture.
        blockhash_at=lambda block, **kw: ("0xaa" if int(block) == 0x71 else "null"),
        peer_count=lambda service, **kw: 3,
        dump_logs=lambda *a, **kw: None,
    )
    world.update(over)
    return _live_ctx(monkeypatch, **world)


def test_assert_crash_survivor_passes_when_the_victim_realigns(monkeypatch):
    ctx, _ = _crash_world(monkeypatch)
    asserts_fault.assert_crash_survivor(ctx)


def test_assert_crash_survivor_fails_when_the_container_id_will_not_resolve(monkeypatch):
    """`docker compose ps -q` is RUNNING-only. An empty answer means there is nothing to kill,
    and killing "" would be a no-op the case would then report as a successful crash."""
    ctx, _ = _crash_world(monkeypatch, compose_ps_q=lambda service, **kw: "")
    with pytest.raises(SmokeFailure, match="could not resolve validator-3 container id"):
        asserts_fault.assert_crash_survivor(ctx)


def test_assert_crash_survivor_fails_when_the_chain_stalled_with_one_node_crashed(monkeypatch):
    """The premise: without an EL gap there is nothing to backfill, and the recovery half would
    pass over a node that had nothing to recover."""
    ctx, _ = _crash_world(monkeypatch, finalized_dec=lambda **kw: 101)
    with pytest.raises(SmokeFailure, match="chain stalled with 1/4 crashed"):
        asserts_fault.assert_crash_survivor(ctx)


def test_assert_crash_survivor_fails_when_the_victim_wedges(monkeypatch):
    """Problem A itself: the node restarts, answers nothing, and never realigns."""
    ctx, _ = _crash_world(monkeypatch, check_node=lambda service, **kw: "null|null")
    with pytest.raises(SmokeFailure, match="did not realign after crash"):
        asserts_fault.assert_crash_survivor(ctx)


def test_assert_crash_survivor_fails_when_the_victim_comes_back_on_a_fork(monkeypatch):
    """Same height, different block. A height-only comparison would call this recovered."""
    ctx, _ = _crash_world(monkeypatch, check_node=lambda service, **kw: "0x71|0xbb")
    with pytest.raises(SmokeFailure, match="did not realign after crash"):
        asserts_fault.assert_crash_survivor(ctx)


def test_assert_crash_survivor_fails_when_the_victim_never_backfilled_its_gap(monkeypatch):
    """THE VACUOUS PASS, driven through the body. The victim comes back on the producer's chain
    at 65 — its own persisted tail — while the chain reached 112 without it. Same-height identity
    alone blesses that (a live run printed `realigned at 0x41(=65) … (v0=0x50(=80))` and passed);
    the `head` floor is the only thing that rejects it."""
    ctx, _ = _crash_world(
        monkeypatch,
        check_node=lambda service, **kw: "0x41|0xtail",
        blockhash_at=lambda block, **kw: ("0xaa" if int(block) == 0x71
                                          else "0xtail" if int(block) == 0x41 else "null"))
    with pytest.raises(SmokeFailure, match="did not realign after crash"):
        asserts_fault.assert_crash_survivor(ctx)


def test_the_crash_survivor_OK_line_prints_the_producer_height_too(monkeypatch, capsys):
    """A pass that cannot be audited from its own output is how the vacuity survived a live run:
    bash printed `(v0=…)` all along and the port printed only the victim."""
    ctx, _ = _crash_world(monkeypatch)
    asserts_fault.assert_crash_survivor(ctx)
    out = capsys.readouterr().out
    assert "realigned at 0x71|0xaa" in out and "v0=0x71|0xaa" in out
    assert "floor=head-while-down=112" in out


def test_the_crashed_victim_is_restarted_even_though_the_kill_is_raw(monkeypatch):
    """`docker kill` bypasses compose, so nothing else will bring the container back — the
    restart is the case's own responsibility and is not something teardown does for it."""
    ctx, runner = _crash_world(monkeypatch)
    asserts_fault.assert_crash_survivor(ctx)
    argvs = runner.argvs()
    assert ["docker", "kill", CID] in argvs and ["docker", "start", CID] in argvs
    assert argvs.index(["docker", "kill", CID]) < argvs.index(["docker", "start", CID])


# ── full-restart ───────────────────────────────────────────────────────────

def _restart_world(monkeypatch, **over):
    world = dict(
        baseline_height=lambda **kw: 100,
        shutdown_flushed=lambda service, **kw: True,
        check_external=lambda port, **kw: "0x70|0xaa",
        check_node=lambda service, **kw: "0x70|0xaa",
        # A block stamped AFTER the restart instant. Read off the real clock rather than pinned:
        # the case samples `restart_at` from `time.time()` at run time, and `_live_ctx` only
        # fast-forwards `monotonic`.
        timestamp_at=lambda block, **kw: int(time.time()) + 60,
        # The resumption wait reads the finalized head separately from the reconverge poll.
        finalized_dec=lambda **kw: 0x70,
        # The producer's chain for the fork half of the reconverge gate — see `_crash_world` on
        # why this is stubbed rather than left to reach a conftest-blocked `cast block`.
        blockhash_at=lambda block, **kw: ("0xaa" if int(block) == 0x70 else "null"),
        dump_logs=lambda *a, **kw: None,
    )
    world.update(over)
    return _live_ctx(monkeypatch, **world)


def test_assert_full_restart_passes_when_the_fleet_resumes_from_disk(monkeypatch):
    ctx, _ = _restart_world(monkeypatch)
    asserts_fault.assert_full_restart(ctx)


def test_assert_full_restart_fails_when_a_validator_did_not_exit_zero(monkeypatch):
    """THE flush assertion, and the direction only a unit test can drive: a validator SIGKILLed
    at the stop ceiling comes back, resyncs from its peers and reconverges perfectly — the
    reconverge check below would bless a node that lost its persisted tail."""
    ctx, _ = _restart_world(monkeypatch,
                            shutdown_flushed=lambda service, **kw: service != "validator-2")
    with pytest.raises(SmokeFailure, match="validator-2 did not exit cleanly"):
        asserts_fault.assert_full_restart(ctx)


def test_assert_full_restart_checks_all_four_not_just_the_first(monkeypatch):
    ctx, _ = _restart_world(monkeypatch,
                            shutdown_flushed=lambda service, **kw: service != "validator-3")
    with pytest.raises(SmokeFailure, match="validator-3 did not exit cleanly"):
        asserts_fault.assert_full_restart(ctx)


def test_assert_full_restart_fails_when_the_network_does_not_reconverge(monkeypatch):
    ctx, _ = _restart_world(monkeypatch,
                            check_node=lambda service, **kw: ("0x69|0xbb"
                                                              if service == "validator-3"
                                                              else "0x70|0xaa"))
    with pytest.raises(SmokeFailure, match="did not reconverge"):
        asserts_fault.assert_full_restart(ctx)


def test_assert_full_restart_fails_when_the_fleet_came_back_below_the_persisted_head(monkeypatch):
    """Everyone agrees, everyone is live, and the chain is BEHIND where it was stopped — a fleet
    that lost its tail and resynced to a shorter chain."""
    ctx, _ = _restart_world(monkeypatch, check_external=lambda port, **kw: "0x40|0xaa",
                            check_node=lambda service, **kw: "0x40|0xaa")
    with pytest.raises(SmokeFailure, match="did not reconverge"):
        asserts_fault.assert_full_restart(ctx)


def test_assert_full_restart_fails_when_the_fleet_never_produced_a_block_after_the_restart(
        monkeypatch):
    """THE REGRESSION. All five come back on the persisted head and NOTHING resumes — no block is
    ever produced on top of it. `>= pre` calls that reconvergence (it is exactly what both trees
    printed: `all 5 reconverged at 0x41 (>= pre=65)`), and it proves only that everyone came back
    on the same tail."""
    ctx, _ = _restart_world(monkeypatch, check_external=lambda port, **kw: "0x64|0xaa",
                            check_node=lambda service, **kw: "0x64|0xaa",
                            blockhash_at=lambda block, **kw: ("0xaa" if int(block) == 0x64
                                                              else "null"))
    with pytest.raises(SmokeFailure, match="did not reconverge"):
        asserts_fault.assert_full_restart(ctx)


def test_assert_full_restart_fails_when_the_fleet_came_back_on_its_PERSISTED_TAIL(monkeypatch):
    """THE SAMPLING-MOMENT HOLE, driven — and this is the exact shape the live run showed on
    2026-08-03. Every height check passes: the fleet is agreed, on the producer's chain and
    STRICTLY past `pre`. It gets there without producing anything, because `pre` is sampled BEFORE
    a 40s stop window and the blocks written inside that window are on disk and above the floor.

    THE TAIL RUNS SEVERAL BLOCKS PAST THE CONVERGED HEIGHT and every one of them is pre-restart —
    live it was `pre=65` with 66, 67 and 68 all older than the restart. So a check that merely
    looked one block up, or that took a block's EXISTENCE for resumption, still sees nothing but
    tail. Only a stamp at or past `restart_at` distinguishes them."""
    stopped = int(time.time()) - 3600
    ctx, _ = _restart_world(monkeypatch,
                            timestamp_at=lambda block, **kw: (stopped + int(block)
                                                              if int(block) <= 0x74 else 0),
                            finalized_dec=lambda **kw: 0x70)
    with pytest.raises(SmokeFailure) as e:
        asserts_fault.assert_full_restart(ctx)
    assert "produced NO block" in e.value.message
    assert "PERSISTED TAIL" in e.value.message


def test_an_unreadable_timestamp_fails_the_restart_witness_rather_than_passing_it(monkeypatch):
    """`block_field_at` answers `"null"` for an unreachable node as well as for a missing field,
    and `hex_to_dec` maps that to 0. Zero is older than any restart instant, so the read failure
    lands on the RED side — the direction a witness has to fail in."""
    ctx, _ = _restart_world(monkeypatch, timestamp_at=lambda block, **kw: 0)
    with pytest.raises(SmokeFailure, match="produced NO block"):
        asserts_fault.assert_full_restart(ctx)


def test_the_restart_diagnostic_never_calls_a_PRE_restart_block_resumption(monkeypatch, capsys):
    """THE INFERENCE BUG, driven. The dump concluded "the chain KEPT CLIMBING, so the fleet
    resumed" from a block above the converged height merely EXISTING — while printing that block's
    stamp showing it was OLDER than the restart. A next block that is itself pre-restart is more
    tail, not evidence of anything."""
    stopped = int(time.time()) - 3600
    ctx, _ = _restart_world(monkeypatch,
                            timestamp_at=lambda block, **kw: (stopped + int(block)
                                                              if int(block) <= 0x74 else 0),
                            finalized_dec=lambda **kw: 0x70)
    with pytest.raises(SmokeFailure):
        asserts_fault.assert_full_restart(ctx)
    out = capsys.readouterr().out
    assert "resumption is unproven" in out and "came back on its tail" in out
    assert "resume" not in out.replace("resumption is unproven", "")


def test_the_restart_diagnostic_names_the_FIRST_post_restart_block_when_one_exists(
        monkeypatch, capsys):
    """The other direction: a tail that runs 3 blocks past the converged height and THEN a real
    produced block. The diagnostic must name that block, because it is the evidence that the gate
    was reading the tail rather than that the fleet was dead."""
    now = int(time.time())
    ctx, _ = _restart_world(
        monkeypatch,
        # 0x70..0x73 are tail (pre-restart); 0x74 is the first produced block.
        timestamp_at=lambda block, **kw: (now - 100 if int(block) <= 0x73 else now + 60),
        # …but the finalized head still sits on the tail, so the WAIT expires and the case fails.
        finalized_dec=lambda **kw: 0x70)
    with pytest.raises(SmokeFailure):
        asserts_fault.assert_full_restart(ctx)
    out = capsys.readouterr().out
    assert "first POST-restart block 116" in out and "the chain DID resume" in out


def test_assert_full_restart_fails_when_one_node_is_wedged_at_the_persisted_head(monkeypatch):
    """The ragged shape of the same thing: four validators resume and climb, the fifth never
    leaves the block it was stopped on. It is on the right chain — same-height identity confirms
    it — and under `>= pre` it clears the floor by sitting still."""
    ctx, _ = _restart_world(
        monkeypatch,
        check_node=lambda service, **kw: ("0x64|0xbb" if service == "validator-2"
                                          else "0x70|0xaa"),
        blockhash_at=lambda block, **kw: ("0xaa" if int(block) == 0x70
                                          else "0xbb" if int(block) == 0x64 else "null"))
    with pytest.raises(SmokeFailure, match="did not reconverge"):
        asserts_fault.assert_full_restart(ctx)


def test_the_full_restart_OK_line_prints_every_node_not_just_the_producer(monkeypatch, capsys):
    """There is no victim/hub split here, so the auditable evidence is the whole set against the
    floor. `(>= pre=65)` next to a single reading is what made the weakened case look fine."""
    ctx, _ = _restart_world(monkeypatch)
    asserts_fault.assert_full_restart(ctx)
    out = capsys.readouterr().out
    assert "(> pre=100)" in out
    assert out.count("0x70|0xaa") >= 5, "one entry per node on the OK line"
    assert "full-node=0x70|0xaa" in out


def test_assert_full_restart_fails_when_the_fleet_came_back_at_genesis(monkeypatch):
    """Every data directory wiped, all five in perfect agreement about starting over. Byte
    equality alone would call it reconvergence."""
    ctx, _ = _restart_world(monkeypatch, check_external=lambda port, **kw: "0x0|0x0",
                            check_node=lambda service, **kw: "0x0|0x0")
    with pytest.raises(SmokeFailure, match="did not reconverge"):
        asserts_fault.assert_full_restart(ctx)


def test_the_committee_is_started_again_after_the_flush_check(monkeypatch):
    ctx, runner = _restart_world(monkeypatch)
    asserts_fault.assert_full_restart(ctx)
    assert ["docker", "compose", "start", *VALS] in runner.argvs()
