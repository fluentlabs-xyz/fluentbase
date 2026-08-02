"""proc.py — the ONE subprocess seam of the harness (docker / cast / forge / coreutils).

`subprocess` is imported HERE AND NOWHERE ELSE in the package. tests/test_exec_seam.py derives
that fact by AST from the source tree (never from a hand-written mirror) and fails on any module
that grows its own `subprocess` / `os.system` / `os.popen` / `os.exec*` / `os.spawn*` call. The
one register of permitted exceptions is `EXEC_SEAM_EXCEPTIONS` below — empty today — and the test
reads it rather than keeping its own copy.

This is not tidiness. A call site that spawns its own process re-implements four things and gets
at least one of them wrong: the wall-clock ceiling, dry-run suppression, transcript recording, and
env handling. `actions._run_env` built its own env dict, dropped the ambient `COMPOSE_FILE`, and
turned every byzantine actuation into a silent no-op for four days (v61.11) — the seam's env merge
(`{**os.environ, **self.env, **overlay}`) is what makes that unrepresentable.

TWO CHANNELS, one exec site:

  * the CHOREOGRAPHY channel — `Runner.run/run_capture/run_ok/run_checked/write_file/step/
    pipe_to_exec` on a RECORDING runner. Every call lands in `.log`, which IS the dry-run
    transcript and the unit-test oracle.
  * the AMBIENT channel — module-level `read()` / `read_capture()` (and any Runner built with
    `record=False`). Executes through the same code path, inherits the same env, honors the same
    timeouts, but keeps NO transcript. Reads and unbounded background loops live here.

The split is deliberate and load-bearing: the dry-run transcript is the primary evidence that a
refactor changed nothing, so it must contain exactly the write-side choreography and nothing
else. Recording a status probe or a spammer's 2-second `cast send` loop into it would both
destroy that oracle and grow without bound over a 9-hour run.

Every WRITE module (chain.writes, bringup, hostpool, reconcilers, sender) issues its docker /
cast / forge commands through a single `Runner` so that:

  * a unit test can STUB the seam and record the EXACT argv each helper issued — the bash
    command line is the oracle (soak-gate-test.sh + the bash sources quote them);
  * `--dry-run-bringup` / `--dry-run-tick` can PRINT the full command sequence (in order,
    with the env delta) WITHOUT executing, to diff against a known-good bash log offline;
  * operator CLI-version parity is kept (we shell out to the same `docker`/`cast`/`forge`
    binaries bash does — never the docker SDK / web3, per analysis §2).

PORT-NOTE (`timeout N docker ...`): the bash write helpers wrap a few exec reads in the
`timeout(1)` binary (`timeout 15 docker compose exec -T validator-0 cat ...`). The Python
port drops the `timeout` binary prefix and uses subprocess's own `timeout=` kwarg instead —
byte-identical hard ceiling, one fewer process — exactly as the already-merged read side
(rpc.py / nodes.py) does. The recorded argv is therefore the bare `docker/cast/forge` line
(the oracle), never the `timeout`-wrapped one.

PORT-NOTE (subshell write-loss is impossible): bash captured helper output with `$(...)`,
which spawned a subshell and dropped any global the helper mutated. Here a helper returns a
value and the caller mutates state directly — the whole defect class is gone structurally.
"""

from __future__ import annotations

import os
import subprocess
from dataclasses import dataclass, field

# Modules permitted to spawn a process WITHOUT going through this one. Keys are dotted module
# names relative to the package (`sim.status`), values are the reason. EMPTY IS THE INTENDED
# STATE. tests/test_exec_seam.py reads this dict — it keeps no list of its own — so an entry
# here is the only way to make an exception, and it is visible in the code rather than silent.
EXEC_SEAM_EXCEPTIONS: dict = {}


def _is_path(path) -> bool:
    """True if `path` can be a filesystem path we would open for writing. `<...>` is this
    module's own PSEUDO-ARGV convention for a non-command transcript entry (`<write-file>`,
    `<step>`), so a value carrying it is a label that took a wrong turn, never a path."""
    return (isinstance(path, str) and path.strip() == path and bool(path)
            and "<" not in path and ">" not in path)


class ProcError(RuntimeError):
    """A lifecycle command (compose up --build / down -v / force-recreate / forge DeployStaking)
    that FAILED or TIMED OUT. Raised by run_checked so bring-up aborts LOUDLY (the v51/v54 nonce-war
    class) instead of a swallowed False that lets the run continue on a broken chain."""

    def __init__(self, result: "RunResult", note: str = ""):
        self.result = result
        self.note = note
        super().__init__(
            f"command failed{f' ({note})' if note else ''}: "
            f"{'TIMED OUT' if result.timed_out else f'rc={result.rc}'} "
            f"argv={' '.join(result.argv)} stderr={result.stderr[-400:]!r}")


