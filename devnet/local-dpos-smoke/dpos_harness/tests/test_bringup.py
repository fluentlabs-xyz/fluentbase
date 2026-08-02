"""test_bringup.py — the sim_bring_up choreography as a recorded command sequence. Drives
BringUp with a dry Runner and asserts the ORDER + the exact key command lines (the bash
sim_bring_up 668-903 is the oracle)."""

import ast
import inspect
import json
import os

import pytest

from dpos_harness.stack import compose_gen
from dpos_harness.stack import bringup as bringup_mod
from dpos_harness.stack.bringup import BringUp, StackSpec
from dpos_harness.sim.orchestrator import SimConfig
from dpos_harness.core import proc
from dpos_harness.core.proc import Runner


class _StopBringUp(Exception):
    """Sentinel to abort bu.run() right after the compose-gen call (before any docker)."""


def _spec(**kw):
    """A StackSpec via the sim's own projection, so these tests keep exercising SimConfig's
    derived arithmetic (spares/rotation_slots → val_containers → identity_pool)."""
    return SimConfig(**kw).stack_spec()


def _run():
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    r = Runner(dry=True)
    bu = BringUp(cfg.stack_spec(), r)
    bu.run()
    return r, bu


def _lines(r):
    return [" ".join(a.argv) for a in r.log]


def _idx(lines, *needles):
    for i, l in enumerate(lines):
        if all(n in l for n in needles):
            return i
    return -1


def test_bringup_phase_order():
    """The load-bearing ORDER: preflight down → phase-A up → DeployStaking → regen reader →
    setDposActivationBlock → stop spares → --dpos cold-restart."""
    r, _ = _run()
    L = _lines(r)
    down = _idx(L, "docker compose down -v --remove-orphans")
    upA = _idx(L, "docker compose up --build -d")
    deploy = _idx(L, "DeployStaking.s.sol:DeployStaking")
    regen = _idx(L, "sh -c cat > /runtime/staking-reader.json")
    act = _idx(L, "setDposActivationBlock")
    stop_spare = _idx(L, "docker compose stop validator-")
    cold = _idx(L, "up -d --force-recreate validator-0")
    assert 0 <= down < upA < deploy < regen < act < stop_spare < cold


def test_bringup_passes_val_containers_not_pool(monkeypatch):
    """caller-arg oracle: bringup must pass cfg.val_containers as N to compose_gen.generate,
    NOT the identity pool (the IDENTITY bound) nor the committee target. validators=4/spares=1/
    rotation=1 → val_containers=6, identity_pool=12; N MUST be 6, never 12 or 4."""
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    assert (cfg.val_containers, cfg.identity_pool) == (6, 12)
    r = Runner(dry=False)                     # non-dry so the `if not dry: generate(...)` runs
    bu = BringUp(cfg.stack_spec(), r)
    seen = {}

    def _rec(n, target=None, identity_pool=None, out_dir="."):
        seen.update(n=n, target=target, identity_pool=identity_pool)
        raise _StopBringUp

    # NB: no write_file stub. The marker that precedes generate() is a Runner.step(), which
    # touches no FS — a non-dry Runner used to need that stub precisely BECAUSE the marker was
    # a write_file whose path argument was a label (see test_compose_gen_marker_writes_no_file).
    monkeypatch.setattr(compose_gen, "generate", _rec)
    with pytest.raises(_StopBringUp):
        bu.run()
    assert seen == {"n": 6, "target": 4, "identity_pool": 12}
    assert seen["n"] == cfg.val_containers
    assert seen["n"] != cfg.identity_pool     # the exact v61.9 confusion


def test_compose_gen_marker_writes_no_file(tmp_path, monkeypatch):
    """The `<compose-gen>` junk-file bug: bring-up recorded the compose-generation step by
    calling `Runner.write_file("<compose-gen>", ...)` — a transcript LABEL in the argument that
    names a file to open. Under a LIVE (non-dry) Runner that created a 19-byte file literally
    named `<compose-gen>` in the smoke directory on every run: untracked, un-gitignored, and
    skipped three times before it was chased.

    Two properties, both asserted against a REAL (non-dry) Runner in a scratch cwd:
      1. the marker is recorded in the transcript, so the dry-run sequence still shows it; and
      2. it creates NO file at all — the step channel cannot write."""
    monkeypatch.chdir(tmp_path)
    before = set(os.listdir(tmp_path))
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    r = Runner(dry=False)                     # LIVE runner — the mode that created the junk file
    bu = BringUp(cfg.stack_spec(), r)

    def _stop(*a, **k):
        raise _StopBringUp

    monkeypatch.setattr(compose_gen, "generate", _stop)
    with pytest.raises(_StopBringUp):
        bu.run()

    marker = [i for i in r.log if i.argv[:1] == ["<step>"]]
    assert [i.argv[1] for i in marker] == ["compose-gen"]
    assert "N=6" in marker[0].note                          # still carries the topology numbers
    assert marker[0].kind == "step" and not any(i.kind == "file" for i in r.log)
    assert set(os.listdir(tmp_path)) == before              # nothing written, by any name


