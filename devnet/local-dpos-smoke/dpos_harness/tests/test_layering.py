"""The layering contract, enforced statically so it cannot rot back.

The package is a stack of layers with a STRICT downward dependency (see dpos_harness/__init__).
An import from a lower layer to a higher one is a build error, not a discussion — and the reason
it needs a test rather than a review habit is that every one of the three defects this suite
grew out of was invisible in a diff: `golden.py` reached up into `cases` and `sim` to compute
its own invalidation hash, `dispatch` and `orchestrator` imported each other (surviving only
because one leg was hand-placed inside a function), and `shadow` imported `cli` for one helper.

Everything here is DERIVED by AST from the source. Nothing is hand-listed except the layer
order itself and the one same-layer edge the design permits.
"""

from __future__ import annotations

import ast
import pathlib

import dpos_harness

PKG = pathlib.Path(dpos_harness.__file__).parent
PKG_NAME = dpos_harness.__name__

# Lowest first. A module in layer i may import layers 0..i; `cli` sits above everything as the
# entrypoint and is not a layer.
LAYERS = ["core", "chain", "stack", "checks", "sim", "cases"]
RANK = {name: i for i, name in enumerate(LAYERS)}

# The one same-layer edge the design keeps, and its direction. `golden` builds a snapshot by
# driving a `BringUp`; the reverse edge (bringup asking golden to restore) was removed by moving
# the golden-vs-fresh decision to the caller.
#
# It is DOCUMENTATION plus two assertions (the edge still exists; its reverse does not), and it is
# deliberately NOT consulted by the cycle check. It used to be: the cycle check skipped any edge
# whose reverse was registered here, so registering BOTH directions of a pair would have silenced
# both iterations of a genuine 2-cycle — an exemption that could hide exactly what the test is for.
# Now an entry cannot hide anything, because nothing about cycle detection reads this set.
ALLOWED_SAME_LAYER = {("stack.golden", "stack.bringup")}


def _modules():
    """Every package module as (dotted-name-without-`dpos_harness.`, path). Tests excluded —
    they legitimately import across every layer."""
    for p in sorted(PKG.rglob("*.py")):
        rel = p.relative_to(PKG)
        if rel.parts[0] == "tests" or "__pycache__" in rel.parts:
            continue
        parts = list(rel.with_suffix("").parts)
        if parts[-1] == "__init__":
            parts = parts[:-1]
        if not parts:
            continue
        yield ".".join(parts), p


def _edges():
    """All intra-package import edges as (importer, imported), both dotted names — including the
    ones written inside functions, which is exactly where an upward import hides.

    THREE import spellings reach a sibling module, and the scan must see all three or the contract
    is enforced only against the style production happens to use today:
      * `from ..core import nodes`      — relative, resolved the way Python does;
      * `from dpos_harness.core import nodes` — absolute `ImportFrom`, which the scan used to drop
        outright (`node.level == 0` was a `continue`);
      * `import dpos_harness.core.nodes`      — plain `Import`, which the scan never looked at.
    The test files themselves are written in the absolute style, so the pattern is one a
    contributor is actively taught; leaving it unscanned made the whole suite a style check.

    A name is resolved to a MODULE where one exists: `from ..core import nodes` yields
    `core.nodes`, not `core`, because `nodes` is a real module. `from ..chain.writes import Chain`
    yields `chain.writes` — `Chain` is a class, not a module. Collapsing the first case to the
    package `core` would make it a hub that every layer imports and could manufacture a cycle
    through `__init__` that does not exist in the source."""
    known = {name for name, _ in _modules()}
    out = set()
    for name, path in _modules():
        pkg_parts = name.split(".")[:-1]          # the importing module's package
        tree = ast.parse(path.read_text())
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                for a in node.names:
                    head = a.name.split(".")
                    if head[0] == PKG_NAME and head[1:]:
                        out.add((name, ".".join(head[1:])))
                continue
            if not isinstance(node, ast.ImportFrom):
                continue
            if node.level:
                base = (pkg_parts[: len(pkg_parts) - (node.level - 1)] if node.level > 1
                        else pkg_parts)
                mod_parts = node.module.split(".") if node.module else []
            elif node.module and node.module.split(".")[0] == PKG_NAME:
                base, mod_parts = [], node.module.split(".")[1:]
            else:
                continue                          # stdlib / third-party
            head = base + mod_parts
            if not mod_parts:                     # `from . import rounds` / `from .. import cli`
                for a in node.names:
                    out.add((name, ".".join(head + [a.name])))
                continue
            hit = False
            for a in node.names:
                cand = ".".join(head + [a.name])
                if cand in known:                 # imported name IS a module
                    out.add((name, cand))
                    hit = True
            if not hit:                           # imported names are symbols in `head`
                out.add((name, ".".join(head)))
    # A self-edge is NOT filtered: a module importing itself is a 1-cycle, and the cycle check
    # below is the right place for it to surface.
    return {(s, d) for s, d in out if d}


