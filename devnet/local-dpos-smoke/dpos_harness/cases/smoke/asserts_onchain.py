"""asserts_onchain.py — the four ON-CHAIN case bodies (`case-liveness.sh`, `case-byzantine.sh`,
`case-cert-catchup.sh`, `case-vrf-dkg-restart-midwindow.sh`).

These are the first cases in the suite that read the STAKING SURFACE: the committee enumeration,
the windowed participation counters, the validator status byte and the ChainConfig floor. Every
one of those readers already existed in `core/nodes.py` — this chunk RECONCILES with them through
`SmokeCtx` rather than re-porting `lib.sh`'s wrappers a second time.

Same split as everywhere else here: this module decides what to read and when,
`verdicts_onchain` decides what the readings mean.

═══ WHAT IS DIFFERENT ABOUT THIS CLUSTER, AND WHAT A PORT CAN QUIETLY LOSE ═══════════════

1. **`cert-catchup` proves a NEGATIVE over a race window.** A graceful-restarted validator must
   park on a block whose `h+K` attestation body has not backfilled yet (guard #2 — there is no
   cert in this condition any more) WITHOUT logging the pre-fix budget-shutdown ERROR and WITHOUT
   a consensus exit. Both are log-count DELTAS across the restart, and both pass when the count
   did not move — which is also what happens when the reader is silent, when the window never
   opened, or when the grepped string no longer exists in the product (which is exactly what had
   happened: see `vo.PARK_LOG`). The three guards are the ANSI strip (`ctx.log_count` reads
   ANSI-stripped, §2.4 item 2), the MANDATORY park gate as positive control, and the
   product-source assertion on `PARK_LOG` in `test_smoke_onchain_cases.py`. THE BUDGETS ARE THE
   ASSERTION: the 28-block gap is below the re-jump threshold so the catch-up walks rather than
   jumps, and below `vo.CATCHUP_GAP_CEILING` so neither boot drift nor the catch-up's own stall
   pushes `vo.effective_catchup_gap` over it. Shortening any wait here disarms the case silently.

2. **`vrf-dkg-restart-midwindow` times its restart off the ON-DISK JOURNAL, never a height.** The
   DKG actor's clock leads EL-finalized by K=3 blocks, so a height of `seal_deadline+1` in
   EL-finalized terms is already past the actor's seal — a height-keyed restart there reloads the
   persisted share and skips resume entirely (the original false-RED). The journal is written at
   DEAL-START and evicted only at the past-boundary sweep, so polling for it lands the restart
   inside the open window on any host. The SHARE-ABSENT half of the gate is equally load-bearing:
   without it the restart can land post-finalize, where `maybe_start` early-returns on the store
   hit and no resume line is ever logged.

3. **`liveness` rotates the victim across three validators and its jail is UNRECOVERABLE.** Its
   cycles can push the participation counter to a jail, which permanently shrinks the committee,
   so it stays an isolated stack and is deliberately NOT in the `smoke-fault` aggregate — see
   `fault.py`'s header, which already records the exclusion.

4. **`byzantine` asserts TWO things, not one**: the equivocator is slashed and jailed, AND the
   honest chain keeps advancing afterwards. The second is not a victory lap — a post-jail
   committee-drop epoch-boundary wedge is a real DPoS failure class that the pre-jail converge
   structurally cannot see, because the committee only changes at the jail.

═══ THE PARTICIPATION SENTINELS ══════════════════════════════════════════════════════════

Three of these four read `participation`, and its three-valued answer (`-1` not in committee,
`-2` read failed, real counters) is never flattened here: every branch goes through
`verdicts_onchain.credit_state`. §2.4 item 5, `tests/test_sentinels.py`.
"""

from __future__ import annotations

import os

from . import beacon, verdicts_onchain as vo
from ...core import topology


def _ok(ctx, case: str, message: str) -> None:
    """The `OK (<case>): …` line. Suppressed under `--dry-run`, where nothing was measured."""
    if not ctx.dry:
        print(f"OK ({case}): {message}", flush=True)


def _say(ctx, message: str) -> None:
    """A progress line, also suppressed under dry — the transcript is the artifact there."""
    if not ctx.dry:
        print(message, flush=True)


def _addresses(ctx, case: str):
    """`mapfile -t ADDR < <(… cat /runtime/addresses.json | jq -r '.validators[]')` plus its
    four-element guard (`case-liveness.sh:30-31`, `case-byzantine.sh:22`)."""
    addrs = ctx.runtime_addresses()
    ok, msg = vo.evaluate_validator_addresses(addrs)
    ctx.check(case, ok, msg)
    return addrs


def _wait_bootstrap_dkg(ctx, case: str, timeout, tail=None) -> None:
    """Advance past epoch 2 BEFORE any disruption (`case-liveness.sh:114-117`,
    `case-cert-catchup.sh:105-112`).

    Not a settle wait. `bring_up_dpos` returns at the migration anchor (relative epoch 0), but the
    bootstrap live-DKG for `DETERMINISTIC_BOOTSTRAP_EPOCH=2` only deals during epoch 1 and
    finalizes just before `epoch_start(2)`. A victim stopped before that never attends ANY
    ceremony, so it is permanently shareless on rejoin (no reshare in v1) and cannot re-promote to
    signer — out of v1 scope. Advancing first is what makes the restart exercise the SUPPORTED
    path: a member that attended its bootstrap DKG, restarts, reloads its on-disk share, and
    catches up.

    `tail=None` means NO log dump on failure, which is `case-liveness.sh:115-116` — it echoes and
    exits. `case-cert-catchup.sh:109` dumps `--tail=120 validator-0`. Kept as a parameter rather
    than unified upward: a dump the bash does not do is extra output on a failure path, and one it
    does do is a diagnostic that would go missing.
    """
    target = ctx.activation_block + 2 * ctx.interval + vo.DKG_SETTLE_BLOCKS
    _say(ctx, f"establishing bootstrap DKG: advancing past epoch 2 (finalized >= {target}) "
              "before disruption")
    ctx.check(case, ctx.wait_finalized_ge(target, timeout),
              lambda: (f"chain did not reach epoch 2 (bootstrap DKG, block {target}) before "
                       f"disruption (finalized={ctx.finalized_dec()})"),
              on_fail=(lambda: ctx.dump_logs(tail, topology.PINNED_RPC_HOST))
              if tail is not None else None)
    _say(ctx, "  bootstrap DKG epoch 2 finalized; all validators hold a persisted share")


