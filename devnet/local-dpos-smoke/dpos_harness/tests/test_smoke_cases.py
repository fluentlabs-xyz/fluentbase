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

from dpos_harness.cases.smoke import (asserts, base, beacon, driver, epoch, tx, verdicts, vrf,
                                      vrf_boundary)
from dpos_harness.cases.smoke.driver import SmokeCtx, SmokeFailure
from dpos_harness.core import nodes, proc
from dpos_harness.core.exit_codes import RC_ERROR
from dpos_harness.core.proc import Runner
from dpos_harness.stack.profiles import StaticProfile
from dpos_harness.stack.static_stack import StaticStack

VALS = ["validator-0", "validator-1", "validator-2", "validator-3", "validator-4"]
#: FIVE, where the bash said four. `StaticProfile.committee()` carries the reason: a
#: four-seat stand cannot survive `smoke-byzantine`'s tombstone once the commit reverts
#: below MIN_COMMITTEE_LENGTH instead of carrying the previous committee forward.
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

    ONE DELIBERATE DELTA FROM THE BASH, and it is the last entry before the teardown: the
    `[step] battery` line is `driver.safety_sweep` — the invariant battery's four SAFETY detectors,
    run against the stand every case leaves behind (`driver.SAFETY_SWEEP_ENV` disarms it). Bash has
    no counterpart because bash never ran the battery from a case at all; that was the defect. It
    is a STEP and not a command: under `--dry-run` the sweep records itself and evaluates nothing,
    because a verdict computed over canned readings is meaningless in both directions.
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
        ["<step>", "battery"],
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


def test_a_failed_bring_up_is_torn_down_and_reported_as_an_ERROR(monkeypatch):
    """`StaticStack` tears itself down only on the CONVERGE paths (static_stack.py:153,277,317,
    341). The four lifecycle `run_checked` calls (:208,:231,:298,:312) raise `ProcError` with the
    containers still up — :298/:312 stop and start a running fleet — and this return sits ahead of
    the `try/finally` that owns teardown, so without the call here that stack outlives the case.
    A second `down` on a converge path is a best-effort `run_ok` and costs nothing.

    And it is an ERROR, not a FAIL: nothing was measured, so there is no false property to report.
    """
    calls = []
    monkeypatch.setattr(StaticStack, "tear_down", lambda self: calls.append("teardown"))
    monkeypatch.setattr(StaticStack, "bring_up_dpos",
                        lambda self: (_ for _ in ()).throw(
                            proc.ProcError(proc.RunResult(argv=["docker"], rc=1), "phase1-up")))
    assert driver.run("smoke-x", [lambda ctx: pytest.fail("ran an assertion")]) == RC_ERROR
    assert calls == ["teardown"]


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


# ══ the two fail-loud readers on SmokeCtx ══════════════════════════════════

def test_reading_splits_a_real_answer_into_hex_decimal_and_hash(monkeypatch):
    ctx, _ = _live_ctx(monkeypatch, check_external=lambda port, dry_value="": "0x80|0xdead")
    assert ctx.reading(8545, "v0 finalized", "smoke-x") == ("0x80", 128, "0xdead")


def test_reading_FAILS_LOUD_rather_than_reading_an_unreachable_node_as_height_0(monkeypatch):
    """`hex_to_dec("null")` is 0, and 0 is a usable height: a floor taken from an unreachable node
    is one the live chain passed minutes ago, so the check passes over a gap that never existed."""
    ctx, _ = _live_ctx(monkeypatch, check_external=lambda port, dry_value="": "null|null")
    with pytest.raises(SmokeFailure, match="refusing to read an unreachable node as height 0"):
        ctx.reading(8545, "v0 finalized", "smoke-x")


def test_logs_required_returns_the_log_when_the_daemon_answers(monkeypatch):
    ctx, _ = _live_ctx(monkeypatch, logs_all=lambda svc, dry_value="": "line one\nline two\n")
    assert ctx.logs_required("validator-0", "smoke-x", "panic sweep") == "line one\nline two\n"


def test_logs_required_FAILS_LOUD_on_an_unreadable_log(monkeypatch):
    """`logs_all` answers `""` on a timeout and on a daemon error, and empty text satisfies any
    "the line is not there" — an absence assertion that matches its own absence."""
    ctx, _ = _live_ctx(monkeypatch, logs_all=lambda svc, dry_value="": "   \n")
    with pytest.raises(SmokeFailure, match="returned nothing"):
        ctx.logs_required("validator-0", "smoke-x", "panic sweep")


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
        # One read per stand node BEFORE, one per node AFTER — five each since the stand grew.
        log_count=_seq(5, 5, 5, 5, 5, 6, 6, 6, 6, 6),
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

