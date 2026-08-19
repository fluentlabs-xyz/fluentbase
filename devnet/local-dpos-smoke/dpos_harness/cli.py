"""cli.py — the package entrypoint, grouped by LAYER rather than by history.

  python -m dpos_harness sim   run [--dry-run-bringup|--dry-run-tick]
  python -m dpos_harness sim   status [LOG …] | shadow … | bundle REASON [ID]
  python -m dpos_harness case  growth | quorum | seed-continuity | list
  python -m dpos_harness stack compose-gen N [TARGET] [POOL] | golden [--check]
  python -m dpos_harness stack dry-run-static [--overlay FILE …]
  python -m dpos_harness load  start | stop [PIDFILE]

The surface used to be ten flat subcommands, half of them parsed positionally by hand. The
groups mirror the package layout, so `sim …` is the simulation, `stack …` is the devnet stack
and `load …` is the blaster — a framework tool, not a sim feature. There are deliberately NO
back-compat aliases: the only callers are the four Makefile targets, repointed in the same
change.

ALL SIM_*/LOAD_* env knobs keep their bash DEFAULTS (only the prefixes moved with the rename).
This module does NOT edit any bash file.

Every group imports its command's module LAZILY, inside the handler. That is not style: it is
what keeps `python -m dpos_harness load stop` from importing the orchestrator, the battery and
the whole sim just to kill a pidfile.
"""

from __future__ import annotations

import argparse
import os
import sys

from .core.exit_codes import (RC_ERROR, RC_FAIL, RC_INCONCLUSIVE, RC_PASS,  # noqa: F401
                              RC_USAGE)
from .core.termio import enable_line_buffering

# case name  ->  module under `cases/`. `case <name>` and `case list` both read this single
# registry, so a case added to one is never missing from the other.
#
# The `smoke-*` entries are the ported `scripts/case-*.sh` drivers. They all accept `--dry-run`,
# which walks the whole choreography with canned readings and prints the command transcript — the
# only fidelity evidence available against the bash without a docker daemon.
CASES = {
    "growth": "growth",
    "quorum": "quorum",
    "seed-continuity": "seed_continuity",
    "smoke-rejump-signer": "smoke.rejump_signer",
    "smoke-base": "smoke.base",
    "smoke-tx": "smoke.tx",
    "smoke-epoch": "smoke.epoch",
    "smoke-vrf": "smoke.vrf",
    "smoke-vrf-boundary": "smoke.vrf_boundary",
    "smoke-weighted-vrf": "smoke.weighted_vrf",
    "smoke-fault": "smoke.fault",
    "smoke-deferred": "smoke.deferred",
    "smoke-peers": "smoke.peers",
    "smoke-crash-survivor": "smoke.crash_survivor",
    "smoke-full-restart": "smoke.full_restart",
    "smoke-vrf-fault": "smoke.vrf_fault",
    "smoke-vrf-dkg-liveness": "smoke.vrf_dkg_liveness",
    "smoke-cert-follow": "smoke.cert_follow",
    "smoke-cert-cascade": "smoke.cert_cascade",
    "smoke-tx-cascade": "smoke.tx_cascade",
    "smoke-liveness": "smoke.liveness",
    "smoke-byzantine": "smoke.byzantine",
    "smoke-cert-catchup": "smoke.cert_catchup",
    "smoke-vrf-dkg-restart-midwindow": "smoke.vrf_dkg_restart_midwindow",
    # The PRODUCTION-PATH cluster (chunk 5b). These four run on a DIFFERENT devnet from the twenty
    # above — six validators, a bare genesis and a staking cluster deployed at runtime by forge —
    # so they need foundry and a solidity-contracts checkout, and they are the long ones. There
    # was a fifth, `smoke-byzantine-vrf`; it is RETIRED. Its whole subject was a proposer forging
    # the `PK_E` a change-boundary block asserted, and no block asserts a key any more — the
    # forgery is inexpressible, its `--dpos.byzantine forge-beacon-pk` mode `bail!`s at DPoS
    # start, and both of its witness greps had zero hits in the tree.
    "smoke-production-path": "smoke.production_path",
    "smoke-vrf-rotation": "smoke.vrf_rotation",
    "smoke-vrf-dkg-halt": "smoke.vrf_dkg_halt",
    "smoke-vrf-dkg-durability": "smoke.vrf_dkg_durability",
}


