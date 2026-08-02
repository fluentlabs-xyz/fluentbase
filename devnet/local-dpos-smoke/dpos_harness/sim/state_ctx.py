"""state_ctx.py — the SimState → (battery.Ctx, battery.SimCtx) adapter.

This is the SIM SIDE of the checks split (P4 item 4.4) and it stays here deliberately: the
battery takes two contexts, and translating one simulation's state into them is the
simulation's job, not the framework's. A smoke case that wants the chain-only detectors builds
a `Ctx` itself and instantiates `checks.battery.ChainBattery` — it never comes through here.

  Ctx     ← chain / topology / metrics facts + threshold knobs. Any consumer can supply these.
  SimCtx  ← the churn bookkeeping only this simulation knows (identity pool, seat/refill
            machine, fault ledger). Only the six sim-coupled detectors read it.

`to_ctx` returns BOTH, as one pair, and the orchestrator binds them with `Battery.bind` in one
call — there is no way to refresh the chain facts and leave the sim bookkeeping at last tick's
snapshot.

The observer half (battery.py) names its fields after the bash case-soak.sh globals
(`${VAR:-default}`). The active half (orchestrator.py) holds those same globals as domain-typed
`SimState` sub-dataclasses. Both halves were designed to the SAME bash source, so the field
mapping is aligned by construction; this module is the one explicit place the mapping lives,
and any drift is reconciled + listed here.

FIELD MAP (Ctx / SimCtx field  ←  SimState path). All 1:1 unless noted.
Ctx (generic):

  SIM_TICK             ← state.tick
  SIM_ROUND            ← state.round
  SIM_CUR_EPOCH        ← state.cur_epoch
  SIM_CUR_F            ← state.cur_f
  SIM_CUR_COMMITTEE    ← state.cur_committee
  ADDR2IDX              ← state.address.addr2idx
  SIM_DISRUPTED        ← state.disrupted
  PERMANENTLY_DEAD      ← state.identity.permanently_dead        (CONTAINER-keyed in both)
  EXPECTED_STALL        ← state.expected_stall
  SETTLE_UNTIL_EPOCH    ← state.settle_until_epoch
  SIM_NO_CASCADE       ← cfg.no_cascade                         (a cfg knob, not a live global)
  MIN_COMMITTEE         ← cfg.min_committee
  SIM_BATTERY_COVERAGE ← state.battery_coverage   (set each tick by compute_self_partitioned; F9)

SimCtx (simulation-only):

  EVER_FAULTED          ← state.identity.ever_faulted            (IDENTITY-idx-keyed in both)
  EVER_MEMBERS          ← state.address.ever_members
  OWNER_ADDR_CACHE      ← state.address.owner_addr_cache
  SIM_VALIDATORS       ← cfg.validators
  SIM_SPARE_END        ← cfg.spare_end
  SIM_VAL_CONTAINERS   ← cfg.val_containers
  NEXT_JOINER           ← state.next_joiner
  GROW_LANDED           ← state.grow_landed
  REFILL_RESUMED        ← state.refill_resumed
  REFILL_PENDING        ← state.refill_pending
  PROMOTE_PENDING       ← state.promote_pending
  SLOT_IDENTITY         ← state.container.slot_identity   (values INT-normalized: source is
                            string-valued per bash assoc-array semantics, but the battery's
                            idle-rotation predicate compares against an int idx — see to_ctx)

DRIFT RECONCILED (2 fields — the ONLY drift found; both are battery-owned rolling state with
no SimState home, so they keep their Ctx defaults, which is correct):
  * REFILL_STALL_VERDICT — battery-internal refill-climb verdict ("climbing"/"stall"); the
    orchestrator does not own it (the battery derives it from its own tick history). LEFT at
    the Ctx default "climbing". If the reconciler's refill watchdog later needs to feed it,
    add a `refill_stall_verdict` scalar to SimState and map it here.
  * L3_SPAMMER_ADDR / SIM_DATA_ROOT / SIM_PEER_SPOKE — env/topology facts, not churn state;
    sourced from os.environ via the Ctx defaults (`_envs`, battery.py:100-102), not from
    SimState — so each lands only if something EXPORTS it into this process. SIM_DATA_ROOT
    comes from the launcher (Makefile:240-242) and is empty on a run started without it, which
    leaves the data-root-full watchdog inert BY CONFIGURATION rather than by defect;
    SIM_PEER_SPOKE carries a real default ("validator-1"); L3_SPAMMER_ADDR is exported by the
    bring-up itself once the L3 spammer key is derived (bringup.py:540-547, mirroring bash
    case-soak.sh:945-946) — keeping it a local was exactly why the write-path reporter
    (_inv_write_path) never ran in the port. (SIM_BATTERY_COVERAGE is now real churn state —
    mapped from state.battery_coverage above, F9.)
  * STAKING_RT / CHAIN_CONFIG_RT / LIVENESS_RT — post-deploy runtime facts. NOT env-sourced and
    NOT SimState: the orchestrator passes them as overrides each tick from the Chain it built
    (orchestrator.rt_addrs). Their Ctx default is empty and EVERY live consumer must fill it —
    the sim DEPLOYS these three contracts at run time, so the nodes.py predeploy fallback the
    battery's cast seams take is CODELESS here: the reads return empty and the contract-backed
    detectors report "holds" over nothing. _build_chain refuses to hand back a chain missing
    any; the read-only shadow observer, which has no Chain, resolves the same three from
    /runtime/staking-reader.json at attach and ABORTS if it cannot
    (shadow.resolve_runtime_addrs). Only the unit tests legitimately leave them empty.

No NAME drift was found: every case-soak.sh global the battery reads has an identically-
semantic SimState field. The domain split (container/identity/seat/address) is invisible to
the battery (it reads the flat bash names), so the adapter just re-flattens the sub-dataclasses.
"""

