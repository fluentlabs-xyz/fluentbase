"""asserts.py — the four READ-ONLY assertion bodies of `scripts/asserts.sh`.

Each one is the READING half of an assertion: it decides what to read and when, and hands the
readings to `verdicts.py`, which decides what they mean. Neither half is optional — the reading
half is what a live run proves and the verdict half is what the unit suite proves, and only the
two together cover both directions of every check.

READ-ONLY W.R.T. CONSENSUS is the property that lets `smoke-base` run all four on ONE bring-up:
none of them stops, restarts, jails or reconfigures a node. `assert_tx` sends two transactions and
`assert_vrf` deploys a throwaway probe — both are load ON the chain, not changes TO it. Adding a
node-disturbing step to any of these silently invalidates `smoke-base`.

Ordering inside each body is bash's, and in several places the order IS the assertion:
`assert_vrf` reads its active-count BEFORE letting the chain advance and again after, and
`assert_tx` reads the recipient balance BEFORE sending. Reordering those measures nothing.
"""

from __future__ import annotations

import json
import os

from . import beacon, verdicts
from .driver import BEACON_NODES, SmokeFailure
from ...core import converge, nodes, topology

# <smoke>/dpos_harness/cases/smoke/asserts.py → FOUR dirnames to the smoke dir. Pinned by
# tests/test_path_anchors.py, because a wrong directory here is silent: the probe bytecode would
# come back missing and `assert_vrf`'s C1/C2 step would fail as if the chain were broken.
_SMOKE_DIR = os.path.dirname(os.path.dirname(os.path.dirname(
    os.path.dirname(os.path.abspath(__file__)))))
PROBE_JSON = os.path.join(_SMOKE_DIR, "contracts", "PrevRandaoProbe.json")

VALIDATORS = tuple(topology.validator(i) for i in range(4))

#: `asserts.sh:45` — how long both tx blocks get to finalize.
TX_FINALIZE_TIMEOUT_S = 60
#: `asserts.sh:73` — the alignment/target budget, and `:97` its log-dump depth.
EPOCH_WAIT_S = 220
EPOCH_POLL_S = 2
EPOCH_FAIL_LOG_TAIL = 200
#: `asserts.sh:123`, `:373` — the beacon cases' wait to the sampling window, and `:126` its depth.
VRF_WAIT_S = 300
VRF_FAIL_LOG_TAIL = 120
#: `asserts.sh:169`, `:236` — the per-validator log-tail depth on an active-count failure.
VRF_ACTIVE_LOG_TAIL = 80
#: `asserts.sh:224`, `:335` — how long the head gets to advance the growth margin.
GROWTH_WAIT_S = 30


# ══ smoke-tx ══════════════════════════════════════════════════════════════════════════

