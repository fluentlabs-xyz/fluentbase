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
  G4b  the same for `getRegistryWithKeys`, whose handler writes through a helper.
  G15  the chain-config views' return WIDTH, three spellings of one number: the `uint<N>` the
       shared `sol!` promises, the `StorageU<N>` the contract writes, and the `(uint<N>)` the
       stand asks `cast` to decode. The expected views come from the SHARED crate, so a view
       losing its `config.rs` handler is a named failure rather than a smaller subject.
  G16  every call signature the STAND spells out for `cast` — Python literals and the shell
       scripts' quoted arguments — against the shared crate first and the contract's own
       `derive_keccak256_id!` declarations second. The Python side is type-checked by nothing,
       so a stale spelling goes out under a selector the dispatcher does not know; a name in
       neither source is a failure. Also fails on a hardcoded 4-byte constant that equals a
       shared selector: the G3 rule, pointed at Python.
  G1   evidence-signature namespace: prefix literal (node bls crate vs contract), suffixes
       (commonware `scheme/mod.rs` vs contract), chain-id byte order (both `to_be_bytes`).
  G5-8, G11, G13, G14  numeric literals: MIN_COMMITTEE_LENGTH, BALANCE_COMPACT_PRECISION,
       the u112 compact-stake width, the committee cap 51, the commit horizon (`+ 2` literal
       vs MAX_COMMITTEE_LOOKAHEAD_EPOCHS), the four BLS wire sizes, the 32-byte proposal
       payload. G8 also reads the ONE Python transcription of a Rust constant that no
       `pub use` can reach — `compose_gen.py`'s committee cap — and compares it.
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
import ast
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
CONFIG_RS_NAME = "config.rs"
#: The one Python transcription of a Rust protocol constant (the committee cap). Python cannot
#: import a Rust `const`, so G8 reads this file's literal and compares — the treatment G1 gives
#: commonware's suffixes, for the same reason: no import path exists to make it unnecessary.
COMPOSE_GEN = SMOKE / "dpos_harness/stack/compose_gen.py"
#: Every directory of harness Python whose string literals G16 scans for call signatures.
HARNESS_DIRS = [SMOKE / "dpos_harness", SMOKE / "scripts"]
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


_SELECTORS = {}


def selector(sig: str) -> bytes:
    """The 4-byte selector of `sig`, from `cast` — memoised, because G3, G16 and the live half
    all ask for the same handful and each answer costs a process."""
    if sig not in _SELECTORS:
        rc, out, err = cast("sig", sig)
        if rc != 0:
            raise Unreadable(f"cast sig {sig!r}: {err}")
        _SELECTORS[sig] = bytes.fromhex(out[2:])
    return _SELECTORS[sig]


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


def shared_declarations():
    """Every `function`/`error`/`event` in the ONE shared declaration (`crates/staking-abi`),
    as `name -> (kind, canonical signature)`. Types only: `uint64 indexed epoch` is `uint64`,
    which is what the selector and the topic are hashed over.

    Raises `NoSignatures` on an empty result. The declaration moving again — which is what
    happened on 2026-09-09 — must stop this run, not quietly empty it."""
    text = "\n".join(sol_blocks(read(ABI_RS)))
    decls = {}
    for m in re.finditer(r"\b(function|error|event)\s+(\w+)\s*\(([^)]*)\)", text, re.S):
        kind, name, params = m.group(1), m.group(2), m.group(3)
        types = [p.strip().split()[0] for p in params.split(",") if p.strip()]
        decls[name] = (kind, f"{name}({','.join(types)})")
    if not decls:
        raise NoSignatures(
            f"{ABI_RS} declares no `function` — either the shared `sol!` moved again or "
            "this parser broke. Refusing to report a vacuous pass.")
    return decls


def shared_signatures():
    """The CALL half of [`shared_declarations`], as `signature -> source file` (G3's subject:
    an error selector never appears as a dispatcher entry, so the blob scan must not ask for
    one)."""
    sigs = {sig: ABI_RS.name for kind, sig in shared_declarations().values()
            if kind == "function"}
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


def shared_returns(fn: str) -> str:
    """The `returns ( … )` leg list of a shared `sol!` view, verbatim."""
    text = "\n".join(sol_blocks(read(ABI_RS)))
    return grab(text,
                rf"function {fn}\([^)]*\)\s*external view returns \(([^;]*)\);",
                f"shared {fn} returns")


