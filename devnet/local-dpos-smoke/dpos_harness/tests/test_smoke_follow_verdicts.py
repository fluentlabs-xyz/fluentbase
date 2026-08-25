"""`verdicts_follow.py` — every decision of the three follower cases, driven BOTH ways.

This file carries more weight than its two siblings, because four of the decisions it covers are
NEGATIVE — they pass when something does not happen — and two of those conclude from a fixed
observation window, i.e. they pass on a TIMEOUT:

    evaluate_tamper_no_progress    a tampered-cert follower must finalize NOTHING
    evaluate_bogus_rejected        an unverifiable trust root must be REFUSED
    evaluate_bogus_no_progress     …and nothing may be finalized while refusing
    evaluate_no_isolated_warning   the tx-route monitor must stay SILENT on a healthy uplink

A green live run proves none of them. A healthy chain satisfies a negative assertion by doing
nothing, which is byte-identical to the assertion being absent, inverted, or pointed at the wrong
node — and the tamper case cannot even be given a failing chain to look at without a Byzantine
build. So each of the four has a test below in which THE FORBIDDEN THING HAPPENS, and those four
tests are the only place those branches ever execute.
"""

from __future__ import annotations

import pathlib

import pytest

from dpos_harness.cases.smoke import verdicts_follow as vf


# ══ smoke-cert-follow ══════════════════════════════════════════════════════

def test_align_floor_is_a_full_epoch_away():
    """`case-cert-follow.sh:38` — `anchor + EPOCH_INTERVAL`, so the follower must cross a
    committee handoff rather than merely reproduce the block it already had."""
    assert vf.align_floor(100, 32) == 132
    assert vf.align_floor("100", "32") == 132


def test_mitm_ready_reads_the_listening_line():
    assert vf.mitm_ready("cert-mitm: listening on 0.0.0.0:8546")
    assert not vf.mitm_ready("Collecting websockets\nERROR: no network")
    assert not vf.mitm_ready("")
    assert not vf.mitm_ready(None)


def test_v0_advanced_gate_passes_on_a_live_producer():
    assert vf.evaluate_v0_advanced(100, 140) == (True, "")


def test_v0_advanced_gate_fails_on_a_stalled_producer():
    """THE CONTROL for the tamper negative. A chain-wide stall makes the follower's stillness
    meaningless, and without this the case would report that certificate verification is
    load-bearing on the strength of a dead devnet."""
    ok, msg = vf.evaluate_v0_advanced(140, 140)
    assert not ok and "v0 stalled during tamper phase" in msg
    assert not vf.evaluate_v0_advanced(140, 139)[0], "a REGRESSED producer is not advancement"


@pytest.mark.parametrize("head", ["null", "0x0"])
def test_tamper_no_progress_passes_on_the_two_zero_readings(head):
    """reth leaves `finalized` unset until something is finalized; a node that only ever
    cold-started reports genesis. Both are zero progress."""
    assert vf.evaluate_tamper_no_progress(head) == (True, "")


def test_tamper_no_progress_FAILS_when_the_follower_actually_advanced():
    """THE NEGATIVE, driven. A follower fed only byte-flipped certificates that nonetheless
    finalized block 0x140 means the driver accepted a forged certificate — the single loudest
    failure this whole case exists to produce, and the branch a live run never walks."""
    ok, msg = vf.evaluate_tamper_no_progress("0x140")
    assert not ok
    assert "0x140" in msg and "verification is NOT load-bearing" in msg


def test_tamper_no_progress_does_not_accept_an_empty_reading_as_zero():
    """"" is not one of the two zero readings. An empty answer is a read that failed, and reading
    it as "made no progress" would turn an unreachable RPC into a passing negative assertion —
    the sentinel discipline of §2.4 item 5, in the one place it decides a Byzantine claim."""
    assert not vf.evaluate_tamper_no_progress("")[0]
    assert not vf.evaluate_tamper_no_progress(None)[0]


