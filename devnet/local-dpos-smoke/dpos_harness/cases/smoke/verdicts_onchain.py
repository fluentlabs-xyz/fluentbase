"""verdicts_onchain.py — the PURE decision layer of the four ON-CHAIN cases.

`case-liveness.sh`, `case-byzantine.sh`, `case-cert-catchup.sh`,
`case-vrf-dkg-restart-midwindow.sh`. Same split as `verdicts.py` / `verdicts_fault.py` /
`verdicts_follow.py`: `asserts_onchain.py` decides what to read and when, this module decides
what the readings MEAN, and `tests/test_smoke_onchain_verdicts.py` drives every one of them
through BOTH outcomes.

These four are the first cases in the suite that read the STAKING SURFACE — the committee
enumeration, the windowed participation counters, the validator status byte and the chain-config
floor. Two properties of that surface shape everything below.

═══ 1. THE PARTICIPATION READ HAS THREE ANSWERS, NOT TWO ═════════════════════════════════

`lib.sh:257-270` returns `(seen, certs)` with two sentinel pairs, and they are NOT variants of
each other:

    (-1, -1)   the address is not in committee[epoch] — a fact about the COMMITTEE
    (-2, -2)   the getter call FAILED — a fact about the RPC, and no information at all
    (n,  m)    real counters

Collapsing either onto `(0, 0)` is a false verdict in BOTH directions and that is why it is
called out in §2.4 item 5 and pinned in `tests/test_sentinels.py`:

  * `smoke-liveness` asserts `victim seen < hub seen`. A -2 read as 0 makes that trivially
    true, so a case whose whole point is "the bitmap reached the chain" would pass on a chain
    it could not read.
  * `smoke-vrf-dkg-restart-midwindow` asserts `seen * 10000 >= certs * floorBps`. A -2 read as
    0 is below EVERY positive floor, so the case would fail a node that recovered perfectly —
    and, worse, a -1 read as 0 would report a liveness slash for a node that is not in the
    committee the slash is computed over.

`credit_state` is the ONE place the three are told apart, and every gate below takes its
answer rather than testing the numbers itself.

═══ 2. THE CERT-CATCHUP GATES ARE NEGATIVES OVER A RACE WINDOW ═══════════════════════════

`smoke-cert-catchup` proves a node did NOT do something: no budget-shutdown ERROR, no
consensus-exit line. Both are measured as a log-count DELTA across the restart — snapshot
before, snapshot after — and both pass when the count did not move.

That shape has two ways to pass while checking nothing, and each has a named guard:

  * a SILENT READER. `ctx.log_count` reads the whole log ANSI-STRIPPED (`nodes.logs_all`) for
    exactly this reason: the node writes SGR escapes INSIDE its `key=value` pairs, so an
    unstripped grep for a line that IS present can return zero matches. On an absence-gate a
    silent reader is indistinguishable from a clean run. §2.4 item 2.
  * a WINDOW THAT WAS NEVER OPENED. A victim that rejoined without ever parking satisfies both
    negatives perfectly, which is why `evaluate_park_exercised` exists and is MANDATORY on the
    first cycle: it is the positive control that says the path under test actually ran. Bash
    calls that out in as many words — "A green that never triggered the path is a FALSE PASS".

Return convention: `(ok: bool, message: str)` — empty message on success, the bash diagnostic on
failure. A few return a third element (a witness, a note) where bash prints one.
"""

from __future__ import annotations

import re

from ...core import converge

# ══ the participation sentinels ═══════════════════════════════════════════════════════

#: `lib.sh:262` — the address is not in `getEpochCommittee(epoch)`.
CREDIT_NOT_IN_COMMITTEE = -1
#: `lib.sh:265,269` — the `participation(uint64,uint32)` call failed or parsed short.
CREDIT_READ_FAILED = -2

#: The three states `credit_state` returns. Named so a caller cannot spell one wrong and
#: silently take the `else` branch.
STATE_OK = "ok"
STATE_ABSENT = "not-in-committee"
STATE_FAILED = "read-failed"


def credit_state(seen) -> str:
    """`(-2 | -1 | n)` -> which of the three answers this is.

    Reads only `seen`: `lib.sh` sets BOTH fields to the same sentinel together (`echo "-1 -1"`,
    `echo "-2 -2"`), so the pair is never mixed and testing one field is testing the pair. A
    caller that tested `certs` instead would get the same answer; a caller that tested them
    INDEPENDENTLY would be inventing a fourth state that the getter cannot produce."""
    try:
        v = int(seen)
    except (TypeError, ValueError):
        return STATE_FAILED
    if v == CREDIT_READ_FAILED:
        return STATE_FAILED
    if v == CREDIT_NOT_IN_COMMITTEE:
        return STATE_ABSENT
    return STATE_OK


def credit_diagnosis(seen, who: str) -> str:
    """The clause a failure message appends so a -1 and a -2 do not read alike, NAMING which side
    produced it.

    bash prints the raw `seen=-2` and leaves the reader to know what it means; three of its four
    failure messages in this cluster do explain it inline, and this is that explanation made
    uniform. It is additive — the numbers bash prints are still printed. `who` matters because
    `smoke-liveness` reads TWO addresses per poll and a failed HUB read and a failed VICTIM read
    call for different next steps."""
    state = credit_state(seen)
    if state == STATE_FAILED:
        return (f" [the {who} getter read FAILED (-2) and kept failing — a -2 is never a real 0, "
                "so the counters could not be evaluated at all]")
    if state == STATE_ABSENT:
        return (f" [-1 = the {who} address is NOT in this epoch's committee, so there is no "
                "participation window to read]")
    return ""


# ══ smoke-liveness ════════════════════════════════════════════════════════════════════

#: `case-liveness.sh:30-32` — the committee is exactly four and `validators[i] == validator-i`.
#: A shorter list is a genesis/bring-up problem, and mapping a service to the wrong address would
#: read the HUB's counters as the victim's, which passes the `<` test never.
EXPECTED_VALIDATORS = 4

#: `case-liveness.sh:56` — the budget for the chain to advance the cycle's gap with one node down.
#: Cycle 1 waits `3*interval + 1` = 97 blocks; at 1 blk/s with the victim's leader views timing
#: out (1750 ms) until `skip_timeout` mutes them that is ~100-115 s of chain time.
LIVENESS_ADVANCE_S = 240
#: `:68`, `:79` — the bitmap poll: how long the on-chain counters get to show the victim lagging,
#: and the gap between reads.
PRODUCTION_POLL_BUDGET_S = 90
PRODUCTION_POLL_S = 2
#: `:89`, `:98` — the rejoin poll. This is the HEIGHT half only; the SIGNING half below has its
#: own budget, because it measures an event that can only happen after this one has passed.
LIVENESS_REJOIN_S = 120
LIVENESS_REJOIN_POLL_S = 2
#: The SIGNING half's cadence. Five seconds, not the rejoin poll's two: every iteration reads the
#: victim's WHOLE container log (the only place the promotion is observable — see
#: `SIGNER_PROMOTED_LINE`), and at two seconds that is ~45 full `docker compose logs` reads per
#: cycle against a node the case is simultaneously measuring.
LIVENESS_SIGNING_POLL_S = 5
#: Slack over the worst honest wait, which `liveness_signing_budget` prices in blocks.
LIVENESS_SIGNING_SLACK_S = 60
#: `:115` — the bootstrap-DKG budget before ANY disruption. A victim stopped before its epoch-2
#: ceremony is permanently shareless on rejoin (no reshare in v1), so this is not a settle wait:
#: it is what makes the case exercise the SUPPORTED restart path.
LIVENESS_DKG_WAIT_S = 180
#: `:105` — how far past the epoch-2 boundary the bootstrap wait targets.
DKG_SETTLE_BLOCKS = 8
#: `:49` — the graceful-stop ceiling, the same 40 s reth's `on_graceful_shutdown` gets everywhere.
LIVENESS_STOP_TIMEOUT_S = 40

