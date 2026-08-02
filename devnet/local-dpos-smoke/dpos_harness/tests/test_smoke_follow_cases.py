"""`cases/smoke/` — the three ported FOLLOWER drivers, against `scripts/case-cert-follow.sh`,
`case-cert-cascade.sh` and `case-tx-cascade.sh`.

Three kinds of evidence, as in the two sibling files, plus one that is new to this chunk:

  * THE COMMAND TRANSCRIPT (`--dry-run`) — argv for argv against the bash, IN ORDER. These are
    the first cases whose commands carry per-case `-f` overlay flags and an interpolated
    environment, so the transcript is also where a compose file list or an `export` that arrived
    too late becomes visible.
  * THE WIRING (stubbed readers on the LIVE branches) — a body that reads the right things and
    forgets to apply the verdict passes every transcript test and fails here.
  * THE NEGATIVES, DRIVEN — for each of the three cases, a world in which the forbidden thing
    HAPPENS. `test_smoke_follow_verdicts.py` proves the decisions are right; these prove the
    bodies ask them, on the readings that matter, of the right node.
  * THE SKIP PATH — `cert-follow` phase 3 skips loudly when the MITM sidecar cannot start. That
    branch must skip and say so, and must NOT start the tamper follower; a case that silently
    passed because it could not run is the failure mode this whole project is about.
"""

from __future__ import annotations

import pytest

from dpos_harness.cases.smoke import (asserts_follow, cert_cascade, cert_follow, driver,
                                      tx_cascade, verdicts, verdicts_follow as vf)
from dpos_harness.cases.smoke.driver import SmokeCtx, SmokeFailure
from dpos_harness.core.proc import Runner
from dpos_harness.stack.profiles import StaticProfile
from dpos_harness.stack.static_stack import StaticStack

VALS = ["validator-0", "validator-1", "validator-2", "validator-3"]
UP = ["docker", "compose", "up", "--build", "-d"]
STOP = ["docker", "compose", "stop", "--timeout", "40", *VALS]
DOWN = ["docker", "compose", "down", "-v", "--remove-orphans"]
RPC = "http://localhost:8545"
KEY = "0x" + "00" * 32
MOCK = "0x" + "11" * 20
BOGUS = "0x" + "22" * 20
L3_RPC = "http://localhost:28545"
SENTRY_RPC = "http://localhost:38545"


def _files(overlay):
    """The `-f` array bash keeps in a per-case variable (`CF_COMPOSE`, `CC_COMPOSE`, …)."""
    return ["-f", "docker-compose.yml", "-f", "docker-compose.dpos.yml", "-f", overlay]


def _compose(overlay, *tail):
    return ["docker", "compose", *_files(overlay), *tail]


def _recreate(overlay):
    return _compose(overlay, "up", "-d", "--force-recreate", *VALS)


@pytest.fixture(autouse=True)
def _clean_env(monkeypatch):
    for k in ("EPOCH_INTERVAL", "DPOS_ACTIVATION_BLOCK", "DPOS_EXTRA_COMPOSE",
              "DPOS_CONVERGE_EXCLUDE", "SIM_DATA_ROOT", "SMOKE_KEEP_UP", "RPC", "CHAIN_ID"):
        monkeypatch.delenv(k, raising=False)


def _dry(case_module, argv=("--dry-run",)):
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
    """The real commands, with the in-process `<step>` markers dropped — the argv oracle."""
    return [inv.argv for inv in runner.log if inv.kind != "step"]


def _flat(runner):
    """The WHOLE transcript in order: a command as its first three argv tokens, a `<step>` as its
    detail. Ordering assertions read this, because for these three cases the interleaving of
    reads and service starts is half the meaning."""
    return [inv.note if inv.kind == "step" else " ".join(inv.argv[:3]) for inv in runner.log]


def _seq(runner):
    """The whole transcript as FULL strings — a command joined, a `<step>` as its detail.

    `_flat`'s three-token form cannot separate these cases' commands: every overlay command begins
    `docker compose -f`, so the phase-1 `up`, the stop, the start and the tamper `up` are all the
    same string there. Ordering assertions read this instead."""
    return [inv.note if inv.kind == "step" else " ".join(inv.argv) for inv in runner.log]


def _idx(seq, needle, after=-1):
    """The index of the first entry containing `needle` strictly after `after`."""
    return next(i for i, s in enumerate(seq) if needle in s and i > after)


def _envs(runner, needle):
    """The env DELTA of every command whose argv contains `needle` — bash's `export`, made
    visible. `argvs()` cannot see this, so an export that arrived after the `up` that consumed it
    would be invisible to every other assertion in the file."""
    return [inv.env for inv in runner.log if needle in inv.argv]


