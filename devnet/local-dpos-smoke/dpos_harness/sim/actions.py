"""actions.py — churn menu + the PURE safety gate. Python port of scripts/soak-actions.sh.

Two clearly separated halves, exactly as the bash:
  1. gate_accept() — a PURE function (NO docker/cast/network): reads only its GateInputs and
     returns (accept: bool, reason: str). This is the sim's safety proof — the orchestrator
     populates the inputs from live on-chain reads, the unit test populates them synthetically
     (§8.4). Keeping the safety logic isolated and testable is the no-crutch payoff (D3).
  2. Actuators — DO touch docker/cast (through the proc seam); each is a single-shot reuse of an existing
     case body (the orchestrator owns restore timing). They never decide safety; the gate did.

Plus the pure selectors/economics helpers the dispatcher leans on (_slot_reusable WITH the
seated-guard-first fix, _bench_join_due, _spare_reserved_for_refill, select_promote_candidate,
_warm_debt_step, the DKG-barrier decision core, _promote_nohost_is_leak). `counter_progress` is
re-exported from `core.counters` (shared with the battery) and `sim_regen_staking_reader` now
lives only in `chain.writes`, where its one production caller (`stack.bringup`) reads it.

PORT-NOTE: every bash contortion here exists to dodge a bash trap the port deletes structurally —
`if sim_warm_ready; then` instead of `sim_warm_ready && n=…` (the RUN#7/#8 &&-as-last-statement
errexit gotcha), out-globals instead of `$(f)` (subshell write-loss), `${!arr[@]}` instead of
`${#arr[@]}` (set -u on empty assoc). The INTENT is translated; the trap-dodge is noted where it
shaped a signature (e.g. enqueue returns its seat as a value, not via a global).
"""

from __future__ import annotations

import os
import pathlib
import time
from dataclasses import dataclass, field

from ..core import lifecycle, proc, setops, topology
from ..core.counters import counter_progress


# ── pure classifiers ─────────────────────────────────────────────────────────

_DISRUPTION_ACTIONS = {
    "graceful_stop_restart", "sigkill_restart", "cpu_throttle",
    "dkg_midwindow_restart", "byzantine_equivocate", "byzantine_forge_pk",
}


def adds_disruption(action: str) -> bool:
    """_adds_disruption: actions that add a TRANSIENT disruption (count toward rule 1).
    register_activate (new node) and delegate_shift (a tx) add none; voluntary_exit does not
    (the OUT node stays up)."""
    return action in _DISRUPTION_ACTIONS


def bench_join_due(round_: int, last_fire_round: int, gap: int) -> bool:
    """_bench_join_due (phase-3 D5): ELAPSED rounds since the last fire >= gap. Was
    `ROUND % GAP == 0` (a FIXED phase that anything reliably claiming the multiple-of-GAP
    rounds could phase-lock supply off forever); elapsed-rounds has no phase to lock onto —
    a missed opportunity stays due until taken. PURE."""
    return round_ - last_fire_round >= gap


def spare_reserved_for_refill(spare: int, next_joiner: int, spare_end: int) -> bool:
    """_spare_reserved_for_refill: True iff a SPARE-band container is still earmarked for the
    refill path and must not be stolen as a mint host. Once the NEXT_JOINER cursor reaches
    SIM_SPARE_END the refill path is permanently dead → nothing reserved (bash returns 1/false
    there). PURE.

    UPPER-BOUND (v61 a18): refill only ever consumes the SPARE band `[validators, spare_end)` —
    its NEXT_JOINER cursor never advances past spare_end. So a ROTATION-band container
    (`spare >= spare_end`, i.e. `[spare_end, val_containers)`) is NEVER a refill target and must
    NOT read as reserved. The old `spare >= next_joiner` had no upper bound, so at the steady-state
    full committee (next_joiner == validators, all rotation slots satisfy spare >= next_joiner) it
    falsely reserved rotation slots 12,13 too → promote_host_avail(bench idx) found NO hostable
    rotation slot and NO recyclable tombstone → bench-join never fired → PROMOTABLE stayed empty →
    add_source_avail == 0 → voluntary exits gate 'rule7-no-add-source' and tombstone backfills
    starve. Rotation slots exist SPECIFICALLY to host re-tasked bench identities (orchestrator.py
    'Rotation slots serve their OWN native identity until a rebirth re-tasks them'); the missing
    bound was locking them out of that role."""
    if next_joiner >= spare_end:
        return False   # refill path exhausted → nothing reserved
    return next_joiner <= spare < spare_end   # SPARE band only, at/past the cursor → refill's to consume


