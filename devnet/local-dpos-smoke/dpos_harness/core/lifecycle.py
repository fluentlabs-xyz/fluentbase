"""lifecycle.py — the two graceful-stop barriers, shared by every consumer that stops a node.

`shutdown_flushed` (lib.sh:66-84) and the marshal ack-death tripwire (lib.sh:118-138) are
cluster-B lifecycle helpers, and BOTH the simulation (`sim/actions.py`, on its churn
actuators) and the static smoke stack (`stack/static_stack.py`, at the migration flush gate)
need them. They lived as private methods on `sim.Actuators`; the detection logic now lives
here ONCE and the two consumers wrap it, so the barrier cannot drift between them — which is
the shape of bug this file is defending against (the bash call sites had to repeat the
cursor→stop→flushed→tripwire steps by hand at every site, and one of them didn't).

WHAT IS HERE AND WHAT IS NOT: this module owns the reads, the poll budgets and the VERDICT.
It does not raise, and it does not decide what a verdict means. `sim.Actuators` turns a trip
into a `ChainError` and records a `diag` event; the static stack turns a failed flush into a
retry. Keeping the policy at the consumer is also what lets this module sit in `core` at all
(`core` may not import `chain`, so it could not raise `ChainError` even if it wanted to).

═══ TWO DIAGNOSED BUGS ARE ENCODED HERE ═════════════════════════════════════════════════

1. **Lifecycle state comes from `docker inspect`, never from logs.** `shutdown_flushed` WAS a
   `docker compose logs | grep`, and it raced: under heavy docker-daemon load the log read
   lagged past the poll window and false-reported "flush incomplete" for a node that had
   exited 0 in under a second (diagnosed 2026-06-05; migration-flush-diag.txt). Container
   metadata does not lag that way. Porting this back to a log grep re-creates the bug.
2. **An empty log slice is a lagging read, not a clean one.** A graceful stop always logs
   something, so the tripwire re-reads the `--since` slice while it comes back EMPTY
   (`ACK_READ_RETRIES`) before concluding anything. Accepting the first empty read would
   blind the tripwire under exactly the daemon load that makes a marshal death likely.
"""

from __future__ import annotations

import time
from datetime import datetime, timezone

#: `docker inspect` poll budget for the flush barrier — 10 attempts, 1s apart (lib.sh:77).
FLUSH_POLLS = 10
FLUSH_POLL_S = 1

#: Empty-slice re-read budget for the tripwire — 5 attempts, 1s apart (lib.sh:120).
ACK_READ_RETRIES = 5
ACK_READ_SLEEP_S = 1

#: The line the commonware marshal logs FIRST when it dies. The marshal's run() returns the
#: moment any `Exact` ack is dropped, and the node degrades to a zombie that serves no
#: blocks and no certs. On a GRACEFUL stop this line must be ABSENT: commonware's teardown
#: drops the executor/marshal unpolled, so the marshal never observes the cancellation.
ACK_DEATH = "application did not acknowledge block"

#: NON-graceful paths on which the ack line is LEGITIMATE, and which therefore scope the trip
#: out. `AbortAll` (a subsystem exit) aborts the executor one statement before the marshal in
#: outer.rs, so a multi-threaded runtime can poll the marshal in the gap — discriminated by
#: the supervisor's own exit lines. A deliberate SafetyHalt is discriminated by the executor's.
#: All three are literal SUBSTRINGS in bash too — `\(unexpected\)` there is an escaped paren,
#: not a regex group, so nothing is lost by matching plainly.
ACK_SCOPE_OUT = ("SafetyHalt", "subsystem failed", "subsystem exited cleanly (unexpected)")

#: The three `ack_verdict` outcomes. Only TRIPPED is a finding.
CLEAN = "clean"
SCOPED_OUT = "scoped-out"
TRIPPED = "tripped"


