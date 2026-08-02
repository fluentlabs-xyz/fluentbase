"""asserts_byzantine_vrf.py — `scripts/case-byzantine-vrf.sh`, the one case with per-run state.

A Byzantine boundary proposer asserts a FORGED PK_E in `OrderBlock.beacon_outcome` at a
change-epoch first block. The honest share-holders must reject it at the "C" gate — their real
shares do not lie on the forged poly — so it cannot notarize, never reaches certify, and the forged
key never finalizes (SAFETY), while an honest leader crosses the boundary with the real key
(LIVENESS).

WHY A SINGLE BYZANTINE, not a quorum: under the realistic N3f1 bound (n=5 ⇒ f=1, quorum 4) a forge
reaches the Stage-2 certify hook only if ≥4 nodes notarize it — but flagging four leaves too few
honest share-holders to both reject the forge AND propose the real boundary, so liveness would
stall. There is no single-run split that shows BOTH certify-Nullify and liveness at f=1. The
certify-hook Nullify path is proven by the gated UNIT tests in `beacon/{outcome,certify}.rs`; this
docker case proves the realistic-fault end-to-end safety plus liveness.

═══ WHY THIS CASE HAS A `ByzantineDrive` OBJECT AND THE OTHER FOUR DO NOT ═════════════════

It is the only one of the five whose BRING-UP and ASSERTION share state. The bring-up must send
five extra deployer-funded transfers (the byzantine owner's BLEND, the toggle delegator's gas and
BLEND, and BLEND for the three floor-bumped owners), and the assertion then spends them — so the
toggle delegator's derived key has to survive from one to the other. `prod.run`'s `post_manifest`
seam takes a callable, and a bound method of this object is that callable.

The transfers CANNOT move earlier. `case-byzantine-vrf.sh:190-197` is emphatic: sending them before
`DeployStaking` would advance the deployer nonce and shift DeployStaking's CREATE addresses off the
prediction baked into `staking-reader.json`, so the node would read ChainConfig at the wrong address
and the `--dpos` cold start would fail. The deployer's PRE-deploy tx sequence must stay byte-for-byte
the production-path one — which is exactly what routing this case through the shared
`RotationBringUp` guarantees, rather than merely documents.

═══ WHY REPEATED COMMITTEE FLIPS ══════════════════════════════════════════════════════════

The forge fires in `build_proposal` ONLY when the byzantine node LEADS a change-epoch first block.
On such a block the prior cert carries no seed, so leader election takes the view-1 fallback
`weighted_cdf(stake, H(LEADER_DOMAIN ‖ fallback_seed ‖ view))` — i.e. the byzantine's lead
probability equals its committee STAKE SHARE. It is boosted to ~72%, so ONE boundary fires the
forge ~72% of the time and the case still drives a SEQUENCE for the residual margin: over B
boundaries P(never leads) ≈ (1−0.72)^B, and B=5 puts it at ≈0.17%.

Each flip is WAITED OUT before the next is issued, and that is a correctness fix rather than
politeness. The warmup is ASYMMETRIC — a delegate is effective in snapshot[e+2], an undelegate in
snapshot[e+1] — so an IN at epoch e and an OUT at e+1 write the SAME snapshot epoch and CANCEL:
`committee[*]` never changes, zero change-epoch boundaries surface, and the forge gets zero draws.
"""

from __future__ import annotations

import os
import time

from . import verdicts_rotation as VR
from .asserts_prod import (JOINER_IDX, PROD_NODES, _ok, _say, assert_honest_beacon_window,
                           assert_still_finalizing, owner_addrs, register_joiner,
                           wait_nodes_have)
from ...core import topology
from ...stack.production_path import DEFAULT_MNEMONIC

#: `case-byzantine-vrf.sh:331` — the validators whose 1e18 self-stake is lifted so the joiner is
#: the CLEAN swing member. v0 is already 5e18 (the rotation-proof sequencer) and the byzantine is
#: boosted separately, so the loop skips it.
FLOOR_BUMP_IDXS = (1, 2, 3, 4)