def slot_reusable(served: int, native: int, seated: bool, promotable: str,
                  promote_pending: str, refill_pending: str) -> bool:
    """_slot_reusable — PURE: may a rotation slot currently serving `served` be re-tasked?
    True = REUSABLE, False = still occupied.

    SEATED-GUARD-FIRST (incident bundle-20260720T170507Z, current working tree): a spare's
    NATIVE idx can be refill-consumed straight into the committee (Active, seated) while it
    keeps serving that same native idx (served == native). With the native short-circuit ahead
    of the seated guard, such a consumed spare read as "idle by definition" and the picker
    re-tasked a LIVE committee member's container to host a bench identity — orphaning the seat
    on a dead node. So the SEATED guard runs FIRST, before the served==native short-circuit."""
    if seated:
        return False                                   # live committee member → never evict
    if str(served) == str(native):
        return True                                    # not re-tasked AND not seated → idle
    if setops.in_set(str(served), promotable):
        return False                                   # queued for a future promote → needed
    if promote_pending and str(served) == str(promote_pending):
        return False                                   # promote landing in flight
    if refill_pending and str(served) == str(refill_pending):
        return False                                   # refill landing in flight
    return True                                        # reclaimable


def warm_debt_step(held: int, budget: int, prev: int = 0):
    """_warm_debt_step: PURE 2-tick belt for the per-container warm-debt watchdog. Returns
    (verdict, over) where verdict ∈ ok|warn|fail and over is the new over-deadline tick count.
    held<budget → (ok,0); else prev+1, and >=2 → fail else warn (mirrors RECOVER_OVER_TICKS)."""
    if held < budget:
        return ("ok", 0)
    over = prev + 1
    return (("fail" if over >= 2 else "warn"), over)


def promote_nohost_is_leak(epochs: int, max_epochs: int, stuck: str) -> bool:
    """_promote_nohost_is_leak — PURE: is a promote deferred-no-host for `epochs` WITH a
    should-be-recyclable tombstone present a GENUINE recycle-path leak (True=fail), rather than
    pure host-model saturation (False=keep deferring)? Only >= max epochs AND a non-empty stuck
    set indicts a real bug; pure slot-scarcity defers forever (bundle-20260717T060506Z)."""
    return epochs >= max_epochs and bool((stuck or "").strip())


def promote_gate_reason(read_ok: int, cur: int, settle: int, clean: int, n: int, cap: int) -> str:
    """_promote_gate_reason (phase-3 D4): name the FIRST blocking OUTER-gate predicate when a
    backfill obligation is OPEN but the promote track is gated before candidate selection —
    that shape logged NOTHING, an open obligation could sit behind a dirty/at-cap read for
    epochs with no reason line (no-silent-rounds prong). Empty string when no outer gate
    blocks. PURE."""
    if read_ok != 1:
        return ("committee UNREADABLE this tick (SIM_COMMITTEE_READ_OK=0) — the promote "
                "decision is deferred to the next observed tick")
    if cur < settle:
        return (f"settle window open (cur_epoch {cur} < SETTLE_UNTIL_EPOCH {settle}) — a prior "
                "membership change is still settling")
    if clean != 1:
        return ("committee DIRTY (a CURRENT member is disrupted) — firing now would put a "
                "second perturbation in the on-change DKG window")
    if n >= cap:
        return (f"committee at cap (n={n} >= {cap}) — the seat is already covered; the "
                "SEAT-COVERED clear retires the obligation")
    return ""


# ── promote-candidate selection by ACTIONABILITY (v56 deadlock root fix) ──────

@dataclass
class PromoteChoice:
    """Result of select_promote_candidate — out-globals PROMOTE_CAND / PROMOTE_CAND_KIND /
    PROMOTE_BLOCK_REASON as a value, so no subshell-write-loss dodge is needed."""
    cand: str = ""
    kind: str = ""          # "ready" | "warm" | ""
    block_reason: str = ""
    ok: bool = False


def select_promote_candidate(pool: str, warm_ready, host_avail) -> PromoteChoice:
    """select_promote_candidate: the FIRST warm_ready entry wins (fire this round); failing
    that, the first HOSTABLE entry (kind=warm, warm toward a promote next round). A non-ready
    head-of-line entry therefore no longer SHADOWS a ready one behind it (v56: zero promotes in
    651 rounds). warm_ready(idx)/host_avail(idx) are seams (the orchestrator passes the real
    docker probes; the gate test passes stubs)."""
    idxs = (pool or "").split()
    first_hostable = ""
    for idx in idxs:
        if warm_ready(idx):
            return PromoteChoice(cand=idx, kind="ready", ok=True)
        if not first_hostable and host_avail(idx):
            first_hostable = idx
    if first_hostable:
        return PromoteChoice(cand=first_hostable, kind="warm", ok=True)
    if not idxs:
        reason = ("PROMOTABLE is EMPTY — no bench identity exists to promote "
                  "(bench-join/mint is the only supplier)")
    else:
        reason = (f"none of the {len(idxs)} PROMOTABLE entries [{pool}] is warm_ready, and "
                  "none is hostable (every rotation slot re-tasked AND no recyclable tombstoned "
                  "container)")
    return PromoteChoice(block_reason=reason, ok=False)


