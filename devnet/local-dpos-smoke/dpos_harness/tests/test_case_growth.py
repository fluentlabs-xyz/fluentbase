"""Pure-logic tests for the bug-B GROWTH case verdict layer (no docker / no chain).

Covers the helpers `cases/growth.py` introduces: the pinned-idx-out-of-committee log
matcher, the per-node counter verdict, the per-height verify-false clustering, and the
combined growth verdict. The live bring-up/growth is exercised by `make case-growth`
against the real devnet — NOT here.

WHY THE FIXTURE LINE CHANGED. The old one was the WARN `beacon_gate_decision` emitted at
verify time. That code path went away with the epoch key's departure from `OrderBlock`,
so the string it produced has zero occurrences in the tree — and an ABSENCE verdict over
an unproducible string is green against every possible chain, which is the defect this
file's fixture must not encode. The fixture below is the surviving ERROR, from
`beacon/actor.rs`'s finalize-deferral path."""

from __future__ import annotations

from dpos_harness.cases.growth import (
    BUGB_IDX_METRIC,
    BUGB_IDX_SIGNATURE,
    apply_case_env_defaults,
    cluster_verify_false,
    evaluate_growth_case,
    evaluate_idx_metric,
    growth_voter_idx,
    scan_idx_stall,
)

# The surviving pinned-idx ERROR (`beacon/actor.rs`, the finalize-deferral arm). Rendered
# as one line the way the tracing writer emits it — the Rust source splits it with `\`
# line-continuations, which is exactly why a source grep for it finds a shorter string
# than the runtime one.
_BUGB_LINE = ("ERROR dpos::beacon epoch=41 reason=below-quorum unmappable=1 DKG finalize "
              "deferred past the settle deadline and the pinned set names indices outside "
              "the committed committee — the pinned set and this node's committee disagree")

#: Every scanned validator reporting a clean counter — the shape a healthy run produces.
_CLEAN = {"validator-0": 0, "validator-1": 0}


def test_scan_idx_stall_finds_the_signature():
    log = ("INFO consensus finalized height=27020\n"
           f"{_BUGB_LINE}\n"
           "INFO consensus verify height=27022 ok\n")
    hits = scan_idx_stall(log)
    assert len(hits) == 1
    assert BUGB_IDX_SIGNATURE in hits[0]
    assert hits[0].startswith("ERROR")  # returned stripped


# ── the counter verdict: the PRIMARY witness ──────────────────────────────────

def test_a_non_zero_counter_on_any_node_fails():
    """The condition is chain-wide (the pinned set is agreed data), so one node is enough
    to convict. RED when a pinned idx has no seat in the committed committee."""
    ok, reason = evaluate_idx_metric({"validator-0": 0, "validator-3": 2})
    assert ok is False
    assert BUGB_IDX_METRIC in reason and "validator-3=2" in reason


def test_an_unreadable_counter_everywhere_fails_rather_than_passing():
    """THE POINT OF THIS VERDICT. `beacon_metric` answers -1 for an unreachable endpoint or
    an absent family; treating that as zero is how an absence assertion comes to assert its
    own blindness. RED when nothing answers — e.g. the metric is renamed, the registry
    moves off :9100, or every scrape times out."""
    ok, reason = evaluate_idx_metric({"validator-0": -1, "validator-1": -1})
    assert ok is False
    assert "unreadable on ALL 2" in reason and "never evaluated" in reason


def test_an_empty_scrape_set_fails():
    ok, reason = evaluate_idx_metric({})
    assert ok is False and "no validator was scraped" in reason


def test_a_partly_readable_clean_set_passes_and_names_the_blind_nodes():
    """One readable node is enough to assert (battery._inv_dkg_pinned_idx's floor), but a
    thinning detector must be visible in the PASS text before it reaches zero."""
    ok, reason = evaluate_idx_metric({"validator-0": 0, "validator-1": -1})
    assert ok is True
    assert "1/2" in reason and "unreadable: validator-1" in reason


def test_a_fully_readable_clean_set_names_no_blind_nodes():
    ok, reason = evaluate_idx_metric(_CLEAN)
    assert ok is True and "unreadable" not in reason


def test_scan_idx_stall_clean_log_is_empty():
    log = ("INFO consensus finalized height=27020\n"
           "INFO beacon dkg_ceremony ok epoch=41\n"
           "INFO consensus verify height=27021 committee ok\n")
    assert scan_idx_stall(log) == []


