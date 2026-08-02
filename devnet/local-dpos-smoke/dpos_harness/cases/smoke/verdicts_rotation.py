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

2. **`evaluate_honest_beacon_window` is NOT `evaluate_beacon_window`.** `case-byzantine-vrf.sh:112`
   defines its own copy that OMITS the across-height distinctness check lib.sh's has. That is
   preserved rather than unified: on the byzantine stack the honest set is being compared while one
   node churns forged boundary views, and adding a check bash does not run would fail a case bash
   passes. It is expressed as `require_distinct=False` on the shared verdict, so the two windows
   cannot drift in any other respect.

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
#: `:370` — the LEGACY silent-verifier watchdog WARN. Its ABSENCE from v5's whole log is the
#: assertion; any occurrence means v5 fell back to the wedge the supervisor exists to eliminate.
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


def dkg_lifecycle_lines(logs: str, tail: int = 4):
    """`:240-241` — `grep -E "ceremony started|PK_epoch \\+ share computed \\+ stored" | tail -4`
    over the joiner's ANSI-STRIPPED log.

    No `epoch=` filter, matching bash: the joiner runs exactly one ceremony before it promotes, so
    the messages alone are the evidence and adding an epoch filter would need E_new-1, which the
    case does not compute."""
    hits = [line for line in (logs or "").splitlines()
            if any(m in line for m in DKG_LIFECYCLE_LINES)]
    return hits[-int(tail):] if tail else hits


def evaluate_early_join(hits):
    """`:242-246` — the EARLY-JOIN proof. An ABSENCE here is the failure: the joiner logging no
    ceremony lifecycle means it did NOT deal/receive its committee[E_new] share from its follower
    phase, i.e. it would be a beacon OBSERVER rather than an early-join signer."""
    if hits:
        return True, ""
    return False, ("EARLY-JOIN — validator-5 logged NO live-DKG ceremony lifecycle during E_new-1; "
                   "it did NOT deal/receive its committee[E_new] share from its follower phase "
                   "(would be a beacon OBSERVER, not an early-join signer)")


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

