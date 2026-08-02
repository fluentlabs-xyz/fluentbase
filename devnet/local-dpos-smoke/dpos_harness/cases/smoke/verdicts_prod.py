"""verdicts_prod.py — the PURE decision layer of the production-path SUBSTRATE (`lib.sh:589-1362`).

Same split as `verdicts.py` / `verdicts_fault.py` / `verdicts_follow.py` / `verdicts_onchain.py`:
`prod.py` decides what to read and when, this module decides what the readings MEAN, and
`tests/test_prod_substrate.py` drives every one of them through EVERY outcome.

The difference from the other four is what it is a decision layer FOR. Those are per-case; this one
is under all five production-path cases at once, because the ~620 lines of `lib.sh` they share
carry most of the bug density in the file. Nearly every defensive oddity in that range is a
diagnosed production failure with a bundle id in its comment, and the ones that live HERE — rather
than in the I/O — are the ones a rewrite loses silently.

═══ 1. THREE-VALUED IS NOT BOOLEAN, TWICE ═══════════════════════════════════════════════════

`pp_dkg_qual` (lib.sh:707) returns `1` qualified / `0` deferred / `""` UNREADABLE, and `""` is not
a third flavour of "no". It means the read failed. Mapping it onto "deferred" makes an RPC blip
report that the incoming committee's DKG did not qualify — a real verdict about the chain, invented
from a network hiccup. `dkg_qual_state` is the one place the three are told apart.

`pp_fund_eth` (lib.sh:955) has THREE failure codes and conflating them WAS the bug: in
bundle-20260716T203448Z all three came back as "budget exhausted", so a stalled chain and an
unreadable RPC were both reported as the mint budget running out.

    1  the balance is genuinely below the floor   → the budget really is exhausted
    2  the send failed / was never confirmed      → the chain is congested, the budget is fine
    3  the balance READ failed                    → nothing is known; retry, do not report a floor

Code 3 exists because coercing a failed read to 0 makes it indistinguishable from a real 0, i.e.
from a floor hit.

═══ 2. `cast` PRETTY-PRINTS, AND `printf '%d'` MAKES IT WORSE ═══════════════════════════════

`cast call …(uintN)` prints any value >= 10000 as `22920 [2.292e4]`. bash strips the suffix with
`${out%% *}` (`_pp_cfg_read_retry`) and with `awk '{print $1}'` (`pp_gov_action`'s proposal id).
Dropping either is not a parse error you would notice: `printf '%d' "22920 [2.292e4]"` emits the
int AND exits non-zero, so the `|| echo 0` that follows APPENDS a `0` → `229200`. That is an
activation block past the head, which pins the epoch to 0 and disables all churn, silently.
`first_token` is that strip, and `cfg_value` is the whole read-attempt decision including the
"succeeded but printed nothing" case bash answers with 0.

═══ 3. THE GOVERNANCE QUORUM IS STAKE-WEIGHTED ══════════════════════════════════════════════

`voter_list` has three modes and the third is not a convenience. FluentGovernance's quorum is 2/3
of the delegated STAKE, so the equal-weight `ceil(2/3·n)+1` prefix can fall short of 2/3 of the
POWER on an uneven committee — and once the original pool tombstones and power migrates to MINTED
identities (idx >= POOL, outside any prefix), the prefix under-votes and the proposal is Defeated
on a chain where nothing is wrong.

Return convention: `(ok: bool, message: str)` for the gates — empty message on success, the bash
diagnostic on failure. The classifiers return a value, not a pair.
"""

from __future__ import annotations

import re

from ...core.policy import gov_voter_list

#: `pp_fund_eth`'s four codes (lib.sh:943-948), named so a caller cannot mean 2 and write 3.
FUND_OK = 0
FUND_FLOOR = 1
FUND_SEND = 2
FUND_READ = 3

#: What each code MEANS, for the attribution line the caller writes. The whole point of the three
#: being distinct is that they get three different sentences.
FUND_REASON = {
    FUND_OK: "funded",
    FUND_FLOOR: "owner-0 ETH balance below the mint floor — mint budget exhausted",
    FUND_SEND: "funding send FAILED/unconfirmed (stalled or congested chain) — NOT a floor",
    FUND_READ: "owner-0 balance READ failed (RPC unreachable/garbled) — NOT a floor",
}

#: `_dkg_qual_state` (soak-invariants.sh:2317) — the three-valued DKG-qualification state.
DKG_QUALIFIED = 1
DKG_DEFERRED = 0
DKG_UNREADABLE = -1

