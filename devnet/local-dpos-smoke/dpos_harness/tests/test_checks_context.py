"""test_checks_context.py — the checks/sim boundary (P4 item 4.4), enforced by DERIVATION.

`checks/battery.py` used to read one flat 35-field context, most of it the simulation's churn
bookkeeping, which is why nothing but the simulation could use the battery. It now takes two:
`Ctx` (chain/topology/metrics facts + threshold knobs — anyone can supply these) and `SimCtx`
(churn bookkeeping only the simulation has). The detectors split the same way: `ChainBattery`
holds the chain-only ones and has no `sim` attribute at all; `Battery(ChainBattery)` adds the
five that are sim-coupled by nature.

This file is the thing that keeps that true. Four decouplings in this codebase have been undone
by nothing enforcing them, so NOTHING here is a hand-written list checked against itself:

  * the classification is derived from which class defines the method;
  * "chain-only" is verified by scanning the AST for a sim read, not by trusting the class;
  * the REVERSE holds too — a method may only sit on Battery if it (transitively) reads sim
    state, so a chain-only detector cannot be parked on the sim side to dodge the rule;
  * the table in battery.py's own docstring is parsed and compared against the derived truth,
    so the documentation cannot drift from the code either.
"""

import ast
import inspect
import re

import pytest

from dpos_harness.checks import battery as B
from dpos_harness.checks.battery import Battery, ChainBattery, Ctx, SimCtx

_SRC = inspect.getsource(B)
_TREE = ast.parse(_SRC)


def _class(name):
    for n in _TREE.body:
        if isinstance(n, ast.ClassDef) and n.name == name:
            return n
    raise AssertionError(f"class {name} not found in battery.py")


def _methods(cls_name):
    return {m.name: m for m in _class(cls_name).body if isinstance(m, ast.FunctionDef)}


def _detectors(cls_name):
    return {n for n in _methods(cls_name) if n.startswith("_inv_") and n != "_inv_fail"}


def _sim_reads(fn):
    """Every simulation-state read in one method body: `self.sim.X`, and `alias.X` for any local
    alias of `self.sim`. Returns the set of field names read."""
    aliases = set()
    for n in ast.walk(fn):
        if (isinstance(n, ast.Assign) and isinstance(n.value, ast.Attribute)
                and n.value.attr == "sim" and isinstance(n.value.value, ast.Name)
                and n.value.value.id == "self"):
            aliases |= {t.id for t in n.targets if isinstance(t, ast.Name)}
    out = set()
    for n in ast.walk(fn):
        if not isinstance(n, ast.Attribute):
            continue
        base = n.value
        if isinstance(base, ast.Name) and base.id in aliases:
            out.add(n.attr)
        elif (isinstance(base, ast.Attribute) and base.attr == "sim"
              and isinstance(base.value, ast.Name) and base.value.id == "self"):
            out.add(n.attr)
    return out


def _self_calls(fn):
    return {n.func.attr for n in ast.walk(fn)
            if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute)
            and isinstance(n.func.value, ast.Name) and n.func.value.id == "self"}


# ── the boundary itself ──────────────────────────────────────────────────────

def test_chain_battery_cannot_reach_simulation_state():
    """THE structural rule: not one method defined on ChainBattery reads `self.sim`. It is not a
    convention — ChainBattery has no such attribute, so such a read raises AttributeError on the
    class a non-simulation consumer instantiates."""
    offenders = {name: sorted(_sim_reads(fn))
                 for name, fn in _methods("ChainBattery").items() if _sim_reads(fn)}
    assert offenders == {}, (
        f"chain-only method(s) reaching for simulation state: {offenders}. Either drop the read "
        f"or MOVE the method to Battery and update the classification table in battery.py.")
    assert not hasattr(ChainBattery(), "sim")
    assert hasattr(Battery(), "sim")


def test_every_sim_coupled_detector_actually_needs_simulation_state():
    """The reverse, so the split cannot be dodged by parking a chain-only detector on the sim
    side: every DETECTOR on the subclass must read sim state itself or through a helper it
    calls. (Helpers themselves are exempt — a pure helper belongs next to its caller; they are
    covered by the orphan check below.)"""
    subs = _methods("Battery")
    reaching = {n for n, fn in subs.items() if _sim_reads(fn)}
    for _ in range(len(subs)):                       # transitive closure over self-calls
        grown = {n for n, fn in subs.items() if _self_calls(fn) & reaching}
        if grown <= reaching:
            break
        reaching |= grown
    stragglers = _detectors("Battery") - reaching
    assert stragglers == set(), (
        f"{sorted(stragglers)} sit on Battery but never reach simulation state — they belong on "
        f"ChainBattery, where a framework consumer can use them.")


