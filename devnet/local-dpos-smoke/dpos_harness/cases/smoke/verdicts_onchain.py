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

from ...core import converge, rpc

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

#: `case-liveness.sh:30-32` — the committee is exactly the stand's validators and
#: `validators[i] == validator-i`. FIVE since 2026-09-07, where the bash said four; see
#: `StaticProfile.committee()`. A shorter list is a genesis/bring-up problem, and mapping a
#: service to the wrong address would read the HUB's counters as the victim's, which passes
#: the `<` test never.
EXPECTED_VALIDATORS = 5

#: `case-liveness.sh:56` — the budget for the chain to advance the cycle's gap with one node down.
#: Cycle 1 waits `3*interval + 1` = 97 blocks; at 1 blk/s with the victim's leader views timing
#: out (1750 ms) until `skip_timeout` mutes them that is ~100-115 s of chain time.
LIVENESS_ADVANCE_S = 240
#: The retry budget on the two production-credit SNAPSHOTS, and the gap between attempts. This is
#: a retry, not a wait for a condition to become true: the readings themselves can come back on the
#: -2 getter-failed sentinel, and a sentinel must be retried rather than folded into the delta.
#: The measurement it feeds is bounded by the outage, so nothing here can widen the window.
PRODUCTION_READ_BUDGET_S = 60
PRODUCTION_READ_POLL_S = 2
#: `fluentbase_consensus::K`, the deferred-execution lag. Duplicated from `verdicts_fault` on the
#: same "no cross-case retargeting" rule the rest of this file's copies carry — a fault-side edit
#: must not silently move a liveness-side sampling floor.
RESULT_LAG_K = 3
#: How far past the stop the chain must finalize before the BASELINE snapshot is taken.
#: `recordProduction` is a system call of the block that EXECUTES height h, which under deferred
#: execution is height h+K — so the victim's last blocks before the stop are still being credited
#: for K heights afterwards. Sampling the baseline any earlier attributes those to the outage and
#: turns a correct run red. `+1` because the credit lands when h+K executes, so the state that
#: carries it is visible one height later.
PRODUCTION_SETTLE_BLOCKS = RESULT_LAG_K + 1
#: The settle wait's budget. K+1 blocks at 1 blk/s with one validator down; generous because a
#: timeout here is an infrastructure signal, not the property.
PRODUCTION_SETTLE_S = 90
#: The slack between the height the CYCLE baselined at (`pre`) and the height the stop was
#: observed at. One block: `compose_stop` returns and `finalized_dec` is read immediately, and at
#: 1 blk/s the chain moves at most about a block in between. It is charged against the window
#: below because it shortens it — the measurement starts at `stopped_at + K + 1`, not at
#: `pre + K + 1`.
PRODUCTION_STOP_SLACK_BLOCKS = 1


def production_window_blocks(gap, committee_size=None, k=RESULT_LAG_K,
                             slack=PRODUCTION_STOP_SLACK_BLOCKS):
    """How many blocks of production credit the outage of `gap` blocks can actually MEASURE.

    Derived, not chosen. Both snapshots read a counter that lags the chain by `k`, so what the
    delta spans is heights `[stopped_at + 1 .. (pre + gap) - k]`:

        window = gap - k - 1 - slack

    `- k` because `blocksInEpoch` at the closing snapshot reflects heights up to `finalized - k`;
    `- 1 - slack` because the baseline is deliberately taken `k + 1` past the stop (see
    `PRODUCTION_SETTLE_BLOCKS`) and the stop itself is observed up to `slack` blocks after `pre`.

    `committee_size` is unused here and accepted so callers can pass it uniformly; it is what
    `production_min_gap` needs."""
    return int(gap) - int(k) - 1 - int(slack)


def production_min_gap(committee_size, k=RESULT_LAG_K, slack=PRODUCTION_STOP_SLACK_BLOCKS):
    """The shortest outage over which the production-credit leg CARRIES INFORMATION.

    THE LIVE FAILURE THIS EXISTS FOR. `LIVENESS_CYCLES`'s third cycle has `gap = 5`. Against
    `k = 3` that leaves `production_window_blocks(5) = 0` — the counter cannot move, so the
    control (`blocksInEpoch` grew) is UNSATISFIABLE and the leg failed a healthy chain that had
    just been observed finalizing past its target. The original audit finding said exactly this
    about that cycle: five blocks make the production gate carry no information. The answer is
    not a looser control on that cycle — it is that the leg does not apply to it.

    THE FLOOR IS THE MEASUREMENT'S OWN ARITHMETIC PLUS ONE ROTATION.

      * `k + 1 + slack` is pure overhead: the deferred lag the closing snapshot cannot see, the
        `k + 1` settle the baseline is taken after, and the block the stop read costs. Below this
        the window is empty or negative and BOTH conjuncts are vacuous.
      * `+ committee_size` is what makes the surviving assertion mean something. `dv == 0` is
        exact for a stopped node, but its VALUE as a witness is that an equally-staked member
        which was UP would have been credited — and on `n` equal stakes the leader rotation
        credits a given member once per `n` blocks in expectation. A window shorter than one
        rotation reads 0 for an up member often enough that the assertion stops discriminating.
        Expectation, not a guarantee: this buys "an up victim is expected to be caught", which is
        the honest claim, and the long cycles (window 92 and 27 live) are far past it anyway.

    On this stand: `3 + 1 + 1 + 5 = 10`. Cycles 1, 2 and 4 (gaps 97, 33, 33) assert; cycle 3
    (gap 5) is SKIPPED — loudly, by name, with this arithmetic printed. Its subject is the
    within-epoch walk / re-jump path, which the rejoin and SIGNING legs still cover in full."""
    return int(k) + 1 + int(slack) + int(committee_size)


