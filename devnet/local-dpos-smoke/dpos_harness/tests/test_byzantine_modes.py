"""test_byzantine_modes.py — the cross-language pin on `FLUENT_DPOS_BYZANTINE`.

THE DEFECT THIS EXISTS FOR. The sim actuates a byzantine validator by writing
`FLUENT_DPOS_BYZANTINE=<mode>` into a one-service compose overlay and recreating the container.
The node parses that string in `crates/node/src/dpos.rs` and **fails loud** on anything it does
not know — an `eyre::bail!` at DPoS start, on purpose, so a misconfigured smoke cannot silently
run honest. The two halves are correct in isolation and catastrophic together: when a mode is
retired on the Rust side, the harness keeps asking for it, the node dies at start, and the sim
reads a HARD-KILLED validator as a live byzantine one — it then measures the cluster's response
to a fault that was never injected, and every downstream reconciler waits on a rejoin that
belongs to a container which is not running.

That is exactly what happened to `forge-beacon-pk` (retired when the epoch key left `OrderBlock`)
on `byzantine_forge_pk`, which was DEFAULT-ON via `SIM_BYZANTINE=1`. Nothing said so, because
nothing compared the two vocabularies.

So: `actions.SUPPORTED_BYZANTINE_MODES` is the harness's exhaustive vocabulary, and this file
parses the node's match arms and asserts the harness is a subset of what the node accepts.

WHY PARSE THE RUST rather than restate it. A restatement is a second copy that drifts the same
silent way. The parse is deliberately narrow and anchored on the literal shape of the arms, so a
refactor that moves the match somewhere unrecognisable trips the "found no arms" guard rather
than passing vacuously. The file's ABSENCE is a hard failure too — the smoke lives inside the
repo, so a missing `dpos.rs` means the anchor moved, not that the pin does not apply.
"""

from __future__ import annotations

import os
import re

import pytest

from dpos_harness.sim import actions
from dpos_harness.tests.bash_oracle import SMOKE_DIR

#: `<smoke>` is `devnet/local-dpos-smoke`, so the repo root is two dirnames up.
REPO_ROOT = os.path.normpath(os.path.join(SMOKE_DIR, "..", ".."))
DPOS_RS = os.path.join(REPO_ROOT, "crates", "node", "src", "dpos.rs")

#: `Some("equivocate") => {` / `Some("forge-beacon-pk") => eyre::bail!(` — the arm head plus
#: enough of its body to tell an ACCEPTING arm from a rejecting one. The `Some(other)` catch-all
#: has no string literal and is skipped by construction.
_ARM = re.compile(r'Some\("([a-z0-9-]+)"\)\s*=>\s*(.{0,40})', re.S)


def _node_modes():
    """(accepted, rejected) mode strings, read out of the node's parse site.

    An arm whose body starts a `bail!` REJECTS the mode: the flag parses, the process dies. Every
    other literal arm accepts it."""
    assert os.path.isfile(DPOS_RS), (
        f"{DPOS_RS} not found — the repo-root anchor moved. Fix the anchor; do not skip the pin.")
    with open(DPOS_RS, encoding="utf-8") as fh:
        src = fh.read()
    start = src.find("cfg.byzantine_mode.as_deref()")
    assert start != -1, (
        "could not find `cfg.byzantine_mode.as_deref()` in dpos.rs — the byzantine parse site was "
        "renamed or restructured. Re-anchor this pin rather than deleting it.")
    # Bound the scan to the match block: the next `// Cert upstream` section comment ends it, and
    # a fixed-size window backstops a comment rename.
    block = src[start:start + 2000]
    accepted, rejected = set(), set()
    for mode, tail in _ARM.findall(block):
        (rejected if "bail!" in tail else accepted).add(mode)
    assert accepted or rejected, "parsed no `Some(\"…\")` arms — the arm shape changed"
    return accepted, rejected


def test_the_node_still_accepts_every_mode_the_harness_can_ask_for():
    """RED when a mode the harness may write into `FLUENT_DPOS_BYZANTINE` is retired (moved to a
    `bail!` arm) or dropped from the node's match — i.e. the day the harness would start killing
    validators instead of corrupting them."""
    accepted, rejected = _node_modes()
    harness = set(actions.SUPPORTED_BYZANTINE_MODES)
    assert harness, "the harness declares no byzantine modes at all"
    retired = harness & rejected
    assert not retired, (
        f"{sorted(retired)} is RETIRED in crates/node/src/dpos.rs (the arm bails) but the harness "
        "still lists it in SUPPORTED_BYZANTINE_MODES — a sim action using it hard-kills the "
        "victim instead of making it byzantine")
    unknown = harness - accepted
    assert not unknown, (
        f"{sorted(unknown)} is not an accepted arm in crates/node/src/dpos.rs (accepted: "
        f"{sorted(accepted)}); the node bails on an unknown mode, so the victim would die at "
        "DPoS start")


def test_forge_beacon_pk_is_gone_from_the_harness():
    """The specific regression, named. `forge-beacon-pk` forged the `beacon_outcome` a
    change-boundary block asserted; no block asserts a key any more, so there is nothing to forge
    and no substitute for it. RED if it (or any `forge-*` spelling) comes back anywhere the sim
    can actuate from."""
    assert "forge-beacon-pk" not in actions.SUPPORTED_BYZANTINE_MODES
    assert not [m for m in actions.SUPPORTED_BYZANTINE_MODES if m.startswith("forge")]


@pytest.mark.parametrize("mode", ["forge-beacon-pk", "", "equivocate-typo", "EQUIVOCATE"])
def test_act_byzantine_refuses_an_unsupported_mode(mode):
    """The actuator itself is the last gate, and it refuses BEFORE the `dry_run` return: a dry
    walk that accepts a mode a live run would die on is a transcript that lies. RED if the guard
    is removed or narrowed."""
    act = actions.Actuators.__new__(actions.Actuators)
    act.dry_run = True
    with pytest.raises(ValueError, match="unsupported byzantine mode"):
        act.act_byzantine("validator-2", mode)


def test_act_byzantine_accepts_every_declared_mode():
    """The mirror image — the guard must not reject what the harness declares, or every byzantine
    action becomes a no-op raise. RED if `SUPPORTED_BYZANTINE_MODES` and the guard disagree."""
    act = actions.Actuators.__new__(actions.Actuators)
    act.dry_run = True
    for mode in actions.SUPPORTED_BYZANTINE_MODES:
        act.act_byzantine("validator-2", mode)   # dry: returns after the guard, touches nothing