def test_write_file_rejects_a_transcript_label_as_path(tmp_path):
    """The structural guard behind the bug above: `path` and the transcript label are two
    different arguments of write_file, and passing the label as the path must be LOUD. `<...>`
    is proc.py's own pseudo-argv convention (`<write-file>`, `<step>`), so a value carrying it
    is a label that took a wrong turn. A real path still writes, and the label rides in note=."""
    r = Runner(dry=False)
    for bad in ("<compose-gen>", "<write-file>", "", "  ", " spaced.yml"):
        with pytest.raises(ValueError, match="transcript LABEL"):
            r.write_file(bad, "x")
    assert r.log == []                                      # a rejected write records nothing
    good = str(tmp_path / "docker-compose.sim.reborn-validator-3.gen.yml")
    r.write_file(good, "services: {}\n", note="rebirth-overlay")
    with open(good) as f:
        assert f.read() == "services: {}\n"
    assert r.log[-1].argv == ["<write-file>", good] and r.log[-1].note == "rebirth-overlay"


def test_stack_spec_fields_are_exactly_what_bringup_reads():
    """The StackSpec contract, DERIVED ON BOTH SIDES rather than mirrored in a hand-written
    list: every `spec.<name>` bring-up reads must be a declared field, and every declared field
    must be read by bring-up. A spec whose field set drifts from the code is the decoration this
    replaced — an under-declared field is an AttributeError at run time, and an over-declared one
    is a knob the caller believes has an effect and does not.

    Both sides come from the same source of truth (the AST of bringup.py and the dataclass's own
    __dataclass_fields__), so neither can be quietly widened to make this pass."""
    tree = ast.parse(inspect.getsource(bringup_mod))
    read = set()
    for node in ast.walk(tree):
        if not isinstance(node, ast.Attribute):
            continue
        base = node.value
        # `spec.X` (the run()/_deploy_staking local) or `self.spec.X`
        local = isinstance(base, ast.Name) and base.id == "spec"
        attr = (isinstance(base, ast.Attribute) and base.attr == "spec"
                and isinstance(base.value, ast.Name) and base.value.id == "self")
        if local or attr:
            read.add(node.attr)
    declared = set(StackSpec.__dataclass_fields__)
    assert read, "AST found no spec reads at all — the derivation broke, not the contract"
    assert read == declared, (
        f"bring-up reads {sorted(read - declared)} that StackSpec does not declare; "
        f"StackSpec declares {sorted(declared - read)} that bring-up never reads")


def test_bringup_refuses_a_duck_typed_config():
    """The whole point of the spec: bring-up must NOT accept "anything with the right seven
    attributes". SimConfig still HAS all seven — it is the object the duck-type was built
    around — so if it were accepted the coupling would be intact and this exercise cosmetic."""
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    assert all(hasattr(cfg, f) for f in StackSpec.__dataclass_fields__)   # the duck still quacks
    with pytest.raises(TypeError, match="needs a StackSpec"):
        BringUp(cfg, Runner(dry=True))


def test_a_case_can_boot_the_stack_with_no_sim_config():
    """The framework claim, exercised: a consumer that knows nothing about SimConfig — no
    calm fraction, no stake bands, no PRNG seed — constructs a StackSpec by hand and gets the
    full bring-up sequence. This is what `stack/` being reusable actually means."""
    spec = StackSpec(validators=3, val_containers=3, initial_committee=3, identity_pool=5,
                     no_cascade=1)
    r = Runner(dry=True)
    BringUp(spec, r).run()
    L = _lines(r)
    assert _idx(L, "up -d --force-recreate validator-0 validator-1 validator-2") >= 0
    assert _idx(L, "full-node") == -1                    # no_cascade=1 → no L2/L3 tiers
    assert [a for a in r.log if a.note == "stop-spare"] == []   # val_containers == validators


def test_stop_spares_band_matches_bash():
    """stop-spares band oracle (case-soak.sh:873-875): stop EXACTLY validator-{validators..
    val_containers-1} before the --dpos cold-restart — the spare+rotation tail, never a
    committee/growth idx (<validators) nor a container-less bench idx (>=val_containers)."""
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)   # band = [4,6)
    r = Runner(dry=True)
    BringUp(cfg.stack_spec(), r).run()
    stopped = sorted(int(" ".join(a.argv).rsplit("-", 1)[-1])
                     for a in r.log if a.note == "stop-spare")
    assert stopped == list(range(cfg.validators, cfg.val_containers)) == [4, 5]


def test_byzantine_flag_assert_present():
    """`docker compose run … --entrypoint /usr/local/bin/fluent genesis-init node
    --dpos.byzantine equivocate --help` gates scheduling byz."""
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1, byzantine=1)
    r = Runner(dry=True)
    BringUp(cfg.stack_spec(), r).run()
    assert _idx(_lines(r), "--dpos.byzantine", "equivocate", "--help") >= 0


def test_deploy_staking_env_overlay():
    """DeployStaking rides INITIAL_VALIDATORS/STAKES/ACTIVE_VALIDATORS_LENGTH as an env delta;
    the recorded argv is the bare `forge script …` line (the oracle)."""
    r, _ = _run()
    dep = next(a for a in r.log if "DeployStaking.s.sol:DeployStaking" in " ".join(a.argv))
    assert dep.argv[:2] == ["forge", "script"]
    assert dep.env.get("ACTIVE_VALIDATORS_LENGTH") == "3"
    assert dep.env.get("NETWORK") == "local-dpos-smoke/l2"


