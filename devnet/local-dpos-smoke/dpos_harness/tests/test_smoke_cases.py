"""`cases/smoke/` — the five ported case drivers, against `scripts/case-*.sh` + `asserts.sh`.

Two kinds of evidence live here, and neither substitutes for the other:

  * THE COMMAND TRANSCRIPT (`--dry-run`) — argv for argv against the bash, in order. Without a
    docker daemon this is the only fidelity evidence there is, so it carries the same weight here
    that `test_static_stack.py::test_bring_up_dpos_transcript_is_bash_faithful` carries for the
    bring-up. Every expectation below is quoted against its `asserts.sh` line.
  * THE WIRING (stubbed readers, LIVE branches) — the assertion bodies run for real against
    scripted readings, so a body that reads the right things and then forgets to apply the verdict
    fails here. `test_smoke_verdicts.py` proves the verdicts decide correctly; this file proves
    the bodies ASK them.

The second kind exists because the first cannot see it: a dry run deliberately never fails a
verdict, so a `ctx.check(...)` deleted from a body would leave every transcript test green.
"""

from __future__ import annotations

import pytest

from dpos_harness.cases.smoke import asserts, base, driver, epoch, tx, verdicts, vrf, vrf_boundary
from dpos_harness.cases.smoke.driver import SmokeCtx, SmokeFailure
from dpos_harness.core import proc
from dpos_harness.core.proc import Runner
from dpos_harness.stack.profiles import StaticProfile
from dpos_harness.stack.static_stack import StaticStack

VALS = ["validator-0", "validator-1", "validator-2", "validator-3"]
RPC = "http://localhost:8545"
UP = ["docker", "compose", "up", "--build", "-d"]
STOP = ["docker", "compose", "stop", "--timeout", "40", *VALS]
RECREATE = ["docker", "compose", "-f", "docker-compose.yml", "-f", "docker-compose.dpos.yml",
            "up", "-d", "--force-recreate", *VALS]
DOWN = ["docker", "compose", "down", "-v", "--remove-orphans"]

ZERO = "0x" + "0" * 64


def _mix(tag: str) -> str:
    return "0x" + tag * 64


def _blockmix(n) -> str:
    """A distinct, VALID-HEX prev_randao per height. Hex matters: the log parser extracts the only
    64-hex run on the line, so a stand-in built from arbitrary letters would silently parse as
    nothing and make the log/header cross-check pass over an empty set."""
    return "0x%064x" % int(n)


@pytest.fixture(autouse=True)
def _clean_env(monkeypatch):
    """The case layer reads six environment variables. A value leaked from the operator's shell
    would make every expectation below depend on where the suite was run from."""
    for k in ("EPOCH_INTERVAL", "DPOS_ACTIVATION_BLOCK", "DPOS_EXTRA_COMPOSE",
              "DPOS_CONVERGE_EXCLUDE", "SIM_DATA_ROOT", "EPOCH_MIN_CROSS", "SMOKE_KEEP_UP",
              "RPC", "CHAIN_ID"):
        monkeypatch.delenv(k, raising=False)


def _dry(case_module, argv=("--dry-run",)):
    """Run a case in dry mode, capturing its runner. Returns (rc, argvs)."""
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
    return rc, made["r"].argvs()


# ══ the command transcripts ════════════════════════════════════════════════

