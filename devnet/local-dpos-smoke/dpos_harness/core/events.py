"""events.py — event log + structured failure bundle. Port of scripts/soak-bundle.sh.

  EventLog.sim_event(kind, note)  — append one JSON object to $SIM_OUT/events.jsonl AND mirror
    a human line to stdout. The seed is on EVERY line (a truncated log is still attributable).
    Context (round/epoch/committee/f/disrupted/pending) comes from the orchestrator state.
  EventLog.bundle_dump(reason, inv_id) — write $SIM_OUT/bundle-<ts>/ with everything needed to
    diagnose + replay (plan §5), print summary.txt.

PORT-NOTE: the bash routed every read through `if out=$(cast …)` (never a bare assignment) to
survive set -e mid-capture; Python's try/except is the structural replacement. The heartbeat
`[sim r<N> <kind>] fin= epoch= f= <note>` line SHAPE is PINNED — `sim/status.py` and the load
blaster's marker gate both grep it — so it is reproduced field-for-field.

RENAME-NOTE: the bash emitted `[soak r<N> …]` and its own tools grep for that. The two log
dialects no longer cross: the bash blaster (`scripts/load-heavy.sh`) gating on a PYTHON sim log
needs its marker regex overridden, and vice versa. Use the matching pair (`dpos_harness load
start` with `dpos_harness sim run`) and the question does not arise.
"""

from __future__ import annotations

import datetime
import glob
import json
import os
import tarfile
from dataclasses import dataclass

from . import proc

# THE heartbeat marker, owned here because `_emit` below is what prints it. Any consumer that
# tails an event log and gates on "the run has started" matches THIS, rather than keeping its own
# copy of the literal: `stack.sender`'s funding gate used to carry `r"\[sim r[0-9]+ "` inline,
# which made the framework's load blaster depend on the simulation's output format and gave the
# format two owners that could drift apart (C9).
HEARTBEAT_MARKER_RE = r"\[sim r[0-9]+ "
HEARTBEAT_MARKER_HUMAN = "[sim r<N> "

# Run artifacts land under the smoke dir, where sim-out/ is gitignored. Anchored to THIS file's
# location so it resolves regardless of the caller's cwd — a cwd-relative default scattered the
# event log and the bundles wherever the operator happened to stand, while `stack/golden.py`
# always anchored, so ONE run's artifacts could split across two directories. DEPTH-SENSITIVE:
# this file lives at <smoke>/dpos_harness/core/events.py, so the smoke dir is THREE dirnames up
# (test_path_anchors.py pins that). SIM_OUT still overrides.
_SMOKE_DIR = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SIM_OUT = os.environ.get("SIM_OUT", os.path.join(_SMOKE_DIR, "sim-out"))
SIM_LOG_BYTES = int(os.environ.get("SIM_LOG_BYTES", "5000000"))
SIM_LOG_TAIL = int(os.environ.get("SIM_LOG_TAIL", "50000"))   # docker --tail line cap (F17)


def _now_iso():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _now_stamp():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")


@dataclass
class EventContext:
    """The run-identifying fields every event line and every failure bundle carries.

    WHY IT EXISTS: the event log used to be handed the simulation's `SimState` and DUCK-TYPE
    six attributes plus `state.cfg.seed` off it. `core/` is the bottom layer — nothing below the
    simulation could log an event without constructing the simulation's whole state object, so
    the three case modules and the CLI passed nothing at all and silently logged an empty
    context. These fields are exactly what the log and the bundle render, and a caller with no
    round/committee to report supplies the defaults deliberately instead of by accident."""
    seed: str = ""
    round: int = 0
    epoch: int = 0
    f: int = 0
    committee: str = ""        # space-separated owner addresses
    disrupted: str = ""        # space-separated container names
    pending: str = ""          # space-separated "kind@epoch"


