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

from dpos_harness.cases.smoke import (asserts_follow, cert_cascade, cert_follow, cert_keyless,
                                      driver, tx_cascade, verdicts, verdicts_follow as vf)
from dpos_harness.cases.smoke.driver import SmokeCtx, SmokeFailure
from dpos_harness.core import nodes, rpc
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
#: L3 keeps a host URL — the tx leg signs with host-side `cast`. The sentry has NO host port any
#: more: it is read and written in-container, so it is keyed by SERVICE.
L3_RPC = "http://localhost:28545"


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
        # Phase 4b's pair comes up HERE, right after phase 1's alignment and ~10 minutes before
        # it is used, so it is caught up and keyed when phase 4b needs it — started at phase 4 it
        # ran past its 180 s key budget on a live run. The proxy relays verbatim until armed, so
        # its follower is indistinguishable from the honest one until then.
        _compose(ov, "up", "-d", "cert-mitm-seed"),
        _compose(ov, "up", "-d", "cert-follower-seed"),
        _compose(ov, "stop", "--timeout", "40", "cert-follower"),
        _compose(ov, "start", "cert-follower"),
        _compose(ov, "up", "-d", "cert-mitm"),
        _compose(ov, "up", "-d", "cert-follower-tamper"),
        # The ARMING write has no bash ancestor — it is the mechanism that makes the pass-through
        # window end at a known instant instead of at a guessed one.
        _compose(ov, "exec", "-T", "cert-mitm-seed", "sh", "-c",
                 f"printf '%s' '1' > {vf.SEED_ARM_FILE}"),
        DOWN,
    ]


def test_cert_follow_order_stop_read_restart_is_the_gap_backfill():
    """Phase 2 IS an order: read the follower's height, stop it, let v0 run a full epoch past
    that height, restart, then require alignment. Reading the height after the stop would read a
    dead RPC; restarting before v0 advanced would back-fill an empty gap and pass on a follower
    that cannot back-fill at all."""
    seq = _seq(_dry(cert_follow)[1])
    read_f1 = _idx(seq, "overlay_check_node(cert-follower)")
    stop = _idx(seq, "stop --timeout 40 cert-follower", read_f1)
    advance = _idx(seq, "wait_finalized_ge(", stop)
    restart = _idx(seq, "start cert-follower", advance)
    align = _idx(seq, "overlay_wait_align(cert-follower,", restart)
    assert read_f1 < stop < advance < restart < align


def test_cert_follow_observes_for_the_full_window_between_the_two_v0_reads():
    """The tamper assertion is `v0 advanced AND the follower did not`, measured across ONE
    window. The 45 s sleep must sit BETWEEN the two producer reads and BEFORE the follower read,
    or the case is comparing readings that do not bracket anything."""
    seq = _seq(_dry(cert_follow)[1])
    tamper_up = _idx(seq, "up -d cert-follower-tamper")
    sleep = _idx(seq, f"{vf.TAMPER_OBSERVE_S}s", tamper_up)
    fins = [i for i, s in enumerate(seq) if s == "finalized_dec()"]
    tamper_read = _idx(seq, "overlay_check_node(cert-follower-tamper)")
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
    align = flat.index("overlay_wait_align(cert-follower-l1, > 32, <= 240s)")
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
        # The sentry has NO host port: `admin_addTrustedPeer` goes IN-CONTAINER, and it stays in
        # the transcript because a peer-policy change is choreography.
        _compose(ov, "exec", "-T", "sentry",
                 *rpc.rpc_post_argv(rpc.rpc_body(
                     "admin_addTrustedPeer", [f"enode://{pk3}@172.20.0.31:30303"]))),
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
    read_pk = _idx(seq, "overlay_enode_pubkey(sentry)")
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

#: The phase-4 follower's finalized height, identical BEFORE and AFTER the arming — the frozen
#: reading the negative asserts. A real hex and not `"null"`: this follower has been finalizing
#: honest blocks all along (that pass-through window is what gave it the key), so `overlay_reading`
#: refuses a sentinel here and the two samples double as liveness witnesses.
SEED_FROZEN = "0x140|0xcc"


def _seed_logs(refusals):
    """`cert-follower-seed`'s log, which GROWS across the arming — the shape the count reads.

    Keyed on the `tail` the case asks for, because that is what distinguishes the two kinds of
    read the body does: the key gate reads `SEED_LOG_TAIL`, and the refusal COUNT reads
    `SEED_COUNT_TAIL` exactly twice — once for the baseline before the arming and once after. A
    stub that answered the same text both times would make the delta zero and no world could ever
    pass, which is the one way to get this wrong that looks like a broken verdict."""
    seen = {"counts": 0}

    def logs(tail):
        head = [vf.CF_KEY_LINE + " epoch=2"]
        if tail != vf.SEED_COUNT_TAIL:
            return "\n".join(head)
        seen["counts"] += 1
        n = 0 if seen["counts"] == 1 else refusals
        return "\n".join(head + [f"WARN {vf.SEED_REJECT_LINE}; skipping h={h}"
                                 for h in range(n)])
    return logs


