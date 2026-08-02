"""hostpool.py — the rebirth / host-pool machinery (WRITE side). Python port of the case-soak.sh
container-lifecycle helpers: _retask_slot, _recyclable_containers, sim_warm_identity,
sim_warm_ready, _promote_host_avail, sim_add_source_avail, and the soak-actions.sh docker
choreography sim_rebirth_identity / sim_recycle_container + the beacon-metric readers.

These mutate the domain-typed SimState (container/identity/seat maps) AROUND the already-ported
PURE cores in actions.py (slot_reusable / spare_reserved_for_refill / select_promote_candidate).
The container aliasing defect class (three proven multi-hour failures) is killed structurally: a
re-task ALWAYS severs the SEAT_CONTAINER bridge (_retask_slot), so a seat can never reach a
re-tasked container's new occupant.

Impure seams (docker/RPC) are injected via the HostPool constructor so tests drive them without
a live topology:
  * chain    — chain.writes.Chain (committee_has / owner_addr reads, on the RUNTIME cluster);
  * runner   — proc.Runner (the docker stop/run/up writes + overlay files);
  * confirm  — recovery_confirmed(container) seam (own-finalized caught up to leader);
  * beacon   — beacon_share_delta(idx) seam (dkg_ceremony_ok counter, -1 if unreachable).

PORT-NOTE: bash's `${!SLOT_IDENTITY[@]}` (empty-safe under set-u) and the `if sim_warm_ready`
form (dodging the &&-branch-tail errexit gotcha) are gone — dict iteration + explicit returns.
"""

from __future__ import annotations

import os

from . import actions
from ..core import topology
from ..core.setops import in_set as _in_set