def evaluate_production_applicable(gap, committee_size, k=RESULT_LAG_K,
                                   slack=PRODUCTION_STOP_SLACK_BLOCKS):
    """`(applies, why)` — may the production-credit leg be asserted over an outage of `gap`?

    `why` is printed EITHER WAY. A skipped leg that looks like a passed leg is the defect class
    this whole reading was rewritten to remove, so the skip is named on stdout with the numbers
    that produced it, never inferred from a missing OK line."""
    floor = production_min_gap(committee_size, k=k, slack=slack)
    window = production_window_blocks(gap, k=k, slack=slack)
    if int(gap) >= floor:
        return True, (f"outage gap={gap} >= {floor} (= K{k} + 1 settle + {slack} stop-slack + "
                      f"{committee_size} committee) — the credit delta spans ~{window} blocks")
    return False, (f"outage gap={gap} < {floor} (= K{k} + 1 settle + {slack} stop-slack + "
                   f"{committee_size} committee): the credit delta would span ~{window} blocks, "
                   "which is too short to carry information. `blocksInEpoch` reads a counter that "
                   f"lags the chain by K={k} and the baseline is taken K+1 past the stop, so a "
                   "short outage leaves NO credited heights inside the window — the control is "
                   "unsatisfiable and the victim's zero would be vacuous. This cycle's subject is "
                   "the within-epoch walk / re-jump path, which the rejoin and SIGNING legs below "
                   "cover in full")
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
    """`:31` — one address per stand validator, or the case cannot map a victim to its
    on-chain identity. FIVE since 2026-09-07, where the bash said four — see
    `StaticProfile.committee()` for why the stand grew.

    Fail-loud rather than "use what we got": a short list would leave `ADDR[3]` unset, and under
    `set -u` bash aborts. Here it would index-error three assertions later, in the participation
    read, where it would look like a getter problem."""
    n = len(addrs or [])
    if n == EXPECTED_VALIDATORS:
        return True, ""
    return False, (f"expected {EXPECTED_VALIDATORS} validator addresses, got {n}: "
                   f"{' '.join(addrs or [])}")


def credit_readable(snapshot) -> bool:
    """Is every `(produced, blocksInEpoch)` reading in a `{epoch: pair}` snapshot a REAL counter?

    A -1 and a -2 must never enter arithmetic: subtracting sentinels produces a number that looks
    like a delta and means nothing, and `-2 - -2 == 0` would satisfy the zero-growth verdict below
    on a chain nobody could read."""
    return all(credit_state(p) == STATE_OK and credit_state(t) == STATE_OK
               for p, t in (snapshot or {}).values())


def credit_delta(before: dict, after: dict):
    """`(Δproduced, ΔblocksInEpoch)` summed over every epoch the outage spanned, or `None` if any
    reading in either snapshot is a sentinel.

    Both snapshots are `{epoch: (produced, blocksInEpoch)}`. `after` covers `[E_a .. E_b]` — the
    epoch the outage started in through the one it ended in — and `before` covers only `E_a`, so
    an epoch missing from `before` is one that BEGAN inside the window and whose whole count is
    delta. That is what lets the measurement survive an epoch rollover mid-outage, which the poll
    this replaced could not: it re-read `currentEpoch()` every iteration and compared two counters
    of whatever epoch was current at that instant, so a rollover handed it a fresh epoch where the
    victim's 0 and the hub's 1 satisfied it outright."""
    if not credit_readable(before) or not credit_readable(after):
        return None
    dv = dt = 0
    for epoch, (produced, total) in sorted((after or {}).items()):
        p0, t0 = (before or {}).get(epoch, (0, 0))
        dv += int(produced) - int(p0)
        dt += int(total) - int(t0)
    return dv, dt


def evaluate_production_over_outage(delta, svc, epochs, hub_delta=None):
    """THE PRODUCTION-CREDIT VERDICT, bounded by the OUTAGE: while `svc` was stopped it received
    ZERO production credit, over a window in which credit was demonstrably still being written.

    WHAT THIS REPLACED, AND WHY IT CARRIED NO INFORMATION. The old reading polled
    `producedAt(currentEpoch(), victim) < producedAt(currentEpoch(), hub)` every 2 s for up to 90 s
    and passed on the first instant the inequality held. Stakes are equal by default, so the two
    counters trade the lead continuously on ordinary lottery jitter; over 45 samples the hub leads
    at SOME instant whether or not the victim was ever stopped. And the poll re-read the epoch, so
    a rollover produced a `0 < 1` that is true of a perfectly healthy validator. Nothing in it was
    tied to the outage, which is the one thing the case exists to measure.

    The replacement is a DELTA over the down window and it is exact rather than statistical:

      * `dv == 0` is physically necessary — a stopped process produces no blocks, so any credit
        at all is the failure this leg is named for (a `recordProduction` crediting the wrong
        member, or a stale leader index). It is NOT a threshold that can be widened; there is
        nothing between 0 and 1 to tune.
      * `dt > 0` is the CONTROL that makes the zero mean something. `blocksInEpoch` counts every
        recorded block of the epoch whoever produced it, so a flat `dt` says no production record
        reached the chain during the window — a stalled chain, a dead system call, or a getter
        frozen at a stale value. A zero read against a frozen counter is exactly the vacuous pass
        an absence assertion must refuse.

    The hub's own delta rides the MESSAGE and is deliberately not a gate: `LIVENESS_CYCLES`'s
    third cycle is a 5-block outage, and on a 4-member equal-stake lottery the hub draws none of 5
    slots about a quarter of the time. Gating on it would trade a witness that cannot see the
    property for one that fails a quarter of the runs on correct behaviour."""
    hub_note = "" if hub_delta is None else f", hub gained {hub_delta}"
    if delta is None:
        return False, (f"production credit for {svc} over epochs {epochs} could not be read — a "
                       "sentinel (-1 not-in-committee / -2 getter failed) reached the delta. A "
                       "sentinel is never a real 0, and this leg's PASS is a zero")
    dv, dt = delta
    if int(dt) <= 0:
        return False, (f"no production record reached the chain while {svc} was down: "
                       f"blocksInEpoch over epochs {epochs} grew by {dt}{hub_note}. The victim's "
                       f"produced-delta of {dv} says nothing against a counter that did not move "
                       "— the chain stalled, the `recordProduction` system call is not landing, "
                       "or the getter is frozen")
    if int(dv) != 0:
        return False, (f"{svc} was credited {dv} block(s) over epochs {epochs} WHILE IT WAS "
                       f"STOPPED (blocksInEpoch grew by {dt}{hub_note}) — a stopped process "
                       "produces nothing, so `recordProduction` credited the wrong member or the "
                       "leader index in `extra_data` is stale")
    return True, ""


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
    """`:54-58` — the honest 4-of-5 quorum KEPT finalizing over the blocks IMMEDIATELY after the
    equivocator was tombstoned.

    This covers the immediate window only: the tombstone arms the proposal refusal and the
    transport severance while the committee is unchanged, and a quorum that could not carry the
    chain through that would stall here within a few blocks. The committee-floor boundary is a
    SEPARATE assertion (`evaluate_committed_after_jail` and friends below) — until 2026-09-04 this
    docstring claimed that boundary was unreachable on this stand, and the case was green on a
    build where the whole network died 35 s after the jail (`.dpos-study/EXPERIMENTS.md` §5.3
    E1). It was reachable; the case simply stopped looking three blocks after the jail."""
    if advanced:
        return True, ""
    return False, f"chain stalled after jail (finalized stuck at ~{post_jail})"