@pytest.mark.parametrize("line", vf.TAMPER_REJECT_LINES)
def test_either_refusal_line_satisfies_the_tamper_gate(line):
    """THE STRING IS THE ASSERTION, and this one has been wrong twice. It first grepped for two
    lines that appear NOWHERE in `crates/`; the replacement, `cert-inlet: BLS verify FAILED`, is a
    real string on a real path but UNREACHABLE under this tamper — the flip lands in the trailing
    48-byte beacon seed slot, a compressed G1 point, so `read_seed_slot` fails DECODE and the
    certificate never reaches `CertInlet::ingest` (see `TAMPER_REJECT_LINES`).

    Both accepted lines are parametrised rather than spot-checked, because which one fires depends
    on whether the cold-start pull or the subscription stream sees the first bad certificate."""
    ok, _, matched = vf.evaluate_tamper_rejected(f"2026-08-03T00:00:00Z WARN error=x {line}")
    assert ok
    assert matched == line, "the verdict must report WHICH path refused, not just that one did"


def test_the_matched_line_reaches_the_transcript():
    """The case tears its stack down when it finishes, so the container log is gone before anyone
    can read it afterwards. If the OK line does not name the path that fired, the run leaves no
    record of whether the live subscription or the cold-start pull rejected the certificate."""
    import inspect
    from dpos_harness.cases.smoke import asserts_follow
    src = inspect.getsource(asserts_follow.assert_cert_follow)
    assert "matched = vf.evaluate_tamper_rejected(" in src
    assert "{matched!r}" in src


def test_the_tamper_gate_fails_when_the_follower_said_nothing():
    ok, msg, matched = vf.evaluate_tamper_rejected("cert applied")
    assert not ok and "never delivered" in msg and matched == ""
    for line in vf.TAMPER_REJECT_LINES:
        assert line in msg, "the failure must name every line it would have accepted"
    assert not vf.evaluate_tamper_rejected("")[0]


def test_the_verify_time_string_is_NOT_accepted_as_the_refusal():
    """THE REGRESSION THIS COST A LIVE RUN. `cert-inlet: BLS verify FAILED` is emitted by a real
    path on a real follower, so it reads as a plausible witness — but a certificate whose seed slot
    was corrupted never gets that far. Accepting it would make the gate green on a run where the
    follower never logged anything at all."""
    assert not vf.evaluate_tamper_rejected("WARN cert-inlet: BLS verify FAILED; skipping")[0]


def test_the_tamper_follower_must_be_UP_before_its_silence_counts_as_rejection():
    """The control the phase had no way to make. Zero finalized progress is also what a follower
    that never started reports, and `"null"` is a legitimate PASS on that read — so the liveness
    witness has to be a separate reading."""
    assert vf.evaluate_tamper_alive(True)[0]
    ok, msg = vf.evaluate_tamper_alive(False)
    assert not ok and "not up" in msg


def test_the_tamper_observation_window_is_the_budget_bash_used():
    """§2.4 item 3. The case concludes "zero progress" from 45 s of watching; shortening the
    window weakens the claim by exactly that much and nothing goes red. Pinned so the shortening
    has to be deliberate."""
    assert vf.TAMPER_OBSERVE_S == 45
    assert vf.MITM_UP_S == 90 and vf.MITM_POLL_S == 3
    assert vf.CF_ALIGN_S == 180 and vf.CF_GAP_ADVANCE_S == 120


def test_the_two_followers_are_not_the_same_node():
    """Phase 3 reads the TAMPER follower. Reading the honest one would sample a node that — having
    been stopped and restarted in phase 2 — also makes no progress for a moment: a negative
    assertion passing for entirely the wrong reason.

    SERVICE names, not host ports. The followers publish none: the harness reads them
    in-container, which is what let 38545/48545/58545 be withdrawn from the kernel's ephemeral
    range where any outbound connection could take them."""
    assert vf.CF_SERVICE != vf.TAMPER_SERVICE
    assert (vf.CF_SERVICE, vf.TAMPER_SERVICE) == ("cert-follower", "cert-follower-tamper")


# ══ smoke-cert-follow phase 4 (FLU-1167) ═══════════════════════════════════

KEYLOG = f"INFO {vf.CF_KEY_LINE} epoch=2 group_public=ab12"