def _live_ctx(monkeypatch, overlay, **stubs):
    """A `SmokeCtx` on its LIVE branches with every reader stubbed.

    Same construction as the two sibling files: the Runner stays dry (writes are recorded, nothing
    spawns) while `dry` reports False, so `check` raises and `poll` really loops. The CLOCK is
    fast-forwarded rather than the constants shrunk — two of these cases pass BY timeout, and a
    test that shortened the real budgets would leave somebody free to shorten them for real."""
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
    stack = StaticStack(profile=StaticProfile(extra_overlays=(overlay,)), runner=runner)
    stack.prev_fin = "0x50"
    ctx = SmokeCtx(stack)
    monkeypatch.setattr(SmokeCtx, "dry", property(lambda self: False))
    for name, value in stubs.items():
        monkeypatch.setattr(ctx, name, value)
    return ctx, runner


# ══ the command transcripts ════════════════════════════════════════════════

def test_cert_follow_transcript_is_bash_faithful():
    """`case-cert-follow.sh:35,47,58,66,84`, argv for argv:

        :23  CF_COMPOSE=(-f docker-compose.yml -f docker-compose.dpos.yml -f …cert-follow.yml)
        :35  docker compose "${CF_COMPOSE[@]}" up -d cert-follower
        :47  docker compose "${CF_COMPOSE[@]}" stop --timeout 40 cert-follower
        :58  docker compose "${CF_COMPOSE[@]}" start cert-follower
        :66  docker compose "${CF_COMPOSE[@]}" up -d cert-mitm
        :84  docker compose "${CF_COMPOSE[@]}" up -d cert-follower-tamper

    Everything else the case does is a READ — RPC over urllib or a delegated `core/nodes` reader —
    and is recorded as a marker, not as a process (`core/rpc.py`'s PORT-NOTE)."""
    ov = asserts_follow.CERT_FOLLOW_OVERLAY
    rc, r = _dry(cert_follow)
    assert rc == 0
    assert _cmds(r) == [
        UP, STOP, _recreate(ov),
        _compose(ov, "up", "-d", "cert-follower"),
        _compose(ov, "stop", "--timeout", "40", "cert-follower"),
        _compose(ov, "start", "cert-follower"),
        _compose(ov, "up", "-d", "cert-mitm"),
        _compose(ov, "up", "-d", "cert-follower-tamper"),
        DOWN,
    ]


def test_cert_follow_order_stop_read_restart_is_the_gap_backfill():
    """Phase 2 IS an order: read the follower's height, stop it, let v0 run a full epoch past
    that height, restart, then require alignment. Reading the height after the stop would read a
    dead RPC; restarting before v0 advanced would back-fill an empty gap and pass on a follower
    that cannot back-fill at all."""
    seq = _seq(_dry(cert_follow)[1])
    read_f1 = _idx(seq, "check_external(28545)")
    stop = _idx(seq, "stop --timeout 40 cert-follower", read_f1)
    advance = _idx(seq, "wait_finalized_ge(", stop)
    restart = _idx(seq, "start cert-follower", advance)
    align = _idx(seq, "wait_follower_align(28545", restart)
    assert read_f1 < stop < advance < restart < align


def test_cert_follow_observes_for_the_full_window_between_the_two_v0_reads():
    """The tamper assertion is `v0 advanced AND the follower did not`, measured across ONE
    window. The 45 s sleep must sit BETWEEN the two producer reads and BEFORE the follower read,
    or the case is comparing readings that do not bracket anything."""
    seq = _seq(_dry(cert_follow)[1])
    tamper_up = _idx(seq, "up -d cert-follower-tamper")
    sleep = _idx(seq, f"{vf.TAMPER_OBSERVE_S}s", tamper_up)
    fins = [i for i, s in enumerate(seq) if s == "finalized_dec()"]
    tamper_read = _idx(seq, "check_external(38545)")
    before = [i for i in fins if i < sleep][-1]
    after = [i for i in fins if i > sleep][0]
    assert before < tamper_up < sleep < after < tamper_read