def _cf_logs(svc):
    """What each service says in the healthy world.

    Four different answers, and the two proxies' differ from the two followers': phase 3's
    follower refuses at DECODE and phase 4's at VERIFY, which are different lines on different
    code paths, and reading either one from the wrong service is how a negative passes for the
    wrong reason."""
    if svc == "cert-mitm":
        return vf.MITM_READY_LINE
    if svc == vf.SEED_MITM_SERVICE:
        return "\n".join((vf.MITM_READY_LINE, vf.SEED_ARMED_LINE, vf.SEED_CLEARED_LINE))
    if svc == vf.CF_SERVICE:
        return vf.CF_KEY_LINE + " epoch=2"
    if svc == vf.SEED_TAMPER_SERVICE:
        return vf.CF_KEY_LINE + " epoch=2"
    return vf.TAMPER_REJECT_LINES[0]


def _cf_world(seed_refusals=vf.MIN_SEED_REJECTS + 3, cf_ingesting=True, **over):
    """A healthy cert-follow world: the producer advances, the follower aligns, both MITMs come
    up, the phase-3 tamper follower finalizes nothing, and the phase-4 follower obtains PK_epoch
    and then freezes once the seed slots are cleared."""
    # EIGHT readings, in the order the four phases take them: phase 3's two producer samples,
    # phase 4a's two, phase 4b's two, and the pacing pair. The last gap is inside the band on
    # purpose — a producer that merely moved would satisfy every `v0 advanced` control and still
    # fail pacing, which is the whole reason pacing is a separate instrument.
    fin = iter([100, 140, 140, 180, 180, 220, 220, 220 + verdicts.PACING_MIN_BLOCKS + 5])
    # The phase-4a follower's OWN finalized head. It CLIMBS on every read, and that is the control
    # the phase was missing: a flat admission counter on a follower that finalized nothing is the
    # reading of a dead cert inlet, not of a pinned one. `cf_ingesting=False` freezes it, which is
    # what drives the negative.
    cf_head = [0x64]

    def _cf_head():
        if cf_ingesting:
            cf_head[0] += 0x1e
        return f"{cf_head[0]:#x}|0xaa"
    seed_logs = _seed_logs(seed_refusals)
    world = dict(
        baseline_height=lambda dry_value=0: 100,
        finalized_dec=lambda dry_value=0: next(fin, 400),
        overlay_wait_align=lambda *a, **k: "0x8c|0xaa",
        wait_finalized_ge=lambda *a, **k: True,
        # v0 is the only HOST-port read left in this case — the back-fill target, through the
        # fail-loud `ctx.reading`. The followers are keyed by SERVICE and read in-container.
        check_external=lambda port, dry_value="": "0x8c|0xaa",
        overlay_check_node=lambda svc, dry_value="": (
            _cf_head() if svc == vf.CF_SERVICE
            else SEED_FROZEN if svc == vf.SEED_TAMPER_SERVICE
            else "null|null"),
        # The tamper follower is UP and finalizing nothing — the shape the phase actually asserts.
        # It used to be unreachable, i.e. a "healthy world" whose negative passed vacuously.
        overlay_head_dec=lambda svc, **k: 12,
        shutdown_flushed=lambda *a, **k: True,
        overlay_logs=lambda *svcs, **k: (seed_logs(k.get("tail"))
                                         if svcs[0] == vf.SEED_TAMPER_SERVICE
                                         else _cf_logs(svcs[0])),
        # A scrape that ANSWERED and whose vote-only family did not move. Both halves matter:
        # an empty text is an unread endpoint and must fail, and a family that moved means the
        # pin never reached the cert inlet.
        overlay_el_metrics_text=lambda svc, **k: f"{vf.CF_VOTE_ONLY_FAMILY} 4\n",
        # The OTHER registry on the same container: the commonware one, carrying the adoption
        # counter. Sample names carry the doubled suffix a `prometheus-client` counter renders
        # with, so the fixture goes through `counter_sample` rather than hand-writing it — a
        # fixture that spelled the registered name would pass while the live scrape read nothing.
        overlay_node_metrics_text=lambda svc, **k: (
            f"{nodes.counter_sample(vf.CF_ADOPTED_FAMILY)} 1\n"
            f"{nodes.counter_sample(vf.CF_MISS_FAMILY)} 0\n"),
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
    assert "OK (phase 3 tampered-reject)" in out
    assert "OK (phase 4a PK_epoch obtained)" in out
    assert "OK (phase 4b cleared-seed reject)" in out
    assert "PK_epoch delivery" in out


def test_cert_follow_FAILS_when_the_tamper_follower_finalized_anything(monkeypatch):
    """THE NEGATIVE, driven through the BODY. A tamper follower reading a real height means the
    driver accepted a forged certificate. The body must read port 38545 and fail on it — a live
    run never walks this branch, because producing it needs a Byzantine build."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(overlay_check_node=lambda svc, dry_value="":
                                   "0x140|0xbb" if svc == vf.TAMPER_SERVICE else "0x64|0xaa"))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "verification is NOT load-bearing" in e.value.message


def test_cert_follow_FAILS_when_the_tamper_follower_never_came_UP(monkeypatch):
    """THE VACUOUS PASS THIS PHASE USED TO HAVE. A follower that never started reports exactly the
    zero progress the phase treats as proof of rejection, and `"null"` is a legitimate reading
    here — so the case has to ask a separate question, and `eth_blockNumber` is the one a live
    node answers even with `finalized` unset."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(overlay_head_dec=lambda svc, **k: -1))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "not up" in e.value.message


def test_cert_follow_FAILS_when_the_driver_never_logged_the_REJECTION(monkeypatch):
    """The follower is up, it finalized nothing — and it never said why. That is also what a MITM
    which came up and forwarded NOTHING looks like, so silence cannot be read as refusal."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(overlay_logs=lambda *svcs, **k: (
                           vf.MITM_READY_LINE if svcs[0] == "cert-mitm" else "cert applied")))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "never delivered" in e.value.message


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
                       **_cf_world(overlay_check_node=lambda svc, dry_value="": "null|null"))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "refusing to read an unreachable node as height 0" in e.value.message


# ── cert-follow phase 4 (FLU-1167), driven through the BODY ────────────────

def _no_key(svc):
    """The world where the follower never obtains `PK_epoch` — i.e. the pre-fix behaviour."""
    return "" if svc == vf.CF_SERVICE else _cf_logs(svc)


def test_cert_follow_FAILS_when_the_follower_never_obtains_PK_epoch(monkeypatch):
    """FLU-1167 ITSELF, driven. Before the fix a `--cert-follow` node had no route to the epoch
    artifact at all, so this log line did not exist and every certificate it admitted was checked
    on the multisig quorum alone. The case has to fail on that, not shrug."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(overlay_logs=lambda *svcs, **k: _no_key(svcs[0])))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "did not obtain PK_epoch" in e.value.message


def test_cert_follow_FAILS_when_vote_only_admissions_keep_growing(monkeypatch):
    """The CONSEQUENCE, separately. A follower can log that it holds the key and still verify
    seed-blind if the pin never reaches the cert inlet; the counter is the only thing that sees
    the difference."""
    counts = iter([f"{vf.CF_VOTE_ONLY_FAMILY} 4\n", f"{vf.CF_VOTE_ONLY_FAMILY} 29\n"])
    ctx, _ = _live_ctx(
        monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
        **_cf_world(overlay_el_metrics_text=lambda svc, **k: next(counts, "x 29\n")))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "grew 4 -> 29" in e.value.message


def test_cert_follow_phase4a_FAILS_when_the_FOLLOWER_stopped_INGESTING(monkeypatch):
    """THE F8 CONTROL, driven. The follower obtained PK_epoch, its adoption counters are right,
    the vote-only counter is perfectly flat over the window — and it finalized NOTHING, because
    its cert inlet stopped delivering. A flat counter is exactly what that produces.

    Phase 3 and phase 4b both take a control before concluding from an absence
    (`evaluate_v0_advanced`, and 4b additionally counts refusals); 4a took neither, against the
    module's own stated rule. The chain-side control alone would not catch this either — v0
    advances happily throughout, which is why this phase gets the follower's OWN height."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(cf_ingesting=False))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "finalized nothing over the vote-only window" in e.value.message
    assert vf.CF_VOTE_ONLY_FAMILY in e.value.message


def test_cert_follow_phase4a_FAILS_when_the_PRODUCER_stalled(monkeypatch):
    """The other half of the same gap: a chain-wide stall. Nothing was produced, so nothing was
    delivered, so nothing could have been admitted vote-only — and the phase used to report that
    as "the follower stopped admitting certificates vote-only"."""
    fin = iter([100, 140, 180, 180])
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(finalized_dec=lambda dry_value=0: next(fin, 180)))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "v0 stalled during tamper phase (180\u2192180)" in e.value.message


def test_cert_follow_FAILS_when_the_metrics_endpoint_never_answered(monkeypatch):
    """THE VACUOUS PASS THIS COUNTER WOULD OTHERWISE HAVE. An empty scrape and a family that was
    never incremented both read as "", and one of them is a pass while the other is a measurement
    that never happened. Reading the whole scrape is what tells them apart."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(overlay_el_metrics_text=lambda svc, **k: ""))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "did not answer" in e.value.message


def test_cert_follow_FAILS_when_the_beacon_registry_never_answered(monkeypatch):
    """The SECOND adoption witness must not be satisfiable by silence. The commonware registry
    exists on a follower only under a devnet build plus `--dpos.metrics-port`; drop either and
    the scrape is empty. An empty scrape has to fail here rather than read as "adopted nothing"
    or, worse, be skipped — otherwise a compose regression that removed the flag would quietly
    take the witness away and leave the phase looking as strong as before."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(overlay_node_metrics_text=lambda svc, **k: ""))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "commonware registry" in e.value.message
    assert vf.CF_ADOPTED_FAMILY in e.value.message


def test_cert_follow_FAILS_when_the_adoption_counter_disagrees_with_the_log(monkeypatch):
    """…and it must not be satisfiable by an ANSWERING endpoint either. The two witnesses count
    the same event one line apart in `fetch_and_verify`, so a registry that answers with the
    family at 0 while the log carries the adoption line is not a stale reading — it is the two
    disagreeing, and the case has to say so instead of preferring the one it likes."""
    ctx, _ = _live_ctx(
        monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
        **_cf_world(overlay_node_metrics_text=lambda svc, **k: (
            f"{nodes.counter_sample(vf.CF_ADOPTED_FAMILY)} 0\n"
            f"{nodes.counter_sample(vf.CF_MISS_FAMILY)} 3\n")))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "still 0" in e.value.message


def test_cert_follow_FAILS_when_the_seed_proxy_cleared_nothing(monkeypatch):
    """THE TAMPER MUST WITNESS ITS OWN TAMPERING. A proxy that armed and cleared no slot produces
    exactly the reading a follower that rejected nothing produces — a still node and a quiet log.
    The `tear_journal_to_torn` readback rule, applied to a proxy."""
    quiet = lambda svc: (vf.MITM_READY_LINE + "\n" + vf.SEED_ARMED_LINE
                         if svc == vf.SEED_MITM_SERVICE else _cf_logs(svc))
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(overlay_logs=lambda *svcs, **k: quiet(svcs[0])))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "cleared NO seed slot" in e.value.message


def test_cert_follow_FAILS_when_the_pinned_follower_ACCEPTS_a_cleared_seed(monkeypatch):
    """THE NEGATIVE THIS PHASE EXISTS FOR. The multisig quorum is untouched, so a follower that is
    not checking the seed slot admits every one of these certificates — it refuses nothing, and
    every acceptance ticks the vote-only counter. That is the hole FLU-1167 closed, and it is
    inexpressible in phase 3: a flipped nibble fails DECODE and never reaches the seed arm."""
    counts = iter([f"{vf.CF_VOTE_ONLY_FAMILY} 1\n", f"{vf.CF_VOTE_ONLY_FAMILY} 44\n"])

    # Admitted, not refused: the key line stays, the refusals never appear, and every acceptance
    # ticks the vote-only counter.
    ctx, _ = _live_ctx(
        monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
        **_cf_world(seed_refusals=0,
                    overlay_el_metrics_text=lambda svc, **k: (
                        next(counts, f"{vf.CF_VOTE_ONLY_FAMILY} 44\n")
                        if svc == vf.SEED_TAMPER_SERVICE
                        else f"{vf.CF_VOTE_ONLY_FAMILY} 4\n")))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "the seed slot is not being checked" in e.value.message


def test_cert_follow_FAILS_when_the_seed_follower_refused_nothing_at_all(monkeypatch):
    """A follower that refused nothing cannot be distinguished from one that was handed nothing —
    and either way the phase has proved nothing. The count is what separates "it rejected" from
    "the stream stopped", which a frozen height never could."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(seed_refusals=0))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "refused only 0 certificates" in e.value.message


def test_cert_follow_does_NOT_read_the_seed_followers_HEIGHT_as_the_verdict(monkeypatch):
    """THE INSTRUMENT THAT WAS WRONG, pinned so it cannot come back. A poisoned stream turns on a
    SECOND source for a follower's `finalized`: it refuses every certificate, counts the refusals
    as upstream data faults, rotates, and its EL keeps syncing over devp2p from the validator in
    `--trusted-peers`, after which the steady-state re-jump fast-forwards the anchor onto that EL
    tip. Live, with every certificate refused and the vote-only counter flat, `finalized` still
    went 238 → 271. This world reproduces exactly that — the follower's height RUNS AWAY while it
    refuses everything — and the phase must pass on it."""
    heads = iter(["0x140|0xcc", "0x1ff|0xcc", "0x2ff|0xcc"])
    # Phase 4a's own follower still climbs — that is its ingest control, and it is a DIFFERENT
    # follower from the poisoned one this test is about.
    cf = [0x64]

    def node(svc, dry_value=""):
        if svc == vf.SEED_TAMPER_SERVICE:
            return next(heads, "0x2ff|0xcc")
        if svc == vf.CF_SERVICE:
            cf[0] += 0x1e
            return f"{cf[0]:#x}|0xaa"
        return "null|null"

    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(overlay_check_node=node))
    asserts_follow.assert_cert_follow(ctx)