def assert_tx(ctx) -> None:
    """A value transfer AND a contract call execute, finalize, and have their state APPLIED.

    The contract call is mandatory, not extra coverage. A bare value transfer touches no CALL and
    no SSTORE, so a chain whose EVM never executes anything would pass a transfer-only assertion;
    `MockBlendToken.approve` is what closes the brief's "EVM execution path under DPoS is
    unverified" gap. And "finalized" alone is not the assertion either — a REVERTED approve
    finalizes just as happily, which is why the receipt status and the allowance slot are both
    read back afterwards.
    """
    case = "smoke-tx"
    key = ctx.funded_key()
    sender = ctx.wallet_address(key)
    dead, blend = verdicts.DEAD_ADDR, verdicts.BLEND_ADDR

    # Read BEFORE sending: the assertion is a DELTA, and an "after" with no "before" is a balance.
    bal_before = _balance(ctx, case, dead)

    common = ["--private-key", f"0x{key}", "--rpc-url", ctx.rpc, "--chain", ctx.chain_id]
    tx = ctx.cast_send(common + [dead, "--value", "0.1ether"], note="tx-value-transfer",
                       dry_value='{"transactionHash":"0xvalue"}')
    ctx_tx = ctx.cast_send(common + [blend, "approve(address,uint256)", dead,
                                     str(verdicts.ALLOWANCE)],
                           note="tx-approve", dry_value='{"transactionHash":"0xapprove"}')

    hashes = [tx.get("transactionHash"), ctx_tx.get("transactionHash")]
    ctx.check(case, all(hashes), f"a cast send returned no transactionHash: {hashes}")

    receipts, maxblk = [], 0
    for h in hashes:
        r = ctx.cast_receipt(h, dry_value='{"status":"0x1","blockNumber":"0x80"}')
        receipts.append((h, r.get("status")))
        blk = r.get("blockNumber")
        ctx.check(case, blk is not None, f"receipt {h} carries no blockNumber")
        maxblk = max(maxblk, nodes.hex_to_dec(blk))
    ctx.check(case, *verdicts.evaluate_tx_receipts(receipts))

    ctx.check(case, ctx.wait_finalized_ge(maxblk, TX_FINALIZE_TIMEOUT_S),
              f"tx block {maxblk} not finalized in time")

    bal_after = _balance(ctx, case, dead)
    allowance = verdicts.first_token(
        ctx.cast_call(blend, "allowance(address,address)(uint256)", sender, dead,
                      dry_value=str(verdicts.ALLOWANCE)))
    ctx.check(case, *verdicts.evaluate_tx_state(bal_after - bal_before, allowance))

    _ok(ctx, case, "value transfer + MockBlendToken.approve executed, finalized, and state "
                   "applied under DPoS")


def _balance(ctx, case: str, addr: str) -> int:
    """`cast balance` as an int, FAIL-LOUD on anything else.

    Bash gets this for free: `$(( bal_after - bal_before ))` on a non-numeric reading aborts under
    `set -e`. Python would happily turn an unreachable RPC into a 0 balance and then report a
    delta of exactly the transfer amount in one direction or a bogus failure in the other — an
    unreachable read must never present as a value."""
    raw = ctx.cast_balance(addr)
    tok = verdicts.first_token(raw)
    if not tok.lstrip("-").isdigit():
        raise SmokeFailure(case, f"cast balance {addr} returned {raw!r}, not a number "
                                 "(RPC unreachable?) — refusing to read it as a balance")
    return int(tok)


# ══ smoke-epoch ═══════════════════════════════════════════════════════════════════════

def assert_epoch(ctx) -> None:
    """The chain crosses >= EPOCH_MIN_CROSS epoch boundaries after the swap, on all five readers,
    with a non-empty committee and 1 blk/s pacing.

    This is the permanent regression guard for the epoch-boundary handoff deadlock. The canonical
    smoke's "head > anchor" threshold can be satisfied entirely INSIDE epoch 0 and would hide a
    boundary regression completely, which is why the target is computed from the anchor's epoch
    rather than from the anchor.
    """
    case = "smoke-epoch"
    min_cross = int(os.environ.get("EPOCH_MIN_CROSS", "1"))
    prev_dec = ctx.anchor_dec(case)
    target = verdicts.epoch_target(prev_dec, ctx.interval, min_cross)
    print(f"smoke-epoch: anchor={prev_dec}; require finalized >= {target} "
          f"(cross {min_cross} boundary(ies), interval={ctx.interval})", flush=True)

    canned = [(f"node-{i}", f"{hex(target)}|0xdry") for i in range(5)]

    def aligned_at_target():
        """All five readers past genesis, on the producer's chain at their own heights, and at
        or beyond the target.

        `converge.aligned_reading` is the same verdict `_wait_aligned` applies during bring-up —
        every head neither `"null"` nor `"0x0"`, and every reader's hash equal to the producer's
        block at that reader's height (byte-identical readings short-circuit that read). Readers
        at DIFFERENT heights are fine: they advance independently and are read one after another,
        so demanding one identical tip from five of them is a race, not a check. The null guard
        is what stops an all-nodes-down poll from reading as consensus: five unreachable nodes
        agree perfectly."""
        agreed = converge.aligned_reading(ctx.read_sequencer_nodes(dry_value=canned), None)
        if agreed is None:
            return None
        hd = nodes.hex_to_dec(agreed.split("|", 1)[0])
        return hd if hd >= target else None

    hd = ctx.poll(aligned_at_target, EPOCH_WAIT_S, poll_s=EPOCH_POLL_S, dry_value=target)
    ctx.check(case, bool(hd), f"did not reach finalized >= {target} within {EPOCH_WAIT_S}s",
              on_fail=lambda: ctx.dump_logs(EPOCH_FAIL_LOG_TAIL))

    cur = verdicts.first_token(ctx.staking_call("currentEpoch()(uint256)", dry_value="2"))
    comm = ctx.staking_call("getEpochCommittee(uint64)(address[])", cur, dry_value="[]")
    ctx.check(case, *verdicts.evaluate_committee(comm, cur))

    # 1 blk/s pacing. Measured over a WALL window, so it is a rate and not a height: r0 and r1
    # bracket exactly PACING_WINDOW_S seconds and the bounds are calibrated to that width.
    r0 = ctx.finalized_dec()
    ctx.sleep(verdicts.PACING_WINDOW_S)
    r1 = ctx.finalized_dec()
    delta = r1 - r0
    ctx.check(case, *verdicts.evaluate_pacing(delta))

    _ok(ctx, case, f"all 5 aligned finalized={hd} >= {target} (epoch {cur}), committee "
                   f"non-empty, pacing {delta} blk/{verdicts.PACING_WINDOW_S}s")