@dataclass
class RunResult:
    """The full outcome of one command: argv, stdout, stderr, rc, and whether the wall ceiling
    tripped (TimeoutExpired → timed_out=True with a `<timed-out>` stderr sentinel). Chain.send +
    the lifecycle checks classify transients off .stderr — bash merged stderr via `2>&1`, so a port
    that dropped stderr made the whole transient-retry ladder unreachable (F13)."""
    argv: list
    stdout: str = ""
    stderr: str = ""
    rc: int = 0
    timed_out: bool = False
    # Undecoded stdout, populated ONLY by a binary=True capture (the bundle's `docker compose
    # logs` slice, which is byte-sliced to a size cap and written straight to a file — decoding a
    # multi-MB node log just to re-encode it would both cost and corrupt on a partial UTF-8 tail).
    # A text capture leaves it b"" and a binary capture leaves `stdout` "": one of the two is
    # always empty, so a caller cannot silently read the wrong one and get a plausible answer.
    raw: bytes = b""

    @property
    def ok(self) -> bool:
        return self.rc == 0 and not self.timed_out

    @property
    def spawn_failed(self) -> bool:
        """True when the command never RAN (no such binary, no docker daemon socket, a timeout)
        as opposed to running and failing. `rc` alone cannot say this: a process killed by a
        signal also reports negative, so the sentinels the seam stamps are the discriminator.
        `shadow._runtime_cat` needs the distinction — it must fail loud on "did not run"."""
        return self.timed_out or self.stderr.startswith("<spawn-error>")

    @property
    def merged(self) -> str:
        """stdout+stderr, mirroring bash `cast … 2>&1` — the classification surface."""
        return (self.stdout + ("\n" + self.stderr if self.stderr else "")).strip()


@dataclass
class Invocation:
    """One recorded command: the argv, the env DELTA (only vars the caller overlaid on top
    of os.environ, e.g. a per-recreate COMPOSE_FILE), and the kind."""
    argv: list
    env: dict = field(default_factory=dict)      # overlay only (COMPOSE_FILE=..., etc.)
    kind: str = "run"                            # run | read | file | step
    note: str = ""                               # human label for the dry-run transcript
    cwd: str = None                              # bash `forge_l2` `( cd $SOLIDITY_CONTRACTS_DIR && … )`