class ByzantineDrive:
    """Per-run state shared between the bring-up hook and the assertion — see the module header.

    `byz_idx` is `BYZ_VRF_IDX` (default 2). It must be a node that STAYS in every committee (so it
    leads boundary views and holds a real share to forge from) and must NOT be validator-0, the
    host-RPC node kept honest for greps.
    """

    def __init__(self, byz_idx=None, max_toggles=None, budget_min=None):
        self.byz_idx = int(byz_idx if byz_idx is not None
                           else os.environ.get("BYZ_VRF_IDX", VR.DEFAULT_BYZ_IDX))
        self.max_toggles = int(max_toggles if max_toggles is not None
                               else os.environ.get("BYZ_VRF_MAX_TOGGLES",
                                                   VR.DEFAULT_MAX_TOGGLES))
        self.budget_min = int(budget_min if budget_min is not None
                              else os.environ.get("BYZ_VRF_BUDGET_MIN", VR.DEFAULT_BUDGET_MIN))
        self.toggle_key = ""
        self.toggle_addr = ""

    @property
    def service(self) -> str:
        return topology.validator(self.byz_idx)

    @property
    def honest_nodes(self):
        """`:108-110` — every deriving node EXCEPT the byzantine one. Its own reth may diverge
        while it churns forged boundary views, so including it would fail the cross-node compare
        for the very behaviour the case is provoking. What must agree is the HONEST set."""
        return VR.honest_nodes(PROD_NODES, self.service)

    # -- the bring-up hook ---------------------------------------------------------
    def post_manifest(self, bu) -> None:
        """`:235-250` — the five extra DEPLOYER-funded transfers, sent ONLY after DeployStaking.

        Issued through `bu` rather than a ctx because they are part of the BRING-UP: they run
        inside `RotationBringUp.run`, before the first governance write, exactly where bash has
        them. The two `cast wallet` reads that derive the toggle delegator are here too — bash
        computes them a few lines earlier (`:198`), which is a READ-position difference and cannot
        move a nonce.
        """
        mnem = os.environ.get("FLUENT_DPOS_MNEMONIC", DEFAULT_MNEMONIC)
        self.toggle_key = bu.p.run(["cast", "wallet", "private-key", "--mnemonic", mnem,
                                    "--mnemonic-index", VR.TOGGLE_MNEMONIC_INDEX],
                                   note="toggle-key")
        self.toggle_addr = bu.p.run(["cast", "wallet", "address", "--mnemonic", mnem,
                                     "--mnemonic-index", VR.TOGGLE_MNEMONIC_INDEX],
                                    note="toggle-addr")
        if bu.dry:
            self.toggle_key = self.toggle_key or "0x" + "77" * 32
            self.toggle_addr = self.toggle_addr or "0x" + "77" * 20

        chain = bu.chain
        # The byzantine owner's BLEND — it self-delegates the rotation-proof boost later.
        chain.token_transfer(bu.token, chain.owner_addr(self.byz_idx), VR.BYZ_BLEND_WEI,
                             checked=True)
        # The toggle delegator's gas, then its BLEND (2e18 × 16 in-flips plus headroom, because
        # `undelegate` parks BLEND in a non-respendable queue rather than returning it).
        if not bu.p.run_capture(["cast", "send", self.toggle_addr, "--value", VR.TOGGLE_ETH_WEI,
                                 "--rpc-url", bu.rpc, "--private-key", bu.deployer_key],
                                note="fund-toggle-delegator").ok and not bu.dry:
            bu._fail("fund v5-toggle delegator")
        chain.token_transfer(bu.token, self.toggle_addr, VR.TOGGLE_BLEND_WEI, checked=True)
        # The floor-bumped owners hold genesis ETH but no RUNTIME BLEND, and they self-delegate
        # from their OWN keys below — so they need funding here, post-DeployStaking and nonce-safe.
        for i in FLOOR_BUMP_IDXS:
            if i == self.byz_idx:
                continue
            chain.token_transfer(bu.token, chain.owner_addr(i), VR.FLOOR_BLEND_WEI, checked=True)

    # -- the assertion -------------------------------------------------------------
    def assertion(self, ctx) -> None:
        """`:299-604` — stake setup, the toggle drive, then SAFETY + LIVENESS."""
        addrs = owner_addrs(ctx)
        byz_addr = addrs[self.byz_idx]
        joiner = addrs[JOINER_IDX]

        _stake_setup(ctx, self, addrs)

        # `:347-348` — E0 / GOT0 are read BEFORE the registration sends and are NOT asserted here:
        # this case never claims the committee starts as the initial five (its stake setup has
        # already changed the weights). They are the baseline the first observed CHANGE is measured
        # against.
        e0 = ctx.current_epoch()
        got0 = ctx.committee(e0)
        reg_floor = register_joiner(ctx, addrs, delegate=False)
        ctx.check(bool(ctx.wait_converge(VR.REG_CONVERGE_S, reg_floor)),
                  "nodes lost alignment during registration",
                  on_fail=lambda: ctx.dump_logs(VR.LOG_TAIL_NODE, self.service))

        # `:379-380` — ONE generous allowance from the dedicated delegator, so no toggle needs a
        # re-approve. The delegator is deliberately NOT the joiner's own owner: an `undelegate`
        # from the owner key would trip `OwnerSelfStakeBelowMinimum`.
        ctx.cast_send_checked([ctx.token, "approve(address,uint256)(bool)", ctx.staking_rt,
                               VR.TOGGLE_BLEND_WEI],
                              note="approve-toggle-delegator", key=self.toggle_key)

        state = _drive_flips(ctx, self, e0, got0, joiner, byz_addr)
        ctx.check(*VR.evaluate_forged(state["forged"], self.byz_idx, state["changes"]),
                  on_fail=lambda: _dump_forge_failure(ctx, self))
        _say(ctx, f"validator-{self.byz_idx} forged a PK_E at a change-epoch boundary "
                  f"x{state['forged']} (across {state['changes']} observed committee changes)")

        # ── the SAFETY window, anchored on the epoch the byzantine ACTUALLY forged at ──
        #
        # Its forge `warn!` carries a structured `epoch=<E>`. The LAST one is used: the most recent
        # forged boundary is the one certain to be finalised by the time the window is asserted.
        # The ANSI strip happens in the reader — without it the regex never matches, and in bash
        # the unguarded `$( … | grep … )` then exited non-zero and aborted the case with no
        # diagnostic at all.
        forge_epoch = VR.forge_epoch(
            ctx.logs_all(self.service, dry_value=f"{VR.FORGE_LINE} epoch=4"),
            fallback=state["e_new"])
        ctx.check(*VR.evaluate_forge_epoch(forge_epoch))
        forge_epoch = int(forge_epoch) if str(forge_epoch).isdigit() else 0
        _say(ctx, f"forged boundary epoch = {forge_epoch} (anchoring the SAFETY window there)")

        # The beacon has no on-chain mirror, so the safety proof is the OBSERVABLE output: at and
        # just past the forged boundary every honest node derives a non-zero, byte-IDENTICAL
        # prev_randao. Had a forged seed finalized, the honest set's seeds would fail-verify and
        # fall to the digest fallback — a zero or divergent mixHash — so this window MUST fail on
        # a finalized forge. That is what makes it meaningful rather than decorative.
        boundary = ctx.epoch_first_block(forge_epoch)
        ctx.check(ctx.wait_finalized_ge(boundary, VR.FORGE_BOUNDARY_S),
                  f"chain did not reach the forged-boundary block {boundary} (epoch "
                  f"{forge_epoch})",
                  on_fail=lambda: ctx.dump_logs(VR.LOG_TAIL_NODE, topology.PINNED_RPC_HOST))
        relive_hi = boundary + VR.RELIVE_OFFSET
        ctx.check(ctx.wait_finalized_ge(relive_hi, VR.FORGE_RELIVE_S),
                  f"chain did not advance past the forged boundary ({relive_hi})")
        ctx.check(wait_nodes_have(ctx, relive_hi, nodes=self.honest_nodes),
                  f"not all honest nodes reached the window block {relive_hi}")
        assert_honest_beacon_window(ctx, boundary, relive_hi, f"post-forge E{forge_epoch}",
                                    self.honest_nodes)
        _say(ctx, "honest nodes derive a non-zero, byte-identical prev_randao across the forged "
                  "boundary — the forge corrupted neither the finalized seed nor the derived "
                  "randomness")

        # ── the POSITIVE half of the safety proof: the honest C gate REJECTED it ──────
        cfail = ctx.log_count(topology.PINNED_RPC_HOST, VR.CFAIL_LINE, dry_value=1)
        ctx.check(*VR.evaluate_c_gate_rejected(cfail),
                  on_fail=lambda: _dump_c_gate(ctx))
        _say(ctx, f"validator-0 rejected the forged boundary at the C gate x{cfail} (forged PK_E "
                  "could not notarize → never reached certify)")

        assert_still_finalizing(ctx, verdict=VR.evaluate_byz_liveness)
        _ok(ctx, f"a Byzantine stayer (validator-{self.byz_idx}) FORGED a PK_E at a change-epoch "
                 f"boundary (epoch E{forge_epoch}, x{state['forged']}, reached by toggling v5 in "
                 "and out of the committee until the byzantine led one); the honest share-holders "
                 f"REJECTED it at the C gate (x{cfail}) so it could not notarize → the forged PK_E "
                 "NEVER finalized; all honest nodes derive a non-zero, byte-identical prev_randao "
                 "across the forged boundary; the chain crossed via an honest leader and kept "
                 "finalizing under tx load. SAFETY + LIVENESS.")