#: A healthy agreement-plane log for the boundary case's target epoch, as ONE node emits it. The
#: fields the parsers read (`epoch=`, `view=`, `pinned=`) are spelled exactly as the tracing writer
#: renders them.
_PLANE_EPOCH = asserts.BOUNDARY_TARGET_EPOCH
_PLANE_OK = "\n".join([
    f"INFO {verdicts.AGREE_REHYDRATE_LINE} entries=0",
    f"INFO {verdicts.AGREE_STARTED_LINE} epoch={_PLANE_EPOCH}",
    f"INFO {verdicts.AGREE_DECIDED_LINE} epoch={_PLANE_EPOCH} view=1 pinned=4",
    f"INFO {verdicts.AGREE_ADOPTED_LINE} epoch={_PLANE_EPOCH} pinned=4",
    f"INFO {verdicts.AGREE_SHARE_LINE} epoch={_PLANE_EPOCH}",
])


#: A QUIESCENT node's log: the artifact store rehydrated, and not one ceremony stage — what every
#: committee member emits for an epoch whose `dkgQual` bit is clear, i.e. every epoch on this
#: stand, whose committee never rotates.
_PLANE_QUIET = f"INFO {verdicts.AGREE_REHYDRATE_LINE} entries=0"


def _plane_logs(default=_PLANE_OK, **per_node):
    """`logs_required` stub: `default` for every node, overridden per service."""
    def read(svc, case, what, dry_value=""):
        return per_node.get(svc, default)
    return read


#: The two committee readings the plane branch is chosen from. `_COMMITTEE_B` rotates ONE seat out
#: of `_COMMITTEE_A`, which is exactly the diff `commitEpochCommittee` sets `dkgQual` from.
_COMMITTEE_A = "[" + ", ".join("0x" + c * 40 for c in "1234") + "]"
_COMMITTEE_B = "[" + ", ".join("0x" + c * 40 for c in "1235") + "]"


def _committees(mapping=None, default=_COMMITTEE_A):
    """`staking_call` stub answering `getEpochCommittee(<epoch>)` from `mapping` (epoch -> raw
    `cast` stdout) and `default` everywhere else."""
    mapping = {str(k): v for k, v in (mapping or {}).items()}

    def read(sig, *args, **kw):
        if sig != beacon.COMMITTEE_SIG:
            pytest.fail(f"the plane branch issued an unexpected staking call: {sig}")
        return mapping.get(str(args[0]), default)
    return read


#: committee[3] != committee[2] — a ceremony was due, so the four-stage story must be there.
_CHANGED = _committees({_PLANE_EPOCH: _COMMITTEE_B})
#: every epoch reads the same committee — no ceremony was due, carry-forward served the key.
_UNCHANGED = _committees()


#: Every family the plane observation parses, at zero — a healthy node's registry.
#:
#: IN THE SPELLING THE REGISTRY ACTUALLY RENDERS, which is the whole point of this fixture. The
#: first version wrote `f"{m} 0"` over the REGISTERED names — the same names the reader passes to
#: `gauge_val` — so it agreed with the reader by construction and every test below passed while
#: live run #2 reported all eight families unreadable on all four nodes. A metrics fixture written
#: in the reader's vocabulary tests nothing about the metric. See `_plane_metric_line`.
_PLANE_METRIC_ZERO = "0"


def _plane_metric_line(family: str, value: str = _PLANE_METRIC_ZERO) -> str:
    """One family as a commonware `:9100` scrape renders it: `# HELP`/`# TYPE` under the REGISTERED
    name, the sample under `nodes.counter_sample` of it (the doubled `_total`)."""
    return (f"# HELP {family} agreement-plane counter.\n"
            f"# TYPE {family} counter\n"
            f"{nodes.counter_sample(family)} {value}")


_PLANE_METRIC_TEXT = "\n".join(
    _plane_metric_line(m)
    for m in (verdicts.AGREE_REJECT_METRIC,) + verdicts.AGREE_REPORTED_METRICS)


