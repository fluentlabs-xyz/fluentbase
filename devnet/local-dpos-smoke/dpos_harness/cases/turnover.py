"""turnover.py — the ZERO-OVERLAP committee boundary (FLU-1203), deterministic.

The property under test is a halt that nothing in the tree enforces or detects.
Entering epoch E+1 a member needs σ of epoch E — as the leader-election base for
the seedless arm — and until FLU-1203 the only source of that σ was this node's
own engine during E. A committee with ZERO overlap with its predecessor
therefore had nothing to start from: every leader of the new committee skipped
its view and the chain stopped.

WHERE THAT σ COMES FROM NOW (revised for FLU-1204, `parent_seed` left the block
body). It is no longer read off the previous epoch's terminal BLOCK. The member
names the round from agreed data — `Round(E, terminal_block.proposal_view)` —
and asks its own seed store, which every verified certificate fills
(`capture_certificate_seed`, on both cert doors) and which pins one σ per epoch
against retention eviction. So the transport this case exercises is the
CERTIFICATE, not a block field, and the failure it must still catch is the same
one: an incoming member that cannot obtain σ of the outgoing epoch.

WHY THIS CASE CANNOT RUN ON THE PRODUCTION-PATH STACK. `MIN_COMMITTEE_LENGTH`
is 4, mirrored in the contract, in the node's staking reader, and independently
in `compose_gen`; `commit_epoch_committee` reverts below it and that revert is a
pre-execution system call — a chain stop no transaction can repair. So zero
overlap needs four seats out and four DIFFERENT seats in: at least EIGHT keyed
validators. The production-path stack has six containers fixed by a checked-in
compose file.

WHAT A GREEN RUN HERE IS NOT EVIDENCE FOR. The incoming committee is v4..v7, and
those are exactly the containers OUTSIDE the genesis committee — the ones that keep
`--dpos.follower-upstream`, because they cold-start unregistered and the
authenticated plane serves `active_registry u committee` only. So the nodes whose
boundary this case tests run the WS cert-inlet arm. The sigma capture is shared by
both arms, so the boundary is genuinely carried; but the PLANE arm, which a
production zero-overlap between long-active validators would use, is covered by a
deterministic unit (`the_plane_arms_by_height_pull_captures_the_certificate_seed`)
and not by this run. Do not read green here as evidence for it.

WHY THE PREMISE IS ASSERTED SEPARATELY. "All four seats flipped at ONE commit"
is not something a green finish implies — a case that flipped three of four, or
flipped them one per boundary, would exercise a boundary WITH overlap and pass
for the wrong reason. The premise is its own failing assert, before the
conclusion is looked at, and `evaluate_turnover_case` scores the two in that
order.
"""

import os
import time

from ..core import topology
from ..core.exit_codes import RC_FAIL, RC_PASS, RC_USAGE
from ..core.policy import gov_live_voter_idx