# ── the committee boundary after the jail ─────────────────────────────────────────────
#
# THE BOUNDARY THIS STAND REACHES, AND WHAT HAPPENS THERE. The tombstone makes the offender
# selection-invisible from the epoch after the jail (`set_selection_visible(v, false, E)`,
# contract `consensus.rs::apply_equivocation_penalty`) and removes it from the Active set.
# Committees are committed two epochs ahead (`drive_ahead_commit`, node/src/evm.rs, loop exits
# at `next > current_epoch + 2`), so the first block of epoch `E+1` commits `committee[E+3]`
# from a selection taken at `E+1` — and on this stand (`--peers=5`, `MIN_COMMITTEE_LENGTH = 4`)
# that selection has FOUR members: the offender is gone and the remaining four are exactly the
# floor.
#
#   The stand is five-seat FOR THIS REASON. On the four-seat stand the bash ran, the same jail
#   left THREE, and what happened there depended on the contract:
#     * `bc42042a` and earlier reverted `CommitteeTooSmall(3, 4)` into the node's fail-loud arm
#       and every honest node shut down (R-112, observed live 35 s after the tombstone);
#     * Э0.2 carried `committee[E+2]` into `[E+3]` byte-for-byte instead;
#     * since 2026-09-07 the carry is deleted and the revert is back on every epoch.
#   None of those is what this case is for. It is for "the equivocator is caught, punished, and
#   the network carries on", so the stand is sized to leave a legal committee behind the jail.
#   The below-the-floor path is a DIFFERENT case (`scripts/xp/floor_halt_case.py`).
#
# The case waits PAST that boundary and asserts the RE-SEAT: the committee for `E+3` is committed,
# it does not contain the offender, it is not the previous committee, and `dkgQual[E+3]` is set
# because the membership changed. The witness of the mechanism is the commit line with
# `members=4`: a run where the jail never reached the selection would pass every liveness gate
# for free, and this is what refuses that.

#: `node/src/evm.rs::emit_commit_observability` — the ONLY observable of a commit: the commit is
#: a pre-execution system call, so its `EpochCommitteeCommitted` event never reaches a receipt or
#: `eth_getLogs`. Carries `epoch=<target>` and `members=<seated>`.
COMMITTED_LINE = "epoch_committee_committed"
#: The seats the jail leaves on the five-seat genesis — exactly MIN_COMMITTEE_LENGTH.
BYZANTINE_SEATS_AFTER_JAIL = 4
#: The seats before it.
BYZANTINE_COMMITTEE_SEATS = 5
#: How many epochs past the jail's epoch the observation runs: the boundary commit is on the first
#: block of `E+1` (up to one interval after the jail), and one further epoch is what shows the
#: re-seated committee actually finalizes rather than merely being written. THREE intervals of
#: chain time worst-case; the budget adds boot slack.
CARRY_OBSERVE_EPOCHS = 2
CARRY_WAIT_SLACK_S = 120
#: Poll cadence for the boundary wait, and which lines say a node took the fatal branch.
CARRY_POLL_S = 3
FATAL_LINES = ("executor fatal error", "did not succeed", "OuterEngine exited cleanly")

_COMMITTED_FIELDS_RE = re.compile(r"\bepoch=(\d+)\b.*?\bmembers=(\d+)\b")


def sever_epoch(line: str):
    """The `epoch=<N>` the tombstone watch logged on its severance line (`node/src/dpos.rs`:
    the epoch of the finalized block it read the tombstone at). None when the field is missing —
    the boundary arithmetic cannot start from a guessed epoch."""
    m = re.search(r"\bepoch=(\d+)\b", line or "")
    return int(m.group(1)) if m else None


def carry_wait_target(jail_epoch, interval, activation, epochs=CARRY_OBSERVE_EPOCHS):
    """The finalized height the observation runs to: `epochs` full epochs past the jail's
    epoch. The severance line's epoch is where the tombstone was READ (EL-finalized), which is the
    slash's own epoch or, across a boundary, the one after it; two epochs past it covers the
    boundary block on either reading and one epoch of the re-seated committee finalizing."""
    return epoch_start(activation, interval, int(jail_epoch) + int(epochs)) + int(interval)


