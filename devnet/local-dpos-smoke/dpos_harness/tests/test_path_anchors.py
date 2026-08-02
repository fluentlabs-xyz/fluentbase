"""The `__file__`-relative path anchors, pinned to the smoke dir so re-nesting cannot move them.

Four modules compute the smoke directory by counting dirnames up from their own `__file__`. The
count is a hard-coded integer that encodes how deeply the module sits in the package — so the P3
restructure, which added exactly one level of nesting to every one of them, could have silently
relocated the golden tarball, the status reader's repo root, and the shadow runner's log search
root. It did not (the counts were updated with the move), but nothing would have said so.

`stack/sender.py` has had this guard since v61 a12 and it is the model here: a wrong path there
yields a bare `"0x"` instead of the GasBurner deploy bytecode, and `test_sender.py:38-43` fails on
it. The other three resolve to a DIRECTORY, and a wrong directory is silent — `golden.py` would
write its tarball to a fresh `<smoke>/dpos_harness/sim-out/golden/` and report success. So the
assertion is made explicit: each anchor must land on a directory carrying the smoke dir's
landmarks, and all of them must land on the SAME one.

Why landmarks and not a path built from THIS file: this file is nested too, so deriving the
expected answer from `tests/test_path_anchors.py` would encode the same assumption on both sides
and pass through any uniform re-nesting. The landmarks are properties of the smoke dir itself.
"""

from __future__ import annotations

import os

import pytest

from dpos_harness.cases.smoke import asserts, asserts_follow
from dpos_harness.core import events
from dpos_harness.sim import shadow, status
from dpos_harness.stack import golden

# Files/dirs that exist in <smoke> and in NEITHER of its two nearest wrong answers: `devnet/`
# (one dirname too many) holds only `local-dpos-smoke/`, and `<smoke>/dpos_harness/` (one too few)
# holds the package. Any off-by-one in an anchor's dirname count fails at least one of these.
LANDMARKS = ("Makefile", "contracts", "dpos_harness", "docker-compose.dpos.yml", "scripts")

ANCHORS = {
    "stack.golden._SMOKE_DIR": golden._SMOKE_DIR,
    "sim.status.REPO_SMOKE_DIR": status.REPO_SMOKE_DIR,
    "sim.shadow.REPO_SMOKE_DIR": shadow.REPO_SMOKE_DIR,
    # The fourth anchor sits one level DEEPER than the other three (`cases/smoke/`), which is
    # exactly the case `test_all_anchors_agree` exists for: a per-module dirname count that is
    # right for `stack/` is wrong here, and nothing else would say so.
    "cases.smoke.asserts._SMOKE_DIR": asserts._SMOKE_DIR,
    "cases.smoke.asserts_follow._SMOKE_DIR": asserts_follow._SMOKE_DIR,
    "core.events._SMOKE_DIR": events._SMOKE_DIR,
}


def test_the_mock_rollup_artifact_resolves_and_parses():
    """`smoke-cert-cascade` deploys this at runtime, so a wrong anchor is silent in the same way
    the probe's is — worse, actually: an empty `--create` deploys a CODELESS contract, and a
    `setCheckpoint` call to a codeless address returns empty rather than failing. Phase 1 would
    then report that the follower could not verify the checkpoint, on a chain where nothing is
    wrong except that the Rollup has no code."""
    import json

    assert os.path.isfile(asserts_follow.MOCK_ROLLUP_JSON), asserts_follow.MOCK_ROLLUP_JSON
    with open(asserts_follow.MOCK_ROLLUP_JSON) as fh:
        code = json.load(fh)["bytecode"]["object"]
    assert code.startswith("0x") and len(code) > 2, "the MockRollup artifact carries no bytecode"


def test_the_probe_artifact_resolves_and_parses():
    """`stack/sender.py`'s model: an anchor that resolves to a FILE is checked by reading it.

    A wrong path here is silent in the worst way — `assert_vrf`'s C1/C2 step would report "could
    not read the PrevRandaoProbe bytecode" on a perfectly healthy chain, and the deploy it feeds
    would otherwise go out as an empty `--create`."""
    import json

    assert os.path.isfile(asserts.PROBE_JSON), asserts.PROBE_JSON
    with open(asserts.PROBE_JSON) as fh:
        code = json.load(fh)["bytecode"]["object"]
    assert code.startswith("0x") and len(code) > 2, "the probe artifact carries no bytecode"


@pytest.mark.parametrize("name", sorted(ANCHORS))
def test_anchor_resolves_to_the_smoke_dir(name):
    """Each `__file__`-relative anchor lands on the smoke dir, evidenced by its landmarks."""
    path = ANCHORS[name]
    assert os.path.isdir(path), f"{name} = {path!r} is not a directory"
    missing = [m for m in LANDMARKS if not os.path.exists(os.path.join(path, m))]
    assert not missing, (
        f"{name} resolved to {path!r}, which is missing {missing} — that is not the smoke dir. "
        f"The module's nesting changed without its dirname count changing with it.")


def test_all_anchors_agree():
    """The three anchors sit at three different depths (`stack/` and `sim/` are both one level
    down today, but nothing enforces that they stay together). They must still name one directory
    — a module moved on its own is the case the per-anchor check above cannot distinguish from a
    module moved along with everything else."""
    resolved = {name: os.path.realpath(p) for name, p in ANCHORS.items()}
    assert len(set(resolved.values())) == 1, f"anchors disagree on the smoke dir: {resolved}"


def test_golden_artifacts_land_under_the_smoke_dir():
    """The anchor exists to place the golden tarball. A stale tarball at a moved path is not an
    error at any level — `is_golden_fresh()` simply reports False and the case pays for a full
    boot, which is the expensive, benign-looking regression this whole file guards."""
    smoke = os.path.realpath(golden._SMOKE_DIR)
    assert os.path.realpath(golden.GOLDEN_DIR) == os.path.join(smoke, "sim-out", "golden")
    for artifact in (golden.TARBALL, golden.SIDECAR, golden.FACTS):
        assert os.path.realpath(artifact).startswith(os.path.realpath(golden.GOLDEN_DIR) + os.sep)


@pytest.mark.skipif("SIM_OUT" in os.environ, reason="SIM_OUT overrides the anchored default")
def test_run_artifacts_land_under_the_smoke_dir():
    """`golden.GOLDEN_DIR` is a subdirectory of the same `sim-out/` the event log and the failure
    bundles are written to, so the two must resolve to one place. They did not: this default was
    cwd-relative, which put a run's events.jsonl and its bundles wherever the operator stood while
    its golden tarball stayed anchored — a silent split of one run's artifacts across two trees,
    and an un-gitignored sim-out/ at whatever directory that turned out to be."""
    smoke = os.path.realpath(golden._SMOKE_DIR)
    assert os.path.realpath(events.SIM_OUT) == os.path.join(smoke, "sim-out")
