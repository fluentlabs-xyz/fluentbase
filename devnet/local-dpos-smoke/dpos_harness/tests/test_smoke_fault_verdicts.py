"""`cases/smoke/verdicts_fault.py` — the six DESTRUCTIVE assertions' decisions, both directions.

These verdicts are the only part of the fault suite whose FAIL side can be driven at all. A live
`make smoke-fault` walks the PASS side exclusively, by construction: it runs against a chain that
recovers, and a chain that did not recover ends the run red without having exercised any verdict
it did not reach. So the interesting half — the node that does not come back, the exit code that
is not 0, the chain that does not resume, the beacon that diverges on the gap blocks — is tested
here or it is not tested.

Each test names the `asserts-fault.sh` line its expectation is quoted against.
"""

from __future__ import annotations

import pathlib
import re

import pytest

from dpos_harness.cases.smoke import verdicts, verdicts_fault as vf

K = vf.RESULT_LAG_K


def _chain(*forks):
    """A stand-in producer for the same-height fork check: the canonical chain's block at height
    N hashes to `0x<N>`, except at the heights named in `forks`, and heights above 1000 are ones
    the producer does not hold — `"null"`, exactly as `blockhash_at` coerces them.

    Every verdict below that can be reached with RAGGED readings takes an injected producer. The
    default (`nodes.blockhash_at`) shells out to `cast block`, which the suite's conftest blocks;
    relying on that block to make a unit test come out right is testing the fixture, not the
    verdict."""
    absent = set(forks)
    return lambda h: "null" if h in absent or not 1 <= h <= 1000 else f"0x{h}"


# ══ smoke-deferred: the K-lag invariant ════════════════════════════════════

def test_a_healthy_lag_sample_holds():
    """latest 140, finalized 137, safe 140 — the eager-derive steady state."""
    assert vf.evaluate_lag_sample(140, 140 - K, 140)[0]


def test_finalized_may_not_overclaim():
    """`asserts-fault.sh:85` — THE safety half. `lag < K` means the node called a result final
    that the deferred pipeline has not derived. Every convergence-based case in the tree passes
    happily over this, because they only require the nodes to agree with each other."""
    ok, msg = vf.evaluate_lag_sample(140, 139, 140)
    assert not ok and "overclaims" in msg and f"lag=1 < K={K}" in msg


def test_a_lag_wider_than_the_budget_fails():
    """`:86` — K+2 is in-flight derive plus a 1-block FCU straddle. K+3 is a lagging pipeline."""
    assert vf.evaluate_lag_sample(140, 140 - (K + 2), 140)[0]
    ok, msg = vf.evaluate_lag_sample(140, 140 - (K + 3), 140)
    assert not ok and "drifted" in msg


@pytest.mark.parametrize("latest,final,safe,why", [
    (140, 137, 136, "ancestry finalized ⊆ safe violated"),
    (140, 137, 141, "ancestry safe ⊆ head violated"),
    (140, 137, 137, "safe not tracking the derive tip"),
])
def test_the_three_tier_ancestry_violations(latest, final, safe, why):
    """`:95-100` — `finalized ⊆ safe ⊆ head`, plus safe tracking the derive tip within 2.

    The third one is not redundant with the first two: `safe == finalized` satisfies both
    inequalities and is exactly the PRE-SPLIT world, where there was no separate derive tier."""
    ok, msg = vf.evaluate_lag_sample(latest, final, safe)
    assert not ok and why in msg


def test_the_derive_gap_must_have_been_sampled_at_exactly_k():
    """`:104` — a derive gap pinned wide for all six samples is inside the lag band and still a
    finding: the derive pipeline is lagging and never catches up."""
    wide = [(140, 140 - (K + 2), 140)] * vf.LAG_SAMPLES
    saw_exact, saw_ahead = vf.lag_witnesses(wide, K)
    assert not saw_exact and saw_ahead
    ok, msg = vf.evaluate_lag_witnesses(saw_exact, saw_ahead, K)
    assert not ok and "never sampled at exactly" in msg


def test_a_speculative_lead_does_not_break_the_exact_k_witness():
    """`:93` — THE regression this witness was fixed for. `latest` is the speculative head, so
    `lag = latest − finalized = K + (latest − safe)`; every one of these samples has a 2-block
    speculative lead, which the band check on the line above calls healthy. `lag` is therefore 5
    in all six and NEVER K — the old `lag == K` witness failed the case on a chain in perfect
    eager-derive steady state. `safe − finalized` is K throughout, which is the derive statement.
    """
    speculating = [(142, 137, 140)] * vf.LAG_SAMPLES
    assert all(latest - final != K for latest, final, _ in speculating)
    for latest, final, safe in speculating:
        assert vf.evaluate_lag_sample(latest, final, safe, K)[0]
    saw_exact, saw_ahead = vf.lag_witnesses(speculating, K)
    assert saw_exact and saw_ahead
    assert vf.evaluate_lag_witnesses(saw_exact, saw_ahead, K) == (True, "")


