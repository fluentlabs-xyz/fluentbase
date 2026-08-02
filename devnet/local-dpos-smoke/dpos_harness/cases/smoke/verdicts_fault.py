"""verdicts_fault.py — the PURE decision layer of the six DESTRUCTIVE smoke assertions.

Same split, same reason as `verdicts.py`: `asserts_fault.py` decides what to read, when to break
something and when to put it back; this module decides what the readings MEAN. Nothing here opens
a socket, spawns a process, sleeps, or knows that a node was stopped.

WHY THE SPLIT IS WORTH MORE HERE THAN IT WAS FOR THE READ-ONLY FOUR. A live run of these six can
only ever walk the PASS direction — it runs against a chain that recovers, because a chain that
did not recover would leave the run red with no verdict coverage gained. The FAIL directions are
the whole product of these cases (the node that does not come back, the exit code that is not 0,
the chain that does not resume, the beacon that diverges on the gap blocks) and every one of them
is a unit test in `tests/test_smoke_fault_verdicts.py` or it is untested. There is no third way.

Return convention is `verdicts.py`'s: `(ok: bool, message: str)`, the message empty on success and
otherwise the exact diagnostic bash printed, which the caller prefixes with `FAIL (<case>): `.
"""

from __future__ import annotations

import os
import re

from ...core import converge, nodes

# ══ smoke-deferred ════════════════════════════════════════════════════════════════════

#: `asserts-fault.sh:49` — `RESULT_LAG_K`, mirroring `fluentbase_consensus::K`.
RESULT_LAG_K = 3

#: `asserts-fault.sh:64` — how far past `anchor + K` the chain must be before the lag is sampled.
#: The pre-K ramp clamps finalized to the anchor, and a sample taken inside it reads a lag that is
#: an artifact of the ramp rather than of the derive pipeline.
STEADY_MARGIN = 5
STEADY_WAIT_S = 90

#: `asserts-fault.sh:80` — six readings of a moving chain. The COUNT is part of the assertion:
#: it is what makes "the derive gap never once sat at exactly K" a statement about the pipeline.
LAG_SAMPLES = 6
LAG_SAMPLE_SLEEP_S = 2
#: `:86` — the accepted overshoot above K: +1 for an in-flight derive, +1 for a ~1 blk/s FCU
#: straddle. NOT slack — anything wider is a lag that has drifted off the eager-derive steady
#: state, which is the liveness half of the invariant.
LAG_MAX_OVER = 2
#: `:100` — how far the head may speculate past the derive tip that `safe` rides.
SAFE_TRACK_MAX = 2

#: `asserts-fault.sh:116` — at most six ATOMIC consensus snapshots; the sixth failing is the
#: verdict. A wall deadline here would make the number of snapshots depend on RPC latency.
CONSENSUS_SAMPLES = 6
CONSENSUS_SLEEP_S = 1
#: `:129` — the eth read and the consensus read are two RPCs apart, so one block of skew between
#: the two tiers' answers is measurement, not disagreement.
TIER_SKEW_MAX = 1

#: `asserts-fault.sh:140-159` — the OrderBlock wire codec's FIXED header, as (field, BYTE width),
#: mirroring `crates/dpos/consensus/src/order_block.rs` `OrderBlock::write` field-for-field up to
#: and including `result`. Everything after `result` is variable-length (extra_data, RLP txs, the
#: optional beacon fields), so this prefix is the only part with fixed offsets — which is also why
#: the wire's TOTAL length cannot be pinned, only this prefix's.
#:
#: The field ORDER is part of the wire format. Add a field here when `write` grows one and every
#: offset below follows; `test_smoke_fault_verdicts.py` pins this list against the Rust codec in
#: both directions, so the two cannot drift silently again.
WIRE_HEADER_FIELDS = (
    ("parent", 32),
    ("height", 8),
    ("proposal_view", 8),
    ("timestamp", 8),
    ("fee_recipient", 20),
    ("gas_limit", 8),
    ("result", 32),
)


def wire_hex_offset(field):
    """Hex-char offset of `field` within the fixed header (the wire is read as a hex string, so
    every width doubles)."""
    at = 0
    for name, width in WIRE_HEADER_FIELDS:
        if name == field:
            return at * 2
        at += width
    raise KeyError(field)