def carry_wait_budget_s(interval, slack=CARRY_WAIT_SLACK_S, epochs=CARRY_OBSERVE_EPOCHS):
    return (int(epochs) + 1) * int(interval) + int(slack)


def epoch_start(activation, interval, epoch):
    """First block of relative `epoch` — spelled here rather than imported from `verdicts` on the
    same no-cross-case-retargeting rule the rest of this file's copies carry."""
    return int(activation) + int(epoch) * int(interval)


def committed_lines(log_text: str):
    """`(epoch, members)` for every committee-commit line in an ANSI-stripped log, in order.
    Lines whose fields do not parse are dropped: a commit the node could not describe is not
    evidence of anything."""
    out = []
    for ln in (log_text or "").splitlines():
        if COMMITTED_LINE not in ln:
            continue
        m = _COMMITTED_FIELDS_RE.search(ln)
        if m:
            out.append((int(m.group(1)), int(m.group(2))))
    return out


def evaluate_honest_alive(states: dict, fatal: dict):
    """Every honest node's container is still `running` and none of them wrote a fatal line since
    the pre-jail snapshot. `states` = `{service: docker state}`, `fatal` = `{service: [lines]}`.

    THIS IS THE LINE A MIS-SIZED STAND FAILS ON. Let the genesis be four-seat and the jail leaves
    three, `commitEpochCommittee` reverts `CommitteeTooSmall(3, 4)` into the node's fail-loud arm,
    and the honest nodes exit within milliseconds of each other — `exited` here, and the
    `did not succeed` / `executor fatal error` / `OuterEngine exited cleanly` trio in their logs.

    A `None` state is UNREAD — `docker compose ps` did not run at all (`SmokeCtx.ps_state`) — and
    gets its own verdict. Folding it into `dead` would report a daemon hiccup as the R-112 branch,
    which is the loudest wrong answer this case can give."""
    unread = sorted(s for s, st in states.items() if st is None)
    if unread:
        return False, (f"the container state of {', '.join(unread)} could not be read (`docker "
                       "compose ps` did not run) — refusing to score the committee-floor boundary "
                       "over nodes whose liveness nobody observed")
    dead = {s: st for s, st in sorted(states.items()) if not str(st).startswith("running")}
    hot = {s: ls for s, ls in sorted(fatal.items()) if ls}
    if not dead and not hot:
        return True, ""
    detail = []
    for s, st in dead.items():
        detail.append(f"{s} is {st!r}")
    for s, ls in hot.items():
        detail.append(f"{s} logged {len(ls)} fatal line(s), last: {ls[-1].strip()[:240]!r}")
    return False, ("the committee boundary after the jail KILLED honest nodes — the R-112 branch "
                   "(`commitEpochCommittee` reverted into the fail-loud arm, which means the "
                   "selection fell below MIN_COMMITTEE_LENGTH): " + "; ".join(detail))


def evaluate_committed_after_jail(before: dict, after: dict,
                                  members=BYZANTINE_SEATS_AFTER_JAIL):
    """Every honest node logged at least one committee COMMIT since the pre-jail snapshot, seating
    exactly `members`. `before`/`after` = `{service: [(epoch, members)]}`. Returns
    `(ok, msg, target_epoch)`.

    The anti-vacuity gate. Liveness past the boundary is also what a chain whose jail never
    reached the selection produces (a slash that did not land, a tombstone that did not stamp
    invisibility), and every other assertion in this case is satisfied by that chain: the
    committee would simply keep its five seats and nothing would notice. Only a commit that seats
    `members` says the offender was actually dropped. `members` is pinned to what the jail leaves
    on the five-seat genesis; a different number is a different experiment.

    The agreed epoch is the SHARED one, not each node's FIRST. The baseline is read one node at a
    time (`_floor_snapshot` issues a `docker compose logs` per service), so a commit landing
    between two of those reads is inside one node's baseline and outside another's — their fresh
    slices then start one epoch apart, and comparing first elements called that "they did not
    commit one boundary" on a healthy chain. What the case actually asserts is that every honest
    node witnessed the SAME commit, which is an intersection; an empty one is still the divergence
    the message describes."""
    if not after:
        return False, "no honest node was scanned for the committee-commit line", None
    missing, seen, shared = [], {}, None
    for svc, lines in sorted(after.items()):
        fresh = lines[len(before.get(svc, [])):]
        good = {t[0] for t in fresh if t[1] == int(members)}
        seen[svc] = sorted(good)
        if not good:
            missing.append(f"{svc}: {len(fresh)} fresh commit line(s), none seating "
                           f"members={members}"
                           + (f" (saw {fresh[-1]})" if fresh else ""))
        else:
            shared = good if shared is None else (shared & good)
    if missing:
        return False, (f"no `{COMMITTED_LINE}` seating {members} on: "
                       + "; ".join(missing)
                       + " — the chain crossed the boundary without the committee ever "
                         "shrinking, so the tombstone never reached the selection and "
                         "nothing here exercised the re-seat"), None
    if not shared:
        return False, (f"honest nodes share NO committed epoch ({seen}) — they did not "
                       "commit one boundary"), None
    return True, "", min(shared)