def contract_returns_tuple(text: str, fn: str, _seen=()):
    """The `write_returns(sdk, &( … ))` tuple a contract handler encodes, as
    `(the function it was found in, the tuple text)`.

    ONE level of indirection is followed, because the handlers use it: `get_registry_with_keys`
    writes nothing itself — it tail-calls `write_validators_with_keys`, which does. Not
    following it would make that check unwritable; scanning the whole file for any
    `write_returns` instead would silently read a DIFFERENT handler's tuple, which is worse
    than not checking at all.

    The callee is resolved by ELIMINATION, not by call order: of the helpers the handler hands
    `sdk` to, exactly one must itself write returns. Following the first `callee(sdk, …)` in the
    body instead would read a decoy — a helper called earlier for its side effects — and report
    ITS tuple as this handler's answer, which is a false PASS, the one outcome this file treats
    as worse than a missing check. Zero candidates or more than one is an `Unreadable` naming
    them, never a guess. (`ensure_initialized(sdk)?` and friends are not candidates at all: the
    call pattern demands an argument after `sdk`.)"""
    body = grab(text, rf"fn {fn}<SDK: SharedAPI>\((.*?)\n}}\n", f"contract handler {fn}")
    m = re.search(r"write_returns\(sdk,\s*&\((.*?)\)\)", body, re.S)
    if m:
        return fn, m.group(1)
    writers = []
    for callee in dict.fromkeys(re.findall(r"\b(\w+)\(\s*sdk,", body)):
        if callee in _seen or callee == fn:
            continue
        try:
            cbody = grab(text, rf"fn {callee}<SDK: SharedAPI>\((.*?)\n}}\n", callee)
        except Unreadable:
            continue
        if "write_returns(" in cbody:
            writers.append(callee)
    if len(writers) != 1:
        raise Unreadable(
            f"{fn}: {len(writers)} of the helpers it hands `sdk` to write returns "
            f"({', '.join(writers) or 'none'}) — refusing to pick one")
    return contract_returns_tuple(text, writers[0], _seen + (fn,))


def check_g4b():
    """`getRegistryWithKeys` return SHAPE — G4's subject for the OTHER two-declaration view.

    The node reads this one to build the consensus p2p tier-2 peer set, so a leg that appears
    or disappears here does not revert: the four-array decoder simply stops matching and the
    peer set comes back empty, which looks like a quiet network rather than an ABI break."""
    node = shared_returns("getRegistryWithKeys")
    node_legs = [x for x in node.split(",") if x.strip()]
    fn, tup = contract_returns_tuple(read(contract_src("consensus.rs")),
                                     "get_registry_with_keys")
    c_legs = [x for x in tup.split(",") if x.strip()]
    report("G4b getRegistryWithKeys arity",
           bool(node_legs) and len(node_legs) == len(c_legs),
           f"node returns {len(node_legs)} arrays ({node.strip()}), "
           f"contract writes {len(c_legs)} from {fn}() ({tup.strip()})")


def _camel(snake: str) -> str:
    head, *rest = snake.split("_")
    return head + "".join(w[:1].upper() + w[1:] for w in rest)


def _snake(camel: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "_", camel).lower()


def contract_handler_file(snake: str):
    """Which contract source implements `snake`, or None. What separates a chain-config view
    from any other no-argument `uint` view is where the contract puts its handler, and that is
    read rather than assumed: `nextEpochToCommit` has the same shared shape and lives in
    `consensus.rs` over a different storage."""
    for path in sorted(CONTRACT.glob("*.rs")):
        if re.search(rf"pub fn {snake}<SDK: SharedAPI>\(", read(path)):
            return path.name
    return None


