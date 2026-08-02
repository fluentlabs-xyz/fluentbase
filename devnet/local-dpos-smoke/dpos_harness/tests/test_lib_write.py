"""test_lib_write.py — the WRITE-side command oracle. Each test STUBS proc.Runner (dry, records
argv) and asserts the EXACT `cast`/`docker`/`forge` command line the bash helper issues. The bash
line is quoted in each docstring as the oracle (lib.sh / soak-actions.sh)."""

import shutil
import subprocess

import pytest

from dpos_harness.chain.writes import Chain, ChainError
from dpos_harness.core.proc import Runner


def _chain(**reads):
    r = Runner(dry=True)
    # canned reads so read-dependent flows proceed deterministically.
    r.reads = {
        "docker compose exec": "aa" * 32,          # owner-N.hex body (→ 0x<hex>)
        "cast wallet": "0x000000000000000000000000000000000000dEaD",
        "cast nonce": "0",
        "cast call": "1",
        "cast balance": "1000000000000000000",
        "cast gas-price": "1000000000",
    }
    r.reads.update(reads)
    c = Chain(runner=r, RPC="http://localhost:8545", STAKING_RT="0xSTAKE",
              CHAIN_CONFIG_RT="0xCFG", GOV_ADDR="0xGOV", TOKEN="0xTOKEN", CHAIN_ID="2026",
              PP_PEERS="6")
    return c, r


def _argvs(r):
    return [inv.argv for inv in r.log]


def _find(r, *needles):
    for a in r.log:
        line = " ".join(a.argv)
        if all(n in line for n in needles):
            return a.argv
    return None


def test_owner_key_reads_runtime_hex():
    """pp_owner_key: `docker compose exec -T validator-0 cat /runtime/keys/owner-3.hex`, 0x-prefixed."""
    c, r = _chain()
    k = c.owner_key(3)
    assert _find(r, "docker", "compose", "exec", "-T", "validator-0", "cat",
                 "/runtime/keys/owner-3.hex")
    assert k == "0x" + "aa" * 32


def test_owner_addr_derives_via_cast_wallet():
    """pp_owner_addr: `cast wallet address --private-key 0x<key>`, lowercased."""
    c, r = _chain()
    a = c.owner_addr(0)
    assert _find(r, "cast", "wallet", "address", "--private-key", "0x" + "aa" * 32)
    assert a == "0x000000000000000000000000000000000000dead"


_CK_JSON = (
    '{\n'
    '  "validatorAddress": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",\n'
    '  "blsPubkeyUncompressed": "0xa1b2c3",\n'
    '  "blsPoPUncompressed": "0xdeadbeef",\n'
    '  "peerPubkey": "0x1111111111111111111111111111111111111111111111111111111111111111",\n'
    '  "ownerKey": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"\n'
    '}\n'
)


def test_consensus_keys_one_off_genesis_init():
    """pp_consensus_keys: `docker compose run --rm --no-deps -T --entrypoint
    /usr/local/bin/genesis-bootstrap genesis-init consensus-keys --idx 7 --peers 8 --chain-id 2026`.
    --peers is sized to cover idx (max(PP_PEERS, idx+1)): run_consensus_keys ASSERTS idx<peers, so
    idx 7 at the default PP_PEERS=6 would abort → empty keys → cast parser error on the empty bytes32."""
    c, r = _chain()
    c.consensus_keys(7)
    argv = _find(r, "genesis-init", "consensus-keys")
    assert argv == ["docker", "compose", "run", "--rm", "--no-deps", "-T", "--entrypoint",
                    "/usr/local/bin/genesis-bootstrap", "genesis-init", "consensus-keys",
                    "--idx", "7", "--peers", "8", "--chain-id", "2026"]


def test_consensus_keys_peers_stays_pp_peers_when_idx_below():
    """idx below PP_PEERS keeps --peers==PP_PEERS (byte-identical to bash's fixed export)."""
    c, r = _chain()
    c.consensus_keys(3)
    argv = _find(r, "genesis-init", "consensus-keys")
    assert argv[-4:] == ["--peers", "6", "--chain-id", "2026"]


