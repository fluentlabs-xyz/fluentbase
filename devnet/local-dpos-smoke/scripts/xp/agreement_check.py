#!/usr/bin/env python3
"""Agreement checks for the values declared on BOTH sides of the node/contract boundary
(`.dpos-study/DUPLICATES.md`, 14 groups; R-113..R-119).

    agreement_check.py                 offline: node sources vs contract sources vs the
                                       shipped rWasm blob vs the pinned commonware checkout
    agreement_check.py --rpc URL       ...plus the LIVE half against a running stand
    agreement_check.py --stand N       --rpc http://localhost:28000+N (a scripts/xp stand)

REWRITTEN 2026-09-09 (Э2.1/Э2.2). The contract now lives IN this repo
(`contracts/staking/src`, merge `f16fdd90`), and — the bigger change — groups 3 and 5-14
no longer HAVE two declarations to compare: `crates/staking-abi` holds the one `sol!` and
`crates/types/src/staking_protocol.rs` the one copy of each limit, and both sides import
them. So the offline half changed job: instead of comparing two literals, it checks that
neither side has grown a literal BACK, and that the single declaration is the one the
shipped blob actually dispatches on.

That change is why this file had to move at all: before it, `node_signatures()` read
`sol!` blocks out of `evm.rs` and `reader.rs`, which no longer have any — so G3 found zero
signatures and reported `[ok ] 0 node signatures …`. A check that passes vacuously is
worse than one that is missing, hence `NoSignatures` below.

`STAKING_CONTRACT_SRC` still overrides the contract path, for running this against another
checkout; it now defaults to this repo's `contracts/staking/src`.

Offline (no stand):
  G3   ABI — every call declared in `crates/staking-abi` has its 4-byte selector in the
       shipped `fluentbase_contracts_staking.rwasm` (little-endian word, the rWasm
       dispatcher's own representation), AND the contract's `consts.rs` derives its
       `SIG_*` from that crate (`sig::<abi::…>()`) rather than from a string of its own.
       Fails, never passes vacuously, when zero declarations are found.
  G4   `getEpochCommitteeWithStakes` return SHAPE — the one place where two independent
       spellings survive on purpose: the shared `sol!` return tuple vs the contract
       handler's `write_returns(sdk, &(…))` Rust tuple, which its own codec encodes.
  G1   evidence-signature namespace: prefix literal (node bls crate vs contract), suffixes
       (commonware `scheme/mod.rs` vs contract), chain-id byte order (both `to_be_bytes`).
  G5-8, G11, G13, G14  numeric literals: MIN_COMMITTEE_LENGTH, BALANCE_COMPACT_PRECISION,
       the u112 compact-stake width, the committee cap 51, the commit horizon (`+ 2` literal
       vs MAX_COMMITTEE_LOOKAHEAD_EPOCHS), the four BLS wire sizes, the 32-byte proposal
       payload.
  G9   the contract re-exports `fluentbase_types::SYSTEM_ADDRESS` and has not grown its
       own `SYSTEM_CALLER` literal back.
  G12  fault budget `(n-1)/3`: commonware `N3f1::max_faults` vs the SHARED
       `staking_protocol::fault_tolerance` the contract imports.

Live (a running stand — the half a source scan cannot reach):
  L3   dispatch: an `eth_call` of every signature reaches ITS handler — a view returns
       data, a system call from a non-system sender reverts `OnlySystemCall()`, and from
       the system sender reverts with anything but `UnknownMethod()`/`OnlySystemCall()`.
  L9   the contract's system-caller gate opens for exactly the address the node uses.
  L2   `committee[e]` is ordered by peer pubkey ascending BYTEWISE — the commonware
       `Participant` order the node resolves `signerIdx`/`leaderIndex` in.
  L4   the node's four-array ABI decodes the live `getEpochCommitteeWithStakes` answer.
  L10  the node's epoch formula agrees with the contract's `currentEpoch()` at every
       sampled block height.
  L11  the commit cursor sits at `currentEpoch + 3` at every sampled height: the node's
       `+ 2` horizon literal and the contract's lookahead constant agree in effect.

Exit code: 0 all checks agree, 1 a disagreement, 3 an input could not be read (a check
that cannot read one side is REPORTED as unread and fails the run — never scored as agree).
"""
import glob
import os
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SMOKE = HERE.parent.parent
REPO = SMOKE.parent.parent
#: The staking contract sources. In-tree since the merge `f16fdd90`; the environment
#: variable stays so this can be pointed at another checkout.
CONTRACT = Path(os.environ["STAKING_CONTRACT_SRC"]) if os.environ.get("STAKING_CONTRACT_SRC") \
    else REPO / "contracts/staking/src"