#: `:127-130` — the catch-up spectrum, as `(victim index, gap in EPOCH_INTERVALs, +blocks)`.
#: The ORDER is the case: deep first (the per-epoch soft-enter walk over several boundaries),
#: then a single boundary cross, then a within-epoch gap that crosses none, then an IMMEDIATE
#: re-kill of the node that just rejoined — which is the one that catches stale catch-up state
#: (`highest_entered_epoch` / `highest_observed_epoch`) carried across restarts.
#:
#: WHICH OF THEM ACTUALLY WALK, stated plainly because three of the four names promise a walk they
#: do not get. The steady-state re-jump gate is `min(JUMP_THRESHOLD=1024, epochBlockInterval)`
#: (`rejump_threshold`, dpos.rs:2543), i.e. the INTERVAL itself, so a cycle re-jumps whenever its
#: gap exceeds one epoch:
#:
#:   cycle 1  `3*I + 1`  — over by `2*I + 1`. DELIBERATE: "DEEP" is the deep path.
#:   cycle 2  `I + 1`    — over by ONE BLOCK. Named "SINGLE epoch-boundary cross", which is a
#:                        statement about boundaries, not about the jump gate; the two happen to
#:                        be the same number and the gap lands on the far side of it.
#:   cycle 3  `5`        — the only cycle that really walks.
#:   cycle 4  `I + 1`    — same as cycle 2.
#:
#: NOT CHANGED HERE, and flagged rather than silently retuned: `cycle_walks` computes it and
#: `tests/test_smoke_onchain_verdicts.py` pins the four answers, so the next person to read
#: "single-boundary cross" as "walks one boundary" is contradicted by a test rather than by a live
#: run. Cycles 2 and 4 are also over the gate once boot drift is priced in (the victim keeps
#: falling behind while it boots), so trimming the `+ 1` would not buy the walk back — the gap
#: would have to drop well UNDER the interval, which is cycle 3's job already. What the crossing
#: costs is exactly what `evaluate_liveness_signing` now measures: a re-jumped member is
#: verify-only until it can spawn its per-epoch engine.
LIVENESS_CYCLES = ((3, 3, 1), (2, 1, 1), (1, 0, 5), (3, 1, 1))

#: `epoch_manager.rs:838-841` — the stable greppable token of the ONE `spawn_engine` call site
#: (`:831`), logged only when the spawn RETURNED TRUE. That is what makes it the positive control:
#: every path that leaves the member verify-only — the share-gate (`:718`), the boundary-block
#: defer (`:734`), the promote VALUE gate (`:768`) and the promote SHARE gate (`:797`) — returns
#: BEFORE it, from the same match arm.
#:
#: Duplicated from `verdicts_boundary.PROMOTED_LINE` rather than imported, deliberately and for
#: the reason that file already gives on its own copy: a boundary-side edit must not be able to
#: silently retarget a different case's gate.
SIGNER_PROMOTED_LINE = "promoted to Signer in-process"
#: `epoch_manager.rs:745` — the defer branch, read ONLY as a diagnostic on the failure path. It is
#: not the gate and must not become one: one or two defers during ordinary backfill are normal and
#: self-heal, so "no defers" is neither necessary nor sufficient for "is signing".
SIGNER_DEFER_LINE = "signer spawn deferred"

#: The promote line carries `?epoch`, a `#[derive(Debug)]` newtype, so it renders `epoch=Epoch(4)`
#: — the bare-`u64` form is accepted too because a future `%epoch` would emit it. The `[=:]` is
#: REQUIRED and not optional: without it the `epoch` of "per-epoch BFT engine started" is a
#: candidate match, and the first number after it on the line would be read as the epoch.
_SIGNER_EPOCH_RE = re.compile(r"epoch[=:]\s*(?:Epoch\()?(\d+)")


def cycle_gap(interval, epochs, extra) -> int:
    """`3 * EPOCH_INTERVAL + 1` / `EPOCH_INTERVAL + 1` / `5` — the gap of one cycle."""
    return int(epochs) * int(interval) + int(extra)


def cycle_walks(interval, epochs, extra) -> bool:
    """Does this cycle's gap stay UNDER the steady-state re-jump gate (i.e. does the victim walk
    the boundaries, rather than teleport past them)?

    Reported, never enforced: the case wants the deep path in cycle 1 on purpose. See
    `LIVENESS_CYCLES` for which of the four cross it and why none of them is being retuned."""
    return cycle_gap(interval, epochs, extra) <= rejump_threshold(interval)


def epoch_of(height, interval, activation) -> int:
    """Relative epoch of an absolute height (`OriginEpocher`: `(h - origin) / length`).

    Duplicated from `verdicts_boundary.epoch_of` on the same "no cross-case retargeting" rule as
    `SIGNER_PROMOTED_LINE`; it is three tokens of arithmetic pinned by its own test."""
    return (int(height) - int(activation)) // int(interval)


def liveness_signing_budget(interval) -> int:
    """How long the SIGNING half gets, after the height half has already passed.

    Priced on the worst HONEST wait rather than on a round number. A victim that re-jumped lands
    with the `E-1` boundary block missing from its marshal; boundary seeding (`outer.rs:856`,
    `executor.rs:1942`) is what puts it there so the member promotes inside the LANDING epoch, but
    if that misses the height, the fallback is the next epoch boundary — at most one full epoch
    away, and at 1 blk/s one epoch is `interval` seconds. Plus `LIVENESS_SIGNING_SLACK_S` for the
    reconcile edge and the log read.

    NOT a place to buy a green. A budget that has to be raised to pass is the case reporting that
    the member takes longer than a whole epoch to sign again, which is the finding, not the noise.
    """
    return int(interval) + LIVENESS_SIGNING_SLACK_S


def promoted_epoch_counts(log_text: str) -> dict:
    """`{epoch: how many promote lines named it}` over an ANSI-STRIPPED log.

    THE STRIP IS THE READER'S WHOLE VALIDITY. The node writes SGR escapes INSIDE its `key=value`
    pairs, so a raw line renders `epoch<ESC>=<ESC>Epoch(4)` and this regex matches NOTHING — a
    silent reader, which on a gate that waits for a line to APPEAR is indistinguishable from a
    member that never promoted. `ctx.logs_all` strips unconditionally (§2.4 item 2); bash has to
    pipe through `strip_ansi` and `case-liveness.sh` does.

    A COUNT per epoch, not a set: the victim was already a signer for the live epoch before it was
    stopped, so the epoch is in the log BEFORE the restart too, and a set difference would cancel
    exactly the observation the gate is after."""
    out = {}
    for line in (log_text or "").splitlines():
        if SIGNER_PROMOTED_LINE not in line:
            continue
        m = _SIGNER_EPOCH_RE.search(line)
        if m:
            out[int(m.group(1))] = out.get(int(m.group(1)), 0) + 1
    return out


def defer_count(log_text: str) -> int:
    """`grep -c 'signer spawn deferred'` over the same already-captured text. Diagnostic only."""
    return sum(1 for line in (log_text or "").splitlines() if SIGNER_DEFER_LINE in line)


def promoted_epochs_since(before: dict, after: dict):
    """Every epoch whose promote count ROSE between the two snapshots, ascending."""
    return sorted(e for e, n in (after or {}).items() if int(n) > int((before or {}).get(e, 0)))


def liveness_signing(before: dict, after: dict, min_epoch) -> bool:
    """THE SIGNING GATE: since the pre-stop snapshot, the victim promoted to Signer for an epoch
    at or above `min_epoch`.

    BOTH halves of that sentence are load-bearing.

    A DELTA, because `docker compose stop`/`start` keeps the SAME container and its log persists
    across the cycle — an absolute grep is satisfied by the promotion the victim did on its way
    into the epoch BEFORE it was killed, which is the reading of a node that is now dead.

    AN EPOCH FLOOR, because the delta alone is satisfied by a STALE promotion. A restarted victim
    boots on its persisted tail, in the epoch it was killed in, and reconciles there first: it can
    full-enter that epoch and log the line for it while the committee is three epochs ahead. That
    promotion is real and useless — the member is signing for an epoch nobody is voting on. The
    floor is `epoch_of(pre + gap)`, the epoch the CHAIN PROVABLY REACHED this cycle (step 1
    hard-asserts `wait_finalized_ge(pre + gap)` before the restart), and it is the same anchor the
    height floor already uses."""
    for e in promoted_epochs_since(before, after):
        if e >= int(min_epoch):
            return True
    return False