def check_g15():
    """The chain-config views' return WIDTH: the `uint<N>` the shared `sol!` promises, the
    `StorageU<N>` the contract's handler hands to `write_abi`, and the `(uint<N>)` the STAND
    asks `cast` to decode — three spellings of one width.

    A disagreement here is silent for as long as the values stay small: the encoding is a
    32-byte word either way, and the narrow side starts truncating only once a value outgrows
    it. `crates/staking-abi` says so in its own comment about these three — the node used to
    declare them `uint32` and "got away with it only because the values happen to fit".

    The expected set comes from the SHARED crate — every no-argument view it declares returning
    a single `uint` — and each one is then REQUIRED to have a `config.rs` handler writing a
    `chain_config_storage()` accessor, or the check fails naming it. Deriving the set from
    `config.rs` instead let the subject shrink: three views quietly becoming one still passed,
    because what disappeared also disappeared from the expectation."""
    abi = "\n".join(sol_blocks(read(ABI_RS)))
    cfg = read(contract_src(CONFIG_RS_NAME))
    fields = dict(re.findall(
        r"(\w+):\s*Storage(?:U|Uint)(\d+),",
        grab(read(contract_src("storage.rs")), r"pub struct ChainConfigStorage \{(.*?)\n\}",
             "ChainConfigStorage fields")))
    handlers = dict(re.findall(r"pub fn (\w+)<SDK: SharedAPI>\((.*?)\n}\n", cfg, re.S))
    width, elsewhere, bad = {}, [], []
    for view, ret in re.findall(
            r"function (\w+)\(\)\s*external view returns \(uint(\d+)\);", abi):
        snake = _snake(view)
        body = handlers.get(snake)
        if body is None:
            where = contract_handler_file(snake)
            if where is None:
                bad.append(f"{view}: the shared crate declares it and no contract source "
                           "implements it")
            else:
                elsewhere.append(f"{view}@{where}")
            continue
        slot = re.search(r"chain_config_storage\(\)\s*\.\s*(\w+)_accessor\(\)", body, re.S)
        if slot is None:
            bad.append(f"{view}: its `config.rs` handler no longer writes a "
                       "`chain_config_storage()` accessor")
            continue
        have = fields.get(slot.group(1))
        if have is None:
            bad.append(f"{view}: `{slot.group(1)}` is not a `StorageU<N>` field")
        elif have != ret:
            bad.append(f"{view}: shared declares uint{ret}, the contract stores and "
                       f"`write_abi`s u{have}")
        else:
            width[view] = ret
    # The STAND's own decode annotation is SCORED, not printed: `cast call 'f()(uint32)'` on a
    # `uint64` view returns the right number today and the wrong one after the value grows,
    # which is the same latent break as a narrow declaration and shows up nowhere else.
    stand = {}
    for sig, ret, site in harness_constants()[0]:
        view = sig.split("(")[0]
        if view not in width or not ret:
            continue
        stand.setdefault(view, []).append(site)
        m = re.fullmatch(r"\(uint(\d+)\)", ret.strip())
        if m is None or m.group(1) != width[view]:
            bad.append(f"{site}: the stand decodes {view} as {ret}, declared uint{width[view]}")
    if not width and not bad:
        bad.append("the shared crate declares no no-argument `uint` view — the subject "
                   "vanished, and an empty subject is a FAILURE here, never a pass")
    report("G15 chain-config return widths", bool(width) and not bad,
           f"{len(width)} views agree on all three sides "
           f"({', '.join(f'{k}=uint{v}x{len(stand.get(k, []))} stand sites' for k, v in sorted(width.items()))})"
           f"; not chain-config: {', '.join(elsewhere) or 'none'}"
           if not bad else "; ".join(bad))


def _sol_type(x: str) -> bool:
    """Is `x` a well-formed Solidity ABI type?

    This is what separates a CALL SIGNATURE from a prose label. Accepting any identifier turned
    docstring lines like `getDkgQual(epoch)` and `recordProduction(idx)` into calls whose
    "argument types" then disagreed with the declaration — the check reporting prose as ABI
    drift. A type is: the elementary set, an inline tuple, either one under any number of array
    suffixes."""
    x = x.strip()
    while x.endswith("]"):
        m = re.fullmatch(r"(.*)\[(\d*)\]", x, re.S)
        if not m:
            return False
        x = m.group(1).strip()
    if x.startswith("(") and x.endswith(")"):
        inner = _split_types(x[1:-1])
        return inner is not None
    if x in ("address", "bool", "string", "bytes", "uint", "int"):
        return True
    m = re.fullmatch(r"u?int(\d+)", x)
    if m:
        return int(m.group(1)) % 8 == 0 and 8 <= int(m.group(1)) <= 256
    m = re.fullmatch(r"bytes(\d+)", x)
    return bool(m) and 1 <= int(m.group(1)) <= 32


