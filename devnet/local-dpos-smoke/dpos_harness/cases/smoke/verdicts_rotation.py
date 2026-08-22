"""verdicts_rotation.py — the PURE decision layer of the five PRODUCTION-PATH CASES (chunk 5b).

`verdicts_prod.py` is the decision layer of the shared SUBSTRATE (`lib.sh:589-1362`). This one is
the decision layer of the five DRIVERS that stand on it — `case-production-path` (385),
`case-vrf-rotation` (386), `case-vrf-dkg-halt` (287), `case-vrf-dkg-durability` (483),
`case-byzantine-vrf` (604). Same split as every other chunk: `asserts_prod*.py` decides what to
read and when, this module decides what the readings MEAN, and `tests/test_prod_case_verdicts.py`
drives every one of them through BOTH directions.

Four of the five share ONE trigger — register validator-5, wait for the first ahead-committed
committee that differs from E0's — so the trigger's verdicts live here once rather than four times.
The three that differ afterwards differ completely.

═══ WHAT IS DELIBERATELY *NOT* RE-DERIVED HERE ════════════════════════════════════════════

The DKG on-disk probes (`journal_probe` / `share_probe`), the ANSI-stripped `epoch=<N>` field grep
(`epoch_field_lines`) and the address-scoped marker grep (`slash_hits`) already exist in
`verdicts_onchain.py`, ported in chunk 4 from `case-vrf-dkg-restart-midwindow.sh`, and the halt and
durability cases spell them IDENTICALLY (`case-vrf-dkg-durability.sh:78-135`,
`case-vrf-dkg-halt.sh:65-90`). They are imported, not mirrored. A mirror plus an agreement test
would stay green on every input except the one where the two spellings drift, which is the whole
failure mode.

═══ THREE THINGS THAT LOOK LIKE STYLE AND ARE NOT ═════════════════════════════════════════

1. **The negative assertions are the assertions.** `head_frozen` (halt), `evaluate_no_re_deal`
   (durability) and `evaluate_shareless` (both) are all "the forbidden thing did NOT happen", and
   each one's PASS side is an absence. An absence-grep that silently reads nothing — an unstripped
   log, a wrong service, a `find` under a moved datadir — passes. So every one of them is driven
   in `tests/` by a fixture where the forbidden line IS present, which is the only direction that
   can tell a working grep from a dead one.

2. **There is ONE beacon window and it runs all four checks.** There used to be a second, weaker
   spelling — the honest-set window of `case-byzantine-vrf`, which dropped the across-height
   distinctness check, the only one of the four able to catch a STUCK beacon. It is retired with
   that case, along with the `require_distinct=False` knob that expressed it. Do not re-introduce
   a window that can skip a check.

3. **The committee is compared as a SET STRING.** `pp_committee` already returns sorted+lowercased
   (`verdicts_prod.committee_set`), so `==` between two epochs is a genuine set compare and a
   rotation cannot hide behind a re-ordering.
"""

from __future__ import annotations

import re

from . import verdicts as V
from . import verdicts_onchain as VO
from .verdicts_fault import dkg_margin_blocks

# ── the shared rotation trigger (4 of the 5 cases) ────────────────────────────────────
#: `case-production-path.sh:36` etc — `STAKE_1E18`, the registerValidator self-stake and the
#: approve that precedes it. Spelled identically in all five cases.
STAKE_1E18 = "1000000000000000000"
#: `:228,230` — the delegate that makes v5 outrank an initial validator. `committee[N]` reads
#: `EffBal(N-1)` and a delegate is effective in `EffBal(E+2)`, so v5 enters at E+3 — which the
#: cases COMPUTE by scanning rather than hardcode (see `rotation_reached`).
DELEGATE_2E18 = "2000000000000000000"

#: `:247` — the production-path join scan, and `case-vrf-rotation.sh:196` the rotation scan. Two
#: different budgets for two different questions: the first waits for v5 to appear in the
#: ahead-committed set at all, the second waits for the committee to CHANGE.
JOIN_SCAN_S = 300
ROTATION_SCAN_S = 900

#: The post-registration alignment budget, and the ONE place the five bash files disagree with each
#: other rather than with the port: `case-production-path.sh:236` waits 90 s, the other three wait
#: 180 s for the identical six sends. Both are kept, per case, rather than unified — the shorter one
#: is a real (if incidental) statement that the registration load must not cost the cluster more
#: than 90 s of alignment, and raising it would relax an assertion.
REG_CONVERGE_PP_S = 90
REG_CONVERGE_S = 180


def expected_initial_set(owner_addrs) -> str:
    """`for i in 0..4; do pp_owner_addr $i; done | tr A-F a-f | sort | paste -sd' '`.

    The same normalisation `verdicts_prod.committee_set` applies to the on-chain answer, so the
    two are comparable as strings. Built from the OWNER addresses because that is what the
    committee holds — a service-name comparison would need a mapping that does not exist on-chain.
    """
    return " ".join(sorted(str(a).lower() for a in owner_addrs if a))


def evaluate_initial_committee(got: str, expect: str, epoch=None):
    """`committee(E0) == the initial five` — the bring-up's own sanity gate, run by four cases.

    Two message spellings in the bash and the difference is real: `case-production-path.sh:204`
    omits the epoch, the other three name it (`E0=$E0`). The epoch is what makes a failure
    actionable — "committee != initial 5" says nothing about WHEN — so the named form is used
    whenever the caller has the epoch, which is all four of them."""
    if got == expect:
        return True, ""
    where = f"committee(E0={epoch})" if epoch is not None else "committee"
    return False, f"{where} != initial 5 (got [{got}] want [{expect}])"


def rotation_reached(ahead: str, got0: str, joiner: str) -> bool:
    """`case-vrf-rotation.sh:201` — one poll of the E_new scan, as a predicate.

    THREE conditions and each is load-bearing:

      * `ahead` non-empty — an unreadable committee is not a changed one. Without this an RPC blip
        reads as "the committee emptied", which differs from `got0` and would anchor E_new on a
        failed read (`verdicts_prod.committee_set` returns "" for an unreadable answer, §2.4 item 8).
      * the JOINER is in it — the case drove v5's entry, and a committee that changed for some
        other reason (an eviction, a jail) is not the rotation under test.
      * it DIFFERS from E0's — v5 being present in a set equal to E0's is impossible, but the
        check is bash's and it is the one that survives a re-tuned EffBal timeline.
    """
    if not ahead or not joiner:
        return False
    return _has(ahead, joiner) and ahead != got0


def evaluate_rotation_found(e_new, scan_s=ROTATION_SCAN_S):
    """`:206` — the scan expired without the committee changing."""
    if e_new is not None:
        return True, ""
    return False, (f"committee never changed (v5 never entered an ahead-committed committee "
                   f"within {scan_s}s)")


def evaluate_real_rotation(got_new: str, got0: str, e_new):
    """`:208` — committee(E_new) must actually DIFFER from E0's.

    Re-checked after the scan rather than trusted from it: the scan reads the AHEAD-committed
    committee at `E+1` while this reads `committee[E_new]` once the boundary has been committed,
    and a deferred change (`getDkgQual` false → the incumbent re-committed) makes the second read
    equal to E0's after the first said otherwise."""
    if got_new != got0:
        return True, ""
    return False, f"committee(E_new={e_new}) equals E0's — not actually a rotation"