def test_cert_cascade_transcript_is_bash_faithful():
    """`case-cert-cascade.sh:31,36,40,53,71,85,88,100`, argv for argv. The two `cast send
    --create`s are separate deploys ON PURPOSE — phase 3 needs a SECOND MockRollup, because
    overwriting batch 1 of the first would also break the tier-1 and tier-2 followers still
    reading it."""
    ov = asserts_follow.CERT_CASCADE_OVERLAY
    code = asserts_follow._mock_rollup_bytecode(None, "t")
    send = ["cast", "send", "--private-key", KEY, "--rpc-url", RPC]
    rc, r = _dry(cert_cascade)
    assert rc == 0
    assert _cmds(r) == [
        UP, STOP, _recreate(ov),
        ["docker", "compose", "exec", "-T", "validator-0", "cat", "/runtime/keys/funded.hex"],
        send + ["--create", code, "--json"],
        send + [MOCK, vf.SET_CHECKPOINT_SIG, "1", "0x" + "ab" * 32, "--json"],
        _compose(ov, "up", "-d", "cert-follower-l1"),
        _compose(ov, "up", "-d", "cert-follower-tier2"),
        send + ["--create", code, "--json"],
        send + [BOGUS, vf.SET_CHECKPOINT_SIG, "1", vf.BOGUS_CHECKPOINT_HASH, "--json"],
        _compose(ov, "up", "-d", "cert-follower-l1-bogus"),
        DOWN,
    ]


def test_cert_cascade_exports_each_rollup_address_before_the_up_that_needs_it():
    """bash `export MOCK_ROLLUP_ADDR` at `:50`, before `:53`'s `up`. The compose file interpolates
    it (`--cert-follow.l1-rollup-address=${MOCK_ROLLUP_ADDR:-0x0…0}`), so an export that arrives
    after the `up` leaves the follower pointed at the ZERO address and the case then measures a
    node whose trust root is not there — a phase-1 failure that reads like a product bug."""
    _, r = _dry(cert_cascade)
    assert _envs(r, "cert-follower-l1") == [{vf.MOCK_ROLLUP_ENV: MOCK}]
    assert _envs(r, "cert-follower-tier2") == [{vf.MOCK_ROLLUP_ENV: MOCK}]
    # The bogus follower needs BOTH: bash `export`s are cumulative and the compose file still
    # interpolates MOCK_ROLLUP_ADDR into the tier-1 service definition it is parsing.
    assert _envs(r, "cert-follower-l1-bogus") == [
        {vf.MOCK_ROLLUP_ENV: MOCK, vf.BOGUS_ROLLUP_ENV: BOGUS}]


def test_cert_cascade_waits_for_each_checkpoint_to_FINALIZE_before_starting_its_follower():
    """The follower reads the Rollup at the FINALIZED tag (two-tier lag = K blocks). Starting it
    on a checkpoint that is merely MINED means it looks up a batch that, from its point of view,
    has not been written — a fail-closed follower would then refuse the honest trust root."""
    seq = _seq(_dry(cert_cascade)[1])
    waits = [i for i, s in enumerate(seq) if s.startswith("wait_finalized_ge(")]
    honest_cp = _idx(seq, f"{vf.SET_CHECKPOINT_SIG} 1 0x" + "ab" * 32)
    bogus_cp = _idx(seq, f"{vf.SET_CHECKPOINT_SIG} 1 {vf.BOGUS_CHECKPOINT_HASH}")
    tier1_up = next(i for i, s in enumerate(seq) if s.endswith("up -d cert-follower-l1"))
    bogus_up = _idx(seq, "up -d cert-follower-l1-bogus")
    assert len(waits) == 2, "each of the two checkpoints gets its own finality wait"
    assert honest_cp < waits[0] < tier1_up
    assert bogus_cp < waits[1] < bogus_up


def test_cert_cascade_reads_the_l1_verified_line_only_after_tier1_aligned():
    """Alignment first, then the trust-root grep. Grepping before the follower has done anything
    would read an empty log and fail a correct node."""
    flat = _flat(_dry(cert_cascade)[1])
    align = flat.index("wait_follower_align(28545, > 32, <= 240s)")
    grep = flat.index("overlay_logs(cert-follower-l1, tail=None)")
    assert align < grep


