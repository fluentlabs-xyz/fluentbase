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
    (`asserts-fault.sh:338-349`) and `assert_vrf_dkg_live_heal`'s prev_randao leg (`:443-449`), which run
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


# ══ smoke-vrf-dkg-live-heal ═══════════════════════════════════════════════════════════

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
#: How long the restarted victim gets to pull the live epoch's artifact and finish the recompute,
#: and the poll cadence. It is NOT the catch-up budget: the heal runs off the beacon's own height
#: tick AFTER the node is caught up, the pull is throttled to one attempt per epoch per 5 s, and
#: the recompute waits on the dealer logs the resolver fetches. Generous rather than tight,
#: because a budget that expires reports "it never healed" for a heal that merely had not landed
#: — the wrong bug, and the one the case exists to deny.
DKG_HEAL_S = 240
DKG_HEAL_POLL_S = 3
#: How long the chain gets to finish epoch 2 after the heal, so `producedAt(2, …)` is FINAL when
#: it is read. Sized off the epoch, not off a guess: at ~1 blk/s a 64-block epoch is ~64 s from
#: its first block, and the heal lands ~20 blocks in — this is that with room for a slow host.
DKG_EPOCH_END_S = 240

#: `:450` — the post-rejoin liveness window. Short on purpose: at 1 blk/s six seconds is several
#: blocks, and the question is only whether the chain is still moving.
DKG_LIVENESS_WINDOW_S = 6

#: `asserts-fault.sh:430` — the actor logs this ONLY for an epoch it actually finalized a share
#: for. A member excluded from epoch 2's Joint-Feldman QUAL produces no such line for epoch 2,
#: which is what makes counting its ABSENCE a real assertion rather than a log-volume one.
SHARE_LINE = "live DKG: PK_epoch + share computed + stored"


def dkg_margin_blocks(env=None) -> int:
    """The seal-deadline margin, under the renamed knob shared with the SIM's DKG barrier.

    NO LONGER READ BY THE LIVE-HEAL CASE, deliberately: that case keys on where the deal phase
    OPENS (`dkg_deal_window_open`), and this is where it CLOSES. Kept because the knob is the one
    `sim/reconcilers.py` reads for the same product constant, and because
    `test_the_DEAL_window_opens_at_epoch_1_and_not_at_the_seal_deadline` needs both edges to state
    the difference the case got wrong."""
    raw = (env if env is not None else os.environ).get(DKG_MARGIN_ENV, "")
    try:
        return int(raw) if str(raw).strip() else DKG_MARGIN_BLOCKS
    except (TypeError, ValueError):
        return DKG_MARGIN_BLOCKS


#: The finalized-height LEAD the beacon's clock runs at. `dkg_height = finalized + K`
#: (`crates/node/src/dpos.rs`, the finalized-height poller; `K = 3`, `order_block.rs`), and the
#: ceremony's phases are keyed on THAT clock, so every window edge below is an actor-clock edge
#: converted back into a finalized height by subtracting this.
DKG_CLOCK_LEAD = RESULT_LAG_K


def dkg_deal_window_open(activation_block, interval, lead=DKG_CLOCK_LEAD):
    """The finalized height at which the epoch-2 ceremony's DEAL phase OPENS.

    THIS IS NOT `epoch_start(2) - DKG_MARGIN_BLOCKS`, AND THE DIFFERENCE COST A GREEN-FOR-THE-
    WRONG-REASON RUN. `on_height` calls `maybe_start(now + 1)` on every tick, so committee[2]'s
    ceremony starts the moment the actor's clock first enters epoch 1 and the whole of epoch 1 is
    its deal phase (`beacon/actor.rs`). `DKG_MARGIN_BLOCKS` is only where that phase CLOSES — the
    seal deadline. A victim stopped between the two has already received, ACKED and journaled
    every dealing, which changes what the case tests twice over:

      * its own journal now completes the recompute, so the heal never fetches a dealer log and
        never touches the reveal fallback — the one path this case exists to cover;
      * having acked, no honest dealer reveals its point at all, so the fallback could not run
        even if the logs were fetched.

    Both leave every assertion in the case green. Only `want == dealers` at the recompute sees it,
    which is why this guard and `evaluate_victim_held_nothing` are BOTH gates and neither is
    redundant."""
    return epoch_start_1(activation_block, interval) - int(lead)


def epoch_start_1(activation_block, interval):
    """First block of relative epoch 1 — the epoch during which committee[2] deals."""
    return int(activation_block) + int(interval)