class HostPool:
    def __init__(self, state, chain, runner, confirm=None, beacon=None):
        self.s = state
        self.chain = chain
        self.p = runner
        # LIVE SEAMS — construction stubs. Both are listed in orchestrator.LIVE_SEAMS and MUST be
        # rebound by Orchestrator._bind_live_seams; adding a third here without adding it there
        # fails test_live_seams::test_live_seam_registry_covers_every_construction_stub.
        self._confirm = confirm or (lambda c: False)
        self._beacon = beacon or (lambda idx: -1)

    # -- helpers ----------------------------------------------------------------
    def _committee_seated(self, ident_idx) -> bool:
        addr = self.chain.owner_addr(ident_idx)
        return self.chain.committee_has(addr, self.s.cur_epoch)

    # -- _retask_slot: the SINGLE writer of SLOT_IDENTITY + the bridge sever ----
    def retask_slot(self, container: str, idx):
        """_retask_slot: re-task <container> to serve identity <idx> and SEVER every
        SEAT_CONTAINER bridge still naming it (the aliasing guard, at the re-task itself so it is
        structural not per-path)."""
        seat = self.s.seat.seat_container
        for k in [s for s, c in seat.items() if c == container]:
            del seat[k]
        # NEW-1: store the served identity as a STRING (the production identity-idx domain), so
        # ident_idx / EVER_FAULTED / OWNER_ADDR_CACHE all agree for a reborn container.
        self.s.container.slot_identity[container] = str(idx)

    # -- _recyclable_containers -------------------------------------------------
    def recyclable_containers(self) -> list:
        """_recyclable_containers: sorted tombstoned containers whose slash has LANDED (left the
        committee) and which are no longer against-f (not in DISRUPTED). The exact PERMANENTLY_DEAD
        keyset (any band) — keeps producer/detector domains identical. NO backfill-obligation pin
        (a tombstone never rejoins)."""
        out = []
        for c in sorted(self.s.identity.permanently_dead):
            if _in_set(c, self.s.disrupted):
                continue
            if self._committee_seated(self.s.ident_idx(c)):
                continue
            out.append(c)
        return out

    # -- sim_recycle_container (busy->idle) ------------------------------------
    def recycle_container(self, c: str) -> bool:
        """sim_recycle_container: drop the byz overlay, clear the container-keyed tracking
        (PERMANENTLY_DEAD / DISRUPT_KIND / TOMBSTONE_SETTLE_EPOCH + the against-f slot), and
        SEVER every SEAT_CONTAINER bridge naming $c. The obligation itself is NOT touched (keyed
        by the burned identity).

        NO recreate here, deliberately — but it is a DEPENDENCY, not an omission. Removing the
        overlay file alone does not make a RUNNING container honest; only a recreate does. The
        recreate is warm_identity's very next step: this is called from ONE place
        (warm_identity, HOST SET (ii)) and is always followed immediately by rebirth_identity,
        whose `up -d --no-deps --force-recreate` rebuilds the container from the ambient
        COMPOSE_FILE — which never carried the byz overlay in the first place (act_byzantine
        merges it into a LOCAL env dict for one call, actions.py:533-536, and never mutates
        os.environ or Runner.env). So the reborn container comes back honest.
        If a second caller is ever added that does NOT rebirth, it must issue the bare
        `up -d --no-deps --force-recreate` itself (Actuators.act_byzantine_restore is that
        exact call) — otherwise the container keeps running byzantine with its overlay deleted,
        which is the worst of both (no file left on disk to show why)."""
        self.p.run_ok(["rm", "-f", f"docker-compose.sim.byz-{c}.gen.yml"], note="recycle-rm-byz")
        self.s.identity.permanently_dead.pop(c, None)
        self.s.container.disrupt_kind.pop(c, None)
        self.s.identity.tombstone_settle_epoch.pop(c, None)
        self.s.unmark_disrupted(c)
        seat = self.s.seat.seat_container
        for k in [s for s, host in seat.items() if host == c]:
            del seat[k]
        return True

    # -- sim_rebirth_identity (the docker choreography) ------------------------
    def rebirth_identity(self, slot: str, idx) -> bool:
        """sim_rebirth_identity: stop the slot → wipe ONLY the identity-keyed beacon share dir
        (a one-off genesis-init) → write the PERSISTENT IDENTITY_IDX overlay → append it to BOTH the
        ambient (os.environ) and the Runner COMPOSE_FILE (idempotent, PATH-style), so every later
        compose call in either env dict sees it → up --no-deps --force-recreate. EL/reth data is
        identity-agnostic and never wiped. Returns the `up` result: a resume that does NOT leave
        the slot running must NOT be recorded as a successful host (else a false-HOSTED
        slot_identity entry wedges the promote track forever — v61 a13)."""
        n = slot.rsplit("-", 1)[-1]
        overlay = f"docker-compose.sim.reborn-{slot}.gen.yml"
        self.p.run_ok(["docker", "compose", "stop", "--timeout", "40", slot], note="rebirth-stop")
        self.p.run_ok(["docker", "compose", "run", "--rm", "--no-deps", "-T",
                       "--entrypoint", "/bin/sh", topology.GENESIS_INIT,
                       "-c", f"rm -rf /runtime/reth-data/v{n}/beacon"], note="rebirth-wipe-beacon")
        self.p.write_file(
            overlay,
            f"services:\n  {slot}:\n    environment:\n      IDENTITY_IDX: \"{idx}\"\n",
            note="rebirth-overlay")
        # PATH-append the persistent overlay to the AMBIENT COMPOSE_FILE *and* the Runner's,
        # exactly as bringup._set_compose (bringup.py:167-168) sets both — one process-global
        # value, so writes and reads stay in lockstep.
        #
        # A Runner-env-only append was a LIVE BUG: Actuators and Runner hold two SEPARATE env
        # dicts (orchestrator.py:711 / :760), and act_byzantine deliberately rebuilds its compose
        # list from os.environ (actions.py:682) because Actuators.env is a pre-bring-up snapshot.
        # So a byzantine action on a REBORN slot ran `up --force-recreate` WITHOUT the IDENTITY_IDX
        # overlay: keydir fell back to the NATIVE index (compose_gen.py:345-346), the container came
        # back as an identity that was already burned and removed, the intended identity went silent
        # in the committee, nothing was slashable → byzantine-slash-not-landed. Bash predicted this
        # verbatim and fixed it with a real `export` (soak-actions.sh:644-656).
        #
        # Base ambient-first (`or`, not `.get(k, fallback)`) for actions.py:682's reason: Runner.env
        # is seeded with the empty startup sentinel at Orchestrator.__init__, which is PRESENT-but-
        # empty, so a defaulted read yields "" and would publish an overlay-ONLY COMPOSE_FILE —
        # dropping the base sim files from BOTH envs and making every later compose call a no-op.
        # Assigning both unconditionally (not only on append) re-syncs a Runner.env that still holds
        # that sentinel.
        #
        # Nothing removes the overlay from COMPOSE_FILE afterwards, by design: the file is rewritten
        # in place by a re-rebirth, and it is only unlinked by _sweep_generated_overlays
        # (orchestrator.py:641), which runs AFTER the teardown `down` (orchestrator.py:1065-1067) —
        # bash-identical ordering (case-soak.sh:514-519), so no compose call ever names a file that
        # is already gone.
        cf = os.environ.get("COMPOSE_FILE") or self.p.env.get("COMPOSE_FILE", "")
        if f":{overlay}:" not in f":{cf}:":
            cf = f"{cf}:{overlay}" if cf else overlay
        os.environ["COMPOSE_FILE"] = cf
        self.p.env["COMPOSE_FILE"] = cf
        ok = self.p.run_ok(["docker", "compose", "up", "-d", "--no-deps", "--force-recreate", slot],
                           note="rebirth-up")
        return ok

    # -- sim_warm_identity (host resolution + rebirth) -------------------------
    def warm_identity(self, idx) -> bool:
        """sim_warm_identity: ensure logical identity <idx> is hosted by a live container. Native
        (idx < val_containers) → served by its own container (no-op). An already-hosted bench idx
        → no-op. Else host on (i) a REUSABLE spare/rotation slot, then (ii) a RECYCLABLE tombstoned
        container. Returns True only if a host is now actually running for <idx>; False if no host
        is available OR the chosen host's resume `up` failed (never records a false-HOSTED slot)."""
        cfg = self.s.cfg
        if int(idx) < cfg.val_containers:
            return True
        if int(idx) in [int(v) for v in self.s.container.slot_identity.values()]:
            return True
        # HOST SET (i): reusable spare/rotation slot.
        for snum in range(cfg.validators, cfg.val_containers):
            cand = topology.validator(snum)
            if actions.spare_reserved_for_refill(snum, self.s.next_joiner, cfg.spare_end):
                continue
            if _in_set(cand, self.s.disrupted):
                continue
            if cand in self.s.identity.permanently_dead:
                continue
            served = self.s.container.slot_identity.get(cand, snum)
            seated = self._committee_seated(served)
            if not actions.slot_reusable(served, snum, seated, self.s.promotable,
                                         self.s.promote_pending, self.s.refill_pending):
                continue
            if not self.rebirth_identity(cand, idx):
                continue  # resume did not leave the slot running — do NOT false-HOST it; try next
            self.retask_slot(cand, idx)
            self._rebuild_addr_map()
            return True
        # HOST SET (ii): recyclable tombstoned container.
        for cand in self.recyclable_containers():
            if not self.recycle_container(cand):
                continue
            if not self.rebirth_identity(cand, idx):
                continue  # resume did not leave the slot running — do NOT false-HOST it; try next
            self.retask_slot(cand, idx)
            self._rebuild_addr_map()
            return True
        return False

    def _rebuild_addr_map(self):
        """build_addr_map after a re-task (case-soak.sh:1087/1098): a rebirth changed which slot
        serves an identity, so the addr→container map must reflect the new hosting THIS tick."""
        from ..chain.writes import build_addr_map
        build_addr_map(self.s, self.chain)

    # -- sim_warm_ready (promote fires only when this is true) -----------------
    def warm_ready(self, idx) -> bool:
        """sim_warm_ready: 0 iff idx's host slot is EL-frontier caught up (recovery_confirmed)
        AND its beacon/metrics plane is serving (share_delta >= 0 — not the -1 of a cold-booting
        container). Both gates: never seat a cold-syncer that lands late and churns a future DKG."""
        slot = self._resolve_slot(idx)
        if slot is None:
            return False
        if not self._confirm(slot):
            return False
        return self._beacon(idx) >= 0

    def _resolve_slot(self, idx):
        if int(idx) < self.s.cfg.val_containers:
            return topology.validator(idx)
        for host, served in self.s.container.slot_identity.items():
            if int(served) == int(idx):
                return host
        return None

    # -- _promote_host_avail (no side effects; mirrors warm_identity host set) --
    def promote_host_avail(self, idx) -> bool:
        """_promote_host_avail: True iff identity <idx> can be hosted RIGHT NOW — native/released
        (own live container), an already-hosting slot, a REUSABLE spare/rotation slot, or a
        RECYCLABLE tombstoned container. NO side effects (a gate/selector input must not mutate)."""
        cfg = self.s.cfg
        if int(idx) < cfg.val_containers:
            return True
        if int(idx) in [int(v) for v in self.s.container.slot_identity.values()]:
            return True
        for snum in range(cfg.validators, cfg.val_containers):
            cand = topology.validator(snum)
            if actions.spare_reserved_for_refill(snum, self.s.next_joiner, cfg.spare_end):
                continue
            if _in_set(cand, self.s.disrupted):
                continue
            if cand in self.s.identity.permanently_dead:
                continue
            served = self.s.container.slot_identity.get(cand, snum)
            seated = self._committee_seated(served)
            if actions.slot_reusable(served, snum, seated, self.s.promotable,
                                     self.s.promote_pending, self.s.refill_pending):
                return True
        return bool(self.recyclable_containers())

    # -- sim_add_source_avail (rule-7 GA_ADD_SOURCE_AVAIL) ---------------------
    def add_source_avail(self) -> int:
        """sim_add_source_avail: 1 iff PROMOTABLE has at least one hostable identity."""
        for idx in (self.s.promotable or "").split():
            if self.promote_host_avail(idx):
                return 1
        return 0

    # -- select_promote_candidate (reuses the ported pure selector) -------------
    def select_promote_candidate(self):
        """select_promote_candidate: FIRST warm_ready wins (fire this round), else first hostable
        (warm toward next round). Delegates to actions.select_promote_candidate with these live
        docker seams."""
        return actions.select_promote_candidate(
            self.s.promotable, warm_ready=self.warm_ready, host_avail=self.promote_host_avail)