def _has(committee: str, addr: str) -> bool:
    """`[[ " $SET " == *" $addr "* ]]` — space-delimited membership, never a substring match.

    The padding is the whole trick and dropping it is a real bug: without it `0xabc…` matches any
    address that CONTAINS it, and on a set of 40-hex addresses that is rare enough to pass for
    months and wrong forever."""
    if not committee or not addr:
        return False
    return f" {addr.lower()} " in f" {committee.lower()} "


def committee_has(committee: str, addr: str) -> bool:
    """The public spelling of `_has`, for callers that hold a set string rather than a ctx."""
    return _has(committee, addr)


# ── case-production-path ──────────────────────────────────────────────────────────────

#: `case-production-path.sh:288` — the in-process Verifier→Signer transition. A RESTART-based join
#: would never log it, which is exactly the thing this case exists to distinguish.
PROMOTED_LINE = "promoted to Signer in-process"
#: The LEGACY silent-verifier watchdog WARN. Its ABSENCE from v5's whole log is the assertion; any
#: occurrence means v5 fell back to the wedge the supervisor exists to eliminate.
#:
#: DO NOT DELETE THIS AS A "ZERO-HIT GREP". A plain `grep -rF "NOT in the current committee"` over
#: `crates/` finds nothing, and the string is nevertheless emitted verbatim: the warn is written
#: with a Rust line-continuation, `"… is NOT in the current \" / "committee — run unified mode …"`
#: (`crates/dpos/consensus/src/dpos.rs`, the `committee_watchdog` task), and `\`+newline eats the
#: newline AND the next line's leading whitespace, so the RUNTIME message contains the phrase
#: while the SOURCE never does. Verify a witness against the message the tracing macro builds, not
#: against a source grep.
WATCHDOG_LINE = "NOT in the current committee"
#: `:315` — the demotion probe's settle window. A fixed sleep, not a poll: the question is whether
#: the demoted node is STILL FOLLOWING, and a poll would answer "eventually" where bash asks
#: "within 8 seconds" (§2.4 item 3 — never shorten one of these).
DEMOTION_SETTLE_S = 8
#: `:267` — the budget for v5 to finish catch-up, deal its epoch-E share, promote at the boundary
#: and re-align all seven nodes. 240 s was calibrated for the PRE-fix path where v5 never promoted
#: and is too tight for the real ceremony; the comment at `:262-266` says so.
JOIN_CONVERGE_S = 420
#: `getValidatorStatus`'s status byte for a JAILED validator. Equivocation is the only producer
#: of it now that the liveness jail is deleted, and it always tombstones — so this status is
#: terminal.
STATUS_JAIL = "3"
#: `:382` / `:380` — the closing tx-load check.
FINALIZING_SLEEP_S = 6
#: `:161,169,179,237,319` — fail-path log depths, kept distinct as bash has them.
LOG_TAIL_NODE = 80
LOG_TAIL_ALL = 120

#: `:276` — the noise the v5 join-failure dump filters OUT before printing the tail. Kept as data
#: so the diagnostic prints the same lines bash's `grep -vE` leaves behind.
V5_LOG_NOISE = ("Block added to canonical", "Regular root task", "Forkchoice updated",
                "Canonical chain committed", "Received forkchoice", "Status connected")
#: `:279` — the v0-side grep of the same dump.
V0_JOIN_MARKERS = ("discovery", "handshake", "fac42278", "dial", "listener")
#: `:271,273` — the two commonware peer gauges sampled on a join failure.
PEER_GAUGE_MARKERS = ("buffered_peer_total", "peers_blocked")


def evaluate_plane_native(cmdline: str):
    """`:191-194` — validator-0's LIVE argv (PID 1 is `fluent` via `exec`), read out of
    `/proc/1/cmdline` inside the container. TWO verdicts from one reading:

      * it must LOOK like the DPoS binary (`*fluent*--dpos*`) — an empty or garbled read must fail
        here rather than sail through the second check, which an empty string would PASS;
      * it must NOT carry `--dpos.follower-upstream` — a committee validator that still dials a WS
        hub is the plane de-hub regression (Gap A, task dpos_sync_plane_upstream).

    The order is bash's and it matters: reversing it reports "the plane de-hub regressed" for a
    container whose cmdline could not be read at all."""
    cl = cmdline or ""
    if not ("fluent" in cl and "--dpos" in cl):
        return False, f"could not read v0 live cmdline (got: {cl})"
    if "follower-upstream" in cl:
        return False, ("committee validator-0 carries --dpos.follower-upstream — plane de-hub "
                       "regressed")
    return True, ""


def evaluate_join_committee(got: str, joiner: str, epoch, size):
    """`:295-296` — v5 is in committee(E) AND the committee is still exactly `size`.

    The size check is what makes it a ROTATION rather than a growth: v5 entered the top-5 and the
    lowest-ranked incumbent dropped out. A committee that merely GREW to six would satisfy the
    membership half."""
    if not _has(got, joiner):
        return False, f"v5 not in committee(epoch {epoch}): [{got}]"
    n = len(got.split())
    if n != int(size):
        return False, f"committee size != {size}"
    return True, ""


def displaced_idx(owner_addrs, got: str):
    """`:307-311` — the initial-five member MISSING from the rotated committee, by index.

    First match wins, exactly as bash's `break` does. Returns None when every initial member is
    still in the set, which means no rotation happened and the caller fails loud."""
    for i, addr in enumerate(owner_addrs):
        if addr and not _has(got, addr):
            return i
    return None


def evaluate_displaced(idx):
    """`:312-313` — a displaced member was found, and it is NOT validator-0.

    v0 is the HOST-RPC node every reading in the case goes through; a rotation that displaced it
    would leave the harness measuring the chain through a demoted node, so bash calls that
    "tie-break drift" and fails rather than adapting."""
    if idx is None:
        return False, "could not identify the displaced validator"
    if int(idx) == 0:
        return False, "rotation displaced validator-0 (harness RPC) — tie-break drift"
    return True, ""


def evaluate_still_following(pre: str, post: str, idx):
    """`:314-319` — the DEMOTED validator kept following: two in-container finalized readings,
    `DEMOTION_SETTLE_S` apart, strictly increasing.

    The `null` guard is the sentinel contract (§2.4 item 5): an unreachable node reads `"null"`,
    and `printf '%d' null` would abort bash. Here it must FAIL rather than compare — a node whose
    RPC is gone is not a node that is following."""
    if pre == "null" or post == "null" or not pre or not post:
        return False, (f"displaced validator-{idx} stopped following after demotion "
                       f"({pre} → {post})")
    try:
        ok = int(str(post), 16) > int(str(pre), 16) if str(pre).startswith("0x") \
            else int(post) > int(pre)
    except ValueError:
        return False, (f"displaced validator-{idx} stopped following after demotion "
                       f"({pre} → {post})")
    if ok:
        return True, ""
    return False, f"displaced validator-{idx} stopped following after demotion ({pre} → {post})"

# The liveness-ejection helpers (`victim_pick`, `evaluate_jailed`, `jail_epoch`,
# `eject_check_epoch`, `eject_wait_epochs`, `evaluate_ejected`) are DELETED with the
# participation-floor jail they derived from. `jailedBefore` — the field the whole J-derivation
# read — no longer exists in `getValidatorStatus`. The replacement tier's exclusion term lives in
# `ProductionLiveness.readmitAtEpoch`, but it cannot fire on a smoke stack (the tier ships
# flag-off and `minVerdictDueBlocks = 100` makes it inert at `EPOCH_INTERVAL = 32`), so these were
# retired rather than re-pointed — see the RETIRED block in `asserts_prod`.