def test_tx_transcript_is_bash_faithful():
    """`asserts.sh:21-59` + `case-tx.sh`, argv for argv:

        :21  docker compose exec -T validator-0 cat /runtime/keys/funded.hex
        :22  cast wallet address --private-key 0x$KEY
        :28  cast balance $DEAD --rpc-url $RPC
        :31  cast send --private-key 0x$KEY --rpc-url $RPC --chain $CHAIN_ID $DEAD \
                 --value 0.1ether --json
        :35  cast send … $BLEND approve(address,uint256) $DEAD 12345 --json
        :38  cast receipt $h --rpc-url $RPC --json          (per tx)
        :51  cast balance $DEAD --rpc-url $RPC
        :59  cast call $BLEND allowance(address,address)(uint256) $FROM $DEAD --rpc-url $RPC
    """
    rc, argvs = _dry(tx)
    key = "0x" + "00" * 32
    send = ["cast", "send", "--private-key", key, "--rpc-url", RPC, "--chain", "2026"]
    assert rc == 0
    assert argvs == [
        UP, STOP, RECREATE,
        ["docker", "compose", "exec", "-T", "validator-0", "cat", "/runtime/keys/funded.hex"],
        ["cast", "wallet", "address", "--private-key", key],
        ["cast", "balance", verdicts.DEAD_ADDR, "--rpc-url", RPC],
        send + [verdicts.DEAD_ADDR, "--value", "0.1ether", "--json"],
        send + [verdicts.BLEND_ADDR, "approve(address,uint256)", verdicts.DEAD_ADDR, "12345",
                "--json"],
        ["cast", "receipt", "0xvalue", "--rpc-url", RPC, "--json"],
        ["cast", "receipt", "0xapprove", "--rpc-url", RPC, "--json"],
        ["<step>", "poll"],
        ["cast", "balance", verdicts.DEAD_ADDR, "--rpc-url", RPC],
        ["cast", "call", verdicts.BLEND_ADDR, "allowance(address,address)(uint256)",
         "0x" + "11" * 20, verdicts.DEAD_ADDR, "--rpc-url", RPC],
        DOWN,
    ]


def test_the_balance_is_read_before_and_after_the_sends():
    """The assertion is a DELTA. An "after" with no "before" is a balance, and would pass on a
    chain that never moved a wei — the recipient's balance is nonzero from the first run onward."""
    _, argvs = _dry(tx)
    balances = [i for i, a in enumerate(argvs) if a[:2] == ["cast", "balance"]]
    sends = [i for i, a in enumerate(argvs) if a[:2] == ["cast", "send"]]
    assert len(balances) == 2 and len(sends) == 2
    assert balances[0] < sends[0] and balances[1] > sends[-1]


def test_the_contract_call_is_not_optional():
    """`asserts.sh:33-35` — the approve is what exercises CALL + SSTORE. A transfer-only case
    would pass on a chain whose EVM never executes, which is the gap it was added to close."""
    _, argvs = _dry(tx)
    approves = [a for a in argvs if "approve(address,uint256)" in a]
    assert len(approves) == 1 and verdicts.BLEND_ADDR in approves[0]


def test_epoch_issues_no_chain_writes():
    """`assert_epoch` is a pure observation: it polls, reads two getters and measures a rate.
    Anything else in this transcript would mean the case is disturbing what it measures — and
    `smoke-base` runs it on a SHARED stack, where that would contaminate three other assertions."""
    rc, argvs = _dry(epoch)
    assert rc == 0
    writes = [a for a in argvs if a[0] not in ("<step>",) and a != UP and a != STOP
              and a != RECREATE and a != DOWN]
    assert writes == [], f"assert_epoch issued chain commands: {writes}"


def test_vrf_transcript_deploys_the_probe_and_snapshots_it():
    """`asserts.sh:289-296` — the ONLY two writes in the VRF case. Note neither carries `--chain`:
    the bash does not pass it there, and adding it would be a silent argv change."""
    rc, argvs = _dry(vrf)
    key = "0x" + "00" * 32
    sends = [a for a in argvs if a[:2] == ["cast", "send"]]
    assert rc == 0 and len(sends) == 2
    assert sends[0][:7] == ["cast", "send", "--private-key", key, "--rpc-url", RPC, "--create"]
    assert sends[0][7].startswith("0x60") and sends[0][-1] == "--json"
    assert sends[1] == ["cast", "send", "--private-key", key, "--rpc-url", RPC, "0xprobe",
                        "snapshot()", "--json"]