WIRE_RESULT_OFFSET = wire_hex_offset("result")                              # 168
WIRE_RESULT_LEN = dict(WIRE_HEADER_FIELDS)["result"] * 2                    # 64
#: The whole fixed header must be present before any of it can be sliced. A LENGTH guard cannot
#: see a field inserted BEFORE the slice — the wire only gets longer — so it is not the drift
#: detector; `evaluate_result_commitment` is (it re-locates the hash on mismatch).
WIRE_MIN_LEN = sum(w for _, w in WIRE_HEADER_FIELDS) * 2                    # 232

#: `asserts-fault.sh:182-185` — the throttle and its restore. 0.15 CPU is hard enough that the
#: victim's verify gate starts timing out (EL backpressure -> nullify); 4 is the compose default.
THROTTLE_CPUS = "0.15"
RESTORE_CPUS = "4"
#: `:183` — ~1.5 epochs of throttle. NEVER shorten it: the window IS the measurement interval,
#: and the growth floor below is calibrated to its width (§2.4 item 3).
THROTTLE_WINDOW_S = 45
#: `:186` — blocks the chain must finalize while one EL is throttled. BFT f=1 must hold.
THROTTLE_MIN_GROWTH = 20

#: `asserts-fault.sh:191,171` — the victim's rejoin budget after the unthrottle.
REJOIN_TIMEOUT_S = 180
REJOIN_POLL_S = 3

#: `asserts-fault.sh:177` — the throttle victim. NOT validator-0 (the pinned host RPC every
#: reading in this case comes from) and NOT validator-3 (which the two later cases stop and
#: SIGKILL, so throttling it would stack two faults on one node across the chained run).
DEFERRED_VICTIM_IDX = 1


def evaluate_lag_sample(latest, final, safe, k=RESULT_LAG_K):
    """One K-lag reading (`asserts-fault.sh:85-100`). Five properties, in bash's order.

    The FIRST is the safety half and the only one that is not a tolerance: `lag < K` means the
    node called a result final that the deferred pipeline has not derived yet — a finality
    OVERCLAIM. The other four bound the healthy band and the ancestry `finalized ⊆ safe ⊆ head`,
    which a single-tag probe cannot see at all.
    """
    latest, final, safe, k = int(latest), int(final), int(safe), int(k)
    lag = latest - final
    if lag < k:
        return False, (f"finalized overclaims — lag={lag} < K={k} "
                       f"(latest={latest} finalized={final})")
    if lag > k + LAG_MAX_OVER:
        return False, (f"finality lag drifted — lag={lag} > K+{LAG_MAX_OVER} "
                       f"(latest={latest} finalized={final}; eager-derive budget is K, +1 "
                       "in-flight derive, +1 straddle)")
    if safe < final:
        return False, (f"safe below finalized — safe={safe} < finalized={final} "
                       "(ancestry finalized ⊆ safe violated)")
    if safe > latest:
        return False, (f"safe above latest — safe={safe} > latest={latest} "
                       "(ancestry safe ⊆ head violated)")
    if latest - safe > SAFE_TRACK_MAX:
        return False, (f"safe not tracking the derive tip — latest−safe={latest - safe} > "
                       f"{SAFE_TRACK_MAX} (latest={latest} safe={safe}; eager-derive budget is 0 "
                       "plus in-flight + straddle)")
    return True, ""


def lag_witnesses(samples, k=RESULT_LAG_K):
    """`(saw_exact, saw_safe_ahead)` over `samples` = [(latest, final, safe)].

    Two LIVENESS witnesses that the per-sample band checks above cannot make. `saw_exact` says
    the DERIVE GAP actually reached the eager-derive steady state at least once rather than
    sitting permanently wide, which is a derive pipeline that is lagging but still inside
    tolerance. `saw_safe_ahead` says the safe/finalized SPLIT took effect: before the split,
    `safe` and `finalized` were the same tag, `latest−safe` was always exactly K, and every band
    check in this case would have passed over a chain with no separate derive tier at all.

    WHY `saw_exact` KEYS ON `safe − final`, NOT ON `lag = latest − final`. The executor writes
    `safe` and `finalized` in ONE forkchoice message with `finalized = ordering_tip − K`
    (`crates/dpos/consensus/src/executor.rs:2549-2580`, `order_block.rs:193-195`), so
    `safe − finalized == K` BY CONSTRUCTION is the derive-pipeline statement. `latest` is the
    SPECULATIVE head — advanced at notarization (`executor.rs:2151-2153`) and deliberately not
    rolled back when `correctly_speculated` (`:2586-2589`) — so `lag = K + (latest − safe)`
    carries the speculative lead. `evaluate_lag_sample` above calls `latest − safe` up to
    `SAFE_TRACK_MAX` healthy; keying the witness on `lag == k` would demand a ZERO speculative
    lead, the worst point of a range this same code calls fine, and is the soak's
    `spec-head-lag` "speculation is dead" signature. Do not move it back.
    """
    k = int(k)
    saw_exact = False
    saw_safe_ahead = False
    for latest, final, safe in samples:
        latest, final, safe = int(latest), int(final), int(safe)
        if safe - final == k:
            saw_exact = True
        if latest - safe < k:
            saw_safe_ahead = True
    return saw_exact, saw_safe_ahead