def test_setconsensuskeys_exact_arg_strings_from_ck_json():
    """register_setkeys → setConsensusKeys(address,bytes,bytes,bytes32): the bls pubkey/PoP and
    ed25519 peer key are the RAW 0x hex from pp_consensus_keys (bash `jq -r`), flat, unquoted, no
    JSON-list repr — pinned against the real genesis-bootstrap output shape."""
    c, r = _chain()
    r.reads["cast send"] = '{"status":"0x1"}'
    r.reads["cast call"] = "0x000000000000000000000000000000000000dEaD 2"   # status byte 2 (Pending)
    r.reads["docker compose run"] = _CK_JSON                                # pp_consensus_keys stdout
    c.register_setkeys(6)
    argv = _find(r, "setConsensusKeys(address,bytes,bytes,bytes32)")
    # cast send --json --rpc-url <RPC> <STAKING_RT> <sig> <addr> <bls_pub> <bls_pop> <peer> --private-key <key>
    assert argv[:6] == ["cast", "send", "--json", "--rpc-url", "http://localhost:8545", "0xSTAKE"]
    sig_i = argv.index("setConsensusKeys(address,bytes,bytes,bytes32)")
    addr, bls_pub, bls_pop, peer = argv[sig_i + 1:sig_i + 5]
    assert addr == "0x000000000000000000000000000000000000dead"          # owner_addr (cast wallet), lowercased
    assert bls_pub == "0xa1b2c3"
    assert bls_pop == "0xdeadbeef"
    assert peer == "0x1111111111111111111111111111111111111111111111111111111111111111"
    assert argv[sig_i + 5:sig_i + 7] == ["--private-key", "0x" + "aa" * 32]


def test_consensus_keys_empty_output_fails_loud():
    """A run that yields no keys (e.g. idx>=peers assert abort, docker down) raises instead of
    silently emitting empty cast args — mirrors bash `jq -r` dying under set -e."""
    import dpos_harness.core.proc as proc

    class Rec(proc.Runner):
        def run(self, argv, **kw):
            self.log.append(proc.Invocation(argv=[str(a) for a in argv]))
            return ""                                   # empty stdout (assert abort / docker down)

    rr = Rec(dry=False)
    c2 = Chain(runner=rr, RPC="http://localhost:8545", STAKING_RT="0xSTAKE", PP_PEERS="6")
    try:
        c2.consensus_keys(6)
        assert False, "expected ChainError on empty consensus-keys output"
    except ChainError as e:
        assert "consensus" in str(e).lower()


@pytest.mark.skipif(shutil.which("cast") is None, reason="cast (foundry) not on PATH")
def test_setconsensuskeys_cast_calldata_roundtrip():
    """The raw 0x-hex consensus-key args ABI-encode cleanly under cast (no chain/RPC), while the
    empty-string args (the live regression when consensus_keys returned {}) reproduce cast's
    `parser error`. Proves the fixed construction is what cast accepts."""
    sig = "setConsensusKeys(address,bytes,bytes,bytes32)"
    addr = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
    bls_pub, bls_pop = "0xa1b2c3", "0xdeadbeef"
    peer = "0x" + "11" * 32
    ok = subprocess.run(["cast", "calldata", sig, addr, bls_pub, bls_pop, peer],
                        capture_output=True, text=True)
    assert ok.returncode == 0 and ok.stdout.strip().startswith("0x225cba85")
    bad = subprocess.run(["cast", "calldata", sig, addr, "", "", ""],
                         capture_output=True, text=True)
    assert bad.returncode != 0 and "parser error" in (bad.stderr + bad.stdout).lower()


def test_token_transfer_exact_line():
    """pp_token_transfer: `cast send <token> transfer(address,uint256)(bool) <to> <amt> --rpc-url
    <RPC> --private-key <owner-0 key>`."""
    c, r = _chain()
    c.token_transfer("0xTOKEN", "0xTO", "100")
    argv = _find(r, "transfer(address,uint256)(bool)")
    assert argv == ["cast", "send", "0xTOKEN", "transfer(address,uint256)(bool)", "0xTO", "100",
                    "--rpc-url", "http://localhost:8545", "--private-key", "0x" + "aa" * 32]


def test_committee_getEpochCommittee():
    """pp_committee: `cast call <STAKING_RT> getEpochCommittee(uint64)(address[]) <epoch>
    --rpc-url <RPC>`; sorted lowercased set."""
    c, r = _chain(**{"cast call": "[0xBBBB..., 0xAAAA...]"})
    r.reads["cast call"] = "[0x00000000000000000000000000000000000000BB, " \
                           "0x00000000000000000000000000000000000000Aa]"
    out = c.committee(5)
    assert _find(r, "getEpochCommittee(uint64)(address[])", "5")
    assert out == "0x00000000000000000000000000000000000000aa " \
                  "0x00000000000000000000000000000000000000bb"


def test_send_revert_checked_argv():
    """sim_send: `cast send --json --rpc-url <RPC> <to> <sig> <args...> --private-key <key>`."""
    c, r = _chain()
    r.reads["cast send"] = '{"status":"0x1"}'
    c.send("approve", "0xTOKEN", "approve(address,uint256)(bool)", "0xSTAKE", "1", key="0xKEY")
    argv = _find(r, "approve(address,uint256)(bool)")
    assert argv == ["cast", "send", "--json", "--rpc-url", "http://localhost:8545", "0xTOKEN",
                    "approve(address,uint256)(bool)", "0xSTAKE", "1", "--private-key", "0xKEY"]


