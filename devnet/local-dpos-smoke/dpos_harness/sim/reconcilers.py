"""reconcilers.py — the per-tick reconcilers. Python port of the case-soak.sh docker-read
wiring AROUND the already-ported pure cores:

  * process_restores            (1191) — two-phase confirm-before-free restore state machine
  * process_tombstone_backfills (1314) — against-f FIFO free, seat-covered clear, slash-landed
                                          release, backfill stall watchdog
  * compute_effective_faults    (1960) — the EFFECTIVE_FAULTS union + warm-debt clear/bound
  * compute_dkg_barrier         (1748) — incoming-committee DKG-readiness barrier

The PURE decision cores live elsewhere and are reused verbatim:
  actions.counter_progress / warm_debt_step / dkg_member_ready / the seat mutators
  (enqueue/clear/release), battery.backfill_overdue / dkg_barrier_qual_gate.

Impure docker/RPC reads are SEAM methods on the Reconcilers object so a test drives the whole
state machine with synthetic inputs (mirroring the battery's seam-override discipline):
  finalized_dec(), node_finalized(v), recovery_confirmed(v), blockhash_of(v,h),
  beacon_share_delta(idx), beacon_seed_delta(idx), warm_debt_ready(c, chain_live).
A live Reconcilers binds them to nodes.py + the Chain; the dispatcher owns restart actuation.

Watchdog trips RAISE ChainError(reason_id, msg) (the fail_bundle translation) — the caller
writes a structured bundle. SOFT recover events are appended to `self.events`.

PORT-NOTE: the RUN #7/#8 errexit-`&&`-branch-tail gotcha and the empty-assoc set-u trap that
shaped this bash are gone structurally — explicit `if`, dict iteration over snapshotted keys.
"""

from __future__ import annotations

import os
import time

from . import actions
from ..checks import battery
from ..core import setops, topology
from ..core.setops import in_set as _in_set
from ..chain.writes import ChainError, fatal_or_diag, build_addr_map

# Pinned anchors NEVER rotated out (matches setops.committee_victim_idxs' voluntary/byzantine
# skip). ONE definition, in core/topology.py: the pinned host-RPC node + the cert-upstream
# anchor. Re-spelling them here is how the two lists drift apart.
ANCHOR_CONTAINERS = topology.ANCHOR_CONTAINERS


