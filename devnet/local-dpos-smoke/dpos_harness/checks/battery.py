"""battery.py — the continuous invariant battery (port of soak-invariants.sh).

check_invariants() runs the whole battery against ONE captured snapshot per tick
and RETURNS a status: ok=True, or a violation with inv_fail_id / inv_fail_msg set
(so the orchestrator can write a structured bundle instead of a bare exit). Each
detector is one method returning True (holds) or False (via _inv_fail, which sets
inv_fail_id/msg). SOFT reporters (witness-watch, dkg-finalize-deferred,
exec-saturation, block-rate) always return True and only emit events.

Ported as of the working-tree HEAD 8d090fc2 (soak-invariants.sh, which a Phase-5
agent is concurrently editing — this port reflects the snapshot read at start,
including every phase-4/5 belt/settle-gate). The two vote-bitmap detectors
(participation-scrape, fair-credit) are RETIRED with the bitmap itself — see the
retirement note at their old site.

VERDICT SEMANTICS ARE PRESERVED EXACTLY: same fail ids, same WARN/soft
classifications, same belts/thresholds, same SIM_* knob names (read from the
same env vars). The rolling-state globals are instance attributes mutated in
place, byte-for-byte as the bash inline blocks did. Call ORDER in
check_invariants matches the bash inline order (load-bearing: finalize_stall sets
flat_ticks, which member_liveness / cascade / write_path / spec_head read).

TWO CONTEXTS, TWO CLASSES (P4 item 4.4). `Ctx` carries chain/topology/metrics facts and the
threshold knobs — what ANY consumer driving a stack can supply. `SimCtx` carries the
simulation's churn bookkeeping. `ChainBattery` holds the detectors that need only the first and
has no `sim` attribute at all; `Battery(ChainBattery)` adds the four that need the second, plus
the ordered dispatch. The sim's `sim/state_ctx.py` is the adapter that builds both from
SimState. Classification (derived and enforced by tests/test_checks_context.py — a detector
that starts reading sim state must MOVE to Battery, and one that stops must move up):

  CHAIN-ONLY — 26 of 30, reusable by a non-simulation consumer (on ChainBattery)
    data-root-fill, host-mem, rpc-alive, node-panic, safety-halt, witness-counters,
    dkg-pinned-idx, beacon-active, finalize-stall, klag, deferred-park, witness-embedded,
    witness-watch, dkg-finalize-deferred, exec-saturation, block-rate, spec-head, seed-round,
    beacon-window, result-agreement, finalized-chain, member-liveness, battery-coverage,
    peer-plane, cascade-liveness, write-path

  SIM-COUPLED — 4 of 30, by nature: they judge the harness's own churn (on Battery)
    node-down             — needs the spare-band / idle-rotation-slot predicate
    dkg-commit-binding    — needs the seat + refill state machine
    wrongful-slash        — needs EVER_FAULTED to tell a wrongful jail from an expected one
    committee-membership  — needs the refill/promote pending set and its stall verdict

PORT-NOTE: bash's subshell / set-e / assoc-array hazards cannot exist here —
functions return values (no `$(...)` write-loss), exceptions are explicit (no
errexit branch-tail), dicts KeyError/`.get` (no bare-subscript mis-parse). The
SEMANTICS are identical; the contortions die. Impure reads (RPC/metrics/docker
logs) are seam methods that default to core.nodes and are overridable in
tests, mirroring the gate-test seam-override discipline.
"""

from __future__ import annotations

import os
import re
import time

from ..core import nodes, topology
from ..core.counters import counter_progress
from ..core.setops import in_set as _in_set


# ── knob env reads (the `: "${VAR:=default}"` defaults, same names) ──────────
def _envs(name, default):
    return os.environ.get(name, default)


def _envi(name, default):
    try:
        return int(os.environ.get(name, str(default)))
    except ValueError:
        return int(default)


def _envf(name, default):
    try:
        return float(os.environ.get(name, str(default)))
    except ValueError:
        return float(default)


class _StrictCtx:
    """Base for both context objects: a keyword may only set a field the subclass already
    declared. `self.__dict__.update(kw)` accepted ANYTHING, so `Ctx(EVER_FAULTED=…)` silently
    grew the attribute and the split below would have been a naming convention, not a boundary.
    Rejecting unknown keys is what makes it structural: after the split, handing a simulation
    field to the generic Ctx raises, and reading one off it raises AttributeError."""

    def _apply(self, kw, kind):
        unknown = sorted(set(kw) - set(self.__dict__))
        if unknown:
            raise TypeError(
                f"{type(self).__name__} got unknown field(s) {unknown}. "
                f"{kind} If this is simulation churn bookkeeping it belongs on SimCtx, which the "
                f"battery reaches only through Battery.sim and only from the SIM-COUPLED "
                f"detectors listed in this module's header.")
        self.__dict__.update(kw)


# ── the GENERIC context: what ANY consumer of the battery can supply ─────────
class Ctx(_StrictCtx):
    """Per-tick chain / topology / metrics facts plus the threshold knobs. Everything here is
    observable by anyone driving a stack — a smoke case, a read-only shadow observer, a unit
    test — not just the simulation. Defaults mirror the bash `${VAR:-default}` fallbacks so an
    unset field behaves exactly as an unset bash global; a consumer that populates only what it
    can see on-chain leaves the rest at those defaults (and the coverage / detector-starved
    belts own the resulting vacuity, exactly as in bash).

    It holds NO simulation state. The churn bookkeeping that used to live here — the identity
    pool, the seat/refill machine, the fault ledger — is on SimCtx."""

    def __init__(self, **kw):
        # chain facts observed this tick
        self.SIM_TICK = 0
        self.SIM_ROUND = 0
        self.SIM_CUR_EPOCH = 0
        self.SIM_CUR_F = 1
        self.SIM_CUR_COMMITTEE = ""          # space-joined lowercased addrs
        # topology / deployment facts
        self.ADDR2IDX = {}                    # lowercased addr -> container/service
        self.SIM_NO_CASCADE = 0
        # consumer-declared perturbation windows. NOT sim bookkeeping: any consumer that stops a
        # node or induces a membership change has to declare it, or the liveness detectors judge
        # its own deliberate act as a product failure. This is the honest generic shape of what
        # the sim happens to compute from its churn state.
        self.SIM_DISRUPTED = ""              # space-joined container names, deliberately down
        self.PERMANENTLY_DEAD = {}            # container -> truthy: never coming back
        self.EXPECTED_STALL = 0               # a stall right now is expected, don't judge it
        self.SETTLE_UNTIL_EPOCH = 0           # membership is in flux until this epoch
        # threshold knobs
        self.MIN_COMMITTEE = 0
        self.SIM_BATTERY_COVERAGE = None     # None = unset (no-op), else int
        self.SIM_MEMBERSHIP_SETTLE = _envi("SIM_MEMBERSHIP_SETTLE", 5)
        self.SIM_GROW_LAND_DEADLINE = _envi("SIM_GROW_LAND_DEADLINE", 8)
        # env/topology facts (bash read them as `${VAR:-default}` off the exported environment).
        # They were bare `None` here, which permanently disabled the data-root-full watchdog and
        # the write-path reporter — the two detectors that own them.
        self.L3_SPAMMER_ADDR = _envs("L3_SPAMMER_ADDR", "")
        self.SIM_DATA_ROOT = _envs("SIM_DATA_ROOT", "")
        self.SIM_PEER_SPOKE = _envs("SIM_PEER_SPOKE", topology.CERT_UPSTREAM_ANCHOR)
        # Runtime contract addresses. The sim DEPLOYS staking/chain-config/liveness at run time,
        # so the genesis predeploys are codeless here and reading through them makes every
        # on-chain detector evaluate over nothing and report "holds". Empty is the honest default
        # — the orchestrator injects the post-deploy facts per tick (state_ctx.to_ctx overrides),
        # and a caller with nothing to inject (shadow, unit tests) keeps the nodes.py fallback.
        self.STAKING_RT = ""
        self.CHAIN_CONFIG_RT = ""
        self.LIVENESS_RT = ""
        self._apply(kw, "Ctx carries chain/topology/metrics facts and threshold knobs only.")


# ── the SIM context: churn bookkeeping only the simulation can produce ───────
class SimCtx(_StrictCtx):
    """The simulation's own churn bookkeeping. A non-simulation consumer has none of this and
    should not have to fake it: it drives the five SIM-COUPLED detectors and nothing else.

    Every field here answers a question only the thing DOING the churn can answer — which
    identity a container is currently hosting, which faults this harness caused, which seat is
    mid-refill. That is why these detectors are sim-coupled by nature rather than by accident,
    and why the split is 27/5 instead of 32/0."""

    def __init__(self, **kw):
        # identity-pool bookkeeping
        self.EVER_FAULTED = {}                # identity idx -> action THIS harness applied
        self.EVER_MEMBERS = {}                # addr -> anything: was ever seated
        self.OWNER_ADDR_CACHE = {}            # idx (str) -> lowercased owner addr
        # identity-pool banding (committee | refill spares | rotation slots)
        self.SIM_VALIDATORS = 0
        self.SIM_SPARE_END = 0
        self.SIM_VAL_CONTAINERS = 0
        # the seat / refill state machine
        self.NEXT_JOINER = 0
        self.GROW_LANDED = -1
        self.REFILL_RESUMED = ""
        self.REFILL_PENDING = ""
        self.PROMOTE_PENDING = ""
        self.REFILL_STALL_VERDICT = "climbing"
        self.SLOT_IDENTITY = {}               # container -> hosted identity idx (int)
        self._apply(kw, "SimCtx carries simulation churn bookkeeping only.")


# ── PURE verdict seams (unit-tested; no RPC, no state) ───────────────────────
def settle_gate_open(cur, settle, cap):
    """item 6c: True (RUN the detector) iff past settle OR the settle boundary is
    STUCK far in the future (more than <cap> epochs ahead of cur, so it re-arms);
    False (still legitimately blinded) otherwise."""
    if cur >= settle:
        return True
    if settle - cur > cap:
        return True
    return False


def node_panic_verdict(container, line):
    """PURE: a non-empty panic line is a hard node-panic fail (truncated ~200
    chars); empty -> holds. Returns (fail_id, msg) or None."""
    if not line:
        return None
    return ("node-panic",
            f"{container} PANICKED: {line[:200]} — a process panic on a sim "
            "container (product event attributed at the SOURCE, not a downstream "
            "bench/churn symptom — bundles 20260715T214344Z + 20260716T013222Z)")


def node_down_verdict(container, state, exitcode, line):
    """PURE: an exited/dead state is a hard node-down fail; any other state holds.
    Returns (fail_id, msg) or None."""
    if state not in ("exited", "dead"):
        return None
    return ("node-down",
            f"{container} is DOWN (state={state} exit={exitcode}): {line[:200]} — a "
            "sim container exited and stayed down (no restart policy in the compose). "
            "A CLEAN process exit is NOT a panic (leaves no 'panicked at' line, so the "
            "panic scan misses it) — attributed here at the SOURCE before a downstream "
            "refill/churn watchdog blind-waits on the dead RPC (bundle-20260716T113634Z)")


def wrongful_slash_verdict(jail, expected, mapped):
    """PURE: clear | wrongful | unknown. A jail that is expected (harness-faulted
    identity) clears; an unexpected jail on a mapped identity is wrongful; on an
    unmapped identity, unknown-attribution."""
    if jail != 1:
        return "clear"
    if expected == 1:
        return "clear"
    if mapped == 1:
        return "wrongful"
    return "unknown"


def dkg_qual_is_set(token):
    """PURE: True iff token is the QUALIFIED marker ("1")."""
    return token == "1"


def dkg_qual_state(token):
    """PURE: 1 (QUALIFIED), 0 (DEFERRED), or -1 (UNREADABLE — the "" of a blipped
    read). Callers branch on the THREE-valued state so an RPC blip is UNKNOWN,
    never mis-read as deferred."""
    if token == "1":
        return 1
    if token == "0":
        return 0
    return -1


def dkg_iff_verdict(changed, qs):
    """PURE: the qualify-before-commit REGRESSION check (I4 / v40-class). Returns
    (verdict, msg): committee[E] CHANGED ⟺ getDkgQual(E) NON-EMPTY.
      changed=1,qual=0 -> fail  (shareless open committed)
      changed=1,qual=1 -> ok    (qualified change landed)
      changed=0,qual=1 -> watch (candidate==incumbent re-qualification, benign)
      changed=0,qual=0 -> ok    (deferral / no-change boundary)
      qs=-1 (unreadable) -> watch (RPC blip, not judged)."""
    if qs < 0:
        return ("watch", "getDkgQual unreadable at the boundary — skipped (RPC blip, not judged)")
    if changed == 1 and qs == 0:
        return ("fail",
                "committee CHANGED at the epoch boundary but getDkgQual is EMPTY — a "
                "committee whose DKG never qualified was committed (shareless open; "
                "qualify-before-commit invariant I4 violated — the v40 class)")
    if changed == 0 and qs == 1:
        return ("watch",
                "getDkgQual is set but committee UNCHANGED across the boundary "
                "(candidate==incumbent / overlap re-qualification) — benign")
    return ("ok", "")


def dkg_barrier_qual_gate(verdict, qs):
    """PURE: reconcile a non-hold dkg-barrier verdict (boundary|cap) against the
    on-chain qualification truth. Returns (gate, kind): a non-hold barrier verdict
    is a genuine stuck-DKG FAIL only when the chain truth is UNKNOWN (qs=-1)."""
    if verdict == "hold":
        return ("pass", "hold")
    if qs == 0:
        return ("pass", "deferred")
    if qs == 1:
        return ("pass", "qualified")
    return ("fail", "unknown")