def _stake_setup(ctx, drive: ByzantineDrive, addrs) -> None:
    """`:315-340` — a clean swing member, a rotation-proof byzantine, and a settled "out" floor.

    Initial stakes are v0=5e18 and v1..v4=1e18 each. Two things are needed and neither is optional:

      (i)  the byzantine PERMANENTLY in the top-5, so it runs the per-epoch DKG and holds a real
           share to forge at EVERY boundary. +29e18 ⇒ 30e18, far above both the 2e18 floor tier and
           the joiner's maximum 3e18, so it is always rank-1 and never the swapped node.
      (ii) the joiner as the ONLY member that enters and leaves, with no tie-break ambiguity. Its
           owner self-stake floor is the 1e18 minimum and cannot go lower, so with v1/v3/v4 also at
           1e18 a joiner-OUT would TIE them — a gamble over which of four equal nodes fills the
           bottom slot. Lifting the three non-byzantine 1e18 validators by exactly
           `minStakingAmount` (a smaller bump reverts AmountTooLow) makes the joiner strictly the
           lowest when out and strictly above the floor when in: exactly ONE membership swap each
           way, hence a genuine change-epoch boundary every time.
    """
    byz_key = ctx.owner_key(drive.byz_idx)
    byz_addr = addrs[drive.byz_idx]
    ctx.cast_send_checked([ctx.token, "approve(address,uint256)(bool)", ctx.staking_rt,
                           VR.BYZ_BOOST_WEI], note="approve-byz-boost", key=byz_key)
    ctx.check(ctx.cast_send([ctx.staking_rt, "delegate(address,uint256)", byz_addr,
                             VR.BYZ_BOOST_WEI], note="delegate-byz-boost", key=byz_key),
              f"self-delegate to byzantine validator-{drive.byz_idx}")
    _say(ctx, f"validator-{drive.byz_idx} boosted (+29e18 ⇒ 30e18) — dominant leader stake "
              "(~72% of boundaries)")

    for i in FLOOR_BUMP_IDXS:
        if i == drive.byz_idx:
            continue
        key = ctx.owner_key(i)
        ctx.cast_send_checked([ctx.token, "approve(address,uint256)(bool)", ctx.staking_rt,
                               VR.FLOOR_BUMP_WEI], note=f"approve-floor-bump-v{i}", key=key)
        ctx.check(ctx.cast_send([ctx.staking_rt, "delegate(address,uint256)", addrs[i],
                                 VR.FLOOR_BUMP_WEI], note=f"delegate-floor-bump-v{i}", key=key),
                  f"floor-bump validator-{i}")
    _say(ctx, "v1/v3/v4 lifted +1e18 each (v5-OUT committee floor = 2e18 > v5 floor 1e18)")