BLOB = SMOKE / "contracts" / "fluentbase_contracts_staking.rwasm"
STAKING = "0x0000000000000000000000000000000000520011"

ABI_RS = REPO / "crates/staking-abi/src/lib.rs"
PROTOCOL_RS = REPO / "crates/types/src/staking_protocol.rs"
EVM_RS = REPO / "crates/node/src/evm.rs"
READER_RS = REPO / "crates/dpos/staking-reader/src/reader.rs"
BLS_RS = REPO / "crates/dpos/bls/src/lib.rs"
P2P_CONST_RS = REPO / "crates/dpos/p2p/src/constants.rs"
DIGEST_RS = REPO / "crates/dpos/consensus/src/digest.rs"
TYPES_RS = REPO / "crates/types/src/lib.rs"

results = []


def report(gid, ok, detail):
    results.append((gid, ok, detail))
    print(f"  [{'ok ' if ok else 'FAIL'}] {gid}: {detail}", flush=True)


class Unreadable(Exception):
    pass


class NoSignatures(Exception):
    """G3 found nothing to check. A check with an empty subject must fail, not pass:
    that is exactly how this file reported `[ok ] 0 node signatures` for the whole of
    2026-09-09 after the `sol!` blocks it parsed moved to `crates/staking-abi`."""


class Usage(Exception):
    pass


def read(path: Path) -> str:
    try:
        return path.read_text()
    except OSError as e:
        raise Unreadable(f"{path}: {e}")


def grab(text: str, pattern: str, what: str, flags=re.S):
    m = re.search(pattern, text, flags)
    if not m:
        raise Unreadable(f"{what}: pattern {pattern!r} not found")
    return m.group(1)


def contract_src(name: str) -> Path:
    """A file under the staking contract sources (in-tree by default)."""
    if CONTRACT is None:
        raise Unreadable("STAKING_CONTRACT_SRC is unset and the in-tree default is missing")
    return CONTRACT / name


def commonware_root() -> Path:
    lock = read(REPO / "Cargo.lock")
    rev = grab(lock,
               r'name = "commonware-consensus"\nversion = "[^"]+"\n'
               r'source = "git\+[^#]+#([0-9a-f]+)"',
               "Cargo.lock commonware-consensus rev")
    hits = glob.glob(os.path.expanduser(f"~/.cargo/git/checkouts/monorepo-*/{rev[:7]}"))
    if not hits:
        raise Unreadable(
            f"commonware checkout for rev {rev[:7]} not found under ~/.cargo/git/checkouts")
    return Path(hits[0])


def cast(*args, timeout=30):
    r = subprocess.run(["cast", *args], capture_output=True, text=True, timeout=timeout)
    return r.returncode, r.stdout.strip(), r.stderr.strip()


def selector(sig: str) -> bytes:
    rc, out, err = cast("sig", sig)
    if rc != 0:
        raise Unreadable(f"cast sig {sig!r}: {err}")
    return bytes.fromhex(out[2:])


# ── offline ──────────────────────────────────────────────────────────────────────