def evaluate_validator_addresses(addrs):
    """`:31` — four addresses, or the case cannot map a victim to its on-chain identity.

    Fail-loud rather than "use what we got": a short list would leave `ADDR[3]` unset, and under
    `set -u` bash aborts. Here it would index-error three assertions later, in the participation
    read, where it would look like a getter problem."""
    n = len(addrs or [])
    if n == EXPECTED_VALIDATORS:
        return True, ""
    return False, (f"expected {EXPECTED_VALIDATORS} validator addresses, got {n}: "
                   f"{' '.join(addrs or [])}")


def production_hit(vseen, rseen) -> bool:
    """`:78` — the poll's BREAK condition: both reads are real, the hub has been seen at least
    once, and the victim is strictly behind it.

    `rseen > 0` is not a formality. Early in a window both counters are 0 and `0 < 0` is false,
    so without it the loop would be waiting on a comparison that cannot yet be true; WITH it the
    loop is explicitly waiting for the hub to accumulate evidence first."""
    if credit_state(vseen) != STATE_OK or credit_state(rseen) != STATE_OK:
        return False
    return int(rseen) > 0 and int(vseen) < int(rseen)


def evaluate_production(epoch, vseen, vcerts, rseen, rcerts):
    """`:81-83` — the offline victim signed strictly FEWER certs than the always-up hub.

    This is the exact dual of the removed consecutive-miss counter: "misses rise" became "seen
    lags the hub". `<` rather than `== certs` on purpose — it tolerates a transient view-change
    miss on the hub, the same slack the old `vmc > refmc` carried, without which the case would
    be flaky on a hub that missed one cert for reasons that have nothing to do with the victim."""
    if production_hit(vseen, rseen):
        return True, ""
    return False, (f"production credit wrong (epoch={epoch} victim produced={vseen}/"
                   f"blocksInEpoch={vcerts} hub produced={rseen}/blocksInEpoch={rcerts})"
                   f"{credit_diagnosis(vseen, 'victim')}"
                   f"{credit_diagnosis(rseen, 'hub')}")


def liveness_rejoined(v0: str, vn: str, peers, floor_dec, producer_hash_at=None) -> bool:
    """`:104` — the victim is back on the hub's chain, PAST `floor_dec`, AND has a live reth
    devp2p peer.

    All three, and the peer count is the mechanism under test: the rejoin rides the CONSENSUS
    plane for the epoch walk but the BLOCKS arrive over reth devp2p, so a victim that matched
    the hub with zero peers would be reporting a head it did not sync.

    The alignment half is SAME-HEIGHT identity via `converge.aligned_reading`, not tip-vs-tip.
    This was the tightest budget in the suite — 120s, for a victim that in cycle 1 has to walk
    three epoch boundaries — and a rejoining node is BEHIND the hub for most of that walk by
    construction, so requiring the two tips to coincide made the pass a coincidence and the
    flake inevitable. The fork check is kept at the victim's own height (a victim at height h on
    a different chain still fails, same height or not) and the head guard still stops two
    unreachable nodes (`"null|null"` on both sides, trivially equal) from reading as a
    rejoin — §2.4 item 5.

    THE FLOOR IS `pre + gap`, AND DROPPING IT BROKE THE CASE. The old `v0 == vn` equality was
    silently doing a second job: it could not be satisfied until the victim had actually reached
    the hub, which is what kept the NEXT cycle from stopping a second validator while the first
    was still hundreds of blocks behind. Same-height identity alone has no such property — a
    victim on the right chain at height 135 "rejoins" against a hub at 249 — and cycle 2 then
    stopped its own victim with the cycle-1 victim still ~114 blocks down: 2 of 4 signers, a
    correct BFT stall, a failed case. `pre + gap` is the floor because it is what the CHAIN
    PROVABLY REACHED during this cycle: step (1) hard-asserts `wait_finalized_ge(pre + gap)`
    before the victim is restarted at all. The victim's own `pre` is not a floor — it clears
    that the instant it comes back on its persisted tail, which is precisely the vacuous pass.
    `aligned_reading`'s floor is STRICT and applies PER READER, so the hub is checked too."""
    if converge.aligned_reading([("hub", v0), ("victim", vn)], int(floor_dec),
                                producer_hash_at) is None:
        return False
    try:
        return int(peers) > 0
    except (TypeError, ValueError):
        return False


# ── the two rejoin failures, and why they must not read alike ─────────────────────────
#
# The rejoin gate has TWO halves and they fail for different reasons, at different places, with
# different next steps. Keeping one message for both cost a live run: `finalized=241, pre=241` at
# cycle 2 was reported as a chain that would not advance, and the operator went looking at the
# chain. The chain was correct. The cycle-1 victim was height-aligned and NOT SIGNING, so stopping
# a second validator left 2 live signers of 4 against a quorum of 3 — a correct BFT stall.
#
#   HEIGHT half   `liveness_rejoined` — the victim never came back on the hub's chain above the
#                 floor. It is behind, forked, unreachable or peerless. Look at the VICTIM.
#   SIGNING half  `liveness_signing` — it came back, on the right chain, past the floor, and is
#                 still verify-only: no per-epoch BFT engine, so no proposals and no votes. Look
#                 at the engine-spawn gate (`epoch_manager.rs:565` `reconcile_roles`) and at what
#                 the victim's own log says it is waiting for.


def liveness_not_rejoined_message(svc, peers, vn, v0, floor_dec) -> str:
    """`:127` — the HEIGHT half's diagnostic, plus the clause that says which half this is.

    Same four readings bash prints, re-read on the failure path (that is why the caller hands
    them in from a lambda rather than an f-string: on a PASS they are never issued)."""
    return (f"{svc} did not rejoin (peers={peers}, {svc}={vn}, v0={v0}, "
            f"floor=pre+gap={floor_dec}) — it never came back on the hub's chain above the "
            "floor, so the SIGNING half was never reached. This is the victim's own catch-up: "
            "check its reth devp2p peers and its consensus-plane walk, not the committee.")


def evaluate_liveness_signing(before: dict, after: dict, min_epoch, svc, log_text: str,
                              floor_dec=None):
    """THE SIGNING HALF. Height-aligned is not back: `liveness_signing`, with the message that
    tells the two failures apart.

    The gap between the two is the whole finding of 2026-08-01. A re-jumped member catches up by
    HEIGHT — devp2p hands it the blocks and the height gate goes green — while its per-epoch
    engine never spawned, because the jump teleported the marshal floor past the `E-1` boundary
    block the spawn gate needs (`epoch_manager.rs:734-749`). Three of this case's four cycles gap
    past the re-jump gate (see `LIVENESS_CYCLES`), so this is the NORMAL path here, not a corner.

    The failure names the defer count because that is the one line that says WHICH of the eight
    spawn conditions is holding: a rising `signer spawn deferred` is the boundary-block gate and
    points at boundary seeding (`outer.rs:856`, `executor.rs:1942`); a flat one with no promote at
    all points at the share/VALUE/SHARE gates, which log their own reasons."""
    if liveness_signing(before, after, min_epoch):
        return True, ""
    fresh = promoted_epochs_since(before, after)
    defers = defer_count(log_text)
    stale = (f"it promoted only for epoch(s) {fresh} — BELOW the floor, i.e. for an epoch the "
             "committee has already left (a restarted victim reconciles its persisted tail first)"
             if fresh else
             f"no {SIGNER_PROMOTED_LINE!r} line appeared at all since the pre-stop snapshot")
    return False, (
        f"{svc} is HEIGHT-ALIGNED but NOT SIGNING"
        + (f" (it reached the floor pre+gap={floor_dec})" if floor_dec is not None else "")
        + f": {stale}, so it holds no per-epoch BFT engine for epoch >= {min_epoch} — verify-only, "
          "NO proposals and NO votes. Stopping the next validator now would leave 2 live signers "
          "of 4 against a quorum of 3, which stalls the chain and reads as a product bug; that is "
          "exactly the run this gate exists to stop. "
          f"{svc} logged {SIGNER_DEFER_LINE!r} {defers} time(s) — if that is rising, the "
          "engine-spawn gate is waiting on the E-1 boundary block (epoch_manager.rs:734-749) and "
          "the place to look is boundary seeding after the re-jump; if it is flat, read the "
          "victim's log for the share-gate / promote VALUE-gate / promote SHARE-gate lines "
          "instead. NOT a budget to raise blindly: this member takes longer than a whole epoch to "
          "sign again, which is the finding.")