def evaluate_watchdog_silent(hits, case_note: str = "v5"):
    """`:370-374` — the legacy committee-watchdog WARN is absent from v5's ENTIRE log.

    The second absence assertion in this case, and the one most likely to rot: it greps a plain
    English phrase out of a log full of ANSI, so an unstripped reader returns nothing and this
    passes for the wrong reason. The reading half strips; `tests/` proves the grep fires."""
    if not hits:
        return True, ""
    return False, (f"{case_note} hit the committee watchdog (legacy verifier wedge):\n"
                   + "\n".join(hits))


def evaluate_promoted_in_process(count, joiner: str = "v5"):
    """`:288-289` — the join was the IN-PROCESS Verifier→Signer transition, not a restart."""
    if int(count or 0) >= 1:
        return True, ""
    return False, f"{joiner} log has no in-process promotion (joined some other way?)"


def evaluate_still_finalizing(before, after):
    """`:382` — the background tx load kept the chain finalizing across every transition. Two
    reads with a fixed sleep between them; strictly greater, so a frozen tip fails.

    Spelled per-case in bash with five different labels; the message here is the shared body and
    the caller's FAIL line already carries the case."""
    if int(after) > int(before):
        return True, ""
    return False, f"chain not finalizing under tx load ({after} <= {before})"


# ── case-vrf-rotation ─────────────────────────────────────────────────────────────────

#: `case-vrf-rotation.sh:109` — the per-member beacon ACTIVE line. Defined once in `verdicts.py`
#: (chunk 1 reads the same line off the static stack) and re-exported, not restated.
ACTIVE_LINE = V.ACTIVE_LINE
#: `:240-241` — the live-DKG ceremony lifecycle on the JOINER, logged from its FOLLOWER phase
#: during E_new-1. Two messages, either of which proves it dealt/received in E_new's ceremony.
DKG_LIFECYCLE_LINES = ("live DKG: ceremony started",
                       "live DKG: PK_epoch + share computed + stored")
#: `:144` — the epoch-2 bootstrap wait, and `:152` the baseline window width.
EPOCH2_WAIT_S = 600
BASELINE_WINDOW = 8
#: `:264` — the climb to the E_new boundary, `:294` the few blocks past it, `:299` the follower
#: catch-up before the strict cross-node compare.
E_NEW_CLIMB_S = 900
RELIVE_WAIT_S = 180
NODES_HAVE_S = 180
#: `:293` — how far past the boundary the relive window reaches.
RELIVE_OFFSET = 4
#: `:311-312` — the ACTIVE_LINE growth probe tracks the HEAD (the SPECULATIVE tip where the line
#: fires), not finalized: finalized can catch up to a burst-ahead tip WITHOUT the tip producing new
#: blocks, which reads as a frozen beacon that is merely a lagging cursor.
HEAD_GROWTH_BLOCKS = 3
HEAD_GROWTH_S = 90
#: `:349,369` — the carry-forward waits.
CARRY_WAIT_S = 900
CARRY_WINDOW_S = 300
#: `:265` — the stall-vs-slow discriminator sample gap on an E_new-climb timeout.
STALL_SAMPLE_S = 10


def probe_members(owner_addrs, got_new: str, validator_name):
    """`:217-232` — committee[E_new] mapped back to validator CONTAINERS, validator-0 first.

    Under the always-on beacon plane EVERY member of committee[E_new] holds a share at E_new —
    the stayers dealt it alongside their signer engine and the JOINER dealt it from its follower
    phase — so the probe set is the FULL committee, not just the stayers. Dropping the joiner
    would delete the early-join half of the case.

    validator-0 is hoisted to the front because it is the host-RPC node: its readings are the
    cheapest and a failure there is the most informative."""
    members = [validator_name(i) for i, a in enumerate(owner_addrs) if a and _has(got_new, a)]
    v0 = validator_name(0)
    if v0 in members:
        members = [v0] + [m for m in members if m != v0]
    return members


def evaluate_probe_members(members, joiner_service: str):
    """`:224-228` — the probe set is non-empty AND contains the joiner.

    Two failures with two different meanings: an empty set means the address→container mapping
    broke (a genesis/keys problem), while a set without the joiner means the rotation did not
    bring v5 in — which is a statement about the chain, not about the harness."""
    if not members:
        return False, "could not map committee[E_new] to any validator container"
    if joiner_service not in members:
        return False, (f"{joiner_service} (the joiner) is not in committee[E_new] — the rotation "
                       "did not bring v5 in")
    return True, ""


def dkg_lifecycle_lines(logs: str, epoch, tail: int = 4):
    """The joiner's DKG lifecycle lines FOR `epoch`, over an ANSI-STRIPPED log.

    THE FILTER RUNS BEFORE THE TAIL. `tail` keeps the last N matches, so trimming first would push
    the E_new pair out of a busy log and report a joiner that ran the ceremony as one that never
    did. The unfiltered version matched ANY ceremony the joiner ever ran — a previous epoch's pair
    satisfied the proof for E_new.

    The old docstring justified having no filter by saying the case does not compute E_new-1. The
    code contradicts it — `e_new` is in scope at the call site — and E_new-1 is the wrong epoch
    anyway: both lines carry the epoch they are FOR, not the epoch they are logged in
    (`crates/dpos/consensus/src/beacon/actor.rs:959,1133`), so the filter is `epoch=E_new`.

    Matching goes through `VO.epoch_field_lines`, whose field pattern is right-anchored so
    `epoch=2` does not also match `epoch=20`."""
    hits = [ln for m in DKG_LIFECYCLE_LINES for ln in VO.epoch_field_lines(logs, m, epoch)]
    return hits[-int(tail):] if tail else hits


#: How often the early-join wait re-reads the joiner's log. Coarser than the 1-2s gates nearby
#: because each poll is a WHOLE `docker compose logs` of a growing log, and the thing being waited
#: for takes seconds to tens of seconds, not milliseconds.
EARLY_JOIN_POLL_S = 5


