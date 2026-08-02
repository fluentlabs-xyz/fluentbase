"""rounds.py — the pure, replay-exact per-round decision layer, lifted verbatim out of
orchestrator.py.

WHY IT IS ITS OWN MODULE (and not a convenience split): `dispatch` needs `draw_round` /
`build_gate_inputs` / `apply_action` at import time while `orchestrator` constructs a
`Dispatcher`, so the two modules imported each other. That cycle only survived because
orchestrator's leg was hand-placed INSIDE a function. Both now import this module and the
cycle is gone — nothing about the decisions themselves changed.

What lives here:
  * draw_round  — the FIXED 4-draw churn lottery (u32 delay, mod #ACTIONS, mod #vpool, u32 param)
                  with the victim pool built BEFORE the 3rd draw, the calm bit, and the two forced
                  overrides. THIS is the replay-exact decision layer the differential test pins.
  * build_gate_inputs — gate-input construction from live state.
  * apply_action — the apply dispatch (the SAME one the self-check dry-runs).

SACRED: the 4-draw discipline. Never insert a conditional draw; see draw_round's own docstring.
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass

from . import actions
from ..core import setops

# `state` throughout this module is an `orchestrator.SimState`. The type is deliberately NOT
# imported and NOT written as an annotation, not even under `if TYPE_CHECKING:`: `orchestrator`
# imports this module, so any reference back — runtime or type-only — restores the very
# orchestrator↔rounds shape this module exists to remove, and tests/test_layering.py would have
# to grow an exemption to tolerate it. The plan treats an in-function/type-only leg as a cycle
# too, which is why the annotations are dropped rather than hidden behind a guard.


# ── the 4-draw churn decision layer (SACRED — preserves replay of old seeds) ──

@dataclass
class RoundDecision:
    """The output of draw_round: the chosen action/victim, the calm bit, and the four draws
    (delay + aparam are drawn-but-unused, retained for replay order). is_calm/no_victim/forced
    let the dispatcher (and the differential test) reason about the round without re-drawing."""
    action: str
    victim: str            # "" when the eligible-victim pool was empty
    is_calm: int
    delay: int
    aidx: int
    vidx: int
    aparam: int
    forced: str = ""       # "" | "byzantine" | "voluntary_exit"


def _calm_bit(seed: str, cur_epoch: int, calm_permille: int) -> int:
    """The deterministic epoch calm bit (case-soak.sh:2882-2888). cur_epoch<=2 is FORCED calm
    (the ramp); else sha256("seed:calm:epoch")[:8] as an int, mod 1000 < calm_permille. Kept
    byte-identical to the bash so a seed's calm/storm schedule replays exactly."""
    if cur_epoch <= 2:
        return 1
    h = hashlib.sha256(f"{seed}:calm:{cur_epoch}".encode()).hexdigest()[:8]
    cb = int(h, 16) % 1000
    return 1 if cb < calm_permille else 0


def draw_round(state, actions_pool, victim_pool, cur_epoch: int,
               calm_permille: int) -> RoundDecision:
    """The FIXED 4-draw fault lottery (case-soak.sh:2857-2864) + calm bit + forced overrides.

    SACRED DISCIPLINE (plan §1.1): the victim pool is built BEFORE the draws, then EXACTLY four
    draws are consumed unconditionally IN ORDER — u32(delay), mod(#ACTIONS), mod(#vpool),
    u32(aparam) — so the stream position at round R is always 4·R and any old SIM_SEED replays
    the identical INTENT schedule. NEVER insert a conditional draw here. next_mod handles an
    empty pool (m=0 → 0) so the draw is still consumed.

    An EMPTY victim pool → NO victim (phase-2 item 6): the bash used to fall back to validator-0,
    contradicting v0's pinned-honest invariant AND opening the v0-burned-keys-rebirth hazard. The
    honest response is to NOT ACT; the four draws are already consumed so replay is preserved.

    Forced overrides (byzantine first, then voluntary_exit) are gated on epoch/state and OVERRIDE
    the calm bit; byzantine wins a shared hot epoch (_forced_override precedence). They draw the
    first eligible victim via committee_victim_idxs (which reads NO PRNG), so they do not disturb
    the 4-draw discipline."""
    prng = state.prng
    delay = prng.u32()
    aidx = prng.mod(len(actions_pool))
    vidx = prng.mod(len(victim_pool))
    aparam = prng.u32()

    action = actions_pool[aidx]
    victim = victim_pool[vidx] if victim_pool else ""
    is_calm = _calm_bit(state.cfg.seed, cur_epoch, calm_permille)
    forced = ""

    cfg = state.cfg
    # DEBUG self-heal trigger: force byzantine_equivocate on a live member at/after the configured
    # epoch (post-growth), until COUNT tombstones have applied. Still fully gated (4b/4c). Overrides
    # the calm bit so it actually fires.
    if (cfg.force_byzantine_epoch
            and cur_epoch >= cfg.force_byzantine_epoch
            and state.next_joiner >= cfg.validators
            and state.next_joiner - 1 == state.grow_landed
            and state.tombstones < max(cfg.force_byzantine_count, 1)):
        pool = setops.committee_victim_idxs(state.cur_committee, state.address.addr2idx,
                                            state.disrupted, skip_disrupted=True)
        if pool:
            action, victim, is_calm, forced = "byzantine_equivocate", pool[0], 0, "byzantine"

    # DEBUG voluntary-exit trigger: PRECEDENCE — forced-byzantine claims a shared hot epoch first.
    if (cfg.force_voluntary_exit_epoch > 0 and forced == ""
            and cur_epoch >= cfg.force_voluntary_exit_epoch
            and state.next_joiner >= cfg.validators
            and state.next_joiner - 1 == state.grow_landed
            and state.voluntary_exits < cfg.force_voluntary_exit_count):
        pool = setops.committee_victim_idxs(state.cur_committee, state.address.addr2idx,
                                            state.disrupted, skip_disrupted=True)
        if pool:
            action, victim, is_calm, forced = "voluntary_exit", pool[0], 0, "voluntary_exit"

    return RoundDecision(action=action, victim=victim, is_calm=is_calm, delay=delay,
                         aidx=aidx, vidx=vidx, aparam=aparam, forced=forced)