def test_cert_follow_arms_the_seed_proxy_only_AFTER_its_follower_holds_the_key(monkeypatch):
    """THE ORDER IS THE ASSERTION, and it is the one thing a rewrite can quietly break.
    `observe_cert` — the follower's only trigger for fetching the artifact — runs after a
    certificate VERIFIES. Arm first and the follower stays vote-only forever: it would accept
    every cleared certificate, the negative would never fire, and the case would blame the
    product for the harness's ordering."""
    ctx, runner = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY, **_cf_world())
    asserts_follow.assert_cert_follow(ctx)
    seq = [" ".join(inv.argv) if inv.argv else str(inv) for inv in runner.log]
    up_seed = next(i for i, x in enumerate(seq) if "up -d cert-follower-seed" in x)
    arm = next(i for i, x in enumerate(seq) if vf.SEED_ARM_FILE in x)
    assert up_seed < arm


def test_cert_follow_FAILS_when_the_seed_follower_is_still_back_filling(monkeypatch):
    """THE BUG THIS PHASE SHIPPED WITH. Armed while ~370 blocks behind, the follower advanced on
    certificates delivered before the proxy was armed and the phase reported that as "the seed
    slot is not being checked". The gate has to fire on the backlog, not on the freeze."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(overlay_wait_align=lambda svc, *a, **k: (
                           None if svc == vf.SEED_TAMPER_SERVICE else "0x8c|0xaa")))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "did not catch up with v0" in e.value.message


def test_cert_follow_arms_only_AFTER_the_seed_follower_is_caught_up():
    """ORDER, and the order is the whole of it. The align wait must sit between the key gate and
    the arming write: before the key and the follower cannot reject; after the arming and the
    backlog it was there to wait out has already carried it through the window.

    Read off the DRY transcript rather than a stubbed body, because that is where a poll appears
    as a recorded step at all — the live-branch stubs answer it without recording anything."""
    seq = _seq(_dry(cert_follow)[1])
    up_seed = _idx(seq, "up -d cert-follower-seed")
    key = _idx(seq, f"overlay_logs({vf.SEED_TAMPER_SERVICE}", up_seed)
    align = _idx(seq, f"overlay_wait_align({vf.SEED_TAMPER_SERVICE},", key)
    arm = _idx(seq, vf.SEED_ARM_FILE, align)
    assert up_seed < key < align < arm


def test_cert_follow_measures_pacing_with_the_one_existing_instrument(monkeypatch):
    """The band that produced the historical 26-27 blk/60s reading, and no second instrument.
    `evaluate_v0_advanced` passes on ONE block; a producer limping at half rate satisfies every
    control in this case and only pacing sees it."""
    fin = iter([100, 140, 140, 180, 180, 220, 220, 220 + 27])
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(finalized_dec=lambda dry_value=0: next(fin, 400)))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_follow(ctx)
    assert "block rate off target: 27 blocks" in e.value.message


def test_cert_follow_SKIPS_phase_4b_without_giving_up_phase_4a(monkeypatch, capsys):
    """The seed proxy needs the same pip install phase 3's does, so it has the same loud-skip
    path — and the skip must say what it did NOT test. Phase 4a is unaffected: the honest
    follower talks to v0 directly, so "the follower obtains PK_epoch" still ran for real."""
    def logs(svc):
        return "" if svc == vf.SEED_MITM_SERVICE else _cf_logs(svc)

    ctx, runner = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                            **_cf_world(overlay_logs=lambda *svcs, **k: logs(svcs[0])))
    asserts_follow.assert_cert_follow(ctx)
    out = capsys.readouterr().out
    assert "OK (phase 4a PK_epoch obtained)" in out
    assert "SKIP (phase 4b cleared-seed reject)" in out
    started = [" ".join(inv.argv) for inv in runner.log]
    assert not any("cert-follower-seed" in x for x in started)


def test_cert_cascade_FAILS_LOUD_rather_than_pushing_a_null_checkpoint_hash(monkeypatch):
    """The same guard on the other side: `"null"` is not a bytes32, and letting it through would
    fail inside `cast` three commands later with a message about ABI encoding."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_CASCADE_OVERLAY,
                       **_cc_world(check_external=lambda port, dry_value="": "null|null",
                                   overlay_check_node=lambda svc, dry_value="": "null|null"))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_cascade(ctx)
    assert "refusing to read an unreachable node as height 0" in e.value.message


