"""Seeded integer PRNG for the production sim — Python port of soak-prng.sh.

A counter-mode sha256 stream: byte-stable on any host, unlike bash `$RANDOM`
(16-bit, RNG-impl-dependent across bash builds — NOT portably reproducible, so
it is rejected here). The whole point of the sim is that the logged SIM_SEED
replays the identical INTENT schedule (plan §1.1); that guarantee rests on this
stream being a pure function of (SIM_SEED, counter).

Bit-compatibility contract (analysis §3.1, hand-verified against live bash):
    u32(seed, ctr) = int(sha256(f"{seed}:{ctr}").hexdigest()[:8], 16)
The stream is sha256(ASCII "seed:ctr") — a spec both languages implement
identically — so old seed VALUES and their bundles remain replayable with zero
drift (only the knob's NAME moved when the harness was renamed to `sim`).

PORT-NOTE: bash's canonical hazard here — the draw functions returned their value
in a GLOBAL shell variable and had to be called DIRECTLY (never `$(next_u32)`),
because a command substitution ran in a SUBSHELL and the counter global's
increment would be lost in the parent (every draw identical). In Python the
counter is instance state mutated by a method call, so that hazard CANNOT exist —
the contortion dies, the semantics stay identical.

Draw-ordering contract (unchanged): callers draw in a FIXED order, one u32()/
mod() per logical draw, and MUST NOT insert conditional draws (plan §1.1: stream
position at round R is always 4·R), or replay diverges. That discipline lives in
the orchestrator port, not here.
"""

from __future__ import annotations

import hashlib


class SimPRNG:
    """Counter-mode sha256 PRNG. Bit-identical to soak-prng.sh's next_u32/next_mod.

    The seed is kept as a string exactly as bash interpolates it (its seed global,
    defaulted to ``0``); an int seed is stringified the same way bash would print it.
    """

    def __init__(self, seed, ctr: int = 0):
        # bash sources soak-prng.sh with its counter global defaulting to 0 and its
        # seed global defaulting to "0" only when a standalone sourcer never set it. Mirror
        # that: seed stringified (bash `printf '%s'`), counter starts at 0.
        self.seed = str(seed)
        self.ctr = int(ctr)

    def u32(self) -> int:
        """Advance the stream by one; return the draw in [0, 2^32).

        Mirrors next_u32: the value uses the PRE-increment counter, then the
        counter advances by exactly 1 (one stream position per draw).
        """
        s = hashlib.sha256(f"{self.seed}:{self.ctr}".encode()).hexdigest()[:8]
        self.ctr += 1
        return int(s, 16)

    def mod(self, m: int) -> int:
        """Uniform draw in [0, m) — consumes exactly one stream position.

        Mirrors next_mod: draw a u32, then reduce mod m; m <= 0 yields 0
        (byte-identical to bash's ``if (( m > 0 )); then ... else 0``).
        """
        v = self.u32()
        return v % m if m > 0 else 0