def test_tx_cascade_transcript_is_bash_faithful():
    """`case-tx-cascade.sh:35,53,56,75,86-107,126-140`, argv for argv. Note what is NOT here: the
    two L3 submissions carry `--async` and NO `--json` (bash reads the bare hash off stdout), and
    the receipt reads are issued against TWO different nodes."""
    ov = asserts_follow.TX_CASCADE_OVERLAY
    pk, pk3 = "ab" * 64, "cd" * 64
    dead, blend = verdicts.DEAD_ADDR, verdicts.BLEND_ADDR
    common = ["--private-key", KEY, "--rpc-url", L3_RPC, "--chain", "2026"]
    rc, r = _dry(tx_cascade)
    assert rc == 0
    assert _cmds(r) == [
        UP, STOP, _recreate(ov),
        _compose(ov, "up", "-d", "sentry"),
        _compose(ov, "exec", "-T", "sentry", "sh", "-c",
                 f"printf '%s' 'enode://{pk}@172.20.0.30:30303' > /runtime/sentry-enode.txt"),
        _compose(ov, "up", "-d", "downstream"),
        ["cast", "rpc", "--rpc-url", SENTRY_RPC, "admin_addTrustedPeer",
         f"enode://{pk3}@172.20.0.31:30303"],
        ["cast", "rpc", "--rpc-url", L3_RPC, "net_peerCount"],
        ["docker", "compose", "exec", "-T", "validator-0", "cat", "/runtime/keys/funded.hex"],
        ["cast", "wallet", "address", "--private-key", KEY],
        ["cast", "balance", dead, "--rpc-url", L3_RPC],
        ["cast", "nonce", "0x" + "11" * 20, "--rpc-url", L3_RPC],
        ["cast", "send", "--async", "--nonce", "7", *common, dead, "--value", "0.05ether"],
        ["cast", "send", "--async", "--nonce", "8", *common, blend, "approve(address,uint256)",
         dead, "4242"],
        ["cast", "receipt", "0xvalue", "--rpc-url", RPC, "--json"],
        ["cast", "receipt", "0xapprove", "--rpc-url", RPC, "--json"],
        ["cast", "receipt", "0xvalue", "--rpc-url", L3_RPC, "--json"],
        ["cast", "balance", dead, "--rpc-url", L3_RPC],
        ["cast", "call", blend, "allowance(address,address)(uint256)", "0x" + "11" * 20, dead,
         "--rpc-url", L3_RPC],
        DOWN,
    ]


def test_tx_cascade_reads_the_receipts_off_the_PRODUCER_and_the_round_trip_off_L3():
    """The node each read is issued against IS the assertion. A receipt on validator-0 — which L3
    cannot reach — proves the tx entered a hidden proposer's pool; the same read on L3 proves only
    that L3 has a mempool. Collapsing both onto one RPC deletes the case."""
    _, r = _dry(tx_cascade)
    receipts = [inv.argv for inv in r.log if inv.argv[:2] == ["cast", "receipt"]]
    assert [a[4] for a in receipts] == [RPC, RPC, L3_RPC]
    balances = [inv.argv for inv in r.log if inv.argv[:2] == ["cast", "balance"]]
    assert all(a[-1] == L3_RPC for a in balances), "the delta is L3's own view or it proves nothing"


def test_tx_cascade_writes_the_sentry_enode_before_starting_L3():
    """L3's `--trusted-peers` is read from `/runtime/sentry-enode.txt` AT BOOT. Writing it after
    the `up` leaves a downstream that came up with an empty trusted-peer list, `--trusted-only`
    and `--disable-discovery` — permanently peerless, and the case then blames tx gossip."""
    seq = _seq(_dry(tx_cascade)[1])
    read_pk = _idx(seq, "enode_pubkey(http://localhost:38545)")
    write = _idx(seq, "/runtime/sentry-enode.txt", read_pk)
    up_l3 = _idx(seq, "up -d downstream", write)
    assert read_pk < write < up_l3


def test_tx_cascade_balance_is_read_before_and_after_the_sends():
    """The assertion is a DELTA. The burn address holds a nonzero balance from the first run
    onward, so an "after" with no "before" would pass on a chain that moved nothing."""
    flat = _flat(_dry(tx_cascade)[1])
    want = f"cast balance {verdicts.DEAD_ADDR}"
    balances = [i for i, s in enumerate(flat) if s == want]
    sends = [i for i, s in enumerate(flat) if s == "cast send --async"]
    assert len(balances) == 2 and len(sends) == 2
    assert balances[0] < sends[0] and balances[1] > sends[-1]


def test_the_overlay_list_has_exactly_one_home():
    """Every per-case compose command derives its `-f` array from the PROFILE, so a case cannot
    grow a private, drifting copy. The recreate carries the overlay too — that is the mechanism
    `case-byzantine` uses, and it creates none of the overlay's services because the recreate
    names its services explicitly."""
    for module, overlay in ((cert_follow, asserts_follow.CERT_FOLLOW_OVERLAY),
                            (cert_cascade, asserts_follow.CERT_CASCADE_OVERLAY),
                            (tx_cascade, asserts_follow.TX_CASCADE_OVERLAY)):
        _, r = _dry(module)
        for argv in _cmds(r):
            if argv[:2] == ["docker", "compose"] and "-f" in argv:
                assert argv[2:8] == _files(overlay), argv