def test_no_orphan_helpers_on_the_sim_side():
    """A helper only earns its place on Battery by being called from there; otherwise it is a
    chain-only helper hiding behind the subclass."""
    subs = _methods("Battery")
    called = set().union(*(_self_calls(fn) for fn in subs.values()))
    orphans = set(subs) - called - _detectors("Battery") - {"__init__", "bind",
                                                            "check_invariants"}
    assert orphans == set(), f"{sorted(orphans)} defined on Battery but never called from it"


def test_contexts_are_disjoint_and_reject_the_other_half():
    gen, sim = set(vars(Ctx())), set(vars(SimCtx()))
    assert gen & sim == set()
    for f in sim:                                     # the wall, field by field
        with pytest.raises(TypeError):
            Ctx(**{f: None})
        assert not hasattr(Ctx(), f)
    for f in gen:
        with pytest.raises(TypeError):
            SimCtx(**{f: None})


def test_bind_refreshes_both_halves():
    """There is no single-half setter: the orchestrator binds the pair in one call, so it cannot
    refresh the chain facts and leave the churn bookkeeping at last tick's snapshot."""
    bat = Battery()
    ctx, sim = Ctx(SIM_TICK=9), SimCtx(NEXT_JOINER=4)
    bat.bind(ctx, sim)
    assert bat.ctx is ctx and bat.sim is sim
    with pytest.raises(TypeError):
        bat.bind(ctx)                                 # both or neither


def test_nothing_outside_the_battery_assigns_a_context_half_directly():
    """`bind` is only a discipline if it is the ONLY way in. Python cannot forbid
    `battery.ctx = …`, so this scans the production callers instead: a direct assignment to a
    battery's `.ctx` or `.sim` is a half-bind, which is how the loop leaves one half frozen at
    last tick's snapshot while looking perfectly correct."""
    import pathlib

    from dpos_harness.checks import battery as bat_mod
    root = pathlib.Path(bat_mod.__file__).parent.parent
    bad = []
    for path in sorted(root.rglob("*.py")):
        if path.name == "battery.py" or "tests" in path.parts:
            continue                                  # bind's own implementation; test doubles
        for n in ast.walk(ast.parse(path.read_text())):
            if not isinstance(n, ast.Assign):
                continue
            for t in n.targets:
                if isinstance(t, ast.Attribute) and t.attr in ("ctx", "sim"):
                    bad.append(f"{path.relative_to(root)}:{n.lineno} -> .{t.attr}")
    assert bad == [], f"half-bind(s) bypassing Battery.bind: {bad}"


# ── the classification, derived and pinned against the documented table ──────

def _documented(header):
    """Pull one block of the classification table out of battery.py's module docstring. The
    CHAIN-ONLY block is a comma-separated list; the SIM-COUPLED block is `name — why`."""
    lines = ast.get_docstring(_TREE).split("\n")
    i = next(k for k, ln in enumerate(lines) if ln.strip().startswith(header))
    block = []
    for ln in lines[i + 1:]:
        if not ln.strip():
            break
        block.append(ln)
    entry = re.compile(r"^ {4}([a-z][a-z0-9-]*) +—")
    if any(entry.match(ln) for ln in block):
        return {entry.match(ln).group(1) for ln in block if entry.match(ln)}
    return {w for ln in block for w in re.split(r"[,\s]+", ln.strip()) if w}


def _ids(methods):
    return {m[len("_inv_"):].replace("_", "-") for m in methods}


def test_classification_matches_the_documented_table():
    chain, sim = _detectors("ChainBattery"), _detectors("Battery")
    assert chain & sim == set()
    assert len(chain) == 26 and len(sim) == 4, (
        f"detector classification moved: {len(chain)} chain-only / {len(sim)} sim-coupled. That "
        f"is the deliverable of this split — update the count here AND the table in battery.py.")
    assert _ids(chain) == _documented("CHAIN-ONLY"), "battery.py's CHAIN-ONLY table drifted"
    assert _ids(sim) == _documented("SIM-COUPLED"), "battery.py's SIM-COUPLED table drifted"


def test_the_ordered_dispatch_still_runs_every_detector():
    """Neither half may be silently dropped from check_invariants by the class split."""
    called = _self_calls(_methods("Battery")["check_invariants"])
    missing = (_detectors("ChainBattery") | _detectors("Battery")) - called
    assert missing == set(), f"detector(s) defined but never dispatched: {sorted(missing)}"
