"""policy.py — invariant policy shared by the layers that judge a run.

WHY IT SITS IN `core` AND NOT IN `sim`: `DEMOTED_INVARIANTS` is read from TWO layers —
`chain.writes.fatal_or_diag` (the reconciler/dispatch watchdogs) and `checks.battery._inv_fail`
(the detector battery). It is sim POLICY semantically, but a home in `sim` would make both of
those reads UPWARD imports and break the layering outright. It lives below both readers instead.

It was moved here verbatim from `lib_write.py` (now `chain/writes.py`), where it had been defined
next to only ONE of its two consumers.
"""

from __future__ import annotations


# ── liveness-first invariant policy ─────────────────────────────────────────────
# The sim's ONLY pass/fail signal is chain LIVENESS (finalized advances) plus genuine SAFETY
# witnesses (result-divergence / fork / double-finalization / safety-halt). Every disruptive
# action is already gated ≤f at the SOURCE (actions.gate_accept rule 1), so the chain can never
# be pushed past the BFT bound by the harness.
#
# WHAT THIS SET IS FOR: exactly one class of detector — the harness's own seat/membership
# BOOKKEEPING. Those watchdogs model identity-keyed seats against the chain's top-K committee;
# the two diverge constantly during growth ramps / settle windows and produced endless FALSE
# stalls (v61 a1..a12). Demoting them keeps the state recorded (for future bug-finding) without
# failing the run on a harness-side modelling gap.
#
# That framing describes MOST of the set, not all of it — and reading it as if it described every
# member is how ids that are not bookkeeping at all (per-member liveness, a missing finalized
# block, a dead devp2p plane) ended up here unnoticed. The per-id record below is the authority:
# every id in the set has a line, and an id whose line does not amount to "bookkeeping, or a
# named deliberate exception" does not belong here.
#
# WHAT MUST NEVER GO IN IT: a detector that guards the BATTERY's own validity — one that answers
# "is this battery still able to see the chain at all". `detector-starved` and `battery-coverage`
# are the two, and both sat here anyway with no recorded reason, under a comment that only ever
# described the bookkeeping group; that framing is how the set grew to 22 ids. Demoting them
# silently un-guards everything else: at coverage 0 the un-demoted `safety-halt` asserts over an
# EMPTY node set and reports green. Both were removed on 2026-07-29 (task
# 20260729__sim_port_gaps, F3) once the runtime-address fix removed the single starved read
# channel that had been tripping `detector-starved` every run. If either fires again, fix the
# read channel — do not put it back.
# See memory sim-liveness-is-only-invariant-gate-actions-on-f.
#
# PER-ID RECORD (2026-07-29/30, task 20260729__sim_port_gaps F6). the package is untracked, so there is
# no git log / commit message to hold this: the reason an id is in (or was taken out of) this set
# lives HERE, next to the set, or it does not exist. 14 of the original 22 ids were demoted with no
# recorded reason anywhere — that is what this block exists to prevent recurring. Adding an id
# without a line below is a review blocker.
#
# KEPT — genuinely noisy, exactly the bookkeeping class the comment above describes (33 of the 60
# demoted-id bundles in 171 came from these three; the "45" figure elsewhere also counted `growth`'s
# 12, which were misattributed — see the DELETED group below):
#   bench                      — 15 bundles (promote no-host leak / mint deferral: pool bookkeeping)
#   tombstone-backfill-stall   — 12 bundles + 77 live diags, the loudest line in events.jsonl
#   recover-stall              — 6 bundles
# KEPT — deliberate, each for its own reason:
#   committee-vacancy / committee-stranger / committee-undersized — documented false positives
#       (seat model vs. the chain's live top-K). Two of the three have had fixes land since;
#       re-evaluate then, not now.
#   dkg-barrier-stuck / dkg-deferral-streak — the starved-membership-change class; partly contained
#       by dkg-commit-binding, which is NOT demoted and does fail the run.
#   self-partition-stall       — inert by env default; nothing to calibrate against yet.
#   rotation-stall             — Python-only detector, no bash oracle, never differentially
#       compared against a bash run. Un-demote once a run has compared it.
#   byzantine-slash-not-landed — kept demoted for now (it fires on a ≤f-gated condition the source
#       gate already bounds), but its tracking entry is no longer popped on a NON-landed slash
#       (reconcilers.process_tombstone_backfills), so the condition stays re-evaluatable. It found
#       a real contract bug once (EvidenceDecoderNotConfigured, 2026-07-02); a one-shot pop made
#       the 2026-07-22 live diag impossible to re-check.
# REMOVED 2026-07-29/30 — DELETED, dead entries with no Python call site at all (bash-era
# holdovers; verified by grep — the string "growth" survives only as dispatch/orchestrator
# `last_growth_kind` STATE and the case_growth module, never as a fail id):
#   growth, promote-shareless, refill-shareless
# REMOVED 2026-07-29/30 — UN-DEMOTED, zero recorded justification and zero false-fires in 171
# bundles; each now fails a run again, as its bash counterpart always did:
#   member-stall   — the ONLY per-member liveness check. finalize-stall fires only when EVERY node
#       is frozen (nodes.finalized_dec takes the max across the other running containers), so while
#       this was demoted up to f members could wedge permanently and the run ended green.
#   deferred-park  — member-stall's executor-side twin (the soak7 park signature); same hole.
#   committee-unhosted — this IS the fix that resolved committee-stranger's false positive, and it
#       was demoted along with the thing it fixed.
#   graceful-stop-hole — a rejoined node missing its own finalized block is a data hole, not seat
#       bookkeeping.
#   warm-debt-stall — a member that never re-warms after a restart is a real liveness fault.
#   peer-plane     — one bundle in 171, and it reads as a TRUE positive (devp2p plane down on a
#       seated member; consensus liveness will not reveal it until the node falls behind).
DEMOTED_INVARIANTS = frozenset({
    # reconciler / dispatch deadline watchdogs (seat + membership bookkeeping)
    "recover-stall", "byzantine-slash-not-landed", "tombstone-backfill-stall", "rotation-stall",
    "bench", "self-partition-stall", "dkg-barrier-stuck",
    # battery membership detectors
    "committee-vacancy", "committee-undersized", "committee-stranger",
    # a starved harness-own membership change (promote/refill deferred N boundaries) is the SAME
    # class as tombstone-backfill-stall — bookkeeping, not a chain fault.
    "dkg-deferral-streak",
})