def _drive_flips(ctx, drive: ByzantineDrive, e0, got0, joiner, byz_addr):
    """`:435-522` — the toggle loop plus the drain, as one bounded drive.

    Returns the drive state: how many forges were seen, how many committee CHANGES surfaced, and
    the first epoch the joiner entered (the SAFETY anchor's fallback).

    Both loops watch three things at once — the forge count, the committee timeline, and whether
    THIS flip has landed — because they are the same poll and splitting them would triple the RPC
    load on a chain the case is also measuring."""
    state = {"forged": 0, "changes": 0, "e_new": None, "prev_seen": ""}
    deadline = time.monotonic() + 60 * drive.budget_min
    want_in = True

    for t in range(1, drive.max_toggles + 1):
        if not ctx.dry and time.monotonic() >= deadline:
            _say(ctx, f"[drive budget] wall-clock exhausted after {t - 1} toggles")
            break
        state["forged"] = ctx.log_count(drive.service, VR.FORGE_LINE, dry_value=0)
        if state["forged"] >= 1:
            break

        desc = "IN" if want_in else "OUT"
        _say(ctx, f"[toggle {t}/{drive.max_toggles}] v5 {desc}")
        _toggle(ctx, drive, joiner, desc)
        want_in = not want_in

        surfaced = _await_flip(ctx, drive, state, desc, e0, got0, joiner, byz_addr)
        # A flip that never surfaced within its window means the chain stalled or the warmup
        # arithmetic drifted. STOP toggling rather than spinning out more that would just
        # re-collide, and let the drain plus the final assertion report with diagnostics.
        if state["forged"] < 1 and not surfaced:
            _say(ctx, f"[flip {t}] v5 {desc} did NOT surface within ~5 min — stopping the drive "
                      "(chain stalled / warmup drift)")
            break
        if ctx.dry:
            break

    # DRAIN: the last flips' committee changes surface ~3 epochs AFTER they were issued, so
    # boundaries are still in flight when the loop ends. Without this a forge the byzantine is
    # about to lead would read as a false FAIL.
    if state["forged"] < 1:
        _say(ctx, "[drain] toggles issued; waiting for in-flight change boundaries to surface…")

        def drained():
            state["forged"] = ctx.log_count(drive.service, VR.FORGE_LINE, dry_value=1)
            if state["forged"] >= 1:
                return True
            _observe_committee(ctx, drive, state, e0, got0, joiner, byz_addr)
            return False

        remaining = max(1, int(deadline - time.monotonic()))
        ctx.poll(drained, remaining, poll_s=VR.DRAIN_POLL_S)
    return state