def evaluate_lag_witnesses(saw_exact, saw_safe_ahead, k=RESULT_LAG_K):
    """`asserts-fault.sh:104-105`."""
    if not saw_exact:
        return False, (f"derive gap never sampled at exactly K={k} — safe−finalized (the "
                       "derive-pipeline gap, free of the speculative head lead) never reached "
                       f"the eager-derive steady state in {LAG_SAMPLES} samples (derive pipeline "
                       "lagging)")
    if not saw_safe_ahead:
        return False, (f"safe never sampled ahead of the finalized tier — latest−safe never "
                       f"< K={k} (the safe/finalized split did not take effect)")
    return True, ""


def evaluate_consensus_sample(cons_fin, cons_res, k=RESULT_LAG_K, raw=""):
    """One `consensus_getLatest` snapshot (`asserts-fault.sh:120-123`).

    Returns `(ok, message, is_exact)`. The consensus namespace must tell the same story as the
    eth tags, and this reading is ATOMIC — one RPC, both tiers — so anything outside {K, K+1} is
    a hard tier disagreement rather than the skew the eth cross-check tolerates.

    A `None`/`"null"` field is INCOMPLETE, not zero. Coercing it would compute a gap out of a
    field the node did not answer, and the arithmetic would look exactly like a real reading.
    """
    if (cons_fin is None or cons_res is None
            or str(cons_fin) == "null" or str(cons_res) == "null"):
        return False, f"consensus_getLatest incomplete: {raw}", False
    k = int(k)
    cgap = int(cons_fin) - int(cons_res)
    if cgap not in (k, k + 1):
        msg = (f"consensus tiers disagree — latestFinalized={cons_fin} "
               f"latestResultFinalized={cons_res} cgap={cgap} (want K={k}, transiently "
               "K+1 — eager derive)")
        return False, msg, False
    return True, "", cgap == k


def evaluate_consensus_exact(seen_exact, k=RESULT_LAG_K):
    """`asserts-fault.sh:126` — the gap must have been sampled at exactly K at least once.

    K+1 forever is inside the per-sample band and is still a finding: the result tier is durably
    one block behind the steady state, which is a derive pipeline that never catches up."""
    if not seen_exact:
        return False, (f"consensus cgap never sampled at exactly K={k} — the result tier is "
                       "durably a block behind the eager-derive steady state (derive pipeline "
                       "lagging)")
    return True, ""


def evaluate_tier_skew(eth_final, cons_res, limit=TIER_SKEW_MAX):
    """`asserts-fault.sh:128-129` — the eth `finalized` tag and `latestResultFinalized` are the
    same number read two RPCs apart. Anything past one block is the two tiers disagreeing about
    what has been derived, not the chain having moved between reads."""
    delta = abs(int(eth_final) - int(cons_res))
    if delta > int(limit):
        return False, (f"eth finalized={eth_final} vs latestResultFinalized={cons_res} "
                       f"(skew > {limit})")
    return True, ""