def sol_blocks(text: str):
    """The bodies of every `sol!` macro invocation in `text`, brace-matched.

    Scanning the WHOLE file instead would let any `function name(...)` written in a comment or
    a doc line become a signature this check then demands the contract declare — a FAIL that
    describes prose, not the ABI."""
    out, i = [], 0
    while True:
        m = re.search(r"\bsol!\s*\{", text[i:])
        if not m:
            return out
        start = i + m.end()                     # just past the opening brace
        depth, j = 1, start
        while j < len(text) and depth:
            # `//` runs to end of line and `///` doc comments are where an UNBALANCED brace
            # actually shows up (`/// the map {addr -> stake}` is fine, `{addr` is not). Counting
            # inside them either walks past the macro — and then `node_signatures` reads ordinary
            # Rust as ABI, the fail-OPEN this scoping exists to remove — or never closes.
            if text.startswith("//", j):
                nl = text.find("\n", j)
                j = len(text) if nl < 0 else nl + 1
                continue
            depth += (text[j] == "{") - (text[j] == "}")
            j += 1
        if depth:
            raise Unreadable("a `sol!` block is never closed — refusing to read half of one")
        out.append(text[start:j - 1])
        i = j


def shared_signatures():
    """Every `function name(params)` in the ONE shared declaration
    (`crates/staking-abi`), as canonical signature strings (types only).

    Raises `NoSignatures` on an empty result. The declaration moving again — which is
    what happened on 2026-09-09 — must stop this run, not quietly empty it."""
    text = "\n".join(sol_blocks(read(ABI_RS)))
    sigs = {}
    for m in re.finditer(r"function\s+(\w+)\s*\(([^)]*)\)", text):
        name, params = m.group(1), m.group(2)
        types = []
        for p in params.split(","):
            p = p.strip()
            if p:
                types.append(p.split()[0])
        sigs[f"{name}({','.join(types)})"] = ABI_RS.name
    if not sigs:
        raise NoSignatures(
            f"{ABI_RS} declares no `function` — either the shared `sol!` moved again or "
            "this parser broke. Refusing to report a vacuous pass.")
    return sigs


def check_g3():
    sigs = shared_signatures()
    consts = read(contract_src("consts.rs"))
    blob = BLOB.read_bytes() if BLOB.exists() else None
    if blob is None:
        raise Unreadable(f"{BLOB} missing")
    # The contract must take these selectors from the shared crate, not re-derive them
    # from a string of its own. `sig::<abi::fooCall>()` is the only accepted spelling.
    imported = set(re.findall(r"sig::<abi::(\w+)Call>\(\)", consts))
    strings = set(re.findall(r'derive_keccak256_id!\(\s*"([^"]+)"', consts))
    bad = []
    for sig in sorted(sigs):
        name = sig.split("(")[0]
        sel = selector(sig)
        n_blob = blob.count(sel[::-1])            # rWasm stores the u32 little-endian
        if n_blob != 1:
            bad.append(f"{sig}: blob-selector-0x{sel.hex()} occurs {n_blob}x, want 1")
        if name not in imported:
            # `getUndelegatePeriod` is declared by neither side for the node, so it is
            # not in the shared crate at all; anything that IS there must be imported.
            bad.append(f"{sig}: consts.rs does not derive it via sig::<abi::{name}Call>()")
        if sig in strings:
            bad.append(f"{sig}: consts.rs ALSO re-derives it from a string — second "
                       "declaration grew back")
    report("G3 ABI signatures", not bad,
           f"{len(sigs)} shared declarations, each imported by consts.rs and present "
           f"exactly once in the blob" if not bad else "; ".join(bad))


def check_g4():
    node = grab(read(ABI_RS),
                r"function getEpochCommitteeWithStakes\(uint64 epoch\)\s*"
                r"external view returns \(([^;]*)\);",
                "shared getEpochCommitteeWithStakes returns")
    node_arity = len([t for t in node.split(",") if t.strip()])
    body = grab(read(contract_src("consensus.rs")),
                r"pub fn get_epoch_committee_with_stakes<SDK: SharedAPI>\((.*?)\n}\n",
                "contract handler")
    tup = grab(body, r"write_returns\(sdk,\s*&\((.*?)\)\)", "contract write_returns tuple")
    contract_arity = len([t for t in tup.split(",") if t.strip()])
    report("G4 getEpochCommitteeWithStakes arity", node_arity == contract_arity,
           f"node returns {node_arity} arrays ({node.strip()}), "
           f"contract writes {contract_arity} ({tup.strip()})")