#: The two counters that say whether the INCOMING committee obtained σ of the
#: outgoing epoch. They replace `dpos_parent_seed_boundary_skip_total`, which
#: counted the propose-side witness miss and no longer exists: FLU-1204 deleted
#: the witness, the field it rode in and the propose gate that skipped a view
#: when the store could not answer. The old constant was left reading a family
#: nothing emits, which sums to zero on every node and passes the conclusion
#: assert unconditionally — a green run measuring nothing.
#:
#: They are NOT interchangeable and they fail differently:
#:
#:   epoch_engine_spawn_deferred_total — the member could not get one of the two
#:     `Inline::genesis(E)` inputs (σ at E-1's terminal round, or the E-1
#:     boundary BLOCK), so its per-epoch engine did not spawn and it sits
#:     verify-only: no proposals, no votes. This is the direct heir of the
#:     boundary skip — same cause ("I could not obtain the predecessor's σ"),
#:     same self-healing-on-retry character. It is asserted FLAT for the same
#:     reason the skip was: at a four-seat committee, one deferring member still
#:     leaves a quorum of three, so the chain crosses the boundary anyway and the
#:     liveness assert alone would call a partly-broken transport green.
#:   dpos_fallback_seed_constant_total — the member elected on
#:     `sha256(epoch ‖ sorted peers)`, the PREDICTABLE base, instead of
#:     inheriting σ. A σ MISS cannot reach this branch by construction (a miss
#:     defers the spawn; it never answers "no σ here"), so a rise across this
#:     boundary means the epoch predicate called E-1 beacon-INACTIVE. That is the
#:     silent downgrade, and a zero-overlap boundary is where it would first show.
#:
#: BOTH LIVE ON THE COMMONWARE REGISTRY (:9100), not on reth's metrics-rs
#: exporter at :9200, and both are registered EAGERLY at startup on both node
#: classes (`EpochEngineMetrics::register`). That INVERTS the absence rule the old
#: metrics-rs family needed: an absent family is not a healthy never-incremented
#: zero here, it is a dead scrape or a renamed family, and it is scored UNREAD.
SPAWN_DEFER_METRIC = "epoch_engine_spawn_deferred_total"
CONSTANT_BASE_METRIC = "dpos_fallback_seed_constant_total"
BOUNDARY_METRICS = (SPAWN_DEFER_METRIC, CONSTANT_BASE_METRIC)

#: What a rise in each means, in the failure message. Written per family because
#: "a counter moved" is not a verdict a reader can act on: the two send an
#: operator to different halves of the system.
_MOVED_MEANS = {
    SPAWN_DEFER_METRIC: (
        "an incoming member could not obtain σ of the outgoing epoch (or its "
        "terminal block) and sat verify-only. The boundary was carried by the "
        "REMAINING quorum or by a retry, not by the transport — grep the node "
        "for 'signer spawn deferred', whose two INFO lines say which input was "
        "missing"),
    CONSTANT_BASE_METRIC: (
        "an incoming member elected leaders off the CONSTANT base "
        "(sha256(epoch ‖ sorted peers)) instead of inheriting σ. A σ miss cannot "
        "reach that branch — it defers the spawn — so this says the epoch "
        "predicate called the outgoing epoch beacon-INACTIVE, i.e. the schedule "
        "for this epoch is predictable an epoch ahead"),
}




# ── PURE VERDICT LAYER (docker-free, unit-tested) ─────────────────────────────

def evaluate_turnover_premise(before: str, after: str, wanted_in: set, wanted_out: set):
    """Did all four seats flip at ONE commit?

    Scored on the committee sets alone, and deliberately strict in both
    directions: the incoming set must be seated ENTIRELY and the outgoing set
    must be gone ENTIRELY. A partial flip is the failure this assert exists to
    name — it leaves an overlap, and an overlapping boundary is the case that
    already worked.
    """
    if not before or not after:
        return False, "committee unreadable — refusing to score a turnover over an empty read"
    before_set, after_set = set(before.split()), set(after.split())
    if not wanted_out <= before_set:
        missing = sorted(wanted_out - before_set)
        return False, f"premise: the outgoing seats were not seated to begin with ({missing})"
    still_in = wanted_out & after_set
    if still_in:
        return False, (f"premise: {len(still_in)} of {len(wanted_out)} outgoing seats survived "
                       f"the boundary ({sorted(still_in)}) — this boundary HAS overlap, so a "
                       "green verdict below would mean nothing")
    absent = wanted_in - after_set
    if absent:
        return False, (f"premise: {len(absent)} of {len(wanted_in)} incoming seats never seated "
                       f"({sorted(absent)}) — the flip was partial")
    return True, f"premise: all {len(wanted_in)} seats flipped at one commit"


