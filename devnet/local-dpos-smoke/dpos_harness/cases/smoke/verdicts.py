"""verdicts.py — the PURE decision layer of the four smoke assertions.

Every function here answers "given these readings, does the assertion hold?" and touches no
socket, no clock and no subprocess. That split is the whole point: `asserts.py` decides WHAT to
read and WHEN, this module decides what the readings MEAN, and `tests/test_smoke_verdicts.py`
drives the second half through both its outcomes.

WHY THAT MATTERS HERE SPECIFICALLY. An assertion that can only pass is the failure mode this
project keeps re-finding, and the bash originals are full of checks whose negative branch has
never executed on this machine — `assert_vrf_boundary` had literally never run until the `set -u`
fix in ee0cf6a9. A live run exercises the PASS direction only, by construction (it runs against a
healthy chain). So every FAIL direction below is a unit test or it is untested.

Return convention: `(ok: bool, message: str)`. The message is empty on success and is the exact
diagnostic the bash printed on failure — the caller prefixes it with `FAIL (<case>): `.
"""

from __future__ import annotations

import re

from ...core import nodes

# ══ smoke-tx ══════════════════════════════════════════════════════════════════════════

#: `asserts.sh:29` — 0.1 ETH out of the funded account's 1 ETH, leaving headroom for gas.
TRANSFER_WEI = 100_000_000_000_000_000
#: `asserts.sh:26` — the allowance the MANDATORY contract call writes. Its only job is to be a
#: value the EVM must SSTORE, so that "the tx finalized" and "the tx CHANGED STATE" stay two
#: different questions.
ALLOWANCE = 12345

#: `asserts.sh:24-25` — the burn address and the MockBlendToken predeploy (genesis-baked on the
#: static stack; NOT the sim's runtime-deployed cluster).
DEAD_ADDR = "0x000000000000000000000000000000000000dEaD"
BLEND_ADDR = "0x0000000000000000000000000000000000005207"


def receipt_status_ok(status) -> bool:
    """`[[ "$st" == "0x1" || "$st" == "1" ]]` — foundry has printed both spellings."""
    return str(status) in ("0x1", "1")


def first_token(out: str) -> str:
    """`cast`'s pretty-printed uint, reduced to the bare integer: `12345 [1.234e4]` -> `12345`.

    §2.4 item 9. `asserts.sh:59` does this with `awk '{print $1}'` and dropping it re-creates a
    silent parse corruption — the same class of bug that once read an activation block of 229200
    as `229200 [2.292e5]`, pinned the epoch to 0 and disabled all churn with no error anywhere."""
    parts = (out or "").split()
    return parts[0] if parts else ""


def evaluate_tx_receipts(receipts):
    """`receipts` = [(txhash, status)] in submission order. Both must have executed.

    A reverted `approve` finalizes exactly as happily as a successful one, so the status byte is
    the only thing standing between this case and a green run over a broken EVM."""
    for h, st in receipts:
        if not receipt_status_ok(st):
            return False, f"receipt {h} status={st}"
    return True, ""


def evaluate_tx_state(delta, allowance, want_delta=TRANSFER_WEI, want_allow=ALLOWANCE):
    """The state actually changed: the recipient's balance moved by exactly the transfer, and the
    allowance slot holds the written value.

    Both halves are load-bearing and neither implies the other. The balance delta proves the value
    transfer applied; the allowance proves the EVM executed a CALL and an SSTORE — a bare transfer
    exercises no EVM at all, which is the gap the contract call was added to close."""
    if int(delta) != int(want_delta):
        return False, f"balance delta {delta} != 0.1 ETH"
    if str(allowance) != str(want_allow):
        return False, (f"allowance={allowance} != {want_allow} (EVM SSTORE not applied)")
    return True, ""


# ══ smoke-epoch ═══════════════════════════════════════════════════════════════════════

#: `asserts.sh:83-86` — 60 s of chain time must finalize ~60 blocks at the 1 blk/s pacing. The
#: lower bound tolerates view timeouts and jitter; the upper bound is what catches a pacing
#: REGRESSION (the unpaced chain did ~350 blocks/min, so this is a wide net around a real signal).
PACING_MIN_BLOCKS = 45
PACING_MAX_BLOCKS = 66
PACING_WINDOW_S = 60