#: THE GATE'S SUITE — the AGGREGATES, not their constituents. `smoke-base` and `smoke-fault` run
#: the very same assertion FUNCTIONS the nine single-entry cases run (object identity, not copied
#: bodies — pinned by `tests/test_cli_case_all.py`), so listing both would buy nine extra bring-ups
#: and not one extra check. The nine stay in `CASES` for isolated debugging.
#:
#: ORDER IS CHEAP-AND-READ-ONLY FIRST, DESTRUCTIVE LATER, the production-path substrate LAST: a
#: failure stops the run before anything worse has been done to a stack, and the five prod cases
#: are the long ones that need foundry and a solidity-contracts checkout.
SUITE = [
    "smoke-base", "smoke-weighted-vrf", "smoke-rejump-signer",
    "smoke-liveness", "smoke-fault", "smoke-vrf-dkg-liveness",
    "smoke-cert-follow", "smoke-cert-cascade", "smoke-tx-cascade",
    "smoke-byzantine", "smoke-cert-catchup", "smoke-vrf-dkg-restart-midwindow",
    "growth", "quorum", "seed-continuity",
    "smoke-production-path", "smoke-vrf-rotation", "smoke-vrf-dkg-halt",
    "smoke-vrf-dkg-durability",
]

_RC_NAME = {RC_PASS: "PASS", RC_FAIL: "FAIL", RC_USAGE: "USAGE",
            RC_ERROR: "ERROR", RC_INCONCLUSIVE: "INCONCLUSIVE", 130: "INTERRUPTED"}

#: HOW BAD an outcome is, for picking the suite's exit code. The raw code VALUES cannot serve:
#: `RC_INCONCLUSIVE` is 4 and a `max()` over them would rank it above `RC_ERROR` (3) and
#: `RC_FAIL` (1) — an unevaluated property outranking a real failure.
#:
#: INCONCLUSIVE IS WORSE THAN A PASS and therefore non-zero. A case returns it exactly where a
#: reading failed and the property was never evaluated at all (`seed-continuity`, when the beacon
#: metrics do not answer). An unchecked property is not a checked one, and reporting it as passed
#: is the same silent lie this suite exists to remove. It is not a failure either, so it gets its
#: own code and its own banner line.
#:
#: 130 IS DELIBERATELY ABSENT. An interrupt is not an outcome to rank — folding it in would let a
#: Ctrl-C mask a genuine FAIL from an earlier case. It rides a flag instead.
_RC_RANK = {RC_PASS: 0, RC_INCONCLUSIVE: 1, RC_USAGE: 2, RC_ERROR: 3, RC_FAIL: 4}


