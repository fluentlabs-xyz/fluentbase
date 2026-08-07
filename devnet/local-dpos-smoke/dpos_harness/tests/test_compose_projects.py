"""`topology.DOCKER_PROJECTS` against the compose files that define them.

A hand-written mirror of a production fact goes stale, so this reads the `name:` fields rather
than repeating them. The constant exists because the suite spans FOUR docker projects and the
aggregate's between-case cleanup has to reach all of them: a bare `docker compose down` resolves
one root file and reaps one project, and `--remove-orphans` widens what is removed WITHIN that
project, never across projects.
"""

from __future__ import annotations

import pathlib

import pytest

from dpos_harness.core import topology

SMOKE_DIR = pathlib.Path(__file__).resolve().parents[2]

#: Compose ROOTS — the files that declare a project. Overlays deliberately do not.
ROOTS = {"docker-compose.yml": "fluent-dpos-smoke",
         "docker-compose.production-path.yml": "fluent-dpos-prod-path",
         "docker-compose.sim.gen.yml": "fluent-dpos-sim",
         "docker-compose.soak.gen.yml": "fluent-dpos-soak"}


def _name_of(path: pathlib.Path):
    for line in path.read_text().splitlines():
        if line.startswith("name:"):
            return line.split(":", 1)[1].strip()
    return None


@pytest.mark.parametrize("filename,project", sorted(ROOTS.items()))
def test_each_compose_root_declares_the_project_the_constant_claims(filename, project):
    path = SMOKE_DIR / filename
    if not path.exists():                       # pragma: no cover — generated roots, absent clean
        pytest.skip(f"{filename} is generated at bring-up and not in a clean tree")
    assert _name_of(path) == project
    assert project in topology.DOCKER_PROJECTS


def test_the_constant_lists_every_project_and_nothing_else():
    assert set(topology.DOCKER_PROJECTS) == set(ROOTS.values())


def test_no_OVERLAY_declares_a_project_of_its_own():
    """THE PROPERTY THAT MAKES ORPHAN-REAPING WORK. An overlay's containers join whichever root it
    was merged with, so a bare `down --remove-orphans` on that root reaps `cert-follower`,
    `cert-mitm`, `cert-follower-tamper` and friends as orphans of it — which is what
    `stack/static_stack.py::tear_down` relies on. An overlay that declared its own `name:` would
    put its containers in a project nothing tears down."""
    overlays = [p for p in sorted(SMOKE_DIR.glob("docker-compose*.yml"))
                if p.name not in ROOTS]
    assert overlays, "no overlays found — the glob or the layout changed"
    named = {p.name: _name_of(p) for p in overlays if _name_of(p) is not None}
    assert not named, f"overlays must inherit the root's project, these declare one: {named}"


def test_the_live_roots_COLLIDE_on_host_ports():
    """Why the projects have to be cleaned together rather than one at a time. If they published
    disjoint ports a leftover would be harmless; they do not, so it is not."""
    def ports(name):
        path = SMOKE_DIR / name
        if not path.exists():                   # pragma: no cover — generated root
            return None
        return {ln.split(":")[0].strip().strip('"- ')
                for ln in path.read_text().splitlines() if ":8545\"" in ln or ":8546\"" in ln}

    smoke, prod = ports("docker-compose.yml"), ports("docker-compose.production-path.yml")
    assert smoke and prod
    assert smoke & prod, "the two checked-in roots no longer share a host port — re-read the fix"