def test_vrf_reads_the_commonware_registry_only():
    """`asserts.sh:344` reads :19100 — the commonware registry. `nodes.node_metrics` would also
    concatenate reth's :19200; widening the text a SUBSTRING match runs over can only add hits."""
    assert driver.CONSENSUS_METRICS_URL == "http://localhost:19100/metrics"
    made = {}
    ctx = SmokeCtx(StaticStack(profile=StaticProfile(), runner=Runner(dry=True)))
    ctx.p.step = lambda label, detail: made.setdefault("d", detail)
    ctx.consensus_metrics()
    assert made["d"] == "consensus_metrics(http://localhost:19100/metrics)"


def test_vrf_boundary_issues_no_chain_writes():
    rc, argvs = _dry(vrf_boundary)
    assert rc == 0
    assert not [a for a in argvs if a[0] in ("cast", "curl")]


def test_base_runs_all_four_on_exactly_one_bring_up():
    """`case-base.sh` — the four are read-only w.r.t. consensus, so they share ONE migration
    instead of paying for four. If a second bring-up ever appears here, the sharing is gone and
    the case costs four times what it claims to."""
    rc, argvs = _dry(base)
    assert rc == 0
    assert argvs.count(UP) == 1 and argvs.count(RECREATE) == 1 and argvs.count(DOWN) == 1
    assert argvs[0] == UP and argvs[-1] == DOWN


def test_base_runs_the_four_in_increasing_sophistication():
    """tx -> epoch -> vrf -> vrf-boundary. Fail-fast means the ORDER decides which failure you
    see: the cheapest broken thing must be reported, not the longest assertion that also broke."""
    assert base.ASSERTIONS == [asserts.assert_tx, asserts.assert_epoch, asserts.assert_vrf,
                               asserts.assert_vrf_boundary]


def test_a_dry_run_never_spawns_a_process(monkeypatch):
    """The point of the transcript is that it can be produced anywhere, against no chain."""
    monkeypatch.setattr(proc.subprocess, "run",
                        lambda *a, **k: pytest.fail("the dry run executed a command"))
    for mod in (tx, epoch, vrf, vrf_boundary, base):
        assert mod.run_case(["--dry-run"]) == 0


def test_an_unknown_argument_is_refused():
    assert tx.run_case(["--turbo"]) == 2


# ══ the wrapper: teardown, keep-up, fail-fast ══════════════════════════════

class _Fail(Exception):
    pass


def _stub_stack(monkeypatch, calls):
    """Patch `StaticStack` so `driver.run` brings up and tears down without docker."""
    monkeypatch.setattr(StaticStack, "bring_up_dpos",
                        lambda self: calls.append("bring-up") or "0x40")
    monkeypatch.setattr(StaticStack, "tear_down", lambda self: calls.append("teardown"))


def test_a_failing_assertion_still_tears_the_stack_down(monkeypatch):
    """§2.4 item 12 — bash's `exit 1` fired `trap tear_down EXIT`; a Python `raise` only unwinds.
    Without the `finally` every failed case would leak a running devnet."""
    calls = []
    _stub_stack(monkeypatch, calls)

    def boom(ctx):
        raise SmokeFailure("smoke-x", "nope")

    assert driver.run("smoke-x", [boom]) == 1
    assert calls == ["bring-up", "teardown"]


def test_the_first_failure_stops_the_rest(monkeypatch):
    """Fail-fast, as `case-base.sh` is: the later assertions must not run against a chain that
    already failed a cheaper check, and they must not overwrite its verdict."""
    calls = []
    _stub_stack(monkeypatch, calls)
    ran = []

    def one(ctx):
        ran.append(1)
        raise SmokeFailure("smoke-x", "first")

    def two(ctx):
        ran.append(2)

    assert driver.run("smoke-x", [one, two]) == 1
    assert ran == [1]


def test_a_passing_case_tears_down_too(monkeypatch):
    calls = []
    _stub_stack(monkeypatch, calls)
    assert driver.run("smoke-x", [lambda ctx: None]) == 0
    assert calls == ["bring-up", "teardown"]