def test_cert_follow_FAILS_when_the_follower_never_aligns(monkeypatch):
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_cf_world(overlay_wait_align=lambda *a, **k: None))
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


# ══ smoke-cert-keyless (FLU-1202) ══════════════════════════════════════════

def _keyless_world(keyless=7, flat=True, key_line=True, repaired=None, ingesting=True, **over):
    """The world `smoke-cert-keyless` is built to observe: a follower that came up inside a
    beacon-active epoch with NO key, took `keyless` admissions on the multisig quorum alone, then
    obtained `PK_epoch` and stopped taking them.

    Every knob turns off exactly one of the case's claims, so each negative below is driven by a
    single-field change rather than by a hand-built fixture that could differ in two ways."""
    fin = iter([200, 240])
    cf_head = [0x64]

    def _cf_head():
        if ingesting:
            cf_head[0] += 0x1e
        return f"{cf_head[0]:#x}|0xaa"

    scrapes = {"n": 0}

    def _el(svc, **k):
        # The counter is MONOTONE and the case reads it three times: once for the precondition,
        # then twice to bracket the window. `flat=False` makes the last read move, which is a
        # follower still admitting certificates with the seed slot unchecked.
        scrapes["n"] += 1
        n = keyless if (flat or scrapes["n"] < 3) else keyless + 9
        return f"reth_network_peers 3\n{vf.CF_VOTE_ONLY_FAMILY} {n}\n"

    def _logs(*svcs, **k):
        # `repaired` is THREE-valued because the sweep's mere presence is not the question — its
        # position relative to the adoption is. "after" is the world the first live run was in.
        sweep = f"INFO {vf.KEYLESS_REPAIR_LINE} — re-driving its finalization fetch epoch=Epoch(2)"
        lines = []
        if repaired == "before":
            lines.append(sweep)
        if key_line:
            lines.append(f"INFO {vf.CF_KEY_LINE} epoch=2")
        lines.append("INFO cert-inlet: ingested h=41")
        if repaired == "after":
            lines.append(sweep)
        return "\n".join(lines)

    world = dict(
        baseline_height=lambda dry_value=0: 200,
        finalized_dec=lambda dry_value=0: next(fin, 400),
        overlay_check_node=lambda svc, dry_value="": _cf_head(),
        overlay_el_metrics_text=_el,
        overlay_node_metrics_text=lambda svc, **k: (
            f"{nodes.counter_sample(vf.CF_ADOPTED_FAMILY)} 1\n"
            f"{nodes.counter_sample(vf.CF_MISS_FAMILY)} 0\n"),
        overlay_logs=_logs,
        sleep=lambda _s: None,
    )
    world.update(over)
    return world