class Runner:
    """The exec seam. In LIVE mode it executes; in DRY mode it records and
    returns a canned value so the choreography still walks its full command sequence. Tests
    construct it with dry=True (or replace `.run`) and inspect `.log`.

    `reads` maps an argv-key (the first 2-3 tokens, joined by space) to a canned stdout so a
    dry-run / test can drive a read-dependent branch deterministically.

    `record=False` builds an AMBIENT runner: same exec path, same env merge, same timeouts, but
    nothing is appended to `.log` and nothing is echoed. Use it for work that must not enter the
    choreography transcript — reads (module-level `read`/`read_capture` below), and repeating
    background loops (the spammer threads, the actuators) whose command count is a function of
    wall-clock time rather than of the bring-up sequence."""

    def __init__(self, env: dict = None, dry: bool = False, echo: bool = False,
                 record: bool = True):
        self.env = dict(env or {})
        self.dry = dry
        self.echo = echo                         # print each invocation (dry-run transcript)
        self.record = record                     # False → ambient: executes, transcribes nothing
        self.log: list[Invocation] = []
        self.reads: dict = {}                    # argv-prefix -> canned stdout (dry/test)

    def _note(self, inv: "Invocation") -> None:
        """Transcribe one invocation. The single place `.log` grows, so `record=False` cannot be
        half-honored by a method that appends directly."""
        if not self.record:
            return
        self.log.append(inv)
        if self.echo:
            self._print(inv)

    def _exec(self, argv, overlay, timeout, cwd, text=True, stdin=None):
        """The ONE `subprocess.run` of the package. Env is ALWAYS os.environ first, then this
        runner's env, then the per-call overlay — never a caller-built dict (v61.11)."""
        return subprocess.run(argv, capture_output=True, text=text, timeout=timeout,
                              env={**os.environ, **self.env, **overlay}, cwd=cwd,
                              **({"input": stdin} if stdin is not None else {}))

    # -- the seam ----------------------------------------------------------------
    def run(self, argv, env_overlay=None, timeout=90, note="", cwd=None) -> str:
        """Issue a command; return its stdout (stripped). Records the Invocation first, so a
        stubbed/dry Runner sees the exact argv bash would issue. env_overlay is layered on top
        of os.environ+self.env for THIS call only (the COMPOSE_FILE-append idiom). cwd models
        the bash `forge_l2` wrapper `( cd $SOLIDITY_CONTRACTS_DIR && forge … )`."""
        overlay = dict(env_overlay or {})
        inv = Invocation(argv=[str(a) for a in argv], env=overlay, kind="run", note=note, cwd=cwd)
        self._note(inv)
        if self.dry:
            return self._canned(inv.argv)
        try:
            out = self._exec(inv.argv, overlay, timeout, cwd)
            return (out.stdout or "").strip()
        except Exception:  # noqa: BLE001 — a read helper degrades to "" (caller memoizes/retries)
            return ""

    def run_capture(self, argv, env_overlay=None, timeout=90, note="", cwd=None,
                    binary=False) -> RunResult:
        """Issue a command and return the FULL RunResult (stdout+stderr+rc+timed_out). Records the
        Invocation first. On subprocess.TimeoutExpired the result carries timed_out=True and a
        `<timed-out>` stderr sentinel (so Chain.send maps it to a transient); any other spawn error
        is rc=-1 with the exception text in stderr. env_overlay layers for THIS call only; cwd
        models the bash `forge_l2` `( cd $SOLIDITY_CONTRACTS_DIR && … )` wrapper. `binary=True`
        skips decoding and delivers stdout as `.raw` bytes (see RunResult.raw)."""
        overlay = dict(env_overlay or {})
        argv = [str(a) for a in argv]
        inv = Invocation(argv=argv, env=overlay, kind="run", note=note, cwd=cwd)
        self._note(inv)
        if self.dry:
            return RunResult(argv=argv, stdout=self._canned(argv), rc=0)
        try:
            out = self._exec(argv, overlay, timeout, cwd, text=not binary)
            if binary:
                return RunResult(argv=argv, raw=out.stdout or b"",
                                 stderr=(out.stderr or b"").decode(errors="replace").strip(),
                                 rc=out.returncode)
            return RunResult(argv=argv, stdout=(out.stdout or "").strip(),
                             stderr=(out.stderr or "").strip(), rc=out.returncode)
        except subprocess.TimeoutExpired:
            return RunResult(argv=argv, stderr="<timed-out>", rc=124, timed_out=True)
        except Exception as e:  # noqa: BLE001 — a spawn failure is a real command failure
            return RunResult(argv=argv, stderr=f"<spawn-error> {e}", rc=-1)

    def read(self, argv, timeout=15, env_overlay=None, cwd=None) -> str:
        """Issue a READ-ONLY command; return stdout VERBATIM ("" on any failure/timeout). Unlike
        `run` it does NOT strip: the metrics/log readers parse line structure and a token-reading
        caller strips at its own call site. Not transcribed on an ambient runner (record=False);
        on a recording runner it is transcribed like any other invocation."""
        overlay = dict(env_overlay or {})
        argv = [str(a) for a in argv]
        self._note(Invocation(argv=argv, env=overlay, kind="read", cwd=cwd))
        if self.dry:
            return self._canned(argv)
        try:
            return self._exec(argv, overlay, timeout, cwd).stdout or ""
        except Exception:  # noqa: BLE001 — an observer degrades to "", reports n/a, never aborts
            return ""

    def run_ok(self, argv, env_overlay=None, timeout=90, note="") -> bool:
        """Issue a command for effect; True on rc 0 (bash `... || true` sites). Records first."""
        return self.run_capture(argv, env_overlay=env_overlay, timeout=timeout, note=note).ok

    def run_checked(self, argv, env_overlay=None, timeout=600, note="", cwd=None) -> RunResult:
        """Issue a lifecycle command that MUST succeed (compose up --build / down -v /
        force-recreate / forge DeployStaking). Generous default ceiling (10m) since these are the
        slow ones; raises ProcError on any failure or timeout so bring-up aborts LOUDLY rather than
        continuing on a half-built chain (the v51/v54 nonce-war class). Dry mode is a no-op ok."""
        r = self.run_capture(argv, env_overlay=env_overlay, timeout=timeout, note=note, cwd=cwd)
        if not r.ok:
            raise ProcError(r, note)
        return r

    def write_file(self, path: str, content: str, note="") -> None:
        """Write a generated compose overlay / runtime file to the host FS. Recorded as a
        `file` invocation (argv = ['<write-file>', path]) so the transcript shows it.

        `path` is a REAL filesystem path and `note` is the transcript label — they are two
        different arguments and must stay that way. bringup once passed the label
        `"<compose-gen>"` HERE, so every non-dry run silently created a 19-byte file literally
        named `<compose-gen>` in the smoke directory (untracked, un-gitignored, and skipped
        three times before anyone chased it). `step()` is the channel for a transcript-only
        marker; this guard makes the confusion loud instead of silent."""
        if not _is_path(path):
            raise ValueError(
                f"write_file: {path!r} is a transcript LABEL, not a filesystem path — "
                "use Runner.step(label, ...) for a marker, or pass a real path and put the "
                "label in note=")
        inv = Invocation(argv=["<write-file>", path], kind="file",
                         note=note or f"{len(content)} bytes")
        self._note(inv)
        if self.dry:
            return
        with open(path, "w") as f:
            f.write(content)

    def step(self, label: str, detail: str = "") -> None:
        """Record a transcript-only STEP: work done IN-PROCESS that issues no subprocess and
        writes no file of its own (compose generation, which writes its own files through
        compose_gen). Never touches the filesystem — that is the whole point of it being a
        separate method from write_file."""
        inv = Invocation(argv=["<step>", label], kind="step", note=detail)
        self._note(inv)

    def pipe_to_exec(self, service: str, dest_path: str, content: str, note="") -> bool:
        """The `printf '%s' <content> | docker compose exec -T <svc> sh -c 'cat > /runtime/<p>'`
        idiom (pp_runtime_write). Recorded argv is the docker line (the oracle); content rides
        stdin. Returns True on success."""
        argv = ["docker", "compose", "exec", "-T", service, "sh", "-c",
                f"cat > /runtime/{dest_path}"]
        inv = Invocation(argv=argv, kind="run", note=note or "runtime-write")
        self._note(inv)
        if self.dry:
            return True
        try:
            return self._exec(argv, {}, 15, None, stdin=content).returncode == 0
        except Exception:
            return False

    # -- dry/test canned reads ---------------------------------------------------
    def _canned(self, argv) -> str:
        key2 = " ".join(argv[:2])
        key3 = " ".join(argv[:3])
        if key3 in self.reads:
            return self.reads[key3]
        if key2 in self.reads:
            return self.reads[key2]
        return self.reads.get(argv[0], "")

    def _print(self, inv: Invocation) -> None:
        env = "".join(f"{k}={v} " for k, v in inv.env.items())
        if inv.kind == "file":
            print(f"  [file] {inv.argv[1]}  ({inv.note})")
        elif inv.kind == "step":
            print(f"  [step] {inv.argv[1]}  ({inv.note})")
        else:
            cwd = f"(cd {inv.cwd} && " if inv.cwd else ""
            tail = ")" if inv.cwd else ""
            print(f"  {env}{cwd}{' '.join(inv.argv)}{tail}"
                  + (f"    # {inv.note}" if inv.note else ""))

    # -- transcript --------------------------------------------------------------
    def argvs(self):
        return [inv.argv for inv in self.log]