def evaluate_artifact_wire(wire, height, artifact_raw=""):
    """`asserts-fault.sh:137-159` — the ordering artifact's wire bytes, sliced at the fixed codec
    offset. Returns `(ok, message, committed_result_hex)`.

    Both guards are the difference between a failing test and a lying one. An ABSENT artifact
    would slice to "" and compare equal to nothing; a SHORT one would slice into whatever the
    next field happens to be, and a 64-hex run of some other field is still 64 hex chars — the
    comparison below would fail with a message blaming the chain for a codec change.

    Neither guard sees a field inserted BEFORE `result`: that makes the wire LONGER, so the
    length check passes while the slice lands on a neighbour. `evaluate_result_commitment` is
    what catches that.
    """
    w = wire or ""
    if w.startswith("0x"):
        w = w[2:]
    if not w or w == "null":
        return False, f"no ordering artifact at {height}: {artifact_raw}", ""
    if len(w) < WIRE_MIN_LEN:
        return False, (f"artifact wire too short ({len(w)} hex chars) — OrderBlock codec layout "
                       "changed?"), ""
    return True, "", w[WIRE_RESULT_OFFSET:WIRE_RESULT_OFFSET + WIRE_RESULT_LEN]


def evaluate_result_commitment(committed, derived, artifact_height, n, wire=""):
    """`asserts-fault.sh:160-170` — the artifact at N+K commits the derived EVM block hash of N.

    This is the ONE check in the tree that ties the consensus artifact to the execution result.
    Everything else about deferred execution is a height relationship, and heights agree happily
    on two different chains.

    It is ALSO the layout-drift detector, because a length guard structurally cannot be one: a
    field inserted before `result` lengthens the wire and shifts the slice onto its neighbour. So
    on a mismatch, re-locate the derived hash in the wire. If it is there at another offset, the
    codec moved and the chain is fine — say so, instead of reporting a fabricated safety
    violation. Absent, the mismatch is what it claims to be."""
    d = derived or ""
    if d.startswith("0x"):
        d = d[2:]
    if (committed or "").lower() != d.lower():
        drift = ""
        w = (wire or "").lower()
        if w.startswith("0x"):
            w = w[2:]
        at = w.find(d.lower()) if d else -1
        if at >= 0:
            drift = (f" — LAYOUT CHANGED: the derived hash sits at hex offset {at}, not "
                     f"{WIRE_RESULT_OFFSET}; a field was added to OrderBlock::write ahead of "
                     "`result` and WIRE_HEADER_FIELDS did not follow")
        return False, (f"result commitment mismatch at h={artifact_height} — artifact "
                       f"result={committed}, derived hash({n})={d}{drift}")
    return True, ""


def evaluate_throttle_liveness(pre, during, want=THROTTLE_MIN_GROWTH, window=THROTTLE_WINDOW_S):
    """`asserts-fault.sh:186` — the chain kept finalizing while one validator's EL was starved.

    The property is BFT f=1 with a slow node rather than a dead one: the victim's verify gate
    times out, the leader nullifies its view, and the remaining quorum carries the chain. A
    stall here is the failure the whole deferred design is supposed to rule out."""
    if int(during) >= int(pre) + int(want):
        return True, ""
    return False, (f"chain stalled under one slowed EL — finalized {pre} → {during} in "
                   f"{window}s (want +{want})")


def victim_rejoined(vfin_head, during) -> bool:
    """`asserts-fault.sh:194` — the unthrottled victim's OWN finalized view reached the network
    tip observed at unthrottle time.

    Anchored on `during` and not on "it answers at all": a victim that comes back serving RPC at
    a height from before the throttle is exactly the stuck-catch-up this half exists to catch.
    The `"null"` guard is the sentinel discipline — an unreachable node must not read as 0 and
    then satisfy a `>=` against a small target."""
    if not vfin_head or vfin_head == "null":
        return False
    return nodes.hex_to_dec(vfin_head) >= int(during)


# ══ smoke-peers ═══════════════════════════════════════════════════════════════════════

#: `asserts-fault.sh:241,225` — discovery settle budget, and the devp2p handshake budget. They
#: are separate polls because the two planes settle independently and at different speeds.
PEERS_SETTLE_S = 60
PEERS_POLL_S = 2
RETH_PEERS_SETTLE_S = 60
#: `:263` — after the restart, all three properties must hold TOGETHER within this budget.
RECONNECT_S = 120
RECONNECT_POLL_S = 3
#: `asserts-fault.sh:253,235` — the restarted spoke. validator-1 and not validator-0: the whole
#: case is read THROUGH validator-0's metrics endpoint, so restarting it would take the observer
#: down with the observed.
PEERS_VICTIM_IDX = 1

_ADDR_RE = re.compile(r"0x[0-9a-fA-F]{40}")