def evaluate_turnover_case(premise, fin_before: int, fin_after: int, min_advance: int,
                           counters_before: dict, counters_after: dict):
    """Score the premise FIRST, then the conclusion.

    `counters_*` are `{service: {family: value}}` over [`BOUNDARY_METRICS`]; -1 means the
    node was unreadable OR did not render the family, and is carried as UNREAD rather
    than as zero. Both families are registered eagerly, so "not rendered" is a broken
    scrape, not a healthy never-incremented counter — and a metric nobody could read is
    not evidence that nothing happened.

    Every family is scored, and each has its OWN failure line: a defer and a
    constant-base election are different defects with different repairs, and folding
    them into "a counter moved" would hand the operator a number and no direction.
    """
    ok, why = premise
    if not ok:
        return False, why
    if fin_after - fin_before < min_advance:
        return False, (f"the chain did not advance across the zero-overlap boundary: "
                       f"finalized {fin_before}→{fin_after} (need >= {min_advance}) — this is "
                       "the halt the ticket exists to remove")
    for fam in BOUNDARY_METRICS:
        moved = {}
        for svc, after in counters_after.items():
            av = after.get(fam, -1)
            bv = counters_before.get(svc, {}).get(fam, -1)
            if av >= 0 and bv >= 0 and av > bv:
                moved[svc] = av - bv
        if moved:
            return False, (f"{fam} rose on {sorted(moved)} (by "
                           f"{sorted(moved.values())}) across the zero-overlap boundary — "
                           f"{_MOVED_MEANS[fam]}")
    read = [svc for svc, vals in counters_after.items()
            if all(vals.get(fam, -1) >= 0 for fam in BOUNDARY_METRICS)]
    if not read:
        return False, (f"{' and '.join(BOUNDARY_METRICS)} unreadable on every node — nothing "
                       "was verified. Both are registered at startup on every node class, so "
                       "an absent family is a dead :9100 scrape or a renamed metric, never a "
                       "healthy zero")
    return True, (f"{why}; finalized {fin_before}→{fin_after}, and neither "
                  f"{SPAWN_DEFER_METRIC} nor {CONSTANT_BASE_METRIC} moved on "
                  f"{len(read)} node(s)")


# ── ENV PROFILE ───────────────────────────────────────────────────────────────

def apply_case_env_defaults():
    """Eight validators, a committee of four, no spare band.

    `SIM_VALIDATORS=8` is the floor, not a choice: four seats have to leave and
    four different ones have to arrive, and neither side may take the committee
    below four.
    """
    prof = {
        "SIM_QUICK": "1",
        "SIM_VALIDATORS": "8",
        "SIM_INITIAL_COMMITTEE": "4",
        "SIM_SPARES": "0",
        "SIM_ROTATION_SLOTS": "0",
        "SIM_EPOCH_INTERVAL": "32",
        "SIM_NO_CASCADE": "1",
        "SIM_BYZANTINE": "0",
        "SIM_GEO_LATENCY": "0",
    }
    for k, v in prof.items():
        os.environ.setdefault(k, v)
    return {k: os.environ[k] for k in prof}


#: Floor for the stake put on each challenger, used only when the incumbents'
#: stakes cannot be read. The real figure is COMPUTED from the incumbents — see
#: `challenger_stake`.
CHALLENGER_STAKE_FLOOR = 20 * 10**18


def challenger_stake(incumbent_stakes) -> int:
    """Strictly more than any incumbent holds, by a wide margin.

    Computed rather than hard-coded, because a hard-coded figure is a claim about
    a stake table the case never read — and a wrong one fails as a PARTIAL flip,
    which reads like the defect under test. A live run at 20e18 flat left exactly
    one incumbent seated: selection is `top_k_by_stake_at` with a strict `>`, so
    one genesis validator holding more than the flat figure is all it takes.
    """
    known = [s for s in incumbent_stakes if s > 0]
    if not known:
        return CHALLENGER_STAKE_FLOOR
    return max(CHALLENGER_STAKE_FLOOR, 2 * max(known) + 10**18)

