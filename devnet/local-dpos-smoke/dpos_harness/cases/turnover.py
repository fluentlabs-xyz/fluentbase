"""turnover.py — the ZERO-OVERLAP committee boundary (FLU-1203), deterministic.

The property under test is a halt that nothing in the tree enforces or detects.
At an epoch boundary the first block of E+1 must carry σ of epoch E, and until
FLU-1203 the only source of that σ was this node's own engine during E. A
committee with ZERO overlap with its predecessor therefore had nothing to
witness with: every leader of the new committee skipped its view and the chain
stopped.

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

#: The propose-side witness miss. It is incremented once per skipped view at a
#: boundary, so on a healthy zero-overlap boundary it must not move at all.
#:
#: A metrics-rs counter, so it renders on RETH's :9200 exporter, NOT on the
#: commonware registry at :9100 — `nodes.beacon_metric` reads :9100 only and
#: would answer -1 forever. Read from the EL exporter directly, see `skip_count`.
BOUNDARY_SKIP_METRIC = "dpos_parent_seed_boundary_skip_total"




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
                           skips_before: dict, skips_after: dict):
    """Score the premise FIRST, then the conclusion.

    `skips_*` are per-node readings of [`BOUNDARY_SKIP_METRIC`]; -1 means the
    node was unreadable and is carried as UNREAD rather than as zero — a metric
    nobody could read is not evidence that nothing happened.
    """
    ok, why = premise
    if not ok:
        return False, why
    if fin_after - fin_before < min_advance:
        return False, (f"the chain did not advance across the zero-overlap boundary: "
                       f"finalized {fin_before}→{fin_after} (need >= {min_advance}) — this is "
                       "the halt the ticket exists to remove")
    moved = {svc: after - skips_before.get(svc, 0)
             for svc, after in skips_after.items()
             if after >= 0 and skips_before.get(svc, -1) >= 0 and after > skips_before.get(svc, 0)}
    if moved:
        return False, (f"{BOUNDARY_SKIP_METRIC} moved on {sorted(moved)} — a leader still could "
                       "not witness its parent, so the boundary was carried by luck or by a "
                       "retry, not by the transport")
    read = [svc for svc, v in skips_after.items() if v >= 0]
    if not read:
        return False, f"{BOUNDARY_SKIP_METRIC} unreadable on every node — nothing was verified"
    return True, (f"{why}; finalized {fin_before}→{fin_after} and no boundary skip on "
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

    def skip_count(svc: str) -> int:
        """-1 only when the node could not be SCRAPED.

        An absent family is 0, not unread: this is a metrics-rs counter and
        metrics-rs registers lazily, so a node that never skipped a view never
        renders the line at all — which is the healthy case, and the one the
        conclusion assert is about. Distinguishing the two needs the raw text
        (empty ⇒ nothing answered), exactly as `nodes.refill_spare_attempts`
        does for its own lazily-registered family.
        """
        # THE EL EXPORTER, read on its own rather than out of the concatenated
        # text. This family is a metrics-rs counter and lives on reth's :9200; a
        # merged scrape that only :9100 answered would look "present and zero"
        # for a node whose :9200 is down, and sniffing the merged text for a
        # marker would be a guess about what the other registry renders. Reading
        # the one exporter that owns the family answers the question directly.
        host = topology.host_metrics_urls(svc)
        text = (nodes.metrics_get_url(host[1]) if host is not None
                else nodes.metrics_get_exec(topology.IN_CONTAINER_EL_METRICS_URL,
                                            nodes.compose_exec(svc)))
        if not text.strip():
            return -1
        # Absent ⇒ 0, not unread: metrics-rs registers lazily, so a node that
        # never skipped a view never renders the line at all — which is the
        # healthy case and the one this assert is about.
        value = nodes.beacon_metric_value(text, BOUNDARY_SKIP_METRIC)
        return 0 if value < 0 else value

    def skips(validators):
        return {svc: measured(f"node_metrics({svc})/{BOUNDARY_SKIP_METRIC}",
                              lambda s=svc: skip_count(s), 0)
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
        skips_before = skips(validators)

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
        skips_after = skips(validators)

        if dry:
            teardown()
            print(f"# {len(runner.log)} commands")
            return RC_PASS
        ok, reason = evaluate_turnover_case(premise, fin0, fin_after, interval,
                                            skips_before, skips_after)
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