def test_a_failed_bring_up_is_not_torn_down_twice(monkeypatch):
    """`StaticStack` already tears itself down on a failed bring-up, and bash installs its trap
    AFTER `bring_up_dpos` for the same reason. A second teardown here would be harmless but the
    ProcError path (compose up failed, nothing running) is not the trap's job."""
    calls = []
    monkeypatch.setattr(StaticStack, "tear_down", lambda self: calls.append("teardown"))
    monkeypatch.setattr(StaticStack, "bring_up_dpos",
                        lambda self: (_ for _ in ()).throw(
                            proc.ProcError(proc.RunResult(argv=["docker"], rc=1), "phase1-up")))
    assert driver.run("smoke-x", [lambda ctx: pytest.fail("ran an assertion")]) == 1
    assert calls == []


@pytest.mark.parametrize("honours,kept", [(True, True), (False, False)])
def test_smoke_keep_up_is_honoured_per_case(monkeypatch, honours, kept):
    """ONE of the five bash wrappers reads `SMOKE_KEEP_UP` (`case-vrf.sh:14`); the other four tear
    down unconditionally. Widening it would be a behaviour change dressed as a tidy-up — and the
    four unconditional teardowns are what keeps one case's stack out of the next case's bring-up.
    """
    monkeypatch.setenv("SMOKE_KEEP_UP", "1")
    calls = []
    _stub_stack(monkeypatch, calls)
    driver.run("smoke-x", [lambda ctx: None], honours_keep_up=honours)
    assert ("teardown" in calls) is (not kept)


def test_only_the_vrf_wrapper_opts_into_keep_up(monkeypatch):
    """Asserted through the wrappers themselves, not through `driver.run`'s flag, because the
    thing that can rot is which module passes it."""
    seen = {}
    monkeypatch.setattr(driver, "run",
                        lambda case, a, argv=None, converge_exclude=None, honours_keep_up=False:
                        seen.__setitem__(case, honours_keep_up) or 0)
    for mod in (tx, epoch, vrf, vrf_boundary, base):
        mod.run_case([])
    assert seen == {"smoke-tx": False, "smoke-epoch": False, "smoke-vrf": True,
                    "smoke-vrf-boundary": False, "smoke-base": False}


def test_every_smoke_case_is_registered_in_the_cli():
    """`case <name>` and `case list` read one registry, so a case added to one is never missing
    from the other — and the Makefile targets resolve through it."""
    from dpos_harness import cli
    for name, mod in [("smoke-base", "smoke.base"), ("smoke-tx", "smoke.tx"),
                      ("smoke-epoch", "smoke.epoch"), ("smoke-vrf", "smoke.vrf"),
                      ("smoke-vrf-boundary", "smoke.vrf_boundary")]:
        assert cli.CASES[name] == mod


# ══ the wiring: assertion bodies on their LIVE branches ════════════════════

def _live_ctx(monkeypatch, **stubs):
    """A `SmokeCtx` on its LIVE branches with every reader stubbed.

    The Runner stays dry (so a write is recorded and nothing spawns) while `dry` reports False, so
    `check` raises and `poll` really loops — the two halves are separated deliberately, exactly as
    `test_static_stack.py::_live_stack` does it.

    The CLOCK is fast-forwarded rather than the timeouts being shrunk: `monotonic` jumps a wall
    hour per call, so every poll takes exactly one probe and then expires. Shortening the case
    constants instead would leave the tests passing while somebody quietly shortened the real
    ones — and three cases in this tree pass BY timeout (§2.4 item 3)."""
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


def _tx_world(monkeypatch, **over):
    """A healthy `assert_tx` chain, with named overrides for the failure directions."""
    world = dict(
        funded_key=lambda **kw: "ab" * 32,
        wallet_address=lambda key, **kw: "0xfrom",
        cast_balance=_seq("1000", str(1000 + verdicts.TRANSFER_WEI)),
        cast_send=_seq({"transactionHash": "0x1"}, {"transactionHash": "0x2"}),
        cast_receipt=lambda h, **kw: {"status": "0x1", "blockNumber": "0x80"},
        cast_call=lambda *a, **kw: f"{verdicts.ALLOWANCE} [1.234e4]",
        wait_finalized_ge=lambda target, timeout: True,
        dump_logs=lambda *a, **kw: None,
    )
    world.update(over)
    return _live_ctx(monkeypatch, **world)