def test_the_key_line_is_the_adoption_witness_in_both_directions():
    ok, msg, line = vf.evaluate_key_obtained(KEYLOG, vf.CF_SERVICE)
    assert ok and line == KEYLOG.strip()
    ok, msg, line = vf.evaluate_key_obtained("INFO nothing of the sort", vf.CF_SERVICE)
    assert not ok and "did not obtain PK_epoch" in msg and line == ""


def test_the_two_adoption_witnesses_live_on_two_different_registries():
    """The counter and the log line count the same adoption, and the counter is the SECOND
    witness — it is read beside the log line, never instead of it, because a `--cert-follow` node
    serves the commonware registry only under a devnet build plus a `--dpos.metrics-port` the
    compose overlay has to pass. Pinned so nobody "upgrades" the witness to the counter and
    quietly makes FLU-1167's proof contingent on two devnet knobs.

    The families are pinned too: `dpos_cert_vote_only_admissions_total` is a `metrics::counter!`
    on RETH's recorder and `dpos_follower_artifact_*` are `BeaconMetrics` on the commonware one.
    Reading either from the other endpoint returns nothing forever, and a phase that did would be
    green on an unread family."""
    assert vf.CF_VOTE_ONLY_FAMILY == "dpos_cert_vote_only_admissions_total"
    assert vf.CF_ADOPTED_FAMILY == "dpos_follower_artifact_adopted_total"
    assert vf.CF_MISS_FAMILY == "dpos_follower_artifact_miss_total"
    assert "follower_artifact_adopted" not in vf.CF_KEY_LINE


def test_the_adoption_counter_reads_the_DOUBLED_sample_name():
    """A `prometheus-client` counter registered `X_total` renders its SAMPLE as `X_total_total`
    (`nodes.counter_sample`). A verdict that matched the registered name would find nothing on a
    live scrape and report a keyed follower as unadopted — so the doubling is asserted here
    against the exact text a follower's `:9100` produced live, not against a paraphrase."""
    live = ("dpos_follower_artifact_adopted_total_total 1\n"
            "dpos_follower_artifact_miss_total_total 0\n")
    ok, msg, pair = vf.evaluate_artifact_adopted_counter(live, vf.CF_SERVICE)
    assert ok and msg == ""
    assert "dpos_follower_artifact_adopted_total=1" in pair
    assert "dpos_follower_artifact_miss_total=0" in pair


def test_the_adoption_counter_fails_UNREAD_and_ZERO_apart():
    """The two ways it can not-say-yes are different things and get different messages: an empty
    scrape is a measurement that never happened, a zero on an answering endpoint is the counter
    contradicting the log line. Collapsing them would let a missing `--dpos.metrics-port` read as
    a product bug, and a real disagreement read as a config gap."""
    ok, msg, pair = vf.evaluate_artifact_adopted_counter("", vf.CF_SERVICE)
    assert not ok and "did not answer" in msg and pair == ""
    ok, msg, _ = vf.evaluate_artifact_adopted_counter(
        "dpos_follower_artifact_adopted_total_total 0\n", vf.CF_SERVICE)
    assert not ok and "still 0" in msg
    # An endpoint that answered with OTHER families but none of the follower's: the beacon never
    # registered them, which is not the same as "adopted zero artifacts".
    ok, msg, _ = vf.evaluate_artifact_adopted_counter("runtime_tasks_running 3\n", vf.CF_SERVICE)
    assert not ok and "carries no" in msg


def test_vote_only_admissions_must_stop_once_the_key_lands():
    assert vf.evaluate_vote_only_flat("4", "4", True)[0]
    # An absent family on an endpoint that ANSWERED is a real zero on both samples.
    assert vf.evaluate_vote_only_flat("", "", True)[0]
    ok, msg = vf.evaluate_vote_only_flat("4", "5", True)
    assert not ok and "grew 4 -> 5" in msg