def evaluate_window_open(fin, window_open):
    """The victim must be stopped BEFORE the epoch-2 ceremony's DEAL phase opens.

    The TIMING is the assertion. Fail loud rather than run a test that measures something else:
    the case cannot wait for the next opportunity, because there is no next one — committee[2]'s
    ceremony is the deterministic bootstrap and it happens once per stack."""
    if int(fin) >= int(window_open):
        return False, (f"chain already at/past the epoch-2 DKG DEAL window ({window_open}) — the "
                       "victim would receive and ACK the dealings before it is stopped, which "
                       "makes its own journal sufficient for the heal and takes the reveal "
                       "fallback out of the run entirely. Re-run (the bring-up was slow), or "
                       "raise EPOCH_INTERVAL so the migration finishes further ahead of epoch 1")
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


#: `beacon/actor.rs` — logged ONCE per process by the DkgActor at boot.
#:
#: The case uses it as a PROCESS BOUNDARY: every witness below has to be about the RESTARTED
#: victim, and the restarted victim's log still carries everything the pre-stop process wrote.
ACTOR_STARTED_LINE = "live DKG: actor started"

#: `beacon/actor.rs` — emitted by `start_fresh`, and by NOTHING else. `maybe_start`'s journal
#: tri-state routes `NoFile` to `start_fresh`, `Present` to `resume_from_journal` and `Torn` to
#: the sit-out warn; only the first of the three logs this.
#:
#: THAT EXCLUSIVITY IS THE SETUP WITNESS, and it is the sharpest one available. A `ceremony
#: started epoch=2` on the RESTARTED process says `load_journal(2) == NoFile`, i.e. the victim
#: came back holding no journal for epoch 2 at all — so it received no dealing, sent no ack, and
#: the only way any dealer's sealed log can carry its point is as a public REVEAL. That is the
#: precondition of the reveal-fallback path, established from the log rather than assumed from
#: the stop instant (which is a race against a moving chain, and lost that race once).
CEREMONY_STARTED_LINE = "live DKG: ceremony started"

#: `beacon/actor.rs` — the recompute-heal's ADOPT log, one of the TWO roads a restarted absentee
#: can reach its share by. See `SHARE_ROADS`.
HEAL_LINE = "live DKG: demoted committee member recomputed its share"
#: The same path's ENTRY log, one rung earlier. Printed as a diagnostic on the failure path: its
#: presence with no share line says the artifact arrived and the recompute did not.
HEAL_START_LINE = "starting share recompute-heal"

#: THE TWO ROADS, and why the case must accept EITHER.
#:
#: A victim absent through the whole deal phase replays its DKG clock through epoch 1 as it
#: catches up, so `maybe_start(now + 1)` fires for epoch 2 with `NoFile` and it starts a FRESH
#: ceremony. That ceremony fetches the pinned dealers' logs over the resolver and reconstructs the
#: victim's share from their reveals. What differs between runs is only WHO finishes the job:
#:
#:   * the live ceremony itself, via `finalize_over_pinned` — `SHARE_LINE`, when the
#:     reconstruction completes before the past-boundary sweep takes the ceremony away;
#:   * the demote-heal, via `recompute_scoped` over the journal that ceremony just built —
#:     `HEAL_LINE`, when the sweep gets there first.
#:
#: Both were observed live on the same case, on the same geometry, minutes apart. The reveal
#: fallback is what reconstructs the share on both; the race is between the victim's catch-up
#: speed and the boundary. So the case asserts the DISJUNCTION and names which road ran, and does
#: NOT assert the absence of either line — an earlier version asserted "no `SHARE_LINE`", which
#: made one of the two legitimate outcomes a red run.
SHARE_ROADS = ((SHARE_LINE, "the live ceremony's finalize-over-pinned"),
               (HEAL_LINE, "the demote-heal's scoped recompute"))

#: `outer.rs` — the positive edge for "this epoch left vote-only admission", logged inside
#: `EpochSchemeProvider::register`, the only place the OLD and the NEW pin state are both in hand.
#: Waiting on this rather than on the ABSENCE of a vote-only admission is deliberate: an absence
#: is green whenever certificates merely stopped arriving.
PIN_LINE = "epoch scheme upgraded to PINNED"
#: `epoch_manager.rs` — the in-process Verifier→Signer promotion, the consequence that makes the
#: production leg reachable at all.
PROMOTE_LINE = "promoted to Signer in-process"
#: …and the epoch field spelling those two share. `register` takes `epoch: Epoch` and
#: `reconcile_roles` takes `epoch: Epoch`, both rendered through `?epoch`, so the field reads
#: `epoch=Epoch(2)` and NOT `epoch=2` — `epoch_share_lines`, which anchors on the bare number,
#: cannot match it. Same trap `verdicts_rotation.SHARE_GATE_EPOCH_FMT` records; a witness filtered
#: with the wrong spelling is a witness that never fires.
PIN_EPOCH_FMT = "epoch=Epoch({})"