def _case_all(args) -> int:
    """Run the suite, one case per SUBPROCESS, strictly sequentially.

    A SUBPROCESS AND NOT A CALL, for two reasons that survive the Phase-3 env-restore fix. The
    environment is captured at IMPORT time in four modules — `core/nodes.py` binds `RPC`,
    `core/rpc.py` / `core/events.py` / `stack/sender.py` bind their timeouts — so the first case to
    import them freezes those values for every later case in the same interpreter. And
    `stack/bringup.py` writes `DPOS_ACTIVATION_BLOCK`, which `StaticProfile` reads at CALL time, so
    a sim case running before a static one would hand it a foreign activation block and the whole
    epoch arithmetic would go wrong with nothing to show for it. A process boundary closes both at
    once, which is how `run-all.sh` worked.

    SEQUENTIAL, never parallel: every case brings a stack up on fixed host ports and fixed IPs in
    `172.20.0.0/24`, so two at a time would fight over them.
    """
    from .core import proc, topology

    dry = bool(getattr(args, "dry_run", False))
    names = (os.environ.get("SMOKE_CASES") or "").split() or SUITE
    unknown = [n for n in names if n not in CASES]
    if unknown:
        print(f"unknown case(s) in suite: {unknown}", flush=True)
        return RC_USAGE

    # The one command the aggregate issues itself. Through a Runner so `--dry-run` suppression is
    # the seam's job and not a hand-written `if` here; `record=False` because the suite is the
    # thing that RUNS choreography, not a piece of it, and its transcript belongs to the children.
    cleanup = proc.Runner(dry=dry, record=False)
    result, worst, interrupted = {}, RC_PASS, False
    for name in names:
        print(f"==================== {name} ====================", flush=True)
        argv = [sys.executable, "-m", __package__, "case", name] + (["--dry-run"] if dry else [])
        rc = proc.run_streaming(argv)
        # RECORDED BEFORE ANY BRANCH BELOW CAN LEAVE THE LOOP: an interrupt that escaped without
        # being noted printed "ALL SMOKE CASES PASSED" with exit 0 over a table of MISSING rows.
        result[name] = rc
        if rc == 130:
            interrupted = True
            break
        if _RC_RANK.get(rc, _RC_RANK[RC_ERROR]) > _RC_RANK[worst]:
            worst = rc
        # Defensive, because there is no preflight `down -v` before a STATIC bring-up
        # (`stack/static_stack.py`) — only the generated-profile one has it.
        #
        # PER PROJECT, and a bare `docker compose down` is NOT enough. Every compose ROOT pins its
        # own `name:`, so this suite spans four docker projects: the static cases run in
        # `fluent-dpos-smoke`, `growth`/`quorum`/`seed-continuity` in `fluent-dpos-sim`, and the
        # five production-path cases in `fluent-dpos-prod-path`. A bare `down` from this directory
        # resolves `docker-compose.yml` and therefore reaps ONLY the first of them — `-v` and
        # `--remove-orphans` widen what is removed WITHIN a project, never across projects. All
        # three live roots publish host 8545 and 8546, so a case that died before its own teardown
        # leaves ports bound that the next case cannot bind, and the cleanup meant to prevent
        # exactly that would have walked straight past it.
        for project in topology.DOCKER_PROJECTS:
            cleanup.run_ok(["docker", "compose", "-p", project, "down", "-v", "--remove-orphans"],
                           timeout=300, note=f"suite-cleanup-{project}")

    print("==================== summary ====================", flush=True)
    for name in names:
        print(f"  {name:<34} {_RC_NAME.get(result.get(name), 'MISSING')}", flush=True)
    if interrupted:
        print("SMOKE RUN INTERRUPTED — the remaining cases did not run", flush=True)
    if worst == RC_PASS and interrupted:
        return 130
    print({RC_PASS: "ALL SMOKE CASES PASSED",
           RC_INCONCLUSIVE: "SOME SMOKE CASES COULD NOT BE JUDGED",
           }.get(worst, "SOME SMOKE CASES FAILED"), flush=True)
    return worst


# ── sim ──────────────────────────────────────────────────────────────────────

def _sim_run(args) -> int:
    from .sim.orchestrator import Orchestrator
    orch = Orchestrator()
    if args.dry_run_bringup:
        return orch.dry_run_bringup()
    if args.dry_run_tick:
        return orch.dry_run_tick()
    return orch.run()


def _sim_status(rest) -> int:
    from .sim import status
    return status.main(rest)


def _sim_shadow(rest) -> int:
    from .sim import shadow
    return shadow.main(rest)


# `sim status` and `sim shadow` own their own argparse (`[LOG] [--watch [SECS]]`, `run|compare`
# with --log/--period/--once/--bash-log). Their tails are split off by hand, BEFORE the parser
# below runs, and forwarded verbatim. `argparse.REMAINDER` looks like it would do this and does
# not: it only starts collecting at the first non-option token, so `sim status --watch 5` — no
# LOG, which is the common invocation — dies on "unrecognized arguments" instead of reaching the
# sub-tool. Forwarding raw also keeps `--help` meaning the SUB-tool's help, as it did before.
PASSTHROUGH = {("sim", "status"): _sim_status, ("sim", "shadow"): _sim_shadow}


def _sim_bundle(args) -> int:
    from .core.events import EventLog
    EventLog().bundle_dump(args.reason, args.inv)
    return 0


# ── case ─────────────────────────────────────────────────────────────────────

def _case_run(args) -> int:
    import importlib
    mod = importlib.import_module(f".cases.{CASES[args.case]}", __package__)
    # `--dry-run` is declared as a real flag rather than left to `rest`, for the reason the
    # PASSTHROUGH comment below already records about `argparse.REMAINDER`: REMAINDER only starts
    # collecting at the first NON-option token, so a leading `--dry-run` dies on "unrecognized
    # arguments" before the case ever sees it. Declared here, it is forwarded verbatim, so a case
    # keeps ONE argv contract regardless of who invoked it.
    rest = (["--dry-run"] if getattr(args, "dry_run", False) else []) + list(args.rest or [])
    return mod.run_case(rest)