def test_cert_keyless_transcript_starts_ONE_follower_and_poisons_nothing():
    """The whole case is one `up` on the shared overlay plus reads. It deliberately does NOT touch
    `cert-mitm`, `cert-follower-tamper`, `cert-mitm-seed` or `cert-follower-seed`, which the same
    overlay file defines — starting any of them would put a poisoned certificate stream on the
    stack this case measures its own follower against."""
    ov = asserts_follow.CERT_FOLLOW_OVERLAY
    rc, r = _dry(cert_keyless)
    assert rc == 0
    assert _cmds(r) == [UP, STOP, _recreate(ov), _compose(ov, "up", "-d", "cert-follower"), DOWN]


def test_cert_keyless_is_registered_and_reaches_its_own_assertion(monkeypatch):
    """`case <name>` and `case list` read one registry, and the module has to be the one that runs
    the body — a case wired to the wrong assertion passes every transcript test."""
    from dpos_harness import cli
    assert cli.CASES["smoke-cert-keyless"] == "smoke.cert_keyless"
    assert "smoke-cert-keyless" in cli.SUITE, "a case outside the SUITE never runs"
    seen = {}
    monkeypatch.setattr(cert_keyless.driver, "run",
                        lambda case, a, argv=None, **kw: seen.update(case=case, fns=list(a)) or 0)
    cert_keyless.run_case([])
    assert seen["case"] == "smoke-cert-keyless"
    assert seen["fns"] == [asserts_follow.assert_cert_keyless]