def evaluate_reseated_committee(cur_out: str, prev_out: str, qual_out: str, offender, target):
    """`committee[target]` drops `offender`, differs from `committee[target-1]`, seats exactly
    `BYZANTINE_SEATS_AFTER_JAIL`, and `dkgQual[target] == true` — the contract's own statement
    that the jail reached the selection (`consensus.rs::selected_committee_at` skips a validator
    that is not Active or not selection-visible, and `commitEpochCommittee` sets the DKG bit
    because the derived set differs from the previous one).

    BOTH committees are decoded STRICTLY (`rpc.cast_addr_array`), and an unread side is its own
    verdict — the same rule `evaluate_committee_change` states: a `""` from an unreachable node
    compares unequal to a real committee, so a raw-string compare turns an RPC brownout into
    "the boundary commit kept the offender", an accusation about a side nobody read. `dkgQual`
    gets the same treatment: an empty answer is UNREAD, not `false`."""
    sides = []
    for raw, e in ((prev_out, int(target) - 1), (cur_out, int(target))):
        what = f"getEpochCommittee({e})"
        if not (raw or "").strip():
            return False, (f"{what} returned NOTHING (unreachable node / RPC brownout) — refusing "
                           "to judge the re-seat over a committee nobody read")
        try:
            sides.append(rpc.cast_addr_array(raw, what))
        except rpc.CastDecodeError as exc:
            return False, (f"{what} did not decode as an address array ({exc}) — refusing to "
                           "judge the re-seat over a committee nobody read")
    prev, cur = sides
    if not cur:
        return False, (f"getEpochCommittee({target}) is EMPTY — the boundary epoch was never "
                       "committed")
    victim = (offender or "").strip().lower()
    if victim and victim in cur:
        return False, (f"committee[{target}] still seats the tombstoned {victim} — the jail did "
                       f"not reach the selection:\n  {cur}")
    if len(cur) != BYZANTINE_SEATS_AFTER_JAIL:
        return False, (f"committee[{target}] seats {len(cur)}, expected "
                       f"{BYZANTINE_SEATS_AFTER_JAIL} (the five-seat genesis less the "
                       f"equivocator):\n  {cur}")
    if cur == prev:
        return False, (f"committee[{target}] == committee[{int(target) - 1}] — the boundary "
                       f"commit re-seated the SAME set, so nothing was dropped:\n  {cur}")
    qual = (qual_out or "").strip()
    if not qual:
        return False, (f"getDkgQual({target}) returned NOTHING (unreachable node / RPC brownout) — "
                       "refusing to read an unanswered call as a clear DKG bit")
    if qual != "true":
        return False, (f"getDkgQual({target}) = {qual!r}, expected true: the committee changed, so "
                       "the epoch must mint a ceremony rather than carry the old key forward")
    return True, ""


#: `node/src/dpos.rs:1758` — the tombstone watch's severance, logged once per newly-tombstoned
#: peer by every honest node that reads the flag. THE MECHANISM (§7): the batcher's inactivity
#: rule refreshes `latest_seen` on any accepted message regardless of role, so a slashed member
#: that keeps voting stays "active" forever and its leader slots keep costing a full certification
#: deadline. Only cutting the transport lets `is_active` go false after `skip` views.
TOMBSTONE_SEVER_LINE = "validator tombstoned for equivocation — severing its transport"
#: `:1751` — the same watch's SELF arm. A node never blocks itself, so this is what the offender
#: writes instead. Read only as a diagnostic: it lives in the equivocator's log, not the hub's.
TOMBSTONE_SELF_LINE = "this validator is tombstoned for equivocation on chain"
#: The severance budget. It rides reth's finalized-block watch (no timer since 2026-08-24), so it
#: fires on the first finalized change after the tombstone is on chain — seconds, not epochs.
TOMBSTONE_SEVER_S = 60
TOMBSTONE_SEVER_POLL_S = 3


def tombstone_sever_lines(log_text, marker=TOMBSTONE_SEVER_LINE):
    """Lines where this node severed a tombstoned peer's transport."""
    return [ln for ln in (log_text or "").splitlines() if marker in ln]