# ══ smoke-liveness ════════════════════════════════════════════════════════════════════

#: The canned answer the `--dry-run` transcript gives the promote-ledger reads. A LOG-SHAPED
#: string rather than `""`: it walks the same parse the live reading walks, so a regex that stopped
#: matching the product's rendering fails the dry parse in the tests too, instead of quietly
#: agreeing with an empty string.
_DRY_PROMOTE_LOG = "INFO promoted to Signer in-process: per-epoch BFT engine started epoch=Epoch(9)"


def assert_liveness(ctx) -> None:
    """With 4 validators (f=1, quorum 3) the network keeps finalizing while ONE is offline, the
    offline validator's on-chain participation counter falls behind the always-up hub's, and the
    validator re-syncs and rejoins on restart.

    The victim ROTATES across the three non-hub validators. validator-0 stays up throughout: it is
    the hub whose enode the spokes pin as `--trusted-peers`, so killing it would remove the EL
    transport the rejoin rides on and the case would be measuring a different failure.

    The rejoin mechanism under test is the CONSENSUS-plane epoch walk, not reth pipeline-backfill:
    a validator down across >= 1 epoch boundary resumes in its stale epoch while the committee is
    ahead, the vote backup channel detects ahead-epoch votes, hints the marshal, and the marshal
    walks the finalized tip forward boundary-by-boundary, soft-entering each crossed epoch's
    committee scheme until the live epoch full-enters. The four cycles below span that spectrum at
    four different depths, which is why they are four and not one.

    …and only cycle 3 actually WALKS it. The other three gap past the steady-state re-jump gate
    (`vo.LIVENESS_CYCLES` says which and by how much), so their victims TELEPORT to the frontier
    instead. That is why every cycle now ends on a SIGNING check and not on a height: a re-jumped
    member is height-aligned long before it holds a per-epoch BFT engine, and this case's whole
    premise is that the previous victim is a working signer again before the next one is stopped.
    """
    case = "smoke-liveness"
    addrs = _addresses(ctx, case)
    hub_addr = addrs[0]

    # `tail=None`: `case-liveness.sh:115-116` echoes and exits with no log dump.
    _wait_bootstrap_dkg(ctx, case, vo.LIVENESS_DKG_WAIT_S, tail=None)

    for idx, epochs, extra in vo.LIVENESS_CYCLES:
        _liveness_cycle(ctx, case, addrs, hub_addr, idx, vo.cycle_gap(ctx.interval, epochs, extra))

    _ok(ctx, case, "[v3 deep, v2 single-boundary, v1 within-epoch, v3 double-rejoin] all held "
                   "liveness while down, recorded on-chain miss-count, and rejoined the live tip")