def test_cert_keyless_passes_on_the_world_it_exists_to_observe(monkeypatch, capsys):
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY, **_keyless_world())
    asserts_follow.assert_cert_keyless(ctx)
    out = capsys.readouterr().out
    assert "OK (phase 1 keyless entry)" in out and "7 admission(s)" in out
    assert "OK (phase 2 late key)" in out
    assert "OK (smoke-cert-keyless)" in out
    # The final line has to carry the two numbers a reader needs when this case is the one that
    # went red on a rerun: how big the keyless window was, and where the counter froze.
    assert "entered a beacon-active epoch keyless" in out
    assert f"{vf.CF_VOTE_ONLY_FAMILY}=7 -> 7" in out


def test_cert_keyless_FAILS_ON_THE_PRECONDITION_when_the_key_was_there_all_along(monkeypatch):
    """THE DELIBERATE BREAK, and the one negative this case exists for.

    Give the follower its key before it ever admits a certificate — the counter never leaves 0 —
    and every other reading still says yes: the key line is in the log, the counter is trivially
    flat across the window, v0 advances, the follower finalizes, no repair line anywhere. A case
    that only checked its CONCLUSION would go green here while having observed nothing at all,
    because this is what the majority of nodes on a healthy chain look like.

    So the failure must be the PRECONDITION and must name it. If this test ever starts failing on
    a different message, the gate moved and the case became a tautology."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_keyless_world(keyless=0))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_keyless(ctx)
    assert "KEYLESS WINDOW" in e.value.message
    assert "held the key all along" in e.value.message


def test_cert_keyless_FAILS_when_the_metrics_endpoint_never_ANSWERED(monkeypatch):
    """The other way the precondition can not-say-yes, and it is a different failure with a
    different fix: nothing was measured, rather than a node that keyed too early."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_keyless_world(overlay_el_metrics_text=lambda svc, **k: ""))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_keyless(ctx)
    assert "did not answer" in e.value.message


