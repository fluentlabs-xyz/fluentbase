"""The exec contract, enforced statically so it cannot rot back.

`core/proc.py` is the ONE place the harness spawns a process. Everything else asks it — for the
env merge (os.environ first, per-call overlay last), the wall-clock ceiling, dry-run suppression
and transcript recording. A call site that spawns its own re-implements four things and gets at
least one wrong; `actions._run_env` built its own env dict, dropped the ambient COMPOSE_FILE, and
made every byzantine actuation a silent no-op for four days (v61.11).

Everything here is DERIVED BY AST from the package source. Nothing is hand-listed except the seam
module itself and the exec APIs — and the exception register lives in `proc.EXEC_SEAM_EXCEPTIONS`,
which this file READS rather than mirrors. A hand-written mirror of production facts in a test has
gone stale here four times in two days; a mirror cannot go stale if it does not exist.
"""

from __future__ import annotations

import ast
import pathlib

import pytest

import dpos_harness
from dpos_harness.core import proc

PKG = pathlib.Path(dpos_harness.__file__).parent

# The one module allowed to spawn. Dotted, relative to the package.
SEAM = "core.proc"

# Modules that spawn a process. `subprocess` covers run/Popen/call/check_output/check_call in one
# name; the `os.*` and `pty.*` entries are the back doors that do not mention subprocess at all —
# `os.system("clear")` in `sim.status` was exactly that, invisible to a `grep subprocess` AND to
# the unit suite's `subprocess.run` guard.
EXEC_MODULES = {"subprocess", "pty", "commands"}
EXEC_OS_CALLS = {"system", "popen", "posix_spawn", "posix_spawnp", "startfile", "fork", "forkpty"}
EXEC_OS_PREFIXES = ("exec", "spawn")     # os.execv/execvp/execve/… , os.spawnl/spawnv/…


def _modules():
    """Every non-test package module as (dotted-name, path). Tests are excluded: this file itself
    must be free to reach for `subprocess` when it mutates a module to prove the check bites."""
    for p in sorted(PKG.rglob("*.py")):
        rel = p.relative_to(PKG)
        if rel.parts[0] == "tests" or "__pycache__" in rel.parts:
            continue
        parts = list(rel.with_suffix("").parts)
        if parts[-1] == "__init__":
            parts = parts[:-1]
        if not parts:
            continue
        yield ".".join(parts), p


def _root(node):
    """The leftmost name of a dotted attribute chain: `os.path.exists` → 'os'."""
    while isinstance(node, ast.Attribute):
        node = node.value
    return node.id if isinstance(node, ast.Name) else None


def exec_sites(tree: ast.AST):
    """Every spawn site in one parsed module, as (lineno, what). Four spellings, because a scan
    that sees only the one production happens to use today is a style check:

      * `import subprocess` / `import subprocess as sp`      → Import
      * `from subprocess import run` / `from os import system`→ ImportFrom
      * `os.system(...)` / `os.execvp(...)` / `os.spawnl(...)`→ Attribute call on a module that is
        NOT itself suspicious, so no import scan can catch it
      * `pty.spawn(...)`                                     → same shape, different module

    The import spellings are reported even without a call: importing `subprocess` into a module is
    the observable step, and a module that imports it in order not to use it is noise we would
    rather delete than allow.
    """
    found = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for a in node.names:
                if a.name.split(".")[0] in EXEC_MODULES:
                    found.append((node.lineno, f"import {a.name}"))
        elif isinstance(node, ast.ImportFrom):
            mod = (node.module or "").split(".")[0]
            if node.level == 0 and mod in EXEC_MODULES:
                found.append((node.lineno, f"from {node.module} import …"))
            if node.level == 0 and mod == "os":
                for a in node.names:
                    if a.name in EXEC_OS_CALLS or a.name.startswith(EXEC_OS_PREFIXES):
                        found.append((node.lineno, f"from os import {a.name}"))
        elif isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
            base, attr = _root(node.func), node.func.attr
            if base == "os" and (attr in EXEC_OS_CALLS or attr.startswith(EXEC_OS_PREFIXES)):
                found.append((node.lineno, f"os.{attr}(…)"))
            elif base in EXEC_MODULES:
                found.append((node.lineno, f"{base}.{attr}(…)"))
    return found


def scan():
    """{dotted module: [(lineno, what), …]} for every module that spawns, seam included."""
    out = {}
    for name, path in _modules():
        sites = exec_sites(ast.parse(path.read_text()))
        if sites:
            out[name] = sites
    return out


# ── the contract ────────────────────────────────────────────────────────────────

def test_seam_is_the_only_exec_path():
    """No module outside the seam spawns a process, unless proc.EXEC_SEAM_EXCEPTIONS says so."""
    offenders = {m: s for m, s in scan().items()
                 if m != SEAM and m not in proc.EXEC_SEAM_EXCEPTIONS}
    assert not offenders, (
        "these modules spawn processes outside the proc seam:\n"
        + "\n".join(f"  {m}: " + ", ".join(f"{ln}: {w}" for ln, w in sites)
                    for m, sites in sorted(offenders.items()))
        + "\nRoute them through core.proc (Runner.* for writes, proc.read/read_capture for "
          "reads), or extend the seam. An exception must be registered in "
          "proc.EXEC_SEAM_EXCEPTIONS with a reason.")


