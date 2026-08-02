"""dispatch.py — the full per-round churn dispatch. Python port of case-soak.sh's main-loop
churn block (2231-3118): the landing hooks, PROMOTE ACTIONABILITY block, growth-in-flight
watchdog with deferral credits, growth/refill/promote/bench-join/mint tracks, the SERIALIZE
settle block, the DKG-barrier HOLD, check_resource_pools' exhaustion, the empty-victim skip,
the calm bit, the forced-override triggers, the gate inputs (incl. the D6 ncap-pin watchdog),
and apply. Translated 1:1 from the bash loop ORDER.

The reconcilers (compute_effective_faults / compute_dkg_barrier / process_restores /
process_tombstone_backfills / check_resource_pools) run at TICK-top in the orchestrator; this
module is the CHURN-cadence block that runs once per round. `run_round(cur, n)` returns the
SIM_VERDICT string ("applied"/"skipped"/"recover"/...).

Rolling growth/refill/promote/mint scalars that have no home on SimState live on the Dispatcher
(GROW_FIRE_TS, GROW_OVER_DEADLINE_TICKS, the deferral-credit + refill-stall + warm-stint state
machines, the mint-defer belt). The shared ones (NEXT_JOINER, PROMOTABLE, PROMOTE_PENDING, …)
are read/written on SimState so the battery adapter + gate inputs see the same values.

Impure seams (docker/RPC) are injected so a test drives the whole flow without a live topology:
beacon_share_delta / beacon_share_ready / beacon_metric / node_finalized / finalized_dec /
refill_spare_attempts / top_stake_leader / block_number. A live Dispatcher binds them.

PORT-NOTE: the RUN #7/#8 errexit-`&&`-branch-tail gotcha, the empty-assoc set-u trap, and the
`$(f)` subshell write-loss that shaped this bash are gone structurally (explicit branches, dict
iteration, value returns). The 4-draw PRNG discipline is preserved by delegating to
rounds.draw_round (the sacred replay layer).
"""

from __future__ import annotations

import hashlib
import os

from . import actions
from ..core import setops, topology
from ..chain.writes import fatal_or_diag
from .rounds import draw_round, build_gate_inputs, apply_action


# NOTE (liveness-first cleanup): the RefillStall / GrowDeferral progress-gate state machines and
# refill_conv_ticks were removed — they only ever fed the DEMOTED "growth" watchdog. The refill
# track's real behaviour (resume spare → await recovery_confirmed → activate) needs neither; a
# stalled refill is now a plain "deferring" churn event, and chain LIVENESS is the real gate.