def test_assert_tx_passes_on_a_healthy_chain(monkeypatch):
    ctx, _ = _tx_world(monkeypatch)
    asserts.assert_tx(ctx)          # must not raise


def test_assert_tx_fails_on_a_reverted_receipt(monkeypatch):
    ctx, _ = _tx_world(monkeypatch,
                       cast_receipt=lambda h, **kw: {"status": "0x0", "blockNumber": "0x80"})
    with pytest.raises(SmokeFailure, match="status=0x0"):
        asserts.assert_tx(ctx)


def test_assert_tx_fails_when_the_transfer_did_not_apply(monkeypatch):
    ctx, _ = _tx_world(monkeypatch, cast_balance=_seq("1000", "1000"))
    with pytest.raises(SmokeFailure, match="balance delta"):
        asserts.assert_tx(ctx)


def test_assert_tx_fails_when_the_allowance_slot_is_untouched(monkeypatch):
    """Finalized, non-reverted, balance moved — and the EVM still wrote nothing. This is the only
    check that sees it."""
    ctx, _ = _tx_world(monkeypatch, cast_call=lambda *a, **kw: "0")
    with pytest.raises(SmokeFailure, match="SSTORE"):
        asserts.assert_tx(ctx)


def test_assert_tx_fails_when_the_tx_block_never_finalizes(monkeypatch):
    ctx, _ = _tx_world(monkeypatch, wait_finalized_ge=lambda target, timeout: False)
    with pytest.raises(SmokeFailure, match="not finalized in time"):
        asserts.assert_tx(ctx)


def test_an_unreadable_balance_fails_loud_instead_of_reading_as_zero(monkeypatch):
    """Sentinel discipline. Bash aborts under `set -e` on `$(( "" - "" ))`; Python would happily
    call an unreachable RPC a zero balance and then report a delta that is either exactly right or
    exactly wrong, with no way to tell which."""
    ctx, _ = _tx_world(monkeypatch, cast_balance=_seq(""))
    with pytest.raises(SmokeFailure, match="not a number"):
        asserts.assert_tx(ctx)


def test_assert_tx_fails_when_a_send_returns_no_hash(monkeypatch):
    ctx, _ = _tx_world(monkeypatch, cast_send=_seq({}))
    with pytest.raises(SmokeFailure, match="transactionHash"):
        asserts.assert_tx(ctx)


def test_assert_tx_fails_when_a_receipt_carries_no_block(monkeypatch):
    """`maxblk` would fall to 0 and `wait_finalized_ge(0)` is trivially true — the finality half of
    the assertion would evaporate silently."""
    ctx, _ = _tx_world(monkeypatch, cast_receipt=lambda h, **kw: {"status": "0x1"})
    with pytest.raises(SmokeFailure, match="no blockNumber"):
        asserts.assert_tx(ctx)


# ── epoch ──────────────────────────────────────────────────────────────────

def _epoch_world(monkeypatch, head=128, fin=(1000, 1060), committee="[0xaa]", **over):
    world = dict(
        read_sequencer_nodes=lambda **kw: [(f"n{i}", f"{hex(head)}|0xh") for i in range(5)],
        staking_call=lambda sig, *a, **kw: ("2" if "currentEpoch" in sig else committee),
        finalized_dec=_seq(*fin),
        sleep=lambda s: None,
        dump_logs=lambda *a, **kw: None,
    )
    world.update(over)
    return _live_ctx(monkeypatch, **world)


def test_assert_epoch_passes_when_the_chain_crossed_a_boundary(monkeypatch):
    ctx, _ = _epoch_world(monkeypatch)
    asserts.assert_epoch(ctx)