def test_an_UNREAD_metrics_endpoint_is_not_a_flat_counter():
    """THE TRAP THIS VERDICT IS SHAPED AROUND. "" means an absent family OR a dead scrape, and
    one is a pass while the other is a measurement that never happened. Without the third value
    the strongest reading in the phase would be green forever on a follower with no `--metrics`."""
    ok, msg = vf.evaluate_vote_only_flat("", "", False)
    assert not ok and "did not answer" in msg
    assert not vf.evaluate_vote_only_flat("4", "4", False)[0]


def test_the_key_grep_reads_the_WHOLE_log_and_the_count_grep_does_not():
    """Two greps, two depths, and each depth is the answer to a different question.

    `CF_KEY_LINE` is written ONCE in the follower's first seconds, and both followers now run for
    the better part of ten minutes before phase 4b reads them — a bounded tail scrolls it away and
    reports a healthy, keyed, block-deriving follower as one that never obtained the key. That
    happened on a live run. The refusal COUNT is a delta whose two ends are both recent, so it
    stays bounded and keeps the read off a megabyte log."""
    assert vf.SEED_LOG_TAIL is None
    assert isinstance(vf.SEED_COUNT_TAIL, int) and vf.SEED_COUNT_TAIL > 0


def test_the_seed_follower_must_be_CAUGHT_UP_before_the_arming():
    """The gate that decides whether the phase measures anything. Holding the key says the
    follower CAN reject; being caught up says a rejection is the only thing that can stop it.
    Armed while back-filling, it rides certificates the proxy never touched."""
    assert vf.evaluate_seed_follower_caught_up("0x140|0xcc", 300)[0]
    ok, msg = vf.evaluate_seed_follower_caught_up(None, 300)
    assert not ok and "did not catch up with v0 past 300" in msg
    assert "BEFORE the proxy was armed" in msg


def test_the_caught_up_gate_is_separate_from_the_key_gate():
    """Two different failures with two different fixes: no key means the follower cannot reject
    at all (a product or delivery problem); a backlog means it can, but the window measured
    something else (a harness problem). Collapsing them would send the reader to the wrong one —
    which is exactly what happened before this gate existed."""
    assert not vf.evaluate_seed_follower_caught_up(None, 1)[0]
    assert not vf.evaluate_key_obtained("", vf.SEED_TAMPER_SERVICE)[0]
    a = vf.evaluate_seed_follower_caught_up(None, 1)[1]
    b = vf.evaluate_key_obtained("", vf.SEED_TAMPER_SERVICE)[1]
    assert a != b and "PK_epoch" in b and "catch up" in a


def test_the_seed_proxy_must_witness_its_own_tampering():
    """Both failures are distinguishable, and they call for different reading: never ARMED means
    the arm file did not reach the proxy; armed but nothing CLEARED means every certificate was
    already seedless or the trailing flag+slot offsets no longer match the wire format."""
    good = f"{vf.SEED_ARMED_LINE}\n{vf.SEED_CLEARED_LINE} (first): aabb -> 0000"
    assert vf.evaluate_seed_tamper_landed(good)[0]
    ok, msg = vf.evaluate_seed_tamper_landed("cert-mitm: listening")
    assert not ok and "never logged" in msg and vf.SEED_ARMED_LINE in msg
    ok, msg = vf.evaluate_seed_tamper_landed(vf.SEED_ARMED_LINE)
    assert not ok and "cleared NO seed slot" in msg


def test_the_pinned_follower_must_REFUSE_every_cleared_certificate():
    """The negative, measured on the refusals. A growing count proves two things the follower's
    HEIGHT proved neither of: certificates were still being delivered, and every one was refused."""
    assert vf.evaluate_seed_tamper_refused_every_cert(0, vf.MIN_SEED_REJECTS)[0]
    assert vf.evaluate_seed_tamper_refused_every_cert(12, 12 + vf.MIN_SEED_REJECTS)[0]
    ok, msg = vf.evaluate_seed_tamper_refused_every_cert(0, vf.MIN_SEED_REJECTS - 1)
    assert not ok and "the seed slot is not being checked" in msg
    ok, msg = vf.evaluate_seed_tamper_refused_every_cert(0, 0)
    assert not ok and "refused only 0 certificates" in msg


