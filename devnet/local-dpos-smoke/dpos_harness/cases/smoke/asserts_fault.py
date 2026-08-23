"""asserts_fault.py — the six DESTRUCTIVE assertion bodies of `scripts/asserts-fault.sh`.

Same shape as `asserts.py`: each body decides what to read and when, and hands the readings to
`verdicts_fault.py`. What is different is that every one of these MUTATES the stack — it
restarts, SIGKILLs, CPU-throttles or full-stops nodes — and every one RESTORES it to a healthy,
realigned state before it returns. Its terminal assertion IS the recovery check.

═══ THE RESTORE IS PART OF THE ASSERTION ═════════════════════════════════════════════════

That property is what lets `smoke-fault` chain all six on ONE bring-up, least-invasive first,
with fail-fast: a case hands off to the next only because its own recovery passed. It is also
the thing a port can silently drop. If a body returns with a node still down, the NEXT body runs
against a degraded chain and its verdict means nothing — it would still be green, which is the
failure mode this whole exercise keeps re-finding.

So three rules hold throughout, and none of them is style:

  1. **bash's straight-line order is preserved exactly.** Where bash restores BEFORE applying the
     verdict it brackets (`assert_deferred` unthrottles at :152 and only then checks the growth at
     :153), so does this. Moving the restore after the check would leave a validator throttled on
     every failing run; moving a check before its restore would change what the case measures.
  2. **The one mutation that outlives an exception is bracketed in `finally`.** `docker update
     --cpus` is container CONFIG — a Python exception between the throttle and the restore would
     leave it applied. The stop/kill cases need no such bracket: `driver.run`'s `finally` tears
     the whole stack down, containers included, exactly as bash's `trap tear_down EXIT` did, and
     issuing a `docker start` on a failure path bash never issues would be inventing choreography.
  3. **`shutdown_flushed` is used where bash uses it and nowhere else.** `marshal_ack_tripwire` is
     deliberately NOT called here: `asserts-fault.sh` never calls it (only `lib.sh`'s own stop
     sites and three cases outside this chunk do), and adding it would be new coverage wearing
     the clothes of a port.
"""

from __future__ import annotations

import time

from . import beacon, verdicts, verdicts_fault as vf, verdicts_onchain as vo
from .driver import BEACON_NODES, SmokeFailure
from ...core import nodes, topology

#: The dry stand-in for the restarted victim's log. Non-empty because `logs_required` refuses an
#: empty read, and it carries one line per gate so the dry transcript walks the passing branch of
#: each — a canned log satisfying only some of them would make the poll spin to its budget.
_DRY_RECOVERED_LOG = "\n".join((
    f"INFO {vf.ACTOR_STARTED_LINE} epocher=(dry)",
    f"INFO {vf.CEREMONY_STARTED_LINE} epoch=2",
    f"INFO {vf.SHARE_LINE} epoch=2 height=257",
    f"INFO {vf.PIN_LINE} " + vf.PIN_EPOCH_FMT.format(2),
    f"INFO {vf.PROMOTE_LINE} " + vf.PIN_EPOCH_FMT.format(2),
))


def _survivors(victim: str):
    """`asserts-fault.sh:286,354` — the node set the two VRF fault cases compare while `victim`
    is down. n−f=3 validators PLUS the import follower.

    The follower's membership is the cert-follower half of the property and is not padding: it
    never runs a DKG, so the only way it can hold the same prev_randao is by re-deriving it from
    the cert seed. Dropping it would leave the compare committee-only and would stop proving the
    thing the case is named for."""
    return tuple(s for s in BEACON_NODES if s != victim)


def _ok(ctx, case: str, message: str) -> None:
    """The `OK (<case>): …` line. Suppressed under `--dry-run`, where nothing was measured."""
    if not ctx.dry:
        print(f"OK ({case}): {message}", flush=True)


def _say(ctx, message: str) -> None:
    """A progress line. Also suppressed under dry — the transcript is the artifact there, and a
    running commentary describing measurements that did not happen would read as evidence."""
    if not ctx.dry:
        print(message, flush=True)


# ══ smoke-deferred ════════════════════════════════════════════════════════════════════

def assert_deferred(ctx) -> None:
    """The deferred-execution (F-type) observables the convergence-based cases cannot see.

    Those cases only require cross-node EQUALITY, so a UNIFORM finality overclaim — `finalized ==
    latest` on every node — keeps every one of them green. Three things close that gap:

      1. the K-lag invariant: eth `finalized` trails `latest` by K in steady state (eager derive,
         bundle-20260716T173148Z), transiently K+1 while a derive is in flight, NEVER less — less
         is a result-finality overclaim. The consensus namespace must tell the same story.
      2. result-commitment integrity: the ordering artifact at N+K carries `result` == the derived
         EVM block hash at N. The only check that ties the consensus artifact to execution.
      3. an EL-slowed validator: CPU-throttling one validator must not stall the chain (verify
         budget -> nullify, BFT f=1 holds), and the victim must catch back up afterwards.

    RUNS FIRST in the chain. Its K-lag invariant wants a PRISTINE steady state — before any node
    has been restarted or stopped by a later case — so reordering `smoke-fault` breaks it.
    """
    case = "smoke-deferred"
    k = vf.RESULT_LAG_K

    # Steady state: past the anchor + result window, so the pre-K ramp (finalized clamped to the
    # anchor) cannot skew the lag samples.
    base = ctx.baseline_height(dry_value=100)
    steady = base + k + vf.STEADY_MARGIN
    ctx.check(case, ctx.wait_finalized_ge(steady, vf.STEADY_WAIT_S),
              f"chain did not reach steady state past {steady}")

    _assert_k_lag(ctx, case, k)
    cons_res = _assert_consensus_tiers(ctx, case, k)
    _assert_result_commitment(ctx, case, k, cons_res)
    _assert_el_slowed_validator(ctx, case)


def _block_number_of(ctx, case: str, tag: str, dry_value=0) -> int:
    """`block_number_of <tag>` (asserts-fault.sh:57-59) — `eth_getBlockByNumber` -> decimal.

    FAIL-LOUD on a missing `.result.number`, because bash is: `printf '%d' null` errors and the
    command substitution aborts the case under `set -e`. Reading an unreachable tag as 0 here
    would make `lag = latest - final` an arithmetic fiction — and a fiction that lands inside the
    accepted band roughly as often as outside it."""
    obj = ctx.json_rpc("eth_getBlockByNumber", [tag, False],
                       dry_value={"result": {"number": hex(dry_value)}})
    num = ((obj.get("result") or {}) if isinstance(obj, dict) else {}).get("number")
    if num is None:
        raise SmokeFailure(case, f"eth_getBlockByNumber({tag}) returned no number: {obj} — "
                                 "refusing to read an unreachable tag as a height")
    return nodes.hex_to_dec(num)


