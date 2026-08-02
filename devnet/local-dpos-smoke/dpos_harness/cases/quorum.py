"""quorum.py — the standalone quorum-loss SAFETY case (dpos_harness, deterministic).

WHY THIS EXISTS
    The BFT safety threshold says: with f+1 committee members down, finalization MUST
    halt (no quorum), and once they are restored it MUST resume. That property used to
    be checked by a once-per-run "quorum probe" spliced INTO the random churn sim. It
    was removed (v61 a12): deliberately dropping f+1 violated the ≤f-at-source invariant
    the sim now enforces everywhere, and — worse — its docker stop/start was unreliable
    under live churn (a stale COMPOSE_FILE made `docker compose stop/start <v>` silently
    no-op for some victims, and the restore only checked "the chain resumed", never that
    every stopped container actually came back). That produced a FALSE node-down hard-fail
    while consensus was in fact perfectly healthy.

    The safety property is real and worth testing — just not in the churn loop. This case
    proves it on a CLEAN, STABLE docker set where none of those churn hazards exist.

WHAT THIS CASE DOES (deterministic, SCRIPTED)
    1. Bring up a minimal fast devnet (n=7 committee, geo OFF, short epoch, no churn).
    2. Wait for DPoS active; record baseline.
    3. Stop EXACTLY f+1 committee members (never validator-0, the pinned RPC host).
    4. ASSERT STALL: finalized must PLATEAU (quorum lost) — an advancing finalized means
       the stop was ineffective / quorum was not actually lost = FAIL.
    5. Restart the f+1 victims and CONFIRM EACH is genuinely back up (its own in-container
       finalized read answers), retrying the start on any straggler. This per-victim
       verification is exactly what the old in-sim probe lacked.
    6. ASSERT RECOVERY: finalized must resume past the plateau.
    7. PASS/FAIL + teardown (unless SIM_KEEP_UP=1).

REUSE: BringUp.run(), nodes.finalized_dec / node_fin_in / running_services, the Runner
docker seam (with a LIVE COMPOSE_FILE overlay — the fix the in-sim probe never had),
core.events.EventLog for the failure bundle.
"""

from __future__ import annotations

import os
import time

from ..core import topology


# ── PURE VERDICT LAYER (docker-free, unit-tested in tests/test_case_quorum.py) ──

def evaluate_quorum_case(need: int, plateau_lo: int, plateau_hi: int,
                         victims_down: list, fin_recovered: int):
    """Pure verdict for the quorum-loss safety case (all live inputs pre-gathered). Returns
    (ok, reason).

      need         — f+1, the number of committee members stopped.
      plateau_lo   — finalized sampled after the settle drain (in-flight <=K finalizations done).
      plateau_hi   — finalized sampled after the observation window; a STALL requires plateau_hi
                     == plateau_lo (flat: quorum genuinely lost).
      victims_down — victims still NOT back up after the restore+retry budget (empty = all up).
      fin_recovered— finalized after restoring; RECOVERY requires it to exceed the plateau.

    Deterministic; no I/O."""
    if plateau_hi > plateau_lo:
        return (False, f"NO STALL: finalized advanced {plateau_lo}->{plateau_hi} with f+1={need} "
                       "committee members down — quorum was NOT lost (the stop was ineffective or "
                       "the committee is larger than assumed); the BFT safety threshold did not hold")
    if victims_down:
        return (False, f"RESTART FAILURE: {len(victims_down)} of {need} stopped victim(s) never came "
                       f"back up: {' '.join(victims_down)} (docker start silently failed / the node "
                       "did not rejoin) — the same failure the removed in-sim probe could not see")
    if fin_recovered <= plateau_hi:
        return (False, f"RECOVERY FAILURE: finalized did not resume past the plateau {plateau_hi} "
                       f"after restoring all {need} victims (got {fin_recovered}) — the chain did "
                       "not recover from a quorum loss")
    return (True, f"chain STALLED at f+1={need} down (plateau {plateau_hi}) then RECOVERED to "
                  f"{fin_recovered} with all {need} victims back up — BFT safety threshold holds")


