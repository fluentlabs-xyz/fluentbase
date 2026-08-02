"""setops.py — substring-safe space-set primitives + the committee victim-draw.

Python port of scripts/committee-set.sh. A "set" here is the bash space-separated
word-list string (SIM_DISRUPTED / PROMOTABLE / SIM_CUR_COMMITTEE), kept as a str
so the port stays byte-faithful to how the orchestrator stores it; every operation is
WHOLE-WORD (the `validator-1` vs `validator-10` substring-bug class dies here, exactly
as the bash module intends).

PORT-NOTE: bash open-coded these because `${var//word/}` is a substring replace that
mangles validator-1 inside validator-10; in Python `str.split()` is already whole-word
so the hazard cannot exist — but the API is preserved 1:1 so call sites read identically
to the bash (`_in_set`, `_count`, `_intersect`, `_set_remove`).

This is the ONE home for these primitives. `checks/battery.py`, `sim/hostpool.py` and
`sim/reconcilers.py` each carried a private `_in_set` copy (an artefact of the observer/active
port split, whose rationale was "independent ports of one bash module"); all three now import
`in_set` from here. There is no import-cycle reason to keep a copy — `core` sits below every
consumer — and three copies of a whole-word membership test is three places for the
`validator-1` vs `validator-10` substring bug to come back.
"""

from __future__ import annotations

from . import topology


def count(s: str) -> int:
    """_count: cardinality of a space-set."""
    return len((s or "").split())


def in_set(e: str, s: str) -> bool:
    """_in_set: whole-word membership of e in space-set s."""
    return e in (s or "").split()


def intersect(a: str, b: str) -> str:
    """_intersect: words of a also in b, space-joined with a trailing space (bash echoes
    `out+="$w "`). The trailing space is harmless for a word-set and preserved so any
    downstream length/format assertion matches bash byte-for-byte."""
    bs = set((b or "").split())
    out = "".join(w + " " for w in (a or "").split() if w in bs)
    return out


def set_remove(e: str, s: str) -> str:
    """_set_remove: space-set s with element e removed (whole-word), trailing space kept
    (bash `out+="$w "`). No-op if e absent."""
    return "".join(w + " " for w in (s or "").split() if w != e)


def committee_victim_idxs(committee: str, addr2idx: dict, disrupted: str = "",
                          skip_disrupted: bool = False):
    """committee_victim_idxs: yield each eligible fault/probe victim as `validator-N`, in
    committee order. For each owner-addr in the space-set `committee`, resolve its container
    via ADDR2IDX and SKIP an unmapped addr, the pinned host-RPC anchor validator-0, AND
    validator-1 (FLK-8: v1 is the pinned upstream anchor — v0's hardcoded WS upstream and the
    full-node's 2nd cert-upstream — never a churn victim). With skip_disrupted, also skip any
    container currently in SIM_DISRUPTED.

    Reads ADDR2IDX / SIM_DISRUPTED but NEVER the PRNG — the bash runs it in a process
    substitution precisely because it consumes no draw, so the load-bearing 4-draw discipline
    stays in the caller. This port returns a list (the caller's `mapfile -t`)."""
    out = []
    for a in (committee or "").split():
        ci = addr2idx.get(a)
        if not ci or topology.is_anchor(ci):
            continue
        if skip_disrupted and in_set(ci, disrupted):
            continue
        out.append(ci)
    return out