def _plane_metrics(**per_node):
    """`node_metrics_text` stub: a clean registry for every node unless overridden by service.

    A per-service value REPLACES that node's whole registry text, so `""` is "the scrape said
    nothing" — the unreadable case `SmokeCtx.node_metrics_text` returns on a failed exec."""
    def read(svc, dry_value=""):
        return per_node.get(svc, _PLANE_METRIC_TEXT)
    return read


def _boundary_world(monkeypatch, **over):
    """A healthy boundary case on the COMMITTEE-CHANGED branch — the epoch's committee differs
    from its predecessor's, so the plane owes the full four-stage story."""
    world = dict(
        wait_finalized_ge=lambda target, timeout: True,
        mixhash_of=lambda svc, block, **kw: _blockmix(block),
        dump_logs=lambda *a, **kw: None,
        staking_call=_CHANGED,
        logs_required=_plane_logs(),
        node_metrics_text=_plane_metrics(),
    )
    world.update(over)
    return _live_ctx(monkeypatch, **world)


def _carry_world(monkeypatch, **over):
    """The same case on the CARRY-FORWARD branch — the committee is frozen (which is what the
    static stand actually does), so no ceremony was due and every node's log is quiescent."""
    world = dict(staking_call=_UNCHANGED, logs_required=_plane_logs(_PLANE_QUIET))
    world.update(over)
    return _boundary_world(monkeypatch, **world)


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


# ── vrf-boundary: the EPOCH-KEY AGREEMENT PLANE ────────────────────────────
#
# Every one of these is a world in which the plane is broken in exactly one way while the
# `prev_randao` window above stays PERFECTLY GREEN — which is the whole reason the observation
# exists. If any of them stopped raising, the case would be back to proving only that the beacon
# produced some output.

def _without(*lines):
    """`_PLANE_OK` with the given stage line(s) removed — one silent node."""
    return "\n".join(ln for ln in _PLANE_OK.splitlines()
                      if not any(m in ln for m in lines))


_PLANE_FAILURES = [
    (lambda: dict(logs_required=_plane_logs(**{"validator-2": _without(
        verdicts.AGREE_STARTED_LINE)})),
     "never logged 'instance started'", "a_node_opened_no_instance"),
    (lambda: dict(logs_required=_plane_logs(**{"validator-1": _without(
        verdicts.AGREE_DECIDED_LINE)})),
     "never logged 'set agreed'", "an_instance_ran_and_never_decided"),
    (lambda: dict(logs_required=_plane_logs(**{"validator-3": _without(
        verdicts.AGREE_ADOPTED_LINE)})),
     "never logged 'agreed set adopted'", "the_artifact_never_reached_the_ceremony"),
    (lambda: dict(logs_required=_plane_logs(**{"validator-0": _without(
        verdicts.AGREE_SHARE_LINE)})),
     r"never logged 'PK_epoch \+ share stored'", "the_ceremony_never_finished"),
    (lambda: dict(logs_required=_plane_logs(**{"validator-2": _without(
        verdicts.AGREE_REHYDRATE_LINE)})),
     "IN-MEMORY store", "the_artifact_store_is_not_durable"),
    # THE VIEW. A leader timeout on its own outlasts the pre-boundary window.
    (lambda: dict(logs_required=_plane_logs(**{
        "validator-1": _PLANE_OK.replace("view=1", "view=2")})),
     "DIFFERENT views", "nodes_decided_at_different_views"),
    (lambda: dict(logs_required=_plane_logs(**{
        svc: _PLANE_OK.replace("view=1", "view=2") for svc in beacon.COMMITTEE_NODES})),
     "agreed at view 2, not 1", "a_leader_timeout_was_paid"),
    (lambda: dict(logs_required=_plane_logs(**{
        svc: _PLANE_OK.replace(" view=1", "") for svc in beacon.COMMITTEE_NODES})),
     "carried no `view=` field", "the_view_field_disappeared"),
    # THE PINNED SET. Two sizes = two selections = two PK_epoch.
    (lambda: dict(logs_required=_plane_logs(**{
        "validator-3": _PLANE_OK.replace("pinned=4", "pinned=3")})),
     "DIFFERENT pinned-set sizes", "nodes_pinned_different_sets"),
    (lambda: dict(logs_required=_plane_logs(**{
        svc: _PLANE_OK.replace("pinned=4", "pinned=0") for svc in beacon.COMMITTEE_NODES})),
     "EMPTY pinned set", "the_agreed_set_is_empty"),
    # THE REJECTION COUNTER.
    (lambda: dict(node_metrics_text=_plane_metrics(**{
        "validator-0": _PLANE_METRIC_TEXT.replace(
            _plane_metric_line(verdicts.AGREE_REJECT_METRIC),
            _plane_metric_line(verdicts.AGREE_REJECT_METRIC, "1"))})),
     "non-zero", "a_served_artifact_was_refused_as_misbehaviour"),
    (lambda: dict(node_metrics_text=_plane_metrics(**{
        svc: "" for svc in beacon.COMMITTEE_NODES})),
     "unreadable on ALL 5", "the_rejection_counter_never_answered"),
    # A metric RENAME is the same reading as a dead scrape and must not read as "all clear".
    (lambda: dict(node_metrics_text=_plane_metrics(**{
        svc: "some_other_total 0" for svc in beacon.COMMITTEE_NODES})),
     "unreadable on ALL 5", "the_rejection_family_was_renamed"),
]


