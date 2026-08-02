"""`core/lifecycle.py` — the two graceful-stop barriers, now shared by the sim and the static
stack.

Both encode a diagnosed race, and both races are of the "read lagged, verdict was wrong" kind:

  * the flush barrier reads container METADATA. It WAS a `docker compose logs | grep`, and
    under heavy daemon load the log read lagged past the poll window and false-reported "flush
    incomplete" for a node that had exited 0 in under a second (2026-06-05);
  * the tripwire re-reads an EMPTY `--since` slice before concluding anything, because a
    graceful stop always logs something — an empty slice is a lagging read, not a clean one.

`sim/actions.py` already had these as private methods and its tests (test_apply.py) pin them
through the Actuators surface. Those tests are the reason this file does not restate them: what
is tested HERE is the shared implementation and the one thing the extraction could break — that
both consumers get the same argv, the same budgets and the same verdict vocabulary.
"""

from __future__ import annotations

import re

import pytest

from dpos_harness.core import lifecycle

_ACK = lifecycle.ACK_DEATH


# ── the flush barrier ────────────────────────────────────────────────────────

def test_flush_barrier_reads_metadata_and_uses_ps_aq():
    """Two things in one argv sequence. `ps -aq` (not `-q`) because `-q` is RUNNING-only and
    would resolve nothing for a container that has just stopped — the barrier would then report
    "no container" for every graceful stop. And `docker inspect`, never `logs`."""
    argv = []

    def read(cmd):
        argv.append(cmd)
        return "deadbeefcafe\n" if "ps" in cmd else "false 0\n"

    assert lifecycle.shutdown_flushed("validator-3", read) is True
    assert argv[0] == ["docker", "compose", "ps", "-aq", "validator-3"]
    assert argv[1] == ["docker", "inspect", "deadbeefcafe",
                       "--format", "{{.State.Running}} {{.State.ExitCode}}"]
    assert not any("logs" in c for c in argv), "the barrier went back to log-grepping"


def test_flush_barrier_requires_exited_AND_zero():
    """`false 0` only. A container still running, or exited non-zero (137 = SIGKILL at the
    stop-timeout, i.e. reth never got to persist), is not a flush."""
    for state in ("true 0", "false 137", "false 1", "true 137"):
        reads = iter(["cafe\n"] + [state + "\n"] * lifecycle.FLUSH_POLLS)
        assert lifecycle.shutdown_flushed("v", lambda _c: next(reads), sleep=lambda _s: None) \
            is False


def test_flush_barrier_is_false_without_a_container_id():
    assert lifecycle.shutdown_flushed("v", lambda _c: "", sleep=lambda _s: None) is False


def test_flush_barrier_polls_exactly_its_budget():
    """One cid read plus exactly `FLUSH_POLLS` inspects, with a sleep BETWEEN them and none
    after the last (the port's one deliberate delta from bash, which sleeps a final no-op)."""
    polls, slept = [], []

    def read(cmd):
        polls.append(cmd)
        return "cafe\n" if "ps" in cmd else "true 0\n"

    lifecycle.shutdown_flushed("v", read, sleep=lambda s: slept.append(s))
    assert len(polls) == 1 + lifecycle.FLUSH_POLLS
    assert slept == [lifecycle.FLUSH_POLL_S] * (lifecycle.FLUSH_POLLS - 1)


def test_flush_barrier_tolerates_a_garbled_inspect():
    """An unreadable/short `docker inspect` must not raise out of a barrier — bash's `read -r`
    would simply leave the vars empty. `state[:2]` on a 0- or 1-token split is the guard."""
    reads = iter(["cafe\n"] + [""] * lifecycle.FLUSH_POLLS)
    assert lifecycle.shutdown_flushed("v", lambda _c: next(reads), sleep=lambda _s: None) \
        is False


# ── the ack tripwire ─────────────────────────────────────────────────────────

def test_tripwire_verdict_vocabulary_is_three_valued():
    """CLEAN / SCOPED_OUT / TRIPPED are three different facts. Collapsing scoped-out into
    tripped fails every deliberate-halt case; collapsing it into clean throws away the reason."""
    assert lifecycle.ack_verdict("nothing interesting\n") == lifecycle.CLEAN
    assert lifecycle.ack_verdict(f"{_ACK} 4711\n") == lifecycle.TRIPPED
    assert len({lifecycle.CLEAN, lifecycle.SCOPED_OUT, lifecycle.TRIPPED}) == 3