#: `beacon/metrics.rs` — live-epoch artifact pulls that came back with the artifact. The PULLING
#: side had no success counter before this ticket; only `served` (the serving side) existed, and a
#: served count on some other node cannot say that THIS node received one.
ARTIFACT_PULL_OK_FAMILY = "dpos_dkg_artifact_pull_ok_total"
#: The name that counter's SAMPLE line carries. `prometheus-client` appends `_total` to a counter
#: sample whatever the registered name ends in, so a registered `X_total` renders `X_total_total`
#: — and `node_metric`'s matcher is ANCHORED, so handing it the registered name returns "", which
#: is indistinguishable from a counter that never moved. `nodes.counter_sample` is the one place
#: that doubling lives.
ARTIFACT_PULL_OK_SAMPLE = nodes.counter_sample(ARTIFACT_PULL_OK_FAMILY)


def started_fresh_after_restart(log_text, epoch=2):
    """The `ceremony started` line for `epoch` written by the RESTARTED process, if any.

    Sliced at the LAST `ACTOR_STARTED_LINE`: `docker compose logs` returns the whole container
    log, pre-stop lines included, and a `ceremony started` from before the stop would mean the
    exact opposite of what this witnesses (a victim that WAS present for the deal phase). The
    slice is what makes the reading about the process the case restarted."""
    lines = (log_text or "").splitlines()
    boot = max((i for i, ln in enumerate(lines) if ACTOR_STARTED_LINE in ln), default=-1)
    tail = "\n".join(lines[boot + 1:]) if boot >= 0 else ""
    hits = epoch_share_lines(tail, epoch=epoch, marker=CEREMONY_STARTED_LINE)
    return hits[0] if hits else ""


def evaluate_started_fresh(line, victim, epoch=2):
    """THE CASE MUST VERIFY IT SET UP WHAT IT CLAIMS TO TEST — the same rule the seed-slot MITM
    follows when it reads back its own corruption before anything concludes from a rejection.

    `start_fresh` is the ONLY emitter of this line and `NoFile` is the only arm that reaches it,
    so its presence on the restarted process is proof that the victim came back with no epoch-2
    journal: no dealing received, no ack sent, and therefore no dealer able to do anything with
    its point except REVEAL it publicly. Everything the case says about the reveal fallback rests
    on this one reading.

    Its absence is the failure this case shipped with and did not catch: a victim stopped after
    the deal phase opened has journaled and ACKED every dealing, resumes from that journal instead
    of starting fresh, reconstructs from its own records, and passes every other assertion here
    while testing a path the case is not for."""
    if line:
        return True, ""
    return False, (f"{victim} did not log {CEREMONY_STARTED_LINE!r} for epoch {epoch} after its "
                   "restart — it came back holding a journal, which means it was stopped AFTER "
                   f"the epoch-{epoch} DEAL phase opened and had already received and ACKED the "
                   "dealings. It then reconstructs from its own records and no dealer reveals its "
                   "point, so the reveal-fallback path this case exists to cover did NOT run. "
                   "Re-run; if it recurs the bring-up is landing inside epoch 1 and EPOCH_INTERVAL "
                   "needs raising")


def share_road(log_text, epoch=2):
    """`(line, road)` for whichever road actually delivered the epoch's share, or `None`.

    Checked in `SHARE_ROADS` order, and the ORDER is arbitrary because the two are mutually
    exclusive in practice — the sweep either beat the ceremony or it did not."""
    for marker, road in SHARE_ROADS:
        hits = epoch_share_lines(log_text, epoch=epoch, marker=marker)
        if hits:
            return hits[0], road
    return None