def test_deploy_staking_cwd_and_absolute_output_path():
    """The forge_l2 wrapper is `( cd $SOLIDITY_CONTRACTS_DIR && forge … )`, and DeployStaking's
    `vm.writeJson(out, OUTPUT_PATH)` runs under solidity-contracts' foundry.toml whose
    fs_permissions only allow `./deployments`. Pin the full oracle triple: the invocation's
    cwd == contracts_dir, and OUTPUT_PATH is an ABSOLUTE path inside <contracts>/deployments
    (a relative path resolves against forge's root=cwd and escapes the allowed dir → v61.3 bug)."""
    r, bu = _run()
    dep = next(a for a in r.log if "DeployStaking.s.sol:DeployStaking" in " ".join(a.argv))
    # cwd mirrors the bash `cd $SOLIDITY_CONTRACTS_DIR`
    assert dep.cwd == bu.contracts_dir
    out = dep.env.get("OUTPUT_PATH")
    # writer path == the manifest the readers use (single source of truth)
    assert out == bu.manifest
    # ABSOLUTE and inside <contracts_dir>/deployments so foundry fs_permissions allow the write
    assert os.path.isabs(out)
    assert out == os.path.join(os.path.abspath(bu.contracts_dir),
                               "deployments", "runtime-deployment.json")


def test_manifest_reader_writer_agree_on_absolute_path():
    """The DeployStaking OUTPUT_PATH (writer), _read_manifest's open(), and the
    sim_regen_staking_reader arg (reader) must all be the SAME absolute file — otherwise the
    forge write (cwd=contracts_dir) and the Python read (cwd=smoke dir) resolve to different
    files. All three consume bu.manifest, which is now absolute."""
    r, bu = _run()
    dep = next(a for a in r.log if "DeployStaking.s.sol:DeployStaking" in " ".join(a.argv))
    assert os.path.isabs(bu.manifest)
    assert dep.env.get("OUTPUT_PATH") == bu.manifest


def test_token_create_runs_under_contracts_dir():
    """MockBlendToken forge-create shares the forge_l2 wrapper semantics (cwd=contracts_dir)."""
    r, bu = _run()
    tok = next(a for a in r.log if "MockBlendToken.sol:MockBlendToken" in " ".join(a.argv))
    assert tok.cwd == bu.contracts_dir


def test_setconsensuskeys_for_initial_committee():
    """setConsensusKeys is issued for v0..v(initial-1) = 3 validators."""
    r, _ = _run()
    n = sum(1 for l in _lines(r) if "setConsensusKeys(address,bytes,bytes,bytes32)" in l)
    assert n == 3


def test_no_cascade_skips_downstream():
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1, no_cascade=1)
    r = Runner(dry=True)
    BringUp(cfg.stack_spec(), r).run()
    assert _idx(_lines(r), "up -d downstream") == -1
    # full-node is NOT in the cold-restart up list under no_cascade
    cold = next(l for l in _lines(r) if "up -d --force-recreate validator-0" in l)
    assert "full-node" not in cold


# ── L3 cascade enode capture (v61.7 downstream-crash root) ───────────────────
def test_cascade_l2_enode_has_nonempty_id():
    """The pinned l2-enode.txt write MUST carry a full 128-hex node id — never the v61.7
    `enode://@172.20.0.250:30303` (empty id → clap 'Failed to parse id', downstream exit 2)."""
    r, _ = _run()
    write = next(a for a in r.log if a.note == "l2-enode-write")
    payload = " ".join(write.argv)
    assert "enode://@" not in payload
    assert f"enode://{'0' * 128}@172.20.0.250:30303" in payload   # dry pubkey = 128 zeros


def test_cascade_l3_spammer_distinct_key_and_l3_endpoint(monkeypatch):
    """Faithful to bash sim_start_cascade_l3: the L3 write-path spammer uses a DISTINCT key
    (mnemonic index 7 — never index 6, which would nonce-race the v0 spammer), is funded via
    owner 0, and submits to L3's OWN RPC (28545), not the L2 full-node (18545)."""
    from dpos_harness.stack.bringup import SpammerPool
    calls = []
    monkeypatch.setattr(SpammerPool, "start",
                        lambda self, key, to, url, note="": calls.append((url, note)))
    r, _ = _run()
    L = _lines(r)
    # distinct key at index 7 (address + private-key both index 7), funded via cast send
    assert _idx(L, "cast wallet private-key", "--mnemonic-index 7") >= 0
    assert next(a for a in r.log if a.note == "l3-spammer-key").argv[-1] == "7"
    assert any(a.note == "fund-l3-spammer" and a.argv[:2] == ["cast", "send"] for a in r.log)
    # the v0 spammer (index 6) is a SEPARATE key — no collision
    assert next(a for a in r.log if a.note == "spammer-key").argv[-1] == "6"
    # spammer routed to L3:28545, NOT L2:18545
    l3 = next(url for url, note in calls if note == "l3")
    assert l3 == "http://localhost:28545"


def test_cascade_exports_l3_spammer_addr(monkeypatch):
    """F1a follow-up (2026-07-30): bash exported L3_SPAMMER_ADDR (case-soak.sh:945-946); the port
    kept it a local, so battery.Ctx (which sources the field from the environment) always saw ""
    and the write-path reporter — "L3-submitted txs not landing, sender nonce flat" — was INERT for
    the whole life of the Python harness."""
    monkeypatch.setenv("L3_SPAMMER_ADDR", "")
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    r = Runner(dry=True)
    r.reads["cast wallet"] = "0x00000000000000000000000000000000000000aa"
    BringUp(cfg.stack_spec(), r).run()
    assert os.environ["L3_SPAMMER_ADDR"] == "0x00000000000000000000000000000000000000aa"


def test_cascade_does_not_clobber_l3_spammer_addr_with_an_unread_value(monkeypatch):
    """An unreadable/dry `cast wallet address` must not overwrite an operator-set value with ""."""
    monkeypatch.setenv("L3_SPAMMER_ADDR", "0x00000000000000000000000000000000000000bb")
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    BringUp(cfg.stack_spec(), Runner(dry=True)).run()          # no canned read → address comes back ""
    assert os.environ["L3_SPAMMER_ADDR"] == "0x00000000000000000000000000000000000000bb"