def _toggle(ctx, drive: ByzantineDrive, joiner: str, desc: str) -> None:
    """`toggle_v5 in|out` (`:382-395`) — the delegation change that flips the joiner's membership.

    From the DEDICATED delegator, never the joiner's own owner key: an `undelegate` from the owner
    would trip `OwnerSelfStakeBelowMinimum` and the OUT half of the drive would fail on every
    iteration."""
    if desc == "IN":
        ok = ctx.cast_send([ctx.staking_rt, "delegate(address,uint256)", joiner, VR.V5_IN_AMOUNT],
                           note="toggle-in-v5", key=drive.toggle_key)
        ctx.check(ok, "delegate-in v5", on_fail=lambda: ctx.dump_logs(60, drive.service))
    else:
        ok = ctx.cast_send([ctx.staking_rt, "undelegate(address,uint256)", joiner,
                            VR.V5_IN_AMOUNT], note="toggle-out-v5", key=drive.toggle_key)
        ctx.check(ok, "undelegate-out v5", on_fail=lambda: ctx.dump_logs(60, drive.service))


def _await_flip(ctx, drive, state, desc, e0, got0, joiner, byz_addr) -> bool:
    """`:457-489` — block until THIS flip lands in the ahead-committed committee, watching the
    forge count and the committee timeline meanwhile."""
    def landed():
        state["forged"] = ctx.log_count(drive.service, VR.FORGE_LINE, dry_value=0)
        if state["forged"] >= 1:
            return True
        now_e = _observe_committee(ctx, drive, state, e0, got0, joiner, byz_addr)
        ahead = ctx.committee(now_e + 1, dry_value=f"{joiner} {byz_addr}")
        return VR.toggle_landed(desc, ahead, joiner)

    return bool(ctx.poll(landed, VR.FLIP_S, poll_s=VR.FLIP_POLL_S))