def _split_types(params: str):
    """`params` split on the commas that are NOT inside an inline tuple, or None when any
    component is not a Solidity type. `""` is the legal empty list."""
    out, depth, cur = [], 0, ""
    for ch in params:
        depth += (ch == "(") - (ch == ")")
        if ch == "," and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += ch
    if cur.strip() or out:
        out.append(cur)
    types = [x.strip() for x in out]
    return types if all(_sol_type(x) for x in types) else None


def _parse_sig(s: str):
    """`(name(types), the cast return annotation or "")` for a `cast`-style signature literal,
    or None when the string is not one.

    The return annotation is split off, not compared here: a selector is a hash of
    `name(types)` alone and `cast sig` ignores the rest, so that tail is what the CALLER wants
    back rather than what the contract declares (G15 is where it is scored, for the views whose
    width is declared). It is brace-matched because it can nest
    (`(address[],(bytes,bytes32,uint64)[],…)`).

    The WHOLE literal must be the signature — nothing after the parameter list but that one
    optional group, and every argument a real ABI type. Without those two rules the first line
    of a docstring (`getDkgQual(epoch) normalized to …`, `nodes.py`) parses as a call, and the
    check reports prose as ABI drift.

    OUT OF SCOPE, stated rather than left to be discovered: a signature assembled at runtime —
    an f-string, a `+` concatenation, a `.format()` — is not a literal and this reads none of
    them. Implicit adjacent-string concatenation IS covered, because Python folds it into one
    constant before the AST sees it."""
    m = re.match(r"([A-Za-z_]\w*)\(", s)
    if not m:
        return None
    depth = 0
    for j in range(m.end() - 1, len(s)):
        depth += (s[j] == "(") - (s[j] == ")")
        if depth == 0:
            params, tail = s[m.end():j], s[j + 1:]
            break
    else:
        return None
    if tail and not re.fullmatch(r"\(.*\)", tail, re.S):
        return None
    types = _split_types(params)
    if types is None:
        return None
    return f"{m.group(1)}({','.join(types)})", tail


def _selector_value(node):
    """The 4-byte value a constant spells, or None. Three spellings, because the harness uses
    three: `0x1752910e`, the bare `"0a87ec8d"` (`floor_halt_case.py`, pinned out of the
    contract), and a 4-byte `bytes` literal.

    KNOWN FALSE-POSITIVE HAZARD, accepted: an ordinary integer constant that happens to equal a
    selector is flagged as a re-transcription. Selectors are ~4e9-magnitude values and the
    harness's other integers are heights, counts and timeouts, so the collision has not
    happened; if it ever does, the fix is to name the site, not to widen the rule."""
    v = node.value
    if isinstance(v, bool):
        return None
    if isinstance(v, int):
        return v if 0 <= v <= 0xFFFFFFFF else None
    if isinstance(v, bytes):
        return int.from_bytes(v, "big") if len(v) == 4 else None
    if isinstance(v, str) and re.fullmatch(r"(0x)?[0-9a-fA-F]{8}", v.strip()):
        return int(v.strip(), 16)
    return None


def harness_constants():
    """Every signature-shaped literal and every selector-shaped constant in the harness — its
    Python out of the AST, its shell out of quoted `cast` arguments — each with its site.

    Python is read from the AST, never as text. A `#` comment is not in the AST at all, and a
    docstring is never EXACTLY a signature — and the harness's prose is full of lines like
    `getEpochCommittee(epoch)`, every one of which a text scan turns into a call this check
    then demands the contract declare. That is the fail-OPEN `sol_blocks` already exists to
    remove, one layer up. Shell has no AST to read, so whole-line comments are dropped and the
    quoted words are held to the same signature grammar, which is what keeps prose out.

    The shell half is not optional: `scripts/xp/*.sh` hand `cast call`/`cast send` signatures
    that no Python file mentions, and they are the same class of unchecked string."""
    sigs, sels = [], []
    for path in sorted(q for d in HARNESS_DIRS for q in d.rglob("*.py")):
        where = path.relative_to(SMOKE)
        try:
            tree = ast.parse(read(path))
        except SyntaxError as e:
            raise Unreadable(f"{path}: {e}")
        for node in ast.walk(tree):
            if not isinstance(node, ast.Constant):
                continue
            site = f"{where}:{node.lineno}"
            parsed = _parse_sig(node.value.strip()) if isinstance(node.value, str) else None
            if parsed:
                sigs.append((parsed[0], parsed[1], site))
                continue
            value = _selector_value(node)
            if value is not None:
                sels.append((value, site))
    for path in sorted(q for d in HARNESS_DIRS for q in d.rglob("*.sh")):
        where = path.relative_to(SMOKE)
        for lineno, line in enumerate(read(path).splitlines(), 1):
            if line.lstrip().startswith("#"):
                continue
            for quoted in re.findall(r"'([^']*)'|\"([^\"]*)\"", line):
                parsed = _parse_sig((quoted[0] or quoted[1]).strip())
                if parsed:
                    sigs.append((parsed[0], parsed[1], f"{where}:{lineno}"))
    return sigs, sels


