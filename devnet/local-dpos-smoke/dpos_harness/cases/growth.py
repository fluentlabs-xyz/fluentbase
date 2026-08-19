"""growth.py — the "bug-B" committee-GROWTH regression case (dpos_harness, deterministic).

WHAT BUG B WAS
    During a committee GROWTH the DKG dealer-log idx was numbered against a
    LIVE-RECOMPUTED selection view that could exceed the COMMITTED committee size,
    so a proposer emitted a `dkg_logs` entry with `idx >= committee[E+1].len()`.
    Every verifier then voted false (application.rs::beacon_gate_decision:
    "dkg_logs: an idx is out of committee[E+1] range — voting false") and the epoch
    boundary FINALIZE-STALLED. The fix (2-epoch warm-up) freezes the committee once
    so record and verify read the SAME slot → the idx is always in range.

WHERE ITS FINGERPRINT LIVES NOW
    `beacon_gate_decision` and the whole `dkg_logs` block field are gone with the
    epoch key's departure from `OrderBlock`, so that WARN cannot be emitted and an
    absence-grep for it is green against every possible chain. The condition it named
    is still detectable, one layer down: the pinned dealer-log set is agreed on the
    epoch-key agreement plane, and an index in it with no seat in the committed
    committee is counted on `dpos_dkg_pinned_idx_out_of_range_total` and reported by
    an ERROR when the ceremony then misses its settle deadline. Step 5 below asserts
    both, per validator. See the comment above `BUGB_IDX_METRIC`.

WHAT THIS CASE DOES (deterministic, SCRIPTED — not the random churn sim)
    1. Bring up a MINIMAL fast devnet (few validators, geo-latency OFF, short epoch,
       cascade OFF) via the shared Bringup, reusing SimConfig + the env contract.
    2. Wait for DPoS active + a few finalized blocks; record baseline fin0 + epoch0.
    3. Force a committee GROWTH across an epoch boundary — register_activate(idx,
       raise_cap=1) for the growth joiners (idx initial_committee..validators-1),
       each spaced so the change lands at a boundary (the exact a14 trigger), and
       each voting with the CURRENT committee (`growth_voter_idx`) because a joiner
       holds 3e18 of a stake-weighted quorum and would otherwise never vote.
       Confirm the growth LANDS: activeValidatorsLength() increases AND the new
       validator seats in the live committee.
    4. ASSERT LIVENESS: finalized_dec() must advance by >= a full epoch's worth of
       blocks within a deadline spanning several boundaries. A flat finalized =
       finalize-stall = bug B present = FAIL.
    5. ASSERT NO PINNED-IDX STALL: on EVERY running validator, read
       `dpos_dkg_pinned_idx_out_of_range_total` (non-zero = FAIL, unreadable on all of
       them = FAIL) and scan its log for the pinned-idx ERROR (presence = FAIL).
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
from ..core.exit_codes import RC_FAIL, RC_PASS, RC_USAGE
from ..core.policy import gov_live_voter_idx


# ── PURE VERDICT LAYER (docker-free, unit-tested in tests/test_case_growth.py) ──

# THE BUG-B FINGERPRINT MOVED, AND THE OLD ONE HAD ZERO HITS.
#
# It used to be `"idx is out of committee"`, the WARN a verifier emitted from
# `application.rs::beacon_gate_decision` when a proposed `dkg_logs` entry named an idx
# past the end of `committee[E+1]`. That whole verify path is GONE with the epoch key's
# departure from `OrderBlock` — a block carries no `dkg_logs` and asserts nothing about
# the beacon — so the string occurs NOWHERE in the tree, and an ABSENCE verdict over a
# string that cannot be emitted passes by construction, on every input, forever.
#
# The MECHANISM did not go away; it moved to the ceremony. The pinned dealer-log set is
# agreed on the epoch-key agreement plane, and `Ceremony::scoped_pinned_logs` SKIPS a
# pinned idx that has no seat in the committed committee (holding it would wedge the
# ceremony silently). The actor counts every such skip on
# `dpos_dkg_pinned_idx_out_of_range_total` and, when the ceremony then misses its settle
# deadline, logs an ERROR naming it.
#
# The COUNTER is the primary witness — it fires on every occurrence — and the log line is
# the secondary one, because it is emitted once per (epoch, reason) and only on the
# deferral path, so the counter can be non-zero with no line. The sim's invariant battery
# asserts the same counter (`checks/battery._inv_dkg_pinned_idx`); this is the
# scripted-case half of the same detector.
BUGB_IDX_METRIC = "dpos_dkg_pinned_idx_out_of_range_total"
BUGB_IDX_SIGNATURE = "names indices outside the committed committee"


def scan_idx_stall(log_text: str):
    """Return the log lines carrying the pinned-idx-out-of-committee ERROR. Non-empty ⇒
    the consensus-agreed pinned set named a dealer position that does not exist in the
    committed committee, the ceremony skipped it, and the DKG then missed its settle
    deadline — the 2026-07-21 idx-stall class in its surviving form."""
    return [ln.strip() for ln in log_text.splitlines() if BUGB_IDX_SIGNATURE in ln]


def evaluate_idx_metric(per_node: dict):
    """Verdict over `{service: dpos_dkg_pinned_idx_out_of_range_total}`, where a value
    below 0 means the scrape did not answer (`nodes.beacon_metric`'s -1 sentinel).

    Returns `(ok, reason)`. TWO ways to go red, and the second is the point:

      * any node reporting >= 1 — a pinned idx had no seat in the committed committee;
      * NO node answering at all — an unreadable counter is not a zero one, and "no
        idx-stall" over a detector that never spoke is the exact false-green this
        verdict replaced.

    One readable node is enough to assert, following `battery._inv_dkg_pinned_idx`: the
    pinned set is AGREED data, so the condition is chain-wide and any honest member that
    can be scraped sees it. Unreadable nodes are named in the PASS reason too, so a
    thinning detector is visible before it reaches zero."""
    if not per_node:
        return (False, f"{BUGB_IDX_METRIC}: no validator was scraped at all — refusing to "
                       "report 'no idx-stall' over an empty detector")
    hot = {s: v for s, v in per_node.items() if v >= 1}
    if hot:
        detail = ", ".join(f"{s}={v}" for s, v in sorted(hot.items()))
        return (False, f"{BUGB_IDX_METRIC} non-zero ({detail}) — a consensus-pinned dealer-log "
                       "index named a position OUTSIDE the committed committee and the ceremony "
                       "skipped it (the 2026-07-21 idx-stall class). MUST be 0.")
    readable = [s for s, v in per_node.items() if v >= 0]
    if not readable:
        return (False, f"{BUGB_IDX_METRIC} was unreadable on ALL {len(per_node)} scanned "
                       f"validator(s) ({', '.join(sorted(per_node))}) — an unread counter is not "
                       "a zero one, so the idx-stall property was never evaluated")
    blind = sorted(set(per_node) - set(readable))
    note = f" ({len(blind)} unreadable: {', '.join(blind)})" if blind else ""
    return (True, f"{BUGB_IDX_METRIC}=0 on {len(readable)}/{len(per_node)} validator(s){note}")


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


def evaluate_growth_case(fin0: int, fin_now: int, min_advance: int, per_node_logs: dict,
                         per_node_idx_metric: dict):
    """Pure verdict for the growth case (every live input pre-gathered). Returns
    (ok, reason). Deterministic; no I/O.

    THREE ways to go red:

      * finalized did not advance by `min_advance` across the growth window — the
        finalize-stall bug B produced. The verify-false HEIGHT CLUSTERS ride the reason,
        because a burst of rejections at ONE height is what separates "the whole committee
        rejected the same proposal" from "the chain is merely slow";
      * `dpos_dkg_pinned_idx_out_of_range_total` non-zero anywhere, or unreadable
        everywhere (see `evaluate_idx_metric`);
      * the pinned-idx ERROR line present in any node's log."""
    advanced = fin_now - fin0
    if advanced < min_advance:
        clusters: dict[int, int] = {}
        for text in per_node_logs.values():
            for height, n in cluster_verify_false(text).items():
                clusters[height] = clusters.get(height, 0) + n
        worst = sorted(clusters.items(), key=lambda kv: -kv[1])[:3]
        burst = ("; verify-false bursts by height: "
                 + ", ".join(f"h={h}x{n}" for h, n in worst)) if worst else ""
        return (False, f"finalize-stall: finalized advanced {advanced} block(s) < required "
                       f"{min_advance} across the growth window (fin0={fin0} finN={fin_now}) — "
                       f"the boundary did not finalize{burst}")
    ok, reason = evaluate_idx_metric(per_node_idx_metric)
    if not ok:
        return (False, reason)
    hits = {svc: lines for svc, lines in
            ((s, scan_idx_stall(t)) for s, t in per_node_logs.items()) if lines}
    if hits:
        detail = "; ".join(f"{s}: {len(v)} line(s) e.g. {v[0]!r}" for s, v in hits.items())
        return (False, f"pinned-idx-out-of-committee ERROR present — {detail}")
    return (True, f"finalized advanced {advanced} block(s) (>= {min_advance}); {reason}; no "
                  f"pinned-idx ERROR across {len(per_node_logs)} validator node(s)")


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


#: The canned readings a `--dry-run` walk answers its measurement reads with. They are announced
#: in the transcript and never scored: a verdict computed over canned readings is meaningless in
#: both directions, which is the rule `driver.SmokeCtx.check` already states for the ported cases.
_DRY_FIN = 100
_DRY_SERVICES = (topology.validator(0), topology.validator(1))

# ── LIVE POLL HELPERS (bounded; return None/raise on the deadline) ─────────────

def _await_dpos_active(chain, nodes, deadline_s: int, dry=False):
    """Wait until DPoS is active (epoch>=1) AND finalized is advancing (two rising
    samples). Returns (fin, epoch). Bringup.run() already converges past the anchor,
    so this normally returns on the first poll — it is the explicit readiness gate.

    DRY: one probe of the chain-side read (so the transcript shows where the case looks), a canned
    height, and no sleep. `nodes.finalized_dec` is not Runner-backed, so under dry it is not issued
    at all."""
    if dry:
        return _DRY_FIN, chain.current_epoch()
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


def _await_cap_increase(chain, cap_before: int, deadline_s: int, dry=False):
    """Poll activeValidatorsLength until it exceeds cap_before; return the new cap
    or None on the deadline (growth did not land on-chain).

    DRY: the read is ISSUED ONCE through the Runner (it belongs in the transcript) and its answer
    is discarded for a canned rise. A dry Runner has no chain to answer from, so scoring the real
    answer would fail the case for the one reason a rehearsal cannot be evidence about."""
    if dry:
        chain.active_validators_length()
        return cap_before + 1
    deadline = time.time() + deadline_s
    while time.time() < deadline:
        cap = chain.active_validators_length()
        if cap > cap_before:
            return cap
        time.sleep(5)
    return None


def _await_live_seat(chain, addr: str, deadline_s: int, dry=False):
    """Poll the LIVE committee until it contains addr (the growth warm-up lands the
    new member at the boundary). Return the epoch it seated at, or None on deadline.
    Crossing to the seat naturally spans the 2-epoch warm-up boundaries.

    DRY: both reads issued once through the Runner, the membership answer discarded for a canned
    seat — same reason as `_await_cap_increase`."""
    if dry:
        cur = chain.current_epoch()
        chain.committee_has(addr, cur)
        return cur
    deadline = time.time() + deadline_s
    while time.time() < deadline:
        cur = chain.current_epoch()
        if chain.committee_has(addr, cur):
            return cur
        time.sleep(5)
    return None


def growth_voter_idx(chain, cfg, epoch):
    """The governance voter set for ONE growth step: the owner idxs of the CURRENT committee,
    or None (unreadable committee) → gov's `PP_GOV_VOTERS` prefix.

    WHY THE PREFIX IS NOT ENOUGH HERE — measured on the live chain, not inferred. Every joiner
    `register_activate` lands holds 3e18 (a 1e18 `registerValidator` self-stake plus a 2e18
    `delegate`), and FluentGovernance's quorum is 2/3 of the delegated STAKE. After growth #1 the
    joiner is 43% of a 7e18 voting supply and, pinned to the initial four owners, never votes:
    `activate-5` carried forVotes 4e18 against quorum 4.666e18 and came back Defeated on a chain
    where nothing whatsoever was wrong. The votes all arrived, 3 blocks into a 10-block window —
    the harness was simply under-voting.

    The ceiling is the case's OWN validator count, not a minted high-water like the sim's: growth
    never mints, and `register_activate` caps the seat count at `SIM_VALIDATORS`, so idx
    0..validators-1 spans every identity that can be seated for the whole run.
    """
    return gov_live_voter_idx(chain.committee(epoch), chain.owner_addr, cfg.validators - 1)


# ── THE CASE ───────────────────────────────────────────────────────────────────

def run_case(argv=None) -> int:
    argv = list(argv or [])
    dry = "--dry-run" in argv
    unknown = [a for a in argv if a != "--dry-run"]
    if unknown:
        print(f"case-growth: unrecognised argument(s) {unknown} (only --dry-run is accepted)",
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
    growth_joiners = list(range(cfg.initial_committee, cfg.validators))
    if not growth_joiners:
        print(f"CASE-GROWTH SETUP ERROR: initial_committee={cfg.initial_committee} >= "
              f"validators={cfg.validators} — no room for a growth (raise SIM_VALIDATORS)",
              flush=True)
        return RC_USAGE

    interval = int(os.environ.get("SIM_EPOCH_INTERVAL", "32"))
    rpc = os.environ.get("RPC", topology.DEFAULT_RPC_URL)
    keep_up = os.environ.get("SIM_KEEP_UP", "0") == "1"
    env = {"RPC": rpc, "COMPOSE_FILE": os.environ.get("COMPOSE_FILE", ""),
           "CHAIN_ID": os.environ.get("CHAIN_ID", str(topology.CHAIN_ID))}
    runner = Runner(env=env, dry=dry, echo=dry)
    saved_env = save_exported_env()
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

    def measured(label: str, live, dry_value):
        """A measurement read. Live: issued. Dry: recorded as a transcript marker and answered
        with `dry_value`, never issued — these reads go straight to `core/nodes`, which has no dry
        seam of its own. `driver.SmokeCtx._delegated` is the same shape; these three cases predate
        the ctx and have nothing to hang it on."""
        if dry:
            runner.step("read", label)
            return dry_value
        return live()

    def fail(reason: str) -> int:
        print(f"CASE-GROWTH FAIL: {reason}", flush=True)
        try:
            EventLog().bundle_dump(reason, "case-growth")
        except Exception:  # noqa: BLE001 — a best-effort bundle must not mask the fault
            pass
        teardown()
        return RC_FAIL

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
                bu.run_from_golden(lambda runner: golden.restore_golden(spec, runner))
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
        # gov voter FALLBACK prefix = the initial committee (bringup already exported
        # PP_GOV_VOTERS). Only reached when the live committee is unreadable — the growth calls
        # below pass the live set explicitly.
        os.environ.setdefault("PP_GOV_VOTERS", str(cfg.initial_committee))

        # 2. readiness baseline
        fin0, epoch0 = _await_dpos_active(chain, nodes, deadline_s=300, dry=dry)
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
            voters = growth_voter_idx(chain, cfg, ep_at)
            print(f"CASE-GROWTH: growth #{j + 1}/{len(growth_joiners)} — register+activate "
                  f"idx {idx} at epoch {ep_at} (cap {cap}, gov voters "
                  f"{voters if voters is not None else 'PREFIX (committee unreadable)'})",
                  flush=True)
            chain.register_activate(idx, raise_cap=1, voter_idx=voters)

            new_cap = _await_cap_increase(chain, cap, deadline_s=interval * 4 + 120, dry=dry)
            if new_cap is None:
                return fail(f"growth idx {idx}: activeValidatorsLength never rose above {cap} "
                            "— the cap bump did not land on-chain")
            addr = chain.owner_addr(idx)
            seated = _await_live_seat(chain, addr, deadline_s=interval * 5 + 120, dry=dry)
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
        fin_now = measured("finalized_dec()", nodes.finalized_dec, target)
        while fin_now < target and time.time() < live_deadline:
            time.sleep(5)
            fin_now = nodes.finalized_dec()

        # 5. NO PINNED-IDX STALL: over every running validator, the counter AND the log.
        #
        # The counter is read per node off the in-container commonware registry (:9100) — only
        # validator-0 publishes a host port, so a v0-only read would be the hub's view of every
        # other node's ceremony. `beacon_metric` answers -1 for an unreachable endpoint or an
        # absent family, and `evaluate_idx_metric` treats -1 as UNREAD rather than as zero.
        window = f"{int(time.monotonic() - growth_start) + 60}s"
        running = measured("running_services()", nodes.running_services, list(_DRY_SERVICES))
        if not running:
            return fail("`docker compose ps` returned no running services — refusing to report "
                        "'no idx-stall' over an empty node set")
        validators = [svc for svc in running if topology.is_validator(svc)]
        per_node_logs = {svc: measured(f"logs_since({svc}, {window})",
                                       lambda s=svc: nodes.logs_since(s, window), "")
                         for svc in validators}
        per_node_idx = {svc: measured(f"beacon_metric({svc}, {BUGB_IDX_METRIC})",
                                      lambda s=svc: nodes.beacon_metric(s, BUGB_IDX_METRIC), 0)
                        for svc in validators}
        print(f"CASE-GROWTH: scanned {len(per_node_logs)} validator log(s) over the last {window} "
              f"and their {BUGB_IDX_METRIC}", flush=True)

        # 6. verdict
        if dry:
            teardown()
            print(f"# {len(runner.log)} commands")
            return RC_PASS
        ok, reason = evaluate_growth_case(fin0, fin_now, min_advance, per_node_logs, per_node_idx)
        if not ok:
            return fail(reason)
        span = (fin_now - fin0) // max(interval, 1)
        print(f"CASE-GROWTH PASS: committee grew across {boundaries} boundaries, finalized "
              f"advanced fin0={fin0}→finN={fin_now} (~{span} epoch-span); {reason}",
              flush=True)
        teardown()
        return RC_PASS

    except ChainError as e:
        return fail(f"chain error [{e.reason_id}]: {e.message}")
    except KeyboardInterrupt:
        print("CASE-GROWTH: interrupted", flush=True)
        teardown()
        return 130
    finally:
        restore_exported_env(saved_env)