def test_the_teardown_stays_bare_and_reaps_the_overlay_containers_as_orphans():
    """`--remove-orphans` on a BARE `down` is what removes the follower/sentry containers: they
    are not in `docker-compose.yml`'s project config, so they are orphans of it. Naming the
    overlay here would be a different teardown, and dropping the flag would leave every one of
    these containers running into the next case."""
    for module in (cert_follow, cert_cascade, tx_cascade):
        assert _cmds(_dry(module)[1])[-1] == DOWN


# ══ the wiring: cert-follow ════════════════════════════════════════════════

def _cf_world(**over):
    """A healthy cert-follow world: the producer advances, the follower aligns, the MITM comes up
    and the tamper follower finalizes nothing."""
    fin = iter([100, 100, 100, 140])
    world = dict(
        finalized_dec=lambda dry_value=0: next(fin, 140),
        wait_follower_align=lambda *a, **k: "0x8c|0xaa",
        wait_finalized_ge=lambda *a, **k: True,
        check_external=lambda port, dry_value="": ("0x64|0xaa" if port == vf.CF_PORT
                                                   else "null|null"),
        shutdown_flushed=lambda *a, **k: True,
        overlay_logs=lambda *svcs, **k: (vf.MITM_READY_LINE if svcs[0] == "cert-mitm" else ""),
        sleep=lambda _s: None,
    )
    world.update(over)
    return world


def test_cert_follow_passes_on_a_healthy_world(monkeypatch, capsys):
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY, **_cf_world())
    asserts_follow.assert_cert_follow(ctx)
    out = capsys.readouterr().out
    assert "OK (phase 1 subscribe-align)" in out
    assert "OK (phase 2 gap back-fill)" in out
    assert "tampered-cert rejection all verified" in out


def test_cert_follow_FAILS_when_the_tamper_follower_finalized_anything(monkeypatch):
    """THE NEGATIVE, driven through the BODY. A tamper follower reading a real height means the
    driver accepted a forged certificate. The body must read port 38545 and fail on it — a live
    run never walks this branch, because producing it needs a Byzantine build."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(check_external=lambda port, dry_value="":
                                   "0x140|0xbb" if port == vf.TAMPER_PORT else "0x64|0xaa"))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "verification is NOT load-bearing" in e.value.message


def test_cert_follow_FAILS_when_v0_stalled_rather_than_crediting_the_follower(monkeypatch):
    """The control. With the producer frozen, the follower's stillness proves nothing, and the
    case must say so instead of reporting a passing negative assertion."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(finalized_dec=lambda dry_value=0: 100))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "v0 stalled during tamper phase" in e.value.message


def test_cert_follow_SKIPS_loudly_and_starts_no_tamper_follower_without_the_mitm(
        monkeypatch, capsys):
    """The offline-pip path (`case-cert-follow.sh:78`). It must print SKIP with its reason, keep
    the exit status of the two positive phases, and NOT start the tamper follower — a tamper
    follower fed by a proxy that never came up would make no progress for the wrong reason and
    the negative would pass vacuously."""
    ctx, runner = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                            **_cf_world(overlay_logs=lambda *svcs, **k: ""))
    asserts_follow.assert_cert_follow(ctx)
    out = capsys.readouterr().out
    assert "SKIP (phase 3 tampered-reject)" in out and "offline pip" in out
    assert "subscribe-align + gap back-fill verified" in out
    assert "tampered-cert rejection all verified" not in out
    started = [" ".join(inv.argv) for inv in runner.log]
    assert not any("cert-follower-tamper" in s for s in started)


def test_cert_follow_FAILS_LOUD_rather_than_reading_an_unreachable_follower_as_height_0(
        monkeypatch):
    """§2.4 item 5 at the one place in this case where a reading is used as DATA. `hex_to_dec`
    maps the `null` sentinel to 0, and 0 is a usable height: the back-fill target would become
    block 33, which a live chain passed minutes ago, so phase 2 would pass over a gap that was
    never created."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(check_external=lambda port, dry_value="": "null|null"))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "refusing to read an unreachable node as height 0" in e.value.message


def test_cert_cascade_FAILS_LOUD_rather_than_pushing_a_null_checkpoint_hash(monkeypatch):
    """The same guard on the other side: `"null"` is not a bytes32, and letting it through would
    fail inside `cast` three commands later with a message about ABI encoding."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_CASCADE_OVERLAY,
                       **_cc_world(check_external=lambda port, dry_value="": "null|null"))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_cascade(ctx)
    assert "refusing to read an unreachable node as height 0" in e.value.message


def test_cert_follow_FAILS_when_the_follower_never_aligns(monkeypatch):
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(wait_follower_align=lambda *a, **k: None))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "did not align with v0 past" in e.value.message