class Reconcilers:
    def __init__(self, state, chain, act, hostpool=None, events=None):
        self.s = state
        self.chain = chain
        self.act = act                 # actions.Actuators (restart actuators)
        self.hp = hostpool
        self.events = events if events is not None else []
        self._diag_seen = set()        # latch: one diag per (demoted-watchdog, entity) per run
        # rolling scalars the bash kept as globals
        self.effective_chain_prev = -1
        self.ever_members_count_prev = 0
        self.dkg_barrier_target = ""
        self.dkg_barrier_since_ts = 0
        self.dkg_barrier_over_ticks = 0
        # _dkg_barrier_verdict HEIGHT-geometry window pins (soak-invariants.sh:2231).
        self.dkg_barrier_fin0 = -1        # finalized when THIS hold window opened
        self.dkg_barrier_estart = -1      # pinned epoch_start(E_incoming) (-1 = no incoming geometry)
        self.dkg_barrier_verdict = "hold"
        self.dkg_barrier_verdict_msg = ""
        self.dkg_barrier_blocks_left = ""
        self.dkg_margin_blocks = int(os.environ.get("SIM_DKG_MARGIN_BLOCKS", "20"))
        self.epoch_interval = int(os.environ.get("SIM_EPOCH_INTERVAL",
                                                 os.environ.get("EPOCH_INTERVAL", "64")))
        self.dpos_activation_block = int(os.environ.get("DPOS_ACTIVATION_BLOCK", "0"))
        self._act_memo = None             # parent-shell memo of the on-chain activation
        # knobs
        self.recover_deadline = int(os.environ.get("SIM_RECOVER_DEADLINE_SECS", "300"))
        self.recover_lag = int(os.environ.get("SIM_RECOVER_LAG", "8"))
        self.warm_debt_max = int(os.environ.get("SIM_WARM_DEBT_MAX_EPOCHS", "12"))
        self.slash_land_margin = int(os.environ.get("SIM_SLASH_LAND_MARGIN", "3"))
        self.membership_settle = int(os.environ.get("SIM_MEMBERSHIP_SETTLE", "5"))
        # fresh-key TOP-UP floor (v61 a27): keep this many ready-to-consume pool identities minted
        # ahead of next_rekey_idx. A FIXED headroom (not proportional to cumulative usage) so the pool
        # stops growing once the drain rate settles. 1 suffices functionally (a byzantine rekey fires
        # ≤1 per settle window); 3 covers a transient mint-defer and sits inside the quick pool too.
        self.min_key_headroom = int(os.environ.get("SIM_MIN_KEY_HEADROOM", "3"))
        self.grow_land_deadline = int(os.environ.get("SIM_GROW_LAND_DEADLINE", "8"))
        self.dkg_window_fragile = 0
        self.dkg_barrier_notready = ""
        # SELF_PARTITIONED proxy (compute_self_partitioned, case-soak.sh:1906)
        self.self_partition_flat_ticks = int(
            os.environ.get("SIM_SELF_PARTITION_FLAT_TICKS", "4"))
        self.self_partition_max_epochs = int(
            os.environ.get("SIM_SELF_PARTITION_MAX_EPOCHS", "0"))
        # resource-pool headroom report (check_resource_pools, case-soak.sh:1575)
        self.pool_report_every = int(os.environ.get("SIM_POOL_REPORT_EVERY", "20"))
        self.pool_last_report = -1
        self.spare_exhausted_announced = 0

    # ── impure seams (overridable in tests) ───────────────────────────────────
    def finalized_dec(self):
        from ..core import nodes
        return nodes.finalized_dec()

    def node_finalized(self, v):
        from ..core import nodes
        return nodes.node_fin_in(v)

    def recovery_confirmed(self, v):
        """recovery_confirmed: own finalized caught within SIM_RECOVER_LAG of the v0 leader."""
        nh = self.node_finalized(v)
        if nh < 0:
            return False
        v0 = self.finalized_dec()
        if v0 <= 0:
            return False
        return nh + self.recover_lag >= v0

    def blockhash_of(self, v, h):
        from ..core import nodes
        return nodes.blockhash_of(v, h)

    # LIVE SEAMS — construction stubs, listed in orchestrator.LIVE_SEAMS and rebound by
    # Orchestrator._bind_live_seams. A new stub here must be added there too (enforced by
    # test_live_seams::test_live_seam_registry_covers_every_construction_stub).
    def beacon_share_delta(self, idx):
        return -1  # bound in the live wiring to sim-actions sim_beacon_share_delta

    def beacon_seed_delta(self, idx):
        return -1

    def warm_debt_ready(self, c, chain_live):
        """_warm_debt_ready: beacon_seed_active rose past the post-restart baseline AND frontier
        caught up on a LIVE chain. Recaptures WARM_DEBT_BASE on a counter reset (restart)."""
        idx = self.s.ident_idx(c)
        cur = self.beacon_seed_delta(idx)
        if cur < 0:
            return False
        base = self.s.container.warm_debt_base.get(c)
        if base is None or cur < base:
            self.s.container.warm_debt_base[c] = cur
            return False
        return cur > base and chain_live == 1 and self.recovery_confirmed(c)

    # ── warm_debt_add (a restart reloads a persisted share; hold against-f) ────
    def warm_debt_add(self, c):
        cs = self.s.container
        cs.warm_debt[c] = 1
        cs.warm_debt_base.pop(c, None)
        cs.warm_debt_since[c] = self.s.cur_epoch
        cs.warm_debt_over_ticks[c] = 0

    # ── process_restores ──────────────────────────────────────────────────────
    def process_restores(self, cur):
        c = self.s.container
        now = time.time()
        # Phase 1: ISSUE the restore once its epoch deadline passes (victim STAYS against-f).
        for v in list(c.restore_at):
            if v in c.recovering:
                continue
            if cur < c.restore_at[v]:
                continue
            if self.dkg_window_fragile == 1:
                continue  # mechanism A: defer ALL restores while the DKG window is fragile
            # Every surviving restore kind is committee-composition NEUTRAL (same identity, same
            # seat). The promote-serialization defer that used to sit here covered exactly one
            # composition-CHANGING restore — the liveness_jail un-jail (activateValidator) — which
            # was deleted with the action. Re-add it if a restore path ever issues a membership
            # write again; the DKG-fragility barrier above gates mesh churn, not a second
            # concurrent composition change.
            kind = c.disrupt_kind.get(v)
            if kind in ("graceful_stop_restart", "dkg_midwindow_restart"):
                self.act.act_graceful_start(v)
            elif kind == "sigkill_restart":
                self.act.act_sigkill_start(v)
            elif kind == "cpu_throttle":
                self.act.act_cpu_restore(v)
            elif kind == "byzantine_forge_pk":
                self.act.act_byzantine_restore(v)
            c.recovering[v] = now
            c.recover_over_ticks[v] = 0
            c.recover_leadfin[v] = self.finalized_dec()
            self.warm_debt_add(v)
            self.events.append(("recover", f"restore issued for {v} ({kind}); holding against-f"))
        # Phase 2: CONFIRM rejoin on a LIVE chain, THEN free the slot.
        for v in list(c.recovering):
            lf = self.finalized_dec()
            prev = c.recover_leadfin.get(v, 0)
            c.recover_leadfin[v] = lf
            chain_live = 1 if lf > prev else 0
            if chain_live == 1 and self.recovery_confirmed(v):
                stop_fin = c.stop_fin.get(v)
                if stop_fin and stop_fin > 0:
                    bh = self.blockhash_of(v, stop_fin)
                    if bh in ("null", "", None):
                        fatal_or_diag(
                            self.events, "graceful-stop-hole",
                            f"{v} rejoined but block {stop_fin} (its finalized at stop) is MISSING",
                            seen=self._diag_seen, key=v)
                if c.disrupt_kind.get(v) == "dkg_midwindow_restart" and self.s.shareless > 0:
                    self.s.shareless -= 1
                self.events.append(("recover", f"confirmed rejoin {v} — against-f slot freed"))
                self.s.unmark_disrupted(v)
                for m in (c.stop_fin, c.restore_at, c.disrupt_kind, c.recovering,
                          c.recover_over_ticks, c.recover_leadfin):
                    m.pop(v, None)
            elif now - c.recovering[v] > self.recover_deadline:
                c.recover_over_ticks[v] = c.recover_over_ticks.get(v, 0) + 1
                if c.recover_over_ticks[v] >= 2:
                    fatal_or_diag(self.events, "recover-stall",
                                  f"{v} did not rejoin within {self.recover_deadline}s",
                                  seen=self._diag_seen, key=v)
            else:
                c.recover_over_ticks[v] = 0

    # ── compute_effective_faults ──────────────────────────────────────────────
    def compute_effective_faults(self, cur):
        """The EFFECTIVE_FAULTS union = raw DISRUPTED ∪ warm-debted ∪ permdead-still-seated. Also
        clears warm-debt on the PROVEN warm signal (never EL-height, never on a frozen chain) and
        bounds it (fail-loud past SIM_WARM_DEBT_MAX_EPOCHS)."""
        c = self.s.container
        lf = self.finalized_dec()
        chain_live = 1 if self.effective_chain_prev >= 0 and lf > self.effective_chain_prev else 0
        for k in list(c.warm_debt):
            if self.warm_debt_ready(k, chain_live):
                for m in (c.warm_debt, c.warm_debt_base, c.warm_debt_since, c.warm_debt_over_ticks):
                    m.pop(k, None)
                self.events.append(("recover", f"warm-debt cleared {k} (proven beacon-warm)"))
                continue
            held = max(cur - c.warm_debt_since.get(k, cur), 0)
            verdict, over = actions.warm_debt_step(held, self.warm_debt_max,
                                                   c.warm_debt_over_ticks.get(k, 0))
            c.warm_debt_over_ticks[k] = over
            if verdict == "fail":
                fatal_or_diag(self.events, "warm-debt-stall",
                              f"{k} WARM-DEBTED {held} epochs (>= {self.warm_debt_max}) — "
                              "never re-warmed after restart", seen=self._diag_seen, key=k)
        # refresh SELF_PARTITIONED + SIM_BATTERY_COVERAGE (case-soak.sh:1987) BEFORE the union so
        # the fold below and the gate read this tick's proxy set.
        self.compute_self_partitioned(chain_live)
        self._self_partition_sweep(cur)
        if c.self_partitioned:
            self.events.append((
                "self_partition",
                f"SELF_PARTITIONED non-empty [{' '.join(c.self_partitioned)}] — a SEATED member's "
                "beacon_seed_active is flat while the chain is live (Bug-A signature); counted in "
                "EFFECTIVE_FAULTS so no new fault stacks on top"))
        # union (container-name domain), dedup'd
        parts = list(self.s.disrupted.split())
        for k in c.warm_debt:
            if k not in parts:
                parts.append(k)
        for dv in self._permdead_seated():
            if dv not in parts:
                parts.append(dv)
        for k in c.self_partitioned:            # F9: fold SELF_PARTITIONED into the union
            if k not in parts:
                parts.append(k)
        self.s.effective_faults = " ".join(parts)
        self.effective_chain_prev = lf if lf >= 0 else self.effective_chain_prev
        return self.s.effective_faults

    # ── compute_self_partitioned (case-soak.sh:1906) ──────────────────────────
    def compute_self_partitioned(self, chain_live):
        """No-progress proxy: a SEATED committee member whose beacon_seed_active stays flat for
        >= SELF_PARTITION_FLAT_TICKS LIVE-chain ticks is Bug-A self-partitioned. Also writes
        SIM_BATTERY_COVERAGE = the count of CURRENT-committee members that this tick BOTH resolved
        to a known container (ADDR2IDX) AND responded to their metrics probe (seed_delta >= 0), so a
        vacuous cross-node tick reads as 0 coverage (F9). Counter-reset safety: a BACKWARD step is a
        container RESTART (re-baseline + clear the streak), NOT a flat tick — folding them was the
        false-SELF_PARTITIONED bug."""
        c = self.s.container
        a2i = self.s.address.addr2idx
        self.s.battery_coverage = 0
        for a in self.s.cur_committee.split():
            cn = a2i.get(a.lower())
            if not cn:
                continue                        # committee addr does not resolve to a container
            idx = self.s.ident_idx(cn)
            cur_seed = self.beacon_seed_delta(idx)
            if cur_seed < 0:
                continue                        # unreachable read → not responding (or scrape blip)
            self.s.battery_coverage += 1        # resolved AND responding this tick
            if chain_live == 0:
                continue                        # frozen chain → cannot distinguish partition
            prev = c.seed_active_prev.get(cn)
            c.seed_active_prev[cn] = cur_seed
            st = actions.counter_progress(cur_seed, prev)
            if st == "flat":
                c.seed_flat_ticks[cn] = c.seed_flat_ticks.get(cn, 0) + 1
            elif st in ("restart", "progress"):
                c.seed_flat_ticks[cn] = 0
                for m in (c.self_partitioned, c.self_partition_since, c.self_partition_over_ticks):
                    m.pop(cn, None)
            elif st == "baseline":
                c.seed_flat_ticks[cn] = 0
            if c.seed_flat_ticks.get(cn, 0) >= self.self_partition_flat_ticks:
                c.self_partitioned[cn] = 1
                if cn not in c.self_partition_since:
                    c.self_partition_since[cn] = self.s.cur_epoch

    def _self_partition_sweep(self, cur):
        """BOUNDED deadline sweep (case-soak.sh:1994). SIM_SELF_PARTITION_MAX_EPOCHS=0 (default)
        leaves it warn-only (the visible-cost ladder in dispatch owns that path); >0 re-arms a
        2-tick-belt fail for a genuinely stuck member."""
        c = self.s.container
        if self.self_partition_max_epochs <= 0:
            return
        for k in list(c.self_partitioned):
            held = max(cur - c.self_partition_since.get(k, cur), 0)
            verdict, over = actions.warm_debt_step(held, self.self_partition_max_epochs,
                                                   c.self_partition_over_ticks.get(k, 0))
            c.self_partition_over_ticks[k] = over
            if verdict == "fail":
                fatal_or_diag(
                    self.events, "self-partition-stall",
                    f"{k} SELF-PARTITIONED for {held} epochs (>= "
                    f"{self.self_partition_max_epochs}): beacon_seed_active flat while the chain "
                    "advanced — seated but contributing nothing (Bug-A p2p-block signature)",
                    seen=self._diag_seen, key=k)

    # ── reconcile_inflight_membership (case-soak.sh:1646) ─────────────────────
    def reconcile_inflight_membership(self, cur, n_now):
        """Per-tick reconciler for in-flight membership state, reachable regardless of which
        dispatcher branch last fired. Drops a STUCK REFILL_RESUMED when the resume is no longer
        meaningful — the spare ACTIVATED (seated) or the seat is no longer needed (chain's own
        top-K auto-refilled, n >= cap) — which otherwise latches forever (its setting elif needs
        n < cap to re-enter) and silently suppresses ALL fault churn (settle-window guard),
        pins GA_CHANGE_PENDING, and starves the tombstone-backfill-stall watchdog. NEXT_JOINER is
        NOT advanced (the spare is conserved for a future genuine hole)."""
        s = self.s
        if not s.refill_resumed:
            return
        ridx = s.refill_resumed.rsplit("-", 1)[-1]
        why = ""
        if n_now >= self.s.cfg.validators:
            why = (f"committee is back at cap ({n_now} >= {self.s.cfg.validators}) — the chain's "
                   "own top-K auto-refilled the seat, so the resumed spare is no longer needed")
        elif self.chain.committee_has(self.chain.owner_addr(ridx), cur):
            why = (f"the resumed spare {topology.validator(ridx)} is SEATED in committee[{cur}] — its refill "
                   "has landed")
        if why:
            s.refill_resumed = ""
            self.events.append((
                "reconcile",
                f"in-flight RECONCILE: dropped stuck REFILL_RESUMED={topology.validator(ridx)} — {why}. "
                "Unblocks fault churn, un-pins GA_CHANGE_PENDING, re-arms the backfill-stall "
                "watchdog. The spare is conserved (NEXT_JOINER unchanged)"))

    # ── check_resource_pools (case-soak.sh:1575) ──────────────────────────────
    def check_resource_pools(self, cur):
        """Finite-pool headroom report + LOUD, honestly-attributed exhaustion (never a silent
        fall-through). Announces the spare-band exhaustion REGIME CHANGE once; fail_bundles
        (ChainError test-resource-exhausted) only on identity exhaustion WITH unmet demand (an open
        seat obligation and no promotable left); emits a POOLS headroom line every
        SIM_POOL_REPORT_EVERY epochs."""
        s = self.s
        cfg = s.cfg
        id_left = self._identity_supply_left()
        spare_left = self._spare_supply_left()
        promo_n = setops.count(s.promotable)
        pb_open = 1 if s.seat.pending_backfill_by_seat else 0

        if spare_left == 0 and self.spare_exhausted_announced == 0:
            self.spare_exhausted_announced = 1
            self.events.append((
                "warn",
                f"RESOURCE REGIME CHANGE @epoch {cur}: the SPARE band is exhausted "
                f"(NEXT_JOINER={s.next_joiner} == SIM_SPARE_END={cfg.spare_end}). The refill "
                "self-heal path is now PERMANENTLY DEAD — every future tombstone must be backfilled "
                "by the PROMOTE track alone. Raise SIM_SPARES to extend the refill regime"))

        if id_left <= 0 and promo_n == 0 and pb_open == 1:
            raise ChainError(
                "test-resource-exhausted",
                f"TEST RESOURCE EXHAUSTED — NOT A PRODUCT FAILURE. The harness identity pool is "
                f"empty (NEXT_BENCH_JOINER={s.next_bench_joiner} reached SIM_MAX_MINT_IDX="
                f"{self._max_mint_idx()}), PROMOTABLE is empty, and a seat obligation is OPEN with "
                "nothing left to fill it. The chain is NOT at fault — raise SIM_MAX_MINT (indices "
                "burn permanently, so a bigger pool is the only remedy)")

        if self.pool_last_report < 0 or cur - self.pool_last_report >= self.pool_report_every:
            self.pool_last_report = cur
            self.events.append((
                "resource_report",
                f"POOLS @epoch {cur} — identities: {id_left} left (cursor {s.next_bench_joiner}/"
                f"{self._max_mint_idx()}, PROMOTABLE={promo_n} banked) | spares: {spare_left}/"
                f"{cfg.spares} left (refill {'DEAD' if spare_left == 0 else 'live'}) | burn so far: "
                f"{s.tombstones} tombstones + {s.voluntary_exits} exits over {s.round} rounds"))
        return

    def _max_mint_idx(self):
        return int(os.environ.get("SIM_MAX_MINT_IDX", str(self.s.cfg.identity_pool - 1)))

    def _identity_supply_left(self):
        """_identity_supply_left: mint cursor headroom (SIM_MAX_MINT_IDX − NEXT_BENCH_JOINER)."""
        return self._max_mint_idx() - self.s.next_bench_joiner

    def _spare_supply_left(self):
        """_spare_supply_left: unconsumed spares (SIM_SPARE_END − NEXT_JOINER, floored at 0)."""
        return max(self.s.cfg.spare_end - self.s.next_joiner, 0)

    def _permdead_seated(self):
        """_permdead_seated: PERMANENTLY_DEAD tombstones STILL seated in committee[cur] (mapped
        addr→container via ADDR2IDX) — still an effective fault until the E+2 slash lands."""
        cc = set()
        a2i = self.s.address.addr2idx
        for a in self.s.cur_committee.split():
            cn = a2i.get(a.lower())
            if cn:
                cc.add(cn)
        return [dv for dv in self.s.identity.permanently_dead if dv in cc]

    # ── compute_dkg_barrier ───────────────────────────────────────────────────
    def compute_dkg_barrier(self, cur):
        """The incoming-committee DKG-readiness barrier. Sets self.dkg_window_fragile (1 iff an
        imminent committee[cur+1] change has some member lacking a fresh share, OR any current
        member is warm-debted). A held barrier defers ALL churn; a non-hold verdict is a genuine
        stuck-DKG fail ONLY when getDkgQual is unreadable (dkg_barrier_qual_gate)."""
        nxt = self.chain.committee(cur + 1)
        imminent = 1 if (nxt and nxt != self.s.cur_committee) else 0
        wd = 1 if self.s.container.warm_debt else 0
        if imminent == 0 and wd == 0:
            self.dkg_window_fragile = 0
            self.dkg_barrier_notready = ""
            self.dkg_barrier_since_ts = 0
            self.dkg_barrier_over_ticks = 0
            self.dkg_barrier_target = ""
            self.s.identity.dkg_barrier_base.clear()
            return 0
        self.dkg_window_fragile = 0
        self.dkg_barrier_notready = ""
        base = self.s.identity.dkg_barrier_base
        if imminent == 1:
            incoming = []
            all_mapped = 1
            a2i = self.s.address.addr2idx
            for a in nxt.split():
                cn = a2i.get(a.lower())
                if cn:
                    incoming.append(str(self.s.ident_idx(cn)))
                else:
                    all_mapped = 0
            if nxt != self.dkg_barrier_target:
                self.dkg_barrier_target = nxt
                base.clear()
                self.dkg_barrier_since_ts = 0
                self.dkg_barrier_over_ticks = 0
            for m in incoming:
                if m in base:
                    continue
                d = self.beacon_share_delta(m)
                if d >= 0:
                    base[m] = d
            # fragility: any incoming member not dkg_member_ready
            notready = []
            for m in incoming:
                ready, action = actions.dkg_member_ready(base.get(m), self.beacon_share_delta(m))
                if action == "rebaseline":
                    base[m] = self.beacon_share_delta(m)
                if not ready:
                    notready.append(m)
            if notready:
                self.dkg_window_fragile = 1
                self.dkg_barrier_notready = " ".join(notready) + " "
            if all_mapped == 0:
                self.dkg_window_fragile = 1
                self.dkg_barrier_notready += "<unmapped-incoming-member> "
        else:
            self.dkg_barrier_target = ""
            base.clear()
        if wd == 1:
            self.dkg_window_fragile = 1
            self.dkg_barrier_notready += "<warm-debt> "
        if self.dkg_window_fragile == 1:
            if self.dkg_barrier_since_ts == 0:
                self.dkg_barrier_since_ts = time.time()
                self._dkg_barrier_window_reset()
            # HEIGHT-gated no-hang bound (soak-invariants.sh:_dkg_barrier_verdict): the product's
            # DKG deadline is a HEIGHT (ceremony seals at epoch_start(E_incoming) − margin), so the
            # harness expiry must be too — never a wall-clock backstop. A non-hold verdict is a
            # genuine stuck-DKG FAIL only when getDkgQual is UNREADABLE (dkg_barrier_qual_gate);
            # a deferral / late-qualification EXPLAINS an un-warm crossing (qualify-before-commit).
            fin = self.finalized_dec()
            act = self._dkg_barrier_act()
            verdict = self._dkg_barrier_verdict(fin, cur, act, self.epoch_interval, imminent)
            if verdict != "hold":
                qs = self._dkg_qual_state(cur + 1)
                gate, kind = battery.dkg_barrier_qual_gate(verdict, qs)
                if gate == "pass":
                    self.dkg_window_fragile = 0
                    self.dkg_barrier_notready = ""
                    self.dkg_barrier_since_ts = 0
                    self.dkg_barrier_over_ticks = 0
                    self._dkg_barrier_window_reset()
                else:
                    self.dkg_barrier_over_ticks += 1
                    if self.dkg_barrier_over_ticks >= 2:
                        fatal_or_diag(
                            self.events, "dkg-barrier-stuck",
                            f"incoming committee[{cur + 1}] members [{self.dkg_barrier_notready.strip()}]"
                            f" never finalized a fresh committee-epoch DKG and getDkgQual({cur + 1}) is"
                            f" UNREADABLE: {self.dkg_barrier_verdict_msg}",
                            seen=self._diag_seen, key=self.dkg_barrier_notready.strip())
            else:
                self.dkg_barrier_over_ticks = 0
        else:
            self.dkg_barrier_since_ts = 0
            self.dkg_barrier_over_ticks = 0
            self._dkg_barrier_window_reset()
        return self.dkg_window_fragile

    def _dkg_barrier_window_reset(self):
        self.dkg_barrier_fin0 = -1
        self.dkg_barrier_estart = -1
        self.dkg_barrier_verdict = "hold"
        self.dkg_barrier_verdict_msg = ""
        self.dkg_barrier_blocks_left = ""

    def _dkg_barrier_act(self):
        """_dkg_barrier_act: PARENT-shell memoized on-chain getDposActivationBlock (immutable once
        set — read once, retried while unresolved), falling back to the exported
        DPOS_ACTIVATION_BLOCK. NEVER an ambient _PP_ACT_CACHE (that lives only inside a subshell)."""
        if self._act_memo is None:
            a = self.chain._pp_cfg_read_retry("getDposActivationBlock()(uint64)")
            if a is not None and a > 0:
                self._act_memo = a
        return self._act_memo if self._act_memo is not None else self.dpos_activation_block

    def _dkg_barrier_verdict(self, fin, cur, act, interval, imm):
        """_dkg_barrier_verdict (soak-invariants.sh:2273): hold | boundary | cap. First measurable
        call (fin>0) after a reset pins fin0 and — iff imminent — epoch_start(cur+1)=act+(cur+1)*
        interval. cap = hold outlived 3 full intervals of BLOCKS; boundary = fin crossed
        epoch_start with the DKG still un-warm; fin<=0 (RPC blip) is unmeasurable → hold."""
        self.dkg_barrier_verdict = "hold"
        self.dkg_barrier_verdict_msg = ""
        self.dkg_barrier_blocks_left = ""
        if fin <= 0:
            return "hold"
        if self.dkg_barrier_fin0 < 0:
            self.dkg_barrier_fin0 = fin
            self.dkg_barrier_estart = -1
            if imm == 1 and act > 0 and interval > 0:
                self.dkg_barrier_estart = act + (cur + 1) * interval
        if interval > 0 and fin - self.dkg_barrier_fin0 >= 3 * interval:
            self.dkg_barrier_verdict = "cap"
            self.dkg_barrier_verdict_msg = (
                f"barrier held while finalized advanced {fin - self.dkg_barrier_fin0} blocks "
                f"(>= 3 x interval={interval}) past the hold-open height {self.dkg_barrier_fin0}")
            return "cap"
        if self.dkg_barrier_estart > 0 and fin >= self.dkg_barrier_estart:
            self.dkg_barrier_verdict = "boundary"
            self.dkg_barrier_verdict_msg = (
                f"finalized {fin} crossed epoch_start(E_incoming)={self.dkg_barrier_estart} "
                f"(seal trigger was {self.dkg_barrier_estart - self.dkg_margin_blocks}) — the "
                "incoming epoch began without its DKG")
            return "boundary"
        if self.dkg_barrier_estart > 0:
            self.dkg_barrier_blocks_left = self.dkg_barrier_estart - self.dkg_margin_blocks - fin
        return "hold"

    def _dkg_qual_state(self, epoch):
        tok = self.chain.p.run(
            ["cast", "call", self.chain.staking_rt, "getDkgQual(uint64)(bool)", str(epoch),
             "--rpc-url", self.chain.rpc], note="dkg-qual")
        t = "".join((tok or "").split())
        return 1 if t == "true" else (0 if t == "false" else -1)

    # ── process_tombstone_backfills ───────────────────────────────────────────
    def process_tombstone_backfills(self, cur):
        """Against-f FIFO free on a new committee face, the seat-covered clear, the slash-landed
        against-f release, and the backfill-stall watchdog (battery.backfill_overdue)."""
        s = self.s
        for a in s.cur_committee.split():
            s.address.ever_members[a.lower()] = 1
        ever_count = len(s.address.ever_members)
        new_faces = ever_count - self.ever_members_count_prev
        self.ever_members_count_prev = ever_count

        seat_of_container = {c: k for k, c in s.seat.seat_container.items()}
        # against-f FREE (FIFO by enqueue epoch) over PERMANENTLY_DEAD ∩ DISRUPTED.
        order = []
        for v in s.identity.permanently_dead:
            if not _in_set(v, s.disrupted):
                continue
            oseat = seat_of_container.get(v)
            stamp = s.seat.pending_backfill_by_seat.get(oseat, 0) if oseat is not None else 0
            order.append((stamp, v))
        for _stamp, v in sorted(order, key=lambda t: t[0]):
            if new_faces > 0:
                new_faces -= 1
                fseat = seat_of_container.get(v)
                if fseat is not None:
                    actions.release_covered_seat(s, fseat)
                else:
                    s.unmark_disrupted(v)
                self.events.append(("recover", f"tombstone backfill confirmed — {v} freed"))

        # #3 SLASH-DIDN'T-LAND early-fail.
        for sv in list(s.identity.tombstone_settle_epoch):
            if cur <= s.identity.tombstone_settle_epoch[sv] + self.slash_land_margin:
                continue
            svaddr = self.chain.owner_addr(sv)
            if not svaddr:
                continue        # unreadable this tick — retry; never discharge on a failed read
            if self.chain.committee_has(svaddr, cur):
                fatal_or_diag(self.events, "byzantine-slash-not-landed",
                              f"byzantine slash did not remove identity {sv} within settle+margin",
                              seen=self._diag_seen, key=sv)
                # KEEP TRACKING (2026-07-30). This used to pop unconditionally, "so the demoted
                # watchdog does not re-diag every tick" — but fatal_or_diag's (id, key) latch
                # ALREADY bounds that to one line per identity per run, so the pop bought nothing
                # and cost everything: the condition became unobservable after its first tick, so
                # a slash landing LATE looked identical to one never landing, and the live
                # 2026-07-22 diag ("did not remove identity 14") can no longer be re-checked
                # against a later committee read. This detector found a real contract bug once
                # (EvidenceDecoderNotConfigured, 2026-07-02); the condition must stay re-evaluated
                # every tick until the slash actually lands, whatever its demotion status.
                continue
            # LANDED: the identity really is out of the committee — the obligation is discharged.
            s.identity.tombstone_settle_epoch.pop(sv, None)

        # SEAT-COVERED clear (full cap AND victim gone ⇒ seat re-covered on-chain).
        pb = s.seat.pending_backfill_by_seat
        if pb:
            cap = self.chain.active_validators_length() or int(os.environ.get("SIM_VALIDATORS", "14"))
            csz = len(s.cur_committee.split())
            if csz >= cap:
                for v in list(pb):
                    vaddr = self.chain.owner_addr(v)
                    if not vaddr:
                        continue
                    if self.chain.committee_has(vaddr, cur):
                        continue
                    actions.release_covered_seat(s, v)
                    self.events.append(("recover", f"seat-covered clear — identity {v} left"))

        # TOMBSTONE against-f RELEASE ON SLASH (independent of cap).
        for td in list(s.identity.permanently_dead):
            if not _in_set(td, s.disrupted):
                continue
            tdaddr = self.chain.owner_addr(self.s.ident_idx(td))
            if not tdaddr:
                continue
            if self.chain.committee_has(tdaddr, cur):
                continue
            s.unmark_disrupted(td)
            self.events.append(("recover", f"tombstone {td}'s slash landed — against-f slot released"))

        # backfill stall watchdog — DIAGNOSTIC ONLY (liveness-first policy). A tombstone whose seat
        # is not explicitly backfilled within the epoch budget records a diag (the seat/committee
        # bookkeeping routinely lags the chain's top-K during growth ramps and settle windows, so
        # this is not a run-failing condition — chain liveness is). fatal_or_diag never raises for
        # a demoted id. See core.policy.DEMOTED_INVARIANTS.
        for v in list(pb):
            overdue, budget = battery.backfill_overdue(cur, pb[v], 0,
                                                       self.membership_settle,
                                                       self.grow_land_deadline)
            if overdue:
                fatal_or_diag(self.events, "tombstone-backfill-stall",
                              f"seat vacated by identity {v} never had its backfill land "
                              f"within {budget} epochs", seen=self._diag_seen, key=v)

    # ── chain-driven ROTATION reconciler (v61 a20) ────────────────────────────
    def _rotation_voter_idx(self, cur):
        """Owner-idxs of the CURRENT committee members — the gov voter set for a rekey's
        activate_bench (mirrors dispatch._live_voter_idx). None → gov's 0..need-1 prefix."""
        comm = set(self.chain.committee(cur).split())
        if not comm:
            return None
        out = []
        for i in range(self.s.max_minted_idx + 1):
            a = self.chain.owner_addr(i)
            if a and a.lower() in comm:
                out.append(i)
        return out or None

    def _rekey_container(self, container, cur) -> bool:
        """Rebirth <container> under a FRESH pool identity and re-stake it LOW so it rejoins the warm
        bench ('old container, new key'). Returns True when the container is HANDLED (rekeyed or the
        fresh-key supply is exhausted → drop it from the queue), False when the caller should retry
        next tick (a rebirth that did not leave the container running). Rebuilds ADDR2IDX after the
        re-task so the fresh identity's owner-addr → served-idx mapping is LIVE (v61 a22 FIX-2:
        retask_slot alone never refreshed the map, so a rekeyed identity was invisible to
        committee_victim_idxs / detectors when it later entered the committee)."""
        s = self.s
        cfg = s.cfg
        fresh = s.next_rekey_idx
        if fresh >= cfg.identity_pool:
            self.events.append(("churn", f"rekey {container}: fresh-key supply exhausted "
                                         f"(idx {fresh} >= pool {cfg.identity_pool}); idle"))
            return True
        reborn = self.hp.rebirth_identity(container, fresh) if self.hp is not None else True
        if not reborn:
            self.events.append(("churn", f"rekey {container} -> idx {fresh}: rebirth resume "
                                         "did not leave the container running; retry next tick"))
            return False
        if self.hp is not None:
            self.hp.retask_slot(container, fresh)
        try:
            # rejoin at the SAME Lo band as the provisioned bench (bench_lo total = 1e18 register +
            # this delegate) so a rekeyed identity is a peer of the warm bench, not an outlier.
            self.chain.activate_bench(fresh, max(0, int(cfg.bench_lo_wei) - 10 ** 18),
                                      voter_idx=self._rotation_voter_idx(cur))
        except Exception as e:  # noqa: BLE001 — a deferral must not abort the run
            self.events.append(("churn", f"rekey {container} -> idx {fresh}: activate_bench "
                                         f"deferred: {e}"))
        try:
            build_addr_map(s, self.chain)            # FIX-2: the fresh idx joins ADDR2IDX now
        except Exception as e:  # noqa: BLE001
            self.events.append(("churn", f"rekey {container}: addr-map rebuild deferred: {e}"))
        s.next_rekey_idx += 1
        s.identity.permanently_dead.pop(container, None)
        s.container.disrupt_kind.pop(container, None)
        s.unmark_disrupted(container)
        self.events.append(("churn", f"REKEYED {container} under fresh identity {fresh} "
                                     "(old container, new key) → rejoined the warm bench"))
        return True

    def _return_to_bench(self, container, idx, cur) -> bool:
        """Return a VOLUNTARILY-departed identity to the warm bench under its OWN key. It self-
        undelegated (not slashed / not tombstoned), so it can legitimately come back — like an
        operator that paused and rejoins. Re-delegate it up to the Lo band; the container keeps
        serving the SAME idx (no rebirth, NO fresh pool key consumed). This is the whole reason the
        fresh-key supply drains only on byzantine (permanent) departures: voluntary rotations recycle
        in place. Returns True when handled (stake lifted), False to retry next tick on a deferral."""
        s = self.s
        try:
            # Re-fund to the member floor FIRST. A voluntary undelegate parks the stake in the staking
            # contract's UNBONDING queue, NOT the spendable balance (verified on-chain: a returned
            # member reads stake 5e18 + balance 10e18, the undelegated 46e18 nowhere spendable). Left
            # as-is, the ~10e18 belt leftover is far too little to re-boost the member to Hi when the
            # chain later re-promotes it — committee-Hi maintain would defer on ERC20InsufficientBalance
            # and the committee would carry a Lo member (band collapse → involuntary displacement is
            # back). Top up to member_fund_wei so a future 45e18 Hi-boost is always affordable, decoupled
            # from the unbonding timer (deficit-based: an already-funded member, e.g. via returned
            # unbonding, is a no-op).
            self.chain.fund_blend(idx, int(s.cfg.member_fund_wei))
            self.chain.self_delegate_to(idx, int(s.cfg.bench_lo_wei))
        except Exception as e:  # noqa: BLE001 — a transient revert just retries next tick
            self.events.append(("churn", f"return {container} (idx {idx}) to warm bench "
                                         f"deferred: {e}"))
            return False
        s.container.disrupt_kind.pop(container, None)
        s.unmark_disrupted(container)
        self.events.append(("churn", f"RETURNED {container} (identity {idx}) to the warm bench under "
                                     "its own key (voluntary exit — re-funded, not slashed, no fresh key)"))
        return True

    def replenish_key_supply(self, cur):
        """Lazy fresh-key TOP-UP (the user's '>50% used' idea): with voluntary exits recycling in
        place, the fresh-key band drains ONLY on byzantine (permanent) departures — but a long run
        could still spend it. When more than half the band is spent, mint ONE new pool identity per
        tick on demand (write-keys → fund ETH+BLEND → register→Pending) and extend the pool, so
        rotation never idles on 'fresh-key exhausted'. Belt-paced (one mint/tick) to avoid a mempool
        storm. A newly-minted idx is a bench KEY, NOT a running peer — it never enters --peers (it
        rides an existing container's network slot when a byzantine rekey later retasks into it), so
        this does NOT inflate the peer count the way a large pre-baked pool would. bench_provisioned-
        gated (the supply only matters once rotation is armed)."""
        s = self.s
        if not s.bench_provisioned:
            return
        # Maintain a FIXED available-headroom of ready-to-consume keys (v61 a27 FIX), NOT a reserve
        # proportional to cumulative usage. The old `spent*2 >= band` test compared cumulative spent
        # (next_rekey_idx - val_containers) against a band that itself grows by 1 every mint, so the
        # maintained reserve tracked cumulative spent → identity_pool grew UNBOUNDED (observed 20→33).
        # available = keys minted but not yet consumed; belt-mint ONE/tick until it reaches the floor,
        # then hold. Each byzantine rekey drops it by 1 → exactly one more mint. Fixed → pool stops
        # growing once the drain rate settles (and with eviction recycling, only byzantine drains).
        available = s.cfg.identity_pool - s.next_rekey_idx
        if available >= self.min_key_headroom:
            return
        j = s.cfg.identity_pool                     # next idx beyond the current pool
        try:
            rc = self.chain.mint_identity(j, int(os.environ.get("SIM_MINT_ETH_WEI",
                                                                 "10000000000000000")))
        except Exception as e:  # noqa: BLE001 — mint deferral just retries next tick
            self.events.append(("churn", f"key-supply replenish mint idx {j} deferred: {e}"))
            return
        if rc != 0:
            return                                  # transient fund defer (rc 2): idx NOT consumed
        s.cfg.identity_pool += 1
        s.max_minted_idx = s.cfg.identity_pool - 1
        self.events.append(("churn", f"KEY-SUPPLY +1: minted pool identity {j} (pool → "
                                     f"{s.cfg.identity_pool}); byzantine-drain runway extended"))

    def process_rotations(self, cur):
        """process_rotations (v61 a20): the chain-driven rotation reconciler for the DRAWN-VICTIM
        departures (byzantine slash / voluntary undelegate). Each marked its container in
        REKEY_PENDING; the chain re-selects the stake-weighted top-k on the stake/status change
        (~E+3). Once the departed identity has actually LEFT the on-chain committee, rekey its
        container under a fresh pool key. rotation-stall is DIAGNOSTIC-ONLY (never escalates). No-op
        off-phase (the map is only populated in the rotation phase)."""
        s = self.s
        if not s.rekey_pending:
            return
        cfg = s.cfg
        for container in list(s.rekey_pending):
            info = s.rekey_pending[container]
            addr = self.chain.owner_addr(info["idx"])
            if addr and self.chain.committee_has(addr, cur):
                # departure has not re-selected out yet.
                if cur - info["epoch"] > cfg.rotation_confirm_epochs:
                    if info["cause"] == "byzantine" and not info.get("force_removed"):
                        # STARVED EQUIVOCATOR (v61 a27): a byzantine victim only equivocates on a
                        # PROPOSAL SLOT; if it never won one within the horizon the slasher saw no
                        # evidence and never jailed it, so it sits seated forever and the rekey wedges
                        # in rekey_pending. Force the terminal roster removal via governance so the
                        # rekey can proceed. removeValidator preempts the (never-arriving) equivocation
                        # — tradeoff: the slasher path isn't exercised for THIS victim, but most
                        # byzantine win a slot and land naturally; this only rescues the rare wedge.
                        # Latched (force_removed) so the gov round issues exactly once.
                        try:
                            self.chain.remove_validator(int(info["idx"]),
                                                        voter_idx=self._rotation_voter_idx(cur))
                            info["force_removed"] = cur
                            self.events.append(("churn",
                                f"byzantine {container} (idx {info['idx']}) never got a proposal slot "
                                f"to equivocate within {cfg.rotation_confirm_epochs} epochs — FORCED "
                                "removeValidator"))
                        except Exception as e:  # noqa: BLE001 — best-effort gov write
                            self.events.append(("churn", f"forced removeValidator idx {info['idx']} "
                                                         f"deferred: {e}"))
                    elif info.get("force_removed"):
                        if cur - info["force_removed"] > cfg.rotation_confirm_epochs:
                            fatal_or_diag(self.events, "rotation-stall",
                                          f"byzantine identity {info['idx']} still seated even after "
                                          f"forced removeValidator @epoch {info['force_removed']}",
                                          seen=self._diag_seen,
                                          key=f"rotstall:{container}:{info['epoch']}")
                    else:
                        fatal_or_diag(self.events, "rotation-stall",
                                      f"{info['cause']} departure of identity {info['idx']} (container "
                                      f"{container}) has not left the committee within "
                                      f"{cfg.rotation_confirm_epochs} epochs",
                                      seen=self._diag_seen,
                                      key=f"rotstall:{container}:{info['epoch']}")
                continue
            # departed identity is OUT of the committee. BYZANTINE = slashed/tombstoned → it must NOT
            # return under the same key → rekey the container under a FRESH pool key (consumes supply).
            # VOLUNTARY = not malicious, only self-undelegated → RETURN the SAME identity to the warm
            # bench under its own key (an operator that paused rejoins); NO fresh key consumed. So the
            # fresh-key supply drains ONLY on byzantine, which is why a small pool + lazy top-up lasts.
            handled = (self._rekey_container(container, cur)
                       if info["cause"] == "byzantine"
                       else self._return_to_bench(container, info["idx"], cur))
            if handled:
                del s.rekey_pending[container]

    def process_evictions(self, cur):
        """process_evictions (v61 a22 FIX-3): the EMERGENT-victim reconciler for stake-eviction
        ('вытеснен'). Unlike byzantine/voluntary, eviction does not target a drawn victim — it boosts
        a warm bench ABOVE the (uniform) committee tier, and the chain drops whichever incumbent is
        weakest. So the victim is discovered HERE, by diffing the committee against the apply-time
        snapshot: once the boosted bench has ENTERED and a snapshot member has LEFT, that departed
        member is the one evicted → undelegate its (still-Hi) stake so it cannot bounce back, and
        rekey ITS container. Diagnostic-only rotation-stall if nothing moves past the horizon."""
        s = self.s
        if not s.evict_pending:
            return
        cfg = s.cfg
        for key in list(s.evict_pending):
            info = s.evict_pending[key]
            bench_addr = self.chain.owner_addr(info["bench_idx"])
            bench_in = bool(bench_addr) and self.chain.committee_has(bench_addr, cur)
            cur_comm = {a.lower() for a in self.chain.committee(cur).split()}
            departed = [a for a in info["snapshot"] if a not in cur_comm]
            if not (bench_in and departed):
                if cur - info["epoch"] > cfg.rotation_confirm_epochs:
                    fatal_or_diag(self.events, "rotation-stall",
                                  f"stake-eviction via bench idx {info['bench_idx']} did not evict a "
                                  f"committee member within {cfg.rotation_confirm_epochs} epochs "
                                  f"(bench_in={bench_in}, departed={len(departed)})",
                                  seen=self._diag_seen,
                                  key=f"evictstall:{key}:{info['epoch']}")
                continue
            for victim_addr in departed:
                # ADDR2IDX maps an owner-addr → the CONTAINER serving it (build_addr_map); the
                # evicted member's identity idx is what that container serves.
                container = info["addr_idx"].get(victim_addr)
                if not container:
                    continue
                vidx = s.ident_idx(container)
                if container in ANCHOR_CONTAINERS:
                    # An anchor must NEVER be rekeyed — its container hosts the pinned host-RPC and the
                    # spec-derive telemetry the invariants sample; a rebirth (kind=Restart) makes it
                    # rederive-100% and blips the RPC. The anchor-stake band should keep the chain from
                    # ever dropping it, but if a boundary race still does, RE-SEAT it under its own key
                    # (re-fund + self-delegate back above the eviction reach) — no container restart.
                    try:
                        self.chain.fund_blend(vidx, int(cfg.anchor_fund_wei))
                        self.chain.self_delegate_to(vidx, int(cfg.anchor_stake_wei))
                    except Exception as e:  # noqa: BLE001
                        self.events.append(("churn", f"anchor {container} eviction re-seat deferred: {e}"))
                    self.events.append(("churn", f"anchor {container} was evicted — RE-SEATED under its "
                                                 "own key (NOT rekeyed; host-RPC/spec-derive preserved)"))
                    continue
                # drop the evicted member's lingering Hi stake so it cannot immediately re-select back
                # in, THEN RECYCLE it under its OWN key (v61 a27). A stake-eviction victim is NOT
                # malicious — it was merely out-ranked by a boosted bench, never slashed/jailed, and
                # its Hi stake is its own self-delegated stake (status stays Active). So it can return
                # exactly like a voluntary exit: no fresh key, no container rebirth (the anchor branch
                # above already recycles this way — this makes the whole eviction path uniform). Only
                # byzantine (jail+tombstone) still consumes the fresh-key supply.
                try:
                    self.chain.undelegate(vidx, cfg.voluntary_undelegate_wei)
                except Exception as e:  # noqa: BLE001 — best-effort; the return still proceeds
                    self.events.append(("churn", f"stake-evict cleanup undelegate idx {vidx} "
                                                 f"deferred: {e}"))
                self.events.append(("churn", f"stake-evict LANDED: identity {vidx} (container "
                                             f"{container}) pushed out of the top-k by bench idx "
                                             f"{info['bench_idx']}; returning to bench"))
                self._return_to_bench(container, vidx, cur)
            del s.evict_pending[key]

    def maintain_committee_hi(self, cur):
        """maintain_committee_hi (v61 a24): keep every SEATED committee member at the Hi band. When a
        departure lands, the chain promotes a warm bench that entered at the Lo band (bench_lo);
        left there, the committee accumulates Lo members and the committee↔bench stake gap collapses,
        so a newly provisioned/rekeyed Lo bench TIES with a Lo incumbent → involuntary displacement
        returns and eviction's 'weakest incumbent' stops being a clean Hi member. Each tick, SELF-
        delegate any below-Hi seated member UP to committee_hi_wei (deficit-based/idempotent, member's
        own key — so the сам-вышел path can still self-undelegate). A just-promoted Lo bench is thus
        lifted to Hi on the next tick and the committee stays uniform Hi with the bench cleanly below.

        Gated on bench_provisioned, NOT merely rotation_phase (v61 a24 FIX): before the belt has
        STRATIFIED the two bands it OWNS every seated stake — it funds each genesis member's 0-balance
        owner (fund_blend) and self-delegates it to Hi, cursor by cursor. Running here first only spams
        0-balance ERC20InsufficientBalance defers for members the belt has not reached yet; the belt
        stratifies committee (cursor 0..validators) BEFORE any bench, so no inversion window exists to
        race. Post-STRATIFIED every seatable identity is minted-funded, so a below-Hi seated member is
        always a just-promoted Lo bench that HAS balance to self-stake. Best-effort (a deferral retries)."""
        s = self.s
        if not s.bench_provisioned:
            return
        hi = int(s.cfg.committee_hi_wei)
        anchor_stake = int(s.cfg.anchor_stake_wei)
        a2i = s.address.addr2idx
        for addr in self.chain.committee(cur).split():
            container = a2i.get(addr)
            if not container:
                continue
            if container in s.rekey_pending:
                # a DEPARTING victim (byzantine/voluntary) that undelegated its own stake BELOW the
                # tier on purpose and is awaiting the chain to drop it from the top-k. Re-boosting it
                # back to Hi would re-delegate the very stake the departure just freed and CANCEL the
                # rotation → the identity never leaves → process_rotations stalls forever. Leave it.
                continue
            idx = s.ident_idx(container)
            # ANCHORS (validator-0/1) sit ABOVE the eviction reach (anchor_stake > bench_lo+evict_stake)
            # so the emergent-victim eviction can never make one the weakest incumbent → never drops /
            # rekeys / restarts the host-RPC + spec-derive node. They need funding to clear the higher
            # band (validator-1 is belt-funded only to member_fund_wei; validator-0 is owner-0, self-
            # funded → fund_blend is a no-op deficit there).
            if container in ANCHOR_CONTAINERS:
                target, floor = anchor_stake, int(s.cfg.anchor_fund_wei)
            else:
                target, floor = hi, None
            try:
                if self.chain.validator_stake(addr) < target:
                    if floor is not None:
                        self.chain.fund_blend(idx, floor)
                    self.chain.self_delegate_to(idx, target)
            except Exception as e:  # noqa: BLE001 — a deferral just retries next tick
                self.events.append(("churn", f"committee-Hi maintain {container} (idx {idx}) "
                                             f"deferred: {e}"))