def test_the_refusal_count_replaced_a_HEIGHT_reading_that_measured_the_EL_transport():
    """WHY THE INSTRUMENT CHANGED, pinned so it cannot drift back. A follower fed a poisoned
    stream refuses every certificate, counts the refusals as upstream data faults, rotates, and
    its EL keeps syncing over devp2p from the validator in `--trusted-peers`; the steady-state
    re-jump then fast-forwards the anchor onto that EL tip. Live, with every certificate correctly
    refused and `dpos_cert_vote_only_admissions_total` flat, `finalized` still advanced 238 → 271.
    A height-delta verdict called that "the seed slot is not being checked"."""
    assert not hasattr(vf, "evaluate_seed_tamper_frozen")
    assert not hasattr(vf, "SEED_SLACK_BLOCKS")
    assert vf.seed_reject_count("\n".join([f"WARN {vf.SEED_REJECT_LINE} h=1"] * 7)) == 7
    assert vf.seed_reject_count("nothing here\nnor here") == 0
    assert vf.seed_reject_count("") == 0


def test_the_refusals_must_come_from_a_PINNED_scheme_and_not_a_downgrade():
    """`verify_certificate` early-returns `true` after the quorum arm when the scheme carries no
    `cert_seed_pin`, so an UNPINNED follower ACCEPTS a cleared slot and ticks the vote-only
    counter. Flat is the statement "nothing was admitted with the seed unchecked", which is what
    the refusal count has to be paired with."""
    assert vf.evaluate_seed_vote_only_flat("1", "1", True)[0]
    assert vf.evaluate_seed_vote_only_flat("", "", True)[0]
    ok, msg = vf.evaluate_seed_vote_only_flat("1", "9", True)
    assert not ok and "grew 1 -> 9" in msg and "lost the pin" in msg
    ok, msg = vf.evaluate_seed_vote_only_flat("", "", False)
    assert not ok and "did not answer" in msg


def test_the_seed_refusal_line_is_the_VERIFY_one_and_not_phase_3s_decode_lines():
    """The two phases cannot share a constant. A flipped nibble breaks the G1 point so the
    certificate fails DECODE and never reaches `CertInlet::ingest`; a cleared slot decodes
    perfectly and fails at VERIFY, which is the only arm that can prove the follower is using
    `PK_epoch`."""
    assert vf.SEED_REJECT_LINE not in vf.TAMPER_REJECT_LINES
    assert vf.seed_reject_count(f"WARN {vf.SEED_REJECT_LINE}; skipping") == 1
    # Phase 3's lines are a DIFFERENT event and must not be counted as seed refusals — a decode
    # failure says nothing about whether the seed slot was checked.
    for line in vf.TAMPER_REJECT_LINES:
        assert vf.seed_reject_count(f"WARN {line}") == 0


def test_the_phase_4_services_are_their_own_nodes():
    """Four distinct services now. Reading phase 4's verdict off phase 3's follower — which
    refuses every certificate and so never calls `observe_cert` — would report "no key obtained"
    as a property of the product rather than of the topology."""
    names = {vf.CF_SERVICE, vf.TAMPER_SERVICE, vf.SEED_MITM_SERVICE, vf.SEED_TAMPER_SERVICE}
    assert len(names) == 4


def test_the_post_arm_observation_window_is_a_budget_and_not_a_settle_time():
    """THE WINDOW IS THE ASSERTION (§2.4 item 3): the phase concludes "it stopped" from the
    absence of movement across it, so shortening it weakens the claim by exactly that much and
    nothing goes red. Pinned at the value the case was written against."""
    assert vf.SEED_OBSERVE_S == 45
    assert vf.CF_VOTE_ONLY_WINDOW_S == 30


# ══ smoke-cert-cascade ═════════════════════════════════════════════════════

def test_l1_checkpoint_verified_needs_the_line():
    assert vf.evaluate_l1_checkpoint_verified("… L1 Rollup checkpoint verified batch=1 …") == \
        (True, "")


def test_l1_checkpoint_verified_fails_when_the_assert_never_ran():
    """A follower with a broken `--cert-follow.l1-rpc-url` still ALIGNS off the cert feed, so
    alignment alone would report a trust root that was never consulted."""
    ok, msg = vf.evaluate_l1_checkpoint_verified("cert applied at 130\ncert applied at 131")
    assert not ok and "the L1 checkpoint assert never ran" in msg