def test_cert_keyless_FAILS_when_the_key_never_ARRIVED(monkeypatch):
    """The keyless window happened and never closed — which is FLU-1167's symptom, not FLU-1202's
    property. The case must not report a permanently seed-blind follower as a pass."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_keyless_world(key_line=False))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_keyless(ctx)
    assert "did not obtain PK_epoch" in e.value.message


def test_cert_keyless_FAILS_when_admissions_KEEP_COMING_after_the_key(monkeypatch):
    """The conclusion, driven false. A counter that still moves once the key is held means the
    live key store read is not reaching the certificate path — exactly the staleness FLU-1202
    deleted, come back."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_keyless_world(flat=False))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_keyless(ctx)
    assert "still being admitted with the seed slot" in e.value.message


def test_cert_keyless_FAILS_when_the_REPAIR_SWEEP_reached_the_epoch_FIRST(monkeypatch):
    """`repair_keyless_schemes` survives FLU-1202 as a below-frontier `ensure_key` + finalization
    re-drive, and if it reached the epoch before this node adopted its own key then the two roads
    to that key are indistinguishable from the log.

    A follower DOES run this sweep — `launch_follower` builds an `EpochManager` — so this is a
    live discriminator and not a structural tautology. The first live run of the case proved it by
    firing the line; what that run also proved is that the sweep FOLLOWS the adoption, which is
    why the case asserts order and `repaired="before"` is the only failing world."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_keyless_world(repaired="before"))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_keyless(ctx)
    assert "BEFORE it logged the key adoption" in e.value.message


def test_cert_keyless_PASSES_when_the_sweep_merely_FOLLOWED_the_key(monkeypatch, capsys):
    """THE WORLD THE FIRST LIVE RUN WAS IN, and the one an absence assertion called a failure.

    The sweep reaches an epoch when that epoch drops below the frontier, which on this geometry
    happens inside the case's own window — 32 s after the adoption, live. That is a consequence of
    the tested event, not a competing explanation for it, and the case must say so in its output
    rather than go red."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_keyless_world(repaired="after"))
    asserts_follow.assert_cert_keyless(ctx)
    out = capsys.readouterr().out
    assert "OK (smoke-cert-keyless)" in out
    assert "AFTER the adoption" in out and "rather than delivering it" in out


def test_cert_keyless_FAILS_when_the_follower_stopped_INGESTING(monkeypatch):
    """The control the flat counter cannot do without: a follower whose upstream died admits
    nothing, so its counter is flat for a reason that has nothing to do with the seed check."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_keyless_world(ingesting=False))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_keyless(ctx)
    assert "finalized nothing over the vote-only window" in e.value.message


def test_cert_keyless_FAILS_when_v0_STALLED_during_the_window(monkeypatch):
    """…and the chain-side half of the same control."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_FOLLOW_OVERLAY,
                       **_keyless_world(finalized_dec=lambda dry_value=0: 200))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_keyless(ctx)
    assert "v0 stalled" in e.value.message