def _assert_k_lag(ctx, case: str, k: int) -> None:
    """Step 1 — six readings of the three eth tags (`asserts-fault.sh:75-107`).

    The per-sample band is applied INSIDE the loop, as bash does, so the first bad reading is the
    one reported rather than the last. The two witnesses are applied after all six, because they
    are statements about the SET of samples and no single reading can make them."""
    def sample(i):
        latest = _block_number_of(ctx, case, "latest", dry_value=140)
        final = _block_number_of(ctx, case, "finalized", dry_value=140 - k)
        safe = _block_number_of(ctx, case, "safe", dry_value=140)
        ctx.check(case, *vf.evaluate_lag_sample(latest, final, safe, k))
        return (latest, final, safe)

    samples = ctx.repeat(vf.LAG_SAMPLES, sample, sleep_s=vf.LAG_SAMPLE_SLEEP_S,
                         label="k-lag (latest/finalized/safe)")
    saw_exact, saw_safe_ahead = vf.lag_witnesses(samples, k)
    ctx.check(case, *vf.evaluate_lag_witnesses(saw_exact, saw_safe_ahead, k))
    _say(ctx, f"  K-lag (eth): latest − finalized within {k}..{k + vf.LAG_MAX_OVER} and "
              f"safe − finalized == {k} (eager-derive steady state) held across "
              f"{vf.LAG_SAMPLES} samples")
    _say(ctx, f"  safe-tier: finalized ≤ safe ≤ latest, latest−safe < {k} held")


def _assert_consensus_tiers(ctx, case: str, k: int) -> int:
    """Step 1b — the consensus namespace agrees (`asserts-fault.sh:115-130`). Returns
    `latestResultFinalized`, which step 2 anchors its artifact lookup on."""
    seen = {}

    def sampled_exact(i):
        obj = ctx.json_rpc("consensus_getLatest", [],
                           dry_value={"result": {"latestFinalized": {"height": 140},
                                                 "latestResultFinalized": 140 - k}})
        res = (obj.get("result") or {}) if isinstance(obj, dict) else {}
        lf = res.get("latestFinalized")
        cons_fin = lf.get("height") if isinstance(lf, dict) else None
        cons_res = res.get("latestResultFinalized")
        ok, msg, exact = vf.evaluate_consensus_sample(cons_fin, cons_res, k, obj)
        ctx.check(case, ok, msg)
        seen["fin"], seen["res"] = cons_fin, cons_res
        return exact

    exact = ctx.sample_until(vf.CONSENSUS_SAMPLES, sampled_exact, sleep_s=vf.CONSENSUS_SLEEP_S,
                             label="consensus tier gap")
    ctx.check(case, *vf.evaluate_consensus_exact(bool(exact), k))

    cons_res = int(seen.get("res") or (140 - k))
    eth_final = _block_number_of(ctx, case, "finalized", dry_value=cons_res)
    ctx.check(case, *vf.evaluate_tier_skew(eth_final, cons_res))
    _say(ctx, f"  K-lag (consensus): latestFinalized={seen.get('fin')} − "
              f"latestResultFinalized={cons_res} == {k} (eager derive), matches eth "
              f"finalized={eth_final}")
    return cons_res


