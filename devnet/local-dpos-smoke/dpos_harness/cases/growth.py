"""growth.py — the "bug-B" committee-GROWTH regression case (dpos_harness, deterministic).

WHAT BUG B WAS
    During a committee GROWTH the DKG dealer-log idx was numbered against a
    LIVE-RECOMPUTED selection view that could exceed the COMMITTED committee size,
    so a proposer emitted a `dkg_logs` entry with `idx >= committee[E+1].len()`.
    Every verifier then voted false (application.rs::beacon_gate_decision:
    "dkg_logs: an idx is out of committee[E+1] range — voting false") and the epoch
    boundary FINALIZE-STALLED. The fix (2-epoch warm-up) freezes the committee once
    so record and verify read the SAME slot → the idx is always in range.

WHAT THIS CASE DOES (deterministic, SCRIPTED — not the random churn sim)
    1. Bring up a MINIMAL fast devnet (few validators, geo-latency OFF, short epoch,
       cascade OFF) via the shared Bringup, reusing SimConfig + the env contract.
    2. Wait for DPoS active + a few finalized blocks; record baseline fin0 + epoch0.
    3. Force a committee GROWTH across an epoch boundary — register_activate(idx,
       raise_cap=1) for the growth joiners (idx initial_committee..validators-1),
       each spaced so the change lands at a boundary (the exact a14 trigger).
       Confirm the growth LANDS: activeValidatorsLength() increases AND the new
       validator seats in the live committee.
    4. ASSERT LIVENESS: finalized_dec() must advance by >= a full epoch's worth of
       blocks within a deadline spanning several boundaries. A flat finalized =
       finalize-stall = bug B present = FAIL.
    5. ASSERT NO BUG-B SIGNATURE: scan EVERY running validator's logs for the
       dkg_logs idx-out-of-committee line. Presence = FAIL.
    6. On FAIL: dump a bundle (reuse core/events.py) and exit non-zero. On PASS: print
       the CASE-GROWTH PASS line and exit 0. Always teardown unless SIM_KEEP_UP=1.

REUSE, do not reinvent: Bringup.run() (bring-up), Chain.register_activate /
active_validators_length (the growth actuator), nodes.finalized_dec / logs_since /
running_services (liveness + log reads), core.events.EventLog (the failure bundle).
"""

from __future__ import annotations

import os
import re
import time

from ..core import topology


# ── PURE VERDICT LAYER (docker-free, unit-tested in tests/test_case_growth.py) ──

# The definitive bug-B fingerprint, emitted by every verifier that rejects an
# over-large dealer-log idx (crates/dpos/consensus/src/application.rs:1100). A
# substring match is version-robust (the surrounding height=/epoch=/n= tracing
# fields vary; this phrase is pinned to the reject).
BUGB_IDX_SIGNATURE = "idx is out of committee"


def scan_idx_stall(log_text: str):
    """Return the list of log lines carrying the bug-B dkg_logs idx-out-of-range
    signature. Non-empty ⇒ a proposer numbered a dealer-log idx against a stale /
    over-large selection view → every verifier votes false → finalize-stall."""
    return [ln.strip() for ln in log_text.splitlines() if BUGB_IDX_SIGNATURE in ln]


def cluster_verify_false(log_text: str, needle: str = "voting false"):
    """Count `needle` verify-rejections per block height (the tracing `height=<N>`
    field), returning {height: count}. A single height carrying a BURST of
    rejections is the finalize-stall fingerprint — the whole committee rejecting
    the SAME proposal. Lines with no parseable height bucket under -1."""
    out: dict[int, int] = {}
    for ln in log_text.splitlines():
        if needle not in ln:
            continue
        m = re.search(r"height[=:\s]+(\d+)", ln)
        h = int(m.group(1)) if m else -1
        out[h] = out.get(h, 0) + 1
    return out