# ══ smoke-byzantine ═══════════════════════════════════════════════════════════════════

#: `case-byzantine.sh:14` — the equivocation overlay. It sets `FLUENT_DPOS_BYZANTINE=equivocate`
#: on validator-3 and needs the image built with the `dpos-devnet-byzantine` cargo feature.
BYZANTINE_OVERLAY = "docker-compose.byzantine.yml"
#: `:15` — validator-3's reth NEVER finalizes, so it must be dropped from the post-swap
#: alignment set or the bring-up fails a case that is behaving exactly as designed.
BYZANTINE_EXCLUDE = "validator-3"
#: `:22` — which committee slot equivocates.
BYZANTINE_VICTIM_IDX = 3

#: `:26`, `:34` — how long the slash→jail gets, and the poll cadence.
JAIL_TIMEOUT_S = 200
JAIL_POLL_S = 3
#: `:54` — the post-jail liveness assertion: this many blocks, in this many seconds.
POST_JAIL_BLOCKS = 3
POST_JAIL_S = 90
#: `:41`, `:44`, `:46`, `:59` — fail-path log depths. The two marker greps read deep (600); the
#: plain tails are shallower.
BYZ_MARKER_TAIL = 600
BYZ_LOG_TAIL = 120
BYZ_STALL_TAIL = 200

#: `getValidatorStatus`'s status byte for a jailed validator. There is no public `tombstoned()`
#: getter, so this IS the observable (Addendum D).
STATUS_JAIL = "3"

#: `:42`, `:45` — the two `grep -iE` alternations of the failure diagnostic. Kept as data so the
#: dump is the same set of markers bash looks for, rather than a paraphrase.
BYZ_EQUIVOCATOR_MARKERS = ("BYZANTINE", "equivocat", "cannot sign", "no local share",
                           "decode vote", "broadcast")
BYZ_SLASHER_MARKERS = ("slash", "conflict", "equivocat", "evidence")


def evaluate_jailed(status):
    """`:36` — the equivocator was slashed on-chain and jailed.

    An EMPTY status is a failed read, not "not jailed", and it lands here as a failure either
    way; the message names it so the operator is not sent looking for a slasher bug when the RPC
    was the thing that did not answer."""
    st = (status or "").strip()
    if st == STATUS_JAIL:
        return True, ""
    if not st:
        return False, ("validator-3 not jailed within 200s (getValidatorStatus.status read "
                       "EMPTY — the RPC did not answer, so the jail could not be observed)")
    return False, f"validator-3 not jailed within 200s (getValidatorStatus.status={st})"


def evaluate_post_jail_liveness(advanced, post_jail):
    """`:54-58` — the honest 3-of-4 quorum KEPT finalizing after the equivocator was dropped.

    The SECOND assertion of this case and not a victory lap: a post-jail committee-drop
    epoch-boundary wedge is a real DPoS failure class, and the pre-jail converge cannot see it —
    the committee only changes at the jail."""
    if advanced:
        return True, ""
    return False, f"chain stalled after jail (finalized stuck at ~{post_jail})"


def grep_markers(logs: str, markers) -> str:
    """`grep -iE "a|b|c"` over an already-ANSI-stripped log. Case-insensitive, substring."""
    lowered = [m.lower() for m in markers]
    return "\n".join(line for line in (logs or "").splitlines()
                     if any(m in line.lower() for m in lowered))


# ══ smoke-cert-catchup ════════════════════════════════════════════════════════════════

#: `case-cert-catchup.sh:79-91` — the four greppable log signatures, verbatim from the source.
#: Two are POSITIVE (the path fired) and two are NEGATIVE (the pre-fix behaviour must be gone).
#:
#: PARK_LOG is the ACTION half of the guard-#2 warn! (executor.rs:1473-1477), which the Rust
#: source splits over two lines with a `\` continuation:
#:     "guard #2: committee-attested body at h+K not backfilled yet; \
#:     PARKING derive + hinting peers (event-driven re-poke, no give-up timer)"
#: The two-word action token is the shortest unique slice of it, and that is not a style
#: preference. There is NO cert in this condition any more: the pre-B′ cert-miss park classes were
#: DELETED (DPOS_ARCHITECTURE.md, "the one remaining park is guard #2's absent h+K attestation
#: body"), and that deletion REWROTE the condition half of this very `warn!` —
#: `finalization cert not local yet; PARKING derive + hinting peers (…)` became
#: `guard #2: committee-attested body at h+K not backfilled yet; PARKING derive + …`. The ACTION
#: half survived byte-identical; the condition half is the one that moved. Until 2026-07-31 both
#: trees grepped the old condition, so the park gate could not fire at ANY gap. (HEAD's
#: executor.rs:1133 still emits the old wording — the rewrite lives in the working tree, which is
#: what docker builds; soak bundles pin the changeover between bundle-20260707T182226Z and
#: bundle-20260717T074614Z.) `test_smoke_onchain_cases.py` now asserts this string against the
#: product source, which is the guard that was missing then.
PARK_LOG = "PARKING derive"                           # executor.rs:1473, the guard-#2 park warn!
REJUMP_LOG = "EL-sync fast-forwarded the anchor"      # cold_start_jump.rs:874, the re-jump land
OLD_FATAL = "cannot derive beacon prev_randao"        # DELETED pre-fix ERROR; regression tripwire
EXIT_LOG = "OuterEngine exited cleanly"               # node/dpos.rs:539, the consensus_exit