# ── the AMBIENT read channel ────────────────────────────────────────────────────
#
# Observers (rpc, nodes, status, shadow, golden, the bundle) hold no Runner: they are module-level
# functions reached from a dozen call sites, and threading a runner handle through all of them
# would buy nothing — a read has no dry-run variant (there is no state to not-mutate) and no place
# in the write transcript.
#
# So they share ONE ambient, NON-RECORDING runner. It executes through the same `_exec`, so it
# inherits the same env merge (crucially the ambient COMPOSE_FILE that `BringUp._set_compose`
# exports) and the same timeout discipline, while `.log` stays empty forever. That last property
# is a hard requirement, not an optimisation: `--dry-run-bringup` prints its runner's log, reads
# happen throughout bring-up, and a read that transcribed itself would corrupt the one artifact
# that proves a refactor changed nothing.
_AMBIENT = Runner(record=False)


def read(argv, timeout: float = 15, cwd: str = None, env_overlay: dict = None) -> str:
    """Run a READ-ONLY command; return its stdout VERBATIM (no strip — callers that want a token
    strip it themselves, and the metrics/log readers need the newlines), "" on any failure or
    timeout. Mirrors the bash `... 2>/dev/null || true` discipline every observer used."""
    return _AMBIENT.read(argv, timeout=timeout, cwd=cwd, env_overlay=env_overlay)


def read_capture(argv, timeout: float = 15, cwd: str = None, env_overlay: dict = None,
                 binary: bool = False) -> RunResult:
    """Run a READ-ONLY command and return the full RunResult — for the readers that must tell
    "empty answer" from "could not run" (`shadow._runtime_cat` raises on the latter, `golden`
    treats a non-zero rc as absent, the bundle records a per-service capture gap). Same non-
    recording ambient channel as `read`; `binary=True` delivers bytes in `.raw`."""
    return _AMBIENT.run_capture(argv, timeout=timeout, cwd=cwd, env_overlay=env_overlay,
                                binary=binary)