def evaluate_growth_case(fin0: int, fin_now: int, min_advance: int, per_node_logs: dict):
    """Pure verdict for the growth case (both live inputs pre-gathered). Returns
    (ok, reason). FAILS if finalized did not advance by `min_advance` across the
    growth window (finalize-stall) OR any node emitted the bug-B idx-out-of-range
    signature. Deterministic; no I/O."""
    advanced = fin_now - fin0
    if advanced < min_advance:
        return (False, f"finalize-stall: finalized advanced {advanced} block(s) < required "
                       f"{min_advance} across the growth window (fin0={fin0} finN={fin_now}) — "
                       "the boundary did not finalize (bug B present)")
    hits = {svc: lines for svc, lines in
            ((s, scan_idx_stall(t)) for s, t in per_node_logs.items()) if lines}
    if hits:
        detail = "; ".join(f"{s}: {len(v)} line(s) e.g. {v[0]!r}" for s, v in hits.items())
        return (False, f"bug-B dkg_logs idx-out-of-committee signature present — {detail}")
    return (True, f"finalized advanced {advanced} block(s) (>= {min_advance}); no idx-stall "
                  f"across {len(per_node_logs)} validator node(s)")


# ── ENV PROFILE (minimal, fast, deterministic) ─────────────────────────────────

def apply_case_env_defaults():
    """Set the minimal fast GROWTH profile — few validators, geo-latency OFF, short
    epoch, cascade OFF, byzantine OFF — WITHOUT clobbering an operator override
    (setdefault). initial_committee < validators is what leaves room for growth
    (register_activate caps at SIM_VALIDATORS). Returns the resolved profile dict."""
    prof = {
        "SIM_QUICK": "1",                # fast profile overlay
        "SIM_VALIDATORS": "6",           # target active-set size (room to grow 4→6)
        "SIM_INITIAL_COMMITTEE": "4",    # start (INITIAL_F=(4-1)//3=1, the >=1 floor)
        "SIM_SPARES": "0",               # no spare band — every container is a committee slot
        "SIM_ROTATION_SLOTS": "0",       # no rotation slots — val_containers == validators == 6
        "SIM_EPOCH_INTERVAL": "32",      # 32 blocks/epoch ≈ 32s at 1 blk/s — quick boundaries
        "SIM_NO_CASCADE": "1",           # skip the L3 sentry cascade (faster, fewer containers)
        "SIM_BYZANTINE": "0",            # scripted growth only — no fault lottery
        "SIM_GEO_LATENCY": "0",          # geo-latency OFF
    }
    for k, v in prof.items():
        os.environ.setdefault(k, v)
    # register_activate reads SIM_VALIDATORS as the cap ceiling; keep it consistent.
    return {k: os.environ[k] for k in prof}


def golden_spec():
    """Build the `stack.golden.GoldenSpec` this case's snapshot is keyed to.

    The golden snapshot has always been baked against THIS case's profile — `golden.py` used to
    reach up into this module and call `apply_case_env_defaults()` itself, which is what made
    `stack` depend on `cases`. The dependency is real; only its direction was wrong. The profile
    is applied here first, exactly as it was there, so a `--check` from a clean shell still
    hashes the config the build baked with.

    The `epoch_interval` fallback of "32" is carried over verbatim from `_golden_config`; it is
    unreachable in practice (the profile above already setdefault's SIM_EPOCH_INTERVAL), and
    removing it belongs to the P4 env-default cleanup, not here."""
    from ..sim.orchestrator import SimConfig
    from ..stack.golden import GoldenSpec

    apply_case_env_defaults()
    cfg = SimConfig()
    return GoldenSpec(
        config={
            "validators": cfg.validators,
            "initial_committee": cfg.initial_committee,
            "val_containers": cfg.val_containers,
            "identity_pool": cfg.identity_pool,
            "epoch_interval": int(os.environ.get("SIM_EPOCH_INTERVAL", "32")),
        },
        stack_spec=cfg.stack_spec(),
        validators=cfg.validators,
        val_containers=cfg.val_containers,
        identity_pool=cfg.identity_pool,
        await_dpos_active=_await_dpos_active,
    )