def _liveness_cycle(ctx, case: str, addrs, hub_addr: str, idx: int, gap: int) -> None:
    """One stop → advance → production-credit → restart → rejoin → SIGNING cycle
    (`case-liveness.sh`)."""
    svc = topology.validator(idx)
    vic = addrs[idx]
    pre = ctx.baseline_height()
    _say(ctx, f"── cycle: victim={svc} ({vic}), target gap >= {gap} blocks (pre={pre}) ──")

    # The victim's Verifier→Signer ledger BEFORE the stop. `docker compose stop`/`start` keeps the
    # SAME container and its log persists across the cycle, so the promotion this member did on
    # its way into the epoch it is about to be killed in is still in there. Only a DELTA against
    # this snapshot is evidence about the node that came back — the same reason `cert-catchup`
    # snapshots its four counters rather than grepping absolutes.
    promoted_before = vo.promoted_epoch_counts(ctx.logs_all(svc, dry_value=_DRY_PROMOTE_LOG))
    ctx.compose_stop(svc, timeout=vo.LIVENESS_STOP_TIMEOUT_S, note=f"liveness-stop {svc}")

    # 1) BFT f=1 holds: the chain keeps finalizing with one of four down, and advances the gap.
    ctx.check(case, ctx.wait_finalized_ge(pre + gap, vo.LIVENESS_ADVANCE_S),
              lambda: (f"chain did not advance {gap} blocks with {svc} down "
                       f"(finalized={ctx.finalized_dec()}, pre={pre})"))
    _say(ctx, f"  chain finalized past {pre + gap} with {svc} down (BFT f=1 holds)")

    # 2) The PRODUCTION RECORD reached the chain and is credited to the right member.
    #
    # Nothing punitive happens to the victim here — the production tier ships flag-off and is
    # inert at the smoke epoch length anyway — so it stays in committee for the rejoin leg below.
    # The poll retries on the -2 read-failed sentinel rather than letting it satisfy the
    # comparison: a failed read collapsing to 0 would make `victim < hub` trivially true and hide
    # a getter regression.
    epoch, vprod, vtotal, rprod, rtotal = _poll_production(ctx, vic, hub_addr)
    ctx.check(case, *vo.evaluate_production(epoch, vprod, vtotal, rprod, rtotal))
    _say(ctx, f"  on-chain production credit correct: producedAt(epoch={epoch}, {svc})={vprod} < "
              f"hub={rprod} (blocksInEpoch={rtotal})")

    # 3) Rejoin: the victim realigns with the hub AND has a live reth devp2p peer.
    ctx.compose_start(svc, note=f"liveness-start {svc}")

    def rejoined():
        return _liveness_rejoin_probe(ctx, svc, pre + gap)

    reading = ctx.poll(rejoined, vo.LIVENESS_REJOIN_S,
                       poll_s=vo.LIVENESS_REJOIN_POLL_S, dry_value=("0x0|0x0", 1, "0x0|0x0"))
    ctx.check(case, bool(reading),
              lambda: vo.liveness_not_rejoined_message(
                  svc, ctx.peer_count(svc), ctx.check_node(svc),
                  ctx.check_external(topology.HOST_RPC_PORT), pre + gap))
    if reading:
        # BOTH heights on the OK line. The hub's reading is not decoration: the pass this case
        # printed while it was broken — a victim 114 blocks behind reading as rejoined — was
        # auditable from the OK line the moment the hub's height sat next to the victim's, and
        # bash printed it all along. A pass nobody can check from its own output is how the
        # regression survived a live run.
        _say(ctx, f"  OK: {svc} rejoined at {reading[0]} with reth peers={reading[1]} "
                  f"(v0={reading[2]}, floor=pre+gap={pre + gap})")

    # 4) …AND IT IS SIGNING AGAIN. Height is not participation.
    #
    # THE DEFECT THIS CLOSES. Three of the four cycles gap PAST the steady-state re-jump gate
    # (`min(1024, interval)`, dpos.rs:2556 — and `re_jump` is `Some` for these validators: the
    # plane-native default hands every `--dpos` node an upstream, node/dpos.rs:1683). A re-jumped
    # member catches up by HEIGHT — devp2p serves it the blocks and the gate above goes green —
    # while its per-epoch BFT engine never spawned, because the jump teleported the marshal floor
    # past the `E-1` boundary block the spawn gate needs (epoch_manager.rs:734-749). It is then
    # verify-only: no proposals, no votes. The case marched on, stopped a SECOND validator, and
    # 2 live signers of 4 against a quorum of 3 stalled the chain at `finalized=241, pre=241` —
    # a correct BFT stall reported as a product bug.
    #
    # The signal is the promote line and NOT a proposal. A member that IS signing can still skip
    # its first proposals (`application.rs:911`, `parent-seed witness not in SeedStore; skipping
    # propose`), so "produced a block" would trade this false PASS for a false FAIL.
    min_epoch = vo.epoch_of(pre + gap, ctx.interval, ctx.activation_block)
    box = {"logs": ""}

    def signing_again():
        box["logs"] = ctx.logs_all(svc, dry_value=_DRY_PROMOTE_LOG)
        return vo.liveness_signing(promoted_before, vo.promoted_epoch_counts(box["logs"]),
                                   min_epoch)

    ctx.poll(signing_again, vo.liveness_signing_budget(ctx.interval),
             poll_s=vo.LIVENESS_SIGNING_POLL_S)
    promoted_after = vo.promoted_epoch_counts(box["logs"])
    ctx.check(case, *vo.evaluate_liveness_signing(promoted_before, promoted_after, min_epoch,
                                                  svc, box["logs"], pre + gap))
    _say(ctx, f"  OK: {svc} is SIGNING again — promoted to Signer for epoch(s) "
              f"{vo.promoted_epochs_since(promoted_before, promoted_after)} "
              f"(floor epoch={min_epoch}, from pre+gap={pre + gap})")


def _poll_production(ctx, vic: str, hub_addr: str):
    """`case-liveness.sh` — poll the two production reads until the victim lags the hub.

    Returns the LAST readings whether or not they hit, because the failure message names them:
    bash keeps `$vseen/$vcerts/$rseen/$rcerts` across the loop for exactly that. `currentEpoch()`
    is re-read every iteration, deliberately — the window can roll over mid-poll and reading the
    counters of an epoch that has just been superseded is how a correct victim reads as absent.
    """
    box = {"epoch": "?", "v": (vo.CREDIT_READ_FAILED,) * 2, "r": (vo.CREDIT_READ_FAILED,) * 2}

    def victim_lags_hub():
        box["epoch"] = _first_token(ctx.staking_call("currentEpoch()(uint64)", dry_value="2"))
        box["v"] = ctx.production(box["epoch"], vic, dry_value=(3, 10))
        box["r"] = ctx.production(box["epoch"], hub_addr, dry_value=(10, 10))
        return vo.production_hit(box["v"][0], box["r"][0])

    ctx.poll(victim_lags_hub, vo.PRODUCTION_POLL_BUDGET_S, poll_s=vo.PRODUCTION_POLL_S)
    return box["epoch"], box["v"][0], box["v"][1], box["r"][0], box["r"][1]


def _liveness_rejoin_probe(ctx, svc: str, floor_dec: int):
    """One rejoin poll iteration -> `(reading, peers, hub_reading)` once aligned, else False.

    The reads are issued in bash's order (hub, victim, peers) and all three every iteration: the
    peer count is not a diagnostic here but half of the gate, so skipping it while the heads
    disagree would let a peerless victim that matched a stalled hub read as a rejoin.

    The hub's reading rides back out with the victim's so the OK line can print both."""
    v0 = ctx.check_external(topology.HOST_RPC_PORT)
    vn = ctx.check_node(svc)
    peers = ctx.peer_count(svc)
    return ((vn, peers, v0)
            if vo.liveness_rejoined(v0, vn, peers, floor_dec, ctx.blockhash_at) else False)


def _first_token(out: str) -> str:
    """The first whitespace token of a `cast call` answer. `cast` right-justifies and may append a
    bracketed decimal form; the epoch is the first token in every rendering."""
    return (out or "").strip().split()[0] if (out or "").strip() else ""


# ══ smoke-byzantine ═══════════════════════════════════════════════════════════════════