def test_a_wrong_derive_gap_fails_even_when_the_head_lag_reads_k():
    """The other direction, and the one that proves the witness still bites: `latest − finalized`
    is EXACTLY K in all six samples (the old witness passed on this), the whole per-sample band
    passes, and yet `safe − finalized` is 1 — the derive tier is not sitting K behind the tip it
    rides, which is what the invariant is about."""
    wrong_gap = [(140, 137, 138)] * vf.LAG_SAMPLES
    assert all(latest - final == K for latest, final, _ in wrong_gap)
    for latest, final, safe in wrong_gap:
        assert vf.evaluate_lag_sample(latest, final, safe, K)[0]
    saw_exact, saw_ahead = vf.lag_witnesses(wrong_gap, K)
    assert not saw_exact
    ok, msg = vf.evaluate_lag_witnesses(saw_exact, saw_ahead, K)
    assert not ok and "never sampled at exactly" in msg and "safe−finalized" in msg


def test_safe_must_have_been_sampled_ahead_of_the_finalized_tier():
    """`:105` — the pre-split world: `safe == finalized == latest−K`. Every per-sample band check
    passes over it, so without this witness the case would report a chain with no derive tier as
    a healthy two-tier chain. (Since the exact-K witness moved onto `safe − finalized`, the
    pre-split world now trips BOTH witnesses — `safe == finalized` reads a derive gap of 0 — so
    the message this test pins is driven directly.)"""
    pre_split = [(140, 140 - K, 140 - K)] * vf.LAG_SAMPLES
    saw_exact, saw_ahead = vf.lag_witnesses(pre_split, K)
    assert not saw_exact and not saw_ahead
    ok, msg = vf.evaluate_lag_witnesses(True, saw_ahead, K)
    assert not ok and "safe never sampled ahead" in msg


def test_a_healthy_sample_set_makes_both_witnesses():
    assert vf.lag_witnesses([(140, 140 - K, 140)], K) == (True, True)


# ══ smoke-deferred: the consensus tiers ════════════════════════════════════

def test_the_consensus_gap_is_k_or_k_plus_one():
    assert vf.evaluate_consensus_sample(140, 140 - K, K) == (True, "", True)
    assert vf.evaluate_consensus_sample(140, 140 - K - 1, K) == (True, "", False)


def test_a_consensus_gap_outside_the_band_is_a_tier_disagreement():
    """`:122` — the snapshot is ATOMIC (one RPC, both tiers), so anything outside {K, K+1} is a
    real disagreement rather than the skew the eth cross-check tolerates."""
    ok, msg, exact = vf.evaluate_consensus_sample(140, 130, K)
    assert not ok and not exact and "tiers disagree" in msg and "cgap=10" in msg


def test_an_incomplete_consensus_reading_is_not_a_gap_of_zero():
    """`:120` — a `null` field must not be coerced. The arithmetic over a field the node did not
    answer looks exactly like a real reading and would land inside the band about as often as
    outside it."""
    for fin, res in ((None, 137), (140, None), ("null", 137), (140, "null")):
        ok, msg, exact = vf.evaluate_consensus_sample(fin, res, K)
        assert not ok and not exact and "incomplete" in msg


def test_the_result_tier_must_reach_exactly_k_at_least_once():
    """`:126` — durably K+1 is a derive pipeline permanently one block behind."""
    assert vf.evaluate_consensus_exact(True, K)[0]
    ok, msg = vf.evaluate_consensus_exact(False, K)
    assert not ok and "durably a block behind" in msg


@pytest.mark.parametrize("eth,cons,ok", [(137, 137, True), (137, 138, True), (137, 139, False)])
def test_the_two_tiers_may_skew_by_one_block_only(eth, cons, ok):
    """`:129` — the eth read and the consensus read are two RPCs apart."""
    assert vf.evaluate_tier_skew(eth, cons)[0] is ok


# ══ smoke-deferred: result-commitment integrity ════════════════════════════