def test_cluster_verify_false_buckets_by_height():
    # a burst of rejections at ONE height (27021) = the finalize-stall fingerprint.
    log = "\n".join([
        "WARN verify height=27021 voting false reason=dkg_logs_idx",
        "WARN verify height=27021 voting false reason=dkg_logs_idx",
        "WARN verify height=27021 voting false reason=dkg_logs_idx",
        "WARN verify height=27099 voting false reason=bad_signature",
        "INFO verify height=27100 accepted",
    ])
    clusters = cluster_verify_false(log)
    assert clusters[27021] == 3
    assert clusters[27099] == 1
    assert 27100 not in clusters


def test_cluster_verify_false_no_height_buckets_under_minus_one():
    log = "WARN verify voting false (no height field here)\n"
    assert cluster_verify_false(log) == {-1: 1}


def test_evaluate_growth_case_pass():
    logs = {"validator-0": "INFO finalized height=200 ok",
            "validator-1": "INFO beacon dkg ok epoch=6"}
    ok, reason = evaluate_growth_case(fin0=100, fin_now=140, min_advance=32,
                                      per_node_logs=logs, per_node_idx_metric=_CLEAN)
    assert ok is True
    assert "advanced 40" in reason
    assert f"{BUGB_IDX_METRIC}=0" in reason and "2 validator" in reason


def test_evaluate_growth_case_fails_on_finalize_stall():
    # finalized did not move a full epoch across the growth window => bug B present.
    ok, reason = evaluate_growth_case(fin0=100, fin_now=108, min_advance=32,
                                      per_node_logs={"validator-0": "clean"},
                                      per_node_idx_metric=_CLEAN)
    assert ok is False
    assert "finalize-stall" in reason
    assert "advanced 8" in reason and "fin0=100" in reason


def test_the_stall_reason_carries_the_verify_false_height_bursts():
    """The diagnostic that separates "the whole committee rejected the same proposal" from
    "the chain is slow". RED if the clustering is dropped from the reason."""
    burst = "\n".join(["WARN verify height=27021 voting false"] * 3
                       + ["WARN verify height=27099 voting false"])
    ok, reason = evaluate_growth_case(fin0=100, fin_now=100, min_advance=32,
                                      per_node_logs={"validator-0": burst},
                                      per_node_idx_metric=_CLEAN)
    assert ok is False
    assert "h=27021x3" in reason and "h=27099x1" in reason


def test_evaluate_growth_case_fails_on_a_hot_counter_even_with_clean_logs():
    """The counter is the PRIMARY witness precisely because the ERROR is emitted once per
    (epoch, reason) and only on the deferral path — the counter can be non-zero with no
    line at all. RED whenever a pinned idx is unmappable, log or no log."""
    ok, reason = evaluate_growth_case(fin0=100, fin_now=200, min_advance=32,
                                      per_node_logs={"validator-0": "all quiet"},
                                      per_node_idx_metric={"validator-0": 1})
    assert ok is False
    assert BUGB_IDX_METRIC in reason and "MUST be 0" in reason


def test_evaluate_growth_case_fails_on_bugb_signature():
    # liveness advanced fine and the counter is clean, but a node carries the ERROR.
    logs = {"validator-0": "INFO finalized height=300 ok",
            "validator-2": _BUGB_LINE}
    ok, reason = evaluate_growth_case(fin0=100, fin_now=200, min_advance=32,
                                      per_node_logs=logs,
                                      per_node_idx_metric={"validator-0": 0, "validator-2": 0})
    assert ok is False
    assert "pinned-idx-out-of-committee ERROR" in reason
    assert "validator-2" in reason


def test_signature_failure_wins_only_after_liveness_gate():
    # a stall AND a signature: the finalize-stall verdict is reported first (the
    # liveness gate is checked before the log scan) — deterministic ordering.
    logs = {"validator-1": _BUGB_LINE}
    ok, reason = evaluate_growth_case(fin0=100, fin_now=100, min_advance=32,
                                      per_node_logs=logs, per_node_idx_metric={"validator-1": 3})
    assert ok is False
    assert "finalize-stall" in reason