def test_seam_itself_still_spawns():
    """Guards the inverse failure: a scanner that finds nothing anywhere (a broken AST walk, a
    renamed package root) would pass the contract vacuously forever."""
    assert SEAM in scan(), "the scan cannot even see the seam's own subprocess use — it is blind"


def test_exception_register_is_empty_and_only_names_real_modules():
    """The register is the ONLY exception channel, so it must not silently accumulate. Every entry
    needs a real module and a written reason."""
    known = {name for name, _ in _modules()}
    for mod, reason in proc.EXEC_SEAM_EXCEPTIONS.items():
        assert mod in known, f"EXEC_SEAM_EXCEPTIONS names {mod!r}, which is not a package module"
        assert isinstance(reason, str) and reason.strip(), f"{mod} is excepted with no reason"
    assert proc.EXEC_SEAM_EXCEPTIONS == {}, (
        "an exception was added. That is allowed, but it must be a deliberate review decision: "
        "update this assertion in the same change, with the reason.")


def test_scanner_detects_every_spelling(tmp_path):
    """The scanner is the whole contract, so its detection is asserted directly rather than
    trusted. Each line below is a spelling that has actually been used in this package or is one
    `import` away from it."""
    src = (
        "import subprocess\n"
        "import subprocess.abc as sp\n"
        "from subprocess import run\n"
        "from os import system\n"
        "from os import execvp\n"
        "import os, pty\n"
        "def f():\n"
        "    os.system('clear')\n"
        "    os.execvp('sh', [])\n"
        "    os.spawnl(os.P_WAIT, 'sh')\n"
        "    os.posix_spawn('sh', [], {})\n"
        "    pty.spawn('sh')\n"
        "    subprocess.Popen(['x'])\n"
        "    sp.run(['x'])\n"          # aliased import: caught at the import, not the call
        "    os.path.exists('/')\n"    # NOT an exec site
        "    os.environ.get('X')\n"    # NOT an exec site
    )
    sites = exec_sites(ast.parse(src))
    whats = [w for _, w in sites]
    for expected in ("import subprocess", "from subprocess import …", "from os import system",
                     "from os import execvp", "import pty", "os.system(…)", "os.execvp(…)",
                     "os.spawnl(…)", "os.posix_spawn(…)", "pty.spawn(…)", "subprocess.Popen(…)"):
        assert expected in whats, f"scanner missed {expected}: {whats}"
    assert not any("path" in w or "environ" in w for w in whats), whats


def test_a_planted_spawn_is_caught():
    """The mutation, run for real: plant a module that spawns directly and assert the contract
    fails. Without this the test could be asserting over an empty scan and nobody would know.

    The plant is a NEW file, never an edit to a tracked one: if the run is killed between the
    write and the finally, the worst outcome is a stray untracked file, not a corrupted module.
    """
    planted = PKG / "sim" / "_seam_mutation_probe.py"
    try:
        planted.write_text("import subprocess\n\n\ndef go():\n    return subprocess.run(['true'])\n")
        with pytest.raises(AssertionError, match=r"sim\._seam_mutation_probe"):
            test_seam_is_the_only_exec_path()
        # and it is caught by the REGISTER too — proving the exception channel is the only escape
        proc.EXEC_SEAM_EXCEPTIONS["sim._seam_mutation_probe"] = "mutation probe"
        try:
            test_seam_is_the_only_exec_path()          # excepted → passes
        finally:
            proc.EXEC_SEAM_EXCEPTIONS.pop("sim._seam_mutation_probe")
    finally:
        planted.unlink(missing_ok=True)
    test_seam_is_the_only_exec_path()          # and the revert restores the contract


# ── the seam's env handling, as a regression test for the v61.11 incident ───────