#: Signatures the stand hands `cast` that the STAKING contract does not declare because they
#: are not its calls: the BLEND ERC20 it funds and approves with, the OpenZeppelin Governor it
#: routes privileged setters through, and the checkpoint predeploy. Listed one by one rather
#: than matched by a pattern, so that a name in NEITHER the shared crate nor `consts.rs` nor
#: this table is a FAILURE, and admitting a new off-contract call is a visible diff.
OFF_CONTRACT = {
    "approve(address,uint256)": "BLEND ERC20",
    "burn(uint256)": "BLEND ERC20",
    "propose(address[],uint256[],bytes[],string)": "OZ Governor",
    "castVote(uint256,uint8)": "OZ Governor",
    "execute(address[],uint256[],bytes[],bytes32)": "OZ Governor",
    "hashProposal(address[],uint256[],bytes[],bytes32)": "OZ Governor",
    "proposalDeadline(uint256)": "OZ Governor",
    "state(uint256)": "OZ Governor",
    "setCheckpoint(uint256,bytes32)": "checkpoint predeploy",
}
#: Signature-shaped literals that are NOT calls: a test naming a spelling to assert the harness
#: does not emit it, or to feed `cast calldata` for its parser behaviour alone. They name
#: handlers the contract deliberately does not have, so the reference sets cannot resolve them
#: and the sites are recorded here instead of being skipped by a rule that would also hide a
#: real one.
NOT_CALLED = {
    "registerValidator(address,uint16,uint256)":
        "tests/test_prod_cases.py asserts the retired 3-argument form is NOT emitted",
    "removeValidator(address)":
        "tests/test_rotation.py asserts force-disable does not reach for a handler that never "
        "existed",
    "setConsensusKeys(address,bytes,bytes,bytes32)":
        "tests/test_lib_write.py round-trips the RETIRED setter through `cast calldata` to pin "
        "cast's own parser behaviour; nothing sends it",
}


def contract_declarations():
    """`name -> {signature, …}` for every selector the contract derives from a string of its
    own (`derive_keccak256_id!`), i.e. the surface the shared crate deliberately does not carry.

    The stand calls a lot of it — `delegate`, `activateValidator`, `getEpochCommittee`, the
    governance setters — and until this was read, G16 skipped every one of them silently
    because their names are not in `crates/staking-abi`."""
    decls = {}
    for sig in re.findall(r'derive_keccak256_id!\(\s*"([^"]+)"', read(contract_src("consts.rs"))):
        decls.setdefault(sig.split("(")[0], set()).add(sig)
    if not decls:
        raise Unreadable(f"{contract_src('consts.rs')} derives no selector from a string — "
                         "either the spelling changed or this parser broke")
    return decls