def test_cert_keyless_reads_the_repair_grep_AFTER_the_observation_window(monkeypatch):
    """ORDER, and it is load-bearing. The sweep's re-drive can land at any point up to the last
    reading the case takes, so a grep issued at key-arrival time would be silent about the whole
    interval the flatness verdict covers. The body therefore reads the log a SECOND time, after
    the sleep."""
    seq = _seq(_dry(cert_keyless)[1])
    sleep = _idx(seq, f"{vf.CF_VOTE_ONLY_WINDOW_S}s")
    logs = [i for i, s in enumerate(seq) if s.startswith("overlay_logs(cert-follower")]
    assert len(logs) == 2, "one read for the key gate, one for the repair absence"
    assert logs[0] < sleep < logs[1]


def test_cert_keyless_brackets_the_window_with_two_scrapes_and_two_producer_reads():
    """The flatness verdict is a DELTA and both controls are deltas too, so the sleep has to sit
    strictly between each pair. A window whose ends were both sampled on the same side of the
    sleep measures nothing and passes."""
    seq = _seq(_dry(cert_keyless)[1])
    sleep = _idx(seq, f"{vf.CF_VOTE_ONLY_WINDOW_S}s")
    scrapes = [i for i, s in enumerate(seq) if s.startswith("overlay_el_metrics_text(")]
    fins = [i for i, s in enumerate(seq) if s == "finalized_dec()"]
    heads = [i for i, s in enumerate(seq) if s.startswith("overlay_check_node(cert-follower")]
    # Three scrapes: the precondition poll, then the window's two ends.
    assert len(scrapes) == 3 and scrapes[0] < scrapes[1] < sleep < scrapes[2]
    assert len(fins) == 2 and fins[0] < sleep < fins[1]
    assert len(heads) == 2 and heads[0] < sleep < heads[1]


# ══ the wiring: cert-cascade ═══════════════════════════════════════════════

def _cc_world(**over):
    world = dict(
        baseline_height=lambda dry_value=0: 100,
        finalized_dec=lambda dry_value=0: 100,
        funded_key=lambda **k: "00" * 32,
        cast_send=lambda tail, note, dry_value="{}": (
            {"contractAddress": MOCK if note == "cc-deploy-mock-rollup" else BOGUS}
            if "--create" in [str(t) for t in tail] else {"blockNumber": "0x80"}),
        check_external=lambda port, dry_value="": ("0x80|0x" + "ab" * 32
                                                   if port == 8545 else "null|null"),
        overlay_check_node=lambda svc, dry_value="": "null|null",
        wait_finalized_ge=lambda *a, **k: True,
        overlay_wait_align=lambda *a, **k: "0x8c|0xaa",
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


def test_cert_cascade_does_NOT_accept_a_dead_container_as_the_refusal(monkeypatch):
    """The inverse of the test that used to live here. `exited` was accepted as proof of refusal,
    which blessed a follower that died for ANY reason — the false PASS `evaluate_bogus_rejected`
    now describes. With the witness gone this world must fail, and it must fail with the
    never-refused message rather than by falling through."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_CASCADE_OVERLAY,
                       **_cc_world(overlay_logs=lambda *svcs, **k:
                                   vf.L1_VERIFIED_LINE if svcs[0] == "cert-follower-l1" else "",
                                   overlay_ps_state=lambda *a, **k: "exited"))
    with pytest.raises(SmokeFailure) as e:
        asserts_follow.assert_cert_cascade(ctx)
    assert e.value.message == vf.BOGUS_NOT_REFUSED


def test_cert_cascade_FAILS_when_the_bogus_follower_refused_and_followed_anyway(monkeypatch):
    """THE SECOND HALF of the negative. Logging the refusal and then finalizing past the anchor is
    the fail-open the log grep alone cannot see."""
    ctx, _ = _live_ctx(monkeypatch, asserts_follow.CERT_CASCADE_OVERLAY,
                       **_cc_world(overlay_check_node=lambda svc, dry_value="":
                                   "0xc8|0xbb" if svc == vf.BOGUS_SERVICE else "null|null"))
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
        overlay_wait_align=lambda *a, **k: "0x8c|0xaa",
        wait_finalized_ge=lambda *a, **k: True,
        overlay_enode_pubkey=lambda svc, dry_value="": "ab" * 64,
        overlay_rpc_write=lambda svc, method, *a, **k: None,
        enode_pubkey=lambda url, dry_value="": "cd" * 64,
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
                       **_txc_world(overlay_enode_pubkey=lambda svc, dry_value="": ""))
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