# ── the DKG-readiness barrier decision core (mechanism A) ─────────────────────

def dkg_member_ready(base, cur) -> bool:
    """_dkg_member_ready decision core: 0 (ready) iff a CAPTURED per-change baseline exists AND
    dkg_ceremony_ok rose >= 1 above it. Counter-reset safe via counter_progress: a member whose
    container is recreated between capture and read reads BELOW its baseline — that is a RESTART,
    not a stall (the caller re-captures). Returns (ready, action) where action ∈ ready|not-ready|
    rebaseline (the caller applies the DKG_BARRIER_BASE side-effect for rebaseline)."""
    if base is None or base == "__none__":
        return (False, "not-ready")           # baseline never captured → not ready
    st = counter_progress(cur, base)
    if st == "unreadable":
        return (False, "not-ready")
    if st == "restart":
        return (False, "rebaseline")          # container recreated → re-baseline, not ready
    return (int(cur) - int(base) >= 1, "ready" if int(cur) - int(base) >= 1 else "not-ready")


# ── THE GATE (pure) ───────────────────────────────────────────────────────────

@dataclass
class GateInputs:
    """gate_accept inputs — the caller sets these before calling; the unit test sets them
    directly (§8.4). Names/semantics 1:1 with the bash GA_* globals."""
    action: str = ""              # GA_ACTION
    victim: str = ""              # GA_VICTIM ("" for register_activate)
    n: int = 0                    # GA_N — current LIVE committee size
    disrupted: str = ""           # GA_DISRUPTED — victim-availability only
    effective: str = ""           # GA_EFFECTIVE — the authoritative EFFECTIVE_FAULTS union
    change_imminent: int = 0      # GA_CHANGE_IMMINENT
    incoming: str = ""            # GA_INCOMING — committee(cur+1) members
    change_pending: int = 0       # GA_CHANGE_PENDING — an in-flight membership change
    shareless: int = 0            # GA_SHARELESS
    min_committee: int = 0        # GA_MIN_COMMITTEE
    pending_min: int = 0          # GA_PENDING_MIN
    leader: str = ""              # GA_LEADER
    round_others: int = 0         # GA_ROUND_OTHERS
    membership_settling: int = 0  # GA_MEMBERSHIP_SETTLING
    add_source_avail: int = 1     # GA_ADD_SOURCE_AVAIL