def backfill_overdue(cur, obligation, pause_credit, membership_settle, grow_land_deadline):
    """<cur> <obligation> [pause_credit] — True iff the ACTIONABLE epoch budget is
    exhausted (cur minus paused epochs > obligation + budget). Returns
    (overdue, budget)."""
    budget = membership_settle + grow_land_deadline
    return (cur - (pause_credit or 0) > obligation + budget, budget)


def grow_land_overdue(cur, fire, deadline_epochs, deferral_credit=0):
    """True iff the epoch-domain growth landing budget is exhausted AFTER crediting
    the deferred epochs."""
    return cur - (deferral_credit or 0) - fire > deadline_epochs


def cp_ncap_only(read_ok, n, cap, joiner_m1, grow_landed):
    """True iff GA_CHANGE_PENDING is held SOLELY by the n<cap hole (D6 coverage-loss
    visibility)."""
    return read_ok == 1 and n < cap and joiner_m1 <= grow_landed


class ChainBattery:
    """The CHAIN-ONLY half of the battery: every detector that judges chain, node or metric
    facts and nothing else. Holds all the rolling state and the impure seams (defaulting to
    core.nodes) so tests drive the detectors with synthetic inputs.

    This class has NO `sim` attribute — that is the boundary, not a comment. A non-simulation
    consumer of the framework instantiates ChainBattery, supplies a `Ctx`, and gets the 27
    detectors listed in the module header; if one of them ever reaches for simulation state it
    raises AttributeError here rather than working by accident on a Battery instance.

    Detector call ORDER lives in Battery.check_invariants, because the ordered battery is the
    full one; the rolling state those detectors share (SIM_FLAT_TICKS in particular) is
    initialised here and read by both halves."""

    def __init__(self, ctx: Ctx = None, events=None):
        self.ctx = ctx or Ctx()
        self.events = events if events is not None else []  # (kind, note) capture stub
        self.inv_fail_id = ""
        self.inv_fail_msg = ""
        self._diag_seen = set()        # latch: one diag per (demoted invariant, entity) per run

        # knobs
        self.RESULT_LAG_K = _envi("RESULT_LAG_K", 3)
        self.SIM_STALL_TICKS = _envi("SIM_STALL_TICKS", 4)
        self.SIM_BEACON_WINDOW = _envi("SIM_BEACON_WINDOW", 6)
        self.SIM_BEACON_CHECK_TICKS = _envi("SIM_BEACON_CHECK_TICKS", 6)
        self.SIM_HALT_SCRAPE_EVERY = _envi("SIM_HALT_SCRAPE_EVERY", 3)
        self.SIM_PARK_TICKS = _envi("SIM_PARK_TICKS", 6)
        self.SIM_SEEDROUND_EVERY = _envi("SIM_SEEDROUND_EVERY", 12)
        self.SIM_EXEC_SAT_THRESHOLD = _envf("SIM_EXEC_SAT_THRESHOLD", 0.7)
        self.SIM_EXEC_SAT_EARLY = _envf("SIM_EXEC_SAT_EARLY", 0.4)
        self.SIM_EXEC_SAT_TICKS = _envi("SIM_EXEC_SAT_TICKS", 3)
        self.SIM_RATE_REPORT_TICKS = _envi("SIM_RATE_REPORT_TICKS", 20)
        self.SIM_RATE_WARN = _envf("SIM_RATE_WARN", 0.5)
        self.SIM_SPEC_DEAD_MIN_RATE = _envf("SIM_SPEC_DEAD_MIN_RATE", 0.7)
        self.SIM_SETTLE_GATE_CAP = _envi("SIM_SETTLE_GATE_CAP", 12)
        self.SIM_DATA_FILL_WARN_PCT = _envi("SIM_DATA_FILL_WARN_PCT", 75)
        self.SIM_DATA_FILL_FAIL_PCT = _envi("SIM_DATA_FILL_FAIL_PCT", 90)
        self.SIM_MEM_AVAIL_WARN_GB = _envi("SIM_MEM_AVAIL_WARN_GB", 8)
        self.SIM_MEM_AVAIL_FAIL_GB = _envi("SIM_MEM_AVAIL_FAIL_GB", 5)
        self.SIM_RPC_DEAD_BELT = _envi("SIM_RPC_DEAD_BELT", 4)
        self.SIM_PANIC_FIRST_WINDOW = _envi("SIM_PANIC_FIRST_WINDOW", 30)
        self.SIM_PANIC_CONTAINERS_EVERY = _envi("SIM_PANIC_CONTAINERS_EVERY", 10)
        self.SIM_ONESHOT_SERVICES = _envs("SIM_ONESHOT_SERVICES", topology.GENESIS_INIT)
        self.SIM_FORK_RECHECK_DELAY = _envi("SIM_FORK_RECHECK_DELAY", 2)
        self.SIM_DET_STARVE_TICKS = _envi("SIM_DET_STARVE_TICKS", self.SIM_STALL_TICKS * 2)
        self.SIM_COVERAGE_MIN_TICKS = _envi("SIM_COVERAGE_MIN_TICKS", self.SIM_STALL_TICKS * 2)
        self.SIM_PEER_EVERY = _envi("SIM_PEER_EVERY", 6)
        self.SIM_MEMBERSHIP_BELT = _envi("SIM_MEMBERSHIP_BELT", self.SIM_STALL_TICKS)
        self.SIM_WRONGSLASH_EVERY = _envi("SIM_WRONGSLASH_EVERY", 12)
        self.SIM_WRONGSLASH_BELT = _envi("SIM_WRONGSLASH_BELT", 2)
        self.SIM_JAIL_STATUS_BYTE = _envi("SIM_JAIL_STATUS_BYTE", 3)
        self.SIM_FIN_WALK_MAX = _envi("SIM_FIN_WALK_MAX", 64)
        self.SIM_DKG_DEFER_FAIL_STREAK = _envi("SIM_DKG_DEFER_FAIL_STREAK", 3)
        self.SIM_PART_SCRAPE_MAX_WINDOWS = _envi("SIM_PART_SCRAPE_MAX_WINDOWS", 4)
        self.SIM_BLOCK_INTERVAL = _envf("SIM_BLOCK_INTERVAL", 1)
        self.SIM_MEMBERSHIP_SETTLE = self.ctx.SIM_MEMBERSHIP_SETTLE
        self.SIM_GROW_LAND_DEADLINE = self.ctx.SIM_GROW_LAND_DEADLINE

        self._reset_rolling()

    def _reset_rolling(self):
        # read-health gate
        self.SIM_RPC_GOOD_FIN = 0
        self.SIM_RPC_GOOD_ROUND = 0
        self.SIM_RPC_ZERO_TICKS = 0
        # finalize-stall
        self.SIM_LAST_FIN = -1
        self.SIM_FLAT_TICKS = 0
        # k-lag
        self.SIM_KLAG_CATCHUP_TICKS = 0
        self.SIM_CGAP_OVER_TICKS = 0
        # cascade / write-path
        self.SIM_L2_LAST = -1; self.SIM_L2_FLAT = 0
        self.SIM_L3_LAST = -1; self.SIM_L3_FLAT = 0
        self.SIM_WP_LAST = -1; self.SIM_WP_FLAT = 0
        # beacon-active window
        self.SIM_BEACON_SA_BASE = -1; self.SIM_BEACON_FB_BASE = -1
        self.SIM_BEACON_FIN_BASE = -1; self.SIM_BEACON_WIN_TICKS = 0
        # shared per-tick scrape
        self.SIM_SCRAPE_TEXT = {}
        self.SIM_SCRAPE_NODES = []
        # deferred-park
        self.SIM_PARK_VAL = {}; self.SIM_PARK_ACC = {}
        # dkg finalize-deferred watch (SOFT, per-node last value)
        self.SIM_DKG_DEFERRED_LAST = {}
        # witness watch
        self.SIM_WIT_WATCH = {}
        self.SIM_WIT_EMB_BASE = -1; self.SIM_WIT_EMB_FIN = -1; self.SIM_WIT_EMB_SET = ""
        # exec-saturation
        self.SIM_EXEC_SUM = {}; self.SIM_EXEC_CNT = {}
        self.SIM_EXEC_SAT_ACC = 0; self.SIM_EXEC_EARLY_FIRED = 0; self.SIM_EXEC_LAST_OCC = ""
        # block-rate
        self.SIM_RATE_LAST_FIN = -1; self.SIM_RATE_LAST_TS = 0
        self.SIM_RATE_TICKS = 0; self.SIM_RATE_SLOW_ACC = 0; self.SIM_RATE_LAST = ""
        # spec-head
        self.SIM_SPECGAP_TICKS = 0; self.SIM_SPECGAP_FIN0 = -1; self.SIM_SPECGAP_TS0 = 0
        # data-root / host-mem episode latches
        self.SIM_DATA_FILL_WARNED = 0
        self.SIM_MEM_LOW_WARNED = 0
        # panic scan
        self.SIM_PANIC_LAST_SCAN = 0
        self.SIM_PANIC_CONTAINERS_AT = -1
        self.SIM_PANIC_CONTAINERS = []
        # per-tick tier + node-fin cache
        self.SIM_BW_L2_FIN = -1; self.SIM_BW_L3_FIN = -1
        self.SIM_NODE_FIN = {}
        # detector-starvation belt
        self.SIM_DET_SAMPLES = {}; self.SIM_DET_STARVED = {}; self.SIM_DET_ANNOUNCED = {}
        # battery coverage
        self.SIM_COVERAGE_LOW_TICKS = 0
        # peer plane
        self.SIM_PEER_LAST_SPOKE = ""; self.SIM_PEER_ZERO = 0
        # dkg iff / deferral streak
        self.SIM_DKG_IFF_LAST_EPOCH = -1
        self.SIM_DKG_DEFER_STREAK = 0; self.SIM_DKG_DEFER_TARGET = ""
        # finalized-chain ledger
        self.SIM_FIN_LEDGER = {}; self.SIM_FIN_CURSOR = {}
        # member liveness
        self.SIM_MEMBER_FIN = {}; self.SIM_MEMBER_FLAT = {}; self.SIM_MEMBER_ARMED = {}
        # wrongful slash
        self.SIM_WRONGSLASH_ACC = {}
        # committee membership
        self.SIM_MEMBERSHIP_SIZE_ACC = 0
        self.SIM_MEMBERSHIP_VAC_EPOCH = -1
        self.SIM_MEMBERSHIP_STRANGER_ACC = {}
        self.BACKFILL_PAUSE_CREDIT = {}; self.BACKFILL_PAUSE_LASTE = {}
        # participation scrape
        # fair credit

    # ── event + fail helpers ────────────────────────────────────────────────
    def sim_event(self, kind, note):
        """Capture stub (matches the gate-test sim_event): record (kind, note).
        The production writer (events.jsonl) is owned by the shadow runner."""
        self.events.append((kind, note))

    def _inv_fail(self, fail_id, msg, entity=None):
        # Liveness-first policy: a DEMOTED bookkeeping detector records its state as a diagnostic
        # (for future bug-finding) and reports as HOLDING so check_invariants continues — only
        # liveness + genuine safety witnesses fail the run. See core.policy.DEMOTED_INVARIANTS and
        # memory sim-liveness-is-only-invariant-gate-actions-on-f.
        #
        # LATCH KEY = (fail_id, entity), the same shape the write side uses (chain.writes.fatal_or_diag).
        # Keyed on fail_id ALONE (as it was until 2026-07-30) a demoted detector emitted exactly ONE
        # line per run — the FIRST offender's message, with that offender's threshold counter frozen
        # into it — so a second stuck seat/member/address was invisible and the counts were a per-run
        # boolean, not a frequency. Callers with a natural entity (the seated address, the node) pass
        # it; callers whose condition is about the committee as a whole pass nothing rather than
        # inventing a key, and keep the old one-line-per-run behaviour, which is correct for them.
        from ..core.policy import DEMOTED_INVARIANTS
        if fail_id in DEMOTED_INVARIANTS:
            latch = (fail_id, entity)
            if latch not in self._diag_seen:        # latch: one diag per (invariant, entity) per run
                self._diag_seen.add(latch)
                self.sim_event("diag", f"[{fail_id}] {msg}")
            return True
        self.inv_fail_id = fail_id
        self.inv_fail_msg = msg
        return False

    # ── impure seams (overridable; default to core.nodes) ────────────────
    def _now(self):
        return int(time.time())

    def _fork_sleep(self):
        time.sleep(self.SIM_FORK_RECHECK_DELAY)

    def _finalized_dec(self):
        return nodes.finalized_dec()

    def _eth_latest(self):
        return nodes.sim_eth_latest()

    def _consensus_latest(self):
        return nodes.sim_consensus_latest()

    def _node_metrics(self, svc):
        return nodes.node_metrics(svc)

    def _fin_at(self, port):
        return nodes.sim_fin_at(port)

    def _node_fin_in(self, svc):
        return nodes.node_fin_in(svc)

    def _node_roots_of(self, svc, h):
        return nodes.node_roots_of(svc, h)

    def _mixhash_of(self, svc, b):
        return nodes.mixhash_of(svc, b)

    def _blockhash_of(self, svc, b):
        return nodes.blockhash_of(svc, b)

    def _peer_count(self, svc):
        return nodes.peer_count(svc)

    def _v0_running(self):
        svcs = nodes.running_services()
        return topology.PINNED_RPC_HOST in svcs

    def _panic_containers(self):
        return nodes.running_services()

    def _panic_grep(self, container, since):
        out = nodes.logs_since(container, since)
        for line in out.splitlines():
            if "panicked at" in line:
                return line
        return ""

    def _down_containers(self):
        return nodes.down_services()

    def _down_logtail(self, container):
        out = nodes.rpc._run_read(
            nodes.rpc.DOCKER_COMPOSE + ["logs", "--no-color", "--tail", "1", container],
            nodes.rpc.RPC_EXEC_TIMEOUT,
        )
        lines = out.splitlines()
        return lines[-1] if lines else ""

    def _data_root_pcent(self):
        root = self.ctx.SIM_DATA_ROOT
        out = nodes.rpc._run_read(["df", "--output=pcent", root], 10)
        digits = re.sub(r"[^0-9]", "", out.splitlines()[-1]) if out.splitlines() else ""
        return digits

    def _data_root_free(self):
        root = self.ctx.SIM_DATA_ROOT
        out = nodes.rpc._run_read(["df", "-h", "--output=avail", root], 10)
        lines = out.splitlines()
        return lines[-1].strip() if lines else ""

    def _mem_avail_kb(self):
        try:
            with open("/proc/meminfo") as f:
                for line in f:
                    if line.startswith("MemAvailable:"):
                        return line.split()[1]
        except Exception:
            pass
        return ""

    def _spec_derive_paths(self, secs):
        """Finalized-derive PATH evidence over the last <secs>s -> (reuse, rederive) from the
        dpos::derive_seed 'finalized derive path' telemetry, AGGREGATED over the live SEATED committee
        (v61 a26). It used to read ONLY validator-0 — but a25's eviction can rekey/restart v0 (it is
        the emergent-victim path's target too), and a just-restarted node rederives 100% while it
        catches up (no spec state to reuse yet). Sampling that ONE node then false-fired 'nothing is
        being speculated' while the rest of the committee speculated fine (v4: 1480 reuse / 532
        rederive at the same instant). Summing across all healthy seated members lets the speculating
        majority dominate, so a single catching-up node no longer trips the invariant. (_committee_
        containers already drops SIM_DISRUPTED ∪ PERMANENTLY_DEAD; falls back to v0 if it is empty.)"""
        svcs = self._committee_containers() or [topology.PINNED_RPC_HOST]
        reuse = rederive = 0
        for svc in svcs:
            out = nodes.logs_since(svc, f"{secs}s")
            for line in out.splitlines():
                if "derive-seed: finalized derive path" not in line:
                    continue
                if "finalized-spec-reuse" in line:
                    reuse += 1
                if "finalized-rederive" in line:
                    rederive += 1
        return reuse, rederive

    # cast reads (runtime-deploy staking) — overridable
    def _pp_dkg_qual(self, epoch):
        return nodes.pp_dkg_qual(epoch, self.ctx.STAKING_RT)

    def _pp_committee(self, epoch):
        return nodes.pp_committee(epoch, self.ctx.STAKING_RT)

    def _pp_validator_status(self, addr):
        return nodes.pp_validator_status(addr, self.ctx.STAKING_RT)

    def _liveness_call(self, sig, *args):
        return nodes.liveness_call(sig, *args, addr=self.ctx.LIVENESS_RT)

    def _staking_call(self, sig, *args):
        return nodes.staking_call(sig, *args, addr=self.ctx.STAKING_RT)

    def _chainconfig_call(self, sig, *args):
        return nodes.chainconfig_call(sig, *args, addr=self.ctx.CHAIN_CONFIG_RT)

    def _production(self, epoch, addr):
        return nodes.production(epoch, addr, staking_rt=self.ctx.STAKING_RT,
                                   liveness_rt=self.ctx.LIVENESS_RT)

    def _seed_round_line(self, svc, height, since="180s"):
        """Last dpos::derive_seed 'block derived' line for <height> in <svc>'s recent
        slice (ANSI-stripped). "" if none."""
        out = nodes.logs_since(svc, since)
        target = f"height={height} "
        last = ""
        for line in out.splitlines():
            if "derive-seed: block derived" in line and target in line:
                last = line
        return last

    # ── excluded / helper predicates ────────────────────────────────────────
    def _excluded(self, container):
        c = self.ctx
        return _in_set(container, c.SIM_DISRUPTED) or (container in c.PERMANENTLY_DEAD and c.PERMANENTLY_DEAD[container])

    def _committee_containers(self):
        """Live committee containers (SIM_CUR_COMMITTEE -> ADDR2IDX), minus
        SIM_DISRUPTED ∪ PERMANENTLY_DEAD."""
        c = self.ctx
        out = []
        for a in (c.SIM_CUR_COMMITTEE or "").split():
            svc = c.ADDR2IDX.get(a.lower())
            if not svc:
                continue
            if self._excluded(svc):
                continue
            out.append(svc)
        return out

    # ── metric-parse helpers (delegated to nodes) ───────────────────────────
    def _metric_val(self, text, family, label):
        return nodes.metric_val(text, family, label)

    def _gauge_val(self, text, suffix):
        return nodes.gauge_val(text, suffix)

    def _summary_agg(self, text, family):
        return nodes.summary_agg(text, family)

    # ── the per-tick scrape build ───────────────────────────────────────────
    def _scrape_metrics(self, v0_text):
        c = self.ctx
        self.SIM_SCRAPE_TEXT = {topology.PINNED_RPC_HOST: v0_text}
        self.SIM_SCRAPE_NODES = [topology.PINNED_RPC_HOST]
        if c.SIM_TICK % self.SIM_HALT_SCRAPE_EVERY == 0:
            for sa in (c.SIM_CUR_COMMITTEE or "").split():
                ss = c.ADDR2IDX.get(sa.lower())
                if not ss or topology.is_pinned_rpc_host(ss):
                    continue
                if self._excluded(ss):
                    continue
                self.SIM_SCRAPE_TEXT[ss] = self._node_metrics(ss)
                self.SIM_SCRAPE_NODES.append(ss)

    def _prime_node_fin(self, fin):
        c = self.ctx
        self.SIM_NODE_FIN = {}
        for fa in (c.SIM_CUR_COMMITTEE or "").split():
            fs = c.ADDR2IDX.get(fa.lower())
            if not fs or topology.is_pinned_rpc_host(fs):
                continue
            if self._excluded(fs):
                continue
            if fs in self.SIM_NODE_FIN:
                continue
            self.SIM_NODE_FIN[fs] = self._node_fin_in(fs)

    def _node_fin_cached(self, svc):
        if svc in self.SIM_NODE_FIN:
            return self.SIM_NODE_FIN[svc]
        return self._node_fin_in(svc)

    # ── detector-starvation belt ────────────────────────────────────────────
    def _detector_sampled(self, det, chan, n, floor):
        """Publish this tick's sample count and assert on sustained starvation.
        Call on EVERY evaluated tick (both sufficient and insufficient), never on
        a legitimately-skipped tick. Returns True (holds) / False (belt tripped)."""
        self.SIM_DET_SAMPLES[det] = n
        if n >= floor:
            self.SIM_DET_STARVED[det] = 0
            self.SIM_DET_ANNOUNCED[det] = 0
            return True
        run = self.SIM_DET_STARVED.get(det, 0) + 1
        self.SIM_DET_STARVED[det] = run
        if run >= self.SIM_DET_STARVE_TICKS:
            return self._inv_fail(
                "detector-starved",
                f"{det} compared only {n} node(s) (< floor {floor}) for {run} "
                f"consecutive evaluated ticks (>= {self.SIM_DET_STARVE_TICKS}) over the "
                f"{chan} channel — the detector has been returning GREEN while sampling "
                "too few nodes to prove ANYTHING. This is a battery failure, not chain "
                "health: the read channel to the committee is down (container-internal "
                "RPC brownout), or the nodes are lagging validator-0 past the probe "
                f"ceiling. SIM_BATTERY_COVERAGE does NOT cover this — it measures the "
                f"Prometheus METRICS channel, not {chan}")
        if self.SIM_DET_ANNOUNCED.get(det, 0) == 0:
            self.SIM_DET_ANNOUNCED[det] = 1
            self.sim_event(
                "detector_undersampled",
                f"{det} compared only {n} node(s) (< floor {floor}) over the {chan} "
                "channel — the detector cannot assert this tick and its silence is NOT "
                f"evidence of health (reported now, hard detector-starved fail after "
                f"{self.SIM_DET_STARVE_TICKS} consecutive evaluated ticks)")
        return True

    # ── host-side watchdogs ─────────────────────────────────────────────────
    def _inv_data_root_fill(self):
        c = self.ctx
        if not c.SIM_DATA_ROOT:
            return True
        pct = self._data_root_pcent()
        if not (pct and pct.isdigit()):
            return True
        pct = int(pct)
        if pct >= self.SIM_DATA_FILL_FAIL_PCT:
            free = self._data_root_free()
            return self._inv_fail(
                "data-root-full",
                f"data root {c.SIM_DATA_ROOT} tmpfs/disk at {pct}% (>= "
                f"{self.SIM_DATA_FILL_FAIL_PCT}%, {free or '?'} free) — NOT a product "
                "failure: the run outgrew its RAM/disk root before any node died of "
                "ENOSPC; shorten the run or use a disk-backed SIM_DATA_ROOT")
        if pct >= self.SIM_DATA_FILL_WARN_PCT:
            if self.SIM_DATA_FILL_WARNED == 0:
                self.SIM_DATA_FILL_WARNED = 1
                free = self._data_root_free()
                self.sim_event(
                    "data_root_filling",
                    f"data root {c.SIM_DATA_ROOT} at {pct}% (warn >= "
                    f"{self.SIM_DATA_FILL_WARN_PCT}%, {free or '?'} free) — reported, not "
                    f"asserted; hard data-root-full fail at {self.SIM_DATA_FILL_FAIL_PCT}%")
        else:
            self.SIM_DATA_FILL_WARNED = 0
        return True

    def _inv_host_mem(self):
        kb = self._mem_avail_kb()
        if not (kb and str(kb).isdigit()):
            return True
        kb = int(kb)
        warn_kb = self.SIM_MEM_AVAIL_WARN_GB * 1048576
        fail_kb = self.SIM_MEM_AVAIL_FAIL_GB * 1048576
        tenths = kb * 10 // 1048576
        avail_gb = f"{tenths // 10}.{tenths % 10}"
        if kb <= fail_kb:
            return self._inv_fail(
                "host-mem-exhausted",
                f"host MemAvailable {avail_gb} GiB (<= {self.SIM_MEM_AVAIL_FAIL_GB} GiB) "
                "— host memory exhaustion: RAM data root + container RSS exceeding "
                "physical RAM. NOT a product failure: the RAM-backed sim outgrew the "
                "host; shorten the run, raise physical RAM, or use a disk-backed "
                "SIM_DATA_ROOT")
        if kb <= warn_kb:
            if self.SIM_MEM_LOW_WARNED == 0:
                self.SIM_MEM_LOW_WARNED = 1
                self.sim_event(
                    "host_mem_low",
                    f"host MemAvailable {avail_gb} GiB (warn <= {self.SIM_MEM_AVAIL_WARN_GB} "
                    f"GiB) — reported, not asserted; hard host-mem-exhausted fail at "
                    f"{self.SIM_MEM_AVAIL_FAIL_GB} GiB (RAM data root + container RSS "
                    "approaching physical RAM)")
        else:
            self.SIM_MEM_LOW_WARNED = 0
        return True

    # ── read-health gate (F5) ───────────────────────────────────────────────
    def _inv_rpc_alive(self, fin):
        c = self.ctx
        if fin > 0:
            self.SIM_RPC_GOOD_FIN = fin
            self.SIM_RPC_GOOD_ROUND = c.SIM_ROUND
            self.SIM_RPC_ZERO_TICKS = 0
            return True
        if self.SIM_RPC_GOOD_FIN == 0:
            return True  # bring-up: fin never positive yet
        self.SIM_RPC_ZERO_TICKS += 1
        belt = self.SIM_RPC_DEAD_BELT
        if self._v0_running():
            if self.SIM_RPC_ZERO_TICKS >= 2 * belt:
                return self._inv_fail(
                    "rpc-dead",
                    f"primary RPC {nodes.RPC} ({topology.PINNED_RPC_HOST}) reads finalized=0 "
                    f"for {self.SIM_RPC_ZERO_TICKS} consecutive ticks "
                    f"(>= 2x belt {2 * belt}) "
                    f"after last-good fin={self.SIM_RPC_GOOD_FIN} (round "
                    f"{self.SIM_RPC_GOOD_ROUND}) — the v0 CONTAINER is running but its RPC "
                    "has been dark far past the ~40 s brownout window; the node process is "
                    "wedged, not merely recreating (bundle-20260715T214344Z)")
            self.sim_event(
                "rpc_brownout",
                f"primary RPC reads finalized=0 for {self.SIM_RPC_ZERO_TICKS} tick(s) but "
                f"the {topology.PINNED_RPC_HOST} CONTAINER is RUNNING — treating as a "
                "host-RPC brownout "
                f"(self-induced by --force-recreate churn, ~40 s FLK-5); waiting (hard "
                f"rpc-dead only at 2x belt = {2 * belt} ticks). Last-good "
                f"fin={self.SIM_RPC_GOOD_FIN} (round {self.SIM_RPC_GOOD_ROUND})")
            return True
        if self.SIM_RPC_ZERO_TICKS >= belt:
            return self._inv_fail(
                "rpc-dead",
                f"primary RPC {nodes.RPC} ({topology.PINNED_RPC_HOST}) reads finalized=0 for "
                f"{self.SIM_RPC_ZERO_TICKS} consecutive ticks (>= belt {belt}) after "
                f"last-good fin={self.SIM_RPC_GOOD_FIN} (round {self.SIM_RPC_GOOD_ROUND}) "
                "AND the v0 container is NOT running — the RPC-target node is "
                "dead/unreachable; refusing to run the battery on zeroed reads "
                "(bundle-20260715T214344Z)")
        return True

    # ── node panic / down ───────────────────────────────────────────────────
    def _inv_node_panic(self):
        c = self.ctx
        now = self._now()
        if self.SIM_PANIC_LAST_SCAN == 0:
            since = f"{self.SIM_PANIC_FIRST_WINDOW}s"
        else:
            since = f"{now - self.SIM_PANIC_LAST_SCAN + 2}s"
        if (self.SIM_PANIC_CONTAINERS_AT < 0
                or c.SIM_TICK - self.SIM_PANIC_CONTAINERS_AT >= self.SIM_PANIC_CONTAINERS_EVERY):
            self.SIM_PANIC_CONTAINERS = self._panic_containers()
            self.SIM_PANIC_CONTAINERS_AT = c.SIM_TICK
        self.SIM_PANIC_LAST_SCAN = now
        for container in self.SIM_PANIC_CONTAINERS:
            if not container:
                continue
            line = self._panic_grep(container, since)
            v = node_panic_verdict(container, line)
            if v:
                return self._inv_fail(*v)
        return True

    def _inv_safety_halt(self):
        for hn in self.SIM_SCRAPE_NODES:
            htext = self.SIM_SCRAPE_TEXT.get(hn, "")
            if not htext:
                continue
            for hr in ("result_divergence", "l1_fork", "el_invalid"):
                hv = self._metric_val(htext, "dpos_sync_degraded", f'reason="{hr}"')
                if hv and _num(hv) >= 1:
                    return self._inv_fail(
                        "safety-halt",
                        f"{hn} raised SafetyHalt dpos_sync_degraded{{reason={hr}}}={hv} — "
                        "the node PARKED refusing the chain (fork-safety latch, SAFETY). A "
                        "correct-minority halt this battery used to run GREEN on.")
        return True

    # ── witness counters (§9 hard zeros) ────────────────────────────────────
    def _inv_witness_counters(self):
        for wn in self.SIM_SCRAPE_NODES:
            wt = self.SIM_SCRAPE_TEXT.get(wn, "")
            if not wt:
                continue
            for wr in ("pre_bootstrap", "missing", "pin", "bad_signature"):
                wv = self._metric_val(wt, "dpos_parent_seed_reject_total", f'reason="{wr}"')
                if _isint(wv) and int(wv) >= 1:
                    return self._inv_fail(
                        "witness-reject",
                        f"{wn} voted false on a parent-seed witness: "
                        f"dpos_parent_seed_reject_total{{reason={wr}}}={wv} — MUST be 0 on an "
                        "honest run (a non-zero value splits honest voters / signals a "
                        "downgrade attempt; plan §3)")
            wv = self._metric_val(wt, "dpos_parent_view_mismatch_total", "")
            if _isint(wv) and int(wv) >= 1:
                return self._inv_fail(
                    "witness-view-mismatch",
                    f"{wn} saw dpos_parent_view_mismatch_total={wv} — a block was certified "
                    "at a view it did not self-attest (P4 tripwire; rule SA/PIN un-backed)")
            wv = self._metric_val(wt, "dpos_group_key_invariant_violation_total", "")
            if _isint(wv) and int(wv) >= 1:
                return self._inv_fail(
                    "group-key-invariant",
                    f"{wn} saw dpos_group_key_invariant_violation_total={wv} — a SAME-epoch "
                    "PK_E miss on a voting member (W1 violated)")
            wv = self._metric_val(wt, "dpos_spec_round_mismatch_total", "")
            if _isint(wv) and int(wv) >= 1:
                return self._inv_fail(
                    "spec-round-mismatch",
                    f"{wn} saw dpos_spec_round_mismatch_total={wv} — speculative seed round "
                    "!= finalized witness round (P2 regression: boundary sibling-reorg class)")
        return True

    # ── dkg pinned-idx out of range (2026-07-21 idx-stall class, HARD) ──────
    def _inv_dkg_pinned_idx(self):
        """dpos_dkg_pinned_idx_out_of_range_total MUST be 0. Non-zero means a consensus-PINNED
        dealer-log index named a position outside the committed committee and the ceremony skipped
        it — the 2026-07-21 dkg_logs idx-stall class resurfacing (a selection view that moved under
        an epoch that had already started). The counter carries no labels.

        Registered on the COMMONWARE registry (:9100, host-mapped :19100 on the pinned RPC host),
        the same one that carries beacon_seed_active_total / dkg_ceremony_ok_total — so it rides
        the shared per-tick SIM_SCRAPE_TEXT scrape (node_metrics concatenates :9100 and reth's
        :9200) and costs no extra read.

        It is a per-node counter, so it is asserted per node and one readable node is enough to
        assert: floor 1 through _detector_sampled. A tick where NO scraped node exposes the family
        is reported as undersampled and hard-fails after the starvation belt — silence from an
        unreadable detector is not evidence of health, which is the exact failure mode this
        detector exists to remove."""
        readable = 0
        for n in self.SIM_SCRAPE_NODES:
            t = self.SIM_SCRAPE_TEXT.get(n, "")
            if not t:
                continue
            v = self._metric_val(t, "dpos_dkg_pinned_idx_out_of_range_total", "")
            if not _isint(v):
                continue
            readable += 1
            if int(v) >= 1:
                return self._inv_fail(
                    "dkg-pinned-idx-out-of-range",
                    f"{n} saw dpos_dkg_pinned_idx_out_of_range_total={v} — a consensus-pinned "
                    "dealer-log index named a position OUTSIDE the committed committee and the "
                    "ceremony skipped it. MUST be 0: this is the 2026-07-21 dkg_logs idx-stall "
                    "class (the epoch-frozen selection view moved after its epoch started), which "
                    "wedges DKG finalize chain-wide. Grep the node for target dpos::beacon "
                    '"names indices outside the committed committee"')
        return self._detector_sampled(
            "dkg-pinned-idx", "commonware :9100 dpos_dkg_pinned_idx_out_of_range_total",
            readable, 1)

    # ── beacon active (COV-2) ───────────────────────────────────────────────
    def _inv_beacon_active(self, fin, v0m):
        c = self.ctx
        settle_ok = settle_gate_open(c.SIM_CUR_EPOCH, c.SETTLE_UNTIL_EPOCH, self.SIM_SETTLE_GATE_CAP)
        if (c.EXPECTED_STALL == 0 and fin > 0 and c.SIM_CUR_EPOCH >= 3 and settle_ok) and v0m:
            sa = self._metric_val(v0m, "beacon_seed_active", "")
            fb = self._metric_val(v0m, "beacon_digest_fallback", "")
            if sa and fb:
                sa_i, fb_i = _num(sa), _num(fb)
                if self.SIM_BEACON_SA_BASE < 0:
                    self.SIM_BEACON_SA_BASE = sa_i; self.SIM_BEACON_FB_BASE = fb_i
                    self.SIM_BEACON_FIN_BASE = fin; self.SIM_BEACON_WIN_TICKS = 0
                elif (counter_progress(sa_i, self.SIM_BEACON_SA_BASE) == "restart"
                      or counter_progress(fb_i, self.SIM_BEACON_FB_BASE) == "restart"):
                    # RESTART, not a stall — re-baseline both counters + the window.
                    self.SIM_BEACON_SA_BASE = sa_i; self.SIM_BEACON_FB_BASE = fb_i
                    self.SIM_BEACON_FIN_BASE = fin; self.SIM_BEACON_WIN_TICKS = 0
                else:
                    self.SIM_BEACON_WIN_TICKS += 1
                    if fb_i > self.SIM_BEACON_FB_BASE:
                        return self._inv_fail(
                            "beacon-digest-fallback",
                            f"beacon_digest_fallback_total grew {self.SIM_BEACON_FB_BASE} -> "
                            f"{fb_i} on v0 while finalizing — a block fell back to "
                            "order.digest() (beacon silently degraded)")
                    if self.SIM_BEACON_WIN_TICKS >= self.SIM_BEACON_CHECK_TICKS:
                        if (fin > self.SIM_BEACON_FIN_BASE + self.SIM_BEACON_WINDOW
                                and sa_i <= self.SIM_BEACON_SA_BASE):
                            return self._inv_fail(
                                "beacon-inactive",
                                f"beacon_seed_active_total FLAT ({self.SIM_BEACON_SA_BASE}) on "
                                f"v0 over {self.SIM_BEACON_WIN_TICKS} ticks while finalized "
                                f"advanced {self.SIM_BEACON_FIN_BASE} -> {fin} — threshold "
                                "beacon silently fell to digest fallback")
                        self.SIM_BEACON_SA_BASE = sa_i; self.SIM_BEACON_FB_BASE = fb_i
                        self.SIM_BEACON_FIN_BASE = fin; self.SIM_BEACON_WIN_TICKS = 0
        else:
            self.SIM_BEACON_SA_BASE = -1; self.SIM_BEACON_FB_BASE = -1
            self.SIM_BEACON_FIN_BASE = -1; self.SIM_BEACON_WIN_TICKS = 0
        return True

    # ── finalize stall (Inv-1) ──────────────────────────────────────────────
    def _inv_finalize_stall(self, fin):
        c = self.ctx
        if c.EXPECTED_STALL == 0:
            if fin <= 0:
                # FLK-5: an unreachable/blipped v0 RPC coerces to 0; don't poison the
                # baseline — reset the belt and skip (mirrors Inv-2's fin==0 reset).
                self.SIM_FLAT_TICKS = 0
            elif self.SIM_LAST_FIN >= 0 and fin <= self.SIM_LAST_FIN:
                self.SIM_FLAT_TICKS += 1
                if self.SIM_FLAT_TICKS >= self.SIM_STALL_TICKS:
                    return self._inv_fail(
                        "finalize-stall",
                        f"finalized FLAT at {fin} for {self.SIM_FLAT_TICKS} ticks (>= "
                        f"{self.SIM_STALL_TICKS}) — chain stalled")
            else:
                self.SIM_FLAT_TICKS = 0
        else:
            self.SIM_FLAT_TICKS = 0
        if fin > 0:
            self.SIM_LAST_FIN = fin  # advance baseline only on a REAL read
        return True

    # ── k-lag exactness (Inv-2/2b) ──────────────────────────────────────────
    def _inv_klag(self, fin, latest, lat, lrf):
        K = self.RESULT_LAG_K
        if fin > 0 and latest >= fin:
            gap = latest - fin
            if gap < K:
                return self._inv_fail(
                    "k-lag-underflow",
                    f"eth latest-finalized={gap} < K={K} (result-final overclaim, SAFETY) "
                    f"latest={latest} fin={fin}")
            if lat > 0 and lrf > 0:
                cgap = lat - lrf
                if cgap < K:
                    return self._inv_fail(
                        "k-lag-consensus-overclaim",
                        f"consensus two-tier overclaim/inversion: latestFinalized={lat} "
                        f"latestResultFinalized={lrf} cgap={cgap} < K={K} (result finalized "
                        "ahead of inclusion — SAFETY)")
                if cgap > K + 1:
                    self.SIM_CGAP_OVER_TICKS += 1
                    if self.SIM_CGAP_OVER_TICKS >= self.SIM_STALL_TICKS:
                        # F2: gate the liveness belt on the exec-occupancy regime signal.
                        if (self.SIM_EXEC_LAST_OCC
                                and float(self.SIM_EXEC_LAST_OCC) > self.SIM_EXEC_SAT_THRESHOLD):
                            self.sim_event(
                                "klag_consensus_degraded_rate",
                                f"consensus latestFinalized-latestResultFinalized={cgap} > "
                                f"K+1={K + 1} for {self.SIM_CGAP_OVER_TICKS} ticks but EL exec "
                                f"occupancy {self.SIM_EXEC_LAST_OCC} > "
                                f"{self.SIM_EXEC_SAT_THRESHOLD} (execution-saturated regime — "
                                "deferred execution is not back-pressured; degraded rate OK) — "
                                f"reported, not asserted (lat={lat} lrf={lrf})")
                            self.SIM_CGAP_OVER_TICKS = 0
                        else:
                            return self._inv_fail(
                                "k-lag-consensus-stall",
                                f"consensus latestFinalized-latestResultFinalized={cgap} > "
                                f"K+1={K + 1} for {self.SIM_CGAP_OVER_TICKS} ticks (>= "
                                f"{self.SIM_STALL_TICKS}) — result tier stalled behind "
                                "inclusion beyond the eager-derive in-flight transient, EL NOT "
                                f"execution-saturated (occupancy="
                                f"{self.SIM_EXEC_LAST_OCC or 'unmeasured'}) so the backlog is "
                                f"not load-explained (lat={lat} lrf={lrf})")
                else:
                    self.SIM_CGAP_OVER_TICKS = 0
            self.SIM_KLAG_CATCHUP_TICKS = 0
        elif fin > 0 and latest > 0 and latest < fin:
            self.SIM_KLAG_CATCHUP_TICKS += 1
            if self.SIM_KLAG_CATCHUP_TICKS >= self.SIM_STALL_TICKS:
                return self._inv_fail(
                    "k-lag-catchup-overclaim",
                    f"eth latest={latest} < finalized={fin} for {self.SIM_KLAG_CATCHUP_TICKS} "
                    f"ticks (>= {self.SIM_STALL_TICKS}) — finalized ahead of execution head "
                    "(result-final overclaim, SAFETY)")
        else:
            self.SIM_KLAG_CATCHUP_TICKS = 0
        return True

    # ── deferred park (§9 item 2) ───────────────────────────────────────────
    def _inv_deferred_park(self):
        c = self.ctx
        if c.EXPECTED_STALL == 1:
            self.SIM_PARK_VAL = {}; self.SIM_PARK_ACC = {}
            return True
        for pn in self.SIM_SCRAPE_NODES:
            pt = self.SIM_SCRAPE_TEXT.get(pn, "")
            if not pt:
                continue
            pv = self._gauge_val(pt, "deferred_height")
            if not _isint(pv):
                continue
            pv_i = int(pv)
            if pv_i == 0:
                self.SIM_PARK_VAL.pop(pn, None)
                self.SIM_PARK_ACC.pop(pn, None)
            elif str(pv_i) == str(self.SIM_PARK_VAL.get(pn, "")):
                pw = 1 if topology.is_pinned_rpc_host(pn) else self.SIM_HALT_SCRAPE_EVERY
                self.SIM_PARK_ACC[pn] = self.SIM_PARK_ACC.get(pn, 0) + pw
                if self.SIM_PARK_ACC[pn] >= self.SIM_PARK_TICKS:
                    return self._inv_fail(
                        "deferred-park",
                        f"{pn} executor PARKED: deferred_height flat at {pv_i} for >= "
                        f"{self.SIM_PARK_TICKS} ticks — the soak7 signature (under B′ the tip "
                        "block is HELD, never parked; a durable park means the witness pipeline "
                        "shift regressed or the node is truly isolated)")
            else:
                self.SIM_PARK_VAL[pn] = str(pv_i); self.SIM_PARK_ACC[pn] = 0
        return True

    # ── witness embedded (§9 item 1) ────────────────────────────────────────
    def _inv_witness_embedded(self, fin):
        c = self.ctx
        if not len(self.SIM_SCRAPE_NODES) > 1:
            return True
        settle_ok = settle_gate_open(c.SIM_CUR_EPOCH, c.SETTLE_UNTIL_EPOCH, self.SIM_SETTLE_GATE_CAP)
        if c.EXPECTED_STALL == 0 and fin > 0 and c.SIM_CUR_EPOCH >= 3 and settle_ok:
            total = 0
            sig = " ".join(self.SIM_SCRAPE_NODES)
            for en in self.SIM_SCRAPE_NODES:
                ev = self._metric_val(self.SIM_SCRAPE_TEXT.get(en, ""),
                                      "dpos_parent_seed_embedded_total", "")
                if _isint(ev):
                    total += int(ev)
            if (self.SIM_WIT_EMB_BASE < 0 or sig != self.SIM_WIT_EMB_SET
                    or total < self.SIM_WIT_EMB_BASE):
                self.SIM_WIT_EMB_BASE = total; self.SIM_WIT_EMB_FIN = fin; self.SIM_WIT_EMB_SET = sig
            elif fin > self.SIM_WIT_EMB_FIN + self.SIM_BEACON_WINDOW:
                if total <= self.SIM_WIT_EMB_BASE:
                    return self._inv_fail(
                        "witness-not-embedded",
                        f"dpos_parent_seed_embedded_total FLAT chain-wide "
                        f"({self.SIM_WIT_EMB_BASE}) while finalized advanced "
                        f"{self.SIM_WIT_EMB_FIN} -> {fin} — blocks are finalizing WITHOUT "
                        "parent-seed witnesses (propose-side embed dead; plan §3)")
                self.SIM_WIT_EMB_BASE = total; self.SIM_WIT_EMB_FIN = fin; self.SIM_WIT_EMB_SET = sig
        else:
            self.SIM_WIT_EMB_BASE = -1; self.SIM_WIT_EMB_FIN = -1; self.SIM_WIT_EMB_SET = ""
        return True

    # ── witness watch (SOFT) ────────────────────────────────────────────────
    def _inv_witness_watch(self):
        if not len(self.SIM_SCRAPE_NODES) > 1:
            return True
        specs = [
            ("dpos_parent_seed_boundary_skip_total", ""),
            ("dpos_parent_seed_boundary_unverified_total", 'reason="unknown"'),
            ("dpos_parent_seed_boundary_unverified_total", 'reason="read_failed"'),
            ("dpos_parent_seed_lookup_miss_total", ""),
            ("dpos_spec_seed_recanonicalized_total", ""),
            ("dpos_witness_child_refetch_total", ""),
            ("dpos_cert_inlet_carry_forward_pin_verify_failed_total", ""),
        ]
        for fam, lbl in specs:
            total = 0
            for n in self.SIM_SCRAPE_NODES:
                v = self._metric_val(self.SIM_SCRAPE_TEXT.get(n, ""), fam, lbl)
                if _isint(v):
                    total += int(v)
            key = f"{fam}|{lbl}"
            prev = self.SIM_WIT_WATCH.get(key, 0)
            if total > prev:
                disp = fam if not lbl else f"{fam}{{{lbl}}}"
                self.sim_event(
                    "warn",
                    f"witness-watch: {disp} rose {prev} -> {total} chain-wide (reported, not "
                    "asserted — see _inv_witness_watch for what a sustained rise means)")
            self.SIM_WIT_WATCH[key] = total
        return True

    # ── dkg finalize deferred (SOFT) ────────────────────────────────────────
    def _inv_dkg_finalize_deferred(self):
        """dpos_dkg_finalize_deferred_total — a ceremony reached its settle deadline without
        qualifying and deferred to the epoch boundary (counted once per epoch per reason). Same
        commonware registry (:9100) as the pinned-idx counter, no labels; the reason
        (missing_body / below_quorum) rides the log line, not the metric.

        REPORT ONLY. A rise means a DKG was LATE, which used to be invisible — a signal to read
        the logs, not a verdict. There is no baseline for how often a healthy chain defers, so
        asserting on it would only manufacture flake; revisit once sims have produced one.

        Tracked PER NODE, not as a chain-wide total: the scrape set changes size between ticks
        (SIM_HALT_SCRAPE_EVERY only samples the committee periodically), so a summed total would
        report a 'rise' every time more nodes were scraped. A node restart resets its counter
        below the last value and is silently re-baselined."""
        for n in self.SIM_SCRAPE_NODES:
            t = self.SIM_SCRAPE_TEXT.get(n, "")
            if not t:
                continue
            v = self._metric_val(t, "dpos_dkg_finalize_deferred_total", "")
            if not _isint(v):
                continue
            cur = int(v)
            prev = self.SIM_DKG_DEFERRED_LAST.get(n)
            self.SIM_DKG_DEFERRED_LAST[n] = cur
            if prev is None or cur <= prev:
                continue
            self.sim_event(
                "warn",
                f"dkg-finalize-deferred: {n} dpos_dkg_finalize_deferred_total rose {prev} -> "
                f"{cur} — a DKG ceremony missed its settle deadline and deferred to the epoch "
                "boundary (reported, not asserted — no baseline yet). Read that node's "
                'target dpos::beacon WARN "DKG finalize deferred past the settle deadline" for '
                "the reason field (missing_body / below_quorum)")
        return True

    # ── exec saturation (SOFT) ──────────────────────────────────────────────
    def _exec_occupancy(self):
        dsum = 0.0
        dcnt = 0
        self.SIM_EXEC_LAST_OCC = ""
        for n in self.SIM_SCRAPE_NODES:
            t = self.SIM_SCRAPE_TEXT.get(n, "")
            if not t:
                continue
            s, c = self._summary_agg(t, "reth_dpos_derive_el_apply_duration_seconds")
            if not c > 0:
                continue
            ps = self.SIM_EXEC_SUM.get(n)
            pc = self.SIM_EXEC_CNT.get(n)
            self.SIM_EXEC_SUM[n] = s; self.SIM_EXEC_CNT[n] = c
            if ps is None:
                continue  # first sighting -> baseline only
            if c < pc or s < ps:
                continue  # counter reset (restart) -> re-baselined above
            dcnt += c - pc
            dsum += s - ps
        if not dcnt > 0:
            return
        self.SIM_EXEC_LAST_OCC = f"{dsum / dcnt / (self.SIM_BLOCK_INTERVAL or 1):.3f}"

    def _inv_exec_saturation(self):
        self._exec_occupancy()
        occ = self.SIM_EXEC_LAST_OCC
        if not occ:
            self.SIM_EXEC_SAT_ACC = 0
            return True
        occ_f = float(occ)
        if not self.SIM_EXEC_EARLY_FIRED and occ_f > self.SIM_EXEC_SAT_EARLY:
            self.SIM_EXEC_EARLY_FIRED = 1
            self.sim_event(
                "info",
                f"exec-saturation: first sighting of per-block EL occupancy {occ} > "
                f"{self.SIM_EXEC_SAT_EARLY} of the {self.SIM_BLOCK_INTERVAL}s block interval "
                "(early signal; reported, not asserted)")
        if occ_f > self.SIM_EXEC_SAT_THRESHOLD:
            self.SIM_EXEC_SAT_ACC += 1
            if self.SIM_EXEC_SAT_ACC == self.SIM_EXEC_SAT_TICKS:
                q90 = self._metric_val(
                    self.SIM_SCRAPE_TEXT.get(topology.PINNED_RPC_HOST, ""),
                                       "reth_dpos_derive_el_apply_duration_seconds",
                                       'quantile="0.9"')
                q90s = f" (v0 q0.9={q90}s)" if q90 else ""
                self.sim_event(
                    "warn",
                    f"exec-saturation: mean per-block EL derive+import occupancy {occ} > "
                    f"{self.SIM_EXEC_SAT_THRESHOLD} of the {self.SIM_BLOCK_INTERVAL}s block "
                    f"interval for {self.SIM_EXEC_SAT_ACC} consecutive ticks{q90s} — tx load "
                    "approaching execution saturation (reported, not asserted)")
        else:
            self.SIM_EXEC_SAT_ACC = 0
        return True

    # ── block rate (SOFT) ───────────────────────────────────────────────────
    def _inv_block_rate(self, fin):
        c = self.ctx
        now = self._now()
        if c.EXPECTED_STALL == 1:
            self.SIM_RATE_SLOW_ACC = 0; self.SIM_RATE_LAST = ""; self.SIM_RATE_LAST_FIN = -1
            return True
        if fin <= 0:
            self.SIM_RATE_SLOW_ACC = 0; self.SIM_RATE_LAST = ""
            return True
        if self.SIM_RATE_LAST_FIN < 0 or fin < self.SIM_RATE_LAST_FIN:
            self.SIM_RATE_LAST_FIN = fin; self.SIM_RATE_LAST_TS = now
            self.SIM_RATE_SLOW_ACC = 0; self.SIM_RATE_LAST = ""
            return True
        dfin = fin - self.SIM_RATE_LAST_FIN
        dts = now - self.SIM_RATE_LAST_TS
        if not dts > 0:
            self.SIM_RATE_LAST = ""
            return True
        self.SIM_RATE_LAST_FIN = fin; self.SIM_RATE_LAST_TS = now
        self.SIM_RATE_LAST = f"{dfin / dts:.3f}"
        self.SIM_RATE_TICKS += 1
        if self.SIM_RATE_TICKS % self.SIM_RATE_REPORT_TICKS == 0:
            self.sim_event(
                "info",
                f"block-rate: rate={self.SIM_RATE_LAST} blk/s (window {dts}s) — periodic "
                "timeline sample (reported, not asserted)")
        if float(self.SIM_RATE_LAST) < self.SIM_RATE_WARN:
            self.SIM_RATE_SLOW_ACC += 1
            if self.SIM_RATE_SLOW_ACC == 2:
                self.sim_event(
                    "warn",
                    f"block-rate: rate={self.SIM_RATE_LAST} blk/s < {self.SIM_RATE_WARN} for "
                    f"{self.SIM_RATE_SLOW_ACC} consecutive measured ticks — chain SLOW without "
                    "stalling (finalize-stall stays silent; reported, not asserted)")
        else:
            self.SIM_RATE_SLOW_ACC = 0
        return True

    # ── spec head (§9 item 2′) ──────────────────────────────────────────────
    def _inv_spec_head(self, fin, latest):
        c = self.ctx
        K = self.RESULT_LAG_K
        if (c.EXPECTED_STALL == 0 and fin > 0 and latest > 0 and latest >= fin
                and self.SIM_FLAT_TICKS == 0):
            if latest - fin == K:
                now = self._now()
                if self.SIM_SPECGAP_TICKS == 0:
                    self.SIM_SPECGAP_FIN0 = fin; self.SIM_SPECGAP_TS0 = now
                self.SIM_SPECGAP_TICKS += 1
                if self.SIM_SPECGAP_TICKS >= self.SIM_STALL_TICKS:
                    dfin = fin - self.SIM_SPECGAP_FIN0
                    dts = now - self.SIM_SPECGAP_TS0
                    if dfin <= 0 or dts <= 0:
                        self.SIM_SPECGAP_TICKS = 0; self.SIM_SPECGAP_FIN0 = -1; self.SIM_SPECGAP_TS0 = 0
                        return True
                    rate = dfin / dts
                    if rate >= self.SIM_SPEC_DEAD_MIN_RATE:
                        reuse, red = self._spec_derive_paths(dts + 30)
                        if red > reuse:
                            return self._inv_fail(
                                "spec-head-lag",
                                f"eth latest−finalized == K={K} for {self.SIM_SPECGAP_TICKS} "
                                f"ticks at rate={rate:.3f} blk/s (>= {self.SIM_SPEC_DEAD_MIN_RATE}, "
                                f"healthy) AND finalized derives are rederive-DOMINANT "
                                f"(reuse={reuse} rederive={red} over the window) — nothing is "
                                "being speculated: the EL head is riding FINALIZATION (Fix-1 "
                                "regression).")
                        self.SIM_SPECGAP_TICKS = 0; self.SIM_SPECGAP_FIN0 = -1; self.SIM_SPECGAP_TS0 = 0
                        return True
                    self.sim_event(
                        "spec_lag_degraded_rate",
                        f"eth latest−finalized == K={K} for {self.SIM_SPECGAP_TICKS} ticks but "
                        f"rate={rate:.3f} blk/s < threshold={self.SIM_SPEC_DEAD_MIN_RATE} "
                        f"(latest={latest} fin={fin}) — WAN-serialized regime; reported, not "
                        "asserted")
                    self.SIM_SPECGAP_TICKS = 0; self.SIM_SPECGAP_FIN0 = -1; self.SIM_SPECGAP_TS0 = 0
            else:
                self.SIM_SPECGAP_TICKS = 0; self.SIM_SPECGAP_FIN0 = -1; self.SIM_SPECGAP_TS0 = 0
        else:
            self.SIM_SPECGAP_TICKS = 0; self.SIM_SPECGAP_FIN0 = -1; self.SIM_SPECGAP_TS0 = 0
        return True

    # ── seed round (§9 item 6 / F4) ─────────────────────────────────────────
    def _inv_seed_round(self, fin):
        c = self.ctx
        if not (c.SIM_TICK % self.SIM_SEEDROUND_EVERY == 0):
            return True
        if not (c.EXPECTED_STALL == 0 and fin > 4):
            return True
        b = fin - 2
        first = None
        first_svc = None
        resp = 0
        for sa in (c.SIM_CUR_COMMITTEE or "").split():
            ss = c.ADDR2IDX.get(sa.lower())
            if not ss:
                continue
            if self._excluded(ss):
                continue
            line = self._seed_round_line(ss, b)
            if not line:
                continue
            # _sr="${_line#*seed_round=}"; _sr="${_sr%% prev_randao=*}"
            if "seed_round=" not in line:
                continue
            sr = line.split("seed_round=", 1)[1].split(" prev_randao=", 1)[0]
            if not sr or sr == line:
                continue
            resp += 1
            if first is None:
                first = sr; first_svc = ss
            elif sr != first:
                return self._inv_fail(
                    "seed-round-divergent",
                    f"nodes disagree on the seed ROUND consumed at height {b} (F4) — {ss} "
                    f"derived from seed_round={sr} vs {first_svc} from {first} "
                    "(dpos::derive_seed telemetry)")
        return self._detector_sampled(
            "seed-round", f"docker-logs (dpos::derive_seed at height {b})", resp, 2)

    # ── beacon window (Inv-3 + G1) ──────────────────────────────────────────
    def _inv_beacon_window(self, fin):
        c = self.ctx
        self.SIM_BW_L2_FIN = -1; self.SIM_BW_L3_FIN = -1
        if not fin > self.SIM_BEACON_WINDOW + 1:
            return True
        lo = fin - self.SIM_BEACON_WINDOW
        hi = fin - 1
        beacon_nodes = []
        for ba in (c.SIM_CUR_COMMITTEE or "").split():
            bs = c.ADDR2IDX.get(ba.lower())
            if not bs:
                continue
            if self._excluded(bs):
                continue
            beacon_nodes.append(bs)
        tiers = [] if c.SIM_NO_CASCADE == 1 else list(topology.CASCADE_TIERS)
        for bs in tiers:
            if self._excluded(bs):
                continue
            beacon_nodes.append(bs)
        min_resp = c.SIM_CUR_F + 1
        bw_best_resp = 0
        seen = []
        ceil = {}
        for svc in beacon_nodes:
            if topology.is_pinned_rpc_host(svc):
                ceil[svc] = fin
            elif svc == topology.FULL_NODE:
                self.SIM_BW_L2_FIN = self._fin_at(topology.HOST_L2_RPC_PORT)
                ceil[svc] = self.SIM_BW_L2_FIN
            elif svc == topology.DOWNSTREAM:
                self.SIM_BW_L3_FIN = self._fin_at(topology.HOST_L3_RPC_PORT)
                ceil[svc] = self.SIM_BW_L3_FIN
            else:
                ceil[svc] = self._node_fin_cached(svc)
        for b in range(lo, hi + 1):
            first = None
            agree = 1
            responders = 0
            fhash = None; fhash_svc = None; hash_agree = 1; hash_resp_val = 0
            hash_bad_svc = None; hash_bad = None
            for svc in beacon_nodes:
                if not (ceil.get(svc, -1) >= b):
                    continue
                mh = self._mixhash_of(svc, b)
                if mh == "null" or not mh:
                    continue
                if nodes.is_zero_hash(mh):
                    return self._inv_fail(
                        "beacon-zero",
                        f"prev_randao ZERO at block {b} on {svc} (beacon stalled / fell to "
                        "digest)")
                responders += 1
                if first is None:
                    first = mh
                elif mh != first:
                    agree = 0
                bh = self._blockhash_of(svc, b)
                if bh != "null" and bh:
                    if svc not in topology.CASCADE_TIERS:
                        hash_resp_val += 1
                        if fhash is None:
                            fhash = bh; fhash_svc = svc
                        elif bh != fhash:
                            hash_agree = 0; hash_bad_svc = svc; hash_bad = bh
            if responders > bw_best_resp:
                bw_best_resp = responders
            if responders < min_resp:
                continue
            if hash_resp_val >= min_resp and hash_agree == 0:
                self._fork_sleep()
                r1 = self._blockhash_of(fhash_svc, b)
                r2 = self._blockhash_of(hash_bad_svc, b)
                if r1 and r1 != "null" and r1 == r2:
                    self.sim_event(
                        "fork_transient_skew",
                        f"finalized-hash disagreement at height {b} healed on re-read — first "
                        f"read {hash_bad_svc}={hash_bad} vs {fhash_svc}={fhash}, re-read "
                        f"both={r1} (mid-reorg/read skew, NOT a fork; reported, not asserted)")
                else:
                    return self._inv_fail(
                        "fork-detected",
                        f"nodes disagree on finalized block hash at height {b} (FORK, persisted "
                        f"across a {self.SIM_FORK_RECHECK_DELAY}s re-read) — first read "
                        f"{hash_bad_svc}={hash_bad} vs {fhash_svc}={fhash}; re-read "
                        f"{hash_bad_svc}={r2} vs {fhash_svc}={r1}")
            if agree == 0:
                return self._inv_fail(
                    "beacon-divergent",
                    f"nodes disagree on prev_randao at block {b} (divergent seed)")
            seen.append(first)
        if not self._detector_sampled(
                "beacon-fork",
                f"json-rpc block reads (mixHash/hash over [{lo}..{hi}]; {len(beacon_nodes)} probe nodes)",
                bw_best_resp, min_resp):
            return False
        if len(seen) > 1:
            if len(set(seen)) == 1:
                return self._inv_fail(
                    "beacon-stuck",
                    f"prev_randao identical across {len(seen)} blocks [{lo}..{hi}] (stuck "
                    "randomness)")
        return True

    # ── result agreement (SAFETY) ───────────────────────────────────────────
    def _inv_result_agreement(self, fin):
        c = self.ctx
        h = fin - self.RESULT_LAG_K
        if not h > 0:
            return True
        rnodes = []
        for ra in (c.SIM_CUR_COMMITTEE or "").split():
            rs = c.ADDR2IDX.get(ra.lower())
            if not rs:
                continue
            if self._excluded(rs):
                continue
            rnodes.append(rs)
        min_resp = c.SIM_CUR_F + 1
        svcs = []
        roots_all = []
        resp = 0
        for rs in rnodes:
            ceil = fin if topology.is_pinned_rpc_host(rs) else self._node_fin_cached(rs)
            if not (ceil >= h):
                continue
            roots = self._node_roots_of(rs, h)
            if roots.startswith("null") or not roots:
                continue
            resp += 1; svcs.append(rs); roots_all.append(roots)
        caddrs = (c.SIM_CUR_COMMITTEE or "").split()
        if not self._detector_sampled(
                "result-agreement",
                f"json-rpc (in-container eth_getBlockByNumber at fin−K={h}; resolved "
                f"{len(rnodes)} of {len(caddrs)} committee addrs)", resp, min_resp):
            return False
        if not resp >= min_resp:
            return True
        best = None; best_n = 0; tie = 0
        for i in range(resp):
            cnt = sum(1 for j in range(resp) if roots_all[j] == roots_all[i])
            if cnt > best_n:
                best = roots_all[i]; best_n = cnt; tie = 0
            elif cnt == best_n and roots_all[i] != best:
                tie = 1
        if best_n == resp:
            return True
        maj_svc = None; min_svc = None; min_roots = None
        for i in range(resp):
            if roots_all[i] == best:
                if maj_svc is None:
                    maj_svc = svcs[i]
            else:
                if min_svc is None:
                    min_svc = svcs[i]; min_roots = roots_all[i]
        split_note = ""; maj_label = "MAJORITY"; min_label = "MINORITY"
        if tie == 1:
            maj_label = "SIDE-A"; min_label = "SIDE-B"
            split_note = (f" (NO majority — the responders split {best_n}/{resp - best_n}; "
                          "neither side is presumed honest)")
        self._fork_sleep()
        r1 = self._node_roots_of(maj_svc, h)
        r2 = self._node_roots_of(min_svc, h)
        if not r1 or r1.startswith("null") or not r2 or r2.startswith("null"):
            self.sim_event(
                "result_recheck_unreadable",
                f"executed-result disagreement at finalized height {h} could NOT be confirmed "
                f"— the re-read returned the unreachable sentinel ({maj_svc}=[{r1 or '<empty>'}] "
                f"{min_svc}=[{r2 or '<empty>'}]); INCONCLUSIVE, not a divergence")
            return True
        if r1 == r2:
            self.sim_event(
                "result_transient_skew",
                f"executed-result disagreement at finalized height {h} healed on re-read — "
                f"re-read both=[{r1}] (read race, NOT a divergence; reported, not asserted)")
            return True
        return self._inv_fail(
            "result-divergence",
            f"nodes disagree on the EXECUTED result at finalized height {h} = fin({fin})−K("
            f"{self.RESULT_LAG_K}) — {min_label} {min_svc} stateRoot/receiptsRoot=[{min_roots}] "
            f"vs {maj_label} ({best_n} of {resp} responders, e.g. {maj_svc})=[{best}]{split_note} "
            f"(re-read {min_svc}=[{r2}] vs {maj_svc}=[{r1}]); a two-sided SAFETY violation "
            "detected INDEPENDENTLY of the product's own result_matches metric")

    # ── finalized chain walk (double-finalization) ──────────────────────────
    def _inv_finalized_chain(self, fin):
        c = self.ctx
        if not fin > 0:
            return True
        cnodes = []
        for ca in (c.SIM_CUR_COMMITTEE or "").split():
            cs = c.ADDR2IDX.get(ca.lower())
            if not cs:
                continue
            if self._excluded(cs):
                continue
            cnodes.append(cs)
        walked = 0
        cur0 = fin - self.SIM_FIN_WALK_MAX
        if cur0 < 0:
            cur0 = 0
        for node in cnodes:
            ceil = fin if topology.is_pinned_rpc_host(node) else self._node_fin_cached(node)
            if not (isinstance(ceil, int) and ceil > 0):
                continue
            lo = self.SIM_FIN_CURSOR.get(node, cur0) + 1
            if lo > ceil:
                walked += 1
                continue
            hi = lo + self.SIM_FIN_WALK_MAX - 1
            if hi > ceil:
                hi = ceil
            h = lo
            while h <= hi:
                hash_ = self._blockhash_of(node, h)
                if not hash_ or hash_ == "null":
                    break
                first = self.SIM_FIN_LEDGER.get(h)
                if not first:
                    self.SIM_FIN_LEDGER[h] = hash_
                elif hash_ != first:
                    self._fork_sleep()
                    r = self._blockhash_of(node, h)
                    if not r or r == "null":
                        break
                    if r != first:
                        return self._inv_fail(
                            "double-finalization",
                            f"nodes finalized DIFFERENT blocks at height {h} (DOUBLE "
                            f"FINALIZATION, persisted across a {self.SIM_FORK_RECHECK_DELAY}s "
                            f"re-read) — {node}=[{r}] vs the first-agreed ledger hash=[{first}]; "
                            f"height {h} is at-or-below {node}'s own finalized tag {ceil}")
                self.SIM_FIN_CURSOR[node] = h
                h += 1
            walked += 1
        # prune the ledger below the global-minimum cursor
        if self.SIM_FIN_CURSOR:
            mc = min(self.SIM_FIN_CURSOR.values())
            if mc > 0:
                for lh in list(self.SIM_FIN_LEDGER.keys()):
                    if lh < mc:
                        del self.SIM_FIN_LEDGER[lh]
        return self._detector_sampled(
            "finalized-chain", "json-rpc finalized block-hash walk", walked, 2)

    # ── member liveness (audit item 2) ──────────────────────────────────────
    def _inv_member_liveness(self, fin):
        c = self.ctx
        if c.EXPECTED_STALL != 0 or fin <= 0 or self.SIM_FLAT_TICKS != 0:
            return True
        if not settle_gate_open(c.SIM_CUR_EPOCH, c.SETTLE_UNTIL_EPOCH, self.SIM_SETTLE_GATE_CAP):
            return True
        mnodes = []
        for ma in (c.SIM_CUR_COMMITTEE or "").split():
            ms = c.ADDR2IDX.get(ma.lower())
            if not ms:
                continue
            if self._excluded(ms):
                continue
            mnodes.append(ms)
        sampled = 0
        for node in mnodes:
            mfin = fin if topology.is_pinned_rpc_host(node) else self._node_fin_cached(node)
            if not (isinstance(mfin, int) and mfin > 0):
                continue
            last = self.SIM_MEMBER_FIN.get(node)
            if last is None:
                self.SIM_MEMBER_FIN[node] = mfin; self.SIM_MEMBER_FLAT[node] = 0; self.SIM_MEMBER_ARMED[node] = 0
            elif mfin > last:
                self.SIM_MEMBER_FIN[node] = mfin; self.SIM_MEMBER_FLAT[node] = 0; self.SIM_MEMBER_ARMED[node] = 1
            elif mfin < last:
                self.SIM_MEMBER_FIN[node] = mfin; self.SIM_MEMBER_FLAT[node] = 0; self.SIM_MEMBER_ARMED[node] = 0
            elif self.SIM_MEMBER_ARMED.get(node, 0) == 1:
                self.SIM_MEMBER_FLAT[node] = self.SIM_MEMBER_FLAT.get(node, 0) + 1
                if self.SIM_MEMBER_FLAT[node] >= self.SIM_STALL_TICKS:
                    return self._inv_fail(
                        "member-stall",
                        f"{node} OWN finalized FLAT at {mfin} for {self.SIM_MEMBER_FLAT[node]} "
                        f"ticks (>= {self.SIM_STALL_TICKS}) after proven progress while the "
                        f"chain advanced (v0 fin={fin}) — a seated committee member is silently "
                        "WEDGED")
            if self.SIM_MEMBER_ARMED.get(node, 0) == 1:
                sampled += 1
        return self._detector_sampled(
            "member-liveness",
            f"json-rpc own-finalized reads (armed/{len(mnodes)} live committee members)",
            sampled, 2)

    # ── battery coverage ────────────────────────────────────────────────────
    def _inv_battery_coverage(self):
        c = self.ctx
        cov = c.SIM_BATTERY_COVERAGE
        if not isinstance(cov, int):
            return True
        min_cov = c.SIM_CUR_F + 1
        if cov >= min_cov:
            self.SIM_COVERAGE_LOW_TICKS = 0
            return True
        self.SIM_COVERAGE_LOW_TICKS += 1
        if self.SIM_COVERAGE_LOW_TICKS >= self.SIM_COVERAGE_MIN_TICKS:
            return self._inv_fail(
                "battery-coverage",
                f"cross-node battery reached only {cov} committee node(s) (< f+1 = {min_cov}) "
                f"for {self.SIM_COVERAGE_LOW_TICKS} consecutive ticks (>= "
                f"{self.SIM_COVERAGE_MIN_TICKS}) on the Prometheus METRICS channel — the "
                "battery is running over a sub-quorum (possibly EMPTY) node set. This is a "
                "battery failure, not chain health")
        return True

    # ── peer plane (Inv-7) ──────────────────────────────────────────────────
    def _inv_peer_plane(self):
        c = self.ctx
        if not (c.SIM_TICK % self.SIM_PEER_EVERY == 0):
            return True
        pref = c.SIM_PEER_SPOKE or topology.CERT_UPSTREAM_ANCHOR
        spoke = ""
        seated = []
        for pa in (c.SIM_CUR_COMMITTEE or "").split():
            ps = c.ADDR2IDX.get(pa.lower())
            if not ps:
                continue
            if self._excluded(ps):
                continue
            if ps == pref:
                spoke = ps
            if not topology.is_pinned_rpc_host(ps):
                seated.append(ps)
        if not spoke:
            spoke = seated[0] if seated else ""
        if not spoke:
            return True
        if spoke != self.SIM_PEER_LAST_SPOKE:
            self.SIM_PEER_ZERO = 0; self.SIM_PEER_LAST_SPOKE = spoke
        pc = self._peer_count(spoke)
        if pc == 0:
            self.SIM_PEER_ZERO += 1
            if self.SIM_PEER_ZERO >= 4:
                return self._inv_fail(
                    "peer-plane",
                    f"reth net_peerCount=0 on {spoke} (seated committee member) across 4 "
                    "steady-state checks (devp2p plane down)")
        else:
            self.SIM_PEER_ZERO = 0
        return True

    # ── cascade liveness (Inv-8 SOFT) ───────────────────────────────────────
    def _inv_cascade_liveness(self, fin):
        c = self.ctx
        if c.SIM_NO_CASCADE != 1 and (c.EXPECTED_STALL == 0 and self.SIM_FLAT_TICKS == 0
                                       and fin > self.SIM_BEACON_WINDOW):
            l2f = (self.SIM_BW_L2_FIN if self.SIM_BW_L2_FIN >= 0
                   else self._fin_at(topology.HOST_L2_RPC_PORT))
            if l2f > self.SIM_L2_LAST:
                self.SIM_L2_FLAT = 0; self.SIM_L2_LAST = l2f
            else:
                self.SIM_L2_FLAT += 1
                if self.SIM_L2_FLAT == 4 * self.SIM_STALL_TICKS:
                    self.sim_event("warn",
                                    f"L2 cert-follower ({topology.FULL_NODE}) finalized "
                                    f"lagging at {l2f} for "
                                    f"{self.SIM_L2_FLAT} ticks while v0 advances (follower health "
                                    "— cert-cascade lag, NOT chain-safety)")
            l3f = (self.SIM_BW_L3_FIN if self.SIM_BW_L3_FIN >= 0
                   else self._fin_at(topology.HOST_L3_RPC_PORT))
            if l3f > self.SIM_L3_LAST:
                self.SIM_L3_FLAT = 0; self.SIM_L3_LAST = l3f
            else:
                self.SIM_L3_FLAT += 1
                if self.SIM_L3_FLAT == 4 * self.SIM_STALL_TICKS:
                    self.sim_event("warn",
                                    f"L3 cascade ({topology.DOWNSTREAM}) finalized "
                                    f"lagging at {l3f} for "
                                    f"{self.SIM_L3_FLAT} ticks while v0 advances (follower health "
                                    "— cert-cascade lag, NOT chain-safety)")
        return True

    # ── participation scrape / fair credit — RETIRED ────────────────────────
    # Both detectors watched the vote BITMAP: `_inv_participation_scrape` logged per-member
    # seen/certs at each window finalize, and `_inv_fair_credit` asserted that a fully-live
    # member stayed above the participation floor despite commonware's first-quorum vote
    # shutoff. That fairness defect is closed AT THE ROOT — the chain no longer records who
    # signed a cert, it records who PRODUCED each block, verified by every honest voter
    # against the consensus-supplied round leader. There is no bitmap to be unfair about, no
    # participation floor to sit below, and `lastFinalizedEpochP1` (both detectors' clock) no
    # longer exists. `_inv_fair_credit` also armed on `dpos_union_extra_signers`, a metric
    # deleted with the vote tap that fed it.

    def _inv_write_path(self):
        c = self.ctx
        if c.EXPECTED_STALL == 0 and self.SIM_FLAT_TICKS == 0 and c.L3_SPAMMER_ADDR:
            out = nodes.rpc._run_read(
                ["cast", "nonce", c.L3_SPAMMER_ADDR, "--rpc-url", nodes.RPC],
                nodes.rpc.RPC_EXEC_TIMEOUT)
            try:
                wpn = int(out.strip())
            except Exception:
                wpn = 0
            if wpn > self.SIM_WP_LAST:
                self.SIM_WP_FLAT = 0; self.SIM_WP_LAST = wpn
            else:
                self.SIM_WP_FLAT += 1
                if self.SIM_WP_FLAT == 4 * self.SIM_STALL_TICKS:
                    self.sim_event("warn",
                                    f"write-path: L3-submitted txs not landing — sender nonce flat "
                                    f"at {wpn} for {self.SIM_WP_FLAT} ticks while v0 finalizes")
        return True



