"""test_lib_write.py — the WRITE-side command oracle. Each test STUBS proc.Runner (dry, records
argv) and asserts the EXACT `cast`/`docker`/`forge` command line the bash helper issues. The bash
line is quoted in each docstring as the oracle (lib.sh / soak-actions.sh)."""

import shutil
import subprocess

import pytest

from dpos_harness.chain.writes import Chain, ChainError
from dpos_harness.core import rpc
from dpos_harness.core.proc import Runner


def _chain(_runner=None, **reads):
    r = _runner if _runner is not None else Runner(dry=True)
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


def test_register_validator_exact_arg_strings_from_ck_json():
    """register_setkeys → the 6-arg `registerValidator(address,uint16,uint256,bytes,bytes,bytes32)`:
    the bls pubkey/PoP and ed25519 peer key are the RAW 0x hex from pp_consensus_keys (bash
    `jq -r`), flat, unquoted, no JSON-list repr — pinned against the real genesis-bootstrap
    output shape.

    `setConsensusKeys` HAS NO COUNTERPART on the module; its three arguments moved here, and the
    PoP is verified inside this same call. Positions 4-6 are what this pins, because the
    six-argument form shares one selector and a transposed pair would be a silently wrong
    registration rather than a revert."""
    c, r = _chain()
    r.reads["cast send"] = '{"status":"0x1"}'
    r.reads["cast call"] = "0x000000000000000000000000000000000000dEaD 2"   # status byte 2 (Pending)
    r.reads["docker compose run"] = _CK_JSON                                # pp_consensus_keys stdout
    c.register_setkeys(6)
    sig = "registerValidator(address,uint16,uint256,bytes,bytes,bytes32)"
    argv = _find(r, sig)
    # cast send --json --rpc-url <RPC> <STAKING_RT> <sig> <addr> <commission> <stake>
    #           <bls_pub> <bls_pop> <peer> --private-key <key>
    assert argv[:6] == ["cast", "send", "--json", "--rpc-url", "http://localhost:8545", "0xSTAKE"]
    sig_i = argv.index(sig)
    addr, commission, stake, bls_pub, bls_pop, peer = argv[sig_i + 1:sig_i + 7]
    assert addr == "0x000000000000000000000000000000000000dead"          # owner_addr, lowercased
    assert (commission, stake) == ("0", "1000000000000000000")
    assert bls_pub == "0xa1b2c3"
    assert bls_pop == "0xdeadbeef"
    assert peer == "0x1111111111111111111111111111111111111111111111111111111111111111"
    assert argv[sig_i + 7:sig_i + 9] == ["--private-key", "0x" + "aa" * 32]
    assert _find(r, "setConsensusKeys") is None, "setConsensusKeys has no counterpart on the module"


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
    """_sim_register_setkeys: consensus-keys read → approve → the 6-arg registerValidator →
    (status==2).

    The keys are read BEFORE the approve, not between the approve and the register: they are now
    arguments to the register itself, and a keys read that fails must do so before any money has
    moved. The status assert survives the collapse as a swallowed-revert detector — a `cast send`
    with no receipt is confirmed by the sender's nonce advancing, and a REVERTED tx advances the
    nonce just as a successful one does."""
    c, r = _chain()
    r.reads["cast send"] = '{"status":"0x1"}'
    r.reads["cast call"] = "0x000000000000000000000000000000000000dEaD 2"  # status byte = 2 (Pending)
    c.register_setkeys(4)
    lines = [" ".join(a.argv) for a in r.log]
    keys = next(i for i, l in enumerate(lines) if "consensus-keys" in l)
    approve = next(i for i, l in enumerate(lines) if "approve(address,uint256)(bool)" in l)
    register = next(i for i, l in enumerate(lines)
                    if "registerValidator(address,uint16,uint256,bytes,bytes,bytes32)" in l)
    status = next(i for i, l in enumerate(lines) if "getValidatorStatus(address)" in l)
    assert keys < approve < register < status


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


def test_growth_raises_the_cap_and_refill_does_not():
    """WIRING: the GROWTH path issues exactly one `setActiveValidatorsLength` calldata, and it is
    aimed at the STAKING runtime.

    The target is the assertion, not a formality. `setActiveValidatorsLength` moved onto the
    staking module with the rest of ChainConfig, and a governance proposal into an address with
    no code is INVISIBLE — OZ's `Address.verifyCallResult` never checks `target.code.length`, so
    the Governor emits `ProposalExecuted`, the receipt reads 0x1, and the cap silently never
    moves. Nothing downstream would notice: the sim would keep registering validators against a
    committee that stopped growing.

    This test used to also assert that two `getEpochCommittee` reads bracketed the write — the
    selection-view epoch-purity probe, deleted 2026-09-07 because its retargeted form could not
    fail on any path. See `Chain.register_activate` for what the probe guarded and why the class
    is now closed by construction."""
    c, r = _chain()
    c.validator_status = lambda addr: "2"          # register post-assert (canned reads are scalar)
    c.register_activate(6, raise_cap=1)
    lines = [" ".join(a.argv) for a in r.log]
    cd = [ln for ln in lines if "calldata setActiveValidatorsLength(uint32)" in ln]
    assert len(cd) == 1, lines
    assert any("propose" in ln and "0xSTAKE" in ln for ln in lines), lines


def test_the_committee_read_refuses_an_answer_that_is_not_an_address_array():
    """A `cast` handed a signature the contract has outgrown decodes the PREFIX and prints it,
    rc 0. The old regex sweep over raw stdout accepted that as a committee, and `committee_has`
    then answered questions about it. An EMPTY answer still means "nobody answered" and still
    yields "" — that is an RPC brownout, not a drift, and the callers that treat "" as unreadable
    must keep being able to."""
    c, _r = _chain(**{"cast call": "0x000000000000000000000000000000000000dEaD"})
    with pytest.raises(rpc.CastDecodeError):
        c.committee(4)
    c2, _r2 = _chain(**{"cast call": ""})
    assert c2.committee(4) == ""


def test_refill_path_raises_no_cap():
    """REFILL (raise_cap=0) fills a hole under the existing cap, so it must issue no cap-raise.
    A refill that raised the cap would grow the committee on a path whose whole point is that it
    does not."""
    c, r = _chain()
    c.validator_status = lambda addr: "2"
    c.register_activate(6, raise_cap=0)
    lines = " ".join(" ".join(a.argv) for a in r.log)
    assert "setActiveValidatorsLength" not in lines


def test_current_epoch_parses_the_hex_head_through_the_shared_helper():
    """`Chain.current_epoch` had NO test: its `_hex_to_dec` call site was the one former copy the
    dedup pass could not prove covered (breaking the merged helper left this path green). The head
    arrives as a 0x hex string from check_external, so a decimal-only parse silently pins the epoch
    to 0 — the "false watchdog trip" class the memoization exists to avoid."""
    c, _r = _chain()
    c._head_hex = lambda: "0x1388"                 # 5000
    c._pp_cfg_read_retry = lambda sig: 32 if "Interval" in sig else 360
    assert c.current_epoch() == (5000 - 360) // 32
