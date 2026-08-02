"""Differential replay determinism — the CUTOVER-gate test.

Proves the Python orchestrator issues the IDENTICAL 4-draw + calm + dispatch DECISION schedule
that the bash does for a given seed, so old seeds/bundles replay with zero drift (analysis
§3.1 draw-ordering contract; the whole point of the sim).

Method (the brief's differential test): drive N rounds of the DRAW + dispatch DECISION layer
side-by-side. The BASH side sources the REAL scripts/soak-prng.sh (the actual PRNG, via
subprocess) and reproduces case-soak.sh's exact per-round decision (u32 delay, mod #ACTIONS, mod
#vpool, u32 aparam, then the calm bit and empty-pool→no-victim rule) with docker STUBBED OUT (a
synthetic committee). The PYTHON side runs Orchestrator.decide_round with the same synthetic
state. Compare the chosen (action, victim, is_calm, aparam) sequences.

Because the PRNG is sha256(ASCII "seed:ctr") — bit-identical in both languages (test_prng.py pins
it) — and the arithmetic is identical, the sequences MUST match exactly. A divergence means the
draw ORDER or the indexing drifted, which is exactly the replay-breaking regression this guards.

The bash side is now FROZEN into `fixtures/bash-oracle/round-schedules.json`, so this test keeps
asserting after `soak-prng.sh` is deleted instead of silently skipping; the live bash run remains
as a guarded check that the fixture still matches the script. Both drivers live in
`bash_oracle.py` — see its banner."""

from __future__ import annotations

import os

import pytest

from dpos_harness.tests import bash_oracle


def _py_schedule(seed, ncommittee, nactions, calm_fraction, rounds):
    os.environ.update(
        SIM_SEED=str(seed), SIM_VALIDATORS=str(ncommittee),
        SIM_INITIAL_COMMITTEE=str(ncommittee), SIM_SPARES="0", SIM_ROTATION_SLOTS="0",
        SIM_BYZANTINE=("1" if nactions == 7 else "0"), SIM_VOLUNTARY_EXIT="0",
        SIM_CALM_FRACTION=str(calm_fraction), SIM_FORCE_BYZANTINE_EPOCH="0",
        SIM_FORCE_VOLUNTARY_EXIT_EPOCH="0",
    )
    # fresh import so SimConfig re-reads the env
    import importlib
    from dpos_harness.sim import orchestrator
    importlib.reload(orchestrator)
    o = orchestrator.Orchestrator(dry_run=True)
    addrs = [f"0x{i:040x}" for i in range(ncommittee)]
    o.state.cur_committee = " ".join(addrs)
    o.state.address.addr2idx = {a: f"validator-{i}" for i, a in enumerate(addrs)}
    sched = []
    for r in range(1, rounds + 1):
        d = o.decide_round(r)
        sched.append([d.action, d.victim or "<none>", str(d.is_calm), str(d.aparam)])
    return sched


_FIXTURE_CASES = bash_oracle.round_fixture()
_IDS = [c["id"] for c in _FIXTURE_CASES]


@pytest.mark.parametrize("case", _FIXTURE_CASES, ids=_IDS)
def test_differential_replay(case):
    """ALWAYS RUN: the Python decision schedule is byte-identical to the frozen bash schedule."""
    want = case["schedule"]
    got = _py_schedule(case["seed"], case["ncommittee"], case["nactions"],
                       case["calm"], case["rounds"])
    assert len(want) == case["rounds"] and len(got) == case["rounds"]
    # first divergence (if any) is reported with its round for a precise failure
    for i, (b, p) in enumerate(zip(want, got), start=1):
        assert b == p, f"round {i}: bash={b} python={p} (seed={case['seed']})"


def test_fixture_covers_every_pinned_case():
    """The fixture and `ROUND_CASES` come from one list — a case added without regenerating would
    otherwise vanish from the parametrisation instead of failing."""
    assert _IDS == [bash_oracle.fixture_key(c) for c in bash_oracle.ROUND_CASES]


@pytest.mark.skipif(not bash_oracle.bash_available(),
                    reason="soak-prng.sh is gone — the frozen fixture is now the only oracle")
@pytest.mark.parametrize("case", _FIXTURE_CASES, ids=_IDS)
def test_frozen_fixture_still_matches_live_bash(case):
    """While the script exists, prove the FIXTURE is a faithful capture of it. Expected to skip
    after P6 deletes the bash sim stack."""
    live = bash_oracle.bash_round_schedule(case["seed"], case["ncommittee"], case["nactions"],
                                           case["calm"], case["rounds"])
    assert live == case["schedule"], (
        f"the live bash no longer produces the frozen schedule for {case['id']} — regenerate "
        f"deliberately (python3 -m dpos_harness.tests.bash_oracle), do not edit the fixture")


def test_stream_position_is_4R():
    """The stream position after R rounds is exactly 4·R (four draws per round, unconditional) —
    the analysis §3.1 invariant that makes replay hold. Verified via the Python PRNG counter."""
    from dpos_harness.sim.orchestrator import Orchestrator
    os.environ.update(SIM_SEED="20260717", SIM_VALIDATORS="7", SIM_INITIAL_COMMITTEE="7",
                      SIM_SPARES="0", SIM_ROTATION_SLOTS="0", SIM_BYZANTINE="1",
                      SIM_VOLUNTARY_EXIT="0", SIM_FORCE_BYZANTINE_EPOCH="0",
                      SIM_FORCE_VOLUNTARY_EXIT_EPOCH="0")
    o = Orchestrator(dry_run=True)
    addrs = [f"0x{i:040x}" for i in range(7)]
    o.state.cur_committee = " ".join(addrs)
    o.state.address.addr2idx = {a: f"validator-{i}" for i, a in enumerate(addrs)}
    for r in range(1, 21):
        o.decide_round(r)
        assert o.state.prng.ctr == 4 * r, f"after round {r}: ctr={o.state.prng.ctr} (want {4*r})"