# ══ smoke-vrf ═════════════════════════════════════════════════════════════════════════

def assert_vrf(ctx) -> None:
    """The threshold randomness beacon drives prev_randao under DPoS, end to end.

    Six steps, and every one of them catches something no other step can see:

      1. a WINDOW of finalized blocks, non-zero, byte-identical across all five nodes at EVERY
         height, and all distinct across heights — determinism plus real variance;
      2. every validator logged the threshold path (`assurance=true`, never the digest fallback)
         at least MIN_ACTIVE_BLOCKS times;
      2b. and that count still GROWS while the chain advances — a beacon that fell back AFTER
         warm-up keeps a frozen count that step 2 alone would pass forever;
      3. the recent finalized headers' mixHashes appear among the values validator-0 logged —
         step 1 read the header, step 2 read the log, this ties them together;
      5. C1/C2 — the EVM-visible `block.prevrandao` equals the header mixHash, i.e. the value
         reached EXECUTION and not only the header (and the probe tx makes a tx-BEARING block,
         so the beacon is exercised under load rather than on empty blocks — H2);
      6. D1 — `beacon_digest_fallback` flat while `beacon_seed_active` grows.

    E1 (cert-follower mixHash == validators) needs no step of its own: the import follower is in
    `BEACON_NODES`, so step 1 already asserts it at every height.
    """
    case = "smoke-vrf"
    epoch2_start = verdicts.beacon_active_epoch_start(ctx.activation_block, ctx.interval)
    epoch2_probe = epoch2_start + verdicts.VRF_WINDOW
    print(f"smoke-vrf: beacon active from epoch 2 (start={epoch2_start}); waiting for "
          f"finalized >= {epoch2_probe}", flush=True)
    ctx.check(case, ctx.wait_finalized_ge(epoch2_probe, VRF_WAIT_S),
              f"chain did not reach finalized {epoch2_probe} (epoch-2 window)",
              on_fail=lambda: ctx.dump_logs(VRF_FAIL_LOG_TAIL, topology.validator(0)))

    fin = ctx.finalized_dec(dry_value=epoch2_probe)
    ctx.check(case, fin > 0, "no finalized block")

    # ── 1) the cross-node window ──────────────────────────────────────────────────
    lo = verdicts.window_lo(fin, verdicts.VRF_WINDOW, epoch2_start)
    print(f"smoke-vrf: DPoS up, finalized={fin}; checking prev_randao over blocks "
          f"[{lo}..{fin}] on {len(BEACON_NODES)} nodes", flush=True)
    ctx.check(case, beacon.wait_nodes_have(ctx, fin),
              f"nodes did not all reach block {fin} within {beacon.NODES_HAVE_TIMEOUT_S}s")
    mixes = beacon.assert_beacon_window(ctx, case, lo, fin, "vrf-window")
    if not ctx.dry:
        print(f"smoke-vrf: {len(mixes)} blocks, {len(mixes)} distinct non-zero prev_randao, all "
              f"{len(BEACON_NODES)} nodes byte-identical at every height", flush=True)

    # ── 2) sustained threshold-verified derives ───────────────────────────────────
    # Read / check / report PER VALIDATOR, as bash does, so a failure on validator-2 still shows
    # what validator-0 and validator-1 counted. Batching the reads and checking afterwards would
    # print nothing before the first failure, which is where the comparison lives.
    before = {}
    for v in VALIDATORS:
        c = int(ctx.log_count(v, verdicts.ACTIVE_LINE,
                              dry_value=verdicts.MIN_ACTIVE_BLOCKS) or 0)
        ok, msg, _ = verdicts.evaluate_active_counts({v: c})
        ctx.check(case, ok, msg, on_fail=lambda svc=v: ctx.dump_logs(VRF_ACTIVE_LOG_TAIL, svc))
        before[v] = c
        print(f"smoke-vrf: {v} — threshold prev_randao active x{c}", flush=True)

    # ── 2b) …and still growing NOW ────────────────────────────────────────────────
    _await_head_growth(ctx, case, required=True)
    for v in VALIDATORS:
        after = int(ctx.log_count(v, verdicts.ACTIVE_LINE,
                                  dry_value=verdicts.MIN_ACTIVE_BLOCKS + 1) or 0)
        ok, msg, _ = verdicts.evaluate_active_growth({v: before[v]}, {v: after})
        ctx.check(case, ok, msg, on_fail=lambda svc=v: ctx.dump_logs(VRF_ACTIVE_LOG_TAIL, svc))
        print(f"smoke-vrf: {v} — active-count grew {before[v]} → {after} (beacon live now)",
              flush=True)

    # ── 3) the logged value IS the on-chain header value ──────────────────────────
    raw = ctx.logs_all(topology.validator(0))
    logged = verdicts.parse_logged_randaos(raw)
    ok, msg = verdicts.evaluate_logged_randaos_present(logged)
    ctx.check(case, ok, msg, on_fail=lambda: _dump_active_samples(raw))
    fin3 = ctx.finalized_dec(dry_value=epoch2_probe)
    chk_lo = verdicts.logged_check_lo(fin3)
    onchain = [(n, ctx.mixhash_at(n)) for n in range(chk_lo, fin3 + 1)]
    ctx.check(case, *verdicts.evaluate_logged_vs_onchain(logged, onchain))
    if not ctx.dry:
        print(f"smoke-vrf: all {fin3 - chk_lo + 1} recent finalized mixHashes [{chk_lo}..{fin3}] "
              "were logged as threshold-active beacon values (header == deriver H(seed))",
              flush=True)

    # ── 5) C1/C2 (+H2): the value reached EVM EXECUTION ───────────────────────────
    _assert_evm_prevrandao(ctx, case)

    # ── 6) D1: the two beacon counters ────────────────────────────────────────────
    _assert_beacon_metrics(ctx, case)

    _ok(ctx, case,
        f"threshold-beacon prev_randao active+sustained+still-growing on all "
        f"{len(VALIDATORS)} validators (>={verdicts.MIN_ACTIVE_BLOCKS} blocks) from epoch 2, "
        f"non-zero and {len(mixes)}/{len(mixes)} distinct and byte-identical across all "
        f"{len(BEACON_NODES)} nodes at every height in [{lo}..{fin}], recent logged H(seed) "
        "values match on-chain mixHashes")