def check_g1():
    node_prefix = grab(
    read(BLS_RS),
    r'ns\.extend_from_slice\(b"([^"]+)"\);',
     "node namespace prefix")
    node_chain = "to_be_bytes" in grab(
    read(BLS_RS),
    r"pub fn fluent_namespace\(chain_id: u64\) -> Vec<u8> \{(.*?)\n\}",
     "fluent_namespace body")
    cbody = grab(
    read(
        contract_src("consensus.rs")),
        r"fn namespace<SDK: SharedAPI>\(sdk: &SDK, kind: u8\) -> Bytes \{(.*?)\n\}",
         "contract namespace fn")
    c_prefix = grab(cbody, r'b"([^"]+)"\.to_vec\(\)', "contract prefix")
    c_chain = "to_be_bytes" in cbody
    c_suffixes = re.findall(r'=>\s*b"(_[A-Z]+)"', cbody)
    cw = read(commonware_root() / "consensus/src/simplex/scheme/mod.rs")
    cw_suffixes = {k: v for k, v in re.findall(r'const (\w+)_SUFFIX: &\[u8\] = b"([^"]+)";', cw)}
    want = [cw_suffixes.get("NOTARIZE"), cw_suffixes.get("NULLIFY"), cw_suffixes.get("FINALIZE")]
    ok = (node_prefix == c_prefix and node_chain and c_chain and c_suffixes == want)
    report("G1 evidence namespace", ok,
           f"prefix node={
    node_prefix!r} contract={
        c_prefix!r}; chain-id big-endian node={node_chain} "
           f"contract={c_chain}; suffixes contract={c_suffixes} commonware={want}")


def literal(text, pattern, what):
    return int(grab(text, pattern, what).replace("_", ""))