def test_bogus_rejected_by_the_logged_refusal():
    ok, _, witness = vf.evaluate_bogus_rejected("checkpoint hash NOT in the local chain")
    assert ok and witness == "refusal logged"


def test_a_DEAD_container_is_not_a_refusal():
    """THE `exited` WITNESS, DELETED. It read a container state as proof that a trust root
    worked, so a follower killed by a bad address, an OOM or an unresolvable L1 URL passed the
    check — and an exit code cannot separate any of those from a refusal, since all of them are
    eyre-1. Nothing is lost: the refusal is LOGGED before the exit
    (`bins/fluent/src/main.rs:407` then `:429`) and the string is contract-pinned
    (`crates/dpos/consensus/src/cold_start_jump.rs:894-897`)."""
    ok, msg, witness = vf.evaluate_bogus_rejected("")
    assert not ok and witness == "" and msg == vf.BOGUS_NOT_REFUSED


def test_bogus_rejected_FAILS_while_the_follower_is_happily_running():
    """THE NEGATIVE, driven. A running container with no refusal in its log is a follower that
    accepted a checkpoint hash existing in no block of the chain — the trust root failing OPEN."""
    ok, msg, witness = vf.evaluate_bogus_rejected("cert applied at 131")
    assert not ok and witness == ""
    assert msg == vf.BOGUS_NOT_REFUSED


def test_bogus_no_progress_passes_on_an_unset_finalized():
    assert vf.evaluate_bogus_no_progress("null|null", 100) == (True, "")


def test_bogus_no_progress_passes_at_or_below_the_anchor():
    """The follower shares the devnet's genesis, so a height at/below the pre-case anchor is
    state it could have had without following anything."""
    assert vf.evaluate_bogus_no_progress("0x64|0xaa", 100) == (True, "")


def test_bogus_no_progress_FAILS_when_it_refused_and_followed_anyway():
    """THE NEGATIVE, driven. Printing the refusal and then finalizing past the anchor is a
    fail-closed trust root that did not close, and the log grep alone cannot see it."""
    ok, msg = vf.evaluate_bogus_no_progress("0xc8|0xbb", 100)
    assert not ok and "made finalized progress" in msg and "0xc8" in msg


def test_contract_address_is_fail_loud_on_a_receipt_without_one():
    """A missing address would flow into `MOCK_ROLLUP_ADDR=None`, the compose file's `:-0x0…0`
    default would take over, and the case would measure a follower pointed at the zero address."""
    ok, _, addr = vf.contract_address({"contractAddress": "0x" + "11" * 20}, "MockRollup")
    assert ok and addr == "0x" + "11" * 20
    for bad in ({}, {"contractAddress": None}, {"contractAddress": "0x1234"}, None):
        ok, msg, addr = vf.contract_address(bad, "MockRollup")
        assert not ok and addr == "" and "no contractAddress" in msg


def test_receipt_block_is_fail_loud_and_decimal():
    ok, _, blk = vf.receipt_block({"blockNumber": "0x80"}, "setCheckpoint")
    assert ok and blk == 128
    ok, msg, blk = vf.receipt_block({}, "setCheckpoint")
    assert not ok and blk == 0 and "no blockNumber" in msg


def test_the_bogus_checkpoint_hash_is_not_a_plausible_block_hash():
    """It has to be a hash that exists nowhere in the chain — that is the whole experiment. Full
    32 bytes so the ABI encoder accepts it as a bytes32."""
    assert vf.BOGUS_CHECKPOINT_HASH.startswith("0xdeadbeef")
    assert len(vf.BOGUS_CHECKPOINT_HASH) == 66


# ══ smoke-tx-cascade ═══════════════════════════════════════════════════════

def test_enode_urls_are_rebuilt_against_the_fixed_compose_ips():
    """`admin_nodeInfo`'s embedded IP is unreliable inside docker, so only the pubkey comes from
    the node. A sentry enode carrying L3's address peers the two tiers backwards."""
    pk = "ab" * 64
    assert vf.sentry_enode(pk) == f"enode://{pk}@172.20.0.30:30303"
    assert vf.downstream_enode(pk) == f"enode://{pk}@172.20.0.31:30303"