def _await_head_growth(ctx, case: str, required: bool) -> None:
    """Let the chain advance GROWTH_BLOCKS blocks (~1 blk/s), tracking the HEAD.

    The head, not finalized, and that is a diagnosed bug rather than a preference: the active line
    fires at the SPECULATIVE TIP (notarize-time derive under deferred execution). Right after the
    sequencer→DPoS migration the tip bursts ahead and finalized can catch up to it WITHOUT the tip
    producing new blocks — the old finalized-based wait returned with a frozen count and reported
    a false "beacon stopped".

    `required` is bash's difference between step 2b (which FAILS on a head that will not move —
    it cannot observe a sustained beacon without new blocks) and step 6 (which merely `break`s and
    goes on to compare the counters over whatever the window produced)."""
    head0 = ctx.head_dec()
    want = head0 + verdicts.GROWTH_BLOCKS

    def head_advanced():
        return ctx.head_dec() >= want

    grew = ctx.poll(head_advanced, GROWTH_WAIT_S)
    if required:
        ctx.check(case, bool(grew),
                  f"head did not advance >= {verdicts.GROWTH_BLOCKS} blocks past {head0} within "
                  f"{GROWTH_WAIT_S}s — cannot observe a sustained beacon")


def _dump_active_samples(raw: str) -> None:
    """The diagnostic bash prints when NO value parses out of the active lines: a couple of raw
    lines, so the reader can see whether the line is absent or merely unparseable."""
    print("  --- sample raw active lines (diagnostic) ---", flush=True)
    hits = [ln for ln in (raw or "").splitlines() if verdicts.ACTIVE_LINE in ln]
    for ln in hits[:2]:
        print(f"  {ln}", flush=True)
    if not hits:
        print("  (no line matched the active marker at all)", flush=True)