# ── governance voter selection ──────────────────────────────────────────────────
# `pp_gov_action`'s three voter-selection modes (lib.sh:1105-1132). Here for the SAME reason as
# `DEMOTED_INVARIANTS`: two layers read it — `chain.writes.Chain._voter_list` (the write side) and
# `cases.smoke.verdicts_prod.voter_list` (the case layer's pure decision surface) — so a home in
# either would make the other's read an upward import.
#
# It was duplicated once, and the duplicate got the CLAMP ORDER wrong, which is why it is shared
# rather than mirrored with an agreement test: the two orders differ only at `voters == 0`, so a
# mirror would have stayed green.

def gov_voter_list(voters: int, vote_all: bool = False, explicit=None):
    """The voter idx list for one governance action, in bash's PRECEDENCE order.

    1. `explicit` (bash `PP_GOV_VOTER_IDX`) wins over everything. FluentGovernance's quorum is 2/3
       of the delegated STAKE, and once committee power migrates to MINTED identities (idx >= POOL,
       outside any prefix) a fixed prefix under-votes → the proposal is Defeated on a chain where
       nothing is wrong. The sim's churn callers pass the CURRENT committee's owner idxs.
    2. `vote_all` (bash `PP_GOV_VOTE_ALL=1`) → every listed voter. Same reason, blunter. All votes
       are "for", so this only ever ADDS for-power and can never flip a passing proposal.
    3. otherwise the `ceil(2/3·voters)+1` PREFIX — the equal-weight heuristic, correct on the
       uniform 5-validator production-path committee every non-sim caller runs on.

    The clamp is bash's, in bash's ORDER, and that is not `min(max(need, 1), voters)`:

        (( need > voters )) && need="$voters"     # first: never index a key that does not exist
        (( need < 1 ))      && need=1             # then:  never send zero votes

    At `voters == 0` the two disagree — bash yields `[0]`, the tidier spelling yields `[]`, i.e. a
    proposal with no votes at all, which then reads as the Governor defeating it.
    """
    if explicit is not None:
        if isinstance(explicit, str):
            return [int(x) for x in explicit.split()]
        return [int(x) for x in explicit]
    voters = int(voters)
    need = voters if vote_all else (2 * voters + 2) // 3 + 1
    if need > voters:
        need = voters
    if need < 1:
        need = 1
    return list(range(need))