#: `:65-66` — the case runs on 64-block epochs, mirrored into the host-side chain math. At the
#: default 32 the pure-park window (`gap < re_jump_threshold`) is too shallow to force a
#: multi-block derive-walk at all, so this is the assertion's premise, not a preference.
#:
#: ═══ DO NOT RAISE THIS TO 128. IT DOES NOT BOOT. ═══════════════════════════════════════════
#:
#: The interval IS the re-jump ceiling (`min(JUMP_THRESHOLD=1024, epochBlockInterval)`,
#: dpos.rs:2543 validator / :3237 follower), so raising it is the obvious way to buy the
#: derive-walk more headroom — see `catchup_gap_ceiling`. It was tried, live, on 2026-07-31, and
#: the stack never reached the case at all:
#:
#:     waiting for the sequencer to finalize >= dposActivationBlock=256 (relative epoch 0)
#:       sequencer finalized 256 >= activation 256; proceeding to swap
#:     FAIL: DPoS chain did not converge past anchor 0x100
#:
#: The chain produced ZERO blocks past the anchor. validator-0 logged `dpos: proposing order
#: block height=257` and every view from 4 to 115 answered `proposal failed verification` —
#: including on the proposer's OWN node — for the full 120 s converge budget, at EPOCH 0, before
#: any DKG and before any of this case's logic ran. It is a structural reject, not a timeout: no
#: budget anywhere makes it pass. Intervals 32 and 64 both reached `DPoS stack live` on the SAME
#: build in the same run (`smoke-vrf-dkg-durability`, `smoke-vrf-dkg-halt`), so the interval is
#: the variable. The rejection itself is a real product finding with its own ticket; it is not
#: this case's to carry. The lever here is the GAP instead — see `CATCHUP_GAP`.
CATCHUP_EPOCH_INTERVAL = 64
#: `:72-74` — victim, gap and the optional deep cycle. validator-2 is a SPOKE that pins the hub
#: as a trusted peer, which is the prod-observed victim shape.
#:
#: THE GAP IS THE LEVER, because the interval above cannot be. It was 40, paired with
#: `CATCHUP_NETEM_DELAY_MS` 1000, and that pair was measured to park — 4 parks, `deferred_height`
#: peak 309. Then the identical config re-ran and FAILED: park warns 0 → 0, re-jump landings
#: 1 → 2. The delay that opens the park window is the same force that grows the victim's gap
#: toward the re-jump threshold.
#:
#: 40 WAS NOT OVER THE CEILING — and that is the lesson, not a footnote. `catchup_gap_ceiling(64,
#: 1000)` is 46, so 40 sat 6 blocks under it, and `effective_catchup_gap(40, 1000)` = 57 sat 7
#: blocks under the 64-block threshold. It still lost the walk on a re-run. "Fits under the
#: ceiling" is not the bar; "fits with room" is. At that delay 28 gave 18 blocks of ceiling
#: headroom and 21 under the threshold — three times the margin a single re-run was able to eat.
#:
#: 28 AT delay 1000 WAS ITSELF MARGINAL ON THE OTHER SIDE: two independent bring-ups of that exact
#: config went 0 parks (bash) and 1 park (py) — it parked about half the time. The fix was to spend
#: the margin on the DELAY rather than the gap, and it worked: at `CATCHUP_NETEM_DELAY_MS` 3000,
#: two independent bring-ups went 2 parks and 3 parks, neither re-jumped, `deferred_height` peak
#: 305 on both. That is the configuration this case now ships, and `CATCHUP_GAP_CEILING` follows
#: the default delay — so the live headroom is `32 - 28 = 4` blocks, NOT the 18 that 1000 ms gave.
#:
#: THE MARGIN IS THIN AND IT WAS CHOSEN ON EVIDENCE. Read `28 <= 32` with this attached: the model
#: calls this configuration marginal. `effective_catchup_gap(28, 3000)` is 53 against a 64-block
#: threshold, and the second-order pass in `catchup_gap_ceiling` — the stall adding backlog which
#: adds a round which adds stall — puts it at 59, inside the ~10-block band that docstring defines
#: as marginal. Four live bring-ups say otherwise: 4/4 parked, 0/4 re-jumped.
#:
#: The measurement wins over the model here DELIBERATELY, because the model has now been shown
#: wrong in BOTH directions on this very case. It under-priced: 40 at delay 1000 sat 6 blocks under
#: its ceiling and 7 under the threshold, and still lost the walk to `maybe_re_jump` on a re-run.
#: And it over-priced badly enough to forbid the configuration that works: the threshold-priced
#: `rounds` it used to carry put (28, 3000) at a stall of 27 and a ceiling of 27, so the case
#: refused to start at its own gap. A first-order approximation that errs in both directions is a
#: guard-rail, not an oracle. It is kept to catch gross misconfiguration; it does not get a vote
#: against four green bring-ups.
#:
#: WHAT WOULD REOPEN THIS: a re-jump landing at the default (the model's failure mode, and the
#: thing 4 blocks of headroom is thin against), or a park count that drops back toward zero. Either
#: means the margin really is too thin, and the answer then is a SMALLER GAP at this delay —
#: `catchup_gap_ceiling(64, 3000)` = 32 leaves room to come down, and the interval cannot go up
#: (it does not boot).
CATCHUP_VICTIM = "validator-2"
CATCHUP_GAP = 28
#: `:76` — the effective re-jump gate is `min(JUMP_THRESHOLD=1024, epochBlockInterval)`
#: (dpos.rs:2543 validator / :3237 follower).
JUMP_THRESHOLD = 1024
#: `:105`, `:107` — the bootstrap-DKG wait before any disruption (mirrors `case-liveness`).
#:
#: SIZED FOR THIS CASE'S INTERVAL, and it does not travel: `_wait_bootstrap_dkg` waits for
#: `activation + 2*interval + DKG_SETTLE_BLOCKS` = 264, off a `2*interval` = 128 anchor, so it
#: covers ~136 blocks at 1 blk/s and 360 s is 2.6× headroom. Anyone changing
#: `CATCHUP_EPOCH_INTERVAL` has to move this with it — read that constant's warning first.
CATCHUP_DKG_WAIT_S = 360
#: `:135` — the graceful stop, and `:159` the rejoin budget and `:168` its cadence.
CATCHUP_STOP_TIMEOUT_S = 40
CATCHUP_REJOIN_S = 240
CATCHUP_REJOIN_POLL_S = 2
#: `:109`, `:138`, `:147`, `:172` — fail-path log depths, kept distinct as bash has them.
CATCHUP_LOG_TAIL = 120
CATCHUP_FLUSH_TAIL = 80
CATCHUP_DEEP_TAIL = 200

#: THE LEVER. One-way egress delay added to the VICTIM for its catch-up window only, and the
#: reason this case can gate on the park at all.
#:
#: Guard #2 parks where the executor's derive DRAINS the marshal's contiguous dispatched prefix
#: faster than repair extends it (MAX_REPAIR=20 / MAX_PENDING_ACKS=16, outer.rs:223,229). At zero
#: latency the fetch always wins on a 4-node LAN with small bodies, so the park fires at NO value
#: of `CATCHUP_GAP` — and depth is not the lever either (see `evaluate_park_exercised`). Add an
#: RTT and each repair batch pays it while the derive stays local and untouched: the dispatched
#: prefix advances in <=20-block steps per round trip and the derive can reach its edge.
#:
#: NOT a crutch — the production condition restored. Real validators are geographically
#: distributed and always pay an RTT; a zero-latency LAN is the unrealistic condition. The soak
#: shows it empirically: 43 of 323 bundles carry a park and its one relevant difference is its
#: geo-latency toggle, which bakes per-region `tc netem` into every validator
#: (`gen-soak-compose.sh:146-168`). Same tool, same verb, same image (`Dockerfile:87` installs
#: iproute2); this case applies ONE uniform delay to ONE node instead of a five-region matrix.
#:
#: `delay`, never `rate`: throttling bandwidth would also slow the live finalization stream that
#: keeps the ordering tip climbing — the wrong side of the race. A CPU throttle (the
#: `docker update --cpus` lever `asserts_fault` uses) is rejected for the mirror reason: it slows
#: the DERIVE, which must stay fast.
#:
#: 3000, not the 1000 this started at, and the change is MEASURED: at 1000 the case parked on about
#: half its bring-ups (0 parks then 1 park across two), at 3000 it parked on all of them (2 then 3,
#: `deferred_height` peak 305, no re-jump either time). More RTT per repair batch buys the derive
#: its overtake; the derive itself is untouched. The cost is stall, which is why
#: `CATCHUP_GAP_CEILING` drops from 46 to 32 — see `CATCHUP_GAP` for why 4 blocks of headroom is
#: accepted here on evidence.
#:
#: Env-overridable as `CERT_CATCHUP_DELAY_MS` in BOTH trees, and the two defaults are asserted
#: equal in `tests/test_smoke_onchain_cases.py` — one live iteration retunes it in one place.
CATCHUP_NETEM_DELAY_MS = 3000
#: Every service sits on the single `fluent-net` bridge (`docker-compose.yml:10-15`), so the
#: interface name is deterministic — the same assumption `gen-soak-compose.sh:152` makes.
CATCHUP_NETEM_IFACE = "eth0"
#: The case's compose overlay. It carries ONE thing, `NET_ADMIN` (what `tc qdisc add` needs), and
#: shapes nothing on its own. A per-case overlay rather than an edit to the shared
#: `docker-compose.yml`: this is one case's requirement, not the stack's.
CATCHUP_OVERLAY = "docker-compose.cert-catchup.yml"


def rejump_threshold(interval) -> int:
    """`:76` — `min(EPOCH_INTERVAL, 1024)`. The gap above which a catch-up re-jumps instead of
    walking, so cycle 1's gap has to sit BELOW it or the re-jump steals the derive-walk the case
    is trying to observe."""
    return min(int(interval), JUMP_THRESHOLD)


def deep_gap(interval) -> int:
    """`:75` — `2 * EPOCH_INTERVAL + EPOCH_INTERVAL / 2`, the optional deep cycle's gap. Integer
    division, as bash's `$(( ))` is."""
    return 2 * int(interval) + int(interval) // 2