def _live_bu(monkeypatch):
    """A BringUp on a non-dry runner with the enode retry-sleep neutralized — exercises the live
    pubkey-read/retry branch without real wall-time."""
    from dpos_harness.stack import bringup as bmod
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    r = Runner(dry=True)
    bu = BringUp(cfg.stack_spec(), r)
    bu.p.dry = False
    monkeypatch.setattr(bmod.time, "sleep", lambda s: None)
    return bu, r


def test_cascade_hardfails_on_empty_l2_pubkey(monkeypatch):
    """An empty/bad L2 pubkey (after the bounded retry) MUST raise (bash `[[ =~ ^[0-9a-fA-F]{128}$
    ]] || return 1`) rather than silently write a malformed l2-enode.txt — the v61.7 crash."""
    from dpos_harness.core import rpc
    from dpos_harness.chain.writes import ChainError
    bu, r = _live_bu(monkeypatch)
    monkeypatch.setattr(rpc, "_enode_pubkey", lambda url: "")
    try:
        bu._start_cascade_l3()
        assert False, "expected ChainError on empty L2 enode pubkey"
    except ChainError:
        pass
    # the raise fires BEFORE any write is recorded (no malformed l2-enode.txt escapes)
    assert not any(a.note == "l2-enode-write" for a in r.log)


def test_capture_enode_pubkey_retries_then_succeeds(monkeypatch):
    """Bash-equivalent bounded retry: the pk read polls admin_nodeInfo until the 128-hex pubkey
    appears (v61.8: full-node RPC not up at the first read), sleeping BETWEEN attempts only."""
    from dpos_harness.core import rpc
    from dpos_harness.stack.bringup import _ENODE_PK_SLEEP
    bu, _ = _live_bu(monkeypatch)
    seq = ["", "", "a" * 128]
    calls = []

    def flaky(url):
        calls.append(url)
        return seq.pop(0)

    sleeps = []
    monkeypatch.setattr(rpc, "_enode_pubkey", flaky)
    from dpos_harness.stack import bringup as bmod
    monkeypatch.setattr(bmod.time, "sleep", lambda s: sleeps.append(s))
    pk = bu._capture_enode_pubkey("http://localhost:18545")
    assert pk == "a" * 128
    assert calls == ["http://localhost:18545"] * 3     # two empties then success
    assert sleeps == [_ENODE_PK_SLEEP, _ENODE_PK_SLEEP]  # slept between the empties, NOT after success


def test_capture_enode_pubkey_bounded_exhaustion(monkeypatch):
    """The retry is BOUNDED (never infinite): a permanently-down node yields exactly
    _ENODE_PK_RETRIES attempts then "" — the caller then hard-fails loudly."""
    from dpos_harness.core import rpc
    from dpos_harness.stack.bringup import _ENODE_PK_RETRIES
    bu, _ = _live_bu(monkeypatch)
    calls = []

    def down(url):
        calls.append(url)
        return ""

    monkeypatch.setattr(rpc, "_enode_pubkey", down)
    pk = bu._capture_enode_pubkey("http://localhost:18545")
    assert pk == ""
    assert len(calls) == _ENODE_PK_RETRIES


def test_capture_enode_pubkey_dry_is_canned(monkeypatch):
    """Dry → canned 128 zeros, no RPC and no sleep (the choreography still walks the sequence)."""
    from dpos_harness.core import rpc
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    bu = BringUp(cfg.stack_spec(), Runner(dry=True))
    monkeypatch.setattr(rpc, "_enode_pubkey",
                        lambda url: (_ for _ in ()).throw(AssertionError("no RPC in dry")))
    assert bu._capture_enode_pubkey("http://localhost:18545") == "0" * 128


def test_cascade_l2_read_strictly_after_convergence_gate(monkeypatch):
    """Acquisition-order oracle (bash chronology): the cold-restart convergence gate — bash's
    `_wait_aligned "$cold_wait" "$ANCHOR" _read_sim_nodes` (case-soak.sh:890, INCLUDES full-node
    @18545) — is invoked BEFORE the L2 pk2 read, which in turn precedes the L3 pk3 read. This pins
    the sequence bash relies on. The FIRST floored gate (floor != "") is the cold-restart converge;
    dry captures anchor="0x0" from the activation converge head."""
    events = []
    orig_wait = BringUp._wait_aligned
    orig_cap = BringUp._capture_enode_pubkey

    def wait_spy(self, timeout, floor, reader):
        events.append(("wait_aligned", floor))
        return orig_wait(self, timeout, floor, reader)

    def cap_spy(self, url):
        events.append(("capture", url))
        return orig_cap(self, url)

    monkeypatch.setattr(BringUp, "_wait_aligned", wait_spy)
    monkeypatch.setattr(BringUp, "_capture_enode_pubkey", cap_spy)
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    BringUp(cfg.stack_spec(), Runner(dry=True)).run()
    # FIRST floored gate (non-empty floor = the anchor) = cold-restart converge.
    gate = next(i for i, e in enumerate(events) if e[0] == "wait_aligned" and e[1] not in ("", None))
    pk2 = events.index(("capture", "http://localhost:18545"))
    pk3 = events.index(("capture", "http://localhost:28545"))
    assert gate < pk2 < pk3


def test_dry_run_executes_nothing_but_records():
    r, _ = _run()
    assert len(r.log) > 50           # a full bring-up sequence
    assert r.dry is True


