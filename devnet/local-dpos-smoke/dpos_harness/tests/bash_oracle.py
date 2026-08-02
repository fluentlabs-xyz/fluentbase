"""bash_oracle.py — the frozen cross-language pin against `scripts/soak-prng.sh`.

Two tests (`test_prng.py`, `test_replay.py`) exist to prove the Python PRNG and the Python
round-decision layer are bit-identical to the bash the port replaced. They did that by shelling
out to the real `scripts/soak-prng.sh` on every run. That script is scheduled for deletion (P6),
and a `skipif(not bash_available)` test degrades to SKIPPED the day it goes — silently, with a
green suite.

So the streams are FROZEN here as checked-in fixtures, captured while the script still exists,
and the tests assert Python == fixture UNCONDITIONALLY. The live bash comparison stays too, but
now it asserts BASH == FIXTURE: a fixture is only worth something if the thing it was generated
from confirms it, and that confirmation has to happen before the thing disappears. When
`soak-prng.sh` goes, the fixture assertions keep running and the confirmation step skips.

Regenerate (only ever against a live `soak-prng.sh`, and only if the bash oracle itself changed):

    python3 -m dpos_harness.tests.bash_oracle

═══════════════════════════════════════════════════════════════════════════════════════════════
THE ONE DECLARED EXCEPTION TO THE `SOAK_*` / `LH_*` NAMING BAN.

`test_no_legacy_env_names.py` scans every module in the package for the pre-rename env prefixes
and fails on a survivor. This file is its single allow-listed module, and `BASH_ENV_NAMES` below
is the exhaustive list of what it is allowed to hold: the names belong to the BASH script we
shell out to, not to us. Renaming them here would not rename them in `soak-prng.sh` — it would
just break the oracle. Nothing else in the package may name them.
═══════════════════════════════════════════════════════════════════════════════════════════════
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess

# The bash script's own globals. See the banner above: this tuple IS the exception list, and the
# naming test reads it from here rather than carrying a hand-written copy.
BASH_ENV_NAMES = ("SOAK_SEED", "SOAK_RAND", "SOAK_PRNG_CTR")

_HERE = os.path.dirname(os.path.abspath(__file__))
# DEPTH-SENSITIVE: <smoke>/dpos_harness/tests/bash_oracle.py → two dirnames up is the smoke dir.
SMOKE_DIR = os.path.normpath(os.path.join(_HERE, "..", ".."))
SCRIPTS_DIR = os.path.join(SMOKE_DIR, "scripts")
PRNG_SH = os.path.join(SCRIPTS_DIR, "soak-prng.sh")
FIXTURE_DIR = os.path.join(_HERE, "fixtures", "bash-oracle")
PRNG_FIXTURE = os.path.join(FIXTURE_DIR, "prng-streams.json")
ROUND_FIXTURE = os.path.join(FIXTURE_DIR, "round-schedules.json")

# The parameter sets the two tests pin. Held here so the fixture and the tests can never drift
# apart: regenerating reads this list, and each test iterates the fixture it produced.
PRNG_SEEDS = ("42", "0", "123456789", "soak-live-v60")
PRNG_DRAWS = 8
PRNG_MOD = 7

ROUND_CASES = (
    # seed, ncommittee, nactions, calm_fraction, rounds
    ("20260717", 7, 7, 0.4, 40),
    ("20260717", 7, 5, 0.4, 40),   # byzantine OFF (5-action pool) — modulus changes, must match
    ("42", 10, 7, 0.5, 40),
    ("cascadefull1", 13, 7, 0.4, 40),
    ("0", 4, 7, 0.4, 40),          # minimal committee, no eligible victims (v0/v1) → <none>
)


def bash_available() -> bool:
    """True while a real bash AND a real soak-prng.sh are on disk. False after P6 deletes it."""
    return bool(shutil.which("bash")) and os.path.isfile(PRNG_SH)


# ── the two bash drivers ──────────────────────────────────────────────────────────────────────

_PRNG_DRIVER = r'''
source "$PRNG_SH"
export SOAK_SEED="$1"
for ((i=0;i<$2;i++)); do next_u32; printf '%s\n' "$SOAK_RAND"; done
next_mod "$3"; printf 'MOD %s\n' "$SOAK_RAND"
'''

# Reproduces case-soak.sh's per-round decision (case-soak.sh:2857-2888 — the 4-draw block + calm
# bit + empty-pool rule) with docker stubbed out, sourcing the REAL PRNG.
_ROUND_DRIVER = r'''
set -euo pipefail
SOAK_SEED="$1"; ncommittee="$2"; nactions="$3"; calm_fraction="$4"; rounds="$5"
source "$PRNG_SH"
ACTIONS=(graceful_stop_restart sigkill_restart cpu_throttle dkg_midwindow_restart delegate_shift)
(( nactions == 7 )) && ACTIONS+=(byzantine_equivocate byzantine_forge_pk)
_vpool=(); for ((i=2;i<ncommittee;i++)); do _vpool+=("validator-$i"); done
calm_permille=$(awk -v f="$calm_fraction" 'BEGIN{printf "%d", f*1000}')
for ((r=1;r<=rounds;r++)); do
    cur_epoch=$r
    next_u32; delay=$SOAK_RAND
    next_mod "${#ACTIONS[@]}"; aidx=$SOAK_RAND
    next_mod "${#_vpool[@]}"; vidx=$SOAK_RAND
    next_u32; aparam=$SOAK_RAND
    action="${ACTIONS[$aidx]}"
    if (( ${#_vpool[@]} > 0 )); then victim="${_vpool[$vidx]}"; else victim="<none>"; fi
    is_calm=0
    if (( cur_epoch <= 2 )); then is_calm=1
    else
        cb=$(( $(printf '%d' "0x$(printf '%s' "$SOAK_SEED:calm:$cur_epoch" | sha256sum | cut -c1-8)") % 1000 ))
        (( cb < calm_permille )) && is_calm=1
    fi
    printf '%s %s %s %s\n' "$action" "$victim" "$is_calm" "$aparam"
done
'''


def _run(driver, args):
    env = dict(os.environ, PRNG_SH=PRNG_SH)
    return subprocess.run(
        ["bash", "-c", driver, "driver", *[str(a) for a in args]],
        cwd=SMOKE_DIR, env=env, capture_output=True, text=True, check=True).stdout


def bash_prng_stream(seed, draws=PRNG_DRAWS, mod=PRNG_MOD):
    """The live bash PRNG: `draws` u32 values then one next_mod. -> {"u32": [...], "mod": int}."""
    out = _run(_PRNG_DRIVER, [seed, draws, mod]).split()
    assert out[draws] == "MOD", f"unexpected bash PRNG output: {out!r}"
    return {"u32": [int(x) for x in out[:draws]], "mod": int(out[draws + 1])}


def bash_round_schedule(seed, ncommittee, nactions, calm, rounds):
    """The live bash round decisions -> [[action, victim, is_calm, aparam], …]."""
    out = _run(_ROUND_DRIVER, [seed, ncommittee, nactions, calm, rounds])
    return [ln.split() for ln in out.splitlines()]


# ── the frozen fixtures ───────────────────────────────────────────────────────────────────────

def _load(path):
    with open(path) as f:
        return json.load(f)


def prng_fixture():
    return _load(PRNG_FIXTURE)["streams"]


def round_fixture():
    return _load(ROUND_FIXTURE)["cases"]


def fixture_key(case):
    """The fixture key for a ROUND_CASES entry — stable, filename-safe, and the pytest id."""
    seed, ncommittee, nactions, calm, rounds = case
    return f"{seed}-{ncommittee}-{nactions}-{calm}-{rounds}"


def regenerate():
    """Capture both streams from the live bash into the fixture files."""
    if not bash_available():
        raise SystemExit(f"refusing to regenerate: no live bash oracle at {PRNG_SH}")
    os.makedirs(FIXTURE_DIR, exist_ok=True)
    note = ("Captured from the live scripts/soak-prng.sh. Do NOT hand-edit; regenerate with "
            "`python3 -m dpos_harness.tests.bash_oracle` while the script still exists.")

    streams = {s: bash_prng_stream(s) for s in PRNG_SEEDS}
    with open(PRNG_FIXTURE, "w") as f:
        json.dump({"_note": note, "source": "scripts/soak-prng.sh", "draws": PRNG_DRAWS,
                   "mod": PRNG_MOD, "streams": streams}, f, indent=2, sort_keys=True)
        f.write("\n")

    cases = []
    for c in ROUND_CASES:
        seed, ncommittee, nactions, calm, rounds = c
        cases.append({"id": fixture_key(c), "seed": seed, "ncommittee": ncommittee,
                      "nactions": nactions, "calm": calm, "rounds": rounds,
                      "schedule": bash_round_schedule(*c)})
    with open(ROUND_FIXTURE, "w") as f:
        json.dump({"_note": note, "source": "scripts/soak-prng.sh + the case-soak.sh:2857-2888 "
                                            "round-decision block, reproduced in bash_oracle.py",
                   "cases": cases}, f, indent=2)
        f.write("\n")
    print(f"wrote {PRNG_FIXTURE}\nwrote {ROUND_FIXTURE}")


if __name__ == "__main__":
    regenerate()