def assert_byzantine(ctx) -> None:
    """validator-3 equivocates (double-signs Notarize/Finalize votes via the vendored
    `VoteEquivocator`, behind the `dpos-devnet-byzantine` build feature and the
    `FLUENT_DPOS_BYZANTINE=equivocate` env the overlay sets).

    TWO assertions, and they are independent:

      1. the offending validator is slashed on-chain and JAILED (`ValidatorStatus.Jail == 3`);
      2. the honest 3-of-4 quorum KEEPS finalizing once the equivocator is dropped.

    There is no public `tombstoned()` getter, so (1) is asserted through `getValidatorStatus`
    (Addendum D). (2) guards a post-jail / committee-drop epoch-boundary wedge — a real DPoS
    failure class the pre-jail converge cannot cover, because the committee only changes AT the
    jail.
    """
    case = "smoke-byzantine"
    addrs = _addresses(ctx, case)
    vic = addrs[vo.BYZANTINE_VICTIM_IDX].strip()
    _say(ctx, f"smoke-byzantine: validator-3 ({vic}) equivocating; honest quorum live "
              f"(anchor={ctx.stack.prev_fin})")

    # ── 1: the on-chain equivocation slash → Jail ─────────────────────────────────────
    box = {"status": ""}

    def jailed():
        box["status"] = ctx.validator_status(vic, dry_value=vo.STATUS_JAIL)
        return (box["status"] or "").strip() == vo.STATUS_JAIL

    ctx.poll(jailed, vo.JAIL_TIMEOUT_S, poll_s=vo.JAIL_POLL_S)
    ctx.check(case, *vo.evaluate_jailed(box["status"]),
              on_fail=lambda: _dump_byzantine(ctx))
    _say(ctx, "smoke-byzantine: validator-3 jailed (status=Jail) by equivocation slashing")

    # ── 2: liveness AFTER the jail ────────────────────────────────────────────────────
    post_jail = ctx.baseline_height()
    advanced = ctx.wait_finalized_ge(post_jail + vo.POST_JAIL_BLOCKS, vo.POST_JAIL_S)
    ctx.check(case, *vo.evaluate_post_jail_liveness(advanced, post_jail),
              on_fail=lambda: ctx.dump_logs(vo.BYZ_STALL_TAIL, topology.validator(0),
                                            topology.validator(1), topology.validator(3)))
    _ok(ctx, case, f"honest chain advanced past {post_jail} after the jail "
                   f"(now {ctx.finalized_dec()})")


def _dump_byzantine(ctx) -> None:
    """`case-byzantine.sh:40-46` — the failure diagnostic, EQUIVOCATOR FIRST.

    The order is the diagnosis: the first question is whether validator-3 took the Equivocate path
    at all (can its scheme sign? does it have a local share? is it broadcasting conflicting
    votes?), because a slasher that saw nothing and an equivocator that never equivocated look
    identical from validator-0's side."""
    print("--- validator-3 (equivocator) byzantine markers ---", flush=True)
    print(vo.grep_markers(ctx.logs_tail(topology.validator(3), tail=vo.BYZ_MARKER_TAIL),
                          vo.BYZ_EQUIVOCATOR_MARKERS), flush=True)
    print("--- validator-0/1 slasher / conflict markers ---", flush=True)
    print(vo.grep_markers(ctx.logs_tail(topology.validator(0), topology.validator(1),
                                        tail=vo.BYZ_MARKER_TAIL),
                          vo.BYZ_SLASHER_MARKERS), flush=True)
    print("--- validator-0 tail ---", flush=True)
    ctx.dump_logs(vo.BYZ_LOG_TAIL, topology.validator(0))


# ══ smoke-cert-catchup ════════════════════════════════════════════════════════════════