def _assert_evm_prevrandao(ctx, case: str) -> None:
    """C1/C2 — deploy `PrevRandaoProbe`, snapshot `block.prevrandao`, compare to the header.

    The probe's `Snapshot(uint256 indexed blockNumber, uint256 prevrandao)` has `prevrandao` as
    its only NON-indexed argument, so the value is the log's `data` field. Bash shells out to
    `python3 -c` three times to read the JSON artifact and pick fields apart; here the same work
    is in-process, which changes no argv that reaches the chain."""
    key = ctx.funded_key()
    bytecode = _probe_bytecode(ctx, case)
    deploy = ctx.cast_send(["--private-key", f"0x{key}", "--rpc-url", ctx.rpc,
                            "--create", bytecode],
                           note="vrf-probe-deploy",
                           dry_value='{"contractAddress":"0xprobe"}')
    probe = deploy.get("contractAddress")
    ctx.check(case, bool(probe), f"PrevRandaoProbe deploy returned no contractAddress: {deploy}")
    print(f"smoke-vrf: PrevRandaoProbe deployed at {probe}", flush=True)

    rcpt = ctx.cast_send(["--private-key", f"0x{key}", "--rpc-url", ctx.rpc, probe, "snapshot()"],
                         note="vrf-probe-snapshot",
                         dry_value='{"blockNumber":"0x80","logs":[{"data":"0x' + "11" * 32 + '"}]}')
    logs = rcpt.get("logs") or []
    ctx.check(case, bool(logs), f"snapshot() receipt carries no logs: {rcpt}")
    snap_block = nodes.hex_to_dec(rcpt.get("blockNumber"))
    evm_pr = str((logs[0] if logs else {}).get("data", "")).lower()
    hdr_pr = ctx.mixhash_at(snap_block, dry_value=evm_pr)
    ctx.check(case, *verdicts.evaluate_evm_prevrandao(evm_pr, hdr_pr, snap_block))
    print(f"smoke-vrf: C1/C2 — EVM block.prevrandao == header mixHash at tx-bearing block "
          f"{snap_block} (beacon H(seed) reached EVM execution)", flush=True)


def _probe_bytecode(ctx, case: str) -> str:
    try:
        with open(PROBE_JSON) as fh:
            return json.load(fh)["bytecode"]["object"]
    except (OSError, ValueError, KeyError, TypeError) as e:
        raise SmokeFailure(case, f"could not read the PrevRandaoProbe bytecode from "
                                 f"{PROBE_JSON}: {e}")