def test_assert_epoch_reads_epoch_min_cross(monkeypatch):
    """`make smoke-epoch` exports `EPOCH_MIN_CROSS=1`; the DKG cases raise it. The target must move
    with it or the knob is decoration."""
    monkeypatch.setenv("EPOCH_MIN_CROSS", "3")
    ctx, _ = _epoch_world(monkeypatch, head=128)     # anchor 0x50=80 -> epoch 2, target 6*32=192
    with pytest.raises(SmokeFailure, match="finalized >= 192"):
        asserts.assert_epoch(ctx)


def test_assert_epoch_fails_when_the_nodes_never_align(monkeypatch):
    """Divergent readings are never "converged" — and five UNREACHABLE nodes agree perfectly,
    which is why `aligned_reading` rejects a `"null"` head explicitly."""
    ctx, _ = _epoch_world(monkeypatch,
                          read_sequencer_nodes=lambda **kw: [("a", "null|null")] * 5)
    with pytest.raises(SmokeFailure, match="did not reach finalized"):
        asserts.assert_epoch(ctx)


def test_assert_epoch_fails_on_an_empty_committee(monkeypatch):
    ctx, _ = _epoch_world(monkeypatch, committee="[]")
    with pytest.raises(SmokeFailure, match="getEpochCommittee"):
        asserts.assert_epoch(ctx)


@pytest.mark.parametrize("fin,why", [((1000, 1010), "45..66"), ((1000, 1350), "45..66")])
def test_assert_epoch_fails_on_off_target_pacing(monkeypatch, fin, why):
    """Both directions. The fast one is the pacing REGRESSION guard — the unpaced chain did ~350
    blocks/min and would sail through a lower bound alone."""
    ctx, _ = _epoch_world(monkeypatch, fin=fin)
    with pytest.raises(SmokeFailure, match=why):
        asserts.assert_epoch(ctx)


def test_assert_epoch_fails_loud_without_a_migration_anchor(monkeypatch):
    """`PREV_FIN` starts as the literal string `"null"`. Coercing it to 0 would retarget the case
    at block `2*interval` — a height every live chain passes — and the boundary guard would be
    gone with nothing to show for it."""
    ctx, _ = _epoch_world(monkeypatch)
    ctx.stack.prev_fin = "null"
    with pytest.raises(SmokeFailure, match="no migration anchor"):
        asserts.assert_epoch(ctx)


# ── vrf ────────────────────────────────────────────────────────────────────

_ACTIVE_LOG = "\n".join(
    f"INFO {verdicts.ACTIVE_LINE} round=Round{{2,{i}}} prev_randao={_mix(chr(97 + i))}"
    for i in range(8))


def _vrf_world(monkeypatch, **over):
    """A healthy beacon: a varying window, growing active counts, logged values that match the
    headers, matching EVM/header prev_randao, and metrics that move the right way."""
    heights = {}

    def mixhash(*args, **kw):
        block = args[-1] if len(args) > 1 else args[0]
        return heights.setdefault(int(block), _blockmix(block))

    world = dict(
        wait_finalized_ge=lambda target, timeout: True,
        finalized_dec=lambda **kw: 136,
        head_dec=_seq(200, 210),
        mixhash_of=lambda svc, block, **kw: mixhash(svc, block),
        mixhash_at=lambda block, **kw: mixhash(block),
        log_count=_seq(5, 5, 5, 5, 6, 6, 6, 6),
        logs_all=lambda svc, **kw: "\n".join(
            f"INFO {verdicts.ACTIVE_LINE} prev_randao={mixhash(n)}" for n in range(120, 140)),
        consensus_metrics=_seq("beacon_digest_fallback_total 0\nbeacon_seed_active_total 7\n",
                               "beacon_digest_fallback_total 0\nbeacon_seed_active_total 9\n"),
        funded_key=lambda **kw: "ab" * 32,
        cast_send=_seq({"contractAddress": "0xprobe"},
                       {"blockNumber": "0x80", "logs": [{"data": mixhash(0x80)}]}),
        dump_logs=lambda *a, **kw: None,
    )
    world.update(over)
    return _live_ctx(monkeypatch, **world)