@pytest.mark.parametrize("world,match", [(w, m) for w, m, _ in _PLANE_FAILURES],
                         ids=[i for _, _, i in _PLANE_FAILURES])
def test_assert_vrf_boundary_fails_on_a_broken_agreement_plane(monkeypatch, world, match):
    ctx, _ = _boundary_world(monkeypatch, **world())
    with pytest.raises(SmokeFailure, match=match):
        asserts.assert_vrf_boundary(ctx)


def test_a_missing_stage_names_which_silence_the_plane_hit(monkeypatch):
    """The three silence lines are NOT verdicts (the plane retries, so any of them can appear on a
    run that converges) — they are the diagnostic that says WHICH silence a missing stage was. A
    failure that does not carry them sends the reader back to the raw logs."""
    broken = _without(verdicts.AGREE_DECIDED_LINE) + (
        f"\nWARN {verdicts.AGREE_BELOW_QUORUM_LINE} epoch={_PLANE_EPOCH}")
    ctx, _ = _boundary_world(monkeypatch,
                             logs_required=_plane_logs(**{"validator-1": broken}))
    with pytest.raises(SmokeFailure, match="below quorum, not proposing"):
        asserts.assert_vrf_boundary(ctx)


def test_a_missing_stage_with_no_silence_line_says_so_explicitly(monkeypatch):
    """The other half: a stage missing with none of the three silences logged is a DIFFERENT
    finding (the plane never named a reason), and the message must not read as if it had."""
    ctx, _ = _boundary_world(monkeypatch, logs_required=_plane_logs(
        **{"validator-1": _without(verdicts.AGREE_DECIDED_LINE)}))
    with pytest.raises(SmokeFailure, match="a reason the plane never logged"):
        asserts.assert_vrf_boundary(ctx)


def test_the_plane_scan_filters_by_epoch(monkeypatch):
    """Every stage line carries `epoch=<N>`. A node that ran the whole story for a DIFFERENT epoch
    has not run it for this one — and an unanchored grep would score it as if it had."""
    wrong = _PLANE_OK.replace(f"epoch={_PLANE_EPOCH}", f"epoch={_PLANE_EPOCH}0")
    ctx, _ = _boundary_world(monkeypatch, logs_required=_plane_logs(**{"validator-2": wrong}))
    with pytest.raises(SmokeFailure, match="never logged 'instance started'"):
        asserts.assert_vrf_boundary(ctx)


def test_the_plane_reads_the_committee_and_not_the_import_follower(monkeypatch):
    """`full-node` is not a `--dpos` node and runs no agreement instance, so including it would
    make every stage verdict a guaranteed red. The scan set is the committee."""
    seen = []
    ctx, _ = _boundary_world(monkeypatch, logs_required=lambda svc, case, what, dry_value="": (
        seen.append(svc), _PLANE_OK)[1])
    asserts.assert_vrf_boundary(ctx)
    assert seen == list(beacon.COMMITTEE_NODES)
    assert "full-node" not in seen


# ── vrf-boundary: the CARRY-FORWARD branch of the plane observation ────────
#
# A ceremony runs only where the committee CHANGED (`dkgQual[e] = committee[e] != committee[e-1]`,
# `staking-reader/reader.rs:139`). The static stand never rotates, so every one of its epochs takes
# the branch below — the four-stage demand above was a red on a healthy chain, the mirror of the
# false greens this observation exists to remove. Both branches assert, and both are driven here.