def _layer(mod: str) -> str:
    head = mod.split(".")[0]
    return head if head in RANK else ""


def test_no_upward_imports():
    """No module may import from a layer above its own. `cli` is exempt as the entrypoint; a
    layerless module may not be imported BY a layered one, which the same rule already covers."""
    bad = []
    for src, dst in sorted(_edges()):
        ls, ld = _layer(src), _layer(dst)
        if not ls or not ld:            # cli / __main__ — the entrypoint may reach anywhere
            continue
        if RANK[ld] > RANK[ls]:
            bad.append(f"{src} -> {dst}  ({ls} imports {ld}, which sits above it)")
    assert not bad, "upward imports:\n  " + "\n  ".join(bad)


def _find_cycles():
    """Every cycle reachable in the intra-package import graph, as a list of node paths.

    A real traversal, not a pair test. The previous check was `(d, s) in edges` — a 2-cycle
    detector wearing the name of a cycle detector: `a -> b -> c -> a` passed it cleanly, and a
    three-module cycle is the SHAPE a layered package actually grows (the `golden -> case_growth ->
    orchestrator` reach-up that motivated P3 was one hop away from being exactly that).

    Standard colour-marked DFS: an edge into a node still on the stack (GRAY) is a back edge, and
    the stack slice from that node is the cycle. Every module participates, `cli` and `__main__`
    included — they are exempt from the LAYER rule as entrypoints, but nothing may import them
    back, and a cycle through the entrypoint is a defect like any other."""
    graph = {}
    for s, d in _edges():
        graph.setdefault(s, set()).add(d)
    white, gray, black = 0, 1, 2
    colour = {}
    found = []
    stack = []

    def walk(node):
        colour[node] = gray
        stack.append(node)
        for nxt in sorted(graph.get(node, ())):
            if colour.get(nxt, white) == gray:
                found.append(stack[stack.index(nxt):] + [nxt])
            elif colour.get(nxt, white) == white:
                walk(nxt)
        stack.pop()
        colour[node] = black

    for start in sorted(graph):
        if colour.get(start, white) == white:
            walk(start)
    return found


def test_no_import_cycles():
    """No cycle anywhere in the package, of ANY length. `golden -> bringup` is one-way; nothing
    else may import back.

    ALLOWED_SAME_LAYER is deliberately not consulted here — see its comment. An exemption that can
    delete an edge from the graph the cycle detector runs on is an exemption that can hide a cycle,
    and this test is the one thing standing between the package and the import knot P3 untied."""
    cycles = sorted(" -> ".join(c) for c in _find_cycles())
    assert not cycles, "import cycles:\n  " + "\n  ".join(cycles)


def test_every_module_lives_in_a_layer():
    """A module dropped at the package root instead of into a layer would silently escape both
    checks above (it has no rank). Only the two entrypoints are allowed there."""
    rootless = sorted(m for m, _ in _modules() if "." not in m and m and m not in RANK)
    assert rootless == ["__main__", "cli"], f"unlayered modules at the package root: {rootless}"


def test_allowed_same_layer_edges_still_exist_and_stay_one_way():
    """An exemption nobody needs any more is an exemption that hides the next violation. Every
    entry in ALLOWED_SAME_LAYER must correspond to a real edge — and to a real ONE-WAY edge.

    The direction half is the point. The entry documents `golden -> bringup` as deliberate
    precisely BECAUSE the reverse leg was removed; if `bringup` ever imports `golden` back, the
    entry has stopped describing the design. `test_no_import_cycles` would also fire (it no longer
    consults this set), so the reverse edge is caught twice — once as a cycle, once as a claim
    here that is no longer true."""
    edges = _edges()
    stale = sorted(e for e in ALLOWED_SAME_LAYER if e not in edges)
    assert not stale, f"ALLOWED_SAME_LAYER carries edges that no longer exist: {stale}"
    reversed_ = sorted((d, s) for s, d in ALLOWED_SAME_LAYER if (d, s) in edges)
    assert not reversed_, (f"ALLOWED_SAME_LAYER documents a one-way edge, but the reverse legs "
                           f"{reversed_} now exist too")