def evaluate_early_join(hits, epoch, window_closed=False, budget_s=None):
    """The EARLY-JOIN proof: BOTH lifecycle lines for `epoch`, observed BEFORE the boundary.

    `ceremony started` on its own is a ceremony that BEGAN AND FAILED QUALIFICATION — the joiner
    dealt, never reached `PK_epoch + share computed + stored`, and enters E_new holding no share.
    That shareless observer is precisely the failure this check exists to catch, and accepting
    either line let it through.

    IT IS A WAIT, NOT A SAMPLE. Live on 2026-08-03 the joiner logged `ceremony started epoch=6` and
    the case read the log NINE SECONDS later, before the ceremony had finalized — a red gate over a
    healthy run. The DKG for E_new runs during E_new-1 and seals near its end, so one sample taken
    right after `scan_for_rotation` returns is far too early.

    `window_closed` IS THE UPPER BOUND AND IT IS NOT A BUDGET. The claim is that the joiner
    completed the ceremony FROM ITS FOLLOWER PHASE, and that phase ends when E_new's first block is
    finalized — after that it is a member, and a share stored then is a strictly weaker property
    than the one the OK line asserts. So the wait stops at the boundary even with budget left.

    NOT the seal deadline (`epoch_start(E_new) − DKG_MARGIN_BLOCKS`), which is tighter: that is the
    PRODUCT's contract for when a ceremony must seal, and `smoke-vrf-dkg-halt` /
    `smoke-vrf-dkg-durability` are the cases that rail on it. A share stored between the seal
    deadline and the boundary would still be from the follower phase, so failing it here would red
    this case for a property it does not own and its message does not describe."""
    started = any(DKG_LIFECYCLE_LINES[0] in h for h in hits)
    stored = any(DKG_LIFECYCLE_LINES[1] in h for h in hits)
    if started and stored:
        return True, ""
    if not started:
        return False, (f"EARLY-JOIN — validator-5 logged NO ceremony at all for committee"
                       f"[{epoch}]: it never even began the DKG from its follower phase, so it "
                       "enters E_new as a beacon OBSERVER, not a signer")
    if window_closed:
        return False, (f"EARLY-JOIN — validator-5 started committee[{epoch}]'s DKG but stored no "
                       f"share before E_new's first block finalized. The follower-phase window is "
                       "CLOSED: a share it stores now would be stored as a MEMBER, which is not "
                       "the property this case asserts")
    return False, (f"EARLY-JOIN — validator-5 started committee[{epoch}]'s DKG but had stored no "
                   f"share after {budget_s}s, with the follower-phase window still open (E_new's "
                   "first block is not finalized yet) — the ceremony is not completing")


#: How many lifecycle lines the failure dump keeps. A bounded tail, not the log: the point is to
#: show WHICH epochs the joiner ran ceremonies for, and a handful of rows settles that.
DKG_AUDIT_TAIL = 40


#: The tracing `epoch=<N>` field, parsed out of a log line. Lives HERE, beside its only surviving
#: reader: it used to sit at the bottom of the file in the byzantine-vrf section and was read from
#: 500 lines above, so retiring that case took the constant with it.
_EPOCH_FIELD_RE = re.compile(r"epoch=(\d+)")


def dkg_lifecycle_audit(logs: str, tail: int = DKG_AUDIT_TAIL):
    """EVERY lifecycle line with its epoch parsed out, UNFILTERED — `[(epoch, line), …]`.

    The failure dump, and it exists because the filtered list cannot be one: `dkg_lifecycle_lines`
    returns `[]` in precisely the case the filter is what is wrong, so printing it shows nothing at
    all. This shows what the filter REJECTED, which separates the three readings of a red gate:
    a ceremony logged for a DIFFERENT epoch means the filter is looking at the wrong number; no
    ceremony at any epoch means the joiner never dealt; `ceremony started` alone at the right
    epoch means it dealt and failed qualification.

    `"?"` for a line with no `epoch=` field at all — that would itself be a finding, since both
    lifecycle lines are logged with one (`beacon/actor.rs:959,1133`)."""
    rows = []
    for line in (logs or "").splitlines():
        if any(m in line for m in DKG_LIFECYCLE_LINES):
            hit = _EPOCH_FIELD_RE.search(line)
            rows.append((hit.group(1) if hit else "?", line.strip()))
    return rows[-int(tail):] if tail else rows


def evaluate_joiner_holds_share(present: bool, epoch, joiner_idx):
    """The joiner holds a NON-EMPTY share file for E_new on disk.

    This is the only witness available that DISCRIMINATES. `ACTIVE_LINE` growth does not: the
    deriver logs `beacon: threshold prev_randao active` on EVERY node whose certificate carries a
    seed (`crates/node/src/derive.rs:78-86`), regardless of whether that node contributed a
    partial — so a shareless observer grows the count exactly like a real signer.

    WHAT THE PROBE DOES NOT SHOW: that the share is valid, or that it sits on the committed
    polynomial. The file could be stale or corrupt. All that is claimed is a non-empty file FOR
    THIS EPOCH, which is strictly more than the previous check asserted."""
    if present:
        return True, ""
    return False, (f"validator-{joiner_idx} holds no beacon-share-e{epoch}.bin — it entered "
                   f"committee[{epoch}] without a share, i.e. as a beacon OBSERVER rather than a "
                   "signer (the ACTIVE_LINE count cannot see this: the deriver logs that line on "
                   "every node that receives a seeded certificate)")


def evaluate_member_active_growth(before, after, service: str, joiner_service: str):
    """`:318-326` — the per-member ACTIVE_LINE count GREW across the rotation boundary.

    `after <= before` is the failure, not `after == before`: a log that was truncated or re-read
    from a restarted container can go DOWN, and that is no more acceptable than standing still.

    The role word in the message is bash's and it is the diagnosis: a frozen count on a STAYER
    means the beacon did not relive, while a frozen count on the JOINER means it is a shareless
    observer — two different bugs behind one number."""
    if int(after) > int(before):
        return True, ""
    role = "EARLY-JOIN newcomer" if service == joiner_service else "share-holder"
    return False, (f"{service} ({role}) active-count frozen at {after} across the E_new boundary "
                   "— it is NOT casting verified seed partials as a committee[E_new] member "
                   "(beacon did not relive / it is a shareless observer)")


def carry_window_hi(lo, epoch_len):
    """`:349,367` — `S_LO + (EPOCH_LEN > 5 ? 4 : EPOCH_LEN - 1)`.

    The ternary is not a micro-optimisation: on a tuned devnet with a very short epoch, `lo + 4`
    would reach into the NEXT epoch and the window would no longer be "inside E_s", which is the
    whole claim. Below six blocks the window is the epoch minus its boundary block."""
    epoch_len = int(epoch_len)
    return int(lo) + (4 if epoch_len > 5 else epoch_len - 1)


def is_stable_epoch(c_e: str, c_p: str) -> bool:
    """`:357` — a STABLE epoch: its committee is readable AND equal to its predecessor's.

    The non-empty half is the guard: two unreadable committees are both "" and compare EQUAL, so
    without it an RPC outage would present as the stable epoch the case is looking for and the
    carry-forward window would be asserted over an epoch nobody read."""
    return bool(c_e) and c_e == c_p


def evaluate_stable_epoch(e_s, lo, hi):
    """`:361` — a stable epoch was found in the scanned range."""
    if e_s is not None:
        return True, ""
    return False, (f"no stable (unchanged-committee) epoch found in [{lo}, {hi}] — cannot test "
                   "carry-forward")


def stall_verdict(h1, h2) -> str:
    """`:266` — the stall-vs-slow discriminator printed on an E_new-climb timeout.

    Two finalized readings `STALL_SAMPLE_S` apart. bash embeds the ternary inside the FAIL string;
    it is a verdict about the chain and belongs here."""
    return ("advancing, just slow: raise the budget" if int(h2) > int(h1)
            else "FROZEN: real stall at/before the rotation")


# ── the TORN-JOURNAL recipe (shared by BOTH DKG cases) ────────────────────────────────
#
# The only way to make a restarted committee member sit out its OWN epoch's ceremony. The
# alternative — killing it and bringing it back — does NOT work any more: `maybe_start`'s
# `JournalLoad::Present` arm re-derives the seeded dealer whenever
# `last_height < epoch_start(target) − DKG_MARGIN_BLOCKS` (`beacon/actor.rs:1046-1054`), so a
# victim restarted before the seal deadline seals normally and the ceremony finalizes. The
# `JournalLoad::Torn` arm (`:1056-1069`) is the one that sits out UNCONDITIONALLY, and
# `torn_warned` (`:998`) makes that permanent for the epoch.
#
# `case-vrf-dkg-durability` phase 3 tears ONE journal (quorum survives ⇒ the chain stays live);
# `case-vrf-dkg-halt` tears TWO (dealer-quorum unreachable ⇒ the boundary is never crossed). Same
# recipe, opposite claim, so it lives here once.