def test_the_plane_branch_is_read_from_the_two_committees(monkeypatch):
    """The branch must come from the CHAIN and never from "no stage line was logged" — inferring
    the absence of a ceremony from the absence of its logs would let the assertion agree with
    itself in both directions, which is what made the unconditional demand circular."""
    seen = []
    ctx, _ = _carry_world(monkeypatch, staking_call=lambda sig, *a, **kw: (
        seen.append((sig, a[0])), _COMMITTEE_A)[1])
    asserts.assert_vrf_boundary(ctx)
    assert seen == [(beacon.COMMITTEE_SIG, _PLANE_EPOCH - 1),
                    (beacon.COMMITTEE_SIG, _PLANE_EPOCH)]


def test_a_frozen_committee_passes_with_no_ceremony_at_all(monkeypatch):
    """The live shape of `smoke-base`: no instance started for the target epoch, and the boundary
    window shows the epoch was served a key anyway — carry-forward did its job."""
    ctx, _ = _carry_world(monkeypatch)
    asserts.assert_vrf_boundary(ctx)


def test_a_ceremony_on_an_unchanged_committee_FAILS(monkeypatch):
    """The carry branch's first red, and it is a real defect: `chain_key_epoch` never names a
    bit-clear epoch, so a node that ran an instance here holds a key the chain declines to serve —
    the plane's trigger has drifted off the on-chain committee diff."""
    ctx, _ = _carry_world(monkeypatch,
                          logs_required=_plane_logs(_PLANE_QUIET, **{"validator-2": _PLANE_OK}))
    with pytest.raises(SmokeFailure, match="ran an epoch-key ceremony for epoch 3"):
        asserts.assert_vrf_boundary(ctx)


def test_the_carry_branch_names_which_stages_it_saw(monkeypatch):
    """Which stage appeared says WHERE the trigger drifted — an instance that started and never
    decided is a different finding from a full ceremony that minted a declined key."""
    started_only = f"{_PLANE_QUIET}\nINFO {verdicts.AGREE_STARTED_LINE} epoch={_PLANE_EPOCH}"
    ctx, _ = _carry_world(monkeypatch,
                          logs_required=_plane_logs(_PLANE_QUIET,
                                                    **{"validator-1": started_only}))
    with pytest.raises(SmokeFailure, match="validator-1: instance started"):
        asserts.assert_vrf_boundary(ctx)


def test_the_carry_branch_FAILS_when_the_window_never_reaches_the_target_epoch(monkeypatch):
    """The carry branch's second red. Its key evidence is the boundary window this case already
    verified, and the window's geometry is computed independently of the target epoch — so a
    re-pointed target (or a drifted `BOUNDARY_HALF_WINDOW`) would otherwise let the branch report
    a green carried key from readings taken entirely in a DIFFERENT epoch."""
    monkeypatch.setattr(asserts, "BOUNDARY_TARGET_EPOCH", 5)
    ctx, _ = _carry_world(monkeypatch)
    with pytest.raises(SmokeFailure, match="does not reach into epoch 5"):
        asserts.assert_vrf_boundary(ctx)


def test_the_carry_branch_still_asserts_the_durable_artifact_store(monkeypatch):
    """The store is a property of the node's WIRING, not of this epoch's ceremony — an epoch that
    ran none is no reason to stop reading it, and its absence is silent by construction."""
    ctx, _ = _carry_world(monkeypatch, logs_required=_plane_logs(_PLANE_QUIET,
                                                                **{"validator-3": "INFO up"}))
    with pytest.raises(SmokeFailure, match="IN-MEMORY store"):
        asserts.assert_vrf_boundary(ctx)


# ══ the plane's eight families are read in the spelling the REGISTRY renders ══════════════