def test_cert_follow_only_warns_when_the_follower_did_not_flush(monkeypatch, capsys):
    """`shutdown_flushed` resolves the container through the BARE compose project, which does not
    define `cert-follower` at all — so it reports "not clean" for a follower that exited
    perfectly. Bash warns and continues; failing here would fail the case on the reader."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(shutdown_flushed=lambda *a, **k: False))
    asserts_follow.assert_cert_follow(ctx)
    assert "(warning) cert-follower did not exit cleanly" in capsys.readouterr().out


# ══ the wiring: cert-cascade ═══════════════════════════════════════════════

def _cc_world(**over):
    world = dict(
        finalized_dec=lambda dry_value=0: 100,
        funded_key=lambda **k: "00" * 32,
        cast_send=lambda tail, note, dry_value="{}": (
            {"contractAddress": MOCK if note == "cc-deploy-mock-rollup" else BOGUS}
            if "--create" in [str(t) for t in tail] else {"blockNumber": "0x80"}),
        check_external=lambda port, dry_value="": ("0x80|0x" + "ab" * 32
                                                   if port == 8545 else "null|null"),
        wait_finalized_ge=lambda *a, **k: True,
        wait_follower_align=lambda *a, **k: "0x8c|0xaa",
        overlay_logs=lambda *svcs, **k: (vf.L1_VERIFIED_LINE if svcs[0] == "cert-follower-l1"
                                         else vf.BOGUS_REJECT_LINE),
        overlay_ps_state=lambda *a, **k: "running",
    )
    world.update(over)
    return world


def test_cert_cascade_passes_on_a_healthy_world(monkeypatch, capsys):
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_CASCADE_OVERLAY, **_cc_world())
    asserts_follow.assert_cert_cascade(ctx)
    out = capsys.readouterr().out
    assert "OK (phase 1 L1-checkpoint align)" in out
    assert "OK (phase 2 cascade)" in out
    assert "fail-closed reject all verified" in out


def test_cert_cascade_FAILS_when_the_bogus_follower_never_refuses(monkeypatch):
    """THE NEGATIVE, driven. A running container with a clean log is a trust root that failed
    OPEN. The body must exhaust its budget and fail rather than fall through."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_CASCADE_OVERLAY,
                       **_cc_world(overlay_logs=lambda *svcs, **k:
                                   vf.L1_VERIFIED_LINE if svcs[0] == "cert-follower-l1" else ""))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_cascade(ctx)
    assert e.value.message == vf.BOGUS_NOT_REFUSED


def test_cert_cascade_accepts_an_EXIT_as_the_refusal(monkeypatch):
    """The second witness: the node may refuse and die before its log can be read."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_CASCADE_OVERLAY,
                       **_cc_world(overlay_logs=lambda *svcs, **k:
                                   vf.L1_VERIFIED_LINE if svcs[0] == "cert-follower-l1" else "",
                                   overlay_ps_state=lambda *a, **k: "exited"))
    asserts_follow.assert_cert_cascade(ctx)


def test_cert_cascade_FAILS_when_the_bogus_follower_refused_and_followed_anyway(monkeypatch):
    """THE SECOND HALF of the negative. Logging the refusal and then finalizing past the anchor is
    the fail-open the log grep alone cannot see."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_CASCADE_OVERLAY,
                       **_cc_world(check_external=lambda port, dry_value="":
                                   {8545: "0x80|0x" + "ab" * 32,
                                    vf.BOGUS_PORT: "0xc8|0xbb"}.get(port, "null|null")))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_cascade(ctx)
    assert "made finalized progress" in e.value.message


def test_cert_cascade_FAILS_when_tier1_aligned_without_running_the_L1_assert(monkeypatch):
    """Alignment off the cert feed does not imply the L1 trust root was consulted — that is the
    whole difference between this case's phase 1 and cert-follow's."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_CASCADE_OVERLAY,
                       **_cc_world(overlay_logs=lambda *svcs, **k: "cert applied at 131"))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_cascade(ctx)
    assert "the L1 checkpoint assert never ran" in e.value.message


def test_cert_cascade_FAILS_LOUD_when_a_deploy_returns_no_address(monkeypatch):
    """Otherwise the compose file's `:-0x0…0` default takes over and the case measures a follower
    pointed at the zero address."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_CASCADE_OVERLAY,
                       **_cc_world(cast_send=lambda tail, note, dry_value="{}": {}))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_cascade(ctx)
    assert "no contractAddress" in e.value.message