def test_activation_wait_precedes_cold_restart(monkeypatch):
    """Bring-up MUST chain-pace-wait for finalized >= ACT (the sequencer's clean-halt) BEFORE the
    --dpos cold-restart — never return / restart on a still-bare pre-activation chain (the
    fin=216-vs-act=360 premature-return class). Spy the ChainPaced.wait seam and assert (i) it
    fires exactly once under key 'activation', (ii) with the bash budget ACT-HEAD+interval, (iii)
    before the cold-restart command is recorded, and (iv) its cond is the finalized>=ACT oracle."""
    from dpos_harness.core import chainpaced
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    r = Runner(dry=True)
    bu = BringUp(cfg.stack_spec(), r)
    seen = []

    def spy(self, key, cond_fn, domain, budget, froz=120, poll=2):
        if key == "activation":
            seen.append({"cond": cond_fn, "domain": domain, "budget": budget,
                         "log_len_at_call": len(r.log)})
        return True

    monkeypatch.setattr(chainpaced.ChainPaced, "wait", spy)
    bu.run()

    assert len(seen) == 1
    w = seen[0]
    assert w["domain"] == "blocks"
    # dry HEAD=0x0 → head=0, interval default 64 → ACT = ((0//64)+2)*64 = 128; budget = 128-0+64.
    assert w["budget"] == 128 - 0 + 64
    cold_idx = _idx(_lines(r), "up -d --force-recreate validator-0")
    assert cold_idx >= 0
    # the activation wait was invoked while the cold-restart was NOT yet recorded → strictly before.
    assert w["log_len_at_call"] <= cold_idx
    # wait-oracle: the cond is finalized >= ACT(128), evaluated live against _finalized_dec.
    bu._finalized_dec = lambda: 200
    assert w["cond"]() is True
    bu._finalized_dec = lambda: 100
    assert w["cond"]() is False


# ── COMPOSE_FILE lifecycle (v61.6 exec-read fin=-1/roots=null fix) ─────────────
# Bug: bring-up set only the Runner's env; the BARE `docker compose exec` READ helpers
# (rpc.py/nodes.py) inherit only os.environ → resolved no/wrong project → sentinel. Fix mirrors
# the bash `export COMPOSE_FILE=...` (case-soak.sh :675/:877) into the process-global env.
from dpos_harness.stack.bringup import BASE_COMPOSE, DPOS_COMPOSE, resolve_compose_env


def test_phaseA_switch_export_process_global_compose(monkeypatch):
    """The two lifecycle exports land in os.environ in order: phase-A base, then the DPOS overlay
    chain at the phase-B switch — so EVERY subsequent subprocess (Runner writes AND bare exec
    reads) inherits it exactly like the bash `export COMPOSE_FILE` children did."""
    monkeypatch.setenv("COMPOSE_FILE", "")            # snapshot+auto-restore the process env key
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    r = Runner(dry=True)
    bu = BringUp(cfg.stack_spec(), r)
    seq = []
    orig = BringUp._set_compose

    def spy(self, value):
        orig(self, value)
        seq.append((value, os.environ["COMPOSE_FILE"], self.p.env["COMPOSE_FILE"]))

    monkeypatch.setattr(BringUp, "_set_compose", spy)
    bu.run()
    # order: base first (phase A), dpos chain second (phase B switch); each stamped BOTH env sinks.
    assert [v for v, _, _ in seq] == [BASE_COMPOSE, DPOS_COMPOSE]
    assert seq[0][1] == BASE_COMPOSE and seq[0][2] == BASE_COMPOSE
    assert seq[1][1] == DPOS_COMPOSE and seq[1][2] == DPOS_COMPOSE
    # end-state: the process-global env carries the DPOS chain (`:`-separated, relative names).
    assert os.environ["COMPOSE_FILE"] == DPOS_COMPOSE
    assert DPOS_COMPOSE == "docker-compose.sim.gen.yml:docker-compose.sim.dpos.gen.yml"


def test_exec_read_inherits_compose_file_after_bringup(monkeypatch):
    """Argv/env oracle: a bare `docker compose exec` READ invoked AFTER bring-up carries the DPOS
    COMPOSE_FILE in its inherited env — rpc._run_read passes no env=, so the child inherits
    os.environ, which the phase-B switch has stamped (was: unset → wrong project → fin=-1)."""
    from dpos_harness.core import rpc
    monkeypatch.setenv("COMPOSE_FILE", "")
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    r = Runner(dry=True)
    BringUp(cfg.stack_spec(), r).run()
    seen = {}

    def fake_run(cmd, **kw):
        seen["argv"] = list(cmd)
        # The child's env is what the seam BUILT, not what this process happens to hold — that is
        # the property the fin=-1 incident was about, so read it from the call, not from os.environ.
        seen["compose_file"] = kw["env"].get("COMPOSE_FILE")
        class _R:  # noqa: D401
            stdout = "{}"
            stderr = ""
            returncode = 0
        return _R()

    monkeypatch.setattr(proc.subprocess, "run", fake_run)
    rpc.rpc_post_exec(rpc.rpc_body("eth_getBlockByNumber", ["0x1", False]),
                      rpc.compose_exec("validator-3"))
    assert seen["argv"][:5] == ["docker", "compose", "exec", "-T", "validator-3"]
    assert seen["compose_file"] == DPOS_COMPOSE


def test_resolve_compose_env_honors_exported(monkeypatch):
    monkeypatch.setenv("COMPOSE_FILE", "custom.yml")
    assert resolve_compose_env() == "custom.yml"
    assert os.environ["COMPOSE_FILE"] == "custom.yml"   # unchanged


