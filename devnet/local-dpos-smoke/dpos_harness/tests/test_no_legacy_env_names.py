"""No pre-rename env-var name survives in the package.

The P5 rename moved the simulation's knobs from `SOAK_*` to `SIM_*` and the blaster's from `LH_*`
to `LOAD_*`. ~85 + 21 names across 58 modules is exactly the kind of sweep that leaves a single
survivor behind — an `os.environ.get("SOAK_<knob>")` that now silently reads nothing and falls
back to its default, which is the failure mode that produces a green suite and a wrong live run.

This test derives the answer by SCANNING THE SOURCE rather than checking a list someone typed:
every `.py` in the package, every occurrence of either legacy prefix. A name nobody remembered is
caught the same as one everybody did.

THE ONE EXCEPTION is `tests/bash_oracle.py`, which shells out to `scripts/soak-prng.sh` and must
therefore speak the BASH script's variable names. It declares them in `BASH_ENV_NAMES`, and this
test reads that declaration instead of restating it — so the exception can neither drift nor
quietly widen (the second test below pins it to exactly what is declared)."""

from __future__ import annotations

import os
import re

import pytest

from dpos_harness.tests import bash_oracle

PACKAGE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# A legacy env-var NAME: either prefix followed by at least one more name character. The trailing
# character is what makes this a name rather than a glob — prose that discusses the rename itself
# writes the prefixes as `SOAK_*` / `LH_*`, and this file, `__init__.py` and the oracle's banner
# all have to be able to say that without tripping their own ban.
LEGACY = re.compile(r"\b(?:SOAK|LH)_[A-Z0-9][A-Z0-9_]*")

# The sole module allowed to name the bash script's own globals. Path relative to the package.
ORACLE_MODULE = os.path.join("tests", "bash_oracle.py")


def _package_modules():
    for dp, dirs, files in os.walk(PACKAGE_DIR):
        dirs[:] = [d for d in dirs if d not in ("__pycache__", "fixtures")]
        for fn in sorted(files):
            if fn.endswith(".py"):
                yield os.path.relpath(os.path.join(dp, fn), PACKAGE_DIR)


MODULES = sorted(_package_modules())


def test_the_scan_actually_sees_the_package():
    """A walk that silently matched nothing would make every assertion below vacuous."""
    assert len(MODULES) > 50, MODULES
    assert ORACLE_MODULE in MODULES


@pytest.mark.parametrize("rel", [m for m in MODULES if m != ORACLE_MODULE])
def test_no_legacy_env_prefix_survives(rel):
    """Every module except the declared bash-oracle exception is free of `SOAK_*` / `LH_*`."""
    with open(os.path.join(PACKAGE_DIR, rel)) as f:
        src = f.read()
    hits = sorted(set(LEGACY.findall(src)))
    assert not hits, (
        f"{rel} still names pre-rename env vars {hits} — `SOAK_*` became `SIM_*` and `LH_*` "
        f"became `LOAD_*`. If this genuinely has to speak to bash, it belongs in "
        f"{ORACLE_MODULE}, not here.")


def test_the_oracle_exception_is_exactly_what_it_declares():
    """The exception is bounded by its own declaration: `bash_oracle.py` may name the three bash
    globals it sources and nothing else. Widening it means editing `BASH_ENV_NAMES` on purpose."""
    with open(os.path.join(PACKAGE_DIR, ORACLE_MODULE)) as f:
        src = f.read()
    hits = set(LEGACY.findall(src))
    declared = set(bash_oracle.BASH_ENV_NAMES)
    assert hits == declared, (
        f"{ORACLE_MODULE} names {sorted(hits - declared)} beyond its declared exception "
        f"{sorted(declared)} (or has stopped using {sorted(declared - hits)})")