def test_apply_case_env_defaults_leaves_room_for_growth(monkeypatch):
    for k in ("SIM_QUICK", "SIM_VALIDATORS", "SIM_INITIAL_COMMITTEE", "SIM_SPARES",
              "SIM_ROTATION_SLOTS", "SIM_EPOCH_INTERVAL", "SIM_NO_CASCADE",
              "SIM_BYZANTINE", "SIM_GEO_LATENCY"):
        monkeypatch.delenv(k, raising=False)
    prof = apply_case_env_defaults()
    # initial_committee < validators is the whole point (room for register_activate to grow).
    assert int(prof["SIM_INITIAL_COMMITTEE"]) < int(prof["SIM_VALIDATORS"])
    # INITIAL_F = (initial_committee - 1)//3 must be >= 1 (the harness floor).
    assert (int(prof["SIM_INITIAL_COMMITTEE"]) - 1) // 3 >= 1
    assert prof["SIM_NO_CASCADE"] == "1" and prof["SIM_BYZANTINE"] == "0"


def test_apply_case_env_defaults_respects_operator_override(monkeypatch):
    monkeypatch.setenv("SIM_VALIDATORS", "9")
    monkeypatch.setenv("SIM_EPOCH_INTERVAL", "48")
    prof = apply_case_env_defaults()
    assert prof["SIM_VALIDATORS"] == "9"      # setdefault must not clobber
    assert prof["SIM_EPOCH_INTERVAL"] == "48"


# ── the governance voter set (the stake-weighted-quorum fix) ───────────────────

class _Cfg:
    def __init__(self, validators):
        self.validators = validators


class _VoterChain:
    """The two reads `growth_voter_idx` makes, and nothing else. `committee` answers "" for an
    unreadable committee exactly as `Chain.committee` does (its `Runner.run` swallows the RPC
    error), and `owner_addr` answers "" for an idx with no key, exactly as `Chain.owner_addr`."""

    def __init__(self, committee, addrs):
        self._committee = committee
        self._addrs = dict(addrs)
        self.asked = []
        self.epochs = []

    def committee(self, epoch):
        self.epochs.append(epoch)
        return self._committee

    def owner_addr(self, idx):
        self.asked.append(idx)
        return self._addrs.get(idx, "")


_ADDRS = {i: f"0xowner{i}" for i in range(6)}


def test_growth_votes_with_the_live_committee_not_the_initial_prefix():
    """THE REGRESSION. After growth #1 the committee is the four originals PLUS the joiner at idx
    4, who holds 3e18 of a 7e18 voting supply. Voting the initial 4-owner prefix leaves forVotes
    at 4e18 against a 4.666e18 stake quorum → `activate-5` Defeated. The joiner must be in the
    set."""
    chain = _VoterChain(" ".join(_ADDRS[i] for i in range(5)), _ADDRS)
    assert growth_voter_idx(chain, _Cfg(6), epoch=7) == [0, 1, 2, 3, 4]
    assert chain.epochs == [7]           # the committee is read AT the epoch the caller names


def test_growth_voter_idx_spans_the_last_seatable_idx():
    """The ceiling is `validators - 1` INCLUSIVE — the highest idx growth can ever seat. An
    off-by-one here silently drops the final joiner's vote, which is the one growth step where the
    committee is largest and the quorum hardest to reach."""
    chain = _VoterChain(_ADDRS[5], _ADDRS)
    assert growth_voter_idx(chain, _Cfg(6), epoch=7) == [5]
    assert chain.asked == [0, 1, 2, 3, 4, 5]   # and it probes no identity the case does not own


def test_unreadable_committee_falls_back_to_the_gov_prefix():
    """An RPC brownout returns "" from `Chain.committee`. That must reach gov as None (→ the
    PP_GOV_VOTERS prefix, which at least votes), and must not cost a single owner-addr read."""
    chain = _VoterChain("", _ADDRS)
    assert growth_voter_idx(chain, _Cfg(6), epoch=7) is None
    assert chain.asked == []


def test_a_committee_of_strangers_is_never_an_empty_voter_list():
    """A readable committee none of whose members map to a case identity yields None, NOT []. An
    empty explicit list is the one answer worse than the prefix: gov's explicit branch would send
    the proposal with zero votes, which the Governor defeats by construction."""
    chain = _VoterChain("0xstranger0 0xstranger1", _ADDRS)
    assert growth_voter_idx(chain, _Cfg(6), epoch=7) is None