def test_gov_action_propose_vote_execute_sequence():
    """pp_gov_action: keccak → hashProposal → propose → castVote(For)×voters → execute. Assert the
    ORDER + the exact propose/execute selectors."""
    c, r = _chain()
    c.gov_action("0xCFG", "0xCALLDATA", "setX", voter_idx=[0, 1])
    lines = [" ".join(a.argv) for a in r.log]
    seq = [i for i, l in enumerate(lines)
           if "cast keccak setX" in l or "hashProposal" in l or "propose(address[]" in l
           or "castVote(uint256,uint8)" in l or "execute(address[]" in l]
    kinds = []
    for i in seq:
        l = lines[i]
        kinds.append("keccak" if "keccak" in l else "hash" if "hashProposal" in l
                     else "propose" if "propose(" in l else "vote" if "castVote" in l
                     else "execute")
    assert kinds[0] == "keccak"
    assert "hash" in kinds and "propose" in kinds
    assert kinds.count("vote") == 2                    # two voters
    assert kinds[-1] == "execute"
    prop = _find(r, "propose(address[],uint256[],bytes[],string)(uint256)")
    assert prop[:4] == ["cast", "send", "0xGOV",
                        "propose(address[],uint256[],bytes[],string)(uint256)"]


def test_register_setkeys_flow_order():
    """_sim_register_setkeys: approve → registerValidator → (status==2) → setConsensusKeys."""
    c, r = _chain()
    r.reads["cast send"] = '{"status":"0x1"}'
    r.reads["cast call"] = "0x000000000000000000000000000000000000dEaD 2"  # status byte = 2 (Pending)
    c.register_setkeys(4)
    lines = [" ".join(a.argv) for a in r.log]
    approve = next(i for i, l in enumerate(lines) if "approve(address,uint256)(bool)" in l)
    register = next(i for i, l in enumerate(lines) if "registerValidator(address,uint16,uint256)" in l)
    setkeys = next(i for i, l in enumerate(lines) if "setConsensusKeys(address,bytes,bytes,bytes32)" in l)
    assert approve < register < setkeys


def test_fund_eth_distinct_codes_floor():
    """pp_fund_eth: owner-0 balance below the floor → rc 1 (floor), surfaced honestly."""
    c, r = _chain()
    r.dry = False
    r.reads["cast balance"] = "1"                      # 1 wei ≪ floor
    # execute the real (recorded) path with a NON-dry runner but stubbed subprocess:
    import dpos_harness.core.proc as proc

    class Rec(proc.Runner):
        def run(self, argv, **kw):
            self.log.append(proc.Invocation(argv=[str(a) for a in argv]))
            return self.reads.get(" ".join(argv[:2]), "")

    rr = Rec(dry=False)
    rr.reads = {"cast balance": "1", "docker compose": "aa" * 32,
                "cast wallet": "0x00000000000000000000000000000000000000de"}
    c2 = Chain(runner=rr, RPC="http://localhost:8545")
    assert c2.fund_eth("0xTO", 999) == 1


def test_dry_runner_records_but_does_not_execute(tmp_path):
    """The dry seam records the argv (the oracle) and never shells out (no live topology)."""
    c, r = _chain()
    c.token_transfer("0xT", "0xTo", 1)
    assert len(r.log) >= 1
    assert r.log[0].argv[0] in ("docker", "cast")   # owner_key read is first


# ── selection-view epoch-purity probe (2026-07-21 dkg_logs idx-stall regression) ──
def _probe_chain(views, epoch=7):
    """A Chain with the two probe reads and the governance write STUBBED: `views` is the queue
    selection_view_at() returns (None models an unreadable RPC). Records the gov calls."""
    from dpos_harness.core.proc import Invocation
    c, r = _chain()
    q = list(views)
    c.staking_current_epoch = lambda: epoch
    c.selection_view_at = lambda e: q.pop(0)
    c.calldata = lambda sig, *a: f"0x{sig}"
    c.gov_action = lambda *a, **kw: r.log.append(
        Invocation(argv=["<gov-action>", *[str(x) for x in a]]))
    return c, r


def test_selection_view_purity_fails_the_run_on_a_changed_view():
    """soak-actions.sh growth branch: a selection view that MOVED across setActiveValidatorsLength
    means the cap leg is live again — the proven idx-stall class. Fail-loud, never a diagnostic."""
    c, _ = _probe_chain(["[0xa] [(0x1,0x2,3)]", "[0xa 0xb] [(0x1,0x2,3) (0x4,0x5,6)]"])
    with pytest.raises(ChainError) as e:
        c._cap_raise_with_purity_probe(9, 5, 6, None)
    assert e.value.reason_id == "selection-view-purity"
    assert "epoch 7 CHANGED" in e.value.message and "2026-07-21" in e.value.message