def gate_accept(ga: GateInputs):
    """gate_accept — PURE. Returns (accept: bool, reason: str). Reject precedence mirrors
    plan §3.3 rules 1-7 (+ victim-availability). Byte-faithful to the bash logic;
    every rule's incident rationale is transcribed on the branch."""
    # f from the live committee size.
    f = (ga.n - 1) // 3

    # Victim availability: never re-hit a disrupted node; validator-0 is pinned honest.
    if ga.victim:
        if topology.is_pinned_rpc_host(ga.victim):
            return (False, "victim-is-pinned-host-rpc")
        if setops.in_set(ga.victim, ga.disrupted):
            return (False, "victim-already-disrupted")

    adds = adds_disruption(ga.action)

    # Rule 1 — transient quorum: the chain survives exactly f EFFECTIVE faults (live = n-f =
    # 2f+1 = quorum); reject the (f+1)-th. Counts GA_EFFECTIVE (the authoritative union), NOT
    # bare SIM_DISRUPTED — a beacon-warm-debted or self-partitioned member is a fault consensus
    # can't count (epoch-31 hole). Projection: +1 only if the victim is not already in the union.
    if adds:
        cur = setops.count(ga.effective)
        proj = cur
        if not ga.victim or not setops.in_set(ga.victim, ga.effective):
            proj = cur + 1
        if proj > f:
            return (False, f"rule1-transient-quorum (effective {proj} > f={f})")

    # Rule 2 — DKG-window safety: in the E-1 window of a change, only dkg_midwindow_restart may
    # touch an INCOMING member, and never > f of them (>f incoming pre-seal = terminal halt).
    if ga.change_imminent == 1 and ga.victim and setops.in_set(ga.victim, ga.incoming):
        if ga.action != "dkg_midwindow_restart":
            return (False, "rule2-dkg-window (only dkg_midwindow_restart may touch incoming in E-1)")
        inc_dis = setops.count(setops.intersect(ga.disrupted, ga.incoming))
        if inc_dis + 1 > f:
            return (False, f"rule2-dkg-window (incoming disrupted {inc_dis + 1} > f={f})")

    # Rule 2b — DKG-GOSSIP mesh-churn window. A kill/recreate/rejoin fault churns the p2p +
    # beacon MESH as the container drops and cold-sync-REJOINS; in the compressed test epoch a
    # restart overlapping a committee change can make almost the whole committee[E] DKG fail to
    # collect n-f QUAL dealer logs → permanent finalize-stall (seed 20260717 e101→102, e60→61).
    # Refuse such a restart whenever an imminent OR in-flight committee change overlaps its
    # ~2-epoch down+REJOIN window. dkg_midwindow_restart stays the ALLOWED exception; cpu_throttle
    # is not listed (no recreate → no mesh churn). Only DEFERS (retries next round).
    if ga.action in ("sigkill_restart", "graceful_stop_restart", "byzantine_forge_pk"):
        if ga.victim and (ga.change_imminent == 1 or ga.change_pending == 1):
            return (False, "rule2b-dkg-mesh-window (kill/recreate restart churns an imminent/"
                           "in-flight committee[E] DKG gossip window)")

    # Rule 3 — shareless <= f. A down member of the incoming committee during E-1 is PREDICTABLY
    # shareless on restart (missed the ceremony), so count it even though its metric is unreadable.
    if adds:
        sl = ga.shareless
        if ga.action == "dkg_midwindow_restart" or (
            ga.change_imminent == 1 and ga.victim and setops.in_set(ga.victim, ga.incoming)
        ):
            sl += 1
        if sl > f:
            return (False, f"rule3-shareless ({sl} > f={f})")

    # (Rule 4 — the transient-shrink floor — is GONE with the `liveness_jail` action it gated;
    # every surviving membership loss is permanent and is priced by rule 4b/4c below.)

    # Rule 4b — byzantine equivocation is a PERMANENT tombstone. Never drop the LIVE committee
    # below the absolute floor; a spare refill restores toward cap afterwards (E+3).
    if ga.action == "byzantine_equivocate":
        if ga.n - 1 < ga.min_committee:
            return (False, f"rule4b-byzantine-floor (tombstone drops below {ga.min_committee})")
        # Rule 4c — SERIALIZE membership changes: overlapping a tombstone with a growth/refill in
        # flight makes committee[E] include a not-ready joiner AND the tombstoned member → the DKG
        # under-qualifies → beacon DEADLOCK (seed cascadefull1 epoch 6).
        if ga.membership_settling == 1:
            return (False, "rule4c-serialize-membership (a growth/refill/tombstone is still settling)")

    # Rule 5 — pending projection: no committee in {cur+1..cur+3} may fall below the floor.
    if ga.pending_min < ga.min_committee:
        return (False, f"rule5-pending-projection (future committee {ga.pending_min} < {ga.min_committee})")

    # Rule 6 — leader-liveness (SOFT, WeightedVRF): don't drop the top-stake leader concurrently
    # with another disruption (false-positive-stall guard).
    if ga.victim and ga.victim == ga.leader and ga.round_others >= 1:
        return (False, "rule6-leader-liveness (top-stake leader + concurrent disruption)")

    # Rule 7 — voluntary_exit: the SOLE reversible committee-shrink. Floor + serialize (like
    # 4b/4c) PLUS an add-source starvation guard (a voluntary_exit is OPTIONAL, unlike a safety
    # tombstone) PLUS a post-shrink f recompute.
    if ga.action == "voluntary_exit":
        if ga.n - 1 < ga.min_committee:
            return (False, f"rule7-floor (exit drops below {ga.min_committee})")
        if ga.membership_settling == 1:
            return (False, "rule7-serialize-membership (a change is still settling)")
        if ga.add_source_avail == 0:
            return (False, "rule7-no-add-source (nothing available to backfill the exit)")
        # (d) post-shrink quorum: an exit removes a seat → n'=n-1, f'=(n-2)/3. The faults still
        # SEATED after the departure must fit the SMALLER f' (n=7 f=2, 2 seated faults → exit
        # healthy → n'=6 f'=1 → 2 > 1 → loss).
        fp = (ga.n - 2) // 3
        remain = setops.count(ga.effective)
        if ga.victim and setops.in_set(ga.victim, ga.effective):
            remain -= 1
        if remain > fp:
            return (False, f"rule7d-post-shrink-quorum (remaining faults {remain} > f'={fp} "
                           f"at n'={ga.n - 1})")

    return (True, "")


# ── seat/backfill obligation mutators (operate on orchestrator SimState) ─────
# These mutate the case-soak.sh globals; in the port they take the SimState and mutate its
# domain-typed maps. See orchestrator.SimState for the domain contract. Kept here (not on the
# state) because they mirror soak-actions.sh's producer/clearer functions 1:1.