def evaluate_tombstone_severed(lines, observer):
    """The POSITIVE witness that the jail had a CONSENSUS consequence, not just a contract one.

    `getValidatorStatus == Jail` is a contract reading; it says the slash landed, and nothing
    about whether any node acted on it. The three blocks of post-jail liveness do not say it
    either — the honest quorum was already 4-of-5 before the jail and would advance identically
    if every node ignored the tombstone completely. Between them the case named the severance in
    its OK line and read neither half of it.

    This is the half that is observable IMMEDIATELY and on this topology: the watch is driven from
    CHAIN STATE off the finalized-block watch, so it fires on the first finalized change after the
    tombstone is committed, with no committee change involved."""
    if lines:
        return True, ""
    return False, (f"{observer} never logged {TOMBSTONE_SEVER_LINE!r} — the equivocator is JAILED "
                   "on chain and no honest node acted on it. The tombstone watch reads the "
                   "`tombstoned` leg of the committee snapshot off the finalized-block watch "
                   "(node/dpos.rs:1739); if that read is failing it logs "
                   "'beacon plane: tombstone read failed' at debug instead. Without the "
                   "severance the offender's `latest_seen` keeps being refreshed by its own "
                   "votes, `is_active` never goes false, and every one of its leader slots "
                   "costs the full certification deadline")


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
#: ═══ THE "128 DOES NOT BOOT" PROHIBITION THAT USED TO LIVE HERE IS WITHDRAWN ═══════════════
#:
#: It said, in capitals, that 128 was a STRUCTURAL REJECT and that "no budget anywhere makes it
#: pass". Both halves were wrong, and the error was expensive: the prohibition is what forced
#: `CATCHUP_GAP` down to 28 with four blocks of headroom (see its comment) instead of buying the
#: headroom from the interval, and it stands recorded in `WEIGHTED_EPOCH_INTERVAL` and in two
#: `asserts_onchain` messages as a product bug that was never one.
#:
#: WHAT WAS ACTUALLY OBSERVED on 2026-07-31, at interval 128 / activation 256, is real and is
#: kept verbatim:
#:
#:     waiting for the sequencer to finalize >= dposActivationBlock=256 (relative epoch 0)
#:       sequencer finalized 256 >= activation 256; proceeding to swap
#:     FAIL: DPoS chain did not converge past anchor 0x100
#:
#: The chain produced ZERO blocks past the anchor; validator-0 logged `dpos: proposing order
#: block height=257` and every view from 4 to 115 answered `proposal failed verification`,
#: including on the proposer's OWN node.
#:
#: WHAT IT WAS: a CLOCK-DRIFT TIMEOUT, and the drift is the harness's own doing. The static
#: sequencer used to pace at 250 ms while a block's timestamp is `max(parent + 1, now)`
#: (crates/node/src/payload.rs:31-34) off a genesis timestamp of 0 — so chain time gained 0.75 s
#: on the wall clock per block and stood ~0.75 × (activation − 1) seconds in the future at the
#: swap. Post-swap the proposer stamps `max(now, parent + 1)` (application.rs:741-746) and
#: EVERY node, its own included, rejects a block more than
#: `TIMESTAMP_FUTURE_TOLERANCE_SECS = 1` ahead of its clock (application.rs:104, enforced :631).
#: So the chain cannot produce a block until real time catches the drift up, and the numbers are
#: exactly the observed behaviour: ~47 s at activation 64, ~95 s at 128, ~191 s at 256, against
#: a `DPOS_CONVERGE_S` of 120. That is also why intervals 32 and 64 came up on the same build in
#: the same run — 47 s and 95 s fit under 120, 191 s does not.
#:
#: THE INTERVAL WAS NEVER THE VARIABLE — the ACTIVATION HEIGHT was, and the interval only sets
#: it (`2 * interval`). A probe on 2026-08-20, STILL ON THE 250 ms SEQUENCER, one pinned image:
#:
#:     interval 128 / activation 256, converge 120  ->  did not come up
#:     interval  64 / activation 256, converge 120  ->  did not come up   (interval exonerated)
#:     interval 128 / activation 128, converge 120  ->  up in 137 s        (height exonerates it)
#:     interval 128 / activation 256, converge 400  ->  up in 263 s        (it was a timeout)
#:
#: WHAT FIXED IT: the sequencer now paces at 1 blk/s (`docker-compose.yml`, matching
#: `docker-compose.production-path.yml:133` and the sim/soak pair), so chain time tracks the wall
#: clock and the drift is ZERO at every activation height. `_wait_activation` was made
#: chain-paced in the same change, because a 1 s cadence turns the old 180 s wall-clock
#: activation budget into a ~180-block ceiling — the same trap one interval further out.
#:
#: CONFIRMED LIVE on the fixed stand, 2026-08-20, one pinned image, `DPOS_CONVERGE_S` left at
#: its unchanged 120 — the budget the old note said "no budget anywhere" could rescue:
#:
#:     interval  32 / activation  64  ->  up in 343 s, +29 blocks in the 30 s past the anchor
#:     interval 128 / activation 256  ->  up in 287 s, +30 blocks in the 30 s past the anchor
#:
#: Both reported a tip age of 0 s — chain time level with the wall clock, where the 250 ms
#: sequencer would have left it 47 s and 191 s in the future. (The 343 s at the SMALLER interval
#: is not a contradiction: that arm paid for the one-time image build.)
#:
#: SO 128 IS AVAILABLE NOW, and this constant stays at 64 only because nothing has re-tuned the
#: case for it: `CATCHUP_GAP` is measured against `catchup_gap_ceiling(64, 3000) = 32` and every
#: budget below is sized off `2 * interval = 128` (see `CATCHUP_DKG_WAIT_S`). Raising the
#: interval means re-deriving the gap and re-running the case live — not editing a literal.
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
#: `catchup_gap_ceiling(64, 3000)` = 32 leaves room to come down. RAISING THE INTERVAL IS ALSO AN
#: OPTION AGAIN as of 2026-08-20 — the "128 does not boot" finding it was closed against was a
#: sequencer clock-drift timeout in the STAND, now fixed (`CATCHUP_EPOCH_INTERVAL`) — but it is a
#: bigger change than it looks: the gap must be re-derived against `catchup_gap_ceiling(128,
#: 3000)` and every budget sized off `2 * interval` moved with it, and the result is only worth
#: anything if the case is re-run live. Nothing here may move on the argument alone.
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
#: latency the fetch always wins on a zero-RTT LAN with small bodies, so the park fires at NO value
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
    ms it would be 104 — reachable now that the stand's interval ceiling is gone (see
    `CATCHUP_EPOCH_INTERVAL`), though this case has not been re-tuned for it. The literal 52
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

#: THE RESTART'S HEADROOM before the epoch-E boundary, in finalized blocks (= seconds at 1 blk/s).
#:
#: WHY IT EXISTS. `compose_restart` is a graceful SIGTERM (reth's 40 s ceiling) followed by a full
#: node boot, and the chain keeps producing throughout. If the boundary lands inside that interval,
#: the restarted node reaches its epoch-E share by the DEMOTE-HEAL — `recompute_scoped` over the
#: retained dealer logs — instead of by `resume_from_journal`, and `RESUME_LINE` is never written.
#: That is correct product behaviour, and `evaluate_resumed` used to report it as "the
#: journal+resume path never ran", i.e. as the bug the case exists to catch. Exactly the shape
#: `verdicts_fault.SHARE_ROADS` records on the live-heal case.
#:
#: It is a RAIL and not a second accepted road, and that distinction is the whole point of this
#: case: the module docstring's subsumption argument is that `smoke-vrf-dkg-durability` phase 1
#: already covers "a restarted member ends up with a consistent share" WITHOUT emitting a resume
#: line, and that the journal→resume path is what is otherwise untested. Accepting the heal road
#: here would make this case green on runs where its own subject did not execute — which is the
#: durability case with more steps.
#:
#: SIZED, not chosen. The gate's share-ABSENT half already stops holding at
#: `epoch_start(E) − DKG_MARGIN_BLOCKS − K` (= 23 blocks out) because that is where the ceremony
#: finalizes and writes the share, so anything at or below 23 would be a no-op. 30 buys a restart
#: budget past that edge while leaving the detection band wide: the journal is written when the
#: actor's clock first enters epoch E−1, i.e. around `epoch_start(E) − interval − K` = 67 blocks
#: out on this stand's tuned 64-block interval, so the band is [67 .. 30].
MIDWINDOW_RESTART_MARGIN = 30
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