@pytest.mark.parametrize("marker", lifecycle.ACK_SCOPE_OUT)
def test_tripwire_each_scope_out_marker_alone_suffices(marker):
    """The ack line IS legitimate when the stop was not graceful: an AbortAll subsystem exit
    (outer.rs aborts the executor one statement before the marshal, so a multi-threaded runtime
    can poll the marshal in the gap), or a SafetyHalt."""
    assert lifecycle.ack_verdict(f"{_ACK}\n{marker} whatever\n") == lifecycle.SCOPED_OUT


def test_tripwire_scope_out_without_the_ack_line_is_clean_not_scoped_out():
    """Order matters and this is why it is asserted. A slice carrying a SafetyHalt and NO ack
    line is CLEAN — the halt is a fact about the executor, not about the marshal, and reporting
    it as "scoped out" would imply a suppressed marshal finding that never existed."""
    assert lifecycle.ack_verdict("SafetyHalt: result divergence\n") == lifecycle.CLEAN


@pytest.mark.parametrize("logs", ["", None])
def test_tripwire_never_trips_on_absence(logs):
    """An unreadable log and a genuinely empty slice are indistinguishable and both end CLEAN.
    A tripwire that fired on absence would fail every case whose logs it could not read."""
    assert lifecycle.ack_verdict(logs) == lifecycle.CLEAN


def test_ack_slice_re_reads_while_empty():
    """A graceful stop ALWAYS logs something, so an empty slice means a lagging docker-logs
    read. Accepting the first empty read would blind the tripwire under exactly the daemon load
    that makes a marshal death likely."""
    reads, slept = [], []
    slices = iter(["", "", f"{_ACK}\n"])

    def logs_since(v, since):
        reads.append((v, since))
        return next(slices)

    got = lifecycle.ack_slice("validator-3", "2026-07-30T00:00:00Z", logs_since,
                              sleep=lambda s: slept.append(s))
    assert lifecycle.ack_verdict(got) == lifecycle.TRIPPED
    assert len(reads) == 3 and slept == [lifecycle.ACK_READ_SLEEP_S] * 2
    assert {since for _, since in reads} == {"2026-07-30T00:00:00Z"}, \
        "the cursor drifted between retries — the slice would stop being THIS stop's"


def test_ack_slice_gives_up_after_its_budget():
    reads = []
    got = lifecycle.ack_slice("v", "cursor", lambda *_a: reads.append(1) or "",
                              sleep=lambda _s: None)
    assert got == "" and len(reads) == lifecycle.ACK_READ_RETRIES


def test_ack_slice_stops_at_the_first_non_empty_slice():
    reads = []
    got = lifecycle.ack_slice("v", "cursor", lambda *_a: reads.append(1) or "a line\n",
                              sleep=lambda _s: None)
    assert got == "a line\n" and len(reads) == 1


# ── the cursor ───────────────────────────────────────────────────────────────

def test_log_cursor_is_utc_rfc3339_to_the_second():
    """`date -u +%Y-%m-%dT%H:%M:%SZ`. The format is load-bearing: docker compares the `--since`
    cursor against DAEMON-side host timestamps, so a local-time or sub-second spelling silently
    selects the wrong slice."""
    assert re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", lifecycle.log_cursor())


def test_the_finding_message_names_the_service_and_the_line():
    """The message is the only thing an operator sees. It has to say which node and which line."""
    msg = lifecycle.ack_tripwire_message("validator-3")
    assert "validator-3" in msg and _ACK in msg and "graceful, unhalted" in msg


# ── the two consumers agree ──────────────────────────────────────────────────

def test_the_sim_actuators_delegate_to_this_module():
    """The extraction's whole point: `sim.Actuators` must not keep a private second copy of the
    ack constants, because the value of ONE definition is that a marshal-log change updates both
    consumers. Asserted by identity, not by equality."""
    from dpos_harness.sim import actions
    assert actions.Actuators._ACK_DEATH is lifecycle.ACK_DEATH
    assert actions.Actuators._ACK_SCOPE_OUT is lifecycle.ACK_SCOPE_OUT