def test_assert_vrf_passes_on_a_healthy_beacon(monkeypatch):
    ctx, _ = _vrf_world(monkeypatch)
    asserts.assert_vrf(ctx)


def _mix_one_node_diverges(svc, block, **kw):
    return _mix("f") if svc == "full-node" and int(block) == 133 else _blockmix(block)


#: Every FAIL branch of `assert_vrf`: `(world-override factory, expected SmokeFailure text)`.
#: A factory rather than a plain dict because several rows hand the world a STATEFUL `_seq`
#: reader, which has to be built for the run rather than shared from collection time.
_VRF_FAILURES = [
    (lambda: dict(mixhash_of=lambda svc, block, **kw: ZERO),
     "prev_randao is zero", "fails_on_a_zero_prev_randao"),
    # The safety property. `full-node` is in the compared set, so this also covers E1.
    (lambda: dict(mixhash_of=_mix_one_node_diverges),
     "disagree on prev_randao at block 133", "fails_when_one_node_derives_a_different_seed"),
    # Constant, non-zero and perfectly node-agreed — every check but the variance one passes.
    (lambda: dict(mixhash_of=lambda svc, block, **kw: _mix("a")),
     "not varying", "fails_on_a_stuck_beacon"),
    (lambda: dict(log_count=_seq(5, 1, 5, 5, 6, 6, 6, 6)),
     r"only 1 times", "fails_when_a_validator_is_below_the_active_floor"),
    # Step 2b. The counts clear the floor comfortably and never move — the static threshold alone
    # reports this beacon as healthy for the rest of the run.
    (lambda: dict(log_count=_seq(50, 50, 50, 50, 50, 50, 50, 50)),
     "frozen at 50", "fails_when_the_active_count_freezes"),
    # Without new blocks there is nothing to observe, so this is a failure rather than a pass —
    # a frozen chain must not read as a sustained beacon.
    (lambda: dict(head_dec=_seq(200)),
     "cannot observe a sustained beacon", "fails_when_the_head_will_not_advance"),
    # The silent-reader shape: the log is there, the values are not extractable. It must fail,
    # because an empty `logged` set makes the header cross-check meaningless.
    (lambda: dict(logs_all=lambda svc, **kw: "INFO nothing to see"),
     "no prev_randao value parsed", "fails_when_no_value_parses_out_of_the_logs"),
    # The header carries a prev_randao the deriver never produced on the assurance path.
    (lambda: dict(logs_all=lambda svc, **kw:
                  f"INFO {verdicts.ACTIVE_LINE} prev_randao={_blockmix(999)}"),
     "never logged by validator-0", "fails_when_a_header_value_was_never_logged"),
    # C1/C2. Everything before this proves the value reached the HEADER; only this proves it
    # reached EXECUTION, which is what a contract reading `block.prevrandao` gets.
    (lambda: dict(cast_send=_seq({"contractAddress": "0xprobe"},
                                 {"blockNumber": "0x80", "logs": [{"data": _mix("9")}]})),
     "did not reach EVM execution", "fails_when_the_evm_saw_a_different_prev_randao"),
    (lambda: dict(consensus_metrics=_seq(
        "beacon_digest_fallback_total 0\nbeacon_seed_active_total 7\n",
        "beacon_digest_fallback_total 2\nbeacon_seed_active_total 9\n")),
     "beacon_digest_fallback grew", "fails_when_the_digest_fallback_counter_grows"),
    (lambda: dict(consensus_metrics=_seq(
        "beacon_digest_fallback_total 0\nbeacon_seed_active_total 7\n")),
     "did not grow", "fails_when_the_seed_active_counter_is_flat"),
    # The shape a metric RENAME takes. It must not read as "the beacon is dead".
    (lambda: dict(consensus_metrics=_seq("some_other_total 1\n")),
     "beacon metrics absent", "fails_when_the_beacon_metrics_are_absent"),
    # `wait_nodes_have` — the follower lags the validators over devp2p, so a window issued the
    # instant the tip moves would race its catch-up. A follower that is genuinely stuck fails.
    (lambda: dict(mixhash_of=lambda svc, block, **kw:
                  "null" if svc == "full-node" else _mix("a")),
     "did not all reach block", "fails_when_a_node_never_gets_the_top_block"),
]