#: OZ Governor `ProposalState` names (lib.sh:996). The order is the enum's, not alphabetical.
GOV_STATE_NAMES = ("Pending", "Active", "Canceled", "Defeated", "Succeeded",
                   "Queued", "Expired", "Executed")
#: The states that are terminal AND not Succeeded — `pp_gov_wait_succeeded` names them rather than
#: waiting them out, because a Defeated proposal never becomes Succeeded and a timeout would
#: report the wrong cause (lib.sh:1025).
GOV_TERMINAL_FAILURES = ("2", "3", "6")   # Canceled, Defeated, Expired
GOV_SUCCEEDED = "4"
#: `head > end + 5` (lib.sh:1033) — margin for the state transition to surface after the voting
#: end block. Not slack: the Governor's state only flips once a block past the deadline is mined.
GOV_DEADLINE_MARGIN = 5

#: The three keys of `/runtime/staking-reader.json` the create-nonce drift detector compares
#: (lib.sh:1328-1330), mapped to their manifest counterparts.
STAKING_READER_PAIRS = (
    ("staking_address", "staking"),
    ("chain_config_address", "chain_config"),
    ("liveness_slashing_address", "liveness_slashing"),
)

_ADDR_RE = re.compile(r"0x[0-9a-fA-F]{40}")


# ══ 1. `cast` output parsing ═══════════════════════════════════════════════════════════════

def first_token(out) -> str:
    """`${out%% *}` / `awk '{print $1}'` — the `[sci]` pretty-print strip (§2.4 item 9).

    Whitespace-split rather than a `[`-split: bash splits on the first SPACE, and a value that
    somehow arrived without the bracket (a small uint) must come through unchanged."""
    s = (out or "").strip()
    return s.split()[0] if s else ""


def cfg_value(ok: bool, stdout: str):
    """One attempt of `_pp_cfg_read_retry` (lib.sh:719-735). Returns the int, or None to RETRY.

    Bash branches on cast's EXIT CODE — `if out=$(pp_chainconfig_call …)` — and NOT on emptiness,
    which makes three cases, not two:

        ok, "22920 [2.292e4]"  ->  22920   the suffix is stripped before the parse
        ok, ""                 ->  0       `printf '%d' "" || echo 0`; a legitimate answer, NOT a
                                           retry — the getter answered and said nothing
        not ok, anything       ->  None    a transient; the caller retries, then memoizes

    The middle row is the one the first port lost. Retrying a successful empty read three times and
    then reporting a transient failure makes the caller fall back to a MEMOIZED interval — i.e.
    compute a non-zero epoch off a ChainConfig that just told it there is none."""
    if not ok:
        return None
    tok = first_token(stdout)
    if not tok:
        return 0
    try:
        return int(tok)
    except ValueError:
        return 0        # bash: `printf '%d' <non-numeric>` fails -> `|| echo 0`


def current_epoch(head, act, interval) -> int:
    """`pp_current_epoch`'s arithmetic (lib.sh:756-758), isolated from its reads and its memo.

    Two guards, both fail-SAFE (return epoch 0) rather than fail-loud, because this runs inside a
    poll loop where a loud failure would abort a night-long run over one bad read:

      * `interval == 0` — the ChainConfig has no interval yet (pre-deploy, or a codeless read).
        Dividing by it would raise; reporting epoch 0 says "DPoS has not started", which is true.
      * `head < act` — the chain has not reached the activation block. A negative relative epoch
        is not a thing; before activation the relative epoch is 0."""
    interval = int(interval or 0)
    head = int(head or 0)
    act = int(act or 0)
    if interval == 0 or head < act:
        return 0
    return (head - act) // interval


def epoch_first_block(act, epoch_len, epoch) -> int:
    """`epoch_first_block <n>` (lib.sh:1362) — `ACT + n * EPOCH_LEN`, the first block of relative
    epoch <n>."""
    return int(act) + int(epoch) * int(epoch_len)