def _assert_beacon_metrics(ctx, case: str) -> None:
    """D1 — on a beacon-active chain `beacon_digest_fallback` must not grow while
    `beacon_seed_active` must.

    Substring matching tolerates the OpenMetrics `_total` suffix convention. On an absent series
    the failure prints the beacon/dkg families that ARE exported, so a metric RENAME surfaces as a
    rename rather than as "the beacon is dead"."""
    text = ctx.consensus_metrics(dry_value="beacon_digest_fallback_total 0\n"
                                           "beacon_seed_active_total 7\n")
    fb0 = nodes.metric_first_val(text, "beacon_digest_fallback")
    sa0 = nodes.metric_first_val(text, "beacon_seed_active")
    ctx.check(case, *verdicts.evaluate_beacon_metrics_present(fb0, sa0),
              on_fail=lambda: _dump_beacon_families(text))

    _await_head_growth(ctx, case, required=False)

    text1 = ctx.consensus_metrics(dry_value="beacon_digest_fallback_total 0\n"
                                            "beacon_seed_active_total 9\n")
    fb1 = nodes.metric_first_val(text1, "beacon_digest_fallback")
    sa1 = nodes.metric_first_val(text1, "beacon_seed_active")
    ctx.check(case, *verdicts.evaluate_beacon_metrics_delta(fb0, sa0, fb1, sa1))
    print(f"smoke-vrf: D1 — beacon_digest_fallback flat ({fb0}) while beacon_seed_active grew "
          f"({sa0} → {sa1}) over {verdicts.GROWTH_BLOCKS} blocks", flush=True)


def _dump_beacon_families(text: str) -> None:
    print("  beacon_/dkg_ series present:", flush=True)
    shown = 0
    for line in (text or "").splitlines():
        if line.startswith("#"):
            continue
        low = line.lower()
        if "beacon" in low or "dkg" in low:
            print(f"  {line}", flush=True)
            shown += 1
            if shown >= 20:
                return
    if not shown:
        print("  (none — the registry exported no beacon/dkg family at all)", flush=True)


# ══ smoke-vrf-boundary ════════════════════════════════════════════════════════════════

def assert_vrf_boundary(ctx) -> None:
    """F1 — the threshold beacon survives an EPOCH BOUNDARY on a STABLE committee.

    The beacon activates at epoch 2 (deterministic bootstrap), so the first stable CARRY-FORWARD
    boundary is 2→3; 0→1 and 1→2 are keyless / the bootstrap commit and would prove something
    else. The carry-forward itself has no on-chain mirror to read, so "the beacon stayed live,
    node-agreed and varying ACROSS the boundary" is the whole of the proof — which is why the
    window straddles the boundary block rather than starting after it.
    """
    case = "smoke-vrf-boundary"
    boundary = verdicts.boundary_block(ctx.activation_block, ctx.interval)
    target = boundary + verdicts.VRF_WINDOW
    print(f"smoke-vrf-boundary: waiting for finalized >= {target} (epoch-2→3 boundary at block "
          f"{boundary})", flush=True)
    ctx.check(case, ctx.wait_finalized_ge(target, VRF_WAIT_S),
              f"chain did not reach finalized {target}",
              on_fail=lambda: ctx.dump_logs(VRF_FAIL_LOG_TAIL, topology.validator(0)))

    lo = boundary - verdicts.BOUNDARY_HALF_WINDOW
    hi = boundary + verdicts.BOUNDARY_HALF_WINDOW
    ctx.check(case, beacon.wait_nodes_have(ctx, hi), f"not all nodes reached block {hi}")
    beacon.assert_beacon_window(ctx, case, lo, hi, f"epoch-boundary-{boundary}")
    print(f"smoke-vrf-boundary: F1 — beacon active + byte-identical across the epoch-2→3 "
          f"boundary (block {boundary})", flush=True)

    _ok(ctx, case, f"threshold beacon active + node-agreed + varying across the epoch-2→3 "
                   f"boundary (block {boundary}); the per-epoch carry-forward kept the beacon "
                   "live with no break in node-agreed prev_randao")


def _ok(ctx, case: str, message: str) -> None:
    """The `OK (<case>): …` line. Suppressed under `--dry-run`, where nothing was measured and a
    green line would be a lie in a transcript people will read as evidence."""
    if not ctx.dry:
        print(f"OK ({case}): {message}", flush=True)