def test_resolve_compose_env_discovers_dpos_chain(monkeypatch, tmp_path):
    """Attach path (shadow/status, no bring-up): with the generated compose PAIR on disk, resolve
    to the DPOS overlay chain (a running sim is in phase B) — mirroring soak-status.sh's on-disk
    discovery — and stamp os.environ so bare exec reads resolve the project."""
    monkeypatch.setenv("COMPOSE_FILE", "")
    monkeypatch.delenv("COMPOSE_FILE")                  # truly-unset the attach precondition
    (tmp_path / "docker-compose.sim.gen.yml").write_text("x")
    (tmp_path / "docker-compose.sim.dpos.gen.yml").write_text("x")
    assert resolve_compose_env(str(tmp_path)) == DPOS_COMPOSE
    assert os.environ["COMPOSE_FILE"] == DPOS_COMPOSE


def test_resolve_compose_env_base_only(monkeypatch, tmp_path):
    monkeypatch.setenv("COMPOSE_FILE", "")
    monkeypatch.delenv("COMPOSE_FILE")
    (tmp_path / "docker-compose.sim.gen.yml").write_text("x")
    assert resolve_compose_env(str(tmp_path)) == BASE_COMPOSE
    assert os.environ["COMPOSE_FILE"] == BASE_COMPOSE


def test_resolve_compose_env_nothing_generated(monkeypatch, tmp_path):
    monkeypatch.setenv("COMPOSE_FILE", "")
    monkeypatch.delenv("COMPOSE_FILE")
    assert resolve_compose_env(str(tmp_path)) == ""
    assert "COMPOSE_FILE" not in os.environ


# ── _wait_aligned multi-node alignment poll (lib.sh:146-163 port) ────────────
def _live_wait_env(monkeypatch, step=0.4):
    """A live (non-dry) BringUp with a monotonic clock that advances `step` per call and sleep
    neutralized — exercises the real poll loop without wall-time."""
    from dpos_harness.stack import bringup as bmod
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    bu = BringUp(cfg.stack_spec(), Runner(dry=True))
    bu.p.dry = False
    clock = {"t": 0.0}

    def mono():
        clock["t"] += step
        return clock["t"]

    slept = []
    monkeypatch.setattr(bmod.time, "monotonic", mono)
    monkeypatch.setattr(bmod.time, "sleep", lambda s: slept.append(s))
    return bu, slept


def test_wait_aligned_passes_when_aligned_past_floor(monkeypatch):
    """All readings identical, head non-null and strictly past the hex floor → returns the reading
    on the first poll (no sleep)."""
    bu, slept = _live_wait_env(monkeypatch)
    reader = lambda: [("a", "0x10|0xhh"), ("b", "0x10|0xhh")]  # noqa: E731
    assert bu._wait_aligned(30, "0x0f", reader) == "0x10|0xhh"   # 0x10=16 > 0x0f=15
    assert slept == []


def test_wait_aligned_polls_until_laggard_catches_up(monkeypatch):
    """A lagging node keeps the poll going (sleeping between reads); once it catches up and all
    readings equal AND past the floor, the poll returns — sleeping only BETWEEN the unaligned reads."""
    bu, slept = _live_wait_env(monkeypatch)
    seq = [
        [("a", "0x20|0xh"), ("b", "0x1f|0xg")],     # b lags by one block
        [("a", "0x20|0xh"), ("b", "0x1f|0xg")],     # still lagging
        [("a", "0x21|0xh2"), ("b", "0x21|0xh2")],   # converged, past floor
    ]
    it = iter(seq)
    assert bu._wait_aligned(60, "0x10", lambda: next(it)) == "0x21|0xh2"
    assert len(slept) == 2   # between the two unaligned polls, NOT after success


def test_wait_aligned_null_head_never_aligns(monkeypatch):
    """A null head (unreachable node sentinel) is NOT convergence even when all readings are equal —
    the poll rejects it and times out loudly."""
    from dpos_harness.chain.writes import ChainError
    bu, _ = _live_wait_env(monkeypatch)
    reader = lambda: [("a", "null|null"), ("b", "null|null")]  # noqa: E731
    with pytest.raises(ChainError):
        bu._wait_aligned(1, "", reader)


def test_wait_aligned_genesis_head_never_aligns(monkeypatch):
    """0x0 (genesis) is likewise rejected (bash `head != 0x0`)."""
    from dpos_harness.chain.writes import ChainError
    bu, _ = _live_wait_env(monkeypatch)
    reader = lambda: [("a", "0x0|0x0"), ("b", "0x0|0x0")]  # noqa: E731
    with pytest.raises(ChainError):
        bu._wait_aligned(1, "", reader)


def test_wait_aligned_timeout_names_divergent(monkeypatch):
    """On expiry the ChainError names every node's last reading (so the divergent/lagging one is
    visible) and states the floor — bash's loud-on-expiry diagnostic quality."""
    from dpos_harness.chain.writes import ChainError
    bu, _ = _live_wait_env(monkeypatch)
    reader = lambda: [("validator-0@8545", "0x20|0xh"), ("validator-3", "0x1f|0xg")]  # noqa: E731
    with pytest.raises(ChainError) as ei:
        bu._wait_aligned(1, "0x10", reader)
    msg = str(ei.value)
    assert "validator-3" in msg and "0x1f" in msg
    assert "validator-0@8545" in msg
    assert "past floor 0x10" in msg