def _assert_result_commitment(ctx, case: str, k: int, n: int) -> None:
    """Step 2 — the artifact at N+K commits the derived hash of N (`asserts-fault.sh:134-171`)."""
    height = n + k
    canned_hash = "0x" + "cd" * 32
    artifact = ctx.json_rpc("consensus_getFinalization", [{"height": height}],
                            dry_value={"result": {"block": "0x" + "ab" * (vf.WIRE_RESULT_OFFSET
                                                                         // 2)
                                                          + "cd" * 32 + "ef" * 32}})
    wire = ((artifact.get("result") or {}) if isinstance(artifact, dict) else {}).get("block")
    ok, msg, committed = vf.evaluate_artifact_wire(wire, height, artifact)
    ctx.check(case, ok, msg)

    block = ctx.json_rpc("eth_getBlockByNumber", [hex(int(n)), False],
                         dry_value={"result": {"hash": canned_hash}})
    derived = ((block.get("result") or {}) if isinstance(block, dict) else {}).get("hash") or ""
    ctx.check(case, *vf.evaluate_result_commitment(committed, derived, height, n, wire=wire))
    _say(ctx, f"  result commitment: artifact({height}).result == eth hash({n}) == "
              f"0x{(derived or '')[2:18]}…")


def _assert_el_slowed_validator(ctx, case: str) -> None:
    """Step 3 — throttle one validator's CPU, prove the chain stays live, then prove the victim
    rejoins (`asserts-fault.sh:173-201`).

    THE ORDER IS THE TEST, and it is bash's exactly: read `pre`, throttle, wait out the window,
    read `during`, RESTORE, and only then judge. Judging before the restore would leave a
    validator pinned at 0.15 CPU on every failing run — and, in the chained `smoke-fault`, would
    hand the next four assertions a chain that is quietly degraded rather than one that is broken.

    The `finally` is what makes that guarantee hold for an exception too. It does not change the
    passing path: the restore lands in the transcript in exactly the position bash puts it.
    """
    victim = topology.validator(vf.DEFERRED_VICTIM_IDX)
    cid = ctx.compose_ps_q(victim)
    ctx.check(case, bool(cid), f"no container for {victim}")

    pre = ctx.finalized_dec(dry_value=140)
    _say(ctx, f"  throttling {victim} to {vf.THROTTLE_CPUS} cpu (pre={pre})")
    ctx.docker_update_cpus(cid, vf.THROTTLE_CPUS, note="throttle-victim")
    try:
        ctx.sleep(vf.THROTTLE_WINDOW_S)
        during = ctx.finalized_dec(dry_value=pre + vf.THROTTLE_MIN_GROWTH)
    finally:
        ctx.docker_update_cpus(cid, vf.RESTORE_CPUS, note="unthrottle-victim")
    ctx.check(case, *vf.evaluate_throttle_liveness(pre, during))
    _say(ctx, f"  chain stayed live under throttle: finalized {pre} → {during}")

    # Rejoin: the victim's OWN finalized view must reach the network tip observed at unthrottle
    # time — not merely "it answers RPC again", which a node stuck mid-catch-up also does.
    def rejoined():
        head = ctx.check_node(victim, dry_value=f"{hex(during)}|0xh").split("|", 1)[0]
        return head if vf.victim_rejoined(head, during) else None

    got = ctx.poll(rejoined, vf.REJOIN_TIMEOUT_S, poll_s=vf.REJOIN_POLL_S,
                   dry_value=hex(during))
    ctx.check(case, bool(got),
              lambda: (f"{victim} did not rejoin after unthrottle (victim="
                       f"{ctx.check_node(victim)}, "
                       f"v0={ctx.check_external(topology.HOST_RPC_PORT)})"))
    _ok(ctx, case, "K-lag invariant + result commitment + EL-slowed liveness "
                   f"(victim rejoined at {got} >= {during})")


# ══ smoke-peers ═══════════════════════════════════════════════════════════════════════

def assert_peers(ctx) -> None:
    """Both peer planes connect, and a RESTARTED node re-establishes both.

      * the commonware CONSENSUS plane: discovery connects each validator to its committee peers,
        observed through validator-0's host-published registry. The tracked peer set equals the
        on-chain committee (Addendum B), so a healthy node settles at exactly `committee_size-1`.
      * the reth DEVP2P plane (the EL transport for block sync and catch-up): each spoke pins
        validator-0's enode via `--trusted-peers`, so `net_peerCount > 0`. This is the regression
        guard for `dpos_rejoin_el_sync_devp2p` — under `--dpos` the override must keep reth
        peering wired, and when it does not, the breakage only surfaces much later as a node that
        cannot rejoin.

    The restart is graceful (`docker compose restart`), so this is the LEAST invasive of the five
    mutations and runs second in the chain.
    """
    case = "smoke-peers"
    victim = topology.validator(vf.PEERS_VICTIM_IDX)

    cur = verdicts.first_token(ctx.staking_call("currentEpoch()(uint256)", dry_value="2"))
    committee = ctx.staking_call("getEpochCommittee(uint64)(address[])", cur,
                                 dry_value="[0x" + "11" * 20 + ", 0x" + "22" * 20 + "]")
    size = vf.committee_size(committee)
    expect = size - 1
    _say(ctx, f"smoke-peers: committee_size={size} → expect connected={expect} on "
              f"{topology.PINNED_RPC_HOST}")

    canned = "\n".join(f'outer_engine_buffered_peer_total{{sequencer="{i:02x}"}} 1'
                       for i in range(max(expect, 0)))

    def connected():
        return vf.connected_count(ctx.peers_metrics(dry_value=canned))

    ctx.poll(lambda: connected() == expect, vf.PEERS_SETTLE_S, poll_s=vf.PEERS_POLL_S)
    count = connected()
    ctx.check(case, *vf.evaluate_connected(count, expect),
              on_fail=lambda: _dump_peer_series(ctx))
    _say(ctx, f"  initial: connected={count} (== committee_size-1)")

    # devp2p handshakes can lag commonware discovery, so this gets its own poll rather than
    # sharing the one above — a combined wait would report whichever plane is slow as the failure.
    ctx.poll(lambda: ctx.peer_count(victim) > 0, vf.RETH_PEERS_SETTLE_S, poll_s=vf.PEERS_POLL_S)
    peers = ctx.peer_count(victim)
    ctx.check(case, *vf.evaluate_reth_peers(peers, victim))
    _say(ctx, f"  initial: {victim} reth devp2p peers={peers} (> 0)")

    # Reconnect. The chain-advance leg is what makes this a rejoin assertion: two peer planes can
    # reconnect perfectly around a node that contributes nothing.
    pre = ctx.baseline_height(dry_value=140)
    ctx.compose_restart(victim, note="peers-restart-victim")

    def back():
        return vf.peers_reconnected(connected(), expect, ctx.peer_count(victim),
                                    ctx.finalized_dec(dry_value=pre + 1), pre)

    got = ctx.poll(back, vf.RECONNECT_S, poll_s=vf.RECONNECT_POLL_S)
    ctx.check(case, bool(got),
              lambda: (f"after {victim} restart connected={connected()} (want {expect}), reth "
                       f"peers={ctx.peer_count(victim)} (want >0), "
                       f"finalized={ctx.finalized_dec()} (want > {pre})"))
    _ok(ctx, case, f"commonware connected={expect} + {victim} reth peers>0 + chain advanced past "
                   f"{pre} after restart")


def _dump_peer_series(ctx) -> None:
    """`asserts-fault.sh:247` — on a connected-count failure, print the peer series that ARE
    exported, so a metric RENAME surfaces as a rename rather than as "discovery is broken"."""
    text = ctx.peers_metrics()
    print("  buffered_peer_total / peer_performance series present:", flush=True)
    hits = [ln for ln in (text or "").splitlines()
            if "buffered_peer_total" in ln or "peer_performance" in ln]
    for ln in hits[:20]:
        print(f"  {ln}", flush=True)
    if not hits:
        print("  (none — the registry exported no peer family at all)", flush=True)


# ══ smoke-vrf-fault ═══════════════════════════════════════════════════════════════════

def assert_vrf_fault(ctx) -> None:
    """The threshold beacon under FAULT + RESTART + deep catch-up.

      A1 — with an f=1 validator DOWN the beacon SURVIVES: the n−f seed quorum of survivors still
           recovers the threshold seed, and prev_randao stays threshold-active and byte-identical
           on the survivors for the whole time the node is down.
      B3/B4 — the downed validator RESTARTS, reloads its share and CATCHES UP: every gap block it
           missed comes back with the CERT-RECOVERED threshold seed (assurance), not the
           `order.digest()` fallback. Byte-identical to a node that never went down — a fork or a
           fallback would diverge. This folds item I (keyless restart) and the executor catch-up
           seed-availability invariant.

    The victim is stopped GRACEFULLY and restarted, so the stack is whole again on return.
    """
    case = "smoke-vrf-fault"
    victim = topology.validator(vf.VRF_FAULT_VICTIM_IDX)
    survivors = _survivors(victim)

    # The beacon is threshold-active from EPOCH 2 (deterministic bootstrap). Both the fault window
    # and the victim's share reload must exercise a SEEDED epoch, so wait into epoch 2 first —
    # a fault injected during the seedless epochs would prove nothing about the threshold seed.
    epoch2_probe = (verdicts.beacon_active_epoch_start(ctx.activation_block, ctx.interval)
                    + vf.VRF_FAULT_EPOCH2_MARGIN)
    _say(ctx, f"smoke-vrf-fault: beacon active from epoch 2; waiting for finalized >= "
              f"{epoch2_probe} before the fault")
    ctx.check(case, ctx.wait_finalized_ge(epoch2_probe, vf.VRF_FAULT_EPOCH2_WAIT_S),
              f"chain did not reach the epoch-2 window ({epoch2_probe}) before the fault",
              on_fail=lambda: ctx.dump_logs(vf.VRF_FAULT_LOG_TAIL, topology.validator(0)))
    ctx.check(case, ctx.finalized_dec(dry_value=epoch2_probe) > 0, "no finalized block")

    # A1 — one validator down (f=1). With 4 validators the seed quorum is n−f=3, so the three
    # survivors still recover the threshold seed and the beacon stays live.
    _say(ctx, f"smoke-vrf-fault: stopping {victim} (f=1 fault) — the beacon must stay live on the "
              "survivors")
    ctx.compose_stop(victim, note="vrf-fault-stop-victim")
    down_at = ctx.finalized_dec(dry_value=epoch2_probe)
    gap_target = down_at + vf.VRF_FAULT_GAP
    ctx.check(case, ctx.wait_finalized_ge(gap_target, vf.VRF_FAULT_GAP_WAIT_S),
              f"A1 — chain stalled with {victim} down (survivors below n−f quorum?)",
              on_fail=lambda: ctx.dump_logs(vf.VRF_FAULT_LOG_TAIL, topology.validator(0)))

    a_lo = down_at + 2
    a_hi = gap_target - 1
    ctx.check(case, beacon.wait_nodes_have(ctx, a_hi, vf.VRF_FAULT_NODES_HAVE_S, survivors),
              f"A1 — survivors did not all reach block {a_hi}")
    beacon.assert_beacon_window(ctx, case, a_lo, a_hi, "f=1-down", survivors)
    _say(ctx, f"smoke-vrf-fault: A1 — beacon survived the f=1 fault, active + byte-identical on "
              f"survivors over [{a_lo}..{a_hi}]")

    # B3/B4 — restart the victim and require the gap blocks back with the SAME threshold seed.
    _say(ctx, f"smoke-vrf-fault: restarting {victim} — it must catch up the gap with verified "
              "prev_randao")
    ctx.compose_start(victim, note="vrf-fault-start-victim")
    _await_catchup(ctx, case, victim, a_hi, vf.VRF_FAULT_CATCHUP_S, vf.VRF_FAULT_CATCHUP_POLL_S,
                   f"B4 — {victim} did not catch up to block {a_hi} within the deadline")

    rows = [(n, ctx.mixhash_in(victim, n, dry_value=f"0x{n:064x}"),
             ctx.mixhash_at(n, dry_value=f"0x{n:064x}"))
            for n in range(a_lo, a_hi + 1)]
    ctx.check(case, *vf.evaluate_gap_mixhashes(
        rows, victim,
        f"B4 — restarted {victim} derived divergent prev_randao on gap blocks (fell to fallback "
        "/ forked instead of recovering the cert seed):"))
    _say(ctx, f"smoke-vrf-fault: B3/B4 — {victim} restarted, caught up, and re-obtained the gap "
              f"[{a_lo}..{a_hi}] with the byte-identical threshold prev_randao (assurance, not "
              "fallback)")

    _ok(ctx, case, "beacon survived the f=1 fault; the downed validator restarted, reloaded its "
                   "share, and caught up the gap with verified threshold prev_randao")


def _await_catchup(ctx, case: str, service: str, block, timeout, poll_s, message) -> None:
    """Wait until `service` can serve `block` at all, then fail loud with its log tail.

    Shared by both VRF fault cases, which do the identical `mixhash_in` spin. The reading being
    waited for is deliberately the VICTIM's own in-container view: the point is that the restarted
    node has the block, not that some node does."""
    def has_it():
        mh = ctx.mixhash_in(service, block, dry_value=f"0x{int(block):064x}")
        return bool(mh) and mh != "null"

    ctx.check(case, bool(ctx.poll(has_it, timeout, poll_s=poll_s)), message,
              on_fail=lambda: ctx.dump_logs(vf.VRF_FAULT_LOG_TAIL, service))


# ══ smoke-vrf-dkg-live-heal ═══════════════════════════════════════════════════════════

def assert_vrf_dkg_live_heal(ctx) -> None:
    """A committee member that missed its OWN epoch key agreement obtains it and signs inside
    that epoch (FLU-1166). Same scenario as before; the verdict is the opposite one.

    WHAT INVERTED, AND WHY IT USED TO READ THE OTHER WAY. Nothing ever fetched the agreed artifact
    for the LIVE epoch: the epoch manager's repair sweep excludes `epoch >= frontier` by design,
    and the demoted-member recompute-heal bailed silently when the artifact was not already local.
    So a member absent through its own DKG window sat the whole epoch out as a verifier and, if it
    healed at all, healed into an epoch that was already over. That sit-out is what this case
    asserted, and asserting it was pinning a hole in place. The `DkgActor` now asks for the live
    epoch's artifact at exactly the point where "member of committee[E], holds no share, has no
    artifact" is fully known, and everything downstream of it already worked.

    THIS IS THE FIRST LIVE COVERAGE OF THE REVEAL-FALLBACK PATH, which has no test anywhere in
    the repo — unit or otherwise. The victim is offline for the WHOLE deal phase, so it receives
    no dealing and acknowledges none; every honest dealer therefore REVEALS the victim's private
    point publicly in its own sealed log, and the reconstruction rebuilds the share from those
    reveals rather than from an ack the victim never sent. Nothing else in the tree exercises it.

    That claim rests on ONE reading, and the case gates on it rather than on the stop instant:
    `live DKG: ceremony started epoch=2` written by the RESTARTED process. `start_fresh` is its
    only emitter and `JournalLoad::NoFile` is the only arm that reaches `start_fresh`, so the line
    says the victim came back holding no epoch-2 journal at all. Inferring the same thing from
    "we stopped it early enough" is a race against a moving chain, and that race has been lost.

    TWO ROADS REACH THE SHARE, AND THE CASE ACCEPTS EITHER. As it catches up, the restarted victim
    replays its DKG clock through epoch 1, so `maybe_start(now + 1)` fires for epoch 2 and — with
    no journal — starts a FRESH ceremony. That ceremony fetches the pinned dealers' logs over the
    resolver and rebuilds the share from their reveals. What differs between runs is only who
    finishes: the live ceremony's `finalize_over_pinned` (`SHARE_LINE`) when it beats the
    past-boundary sweep, or the demote-heal's scoped recompute (`HEAL_LINE`) over the journal that
    ceremony just built, when the sweep gets there first. BOTH were observed live on this case,
    on this geometry, minutes apart. An earlier version asserted the heal road AND the absence of
    the other road's line, which made one of two legitimate outcomes a red run.

    ABSENTEES MUST STAY <= f, AND AT n=4 THAT IS EXACTLY ONE. `max_reveals = f` over the PLAYER
    set, enforced twice: a dealer withholds its ENTIRE log on `TooManyReveals`, and verifiers
    reject such a log. Downing a second validator would therefore not make this a harder version
    of the same test — every honest dealer's log would self-destruct, the ceremony would fail
    outright, and the case would be measuring a failed DKG. Do not "improve" the coverage by
    adding a victim.

    THE TIMING IS STILL THE ASSERTION, AND THE EDGE IT KEYS ON MOVED. The victim must be down
    before the DEAL phase opens, which is when the actor's clock first enters epoch 1 —
    `epoch_start(1) - K`, because `maybe_start(now + 1)` runs on every tick and the clock is
    `finalized + K`. It is NOT `epoch_start(2) - DKG_MARGIN_BLOCKS`; that is where the phase
    CLOSES. See `vf.dkg_deal_window_open`.

    Seven legs:
      1. the chain reaches epoch 2 and finalizes WHILE the victim is down (the beacon went live on
         the n-f=3 survivors);
      2. the restarted victim starts a FRESH epoch-2 ceremony — it held no journal, so it was
         genuinely absent and its points can only come from the dealers' public reveals;
      3. it PULLED epoch 2's agreed artifact (`dpos_dkg_artifact_pull_ok_total > 0`);
      4. it RECOVERED the epoch-2 share, by either road;
      5. its epoch-2 scheme is upgraded to PINNED and it is SEATED as a signer for epoch 2, so its
         own certificate admission stops being seed-blind and it can vote;
      6. the chain stays live with it back in the quorum and its epoch-2 prev_randao is
         byte-identical to the survivors';
      7. it PRODUCED in epoch 2 — `producedAt(2, victim) > 0`, read once epoch 2 has ENDED so the
         counter is FINAL, which a share-less member cannot do and which is the only leg that
         observes the whole chain of consequences at once.

    PACING RIDES THE SHORTHANDED SPAN, not the post-recovery one. The victim is seated ~16 blocks
    into a 64-block epoch 2, so a 60 s window opened after that would run off the end of the epoch
    — and the shorthanded span is where this case's fault actually is (n-f=3 carrying the chain
    with a committee member down), which makes it the more interesting of the two anyway.

    THE PRODUCTION COUNTER IS READ AFTER EPOCH 2 HAS ENDED. `producedAt(2, …)` climbs for as long
    as the epoch runs, so a sample taken at the moment of recovery reports a partial count on a
    4-member stake-weighted lottery, and a zero there is indistinguishable from a recovery that did
    not work. The case waits past `epoch_start(3)` plus the K-block deferred-execution lag — the
    counter for height h is written when h EXECUTES — and reads the FINAL count.
    """
    case = "smoke-vrf-dkg-live-heal"
    victim = topology.validator(vf.VRF_FAULT_VICTIM_IDX)
    survivors = _survivors(victim)

    epoch2_start = verdicts.beacon_active_epoch_start(ctx.activation_block, ctx.interval)
    epoch3_start = verdicts.epoch_start(ctx.activation_block, ctx.interval, 3)
    # THE DEAL WINDOW, NOT THE SEAL DEADLINE. `maybe_start(now + 1)` runs on every height tick, so
    # committee[2]'s ceremony opens the instant the actor's clock enters epoch 1 — the whole of
    # epoch 1 is its deal phase, and `DKG_MARGIN_BLOCKS` is only where that phase CLOSES. Guarding
    # on the margin let the victim be stopped mid-phase, having already acked every dealing, which
    # is a different experiment that passes every other assertion in this case. See
    # `vf.dkg_deal_window_open`.
    deal_open = vf.dkg_deal_window_open(ctx.activation_block, ctx.interval)
    boundary_probe = epoch2_start + vf.DKG_BOUNDARY_MARGIN

    _say(ctx, f"smoke-vrf-dkg-live-heal: bringing {victim} down BEFORE the epoch-2 DKG DEAL "
              f"window opens (finalized < {deal_open} = epoch_start(1) - K) — ONE victim, "
              "because absentees must stay <= f")
    ctx.check(case, *vf.evaluate_window_open(ctx.finalized_dec(dry_value=0), deal_open))
    ctx.compose_stop(victim, note="dkg-live-heal-stop-victim")

    # THE VICTIM'S ON-CHAIN ADDRESS, resolved AFTER the stop and not before it. The window guard
    # is the ONE read that may precede the stop — the case's whole timing argument is that nothing
    # slows the interval between reading the height and taking the node down. This read costs a
    # `docker compose exec … cat` against validator-0's mount, needs the victim only to be a
    # committee member (not to be running), and failing here rather than at the production leg
    # six minutes later is the reason it is not deferred further.
    addrs = ctx.runtime_addresses()
    if not ctx.dry and len(addrs) <= vf.VRF_FAULT_VICTIM_IDX:
        raise SmokeFailure(case, f"/runtime/addresses.json listed {len(addrs)} validators — "
                                 f"cannot resolve {victim}'s on-chain address")
    victim_addr = (addrs[vf.VRF_FAULT_VICTIM_IDX] if len(addrs) > vf.VRF_FAULT_VICTIM_IDX
                   else "0x" + "a3" * 20)

    # PACING, MEASURED SHORTHANDED, and measured HERE. The existing instrument and the existing
    # band (45..66 per 60 s) — the one that produced the historical 26-27 blk/60s regression
    # reading — over the window where this case's fault is actually applied: n-f=3 validators
    # carrying the chain with a committee member down.
    #
    # WHY NOT AFTER THE RECOVERY, WHICH IS WHERE IT WOULD READ MORE INTERESTINGLY. There is no
    # room. The victim is seated ~16 blocks into a 64-block epoch 2, so any 60 s window after that
    # runs into the epoch-2→3 boundary the production leg is waiting for, and would be measuring
    # the boundary rather than the shorthanded chain. The window costs nothing here: the wait for
    # the boundary probe below is longer than it either way.
    r0 = ctx.finalized_dec(dry_value=deal_open - 40)
    ctx.sleep(verdicts.PACING_WINDOW_S)
    r1 = ctx.finalized_dec(dry_value=deal_open - 40 + verdicts.PACING_MIN_BLOCKS + 5)
    ctx.check(case, *verdicts.evaluate_pacing(r1 - r0))
    _say(ctx, f"smoke-vrf-dkg-live-heal: pacing on the n-f=3 survivors with {victim} down: "
              f"{r1 - r0} blk/{verdicts.PACING_WINDOW_S}s")

    # 1) the chain crosses the epoch-2 boundary on the n-f=3 survivors: committee[2]'s DKG
    #    completed WITHOUT the offline member, and the beacon is live from epoch 2.
    _say(ctx, f"smoke-vrf-dkg-live-heal: waiting for finalized >= {boundary_probe} with {victim} "
              "down (n-f=3 quorum must seed epoch 2)")
    ctx.check(case, ctx.wait_finalized_ge(boundary_probe, vf.DKG_BOUNDARY_WAIT_S),
              f"chain did not reach the epoch-2 boundary with {victim} down (survivors below n-f "
              "quorum / DKG could not complete shorthanded)",
              on_fail=lambda: ctx.dump_logs(vf.VRF_FAULT_LOG_TAIL, topology.validator(0)))
    ctx.check(case, beacon.wait_nodes_have(ctx, boundary_probe, vf.DKG_NODES_HAVE_S, survivors),
              f"survivors did not all reach {boundary_probe}")
    beacon.assert_beacon_window(ctx, case, epoch2_start, boundary_probe, "dkg-live-heal-epoch2",
                                survivors)
    _say(ctx, f"smoke-vrf-dkg-live-heal: beacon went LIVE at epoch 2 on the {len(survivors)} "
              f"survivors while {victim} was offline for its whole DKG window")

    # 2) restart the victim; it must obtain the key and heal INSIDE epoch 2.
    _say(ctx, f"smoke-vrf-dkg-live-heal: restarting {victim} — it must PULL epoch 2's artifact "
              "and recompute its share from the dealers' reveals")
    ctx.compose_start(victim, note="dkg-live-heal-start-victim")
    _await_catchup(ctx, case, victim, boundary_probe, vf.DKG_CATCHUP_S, vf.DKG_CATCHUP_POLL_S,
                   f"{victim} did not catch up to {boundary_probe} after restart")

    # The reconstruction is off the beacon's own height tick, so it lands a few seconds after the
    # catch-up rather than with it. ONE log read per iteration answers every grep below — they are
    # all questions about the same text (the `asserts_onchain.resumed` trade).
    box = {"fresh": "", "road": None, "pin": [], "promote": [], "logs": ""}

    def recovered():
        box["logs"] = ctx.logs_required(victim, case, "epoch-2 key recovery",
                                        dry_value=_DRY_RECOVERED_LOG)
        box["fresh"] = vf.started_fresh_after_restart(box["logs"], epoch=2)
        box["road"] = vf.share_road(box["logs"], epoch=2)
        box["pin"] = vf.pin_upgrade_lines(box["logs"], epoch=2)
        box["promote"] = vf.promote_lines(box["logs"], epoch=2)
        return all((box["fresh"], box["road"], box["pin"], box["promote"]))

    ctx.poll(recovered, vf.DKG_HEAL_S, poll_s=vf.DKG_HEAL_POLL_S)
    # WHERE the chain was when the victim was seated. Read here and not later, because every read
    # after this moves it: it is what tells the production leg's report how much of epoch 2 the
    # member actually had to be elected in, which is the difference between a lottery loss and a
    # broken recovery when the credit comes back small.
    seated_at = ctx.finalized_dec(dry_value=boundary_probe + 12)

    def dump():
        ctx.dump_logs(vf.VRF_FAULT_LOG_TAIL, victim)

    # THE CASE VERIFIES IT SET UP WHAT IT CLAIMS TO TEST, the same way the seed-slot MITM reads
    # back its own corruption before anything concludes from a rejection. `start_fresh` is the
    # only emitter of `ceremony started` and `JournalLoad::NoFile` is the only arm that reaches
    # it, so this line on the RESTARTED process is proof the victim came back holding no epoch-2
    # journal — no dealing received, no ack sent, and therefore nothing any dealer could do with
    # its point except reveal it publicly. That is the reveal fallback's precondition, read off
    # the log instead of inferred from the stop instant.
    ctx.check(case, *vf.evaluate_started_fresh(box["fresh"], victim, epoch=2), on_fail=dump)
    # …the mechanism the ticket is about…
    ctx.check(case, *vf.evaluate_artifact_pull_ok(
        ctx.node_metric(victim, vf.ARTIFACT_PULL_OK_SAMPLE, dry_value="1"), victim), on_fail=dump)
    # …and the outcome, by EITHER road (see `vf.SHARE_ROADS`).
    ctx.check(case, *vf.evaluate_share_acquired(box["road"], victim, epoch=2), on_fail=dump)
    ctx.check(case, *vf.evaluate_pin_upgraded(box["pin"], victim, epoch=2), on_fail=dump)
    ctx.check(case, *vf.evaluate_promoted(box["promote"], victim, epoch=2), on_fail=dump)
    _say(ctx, f"smoke-vrf-dkg-live-heal: {victim} came back with NO epoch-2 journal, pulled the "
              "epoch's agreed artifact, rebuilt its share from the other members' sealed dealer "
              f"logs via {box['road'][1]}, left vote-only admission and was seated as a signer:")
    for line in [box["fresh"], box["road"][0], *box["pin"], *box["promote"]]:
        _say(ctx, f"    {line}")

    # 3) the chain is still live with the recovered member back in the quorum, and its epoch-2
    #    prev_randao is byte-identical to the survivors'. BEFORE the production wait, not after:
    #    these two are the cheapest legs and the production wait is the longest, so a failure in
    #    either is reported minutes earlier this way.
    before = ctx.finalized_dec(dry_value=boundary_probe + 12)
    rows = [(v, ctx.mixhash_in(victim, v, dry_value=f"0x{v:064x}"),
             ctx.mixhash_at(v, dry_value=f"0x{v:064x}"))
            for v in range(epoch2_start, boundary_probe + 1)]
    ctx.check(case, *vf.evaluate_gap_mixhashes(
        rows, victim,
        f"restarted {victim} derived divergent prev_randao (did not recover the cert seed):"))
    ctx.sleep(vf.DKG_LIVENESS_WINDOW_S)
    after = ctx.finalized_dec(dry_value=before + 6)
    ctx.check(case, *vf.evaluate_still_finalizing(before, after, victim))
    _say(ctx, f"smoke-vrf-dkg-live-heal: chain still finalizing with {victim} back in the quorum "
              f"({before} -> {after}); its epoch-2 prev_randao is byte-identical to the "
              "survivors'")

    # 4) THE LOAD-BEARING LEG — it PRODUCED inside the epoch it was elected for. A share-less
    #    member cannot; every leg above is a step on the road to this one.
    #
    # READ ONCE EPOCH 2 HAS ENDED, and that is the whole of what makes it a measurement rather
    # than a coin toss. `producedAt(2, idx)` climbs for as long as epoch 2 runs, so a read taken
    # seconds after the victim is seated reports how many slots it has won SO FAR — which on a
    # 4-member stake-weighted lottery is frequently zero, and a zero there is indistinguishable in
    # the failure message from a recovery that did not work. That is exactly how this case first
    # went red: seated at height 257, read at ~260 (`producedAt=0`), 10 of the epoch's 64 blocks by
    # the time the epoch actually ended.
    #
    # The floor is `epoch_start(3) + K` and not the boundary itself, because the counter for
    # height h is written when h EXECUTES and that is K heights later: epoch 2's last block
    # (`epoch3_start - 1`) is only credited at `epoch3_start - 1 + K`. One block past that, the
    # count is FINAL and the sample is the whole epoch rather than a prefix of it.
    #
    # This makes the leg ride the epoch-2→3 crossing, which is deliberate: a chain that cannot
    # cross fails here on the wait, with the height it stopped at, rather than surfacing minutes
    # later as an unexplained `producedAt=0`.
    sample_floor = epoch3_start + vf.RESULT_LAG_K
    _say(ctx, f"smoke-vrf-dkg-live-heal: {victim} seated at finalized={seated_at}; letting epoch 2 "
              f"run out (to {sample_floor} = epoch_start(3) + K) so production credit is FINAL "
              "when it is read")
    ctx.check(case, ctx.wait_finalized_ge(sample_floor, vf.DKG_EPOCH_END_S),
              f"chain did not finalize to {sample_floor} (= epoch_start(3) + K) — it stalled "
              f"below that floor, either still inside epoch 2 or already across the epoch-2→3 "
              f"boundary at {epoch3_start} but short of the K deferred-execution heights that "
              f"credit epoch 2's last block. Either way `producedAt(2, …)` is still climbing, "
              f"so reading it here would say nothing about {victim}'s recovery",
              on_fail=dump)
    produced, total = _poll_production_for(ctx, 2, victim_addr)
    ctx.check(case, *vo.evaluate_production_readable(produced, total, victim), on_fail=dump)
    # The retired room gate (`evaluate_heal_left_room`) was the only thing that stopped a
    # very late heal from being reported as a broken recovery when it was really a lost
    # lottery draw. Reading the FINAL counter shrinks that window to a few blocks but does
    # not close it, so the seating context rides the failure message instead of a new gate.
    # `DKG_HEAL_S` is 240 s against a ~64 s epoch, so the seating can also land AT or PAST the
    # boundary — that reading gets its own wording rather than a negative block count.
    room = epoch3_start - seated_at
    room_note = (f"leaving it {room} blocks of epoch 2 to be elected in" if room > 0 else
                 f"which is {-room} blocks past the epoch-2→3 boundary at {epoch3_start} — no "
                 "block of epoch 2 was left for it to be elected in")
    produced_ok, produced_why = vo.evaluate_produced_something(produced, victim)
    if not produced_ok:
        produced_why += (
            f" — seated at {seated_at}, {room_note}; "
            + ("a seating within a handful of blocks of the boundary can lose the "
               "stake-weighted lottery outright, so read that number before calling this a "
               "recovery failure" if room > 0 else
               "the heal landed after the epoch it was elected for had already ended, so this "
               "reads as a LATE heal rather than evidence the recovered share never worked"))
    ctx.check(case, produced_ok, produced_why, on_fail=dump)
    _say(ctx, f"smoke-vrf-dkg-live-heal: {victim} PRODUCED in epoch 2 (producedAt={produced} of "
              f"blocksInEpoch={total}, FINAL — it was seated at {seated_at}, {room_note})")

    _ok(ctx, case, "a member offline through its whole epoch-2 DKG window PULLED the epoch's "
                   "agreed artifact, recomputed its share from the dealers' public reveals (the "
                   "reveal-fallback path, first live coverage), left vote-only admission, and "
                   "PRODUCED inside epoch 2 — while the chain finalized throughout on the n-f=3 "
                   "survivors and its prev_randao stayed byte-identical to theirs")


def _poll_production_for(ctx, epoch, addr: str):
    """The bounded production-credit retry, in `asserts_onchain._poll_production_for`'s shape.

    A SAMPLE COUNT, not a deadline: read once, then retry a few times. The retry exists for the
    -2 read-failed sentinel and for an epoch with no recorded blocks yet; a persistent -2 or a
    zero-block epoch must FAIL the case rather than fall through, which is what
    `evaluate_production_readable` does with whatever this returns."""
    box = {"p": (vo.CREDIT_READ_FAILED, vo.CREDIT_READ_FAILED)}

    def sample(_i):
        box["p"] = ctx.production(epoch, addr, dry_value=(10, 10))
        return vo.credit_ready(box["p"][0], box["p"][1])

    ctx.sample_until(vo.PART_RETRIES, sample, sleep_s=vo.PART_RETRY_SLEEP_S,
                     label=f"producedAt(epoch={epoch})")
    return box["p"]


# ══ smoke-crash-survivor ══════════════════════════════════════════════════════════════

def assert_crash_survivor(ctx) -> None:
    """Problem A — a validator CRASHED ungracefully mid-operation recovers and realigns.

    SIGKILL means no persistence flush, so the node comes back with an EL that is behind by
    whatever the chain produced while it was gone. It must backfill that gap and realign to the
    honest finalized head instead of WEDGING on a missing block. Contrast `smoke-liveness`, which
    uses a graceful `stop` and therefore proves something much weaker.

    A restart that merely returns is NOT the assertion, and neither is a restart that merely
    agrees. The verdict has two halves: the victim is on the PRODUCER'S CHAIN at its own height
    (so a fork at the same height fails, and two unreachable nodes — trivially equal — do not
    pass) AND it is past `head`, the height the chain was measured at while the victim was down.
    Without that second half the case is vacuous: coming back on its own persisted tail satisfies
    everything else, which is exactly what a live run printed — `realigned at 0x41(=65)` against
    `v0=0x50(=80)`, fifteen blocks of un-backfilled gap, PASSED.

    Three details are load-bearing and each was a diagnosed flake:
      * the kill and the restart go through the RAW container id, bypassing compose, because
        `docker compose start` re-runs the service's dependencies and re-running `genesis-init`
        races the ungraceful path;
      * the gap wait is SOFT (`|| true` in bash) and the hard floor is `pre + 3`, so a slow chain
        does not fail a case about recovery;
      * the recovery budget is ten minutes, deliberately, because the question it answers is
        whether the post-crash `connected_peers=0` is PERMANENT or merely slow — a short deadline
        reports the second as the first.
    """
    case = "smoke-crash-survivor"
    victim = topology.validator(vf.CRASH_VICTIM_IDX)
    pre = ctx.baseline_height(dry_value=140)

    cid = ctx.compose_ps_q(victim)
    ctx.check(case, bool(cid), f"could not resolve {victim} container id")
    _say(ctx, f"smoke-crash-survivor: SIGKILL {victim} ({cid}) at finalized={pre} (ungraceful, "
              "no flush)")
    ctx.docker_kill(cid, note="crash-sigkill-victim")

    # Let the chain build the gap. Soft target; the hard assert is the +3 floor below.
    ctx.wait_finalized_ge(pre + vf.CRASH_GAP, vf.CRASH_GAP_WAIT_S)
    head = ctx.finalized_dec(dry_value=pre + vf.CRASH_GAP)
    ctx.check(case, *vf.evaluate_chain_advanced_while_crashed(head, pre),
              on_fail=lambda: ctx.dump_logs(vf.CRASH_STALL_LOG_TAIL))
    _say(ctx, f"  chain advanced to {head} with {victim} crashed (gap ~{head - pre} blocks)")

    _say(ctx, f"  restarting crashed {victim} ...")
    ctx.docker_start(cid, note="crash-restart-victim")

    tick = [0]

    def realigned():
        v0 = ctx.check_external(topology.HOST_RPC_PORT, dry_value=f"{hex(head)}|0xh")
        v3 = ctx.check_node(victim, dry_value=f"{hex(head)}|0xh")
        if vf.crash_survivor_realigned(v0, v3, head, ctx.blockhash_at):
            return (v3, v0)
        # The decisive diagnostic: a periodic peer probe, so the log says whether the victim is
        # re-peering slowly or not at all.
        if tick[0] % vf.CRASH_PEER_PROBE_EVERY == 0:
            _say(ctx, f"  t+{tick[0] * vf.CRASH_POLL_S}s: {victim} peers="
                      f"{ctx.peer_count(victim)} {victim}={v3} v0={v0}")
        tick[0] += 1
        return None

    got = ctx.poll(realigned, vf.CRASH_RECOVER_S, poll_s=vf.CRASH_POLL_S,
                   dry_value=("0x0|0x0", "0x0|0x0"))
    ctx.check(case, bool(got),
              lambda: (f"{victim} did not realign after crash+restart "
                       f"(v0={ctx.check_external(topology.HOST_RPC_PORT)}, "
                       f"{victim}={ctx.check_node(victim)}, floor=head-while-down={head})\n"
                       "  (Problem A: crash survivor wedged on a missing EL block — needs 2b "
                       "FCU-driven recovery)"),
              on_fail=lambda: ctx.dump_logs(vf.CRASH_FAIL_LOG_TAIL, victim))
    # BOTH heights, as bash has printed all along. This case's vacuous pass — `realigned at
    # 0x41(=65) … (v0=0x50(=80))` — was legible in bash's own output; the port printed only the
    # victim, and that asymmetry is what let 15 blocks of lag read as a recovery.
    _ok(ctx, case, f"{victim} recovered from crash and realigned at {got[0]} (v0={got[1]}, "
                   f"floor=head-while-down={head})")


# ══ smoke-full-restart ════════════════════════════════════════════════════════════════

def _dump_restart_window(ctx, pre, head_dec, restart_at) -> None:
    """The full-restart failure artifact: the whole timestamp picture, not just the one number
    the verdict compared.

    THE DECIDING ROW IS `head+1`. A block at or below `restart_at` is ambiguous on its own — it is
    what a persisted tail looks like AND what the last pre-stop block looks like on a fleet that
    came back and then went on producing. Whether the chain KEPT CLIMBING past it is what separates
    them, so the block above is read: a real timestamp there means the fleet resumed and this gate
    simply sampled the tail, and `<none>` means nothing was produced after the restart at all.

    Read lazily, on the failure path only — `ctx.check` evaluates `on_fail` nowhere else, and these
    are three extra RPCs that a passing run must not pay for."""
    parent_ts = ctx.timestamp_at(head_dec - 1)
    here_ts = ctx.timestamp_at(head_dec)
    print(f"    FULL-RESTART diagnostic — floor pre={pre}, restart_at={restart_at}", flush=True)
    print(f"      converged height {head_dec}: timestamp={here_ts} "
          f"({here_ts - restart_at:+d}s vs restart)", flush=True)
    print(f"      parent {head_dec - 1}: timestamp={parent_ts}", flush=True)

    # EXISTENCE IS NOT RESUMPTION. This scan used to conclude "the fleet resumed" from a block
    # merely BEING there, while printing a stamp that showed it was pre-restart — a block above
    # the converged height that is itself older than the restart is more TAIL, not evidence of
    # anything. It may only say "produced" for a stamp that clears `restart_at`.
    tail_top = head_dec
    for h in range(head_dec + 1, head_dec + 1 + vf.FULL_RESTART_SCAN_BLOCKS):
        ts = ctx.timestamp_at(h)
        if not ts:
            break
        if ts >= restart_at:
            print(f"      first POST-restart block {h}: timestamp={ts} "
                  f"({ts - restart_at:+d}s vs restart) — the chain DID resume; the gate was "
                  "reading the tail", flush=True)
            ctx.dump_logs(vf.FULL_RESTART_LOG_TAIL)
            return
        tail_top = h
    span = tail_top - head_dec
    print(f"      no post-restart block within {vf.FULL_RESTART_SCAN_BLOCKS} of the converged "
          f"height: the persisted tail runs to at least {tail_top} "
          f"({span} block(s) past it) and every one of those is PRE-restart, so resumption is "
          "unproven — the fleet came back on its tail", flush=True)
    ctx.dump_logs(vf.FULL_RESTART_LOG_TAIL)


def assert_full_restart(ctx) -> None:
    """Stop ALL four validators, verify each persisted, restart them, and require the network to
    reconverge FROM THE PERSISTED FINALIZED HEAD — DPoS cold restart from disk for the whole set,
    not just for the migration anchor.

    RUNS LAST in the chain: it is the most invasive thing in the file, and there is nothing left
    for a later case to measure if it goes wrong.

    THE EXIT-CODE CHECK IS THE FLUSH ASSERTION, and it is the reason `shutdown_flushed` reads
    `docker inspect` container metadata rather than logs. Nothing else in this case can see
    whether persistence actually landed: a validator SIGKILLed at the 40s stop ceiling comes back,
    resyncs from its peers, and reconverges perfectly — the reconverge check below would bless a
    node that lost its tail. Exit code 0 is the only witness, and the log-grep the barrier was
    originally written as raced under daemon load and false-reported "flush incomplete" for a
    container that had exited 0 in under a second.
    """
    case = "smoke-full-restart"
    vals = list(ctx.profile.committee())
    pre = ctx.baseline_height(dry_value=140)
    _say(ctx, f"smoke-full-restart: stopping all {len(vals)} validators at finalized={pre}")
    ctx.compose_stop(*vals, timeout=vf.FULL_RESTART_STOP_TIMEOUT_S,
                     note="full-restart-stop-committee")
    for v in vals:
        ctx.check(case, *vf.evaluate_flushed(v, ctx.shutdown_flushed(v)))
    _say(ctx, "  all persisted (exit 0); restarting")
    restart_at = int(time.time())
    ctx.compose_start(*vals, note="full-restart-start-committee")

    labels = [topology.PINNED_RPC_HOST, *vals[1:], "full-node"]

    def reconverged():
        readings = [ctx.check_external(topology.HOST_RPC_PORT, dry_value=f"{hex(pre + 1)}|0xh")]
        readings += [ctx.check_node(v, dry_value=f"{hex(pre + 1)}|0xh") for v in vals[1:]]
        readings.append(ctx.check_external(topology.HOST_L2_RPC_PORT,
                                           dry_value=f"{hex(pre + 1)}|0xh"))
        return (readings
                if vf.full_restart_reconverged(readings, pre, ctx.blockhash_at) else None)

    got = ctx.poll(reconverged, vf.FULL_RESTART_RECONVERGE_S, poll_s=vf.FULL_RESTART_POLL_S,
                   dry_value=[f"{hex(pre + 1)}|0xh"] * (len(vals) + 1))
    ctx.check(case, bool(got), f"network did not reconverge after full restart (> pre={pre})",
              on_fail=lambda: ctx.dump_logs(vf.FULL_RESTART_LOG_TAIL))

    # AND THE CHAIN RESUMED — a SEPARATE wait, because reconvergence cannot witness it. `pre` is
    # sampled before a 40s stop window (`vf.FULL_RESTART_STOP_TIMEOUT_S`), so the blocks written
    # while the fleet was stopping are on disk and clear the floor by themselves; the poll above
    # therefore returns on the persisted tail, whose stamps all predate the restart. Only a block
    # the chain produced AFTER `restart_at` proves it came back to life.
    head_dec = nodes.hex_to_dec(str((got or [""])[0]).split("|", 1)[0])

    def resumed():
        h = ctx.finalized_dec(dry_value=head_dec + 1)
        if h <= 0:
            return False
        ts = ctx.timestamp_at(h, dry_value=restart_at)
        return (h, ts) if vf.evaluate_produced_after_restart(ts, restart_at)[0] else False

    # The RECONVERGE budget, reused rather than a new number: it is the one already tuned for "this
    # fleet is coming back to life" on this case, and at ~1 blk/s plus K=3 result lag a resumed
    # fleet finalizes a fresh block in seconds — so 120s is headroom, and erring generous only
    # makes a dead fleet red slowly.
    produced = ctx.poll(resumed, vf.FULL_RESTART_RECONVERGE_S, poll_s=vf.FULL_RESTART_POLL_S,
                        dry_value=(head_dec + 1, restart_at))
    ctx.check(case, *vf.evaluate_resumed_production(produced, head_dec, pre, restart_at,
                                                    vf.FULL_RESTART_RECONVERGE_S),
              on_fail=lambda: _dump_restart_window(ctx, pre, head_dec, restart_at))
    _say(ctx, f"  chain RESUMED: block {produced[0]} stamped {produced[1]} "
              f"({produced[1] - restart_at:+d}s vs the restart)")

    # EVERY node's height on the OK line, not just the producer's. There is no victim/hub split
    # here — all five are peers — so the auditable evidence is the whole ragged set against the
    # floor, and the pass that had to be caught by eye (`all 5 reconverged at 0x41 (>= pre=65)`,
    # i.e. exactly AT the floor, nobody having moved) is now unrepresentable.
    detail = ", ".join(f"{lab}={r}" for lab, r in zip(labels, got or []))
    _ok(ctx, case, f"all {len(vals) + 1} reconverged at {(got or [''])[0]} (> pre={pre}) after "
                   f"full stop/start [{detail}]")