class EventLog:
    """Owns events.jsonl + the failure bundle. `context` is a CALLABLE returning an
    `EventContext`, re-read on every event — the values move each round, so a snapshot taken at
    construction would freeze the log at round 0. Omitting it logs the neutral context."""

    def __init__(self, context=None, out_dir: str = None, dry_run: bool = False):
        self.context = context
        self.out = out_dir or SIM_OUT
        self.dry_run = dry_run
        # DEMOTED TRIPS (2026-07-30): fail_id -> {count, first, first_ts}. A demoted invariant does
        # not fail the run — it files a "diag" event — and nothing in the bundle named one, so a
        # demoted trip was visible only if some OTHER detector failed first, and then only as a
        # buried "kind":"diag" line inside a copied-whole, run-spanning events.jsonl. Accumulated
        # IN-PROCESS (not by re-reading events.jsonl, which is append-only and shared across runs —
        # every bundle's copy starts with lines from earlier, even bash-era, runs).
        self.demoted_trips = {}

    def _ctx(self) -> EventContext:
        return self.context() if self.context else EventContext()

    def _finalized(self):
        try:
            from . import nodes
            return nodes.finalized_dec()
        except Exception:
            return 0

    def sim_event(self, kind: str, note: str = ""):
        """Append a JSON event + mirror the pinned human heartbeat line. No-op under dry_run
        (the startup self-check must touch no RPC/log)."""
        if self.dry_run:
            return
        os.makedirs(self.out, exist_ok=True)
        ctx = self._ctx()
        fin = self._finalized()
        rec = {
            "ts": _now_iso(), "round": ctx.round, "seed": ctx.seed,
            "kind": kind, "intended": "", "verdict": "", "applied_action": "",
            "finalized_block": fin, "epoch": ctx.epoch, "f": ctx.f,
            "committee": (ctx.committee or "").split(),
            "disrupted": (ctx.disrupted or "").split(),
            "pending": (ctx.pending or "").split(),
            "note": note,
        }
        if kind == "diag":
            self._record_demoted_trip(note, rec["ts"], rec["round"])
        # events.jsonl drives external tooling — flush per event so a tailing consumer sees it
        # immediately (not on a block-buffer boundary or process exit).
        with open(os.path.join(self.out, "events.jsonl"), "a") as f:
            f.write(json.dumps(rec) + "\n")
            f.flush()
            os.fsync(f.fileno())
        # PINNED heartbeat: soak-status.sh render_progress_section + load-heavy's marker grep read
        # `  [sim r<N> <kind>] fin=<> epoch=<> f=<> <note>`. flush=True so a redirected sim log
        # shows the line at once (the marker gate + operator monitor both depend on it).
        print(f"  [sim r{ctx.round} {kind}] fin={fin} "
              f"epoch={ctx.epoch} f={ctx.f} {note}", flush=True)

    def _record_demoted_trip(self, note: str, ts: str, rnd):
        """Account one demoted-invariant trip. Both diag producers prefix the id in brackets
        (battery._inv_fail and chain.writes.fatal_or_diag emit `[<fail_id>] <msg>`); the one that does
        not (actions._marshal_ack_tripwire, which raises anyway) is filed under `unlabelled` rather
        than dropped."""
        fid = "unlabelled"
        if note.startswith("[") and "]" in note:
            fid = note[1:note.index("]")]
        t = self.demoted_trips.get(fid)
        if t is None:
            self.demoted_trips[fid] = {"count": 1, "first": note, "first_ts": ts, "first_round": rnd}
        else:
            t["count"] += 1

    def _demoted_trip_lines(self):
        """The bundle-summary block. One line per (id, occurrences, first sighting) — the diag
        LATCH means `count` is per distinct entity, not per tick, so a 2 there means two seats /
        members / addresses tripped it, which is the thing worth reading in a failure bundle."""
        if not self.demoted_trips:
            return ["DEMOTED TRIPS   : none this run"]
        out = [f"DEMOTED TRIPS   : {len(self.demoted_trips)} demoted invariant(s) fired this run "
               "(diagnostic — they did NOT fail it):"]
        for fid, t in sorted(self.demoted_trips.items()):
            out.append(f"  {fid} x{t['count']} — first r{t['first_round']} {t['first_ts']}: "
                       f"{t['first']}")
        return out

    def bundle_dump(self, reason: str, inv_id: str = "unknown"):
        """Structured bundle on the first invariant_fail / trap. Best-effort per artifact (a down
        node's logs/RPC may be unavailable — that absence is itself a signal)."""
        ctx = self._ctx()
        ts = _now_stamp()
        d = os.path.join(self.out, f"bundle-{ts}")
        for sub in ("logs", "rpc", "compose"):
            os.makedirs(os.path.join(d, sub), exist_ok=True)
        fin = self._finalized()

        # "halt just before this round" — clamped at 0, since a caller with no round to report
        # (the case modules and the CLI bundle command) would otherwise print a nonsense -1.
        stop_round = max(ctx.round - 1, 0)
        summary = "\n".join([
            "SIM FAILURE BUNDLE",
            f"broken invariant : {inv_id}",
            f"reason           : {reason}",
            f"seed             : {ctx.seed}",
            f"round            : {ctx.round}",
            f"finalized_block  : {fin}",
            f"epoch            : {ctx.epoch}",
            f"committee        : {ctx.committee}",
            f"f                : {ctx.f}",
            f"disrupted        : {ctx.disrupted}",
            f"pending          : {ctx.pending}",
            "",
            *self._demoted_trip_lines(),
            "",
            "REPLAY (reproduces the INTENT schedule; applied churn may differ — plan §1.2):",
            f"  SIM_SEED={ctx.seed} make smoke-sim",
            f"  (SIM_STOP_ROUND={stop_round} to halt just before this round)",
            "",
        ])
        with open(os.path.join(d, "summary.txt"), "w") as f:
            f.write(summary)

        ev = os.path.join(self.out, "events.jsonl")
        if os.path.isfile(ev):
            with open(ev) as src, open(os.path.join(d, "events.jsonl"), "w") as dst:
                dst.write(src.read())

        # ALL container logs (tail-bounded), enumerating EVERY compose service dynamically (a
        # reborn-validator-N slot serves an identity the fixed loop would omit). F17: bound the
        # docker fetch with --tail (lines) so a multi-GB log is never streamed into memory just to
        # byte-slice it, and LOG A MARKER for any service whose logs could not be captured — a
        # down node's absence is itself a diagnostic signal, never a silent gap.
        services = self._services()
        missed = []
        if not services:
            missed.append("<ALL — `docker compose ps -a --services` enumerated nothing, so this "
                          "bundle has no per-service logs>")
        for svc in services:
            r = proc.read_capture(
                ["docker", "compose", "logs", "--no-color", "--tail", str(SIM_LOG_TAIL), svc],
                timeout=30, binary=True)
            if not r.ok or not r.raw:
                # The seam turns a spawn failure/timeout into rc=-1/124 with a sentinel stderr,
                # so the old `except` branch is now this same line — one shape for every way a
                # capture can come back empty, and the reason is recorded either way.
                missed.append(f"{svc} (rc={r.rc}, {len(r.raw)} bytes)")
                continue
            try:
                with open(os.path.join(d, "logs", f"{svc}.log"), "wb") as f:
                    f.write(r.raw[-SIM_LOG_BYTES:])
            except OSError as e:            # unwritable bundle dir is itself a signal
                missed.append(f"{svc} ({type(e).__name__})")
        if missed:
            with open(os.path.join(d, "logs", "_MISSED_SERVICES.txt"), "w") as f:
                f.write("services whose logs could NOT be captured (their absence is a signal):\n"
                        + "\n".join(missed) + "\n")
            print(f"bundle: {len(missed)} service(s) had no capturable logs — see logs/_MISSED_SERVICES.txt")

        # Base compose files PLUS the per-run generated overlays (soak-bundle.sh:154-160). The base
        # files say nothing about WHICH on-chain identity a re-tasked slot serves: that mapping
        # exists ONLY in `docker-compose.sim.reborn-<slot>.gen.yml` (IDENTITY_IDX,
        # hostpool.rebirth_identity), and which victim is currently equivocating exists ONLY in
        # `docker-compose.sim.byz-<victim>.gen.yml` (actions.act_byzantine). Both are unlinked on
        # exit by _sweep_generated_overlays, so a bundle that does not copy them loses the evidence
        # permanently — all 29 bundles from 2026-07-22/23 have zero reborn files, which is exactly
        # why a byzantine-slash-not-landed diagnosis had to INFER the container's identity mapping.
        for name in (["docker-compose.sim.gen.yml", "docker-compose.sim.dpos.gen.yml"]
                     + sorted(glob.glob("docker-compose.sim.reborn-*.gen.yml"))
                     + sorted(glob.glob("docker-compose.sim.byz-*.gen.yml"))):
            if os.path.isfile(name):
                try:
                    with open(name) as src, open(os.path.join(d, "compose", name), "w") as dst:
                        dst.write(src.read())
                except Exception:
                    pass

        print("================ SIM FAILURE ================")
        print(summary)
        print(f"bundle: {d}")
        print("=============================================")
        return d

    def _services(self):
        """Every compose service, enumerated from the live project — a reborn validator-N slot
        serves an identity a fixed loop would omit.

        There is NO topology fallback. This used to synthesise `validator-0..13` + `full-node`
        from a hardcoded 14 whenever the enumeration came back empty, which put a service list
        into the bundle that was neither read from anything nor true of the run: at any other
        committee size it named containers that never existed and omitted ones that did. An
        empty list here is reported as an explicit gap by `bundle_dump`; a wrong container count
        in a failure bundle is worse than an absent one, because it is read as evidence."""
        # docker unavailable → proc.read yields "" → an empty list, which bundle_dump reports
        # as an explicit gap (never a synthesised topology).
        out = proc.read(["docker", "compose", "ps", "-a", "--services"], timeout=15)
        return sorted(set(s for s in out.splitlines() if s.strip()))