def _case_list(_args) -> int:
    for name in CASES:
        print(name)
    return RC_PASS


# ── stack ────────────────────────────────────────────────────────────────────

def _stack_compose_gen(args) -> int:
    from .stack import compose_gen
    base, dpos = compose_gen.generate(args.n, args.target, args.pool)
    print(f"generated {base} + {dpos} for N={args.n}")
    return 0


def _stack_dry_run_static(args) -> int:
    """Print the STATIC profile's bring-up command transcript and exit.

    The static stack is what the smoke cases run on, and its whole value is that its command
    sequence matches `lib.sh` argv for argv. This is the only way to check that without docker:
    every write command, in order, with its env delta. `DPOS_EXTRA_COMPOSE` is honoured, so the
    per-case overlay shows up in the recreate line exactly where a case would put it.
    """
    from .core.proc import Runner
    from .stack.profiles import StaticProfile
    from .stack.static_stack import StaticStack

    profile = StaticProfile.from_env(
        extra_overlays=(tuple(args.overlay) if args.overlay else None))
    runner = Runner(dry=True, echo=True)
    print(f"# dpos_harness dry-run-static (profile={profile.name}, "
          f"interval={profile.epoch_interval}, activation={profile.activation_block}, "
          f"overlays={list(profile.extra_overlays)})")
    StaticStack(profile=profile, runner=runner).bring_up_dpos()
    print(f"# {len(runner.log)} commands")
    return 0


def _stack_dry_run_rotation(args) -> int:
    """Print `pp_bring_up_rotation`'s command transcript and exit.

    The production-path counterpart of `dry-run-static`, and it exists for the same reason: the
    14-phase runtime-forge bring-up under the five production-path cases has no prior art in
    Python, so its argv sequence is the only thing that can be checked against `lib.sh` without a
    docker daemon."""
    from .cases.smoke import prod
    return prod.dry_run_transcript(label=args.label)


def _stack_golden(args) -> int:
    """The golden snapshot is keyed to the growth case's profile, so the CASE builds the spec
    and hands it down — `stack/golden.py` no longer reaches up to fetch it for itself."""
    from .cases import growth
    from .stack import golden
    spec = growth.golden_spec()
    if args.check:
        fresh = golden.is_golden_fresh(spec)
        print(f"golden: {'FRESH' if fresh else 'STALE/ABSENT'} "
              f"(hash={golden.golden_hash(spec)}, tarball={golden.TARBALL})")
        return 0
    golden.build_golden(spec)
    return 0


# ── load ─────────────────────────────────────────────────────────────────────

def _load_start(_args) -> int:
    from .stack import sender
    return sender.Sender().run()


def _load_stop(args) -> int:
    from .stack import sender
    return sender.stop(args.pidfile)


# ── parser ───────────────────────────────────────────────────────────────────