# ── ENV PROFILE (minimal, fast, deterministic) ─────────────────────────────────

def apply_case_env_defaults():
    """Set the minimal fast SAFETY profile — a 7-member committee (f=2, f+1=3), geo OFF, short
    epoch, cascade OFF, byzantine OFF, NO growth/spares (a static committee = clean docker set,
    no SLOT_IDENTITY rebinding). setdefault so an operator override wins. Returns the profile."""
    prof = {
        "SIM_QUICK": "1",
        "SIM_VALIDATORS": "7",           # n=7 → f=(7-1)//3=2, f+1=3, quorum 2f+1=5
        "SIM_INITIAL_COMMITTEE": "7",    # whole set in committee — no growth, static membership
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


# ── LIVE POLL HELPERS ──────────────────────────────────────────────────────────

def _await_dpos_active(nodes, chain, deadline_s: int):
    """Wait until DPoS is active (epoch>=1) and finalized is advancing (two rising samples).
    Returns (fin, epoch)."""
    deadline = time.time() + deadline_s
    prev = -1
    rising = 0
    ep = fin = 0
    while time.time() < deadline:
        ep = chain.current_epoch()
        fin = nodes.finalized_dec()
        if ep >= 1 and fin > 0:
            if fin > prev:
                rising += 1
                prev = fin
            if rising >= 2:
                return fin, ep
        time.sleep(5)
    from ..chain.writes import ChainError
    raise ChainError("await-active",
                     f"DPoS not active/finalizing within {deadline_s}s (epoch={ep}, fin={fin})")


def _wait_victims_up(nodes, runner, compose_overlay, victims, budget_s, poll_s=3.0):
    """After restoring the victims, confirm EACH is genuinely back up (its own in-container
    finalized read answers > 0), retrying the idempotent `docker compose start` on any straggler.
    Returns the victims STILL down after the budget (empty = all up). This per-victim verification
    is the exact gap that let the removed in-sim probe declare "recovered" while a victim stayed
    down — the chain resumes the instant quorum returns, so f of the f+1 restarting is enough to
    hide a straggler."""
    deadline = time.time() + budget_s
    pending = list(victims)
    while pending:
        pending = [v for v in pending if nodes.node_fin_in(v) <= 0]
        if not pending or time.time() >= deadline:
            break
        for v in pending:
            runner.run(["docker", "compose", "start", v],
                       env_overlay=compose_overlay(), timeout=120, note=f"probe-restart-retry-{v}")
        time.sleep(poll_s)
    return pending


# ── THE CASE ───────────────────────────────────────────────────────────────────

def run_case(argv=None) -> int:
    prof = apply_case_env_defaults()

    from ..sim.orchestrator import SimConfig
    from ..stack.bringup import BringUp
    from ..core.proc import Runner
    from ..chain.writes import Chain, ChainError
    from ..core.events import EventLog
    from ..core import nodes

    cfg = SimConfig()
    n = cfg.initial_committee
    f = (n - 1) // 3
    need = f + 1
    if need >= n:
        print(f"CASE-QUORUM SETUP ERROR: committee n={n} too small for an f+1={need} drop "
              "(raise SIM_INITIAL_COMMITTEE)", flush=True)
        return 2

    interval = int(os.environ.get("SIM_EPOCH_INTERVAL", "32"))
    rpc = os.environ.get("RPC", topology.DEFAULT_RPC_URL)
    keep_up = os.environ.get("SIM_KEEP_UP", "0") == "1"
    # NB: do NOT seed self.env with COMPOSE_FILE — a start-time snapshot would be empty and, being
    # layered OVER os.environ, would blank the fresh value bringup exports. Every docker call below
    # instead passes a LIVE compose overlay read at call time (the fix the in-sim probe lacked).
    runner = Runner(env={"RPC": rpc, "CHAIN_ID": os.environ.get("CHAIN_ID", str(topology.CHAIN_ID))})
    bu = BringUp(cfg.stack_spec(), runner)

    def compose_overlay():
        return {"COMPOSE_FILE": os.environ.get("COMPOSE_FILE", "")}

    # Static committee (no churn) ⇒ members are validator-0..validator-(n-1); victims are the first
    # f+1 NON-zero indices (validator-0 is the pinned RPC host — never stop it).
    victims = [topology.validator(i) for i in range(1, need + 1)]

    print(f"CASE-QUORUM: profile {prof} — n={n} f={f} → stopping f+1={need}: {' '.join(victims)}",
          flush=True)

    def teardown():
        try:
            bu.spammers.stop()
        except Exception:  # noqa: BLE001
            pass
        if not keep_up:
            runner.run_ok(["docker", "compose", "down", "-v", "--remove-orphans"],
                          timeout=300, note="teardown-down")
        else:
            print("CASE-QUORUM: SIM_KEEP_UP=1 — leaving the stack up", flush=True)

    def fail(reason: str) -> int:
        print(f"CASE-QUORUM FAIL: {reason}", flush=True)
        try:
            EventLog().bundle_dump(reason, "case-quorum")
        except Exception:  # noqa: BLE001
            pass
        teardown()
        return 1

    try:
        bu.run()
        chain = Chain(runner=runner, RPC=rpc, STAKING_RT=bu.staking_rt,
                      CHAIN_CONFIG_RT=bu.chain_config_rt, GOV_ADDR=bu.gov_addr,
                      LIVENESS_RT=bu.liveness_rt, TOKEN=bu.token,
                      CHAIN_ID=os.environ.get("CHAIN_ID", str(topology.CHAIN_ID)))

        fin0, epoch0 = _await_dpos_active(nodes, chain, deadline_s=300)
        print(f"CASE-QUORUM: DPoS active — baseline fin0={fin0} epoch0={epoch0}", flush=True)

        # 1. STOP f+1 committee members.
        for v in victims:
            runner.run(["docker", "compose", "stop", "--timeout", "40", v],
                       env_overlay=compose_overlay(), timeout=120, note=f"probe-stop-{v}")
        print(f"CASE-QUORUM: stopped f+1={need} members — expecting a finalized plateau", flush=True)

        # 2. CONFIRM STALL: drain in-flight (<=K) finalizations, then require a flat plateau.
        settle_s = 5 + 2 * int(os.environ.get("RESULT_LAG_K", "3"))
        window_s = 8 + 2 * int(os.environ.get("RESULT_LAG_K", "3"))
        time.sleep(settle_s)
        plateau_lo = nodes.finalized_dec()
        time.sleep(window_s)
        plateau_hi = nodes.finalized_dec()
        print(f"CASE-QUORUM: plateau samples {plateau_lo} -> {plateau_hi} "
              f"({'FLAT — quorum lost' if plateau_hi <= plateau_lo else 'ADVANCING — no stall'})",
              flush=True)

        # 3. RESTORE + per-victim up-confirm (the gap the old probe had).
        for v in victims:
            runner.run(["docker", "compose", "start", v],
                       env_overlay=compose_overlay(), timeout=120, note=f"probe-start-{v}")
        recover_budget = 180 + cfg.validators * 30
        down = _wait_victims_up(nodes, runner, compose_overlay, victims, recover_budget)
        if down:
            return fail(evaluate_quorum_case(need, plateau_lo, plateau_hi, down, plateau_hi)[1])
        print(f"CASE-QUORUM: all {need} victims confirmed back up — awaiting finalized resume",
              flush=True)

        # 4. CONFIRM RECOVERY: finalized must climb past the plateau.
        target = plateau_hi + 1
        deadline = time.time() + recover_budget
        fin_recovered = nodes.finalized_dec()
        while fin_recovered < target and time.time() < deadline:
            time.sleep(5)
            fin_recovered = nodes.finalized_dec()

        # 5. verdict.
        ok, reason = evaluate_quorum_case(need, plateau_lo, plateau_hi, [], fin_recovered)
        if not ok:
            return fail(reason)
        print(f"CASE-QUORUM PASS: {reason}", flush=True)
        teardown()
        return 0

    except ChainError as e:
        return fail(f"chain error [{e.reason_id}]: {e.message}")
    except KeyboardInterrupt:
        print("CASE-QUORUM: interrupted", flush=True)
        teardown()
        return 130