# ── gate-input construction from live state (case-soak.sh:3034-3105) ──────────

def build_gate_inputs(state, action: str, victim: str, n: int,
                      nxt_committee: str, top_leader: str, add_source_avail: int = 1
                      ) -> actions.GateInputs:
    """Populate the PURE gate inputs from live state — the exact assignment block the bash runs
    before gate_accept. Kept faithful including the conservative GA_CHANGE_PENDING when the
    committee read failed (an unobserved committee is treated as "a change may be in flight")."""
    cfg = state.cfg
    change_imminent = 1 if (nxt_committee and nxt_committee != state.cur_committee) else 0
    incoming = _incoming_idx_set(state, nxt_committee)

    change_pending = 0
    if (state.committee_read_ok == 0 or n < cfg.validators
            or state.next_joiner - 1 > state.grow_landed
            or state.promote_pending or state.refill_pending or state.refill_resumed
            or state.cur_epoch < state.settle_until_epoch):
        change_pending = 1

    membership_settling = 1 if state.cur_epoch < state.settle_until_epoch else 0
    return actions.GateInputs(
        action=action, victim=victim, n=n,
        disrupted=state.disrupted, effective=state.effective_faults,
        change_imminent=change_imminent, incoming=incoming, change_pending=change_pending,
        shareless=state.shareless, min_committee=cfg.min_committee,
        pending_min=n,  # conservative: projected shrink bounded by gate rule 4/5 admission
        leader=top_leader, round_others=setops.count(state.disrupted),
        membership_settling=membership_settling, add_source_avail=add_source_avail,
    )


def _incoming_idx_set(state, nxt_committee: str) -> str:
    """_incoming_idx_set: map incoming owner-addrs (committee[cur+1]) to their container idxs via
    ADDR2IDX, space-joined (the set rule-2/3 test membership against). Empty when unmapped."""
    out = []
    for a in (nxt_committee or "").split():
        ci = state.address.addr2idx.get(a)
        if ci:
            out.append(ci)
    return " ".join(out)


# ── the apply dispatch (gate already said safe; case-soak.sh:2035-2077) ───────

def _apply_chain_write(fn, arg, event, label) -> bool:
    """Run one apply-arm chain write best-effort; True iff it landed.

    The module-level twin of Dispatcher._rotation_chain_write (dispatch.py:640-649), which
    apply_action cannot reach — that one is a Dispatcher METHOD and this is a module-level
    function. Same contract: a revert/transient logs a churn diagnostic and returns False (the
    action is abandoned this round, retried on a later draw) instead of aborting a night-long run.

    Why an EXCEPTION is the failure signal: Chain.voluntary_exit (chain/writes.py:582-589) returns
    True unconditionally or RAISES ChainError — propagated from gov_action/_status_assert. There
    is no False to test, and a False return must NOT be added to Chain: that would diverge it from
    the rotation caller, which already handles the raise. Only ChainError is caught — a genuine
    bug (TypeError, AttributeError) still propagates loudly."""
    from ..chain.writes import ChainError
    try:
        fn(arg)
        return True
    except ChainError as e:
        if event:
            event("churn", f"chain-write deferred ({label}): {e}")
        return False