def test_enode_pubkey_gate_accepts_only_128_hex():
    assert vf.evaluate_enode_pubkey("ab" * 64, "sentry") == (True, "")
    for bad in ("", None, "ab" * 63, "ab" * 65, "zz" * 64):
        ok, msg = vf.evaluate_enode_pubkey(bad, "sentry")
        assert not ok and "bad sentry enode pubkey" in msg


def test_l3_peer_gate_fails_on_an_isolated_node():
    """The HARD half. Zero peers means the tx uplink is absent, so every write-path assertion
    below would be measuring a transaction sitting in L3's own pool forever."""
    ok, msg, note = vf.evaluate_l3_peers(0)
    assert not ok and note == "" and "NO devp2p peer" in msg


def test_l3_peer_gate_passes_at_one_with_no_note():
    assert vf.evaluate_l3_peers(1) == (True, "", "")


def test_l3_peer_gate_passes_above_one_WITH_a_note():
    """The soft half, kept soft ON PURPOSE — bash prints a NOTE here and does not fail. Privacy
    is enforced structurally (`--trusted-only`, `--disable-discovery`, a one-entry trusted-peers
    list), not by this count; promoting the note to a failure would make the case flaky on a
    transient second connection without adding an assertion the topology does not give."""
    ok, msg, note = vf.evaluate_l3_peers(2)
    assert ok and msg == ""
    assert "expected 1 = sentry only" in note


def test_peer_count_parses_the_quoted_hex_and_refuses_garbage():
    assert vf.peer_count_from_rpc('"0x1"') == 1
    assert vf.peer_count_from_rpc("0xa") == 10
    for junk in ("", None, "error", '""'):
        assert vf.peer_count_from_rpc(junk) == 0, "an unreadable answer must not read as a peer"


def test_tx_mined_accepts_both_status_spellings_and_rejects_a_revert():
    assert vf.evaluate_tx_mined("0xaa", "0x1") == (True, "")
    assert vf.evaluate_tx_mined("0xaa", "1") == (True, "")
    ok, msg = vf.evaluate_tx_mined("0xaa", "0x0")
    assert not ok and "devp2p tx-gossip relay L3→L2→validator failed" in msg
    assert not vf.evaluate_tx_mined("0xaa", "")[0]


def test_l3_synced_receipt_names_the_round_trip_not_the_mining():
    ok, msg = vf.evaluate_l3_synced_receipt("0xaa", "")
    assert not ok and "L3 never synced the receipt" in msg


def test_l3_state_needs_both_halves():
    assert vf.evaluate_l3_state(vf.TXC_TRANSFER_WEI, vf.TXC_ALLOW) == (True, "")
    ok, msg = vf.evaluate_l3_state(vf.TXC_TRANSFER_WEI - 1, vf.TXC_ALLOW)
    assert not ok and "balance delta" in msg
    ok, msg = vf.evaluate_l3_state(vf.TXC_TRANSFER_WEI, 0)
    assert not ok and "EVM SSTORE not synced" in msg


def test_l3_state_transfer_is_the_0_05_ether_bash_sends():
    """`case-tx-cascade.sh:126` sends 0.05 ether and `:138` compares against the literal
    50000000000000000. A drift between the two makes the case fail on a correct chain."""
    assert vf.TXC_TRANSFER_WEI == 50_000_000_000_000_000
    assert vf.TXC_ALLOW == 4242


def test_the_follower_ingest_control_needs_the_FOLLOWERS_own_height():
    """The F8 verdict. `evaluate_v0_advanced` answers "was anything produced"; this answers "did
    THIS follower take any of it", which is the question a counter on that follower is defended by.
    A follower whose cert inlet died reads flat on both the counter and its own height while the
    producer runs on."""
    assert vf.evaluate_follower_ingested(100, 130, "cert-follower")[0]
    ok, msg = vf.evaluate_follower_ingested(130, 130, "cert-follower")
    assert not ok and "finalized nothing over the vote-only window (130\u2192130)" in msg
    assert vf.CF_VOTE_ONLY_FAMILY in msg
    # a follower going BACKWARDS is not ingesting either.
    assert not vf.evaluate_follower_ingested(130, 129, "cert-follower")[0]