def test_wait_aligned_dry_is_noop(monkeypatch):
    """Dry → no reads, no sleeps, immediate True (the transcript is about WRITE commands)."""
    bu = BringUp(_spec(validators=4, initial_committee=3, spares=1, rotation_slots=1),
                 Runner(dry=True))
    boom = lambda: (_ for _ in ()).throw(AssertionError("no reads in dry"))  # noqa: E731
    assert bu._wait_aligned(300, "0x0", boom) is True


def test_wait_aligned_callsites_bash_faithful(monkeypatch):
    """Call-site oracle: each live `_wait_aligned` is wired with the bash-faithful (timeout, floor,
    reader) triple. converge_wait = 180 + val_containers*20 (case-soak.sh:703); activation reuses
    it (:862); cold-restart = +90 with the ANCHOR floor (:889-890); L3 = 150 + val_containers*15
    with the anchor floor and the cascade reader (:933-934)."""
    monkeypatch.setenv("SIM_GEO_LATENCY", "0")   # geo margin 0 → bare bash budgets
    calls = []

    def spy(self, timeout, floor, reader):
        calls.append((timeout, floor, reader.__name__))
        return True

    monkeypatch.setattr(BringUp, "_wait_aligned", spy)
    cfg = SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1)
    bu = BringUp(cfg.stack_spec(), Runner(dry=True))
    bu.run()
    c = cfg.val_containers                       # 4 + 1 + 1 = 6
    converge = 180 + c * 20                       # 300
    assert calls[0] == (converge, "", "_read_sim_nodes")            # phase-A converge
    assert calls[1] == (converge, "", "_read_sim_nodes")            # activation converge
    assert calls[2] == (converge + 90, "0x0", "_read_sim_nodes")    # cold-restart (dry anchor 0x0)
    assert calls[3] == (150 + c * 15, "0x0", "_read_cascade_node")   # L3 through L2


def test_read_sim_nodes_reader_shape(monkeypatch):
    """_read_sim_nodes = validator-0 via host 8545 + validator-1..validators-1 via exec + full-node
    via host 18545 (bash _read_sim_nodes, case-soak.sh:358-363). Labels name each node."""
    from dpos_harness.core import nodes
    monkeypatch.setattr(nodes, "check_external", lambda port: f"0x5|0xh@{port}")
    monkeypatch.setattr(nodes, "check_node", lambda svc: f"0x5|0xh@{svc}")
    bu = BringUp(_spec(validators=4, initial_committee=3, spares=1, rotation_slots=1),
                 Runner(dry=True))
    labels = [lbl for lbl, _ in bu._read_sim_nodes()]
    assert labels == ["validator-0@8545", "validator-1", "validator-2", "validator-3",
                      "full-node@18545"]


def test_read_sim_nodes_no_cascade_drops_fullnode(monkeypatch):
    from dpos_harness.core import nodes
    monkeypatch.setattr(nodes, "check_external", lambda port: "0x5|0xh")
    monkeypatch.setattr(nodes, "check_node", lambda svc: "0x5|0xh")
    bu = BringUp(_spec(validators=4, initial_committee=3, spares=1, rotation_slots=1,
                            no_cascade=1), Runner(dry=True))
    labels = [lbl for lbl, _ in bu._read_sim_nodes()]
    assert "full-node@18545" not in labels
    assert bu._read_cascade_node()[0][0] == "downstream@28545"


def test_shadow_attach_resolves_compose_without_bringup(monkeypatch, tmp_path):
    """The shadow observer attaches to a RUNNING topology WITHOUT bring-up — it must still resolve
    the ambient COMPOSE_FILE so its nodes.py exec reads hit the sim project. Drive one `--once`
    tick with the live reads stubbed and assert the on-disk discovery stamped the DPOS chain."""
    import types
    from dpos_harness.sim import shadow
    from dpos_harness.checks.battery import Battery
    monkeypatch.setenv("COMPOSE_FILE", "")
    monkeypatch.delenv("COMPOSE_FILE")
    monkeypatch.chdir(tmp_path)                        # resolve_compose_env discovers in cwd
    (tmp_path / "docker-compose.sim.gen.yml").write_text("x")
    (tmp_path / "docker-compose.sim.dpos.gen.yml").write_text("x")
    monkeypatch.setattr(shadow, "discover_running_pid", lambda: None)
    monkeypatch.setattr(shadow, "_build_ctx", lambda tick, rnd, addrs: shadow.Ctx())
    monkeypatch.setattr(shadow, "resolve_runtime_addrs", lambda: _RT_ADDRS)
    monkeypatch.setattr(Battery, "check_invariants", lambda self: True)
    monkeypatch.setattr(shadow.nodes, "finalized_dec", lambda: 0)
    args = types.SimpleNamespace(log=str(tmp_path / "shadow.log"), period=0, once=True)
    shadow.run(args)
    assert os.environ["COMPOSE_FILE"] == DPOS_COMPOSE


# ── F8: the shadow's deployed-address resolver ───────────────────────────────
_RT_ADDRS = {"STAKING_RT": "0x" + "a" * 40,
             "CHAIN_CONFIG_RT": "0x" + "b" * 40,
             "LIVENESS_RT": "0x" + "c" * 40}


def _stub_runtime_cat(monkeypatch, out):
    """Stand in for the `docker compose exec -T validator-0 cat /runtime/…` read. `out` is
    either the file body or an exception instance to raise (the unreachable-container case)."""
    from dpos_harness.sim import shadow

    def fake(path, timeout=15):
        assert path == shadow.STAKING_READER_JSON
        if isinstance(out, Exception):
            raise out
        return out
    monkeypatch.setattr(shadow, "_runtime_cat", fake)