class Dispatcher:
    def __init__(self, state, chain, act, hostpool, reconcilers, events=None):
        self.s = state
        self.chain = chain
        self.act = act
        self.hp = hostpool
        self.rec = reconcilers
        self.events = events if events is not None else []
        self._diag_seen = set()        # latch: one diag per (demoted-watchdog, entity) per run
        self.cfg = state.cfg
        self.calm_permille = int(self.cfg.calm_fraction * 1000)
        # dispatch-local rolling state (no SimState home)
        self.growth_gap = self.cfg.growth_gap
        self.membership_settle = self.cfg.membership_settle
        self.promote_nohost_max = int(os.environ.get("SIM_PROMOTE_NOHOST_MAX_EPOCHS", "10"))
        self.promote_warm_max_rounds = int(os.environ.get("SIM_PROMOTE_WARM_MAX_ROUNDS", "6"))
        self.mint_buffer = int(os.environ.get("SIM_MINT_BUFFER", "2"))
        self.bench_join_gap = self.cfg.bench_join_gap
        self.spare_end = self.cfg.spare_end
        self.max_mint_idx = int(os.environ.get("SIM_MAX_MINT_IDX",
                                               str(self.cfg.identity_pool - 1)))
        self.mint_eth_wei = os.environ.get("SIM_MINT_ETH_WEI", "10000000000000000")
        self.warm_stint = {}                  # identity idx -> consecutive warming rounds
        self.mint_defer_idx = ""
        self.mint_defer_acc = 0

    # ── impure seams (bound live; overridable in tests) ───────────────────────
    def finalized_dec(self):
        return self.rec.finalized_dec()

    def node_finalized(self, v):
        return self.rec.node_finalized(v)

    def beacon_share_delta(self, idx):
        return self.rec.beacon_share_delta(idx)

    def beacon_share_ready(self, idx, base):
        """sim_beacon_share_ready: True iff idx obtained a NEW committee-epoch share since the
        baseline. Returns (ready, reset). reset=1 means the container restarted (counter went
        backward) → the caller re-snapshots."""
        now = self.beacon_share_delta(idx)
        st = actions.counter_progress(now, base)
        if st == "unreadable":
            return (False, 0)
        if st == "restart":
            return (False, 1)
        return (now - base >= 1, 0)

    # LIVE SEAMS — construction stubs, listed in orchestrator.LIVE_SEAMS and rebound by
    # Orchestrator._bind_live_seams (top_stake_leader deliberately excepted — it is inert in bash
    # too). A new stub here must be added there too (enforced by
    # test_live_seams::test_live_seam_registry_covers_every_construction_stub).
    def beacon_metric(self, idx, family):
        return -1

    def refill_spare_attempts(self, v):
        return 0

    def top_stake_leader(self):
        return ""

    def event(self, kind, msg):
        self.events.append((kind, msg))

    # ── the churn round ───────────────────────────────────────────────────────
    def run_round(self, cur, n):
        """One churn round (case-soak.sh 2249-3118). Returns the verdict string."""
        s = self.s
        s.round += 1
        # ── ROTATION-PHASE routing (v61 a20): once GROWTH has filled the cap, switch to the
        #    chain-driven rotation phase. The committee is on-chain a stake-weighted top-k that the
        #    chain re-selects on EVERY stake/status change (auto-committed onlySystemCall), so
        #    phase-2 does NOT micro-manage membership (no register_activate / bench_promote / cap
        #    tinkering) — it only triggers a departure (stake/status op) and rekeys the departed
        #    container under a fresh key. The whole growth-phase machinery below runs UNCHANGED
        #    until the latch flips. See plan.md / research.md (task 20260723__soak_rotation_replacement).
        if not s.rotation_phase and self._growth_complete(n):
            s.rotation_phase = 1
            s.next_rekey_idx = max(s.next_rekey_idx, s.next_bench_joiner)
            self.event("churn", f"GROWTH COMPLETE (committee n={n} == cap {self.cfg.validators}) — "
                                "entering chain-driven ROTATION phase (stake/status departures + rekey)")
        if s.rotation_phase:
            return self._rotation_round(cur, n)
        # ── growth/refill LANDING high-water hook ──
        if (s.next_joiner - 1 > s.grow_landed
                and self.chain.committee_has(self.chain.owner_addr(s.next_joiner - 1), cur)):
            s.grow_landed = s.next_joiner - 1
            if s.last_growth_kind == "refill":
                actions.clear_oldest_pending_backfill(s)
        # ── PROMOTE landing hook (qualified-signer) ──
        if s.promote_pending:
            ready, reset = self.beacon_share_ready(s.promote_pending,
                                                   _int(s.promote_share_base, 0))
            if self.chain.committee_has(self.chain.owner_addr(s.promote_pending), cur) and ready:
                s.promote_share_base = ""
                s.promote_land_epoch = ""
                s.promote_over_ticks = 0
                s.promote_pending = ""
                actions.clear_oldest_pending_backfill(s)
            elif reset == 1:
                s.promote_share_base = str(self.beacon_share_delta(s.promote_pending))
                s.promote_land_epoch = ""
                s.promote_over_ticks = 0
        # ── REFILL qualified-landing DISARM ──
        if s.refill_pending:
            ready, reset = self.beacon_share_ready(s.refill_pending, _int(s.refill_share_base, 0))
            if self.chain.committee_has(self.chain.owner_addr(s.refill_pending), cur) and ready:
                s.refill_share_base = ""
                s.refill_land_epoch = ""
                s.refill_over_ticks = 0
                s.refill_pending = ""
            elif reset == 1:
                s.refill_share_base = str(self.beacon_share_delta(s.refill_pending))
                s.refill_land_epoch = ""
                s.refill_over_ticks = 0
        membership_clean = self._membership_clean(cur)
        pending_backfill_nonempty = 1 if s.seat.pending_backfill_by_seat else 0

        # ── DKG-barrier HOLD: skip ALL churn this round ──
        if self.rec.dkg_window_fragile == 1:
            self.event("churn", f"dkg-barrier HELD (epoch {cur}): incoming committee[{cur + 1}] "
                                f"not beacon-warm [{self.rec.dkg_barrier_notready.strip()}]")
            return "skipped"

        # ── pre-warm hook ──
        if (pending_backfill_nonempty == 1 and not s.promote_pending
                and not (s.next_joiner >= self.cfg.validators
                         and s.next_joiner < self.spare_end
                         and s.next_joiner - self.cfg.validators < s.tombstones)):
            choice = self.hp.select_promote_candidate()
            if choice.ok and choice.kind == "warm":
                self.hp.warm_identity(choice.cand)

        # ── PROMOTE ACTIONABILITY block ──
        promote_actionable = self._promote_actionability(cur, n, membership_clean,
                                                         pending_backfill_nonempty)

        # ── growth-in-flight watchdog / growth / refill / promote / bench-mint ──
        verdict = self._growth_refill_promote_bench(cur, n, membership_clean, promote_actionable)
        if verdict is not None:
            return verdict

        # ── HEAL-BEFORE-HARM: an OPEN backfill obligation on a BELOW-CAP committee suppresses NEW
        #    faults so the vacated seat heals back to full before more damage lands. Without this the
        #    fault lottery (calm_fraction 0.4 → ~60% of rounds draw a fault) re-dirties the committee
        #    faster than transients clear (~3 epochs each), so `membership_clean` is ~never 1 → the
        #    promote track is permanently gated ("committee DIRTY") → the seat never backfills and the
        #    committee bleeds to the min-committee floor (7-9) instead of holding the cap (10). This is
        #    a FINITE drain, not a deadlock: process_restores (transient rejoin) and tombstone slash-
        #    lands run at tick-top REGARDLESS of the lottery, so membership_clean reaches 1, the
        #    promote fires, the obligation clears, the committee returns to cap, and the lottery
        #    resumes. It orders healing ahead of new damage — it does NOT reduce total churn. (v61 a17)
        if pending_backfill_nonempty == 1 and n < self.cfg.validators:
            self.event("churn", f"heal-before-harm: backfill obligation open (committee n={n} < cap "
                                f"{self.cfg.validators}) — suppressing the fault lottery until the "
                                "vacated seat is backfilled to full")
            return "skipped"

        # ── SERIALIZE settle block ──
        if (cur < s.settle_until_epoch or s.promote_pending or s.refill_pending or s.refill_resumed):
            self.event("churn", f"settle-window (until epoch {s.settle_until_epoch}) or membership "
                                "change in flight — no fault churn")
            return "skipped"

        # ── GROWTH-COMPLETE gate: no fault churn until the committee has reached full cap.
        #    Growth (raising the on-chain cap `_activeValidatorsLength` 7→validators via one
        #    register_activate cap-raise per seat) and rotation (exit/backfill WITHIN full cap) must
        #    stay SEQUENTIAL phases. If a departure fires mid-growth it deadlocks: bench_promote sets
        #    promote_pending, but the promoted identity cannot seat while the cap is still below
        #    `validators` (the committee is top-`getActiveValidatorsLength()`, already full at the
        #    lower cap) — and the growth step that would raise the cap is itself gated by
        #    `not promote_pending` (the GROWTH elif). Circular → the committee freezes below cap
        #    forever. PROVEN v61 a18 (fin~1746 @epoch22): a voluntary_exit at epoch 8 (committee still
        #    at 8/cap 8) wedged the cap at 9 with idx-14's promote_pending stuck. Gating churn on a
        #    fully-grown committee is the root fix; heal-before-harm above only covers the WITH-
        #    obligation dip, this covers the pre-growth (no-obligation) window. bench_join supply and
        #    the growth track run ABOVE this gate, so PROMOTABLE still stocks during growth — ready
        #    for the first departure the moment the committee reaches cap. (v61 a19)
        if n < self.cfg.validators:
            self.event("churn", f"growth-incomplete (committee n={n} < cap {self.cfg.validators}) — "
                                "no fault churn until the committee is fully grown")
            return "skipped"

        # ── fault lottery: 4-draw + victim pool + calm + forced overrides ──
        return self._fault_lottery(cur, n)

    # ── PROMOTE ACTIONABILITY (v56 root fix) ──────────────────────────────────
    def _promote_actionability(self, cur, n, membership_clean, pending_backfill_nonempty):
        s = self.s
        promote_actionable = 0
        if pending_backfill_nonempty == 1 and not s.promote_pending:
            pgate = actions.promote_gate_reason(s.committee_read_ok, cur, s.settle_until_epoch,
                                                membership_clean, n, self.cfg.validators)
            if pgate:
                self.event("promote_gated", f"promote track GATED: {pgate}")
            else:
                choice = self.hp.select_promote_candidate()
                if choice.ok:
                    s.promote_cand = choice.cand
                    s.promote_cand_kind = choice.kind
                    if choice.kind == "warm" and self._warm_stint_expired(choice.cand):
                        s.promotable = (setops.set_remove(choice.cand, s.promotable)
                                        + " " + str(choice.cand)).strip()
                        self.event("promote_warm_rotated",
                                   f"promote candidate {topology.validator(choice.cand)} rotated to tail")
                    else:
                        promote_actionable = 1
                elif not s.promotable:
                    s.promote_nohost_since_epoch = 0
                    self.event("promote_blocked",
                               f"promote track IDLE: {choice.block_reason}")
                else:
                    if s.promote_nohost_since_epoch == 0:
                        s.promote_nohost_since_epoch = cur
                    nohost_epochs = cur - s.promote_nohost_since_epoch
                    stuck = self.hp_stuck_recyclable()
                    if actions.promote_nohost_is_leak(nohost_epochs, self.promote_nohost_max, stuck):
                        fatal_or_diag(self.events, "bench", f"promote no-host leak: {stuck}",
                                      seen=self._diag_seen, key=f"nohost:{stuck}")
                    self.event("promote_deferred_no_host",
                               f"no host for any promote candidate ({nohost_epochs}/"
                               f"{self.promote_nohost_max} epochs)")
        return promote_actionable

    def hp_stuck_recyclable(self):
        """_stuck_recyclable_tombstones: tombstones that SHOULD be recyclable (slash landed) but
        are NOT in _recyclable_containers — the genuine-recycle-leak signal."""
        recyc = set(self.hp.recyclable_containers())
        out = []
        for c in self.s.identity.permanently_dead:
            addr = self.chain.owner_addr(self.s.ident_idx(c))
            if addr and self.chain.committee_has(addr, self.s.cur_epoch):
                continue  # still seated
            if c in recyc:
                continue
            out.append(c)
        return " ".join(out)

    # ── warm-stint budget (phase-3 D1) ────────────────────────────────────────
    def _warm_stint_charge(self, idx):
        self.warm_stint[idx] = self.warm_stint.get(idx, 0) + 1

    def _warm_stint_expired(self, idx):
        if self.warm_stint.get(idx, 0) >= self.promote_warm_max_rounds:
            self.warm_stint[idx] = 0
            return True
        return False

    # ── growth / refill / promote / bench-mint dispatch chain ─────────────────
    def _growth_refill_promote_bench(self, cur, n, membership_clean, promote_actionable):
        s = self.s
        # growth-in-flight SERIALIZE (one membership change at a time): a growth/refill fired but
        # its joiner has not yet landed → hold. The landing-deadline watchdog that used to live here
        # was demoted-diagnostic and is removed with the GrowDeferral/RefillStall machinery;
        # serialization is the only live effect kept — chain LIVENESS is the real stall gate.
        if s.next_joiner > self.cfg.initial_committee and s.next_joiner - 1 > s.grow_landed:
            return None

        # GROWTH elif
        if (s.next_joiner < self.cfg.validators and cur >= s.settle_until_epoch
                and cur >= s.next_growth_epoch and membership_clean == 1 and not s.promote_pending):
            self.chain.register_activate(s.next_joiner, 1, voter_idx=self._live_voter_idx(cur))
            s.last_growth_kind = "growth"
            s.settle_until_epoch = cur + self.membership_settle
            s.next_growth_epoch = cur + self.membership_settle + self.growth_gap
            self.event("churn", f"APPLIED register_activate {topology.validator(s.next_joiner)} "
                                f"(committee {n}->{n + 1})")
            s.next_joiner += 1
            return "applied"

        # REFILL elif (mechanism B: resume → frontier → activate)
        if (s.committee_read_ok == 1 and s.next_joiner >= self.cfg.validators
                and s.next_joiner < self.spare_end
                and s.next_joiner - self.cfg.validators < s.tombstones
                and cur >= s.settle_until_epoch and membership_clean == 1 and n < self.cfg.validators
                and not s.promote_pending):
            v = topology.validator(s.next_joiner)
            if s.refill_resumed != v:
                self.act._run_ok(self.act._compose("up", "-d", "--no-deps", "--force-recreate", v)) \
                    if hasattr(self.act, "_run_ok") else self.chain.p.run_ok(
                    ["docker", "compose", "up", "-d", "--no-deps", "--force-recreate", v],
                    note="refill-resume")
                s.refill_resumed = v
                self.event("churn", f"refill: resumed spare {v}; awaiting FRONTIER catch-up")
                return "skipped"
            if not self.rec.recovery_confirmed(v):
                # a refill that never frontier-syncs is a plain deferral now (its stall watchdog was
                # demoted-diagnostic); chain liveness catches a genuinely wedged cluster.
                self.event("churn", f"refill: {v} not yet frontier-synced — deferring")
                return "skipped"
            self.chain.register_activate(s.next_joiner, 0, voter_idx=self._live_voter_idx(cur))
            s.refill_resumed = ""
            s.last_growth_kind = "refill"
            s.settle_until_epoch = cur + self.membership_settle
            s.refill_pending = str(s.next_joiner)
            s.refill_share_base = str(self.beacon_share_delta(s.next_joiner))
            s.refill_land_epoch = ""
            s.refill_over_ticks = 0
            self.event("churn", f"APPLIED refill {v} (frontier-synced spare replaces tombstone)")
            s.next_joiner += 1
            return "applied"

        # PROMOTE elif
        if promote_actionable == 1:
            return self._promote_fire(cur)

        # BENCH-JOIN / MINT elif — DISABLED (v61 a24). This OLD supply track minted identities
        # idx val_containers..pool as Pending and RE-TASKED rotation containers (validator-12/13) to
        # serve them. The chain-driven rotation model has no use for it: the warm bench is exactly
        # provision_warm_bench's set (idx validators..val_containers on their NATIVE containers), and
        # fresh keys come from rekey (val_containers..pool). Left running, it minted stray Pending
        # idx-14/15 and desynced container↔identity, so _pick_bench_for_evict boosted a Pending idx
        # that can never seat → the eviction stalled (bench_in=False). next_bench_joiner therefore
        # stays at its init (val_containers), so next_rekey_idx = max(val_containers, val_containers).
        # if (s.next_bench_joiner < self.max_mint_idx
        #         and actions.bench_join_due(s.round, s.last_bench_join_round, self.bench_join_gap)
        #         and setops.count(s.promotable) < self.mint_buffer
        #         and self.hp.promote_host_avail(s.next_bench_joiner)):
        #     return self._bench_join_mint(cur)

        return None

    def _promote_fire(self, cur):
        s = self.s
        cand = s.promote_cand
        if s.promote_cand_kind == "ready":
            s.promote_nohost_since_epoch = 0
            base = self.beacon_share_delta(cand)
            if base < 0 and int(cand) >= self.cfg.val_containers:
                base = 0
            if base < 0:
                self.event("churn", f"defer bench_promote {topology.validator(cand)} — share baseline "
                                    "unreadable at fire (-1) on a released-native container")
                return "skipped"
            self.chain.bench_promote(cand, voter_idx=self._live_voter_idx(cur))
            s.promote_pending = str(cand)
            s.promote_share_base = str(base)
            s.promote_land_epoch = ""
            s.promote_over_ticks = 0
            s.promotable = setops.set_remove(cand, s.promotable)
            self.warm_stint.pop(cand, None)
            s.settle_until_epoch = cur + self.membership_settle
            self.event("churn", f"APPLIED bench_promote {topology.validator(cand)} (committee cap-1->cap)")
            return "applied"
        # kind=warm: keep warming
        if self.hp.warm_identity(cand):
            s.promote_nohost_since_epoch = 0
            self._warm_stint_charge(cand)
            self.event("promote_deferred_warming",
                       f"promote candidate {topology.validator(cand)} HOSTED but not yet warm_ready")
        else:
            self.event("promote_deferred_no_host",
                       f"host for promote candidate {topology.validator(cand)} DISAPPEARED (in-round race)")
        return "skipped"

    def _bench_join_mint(self, cur):
        s = self.s
        idx = s.next_bench_joiner
        if idx < self.cfg.identity_pool:
            self.chain.register_only(idx)
            note = "pre-baked bench"
        else:
            rc = self.chain.mint_identity(idx, self.mint_eth_wei)
            if rc == 2:  # transient fund failure → DEFER (idx NOT consumed)
                if str(idx) == self.mint_defer_idx:
                    self.mint_defer_acc += 1
                else:
                    self.mint_defer_idx = str(idx)
                    self.mint_defer_acc = 1
                if self.mint_defer_acc >= 3:
                    fatal_or_diag(self.events, "bench", f"mint DEFERRED {self.mint_defer_acc} rounds "
                                  f"for {topology.validator(idx)} — persistent transient fund failure",
                                  seen=self._diag_seen, key=f"mint:{idx}")
                self.event("churn", f"mint DEFERRED for {topology.validator(idx)} "
                                    f"(attempt {self.mint_defer_acc}/3; idx NOT consumed)")
                return "skipped"
            self.mint_defer_idx = ""
            self.mint_defer_acc = 0
            note = "MINTED fresh idx (write-keys+fund)"
        s.promotable = f"{s.promotable} {idx}".strip()
        self.event("churn", f"APPLIED bench_join {topology.validator(idx)} ({note}; Pending)")
        self.hp.warm_identity(idx)
        s.next_bench_joiner += 1
        s.last_bench_join_round = s.round
        return "applied"

    # ── fault lottery (4-draw + calm + forced overrides + gate + apply) ───────
    def _fault_lottery(self, cur, n):
        s = self.s
        pool = self.cfg.actions_pool()
        vpool = setops.committee_victim_idxs(s.cur_committee, s.address.addr2idx, s.disrupted)
        d = draw_round(s, pool, vpool, cur, self.calm_permille)
        action, victim, is_calm, forced = d.action, d.victim, d.is_calm, d.forced

        if not victim:
            self.event("churn", "no eligible victim this round — SKIPPING the fault lottery")
            return "skipped"

        if is_calm == 1:
            self.event("churn", f"calm epoch {cur} — no churn")
            return "skipped"

        # gate inputs + gate_accept (the ≤f SAFETY gate — the load-bearing decision)
        nxt = self.chain.committee(cur + 1)
        gi = build_gate_inputs(s, action, victim, n, nxt, self.top_stake_leader(),
                               self.hp.add_source_avail() if self.cfg.voluntary_exit == 1 else 1)
        ok, reason = actions.gate_accept(gi)
        if not ok:
            self.event("churn", f"GATE SKIP {action} {victim} — {reason}")
            return "skipped"
        apply_action(s, self.act, action, victim, cur, chain=self.chain, event=self.event)
        return "applied"

    # ══ CHAIN-DRIVEN ROTATION PHASE (v61 a20) ═════════════════════════════════
    def _growth_complete(self, n):
        """Growth is done when the committee has reached the full cap AND every growth joiner has
        been issued and landed — the boundary between phase-1 (grow the cap) and phase-2 (rotate
        within it)."""
        return (n >= self.cfg.validators
                and self.s.next_joiner >= self.cfg.validators
                and self.s.grow_landed >= self.cfg.validators - 1)

    def _rotation_round(self, cur, n):
        """Phase-2 round: provision the warm bench once, hold on a fragile DKG window or an open
        rotation settle window, else run the (sacred 4-draw) lottery and apply a chain-driven
        departure. The chain re-selects the committee; process_rotations does the rekey."""
        s = self.s
        # ── PROVISIONING BELT (v61 a23): do ONE provisioning step per round and HOLD the lottery
        #    until the warm bench is fully stratified. The a22 all-at-once provisioning submitted 4
        #    bench-activate gov proposals (10 async votes each) + ~30 stratify txs in ONE round; under
        #    burn-mode sender load the votes/proposals never mined inside the voting window → gov
        #    Defeated / not-Active → the bench never provisioned. GROWTH proves one gov action per
        #    round is reliable (7→10 landed), so the belt paces provisioning the same way: one
        #    committee-member stratify or one bench activate per round, retried on a flaky gov defer.
        if not s.bench_provisioned:
            self.provision_warm_bench(cur)
            return "skipped"           # no departures until the warm bench is armed
        if self.rec.dkg_window_fragile == 1:
            self.event("churn", f"dkg-barrier HELD (epoch {cur}): incoming committee[{cur + 1}] "
                                f"not beacon-warm [{self.rec.dkg_barrier_notready.strip()}]")
            return "skipped"
        if cur < s.settle_until_epoch:
            self.event("churn", f"rotation settle-window (until epoch {s.settle_until_epoch}) — "
                                "one departure at a time; no churn")
            return "skipped"
        return self._rotation_lottery(cur, n)

    def provision_warm_bench(self, cur):
        """One-shot two-band STRATIFICATION (v61 a21): lift every SEATED committee member to a
        uniform Hi band and stand up the warm bench at a uniform Lo band, cleanly separated
        (bench_lo < committee_hi). This is what makes rotation CAUSE-DRIVEN: with the bench well
        below every incumbent, provisioning it displaces NOBODY (the a20 involuntary churn is gone)
        — only a drawn departure (undelegate / slash / evict-delegate) then moves the top-k boundary.

        Paced as a BELT — ONE step per round (v61 a23), advanced by provision_cursor, so no round
        floods the mempool with gov proposals (the a22 all-at-once storm lost its votes under load):
          - cursor in [0, validators): stratify committee member <cursor> — fund its owner BLEND to
            member_fund_wei, then SELF-delegate up to committee_hi_wei (self, not owner-0, so the
            сам-вышел path can later self-undelegate its own stake).
          - cursor in [validators, val_containers): activate warm-bench idx <cursor> to bench_lo_wei
            TOTAL (register 1e18 + delegate bench_lo-1e18), NO cap raise (growth-only).
          - cursor == val_containers: latch bench_provisioned + emit STRATIFIED.
        A step only advances the cursor on SUCCESS; a flaky-gov deferral holds the cursor and retries
        the SAME step next round (activate_bench is idempotent, so a retry of a Pending idx just
        re-votes the activate). fund_blend/self_delegate_to are deficit-based no-ops once satisfied."""
        s = self.s
        hi = int(self.cfg.committee_hi_wei)
        cur_idx = s.provision_cursor
        if cur_idx < self.cfg.validators:
            # one committee member → Hi band (self-delegated)
            try:
                self.chain.fund_blend(cur_idx, int(self.cfg.member_fund_wei))
                self.chain.self_delegate_to(cur_idx, hi)
                s.provision_cursor += 1
            except Exception as e:  # noqa: BLE001 — a deferral holds the cursor (retry next round)
                self.event("churn", f"committee-stratify {topology.validator(cur_idx)} deferred: {e}")
            return
        if cur_idx < self.cfg.val_containers:
            # one warm-bench idx → Lo band
            svc = topology.validator(cur_idx)
            if hasattr(self.act, "_run_ok"):
                self.act._run_ok(self.act._compose("up", "-d", "--no-deps", svc))
            bench_delegate = max(0, int(self.cfg.bench_lo_wei) - 10 ** 18)  # +1e18 register = bench_lo
            try:
                self.chain.activate_bench(cur_idx, bench_delegate, voter_idx=self._live_voter_idx(cur))
                self.event("churn", f"warm-bench provisioned {topology.validator(cur_idx)} "
                                    f"(Lo band {self.cfg.bench_lo_wei} < committee Hi {hi})")
                s.provision_cursor += 1
            except Exception as e:  # noqa: BLE001 — a deferral holds the cursor (retry next round)
                self.event("churn", f"warm-bench provision {topology.validator(cur_idx)} deferred: {e}")
            return
        s.bench_provisioned = 1
        self.event("churn", f"STRATIFIED: committee → Hi {hi}, bench → Lo "
                            f"{self.cfg.bench_lo_wei} (cause-driven rotation armed)")

    def _rotation_lottery(self, cur, n):
        """The phase-2 lottery: the SAME sacred 4-draw + calm/victim discipline as _fault_lottery,
        routed to the chain-driven rotation apply. Transient faults stay reversible (same identity
        rejoins); byzantine / voluntary / eviction are permanent (rekey)."""
        s = self.s
        pool = self.cfg.actions_pool()
        vpool = setops.committee_victim_idxs(s.cur_committee, s.address.addr2idx, s.disrupted)
        d = draw_round(s, pool, vpool, cur, self.calm_permille)
        action, victim, is_calm = d.action, d.victim, d.is_calm
        if not victim:
            self.event("churn", "no eligible victim this round — SKIPPING the rotation lottery")
            return "skipped"
        if is_calm == 1:
            self.event("churn", f"calm epoch {cur} — no churn")
            return "skipped"
        nxt = self.chain.committee(cur + 1)
        gi = build_gate_inputs(s, action, victim, n, nxt, self.top_stake_leader(), 1)
        ok, reason = actions.gate_accept(gi)
        if not ok:
            self.event("churn", f"GATE SKIP {action} {victim} — {reason}")
            return "skipped"
        self._rotation_apply(cur, action, victim)
        return "applied"

    def _rotation_apply(self, cur, action, victim):
        """Phase-2 apply: permanent departures issue a STAKE/STATUS op ONLY (the chain re-selects)
        and mark the container for rekey; transient faults reuse the reversible apply_action.
        NEVER calls register_activate / bench_promote / setActiveValidatorsLength."""
        s = self.s
        # EVER_FAULTED (the wrongful-slash expectation set) — stamped HERE for the two permanent
        # departures this method handles INLINE, mirroring orchestrator.apply_action:566. Only the
        # `else` arm below reaches apply_action, so without this the rotation phase equivocated a
        # member, the product correctly slashed+tombstoned it, and wrongful-slash then failed the
        # run because the identity was never in the ledger (bundle-20260730T071018Z). Keyed by
        # ident_idx(victim) — the IDENTITY, not the container (see EVER_FAULTED, case-soak.sh:368).
        # BEFORE the arms on purpose: voluntary_exit returns early on a chain-write deferral, and a
        # fault ATTEMPTED is still a fault the identity may be jailed for.
        # delegate_shift is deliberately absent — its victim is emergent, AND an out-ranked
        # incumbent is never faulted (it returns to the bench unslashed), so stamping it would hand
        # the detector a false expectation for an honest identity. Matches the bash action set
        # (case-soak.sh:2046-2048) and apply_action's own tuple.
        if action in ("byzantine_equivocate", "voluntary_exit") and victim:
            s.identity.ever_faulted[s.ident_idx(victim)] = action
        if action == "byzantine_equivocate":
            # docker equivocation → on-chain evidence → StakingDpos slash/evict (NOT halt-guarded).
            self.act.act_byzantine(victim, "equivocate")
            s.mark_disrupted(victim)
            s.container.disrupt_kind[victim] = action
            s.identity.permanently_dead[victim] = 1
            s.tombstones += 1
            self._mark_rekey(victim, cur, "byzantine")
            s.settle_until_epoch = cur + self.membership_settle
            self.event("churn", f"APPLIED byzantine_equivocate {victim} — equivocation → on-chain "
                                "slash/evict; chain re-selects a warm bench; rekey pending")
        elif action == "voluntary_exit":
            # сам вышел: drop OWN stake below the bench tier → chain drops it from top-k. NO
            # disableValidator (reversible), NO promotable re-queue (that was the a18 bounce bug).
            # Best-effort (an insufficient-stake revert must not abort a night-long run — the
            # rotation reconciler emits a diagnostic if the departure never lands).
            vidx = s.ident_idx(victim)
            if not self._rotation_chain_write(
                    lambda: self.chain.undelegate(vidx, self.cfg.voluntary_undelegate_wei),
                    f"voluntary_exit {victim} undelegate"):
                return
            s.container.disrupt_kind[victim] = action
            s.voluntary_exits += 1
            self._mark_rekey(victim, cur, "voluntary")
            s.settle_until_epoch = cur + self.membership_settle
            self.event("churn", f"APPLIED voluntary_exit {victim} — undelegated own stake; chain "
                                "drops it from the top-k; rekey pending")
        elif action == "delegate_shift":
            # вытеснен (v61 a22 FIX-3): EMERGENT victim. The committee is uniform Hi, so boosting a
            # bench ABOVE the tier drops whichever incumbent the chain ranks weakest — NOT the drawn
            # victim. So we do NOT pre-mark a rekey here; we boost a warm bench and SNAPSHOT the
            # committee. process_evictions later diffs the committee, finds the member that actually
            # left, undelegates its lingering stake, and rekeys ITS container.
            bench = self._pick_bench_for_evict(victim)
            if bench is None:
                self.event("churn", "stake-evict — no warm-bench key available; SKIPPING")
                return
            if not self._rotation_chain_write(
                    lambda: self.chain.stake_delegate(bench, self.cfg.evict_stake_wei),
                    f"stake_evict delegate->bench {bench}"):
                return
            snapshot = [a.lower() for a in s.cur_committee.split()]
            a2i = s.address.addr2idx
            s.evict_pending[str(bench)] = {
                "epoch": cur, "bench_idx": bench, "snapshot": snapshot,
                "addr_idx": {a: a2i.get(a) for a in snapshot},
            }
            s.settle_until_epoch = cur + self.membership_settle
            self.event("churn", f"APPLIED stake_evict (delegate {self.cfg.evict_stake_wei} → bench "
                                f"idx {bench}; chain drops the weakest incumbent; victim resolved "
                                "on landing)")
        else:
            # transient faults: reversible, same identity rejoins → NO rekey.
            apply_action(s, self.act, action, victim, cur, chain=self.chain, event=self.event)

    def _rotation_chain_write(self, fn, label):
        """Run a rotation-phase chain write best-effort: a revert/transient logs a diagnostic and
        returns False (the departure is abandoned this round, retried on a later draw) instead of
        aborting the run — liveness-first, and the rotation reconciler flags a departure that never
        lands anyway.

        ONLY ChainError is caught, matching its module-level twin orchestrator._apply_chain_write.
        A bare `except Exception` here (until 2026-07-30) swallowed genuine harness bugs —
        TypeError/AttributeError/KeyError from the call itself — as "chain-write deferred", i.e. a
        broken rotation path would log a churn line every round and the run would end green."""
        from ..chain.writes import ChainError
        try:
            fn()
            return True
        except ChainError as e:
            self.event("churn", f"rotation chain-write deferred ({label}): {e}")
            return False

    def _mark_rekey(self, container, cur, cause):
        """Mark a permanently-departed container for rekey. process_rotations rebirths it under a
        fresh pool key ONCE its identity has actually left the on-chain committee."""
        self.s.rekey_pending[container] = {"epoch": cur, "cause": cause,
                                           "idx": self.s.ident_idx(container)}

    def _pick_bench_for_evict(self, victim):
        """A warm-bench identity (spare/rotation band) that is ACTIVE, NOT the victim and NOT
        currently seated — the key to boost above the victim so the chain evicts the incumbent.
        The ACTIVE guard (v61 a24) is load-bearing: only an Active, staked, non-seated bench can be
        boosted INTO the top-k. Boosting a Pending/Jail candidate just piles stake onto an identity
        the selection ignores → the eviction never lands (bench_in=False stall)."""
        for idx in range(self.cfg.validators, self.cfg.val_containers):
            svc = topology.validator(idx)
            if svc == victim:
                continue
            served = self.s.ident_idx(svc)
            addr = self.chain.owner_addr(served)
            if not addr:
                continue
            if self.chain.committee_has(addr, self.s.cur_epoch):
                continue  # already seated → not a bench
            if self.chain.validator_status(addr) != "1":
                continue  # not Active (Pending/Jail/absent) → cannot be selected into the top-k
            return served
        return None

    # ── helpers ────────────────────────────────────────────────────────────────
    def _membership_clean(self, cur):
        for dv in self.s.disrupted.split():
            addr = self.chain.owner_addr(self.s.ident_idx(dv))
            if addr and self.chain.committee_has(addr, cur):
                return 0
        return 1

    def _live_voter_idx(self, cur):
        """sim_live_voter_idx: owner-idx of every CURRENT committee member (the stake-holders the
        stake-weighted quorum sums). Returns a space list, or None → the prefix path."""
        comm = set(self.chain.committee(cur).split())
        if not comm:
            return None
        out = []
        for i in range(self.s.max_minted_idx + 1):
            a = self.chain.owner_addr(i)
            if a and a.lower() in comm:
                out.append(i)
        return out or None


def _int(v, default):
    try:
        return int(v)
    except (TypeError, ValueError):
        return default