#: `case-vrf-dkg-durability.sh:129` — the `JournalLoad::Torn` arm's product warn
#: (`crates/dpos/consensus/src/beacon/actor.rs:1060-1067`). Grepped MESSAGE-then-`epoch=`.
TORN_LINE = "live DKG: ceremony journal present but unreadable/torn"
#: `:134` — a SECOND one of these after a torn restart would mean the node RE-DEALT
#: (`actor.rs:1133`).
CEREMONY_STARTED_LINE = "live DKG: ceremony started"
#: `:406-407` — the corruption recipe: overwrite the first record's 4-byte big-endian length prefix
#: with 0xffffffff (≫ file size) so `load_journal` hits "truncated record body"
#: (`share_state.rs:495`), the first record never decodes, `out.is_empty()` and the arm is Torn —
#: NOT NoFile, which would RE-DEAL. It fires BEFORE `decode_record`, so it is state-agnostic
#: (plaintext vs encrypted framing is irrelevant). Truncating to 0 bytes is the WRONG recipe:
#: `share_state.rs:483` maps an empty file to NoFile.
#: OCTAL escapes because genesis-init's /bin/sh has POSIX `\ooo` and not the bash/coreutils `\xHH`.
TORN_MAGIC = "ffffffff"
#: `:428-429` — the torn sit-out poll.
TORN_POLL_S = 120
TORN_POLL_GAP_S = 3


# ── case-vrf-dkg-halt ─────────────────────────────────────────────────────────────────

#: The POSITIVE no-share proof, and the successor to a witness that no longer exists.
#:
#: `smoke-vrf-dkg-halt` used to name the mechanism with a propose-time line — "beacon: change-epoch
#: boundary but DKG outcome not ready; skipping propose" — emitted by the boundary gate in
#: `application.rs`. THAT GATE IS GONE: since the epoch key left `OrderBlock` a block asserts
#: nothing about the beacon and there is no boundary gate at all
#: (`application.rs`, `BeaconVerify`'s docstring says so outright). The string had zero hits in the
#: tree, so the case's positive log could only ever print its "not yet flushed" branch.
#:
#: What still happens on a shareless committee is one layer down: `EpochManager::reconcile_roles`
#: resolves the member's beacon share for the epoch, gets `Absent`, and soft-enters a VERIFY-ONLY
#: scheme instead of spawning a participating engine
#: (`crates/dpos/consensus/src/epoch_manager.rs`, the `Role::Signer` share-gate, counted by
#: `epoch_engine_demoted_no_polynomial_total`). With every member demoted the epoch has no signer,
#: the boundary block is never proposed, and the head freezes at the boundary edge — the same
#: observable the case has always asserted, reached by a different road.
SHARE_GATE_LINE = "committee member without a usable DKG share — verify-only (share-gate)"
#: The share-gate line renders its epoch with `?epoch` over a `#[derive(Debug)]` newtype, so the
#: field reads `epoch=Epoch(7)` and NOT `epoch=7`. `epoch_field_lines` (which anchors on the bare
#: number) therefore cannot match it — a witness filtered with the wrong spelling is a witness that
#: never fires.
SHARE_GATE_EPOCH_FMT = "epoch=Epoch({})"


def share_gate_lines(logs: str, epoch):
    """Lines where a node demoted itself to verify-only for `epoch` for lack of a DKG share.

    MESSAGE first, then the epoch FIELD, in the two-grep shape `epoch_field_lines` uses and for the
    same reason (tracing renders fields in an order the case does not control). The field spelling
    is the Debug one — see `SHARE_GATE_EPOCH_FMT`."""
    field = SHARE_GATE_EPOCH_FMT.format(int(epoch))
    return [ln for ln in (logs or "").splitlines()
            if SHARE_GATE_LINE in ln and field in ln]


#: `:77` / `:81` — the seal and finalize product logs, grepped MESSAGE-then-`epoch=` (the
#: order-independent idiom `verdicts_onchain.epoch_field_lines` owns).
SEAL_LINE = "live DKG: dealings sealed"
SHARE_LINE = VO.SHARE_LINE
#: `:168` — the pre-seal gate budget, `:225` the post-restart climb, `:206` the stop→start gap.
PRESEAL_GATE_S = 600
CLIMB_S = 600
RESTART_GAP_S = 3
#: `:252` / `:279` — the two SUSTAINED no-progress windows. THE ASSERTION IS THE TIMEOUT
#: (§2.4 item 3): shortening either one silently disarms the halt proof, because a chain that has
#: merely not produced a block yet is indistinguishable from one that never will over a short
#: enough window. 30 s is well past several 1 blk/s intervals; the 15 s re-confirm is what makes
#: the halt PERMANENT rather than momentary.
FREEZE_WINDOW_S = 30
REFREEZE_WINDOW_S = 15
#: `:253` — the sample gap of the post-failure diagnostic.
FREEZE_SAMPLE_S = 5
#: `:153` — the pre-seal victims are drawn from v1..v4: NOT v0 (stopping it removes the host RPC)
#: and NOT v5 (the joiner is not a stayer).
KILL_CANDIDATE_IDXS = (1, 2, 3, 4)
#: `:151` — how many must be sat out for the DKG to be unreachable on n=5: dealer-quorum is
#: N3f1(5)=4, so 2 sit-outs leave 3 < 4. They are STOPPED only long enough to tear their journals
#: and then come back, so consensus quorum (also 4) is restored and the chain still CLIMBS to the
#: boundary — which is what makes the wedge attributable to the DKG and not to a lost quorum.
KILL_COUNT = 2


def kill_candidates(owner_addrs, got_new: str, want=KILL_COUNT,
                    candidates=KILL_CANDIDATE_IDXS):
    """`:152-157` — the first `want` committee[E_new] STAYERS among v1..v4, in index order.

    Index order is bash's and is not arbitrary here: it is what makes the choice deterministic
    across a re-run, so a failure names the same two nodes twice."""
    out = []
    for i in candidates:
        if i < len(owner_addrs) and owner_addrs[i] and _has(got_new, owner_addrs[i]):
            out.append(i)
        if len(out) >= int(want):
            break
    return out


def evaluate_kill_set(kills, got_new: str, want=KILL_COUNT):
    """`:158` — enough non-leader stayers were found to drive a >f kill."""
    if len(kills) >= int(want):
        return True, ""
    return False, (f"could not find {want} non-leader original committee[E_new] stayers to kill "
                   f"(committee=[{got_new}])")


def preseal_ok(journal_present: bool, sealed: bool, share_absent: bool) -> bool:
    """`:169-171` — the PRE-seal window: the ceremony STARTED (journal on disk) but has neither
    SEALED nor FINALIZED.

    Tearing the journal is NECESSARY but NOT SUFFICIENT, and this gate is the "sufficient" half. A
    victim that had already SEALED has broadcast its dealer log, so the survivors hold enough logs
    to finalize the ceremony no matter what its own journal says afterwards — the tear would sit
    out a node whose contribution is already in flight and the boundary would cross. Only a tear
    taken before the seal deadline actually removes a dealer from the count.

    The journal-present half is equally load-bearing in the other direction: with no journal on
    disk there is nothing to tear, `load_journal` answers `NoFile`, and the restarted node DEALS
    fresh (`share_state.rs:483`) — the opposite of a sit-out."""
    return bool(journal_present) and not bool(sealed) and bool(share_absent)