def enqueue_backfill_obligation(state, container: str, cur_epoch: int) -> int:
    """enqueue_backfill_obligation — the SINGLE producer for PENDING_BACKFILL_BY_SEAT. Resolves
    the SEAT (the logical identity idx the container serves RIGHT NOW, via ident_idx) at ENQUEUE
    time and FREEZES it as the key, records the SEAT_CONTAINER bridge, clears stale pause credit,
    and RETURNS the seat (bash returned it in the global BACKFILL_SEAT to dodge subshell
    write-loss; the port returns it as a value so the correct call shape is the only one).

    NO-OVERWRITE GUARD: a seat already owing a backfill keeps its ORIGINAL (older) epoch stamp —
    refreshing silently extends a live watchdog deadline (how a collapsed seat went
    un-watchdogged)."""
    seat = state.ident_idx(container)
    state.seat.seat_container[seat] = container
    if seat in state.seat.pending_backfill_by_seat:
        return seat   # keep the OLDER stamp (never refresh a live deadline)
    state.seat.pending_backfill_by_seat[seat] = cur_epoch
    state.seat.backfill_pause_credit.pop(seat, None)
    state.seat.backfill_pause_laste.pop(seat, None)
    return seat


def clear_pending_backfill_seat(state, seat: int):
    """clear_pending_backfill_seat: clear the obligation for a SPECIFIC seat (the against-f
    new-face recover is PER-SEAT). DISRUPT_KIND is CONTAINER-keyed → unset through the
    SEAT_CONTAINER bridge, never with the seat key (that would be the cross-domain write the
    re-keying removes). SEAT_CONTAINER dropped last (bridge dead once the obligation retires)."""
    c = state.seat.seat_container.get(seat)
    state.seat.pending_backfill_by_seat.pop(seat, None)
    if c is not None:
        state.container.disrupt_kind.pop(c, None)
    state.seat.seat_container.pop(seat, None)


def release_covered_seat(state, seat: int):
    """_release_covered_seat: the seat opened by this obligation is now DURABLY re-covered on
    chain — free its against-f SIM_DISRUPTED slot (through the SEAT_CONTAINER bridge) AND clear
    its obligation, atomically. A seat whose container was already recycled has NO bridge
    (severed at recycle) and correctly frees nothing (its slot was provably already free)."""
    c = state.seat.seat_container.get(seat)
    if c is not None:
        state.unmark_disrupted(c)
    clear_pending_backfill_seat(state, seat)


def clear_oldest_pending_backfill(state):
    """clear_oldest_pending_backfill — the FIFO clearing owner: releases the OLDEST obligation
    (FIFO-min by stored EPOCH stamp). Routed through release_covered_seat (NOT a bare pop) so the
    obligation-clear and the against-f slot-release stay ATOMIC. No-op when none open."""
    if not state.seat.pending_backfill_by_seat:
        return
    oldest_seat = min(state.seat.pending_backfill_by_seat,
                      key=lambda s: state.seat.pending_backfill_by_seat[s])
    release_covered_seat(state, oldest_seat)


# ── impure actuators (docker / cast via the proc seam) ───────────────────────