def evaluate_share_acquired(road, victim, epoch=2):
    """The INVERTED verdict (FLU-1166): the member that missed its own ceremony gets the epoch key
    and its share INSIDE the epoch it was elected for.

    This case used to assert the opposite — that the member sat the epoch out — because nothing
    fetched the agreed artifact for the LIVE epoch: the epoch manager's repair sweep excludes
    `epoch >= frontier` by design, so a demoted member waited for a fetch that was never going to
    be issued and self-healed one epoch too late, if at all.

    EITHER road counts; see `SHARE_ROADS` for why the case cannot pin one."""
    if road is not None:
        return True, ""
    wanted = " or ".join(repr(m) for m, _ in SHARE_ROADS)
    return False, (f"{victim} logged none of {wanted} for epoch {epoch} — it did not recover its "
                   f"epoch-{epoch} share inside the epoch by either road. Either the live-epoch "
                   "artifact pull never landed (FLU-1166 regressed), or the reconstruction ran "
                   f"and was refused by the fork-safety self-check. Grep it for "
                   f"{HEAL_START_LINE!r}: present means the artifact arrived and the recompute "
                   "did not finish")


def epoch_debug_lines(log_text, epoch=2, marker=PIN_LINE):
    """Lines carrying `marker` and the DEBUG-spelled epoch field for `epoch`.

    MESSAGE first, then the epoch FIELD, in the two-grep shape `verdicts_rotation.share_gate_lines`
    owns and for the same reason: tracing renders fields in an order the case does not control, so
    a single combined pattern would depend on it."""
    field = PIN_EPOCH_FMT.format(int(epoch))
    return [ln for ln in (log_text or "").splitlines()
            if marker in ln and field in ln]


def pin_upgrade_lines(log_text, epoch=2, marker=PIN_LINE):
    """The victim's unpinned → PINNED transitions for `epoch`, if any."""
    return epoch_debug_lines(log_text, epoch=epoch, marker=marker)


def promote_lines(log_text, epoch=2, marker=PROMOTE_LINE):
    """The victim's in-process Verifier→Signer promotions for `epoch`, if any."""
    return epoch_debug_lines(log_text, epoch=epoch, marker=marker)


def evaluate_pin_upgraded(lines, victim, epoch=2):
    """…and the consequence on the VALIDATOR side: its own certificates for the live epoch leave
    vote-only admission.

    Distinct from the share and not implied by it. A node can hold the share and still verify
    certificates seed-blind if nothing re-registers its epoch scheme with the pin attached; before
    this ticket that state lasted `for the LIVE epoch, until the process exits`."""
    if lines:
        return True, ""
    return False, (f"{victim} never logged {PIN_LINE!r} for {PIN_EPOCH_FMT.format(int(epoch))} — "
                   f"its epoch-{epoch} scheme is still UNPINNED, so it is admitting that epoch's "
                   "certificates with the multisig quorum checked and the seed slot not")


def evaluate_promoted(lines, victim, epoch=2):
    """…and the other consequence, without which the production leg below cannot be reached: the
    share it recovered actually seated it.

    Not implied by the share line either. `reconcile_roles` re-resolves the share on its own edge,
    and a share that lands with no edge to wake is a share nobody votes with."""
    if lines:
        return True, ""
    return False, (f"{victim} never logged {PROMOTE_LINE!r} for "
                   f"{PIN_EPOCH_FMT.format(int(epoch))} — it recovered the share but was never "
                   f"seated as a signer for epoch {epoch}, so it cannot produce and the "
                   "production leg below would fail for a reason that is not about the key")


def evaluate_artifact_pull_ok(raw, victim, family=ARTIFACT_PULL_OK_SAMPLE):
    """The mechanism, named: the victim PULLED the live epoch's artifact and got it.

    Read on BOTH roads and it moves on both — verified live: on the run where the live ceremony
    finalized the share itself, this still read 1, because `drive_recompute` had already asked for
    the epoch's artifact while the ceremony was still catching up.

    `""` is UNREAD, not zero, and it fails. `node_metric` answers "" for both an absent family and
    a failed scrape, and this is a counter on a devnet-only endpoint (`--dpos.metrics-port`) that
    a mis-flagged container simply does not serve — coercing that to 0 would let an unscraped node
    satisfy the one assertion that names the fix."""
    text = str(raw or "").strip()
    if not text:
        return False, (f"{family} could not be read off {victim} (absent family or unreachable "
                       "commonware registry) — the live-epoch artifact pull is unwitnessed, and "
                       "an unread counter must never satisfy a positive assertion")
    try:
        n = int(float(text))
    except ValueError:
        return False, f"{family} on {victim} read {text!r}, which is not a number"
    if n > 0:
        return True, ""
    return False, (f"{family}={n} on {victim} — it never obtained the LIVE epoch's agreed "
                   "artifact over the beacon resolver, so nothing could have keyed its share "
                   "(FLU-1166: the repair sweep excludes the frontier by design, and this pull is "
                   "the only thing that covers it)")