def epoch_target(prev_dec, interval, min_cross):
    """`asserts.sh:71` — `((PREV/INTERVAL) + MIN_CROSS + 1) * INTERVAL`.

    The `+1` is not slack: `PREV/INTERVAL` is the epoch the anchor sits IN, so reaching the start
    of that same epoch would cross nothing. This is the first block of the epoch `min_cross`
    boundaries later."""
    interval = int(interval)
    if interval <= 0:
        raise ValueError(f"epoch interval must be positive, got {interval!r}")
    return ((int(prev_dec) // interval) + int(min_cross) + 1) * interval


def evaluate_committee(committee_out, epoch):
    """`asserts.sh:79` — `getEpochCommittee(cur)` must be non-empty.

    An epoch boundary the chain crossed with an EMPTY committee is a boundary that handed off to
    nobody; the height check alone cannot see it."""
    out = (committee_out or "").strip()
    if not out or out == "[]":
        return False, f"getEpochCommittee({epoch}) empty"
    return True, ""


def evaluate_pacing(delta, lo=PACING_MIN_BLOCKS, hi=PACING_MAX_BLOCKS):
    """`asserts.sh:87-88` — blocks finalized over a 60 s wall window, bounded on BOTH sides."""
    d = int(delta)
    if lo <= d <= hi:
        return True, ""
    return False, f"block rate off target: {d} blocks in 60s (want {lo}..{hi})"


# ══ the beacon window (smoke-vrf step 1, smoke-vrf-boundary F1) ═══════════════════════

def window_lo(fin, window, floor):
    """`asserts.sh:130-131` — the low edge of a `window`-block sample ending at `fin`, raised to
    `floor` so no sampled height predates the beacon's activation epoch.

    The floor is the half of this that matters. Blocks below `epoch_start(2)` carry the DIGEST
    fallback mixHash, which is neither zero nor node-divergent — it would sail through every check
    in `evaluate_beacon_window` and quietly turn a beacon assertion into a no-op."""
    fin, window, floor = int(fin), int(window), int(floor)
    lo = fin - window + 1 if fin > window else 1
    return max(lo, floor)


def evaluate_beacon_window(node_names, rows, label="", require_distinct=True):
    """The per-height cross-node compare (`lib.sh:310-343` `assert_beacon_window`, which
    `asserts.sh:132-153` inlines a copy of).

    `rows` = [(height, [mixhash per node, in `node_names` order])].

    Four distinct properties, and each one catches something the others do not:

      * READABLE — a `"null"`/empty reading is a node that does not have the block, i.e. the check
        would have silently skipped it. This must fail, not pass.
      * NON-ZERO — `0x00…0` is the `order.digest()` fallback / a stalled beacon.
      * NODE-AGREED at every height — one node deriving a divergent threshold seed is the safety
        failure this case exists for, and a validator-0-only probe cannot see it.
      * DISTINCT across heights — a STUCK beacon converges perfectly and is non-zero, so the
        cross-node agreement above would pass it. Only the variance check catches it.

    `require_distinct=False` drops the LAST of the four, and exactly one caller asks for that:
    `case-byzantine-vrf.sh:112` defines its own `assert_honest_beacon_window` which OMITS the
    variance check. That is preserved rather than unified — the honest set there is compared while
    one node churns forged boundary views, and adding a check the bash does not run would fail a
    case the bash passes. It is a parameter rather than a second function so the other three
    properties cannot drift between the two windows.

    Returns `(ok, message, mixes)`; `mixes` is the per-height agreed value, empty on failure.
    """
    prefix = f"{label} — " if label else ""
    mixes = []
    for height, vals in rows:
        for svc, mh in zip(node_names, vals):
            if mh == "null" or not mh:
                return (False,
                        f"{prefix}{svc} has no mixHash for block {height} "
                        "(node behind / RPC down)", [])
            if nodes.is_zero_hash(mh):
                return (False,
                        f"{prefix}prev_randao is zero at block {height} on {svc}", [])
        if len(set(vals)) != 1:
            detail = "\n".join(f"  {svc} {mh}" for svc, mh in zip(node_names, vals))
            return (False,
                    f"{prefix}nodes disagree on prev_randao at block {height} — "
                    f"divergent threshold seed:\n{detail}", [])
        mixes.append(vals[0])
    distinct = len(set(mixes))
    if require_distinct and distinct != len(mixes):
        listing = "\n".join(f"  {m}" for m in mixes)
        lo = rows[0][0] if rows else "?"
        hi = rows[-1][0] if rows else "?"
        return (False,
                f"{prefix}prev_randao not varying — {len(mixes)} blocks [{lo}..{hi}] but only "
                f"{distinct} distinct (stuck randomness)\n{listing}", [])
    return True, "", mixes


# ══ smoke-vrf ═════════════════════════════════════════════════════════════════════════

#: `asserts.sh:118` — the window of finalized blocks the cross-node compare samples.
VRF_WINDOW = 8
#: `asserts.sh:161` — the minimum number of threshold-verified derives per validator.
MIN_ACTIVE_BLOCKS = 5
#: `asserts.sh:162` — logged ONLY on `assurance=true`, i.e. a seed verified against the
#: bootstrapped PK_epoch. It never fires on the digest fallback (`derive.rs::resolve_prev_randao`),
#: which is what makes counting it a beacon assertion rather than a log-volume assertion.
ACTIVE_LINE = "beacon: threshold prev_randao active"
#: `asserts.sh:222` — the head must advance this far before the growth re-read.
GROWTH_BLOCKS = 3

_RANDAO_RE = re.compile(r"0x[0-9a-fA-F]{64}")


def evaluate_active_counts(counts, min_blocks=MIN_ACTIVE_BLOCKS):
    """`asserts.sh:167-172` — every validator logged the threshold path at least `min_blocks` times.

    `counts` maps service -> count, in the order to report. A count BELOW the floor means the
    beacon is inactive, intermittent, or fell through to the digest fallback."""
    for svc, c in counts.items():
        if int(c) < int(min_blocks):
            return (False,
                    f"{svc} logged threshold prev_randao only {c} times (< {min_blocks}) — "
                    "beacon inactive/intermittent/fell back to digest", svc)
    return True, "", ""


def evaluate_active_growth(before, after):
    """`asserts.sh:232-239` — the count must GROW while the chain advances.

    This is the check the static `>= MIN_BLOCKS` floor cannot make. A beacon that logged its five
    lines during warm-up and then silently dropped to the digest fallback keeps a FROZEN count
    under live blocks: it passes the floor for the rest of the run and reports a beacon that
    stopped working minutes ago as healthy."""
    for svc, b in before.items():
        a = int(after.get(svc, 0))
        if a <= int(b):
            return (False,
                    f"{svc} active-count frozen at {a} while the chain advanced — "
                    "beacon stopped (fell back to digest)", svc)
    return True, "", ""


def parse_logged_randaos(log_text, active_line=ACTIVE_LINE):
    """The set of prev_randao values validator-0 logged on the assurance path, lowercased.

    `asserts.sh:257-258` takes the only 32-byte hex on each active line — the `round` field is not
    64 hex, so this is format-agnostic across the text `prev_randao=0x…` and JSON
    `"prev_randao":"0x…"` renderings. The caller passes ANSI-STRIPPED text (§2.4 item 2): the node
    writes escapes inside its `key=value` pairs, so an unstripped read can return an empty set off
    a log full of matches — a silent reader, not a failing one."""
    out = set()
    for line in (log_text or "").splitlines():
        if active_line not in line:
            continue
        for m in _RANDAO_RE.findall(line):
            out.add(m.lower())
    return sorted(out)


def evaluate_logged_randaos_present(logged):
    """`asserts.sh:259-266` — parsing NOTHING is a failure, not an empty pass.

    Without this the cross-check below iterates an empty `logged` and reports every block as
    missing, or (if the loop were written the other way round) reports success over no data."""
    if not logged:
        return False, (f"no prev_randao value parsed from validator-0 '{ACTIVE_LINE}' logs")
    return True, ""


def logged_check_lo(fin):
    """`asserts.sh:271` — anchor the log/header cross-check on the most recent finalized blocks.

    Deliberately NOT step 1's window: that one starts at `epoch_start(2)` and can include the
    pre-DPoS sequencer-era prefix, whose mixHash is the digest fallback and was never logged as a
    beacon value. Comparing those would fail for a correct chain."""
    fin = int(fin)
    return fin - 3 if fin > 4 else 1


def evaluate_logged_vs_onchain(logged, onchain):
    """`asserts.sh:272-281` — every recent FINALIZED block's mixHash appears among the values
    validator-0 logged on the assurance path. `onchain` = [(height, mixhash)].

    The direction is deliberate and the reverse is wrong. Anchoring on finalized blocks (never
    rolled back) and asking "was this logged?" ties the header the chain committed to the value
    the deriver computed. Asking "did every logged value land on-chain?" would false-fail: the
    active line fires on SPECULATIVE notarization derives whose bleeding-edge / nullified rounds
    legitimately never canonicalize."""
    have = set(logged)
    missing = [f"{h}={mh}" for h, mh in onchain if mh not in have]
    if missing:
        listing = "\n".join(f"  {m}" for m in missing)
        return False, ("finalized block mixHash(es) never logged by validator-0 as a threshold "
                       "beacon value — header value is not the deriver's H(seed):\n" + listing)
    return True, ""


def evaluate_evm_prevrandao(evm_pr, hdr_pr, block):
    """C1/C2 (`asserts.sh:302-306`) — the EVM-visible `block.prevrandao` equals the header mixHash.

    Everything above this proves the beacon value reached the HEADER. This is the only check that
    it reached EXECUTION, which is what a contract reading `block.prevrandao` actually gets."""
    if (evm_pr or "").lower() != (hdr_pr or "").lower():
        return False, (f"EVM block.prevrandao ({evm_pr}) != header mixHash ({hdr_pr}) at probe "
                       f"block {block} — the beacon value did not reach EVM execution")
    return True, ""


def _metric_float(v, default=0.0):
    try:
        return float(str(v).strip())
    except (TypeError, ValueError):
        return default


def evaluate_beacon_metrics_present(fallback, active):
    """D1 (`asserts.sh:319-323`) — both series must EXIST before a delta over them means anything.

    An absent metric reads as "" and a `"" -> "" ` comparison is not growth; without this gate the
    D1 check below would silently pass on a node exporting no beacon metrics at all."""
    if fallback == "" or fallback is None or active == "" or active is None:
        return False, (f"D1 — beacon metrics absent on :19100 (digest_fallback='{fallback}' "
                       f"seed_active='{active}')")
    return True, ""


def evaluate_beacon_metrics_delta(fb0, sa0, fb1, sa1):
    """D1 (`asserts.sh:327-334`) — over a few blocks on a beacon-active chain:
    `beacon_digest_fallback` must NOT grow (any growth means a block fell to `order.digest()`)
    and `beacon_seed_active` MUST grow (the metric is wired AND the beacon is live).

    The empty-to-0 coercion mirrors bash's `${fb1:-0}` and applies only to the SECOND reading —
    the first pair went through `evaluate_beacon_metrics_present` and is known to exist."""
    f0, s0 = _metric_float(fb0), _metric_float(sa0)
    f1, s1 = _metric_float(fb1), _metric_float(sa1)
    if f1 > f0:
        return False, (f"D1 — beacon_digest_fallback grew {fb0} → {fb1} on a beacon-active chain "
                       "(a block fell to order.digest())")
    if not s1 > s0:
        return False, (f"D1 — beacon_seed_active did not grow ({sa0} → {sa1}) — beacon stalled / "
                       "metric not incrementing")
    return True, ""


# ══ smoke-vrf-boundary ════════════════════════════════════════════════════════════════

#: `asserts.sh:377-378` — half-width of the window straddling the boundary block.
BOUNDARY_HALF_WINDOW = 6


def epoch_start(activation_block, interval, epoch):
    """First block of relative `epoch` (`asserts.sh:120`, `:365`)."""
    return int(activation_block) + int(epoch) * int(interval)


def beacon_active_epoch_start(activation_block, interval):
    """Start of epoch 2 — the first BEACON-ACTIVE epoch, and therefore the floor under every
    beacon assertion.

    Not a tunable and not margin: epoch 1 is seedless (`order.digest()`), committee[2] runs its
    DKG DURING epoch 1, and the first group key PK_2 commits at the epoch-2 boundary. Sampling
    below this point measures the fallback and reports it as a live beacon."""
    return epoch_start(activation_block, interval, 2)


def boundary_block(activation_block, interval):
    """The epoch-2→3 boundary (`asserts.sh:369`) — the first STABLE carry-forward boundary.

    Not 0→1 and not 1→2: both are keyless, and 1→2 is the bootstrap COMMIT rather than a
    carry-forward. Epoch 3 begins at `activation + 3*interval`, on a committee that has not
    changed, so anything that breaks here broke in the per-epoch engine rebuild and not in a DKG."""
    return epoch_start(activation_block, interval, 3)