def check_g16():
    """Every call signature the STAND writes out, against the declaration that owns it.

    The harness builds its calldata by handing `cast` a signature STRING, so nothing on that
    side is type-checked by anything: a renamed argument type goes out under a selector the
    dispatcher does not know, and the failure surfaces as an unrelated revert in the middle of
    a bring-up.

    TWO reference sets, in order: the shared crate, then the contract's own
    `derive_keccak256_id!` declarations. A name in neither — and not in `OFF_CONTRACT` or
    `NOT_CALLED` above — is a FAILURE, because the stand is then calling something no side
    declares. Skipping it, which this check did until the reference set grew, hid 23 signatures
    at ~78 sites.

    ONE class is skipped by rule and named here: a literal with NO arguments whose name is
    unknown. `finalized_dec()`, `running_services()`, `sig()` — a zero-argument string carries
    no ABI evidence at all and is indistinguishable from the Python label it usually is. A
    literal that spells argument TYPES is never ambiguous, so that is where the rule bites.

    The second half is the G3 rule pointed at Python: a 4-byte constant that equals a SHARED
    selector is a re-transcription of a value the crate derives. Contract-derived selectors are
    not in that set — `floor_halt_case.py` pins one deliberately, off the contract, and says
    so at the site."""
    shared = shared_declarations()
    by_name = {name: sig for name, (_, sig) in shared.items()}
    contract = contract_declarations()
    sels = {int.from_bytes(selector(sig), "big"): sig
            for kind, sig in shared.values() if kind in ("function", "error")}
    sigs, consts_seen = harness_constants()
    hit_shared, hit_contract, off, bad = {}, {}, 0, []
    for sig, _ret, site in sigs:
        name, args = sig.split("(", 1)[0], sig[sig.index("(") + 1:-1]
        if sig in NOT_CALLED or sig in OFF_CONTRACT:
            off += 1
        elif name in by_name:
            hit_shared.setdefault(sig, []).append(site)
            if sig != by_name[name]:
                bad.append(f"{site}: the stand calls {sig}, the shared crate declares "
                           f"{by_name[name]}")
        elif name in contract:
            hit_contract.setdefault(sig, []).append(site)
            if sig not in contract[name]:
                bad.append(f"{site}: the stand calls {sig}, consts.rs declares "
                           f"{' / '.join(sorted(contract[name]))}")
        elif args:
            bad.append(f"{site}: the stand calls {sig}, which neither the shared crate nor "
                       "consts.rs declares and no table here admits")
    for value, site in consts_seen:
        if value in sels:
            bad.append(f"{site}: hardcodes 0x{value:08x} — the selector of {sels[value]}, "
                       "which the shared crate derives")
    if not hit_shared and not bad:
        bad.append(f"not one of the {len(by_name)} shared declarations is spelled out anywhere "
                   f"under {', '.join(str(d.relative_to(SMOKE)) for d in HARNESS_DIRS)} — the "
                   "subject vanished, and an empty subject is a FAILURE here, never a pass")
    n_sites = sum(len(v) for v in list(hit_shared.values()) + list(hit_contract.values()))
    report("G16 stand call signatures", bool(hit_shared) and not bad,
           f"{len(hit_shared)} shared + {len(hit_contract)} contract-only signatures agree "
           f"across {n_sites} sites ({off} more admitted as off-contract or not-called): "
           f"{', '.join(sorted(hit_shared))} | {', '.join(sorted(hit_contract))}"
           if not bad else "; ".join(bad))


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


def literal(text, pattern, what, flags=re.S):
    return int(grab(text, pattern, what, flags=flags).replace("_", ""))


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
    # The stand's compose generator refuses N above the cap, and its copy of the number is the
    # one side of this group a `pub use` cannot reach. `grab` raises `Unreadable` — the whole
    # run then exits 3 — when the constant is renamed or inlined back into the guard, which is
    # the only way this comparison could go quiet.
    py = literal(read(COMPOSE_GEN), r"^MAX_COMMITTEE_SIZE = (\d+)\s*(?:#.*)?$",
                 "compose_gen.py cap", flags=re.M)
    report("G8 committee cap", v == 51 and v == py and ok_p and ok_c and gone,
           f"shared={v}; imported by p2p={ok_p} consts.rs={ok_c}; "
           f"old contract name retired={gone}; compose_gen.py transcribes {py}")

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
    # `uint64`, the width the shared crate declares and G15 checks the contract stores at —
    # not the `uint32` this line used to ask `cast` for, which decodes the same word and stops
    # doing so the day the value outgrows 32 bits.
    interval = _uint(live_call(rpc, "getEpochBlockInterval()(uint64)"), "getEpochBlockInterval()")
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
    for fn in (check_g3, check_g4, check_g4b, check_g15, check_g16, check_g1):
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