def test_cert_cascade_pushes_the_hash_it_READ_not_a_constant(monkeypatch):
    """The honest checkpoint must be a block hash that IS in the chain — that is what makes the
    tier-1 verification a real read rather than a formality. Pinned because the bogus path uses a
    literal, and using the literal in both places would make phase 1 assert nothing."""
    sends = []
    world = _cc_world()
    real = world["cast_send"]
    world["cast_send"] = lambda tail, note, dry_value="{}": (sends.append(list(tail))
                                                             or real(tail, note))
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_CASCADE_OVERLAY, **world)
    asserts_follow.assert_cert_cascade(ctx)
    honest = [t for t in sends if vf.SET_CHECKPOINT_SIG in t][0]
    assert honest[-1] == "0x" + "ab" * 32
    bogus = [t for t in sends if vf.SET_CHECKPOINT_SIG in t][1]
    assert bogus[-1] == vf.BOGUS_CHECKPOINT_HASH


# ══ the wiring: tx-cascade ═════════════════════════════════════════════════

def _txc_world(**over):
    world = dict(
        wait_follower_align=lambda *a, **k: "0x8c|0xaa",
        wait_finalized_ge=lambda *a, **k: True,
        enode_pubkey=lambda url, dry_value="": ("ab" * 64 if url == SENTRY_RPC else "cd" * 64),
        cast_rpc=lambda method, *a, **k: '"0x1"',
        funded_key=lambda **k: "00" * 32,
        wallet_address=lambda key, **k: "0x" + "11" * 20,
        cast_nonce=lambda addr, **k: "7",
        cast_send_async=lambda tail, note, dry_value="": (
            "0xvalue" if note == "txc-l3-value-transfer" else "0xapprove"),
        cast_receipt=lambda h, **k: {"status": "0x1", "blockNumber": "0x80"},
        cast_balance=lambda addr, **k: "0",
        cast_call=lambda *a, **k: str(vf.TXC_ALLOW),
        overlay_logs=lambda *svcs, **k: "tx-route ok peers=1",
    )
    world.update(over)
    return world


def _balances(*values):
    box = list(values)

    def read(addr, **k):
        return str(box.pop(0)) if len(box) > 1 else str(box[0])
    return read


def test_tx_cascade_passes_on_a_healthy_world(monkeypatch, capsys):
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.TX_CASCADE_OVERLAY,
                       **_txc_world(cast_balance=_balances(0, vf.TXC_TRANSFER_WEI)))
    asserts_follow.assert_tx_cascade(ctx)
    out = capsys.readouterr().out
    assert "relayed via devp2p tx-gossip to a hidden validator" in out
    assert "L3 devp2p peers=1" in out


def test_tx_cascade_FAILS_when_L3_has_no_devp2p_peer(monkeypatch):
    """The HARD half of the peer check: with no uplink the write path cannot be measured at all,
    so the case must stop here rather than fail 90 receipt polls later with an unrelated message."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.TX_CASCADE_OVERLAY,
                       **_txc_world(cast_rpc=lambda *a, **k: '"0x0"'))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_tx_cascade(ctx)
    assert "L3 has NO devp2p peer" in e.value.message


def test_tx_cascade_only_NOTES_a_second_peer_and_keeps_going(monkeypatch, capsys):
    """The SOFT half, kept soft — bash prints a note and continues. Promoting it would make the
    case flaky on a transient connection without adding an assertion the topology does not
    already give (`--trusted-only` + a one-entry trusted-peers list)."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.TX_CASCADE_OVERLAY,
                       **_txc_world(cast_rpc=lambda *a, **k: '"0x2"',
                                    cast_balance=_balances(0, vf.TXC_TRANSFER_WEI)))
    asserts_follow.assert_tx_cascade(ctx)
    out = capsys.readouterr().out
    assert "NOTE:" in out and "expected 1 = sentry only" in out
    assert "relayed via devp2p tx-gossip" in out