#: `asserts-fault.sh:232` — the commonware p2p `tracker_*` gauges are NOT in `Metrics::encode()`
#: output. The observable per-peer series is keyed by the peer's consensus pubkey, so the
#: connected count is the number of DISTINCT peers validator-0 exchanges broadcasts with.
_PEER_SERIES_RE = re.compile(r'outer_engine_buffered_peer_total\{sequencer="[0-9a-f]+"\}')


def committee_size(committee_out) -> int:
    """`asserts-fault.sh:222` — `cast` prints an `address[]` as `[0x.., 0x..]`; count them."""
    return len(_ADDR_RE.findall(committee_out or ""))


def connected_count(metrics_text) -> int:
    """`asserts-fault.sh:230-234` — distinct `sequencer="…"` label values on the buffered-peer
    family. Deduplicated, because the family renders once per peer per metric and the question is
    how many PEERS there are."""
    return len(set(_PEER_SERIES_RE.findall(metrics_text or "")))


def evaluate_connected(count, expect):
    """`asserts-fault.sh:247` — validator-0 tracks exactly `committee_size - 1` peers.

    EXACT, not `>=`. Tracked peer set == on-chain committee (Addendum B), so a node connected to
    MORE peers than the committee has members is a discovery leak and a node connected to fewer
    is an under-connected committee; both are findings and a `>=` would see neither."""
    if int(count) != int(expect):
        return False, f"connected={count} != {expect}"
    return True, ""


def evaluate_reth_peers(peers, service):
    """`asserts-fault.sh:255` — a spoke holds at least one live reth devp2p peer.

    The regression guard for `dpos_rejoin_el_sync_devp2p`: under `--dpos` the CLI override must
    keep reth's `--trusted-peers` wired, and if it does not, EL rejoin-sync breaks in a way that
    only shows up much later as a node that cannot catch up."""
    if int(peers) <= 0:
        return False, (f"{service} reth devp2p net_peerCount={peers} (want > 0 — --dpos peering "
                       "not wired)")
    return True, ""


def peers_reconnected(count, expect, peers, fin, pre) -> bool:
    """`asserts-fault.sh:265` — all three at once after the restart.

    The third condition is the one that makes this a rejoin test rather than a socket test: two
    peer planes can reconnect around a node that contributes nothing, so the chain must also have
    finalized PAST the restart point."""
    return (int(count) == int(expect) and int(peers) > 0 and int(fin) > int(pre))


# ══ smoke-vrf-fault ═══════════════════════════════════════════════════════════════════

#: `asserts-fault.sh:291` — how far into epoch 2 the chain must be before the fault is injected.
#: The beacon is threshold-active from epoch 2 (deterministic bootstrap), so a fault window that
#: opened earlier would exercise the SEEDLESS epochs and prove nothing about the threshold seed.
VRF_FAULT_EPOCH2_MARGIN = 8
VRF_FAULT_EPOCH2_WAIT_S = 300
#: `:306` — blocks the chain must produce while the victim is down. The gap it will have to
#: re-obtain on restart, and the window the survivors' beacon is sampled over.
VRF_FAULT_GAP = 10
VRF_FAULT_GAP_WAIT_S = 150
VRF_FAULT_NODES_HAVE_S = 90
#: `:327` — the restarted victim's catch-up budget, and its poll cadence.
VRF_FAULT_CATCHUP_S = 150
VRF_FAULT_CATCHUP_POLL_S = 2
#: `:295` — log-tail depth on every fail path in the two VRF fault cases.
VRF_FAULT_LOG_TAIL = 120

#: `asserts-fault.sh:285-286` — the victim, and the four nodes that must stay byte-identical.
#: validator-3 is chosen because it is neither the pinned host RPC (validator-0) nor the pinned
#: cert-upstream anchor (validator-1); the survivor set is n−f=3 validators PLUS the import
#: follower, whose agreement is the cert-follower half of the property.
VRF_FAULT_VICTIM_IDX = 3


