"""gate_accept rule matrix — port of the soak-gate-test.sh §8.4 assertions.

The bash battery drove the REAL gate_accept with synthetic GA_* inputs; this is the 1:1 Python
equivalent (a textbook parametrized (inputs → accept/reason) table). The ~35% bash-idiom
assertions (assoc-array mis-parse, subshell side-effect survival) are dropped by construction —
they guard bugs Python cannot have — with the BEHAVIORAL assertion underneath kept."""

from __future__ import annotations

import pytest

from dpos_harness.sim.actions import GateInputs, gate_accept


def _accept(**kw):
    # The orchestrator ALWAYS sets GA_PENDING_MIN = n (case-soak.sh:3096); mirror that default so
    # a test that isn't probing rule 5 doesn't trip it. Tests exercising rule 5 pass pending_min.
    kw.setdefault("pending_min", kw.get("n", 0))
    ok, reason = gate_accept(GateInputs(**kw))
    return ok, reason


# ── victim availability ───────────────────────────────────────────────────────
def test_reject_victim_v0_pinned():
    ok, r = _accept(action="graceful_stop_restart", victim="validator-0", n=7)
    assert not ok and r == "victim-is-pinned-host-rpc"


def test_reject_victim_already_disrupted():
    ok, r = _accept(action="sigkill_restart", victim="validator-3",
                    disrupted="validator-3", n=7)
    assert not ok and r == "victim-already-disrupted"


# ── rule 1: transient quorum (survive exactly f EFFECTIVE faults; reject the f+1-th) ──
@pytest.mark.parametrize("n,effective,expect_ok", [
    (7, "", True),                       # f=2, 0 effective, +victim=1 <= 2
    (7, "validator-2", True),            # 1 effective + victim=2 <= 2
    (7, "validator-4 validator-5", False),  # 2 effective, victim-3 not among them, +1=3 > f=2
    (4, "", True),                       # f=1, 0 effective, +1 <= 1
    (4, "validator-2", False),           # f=1, 1 effective + victim=2 > 1
])
def test_rule1_effective_union(n, effective, expect_ok):
    ok, r = _accept(action="cpu_throttle", victim="validator-3", n=n,
                    effective=effective, min_committee=4)
    assert ok == expect_ok, r
    if not expect_ok:
        assert r.startswith("rule1")


# ── rules 2 … 7d: the REJECT half of the table ────────────────────────────────
# One row per rejecting rule: the GA_* inputs, and the reason prefix the gate must answer with.
# Precedence is part of what each row asserts (rule4c's n=7 clears 4b's floor and lands on 4c),
# so the inputs are the ones the standalone tests carried, unchanged.
_REJECTS = [
    # rule 2 / 2b: DKG window
    (dict(action="cpu_throttle", victim="validator-3", n=7,
          change_imminent=1, incoming="validator-3", min_committee=4),
     "rule2-dkg-window", "rule2_nondkg_on_incoming_rejected"),
    (dict(action="sigkill_restart", victim="validator-3", n=7,
          change_pending=1, min_committee=4),
     "rule2b", "rule2b_kill_during_inflight_change_rejected"),
    # rule 3: shareless <= f  (+1 for dkg_midwindow → 3 > f=2)
    (dict(action="dkg_midwindow_restart", victim="validator-3", n=7,
          shareless=2, min_committee=4),
     "rule3", "rule3_shareless_over_f"),
    # rule 4b / 4c: floors + serialize (rule 4 went with the `liveness_jail` action it gated)
    (dict(action="byzantine_equivocate", victim="validator-3", n=4, min_committee=4),
     "rule4b", "rule4b_byzantine_floor"),
    (dict(action="byzantine_equivocate", victim="validator-3", n=7,
          min_committee=4, membership_settling=1),
     "rule4c", "rule4c_serialize_membership"),
    # rule 5: pending projection
    (dict(action="cpu_throttle", victim="validator-3", n=7, min_committee=4, pending_min=3),
     "rule5", "rule5_pending_projection"),
    # rule 6: leader-liveness
    (dict(action="cpu_throttle", victim="validator-3", n=7, min_committee=4,
          leader="validator-3", round_others=1),
     "rule6", "rule6_leader_plus_concurrent"),
    # rule 7: voluntary_exit
    (dict(action="voluntary_exit", victim="validator-3", n=4, min_committee=4),
     "rule7-floor", "rule7_floor"),
    (dict(action="voluntary_exit", victim="validator-3", n=7, min_committee=4,
          add_source_avail=0),
     "rule7-no-add-source", "rule7_no_add_source"),
    # n=7 f=2, 2 seated faults, exit a HEALTHY node → n'=6 f'=1 → 2 > 1 → reject
    (dict(action="voluntary_exit", victim="validator-5", n=7, min_committee=4,
          add_source_avail=1, effective="validator-2 validator-3"),
     "rule7d", "rule7d_post_shrink_quorum"),
]


@pytest.mark.parametrize("kw,rule", [(k, r) for k, r, _ in _REJECTS],
                         ids=[i for _, _, i in _REJECTS])
def test_the_gate_rejects(kw, rule):
    ok, r = _accept(**kw)
    assert not ok and r.startswith(rule), r


# ── the ACCEPT half: the near-misses each reject rule must NOT swallow ────────
_ACCEPTS = [
    # victim already effective → proj stays cur, not cur+1: 2 effective, victim already in it
    # → proj=2 <= f=2
    (dict(action="cpu_throttle", victim="validator-2", n=7,
          effective="validator-2 validator-3", min_committee=4),
     "rule1_no_double_count_victim_already_in_union"),
    # cpu_throttle is NOT a mesh-churn action (no recreate) — passes 2b.
    (dict(action="cpu_throttle", victim="validator-3", n=7,
          change_pending=1, min_committee=4),
     "rule2b_cpu_throttle_not_gated_by_mesh_window"),
    (dict(action="cpu_throttle", victim="validator-3", n=7, min_committee=4,
          leader="validator-3", round_others=0),
     "rule6_leader_alone_ok"),
    (dict(action="voluntary_exit", victim="validator-5", n=7, min_committee=4,
          add_source_avail=1, effective=""),
     "rule7_clean_exit_accepts"),
]


@pytest.mark.parametrize("kw", [k for k, _ in _ACCEPTS], ids=[i for _, i in _ACCEPTS])
def test_the_gate_accepts(kw):
    ok, r = _accept(**kw)
    assert ok, r
