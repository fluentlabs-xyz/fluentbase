"""status.py — one-screen, READ-ONLY status of the DPoS production sim. Port of soak-status.sh.

Answers "how is the sim doing?" by scraping the sim LOG (never asking the chain): which log it
read, running/ended (passed or failed + reason + bundle), progress + rate, the headline invariant
failure count, active anomalies, whether the load blaster is alive (the sent=/inflight= heartbeat
grep — the SAME contract load-heavy/sender emit), and host health.

READ-ONLY BY CONSTRUCTION: reads files, /proc, and bounded `docker ps`. Never stops/restarts/kills.
Preserves the CLI feel: `python -m dpos_harness sim status [LOGFILE] [--watch [SECS]]`.

PORT-NOTE: bash deliberately did NOT set -e (a diagnostic must degrade per-section, not abort);
Python's try/except gives the same graceful "n/a (reason)" per probe. The `grep -c` two-line "0\\n0"
trap the bash `_count1` guards against cannot exist here (we count in Python).
"""

from __future__ import annotations

import argparse
import os
import re
import sys
import time

from ..core import proc, termio

VERSION = "1.0-py"
SIM_DATA_ROOT_DEFAULT = "/mnt/storage/fluent-sim"
# DEPTH-SENSITIVE: <smoke>/dpos_harness/sim/status.py → two levels up is the smoke dir.
REPO_SMOKE_DIR = os.path.normpath(os.path.join(os.path.dirname(__file__), "..", ".."))