def test_shadow_resolves_runtime_addrs_from_staking_reader_json(monkeypatch):
    """The resolver parses the file bring-up writes (sim_regen_staking_reader) into the three
    Ctx fields, lowercased."""
    from dpos_harness.sim import shadow
    body = json.dumps({"staking_address": "0x" + "A" * 40,
                       "chain_config_address": "0x" + "b" * 40,
                       "liveness_slashing_address": "0x" + "c" * 40,
                       "ignored": "x"})
    _stub_runtime_cat(monkeypatch, body)
    assert shadow.resolve_runtime_addrs() == _RT_ADDRS


@pytest.mark.parametrize("body,why", [
    ("", "empty read (container gone / compose project not resolved)"),
    ("Error: No such service: validator-0", "an error banner instead of the file"),
    ("[]", "JSON that is not an object"),
    (json.dumps({"staking_address": "0x" + "a" * 40}), "missing keys"),
    (json.dumps({"staking_address": "0x" + "0" * 40,
                 "chain_config_address": "0x" + "b" * 40,
                 "liveness_slashing_address": "0x" + "c" * 40}), "a zero address"),
    (json.dumps({"staking_address": "nope",
                 "chain_config_address": "0x" + "b" * 40,
                 "liveness_slashing_address": "0x" + "c" * 40}), "a malformed address"),
])
def test_shadow_addr_resolution_fails_loud_never_empty(monkeypatch, body, why):
    """FAIL LOUD, never "" — a shadow running the battery through codeless predeploys logs a
    confident OK every tick over a chain it cannot read (task 20260729__sim_port_gaps F8)."""
    from dpos_harness.sim import shadow
    _stub_runtime_cat(monkeypatch, body)
    with pytest.raises(shadow.ShadowAttachError):
        shadow.resolve_runtime_addrs()


def test_shadow_addr_resolution_propagates_read_failure(monkeypatch):
    """An unreadable container is the same verdict as an unparseable file — not an empty dict."""
    from dpos_harness.sim import shadow
    _stub_runtime_cat(monkeypatch, shadow.ShadowAttachError("rc=1 no such service"))
    with pytest.raises(shadow.ShadowAttachError):
        shadow.resolve_runtime_addrs()


def test_shadow_runtime_cat_execs_the_runtime_mount_container(monkeypatch):
    """The shadow's ONLY read of the live topology goes through the one container that mounts
    /runtime. Pinned as argv against the literal `validator-0` (not topology.RUNTIME_MOUNT_HOST,
    which would move with the mutation): aimed at any other service the exec fails, and the
    shadow aborts the attach on a topology detail rather than on a real fault."""
    import subprocess as sp
    from dpos_harness.sim import shadow
    seen = {}

    def fake_run(argv, **kw):
        seen["argv"] = list(argv)
        return sp.CompletedProcess(args=argv, returncode=0, stdout="{}", stderr="")

    monkeypatch.setattr(proc.subprocess, "run", fake_run)
    shadow._runtime_cat(shadow.STAKING_READER_JSON)
    assert seen["argv"] == ["docker", "compose", "exec", "-T", "validator-0",
                            "cat", "/runtime/staking-reader.json"]


def test_shadow_runtime_cat_raises_on_nonzero_rc(monkeypatch):
    """_runtime_cat degrades to NOTHING: a failed exec raises rather than returning "" the way a
    read helper would (the caller cannot tell "" from an empty file)."""
    import subprocess as sp
    from dpos_harness.sim import shadow
    monkeypatch.setattr(
        proc.subprocess, "run",
        lambda *a, **k: sp.CompletedProcess(args=a[0], returncode=1, stdout="", stderr="boom"))
    with pytest.raises(shadow.ShadowAttachError):
        shadow._runtime_cat(shadow.STAKING_READER_JSON)


def test_shadow_run_aborts_without_addresses(monkeypatch, tmp_path):
    """The whole point: an unresolvable attach must NOT enter the tick loop. Non-zero exit, an
    ABORT line in the shadow log, and check_invariants never called."""
    import types
    from dpos_harness.sim import shadow
    from dpos_harness.checks.battery import Battery
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(shadow, "discover_running_pid", lambda: None)
    _stub_runtime_cat(monkeypatch, "")
    ticked = []
    monkeypatch.setattr(Battery, "check_invariants", lambda self: ticked.append(1) or True)
    log = tmp_path / "shadow.log"
    args = types.SimpleNamespace(log=str(log), period=0, once=True)
    assert shadow.run(args) == 2
    assert ticked == []
    assert "ABORT" in log.read_text()


def test_shadow_ctx_carries_the_resolved_addresses(monkeypatch):
    """_build_ctx must put them where the battery's cast seams read them, and the epoch read must
    go through the DEPLOYED ChainConfig — not the codeless predeploy."""
    from dpos_harness.sim import shadow
    seen = []
    monkeypatch.setattr(shadow.nodes, "finalized_dec", lambda: 100)
    monkeypatch.setattr(shadow.nodes, "chainconfig_call",
                        lambda sig, *a, addr=None, **k: seen.append(addr) or "1")
    ctx = shadow._build_ctx(3, 3, _RT_ADDRS)
    assert (ctx.STAKING_RT, ctx.CHAIN_CONFIG_RT, ctx.LIVENESS_RT) == (
        _RT_ADDRS["STAKING_RT"], _RT_ADDRS["CHAIN_CONFIG_RT"], _RT_ADDRS["LIVENESS_RT"])
    assert seen == [_RT_ADDRS["CHAIN_CONFIG_RT"]] * 2