def evaluate_gap_mixhashes(rows, victim, reason, reference="validator-0"):
    """The per-height compare of the RESTARTED victim's gap blocks against the reference chain.

    `rows` = [(height, victim_mixhash, reference_mixhash)]. Shared by `assert_vrf_fault`'s B4
    (`asserts-fault.sh:338-349`) and `assert_vrf_dkg_liveness`'s step 3 (`:443-449`), which run
    the identical loop under two different claims — `reason` carries which.

    A "null"/empty reading is a MISS, not a skip. That is the difference between "the victim
    caught up and derived the same seed" and "the victim never got the block, so nothing was
    compared" — and the second one is what a healthy-looking silence would report."""
    miss = []
    for height, d, v in rows:
        if d == "null" or not d:
            miss.append(f"{height}=missing-on-{victim}")
            continue
        if d != v:
            miss.append(f"{height}: {victim}={d} != {reference}={v}")
    if miss:
        listing = "\n".join(f"  {m}" for m in miss)
        return False, f"{reason}\n{listing}"
    return True, ""


# ══ smoke-vrf-dkg-liveness ════════════════════════════════════════════════════════════

#: `asserts-fault.sh:379` — `DKG_MARGIN_BLOCKS` (consensus/beacon/actor.rs; 10→16→20 for the AM5
#: fetch-before-finalize schedule). The epoch-2 DKG window is [epoch_start(2) − 20, epoch_start(2)).
#:
#: The bash spells the knob with the pre-rename `SOAK_` prefix because it borrows the SIM's
#: variable rather than declaring its own — its
#: own comment says "mirrors soak-invariants.sh's default", and `soak-invariants.sh:2230` calls
#: itself the single source of truth for "the barrier verdict + gate tests + asserts-fault window".
#: The port keeps that SHARING, under the renamed spelling: `sim/reconcilers.py:64` already reads
#: `SIM_DKG_MARGIN_BLOCKS` with the same default, so this reads the same name rather than
#: introducing a second knob that means the same thing and can disagree.
DKG_MARGIN_BLOCKS = 20
DKG_MARGIN_ENV = "SIM_DKG_MARGIN_BLOCKS"
#: `:384` — how far past the boundary the chain must finalize with the victim still down.
DKG_BOUNDARY_MARGIN = 6
DKG_BOUNDARY_WAIT_S = 400
DKG_NODES_HAVE_S = 120
DKG_CATCHUP_S = 150
DKG_CATCHUP_POLL_S = 2
#: `:450` — the post-rejoin liveness window. Short on purpose: at 1 blk/s six seconds is several
#: blocks, and the question is only whether the chain is still moving.
DKG_LIVENESS_WINDOW_S = 6

#: `asserts-fault.sh:430` — the actor logs this ONLY for an epoch it actually finalized a share
#: for. A member excluded from epoch 2's Joint-Feldman QUAL produces no such line for epoch 2,
#: which is what makes counting its ABSENCE a real assertion rather than a log-volume one.
SHARE_LINE = "live DKG: PK_epoch + share computed + stored"


def dkg_margin_blocks(env=None) -> int:
    """The bash `: "${<margin>:=20}"` default (asserts-fault.sh:379), under the renamed knob."""
    raw = (env if env is not None else os.environ).get(DKG_MARGIN_ENV, "")
    try:
        return int(raw) if str(raw).strip() else DKG_MARGIN_BLOCKS
    except (TypeError, ValueError):
        return DKG_MARGIN_BLOCKS


def evaluate_window_open(fin, window_open):
    """`asserts-fault.sh:390-392` — the victim must be stopped BEFORE its DKG window opens.

    The TIMING is the assertion. Stopping it after the window opened means it may already hold a
    share, and the case would then be asserting that a share-holder sits out — which is not true
    and not what it claims. Fail loud rather than run a test that measures something else."""
    if int(fin) >= int(window_open):
        return False, (f"chain already at/past the epoch-2 DKG window ({window_open}) — cannot "
                       "stop the victim before its ceremony (raise the activation gap or run "
                       "earlier)")
    return True, ""


def epoch_share_lines(log_text, epoch=2, marker=SHARE_LINE):
    """The victim's `share computed + stored` lines for `epoch`, if any.

    `epoch=2( |,|$)` (asserts-fault.sh:431) and not a bare `epoch=2`, which would also match
    `epoch=20` and turn a later healthy ceremony into a false finding.

    THE CALLER PASSES ANSI-STRIPPED TEXT, and that is a deliberate strengthening of the bash.
    §2.4 item 2: the node writes SGR escapes INSIDE its `key=value` pairs, so bash's unstripped
    `grep -E "epoch=2( |,|$)"` could never match a real line — its negative assertion was at risk
    of holding vacuously. Stripping can only ADD matches, so it can only make this stricter, and
    a check that cannot fail is worth less than one that can."""
    pat = re.compile(rf"epoch={int(epoch)}( |,|$)")
    return [ln for ln in (log_text or "").splitlines()
            if marker in ln and pat.search(ln)]


