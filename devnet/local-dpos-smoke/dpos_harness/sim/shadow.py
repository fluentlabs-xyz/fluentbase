"""shadow.py — READ-ONLY shadow runner for the Python invariant battery.

Attaches to a RUNNING sim and runs the Python battery on the same cadence,
writing diff-friendly verdict lines to its OWN log. It NEVER mutates the topology:
it only READS (RPC/metrics/docker logs via core.nodes) and reads log files.
Do NOT confuse it with the bash sim — it is an observer running alongside.

Subcommands:
  run [--log FILE] [--period SECS] [--once]
      Auto-discover the live topology (the soak-status.sh way), run the battery
      each tick, and append a verdict line per tick to the shadow log.
  compare --shadow-log FILE --bash-log FILE
      Tail the bash sim log's invariant_fail / warn / WARN lines and the shadow
      log's FAIL/WARN lines, and report agreement / divergence counts.

Shadow-log line format (diff-friendly, one line per event):
  TICK <n> OK fin=<> epoch=<>
  TICK <n> FAIL id=<fail-id> fin=<> epoch=<> msg=<first-120-chars>
  TICK <n> EVENT <kind> <note-first-120-chars>

PORT-NOTE: the shadow attaches read-only, so it cannot reconstruct the
orchestrator's in-memory live-sim state (ADDR2IDX / EVER_FAULTED / SIM_DISRUPTED
…). It populates what it can observe on-chain — the three DEPLOYED contract
addresses, read once at attach out of the running topology's
/runtime/staking-reader.json (see resolve_runtime_addrs; the run path gets the
same facts from the Chain it built), then the epoch from head/interval and the
committee from getEpochCommittee — and leaves the
rest at their bash `${VAR:-default}` defaults — so the committee-dependent
cross-node detectors run over an empty set exactly as bash does when
SIM_CUR_COMMITTEE is empty (coverage / detector-starved belts own that vacuity).
The v0-anchored detectors (finalize-stall, k-lag, spec-head, safety-halt on v0,
beacon-active, node-panic, node-down, block-rate, exec-saturation) run fully.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time

from . import status
from ..core import nodes, proc, topology
from ..checks.battery import Battery, Ctx, SimCtx

# DEPTH-SENSITIVE: <smoke>/dpos_harness/sim/shadow.py → two levels up is the smoke dir. Hoisted
# out of discover_bash_log() so the nesting assumption is a module constant a test can assert;
# as a function local it was re-nested by P3 with nothing able to see it (test_path_anchors.py).
REPO_SMOKE_DIR = os.path.normpath(os.path.join(os.path.dirname(__file__), "..", ".."))


# ── live-topology / log discovery (soak-status.sh parity) ────────────────────
def discover_running_pid():
    """Same discovery contract as `sim/status.py` — the patterns live there (`SIM_PATTERNS`) so
    the two observers can never disagree about what a running simulation looks like."""
    for pat in status.SIM_PATTERNS:
        out = proc.read(["pgrep", "-f", pat]).split()
        if out:
            return out[0]
    return None


def discover_bash_log():
    """LOGFILE discovery order: $SIM_LOG, the running sim's /proc/<pid>/fd/1,
    the newest *sim*.log on disk. Returns a readable path or None."""
    sl = os.environ.get("SIM_LOG")
    if sl and os.access(sl, os.R_OK):
        return sl
    pid = discover_running_pid()
    if pid:
        try:
            t = os.path.realpath(f"/proc/{pid}/fd/1")
            if os.path.isfile(t) and os.access(t, os.R_OK):
                return t
        except Exception:
            pass
    candidates = []
    for root in ["/tmp"] + [REPO_SMOKE_DIR]:
        for dirpath, _dirs, files in os.walk(root):
            depth = dirpath[len(root):].count(os.sep)
            if depth > 6:
                continue
            for f in files:
                if f == "geo-sim.log" or ("sim" in f and f.endswith(".log")):
                    p = os.path.join(dirpath, f)
                    try:
                        candidates.append((os.path.getmtime(p), p))
                    except OSError:
                        pass
    if candidates:
        candidates.sort(reverse=True)
        return candidates[0][1]
    return None


# ── deployed contract addresses (resolved once, at attach) ───────────────────
class ShadowAttachError(RuntimeError):
    """The shadow could not resolve the running sim's DEPLOYED contract addresses.

    FATAL at attach, never demoted to a warning. The sim deploys staking /
    chain-config / liveness at run time, so the genesis predeploys the nodes.py
    seams fall back to are CODELESS here: every contract read returns empty, every
    contract-backed detector evaluates over nothing, and the battery reports
    confident "holds" verdicts about a chain it cannot see. Judging a running sim
    that way is worse than not observing it at all — which is exactly the defect
    (task 20260729__sim_port_gaps, F1 on the run path, F8 here) this class exists
    to make impossible to repeat silently.
    """


STAKING_READER_JSON = "staking-reader.json"

# Ctx field  ←  /runtime/staking-reader.json key. The file is written at bring-up by
# sim_regen_staking_reader (chain/writes.py:880-890) from the DeployStaking manifest, so it
# is the same derive-don't-predict source the live run's Chain carries in memory.
RUNTIME_ADDR_FIELDS = (
    ("STAKING_RT", "staking_address"),
    ("CHAIN_CONFIG_RT", "chain_config_address"),
    ("LIVENESS_RT", "liveness_slashing_address"),
)

_ADDR_RE = re.compile(r"0x[0-9a-fA-F]{40}\Z")


def _runtime_cat(path, timeout=15):
    """`docker compose exec -T validator-0 cat /runtime/<path>` — the same READ shape
    Chain.runtime_cat uses (chain/writes.py:204-208), spelled locally because the shadow owns
    no Runner and must never construct a write-side Chain. Raises ShadowAttachError rather
    than degrading to "" the way a read helper would: the caller cannot tell an empty file
    from an unreachable container, and guessing is the failure this module refuses."""
    argv = ["docker", "compose", "exec", "-T", topology.RUNTIME_MOUNT_HOST,
            "cat", f"/runtime/{path}"]
    out = proc.read_capture(argv, timeout=timeout)
    if out.spawn_failed:
        # proc.read_capture never raises: a spawn failure / timeout arrives as rc=-1 / timed_out
        # with the reason in stderr. This reader must still FAIL LOUD on it (see the docstring),
        # so it re-raises rather than degrading — the one behaviour the seam must not flatten.
        raise ShadowAttachError(
            f"`{' '.join(argv)}` did not run: {_fmt_note(out.stderr)}")
    if out.rc != 0:
        raise ShadowAttachError(
            f"`{' '.join(argv)}` rc={out.rc} stderr={_fmt_note(out.stderr)!r} "
            f"(COMPOSE_FILE={os.environ.get('COMPOSE_FILE', '')!r})")
    return (out.stdout or "").strip()


def resolve_runtime_addrs():
    """Read the three DEPLOYED contract addresses out of the running topology's
    /runtime/staking-reader.json and return them as a {Ctx field: address} dict.

    Raises ShadowAttachError on ANY failure — unreachable container, unparseable file, a
    missing/zero/malformed address. There is deliberately no empty-dict return path."""
    raw = _runtime_cat(STAKING_READER_JSON)
    try:
        obj = json.loads(raw)
    except Exception as e:  # noqa: BLE001 — empty file, partial write, an error banner
        raise ShadowAttachError(
            f"/runtime/{STAKING_READER_JSON} is not JSON ({type(e).__name__}): "
            f"{_fmt_note(raw)!r}") from e
    if not isinstance(obj, dict):
        raise ShadowAttachError(
            f"/runtime/{STAKING_READER_JSON} is not a JSON object: {_fmt_note(raw)!r}")
    addrs = {}
    bad = []
    for field, key in RUNTIME_ADDR_FIELDS:
        val = str(obj.get(key, "") or "").strip().lower()
        if not _ADDR_RE.match(val) or int(val, 16) == 0:
            bad.append(f"{key}={val!r}")
        addrs[field] = val
    if bad:
        raise ShadowAttachError(
            f"/runtime/{STAKING_READER_JSON} carries no usable address for: {', '.join(bad)}")
    return addrs


# ── head-derived epoch (best-effort, on-chain) ───────────────────────────────
def _current_epoch(chain_config_rt):
    """cur relative DPoS epoch from the host head, using getEpochBlockInterval /
    getDposActivationBlock (falls back to env EPOCH_INTERVAL / DPOS_ACTIVATION_BLOCK).
    `chain_config_rt` is the DEPLOYED ChainConfig — reading through the nodes.py predeploy
    default returns empty here and silently pinned the epoch to 0 for every shadow run."""
    head = nodes.finalized_dec()
    interval = 0
    act = 0
    out = nodes.chainconfig_call("getEpochBlockInterval()(uint32)", addr=chain_config_rt)
    m = re.match(r"\s*([0-9]+)", out)
    interval = int(m.group(1)) if m else int(os.environ.get("EPOCH_INTERVAL", "0") or 0)
    out = nodes.chainconfig_call("getDposActivationBlock()(uint64)", addr=chain_config_rt)
    m = re.match(r"\s*([0-9]+)", out)
    act = int(m.group(1)) if m else int(os.environ.get("DPOS_ACTIVATION_BLOCK", "0") or 0)
    if interval <= 0 or head < act:
        return 0
    return (head - act) // interval


def _build_ctx(tick, round_, addrs):
    ctx = Ctx()
    ctx.SIM_TICK = tick
    ctx.SIM_ROUND = round_
    # the resolved deploy facts — without them every battery cast seam (_staking_call /
    # _chainconfig_call / _liveness_call / _pp_committee …) falls back to a codeless predeploy.
    for field, val in addrs.items():                # required: never call this with None/{}
        setattr(ctx, field, val)
    try:
        ctx.SIM_CUR_EPOCH = _current_epoch(ctx.CHAIN_CONFIG_RT)
    except Exception:
        ctx.SIM_CUR_EPOCH = 0
    # SIM_CUR_F / committee / ADDR2IDX are orchestrator state the shadow cannot
    # observe read-only; leave at bash defaults (empty committee -> the cross-node
    # detectors' vacuity is owned by the coverage / detector-starved belts).
    return ctx


def _fmt_note(s):
    return " ".join((s or "").split())[:120]


def run(args):
    log_path = args.log or os.path.join(
        os.environ.get("SIM_OUT", "."), "sim-shadow.log")
    period = args.period or float(os.environ.get("SIM_CHECK_PERIOD", "10"))
    # Attach without bring-up: resolve the ambient COMPOSE_FILE (the value sim_bring_up would
    # have exported) so the nodes.py `docker compose exec` READS below resolve the sim project
    # instead of the default docker-compose.yml — else every in-container read returns the
    # fin=-1 / roots=null sentinel. Honors an operator-exported COMPOSE_FILE; else discovers the
    # generated compose files on disk (dpos overlay chain for a running phase-B sim).
    from ..stack.bringup import resolve_compose_env
    resolve_compose_env()
    pid = discover_running_pid()
    started = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    # The DEPLOYED contract addresses, ONCE, before the first tick. This is an ATTACH
    # PRECONDITION, not a best-effort read: a shadow that cannot resolve them would run the
    # whole battery through codeless predeploys and log "OK" every tick. Deliberately outside
    # the loop's `except Exception` guard below — that guard is for per-tick read failures and
    # must never absorb this one.
    try:
        addrs = resolve_runtime_addrs()
    except ShadowAttachError as e:
        with open(log_path, "a", buffering=1) as log:
            log.write(f"# dpos_harness shadow ABORT {started} could not resolve the deployed "
                      f"contract addresses: {_fmt_note(str(e))}\n")
        print(f"dpos_harness sim shadow: ABORT — could not resolve the deployed contract addresses "
              f"from /runtime/{STAKING_READER_JSON}: {e}\n"
              f"  The battery would read every contract at the genesis predeploy, which is "
              f"CODELESS in this configuration, and report 'holds' over nothing.\n"
              f"  Is the sim running, and is COMPOSE_FILE pointing at it? "
              f"(COMPOSE_FILE={os.environ.get('COMPOSE_FILE', '') or '<unset>'})",
              file=sys.stderr)
        return 2
    with open(log_path, "a", buffering=1) as log:
        log.write(f"# dpos_harness shadow started {started} "
                  f"(running-sim pid={pid or 'none'}, period={period}s)\n")
        log.write("# resolved runtime addresses: "
                  + " ".join(f"{f}={addrs[f]}" for f, _k in RUNTIME_ADDR_FIELDS) + "\n")
        # F10: the Battery holds ROLLING belt state (SIM_FLAT_TICKS, the stranger/coverage/detector
        # accumulators) that must PERSIST across ticks — a fresh Battery every tick reset every belt
        # to zero and no multi-tick detector could ever trip. Construct ONCE; refresh its ctx +
        # clear the per-tick event capture each tick.
        bat = Battery()
        tick = 0
        while True:
            tick += 1
            # An EMPTY SimCtx every tick, deliberately: the shadow attaches read-only and caused
            # no churn, so it has no seat/refill/fault bookkeeping to report. The six sim-coupled
            # detectors then evaluate over defaults, exactly as they did when both halves were one
            # bag — the coverage / detector-starved belts own that vacuity.
            bat.bind(_build_ctx(tick, tick, addrs), SimCtx())
            bat.events.clear()
            try:
                ok = bat.check_invariants()
            except Exception as e:  # a read-side exception is a shadow event, never a topology change
                log.write(f"TICK {tick} SHADOW-ERROR {type(e).__name__}: {_fmt_note(str(e))}\n")
                if args.once:
                    break
                time.sleep(period)
                continue
            fin = nodes.finalized_dec()
            epoch = bat.ctx.SIM_CUR_EPOCH
            if ok:
                log.write(f"TICK {tick} OK fin={fin} epoch={epoch}\n")
            else:
                log.write(f"TICK {tick} FAIL id={bat.inv_fail_id} fin={fin} epoch={epoch} "
                          f"msg={_fmt_note(bat.inv_fail_msg)}\n")
            for kind, note in bat.events:
                log.write(f"TICK {tick} EVENT {kind} {_fmt_note(note)}\n")
            if args.once:
                break
            time.sleep(period)
    return 0


# ── compare ──────────────────────────────────────────────────────────────────
_BASH_FAIL_RE = re.compile(r"\[sim r[0-9]+ invariant_fail\]")
_BASH_WARN_RE = re.compile(r"\[sim r[0-9]+ warn\]")
_BASH_WARN_UP_RE = re.compile(r"\[sim r[0-9]+ WARN\]")
_BASH_FAILID_RE = re.compile(r"invariant_fail\]\s+(?:fin=\S+\s+epoch=\S+\s+f=\S+\s+)?([a-z0-9-]+)")


def _tally_bash(path):
    fails = []
    warn = 0
    warn_up = 0
    with open(path, errors="replace") as f:
        for line in f:
            if _BASH_FAIL_RE.search(line):
                m = _BASH_FAILID_RE.search(line)
                fails.append(m.group(1) if m else "<unknown>")
            if _BASH_WARN_RE.search(line):
                warn += 1
            if _BASH_WARN_UP_RE.search(line):
                warn_up += 1
    return fails, warn, warn_up


def _tally_shadow(path):
    fails = []
    events = 0
    ok = 0
    with open(path, errors="replace") as f:
        for line in f:
            if " FAIL id=" in line:
                m = re.search(r"FAIL id=(\S+)", line)
                fails.append(m.group(1) if m else "<unknown>")
            elif " OK " in line:
                ok += 1
            elif " EVENT " in line:
                events += 1
    return fails, ok, events


def compare(args):
    b_fails, b_warn, b_warn_up = _tally_bash(args.bash_log)
    s_fails, s_ok, s_events = _tally_shadow(args.shadow_log)
    b_set = set(b_fails)
    s_set = set(s_fails)
    agree = sorted(b_set & s_set)
    only_bash = sorted(b_set - s_set)
    only_shadow = sorted(s_set - b_set)
    print(f"bash log   : {args.bash_log}")
    print(f"  invariant_fail={len(b_fails)} (ids={sorted(b_set) or '-'})  "
          f"warn={b_warn}  WARN={b_warn_up}")
    print(f"shadow log : {args.shadow_log}")
    print(f"  OK ticks={s_ok}  FAIL={len(s_fails)} (ids={sorted(s_set) or '-'})  "
          f"events={s_events}")
    print("agreement:")
    print(f"  fail-ids in BOTH        : {agree or '-'}")
    print(f"  fail-ids only in bash   : {only_bash or '-'}")
    print(f"  fail-ids only in shadow : {only_shadow or '-'}")
    divergence = len(only_bash) + len(only_shadow)
    print(f"  agreement={len(agree)}  divergence={divergence}")
    return 0 if divergence == 0 else 1


def main(argv=None):
    from ..core.termio import enable_line_buffering
    enable_line_buffering()                     # line-flush when the shadow log is a redirected pipe
    argv = argv if argv is not None else sys.argv[1:]
    p = argparse.ArgumentParser(prog="dpos_harness sim shadow",
                                description="read-only shadow runner for the Python battery")
    sub = p.add_subparsers(dest="cmd", required=True)

    pr = sub.add_parser("run", help="run the battery against the live sim (read-only)")
    pr.add_argument("--log", default=None, help="shadow log path (default $SIM_OUT/sim-shadow.log)")
    pr.add_argument("--period", type=float, default=None, help="tick period seconds (default $SIM_CHECK_PERIOD or 10)")
    pr.add_argument("--once", action="store_true", help="run a single tick and exit")
    pr.set_defaults(func=run)

    pc = sub.add_parser("compare", help="compare shadow verdicts against the bash sim log")
    pc.add_argument("--shadow-log", required=True)
    pc.add_argument("--bash-log", default=None)
    pc.set_defaults(func=lambda a: compare(_resolve_bash_log(a)))

    args = p.parse_args(argv)
    return args.func(args)


def _resolve_bash_log(args):
    if not args.bash_log:
        args.bash_log = discover_bash_log()
        if not args.bash_log:
            print("dpos_harness sim shadow compare: no bash sim log found (pass --bash-log)", file=sys.stderr)
            sys.exit(2)
    return args


if __name__ == "__main__":
    sys.exit(main())
