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


def evaluate_beacon_window(node_names, rows, label=""):
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

    ALL FOUR ALWAYS RUN. There used to be a `require_distinct=False` escape hatch, and exactly one
    caller asked for it — `smoke-byzantine-vrf`'s honest-set window, which is retired with the
    case. A knob that can silently drop the only check able to catch a STUCK beacon has no
    business outliving the one window that needed it.

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
    if distinct != len(mixes):
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


# ══ the EPOCH-KEY AGREEMENT PLANE ══════════════════════════════════════════════════════
#
# WHY THIS EXISTS AND WHY IT LIVES ON THE BOUNDARY CASE.
#
# The epoch key is no longer carried by a block. It is agreed peer-to-peer DURING epoch E, on a
# second consensus instance, and published as a quorum-signed artifact; the ceremony then adopts
# the agreed dealer-log set and computes `PK_epoch` + this node's share. NOTHING in the harness
# observed that plane — every existing beacon assertion reads the OUTPUT (`prev_randao` on the EL,
# `beacon_seed_active_total`), which is exactly as green when the key was agreed in one view as
# when it was agreed after four leader timeouts, or when it was carried forward from a stale
# epoch. A run can be green end to end and say nothing about the mechanism that produced it.
#
# `smoke-vrf-boundary` is the right home: it is the one cheap, read-only case whose whole subject
# is crossing an epoch boundary, so the target epoch's agreement MUST have converged during the
# previous epoch for the boundary it asserts to be crossable at all. It also rides `smoke-base`,
# so the observation is in the gate rather than in an opt-in extra.
#
# THE FOUR LINES ARE ONE ORDERED STORY and each one fails differently:
#
#   started → agreed(view, pinned) → adopted → PK_epoch + share stored
#
#   no `started`  the node opened no instance for the target — it is not in `committee[target]`
#                 as it reads the chain, or the plane never spawned;
#   no `agreed`   an instance ran and never decided — below quorum, a lost body, a silent leader;
#   no `adopted`  the artifact never reached the beacon actor — the agreement and the ceremony
#                 are wired to different targets, or the write-back is broken;
#   no `share`    the ceremony had the agreed set and still could not finish.
#
# Collapsing them into one "did the beacon work" check is what the existing `prev_randao` window
# already does. Keeping them apart is the entire value.

#: The four happy-path lines, in the order the plane emits them. Every one carries a bare
#: `epoch=<u64>` field, so `epoch_lines` (the anchored two-grep) can filter them.
AGREE_STARTED_LINE = "dkg agree: epoch-key agreement instance started"
AGREE_DECIDED_LINE = "dkg agree: pinned dealer-log set agreed, instance torn down"
AGREE_ADOPTED_LINE = "live DKG: adopting the agreed dealer-log set as this epoch's pinned set"
AGREE_SHARE_LINE = "live DKG: PK_epoch + share computed + stored"

#: The three SILENT-NOTHING discriminators. They are NOT verdicts — the plane retries, so any of
#: them can appear on a run that then converges (`dkg_agree_bar_unmet_total`'s own registration
#: text says "Never fatal — the plane keeps trying"). Asserting their absence would be a flaky
#: red. They are the failure DIAGNOSTIC: when a stage above is missing, these say which of the
#: three silences it was.
AGREE_NO_SET_LINE = "dkg agree: the instance ended without agreeing a set"
AGREE_BELOW_QUORUM_LINE = "dkg agree: below quorum, not proposing"
AGREE_NOT_MEMBER_LINE = "dkg agree: not a member of committee[epoch]; no instance for this target"
AGREE_SILENCE_LINES = (AGREE_NO_SET_LINE, AGREE_BELOW_QUORUM_LINE, AGREE_NOT_MEMBER_LINE)