# ── LIVE POLL HELPERS (bounded; return None/raise on the deadline) ─────────────

def _await_dpos_active(chain, nodes, deadline_s: int):
    """Wait until DPoS is active (epoch>=1) AND finalized is advancing (two rising
    samples). Returns (fin, epoch). Bringup.run() already converges past the anchor,
    so this normally returns on the first poll — it is the explicit readiness gate."""
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


def _await_cap_increase(chain, cap_before: int, deadline_s: int):
    """Poll activeValidatorsLength until it exceeds cap_before; return the new cap
    or None on the deadline (growth did not land on-chain)."""
    deadline = time.time() + deadline_s
    while time.time() < deadline:
        cap = chain.active_validators_length()
        if cap > cap_before:
            return cap
        time.sleep(5)
    return None


def _await_live_seat(chain, addr: str, deadline_s: int):
    """Poll the LIVE committee until it contains addr (the growth warm-up lands the
    new member at the boundary). Return the epoch it seated at, or None on deadline.
    Crossing to the seat naturally spans the 2-epoch warm-up boundaries."""
    deadline = time.time() + deadline_s
    while time.time() < deadline:
        cur = chain.current_epoch()
        if chain.committee_has(addr, cur):
            return cur
        time.sleep(5)
    return None


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
    growth_joiners = list(range(cfg.initial_committee, cfg.validators))
    if not growth_joiners:
        print(f"CASE-GROWTH SETUP ERROR: initial_committee={cfg.initial_committee} >= "
              f"validators={cfg.validators} — no room for a growth (raise SIM_VALIDATORS)",
              flush=True)
        return 2

    interval = int(os.environ.get("SIM_EPOCH_INTERVAL", "32"))
    rpc = os.environ.get("RPC", topology.DEFAULT_RPC_URL)
    keep_up = os.environ.get("SIM_KEEP_UP", "0") == "1"
    env = {"RPC": rpc, "COMPOSE_FILE": os.environ.get("COMPOSE_FILE", ""),
           "CHAIN_ID": os.environ.get("CHAIN_ID", str(topology.CHAIN_ID))}
    runner = Runner(env=env)
    bu = BringUp(cfg.stack_spec(), runner)

    print(f"CASE-GROWTH: profile {prof} — growth joiners {growth_joiners} "
          f"(committee {cfg.initial_committee} → {cfg.validators})", flush=True)

    def teardown():
        try:
            bu.spammers.stop()
        except Exception:  # noqa: BLE001 — never let a spammer-stop mask the verdict
            pass
        if not keep_up:
            runner.run_ok(["docker", "compose", "down", "-v", "--remove-orphans"],
                          timeout=300, note="teardown-down")
        else:
            print("CASE-GROWTH: SIM_KEEP_UP=1 — leaving the stack up", flush=True)

    def fail(reason: str) -> int:
        print(f"CASE-GROWTH FAIL: {reason}", flush=True)
        try:
            EventLog().bundle_dump(reason, "case-growth")
        except Exception:  # noqa: BLE001 — a best-effort bundle must not mask the fault
            pass
        teardown()
        return 1

    try:
        # FAST PATH (opt-in): SIM_USE_GOLDEN=1 + a fresh golden snapshot restores the
        # DPoS-active state in seconds instead of the ~2-4 min full boot. Silently falls
        # back to the full boot when the golden is stale/absent.
        use_golden = os.environ.get("SIM_USE_GOLDEN", "0") == "1"
        if use_golden:
            from ..stack import golden
            spec = golden_spec()
            if golden.is_golden_fresh(spec):
                print("CASE-GROWTH: SIM_USE_GOLDEN=1 and golden is fresh — restoring snapshot "
                      "(skipping full boot)", flush=True)
                bu.run_from_golden(lambda runner: golden.restore_golden(spec, runner),
                                   golden.load_facts)
            else:
                print("CASE-GROWTH: SIM_USE_GOLDEN=1 but golden is stale/absent — full boot",
                      flush=True)
                bu.run()
        else:
            bu.run()
        chain = Chain(runner=runner, RPC=rpc, STAKING_RT=bu.staking_rt,
                      CHAIN_CONFIG_RT=bu.chain_config_rt, GOV_ADDR=bu.gov_addr,
                      LIVENESS_RT=bu.liveness_rt, TOKEN=bu.token,
                      CHAIN_ID=env["CHAIN_ID"])
        # gov voter prefix = the initial committee (bringup already exported PP_GOV_VOTERS).
        os.environ.setdefault("PP_GOV_VOTERS", str(cfg.initial_committee))

        # 2. readiness baseline
        fin0, epoch0 = _await_dpos_active(chain, nodes, deadline_s=300)
        print(f"CASE-GROWTH: DPoS active — baseline fin0={fin0} epoch0={epoch0}", flush=True)

        # 3. SCRIPTED growth across boundaries. Capture a log cursor so the scan is
        #    scoped to the growth window (where bug B would fire), not the whole bring-up.
        growth_start = time.monotonic()
        boundaries = 0
        cap = chain.active_validators_length()
        if cap <= 0:
            return fail("could not read activeValidatorsLength at baseline")
        for j, idx in enumerate(growth_joiners):
            ep_at = chain.current_epoch()
            print(f"CASE-GROWTH: growth #{j + 1}/{len(growth_joiners)} — register+activate "
                  f"idx {idx} at epoch {ep_at} (cap {cap})", flush=True)
            chain.register_activate(idx, raise_cap=1)

            new_cap = _await_cap_increase(chain, cap, deadline_s=interval * 4 + 120)
            if new_cap is None:
                return fail(f"growth idx {idx}: activeValidatorsLength never rose above {cap} "
                            "— the cap bump did not land on-chain")
            addr = chain.owner_addr(idx)
            seated = _await_live_seat(chain, addr, deadline_s=interval * 5 + 120)
            if seated is None:
                return fail(f"growth idx {idx} ({addr}): never seated in the live committee after "
                            f"the cap grew {cap}→{new_cap} — growth did not reach the committee")
            print(f"CASE-GROWTH: growth #{j + 1} LANDED — cap {cap}→{new_cap}, idx {idx} seated at "
                  f"epoch {seated} (crossed a boundary from {ep_at})", flush=True)
            boundaries += 1
            cap = new_cap

        # 4. LIVENESS across several boundaries: require finalized to advance by a full
        #    epoch's worth of blocks (a flat finalized = finalize-stall = bug B).
        min_advance = interval
        target = fin0 + min_advance
        live_deadline = time.time() + interval * 8
        fin_now = nodes.finalized_dec()
        while fin_now < target and time.time() < live_deadline:
            time.sleep(5)
            fin_now = nodes.finalized_dec()

        # 5. NO BUG-B SIGNATURE: scan every running validator's logs over the growth window.
        window = f"{int(time.monotonic() - growth_start) + 60}s"
        per_node_logs = {svc: nodes.logs_since(svc, window)
                         for svc in nodes.running_services() if topology.is_validator(svc)}
        print(f"CASE-GROWTH: scanned {len(per_node_logs)} validator log(s) over the last {window}",
              flush=True)

        # 6. verdict
        ok, reason = evaluate_growth_case(fin0, fin_now, min_advance, per_node_logs)
        if not ok:
            return fail(reason)
        span = (fin_now - fin0) // max(interval, 1)
        print(f"CASE-GROWTH PASS: committee grew across {boundaries} boundaries, finalized "
              f"advanced fin0={fin0}→finN={fin_now} (~{span} epoch-span), no dkg_logs idx-stall",
              flush=True)
        teardown()
        return 0

    except ChainError as e:
        return fail(f"chain error [{e.reason_id}]: {e.message}")
    except KeyboardInterrupt:
        print("CASE-GROWTH: interrupted", flush=True)
        teardown()
        return 130