def evaluate_no_epoch_share(lines, victim, epoch=2):
    """`asserts-fault.sh:432-436` — the offline member holds NO share for the epoch it missed."""
    if lines:
        listing = "\n".join(f"    {ln}" for ln in lines)
        return False, (f"{victim} logged an epoch-{epoch} share despite being offline during the "
                       f"epoch-{epoch} DKG window — it should be SHARELESS for epoch {epoch} "
                       f"(QUAL exclusion):\n{listing}")
    return True, ""


def evaluate_still_finalizing(before, after, victim):
    """`asserts-fault.sh:451` — the chain is still finalizing after the victim rejoined.

    The rejoin of a SHARELESS member is the interesting direction: it must derive prev_randao
    from the cert seed like any verify-only node and must not wedge the seed quorum it is not
    part of."""
    if int(after) > int(before):
        return True, ""
    return False, f"chain not finalizing after {victim} rejoined ({after} <= {before})"


# ══ smoke-crash-survivor ══════════════════════════════════════════════════════════════

#: `asserts-fault.sh:475-476` — the EL gap the crashed node must backfill. A SOFT target: the
#: wait is `|| true` and the hard floor is the +3 below, so a chain that merely paced slowly does
#: not fail a case that is about recovery.
CRASH_GAP = 12
CRASH_GAP_WAIT_S = 90
CRASH_MIN_ADVANCE = 3
#: `:486` — ten minutes. DELIBERATELY long: the question the budget answers is whether the
#: post-ungraceful-crash `connected_peers=0` is PERMANENT or merely slow to re-peer, and a short
#: deadline would report the second as the first.
CRASH_RECOVER_S = 600
CRASH_POLL_S = 3
#: `:495` — how often the peer probe prints, in ticks (so every 30s at the 3s cadence).
CRASH_PEER_PROBE_EVERY = 10
CRASH_STALL_LOG_TAIL = 120
CRASH_FAIL_LOG_TAIL = 80
#: `:468` — the SIGKILL victim. validator-3 again: 3/4 is still quorum, and it is neither anchor.
CRASH_VICTIM_IDX = 3


def evaluate_chain_advanced_while_crashed(head, pre, want=CRASH_MIN_ADVANCE):
    """`asserts-fault.sh:478` — the chain kept finalizing with 1/4 SIGKILLed.

    The premise of the case, not a nicety: if the chain stalled there is no EL gap for the victim
    to backfill, and the recovery half below would pass over a node that had nothing to recover."""
    if int(head) >= int(pre) + int(want):
        return True, ""
    return False, f"chain stalled with 1/4 crashed (finalized={head}, pre={pre})"


def crash_survivor_realigned(reference, victim, floor_dec, producer_hash_at=None) -> bool:
    """`asserts-fault.sh:501` — the victim is back ON THE PRODUCER'S CHAIN and PAST `floor_dec`.

    NOT byte-identical readings. The docstring that used to sit here defended the whole-reading
    compare ("the WHOLE reading, hash included") and it was wrong: the two readings are two
    separate RPC round-trips against two independently-advancing tips on a chain producing a
    block a second, so demanding they coincide is a condition about wall-clock luck, not about
    recovery. Worse here than almost anywhere: the victim is BACKFILLING AN EL GAP the case
    deliberately built, so trailing the hub is the normal shape of the thing being asserted, and
    the ten-minute budget existed precisely because the answer takes time to arrive.

    What the compare existed for is kept whole — `converge.aligned_reading` checks the producer's
    block AT THE VICTIM'S OWN HEIGHT, so a victim at height h on a different chain still fails,
    and a same-height fork fails too (byte-identity no longer holds, so the producer read fires).
    The head guard still rejects `"null"` / `""` / genesis, which is what stops two unreachable
    nodes — trivially equal — from reading as recovery: the exact wedge the case was written for
    (`connected_peers=0`, no blocks, an RPC answering nothing).

    THE FLOOR IS `head` — the finalized height the chain was MEASURED at while the victim was
    down — and running without one made this verdict VACUOUS. The dropped `v0 == vn` equality
    was doing a second, unstated job: it could not be satisfied until the victim had actually
    caught up to the producer. With no floor at all, a live run passed on
    `validator-3 … realigned at 0x41(=65) … (v0=0x50(=80))` — fifteen blocks behind, on its own
    persisted tail, never having backfilled the EL gap the case exists to build. `head` is the
    right floor because it is the one height the chain is PROVEN to have reached during this
    cycle: it is read straight after the gap wait and `evaluate_chain_advanced_while_crashed`
    fails the case unless `head >= pre + 3`. `pre + CRASH_GAP` is NOT usable — that wait is soft
    (`|| true` in bash), so the +12 is a target, not a fact — and the producer's live reading is
    not usable either: `aligned_reading` applies the floor PER READER, so a floor taken from the
    producer's own current height would reject the producer itself."""
    return converge.aligned_reading(
        [("producer", reference), ("victim", victim)], int(floor_dec),
        producer_hash_at) is not None