def evaluate_restart_in_window(fin_after_restart, epoch_start, victim):
    """THE CASE MUST VERIFY IT SET UP WHAT IT CLAIMS TO TEST — read AFTER the restart, because
    that is the only place the question can be answered.

    The pre-restart rail (`MIDWINDOW_RESTART_MARGIN`) makes this unlikely; this makes the
    diagnosis right when it happens anyway. If the epoch boundary passed WHILE the victim was
    down, its ceremony is swept and it reaches the same share by the demote-heal — no
    `RESUME_LINE` at all. Without this reading, `evaluate_resumed` fires next and calls that
    correct behaviour "the journal+resume path never ran", which is the bug this case is named
    for: a FALSE RED on the product, from a setup that did not hold.

    A missed window is a RE-RUN, not a product verdict, and the message says so."""
    if int(fin_after_restart) < int(epoch_start):
        return True, ""
    return False, (f"the epoch-{MIDWINDOW_EPOCH} boundary ({epoch_start}) passed WHILE {victim} "
                   f"was restarting (finalized={fin_after_restart}) — its ceremony is swept, so "
                   "it will reach its share by the demote-heal and never write "
                   f"{RESUME_LINE!r}. The journal→resume path this case exists for did not run: "
                   "this is a MISSED SETUP, not a product failure. Re-run; if it recurs the host "
                   f"is restarting slower than {MIDWINDOW_RESTART_MARGIN} blocks and the margin "
                   "(or EPOCH_BLOCK_INTERVAL) needs raising")


def evaluate_resumed(lines, victim):
    """`:177-180` — the victim logged a POST-RESTART resume for epoch 2.

    Anchored on the RESUME line and not on "share computed": a fast host can finalize and emit
    "share computed" DURING the open window BEFORE the restart, so a whole-log grep for that line
    is green even against a broken resume. The resume line can only be emitted by the restarted
    process, which is what makes its presence proof that the recovery path ran.

    IT IS ONLY A PRODUCT VERDICT ONCE THE SETUP IS PROVEN. There is a second, legitimate road to
    the same share — the demote-heal — and a victim whose boundary passed during the restart takes
    it and writes no resume line at all. `evaluate_restart_in_window` runs FIRST and rules that
    road out; reaching here with the setup proven means the resume genuinely did not happen."""
    if lines:
        return True, ""
    return False, (f"{victim} did NOT log a post-restart 'ceremony resumed from journal' for "
                   f"epoch {MIDWINDOW_EPOCH} — the journal+resume path never ran (this is the "
                   "bug the fix closes). The other road to the same share, the demote-heal, is "
                   "already ruled out: the boundary had not been crossed when the victim came "
                   "back, so its ceremony was still live and `maybe_start` had a journal to "
                   "resume from")


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


# ══ smoke-weighted-vrf ════════════════════════════════════════════════════════════════

#: `case-weighted-vrf.sh:15` — validator-0's genesis stake multiple. `genesis-bootstrap` skews
#: only the FIRST validator (`bootstrap.rs:426-430`), so the committee is 9:1:1:1 and the elector
#: should hand validator-0 ~75% of the views against ~8.3% each.
HEAVY_STAKE_MULT = 9

#: The epoch length this case runs on, and it is a SAMPLE SIZE, not a preference. The property
#: below requires every LIGHT validator to produce at least one block, and one epoch is the whole
#: sample: at p=1/12 per view, 32 views leave a 6% chance that a given light produces nothing and
#: ~17% that at least one of the three does — a coin-flip gate. 64 views bring those to 0.37% and
#: ~1.1%.
#:
#: 128 WOULD BE BETTER AND WAS BELIEVED UNAVAILABLE: the "it does not boot" finding this used to
#: cite is WITHDRAWN — it was a clock-drift timeout in the stand's own sequencer pacing, fixed on
#: 2026-08-20, and `CATCHUP_EPOCH_INTERVAL` carries the whole record. So 128 is reachable, and
#: this constant stays at 64 only because the case has not been re-run on it: `WEIGHTED_EPOCHS`
#: already buys the same sample in epochs, at a runtime cost 128 would roughly halve. A live
#: re-run is what would settle it, not this comment.
WEIGHTED_EPOCH_INTERVAL = 64

#: How many consecutive epochs the measurement spans. TWO, because one is not enough sample and
#: the interval cannot be raised: 64 views leave ~1.1% chance that some light validator produces
#: nothing by luck, and 128 brings that to ~5e-5.
#:
#: The cheaper fix — relaxing "every light produced" to "at least two of three" — was rejected. It
#: still kills a monopoly elector, but it stops catching a SYSTEMATICALLY EXCLUDED validator, a
#: weighting bug where one specific light is never elected. That is a real defect, and a minute of
#: runtime is not worth trading it away.
WEIGHTED_EPOCHS = 2

