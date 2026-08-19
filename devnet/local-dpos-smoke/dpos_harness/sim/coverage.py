"""coverage.py — the per-class action tally: what the run actually EXERCISED.

WHY IT EXISTS: a soak run had no coverage accounting at all. The round verdict was returned by
`dispatch.run_round` and dropped by the caller, and nothing counted actions, so a nine-hour run in
which every round returned "skipped" printed exactly what a run that exercised every branch
printed. The audit's central claim — that whole action classes never fire in a live run — was a
reachability argument no run could confirm or refute. This module is the instrument that makes the
run testify.

WHAT IT COUNTS, and why the three columns are not interchangeable:
  * drawn   — the lottery selected the class. Says the schedule offered it.
  * applied — the action TOOK EFFECT. Recorded at the apply seam, never at selection.
  * skipped — it did not, with the reason, so "never fired" separates into "never drawn",
              "drawn and always gated", and "fired and the chain write never landed".

The class table is PRE-SEEDED with every known class (`ALL_CLASSES`), so a class that never fires
reports `applied: 0` instead of being absent from the report. An absent key reads as "not
instrumented"; an explicit zero is the finding.

PURE: no I/O, no imports from the package. The emitters (orchestrator) render it; nothing here
decides anything, and no counter is ever read by a gate, a predicate or a detector.
"""

from __future__ import annotations

# The fault-lottery classes: the full pool `SimConfig.actions_pool` can produce under ANY flag
# combination (byzantine / voluntary_exit are conditional appends), listed unconditionally so a
# run with the flag off still reports the class as an explicit zero.
LOTTERY_CLASSES = (
    "graceful_stop_restart", "sigkill_restart", "cpu_throttle", "dkg_midwindow_restart",
    "delegate_shift", "byzantine_equivocate", "voluntary_exit",
)

# The membership TRACKS. They are not drawn from the lottery — the dispatcher fires them from its
# own gate chain — so their `drawn` column stays 0 by construction and only `applied`/`skipped`
# carry meaning for them.
TRACK_CLASSES = ("register_activate", "refill", "bench_promote", "bench_join", "provision_bench")

ALL_CLASSES = LOTTERY_CLASSES + TRACK_CLASSES


def gate_reason_key(reason: str) -> str:
    """Bound the cardinality of a gate rejection reason. `gate_accept` returns e.g.
    "rule1-transient-quorum (effective 4 > f=3)" — the parenthetical carries live numbers, so
    keying on the whole string would make every trip its own bucket and the tally unreadable."""
    return (reason or "unknown").split(" (")[0].strip()


class Tally:
    """Counters only. Constructed by the Dispatcher, read by the orchestrator's reporter."""

    def __init__(self, classes=ALL_CLASSES):
        self.rounds = {}                 # verdict string -> rounds that returned it
        self.phase_rounds = {"growth": 0, "rotation": 0}
        self.round_skips = {}            # round-level gate reason -> count (pre-lottery holds)
        self.classes = {c: {"drawn": 0, "applied": 0, "skipped": {}} for c in classes}

    def _slot(self, cls: str):
        s = self.classes.get(cls)
        if s is None:
            s = self.classes[cls] = {"drawn": 0, "applied": 0, "skipped": {}}
        return s

    def draw(self, cls: str):
        self._slot(cls)["drawn"] += 1

    def fire(self, cls: str):
        self._slot(cls)["applied"] += 1

    def skip_action(self, cls: str, reason: str):
        sk = self._slot(cls)["skipped"]
        sk[reason] = sk.get(reason, 0) + 1

    def skip_round(self, reason: str):
        """A round-level hold that never reached the lottery, so it belongs to no class."""
        self.round_skips[reason] = self.round_skips.get(reason, 0) + 1

    def round_verdict(self, rotation_phase, verdict: str):
        v = verdict if isinstance(verdict, str) else str(verdict)
        self.rounds[v] = self.rounds.get(v, 0) + 1
        self.phase_rounds["rotation" if rotation_phase else "growth"] += 1

    # ── rendering ─────────────────────────────────────────────────────────────
    def totals(self):
        drawn = sum(c["drawn"] for c in self.classes.values())
        applied = sum(c["applied"] for c in self.classes.values())
        skipped = sum(sum(c["skipped"].values()) for c in self.classes.values())
        return {"drawn": drawn, "applied": applied, "skipped": skipped,
                "rounds": sum(self.rounds.values()),
                "classes_fired": sum(1 for c in self.classes.values() if c["applied"] > 0),
                "classes_total": len(self.classes)}

    def snapshot(self, demoted=None) -> dict:
        """The machine-readable form written into events.jsonl. `demoted` is EventLog's
        demoted-trip map, carried here so one event line answers both "what fired" and "what
        tripped diagnostically" without a join across two streams."""
        return {
            "totals": self.totals(),
            "rounds": dict(self.rounds),
            "phase_rounds": dict(self.phase_rounds),
            "round_skips": dict(self.round_skips),
            "classes": {k: {"drawn": v["drawn"], "applied": v["applied"],
                            "skipped": dict(v["skipped"])}
                        for k, v in self.classes.items()},
            "demoted_trips": {k: dict(v) for k, v in (demoted or {}).items()},
        }

    def human_lines(self):
        """The operator-readable block. One line per class: drawn / applied / skip reasons."""
        t = self.totals()
        out = [f"ACTION COVERAGE : {t['classes_fired']}/{t['classes_total']} classes fired | "
               f"{t['applied']} applied, {t['drawn']} drawn, {t['skipped']} class-skips over "
               f"{t['rounds']} rounds"]
        rounds = ", ".join(f"{k}={v}" for k, v in sorted(self.rounds.items())) or "none"
        phases = ", ".join(f"{k}={v}" for k, v in sorted(self.phase_rounds.items()))
        out.append(f"  round verdicts: {rounds} | phase rounds: {phases}")
        if self.round_skips:
            holds = ", ".join(f"{k}={v}" for k, v in sorted(self.round_skips.items()))
            out.append(f"  round-level holds: {holds}")
        for cls in sorted(self.classes):
            c = self.classes[cls]
            why = ", ".join(f"{r}={n}" for r, n in sorted(c["skipped"].items()))
            mark = " " if c["applied"] else "!"   # `!` marks a class this run never exercised
            out.append(f"  {mark}{cls:<24} drawn={c['drawn']:<5} applied={c['applied']:<5}"
                       + (f" skipped[{why}]" if why else ""))
        return out