def test_selection_view_purity_fail_id_is_not_demoted():
    """The probe guards a proven stall class, so its id must never be swallowed into a diagnostic
    by the liveness-first policy (DEMOTED_INVARIANTS only ever shrinks)."""
    from dpos_harness.core.policy import DEMOTED_INVARIANTS
    assert "selection-view-purity" not in DEMOTED_INVARIANTS


@pytest.mark.parametrize("views,epoch", [
    ([None, "[0xa] [(0x1,0x2,3)]"], 7),      # before-read failed
    (["[0xa] [(0x1,0x2,3)]", None], 7),      # after-read failed
    ([None, None], None),                    # currentEpoch itself unreadable
])
def test_selection_view_purity_skips_on_an_unreadable_read(views, epoch, capsys):
    """An RPC brownout must WARN and SKIP, never be reported as a purity violation — the whole
    difference between a useful guard and a flaky one (bash `|| echo READ_FAILED`)."""
    c, _ = _probe_chain(views, epoch=epoch)
    c._cap_raise_with_purity_probe(9, 5, 6, None)          # no raise
    out = capsys.readouterr().out
    assert "SKIPPED" in out and "cap 5->6" in out


def test_selection_view_purity_passes_and_reports_the_epoch(capsys):
    """A stable view passes and the success line still names the cap move AND the probed epoch."""
    view = "[0xa 0xb] [(0x1,0x2,3) (0x4,0x5,6)]"
    c, r = _probe_chain([view, view])
    c._cap_raise_with_purity_probe(9, 5, 6, None)
    out = capsys.readouterr().out
    assert "committee cap 5->6" in out and "EffBal lands @E+3" in out
    assert "selection-view purity @epoch 7 OK" in out
    assert _find(r, "<gov-action>", "setActiveValidatorsLength(uint32)")


def test_selection_view_read_returns_none_on_a_failed_call():
    """selection_view_at keeps the READ-FAILED sentinel DISTINCT from a read view (rc!=0 → None),
    which is what lets the probe skip instead of comparing garbage."""
    import dpos_harness.core.proc as proc

    class Rec(proc.Runner):
        def run_capture(self, argv, **kw):
            self.log.append(proc.Invocation(argv=[str(a) for a in argv]))
            return proc.RunResult(argv=[str(a) for a in argv], stderr="server error", rc=1)

    c2 = Chain(runner=Rec(dry=False), RPC="http://localhost:8545", STAKING_RT="0xSTAKE")
    assert c2.selection_view_at(7) is None
    assert c2.staking_current_epoch() is None


def test_growth_cap_raise_brackets_the_write_with_the_probe_reads():
    """WIRING: on the GROWTH path the two getValidatorsWithKeysAt reads bracket the
    setActiveValidatorsLength calldata (before/after), and the epoch read precedes them."""
    c, r = _chain()
    c.validator_status = lambda addr: "2"          # register post-assert (canned reads are scalar)
    c.register_activate(6, raise_cap=1)
    lines = [" ".join(a.argv) for a in r.log]
    view = [i for i, ln in enumerate(lines) if "getValidatorsWithKeysAt(uint64)" in ln]
    cd = [i for i, ln in enumerate(lines) if "calldata setActiveValidatorsLength(uint32)" in ln]
    ep = [i for i, ln in enumerate(lines) if "currentEpoch()(uint64)" in ln]
    assert len(view) == 2 and len(cd) == 1 and len(ep) == 1
    assert ep[0] < view[0] < cd[0] < view[1]


def test_refill_path_runs_no_purity_probe():
    """REFILL (raise_cap=0) issues no cap-raise, so it must issue no probe read either."""
    c, r = _chain()
    c.validator_status = lambda addr: "2"
    c.register_activate(6, raise_cap=0)
    lines = " ".join(" ".join(a.argv) for a in r.log)
    assert "getValidatorsWithKeysAt" not in lines and "setActiveValidatorsLength" not in lines


def test_current_epoch_parses_the_hex_head_through_the_shared_helper():
    """`Chain.current_epoch` had NO test: its `_hex_to_dec` call site was the one former copy the
    dedup pass could not prove covered (breaking the merged helper left this path green). The head
    arrives as a 0x hex string from check_external, so a decimal-only parse silently pins the epoch
    to 0 — the "false watchdog trip" class the memoization exists to avoid."""
    c, _r = _chain()
    c._head_hex = lambda: "0x1388"                 # 5000
    c._pp_cfg_read_retry = lambda sig: 32 if "Interval" in sig else 360
    assert c.current_epoch() == (5000 - 360) // 32