#: `WARMUP_DELAY` (2) + `MAX_COMMITTEE_LOOKAHEAD_EPOCHS` (2). A delegation landing
#: in epoch C is first VISIBLE to selection at C+2, and selection for target T
#: reads T-2 — so the first target that can see it is C+4. Computed rather than
#: hard-coded: the harness's older "+3" comment does not reconcile with the
#: contract's own arithmetic, and a case that hard-codes the wrong offset asserts
#: over the wrong epoch.
TURNOVER_LOOKAHEAD = 4


def first_target_epoch(landed_epoch: int) -> int:
    return landed_epoch + TURNOVER_LOOKAHEAD


_DRY_FIN = 100
_DRY_SERVICES = (topology.validator(0), topology.validator(4))


def run_case(argv=None) -> int:
    argv = list(argv or [])
    dry = "--dry-run" in argv
    unknown = [a for a in argv if a != "--dry-run"]
    if unknown:
        print(f"case-turnover: unrecognised argument(s) {unknown} (only --dry-run is accepted)",
              flush=True)
        return RC_USAGE

    prof = apply_case_env_defaults()

    from ..sim.orchestrator import SimConfig
    from ..stack.bringup import BringUp, restore_exported_env, save_exported_env
    from ..core.proc import Runner
    from ..chain.writes import Chain, ChainError
    from ..core.events import EventLog
    from ..core import nodes

    cfg = SimConfig()
    outgoing = list(range(cfg.initial_committee))
    incoming = list(range(cfg.initial_committee, cfg.validators))
    if len(incoming) < len(outgoing):
        print(f"CASE-TURNOVER SETUP ERROR: {len(incoming)} challengers for {len(outgoing)} seats "
              f"— zero overlap needs at least {2 * cfg.initial_committee} validators "
              f"(SIM_VALIDATORS={cfg.validators})", flush=True)
        return RC_USAGE

    interval = int(os.environ.get("SIM_EPOCH_INTERVAL", "32"))
    rpc = os.environ.get("RPC", topology.DEFAULT_RPC_URL)
    keep_up = os.environ.get("SIM_KEEP_UP", "0") == "1"
    env = {"RPC": rpc, "COMPOSE_FILE": os.environ.get("COMPOSE_FILE", ""),
           "CHAIN_ID": os.environ.get("CHAIN_ID", str(topology.CHAIN_ID))}
    runner = Runner(env=env, dry=dry, echo=dry)
    saved_env = save_exported_env()
    bu = BringUp(cfg.stack_spec(), runner)

    print(f"CASE-TURNOVER: profile {prof} — {outgoing} out, {incoming} in", flush=True)

    def teardown():
        try:
            bu.spammers.stop()
        except Exception:  # noqa: BLE001 — never let a spammer-stop mask the verdict
            pass
        if not keep_up:
            runner.run_ok(["docker", "compose", "down", "-v", "--remove-orphans"],
                          timeout=300, note="teardown-down")

    def measured(label: str, live, dry_value):
        if dry:
            runner.step("read", label)
            return dry_value
        return live()

    def fail(reason: str) -> int:
        print(f"CASE-TURNOVER FAIL: {reason}", flush=True)
        try:
            EventLog().bundle_dump(reason, "case-turnover")
        except Exception:  # noqa: BLE001 — a best-effort bundle must not mask the fault
            pass
        teardown()
        return RC_FAIL

    def boundary_counters(svc: str) -> dict:
        """Both [`BOUNDARY_METRICS`] off ONE read of <svc>'s :9100 scrape. -1 per family
        when the node could not be scraped OR did not render that family.

        ABSENT IS UNREAD HERE, and that inverts what this function used to do. The old
        family was a metrics-rs counter on reth's :9200, registered LAZILY — a node that
        never skipped a view never rendered the line, so absence was the healthy case and
        was scored 0. These two are commonware `ctx.register` counters, registered at
        startup whatever happens afterwards, so a healthy scrape ALWAYS renders them at 0.
        Absence therefore means the endpoint answered something that is not this node's
        metrics, or the family was renamed under us — and scoring that as a zero is the
        precise failure this whole rewrite exists to remove.

        ONE read, both families: two reads would let a counter move between them and be
        attributed to the wrong window. `beacon_metric_value` is the substring/$NF parse
        that already handles prometheus-client's doubled `_total` suffix
        (`epoch_engine_spawn_deferred_total_total`), which is why the constants stay the
        REGISTERED spelling.
        """
        # THE CONSENSUS EXPORTER, read on its own rather than out of the concatenated
        # text: a merged scrape that only :9200 answered would look "present and zero" for
        # a node whose :9100 is down. Reading the one exporter that owns both families
        # answers the question directly.
        host = topology.host_metrics_urls(svc)
        text = (nodes.metrics_get_url(host[0]) if host is not None
                else nodes.metrics_get_exec(topology.IN_CONTAINER_CONSENSUS_METRICS_URL,
                                            nodes.compose_exec(svc)))
        if not text.strip():
            return {fam: -1 for fam in BOUNDARY_METRICS}
        return {fam: nodes.beacon_metric_value(text, fam) for fam in BOUNDARY_METRICS}

    def counters(validators):
        return {svc: measured(f"node_metrics({svc})/{'+'.join(BOUNDARY_METRICS)}",
                              lambda s=svc: boundary_counters(s),
                              {fam: 0 for fam in BOUNDARY_METRICS})
                for svc in validators}

    try:
        bu.run()
        chain = Chain(runner=runner, RPC=rpc, STAKING_RT=bu.staking_rt,
                      CHAIN_CONFIG_RT=bu.chain_config_rt, GOV_ADDR=bu.gov_addr,
                      LIVENESS_RT=bu.liveness_rt, TOKEN=bu.token,
                      CHAIN_ID=env["CHAIN_ID"])
        os.environ.setdefault("PP_GOV_VOTERS", str(cfg.initial_committee))

        fin0 = measured("finalized_dec()", nodes.finalized_dec, _DRY_FIN)
        print(f"CASE-TURNOVER: baseline fin0={fin0}", flush=True)

        # The challengers must be SELECTION-VISIBLE before their stake can rank
        # them, and activation does not touch the cap — the committee stays four
        # seats wide throughout, which is what makes this a turnover rather than
        # a growth.
        # ONE governance round for all four activations, not four rounds.
        #
        # Measured, not guessed: four separate rounds got three activations
        # through and then failed the fourth with `state=` empty — the proposal
        # did not exist, i.e. `propose` itself reverted. Each activation adds the
        # joiner's own 3e18 to the delegated supply, so a series of rounds is
        # judged against a supply that grows underneath it. A batch is proposed
        # once, against the supply as it stood before any of them — and it also
        # says what the case means: these four seats arrive together.
        voters = None if dry else gov_live_voter_idx(
            chain.committee(chain.current_epoch()), chain.owner_addr, cfg.validators - 1)
        for idx in incoming:
            chain.register_setkeys(idx)
        chain.gov_action_batch(
            [chain.staking_rt] * len(incoming),
            [chain.calldata("activateValidator(address)", chain.owner_addr(idx))
             for idx in incoming],
            "activate-turnover-challengers",
            voter_idx=voters,
        )
        for idx in incoming:
            addr = chain.owner_addr(idx)
            if not dry and chain.validator_status(addr) != "1":
                return fail(f"challenger idx {idx} ({addr}) is not Active after the batch "
                            "activation — it cannot be selection-visible, so the flip cannot "
                            "happen for a reason unrelated to the transport")
        print(f"CASE-TURNOVER: challengers {incoming} are Active; cap unchanged", flush=True)

        # ONE epoch, four delegations. If they straddle a boundary the premise
        # assert below refuses the run — that is the point of asserting it.
        stakes = [measured(f"validator_stake(v{i})",
                           lambda i=i: chain.validator_stake(chain.owner_addr(i)), 0)
                  for i in outgoing]
        amount = challenger_stake(stakes)
        print(f"CASE-TURNOVER: incumbent stakes {stakes} ⇒ {amount} per challenger", flush=True)
        # Start the delegations at the TOP of an epoch, so all four have a full
        # interval of headroom to land in one selection epoch. Without this the
        # case is a race against the boundary — a live run straddled 1→2 and
        # failed its own premise for a reason that says nothing about the code
        # under test. The straddle assert below stays as the backstop.
        landed = measured("current_epoch()", chain.current_epoch, 1)
        if not dry:
            roll_deadline = time.time() + interval * 2 + 60
            while chain.current_epoch() == landed and time.time() < roll_deadline:
                time.sleep(2)
            landed = chain.current_epoch()
        print(f"CASE-TURNOVER: delegating at the top of epoch {landed}", flush=True)
        for idx in incoming:
            # Funded by the challenger's OWN owner, not owner-0: four delegations
            # of this size out of one purse is a budget the case would silently
            # share with everything else the harness funds.
            chain.stake_delegate(idx, amount, key=chain.owner_key(idx))
        landed_after = measured("current_epoch()", chain.current_epoch, 1)
        if landed_after != landed:
            return fail(f"premise: the four delegations straddled an epoch boundary "
                        f"({landed}→{landed_after}) — they must share one selection epoch")

        target = first_target_epoch(landed)
        # The LIVE committee, not `target - 1`: committees are committed at most
        # two epochs ahead (`evm.rs::drive_ahead_commit` breaks at
        # `current_epoch + 2`), and `target - 1` is three ahead of `landed`, so it
        # reads as an empty array and the premise would refuse every live run for
        # a reason that has nothing to do with the turnover.
        before = measured(f"committee({landed})", lambda: chain.committee(landed), "")
        print(f"CASE-TURNOVER: delegations landed in epoch {landed}; target epoch {target}",
              flush=True)

        running = measured("running_services()", nodes.running_services, list(_DRY_SERVICES))
        validators = [svc for svc in running if topology.is_validator(svc)]
        if not validators:
            return fail("`docker compose ps` returned no validators — refusing to score a "
                        "boundary over an empty node set")
        counters_before = counters(validators)

        deadline = time.time() + interval * (TURNOVER_LOOKAHEAD + 4) + 240
        now = measured("current_epoch()", chain.current_epoch, target)
        while now < target and time.time() < deadline:
            time.sleep(5)
            now = chain.current_epoch()
        if now < target:
            return fail(f"epoch {target} never arrived (stuck at {now}) — the chain stopped "
                        "before the boundary under test")

        after = measured(f"committee({target})", lambda: chain.committee(target), "")
        addr_in = {chain.owner_addr(i).lower() for i in incoming[:len(outgoing)]}
        addr_out = {chain.owner_addr(i).lower() for i in outgoing}
        premise = evaluate_turnover_premise(before, after, addr_in, addr_out)

        fin_after = measured("finalized_dec()", nodes.finalized_dec, fin0 + interval * 2)
        counters_after = counters(validators)

        if dry:
            teardown()
            print(f"# {len(runner.log)} commands")
            return RC_PASS
        print(f"CASE-TURNOVER: {'+'.join(BOUNDARY_METRICS)} before={counters_before} "
              f"after={counters_after}", flush=True)
        ok, reason = evaluate_turnover_case(premise, fin0, fin_after, interval,
                                            counters_before, counters_after)
        if not ok:
            return fail(reason)
        print(f"CASE-TURNOVER PASS: {reason}", flush=True)
        teardown()
        return RC_PASS

    except ChainError as e:
        return fail(f"chain error [{e.reason_id}]: {e.message}")
    except KeyboardInterrupt:
        print("CASE-TURNOVER: interrupted", flush=True)
        teardown()
        return 130
    finally:
        restore_exported_env(saved_env)