def test_actuator_env_overlay_keeps_the_ambient_compose_file(monkeypatch, tmp_path):
    """`act_byzantine` must reach docker with COMPOSE_FILE = <ambient>:<byz overlay>.

    This is the v61.11 incident: the actuator built `{**os.environ, **self.env}` itself, `self.env`
    is a pre-bring-up snapshot whose COMPOSE_FILE is "", and the merged value became ":overlay" —
    a leading colon that drops every base compose file, so `up --force-recreate` was a silent
    no-op and the victim never turned byzantine. Breaking the seam's env merge must make THIS test
    fail, which is what makes it a regression test and not a restatement of the code.
    """
    import os
    from dpos_harness.sim import actions

    monkeypatch.setenv("COMPOSE_FILE",
                       "docker-compose.sim.gen.yml:docker-compose.sim.dpos.gen.yml")
    monkeypatch.chdir(tmp_path)                # act_byzantine writes its overlay into cwd
    seen = {}

    def fake_run(argv, capture_output=None, text=None, timeout=None, env=None, cwd=None, **kw):
        seen["argv"] = list(argv)
        seen["env"] = dict(env or {})
        import subprocess as _sp
        return _sp.CompletedProcess(args=argv, returncode=0, stdout="", stderr="")

    import subprocess
    monkeypatch.setattr(subprocess, "run", fake_run)

    # self.env carries the PRE-bring-up snapshot — COMPOSE_FILE "" — exactly as in production.
    act = actions.Actuators(env={"COMPOSE_FILE": "", "RPC": "http://x"}, dry_run=False)
    act.act_byzantine("validator-3", "equivocate")

    assert seen["argv"][:3] == ["docker", "compose", "up"]
    cf = seen["env"]["COMPOSE_FILE"]
    assert not cf.startswith(":"), f"leading colon — the v61.11 no-op is back: {cf!r}"
    assert cf == ("docker-compose.sim.gen.yml:docker-compose.sim.dpos.gen.yml"
                  ":docker-compose.sim.byz-validator-3.gen.yml"), cf
    # and the rest of the ambient env survived: the seam merges, it does not replace.
    assert seen["env"].get("PATH") == os.environ.get("PATH")


def test_seam_env_precedence_is_environ_then_runner_then_overlay(monkeypatch):
    """The merge order the actuator test depends on, asserted directly at the seam."""
    monkeypatch.setenv("SEAM_T", "from-environ")
    seen = {}

    def fake_run(argv, capture_output=None, text=None, timeout=None, env=None, cwd=None, **kw):
        seen.update(env or {})
        import subprocess as _sp
        return _sp.CompletedProcess(args=argv, returncode=0, stdout="", stderr="")

    import subprocess
    monkeypatch.setattr(subprocess, "run", fake_run)

    r = proc.Runner(env={"SEAM_T": "from-runner", "SEAM_R": "runner-only"})
    r.run_capture(["true"], env_overlay={"SEAM_T": "from-overlay"})
    assert seen["SEAM_T"] == "from-overlay"     # overlay wins
    assert seen["SEAM_R"] == "runner-only"      # runner env is present
    assert "PATH" in seen                       # os.environ is the base, never replaced

    seen.clear()
    proc.Runner().run_capture(["true"])
    assert seen.get("SEAM_T") == "from-environ"  # a runner with no env passes os.environ through


# ── the ambient channel must stay OUT of the transcript ─────────────────────────

def test_ambient_runner_executes_but_records_nothing(monkeypatch):
    calls = []

    def fake_run(argv, capture_output=None, text=None, timeout=None, env=None, cwd=None, **kw):
        calls.append(list(argv))
        import subprocess as _sp
        return _sp.CompletedProcess(args=argv, returncode=0, stdout="out\n", stderr="")

    import subprocess
    monkeypatch.setattr(subprocess, "run", fake_run)

    r = proc.Runner(record=False, echo=True)
    r.run(["a"]); r.run_capture(["b"]); r.run_ok(["c"]); r.read(["d"]); r.step("s"); r.pipe_to_exec("v", "p", "x")
    assert r.log == [], "an ambient runner transcribed something"
    assert [c[0] for c in calls] == ["a", "b", "c", "d", "docker"]

    rec = proc.Runner()
    rec.run(["a"]); rec.read(["d"]); rec.step("s")
    assert [i.kind for i in rec.log] == ["run", "read", "step"]


def test_module_read_is_the_ambient_channel(monkeypatch):
    """proc.read/read_capture must never grow the module runner's log — reads happen throughout
    bring-up, and `--dry-run-bringup` prints a log that has to stay a fixed 197-command sequence."""
    def fake_run(argv, capture_output=None, text=None, timeout=None, env=None, cwd=None, **kw):
        import subprocess as _sp
        return _sp.CompletedProcess(args=argv, returncode=0,
                                    stdout=b"  bytes  " if text is False else "  padded  ",
                                    stderr="" if text is not False else b"")

    import subprocess
    monkeypatch.setattr(subprocess, "run", fake_run)

    assert proc.read(["cast", "block-number"]) == "  padded  "   # VERBATIM: no strip
    assert proc.read_capture(["docker", "ps"]).stdout == "padded"
    assert proc.read_capture(["docker", "logs"], binary=True).raw == b"  bytes  "
    assert proc._AMBIENT.log == []


def test_read_degrades_to_empty_and_capture_flags_spawn_failure(monkeypatch):
    def boom(*a, **k):
        raise FileNotFoundError("no such binary")

    import subprocess
    monkeypatch.setattr(subprocess, "run", boom)
    assert proc.read(["nope"]) == ""
    r = proc.read_capture(["nope"])
    assert not r.ok and r.spawn_failed and r.rc == -1

    def slow(*a, **k):
        raise subprocess.TimeoutExpired(cmd="x", timeout=1)

    monkeypatch.setattr(subprocess, "run", slow)
    assert proc.read_capture(["slow"]).spawn_failed