#: `case-vrf-dkg-halt.sh:88` — the POSITIVE boundary-skip proof. Fires only when the DKG outcome
#: is genuinely None at the change-epoch boundary (application.rs:473-479).
SKIP_PROPOSE_LINE = "beacon: change-epoch boundary but DKG outcome not ready; skipping propose"
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
    stall would freeze at the kill point and never get here, and the "skipping propose" line would
    never fire — an indistinct stall rather than the DKG-None boundary skip this case isolates."""
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


# ── case-byzantine-vrf ────────────────────────────────────────────────────────────────

#: `case-byzantine-vrf.sh:419` — the forge's own `warn!`. It carries a structured `epoch=<E>` field
#: that the SAFETY window is anchored on.
FORGE_LINE = "BYZANTINE: proposing forged PK_E at boundary"
#: `:581` — the honest C-gate rejection. The f=1 safety mechanism: the honest share-holders' real
#: shares do not lie on the forged poly, so the forge cannot pass C → cannot notarize → never
#: reaches certify.
CFAIL_LINE = "C share-on-poly FAILED for asserted outcome"
#: `:82` — the byzantine stayer. NOT validator-0 (the host-RPC node kept honest for greps).
DEFAULT_BYZ_IDX = 2
#: `:320-326` — the stake boost that makes the byzantine rotation-proof AND dominant: +29e18 on top
#: of its 1e18 self-stake ⇒ 30e18 ≈ 72% of committee stake, so the weighted view-1 fallback elects
#: it at ~72% of change-epoch boundaries.
BYZ_BOOST_WEI = "29000000000000000000"
#: `:330` — the floor bump on the OTHER 1e18 validators. Exactly `minStakingAmount`; a smaller
#: bump reverts AmountTooLow.
FLOOR_BUMP_WEI = "1000000000000000000"
#: `:238` — the byzantine owner's BLEND, and `:243` the toggle delegator's (2e18 × 16 in-flips plus
#: headroom, because undelegate parks BLEND in a non-respendable queue).
BYZ_BLEND_WEI = "30000000000000000000"
TOGGLE_BLEND_WEI = "40000000000000000000"
#: `:249` — the BLEND each floor-bumped owner needs. They hold genesis ETH but no runtime BLEND.
FLOOR_BLEND_WEI = "10000000000000000000"
#: `:239` — the toggle delegator's gas.
TOGGLE_ETH_WEI = "1000000000000000"
#: `:375` — the delegation that flips v5 in and out of the committee.
V5_IN_AMOUNT = "2000000000000000000"
#: `:198` — the toggle delegator's mnemonic index. Distinct from the spammer's 6 and from every
#: validator owner key, so its nonce races nothing.
TOGGLE_MNEMONIC_INDEX = "7"
#: `:420` — confirmed flips to drive. At ~72% leader share, P(never leads any of 5) ≈ 0.17%.
DEFAULT_MAX_TOGGLES = 5
#: `:429` — the whole-drive wall budget, in minutes.
DEFAULT_BUDGET_MIN = 45
#: `:455` — a single flip surfaces ~2-3 epochs after its toggle.
FLIP_S = 300
#: `:488,520` — the toggle-loop and drain poll gaps.
FLIP_POLL_S = 3
DRAIN_POLL_S = 4
#: `:564,570` — the SAFETY-window waits.
FORGE_BOUNDARY_S = 900
FORGE_RELIVE_S = 180


def honest_nodes(all_nodes, byz_service: str):
    """`:108-110` — every deriving node EXCEPT the byzantine one.

    The exclusion is not politeness: the byzantine node's own reth may diverge while it churns
    forged boundary views, so including it would fail the cross-node compare for the very behaviour
    the case is provoking. What must agree is the HONEST set — that is the safety claim."""
    return tuple(s for s in all_nodes if s != byz_service)


def toggle_landed(desc: str, ahead: str, joiner: str) -> bool:
    """`:485-486` — THIS flip has surfaced in the ahead-committed committee: IN ⇒ the joiner is now
    present, OUT ⇒ now absent.

    Waiting for each flip before issuing the next is the fix for the ASYMMETRIC warmup
    (delegate→snapshot[e+2], undelegate→snapshot[e+1]): an IN at epoch e and an OUT at e+1 write
    the SAME snapshot epoch and CANCEL, so `committee[*]` never changes and the forge gets zero
    draws. Waiting puts consecutive flips on distinct snapshot epochs."""
    if not ahead:
        return False
    return _has(ahead, joiner) if desc == "IN" else not _has(ahead, joiner)


def evaluate_byz_in_committee(cur_set: str, byz_addr: str, byz_idx, e_new):
    """`:471-476` — the byzantine is STILL in committee[E_new] after the rotation it drove.

    It has to be: it can only forge at a boundary it LEADS, and it can only lead as a member. A
    byzantine that got swapped out means the stake boost is too small relative to v5's in-flip, and
    bash names that fix rather than reporting "the forge never fired" ten minutes later."""
    if _has(cur_set, byz_addr):
        return True, ""
    return False, (f"the Byzantine node validator-{byz_idx} ({byz_addr}) is NOT in "
                   f"committee[E_new={e_new}] despite the rotation-proof stake boost — raise the "
                   f"byzantine boost above v5's V5_IN_AMOUNT.\n  committee[E_new={e_new}]: "
                   f"{cur_set}")


def evaluate_forged(count, byz_idx, changes_seen):
    """`:524-531` — the forge fired at least once.

    The failure message enumerates the FOUR causes bash lists, because they are genuinely
    different fixes and the run took ~45 minutes to get here: it never led a boundary (raise the
    budget), too few boundaries surfaced (a warmup-timing problem), the image lacks the
    `dpos-devnet-byzantine` feature, or the byzantine fell out of the committee."""
    if int(count or 0) >= 1:
        return True, ""
    return False, (f"validator-{byz_idx} never logged '{FORGE_LINE}' across {changes_seen} "
                   "change-epoch boundaries — the forge never fired. Possible causes: it never led "
                   "ANY change-epoch boundary (raise BYZ_VRF_MAX_TOGGLES / BYZ_VRF_BUDGET_MIN), "
                   "too few committee changes actually surfaced (EffBal/warmup timing), the image "
                   "was NOT built with --features dpos-devnet-byzantine, or the byzantine fell out "
                   "of the committee (boost too small)")


_EPOCH_FIELD_RE = re.compile(r"epoch=(\d+)")


def forge_epoch(logs: str, fallback=None):
    """`:547-549` — the LAST `epoch=<N>` on a forge line, ANSI-STRIPPED first.

    The strip is the bug this line already had once: the tracing renderer wraps the `=` in colour
    escapes (`epoch<ESC>[…m=<ESC>[…m4`), so the regex never matched and — under `set -euo
    pipefail` — the unguarded `$( … | grep … )` exited non-zero and aborted the whole case with NO
    diagnostic. The strip happens in the reading half; this half is given clean text.

    LAST, not first: the most recent forged boundary is the one certain to be finalised by the time
    the SAFETY window is asserted. Falls back to E_new, which is also a change-epoch boundary where
    an honest leader commits the real key, so either is a valid safety anchor."""
    hits = _EPOCH_FIELD_RE.findall("\n".join(
        line for line in (logs or "").splitlines() if FORGE_LINE in line))
    if hits:
        return hits[-1]
    return str(fallback) if fallback not in (None, "") else ""


def evaluate_forge_epoch(epoch):
    """`:550-551` — a forged epoch could be determined from either source."""
    if str(epoch or "").isdigit():
        return True, ""
    return False, ("forge fired but could not determine the forged epoch (neither the log's epoch= "
                   "field nor a recorded v5-entry change boundary) — observability gap; raise the "
                   "committee-poll cadence")


def evaluate_c_gate_rejected(count):
    """`:583-587` — an honest share-holder REJECTED the forged boundary at the C gate.

    The POSITIVE half of the safety proof. The beacon window says the honest set agreed on the real
    seed; this says WHY — the forge could not pass C, so it could not notarize and never reached
    certify. Without it, a run where the byzantine simply never proposed to validator-0 would look
    identical to a run where the gate worked."""
    if int(count or 0) >= 1:
        return True, ""
    return False, (f"validator-0 (honest share-holder) never logged '{CFAIL_LINE}' — it did not "
                   "reject the forged boundary at verify (did the byzantine never get to propose "
                   "to it, or is the forge not differing from the real key?)")


def evaluate_byz_liveness(before, after):
    """`:596` — the chain kept finalizing past the boundary under tx load.

    Same shape as `evaluate_still_finalizing` and a DIFFERENT message: here a frozen chain means a
    byzantine stayer wedged liveness, which is the second half of what the case claims."""
    if int(after) > int(before):
        return True, ""
    return False, (f"chain not finalizing past the boundary under tx load ({after} <= {before}) — "
                   "a byzantine stayer wedged liveness")