@pytest.mark.parametrize("world,match", [(w, m) for w, m, _ in _VRF_FAILURES],
                         ids=[i for _, _, i in _VRF_FAILURES])
def test_assert_vrf(monkeypatch, world, match):
    ctx, _ = _vrf_world(monkeypatch, **world())
    with pytest.raises(SmokeFailure, match=match):
        asserts.assert_vrf(ctx)


def test_assert_vrf_samples_only_epoch_2_and_later(monkeypatch):
    """The window floor. A sample below `epoch_start(2)` reads the DIGEST fallback, which is
    non-zero and node-agreed — it would pass every check and prove nothing."""
    seen = []
    ctx, _ = _vrf_world(monkeypatch, finalized_dec=lambda **kw: 130,
                        mixhash_of=lambda svc, block, **kw: (
                            seen.append(int(block)), _blockmix(block))[1])
    asserts.assert_vrf(ctx)
    assert min(seen) >= verdicts.beacon_active_epoch_start(64, 32)


# ── vrf-boundary ───────────────────────────────────────────────────────────

def _boundary_world(monkeypatch, **over):
    world = dict(
        wait_finalized_ge=lambda target, timeout: True,
        mixhash_of=lambda svc, block, **kw: _blockmix(block),
        dump_logs=lambda *a, **kw: None,
    )
    world.update(over)
    return _live_ctx(monkeypatch, **world)


def test_assert_vrf_boundary_passes_across_a_live_boundary(monkeypatch):
    ctx, _ = _boundary_world(monkeypatch)
    asserts.assert_vrf_boundary(ctx)


def test_assert_vrf_boundary_samples_both_sides_of_the_boundary(monkeypatch):
    """Continuity ACROSS the edge is the property. A window starting after the boundary proves
    only that the new epoch works, which is not what the case claims."""
    seen = []
    ctx, _ = _boundary_world(monkeypatch, mixhash_of=lambda svc, block, **kw: (
        seen.append(int(block)), _blockmix(block))[1])
    asserts.assert_vrf_boundary(ctx)
    boundary = verdicts.boundary_block(64, 32)
    assert min(seen) < boundary < max(seen)
    assert boundary in seen


def test_assert_vrf_boundary_fails_on_a_break_at_the_edge(monkeypatch):
    """The carry-forward failing looks exactly like this: the beacon drops to the digest fallback
    at the first block of the new epoch."""
    boundary = verdicts.boundary_block(64, 32)
    ctx, _ = _boundary_world(monkeypatch, mixhash_of=lambda svc, block, **kw:
                             ZERO if int(block) == boundary else
                             _blockmix(block))
    with pytest.raises(SmokeFailure, match="prev_randao is zero"):
        asserts.assert_vrf_boundary(ctx)


def test_assert_vrf_boundary_fails_when_nodes_diverge_at_the_edge(monkeypatch):
    boundary = verdicts.boundary_block(64, 32)
    ctx, _ = _boundary_world(monkeypatch, mixhash_of=lambda svc, block, **kw:
                             _mix("f") if (svc == "validator-2" and int(block) == boundary)
                             else _blockmix(block))
    with pytest.raises(SmokeFailure, match="disagree on prev_randao"):
        asserts.assert_vrf_boundary(ctx)


def test_assert_vrf_boundary_fails_if_the_chain_never_reaches_the_window(monkeypatch):
    ctx, _ = _boundary_world(monkeypatch, wait_finalized_ge=lambda t, to: False)
    with pytest.raises(SmokeFailure, match="did not reach finalized 168"):
        asserts.assert_vrf_boundary(ctx)
