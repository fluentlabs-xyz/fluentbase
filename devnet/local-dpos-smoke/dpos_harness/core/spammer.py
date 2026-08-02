"""spammer.py — `pp_spammer_start` / `pp_spammer_stop` (lib.sh:1208-1233).

THE ONLY BACKGROUND JOB IN THE BASH TREE, and it is hidden in a library. Six cases depend on
it and none of them mentions backgrounding (§2.4 item 1):

    pp_spammer_start() {
        local from_key="$1" to_addr="$2" rpc="${3:-$RPC}"
        ( while :; do timeout 90 cast send "$to_addr" --value 1 \
              --rpc-url "$rpc" --private-key "$from_key" >/dev/null 2>&1 || true
            sleep 2; done ) &
        PP_SPAMMER_PID=$!
        PP_SPAMMER_PIDS+=("$!")
    }
    pp_spammer_stop() {
        for p in "${PP_SPAMMER_PIDS[@]}"; do [[ -n "$p" ]] && kill "$p" 2>/dev/null || true; done
        PP_SPAMMER_PIDS=(); PP_SPAMMER_PID=""
    }

Four properties are load-bearing and each is preserved below:

  * **`stop()` reaps EVERY spammer ever started, not the last one.** `PP_SPAMMER_PIDS` is an
    ARRAY and `PP_SPAMMER_PID` is only the most recent handle; `case-byzantine-vrf` starts two
    (the committee spammer and a second one after its own rotation bring-up) and its single
    `pp_spammer_stop` in the EXIT trap has to kill both. A pool that tracked one handle would
    leak a `cast send` loop against a torn-down RPC — the 2026-07-14 3.5-hour orphan incident.
  * **the sender key must issue no other transactions.** bash says so in its header: a shared
    key makes the spammer's nonce race the deploy/registration txs (`replacement transaction
    underpriced`). Nothing here can enforce it; the callers derive a dedicated mnemonic index.
  * **`timeout 90` is a hang guard, not a budget** (§2.4 item 14). A follower wedge must make
    the loop RETRY, not hang; the ceiling rides as the seam's `timeout=` (see `proc.py`'s
    PORT-NOTE, which drops the `timeout(1)` binary for subprocess's own kwarg).
  * **a non-default `rpc` routes tx pressure THROUGH the cascade.** Submitting to an L3/L2
    follower makes it gossip L3→L2→validator into the proposer pool, which is the whole point
    of the cascade cases' spammer. Defaulting it here would collapse that into the producer.

PORT-NOTE (threads, not process groups): bash forked a subshell and kept its PID. Here each
spammer is a DAEMON THREAD and `stop()` sets one `Event` — we own our own handles, so there is
no `pkill -f` pattern to self-match (the incident that memory entry is about) and no orphaned
process group if the interpreter dies.

PORT-NOTE (the ambient channel): the sends go through the exec seam on a NON-RECORDING runner.
They are writes and would normally belong in the transcript, but the loop issues one every two
seconds for the whole run: recording it would grow `.log` without bound AND destroy the one
artifact that proves a refactor changed nothing. Executing through the seam is what matters —
the ambient `COMPOSE_FILE`/env merge and the wall ceiling are the seam's, not re-implemented
per thread.
"""

from __future__ import annotations

import threading

from . import proc

#: `timeout 90 cast send …` (lib.sh:1220) — the per-send hang guard.
SEND_TIMEOUT_S = 90
#: `sleep 2` (lib.sh:1222) — the inter-send pause. NOT a knob: it is what makes a synchronous
#: `cast send` self-pace its own nonce.
SEND_PERIOD_S = 2
#: `stop()`'s per-thread join budget. A thread parked in a 90 s `cast send` is not waited out —
#: it is a daemon and the send is idempotent pressure, so the process may exit around it. Waiting
#: the full ceiling would make every teardown 90 s slower for no gain.
JOIN_TIMEOUT_S = 3


class SpammerPool:
    """The `PP_SPAMMER_PIDS[]` array, as an object. `start()` appends, `stop()` reaps all."""

    def __init__(self, dry: bool = False):
        self.dry = dry
        self._stop = threading.Event()
        self.threads: list = []
        #: `PP_SPAMMER_PID` — the MOST RECENT handle. Kept because bash's own bring-up prints it
        #: ("tx spammer started (pid …)"); it is never what `stop()` iterates.
        self.last = None
        self.p = proc.Runner(dry=dry, record=False)

    def start(self, from_key: str, to_addr: str, rpc: str, note: str = "spammer"):
        """`pp_spammer_start <from_key> <to_addr> [rpc]`. Returns the thread (bash's `$!`).

        Dry: records NOTHING and starts NOTHING. The transcript is the write choreography, and a
        thread that fired a `cast send` under `--dry-run` would be a live tx from a dry run."""
        if self.dry:
            return None
        t = threading.Thread(target=self._loop, args=(from_key, to_addr, rpc, note), daemon=True)
        t.start()
        self.threads.append(t)
        self.last = t
        return t

    def _loop(self, from_key, to_addr, rpc, note):
        while not self._stop.is_set():
            # `|| true`: a transient follower wedge just retries on the next tick, exactly as
            # bash's swallowed failure did. run_ok reports rc and discards it.
            self.p.run_ok(["cast", "send", to_addr, "--value", "1", "--rpc-url", rpc,
                           "--private-key", from_key], timeout=SEND_TIMEOUT_S)
            if self._stop.wait(SEND_PERIOD_S):
                break

    def stop(self):
        """`pp_spammer_stop` — reap EVERY spammer this pool ever started, then empty the array.

        Idempotent and never raises: bash's `kill … 2>/dev/null || true` runs from an EXIT trap,
        where a failure to reap must not turn a passing case into a failing one."""
        self._stop.set()
        for t in self.threads:
            t.join(timeout=JOIN_TIMEOUT_S)
        self.threads = []
        self.last = None

    @property
    def running(self) -> int:
        """How many spammer threads this pool is holding. `stop()` empties it; a case that
        asserts tx pressure was PRESENT reads this rather than guessing from the chain."""
        return len(self.threads)