#: The durable artifact store's open line. Present ⇒ a real on-disk journal was opened; ABSENT ⇒
#: `build_artifact_store` took its `partition.is_empty()` early return and handed back an
#: in-memory store, silently, with no log at all — every agreed artifact would then be lost on
#: restart and nothing would say so. That silent downgrade is the reason this is asserted here,
#: on a case that cold-restarts every validator into `--dpos` as part of its bring-up.
AGREE_REHYDRATE_LINE = "rehydrated the agreement-artifact store from disk"

#: THE MOST DIAGNOSTIC NUMBER ON THE PLANE. `view = 1` is the happy path: the first leader
#: proposed and the instance decided in one round. `view > 1` means a LEADER TIMEOUT was paid, and
#: the plane's leader timeout (30 s) on its own exceeds the whole pre-boundary window this case
#: works in (32 blocks at 1 blk/s). So a view above this is not "slower but fine" — it is a run
#: whose key could not have been ready in time, and failing here names that cause instead of
#: letting it resurface as a missing share three assertions later.
#:
#: RAISING THIS IS A DELIBERATE ACT, not a flake workaround: it converts "the agreement is
#: instant" into "the agreement is eventually", which is a different claim about the product.
AGREE_HAPPY_VIEW = 1

#: The one artifact-serving counter that must be ZERO on an all-honest stack. A rejection is a
#: served artifact refused as PROVEN misbehaviour — undecodable bytes, an answer about another
#: epoch, or a certificate that fails against `committee[epoch]` — and it is the only path that
#: costs a peer its standing on the resolver (commonware's `excluded` set has no removal path).
#: With no byzantine node in the stack there is nobody to produce one.
#:
#: The other five (`dkg_agree_body_lost/bar_unmet/logs_omitted`, `artifact_not_yet`,
#: `artifact_unverifiable`, `artifact_pull_exhausted`) are RECORDED and never asserted: each has a
#: legitimate non-zero on a healthy converging run (a delivery race, a peer asking early, a node
#: whose chain view has not caught up), so a zero-gate on them would be a flaky red. They are
#: printed every run so a drift is visible, and they ride the failure diagnostic.
AGREE_REJECT_METRIC = "dpos_dkg_artifact_rejected_total"
AGREE_REPORTED_METRICS = (
    "dkg_agree_body_lost_total",
    "dkg_agree_bar_unmet_total",
    "dkg_agree_logs_omitted_total",
    "dpos_dkg_artifact_served_total",
    "dpos_dkg_artifact_not_yet_total",
    "dpos_dkg_artifact_unverifiable_total",
    "dpos_dkg_artifact_pull_exhausted_total",
)

_AGREE_VIEW_RE = re.compile(r"\bview=(\d+)")
_AGREE_PINNED_RE = re.compile(r"\bpinned=(\d+)")


def epoch_lines(logs: str, message: str, epoch):
    """`grep "<message>" | grep -E "epoch=<N>( |,|$)"` over an ANSI-stripped log.

    TWO greps and not one: tracing renders fields in an order the case does not control, so a
    single pattern spanning message and field would be asserting a render order. The field pattern
    is right-anchored so `epoch=2` does not also match `epoch=20`.

    Same shape as `verdicts_onchain.epoch_field_lines`, kept here because this module must not
    import the production-path chunk — `smoke-base` would then pull it in for four lines."""
    pat = re.compile(r"epoch=" + re.escape(str(epoch)) + r"( |,|$)")
    return [ln for ln in (logs or "").splitlines() if message in ln and pat.search(ln)]


def agreed_view(line: str):
    """The `view=<N>` on a decided-agreement line, as an int, or None if it is not there.

    None is a REAL outcome and never a zero: the field is what the whole observation is for, and a
    line that lost it must fail loudly rather than be scored as view 0."""
    hit = _AGREE_VIEW_RE.search(line or "")
    return int(hit.group(1)) if hit else None


def agreed_pinned(line: str):
    """The `pinned=<N>` (how many dealer logs the agreed set names) on a decided line, or None."""
    hit = _AGREE_PINNED_RE.search(line or "")
    return int(hit.group(1)) if hit else None