def _observe_committee(ctx, drive, state, e0, got0, joiner, byz_addr) -> int:
    """One committee sample, shared by the toggle loop and the drain (`:460-479`, `:510-519`).

    Counts a CHANGE only when a previous set was already observed — the first sample establishes
    the baseline and is not itself a change — and anchors `E_new` on the first change that brings
    the joiner in, asserting there that the byzantine is still a member. That assertion belongs
    HERE, at the moment E_new is identified: a byzantine that got swapped out cannot forge, and
    discovering it forty minutes later at the forge check would report the wrong cause."""
    now_e = ctx.current_epoch()
    cur = ctx.committee(now_e, dry_value=f"{joiner} {byz_addr}")
    if cur and cur != state["prev_seen"]:
        if state["prev_seen"]:
            state["changes"] += 1
            _say(ctx, f"  committee changed at epoch {now_e} (#{state['changes']}): [{cur}]")
        if state["e_new"] is None and VR.committee_has(cur, joiner) and cur != got0:
            state["e_new"] = now_e
            _say(ctx, f"  → E_new={now_e} (v5 first entered the committee; E0={e0})")
            ctx.check(*VR.evaluate_byz_in_committee(cur, byz_addr, drive.byz_idx, now_e))
        state["prev_seen"] = cur
    return now_e


def _dump_forge_failure(ctx, drive: ByzantineDrive) -> None:
    """`:529-530` — the byzantine's recent log, filtered to the byzantine/beacon/boundary markers.
    Forty minutes of drive produced no forge; this is what says whether it ever even reached a
    change-epoch boundary."""
    print(f"  --- byzantine validator-{drive.byz_idx} recent log "
          "(byzantine/beacon/boundary/change-epoch) ---", flush=True)
    markers = ("byzantine", "beacon", "boundary", "change-epoch", "is_change_epoch")
    hits = [ln for ln in ctx.logs_all(drive.service).splitlines()
            if any(m in ln.lower() for m in markers)]
    print("\n".join(hits[-50:]), flush=True)


def _dump_c_gate(ctx) -> None:
    """`:585` — validator-0's tail filtered to the C-gate and beacon markers."""
    markers = ("c share-on-poly", "beacon", "boundary")
    hits = [ln for ln in ctx.logs_all(topology.PINNED_RPC_HOST).splitlines()
            if any(m in ln.lower() for m in markers)]
    print("\n".join(hits[-40:]), flush=True)