def assert_cert_catchup(ctx) -> None:
    """The DETERMINISTIC, fast gate for the executor's catch-up PARK path.

    It reproduces the recover-stall shape on demand instead of waiting for the hour-long
    stochastic soak to drift into it: a GRACEFULLY-restarted committee validator falls behind and
    walks the gap by deriving each finalized OrderBlock in turn. While it is ≥ K (=3,
    order_block.rs:23) behind the ordering tip, GUARD #2 (executor.rs:2444) cross-checks the
    committee-attested `result` at `h+K` BEFORE acking `h` — and the marshal backfills bodies OUT
    OF ORDER inside its repair window, so `h+K`'s BODY can still be absent when `h` is dispatched.

      OLD: the executor waited on a cert budget, then self-shut-down → dropped its ack → the
           marshal Cancels → `OuterEngine exited cleanly (unexpected)` → `reason=consensus_exit`
           → never rejoins.
      NEW: the executor PARKS event-driven (executor.rs:1473), warns once, sets a
           `deferred_height` gauge, hints peers for `h+K`, and re-derives on the marshal delivery
           stream + the 8s FCU heartbeat (`repoke_deferred`, executor.rs:1497).

    THERE IS NO CERT DEPENDENCY LEFT ON THIS PATH — the pre-B′ cert-miss park classes were DELETED
    with the witness (DPOS_ARCHITECTURE.md, "the one remaining park is guard #2's absent h+K
    attestation body"). The name `smoke-cert-catchup` is history; the condition is a missing BODY.

    THREE GATES, and the third is why the first two are not enough:

      1. the restarted victim REJOINS (its finalized height realigns to the hub head);
      2. its log shows the park path FIRING — a rejoin that never parked is a FALSE PASS, because
         the negatives below are satisfied by a path that simply did not run;
      3. it did NOT log the old fatal budget-shutdown ERROR and did NOT log the consensus_exit.

    WHY 64-BLOCK EPOCHS AND A 28-BLOCK GAP, both of which are the assertion and not tuning: the
    effective re-jump gate is `min(JUMP_THRESHOLD=1024, epochBlockInterval)` (dpos.rs:2543
    validator / :3237 follower), so interval=64 opens a 64-block pure-park window. The gap sits
    BELOW that threshold, so the victim catches up via the PURE derive-walk with no re-jump
    stealing the walk — exactly the prod failure regime (prod's threshold 1024 >> its observed
    45-block failure gap) — and WELL under a full window, so being absent for ~44% of it leaves
    `seen` above the 15% participation floor and no liveness slash fires while the victim is down.

    The gap's practical ceiling is `vo.CATCHUP_GAP_CEILING`, NOT the threshold, and it is DERIVED
    (`vo.catchup_gap_ceiling`) rather than hand-set: the frontier climbs while the victim boots
    AND while its shaped catch-up stalls on each repair round trip, so the executor faces
    `vo.effective_catchup_gap(gap)` — 53 blocks for a 28-block gap at the default 3000 ms delay,
    against a 64-block threshold and a ceiling of 32. FOUR blocks of headroom, and that is the
    thinnest number in this case: it is accepted on four green live bring-ups rather than on the
    model, which has been wrong in both directions here (it blessed gap 40 at 1000 ms, which lost
    the walk to `maybe_re_jump` on a re-run, and it forbade the 3000 ms default outright until the
    rounds were repriced on the actual backlog). `vo.CATCHUP_GAP` carries the full argument and
    what would reopen it. Raising the interval to lift the ceiling instead was tried live and the
    stack does not boot at 128 — see `vo.CATCHUP_EPOCH_INTERVAL`.

    AND WHY THE VICTIM RUNS UNDER ADDED LATENCY. Depth is not what makes the park fire; the ratio
    between the executor's derive and the marshal's repair is. At zero RTT the fetch always wins on
    this LAN and the park fires at NO gap, which is why `vo.CATCHUP_NETEM_DELAY_MS` of one-way
    `tc netem delay` is applied to the victim — and to nobody else — for the catch-up window, and
    removed the moment the walk lands. That constant carries the full rationale; the short version
    is that a zero-latency LAN is the unrealistic condition, not the delay.
    """
    case = "smoke-cert-catchup"
    victim = os.environ.get("CERT_CATCHUP_VICTIM", vo.CATCHUP_VICTIM)
    gap = int(os.environ.get("CERT_CATCHUP_GAP", vo.CATCHUP_GAP))
    delay_ms = int(os.environ.get("CERT_CATCHUP_DELAY_MS", vo.CATCHUP_NETEM_DELAY_MS))
    deep = os.environ.get("CERT_CATCHUP_DEEP", "0") == "1"
    deep_gap_blocks = int(os.environ.get("CERT_CATCHUP_DEEP_GAP", vo.deep_gap(ctx.interval)))

    # (0) PRE-FLIGHT: UNSHAPE THE VICTIM BEFORE TOUCHING IT AT ALL.
    #
    #     Unconditional, ignoring failure, and FIRST — before the DKG wait, before any stop, on
    #     every run whether or not this one ever shapes anything. The `finally` at (5a) and bash's
    #     EXIT trap cover the paths where THIS process unwinds; neither fires when the process is
    #     SIGKILLed, which is how a real run ended on 2026-07-30 and left validator-2 carrying a
    #     1 s delay. A container that survives into the next bring-up keeps its qdisc (netem lives
    #     in the container's netns), and a stack left shaped poisons every later case on it and
    #     reads as a product slowdown rather than as this case's doing. Self-healing beats a
    #     cleanup path that cannot be guaranteed to run: the case starts by asserting the
    #     condition it needs instead of trusting that it was restored.
    ctx.netem_clear(victim, vo.CATCHUP_NETEM_IFACE, note=f"catchup-netem-preflight {victim}")
    _say(ctx, f"  netem pre-flight: cleared any stale qdisc on {victim} "
              f"({vo.CATCHUP_NETEM_IFACE})")

    # The gap is bounded by the DERIVED ceiling, not by the threshold — an operator override that
    # sails past it loses the derive-walk to `maybe_re_jump` and fails the park gate six minutes
    # later for a reason that has nothing to do with the park. Fail here instead. The remedy is
    # NOT "raise EPOCH_BLOCK_INTERVAL to lift the ceiling" — that was tried live and does not
    # boot (`vo.CATCHUP_EPOCH_INTERVAL`); it is a smaller gap.
    ceiling = vo.catchup_gap_ceiling(ctx.interval, delay_ms)
    ctx.check(case, gap <= ceiling,
              f"CERT_CATCHUP_GAP={gap} is above the derived ceiling {ceiling} at "
              f"interval={ctx.interval} / delay={delay_ms}ms: the effective gap would be "
              f"{vo.effective_catchup_gap(gap, delay_ms)} against a re-jump threshold of "
              f"{vo.rejump_threshold(ctx.interval)}, so maybe_re_jump would fast-forward past the "
              "derive-walk and the park could not fire. LOWER the gap — raising "
              "EPOCH_BLOCK_INTERVAL to lift the ceiling does not boot (see CATCHUP_EPOCH_INTERVAL).")

    _wait_bootstrap_dkg(ctx, case, vo.CATCHUP_DKG_WAIT_S, tail=vo.CATCHUP_LOG_TAIL)

    # CYCLE 1 (MANDATORY GATE): the pure PARK path.
    _catchup_cycle(ctx, case, victim, gap, "park", require_park=True, delay_ms=delay_ms)

    # CYCLE 2 (OPTIONAL): a gap ABOVE the re-jump threshold, to also exercise the gap-gated
    # re-jump. It tolerates a liveness demotion — a deep gap can leave the victim below the
    # participation floor for a full window — so the park there is best-effort, not a gate.
    if deep:
        _catchup_cycle(ctx, case, victim, deep_gap_blocks, "rejump-deep", require_park=False,
                       delay_ms=delay_ms)

    note = " + gap-gated re-jump" if deep else ""
    _ok(ctx, case, f"{victim} parked on deferred blocks whose h+K attestation body had not "
                   f"backfilled during graceful-restart catch-up, "
                   f"recovered event-driven (delivery/heartbeat re-poke{note}), and "
                   "rejoined the live tip — NO budget-shutdown ERROR, NO consensus_exit")