def activation_block(head, grid: int = 64) -> int:
    """`ACT=$(( ((HEAD / 64) + 2) * 64 ))` (lib.sh:1320) — the next-but-one grid boundary.

    `grid` is a LITERAL 64 in the bash and deliberately NOT the epoch interval: `EPOCH_LEN` is not
    read until the last line of `pp_bring_up_rotation`, so the bash could not have used it. It is a
    parameter here only so a test can drive the arithmetic; the caller passes the literal."""
    return ((int(head) // int(grid)) + 2) * int(grid)


def committee_set(out) -> str:
    """`pp_committee` (lib.sh:685-692) — a cast `address[]` return as a SORTED, LOWERCASED,
    space-joined set, so membership compares regardless of on-chain ordering.

    Two properties are the assertion, not the formatting:

      * **sorted+lowercased** — the chain returns the committee in its own order and in mixed case;
        a case comparing two epochs' committees for equality would otherwise see a rotation in
        every re-ordering.
      * **empty means empty, and never an abort.** bash ends the pipeline in `|| true` because an
        empty committee makes `grep .` exit 1, and under the caller's `set -o pipefail` that
        ABORTS THE WHOLE RUN instead of yielding the "" the assertion can report (§2.4 item 8). A
        Python port that raised on an unparseable read would invert the same intent, so this
        returns "" for anything it cannot find addresses in — including "" itself."""
    return " ".join(sorted(a.lower() for a in _ADDR_RE.findall(out or "")))


def status_byte(out) -> str:
    """`pp_validator_status` (lib.sh:677-681) — the 2nd field of `getValidatorStatus`'s 8-field
    tuple (NotFound=0, Active=1, Pending=2, Jail=3), read over the RUNTIME staking cluster.

    "" when the read failed, and the callers treat that as a hard error rather than as "not
    jailed": an empty-vs-"3" false-green would hide the jail a case exists to observe.

    Field-position parse, matching `cast_field 2` (`rpc.sh`), NOT a regex for the byte: the tuple's
    third field is a uint256 stake that pretty-prints as `1000000000000000000 [1e18]`, so a reader
    that scanned for "a small number" would find the exponent."""
    parts = (out or "").split()
    return parts[1] if len(parts) >= 2 else ""


# ══ 2. three-valued reads ══════════════════════════════════════════════════════════════════

def dkg_qual_token(out) -> str:
    """`pp_dkg_qual` (lib.sh:707-714) — normalize `getDkgQual(uint64)(bool)` to the THREE-valued
    token: `"1"` qualified, `"0"` deferred, `""` UNREADABLE.

    The `tr -d '[:space:]'` is bash's and is kept: cast right-justifies its output, so the raw
    answer is `"  true"`. Anything that is not exactly `true`/`false` after that — an error string,
    an empty answer off a codeless address, a partial read — is UNREADABLE, never `false`."""
    tok = "".join((out or "").split())
    if tok == "true":
        return "1"
    if tok == "false":
        return "0"
    return ""


def dkg_qual_state(token) -> int:
    """`_dkg_qual_state` — `1` QUALIFIED / `0` DEFERRED / `-1` UNREADABLE.

    Mirrors `checks/battery.py::dkg_qual_state`, and `tests/test_prod_substrate.py` asserts the two
    agree on every token rather than either importing the other: `checks/battery.py` is a
    2000-line sim detector module and the case layer must not pull it in for five lines. The test
    is what keeps the pair from drifting."""
    if token == "1":
        return DKG_QUALIFIED
    if token == "0":
        return DKG_DEFERRED
    return DKG_UNREADABLE


def dkg_qual_is_set(token) -> bool:
    """`_dkg_qual_is_set` — True iff the token is the QUALIFIED marker. Deliberately NOT
    `state != DEFERRED`: an unreadable read is not qualified either."""
    return token == "1"


def fund_reason(code) -> str:
    """The attribution sentence for a `pp_fund_eth` return code. An unknown code is named as such
    rather than defaulted to one of the four — a fifth code appearing silently as "budget
    exhausted" is exactly the failure this table exists to prevent."""
    return FUND_REASON.get(code, f"unknown fund_eth code {code!r}")


def fund_is_budget_exhausted(code) -> bool:
    """True ONLY for the genuine pre-check floor. This is the predicate a caller should reach for
    when it wants to stop minting; `code != 0` is the bug (bundle-20260716T203448Z)."""
    return code == FUND_FLOOR


# ══ 3. governance ══════════════════════════════════════════════════════════════════════════

def gov_state_name(byte) -> str:
    """`_pp_gov_state_name` (lib.sh:995-998) — the ProposalState name, `Unknown` outside 0..7.

    Names, not numbers, because the failure line is the whole diagnosis: "Defeated" says the vote
    genuinely failed, "Pending" says votingDelay is not 0, and a bare `state=3` says neither."""
    try:
        b = int(byte)
    except (TypeError, ValueError):
        return "Unknown"
    return GOV_STATE_NAMES[b] if 0 <= b < len(GOV_STATE_NAMES) else "Unknown"


#: `pp_gov_action`'s three voter-selection modes (lib.sh:1105-1132). Defined in `core/policy.py`
#: and re-exported here rather than restated: `chain.writes` reads it too, and the one time it was
#: duplicated the copy got the CLAMP ORDER wrong — bash clamps DOWN to `voters` first and floors at
#: 1 second, so the two spellings differ only at `voters == 0`. A mirror plus an agreement test
#: would have stayed green on every other input, which is exactly why this one is shared.
voter_list = gov_voter_list


def gov_wait_verdict(state, head, end, stalled_for, desc: str = "",
                     frozen_s: int = 120, last_head=None):
    """One poll of `pp_gov_wait_succeeded` (lib.sh:1015-1044) as a pure classifier.

    Returns `(verdict, message)` where verdict is one of:

        "succeeded"  state == 4 — done.
        "terminal"   Canceled / Defeated / Expired — the vote GENUINELY failed. Named, not timed
                     out: a timeout here would report "slow chain" for a proposal that lost.
        "deadline"   past the voting end block + margin and still not Succeeded — again named with
                     the ACTUAL state rather than reported as a timeout.
        "frozen"     the head has not moved for `frozen_s` — a chain stall, which belongs to the
                     finalize-stall / node-down invariants, not to governance.
        "waiting"    keep polling.

    `end` may be None (an unreadable `proposalDeadline`), and that is not a failure: the terminal
    and frozen escapes still bound the wait, which is why bash sets `end=""` and carries on rather
    than aborting. The whole point of the block-paced deadline is that under a blaster-degraded
    rate the voting window CRAWLS — a wall-clock deadline reported a healthy-but-slow chain as a
    governance failure (bundle-20260716T232341Z), so nothing here counts seconds except the
    freeze."""
    state = "" if state is None else str(state)
    if state == GOV_SUCCEEDED:
        return "succeeded", ""
    if state in GOV_TERMINAL_FAILURES:
        return "terminal", (f"proposal {gov_state_name(state)} (state={state}) for: {desc}")
    if end is not None and head is not None and int(head) > int(end) + GOV_DEADLINE_MARGIN:
        return "deadline", (f"proposal {gov_state_name(state)} (state={state}) past voting end "
                            f"block {end} (head={head}, margin {GOV_DEADLINE_MARGIN}) for: {desc}")
    if stalled_for is not None and stalled_for >= frozen_s:
        shown = last_head if last_head is not None else head
        return "frozen", (f"chain FROZEN waiting out the voting window (head={shown} unmoved for "
                          f"{int(stalled_for)}s; voting end block="
                          f"{end if end is not None else 'unreadable'}) for: {desc}")
    return "waiting", ""


# ══ 4. the bring-up's own gates ════════════════════════════════════════════════════════════

def staking_reader_mismatches(pre: dict, manifest: dict):
    """The create-nonce drift detector (lib.sh:1327-1334): every `/runtime/staking-reader.json`
    address that disagrees with the deploy manifest, as `(key, got, want)` triples.

    Case-insensitive on both sides, as bash's `tr 'A-F' 'a-f'` is — the manifest is checksummed
    and the reader file is not, so a byte comparison would report three mismatches on a perfectly
    aligned deploy.

    A MISSING key is a mismatch against "", not a skip. The file is generated with all three; one
    absent means the generator changed, which is exactly the drift this gate is for."""
    out = []
    for reader_key, manifest_key in STAKING_READER_PAIRS:
        want = str(manifest.get(manifest_key, "") or "").lower()
        got = str((pre or {}).get(reader_key, "") or "").lower()
        if got != want:
            out.append((reader_key, got, want))
    return out


def manifest_missing(manifest: dict):
    """The four deploy addresses `pp_bring_up_rotation` asserts are `0x…` (lib.sh:1295-1297), as
    the list of NAMES that are absent or malformed. Empty list = the deploy produced a usable
    manifest.

    `startswith("0x")` is bash's whole test and is kept as such rather than tightened to a
    40-hex-digit check: widening the gate would make it reject a manifest bash accepts, and the
    thing it is guarding against is a jq miss returning `null`, not a subtly wrong address."""
    names = (("STAKING_RT", "staking"), ("CHAIN_CONFIG_RT", "chain_config"),
             ("GOV_ADDR", "governance"), ("LIVENESS_RT", "liveness_slashing"))
    return [name for name, key in names
            if not str((manifest or {}).get(key, "") or "").startswith("0x")]