def evaluate_preseal_gate(reached: bool, k0, k1):
    """`:174` — both victims reached the pre-seal window inside the gate budget."""
    if reached:
        return True, ""
    return False, f"v{k0}/v{k1} never both reached the PRE-seal window for E_new"


def evaluate_window_not_missed(finalized, boundary, what: str):
    """`:180-182`, and the same rail in durability at `:214` and `:380`.

    The rail rides INSIDE the gate loop in all three, and the order within one iteration is bash's:
    the gate is evaluated first and the rail only when the gate is not yet satisfied. Hoisting it
    out would report "the window never opened" for a run whose window merely closed — a wrong
    diagnosis, and 600 s late."""
    if int(finalized) < int(boundary):
        return True, ""
    return False, (f"chain reached the {what} boundary ({boundary}) before the gate was satisfied "
                   "(window missed — re-run)")


def seal_deadline(boundary, margin=None) -> int:
    """`epoch_start(E) − DKG_MARGIN_BLOCKS` — the height at which the ceremony SEALS
    (`crates/dpos/consensus/src/beacon/actor.rs:83`, `DKG_MARGIN_BLOCKS = 20`).

    The margin comes from `verdicts_fault.dkg_margin_blocks()`, the tree's one named constant for
    it (bash spells the same knob with the pre-rename prefix and defaults it identically at
    `asserts-fault.sh:379` and `soak-invariants.sh:2230`), so an override moves every consumer
    together and a second literal cannot drift from the product's."""
    return int(boundary) - (dkg_margin_blocks() if margin is None else int(margin))


def evaluate_seal_window_not_missed(finalized, boundary, what: str, margin=None):
    """`case-vrf-dkg-halt.sh:188-192` — the PRE-SEAL window is still open.

    NOT the boundary. The window closes one margin EARLIER, at the seal deadline: the ceremony
    seals at `epoch_start − DKG_MARGIN_BLOCKS`, so a kill taken in the last 20 blocks before the
    boundary is POST-seal, the survivors finalize on the disseminated Reveals, and the cluster
    RECOVERS — the durability case's outcome, i.e. exactly the thing the halt case exists to
    exclude. Railing on the boundary accepted that silently for a 20-block window and would have
    turned the negative control into a coin flip.

    `evaluate_window_not_missed` stays the rail for the two DURABILITY gates: those wait for a
    POST-seal state (phase 1) and for a journal that lives the whole deal→boundary span (phase 3),
    so the boundary genuinely is their limit."""
    deadline = seal_deadline(boundary, margin)
    if int(finalized) < deadline:
        return True, ""
    return False, (f"chain reached the {what} SEAL DEADLINE ({deadline} = boundary {boundary} − "
                   f"DKG_MARGIN_BLOCKS {int(boundary) - deadline}) before the gate was satisfied — "
                   "any kill from here is POST-seal and RECOVERABLE, which is the opposite of what "
                   "this case asserts (window missed — re-run)")


def evaluate_climbed(head, edge):
    """`:226-230` — the chain CLIMBED to the boundary edge after the restart.

    This is the discriminator, not a warm-up: only a chain whose consensus quorum was RESTORED
    (the two restarted nodes rejoined → 4 of 5 online) can reach `boundary-1`. A genuine consensus
    stall would freeze at the KILL POINT and never get here — an indistinct stall rather than the
    shareless-committee freeze AT THE BOUNDARY EDGE that this case isolates."""
    if int(head) >= int(edge):
        return True, ""
    return False, (f"chain did not climb to the boundary edge ({edge}) within {CLIMB_S} s after "
                   f"the restart (head={head}) — the restarted nodes may not have rejoined "
                   "consensus")


def head_frozen(before, after) -> bool:
    """`head_frozen_for` (`:92`) — `b <= a`, i.e. the head did NOT advance over the window.

    `<=`, not `==`: a head that went BACKWARDS (a re-org, a restarted RPC serving an older tip) is
    not progress either, and bash's `(( b <= a ))` accepts it as frozen."""
    return int(after) <= int(before)


def evaluate_terminal_halt(frozen: bool, h0=None, h1=None):
    """`:252-256` — the SUSTAINED no-progress halt at the boundary edge.

    ***THIS IS THE LINE THE CASE FAILS ON IF THE HALT BEHAVIOUR REGRESSES.*** By the time it runs,
    the case has already PROVEN (Torn-arm line + no re-deal + no share anywhere) that the committee
    is shareless for E_new: `beacon_for_epoch(E_new)` is None and no proposer may build E_new's
    first block (`application.rs:473-479`). A head that advances through this window is therefore a
    chain that crossed a change-epoch boundary with an unfinished beacon key — the exact product
    failure this case exists to catch. `evaluate_below_boundary` is the same claim measured on
    finalized and is the second line to fire.

    The other reading — that the case failed to set the experiment up — is excluded upstream rather
    than here: the pre-seal gate + the seal-deadline rail place the tear before any victim sealed,
    and the corruption readback + the Torn-arm assertion prove the sit-out actually happened."""
    if frozen:
        return True, ""
    return False, (f"TERMINAL control — head did NOT stay frozen at the boundary edge "
                   f"(head {h0} → {h1}) — the chain CROSSED the E_new boundary even though the "
                   "committee is provably SHARELESS for E_new (the Torn sit-out fired, nobody "
                   "computed a share). That is a change-epoch boundary crossed with an unfinished "
                   "DKG key: the halt gate at application.rs:473-479 has REGRESSED")


def evaluate_permanent_halt(frozen: bool):
    """`:279-282` — the halt is PERMANENT: still frozen after the re-confirm window."""
    if frozen:
        return True, ""
    return False, ("the halt was NOT permanent — the head advanced after the freeze window (a "
                   "terminal >f pre-seal halt must never self-heal)")


#: `epoch_manager.rs` — the in-process Verifier→Signer promotion. A stable greppable token (the
#: production-path smoke keys on it too), and its epoch field is the Debug newtype spelling, for
#: the reason `SHARE_GATE_EPOCH_FMT` records: `reconcile_roles` takes `epoch: Epoch` and renders it
#: through `?epoch`, so the field reads `epoch=Epoch(7)`.
PROMOTE_LINE = "promoted to Signer in-process"
PROMOTE_EPOCH_FMT = SHARE_GATE_EPOCH_FMT


def promote_lines(logs: str, epoch):
    """Lines where a node PROMOTED itself to Signer for `epoch`. MESSAGE then epoch FIELD, the
    two-grep shape `share_gate_lines` owns."""
    field = PROMOTE_EPOCH_FMT.format(int(epoch))
    return [ln for ln in (logs or "").splitlines()
            if PROMOTE_LINE in ln and field in ln]