def test_the_plane_reads_the_DOUBLED_total_the_registry_renders(monkeypatch, capsys):
    """THE DEFECT LIVE RUN #2 FOUND, pinned in both halves.

    All eight plane families are `Counter`s whose REGISTERED name already ends in `_total`
    (`beacon/metrics.rs`), and the registry appends its own — so the scrape says `…_total_total`
    while `nodes.gauge_val` matches the name ANCHORED at its end. Reading them under the
    registered spelling returned "" from every node: the gating verdict reported
    `dpos_dkg_artifact_rejected_total` as "unreadable on ALL 4 node(s)" against four nodes that
    were exporting it, and the seven reported families all printed `na`.

    IT PASSED EVERY UNIT TEST AT THE TIME because the canned registry was written in the reader's
    own vocabulary — one bare `f"{family} 0"` line per family, which the anchored matcher happily
    answered for. This test uses registry text in the shape the registry actually produces
    (`# HELP`/`# TYPE` under the registered name, the sample under `counter_sample`), so it fails
    if the read path ever drops back to the registered spelling.

    The reported family is asserted too, and not only the gating one: all eight go through the
    SAME call, seven of them only print, and printing `na` forever is how the eighth's spelling
    bug stayed invisible until it happened to gate something.
    """
    reported = verdicts.AGREE_REPORTED_METRICS[0]
    scrape = "\n".join([
        _plane_metric_line(verdicts.AGREE_REJECT_METRIC, "0"),
        _plane_metric_line(reported, "7"),
    ])
    assert nodes.counter_sample(verdicts.AGREE_REJECT_METRIC) in scrape
    assert f"\n{verdicts.AGREE_REJECT_METRIC} " not in scrape, (
        "the fixture rendered the REGISTERED spelling as a sample line — it would agree with a "
        "reader that has the bug, which is exactly how this defect reached a live run")

    ctx, _ = _carry_world(monkeypatch, node_metrics_text=_plane_metrics(**{
        svc: scrape for svc in beacon.COMMITTEE_NODES}))
    asserts.assert_vrf_boundary(ctx)

    out = capsys.readouterr().out
    assert f"{verdicts.AGREE_REJECT_METRIC}=0 on 5/5 node(s)" in out, (
        "the gating counter was not EVALUATED over the registry the registry renders")
    assert f"{reported}: " in out and "=7" in out, (
        f"{reported} read as unavailable off a scrape that carries it — the seven "
        "reported families are read through the same call as the gating one")
    assert f"{reported}: validator-0=na" not in out


def test_the_dry_plane_registry_is_shaped_like_a_real_one():
    """The `--dry-run` canned registry must carry the SAMPLE spelling too. A dry fixture written in
    the reader's vocabulary walks the happy branch of `_assert_artifact_rejections` no matter what
    the reader passes, which is a transcript that proves nothing about the metric half."""
    for m in (verdicts.AGREE_REJECT_METRIC,) + verdicts.AGREE_REPORTED_METRICS:
        assert f"{nodes.counter_sample(m)} 0" in beacon._DRY_PLANE_METRICS
        assert nodes.gauge_val(beacon._DRY_PLANE_METRICS, nodes.counter_sample(m)) == "0"
        assert nodes.gauge_val(beacon._DRY_PLANE_METRICS, m) == ""


def test_the_carry_branch_still_asserts_the_rejection_counter(monkeypatch):
    """Same reason: an epoch that agreed nothing can still be SERVED (and refuse) an artifact for
    another one, and a rejection is a peer excluded with no way back."""
    ctx, _ = _carry_world(monkeypatch, node_metrics_text=_plane_metrics(**{
        "validator-0": _PLANE_METRIC_TEXT.replace(
            _plane_metric_line(verdicts.AGREE_REJECT_METRIC),
            _plane_metric_line(verdicts.AGREE_REJECT_METRIC, "1"))}))
    with pytest.raises(SmokeFailure, match="non-zero"):
        asserts.assert_vrf_boundary(ctx)


@pytest.mark.parametrize("reading,match", [
    ("", "returned NOTHING"),
    ("[]", "is EMPTY"),
    ("0x" + "1" * 40, "did not decode as an address array"),
])
def test_an_unreadable_committee_picks_NEITHER_branch(monkeypatch, reading, match):
    """Both branches rest on the committee diff, so a committee nobody could read leaves both
    unfounded. Guessing one would assert a ceremony — or its absence — over nothing."""
    ctx, _ = _carry_world(monkeypatch,
                          staking_call=_committees({_PLANE_EPOCH: reading}))
    with pytest.raises(SmokeFailure, match=match):
        asserts.assert_vrf_boundary(ctx)


def test_a_changed_committee_still_demands_the_whole_ceremony(monkeypatch):
    """The routing, from the other side: with committee[3] != committee[2] a quiescent plane is a
    missing key, not a carry-forward, and must fail on the FIRST stage."""
    ctx, _ = _boundary_world(monkeypatch, logs_required=_plane_logs(_PLANE_QUIET))
    with pytest.raises(SmokeFailure, match="never logged 'instance started'"):
        asserts.assert_vrf_boundary(ctx)