def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="python -m dpos_harness")
    groups = p.add_subparsers(dest="group", metavar="{sim,case,stack,load}")

    # -- sim ------------------------------------------------------------------
    sim = groups.add_parser("sim", help="the sim simulation and its read-only observers")
    sim.set_defaults(_parser=sim)
    sim_cmds = sim.add_subparsers(dest="cmd", metavar="{run,status,shadow,bundle}")

    run = sim_cmds.add_parser("run", help="run the simulation (SIM_* env)")
    run.add_argument("--dry-run-bringup", action="store_true",
                     help="print the bring-up command transcript and exit")
    run.add_argument("--dry-run-tick", action="store_true",
                     help="print one dry churn tick and exit")
    run.set_defaults(fn=_sim_run)

    # status / shadow are listed so `sim --help` names them; their args never reach here (see
    # PASSTHROUGH above) and the sub-tool prints its own usage.
    sim_cmds.add_parser("status", add_help=False,
                        help="read-only status dashboard — [LOG] [--watch [SECS]]")
    sim_cmds.add_parser("shadow", add_help=False,
                        help="read-only shadow battery runner — run … | compare …")

    bd = sim_cmds.add_parser("bundle", help="dump a failure bundle")
    bd.add_argument("reason", nargs="?", default="manual")
    bd.add_argument("inv", nargs="?", default="manual")
    bd.set_defaults(fn=_sim_bundle)

    # -- case -----------------------------------------------------------------
    case = groups.add_parser("case", help="self-contained regression cases")
    case.set_defaults(_parser=case)
    case_cmds = case.add_subparsers(dest="cmd", metavar="{" + ",".join(CASES) + ",list}")
    for name in CASES:
        c = case_cmds.add_parser(name, help=f"the {name} case (brings up + tears down)")
        c.add_argument("--dry-run", action="store_true",
                       help="walk the choreography with canned readings and print the command "
                            "transcript; no docker, no chain")
        c.add_argument("rest", nargs=argparse.REMAINDER)
        c.set_defaults(fn=_case_run, case=name)
    allp = case_cmds.add_parser("all", help="run the whole gate suite, one case per subprocess")
    allp.add_argument("--dry-run", action="store_true",
                      help="forward --dry-run to every case and skip the inter-case cleanup; "
                           "no docker, no chain")
    allp.set_defaults(fn=_case_all)
    case_cmds.add_parser("list", help="print the case names").set_defaults(fn=_case_list)

    # -- stack ----------------------------------------------------------------
    stack = groups.add_parser("stack",
                              help="the devnet stack: compose generation, golden snapshot")
    stack.set_defaults(_parser=stack)
    stack_cmds = stack.add_subparsers(
        dest="cmd", metavar="{compose-gen,golden,dry-run-static,dry-run-rotation}")

    cg = stack_cmds.add_parser("compose-gen", help="generate the compose pair")
    cg.add_argument("n", type=int, help="number of validator containers")
    cg.add_argument("target", type=int, nargs="?", default=None, help="target active-set size")
    cg.add_argument("pool", type=int, nargs="?", default=None, help="identity pool size")
    cg.set_defaults(fn=_stack_compose_gen)

    drs = stack_cmds.add_parser(
        "dry-run-static",
        help="print the static-profile bring-up transcript (the lib.sh fidelity oracle)")
    drs.add_argument("--overlay", action="append", default=None, metavar="FILE",
                     help="per-case compose overlay (repeatable); "
                          "defaults to $DPOS_EXTRA_COMPOSE")
    drs.set_defaults(fn=_stack_dry_run_static)

    drr = stack_cmds.add_parser(
        "dry-run-rotation",
        help="print pp_bring_up_rotation's transcript (the production-path fidelity oracle)")
    drr.add_argument("--label", default="smoke-vrf-rotation", metavar="PP_ROT_LABEL",
                     help="the label every FAIL line carries (bash PP_ROT_LABEL)")
    drr.set_defaults(fn=_stack_dry_run_rotation)

    gd = stack_cmds.add_parser("golden", help="build/check the golden DPoS-active snapshot")
    gd.add_argument("--check", action="store_true", help="report freshness without building")
    gd.set_defaults(fn=_stack_golden)

    # -- load -----------------------------------------------------------------
    load = groups.add_parser("load", help="the load blaster lifecycle")
    load.set_defaults(_parser=load)
    load_cmds = load.add_subparsers(dest="cmd", metavar="{start,stop}")
    load_cmds.add_parser("start", help="launch the blaster").set_defaults(fn=_load_start)
    lstop = load_cmds.add_parser("stop", help="stop the blaster")
    lstop.add_argument("pidfile", nargs="?", default=None)
    lstop.set_defaults(fn=_load_stop)

    return p


def main(argv=None) -> int:
    enable_line_buffering()
    argv = list(argv if argv is not None else sys.argv[1:])
    passthrough = PASSTHROUGH.get(tuple(argv[:2]))
    if passthrough is not None:
        return passthrough(argv[2:])

    p = build_parser()
    args = p.parse_args(argv)
    fn = getattr(args, "fn", None)
    if fn is None:
        # a bare invocation, or a group named with no command under it. Print the usage of the
        # level the operator actually reached, not the top one — `dpos_harness sim` should say
        # what `sim` offers.
        getattr(args, "_parser", p).print_usage(sys.stderr)
        return RC_USAGE
    return fn(args)


if __name__ == "__main__":
    sys.exit(main())
