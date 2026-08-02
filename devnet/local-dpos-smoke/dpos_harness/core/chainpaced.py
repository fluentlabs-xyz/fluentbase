"""chainpaced.py — the one chain-paced deadline primitive. Python port of lib.sh's
chain_paced_step / wait_chain_paced (801-873) + _cp_cursor / _cp_attribute.

A pure, resumable verdict machine (step) plus a thin blocking wrapper (wait) that together
replace hand-rolled wall-clock-vs-{block,epoch,progress} conversions. The CALLER evaluates its
own <met> condition; the primitive owns the domain-cursor read, the entry-cursor latch, the
budget arithmetic, and the chain-frozen escape. Used for the gov-confirm + activation waits
(bundle-20260716T232341Z — the wall-clock-vs-chain family: under a blaster-degraded rate the
block-denominated voting window crawls, so the deadline MUST be measured in observed chain
blocks, not seconds).

Seams (test/dry overrides): the domain cursor is read via block_number (blocks), current_epoch
(epochs) or now (wall); the `ticks` cursor is an internal per-key counter the caller resets on
its OWN progress signal. The chain-frozen escape reads head_age_s (blocks/epochs only).

PORT-NOTE: the bash primitive documents "MUST run in the current shell — NEVER $(...)" because a
subshell would drop its CP_* mutations. In Python the state is instance attributes; the whole
hazard is gone. The set-e `&&`-branch-tail traps it dodges are likewise structural non-issues.
"""

from __future__ import annotations

import sys
import time


def _stderr_warn(note: str) -> None:
    """bash: `echo "WARN (chain-paced): $note" >&2`."""
    print(f"WARN (chain-paced): {note}", file=sys.stderr, flush=True)


class ChainPaced:
    def __init__(self, block_number=None, current_epoch=None, head_age_s=None, now=None,
                 sleep=None, dry=False, events=None, warn=None):
        self._block_number = block_number or (lambda: None)
        self._current_epoch = current_epoch or (lambda: None)
        self._head_age_s = head_age_s or (lambda: None)
        self._now = now or time.time
        self._sleep = sleep or (lambda s: time.sleep(s))
        self.dry = dry
        self.events = events if events is not None else []
        # bash `_cp_attribute` (lib.sh:869) emits the attribution through `soak_event WARN` when
        # the sim's bundle writer is sourced, and FALLS BACK to a plain stderr line otherwise —
        # "the lib-only cases (production-path/vrf-*) have no soak_event, so a budget/frozen
        # verdict there is still surfaced". The first port kept only the list, which no lib-only
        # caller reads and, as it happens, no sim caller reads either: `Chain` constructs its
        # ChainPaced with no `events`, so every attribution this machine has ever produced went
        # into a list that is never inspected. The stderr leg is restored as the DEFAULT, which is
        # bash's fallback; a caller with a real event sink passes `warn=`.
        self._warn = warn if warn is not None else _stderr_warn

        self.cp_start = {}     # per-key entry cursor
        self.cp_tick = {}      # per-key internal tick counter
        self.verdict = ""
        self.detail = ""
        self.fail_msg = ""

    def _cursor(self, domain, key):
        if domain == "blocks":
            v = self._block_number()
        elif domain == "epochs":
            v = self._current_epoch()
        elif domain == "wall":
            v = self._now()
        elif domain == "ticks":
            return self.cp_tick.get(key, 0)
        else:
            v = None
        try:
            return int(v) if v is not None and str(v).strip() != "" else None
        except (TypeError, ValueError):
            return None

    def reset(self, key):
        self.cp_start.pop(key, None)

    def step(self, key, met, domain, budget, froz=120):
        """chain_paced_step: sets self.verdict ∈ met|waiting|budget|frozen|read_fail and returns
        it. <met> is 0|1 (the caller's own condition). Dry-run → waiting (no RPC)."""
        self.verdict = ""
        self.detail = ""
        if self.dry:
            self.verdict = "waiting"
            return self.verdict
        if domain == "ticks":
            self.cp_tick[key] = self.cp_tick.get(key, 0) + 1
        cur = self._cursor(domain, key)
        if cur is None:
            self.verdict = "read_fail"        # RPC/read problem — NEVER "the thing didn't happen"
            return self.verdict
        if key not in self.cp_start:
            self.cp_start[key] = cur           # latch the entry cursor on first measurable call
        adv = cur - self.cp_start[key]
        if adv < 0:
            adv = 0
        self.detail = (f"domain={domain} budget={budget} entry={self.cp_start[key]} "
                       f"cursor={cur} advanced={adv}")
        if met == 1:
            self.verdict = "met"
            self.reset(key)
            self.cp_tick.pop(key, None)
            return self.verdict
        if adv >= budget:
            self.verdict = "budget"            # honest failure: chain made the agreed progress, unmet
            return self.verdict
        if froz > 0 and domain in ("blocks", "epochs"):
            age = self._head_age_s()
            try:
                age = int(age)
            except (TypeError, ValueError):
                age = None
            if age is not None and age >= froz:
                self.verdict = "frozen"
                self.detail += f" head_age={age}s"
                return self.verdict
        self.verdict = "waiting"
        return self.verdict

    def _attribute(self, key, verdict, detail):
        if verdict == "budget":
            note = (f"chain-paced '{key}': condition never met while the chain advanced the agreed "
                    f"budget ({detail})")
        elif verdict == "frozen":
            note = (f"chain-paced '{key}': chain FROZEN ({detail}) — deferring to the finalize-stall"
                    " / node-down invariants that own a stalled chain")
        elif verdict == "read_fail":
            note = (f"chain-paced '{key}': domain cursor UNREADABLE ({detail}) — an RPC/read failure,"
                    " NOT a missed condition")
        else:
            note = f"chain-paced '{key}': {verdict} ({detail})"
        self.fail_msg = note
        self.events.append(("WARN", note))
        self._warn(note)

    def wait(self, key, cond_fn, domain, budget, froz=120, poll=2):
        """wait_chain_paced: loop step over cond_fn (returns True when met); True on `met`. On
        budget/frozen emit the shared attribution and return False. A transient read_fail is
        tolerated (keep polling) — only a PERSISTENT one past the same guard terminates."""
        if self.dry:
            return True
        self.reset(key)
        self.cp_tick.pop(key, None)
        while True:
            met = 1 if cond_fn() else 0
            self.step(key, met, domain, budget, froz)
            if self.verdict == "met":
                return True
            if self.verdict in ("budget", "frozen"):
                self._attribute(key, self.verdict, self.detail)
                return False
            # read_fail → transient; keep polling
            self._sleep(poll)