def test_tx_cascade_FAILS_when_the_producer_never_mined_the_L3_submission(monkeypatch):
    """The write path's core claim. A tx that never reaches a proposer's pool has no receipt on
    validator-0, and the case must name the relay rather than the transaction."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.TX_CASCADE_OVERLAY,
                       **_txc_world(cast_receipt=lambda h, **k: {}))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_tx_cascade(ctx)
    assert "devp2p tx-gossip relay L3→L2→validator failed" in e.value.message


def test_tx_cascade_FAILS_when_L3_never_syncs_the_block_back(monkeypatch):
    """The other direction. The receipt exists on the producer; L3 must serve it too, or the
    cascade is one-way and the case would otherwise report a full round trip."""
    seen = {"n": 0}

    def receipt(h, rpc_url=None, **k):
        seen["n"] += 1
        return {"status": "0x1", "blockNumber": "0x80"} if rpc_url == RPC else {}

    ctx, _ = _live_ctx(monkeypatch, asserts_follow.TX_CASCADE_OVERLAY,
                       **_txc_world(cast_receipt=receipt))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_tx_cascade(ctx)
    assert "L3 never synced the receipt" in e.value.message


def test_tx_cascade_FAILS_when_L3_stored_the_block_without_executing_it(monkeypatch):
    """The state half. A follower that stored the block but never ran the EVM serves the receipt
    happily and moves no balance — which is exactly what the delta catches."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.TX_CASCADE_OVERLAY,
                       **_txc_world(cast_balance=_balances(0, 0)))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_tx_cascade(ctx)
    assert "L3 balance delta" in e.value.message


def test_tx_cascade_FAILS_when_the_SSTORE_did_not_reach_L3(monkeypatch):
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.TX_CASCADE_OVERLAY,
                       **_txc_world(cast_balance=_balances(0, vf.TXC_TRANSFER_WEI),
                                    cast_call=lambda *a, **k: "0"))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_tx_cascade(ctx)
    assert "EVM SSTORE not synced" in e.value.message


def test_tx_cascade_FAILS_when_the_monitor_warned_ISOLATED_on_a_healthy_uplink(monkeypatch):
    """THE NEGATIVE, driven. Everything before this proved the route works, so the warning is a
    false positive in a fail-loud path."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.TX_CASCADE_OVERLAY,
                       **_txc_world(cast_balance=_balances(0, vf.TXC_TRANSFER_WEI),
                                    overlay_logs=lambda *s, **k: "WARN tx-route ISOLATED"))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_tx_cascade(ctx)
    assert "false positive" in e.value.message


def test_tx_cascade_FAILS_LOUD_on_an_unreadable_sentry_enode(monkeypatch):
    """An empty pubkey would otherwise be written as `enode://@172.20.0.30:30303`, producing a
    peerless L3 and no error anywhere."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.TX_CASCADE_OVERLAY,
                       **_txc_world(enode_pubkey=lambda url, dry_value="": ""))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_tx_cascade(ctx)
    assert "bad sentry enode pubkey" in e.value.message


def test_tx_cascade_submits_sequential_nonces(monkeypatch):
    """Both sends are `--async` against a node that mines nothing, so cast's per-invocation
    "latest" nonce would hand the same number to both and the second would come back
    "replacement transaction underpriced" — a failure that reads like a tx-path bug."""
    tails = []
    ctx, _ = _live_ctx(
        monkeypatch, asserts_follow.TX_CASCADE_OVERLAY,
        **_txc_world(cast_balance=_balances(0, vf.TXC_TRANSFER_WEI),
                     cast_send_async=lambda tail, note, dry_value="": (
                         tails.append(list(tail)) or
                         ("0xvalue" if note == "txc-l3-value-transfer" else "0xapprove"))))
    asserts_follow.assert_tx_cascade(ctx)
    assert [t[t.index("--nonce") + 1] for t in tails] == ["7", "8"]


def test_tx_cascade_FAILS_LOUD_on_an_unreadable_nonce(monkeypatch):
    """Coercing it to 0 would submit both txs at nonce 0 and 1, which the node rejects as "nonce
    too low" — a failure that reads like a broken write path, three assertions away from the read
    that actually failed."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.TX_CASCADE_OVERLAY,
                       **_txc_world(cast_nonce=lambda addr, **k: ""))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_tx_cascade(ctx)
    assert "could not read sender nonce from L3" in e.value.message


def test_tx_cascade_FAILS_LOUD_on_a_receipt_with_no_block_number(monkeypatch):
    """Defaulting it to 0 would make the finality wait target height 0 — satisfied instantly by
    every live chain — so the case would claim the tx block finalized without ever naming it."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.TX_CASCADE_OVERLAY,
                       **_txc_world(cast_receipt=lambda h, **k: {"status": "0x1"}))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_tx_cascade(ctx)
    assert "no blockNumber" in e.value.message


def test_tx_cascade_FAILS_LOUD_on_an_unreadable_L3_balance(monkeypatch):
    """An unreachable L3 RPC must not present as a 0 balance: the delta would then come out as
    exactly the transfer amount in one direction, i.e. a PASS on a node that answered nothing."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.TX_CASCADE_OVERLAY,
                       **_txc_world(cast_balance=lambda addr, **k: ""))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_tx_cascade(ctx)
    assert "refusing to read it as a balance" in e.value.message