from __future__ import annotations

from ..checks import battery


def to_ctx(state, **overrides):
    """Build the (Ctx, SimCtx) pair from the live SimState. `overrides` lets a caller inject
    observed-only Ctx fields (the STAKING_RT/CHAIN_CONFIG_RT/LIVENESS_RT deploy facts,
    SIM_PEER_SPOKE) the battery also reads.

    Returns BOTH so the caller binds both — `Battery.bind(*to_ctx(state))`. Splitting this into
    two functions would let a caller refresh one and forget the other."""
    cfg = state.cfg
    ctx = battery.Ctx(
        SIM_TICK=state.tick,
        SIM_ROUND=state.round,
        SIM_CUR_EPOCH=state.cur_epoch,
        SIM_CUR_F=state.cur_f,
        SIM_CUR_COMMITTEE=state.cur_committee,
        ADDR2IDX=state.address.addr2idx,
        SIM_DISRUPTED=state.disrupted,
        PERMANENTLY_DEAD=state.identity.permanently_dead,
        EXPECTED_STALL=state.expected_stall,
        SETTLE_UNTIL_EPOCH=state.settle_until_epoch,
        SIM_NO_CASCADE=cfg.no_cascade,
        MIN_COMMITTEE=cfg.min_committee,
        SIM_BATTERY_COVERAGE=state.battery_coverage,
    )
    sim = battery.SimCtx(
        EVER_FAULTED=state.identity.ever_faulted,
        EVER_MEMBERS=state.address.ever_members,
        OWNER_ADDR_CACHE=state.address.owner_addr_cache,
        SIM_VALIDATORS=cfg.validators,
        SIM_SPARE_END=cfg.spare_end,
        SIM_VAL_CONTAINERS=cfg.val_containers,
        NEXT_JOINER=state.next_joiner,
        GROW_LANDED=state.grow_landed,
        REFILL_RESUMED=state.refill_resumed,
        REFILL_PENDING=state.refill_pending,
        PROMOTE_PENDING=state.promote_pending,
        # INT-normalized: the source dict stores hosted-idx as STRINGS (bash assoc-array
        # semantics — orchestrator seeds str(rs), hostpool re-tasks with str(idx)), but the
        # battery's idle-rotation predicate (Battery._node_down_excluded) compares
        # `SLOT_IDENTITY.get(container, idx) != idx` against an INT idx. Passing the raw strings
        # made "12" != 12 always True, so every deliberately-stopped idle rotation slot looked
        # RE-TASKED → node-down fired on validator-12/13 at r0 (live v61.10). Normalize here, at
        # the one mapping seam, so the int-keyed predicate engages (fix stays out of the predicate).
        SLOT_IDENTITY={c: int(v) for c, v in state.container.slot_identity.items()},
    )
    for k, v in overrides.items():
        setattr(ctx, k, v)
    return ctx, sim