def _read(path):
    try:
        with open(path, errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def _countf(text, sub):
    return sum(1 for ln in text.splitlines() if sub in ln)


def _countre(text, pat):
    r = re.compile(pat)
    return sum(1 for ln in text.splitlines() if r.search(ln))


# How a LIVE simulation is discoverable. The sim is a Python process — `python3 -m dpos_harness
# sim run`, normally under one of the `make smoke-sim*` targets — so those are the two patterns.
# The bash `case-soak.sh` pattern this used to carry is deliberately gone: no Makefile target
# launches the bash sim any more, and while the script survives it has its own soak-status.sh.
# The [b]racket idiom keeps the pattern from matching an observer's own command line.
SIM_PATTERNS = (r"[d]pos_harness.*sim run", r"[s]moke-sim")


def discover_running_pid():
    for pat in SIM_PATTERNS:
        out = proc.read(["pgrep", "-f", pat]).split()
        if out:
            return out[0]
    return None


def discover_log(arg_log, pid):
    """LOG discovery order: argument, $SIM_LOG, the running sim's /proc/<pid>/fd/1, the newest
    geo-sim.log/*sim*.log under /tmp/claude-* or the repo. Returns (path, source) or (None, why)."""
    if arg_log:
        if os.access(arg_log, os.R_OK):
            return arg_log, "argument"
        return None, f"log '{arg_log}' (from argument) is not readable"
    sl = os.environ.get("SIM_LOG")
    if sl and os.access(sl, os.R_OK):
        return sl, "$SIM_LOG"
    if pid:
        try:
            t = os.path.realpath(f"/proc/{pid}/fd/1")
            if os.path.isfile(t) and os.access(t, os.R_OK):
                return t, f"running sim pid {pid} (/proc/{pid}/fd/1)"
        except Exception:
            pass
    candidates = []
    roots = [p for p in os.listdir("/tmp") if p.startswith("claude-")]
    search = [os.path.join("/tmp", r) for r in roots] + [REPO_SMOKE_DIR]
    for root in search:
        if not os.path.isdir(root):
            continue
        for dp, _dirs, files in os.walk(root):
            if dp[len(root):].count(os.sep) > 6:
                continue
            for fn in files:
                if fn == "geo-sim.log" or ("sim" in fn and fn.endswith(".log")):
                    p = os.path.join(dp, fn)
                    try:
                        candidates.append((os.path.getmtime(p), p))
                    except OSError:
                        pass
    if candidates:
        candidates.sort(reverse=True)
        return candidates[0][1], "newest *sim*.log on disk"
    return None, None


def _dur(s):
    try:
        s = int(s)
    except (TypeError, ValueError):
        return "n/a"
    d, h, m = s // 86400, (s % 86400) // 3600, (s % 3600) // 60
    if d > 0:
        return f"{d}d {h}h {m}m"
    if h > 0:
        return f"{h}h {m}m"
    return f"{m}m"


def render(log, log_src, pid):
    text = _read(log)
    out = []
    out.append(f"sim-status v{VERSION}  —  {time.strftime('%Y-%m-%d %H:%M:%S')}")
    out.append("-" * 100)
    # log section
    try:
        sz = os.path.getsize(log)
        age = int(time.time() - os.path.getmtime(log))
    except OSError:
        sz, age = 0, 0
    out.append(f"log      : {log}")
    out.append(f"           via {log_src} | {sz} bytes | last write {_dur(age)} ago")

    # state
    failed = _countf(text, "SIM FAILURE BUNDLE")
    passed = _countf(text, "sim finished cleanly")
    if pid:
        out.append(f"state    : RUNNING (pid {pid})")
        if failed > 0:
            out.append("           NOTE: a SIM FAILURE block is already present (teardown in progress?)")
    elif failed > 0:
        out.append("state    : ENDED - FAILED")
        inv = _last_after(text, "broken invariant :")
        reason = _last_after(text, "reason           :")
        seed = _last_after(text, "seed             :")
        bundle = _last_re(text, r"^bundle: (.*)")
        out.append(f"  invariant: {inv or 'n/a'}")
        out.append(f"  reason   : {reason or 'n/a'}")
        out.append(f"  bundle   : {bundle or 'n/a (no bundle: line)'}")
        out.append(f"  seed     : {seed or 'n/a'}")
    elif passed > 0:
        out.append("state    : ENDED - PASSED")
    else:
        out.append("state    : ENDED - INCOMPLETE (no failure block and no clean-finish line)")

    # progress + rate
    out.append("")
    ev_lines = [ln for ln in text.splitlines() if re.search(r"\[sim r[0-9]+ ", ln)]
    if ev_lines:
        last = ev_lines[-1]
        rnd = _search(last, r"\[sim r([0-9]+) ")
        fin = _search(last, r"\bfin=([0-9]+)")
        epoch = _search(last, r"\bfin=[0-9]+ epoch=([0-9]+)")
        f = _search(last, r"\bepoch=[0-9]+ f=([0-9]+)")
        out.append(f"progress : round {rnd or '?'} | epoch {epoch or '?'} | finalized {fin or '?'} | f={f or '?'}")
        rate = _last_re(text, r"rate=([0-9.]+) blk/s")
        out.append(f"rate     : {rate or 'n/a'} blk/s recent")
    else:
        out.append("progress : n/a (no [sim r<N> ...] event lines in this log)")

    # headline
    out.append("-" * 100)
    nfail = _countre(text, r"\[sim r[0-9]+ invariant_fail\]")
    nbundle = _countf(text, "SIM FAILURE BUNDLE")
    nwarn = _countre(text, r"\[sim r[0-9]+ warn\]")
    nWARN = _countre(text, r"\[sim r[0-9]+ WARN\]")
    if nfail == 0 and nbundle == 0:
        out.append("HEADLINE : 0 invariant failures")
    else:
        out.append(f"HEADLINE : {nfail} invariant failure(s), {nbundle} failure bundle(s)")
    out.append(f"counts   : invariant_fail={nfail}  failure-bundle={nbundle}  warn={nwarn}  WARN={nWARN}")

    # anomalies
    out.append("")
    out.append("anomalies (0 = not seen):")
    anomalies = [
        ("dkg-deferral", r"dkg-deferral"),
        ("promote blocked/no-host", r"promote_blocked|promote_deferred_no_host|nohost"),
        ("promote warming (deferred)", r"promote_deferred_warming"),
        ("self-partition", r"self_partition|SELF_PARTITIONED"),
        ("warm-debt HELD (bad)", r"warm-debt-stall|warm-debt HELD"),
        ("result divergence", r"result-diverg|result_diverg|SafetyHalt"),
    ]
    for label, pat in anomalies:
        out.append(f"  {label:<26} {_countre(text, pat)}")
    out.append(f"  {'warm-debt cleared':<26} {_countf(text, 'warm-debt cleared')}  (benign: recoveries)")

    # blaster — the sent=/inflight= heartbeat grep contract
    out.append("")
    out.append(_blaster_section(log))
    # host
    out.append(_host_section())
    return "\n".join(out)


def _blaster_section(log):
    # The Python blaster (`python3 -m dpos_harness load start`) — the supervisor is one process
    # with the sender loops as threads, so this counts 1 per running blaster.
    pids = proc.read(["pgrep", "-f", r"[d]pos_harness.*load start"]).split()
    # …and the legacy bash blaster, which is still a hand-launched tool an operator can point at
    # a live run (README's `./scripts/load-heavy.sh`) until it is deleted. Unlike the bash SIM,
    # nothing has replaced it as an operator entry point, so it stays discoverable.
    pids += proc.read(["pgrep", "-f", r"[l]oad-heavy\.sh"]).split()
    nproc = len(pids)
    lhlog = _newest_lh_log(log)
    status = ""
    if lhlog:
        tail = _read(lhlog).splitlines()[-400:]
        for ln in reversed(tail):
            m = re.search(r"sent=[0-9]+ inflight=[0-9]+(/[0-9]+)?", ln)
            if m:
                status = m.group(0)
                break
    if not lhlog:
        note = "no load-heavy log found"
    elif not status:
        note = "log found but no sent=/inflight= status line in its tail"
    else:
        note = status
    head = f"blaster  : {'RUNNING' if nproc else 'not running'} ({nproc} proc) | {note}"
    if lhlog:
        head += f"\n           log {lhlog}"
    return head


def _newest_lh_log(log):
    cands = []
    roots = [os.path.join("/tmp", p) for p in os.listdir("/tmp") if p.startswith("claude-")]
    roots += [REPO_SMOKE_DIR, os.path.dirname(log)]
    for root in roots:
        if not os.path.isdir(root):
            continue
        for dp, _d, files in os.walk(root):
            if dp[len(root):].count(os.sep) > 4:
                continue
            for fn in files:
                if "load-heavy" in fn and fn.endswith(".log"):
                    p = os.path.join(dp, fn)
                    try:
                        cands.append((os.path.getmtime(p), p))
                    except OSError:
                        pass
    if cands:
        cands.sort(reverse=True)
        return cands[0][1]
    return None


def _host_section():
    r = proc.read_capture(["docker", "ps", "-q"], timeout=3)
    if r.ok:
        ndock = len([x for x in r.stdout.splitlines() if x.strip()])
    else:
        ndock = "n/a (docker unavailable or timed out)"
    return f"host     : docker {ndock} container(s) running"


def _last_after(text, prefix):
    val = ""
    for ln in text.splitlines():
        if prefix in ln:
            val = ln.split(":", 1)[-1].strip() if ":" in ln else ln
            val = ln.split(prefix, 1)[-1].strip()
    return val


def _last_re(text, pat):
    r = re.compile(pat)
    val = ""
    for ln in text.splitlines():
        m = r.search(ln)
        if m:
            val = m.group(1)
    return val


def _search(line, pat):
    m = re.search(pat, line)
    return m.group(1) if m else ""


def main(argv=None):
    argv = argv if argv is not None else sys.argv[1:]
    p = argparse.ArgumentParser(prog="dpos_harness sim status",
                                description="one-screen, read-only status of the DPoS sim")
    p.add_argument("logfile", nargs="?", default=None)
    p.add_argument("--watch", nargs="?", const=10, type=int, default=None)
    args = p.parse_args(argv)

    pid = discover_running_pid()
    log, src = discover_log(args.logfile, pid)
    if not log:
        if args.logfile:
            print(src, file=sys.stderr)
            return 1
        print("sim-status: no sim log found.", file=sys.stderr)
        return 1
    if args.watch:
        try:
            while True:
                termio.clear_screen()
                print(render(log, src, pid))
                print(f"\n(--watch {args.watch}s; Ctrl-C to stop)")
                time.sleep(args.watch)
                pid = discover_running_pid()
        except KeyboardInterrupt:
            return 0
    else:
        print(render(log, src, pid))
    return 0


if __name__ == "__main__":
    sys.exit(main())