def evaluate_did_not_promote(lines, idx, epoch):
    """The POSITIVE half of every "this node sat the epoch out" claim in these two cases.

    WHY THE SHARE-LINE ABSENCE IS NOT ENOUGH ON ITS OWN. Both cases concluded "shareless" purely
    from the absence of the ceremony-finalize log, whose sole emitter is one arm of one function.
    An absence is satisfied by a node that never logged anything, by a log read that returned
    nothing (which `logs_required` now refuses), and — the one that actually matters here — by a
    node that acquired its share through a DIFFERENT road than the one being watched. Since the
    live-epoch artifact pull landed, that second road exists: a member with no share now asks for
    the epoch's artifact and recomputes from the retained dealer logs, and it emits its own line
    when it does, not the ceremony-finalize one. So the absence these cases assert has grown a way
    to be true while the property is false.

    Promotion is the consequence both roads share. A node that holds a usable share for `epoch`
    spawns its per-epoch engine and says so; a node that sits the epoch out cannot. Checking that
    is checking the thing the cases mean, once, instead of checking one of the ways to reach it.

    Both cases stay TRUE negatives for the right reason, and it is worth writing down which:
    the torn journal loads as `JournalLoad::Torn`, and the recompute-heal bails on anything but
    `Present`, so the artifact pull (which does now happen, and does now land) can never complete
    the heal. The sit-out is structural, not a race."""
    if not lines:
        return True, ""
    listing = "\n".join(f"    {ln}" for ln in lines)
    return False, (f"v{idx} PROMOTED to Signer for epoch {epoch} — it is not sitting the epoch "
                   f"out, whatever its share log says:\n{listing}")


def evaluate_no_share_computed(idx, has_share: bool, epoch):
    """`:265-268` — the DKG-None discriminator: NO committee member finalized an E_new share.

    An ABSENCE per node, and the one that separates the DKG-None halt (the ceremony stayed below
    dealer-quorum 4, so nobody has a share) from a hypothetical successful-DKG stall (where the
    survivors WOULD hold one). `tests/` drives it with a share line present."""
    if not has_share:
        return True, ""
    return False, (f"validator-{idx} computed an E_new={epoch} share — the C_r1 ceremony "
                   "FINALIZED, so the kill did not land >f pre-seal (not a terminal halt)")


def evaluate_below_boundary(finalized, boundary):
    """`:284` — finalized never crossed the E_new boundary, i.e. E_new's first block was never
    produced. The other half of the freeze: a head frozen ABOVE the boundary would be a chain that
    crossed and then stalled, which is a different (and non-terminal) failure.

    The SECOND line the case fails on if a shareless committee is allowed across the boundary (the
    first is `evaluate_terminal_halt`). It is kept because it is measured on FINALIZED and on the
    boundary height itself, so a crossing that happened while the tip was momentarily flat still
    fails here."""
    if int(finalized) < int(boundary):
        return True, ""
    return False, (f"finalized={finalized} reached the E_new boundary {boundary} — the chain "
                   "FINALIZED E_new's first block with a shareless committee (no member holds an "
                   "E_new share), so the change-epoch halt did NOT hold")


#: `:273` — a clean option-A stall must carry NO panic anywhere in the project's logs.
PANIC_RE = re.compile(r"panic|thread '.*' panicked", re.IGNORECASE)


def panic_lines(logs: str):
    """`grep -iE "panic|thread '.*' panicked"` over an ANSI-STRIPPED project log."""
    return [line for line in (logs or "").splitlines() if PANIC_RE.search(line)]


def evaluate_no_panic(hits, what: str = "a node"):
    """`:274` / durability `:470` — the halt (or the sit-out) is CLEAN, not a crash."""
    if not hits:
        return True, ""
    return False, (f"{what} PANICKED — the halt must be a clean option-A stall, not a crash:\n"
                   + "\n".join(hits[-10:]))


# ── case-vrf-dkg-durability ───────────────────────────────────────────────────────────
#
# `TORN_LINE`, `CEREMONY_STARTED_LINE`, `TORN_MAGIC` and the torn poll budgets live in the shared
# torn-journal section above — the halt case tears journals too.

#: `:195` — how far past the epoch-2 boundary phase 1 probes, and `:420` the same for phase 3.
BOUNDARY_PROBE_OFFSET = 6
#: `:200,373` — the two gate budgets; `:250,421` the post-restart resume budgets.
GATE_S = 600
RESUME_S = 400
#: `:228` — the CONTROL freeze window with 2 of 5 down. A no-event proof, so it is a timeout and
#: must not be shortened (§2.4 item 3).
STALL_WINDOW_S = 12
#: `:263` — the per-victim durable-share poll after the restart.
SHARE_POLL_S = 60
SHARE_POLL_GAP_S = 2
#: `:428-429` — the torn sit-out poll lives in the shared torn section above (`TORN_POLL_S`).
#: `:304` — the phase-1→phase-3 hand-off health check.
HANDOFF_CONVERGE_S = 120
#: `:261` — phase 1's two victims. NOT the leader v0: the case needs the host RPC alive to observe
#: the stall it deliberately causes.
PHASE1_VICTIMS = (3, 4)
#: `:2` — the bootstrap ceremony phase 1 rides on.
EPOCH2 = 2
#: `:356-365` — the torn victim: v3 by preference (matching the midwindow baseline), else the first
#: non-leader committee[E_new] stayer. v5 is excluded — it is the joiner, not a stayer.
TORN_PREFERRED_IDX = 3
TORN_FALLBACK_IDXS = (1, 2, 4)
#: `:460` — C_r1 must finalize among at least a quorum of the non-torn members.
FINALIZED_QUORUM = 4
#: `:472` — the equivocation markers, scoped to the torn victim's address.
EQUIV_MARKERS = ("ValidatorSlashed", "equivocat")


def torn_victim(owner_addrs, got_new: str, preferred=TORN_PREFERRED_IDX,
                fallbacks=TORN_FALLBACK_IDXS):
    """`:356-365` — the torn victim must be a committee[E_new] member (so it RUNS the C_r1
    ceremony) and must not be the leader.

    v3 first, because that is the midwindow baseline's victim and keeping them the same makes the
    two cases' logs comparable; if the equal-stake tie-break benched v3, any other non-leader
    stayer will do."""
    if preferred < len(owner_addrs) and owner_addrs[preferred] \
            and _has(got_new, owner_addrs[preferred]):
        return preferred
    for i in fallbacks:
        if i < len(owner_addrs) and owner_addrs[i] and _has(got_new, owner_addrs[i]):
            return i
    return None


def evaluate_torn_victim(idx, got_new: str):
    """`:366` — a usable torn victim exists."""
    if idx is not None:
        return True, ""
    return False, ("no non-leader committee[E_new] stayer available as the torn victim "
                   f"(committee=[{got_new}])")


def evaluate_seal_gate(reached: bool):
    """`:203` — both phase-1 victims SEALED epoch 2 inside the gate budget.

    The seal line is MONOTONE (it never un-fires), which is what makes the window wide open from
    seal to the boundary. The first run of this case used a simultaneous `seal && share-absent`
    gate and missed, because the all-in finalize writes the share about one block after the seal."""
    if reached:
        return True, ""
    return False, "v3/v4 never both SEALED epoch 2 within the deadline"


def evaluate_control_stall(frozen: bool):
    """`:228-231` — the CONTROL. With 2 of 5 down only 3 are online, below the consensus quorum of
    4, so the producer cannot notarize and the HEAD must FREEZE.

    A no-event proof, and it is what makes the recovery afterwards non-trivially REQUIRED: without
    it, "the chain kept going" would be indistinguishable from "the victims were never needed"."""
    if frozen:
        return True, ""
    return False, ("CONTROL — head did NOT freeze with 2 of 5 down (only 3 online, < consensus "
                   "quorum 4): it advanced. Either the quorum is not n−f=4 here or a down node is "
                   "still notarizing — the 'recovery required' control is broken")