#: How many blocks of epoch 2 must remain AFTER the share lands for the production leg to be a
#: test rather than a coin toss. Leader election is a stake-weighted lottery over the 4
#: equal-stake members, so a seated member wins ~1 slot in 4: over 32 blocks the chance of winning
#: none is ~1e-4, over the 8 a slow run might leave it is ~10%. Below this the case has not
#: produced the conditions it measures, and says so instead of reporting a lottery loss as a
#: broken heal.
#:
#: Calibrated against a live run: the victim was seated at height 257 — the first block of a
#: [256, 320) epoch — and finished the epoch with 10 of its 64 blocks.
MIN_POST_HEAL_BLOCKS = 32


def evaluate_heal_left_room(heal_at, epoch_end, victim, want=MIN_POST_HEAL_BLOCKS):
    """…and the same rule again, for the production leg.

    A member seated with four blocks of its epoch left can be perfectly healed and still produce
    nothing. That is a fact about the schedule, not about the fix, and reporting it as
    `producedAt=0` would be the same misdiagnosis the deal-window guard prevents upstream."""
    left = int(epoch_end) - int(heal_at)
    if left >= int(want):
        return True, ""
    return False, (f"{victim} was seated at finalized={heal_at} with only {left} blocks of epoch 2 "
                   f"left (want >= {want}) — too few leader slots for `producedAt > 0` to mean "
                   "anything. The recovery itself is fine; the schedule is not. Re-run, or raise "
                   "EPOCH_INTERVAL")


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
#: How far above the reconverged height the FAILURE diagnostic scans for the first block the chain
#: produced after the restart. Bounded because it costs one RPC per block: the live evidence showed
#: the persisted tail running 3 blocks past the floor, and at ~1 blk/s with K=3 result lag a
#: resumed fleet puts a post-restart block within a handful of the tail's top.
FULL_RESTART_SCAN_BLOCKS = 12
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


def evaluate_produced_after_restart(block_ts, restart_at):
    """The block the fleet converged on was PRODUCED after the restart, not lifted off disk.

    THE HOLE THIS CLOSES IS THE SAMPLING MOMENT, not the comparison. `pre` is captured BEFORE the
    stop, and the stop takes up to `FULL_RESTART_STOP_TIMEOUT_S` (40 s) — so everything the chain
    finalized inside that window is on disk and clears `> pre` the moment the containers come back.
    Five nodes with a dead consensus engine satisfy the height floor perfectly.

    A timestamp cannot be replayed into the future: the proposer stamps
    `wall_clock_now().max(parent.timestamp + 1)` (`crates/dpos/consensus/src/application.rs:991`),
    so a block sealed before `compose_start` carries an earlier second and only a freshly PRODUCED
    one can carry a later one.

    An unreadable timestamp reads as 0 and fails here, which is the safe direction — but it is a
    read failure, not a product failure, so the message says which reading it is talking about."""
    if int(block_ts) >= int(restart_at):
        return True, ""
    return False, (f"the fleet converged on a block timestamped {block_ts}, before the restart at "
                   f"{restart_at} — it came back on its persisted tail and produced nothing "
                   "(a 0 here means the timestamp could not be read at all)")


def evaluate_resumed_production(produced, reconverged_head, pre, restart_at, budget_s):
    """The chain PRODUCED a block after the restart — the assertion reconvergence cannot make.

    WHY THIS IS A SEPARATE WAIT. `full_restart_reconverged` returns the instant every reader
    clears `pre`, and the persisted tail clears it unaided: `pre` is sampled BEFORE `compose_stop`,
    so the blocks the fleet wrote while it was stopping are already above the floor and on disk.
    Proven live on 2026-08-03 — floor `pre=65`, and blocks 66, 67 and 68 all carried stamps
    EARLIER than the restart. Reconvergence is a real property (the fleet came back on one chain)
    and it is kept; it simply cannot witness production, so this does.

    The comparison inside is `evaluate_produced_after_restart` and it is unchanged — `>=` with no
    tolerance, for the reason that function's docstring gives. The defect was never the
    comparison; it was which block the comparison was handed."""
    if not reconverged_head:
        return False, ("the fleet never reconverged, so whether it produced anything after the "
                       "restart was never measured")
    if produced:
        return True, ""
    return False, (f"the fleet reconverged at {reconverged_head} (> pre={pre}) but produced NO "
                   f"block within {budget_s}s: every finalized block through {reconverged_head} "
                   f"is stamped before restart_at={restart_at}. It came back on its PERSISTED "
                   "TAIL and the chain never resumed")


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