def check_numbers():
    """Groups 5-8, 11, 13, 14 — the numeric limits.

    Rewritten 2026-09-09: there is one declaration each, in
    `crates/types/src/staking_protocol.rs`, and every consumer `pub use`s it. So the
    question is no longer "are the two literals equal" but "has either side grown its own
    literal back, and does the shared value still hold". Both are checked; a re-grown
    literal is the failure this whole file exists to catch, and it is now the LOUD case
    rather than a silent agreement between two numbers that happen to match."""
    protocol = read(PROTOCOL_RS)
    reader, consts = read(READER_RS), read(contract_src("consts.rs"))
    evm, bls = read(EVM_RS), read(BLS_RS)
    p2p, digest = read(P2P_CONST_RS), read(DIGEST_RS)
    math = read(contract_src("math.rs"))
    storage = read(contract_src("storage.rs"))

    def shared(name, pattern, what):
        return literal(protocol, pattern, what)

    def imports(text, name, where):
        """`name` reaches `text` through a `use`/`pub use` of the shared module rather
        than through a declaration of its own."""
        own = re.search(rf"pub const {name}\s*:", text)
        used = re.search(rf"(?:pub )?use\s+[\w:]*staking_protocol::[^;]*\b{name}\b", text,
                         re.S)
        return (used is not None and own is None), where

    checks = []

    v = shared("MIN_COMMITTEE_LENGTH",
               r"pub const MIN_COMMITTEE_LENGTH: usize = (\d+);", "shared floor")
    ok_r, _ = imports(reader, "MIN_COMMITTEE_LENGTH", "reader.rs")
    ok_c, _ = imports(consts, "MIN_COMMITTEE_LENGTH", "consts.rs")
    report("G5 MIN_COMMITTEE_LENGTH", v == 4 and ok_r and ok_c,
           f"shared={v}; imported by reader.rs={ok_r} consts.rs={ok_c}")

    v = shared("BALANCE_COMPACT_PRECISION",
               r"pub const BALANCE_COMPACT_PRECISION: u128 = ([\d_]+);", "shared precision")
    ok_r, _ = imports(reader, "BALANCE_COMPACT_PRECISION", "reader.rs")
    derived = "staking_protocol::BALANCE_COMPACT_PRECISION_U256" in consts
    report("G6 BALANCE_COMPACT_PRECISION", v == 10_000_000_000 and ok_r and derived,
           f"shared={v}; reader.rs imports={ok_r}; consts.rs uses the derived U256={derived}")

    v = shared("COMPACT_STAKE_BITS", r"pub const COMPACT_STAKE_BITS: usize = (\d+);",
               "shared compact-stake width")
    ok_r, _ = imports(reader, "MAX_COMPACT_STAKE", "reader.rs")
    ok_m = "staking_protocol::COMPACT_STAKE_BITS" in math
    ok_s = "StorageUint112" in storage
    report("G7 compact-stake width", v == 112 and ok_r and ok_m and ok_s,
           f"shared={v}; reader.rs imports MAX_COMPACT_STAKE={ok_r}; "
           f"math.rs U112 built from it={ok_m}; storage.rs uses StorageUint112={ok_s}")

    v = shared("MAX_COMMITTEE_SIZE", r"pub const MAX_COMMITTEE_SIZE: u64 = (\d+);", "shared cap")
    ok_p, _ = imports(p2p, "MAX_COMMITTEE_SIZE", "p2p/constants.rs")
    ok_c, _ = imports(consts, "MAX_COMMITTEE_SIZE", "consts.rs")
    gone = "MAX_ACTIVE_VALIDATORS_LENGTH" not in consts
    report("G8 committee cap", v == 51 and ok_p and ok_c and gone,
           f"shared={v}; imported by p2p={ok_p} consts.rs={ok_c}; "
           f"old contract name retired={gone}")

    body = grab(evm, r"fn drive_ahead_commit\((.*?)\n}\n", "drive_ahead_commit body")
    ok_n = "MAX_COMMITTEE_LOOKAHEAD_EPOCHS" in body and not re.search(
        r"current_epoch \+ \d+", body)
    v = shared("MAX_COMMITTEE_LOOKAHEAD_EPOCHS",
               r"pub const MAX_COMMITTEE_LOOKAHEAD_EPOCHS: u64 = (\d+);", "shared lookahead")
    ok_c, _ = imports(consts, "MAX_COMMITTEE_LOOKAHEAD_EPOCHS", "consts.rs")
    report("G11 commit horizon", v == 2 and ok_n and ok_c,
           f"shared={v}; evm.rs reads the constant (no literal)={ok_n}; consts.rs imports={ok_c}")

    pairs = [("BLS_PUBKEY_LENGTH", 96, "PUBKEY_BYTES"),
             ("BLS_SIGNATURE_LENGTH", 48, "SIGNATURE_BYTES"),
             ("BLS_PUBKEY_UNCOMPRESSED_LENGTH", 256, "PUBKEY_EIP2537_BYTES"),
             ("BLS_SIGNATURE_UNCOMPRESSED_LENGTH", 128, "SIGNATURE_EIP2537_BYTES")]
    detail, ok = [], True
    for name, want, alias in pairs:
        v = literal(protocol, rf"pub const {name}: usize = (\d+);", f"shared {name}")
        aliased = re.search(rf"{name} as {alias}", bls) is not None
        used_c = name in consts or (
            name == "BLS_SIGNATURE_UNCOMPRESSED_LENGTH"
            and "BLS_SIGNATURE_UNCOMPRESSED_LENGTH as BLS_POP_UNCOMPRESSED_LENGTH" in consts)
        ok &= (v == want and aliased and used_c)
        detail.append(f"{name}={v} bls-alias={aliased} consts-use={used_c}")
    report("G13 BLS wire sizes", ok, "; ".join(detail))

    v = shared("PROPOSAL_PAYLOAD_LENGTH",
               r"pub const PROPOSAL_PAYLOAD_LENGTH: usize = (\d+);", "shared payload length")
    ok_d = "staking_protocol::PROPOSAL_PAYLOAD_LENGTH" in digest
    ok_c, _ = imports(consts, "PROPOSAL_PAYLOAD_LENGTH", "consts.rs")
    report("G14 proposal payload length", v == 32 and ok_d and ok_c,
           f"shared={v}; digest.rs reads it={ok_d}; consts.rs imports={ok_c}")

    a = grab(
    read(TYPES_RS),
    r'pub const SYSTEM_ADDRESS: Address = address!\("(0x[0-9a-fA-F]{40})"\);',
     "SYSTEM_ADDRESS").lower()
    # 2026-09-09: the contract no longer carries a literal — it re-exports the shared
    # one. So the check is that it still does, and has not grown its own back.
    reexported = "SYSTEM_ADDRESS as SYSTEM_CALLER" in consts
    own = "pub const SYSTEM_CALLER: Address = address!" in consts
    report(
    "G9 system caller",
    reexported and not own,
     f"fluentbase_types::SYSTEM_ADDRESS={a}; consts.rs re-exports it={reexported}, "
     f"own literal grew back={own}")

    cw = read(commonware_root() / "utils/src/faults.rs")
    cw_body = grab(
    cw,
    r"fn max_faults\(n: impl ToPrimitive\) -> u32 \{(.*?)\n    \}",
     "N3f1::max_faults")
    # 2026-09-09: `fault_tolerance` moved to the shared crate and the contract
    # `pub use`s it, so read the body there and check the contract still imports it.
    shared_body = grab(
    read(PROTOCOL_RS),
    r"pub const fn fault_tolerance\(n: usize\) -> usize \{(.*?)\n\}",
     "shared fault_tolerance")
    a = re.search(r"\(n - 1\) / 3", cw_body) is not None
    b = re.search(r"\(n - 1\) / 3", shared_body) is not None
    c = "pub use staking_protocol::fault_tolerance" in math
    own = "pub fn fault_tolerance" in math
    report(
    "G12 fault budget",
    a and b and c and not own,
     f"commonware max_faults `(n-1)/3`={a}; shared fault_tolerance `(n-1)/3`={b}; "
     f"contract imports it={c}, own copy grew back={own}")