class Battery(ChainBattery):
    """The FULL battery: the 27 chain-only detectors inherited from ChainBattery plus the 5
    that are sim-coupled BY NATURE, and the ordered dispatch that runs them.

    The five below cannot be written without simulation state and pretending otherwise would
    be dishonest: wrongful-slash has to know which faults THIS harness caused (EVER_FAULTED)
    to tell a wrongful jail from an expected one; node-down has to know which container is a
    deliberately-idle rotation slot; the dkg-commit and committee-membership detectors judge
    the seat/refill machine; fair-credit resolves a seated address back to a pool identity
    through OWNER_ADDR_CACHE.

    They read `self.sim`, which exists ONLY on this class. A chain-only detector that reaches
    for simulation state therefore breaks on ChainBattery — the class a non-simulation
    consumer instantiates — instead of quietly working because the sim happened to be there.
    """

    def __init__(self, ctx: Ctx = None, sim: SimCtx = None, events=None):
        super().__init__(ctx=ctx, events=events)
        self.sim = sim if sim is not None else SimCtx()

    def bind(self, ctx: Ctx, sim: SimCtx):
        """Rebind BOTH halves of the per-tick context in one call. Two assignments meant a
        caller could refresh the chain facts and leave the sim bookkeeping frozen at last
        tick's snapshot — the half-bound-seam failure this harness has already paid for once
        (the warm_ready confirm seam). There is no single-half setter."""
        self.ctx = ctx
        self.sim = sim

    # ── node down (sim-coupled: the idle-slot / spare-band predicate) ───────
    def _node_down_excluded(self, container, ec):
        c = self.ctx
        s = self.sim
        idx_s = container.rsplit("-", 1)[-1]
        if _in_set(container, self.SIM_ONESHOT_SERVICES):
            return ec == "0" or ec == 0  # one-shot ran to completion -> excluded; nonzero still fails
        if _in_set(container, c.SIM_DISRUPTED):
            return True
        if container in c.PERMANENTLY_DEAD and c.PERMANENTLY_DEAD[container]:
            return True
        if not idx_s.isdigit():
            return False  # full-node/downstream -> always expected up
        idx = int(idx_s)
        if s.SIM_VALIDATORS <= idx < s.SIM_SPARE_END:
            if idx < s.NEXT_JOINER:
                return False  # refill spare already activated -> expected up
            if container == s.REFILL_RESUMED:
                return False  # currently resuming -> expected up
            return True       # spare never resumed -> deliberately stopped
        if s.SIM_SPARE_END <= idx < s.SIM_VAL_CONTAINERS:
            if s.SLOT_IDENTITY.get(container, idx) != idx:
                return False  # rotation slot hosting a mint -> expected up
            return True       # idle rotation slot -> deliberately stopped
        return False          # committee / growth idx -> expected up

    def _inv_node_down(self):
        for row in self._down_containers():
            if not row:
                continue
            parts = row.split("|")
            container = parts[0]
            st = parts[1] if len(parts) > 1 else ""
            ec = parts[2] if len(parts) > 2 else ""
            if not container:
                continue
            if self._node_down_excluded(container, ec):
                continue
            tail = self._down_logtail(container)
            v = node_down_verdict(container, st, ec, tail)
            if v:
                return self._inv_fail(*v)
        return True

    # ── safety-halt (COV-1) ─────────────────────────────────────────────────
    # ── dkg commit binding (Phase D / I4) ───────────────────────────────────
    def _dkg_pending_onchain(self, sig):
        """1 = corroborated on-chain (keep pending); 0 = definitively NotFound
        (downgrade); -1 = UNKNOWN (keep pending). Only a growth-join sig is a fresh
        registration to check."""
        if sig == "":
            return -1
        sm_in = f"sm:in:{topology.VALIDATOR_PREFIX}"   # built at _inv_dkg_commit_binding
        if sm_in in sig:
            idx = sig.split(sm_in, 1)[1].split(",", 1)[0]
        else:
            return 1
        if not idx.isdigit():
            return -1
        # F2: OWNER_ADDR_CACHE is keyed by str(idx) everywhere (build_addr_map + the wrongful-slash
        # reverse-lookup + EVER_FAULTED share the string domain). `idx` is already the numeric
        # STRING sliced from the sig, so look it up directly — an int() key here was the drift.
        addr = self.sim.OWNER_ADDR_CACHE.get(idx)
        if not (addr and addr.startswith("0x")):
            return -1
        st = self._pp_validator_status(addr)
        if not (st and str(st).isdigit()):
            return -1
        if int(st) == 0:
            return 0
        return 1

    def _dkg_defer_streak_step(self, pending, deferred, sig):
        if pending == 0:
            self.SIM_DKG_DEFER_STREAK = 0; self.SIM_DKG_DEFER_TARGET = ""
            return
        if sig != self.SIM_DKG_DEFER_TARGET:
            self.SIM_DKG_DEFER_STREAK = 0; self.SIM_DKG_DEFER_TARGET = sig
        if deferred == 1:
            self.SIM_DKG_DEFER_STREAK += 1
        else:
            self.SIM_DKG_DEFER_STREAK = 0

    def _dkg_defer_streak_verdict(self, fs):
        if self.SIM_DKG_DEFER_STREAK >= fs:
            return "fail"
        if self.SIM_DKG_DEFER_STREAK >= 2:
            return "warn"
        return "ok"

    def _inv_dkg_commit_binding(self, cur):
        s = self.sim
        if cur < 1 or cur == self.SIM_DKG_IFF_LAST_EPOCH:
            return True
        self.SIM_DKG_IFF_LAST_EPOCH = cur
        q = self._pp_dkg_qual(cur)
        qs = dkg_qual_state(q)
        cprev = self._pp_committee(cur - 1)
        ccur = self._pp_committee(cur)
        if not cprev or not ccur:
            return True
        changed = 1 if ccur != cprev else 0
        verdict, msg = dkg_iff_verdict(changed, qs)
        if verdict == "fail":
            return self._inv_fail(
                "dkg-commit-binding",
                f"epoch boundary {cur - 1}->{cur}: {msg} (committee[{cur - 1}]=[{cprev}] "
                f"committee[{cur}]=[{ccur}] getDkgQual({cur})={q or '<unreadable>'})")
        if verdict == "watch":
            self.sim_event("warn",
                            f"dkg-commit-binding: boundary {cur - 1}->{cur} — {msg} "
                            f"(getDkgQual({cur})={q or '<unreadable>'})")
        if qs >= 0:
            pending = 0; sig = ""
            if s.NEXT_JOINER - 1 > s.GROW_LANDED:
                pending = 1; sig = f"sm:in:{topology.validator(s.NEXT_JOINER - 1)}"
            if s.PROMOTE_PENDING:
                pending = 1; sig = (sig + "," if sig else "") + f"sm:promote:{s.PROMOTE_PENDING}"
            if s.REFILL_PENDING:
                pending = 1; sig = (sig + "," if sig else "") + f"sm:refill:{s.REFILL_PENDING}"
            if pending == 1:
                oc = self._dkg_pending_onchain(sig)
                if oc == 0:
                    pending = 0
                    self.sim_event(
                        "warn",
                        f"dkg-commit-binding: state-machine change [{sig}] is NOT registered "
                        f"on-chain (getValidatorStatus=NotFound) — boundary {cur - 1}->{cur} is "
                        "NOT charged as a chain deferral (item 6a)")
            deferred = 1 if (changed == 0 and qs == 0) else 0
            self._dkg_defer_streak_step(pending, deferred, sig)
            dv = self._dkg_defer_streak_verdict(self.SIM_DKG_DEFER_FAIL_STREAK)
            if dv == "fail":
                return self._inv_fail(
                    "dkg-deferral-streak",
                    f"pending membership change [{sig}] DEFERRED {self.SIM_DKG_DEFER_STREAK} "
                    f"consecutive boundaries (>= {self.SIM_DKG_DEFER_FAIL_STREAK}) without "
                    f"qualifying — the change is STARVED. boundary {cur - 1}->{cur}: committee "
                    f"UNCHANGED=[{ccur}] getDkgQual({cur})={q or '<unreadable>'}", entity=sig)
            if dv == "warn":
                self.sim_event(
                    "warn",
                    f"dkg-deferral-streak: pending change [{sig}] deferred "
                    f"{self.SIM_DKG_DEFER_STREAK} consecutive boundaries (>=2) — watching for "
                    f"starvation (FAIL at {self.SIM_DKG_DEFER_FAIL_STREAK})")
        return True

    # ── wrongful slash (audit item 3) ───────────────────────────────────────
    def _wrongslash_idx_of(self, want):
        want = want.lower()
        for k, v in self.sim.OWNER_ADDR_CACHE.items():
            if str(v).lower() == want:
                return k
        return None

    def _inv_wrongful_slash(self):
        c = self.ctx
        s = self.sim
        if not (c.SIM_TICK % self.SIM_WRONGSLASH_EVERY == 0):
            return True
        readable = 0
        for addr in list(s.EVER_MEMBERS.keys()):
            st = self._pp_validator_status(addr)
            if not (st and str(st).isdigit()):
                self.SIM_WRONGSLASH_ACC.pop(addr, None)
                continue
            readable += 1
            st_i = int(st)
            jail = 1 if st_i == self.SIM_JAIL_STATUS_BYTE else 0
            if jail == 0:
                self.SIM_WRONGSLASH_ACC.pop(addr, None)
                continue
            idx = self._wrongslash_idx_of(addr)
            mapped = 0; expected = 0
            if idx is not None:
                mapped = 1
                if s.EVER_FAULTED.get(idx):
                    expected = 1
            cls = wrongful_slash_verdict(jail, expected, mapped)
            if cls == "clear":
                self.SIM_WRONGSLASH_ACC.pop(addr, None)
                continue
            acc = self.SIM_WRONGSLASH_ACC.get(addr, 0) + 1
            self.SIM_WRONGSLASH_ACC[addr] = acc
            if acc >= self.SIM_WRONGSLASH_BELT:
                if cls == "wrongful":
                    return self._inv_fail(
                        "wrongful-slash",
                        f"on-chain JAIL (status={st_i}) observed for {addr} (identity idx {idx}) "
                        f"which the harness NEVER faulted (idx not in EVER_FAULTED), persisted "
                        f"{acc} observations — a WRONGFUL slash of an honest validator")
                return self._inv_fail(
                    "wrongful-slash",
                    f"on-chain JAIL (status={st_i}) observed for {addr} but the identity is "
                    f"UNRESOLVABLE (not in OWNER_ADDR_CACHE), persisted {acc} observations — "
                    "either a seated stranger was slashed or the identity map regressed")
            self.sim_event(
                "warn",
                f"wrongful-slash: {addr} jailed on-chain, {cls} attribution "
                f"(idx={idx if idx is not None else '<unmapped>'}) — WATCHING (fail-loud after "
                f"{self.SIM_WRONGSLASH_BELT} observations)")
        return self._detector_sampled(
            "wrongful-slash", "on-chain getValidatorStatus reads (EVER_MEMBERS)", readable, 1)

    # ── committee membership (audit item 4) ─────────────────────────────────
    def _backfill_pause_tick(self, v, cur, act):
        if act == 1:
            last = self.BACKFILL_PAUSE_LASTE.get(v)
            if last is not None and cur > last:
                self.BACKFILL_PAUSE_CREDIT[v] = self.BACKFILL_PAUSE_CREDIT.get(v, 0) + cur - last
            self.BACKFILL_PAUSE_LASTE[v] = cur
        else:
            self.BACKFILL_PAUSE_LASTE.pop(v, None)

    def _inv_committee_membership(self):
        c = self.ctx
        s = self.sim
        caddrs = (c.SIM_CUR_COMMITTEE or "").split()
        if not len(caddrs) > 0:
            return True
        sz = len(caddrs)
        f_run = c.SIM_CUR_F
        seat_floor = c.MIN_COMMITTEE
        bft_floor = 3 * f_run + 1
        if sz < bft_floor:
            self.SIM_MEMBERSHIP_SIZE_ACC += 1
            self.SIM_MEMBERSHIP_VAC_EPOCH = -1
            self.BACKFILL_PAUSE_CREDIT.pop("vacancy", None)
            self.BACKFILL_PAUSE_LASTE.pop("vacancy", None)
            if self.SIM_MEMBERSHIP_SIZE_ACC >= self.SIM_MEMBERSHIP_BELT:
                return self._inv_fail(
                    "committee-undersized",
                    f"committee size {sz} < BFT bound 3*f_run+1 = {bft_floor} (f_run={f_run}) for "
                    f"{self.SIM_MEMBERSHIP_SIZE_ACC} ticks (>= {self.SIM_MEMBERSHIP_BELT}) — "
                    f"below the true BFT safety bound (members=[{c.SIM_CUR_COMMITTEE}])")
        elif seat_floor > bft_floor and sz < seat_floor:
            self.SIM_MEMBERSHIP_SIZE_ACC = 0
            ce = c.SIM_CUR_EPOCH
            if self.SIM_MEMBERSHIP_VAC_EPOCH < 0:
                self.SIM_MEMBERSHIP_VAC_EPOCH = ce
                self.BACKFILL_PAUSE_CREDIT.pop("vacancy", None)
                self.BACKFILL_PAUSE_LASTE.pop("vacancy", None)
            vac_inflight = 0
            if ((s.REFILL_RESUMED or s.REFILL_PENDING or s.PROMOTE_PENDING)
                    and s.REFILL_STALL_VERDICT != "stall"):
                vac_inflight = 1
            self._backfill_pause_tick("vacancy", ce, vac_inflight)
            overdue, budget = backfill_overdue(
                ce, self.SIM_MEMBERSHIP_VAC_EPOCH, self.BACKFILL_PAUSE_CREDIT.get("vacancy", 0),
                self.SIM_MEMBERSHIP_SETTLE, self.SIM_GROW_LAND_DEADLINE)
            if overdue:
                return self._inv_fail(
                    "committee-vacancy",
                    f"committee size {sz} below MIN_COMMITTEE {seat_floor} (but >= BFT bound "
                    f"{bft_floor}, f_run={f_run}) for {ce - self.SIM_MEMBERSHIP_VAC_EPOCH} epochs "
                    f"(> ACTIONABLE budget {budget}; clock paused "
                    f"{self.BACKFILL_PAUSE_CREDIT.get('vacancy', 0)} epochs while a backfill was "
                    f"in-flight) — seat vacancy never backfilled (members=[{c.SIM_CUR_COMMITTEE}])")
            self.sim_event(
                "warn",
                f"committee-membership: seat vacancy — size {sz} < MIN_COMMITTEE {seat_floor} but "
                f">= BFT bound {bft_floor} (f_run={f_run}), "
                f"{ce - self.SIM_MEMBERSHIP_VAC_EPOCH}/{budget} epochs elapsed (paused "
                f"{self.BACKFILL_PAUSE_CREDIT.get('vacancy', 0)}, in-flight={vac_inflight}) — "
                "WATCHING (fail-loud past ACTIONABLE budget)")
        else:
            self.SIM_MEMBERSHIP_SIZE_ACC = 0
            self.SIM_MEMBERSHIP_VAC_EPOCH = -1
            self.BACKFILL_PAUSE_CREDIT.pop("vacancy", None)
            self.BACKFILL_PAUSE_LASTE.pop("vacancy", None)
        resolved = 0
        for addr in caddrs:
            cont = c.ADDR2IDX.get(addr.lower())
            if cont:
                resolved += 1
                self.SIM_MEMBERSHIP_STRANGER_ACC.pop(addr.lower(), None)
                continue
            # DEFENSE-IN-DEPTH (consumed-spare eviction, bundle-20260720T170507Z). ADDR2IDX maps
            # only HOSTED identities, so a seated member whose container was re-tasked or died reads
            # as an ADDR2IDX miss — the old code called that a SEATED STRANGER. Before crying
            # stranger, reverse-resolve the address through the immutable OWNER_ADDR_CACHE (the SAME
            # _wrongslash_idx_of seam wrongful-slash uses, which survives container recycle because
            # its key is the identity idx). A HIT means the address IS one of ours, just UNHOSTED (a
            # live seat with no running host — a real fault, still fail-loud past the belt, but
            # reported honestly, not as an intruder). A MISS everywhere is a genuine stranger / map
            # regression. The belt (SIM_MEMBERSHIP_STRANGER_ACC) is SHARED across both paths.
            uidx = self._wrongslash_idx_of(addr.lower())
            sacc = self.SIM_MEMBERSHIP_STRANGER_ACC.get(addr.lower(), 0) + 1
            self.SIM_MEMBERSHIP_STRANGER_ACC[addr.lower()] = sacc
            if sacc >= self.SIM_MEMBERSHIP_BELT:
                if uidx is not None:
                    return self._inv_fail(
                        "committee-unhosted",
                        f"seated member's identity idx {uidx} (addr {addr}) is registered but "
                        f"UNHOSTED (container re-tasked/dead — OWNER_ADDR_CACHE resolves it, "
                        f"ADDR2IDX does not) for {sacc} consecutive ticks (>= "
                        f"{self.SIM_MEMBERSHIP_BELT}) — a live committee seat with no running host")
                return self._inv_fail(
                    "committee-stranger",
                    f"seated committee address {addr} does not resolve to ANY harness-registered "
                    f"identity (ADDR2IDX miss, and no OWNER_ADDR_CACHE match) for {sacc} consecutive "
                    f"ticks (>= {self.SIM_MEMBERSHIP_BELT}) — a SEATED STRANGER, or the address map "
                    "regressed", entity=addr.lower())
            if uidx is not None:
                self.sim_event(
                    "warn",
                    f"committee-membership: seated member idx {uidx} (addr {addr}) is registered but "
                    "UNHOSTED (ADDR2IDX miss, OWNER_ADDR_CACHE hit) — counted + watching (fail-loud "
                    f"after {self.SIM_MEMBERSHIP_BELT} ticks; a container rebirth resolves sooner)")
            else:
                self.sim_event(
                    "warn",
                    f"committee-membership: seated address {addr} is UNMAPPED (no ADDR2IDX entry, no "
                    f"OWNER_ADDR_CACHE match) — counted + watching (fail-loud after "
                    f"{self.SIM_MEMBERSHIP_BELT} ticks; a mint-race resolves sooner)")
        return self._detector_sampled(
            "committee-membership",
            f"committee address resolution ({resolved}/{len(caddrs)} seated addrs resolved)",
            resolved, 1)

    def check_invariants(self):
        """Returns True if every invariant holds this tick, else False with
        inv_fail_id/inv_fail_msg set. Call order matches the bash inline order."""
        c = self.ctx
        self.inv_fail_id = ""; self.inv_fail_msg = ""
        fin = self._finalized_dec()
        latest = self._eth_latest()
        lat, lrf = self._consensus_latest()
        v0m = self._node_metrics(topology.PINNED_RPC_HOST)
        self._scrape_metrics(v0m)
        self._prime_node_fin(fin)

        if not self._inv_data_root_fill():
            return False
        if not self._inv_host_mem():
            return False
        if not self._inv_rpc_alive(fin):
            return False
        if not self._inv_node_panic():
            return False
        if not self._inv_node_down():
            return False
        if not self._inv_battery_coverage():
            return False
        if not self._inv_committee_membership():
            return False
        if not self._inv_safety_halt():
            return False
        if not self._inv_witness_counters():
            return False
        if not self._inv_dkg_pinned_idx():
            return False
        if not self._inv_beacon_active(fin, v0m):
            return False
        if not self._inv_finalize_stall(fin):
            return False
        if not self._inv_klag(fin, latest, lat, lrf):
            return False
        if not self._inv_spec_head(fin, latest):
            return False
        if not self._inv_member_liveness(fin):
            return False
        if not self._inv_deferred_park():
            return False
        if not self._inv_witness_embedded(fin):
            return False
        if not self._inv_beacon_window(fin):
            return False
        if not self._inv_seed_round(fin):
            return False
        if not self._inv_result_agreement(fin):
            return False
        if not self._inv_finalized_chain(fin):
            return False
        if not self._inv_dkg_commit_binding(c.SIM_CUR_EPOCH):
            return False
        if not self._inv_wrongful_slash():
            return False
        if not self._inv_peer_plane():
            return False
        if not self._inv_cascade_liveness(fin):
            return False
        if not self._inv_write_path():
            return False
        # SOFT reporters (always True) — last so a hard fail above wins the bundle.
        self._inv_witness_watch()
        self._inv_dkg_finalize_deferred()
        self._inv_exec_saturation()
        self._inv_block_rate(fin)
        return True


# ── small numeric helpers (bash `(( 16#.. ))` / regex-int guards) ────────────
def _num(v):
    try:
        return int(str(v).strip())
    except Exception:
        return 0


def _isint(v):
    return bool(v) and re.fullmatch(r"[0-9]+", str(v)) is not None