def apply_action(state, act: "actions.Actuators", action: str, victim: str,
                 cur_epoch: int, *, chain, event=None):
    """The apply dispatch — the SAME one the self-check dry-runs. Transient restart/kill/throttle
    faults set RESTORE_AT to cur_epoch (restore ASAP; confirm-before-free still holds the against-f
    slot until a real rejoin). Only the EXPLICIT byzantine / voluntary_exit actions cause a lasting
    membership change.

    EVER_FAULTED (wrongful-slash expectation set) is stamped for EVERY fault-class action BEFORE
    the arms, keyed by ident_idx(victim) — the IDENTITY, not the container (a fault follows the
    identity forever; container-keying would false-fail the burned identity + mask the new
    occupant). delegate_shift / register_activate add no disruption → not stamped.

    `chain` is a REQUIRED KEYWORD arg (chain.writes.Chain), not an Actuators attribute: the two
    gov/stake arms (delegate_shift, voluntary_exit) are CHAIN writes, and Actuators is the
    docker/cast actuator half — giving it a chain would make it a second owner of the chain
    connection and break the pure/impure split this module claims. Keyword-only and with no
    default ON PURPOSE: the arms it feeds used to reach for sim_delegate_shift /
    sim_voluntary_exit on the actuator behind a getattr/hasattr guard, so a name that existed
    nowhere silently no-op'd for the whole life of the harness. A missed call site must now be a
    TypeError naming `chain`, at the call, not a silent skip."""
    cfg = state.cfg
    applied_msg = f"APPLIED {action} {victim} (held against f until confirmed rejoin)"

    if action in ("graceful_stop_restart", "sigkill_restart", "cpu_throttle",
                  "dkg_midwindow_restart", "byzantine_equivocate",
                  "byzantine_forge_pk", "voluntary_exit") and victim:
        state.identity.ever_faulted[state.ident_idx(victim)] = action

    c = state.container
    if action == "graceful_stop_restart":
        act.act_graceful_stop(victim); state.mark_disrupted(victim)
        c.disrupt_kind[victim] = action; c.restore_at[victim] = cur_epoch
    elif action == "sigkill_restart":
        act.act_sigkill_stop(victim); state.mark_disrupted(victim)
        c.disrupt_kind[victim] = action; c.restore_at[victim] = cur_epoch
    elif action == "cpu_throttle":
        act.act_cpu_throttle(victim); state.mark_disrupted(victim)
        c.disrupt_kind[victim] = action; c.restore_at[victim] = cur_epoch
    elif action == "dkg_midwindow_restart":
        # M1: verify-only until it reloads its persisted share (~2 epochs); process_restores then
        # DECREMENTS SIM_SHARELESS (transient, not a ratchet).
        act.act_dkg_midwindow_restart(victim); state.mark_disrupted(victim)
        c.disrupt_kind[victim] = action; state.shareless += 1
        c.restore_at[victim] = cur_epoch + 2
    elif action == "delegate_shift":
        # Chain.delegate_shift (chain/writes.py:591) — approve+delegate 3e18 to the victim's owner
        # addr to shift EffBal and force a rotation @E+3. It takes the SERVED IDENTITY idx, not a
        # container name. The pending marker is registered ONLY on a landed write: a delegate that
        # never happened must not leave a rotation expectation the reconcilers then watch for.
        if _apply_chain_write(chain.delegate_shift, state.ident_idx(victim), event,
                              f"delegate_shift {victim}"):
            state.add_pending(f"delegate@{cur_epoch + 3}")
            applied_msg = f"APPLIED delegate_shift {victim}"
        else:
            applied_msg = ""
    elif action == "byzantine_equivocate":
        # SEAT-keyed PERMANENT tombstone: the obligation and the slash-landed deadline are facts
        # about the BURNED IDENTITY, so neither may be keyed by a container that gets recycled.
        act.act_byzantine(victim, "equivocate"); state.mark_disrupted(victim)
        c.disrupt_kind[victim] = action
        state.identity.permanently_dead[victim] = 1
        state.tombstones += 1
        state.settle_until_epoch = cur_epoch + cfg.membership_settle
        seat = actions.enqueue_backfill_obligation(state, victim, cur_epoch)
        state.identity.tombstone_settle_epoch[seat] = cur_epoch + cfg.membership_settle
    elif action == "byzantine_forge_pk":
        act.act_byzantine(victim, "forge-beacon-pk"); state.mark_disrupted(victim)
        c.disrupt_kind[victim] = action; c.restore_at[victim] = cur_epoch + 1
    elif action == "voluntary_exit":
        # REVERSIBLE departure-then-backfill OUT half. Open the settle window / enqueue ONLY on a
        # REAL exit (a govern round-trip that never landed must NOT open a phantom window). NO
        # mark_disrupted (node stays UP → zero rule-1 budget), NO permanently_dead (identity
        # RELEASED, not burned).
        # Chain.voluntary_exit (chain/writes.py:582) — disableValidator + status==2 assert, keyed by
        # the SERVED IDENTITY idx. A raise (gov never Succeeded / status never landed) is the
        # failure signal and maps to landed=False here; see _apply_chain_write.
        landed = _apply_chain_write(chain.voluntary_exit, state.ident_idx(victim), event,
                                    f"voluntary_exit {victim}")
        if landed:
            seat = actions.enqueue_backfill_obligation(state, victim, cur_epoch)
            c.disrupt_kind[victim] = action
            state.voluntary_exits += 1
            state.promotable = f"{state.promotable} {seat}".strip()
            state.settle_until_epoch = cur_epoch + cfg.membership_settle
            applied_msg = (f"APPLIED voluntary_exit {victim} (committee cap->cap-1; identity "
                           "released to PROMOTABLE; promote track backfills)")
        else:
            applied_msg = ""

    if applied_msg and event:
        event("churn", applied_msg)