def evaluate_agree_stage(stage: str, present: dict, epoch, missing_diag=""):
    """One stage of the ordered story, over `{service: bool}`.

    RED when ANY committee node did not reach the stage for `epoch`. Not "at least one": on a
    static committee every member runs the plane for every epoch, so one silent member is a real
    finding — it is a node that will hold no share and demote itself to verify-only at the
    boundary, which is how a committee quietly loses a signer without the chain looking broken."""
    if not present:
        return False, (f"agreement plane: no node was scanned for '{stage}' at epoch {epoch} — "
                       "refusing to report a stage nothing was read for")
    silent = sorted(s for s, ok in present.items() if not ok)
    if not silent:
        return True, ""
    return False, (f"agreement plane: {len(silent)}/{len(present)} node(s) never logged "
                   f"'{stage}' for epoch {epoch} ({', '.join(silent)}){missing_diag}")


def evaluate_agree_view(views: dict, epoch, happy=AGREE_HAPPY_VIEW):
    """The decided VIEW, over `{service: view or None}`. Three ways to go red:

      * a node's decided line carried NO `view` field — the observation's key number is gone and
        nothing downstream of it means anything;
      * two nodes decided at DIFFERENT views — they are not describing one agreement, so at least
        one of them adopted a set the others never decided;
      * the agreed view is above `AGREE_HAPPY_VIEW` — a leader timeout was paid, and one of those
        alone outlasts the pre-boundary window this case runs in.
    """
    if not views:
        return False, (f"agreement plane: no decided-agreement view was read at epoch {epoch} — "
                       "refusing to report a converged plane with nothing measured")
    blind = sorted(s for s, v in views.items() if v is None)
    if blind:
        return False, (f"agreement plane: the decided line for epoch {epoch} carried no `view=` "
                       f"field on {', '.join(blind)} — the plane's most diagnostic number is "
                       "unreadable, so nothing can be said about how it converged")
    seen = sorted(set(views.values()))
    if len(seen) != 1:
        detail = ", ".join(f"{s}=view {v}" for s, v in sorted(views.items()))
        return False, (f"agreement plane: nodes decided epoch {epoch} at DIFFERENT views "
                       f"({detail}) — they did not observe one agreement, so at least one adopted "
                       "a set the others never decided")
    view = seen[0]
    if view > int(happy):
        return False, (f"agreement plane: epoch {epoch} was agreed at view {view}, not "
                       f"{happy} — at least one 30 s leader timeout was paid, which alone exceeds "
                       "the pre-boundary window the epoch key has to be ready in")
    return True, ""


def evaluate_agree_pinned(pinned: dict, epoch):
    """The agreed set's SIZE must be identical on every node.

    Two nodes that pinned different sets select over different dealer logs and derive DIFFERENT
    `PK_epoch` — the divergence the whole agreement plane exists to prevent, and one the
    `prev_randao` window only notices later, as a disagreement between nodes.

    A size compare and not a set compare, because the size is what the line carries. It is a
    necessary condition, not a sufficient one; the sufficient one is the cross-node `prev_randao`
    identity this case already asserts a few lines later."""
    if not pinned:
        return False, (f"agreement plane: no pinned-set size was read at epoch {epoch}")
    blind = sorted(s for s, v in pinned.items() if v is None)
    if blind:
        return False, (f"agreement plane: the decided line for epoch {epoch} carried no `pinned=` "
                       f"field on {', '.join(blind)}")
    seen = sorted(set(pinned.values()))
    if len(seen) != 1:
        detail = ", ".join(f"{s}=pinned {v}" for s, v in sorted(pinned.items()))
        return False, (f"agreement plane: nodes agreed DIFFERENT pinned-set sizes at epoch "
                       f"{epoch} ({detail}) — they will select over different dealer logs and "
                       "derive different PK_epoch")
    if seen[0] < 1:
        return False, (f"agreement plane: epoch {epoch} agreed an EMPTY pinned set (pinned=0) — "
                       "the ceremony has no dealer log to select over")
    return True, ""