def _catchup_cycle(ctx, case: str, victim: str, gap: int, label: str, require_park: bool,
                   delay_ms: int = vo.CATCHUP_NETEM_DELAY_MS) -> None:
    """One graceful-stop → advance-gap → restart → rejoin cycle (`case-cert-catchup.sh:119-213`).

    THE LOG COUNTS ARE SNAPSHOTTED BEFORE THE STOP, and that is what makes the three log
    assertions mean anything: `docker compose stop`/`start` keeps the SAME container, so its log
    and datadir persist across the cycle. An absolute count would also count history and could
    never tell a regression from a line the node wrote ten minutes ago.

    THE NETEM DELAY IS BRACKETED IN A `finally`, and that is not tidiness. It is applied after the
    restart and removed when the walk lands, so the failure paths in between — a victim that never
    rejoins, a gap that never opens — must still unshape. Bash gets the same guarantee from its
    `cleanup` EXIT trap. A validator left shaped is not a stopped node, it is a slow one: the
    optional deep cycle would then run under conditions nobody chose, and the slowdown would read
    as the product's rather than as this case's.
    """
    threshold = vo.rejump_threshold(ctx.interval)
    park0 = ctx.log_count(victim, vo.PARK_LOG)
    rejump0 = ctx.log_count(victim, vo.REJUMP_LOG)
    fatal0 = ctx.log_count(victim, vo.OLD_FATAL)
    exit0 = ctx.log_count(victim, vo.EXIT_LOG)

    pre = ctx.baseline_height()
    _say(ctx, f"── [{label}] victim={victim}  down-gap>={gap}  re_jump_threshold={threshold}  "
              f"(pre={pre}, park_baseline={park0}) ──")

    # (2) GRACEFUL stop. The fix targets a graceful restart: the flushed tail is what makes the
    #     restart a CATCH-UP rather than a cold re-sync, and `shutdown_flushed` (container exit 0)
    #     is how the case knows which of the two it is about to measure.
    ctx.compose_stop(victim, timeout=vo.CATCHUP_STOP_TIMEOUT_S, note=f"catchup-stop {victim}")
    ctx.check(case, ctx.shutdown_flushed(victim),
              f"[{label}] {victim} did not flush on graceful stop (exit != 0) — not the "
              "graceful-restart scenario the fix targets",
              on_fail=lambda: ctx.dump_logs(vo.CATCHUP_FLUSH_TAIL, victim))
    _say(ctx, f"  {victim} graceful-stopped + flushed (persisted tail)")

    # (3) Advance the live committee (BFT f=1 holds with 3/4 up) by >= gap blocks, so the
    #     restarted victim must catch up across a deep-enough gap to hit the park.
    ctx.check(case, ctx.wait_finalized_ge(pre + gap, gap * 4 + 120),
              lambda: (f"[{label}] chain did not advance {gap} blocks with {victim} down "
                       f"(finalized={ctx.finalized_dec()}, pre={pre})"),
              on_fail=lambda: ctx.dump_logs(vo.CATCHUP_LOG_TAIL, topology.PINNED_RPC_HOST))
    head = ctx.finalized_dec()
    _say(ctx, f"  committee advanced to {head} with {victim} down (gap={head - pre} >= {gap})")

    # (4) Graceful restart — the SAME container, so the same datadir and the accumulated log.
    ctx.compose_start(victim, note=f"catchup-start {victim}")

    # (4a) THE LEVER: shape the VICTIM ONLY, for the catch-up window ONLY. Here, after the
    #      restart, because the delay must bite on the BACKLOG WALK and nothing else — while the
    #      victim was down there was no traffic to shape, and once it has rejoined the delay would
    #      only slow the live chain the case still has to measure. Nobody else is touched: the
    #      committee keeps finalizing at full LAN speed, which is the side of the race that must
    #      stay fast.
    ctx.netem_delay(victim, delay_ms, vo.CATCHUP_NETEM_IFACE, note=f"catchup-netem {victim}")
    _say(ctx, f"  netem: +{delay_ms}ms egress delay on {victim} "
              f"({vo.CATCHUP_NETEM_IFACE}) for the catch-up window")

    # (5) REJOIN, sampling `deferred_height` WHILE polling. The park is transient (the gauge
    #     clears the instant the cert lands), so the sampling has to ride the same loop; it is
    #     corroboration only, and the persistent park warn! below is the authority.
    peak = {"g": 0, "tick": 0}

    # The floor is `pre + gap`, the height the chain PROVABLY reached during this cycle (the
    # `wait_finalized_ge` above is a hard gate). It used to be `pre`, which the victim clears the
    # instant it restarts on its own persisted tail — no catch-up proven.
    floor = pre + gap

    def rejoin_probe():
        g = ctx.deferred_gauge(victim)
        peak["g"] = vo.gauge_peak(peak["g"], g)
        v0 = ctx.check_external(topology.HOST_RPC_PORT)
        vn = ctx.check_node(victim)
        if vo.catchup_rejoined(v0, vn, floor, ctx.blockhash_at):
            return (vn, v0)
        if peak["tick"] % 6 == 0:
            _say(ctx, f"    t+{peak['tick'] * 2}s: {victim}={vn} v0={v0} deferred_height~{g}")
        peak["tick"] += 1
        return False

    try:
        landed = ctx.poll(rejoin_probe, vo.CATCHUP_REJOIN_S, poll_s=vo.CATCHUP_REJOIN_POLL_S,
                          dry_value=("0x0|0x0", "0x0|0x0"))
        ctx.check(case, bool(landed),
                  lambda: (f"[{label}] {victim} did not rejoin within {vo.CATCHUP_REJOIN_S}s "
                           f"({victim}={ctx.check_node(victim)}, "
                           f"v0={ctx.check_external(topology.HOST_RPC_PORT)}, "
                           f"floor=pre+gap={floor})"),
                  on_fail=lambda: ctx.dump_logs(vo.CATCHUP_DEEP_TAIL, victim))
    finally:
        # (5a) The walk is over either way — unshape. On the success path this is bash's explicit
        #      `netem_clear "$VICTIM"`; on the failure path it is what bash's EXIT trap does.
        ctx.netem_clear(victim, vo.CATCHUP_NETEM_IFACE, note=f"catchup-netem-clear {victim}")
    _say(ctx, f"  OK: {victim} rejoined at {landed[0]} (v0={landed[1]}, floor=pre+gap={floor})")
    _say(ctx, f"  netem: cleared on {victim} (catch-up window closed)")

    park1 = ctx.log_count(victim, vo.PARK_LOG)
    rejump1 = ctx.log_count(victim, vo.REJUMP_LOG)
    fatal1 = ctx.log_count(victim, vo.OLD_FATAL)
    exit1 = ctx.log_count(victim, vo.EXIT_LOG)

    # (5b) THE TWO NEGATIVES. Each is a separate regression and each gets its own gate: a future
    #      break that skipped the ERROR and still exited would satisfy the first and fail this.
    ctx.check(case, *vo.evaluate_no_old_fatal(fatal0, fatal1, victim, label),
              on_fail=lambda: ctx.dump_logs(vo.CATCHUP_DEEP_TAIL, victim))
    ctx.check(case, *vo.evaluate_no_consensus_exit(exit0, exit1, victim, label),
              on_fail=lambda: ctx.dump_logs(vo.CATCHUP_DEEP_TAIL, victim))

    _say(ctx, f"  [{label}] park warns: {park0} -> {park1} | re-jump landings: {rejump0} -> "
              f"{rejump1} | deferred_height peak sampled: {peak['g']} | "
              "old-fatal/consensus-exit: none")

    # (5c) THE GATE — the guard-#2 park must have been ACTUALLY exercised. Without it both
    #      negatives above pass on a victim that rejoined without ever parking.
    if require_park:
        ok, msg, parked = vo.evaluate_park_exercised(park0, park1, peak["g"], victim, label,
                                                     threshold, delay_ms)
        ctx.check(case, ok, msg, on_fail=lambda: ctx.dump_logs(vo.CATCHUP_DEEP_TAIL, victim))
        _say(ctx, f"  GATE PASS: guard-#2 park EXERCISED — {victim} parked {parked} fresh "
                  "deferred block(s) on an absent h+K body, recovered event-driven, and rejoined")
    else:
        _say(ctx, "  " + vo.rejump_note(rejump0, rejump1, victim, label, threshold) +
                  f"; park warns {park0} -> {park1}")