class Actuators:
    """The act_* wrappers + gov/register/mint/promote flows. DO touch docker/cast; each is a
    single-shot reuse of an existing case body. They never decide safety (the gate did). The
    orchestrator manages the DISRUPTED/PENDING bookkeeping AROUND these calls.

    Every method is a literal translation of the bash `docker`/`cast` invocation (operator
    CLI-version parity). Under dry_run every method early-returns 0 (the sim_selfcheck flush) —
    the arg-contract validation the bash `${1:?...}` did is now Python's required parameters, so
    a missing arg raises TypeError at call time, not a cryptic set-u abort hours in.

    PORT-NOTE: the exec boundary is stubbable — the unit tests replace `_run`/`_run_ok` so no
    docker/cast is touched. This class is NOT exercised by the pure-logic suites.
    """

    # Declared on the CLASS, not only bound in __init__, because the orchestrator writes it late
    # (`self.act.events = events`) and test_apply's surface assertion resolves every name
    # production reaches for on the actuator against `dir(Actuators)` — which does not see
    # instance attributes. A class-level default keeps that check honest.
    events = None

    def __init__(self, env: dict, dry_run: bool = False, events=None):
        self.env = env                 # RPC/COMPOSE_FILE/STAKING_RT/TOKEN/GOV_ADDR/CHAIN_ID …
        self.dry_run = dry_run
        # The exec seam. Deliberately built with NO env of its own: every actuator must run with
        # exactly the AMBIENT process env (`{**os.environ, **overlay}`), because `self.env` is a
        # snapshot taken in Orchestrator.__init__ BEFORE bring-up and its COMPOSE_FILE is still
        # the empty startup sentinel. Seeding it into the runner would let that stale "" SHADOW
        # the live os.environ value for every command — the v61.11 failure, generalised from one
        # method to all of them. Overlays are passed per call instead (act_byzantine).
        # record=False: the actuators fire every round for hours, and this is not the bring-up
        # choreography the transcript is an oracle for.
        self.p = proc.Runner(dry=dry_run, record=False)
        self.stop_fin = {}             # STOP_FIN: victim -> own finalized height before a stop
        # The shared run event sink — the SAME (kind, note) list Dispatcher/Reconcilers/Battery
        # append to, drained into events.jsonl each tick (orchestrator.run:_drain). Bound late by
        # the orchestrator (the sink is created inside run(), after __init__), so it stays None in
        # unit tests / dry probes and self.event() then drops silently.
        self.events = events

    def event(self, kind, msg):
        """Mirror of Dispatcher.event / Reconcilers.event — append to the shared sink. Silently
        dropped when no sink is bound; a barrier must never fail for want of a logger."""
        if self.events is not None:
            self.events.append((kind, msg))

    # --- exec seams (stubbed in tests; all three delegate to proc.Runner) ---
    def _run_ok(self, cmd, env_overlay=None, timeout=90) -> bool:
        """Run a command, swallow output; True on rc 0. Mirrors the bash `... || true` sites
        that must never abort a night-long sim on a transient docker hiccup.

        `env_overlay` layers vars on top of the AMBIENT env for this one call — it never replaces
        it. That is the whole fix for the byzantine no-op: the only way to add a var is to add it,
        so a caller cannot hand over a dict that silently omits COMPOSE_FILE."""
        if self.dry_run:
            return True
        return self.p.run_ok(cmd, env_overlay=env_overlay, timeout=timeout)

    def _run(self, cmd, timeout=90) -> str:
        """Run a command and return its stdout (VERBATIM — `_cid`/`_cid_any` strip at their own
        call sites and the log readers need the line structure); "" on failure/timeout."""
        if self.dry_run:
            return ""
        return self.p.read(cmd, timeout=timeout)

    def _compose(self, *args):
        return ["docker", "compose", *args]

    def _cid(self, service: str) -> str:
        """_cid: raw container id for the throttle/sigkill paths (docker update --cpus / docker
        kill need an id; victim is RUNNING at kill time). MUST NOT resolve a STOPPED container
        (`compose ps -q` is RUNNING-only) — the sigkill RESTART uses `compose start <svc>`."""
        return self._run(self._compose("ps", "-q", service)).strip()

    def _cid_any(self, service: str) -> str:
        """`compose ps -aq` — the container id INCLUDING a stopped/exited one (lib.sh:78).

        Deliberately a SEPARATE helper from _cid, which is `ps -q` (RUNNING-only) and whose
        docstring forbids resolving a stopped container. _shutdown_flushed must inspect the
        container AFTER it exited, which `-q` cannot do — it would return "" and the barrier would
        report "no container" for every graceful stop."""
        return self._run(self._compose("ps", "-aq", service)).strip()

    # --- the two graceful-stop barriers (lib.sh:66-138) --------------------------
    #
    # Ordering is owned INSIDE the act_* methods (cursor → stop → flushed → tripwire → [start]) so
    # no caller can drift it — the bash call sites (soak-actions.sh:527-531, :558-563) had to
    # repeat all five steps by hand at each site.

    @staticmethod
    def _log_cursor() -> str:
        """`date -u +%Y-%m-%dT%H:%M:%SZ` — the cursor handed to `docker logs --since`.

        The format is load-bearing, not cosmetic: docker compares it against DAEMON-side host
        timestamps, so it must be UTC RFC3339 to the second. Captured by the caller IMMEDIATELY
        BEFORE the stop, so the tripwire greps only THIS stop's log slice and an unrelated
        halt/crash line hours earlier can neither trip it nor scope it out.

        Delegated to `core/lifecycle` so the sim and the static stack cannot end up handing
        docker two different timestamp formats."""
        return lifecycle.log_cursor()

    def _shutdown_flushed(self, v) -> bool:
        """lib.sh:66-84 — BARRIER, not an assertion: delay the restart until the container really
        exited. A graceful `docker compose stop` lets reth's own on_graceful_shutdown persist the
        in-memory tail to MDBX before the container exits 0; a SIGKILL on the 40 s stop-timeout
        exits 137. Poll `docker inspect <cid> --format '{{.State.Running}} {{.State.ExitCode}}'`,
        10 attempts 1 s apart, success on `false 0`. False when the cid is empty or the window
        expires.

        MUST NOT be reimplemented as a `docker compose logs | grep`. That is what this WAS, and it
        raced: under heavy docker-daemon load the log read lagged past the poll window and
        false-reported "flush incomplete" even though the node exited 0 in <1 s (diagnosed
        2026-06-05). Container metadata does not lag that way.

        Both bash call sites do `|| true`, so a False return is NOT fatal — it only means the
        restart proceeds without proof of a clean exit. Python records that on the event sink,
        which bash threw away.

        Port delta: bash sleeps once more after its final poll (a 1 s no-op before `return 1`);
        this skips the trailing sleep.

        The poll itself lives in `core/lifecycle` — shared with the static stack's migration
        flush gate, which needs the identical barrier. The read seam stays HERE (`self._run`),
        so a test that stubs `_run` still records the exact argv. `sleep` is passed explicitly
        rather than defaulted so the pacing is visible at the call site — and so this module's
        `time` stays the one a test neutralises."""
        if self.dry_run:
            return True
        return lifecycle.shutdown_flushed(v, self._run, sleep=time.sleep)

    # Kept as class attributes because the sim's own tests and readers reference them; the
    # definitions live in `core/lifecycle`, shared with the static stack.
    _ACK_DEATH = lifecycle.ACK_DEATH
    _ACK_SCOPE_OUT = lifecycle.ACK_SCOPE_OUT

    def _marshal_ack_tripwire(self, v, since: str) -> None:
        """lib.sh:118-138 — product-safety witness, scoped to graceful, unhalted stops. Raises
        ChainError on a positive detection (the fail-loud translation of the bash `_die`).

        This does NOT route through chain.writes.fatal_or_diag, which is the demotion policy: a
        marshal ack-death is a PRODUCT fault, not the harness's own seat bookkeeping, so it must
        not be silenceable by adding an id to DEMOTED_INVARIANTS. It raises directly and by
        construction has no id in that set.

        Three details carry the whole value, all preserved here:
          * `since` is the pre-stop UTC RFC3339 cursor (see _log_cursor) — the slice is THIS stop;
          * the read is retried up to 5×, 1 s apart, WHILE IT COMES BACK EMPTY. A graceful stop
            always logs something, so an empty slice is a lagging docker-logs read, not a clean
            one (the 2026-06-05 flush-race lesson, same as _shutdown_flushed);
          * it returns clean on an unreadable log. It must never trip on ABSENCE — nodes.logs_since
            yields "" for both an unreadable log and a genuinely empty slice, and after the retries
            both fall through the substring test to the same clean return, exactly as bash does
            (`|| return 0`, then a no-match grep over an empty `logs`)."""
        if self.dry_run:
            return
        logs = lifecycle.ack_slice(v, since, self._logs_since, sleep=time.sleep)
        if lifecycle.ack_verdict(logs) != lifecycle.TRIPPED:
            return
        msg = lifecycle.ack_tripwire_message(v)
        self.event("diag", msg)
        from ..chain.writes import ChainError
        raise ChainError("marshal-ack-tripwire", msg)

    def _logs_since(self, v, since: str) -> str:
        """Docker-log slice from `since` onward, ANSI-stripped; "" if unreadable. Delegated to
        nodes.logs_since (same shape as _node_finalized) rather than a second log reader."""
        try:
            from ..core import nodes
            return nodes.logs_since(v, since)
        except Exception:
            return ""

    # --- transient fault actuators (restore pair owned by the orchestrator) ---
    def act_graceful_stop(self, v):
        if self.dry_run:
            return
        # Pre-stop own-finalized capture for the no-hole rejoin assert (B' §9 item 11(iii)).
        self.stop_fin[v] = self._node_finalized(v)
        # Log cursor BEFORE the stop — the tripwire correlates to THIS stop event only.
        since = self._log_cursor()
        self._run_ok(self._compose("stop", "--timeout", "40", v))
        if not self._shutdown_flushed(v):
            # bash `shutdown_flushed "$v" || true`: never fatal. The note is the visibility bash
            # discarded — a stop that never showed a clean exit 0 is the pre-condition for the
            # truncated-persist class of failure, and is worth having in events.jsonl afterwards.
            self.event("churn", f"shutdown_flushed: {v} showed no clean exit within the barrier")
        self._marshal_ack_tripwire(v, since)

    def act_graceful_start(self, v):
        if not self.dry_run:
            self._run_ok(self._compose("start", v))

    def act_sigkill_stop(self, v):
        if not self.dry_run:
            self._run_ok(["docker", "kill", self._cid(v)])

    def act_sigkill_start(self, v):
        # compose start (NOT `docker start "$(_cid)"` — _cid is empty for a stopped container).
        if not self.dry_run:
            self._run_ok(self._compose("start", v))

    def act_cpu_throttle(self, v):
        if not self.dry_run:
            self._run_ok(["docker", "update", "--cpus", "0.15", self._cid(v)])

    def act_cpu_restore(self, v):
        if not self.dry_run:
            self._run_ok(["docker", "update", "--cpus", "4", self._cid(v)])

    def act_dkg_midwindow_restart(self, v):
        if self.dry_run:
            return
        # FLK-6: --timeout 40 (races reth's 5s graceful-persist ceiling; a truncated CeremonyStore
        # = share-less resume). The WHOLE point of this action is a COMPLETED persist so the victim
        # resumes its share from the on-disk journal, so the _shutdown_flushed barrier below is
        # what makes it a recoverable path rather than a race: it delays the restart until the
        # container is observed exited 0.
        self.stop_fin[v] = self._node_finalized(v)
        since = self._log_cursor()          # cursor BEFORE the stop (see act_graceful_stop)
        self._run_ok(self._compose("stop", "--timeout", "40", v))
        if not self._shutdown_flushed(v):
            self.event("churn", f"shutdown_flushed: {v} showed no clean exit within the barrier")
        self._marshal_ack_tripwire(v, since)
        self._run_ok(self._compose("start", v))

    def act_byzantine(self, v, mode):
        """Restart the victim under a single-service env overlay carrying FLUENT_DPOS_BYZANTINE.
        Stays on the COMPOSE_FILE seam (NO -f) + --no-deps (never re-run genesis-init)."""
        if self.dry_run:
            return
        overlay = f"docker-compose.sim.byz-{v}.gen.yml"
        with open(overlay, "w") as f:
            f.write(f"services:\n  {v}:\n    environment:\n      "
                    f'FLUENT_DPOS_BYZANTINE: "{mode}"\n')
        # Base the merged COMPOSE_FILE on the LIVE ambient value (bringup._set_compose exports
        # the phase-B sim file list into os.environ), NOT self.env — that dict is a snapshot
        # taken in Orchestrator.__init__ BEFORE bring-up, so its COMPOSE_FILE is still the empty
        # startup sentinel. Using it would append ":overlay" onto "", yielding a leading-colon
        # COMPOSE_FILE that drops the base sim files, so `up --force-recreate` becomes a silent
        # no-op and the victim never receives FLUENT_DPOS_BYZANTINE (v61.11: identity never
        # equivocated → byzantine-slash-not-landed). It is now an env OVERLAY on the seam rather
        # than a replacement dict, so os.environ is merged underneath it by construction and the
        # "built its own env" shape that caused this is no longer expressible.
        base = os.environ.get("COMPOSE_FILE") or self.env.get("COMPOSE_FILE", "")
        merged = f"{base}:{overlay}" if base else overlay
        # 120s (not the 90s default): a --force-recreate pulls the container down and back up.
        self._run_ok(self._compose("up", "-d", "--no-deps", "--force-recreate", v),
                     env_overlay={"COMPOSE_FILE": merged}, timeout=120)

    def act_byzantine_restore(self, v):
        """Restore a byzantine victim to HONEST (soak-actions.sh:605-611): drop its env overlay and
        recreate the container from the PLAIN ambient COMPOSE_FILE, so the next launch carries no
        FLUENT_DPOS_BYZANTINE. The restore pair for the RECOVERABLE byzantine_forge_pk action (the
        honest C-gate rejected its forged PK; once honest it rejoins). byzantine_equivocate has no
        restore (permanent tombstone), so this is never called for it.

        THIS MUST RUN BARE — `_run_ok` with NO env_overlay, inheriting the ambient env unmodified.
        act_byzantine applies its overlay EPHEMERALLY for one call (an `env_overlay=` on that one
        invocation; os.environ is never mutated), so the ambient COMPOSE_FILE never carried the
        overlay and a bare recreate is exactly what drops the byzantine env. Passing the merged
        compose list here too would RE-APPLY FLUENT_DPOS_BYZANTINE and leave the victim byzantine —
        the defect that kept four forgers running against f=3 for four days and produced two false
        product-bug diagnoses. The comment at :526-535 is about act_byzantine, which needs the
        merge; this one must not have it.

        --no-deps: never re-run genesis-init (it would clobber addresses.json/peers.json back to
        genesis). Fire-and-forget like bash (`|| true`, output swallowed) — the generic Phase-2
        restore confirm (reconcilers.process_restores) owns the rejoin assert, so there is no
        confirm step here."""
        if self.dry_run:
            return
        try:
            pathlib.Path(f"docker-compose.sim.byz-{v}.gen.yml").unlink(missing_ok=True)
        except OSError:                    # bash `rm -f ... 2>/dev/null || true`
            pass
        self._run_ok(self._compose("up", "-d", "--no-deps", "--force-recreate", v))

    def _node_finalized(self, v) -> int:
        """Own finalized height of a container (lib.sh node_finalized). -1 if unreachable — the
        rejoin confirm self-skips a -1 (only > 0 is asserted). Delegated to nodes.node_fin_in."""
        try:
            from ..core import nodes
            return nodes.node_fin_in(v)
        except Exception:
            return -1