def test_no_isolated_warning_passes_on_a_quiet_log():
    assert vf.evaluate_no_isolated_warning("tx-route ok peers=1\n") == (True, "")
    assert vf.evaluate_no_isolated_warning("") == (True, "")


def test_the_tx_route_monitor_is_still_ABSENT_from_the_product():
    """THE F6 PIN, and it asserts an ABSENCE on purpose.

    `evaluate_no_isolated_warning` greps for `tx-route ISOLATED`. Nothing under `crates/` emits
    it: `crates/node/src/tx_route.rs` — `spawn_tx_route_monitor`, `ISOLATION_GRACE`, the
    `fluent_tx_route_isolated` / `fluent_tx_relay_peers` gauges — was written, unit-tested and
    docker-validated on 2026-06-30 and is absent from the working tree and from every git ref
    (`DPOS_AUDIT.md` B9; doc §8.5.1 keeps the design as the spec to re-implement against). So the
    check cannot fail, and `smoke-tx-cascade`'s OK line used to read as though the tx-route path
    were being watched.

    Deleting the check was the wrong answer: this repo has a recorded incident of exactly this
    shape — an implementation lost in a squash while its test half survived — and the surviving
    half is the only thing in the tree that still names the missing subsystem. So it stays as a
    tripwire, and THIS test is what stops the tripwire from being silent for another two months:
    the day any of those symbols reappears under `crates/`, this goes red and the message says
    what to do with it.

    It is not a check on the product's correctness and it must never be read as one. It is a
    check that the HARNESS's claim and the product's reality still agree."""
    crates = pathlib.Path(__file__).resolve().parents[4] / "crates"
    if not crates.is_dir():
        pytest.skip(f"crates not in this tree ({crates})")
    found = {}
    for src in sorted(crates.rglob("*.rs")):
        try:
            text = src.read_text(encoding="utf-8", errors="replace")
        except OSError:                                     # pragma: no cover — unreadable file
            continue
        for sym in vf.TX_ROUTE_SYMBOLS:
            if sym in text:
                found.setdefault(sym, []).append(str(src))
    assert not found, (
        f"the tx-route monitor is BACK ({found}) — `evaluate_no_isolated_warning` is a tripwire "
        "written against its absence and must now be upgraded to a real assertion: read the "
        "`fluent_tx_relay_peers` gauge to prove the monitor is PRESENT, and only then assert the "
        "ISOLATED string is absent. A bare negative over a subsystem that exists is still "
        "satisfied by a monitor that never ran. Then close DPOS_AUDIT B9 and drop the doc §8.5.1 "
        "warning and the case's NOT-COVERED note.")


def test_no_isolated_warning_FAILS_when_the_monitor_cried_wolf():
    """THE NEGATIVE, driven. Everything before this point proved the route works, so a warning
    here is a false positive in a fail-loud path — worse than useless, because an operator who
    learns to ignore it will ignore the true one too."""
    ok, msg = vf.evaluate_no_isolated_warning("WARN tx-route ISOLATED: no peers\n")
    assert not ok and "false positive" in msg


def test_the_cascade_budgets_are_the_ones_bash_used():
    """Pinned for the same reason as the tamper window: the bogus-refusal poll passes only by
    OBSERVING for its full budget on the failing side, so a shortened budget turns a real
    fail-open into a "did not refuse" that reads like flakiness."""
    assert vf.CC_ALIGN_S == 240 and vf.CC_REJECT_S == 240 and vf.CC_REJECT_POLL_S == 3
    assert vf.CC_FINALIZE_S == 60
    assert vf.TXC_ALIGN_S == 200 and vf.TXC_FINALIZE_S == 120
    assert vf.TXC_RECEIPT_TRIES == 90 and vf.TXC_L3_RECEIPT_TRIES == 120