def evaluate_rejoin(reached: bool, probe):
    """`:250-253` — the 2-DOWN REJOIN: the chain resumed and crossed the epoch-2 boundary once
    both victims were back. A failure here is a real 2-simultaneous-down rejoin wedge (the existing
    fault cases only ever down ONE), which bash says to capture rather than paper over."""
    if reached:
        return True, ""
    return False, (f"chain did not resume + cross the epoch-2 boundary ({probe}) after restarting "
                   "v3+v4 (a 2-simultaneous-down rejoin wedge — capture before papering over)")


def evaluate_resumed_past_stall(finalized, pre_stall):
    """`:254` — finalized advanced past the pre-stall height."""
    if int(finalized) > int(pre_stall):
        return True, ""
    return False, f"finalized did not advance past the pre-stall height {pre_stall} after restart"


def evaluate_share_recovered(present: bool, idx):
    """`:264-267` — the victim still HOLDS its epoch-2 share after the restart.

    The recovery mechanism is the DURABLE SHARE FILE, not the journal resume: the kill lands
    post-finalize, so `beacon-share-e2.bin` was already written and `build_beacon_plane::load_all`
    reloads it into the CeremonyStore with the EXACT share. A victim that came back shareless would
    abstain from every seeded vote and be liveness-slashed."""
    if present:
        return True, ""
    return False, (f"v{idx} has NO epoch-2 share file after the restart — the durable share reload "
                   "did not recover it (it would be shareless → liveness-slashed)")


def evaluate_not_slashed(hits, idx):
    """`:287` — no slash event names the victim. Reuses `verdicts_onchain.slash_hits` for the
    reading, so the markers cannot drift from chunk 4's."""
    if not hits:
        return True, ""
    return False, (f"v{idx} was liveness-slashed despite recovering its share:\n"
                   + "\n".join(hits))


def evaluate_status_readable(status, idx):
    """`:289-290` — the status read succeeded AND the victim is not jailed.

    The empty read is a HARD ERROR and not a "not jailed" (`verdicts_onchain.evaluate_not_jailed`
    makes the same call for the same reason): an empty-vs-"3" false-green would hide the jail the
    check exists to refuse."""
    st = (status or "").strip()
    if not st:
        return False, (f"could not read v{idx} validator status (empty RPC result) — cannot assert "
                       "not-jailed (re-run)")
    if st == STATUS_JAIL:
        return False, f"v{idx} is JAILED (status={STATUS_JAIL}) after the post-seal restart"
    return True, ""


def evaluate_corruption_landed(firstbytes: str, idx):
    """`:413-415` — the journal corruption ACTUALLY landed: the first four bytes read back as
    `ffffffff`.

    A silent file-op no-op — which is what `docker compose exec` on a STOPPED container is — must
    FAIL LOUD here rather than false-green: without the corruption the restart takes the normal
    resume path, the Torn arm never fires, and every later assertion in the phase is measuring a
    node that was never torn."""
    got = (firstbytes or "").strip()
    if got == TORN_MAGIC:
        return True, ""
    return False, (f"v{idx}'s E_new journal corruption did NOT land (first 4 bytes='{got}', want "
                   f"'{TORN_MAGIC}') — the Torn arm would not fire (a silent file-op no-op)")


def evaluate_chain_live_across_torn(reached: bool):
    """`:421-423` — the chain stayed LIVE across the E_new boundary with one Torn sit-out. Quorum
    4 survives exactly one absent member, which is what makes the sit-out graceful rather than
    fatal."""
    if reached:
        return True, ""
    return False, ("CONTROL — chain did NOT stay live across the E_new boundary with one Torn "
                   "sit-out (4 survivors should be a quorum)")


def evaluate_torn_sitout(hit: bool, idx, epoch):
    """`:430-433` — the victim LOGGED the Torn sit-out for E_new. Used by BOTH DKG cases.

    A missing line means the corruption did not land in the Torn arm. The likely wrong outcome is
    NoFile (a zero-byte journal), and a NoFile node RE-DEALS — which the no-re-deal check below
    catches independently, so the two together tell a wrong recipe from a broken product.

    In `case-vrf-dkg-halt` this is the ANTI-VACUITY check: without it a chain halted for any
    unrelated reason (a lost peer, a crashed victim, a consensus stall below the boundary) would
    read as a pass. The case only claims a shareless-committee halt if the victims are provably
    sitting out."""
    if hit:
        return True, ""
    return False, (f"v{idx} did NOT log the Torn sit-out for E_new={epoch} — the corruption did "
                   "not land in the Torn arm (NoFile? a wrong recipe would re-deal)")


def evaluate_no_re_deal(count, idx, epoch):
    """`:436-440` — EXACTLY ONE "ceremony started" for E_new: the original, pre-corruption run.

    A torn resume must NOT re-deal, because a second dealing from the same member on the same epoch
    is self-equivocation. `<= 1` is bash's test and it is the right one: zero would mean the
    ceremony never started at all, which the journal gate already excluded."""
    n = int(count or 0)
    if n <= 1:
        return True, ""
    return False, (f"v{idx} logged {n} 'ceremony started' for E_new={epoch} — it RE-DEALT after "
                   "the torn restart (self-equivocation risk; the corruption fell to NoFile, not "
                   "Torn)")


def evaluate_shareless(has_share: bool, idx, epoch):
    """`:441-443` — the torn victim is SHARELESS for E_new. The positive counterpart of the
    sit-out line: a node that logged Torn and then computed a share anyway did not sit out."""
    if not has_share:
        return True, ""
    return False, (f"v{idx} computed an E_new={epoch} share despite sitting out torn — it should "
                   "be SHARELESS for E_new")


def evaluate_member_finalized(has_share: bool, idx, epoch):
    """`:455-457` — every NON-torn committee[E_new] member computed its E_new share, i.e. C_r1
    finalized among the survivors."""
    if has_share:
        return True, ""
    return False, (f"committee[E_new] member v{idx} did NOT compute its E_new={epoch} share — C_r1 "
                   "did not finalize among the survivors")


def evaluate_finalized_quorum(count, want=FINALIZED_QUORUM):
    """`:460` — enough survivors finalized to constitute a quorum."""
    if int(count) >= int(want):
        return True, ""
    return False, (f"only {count} of the non-torn committee[E_new] members finalized E_new "
                   f"(want >= {want} = quorum)")


def evaluate_no_equivocation(hits, idx):
    """`:473` — no equivocation/slash evidence names the torn victim. A torn resume that re-dealt
    would produce exactly this, so it is the on-chain witness for the log-side no-re-deal check."""
    if not hits:
        return True, ""
    return False, (f"v{idx} produced equivocation/slash evidence (a torn resume must NOT re-deal):"
                   "\n" + "\n".join(hits))


def equiv_hits(logs: str, addr: str):
    """`:472` — `grep -iE "ValidatorSlashed|equivocat" | grep -i "<addr without 0x>"`.

    Delegated to `verdicts_onchain.slash_hits` with a different marker set, rather than re-written:
    the two-grep shape (markers AND address, both `-i`, the `0x` stripped because events render the
    address bare) is identical and is exactly the part a copy would get subtly wrong."""
    return VO.slash_hits(logs, addr, markers=EQUIV_MARKERS)