#: How long the chain gets to finish the measured epochs. A CEILING, not a measurement interval:
#: the worst honest wait is `WEIGHTED_EPOCHS` full epochs (128 blocks at ~1 blk/s) plus result-lag
#: settle, and this is ~3.7× that. Being generous costs a healthy run nothing —
#: `wait_finalized_ge` returns as soon as the boundary lands — while a tight budget would turn a
#: loaded docker daemon into a red weighting verdict.
WEIGHTED_WINDOW_S = 480

#: `case-weighted-vrf.sh:57` — `heavy*2 >= light_max*3`, i.e. a >=1.5x plurality, integer-exact as
#: the original wrote it. At 9:1:1:1 the expectation is ~9x, so 1.5x is far enough below the mean
#: that binomial variance over the window cannot flip it.
WEIGHTED_MARGIN_NUM, WEIGHTED_MARGIN_DEN = 3, 2


def evaluate_stake_skew(stakes, mult):
    """THE PRECONDITION: the genesis skew actually reached the chain.

    The bash original never checked it, and that is why a green run there was unattributed and a
    red one was ambiguous. `HEAVY_STAKE_MULT` travels through `docker-compose.yml` into
    `genesis-init`'s environment; an export that did not land produces an EQUAL-STAKE chain, on
    which the elector is uniform, validator-0 does not lead a plurality, and the case reports
    "weighting is not effective" — a true statement about a chain that was never skewed."""
    if len(stakes) < 2 or any(int(s) <= 0 for s in stakes):
        return False, (f"could not read the on-chain validator stakes ({stakes}) — 0 is the "
                       "read-failed sentinel here, not a real stake")
    heavy, light = int(stakes[0]), int(stakes[1])
    if any(int(s) != light for s in stakes[1:]):
        return False, (f"the light validators are not equally staked ({stakes}) — this case "
                       "assumes 1:1 among them, which is what genesis writes")
    if heavy != light * int(mult):
        return False, (f"HEAVY_STAKE_MULT={mult} did not reach genesis: validator-0's stake "
                       f"{heavy} is {heavy / light:.2f}x the light stake {light}, not {mult}x")
    return True, ""


def evaluate_weighted_election(epochs, counts_by_epoch, mult):
    """Stake-weighted leader election, measured on the ON-CHAIN production counters and SUMMED
    across `epochs`. `counts_by_epoch` is one `[(produced, blocksInEpoch), …]` list per epoch, in
    committee order.

    THREE conditions in order, and the first two are what the bash original could not express.

    (a) THE COUNTERS SUM TO `blocksInEpoch`. A self-check no log tally could offer: `log_count`
        answers 0 on a failed read, so bash's per-window delta could go NEGATIVE and satisfy both
        halves of its condition trivially (heavy=-1 > light_max=-10). On-chain counters cannot be
        lost, cannot be rotated away and cannot go negative, and their own total is published
        alongside them — so a lost read shows up as arithmetic that does not close.

    (b) EVERY LIGHT VALIDATOR PRODUCED AT LEAST ONE BLOCK. Without it a monopoly elector that
        always returns index 0 scores the BEST possible result on the margin criterion — the check
        would REWARD the exact failure it exists to catch. Summing across epochs is what makes the
        condition affordable: one 64-view epoch leaves ~1.1% chance of a light producing nothing by
        luck, which would be a coin-flip gate (`WEIGHTED_EPOCHS`).

    (c) ONLY THEN the margin, as the original wrote it.

    THE SUM IS ONLY VALID WHILE THE COMMITTEE DOES NOT CHANGE BETWEEN THE EPOCHS, which is true of
    the no-rotation stack this case brings up and would need revisiting on a rotation
    substrate. Two thirds of that assumption are self-enforcing: a member that LEFT reads
    `(-1,-1)` for the epoch it missed and is rejected below, and a member that JOINED takes blocks
    nobody in `counts_by_epoch` is credited with, so condition (a) stops closing. What is NOT
    caught is a STAKE change between the epochs — re-delegation would move the weights under the
    measurement, and nothing on this stack issues one.

    WHAT THIS DOES NOT TEST, and the case name should not be read as claiming it: the VRF. The
    seed arm and the fallback arm of `randomness_bytes` are hashed into the SAME CDF by `elect`
    (`crates/dpos/consensus/src/weighted_vrf.rs:136-154,190-198`), so a completely dead beacon
    leaves the leader distribution unchanged. That is why the case also asserts the beacon is
    live, separately, through the metrics."""
    epochs = list(epochs)
    for e, counts in zip(epochs, counts_by_epoch):
        for i, (produced, _) in enumerate(counts):
            state = credit_state(produced)
            if state != STATE_OK:
                return False, (f"validator-{i} production read {state} in epoch {e} "
                               f"({produced}) — refusing to score an election over it")
    produced = [sum(int(counts[i][0]) for counts in counts_by_epoch)
                for i in range(len(counts_by_epoch[0]))]
    total = sum(int(counts[0][1]) for counts in counts_by_epoch)
    span = f"epochs {epochs}" if len(epochs) > 1 else f"epoch {epochs[0]}"
    if sum(produced) != total:
        return False, (f"production counters do not add up over {span}: {produced} sums to "
                       f"{sum(produced)}, blocksInEpoch={total} — a reading was lost, so no "
                       "distribution can be scored")
    heavy, lights = produced[0], produced[1:]
    if min(lights) == 0:
        return False, (f"a light validator produced NOTHING over {span} ({produced}) — the "
                       "elector is not distributing by weight, it is picking a fixed index")
    light_max = max(lights)
    if heavy > light_max and heavy * WEIGHTED_MARGIN_DEN >= light_max * WEIGHTED_MARGIN_NUM:
        return True, ""
    return False, (f"validator-0 ({mult}x stake) did not produce a weighted plurality over "
                   f"{span}: heavy={heavy}, light_max={light_max}, total={total}")