# ══ smoke-full-restart ════════════════════════════════════════════════════════════════

#: `asserts-fault.sh:517` — the graceful stop ceiling. Long enough for reth's
#: `on_graceful_shutdown` to persist the in-memory tail to MDBX; a SIGKILL at the ceiling exits
#: 137 instead of 0 and `shutdown_flushed` sees it.
FULL_RESTART_STOP_TIMEOUT_S = 40
#: `:525` — the reconverge budget after the whole validator set comes back from disk.
FULL_RESTART_RECONVERGE_S = 120
FULL_RESTART_POLL_S = 2
FULL_RESTART_LOG_TAIL = 200


def evaluate_flushed(service, flushed):
    """`asserts-fault.sh:519` — every validator exited with code 0.

    This IS the flush assertion. Nothing else in the case can see whether the persistence
    actually landed: a node that was SIGKILLed at the stop ceiling comes back, resyncs from its
    peers and reconverges perfectly, so the reconverge check below would pass over a validator
    that lost its tail. The exit code is the only witness, which is why the barrier reads
    `docker inspect` container metadata rather than logs."""
    if flushed:
        return True, ""
    return False, f"{service} did not exit cleanly (code 0) on shutdown"


def full_restart_reconverged(readings, pre, producer_hash_at=None) -> bool:
    """`asserts-fault.sh:549-550` — all five on the producer's chain and STRICTLY PAST the
    pre-stop height. `readings` is the producer's first, then validators 1..n, then the full-node.

    SAME-HEIGHT identity, not five-way byte identity. The old compare was only ever true at the
    instant the fleet came back — the whole set had just been stopped, so a coinciding reading
    exists at the persisted head — and one second later the five tips are ragged again for
    entirely healthy reasons. That is the mitigation this site had, not a correctness argument:
    the check passed on the coincidence and would fail the moment the restart staggered.

    `> pre`, NOT `>= pre`, and that is the half the same-height rewrite lost. Byte identity used
    to make `>= pre` mean something stronger than it says: five readings that are equal AND at
    the persisted head can only be the instant of resumption, so the case was in practice
    watching the fleet come back together. Ragged heights break that — one wedged node sitting
    exactly at `pre` while the other four climb satisfies `>= pre` and reads as reconvergence.
    Both trees then passed at EXACTLY the floor (`all 5 reconverged at 0x41 (>= pre=65)`), which
    proves only "everyone came back on the same persisted tail", not "the chain resumed". The
    floor is therefore `pre`, and `aligned_reading`'s floor is STRICT, so every node must be at
    `pre + 1` or better: the chain PRODUCED at least one block after the restart and all five saw
    it. `baseline_height` fails loud below 1, so the floor is never negative.

    What is still NOT asserted here is SUSTAINED liveness — one block past `pre` is all this
    demands, and nothing in this case (it is the last assertion of `smoke-fault` and the only one
    of `smoke-full-restart`) looks again afterwards.

    The guards that mattered are inside `aligned_reading` and are unchanged: an all-down poll
    (five identical `"null|null"` readings) and a fleet that came back at genesis (every data
    directory wiped, all five in perfect agreement about `0x0`) are both rejected on the head —
    the second is the failure this case would otherwise silently bless."""
    labelled = [(f"reading-{i}", r) for i, r in enumerate(readings)]
    return converge.aligned_reading(labelled, int(pre), producer_hash_at) is not None