def evaluate_agree_rejections(per_node: dict):
    """`dpos_dkg_artifact_rejected_total` over `{service: str}` — the raw metric text, `""` when
    the family is absent or the scrape failed.

    RED two ways: any node reporting a non-zero (a served artifact was refused as proven
    misbehaviour, and this stack has no byzantine node to produce one), or NO node answering at
    all — an unread counter is not a zero one, and a rejection detector that never spoke has not
    cleared anything. One readable node is enough to assert, following
    `checks/battery._inv_dkg_pinned_idx`; the unreadable ones are named either way."""
    if not per_node:
        return False, (f"{AGREE_REJECT_METRIC}: no node was scraped — refusing to report a clean "
                       "artifact-rejection count over an empty detector")
    readable, hot = {}, {}
    for svc, raw in per_node.items():
        try:
            val = int(float(str(raw).strip()))
        except (TypeError, ValueError):
            continue
        readable[svc] = val
        if val >= 1:
            hot[svc] = val
    if hot:
        detail = ", ".join(f"{s}={v}" for s, v in sorted(hot.items()))
        return False, (f"{AGREE_REJECT_METRIC} non-zero ({detail}) — a peer served an artifact "
                       "that failed to decode, named another epoch, or carried a certificate that "
                       "does not verify against committee[epoch]. On an all-honest stack there is "
                       "nobody to produce one, and the accused peer is now excluded with no way "
                       "back.")
    if not readable:
        return False, (f"{AGREE_REJECT_METRIC} was unreadable on ALL {len(per_node)} node(s) "
                       f"({', '.join(sorted(per_node))}) — the family is absent or every scrape "
                       "failed, so the rejection property was never evaluated")
    blind = sorted(set(per_node) - set(readable))
    note = f" ({len(blind)} unreadable: {', '.join(blind)})" if blind else ""
    return True, f"{AGREE_REJECT_METRIC}=0 on {len(readable)}/{len(per_node)} node(s){note}"


def evaluate_artifact_store_durable(present: dict):
    """Every `--dpos` node opened a DURABLE artifact store.

    The absence is silent by construction: `build_artifact_store` returns an in-memory store on an
    empty partition and logs NOTHING, so a mis-wired partition looks exactly like a healthy node
    until a restart loses every agreed artifact. The line is the only witness that the on-disk
    journal was opened at all."""
    if not present:
        return False, ("agreement plane: no node was scanned for the artifact-store rehydrate "
                       "line — refusing to report a durable store nothing was read for")
    silent = sorted(s for s, ok in present.items() if not ok)
    if silent:
        return False, (f"agreement plane: {len(silent)}/{len(present)} node(s) never logged "
                       f"'{AGREE_REHYDRATE_LINE}' ({', '.join(silent)}) — `build_artifact_store` "
                       "took its empty-partition early return and handed back an IN-MEMORY store, "
                       "silently. Every agreed artifact is lost on restart and nothing says so.")
    return True, ""


def agree_silence_diag(per_node_logs: dict, epoch):
    """The failure diagnostic for a missing stage: which of the three silences each node hit for
    `epoch`, as one appended clause. Empty when none of them fired — which is itself informative,
    because it means the stage is missing for a reason the plane never named."""
    found = []
    for svc in sorted(per_node_logs):
        for line in AGREE_SILENCE_LINES:
            n = len(epoch_lines(per_node_logs[svc], line, epoch))
            if n:
                found.append(f"{svc}: {line!r} x{n}")
    if not found:
        return ("; none of the three agreement-silence lines fired for this epoch either — the "
                "stage is missing for a reason the plane never logged")
    return "; agreement-silence lines: " + " | ".join(found)