def log_cursor() -> str:
    """`date -u +%Y-%m-%dT%H:%M:%SZ` — the cursor handed to `docker logs --since`.

    The format is load-bearing, not cosmetic: docker compares it against DAEMON-side host
    timestamps, so it must be UTC RFC3339 to the second. The caller captures it IMMEDIATELY
    BEFORE issuing the stop, so the tripwire greps only THIS stop's slice and an unrelated
    halt/crash line hours earlier in the container log can neither trip it nor scope it out.
    """
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def cid_any_argv(service: str):
    """`docker compose ps -aq <service>` — the container id INCLUDING a stopped one.

    Deliberately `-aq`, not `-q`. `-q` is RUNNING-only and would resolve nothing for a
    container that has just stopped, so the barrier would report "no container" for every
    graceful stop — a False that looks like a real finding.
    """
    return ["docker", "compose", "ps", "-aq", service]


def inspect_state_argv(cid: str):
    """`docker inspect <cid> --format '{{.State.Running}} {{.State.ExitCode}}'` (lib.sh:78)."""
    return ["docker", "inspect", cid, "--format", "{{.State.Running}} {{.State.ExitCode}}"]


def shutdown_flushed(service: str, read, sleep=None) -> bool:
    """True iff `service`'s container is observed EXITED with code 0 within the poll budget.

    A BARRIER, not an assertion: it delays the caller until the container really exited, so a
    restart cannot race the persist. A graceful `docker compose stop` lets reth's own
    `on_graceful_shutdown` persist the in-memory tail to MDBX before the container exits 0; a
    SIGKILL on the 40s stop-timeout exits 137 instead. `read(argv) -> str` is the caller's
    read seam.

    False when the container id is empty or the window expires. Both bash call sites do
    `|| true`, so a False is NOT fatal by contract — it means the caller proceeds without
    proof of a clean exit, which is worth RECORDING (the sim does) but not aborting on.

    Port delta from bash: bash sleeps once more after its final poll (a 1s no-op before
    `return 1`); this skips the trailing sleep.
    """
    sleep = sleep or time.sleep
    cid = read(cid_any_argv(service)).strip()
    if not cid:
        return False
    for i in range(FLUSH_POLLS):
        state = read(inspect_state_argv(cid)).split()
        if state[:2] == ["false", "0"]:
            return True
        if i < FLUSH_POLLS - 1:
            sleep(FLUSH_POLL_S)
    return False


def ack_slice(service: str, since: str, logs_since, sleep=None) -> str:
    """The `--since` log slice for THIS stop, re-read while it comes back EMPTY.

    `logs_since(service, since) -> str` is the caller's (ANSI-stripped) log reader. Returns
    the first non-empty slice, or "" after `ACK_READ_RETRIES` attempts. An unreadable log and
    a genuinely empty slice are indistinguishable here and both end as "" — which is correct,
    because `ack_verdict("")` is CLEAN: the tripwire must never trip on ABSENCE.
    """
    sleep = sleep or time.sleep
    logs = ""
    for i in range(ACK_READ_RETRIES):
        logs = logs_since(service, since)
        if logs:
            break
        if i < ACK_READ_RETRIES - 1:
            sleep(ACK_READ_SLEEP_S)
    return logs


def ack_verdict(logs: str) -> str:
    """CLEAN / SCOPED_OUT / TRIPPED for a log slice. Order is bash's and matters: the ack line
    must be present at all before the scope-out is even consulted, so that a slice carrying a
    SafetyHalt and NO ack line reports CLEAN rather than SCOPED_OUT — the two are different
    facts and only one of them is about the marshal.
    """
    if ACK_DEATH not in (logs or ""):
        return CLEAN
    if any(marker in logs for marker in ACK_SCOPE_OUT):
        return SCOPED_OUT
    return TRIPPED


def ack_tripwire_message(service: str) -> str:
    """The tripwire's finding text (bash lib.sh:135-136, verbatim shape)."""
    return (f"TRIPWIRE ({service}): marshal logged '{ACK_DEATH}' on a graceful, unhalted "
            "stop — an Exact ack was dropped somewhere (marshal-death bug)")