GOOD_WIRE = "0x" + "ab" * (vf.WIRE_RESULT_OFFSET // 2) + "cd" * 32 + "ef" * 32
GOOD_HASH = "0x" + "cd" * 32


def test_the_committed_result_is_sliced_at_the_codec_offset():
    """`:140-159` — parent 32 + height 8 + proposal_view 8 + timestamp 8 + fee_recipient 20 +
    gas_limit 8 = byte 84 = hex 168, and `result` is the 32 bytes there."""
    ok, msg, committed = vf.evaluate_artifact_wire(GOOD_WIRE, 140)
    assert ok and committed == "cd" * 32
    assert vf.evaluate_result_commitment(committed, GOOD_HASH, 140, 137)[0]


def test_the_result_offset_is_the_sum_of_the_fields_before_it():
    """The offset is DERIVED, not written twice. Both directions: change a width in
    WIRE_HEADER_FIELDS without the literals here and this fails; change the literals without the
    list and it fails too. That pair is what a bare `WIRE_RESULT_OFFSET = 152` could not do — it
    survived `proposal_view` being inserted into the codec and sliced `gas_limit` instead."""
    widths = dict(vf.WIRE_HEADER_FIELDS)
    before_result = ("parent", "height", "proposal_view", "timestamp", "fee_recipient",
                     "gas_limit")
    assert vf.WIRE_RESULT_OFFSET == 2 * sum(widths[f] for f in before_result) == 168
    assert vf.WIRE_RESULT_LEN == 2 * widths["result"] == 64
    #: the guard falls out of the same list — the fixed header must be present in full
    assert vf.WIRE_MIN_LEN == vf.WIRE_RESULT_OFFSET + vf.WIRE_RESULT_LEN == 232
    assert vf.wire_hex_offset("parent") == 0 and vf.wire_hex_offset("height") == 64


def test_the_field_list_matches_the_rust_codec():
    """The other direction, across trees: WIRE_HEADER_FIELDS must name the same fields, in the
    same order, that `OrderBlock::write` emits before `result`. THE bug this file now pins — the
    product grew a field and the reader did not follow — is a test failure here, not a live
    `make smoke-fault` run reporting a fake safety violation."""
    src = (pathlib.Path(__file__).resolve().parents[4]
           / "crates/dpos/consensus/src/order_block.rs")
    if not src.exists():
        pytest.skip(f"consensus crate not in this tree ({src})")
    body = src.read_text().split("fn write(&self, buf: &mut impl BufMut)", 1)
    assert len(body) == 2, "OrderBlock::write not found — signature changed?"
    emitted = []
    for line in body[1].splitlines():
        m = re.match(r"^\s*(?:self\.(\w+)\.write\(buf\)"
                     r"|buf\.put_slice\(self\.(\w+)\.as_slice\(\)\));\s*$", line)
        if not m:
            continue
        emitted.append(m.group(1) or m.group(2))
        if emitted[-1] == "result":
            break
    assert emitted == [name for name, _ in vf.WIRE_HEADER_FIELDS]


def test_a_missing_artifact_is_a_failure_not_an_empty_comparison():
    """`:139` — an absent artifact slices to "" and would compare equal to another "". The
    height it was asked for is in the message, because "no artifact at N+K" and "the artifact is
    wrong" are different problems."""
    for wire in ("", None, "null", "0xnull"[:2] + "null"):
        ok, msg, committed = vf.evaluate_artifact_wire(wire, 140, artifact_raw="{}")
        assert not ok and committed == "" and "no ordering artifact at 140" in msg


def test_a_short_wire_fails_as_codec_drift_not_as_a_bad_chain():
    """`:158` — without the length guard the slice lands in whatever field follows, and a 64-hex
    run of some other field is still 64 hex chars: the comparison would fail and blame the
    chain for what is a codec change."""
    ok, msg, _ = vf.evaluate_artifact_wire("0x" + "ab" * 100, 140)
    assert not ok and "codec layout changed" in msg and "200 hex chars" in msg


def test_a_result_that_does_not_match_the_derived_hash_fails():
    """`:161-170` — THE check that ties the consensus artifact to execution. Every other deferred
    check is a height relationship, and heights agree happily on two different chains."""
    ok, msg = vf.evaluate_result_commitment("cd" * 32, "0x" + "99" * 32, 140, 137)
    assert not ok and "result commitment mismatch at h=140" in msg
    assert "LAYOUT CHANGED" not in msg  # the hash is nowhere in the wire: a real mismatch


def test_a_shifted_slice_is_reported_as_layout_drift_not_as_a_bad_chain():
    """The check the length guard cannot be. A field inserted BEFORE `result` lengthens the wire,
    so `>= WIRE_MIN_LEN` still passes and the slice lands on the neighbour — which is exactly how
    `proposal_view` produced `result commitment mismatch at h=88` against a correct chain. Here
    the hash is present at another offset, so the verdict must name the codec, not the chain."""
    shifted = "0x" + "ab" * (vf.WIRE_RESULT_OFFSET // 2 + 8) + "cd" * 32 + "ef" * 32
    ok, msg, committed = vf.evaluate_artifact_wire(shifted, 140)
    assert ok and committed != "cd" * 32  # the length guard is happy; the slice is a neighbour
    ok, msg = vf.evaluate_result_commitment(committed, GOOD_HASH, 140, 137, wire=shifted)
    assert not ok and "LAYOUT CHANGED" in msg
    assert f"hex offset {vf.WIRE_RESULT_OFFSET + 16}" in msg


def test_the_commitment_compare_is_case_insensitive():
    """bash lowercases both sides with `${var,,}`; `cast` and the wire disagree on case."""
    assert vf.evaluate_result_commitment("CD" * 32, GOOD_HASH, 140, 137)[0]


# ══ smoke-deferred: the throttled validator ════════════════════════════════

def test_the_chain_must_keep_finalizing_under_one_slowed_el():
    """`:186` — BFT f=1 with a SLOW node rather than a dead one: the victim's verify gate times
    out, the leader nullifies its view, the remaining quorum carries the chain."""
    assert vf.evaluate_throttle_liveness(100, 100 + vf.THROTTLE_MIN_GROWTH)[0]
    ok, msg = vf.evaluate_throttle_liveness(100, 105)
    assert not ok and "chain stalled under one slowed EL" in msg and "100 → 105" in msg


def test_the_victim_must_reach_the_tip_observed_at_unthrottle_time():
    """`:194` — anchored on `during`, not on "it answers RPC again". A victim serving from a
    height BEFORE the throttle is the stuck catch-up this half exists to catch."""
    assert vf.victim_rejoined(hex(150), 140)
    assert vf.victim_rejoined(hex(140), 140)
    assert not vf.victim_rejoined(hex(139), 140)


def test_an_unreachable_victim_does_not_read_as_rejoined():
    """Sentinel discipline: `"null"` must not become 0 and then satisfy a `>=` against a small
    target — nor become a truthy string that passes a laxer test."""
    assert not vf.victim_rejoined("null", 140)
    assert not vf.victim_rejoined("", 140)
    assert not vf.victim_rejoined(None, 140)


# ══ smoke-peers ════════════════════════════════════════════════════════════

METRICS = "\n".join([
    'outer_engine_buffered_peer_total{sequencer="aa"} 3',
    'outer_engine_buffered_peer_total{sequencer="bb"} 1',
    'outer_engine_buffered_peer_total{sequencer="aa"} 4',
    'some_other_total{sequencer="cc"} 9',
])


def test_the_committee_size_counts_the_addresses_cast_printed():
    """`:222` — `cast` renders an `address[]` as `[0x.., 0x..]`."""
    out = "[0x" + "11" * 20 + ", 0x" + "22" * 20 + ", 0x" + "33" * 20 + "]"
    assert vf.committee_size(out) == 3
    assert vf.committee_size("") == 0
    assert vf.committee_size("[]") == 0


def test_connected_peers_are_deduplicated_by_sequencer_label():
    """`:230-234` — the family renders once per peer per metric; the question is how many PEERS.
    Counting lines instead of distinct labels would inflate the answer above the committee size
    and fail a healthy node."""
    assert vf.connected_count(METRICS) == 2
    assert vf.connected_count("") == 0


def test_the_connected_count_is_exact_in_both_directions():
    """`:247` — tracked peer set == on-chain committee. Too FEW is an under-connected committee;
    too MANY is a discovery leak. A `>=` would see neither."""
    assert vf.evaluate_connected(3, 3)[0]
    assert not vf.evaluate_connected(2, 3)[0]
    ok, msg = vf.evaluate_connected(4, 3)
    assert not ok and "connected=4 != 3" in msg


def test_a_spoke_with_no_reth_peer_fails():
    """`:255` — the `dpos_rejoin_el_sync_devp2p` regression guard. When the `--dpos` override
    drops reth's `--trusted-peers`, the breakage only surfaces much later as a node that cannot
    catch up."""
    assert vf.evaluate_reth_peers(1, "validator-1")[0]
    ok, msg = vf.evaluate_reth_peers(0, "validator-1")
    assert not ok and "net_peerCount=0" in msg and "peering not wired" in msg


def test_the_reconnect_needs_all_three_including_chain_progress():
    """`:265` — the chain-advance leg is what makes this a REJOIN test: both peer planes can
    reconnect perfectly around a node that contributes nothing."""
    assert vf.peers_reconnected(3, 3, 1, 101, 100)
    assert not vf.peers_reconnected(2, 3, 1, 101, 100)      # commonware plane short
    assert not vf.peers_reconnected(3, 3, 0, 101, 100)      # devp2p plane down
    assert not vf.peers_reconnected(3, 3, 1, 100, 100)      # chain did not advance


# ══ smoke-vrf-fault / smoke-vrf-dkg-liveness: the gap compare ══════════════

def _rows(over=None):
    """Three healthy gap rows, with named heights replaced by the victim's bad reading."""
    over = over or {}
    return [(n, over.get(n, f"0x{n:064x}"), f"0x{n:064x}") for n in (10, 11, 12)]


def test_matching_gap_mixhashes_hold():
    assert vf.evaluate_gap_mixhashes(_rows(), "validator-3", "why")[0]


def test_a_divergent_gap_block_fails_and_names_both_values():
    """`:343` — the restarted node fell to the `order.digest()` fallback or forked. The message
    carries both sides because "they differ" is not actionable on its own."""
    ok, msg = vf.evaluate_gap_mixhashes(_rows({11: "0x" + "ff" * 32}), "validator-3", "B4 —")
    assert not ok and "B4 —" in msg and "11: validator-3=0x" + "ff" * 32 in msg


@pytest.mark.parametrize("bad", ["null", "", None])
def test_an_unreadable_gap_block_is_a_miss_not_a_skip(bad):
    """`:342` — the difference between "the victim caught up and derived the same seed" and
    "the victim never got the block, so nothing was compared". Skipping the second would report
    a node that recovered NOTHING as a node that recovered everything."""
    ok, msg = vf.evaluate_gap_mixhashes(_rows({12: bad}), "validator-3", "why")
    assert not ok and "12=missing-on-validator-3" in msg


# ══ smoke-vrf-dkg-live-heal ════════════════════════════════════════════════

def test_the_DEAL_window_opens_at_epoch_1_and_not_at_the_seal_deadline():
    """THE BUG THAT MADE THIS CASE GREEN FOR THE WRONG REASON, pinned in both directions.

    `on_height` calls `maybe_start(now + 1)` every tick, so committee[2]'s ceremony opens the
    instant the actor's clock enters epoch 1 and the whole of epoch 1 is its deal phase;
    `DKG_MARGIN_BLOCKS` is only where that phase CLOSES. The clock is `finalized + K`, so the
    finalized-height edge is `epoch_start(1) - K`. Under the case's tuned 64/128 geometry that is
    189 — not the 236 the seal deadline would give, a 47-block band inside which a victim is
    stopped having already acked every dealing."""
    act, interval = 128, 64
    assert vf.dkg_deal_window_open(act, interval) == 189
    assert vf.epoch_start_1(act, interval) == 192
    assert vf.DKG_CLOCK_LEAD == vf.RESULT_LAG_K == 3
    seal_deadline = verdicts.beacon_active_epoch_start(act, interval) - vf.DKG_MARGIN_BLOCKS
    assert seal_deadline == 236
    assert vf.dkg_deal_window_open(act, interval) < seal_deadline


def test_the_victim_must_be_stopped_before_the_deal_window_opens():
    """The TIMING is the assertion, and the case cannot wait for the next opportunity — there is
    none. committee[2]'s ceremony is the deterministic bootstrap; it happens once per stack."""
    assert vf.evaluate_window_open(145, 189)[0]
    ok, msg = vf.evaluate_window_open(189, 189)
    assert not ok and "already at/past the epoch-2 DKG DEAL window (189)" in msg
    assert "reveal fallback" in msg
    assert not vf.evaluate_window_open(220, 189)[0]


BOOT = f"INFO {vf.ACTOR_STARTED_LINE} epocher=(test)"
FRESH = f"INFO {vf.CEREMONY_STARTED_LINE} epoch=2"


def test_a_fresh_ceremony_after_the_restart_is_the_setup_witness():
    """`start_fresh` is the ONLY emitter of `ceremony started`, and `JournalLoad::NoFile` is the
    only arm that reaches it — so the line on the restarted process says the victim came back
    holding no epoch-2 journal, i.e. it received no dealing and acked none, i.e. its points can
    only be in the dealers' logs as public reveals."""
    assert vf.started_fresh_after_restart(f"{BOOT}\n{FRESH}") == FRESH
    assert vf.started_fresh_after_restart(BOOT) == ""


def test_a_ceremony_start_from_BEFORE_the_restart_does_NOT_count():
    """`docker compose logs` returns the whole container log, pre-stop lines included. A start
    from the process that was stopped means the OPPOSITE of what this witnesses — that the victim
    WAS present for the deal phase — so the slice at the last boot marker is load-bearing."""
    assert vf.started_fresh_after_restart(f"{FRESH}\n{BOOT}") == ""
    # Two boots (a restart after a crash) still slice at the LAST one.
    assert vf.started_fresh_after_restart(f"{BOOT}\n{FRESH}\n{BOOT}") == ""


def test_the_fresh_ceremony_grep_is_epoch_anchored():
    assert vf.started_fresh_after_restart(
        f"{BOOT}\nINFO {vf.CEREMONY_STARTED_LINE} epoch=20") == ""


def test_the_setup_gate_says_what_a_missing_fresh_start_means():
    assert vf.evaluate_started_fresh(FRESH, "validator-3")[0]
    ok, msg = vf.evaluate_started_fresh("", "validator-3")
    assert not ok and "came back holding a journal" in msg and "ACKED" in msg


def test_either_road_to_the_share_satisfies_the_recovery_gate():
    """Both were observed live on this case, on the same geometry, minutes apart: the live
    ceremony's finalize-over-pinned when it beats the past-boundary sweep, the demote-heal's
    scoped recompute when it does not. Asserting one — and, worse, the ABSENCE of the other —
    made a legitimate outcome red."""
    for marker, road in vf.SHARE_ROADS:
        got = vf.share_road(f"INFO {marker} epoch=2 height=257")
        assert got is not None and got[1] == road
        assert vf.evaluate_share_acquired(got, "validator-3")[0]
    assert vf.share_road("INFO nothing relevant epoch=2") is None
    ok, msg = vf.evaluate_share_acquired(None, "validator-3")
    assert not ok and "did not recover its epoch-2 share" in msg
    assert vf.HEAL_START_LINE in msg


def test_the_two_roads_are_distinct_lines_and_the_share_line_is_no_longer_forbidden():
    """The earlier version asserted `SHARE_LINE` must be ABSENT, on the premise that a member
    absent through its own window can never reach the ceremony-finalize arm. It can: it restarts
    the ceremony from `NoFile` while catching up and finalizes it over the AGREED pinned set."""
    markers = [m for m, _ in vf.SHARE_ROADS]
    assert vf.SHARE_LINE in markers and vf.HEAL_LINE in markers
    assert len(set(markers)) == 2


def test_the_pin_and_promote_greps_use_the_Debug_epoch_spelling():
    """`EpochSchemeProvider::register` and `reconcile_roles` both take `epoch: Epoch` and render
    it through `?epoch`, so the field is `epoch=Epoch(2)`. A grep anchored on the bare number
    matches NOTHING — the same trap `verdicts_rotation.SHARE_GATE_EPOCH_FMT` records."""
    pin = f"INFO {vf.PIN_LINE} " + vf.PIN_EPOCH_FMT.format(2)
    promote = f"INFO {vf.PROMOTE_LINE} " + vf.PIN_EPOCH_FMT.format(2)
    assert vf.pin_upgrade_lines(pin, 2) == [pin]
    assert vf.promote_lines(promote, 2) == [promote]
    assert vf.pin_upgrade_lines(f"INFO {vf.PIN_LINE} epoch=2", 2) == []
    assert vf.promote_lines(f"INFO {vf.PROMOTE_LINE} epoch=2", 2) == []
    assert vf.pin_upgrade_lines(f"INFO {vf.PIN_LINE} " + vf.PIN_EPOCH_FMT.format(21), 2) == []
    assert vf.promote_lines("INFO something else " + vf.PIN_EPOCH_FMT.format(2), 2) == []


def test_the_epoch_scheme_must_leave_vote_only_admission():
    """A separate consequence from the share, and not implied by it: a member can hold the share
    and still verify certificates seed-blind if nothing re-registers the scheme with the pin."""
    assert vf.evaluate_pin_upgraded(["x"], "validator-3")[0]
    ok, msg = vf.evaluate_pin_upgraded([], "validator-3")
    assert not ok and "still UNPINNED" in msg


def test_the_recovered_member_must_actually_be_SEATED():
    """A share that lands with no edge to wake is a share nobody votes with — and the production
    leg would then fail for a reason that is not about the key."""
    assert vf.evaluate_promoted(["x"], "validator-3")[0]
    ok, msg = vf.evaluate_promoted([], "validator-3")
    assert not ok and "never seated as a signer" in msg


def test_the_artifact_pull_counter_must_have_MOVED():
    assert vf.evaluate_artifact_pull_ok("1", "validator-3")[0]
    assert vf.evaluate_artifact_pull_ok("2.0", "validator-3")[0]
    ok, msg = vf.evaluate_artifact_pull_ok("0", "validator-3")
    assert not ok and "never obtained the LIVE epoch's agreed artifact" in msg


def test_an_UNREAD_pull_counter_is_not_a_zero_and_not_a_pass():
    """`node_metric` answers "" for an absent family AND for a dead scrape, and this counter lives
    on the devnet-only commonware endpoint. Coercing "" to 0 would make the one assertion that
    names the fix satisfiable by an endpoint nobody served."""
    for raw in ("", "   ", None, "n/a"):
        ok, msg = vf.evaluate_artifact_pull_ok(raw, "validator-3")
        assert not ok, raw
        assert ("could not be read off validator-3" in msg) or ("not a number" in msg)


def test_the_pull_counter_sample_name_carries_the_DOUBLED_total_suffix():
    """`prometheus-client` appends `_total` to a counter sample whatever the registered name ends
    in, so a counter registered `X_total` renders `X_total_total`. `node_metric`'s matcher is
    ANCHORED, so handing it the registered name returns "" — indistinguishable from a counter that
    never moved, i.e. a leg that can never pass. Pinned against a scrape captured live."""
    assert vf.ARTIFACT_PULL_OK_FAMILY == "dpos_dkg_artifact_pull_ok_total"
    assert vf.ARTIFACT_PULL_OK_SAMPLE == vf.ARTIFACT_PULL_OK_FAMILY + "_total"
    from dpos_harness.core import nodes as _n
    scrape = "dpos_dkg_artifact_pull_ok_total_total 1\n"
    assert _n.gauge_val(scrape, vf.ARTIFACT_PULL_OK_SAMPLE) == "1"
    assert _n.gauge_val(scrape, vf.ARTIFACT_PULL_OK_FAMILY) == ""


def test_the_seating_must_leave_room_for_a_leader_slot():
    """A member seated with four blocks of its epoch left can be perfectly recovered and still
    produce nothing. That is a fact about the schedule, and the case has to say which of the two
    it is rather than report a lottery loss as a broken recovery."""
    assert vf.evaluate_heal_left_room(275, 320, "validator-3")[0]
    assert vf.evaluate_heal_left_room(320 - vf.MIN_POST_HEAL_BLOCKS, 320, "validator-3")[0]
    ok, msg = vf.evaluate_heal_left_room(312, 320, "validator-3")
    assert not ok and "only 8 blocks of epoch 2 left" in msg
    # Calibrated against a live run: seated at 257 in a [256, 320) epoch, 10 of 64 blocks.
    assert vf.evaluate_heal_left_room(257, 320, "validator-3")[0]


def test_the_chain_must_still_be_finalizing_after_the_rejoin():
    """`:451` — the rejoin of a SHARELESS member must not wedge the seed quorum it is not in."""
    assert vf.evaluate_still_finalizing(100, 106, "validator-3")[0]
    ok, msg = vf.evaluate_still_finalizing(100, 100, "validator-3")
    assert not ok and "not finalizing after validator-3 rejoined (100 <= 100)" in msg
    assert not vf.evaluate_still_finalizing(100, 99, "validator-3")[0]


# ══ smoke-crash-survivor ═══════════════════════════════════════════════════

def test_the_chain_must_advance_while_one_validator_is_crashed():
    """`:478` — the PREMISE of the case. Without an EL gap there is nothing to backfill and the
    recovery half below would pass over a node that had nothing to recover."""
    assert vf.evaluate_chain_advanced_while_crashed(103, 100)[0]
    ok, msg = vf.evaluate_chain_advanced_while_crashed(102, 100)
    assert not ok and "chain stalled with 1/4 crashed" in msg and "finalized=102, pre=100" in msg


def test_realignment_still_catches_a_fork_at_the_same_height():
    """`:501` — a victim at the same height on a DIFFERENT block is the fork this catches, and
    dropping tip-vs-tip identity did not drop it: byte-identity no longer holds, so the producer
    read fires and the producer's block at 100 is not the victim's block."""
    assert vf.crash_survivor_realigned("0x64|0x100", "0x64|0x100", 90, _chain())
    assert not vf.crash_survivor_realigned("0x64|0x100", "0x64|0xDEAD", 90, _chain())


def test_a_victim_still_backfilling_its_gap_is_realigned_once_it_is_on_the_chain():
    """THE FIRST REGRESSION. The case SIGKILLs a validator, deliberately builds an EL gap,
    restarts it and waits ten minutes — so trailing the hub by a little is the normal shape of
    the recovery being asserted, not a fault. Requiring the two tips to coincide made the 600s
    budget a lottery on a chain producing a block a second. Ragged-but-above-the-floor passes."""
    assert vf.crash_survivor_realigned("0x64|0x100", "0x62|0x98", 90, _chain())
    assert vf.crash_survivor_realigned("0x64|0x100", "0x5a|0x90", 80, _chain())


def test_a_victim_forked_at_its_own_height_still_fails():
    """The half the compare existed for, at a RAGGED height: the victim is three behind AND on a
    different block there, so the producer's block at the victim's own height is not its block.
    A height-only compare — the naive way to allow ragged heights — would call it recovered."""
    assert not vf.crash_survivor_realigned("0x64|0x100", "0x61|0xDEAD", 90, _chain())


def test_a_victim_ahead_of_a_producer_that_does_not_hold_that_block_fails():
    """`blockhash_at` renders a block the producer does not hold as `"null"`, which never matches
    a real hash. That is a retry, not an agreement."""
    assert not vf.crash_survivor_realigned("0x64|0x100", "0x7d1|0x2001", 90, _chain())


def test_a_victim_that_never_comes_back_does_not_read_as_realigned():
    """The wedge the case was written for: `connected_peers=0`, no blocks, and an RPC answering
    nothing. Two `"null|null"` readings are byte-identical, so the head guard is the only thing
    standing between that and a green run — and it has to survive the move to same-height
    identity, where an unreachable reading would otherwise just be another ragged one."""
    assert not vf.crash_survivor_realigned("null|null", "null|null", 90, _chain())
    assert not vf.crash_survivor_realigned("0x64|0x100", "null|null", 90, _chain())
    assert not vf.crash_survivor_realigned("", "", 90, _chain())


def test_a_victim_that_never_backfilled_its_gap_is_NOT_realigned():
    """THE SECOND REGRESSION — the one that made this verdict VACUOUS, from a live run:

        validator-3 recovered from crash and realigned at 0x41(=65) … (v0=0x50(=80))

    Fifteen blocks behind, on its own persisted tail, the deliberately-built EL gap never
    backfilled — and it PASSED, because same-height identity alone is satisfied by any node on
    the right chain no matter how far back it is. The floor is the only thing that rejects it,
    and it must be the height the CHAIN reached while the victim was down (`head`), never the
    victim's own pre-stop height."""
    assert not vf.crash_survivor_realigned("0x50|0x80", "0x41|0x65", 77, _chain())
    # The mutation witness: the same reading, judged with no floor — i.e. exactly what the code
    # did before this fix. It passes, which is why the live run was green.
    assert vf.crash_survivor_realigned("0x50|0x80", "0x41|0x65", 0, _chain())


def test_the_realignment_floor_is_checked_on_the_producer_too():
    """`aligned_reading` floors EVERY reading, which is why the floor cannot be the producer's own
    live height: a producer below the floor is a stalled chain, not a recovery."""
    assert not vf.crash_survivor_realigned("0x41|0x65", "0x41|0x65", 77, _chain())


# ══ smoke-full-restart ═════════════════════════════════════════════════════

def test_an_exit_code_other_than_zero_fails_the_flush_assertion():
    """`:519` — THE flush assertion. A validator SIGKILLed at the 40s ceiling exits 137, comes
    back, resyncs from its peers and reconverges perfectly: the reconverge check below would
    bless a node that lost its persisted tail, and the exit code is the only witness."""
    assert vf.evaluate_flushed("validator-2", True)[0]
    ok, msg = vf.evaluate_flushed("validator-2", False)
    assert not ok and msg == "validator-2 did not exit cleanly (code 0) on shutdown"


def test_reconvergence_requires_the_chain_to_have_RESUMED_not_merely_come_back():
    """`:549` — `> pre`, NOT `>= pre`, and this is the half the same-height rewrite lost.

    Byte identity used to make `>= pre` mean more than it says: five EQUAL readings AT the
    persisted head can only be the instant of resumption. Ragged heights break that, and a fleet
    parked forever on its persisted tail — nobody having produced a single block — satisfied the
    old floor exactly. Both trees duly passed at EXACTLY it (`all 5 reconverged at 0x41
    (>= pre=65)`), which proves only that everyone came back on the same tail.

    The second assertion is the mutation witness: `pre - 1` is the old floor, and the wedged
    fleet passes iff it is put back."""
    assert not vf.full_restart_reconverged(["0x64|0xaa"] * 5, 100)   # AT pre — nothing resumed
    assert vf.full_restart_reconverged(["0x64|0xaa"] * 5, 99)        # the OLD floor, pre-1
    assert vf.full_restart_reconverged(["0x65|0xaa"] * 5, 100)       # one block past pre
    assert not vf.full_restart_reconverged(["0x63|0xaa"] * 5, 100)


def test_one_node_wedged_at_the_persisted_head_is_not_reconvergence():
    """THE REGRESSION, in its ragged form. Four validators resume and climb; the fifth comes back
    and never moves off the block it was stopped on. Under `>= pre` that reads as reconvergence —
    the wedged node clears the floor by sitting still and same-height identity confirms it is on
    the right chain, because it is. Under `> pre` it fails, which is the whole point."""
    wedged = ["0x6e|0x110", "0x6e|0x110", "0x64|0x100", "0x6e|0x110", "0x6e|0x110"]
    assert not vf.full_restart_reconverged(wedged, 100, _chain())
    # The mutation witness: the same set against the old `pre - 1` floor.
    assert vf.full_restart_reconverged(wedged, 99, _chain())


def test_a_fleet_that_came_back_ragged_still_reconverges():
    """THE REGRESSION. Five nodes are read serially — v0 by host curl, three by
    `docker compose exec`, the full-node by host curl again — while the chain resumes at a block
    a second, so the coinciding instant that made the old five-way byte-identity pass exists only
    at the moment they come back. One second later the tips are ragged for entirely healthy
    reasons and the old compare failed every poll to the 120s deadline."""
    readings = ["0x64|0x100", "0x64|0x100", "0x63|0x99", "0x64|0x100", "0x62|0x98"]
    assert vf.full_restart_reconverged(readings, 97, _chain())


def test_a_validator_that_came_back_on_a_different_chain_still_fails():
    """The half the compare existed for, at BOTH shapes of fork: same height as the producer on a
    different block, and a ragged height on a different block. Neither is reconvergence."""
    same_height = ["0x64|0x100", "0x64|0x100", "0x64|0xDEAD", "0x64|0x100", "0x64|0x100"]
    assert not vf.full_restart_reconverged(same_height, 99, _chain())
    own_height = ["0x64|0x100", "0x64|0x100", "0x63|0xDEAD", "0x64|0x100", "0x64|0x100"]
    assert not vf.full_restart_reconverged(own_height, 98, _chain())   # 98 clears the floor


def test_a_node_the_producer_cannot_confirm_is_not_reconvergence():
    """A validator reporting a height the producer does not hold reads `"null"`, which never
    matches a real hash — and one unreachable node in an otherwise ragged set must still be
    rejected on its head rather than skipped as "just another laggard"."""
    ahead = ["0x64|0x100", "0x64|0x100", "0x7d1|0x2001", "0x64|0x100", "0x64|0x100"]
    assert not vf.full_restart_reconverged(ahead, 99, _chain())
    one_down = ["0x64|0x100", "0x64|0x100", "null|null", "0x63|0x99", "0x64|0x100"]
    assert not vf.full_restart_reconverged(one_down, 97, _chain())


def test_the_floor_is_checked_on_every_node_not_only_the_producer():
    """Byte-identity used to make "the producer cleared the floor" equivalent to "everyone did".
    Ragged heights break that equivalence, and a validator still below the floor has not resumed
    — which is the entire assertion."""
    lagging = ["0x64|0x100", "0x64|0x100", "0x62|0x98", "0x64|0x100", "0x64|0x100"]
    assert not vf.full_restart_reconverged(lagging, 99, _chain())   # 98 !> pre=99
    assert vf.full_restart_reconverged(lagging, 97, _chain())       # > 97 for all five


def test_an_all_down_fleet_and_a_wiped_fleet_are_both_rejected():
    """Five unreachable nodes agree perfectly, and so do five nodes that came back at genesis.
    The second is the one this case would otherwise silently bless — every validator's data
    directory gone, the chain restarted from zero, and all five in perfect agreement about it."""
    assert not vf.full_restart_reconverged(["null|null"] * 5, 100)
    assert not vf.full_restart_reconverged(["0x0|0x0"] * 5, 0)
    assert not vf.full_restart_reconverged([], 100)


# ══ the full-restart production witness ════════════════════════════════════

def test_the_restart_witness_boundary_is_INCLUSIVE_and_needs_no_tolerance():
    """THE OFF-BY-ONE QUESTION, settled by the arithmetic rather than by what makes a run green.

    `restart_at = int(time.time())` is sampled BEFORE `compose_start`, so `restart_at = floor(T_s)`
    for a sample instant `T_s` that PRECEDES every container start. A produced block is sealed at
    some `T_b > T_s` and the proposer stamps it
    `max(wall_now.as_secs(), parent.timestamp + 1)` (`application.rs:991`). Both arms are
    `>= floor(T_b) >= floor(T_s) = restart_at`: the wall arm because flooring is monotone, the
    parent arm because it only ever raises the value.

    So a genuinely produced block CANNOT carry a stamp below `restart_at`, and `>=` is exactly
    right — a one-second tolerance would buy nothing and would start accepting a tail block sealed
    in the second before the restart."""
    assert vf.evaluate_produced_after_restart(1785766423, 1785766423)[0], "equal is produced"
    assert vf.evaluate_produced_after_restart(1785766424, 1785766423)[0]
    ok, msg = vf.evaluate_produced_after_restart(1785766422, 1785766423)
    assert not ok and "persisted tail" in msg


def test_an_unreadable_timestamp_lands_on_the_RED_side():
    """`block_field_at` answers `"null"` for an unreachable node as well as a missing field, and
    `hex_to_dec` maps that to 0 — older than any restart instant."""
    ok, msg = vf.evaluate_produced_after_restart(0, 1785766423)
    assert not ok and "timestamped 0" in msg


def test_the_restart_failure_path_reads_the_block_ABOVE_the_converged_height():
    """The deciding row. A stamp at or below `restart_at` is what a persisted tail looks like AND
    what the last pre-stop block looks like on a fleet that resumed afterwards; only whether the
    chain kept climbing tells them apart."""
    import inspect
    from dpos_harness.cases.smoke import asserts_fault
    src = inspect.getsource(asserts_fault._dump_restart_window)
    assert "head_dec + 1" in src and "head_dec - 1" in src
    assert "KEPT" in src and "<none>" in src
    body = inspect.getsource(asserts_fault.assert_full_restart)
    assert "_dump_restart_window(ctx, pre, head_dec, restart_at)" in body