# ── live ─────────────────────────────────────────────────────────────────────────

SYSTEM = "0xfffffffffffffffffffffffffffffffffffffffe"
STRANGER = "0x1111111111111111111111111111111111111111"


def revert_code(err: str):
    m = re.search(r'data: "(0x[0-9a-fA-F]*)"', err)
    return m.group(1)[:10] if m else None


def live_call(rpc, sig, *args, sender=None, block=None):
    argv = ["call", "--rpc-url", rpc]
    if sender:
        argv += ["--from", sender]
    if block is not None:
        argv += ["--block", str(block)]
    return cast(*argv, STAKING, sig, *map(str, args))


def _uint(result, what):
    """The leading unsigned integer of a `(rc, stdout, stderr)` read, or `Unreadable` NAMING the
    call. `cast` right-justifies and may append a bracketed decimal form."""
    rc, out, err = result
    if rc != 0:
        raise Unreadable(f"{what}: rc={rc} {err[:200]}")
    token = (out or "").strip().split()
    if not token or not token[0].lstrip("-").isdigit():
        raise Unreadable(f"{what}: expected a number, got {out[:120]!r}")
    return int(token[0])


def check_live(rpc):
    unknown, only_system = "0x" + \
        selector("UnknownMethod()").hex(), "0x" + selector("OnlySystemCall()").hex()

    # L3/L9 — dispatch and the system-caller gate.
    views = ["getEpochCommitteeWithStakes(uint64)", "getRegistryWithKeys()", "getDkgQual(uint64)",
             "getEpochBlockInterval()", "getDposActivationBlock()", "getUndelegatePeriod()",
             "getActiveValidatorsLength()", "nextEpochToCommit()"]
    bad = []
    for sig in views:
        args = ["0"] if "uint64" in sig else []
        rc, out, err = live_call(rpc, sig, *args)
        code = revert_code(err)
        if rc != 0 and code == unknown:
            bad.append(f"{sig}: UnknownMethod")
        elif rc != 0:
            bad.append(f"{sig}: reverted {code} ({err[:80]})")
    probes = [("commitEpochCommittee()", []), ("recordProduction(uint8)", ["0"]),
              ("slashEquivocation(uint64,uint32)", ["0", "0"])]
    gate = []
    for sig, args in probes:
        rc, _, err = live_call(rpc, sig, *args, sender=STRANGER)
        c1 = revert_code(err)
        if c1 != only_system:
            bad.append(
    f"{sig} from a stranger: expected OnlySystemCall {only_system}, got {c1} (rc={rc})")
        rc, _, err = live_call(rpc, sig, *args, sender=SYSTEM)
        c2 = revert_code(err) if rc != 0 else "success"
        if c2 in (unknown, only_system):
            bad.append(f"{sig} from the system address: {c2}")
        gate.append(f"{sig}: stranger→{c1} system→{c2}")
    report("L3 dispatch reaches every handler", not bad,
           "; ".join(gate) if not bad else "; ".join(bad))
    gate_ok = all(f"stranger→{only_system} system→" in g
                  and f"system→{only_system}" not in g for g in gate)
    report("L9 system-caller gate", gate_ok,
           f"OnlySystemCall={only_system} for the stranger only, system sender = {SYSTEM}")

    # L2/L4 — committee order and the four-array decode.
    e = _uint(live_call(rpc, "currentEpoch()(uint64)"), "currentEpoch()")
    rc, out, err = live_call(
    rpc,
    "getEpochCommitteeWithStakes(uint64)"
    "(address[],(bytes,bytes32,uint64)[],uint256[],bool[])", e)
    if rc != 0:
        report("L4 four-array decode", False,
               f"the node's ABI did not decode the live answer for epoch {e}: {err[:200]}")
        report("L2 committee order", False, "skipped: undecodable committee")
    else:
        lines = out.splitlines()
        addrs = re.findall(r"0x[0-9a-fA-F]{40}", lines[0]) if lines else []
        # The tuple line carries the 96-byte BLS key (192 hex) beside the 32-byte peer key
        # (64 hex); anchor both ends so a 64-hex run INSIDE the longer key does not count.
        keys = re.findall(
    r"(?<![0-9a-fA-Fx])0x[0-9a-fA-F]{64}(?![0-9a-fA-F])",
     lines[1]) if len(lines) > 1 else []
        # `cast` annotates each uint as `1000000000000000000 [1e18]`; drop the annotation first.
        stakes = re.findall(
    r"\d+",
    re.sub(
        r"\[[^\]]*e[^\]]*\]",
        "",
         lines[2])) if len(lines) > 2 else []
        tomb = re.findall(r"true|false", lines[3]) if len(lines) > 3 else []
        ok4 = len(lines) >= 4 and len(addrs) == len(keys) == len(tomb) and len(addrs) > 0
        report("L4 four-array decode", ok4,
               f"epoch {e}: addrs={len(addrs)} keys={len(keys)} "
               f"stakes={len(stakes)} tombstoned={len(tomb)}")
        raw = [bytes.fromhex(k[2:]) for k in keys]
        ordered = all(raw[i] < raw[i + 1] for i in range(len(raw) - 1))
        report("L2 committee order", ordered and len(raw) > 1,
               f"peer pubkeys of committee[{e}] strictly ascending bytewise: "
               f"{ordered} ({len(raw)} keys)")

    # L10/L11 — epoch formula and commit horizon at sampled heights.
    #
    # Each read is checked BEFORE it is parsed. Feeding a failed call's empty stdout to `int()`
    # raised a bare ValueError that the caller reported as "[UNREAD] live: invalid literal for
    # int()" — true, and useless: it does not say which of the three calls did not answer.
    interval = _uint(live_call(rpc, "getEpochBlockInterval()(uint32)"), "getEpochBlockInterval()")
    act = _uint(live_call(rpc, "getDposActivationBlock()(uint64)"), "getDposActivationBlock()")
    head = _uint(cast("block-number", "--rpc-url", rpc), "cast block-number")
    samples = sorted({max(act - 1, 1), act, act + 1,
                      act + interval - 1, act + interval, act + interval + 1,
                      *(act + (head - act) * k // 10 for k in range(1, 10)), head})
    samples = [b for b in samples if 1 <= b <= head]
    bad10, bad11, seen = [], [], []
    for b in samples:
        rc, ce, err = live_call(rpc, "currentEpoch()(uint64)", block=b)
        rc2, nx, err2 = live_call(rpc, "nextEpochToCommit()(uint64)", block=b)
        if rc != 0 or rc2 != 0:
            bad10.append(f"block {b}: unreadable ({err[:60]} {err2[:60]})")
            continue
        ce, nx = int(ce.split()[0]), int(nx.split()[0])
        node_epoch = max(b - act, 0) // interval          # reader.rs::epoch_of_block
        if ce != node_epoch:
            bad10.append(f"block {b}: contract currentEpoch={ce} node epoch_of_block={node_epoch}")
        if b >= act and nx != ce + 3:
            bad11.append(f"block {b}: nextEpochToCommit={nx} != currentEpoch+3={ce + 3}")
        seen.append(b)
    report("L10 epoch formula", not bad10,
           f"agreed at {len(seen)} heights {seen[:4]}…{seen[-2:]} "
           f"(interval={interval}, activation={act})"
           if not bad10 else "; ".join(bad10))
    report("L11 commit horizon in effect", not bad11,
           f"nextEpochToCommit == currentEpoch + 3 at every sampled height >= {act}"
           if not bad11 else "; ".join(bad11))


def opt(argv, flag):
    """The value after `flag`, or None when the flag is absent. A flag given as the LAST
    argument has no value: that is a usage error, and `argv[i + 1]` would raise IndexError with
    a traceback instead of saying so."""
    if flag not in argv:
        return None
    i = argv.index(flag)
    if i + 1 >= len(argv):
        raise Usage(f"{flag} needs a value")
    return argv[i + 1]


def main(argv):
    try:
        rpc = opt(argv, "--rpc")
        stand = opt(argv, "--stand")
        if rpc is None and stand is not None:
            if not stand.isdigit():
                raise Usage(f"--stand takes a stand number, got {stand!r}")
            rpc = f"http://localhost:{28000 + int(stand)}"
    except Usage as e:
        print(f"usage error: {e}\n{__doc__}", flush=True)
        return 2
    print(f"=== agreement check: node={REPO} "
          f"contract={CONTRACT or '(STAKING_CONTRACT_SRC unset)'} blob={BLOB.name} "
          f"{'live rpc=' + rpc if rpc else 'offline only'} ===", flush=True)
    unread = []
    for fn in (check_g3, check_g4, check_g1):
        try:
            fn()
        except NoSignatures as e:
            # NOT an UNREAD: a check whose subject vanished has to be a FAILURE, or the
            # run goes green having verified nothing. This is the 2026-09-09 lesson.
            report(fn.__name__, False, f"nothing to check — {e}")
        except Unreadable as e:
            unread.append(str(e))
            print(f"  [UNREAD] {fn.__name__}: {e}", flush=True)
    try:
        check_numbers()
    except Unreadable as e:
        unread.append(str(e))
        print(f"  [UNREAD] numbers: {e}", flush=True)
    if rpc:
        try:
            check_live(rpc)
        except (Unreadable, subprocess.TimeoutExpired, ValueError, IndexError) as e:
            unread.append(f"live: {e}")
            print(f"  [UNREAD] live: {e}", flush=True)
    fails = [g for g, ok, _ in results if not ok]
    print(f"=== {len(results)} checks, {len(fails)} disagree, {len(unread)} unread ===", flush=True)
    if unread:
        return 3
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
