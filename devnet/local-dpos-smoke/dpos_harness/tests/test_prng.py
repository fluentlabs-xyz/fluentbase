"""PRNG bit-compatibility tests — pins the hand-verified draws AND the cross-language stream.

The cross-language pin used to be a live `bash -c 'source soak-prng.sh …'` on every run, skipped
when bash was missing. `soak-prng.sh` is scheduled for deletion, so the streams are now FROZEN in
`fixtures/bash-oracle/prng-streams.json` and asserted UNCONDITIONALLY; the live bash run stays as
a separate, guarded check that the FIXTURE still matches the script it came from. See
`bash_oracle.py` for why that ordering is the point."""

from __future__ import annotations

import pytest

from dpos_harness.core.prng import SimPRNG
from dpos_harness.tests import bash_oracle


def test_pinned_draws_seed42():
    """The exact draws the analysis (§3.1) hand-verified against live bash."""
    p = SimPRNG(42)
    assert p.u32() == 1416840650  # seed=42 ctr=0  (bash 547345ca)
    assert p.u32() == 64878673  # seed=42 ctr=1


def test_pinned_mod_seed42():
    """analysis §3.1 pin: seed=42, one u32 (ctr0) then next_mod 7 draws ctr1
    (64878673) % 7 == 6 — the mod is on the SECOND stream position, matching the
    bash differential (next_u32; next_mod 7 -> 6)."""
    p = SimPRNG(42)
    p.u32()  # consume ctr0
    assert p.mod(7) == 6
    # equivalently, a stream resumed at ctr=1:
    assert SimPRNG(42, ctr=1).mod(7) == 6


def test_mod_zero_and_negative():
    """m <= 0 yields 0, byte-identical to bash `if (( m > 0 )) ... else 0`."""
    p = SimPRNG(42)
    assert p.mod(0) == 0  # still consumes a stream position
    assert p.ctr == 1
    assert p.mod(-5) == 0
    assert p.ctr == 2


def test_counter_advances_one_per_draw():
    p = SimPRNG(7)
    assert p.ctr == 0
    p.u32()
    assert p.ctr == 1
    p.mod(10)
    assert p.ctr == 2


def test_ctr_resume():
    """A stream resumed at ctr=N draws exactly what a fresh stream draws after N."""
    fresh = SimPRNG(99)
    for _ in range(5):
        fresh.u32()
    resumed = SimPRNG(99, ctr=5)
    assert resumed.u32() == fresh.u32()


@pytest.mark.parametrize("seed", bash_oracle.PRNG_SEEDS)
def test_differential_against_frozen_bash(seed):
    """Cross-language pin, ALWAYS RUN: the first 8 u32 draws and a mod-7 of the stream match the
    bash streams frozen in the fixture."""
    want = bash_oracle.prng_fixture()[seed]
    p = SimPRNG(seed)
    got = [p.u32() for _ in range(bash_oracle.PRNG_DRAWS)]
    assert got == want["u32"], f"u32 stream diverged from the bash oracle for seed={seed}"
    assert p.mod(bash_oracle.PRNG_MOD) == want["mod"], f"mod stream diverged for seed={seed}"


def test_fixture_covers_every_pinned_seed():
    """The fixture and the parametrisation come from one list; a seed added to `PRNG_SEEDS`
    without regenerating would otherwise fail as a confusing KeyError inside the test above."""
    assert sorted(bash_oracle.prng_fixture()) == sorted(bash_oracle.PRNG_SEEDS)


@pytest.mark.skipif(not bash_oracle.bash_available(),
                    reason="soak-prng.sh is gone — the frozen fixture is now the only oracle")
@pytest.mark.parametrize("seed", bash_oracle.PRNG_SEEDS)
def test_frozen_fixture_still_matches_live_bash(seed):
    """While the script exists, prove the FIXTURE is a faithful capture of it. This is the check
    that gives the frozen streams their authority; it is expected to skip after P6."""
    assert bash_oracle.bash_prng_stream(seed) == bash_oracle.prng_fixture()[seed], (
        f"scripts/soak-prng.sh no longer produces the frozen stream for seed={seed} — the bash "
        f"changed under the fixture; regenerate deliberately, do not edit the fixture")