#: The frontier keeps climbing at 1 blk/s while the restarted victim boots — MEASURED at 7-10
#: blocks between `docker compose start` and its first `Update::Tip` — so the gap the executor
#: actually faces is never the gap the case asked for.
CATCHUP_BOOT_BLOCKS = 10
#: `outer.rs:223` — the marshal repairs in a sliding window of at most this many bodies per
#: round, so a backlog of B costs `ceil(B / MAX_REPAIR)` fetch round trips.
CATCHUP_MAX_REPAIR = 20


def catchup_stall_blocks(backlog, delay_ms=CATCHUP_NETEM_DELAY_MS) -> int:
    """The blocks of gap the catch-up ACCRUES while it is not deriving, at `CATCHUP_NETEM_DELAY_MS`
    of added one-way RTT.

    Wall-clock converts to gap one for one: the committee produces a block a second, so a second
    the victim spends waiting is a block it falls further behind. Two things make it wait, and
    both are priced in round trips because the delay is exactly one round trip:

      * FETCH — the backlog drains `CATCHUP_MAX_REPAIR` bodies per repair round, so it costs
        `rounds = ceil(backlog / MAX_REPAIR)` round trips;
      * PARKS — each guard-#2 park waits on one further body to arrive, i.e. one further round
        trip. The measured run produced 4 parks against 3 fetch rounds, so parks are priced at
        `rounds + 1`.

    Hence `stall = ceil((rounds + parks) * delay / 1000) = ceil((2*rounds + 1) * delay / 1000)`,
    in blocks. `ceil` on the TOTAL rather than per round, and a zero delay costs zero."""
    rounds = -(-int(backlog) // CATCHUP_MAX_REPAIR)
    rtts = 2 * rounds + 1
    return -(-rtts * int(delay_ms) // 1000)


def effective_catchup_gap(gap, delay_ms=CATCHUP_NETEM_DELAY_MS, boot=CATCHUP_BOOT_BLOCKS) -> int:
    """`GAP + boot + stall` — the gap `maybe_re_jump` (executor.rs:1765-1790) actually sees when
    it decides whether to walk or to fast-forward. The case asks for `gap`; the victim faces this.

    The stall is priced against a backlog of `gap + boot`, the backlog the walk really starts
    from."""
    backlog = int(gap) + int(boot)
    return backlog + catchup_stall_blocks(backlog, delay_ms)


def catchup_gap_ceiling(interval, delay_ms=CATCHUP_NETEM_DELAY_MS,
                        boot=CATCHUP_BOOT_BLOCKS) -> int:
    """The largest `CERT_CATCHUP_GAP` that still leaves `effective_catchup_gap` BELOW the
    steady-state re-jump threshold — i.e. the gap above which the derive-walk is handed to
    `maybe_re_jump` and there is nothing left to park on.

    SOLVED, not closed-form, because `effective_catchup_gap` is a function of the gap: the scan
    below returns the largest gap under the threshold. `effective` is monotone non-decreasing in
    the gap (`rounds` only grows), so the scan is exact.

    THE ROUNDS COME FROM THE ACTUAL BACKLOG, and an earlier revision priced them at a
    FULL-THRESHOLD backlog instead, claiming that over-pricing "can only make the ceiling safer".
    That was wrong twice. It is not conservative in any useful sense — it is simply a different,
    unmeasured number — and it FORBIDS VALID CONFIGURATIONS: it priced (28, 3000 ms) at a stall of
    27 and refused the run, where the backlog-based model prices it at 15 and the effective gap at
    53, eleven blocks clear of the threshold. The measurement settles it: at gap 40 / delay 1000
    the observed effective gap was 57 = 40 + boot 10 + stall 7, and `ceil(50/20) = 3` rounds gives
    exactly 7 while `ceil(64/20) = 4` rounds gives 9. `test_smoke_onchain_verdicts.py` pins the 57.

    At interval 64 / delay 1000 ms the ceiling is 46; at 64 / 3000 ms it is 32; at 128 / 1000
    ms it would be 104, but 128 does not boot (see `CATCHUP_EPOCH_INTERVAL`). The literal 52
    this constant used to be is refused by both (`effective(52, 1000)` = 71, past the threshold).

    WHERE IT IS NOW MORE PERMISSIVE, stated plainly: this is a FIRST-ORDER fixed point, not an
    iterated one. The stall itself adds blocks to the backlog, which can add a repair round, which
    adds stall. At (28, 3000) the honest second pass is `ceil(53/20) = 3` rounds → stall 21 →
    effective 59, still under 64 but with 6 blocks of margin rather than 11. Any configuration
    whose first-order effective gap lands within ~10 of the threshold should be read as marginal,
    and that is exactly the regime the old threshold-based rounds happened to catch — by
    over-pricing everything, including the configurations that work.

    Passing a THRESHOLD where an interval is expected is safe and is what
    `evaluate_park_exercised` does: `rejump_threshold` is idempotent below `JUMP_THRESHOLD`."""
    threshold = rejump_threshold(interval)
    best = 0
    for gap in range(1, threshold):
        if effective_catchup_gap(gap, delay_ms, boot) < threshold:
            best = gap
    return best


#: The gap's PRACTICAL bound at this case's own geometry — DERIVED, never hand-set again.
CATCHUP_GAP_CEILING = catchup_gap_ceiling(CATCHUP_EPOCH_INTERVAL)


def catchup_rejoined(v0: str, vn: str, floor_dec, producer_hash_at=None) -> bool:
    """`:172` — the victim is back on the hub's chain AND past `floor_dec` (`pre + gap`).

    A floor is what makes this a CATCH-UP check rather than a mere agreement check: a victim that
    came back and sat on its persisted tail would eventually match a stalled hub, and the whole
    premise of the case is that the hub moved `gap` blocks while the victim was down. The floor
    used to be `pre`, which the victim clears on its OWN persisted tail the moment it restarts —
    it proves nothing about catching up. `pre + gap` is the height the CHAIN PROVABLY REACHED
    during this cycle (step (3) hard-asserts `wait_finalized_ge(pre + gap)` before the restart),
    so it is the floor. `aligned_reading`'s floor is strict and applies PER READER (the hub is
    past it too, trivially).

    The equality leg was the defect. The victim is walking a deep gap — cycle 2's is over
    2*EPOCH_INTERVAL — on a chain producing a block a second, so it joins the hub's CHAIN long
    before it reaches the hub's TIP; demanding one byte-identical `"height|hash"` from two
    non-atomic reads of two moving tips made the 240s budget a race that passed by luck. Same
    height on a different block still fails, and so does a different height on a different
    chain: `aligned_reading` reads the hub's block at the victim's own height."""
    return converge.aligned_reading([("hub", v0), ("victim", vn)], int(floor_dec),
                                    producer_hash_at) is not None


def evaluate_no_old_fatal(before, after, victim, label):
    """`:183-187` — THE NEGATIVE. The pre-fix budget-shutdown ERROR must not have reappeared.

    A DELTA, not an absolute count, because `docker compose stop`/`start` keeps the SAME
    container: its log persists across the cycle, so an absolute grep would also count lines from
    before the snapshot and could never distinguish a regression from history."""
    if int(after) <= int(before):
        return True, ""
    return False, (f"[{label}] the OLD fatal budget-shutdown ERROR reappeared on {victim} "
                   f"('{OLD_FATAL}') — the fix regressed")


def evaluate_no_consensus_exit(before, after, victim, label):
    """`:188-192` — THE OTHER NEGATIVE. The executor must have PARKED, not shut down.

    Distinct from the one above and not redundant with it: the old fatal is the ERROR the
    executor logged on the way out, and this is the OuterEngine line the node logs when the
    ack is dropped and the marshal cancels. A future regression that skipped the ERROR and still
    exited would satisfy the first gate and fail this one."""
    if int(after) <= int(before):
        return True, ""
    return False, (f"[{label}] {victim} logged the consensus_exit signature ('{EXIT_LOG}') — "
                   "the executor shut down instead of parking")


def evaluate_park_exercised(before, after, max_gauge, victim, label, threshold,
                            delay_ms=CATCHUP_NETEM_DELAY_MS):
    """`:238-247` — THE GATE, and the only POSITIVE control in the case.

    Both negatives above pass perfectly on a victim that rejoined without ever parking, because
    a path that never ran logs neither of the forbidden lines. So the mandatory cycle asserts the
    park warn count ROSE: the fix's path was actually exercised. bash says it in as many words —
    "A green that never triggered the path is a FALSE PASS".

    THE REMEDY IS NOT "DEEPEN THE GAP", and the message says so, because that is the wrong lever
    twice over. Guard #2 parks when the executor derives `h` and the marshal has no BODY at `h+K`
    — but the marshal repairs in a sliding window of MAX_REPAIR=20 anchored at the ack pointer
    and dispatches MAX_PENDING_ACKS=16 bodies ahead of the acks (outer.rs:223,229), so `h+3` sits
    inside an already-issued batch. A deeper gap adds blocks at the SAME body-ahead-of-derive
    margin (no new parks) while pushing `effective_catchup_gap` toward the re-jump threshold,
    where `maybe_re_jump` abandons the walk. What the park needs is the executor's derive
    DRAINING the dispatched prefix — i.e. the victim's body fetch slowed relative to its derive,
    which is what `CATCHUP_NETEM_DELAY_MS` does and what the message therefore points the next
    operator at."""
    if int(after) > int(before):
        return True, "", int(after) - int(before)
    return False, (
        f"[{label}] guard-#2 park NOT exercised (park warns unchanged: {before} == {after}; "
        f"deferred_height peak sampled={max_gauge}, netem delay={delay_ms}ms on {victim}). The "
        "victim rejoined without ever parking. "
        "DO NOT just raise CERT_CATCHUP_GAP: the park needs the executor's derive to DRAIN the "
        "marshal's contiguous dispatched prefix (MAX_REPAIR=20 / MAX_PENDING_ACKS=16, "
        "outer.rs:223,229), and a deeper gap does not change that ratio — it only risks crossing "
        f"re_jump_threshold={threshold} (with boot drift and catch-up stall priced in, the "
        f"practical ceiling is ~{catchup_gap_ceiling(threshold, delay_ms)}) and letting "
        "maybe_re_jump fast-forward past the walk entirely. "
        "The lever is SLOWING THE VICTIM'S BODY FETCH relative to its derive, and it is already "
        f"applied: RAISE CERT_CATCHUP_DELAY_MS (now {delay_ms}) — more RTT per repair batch, "
        "derive untouched. A green that never triggered the path is a FALSE PASS."), 0


def rejump_note(before, after, victim, label, threshold) -> str:
    """`:207-211` — the DEEP cycle's report. Best-effort by design: a re-jump can fast-forward
    past the derive-walk, so this NOTES what happened and fails nothing. Turning it into a gate
    would make the optional cycle flaky on a host fast enough to walk the gap first."""
    delta = int(after) - int(before)
    if delta > 0:
        return (f"[{label}] gap-gated re-jump EXERCISED — {victim} re-jumped {delta} time(s) "
                "past the deep gap (design.md §4.3)")
    return (f"[{label}] NOTE: re-jump landing not observed (the victim rejoined via the "
            f"derive-walk before the gap crossed re_jump_threshold={threshold})")


def gauge_peak(current, sample):
    """`:162` — track the highest INTEGER gauge reading; anything else (`"na"`, a float, a
    mid-restart empty) leaves the peak alone. The gauge is transient, so the peak is the only
    stable thing to report; it is corroboration and never a gate (`evaluate_park_exercised`
    prints it and does not read it)."""
    tok = str(sample).strip()
    if not tok.isdigit():
        return current
    return max(int(current), int(tok))


# ══ smoke-vrf-dkg-restart-midwindow ═══════════════════════════════════════════════════

#: `case-vrf-dkg-restart-midwindow.sh` — the TUNED genesis. 64-block epochs for a generous DKG
#: window (room for the journal poll to land inside it) and activation at `2*interval` so the
#: migration anchor stays in absolute epoch 2. The third knob, `felonyThreshold=1`, is GONE with
#: the participation-floor jail it escalated: the case now asserts the resumed member PRODUCED
#: rather than that it escaped a jail.
MIDWINDOW_EPOCH_INTERVAL = 64
MIDWINDOW_ACTIVATION_BLOCK = 128

#: `:81` — the restarted member. `:84` — how far past the epoch-2 boundary the beacon window runs.
MIDWINDOW_VICTIM = "validator-3"
MIDWINDOW_VICTIM_IDX = 3
MIDWINDOW_EPOCH = 2
BOUNDARY_PROBE_OFFSET = 6

#: `:124` — the journal poll's budget, `:137` its cadence.
JOURNAL_POLL_S = 1
JOURNAL_DEADLINE_S = 400
#: The two truthy outcomes of the journal poll. bash distinguishes them by exiting from INSIDE the
#: loop with two different messages; here the probe returns which one it saw, so a window that
#: CLOSED is never reported as a journal that never appeared. They are strings and not booleans
#: precisely so the caller has to name the one it means.
WINDOW_OPEN = "in-window"
WINDOW_MISSED = "boundary-crossed"
#: `:146`, `:149` — the post-restart boundary crossing, then the all-nodes-have gate.
MIDWINDOW_BOUNDARY_S = 400
MIDWINDOW_NODES_HAVE_S = 180
#: `:166`, `:175` — the resume/share log poll.
RESUME_POLL_BUDGET_S = 120
RESUME_POLL_S = 3
#: `:220-224` — the participation retry: five attempts, four seconds apart.
PART_RETRIES = 5
PART_RETRY_SLEEP_S = 4
#: `:257` — the post-boundary liveness sample.
POST_BOUNDARY_SLEEP_S = 6
#: `:128`, `:148`, `:179` — fail-path log depths.
MIDWINDOW_LOG_TAIL = 160
MIDWINDOW_BOUNDARY_TAIL = 120

#: `:170-173` — the two log lines, and `:92-107` the two on-disk gates.
RESUME_LINE = "live DKG: ceremony resumed from journal"
SHARE_LINE = "live DKG: PK_epoch + share computed + stored"
JOURNAL_FILE = "beacon-dkgjournal-e{epoch}.bin"
SHARE_FILE = "beacon-share-e{epoch}.bin"
DATADIR_GLOB = "/runtime/reth-data/v{idx}"

#: The slash markers. Any of them naming the victim's address is a failure. The two liveness-jail
#: events went with the jail; `ValidatorSlashed` survives only as an EQUIVOCATION event, which is
#: exactly what a torn resume must not trigger.
SLASH_MARKERS = ("ValidatorSlashed", "equivocat")



def journal_probe(idx, epoch) -> str:
    """`:93-95` — the `sh -c` snippet that answers "is the epoch-N ceremony journal on disk?".

    `find` under the datadir tree rather than a hardcoded path, so a future reth datadir-layout
    change cannot silently break the gate into a permanent "absent" — which would make the case
    time out at the journal poll rather than fail at an assertion. `-size +0c` because an empty
    file is a journal that has not been written yet."""
    return (f"find {DATADIR_GLOB.format(idx=idx)} -type f -name "
            f"\"{JOURNAL_FILE.format(epoch=epoch)}\" -size +0c 2>/dev/null | grep -q .")


def share_probe(idx, epoch) -> str:
    """`:106-108` — the same shape for the SHARE file, which is persisted at FINALIZE.

    NO `-size +0c` here, matching bash: the question is existence, and a victim that has begun
    writing the share has finalized. The case wants this to be ABSENT — combined with a present
    journal that is the genuine PRE-FINALIZE mid-window, which is what makes `maybe_start` take
    the store-MISS path and actually run `resume_from_journal`. Restarting after the share exists
    hits the `store.contains_key(2)` early-return and never logs a resume: the original false-RED
    (review [164])."""
    return (f"find {DATADIR_GLOB.format(idx=idx)} -type f -name "
            f"\"{SHARE_FILE.format(epoch=epoch)}\" 2>/dev/null | grep -q .")


def epoch_field_lines(logs: str, message: str, epoch):
    """`:170-173` — `grep "<message>" | grep -E "epoch=<N>( |,|$)"` over an ANSI-STRIPPED log.

    TWO greps, not one, and the order is bash's: the message first, then the epoch FIELD. They
    are kept separate because tracing renders fields in an order the case does not control, so a
    single pattern spanning both would be asserting a render order.

    The field pattern is ANCHORED at its right edge (`( |,|$)`) so `epoch=2` does not also match
    `epoch=20`. The ANSI strip is what makes either grep possible at all: the node writes escapes
    INSIDE the pair, so a raw log line reads `epoch<ESC>=<ESC>2` and a literal `epoch=2` never
    fires — the false-RED that masks a genuine resume, called out at `:161-164`."""
    pat = re.compile(r"epoch=" + re.escape(str(epoch)) + r"( |,|$)")
    return [line for line in (logs or "").splitlines()
            if message in line and pat.search(line)]


def evaluate_resumed(lines, victim):
    """`:177-180` — the victim logged a POST-RESTART resume for epoch 2.

    Anchored on the RESUME line and not on "share computed": a fast host can finalize and emit
    "share computed" DURING the open window BEFORE the restart, so a whole-log grep for that line
    is green even against a broken resume. The resume line can only be emitted by the restarted
    process, which is what makes its presence proof that the recovery path ran."""
    if lines:
        return True, ""
    return False, (f"{victim} did NOT log a post-restart 'ceremony resumed from journal' for "
                   f"epoch {MIDWINDOW_EPOCH} — the journal+resume path never ran (this is the "
                   "bug the fix closes)")


def evaluate_share_computed(lines, victim):
    """`:181-184` — …and the resume CONVERGED. Started-but-never-finished is a distinct failure
    (a resolver fetch or the settle gate did not complete) and gets its own message."""
    if lines:
        return True, ""
    return False, (f"{victim} resumed from journal but did NOT converge to an epoch-"
                   f"{MIDWINDOW_EPOCH} share — resume started but did not finalize (resolver "
                   "fetch or settle gate did not complete)")


def evaluate_prev_randao_window(rows, victim):
    """`:191-197` — the victim's epoch-2 `prev_randao` is byte-identical to the survivors'.

    `rows` is `[(height, victim_mixhash, reference_mixhash)]`. This is what separates a real
    share-holder from a verify-only re-deriver: both produce a mixHash, and only a node that
    holds the share produces the SAME one over every block of the window. A missing block on the
    victim is a failure too, not a skip — it means the victim never derived that height."""
    miss = []
    for height, dh, sv in rows:
        if dh in (None, "", "null"):
            miss.append(f"{height}=missing-on-{victim}")
        elif dh != sv:
            miss.append(f"{height}: {victim}={dh} != validator-0={sv}")
    if not miss:
        return True, ""
    return False, f"{victim} epoch-{MIDWINDOW_EPOCH} prev_randao diverged: " + "; ".join(miss)


def credit_ready(produced, total) -> bool:
    """The retry's stop condition: a real read over an epoch that has recorded blocks.

    `blocksInEpoch > 0` is load-bearing: an epoch with nothing recorded credits everyone 0, so
    "produced something" would be false for a healthy node purely because the read was early.
    Waiting for recorded blocks is what makes the comparison evidence rather than timing."""
    if credit_state(produced) != STATE_OK:
        return False
    try:
        return int(total) > 0
    except (TypeError, ValueError):
        return False


def evaluate_production_readable(produced, total, victim):
    """The three separate failure branches, kept separate.

    A persistent -2 means the harness could not read the chain, a -1 means the committee is not
    what the case assumed, and `blocksInEpoch == 0` means it looked too early. They call for three
    different actions, and collapsing them into "could not evaluate production" would send an
    operator to re-run a case whose committee composition changed."""
    state = credit_state(produced)
    if state == STATE_FAILED:
        return False, (f"producedAt(epoch={MIDWINDOW_EPOCH}) read kept failing (-2 RPC sentinel) "
                       "— cannot prove the member produced (a -2 must never be treated as a "
                       "passing 0)")
    if state == STATE_ABSENT:
        return False, (f"{victim} is not in committee[{MIDWINDOW_EPOCH}] — cannot evaluate its "
                       "production (committee composition changed unexpectedly)")
    try:
        n_total = int(total)
    except (TypeError, ValueError):
        n_total = 0
    if n_total <= 0:
        return False, (f"no epoch-{MIDWINDOW_EPOCH} blocks recorded (blocksInEpoch={total}) — "
                       "cannot evaluate production (re-run / widen the window)")
    return True, ""


def evaluate_produced_something(produced, victim):
    """THE LOAD-BEARING ASSERTION, in the shape the deleted participation floor left behind.

    The negative control is what gives it teeth. WITHOUT the fix the victim is shareless for
    epoch 2 and cannot produce a valid boundary proposal at all, so its credit stays 0 and this
    fires. Asserting only "the chain stayed live" would pass on a configuration where the victim
    contributed nothing.

    Deliberately `> 0` and not a ratio: the retired check compared `seen/certs` against a
    governance floor, but production credit is drawn by a stake-weighted lottery, so a healthy
    member's exact share over ONE epoch is a random variable and any fixed threshold would be an
    invented number. "It produced at all" is the property a shareless node cannot fake."""
    try:
        n = int(produced)
    except (TypeError, ValueError):
        n = 0
    if n > 0:
        return True, ""
    return False, (f"{victim} produced NOTHING in epoch {MIDWINDOW_EPOCH} (producedAt={produced}) "
                   "— it did not recover its share (the fix failed)")


def slash_hits(logs: str, addr: str, markers=SLASH_MARKERS):
    """`:242` — `grep -iE "<slash markers>" | grep -i "<addr without 0x>"`.

    BOTH greps, in bash's order. The address filter is what makes this the VICTIM's slash rather
    than any slash: a case that dropped it would fail on a slash dispatched against some other
    node, and one that dropped the marker filter would match any line mentioning the address.
    The `0x` is stripped because the events render the address bare.

    `markers` is a parameter because `case-vrf-dkg-durability.sh:472` runs the SAME two-grep shape
    over a DIFFERENT alternation (`ValidatorSlashed|equivocat`) to ask whether a torn resume
    re-dealt. Parameterising is what keeps that from becoming a second copy: the copy would get the
    two `-i`s and the `0x` strip right on the day it was written and stay right only by luck."""
    want = (addr or "").lower()
    want = want[2:] if want.startswith("0x") else want
    if not want:
        return []
    markers = [m.lower() for m in markers]
    return [line for line in (logs or "").splitlines()
            if any(m in line.lower() for m in markers) and want in line.lower()]


def evaluate_no_slash(hits, victim):
    """No slash event named the victim. The other half of the production check: the counter says
    it produced, and this says nothing punished it for the restart. Only EQUIVOCATION can slash
    now, so a hit here is a much louder signal than it was under the liveness jail."""
    if not hits:
        return True, ""
    return False, (f"{victim} was slashed despite recovering its share: " + "; ".join(hits))


def evaluate_not_jailed(status, victim):
    """`:245-253` — the status byte, with the EMPTY read as a hard error.

    An empty-vs-"3" false-green would silently hide the very fault the case exists to catch
    (review [225]). So "" fails here, loudly, instead of falling into the `!= "3"` branch that
    would report success. `Jail` now has exactly ONE producer — equivocation, which always
    tombstones — so reaching it is a far stronger signal than it used to be."""
    st = (status or "").strip()
    if not st:
        return False, (f"could not read {victim} validator status (empty RPC result) — cannot "
                       "assert not-jailed (re-run)")
    if st == STATUS_JAIL:
        return False, (f"{victim} is JAILED (status={STATUS_JAIL}) — the only producer of that "
                       "status is equivocation, which is permanent and tombstoning")
    return True, ""


def evaluate_still_finalizing(before, after):
    """`:257-258` — the chain is still finalizing after the boundary. Two reads with a fixed
    sleep between them; strictly greater, so a frozen tip fails."""
    if int(after) > int(before):
        return True, ""
    return False, f"chain not finalizing after the boundary ({after} <= {before})"