# ══ smoke-vrf-dkg-restart-midwindow ═══════════════════════════════════════════════════

def assert_vrf_dkg_restart_midwindow(ctx) -> None:
    """A committee[2] member RESTARTS while its epoch-2 DKG window is STILL OPEN — its ceremony
    journal is on disk (dealt) but not yet finalized or evicted.

    The opposite of `smoke-vrf-dkg-liveness`, which stops its victim BEFORE the window opens so it
    never starts a ceremony at all. Here the partial progress is lost from memory and can only be
    recovered from the on-disk ceremony journal plus the DKG-log recovery resolver: with the fix
    the restarted node RESUMES player-only from `beacon-dkgjournal-e2.bin` (never re-dealing),
    re-fetches missing peer logs over the resolver channel, and converges to `(PK_2, share)` before
    the boundary. WITHOUT it the node stays shareless for epoch 2, abstains from every seeded vote,
    and is liveness-slashed.

    FOUR assertions, in the order the evidence becomes available:

      (a) the victim logged a POST-RESTART resume AND converged to a share;
      (b) the chain crossed the boundary and every node has the window;
      (c) the victim's epoch-2 `prev_randao` is byte-identical to the survivors' — it participated
          as a real share-holder, not as a verify-only re-deriver;
      (d) THE LOAD-BEARING ONE: the victim PRODUCED in the epoch it resumed into.

    (d) is load-bearing because it is a direct positive observation that the resumed member did
    the work: a shareless member cannot produce. It replaced an assertion that the liveness slash
    was AVOIDED, which died with the slash — and which was weaker anyway, observing only that a
    punishment failed to arrive. Asserting merely "the chain stayed live" would pass on a run
    where the victim contributed nothing.
    """
    case = "smoke-vrf-dkg-restart-midwindow"
    addrs = _addresses(ctx, case)
    down = vo.MIDWINDOW_VICTIM
    down_addr = addrs[vo.MIDWINDOW_VICTIM_IDX]
    epoch = vo.MIDWINDOW_EPOCH
    epoch2_start = ctx.activation_block + epoch * ctx.interval
    boundary_probe = epoch2_start + vo.BOUNDARY_PROBE_OFFSET

    # ── 1: restart in the GENUINE mid-window state ────────────────────────────────────
    #
    # Keyed on the ON-DISK JOURNAL, never an EL-finalized height — see the module header. The
    # journal is written at DEAL-START and is NOT evicted at finalize (eviction moved to the
    # past-boundary sweep), so it lives the whole deal→boundary span and a 1 s poll lands inside
    # the window on any host. The share-ABSENT half guarantees the restart is PRE-finalize, so
    # `maybe_start` takes the store-MISS path and actually runs `resume_from_journal`.
    _say(ctx, f"{case}: waiting for {down}'s epoch-{epoch} DKG journal (present) AND share "
              "(absent) — the genuine pre-finalize mid-window so resume provably runs")

    # THE SAFETY RAIL RIDES INSIDE THE PROBE, as bash's does (`:130-136`), and the ORDER within
    # one iteration is bash's: the gate is evaluated first and the rail only when the gate is not
    # yet satisfied. Hoisting the rail out to after the poll would report "the journal never
    # appeared" for a run whose window merely closed — a wrong diagnosis, and 400 s late.
    def mid_window():
        journal = ctx.exec_test(down, vo.journal_probe(vo.MIDWINDOW_VICTIM_IDX, epoch),
                                dry_value=True)
        # The SHARE probe is INVERTED: the gate wants it ABSENT (pre-finalize), which is what
        # makes `maybe_start` take the store-MISS path and actually run `resume_from_journal`.
        if journal and not ctx.exec_test(down, vo.share_probe(vo.MIDWINDOW_VICTIM_IDX, epoch),
                                         dry_value=False):
            return vo.WINDOW_OPEN
        if ctx.finalized_dec() >= epoch2_start:
            return vo.WINDOW_MISSED
        return False

    hit = ctx.poll(mid_window, vo.JOURNAL_DEADLINE_S, poll_s=vo.JOURNAL_POLL_S,
                   dry_value=vo.WINDOW_OPEN)
    ctx.check(case, hit != vo.WINDOW_MISSED,
              f"chain reached the epoch-{epoch} boundary ({epoch2_start}) before {down}'s "
              "journal was observed — the open window was missed (re-run)")
    ctx.check(case, bool(hit),
              f"{down}'s epoch-{epoch} DKG journal never appeared on disk (the ceremony never "
              "started — the journal now lives deal-start→boundary, so a present journal cannot "
              "be missed by the poll)",
              on_fail=lambda: ctx.dump_logs(vo.MIDWINDOW_LOG_TAIL, down))
    now = ctx.finalized_dec()
    _say(ctx, f"{case}: restarting {down} mid-window at finalized={now} (epoch-{epoch} ceremony "
              "journal on disk, not yet finalized)")
    ctx.compose_restart(down, note=f"midwindow-restart {down}")

    # ── 2: the chain crosses the boundary; the victim resumes and converges ───────────
    ctx.check(case, ctx.wait_finalized_ge(boundary_probe, vo.MIDWINDOW_BOUNDARY_S),
              f"chain did not reach the epoch-{epoch} boundary after the mid-window restart",
              on_fail=lambda: ctx.dump_logs(vo.MIDWINDOW_BOUNDARY_TAIL, down))
    ctx.check(case, beacon.wait_nodes_have(ctx, boundary_probe, vo.MIDWINDOW_NODES_HAVE_S),
              f"nodes did not all reach {boundary_probe}")

    # (a) THE RECOVERY ASSERTION. Anchored on the resume line, which only the RESTARTED process
    #     can emit — a fast host can log "share computed" during the open window BEFORE the
    #     restart, so a whole-log grep for that alone is green even against a broken resume.
    box = {"resume": [], "share": []}

    def resumed():
        """ONE log read, TWO greps. bash issues `docker compose logs` twice per iteration
        (`:170`, `:172`), once for each line; one read answers both because they are two questions
        about the same text. The only thing lost is a redundant read of a log that grows to
        megabytes — the same trade `cast_receipt` makes in the follower chunk."""
        logs = ctx.logs_all(down, dry_value=f"{vo.RESUME_LINE} epoch=2\n{vo.SHARE_LINE} epoch=2")
        box["resume"] = vo.epoch_field_lines(logs, vo.RESUME_LINE, epoch)
        box["share"] = vo.epoch_field_lines(logs, vo.SHARE_LINE, epoch)
        return bool(box["resume"]) and bool(box["share"])

    ctx.poll(resumed, vo.RESUME_POLL_BUDGET_S, poll_s=vo.RESUME_POLL_S)

    def dump():
        ctx.dump_logs(vo.MIDWINDOW_LOG_TAIL, down)

    ctx.check(case, *vo.evaluate_resumed(box["resume"], down), on_fail=dump)
    ctx.check(case, *vo.evaluate_share_computed(box["share"], down), on_fail=dump)
    _say(ctx, f"{case}: {down} RESUMED from journal and recovered its epoch-{epoch} share:")
    for line in list(box["resume"]) + list(box["share"]):
        _say(ctx, f"    {line}")

    # (c) The victim's epoch-2 prev_randao is byte-identical to the survivors'.
    rows = [(v, ctx.mixhash_in(down, v), ctx.mixhash_at(v))
            for v in range(epoch2_start, boundary_probe + 1)]
    ctx.check(case, *vo.evaluate_prev_randao_window(rows, down))

    # (d) THE LOAD-BEARING ASSERTION — the restarted member PRODUCED.
    #
    # This used to assert the member stayed above the participation floor and so AVOIDED the
    # liveness slash. That consequence is deleted: there is no floor, no liveness `ValidatorSlashed`
    # and no liveness jail. What survives is the property that actually mattered — a member that
    # resumed its ceremony from the on-disk journal can produce, and a shareless one cannot.
    produced, total = _poll_production_for(ctx, epoch, down_addr)
    ctx.check(case, *vo.evaluate_production_readable(produced, total, down))
    ctx.check(case, *vo.evaluate_produced_something(produced, down))
    ctx.check(case, *vo.evaluate_no_slash(vo.slash_hits(ctx.logs_all_project(), down_addr), down))
    status = ctx.validator_status(down_addr)
    ctx.check(case, *vo.evaluate_not_jailed(status, down))
    _say(ctx, f"{case}: {down} resumed and PRODUCED (producedAt={produced} of "
              f"blocksInEpoch={total}, no equivocation slash, status={status} != jailed)")

    # The chain is still finalizing after the boundary.
    before = ctx.finalized_dec()
    ctx.sleep(vo.POST_BOUNDARY_SLEEP_S)
    ctx.check(case, *vo.evaluate_still_finalizing(before, ctx.finalized_dec()))

    _ok(ctx, case, f"a committee[{epoch}] member restarted mid-window (ceremony journal on disk, "
                   "pre-finalize) RESUMED its ceremony from the on-disk journal, converged to "
                   f"(PK_{epoch}, share), produced byte-identical prev_randao with the survivors, "
                   "and kept PRODUCING, which a shareless node cannot do")


def _poll_production_for(ctx, epoch, addr: str):
    """`case-vrf-dkg-restart-midwindow.sh` — the bounded production-credit retry.

    A SAMPLE COUNT, not a deadline (`ctx.sample_until`): read once, then retry up to five times
    four seconds apart. The retry exists for the -2 read-failed sentinel and for an epoch with no
    recorded blocks yet; a persistent -2 or a zero-block epoch must FAIL the case rather than fall
    through, which is what `evaluate_production_readable` does with whatever this returns."""
    box = {"p": (vo.CREDIT_READ_FAILED, vo.CREDIT_READ_FAILED)}

    def sample(_i):
        box["p"] = ctx.production(epoch, addr, dry_value=(10, 10))
        return vo.credit_ready(box["p"][0], box["p"][1])

    ctx.sample_until(vo.PART_RETRIES, sample, sleep_s=vo.PART_RETRY_SLEEP_S,
                     label=f"producedAt(epoch={epoch})")
    return box["p"]
