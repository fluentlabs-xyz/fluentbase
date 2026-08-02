"""orchestrator.py — the production-sim churn orchestrator. Python port of case-soak.sh.

THE CROWN JEWEL. This is a TRANSLATION port: semantics identical, the SACRED 4-draw PRNG
discipline preserved bit-for-bit so old SIM_SEEDs replay the identical INTENT schedule, and
every incident-citing bash comment transcribed to the ported site.

What lives here:
  * SimConfig  — the SIM_* env knobs (defaults byte-identical to the bash launcher; the
                  prefix is the one thing the rename moved).
  * SimState   — the ~27 assoc-arrays + ~85 globals as typed dataclasses GROUPED BY KEYING
                  DOMAIN (container vs identity vs seat vs address). The domain discipline is the
                  harness's hardest-won lesson ("house disease #1"); the types make a container
                  key that can never reach a seat map STRUCTURAL, where bash could only enforce it
                  by comment.
  * Orchestrator — ties config+state+actuators+prng+battery, the dispatcher tracks, and the
                  main-loop skeleton. The IMPURE boundaries (docker bring-up, lib.sh staking/gov
                  deploy, cast reads) are explicit seams; see INTEGRATION GAPS in the module tail.
  * LIVE_SEAMS  — the completeness contract for _bind_live_seams (see its comment below).

What USED to live here and no longer does: `draw_round` — the FIXED 4-draw churn lottery, the
replay-exact decision layer the differential test pins — together with `build_gate_inputs` and
`apply_action`, all three moved to `sim/rounds.py` by P3 so that `dispatch` and `orchestrator`
could stop importing each other. They are re-exported here (`from .rounds import ...`, :42) for
the callers and tests that still reach for them on this module.

PORT-NOTE (subshell dry-run, case-soak.sh:2077): the bash selfcheck deliberately runs restore
bookkeeping in a SUBSHELL so its mutations are discarded (a speculative "what would this round
do" probe). Python has no free subshell isolation — self_check() uses copy.deepcopy(state) as a
first-class, tested rollback seam (analysis §3.2).
"""

from __future__ import annotations

import copy
import glob
import hashlib
import json
import os
import signal
import time
import types
from dataclasses import dataclass, field

from . import actions
from .rounds import apply_action, draw_round
from ..core import setops, topology
from ..core.events import EventContext
from ..core.prng import SimPRNG
from ..stack.bringup import StackSpec


def _to_secs(v):
    """to_secs (case-soak.sh:571): "5m"/"90s"/"0" → integer seconds (0 = unbounded)."""
    s = str(v).strip()
    try:
        if s.endswith("m"):
            return int(s[:-1]) * 60
        if s.endswith("s"):
            return int(s[:-1])
        return int(s or 0)
    except ValueError:
        return 0


# ── config (the SIM_* launcher knobs; defaults byte-identical to case-soak.sh) ──

def _envi(name, default):
    v = os.environ.get(name)
    try:
        return int(v) if v is not None and v != "" else default
    except ValueError:
        return default


def _envs(name, default):
    v = os.environ.get(name)
    return v if v is not None and v != "" else default


def _envf(name, default):
    v = os.environ.get(name)
    try:
        return float(v) if v is not None and v != "" else default
    except ValueError:
        return default


@dataclass
class SimConfig:
    """Every knob keeps its bash DEFAULT; only the prefix moved when the harness was renamed to
    `sim`, so a replayed run needs the same VALUES under the new names."""
    quick: int = field(default_factory=lambda: _envi("SIM_QUICK", 0))
    validators: int = field(default_factory=lambda: _envi("SIM_VALIDATORS", 0))
    initial_committee: int = field(default_factory=lambda: _envi("SIM_INITIAL_COMMITTEE", 0))
    spares: int = field(default_factory=lambda: _envi("SIM_SPARES", 2))
    rotation_slots: int = field(default_factory=lambda: _envi("SIM_ROTATION_SLOTS", 2))
    byzantine: int = field(default_factory=lambda: _envi("SIM_BYZANTINE", 1))
    voluntary_exit: int = field(default_factory=lambda: _envi("SIM_VOLUNTARY_EXIT", 0))
    no_cascade: int = field(default_factory=lambda: _envi("SIM_NO_CASCADE", 0))
    calm_fraction: float = field(default_factory=lambda: _envf("SIM_CALM_FRACTION", 0.4))
    membership_settle: int = field(default_factory=lambda: _envi("SIM_MEMBERSHIP_SETTLE", 5))
    growth_gap: int = field(default_factory=lambda: _envi("SIM_GROWTH_GAP", 2))
    bench_join_gap: int = field(default_factory=lambda: _envi("SIM_BENCH_JOIN_GAP", 3))
    grow_land_deadline: int = field(default_factory=lambda: _envi("SIM_GROW_LAND_DEADLINE", 8))
    grow_deferral_credit_max: int = field(
        default_factory=lambda: _envi("SIM_GROW_DEFERRAL_CREDIT_MAX", 6))
    mint_buffer: int = field(default_factory=lambda: _envi("SIM_MINT_BUFFER", 2))
    promote_warm_max_rounds: int = field(
        default_factory=lambda: _envi("SIM_PROMOTE_WARM_MAX_ROUNDS", 6))
    promote_nohost_max_epochs: int = field(
        default_factory=lambda: _envi("SIM_PROMOTE_NOHOST_MAX_EPOCHS", 10))
    ncap_pin_warn_epochs: int = field(
        default_factory=lambda: _envi("SIM_NCAP_PIN_WARN_EPOCHS", 10))
    warm_debt_max_epochs: int = field(
        default_factory=lambda: _envi("SIM_WARM_DEBT_MAX_EPOCHS", 12))
    identity_pool: int = field(default_factory=lambda: _envi("SIM_IDENTITY_POOL", 0))
    force_byzantine_epoch: int = field(
        default_factory=lambda: _envi("SIM_FORCE_BYZANTINE_EPOCH", 0))
    force_byzantine_count: int = field(
        default_factory=lambda: _envi("SIM_FORCE_BYZANTINE_COUNT", 1))
    force_voluntary_exit_epoch: int = field(
        default_factory=lambda: _envi("SIM_FORCE_VOLUNTARY_EXIT_EPOCH", 0))
    force_voluntary_exit_count: int = field(
        default_factory=lambda: _envi("SIM_FORCE_VOLUNTARY_EXIT_COUNT", 1))
    seed: str = field(default_factory=lambda: _envs("SIM_SEED", ""))
    stop_round: int = field(default_factory=lambda: _envi("SIM_STOP_ROUND", 0))
    # ── chain-driven ROTATION (v61 a20/a21) — the phase-2 stake BANDS. The committee is on-chain a
    # stake-weighted top-k, so rotation is a pure STAKE game. To make it CAUSE-DRIVEN (only a drawn
    # departure moves a seat, never the mere act of provisioning the bench) the two bands must be
    # cleanly separated and UNIFORM:
    #   BAND INVARIANT   bench_lo  <  committee_hi
    #   provision        every committee member → committee_hi (Hi);  every warm bench → bench_lo (Lo)
    #   вытеснен (evict) bench_lo + evict_stake  >  committee_hi   → the boosted bench out-ranks the
    #                                                                lowest incumbent (it drops out)
    #   сам вышел (volu) committee_hi - voluntary_undelegate  <  bench_lo → the member drops below the
    #                                                                bench (chain drops it from top-k)
    # Genesis members start at ~1e18 and growth at ~3e18 — those OVERLAP a naive 2e18 bench, so the
    # bench used to displace them at provisioning (the a20 involuntary churn). Lifting every member
    # to a uniform Hi (50e18) well above the bench (5e18) removes that overlap; only the drawn cause
    # then moves the boundary. Defaults verified self-consistent: 5<50; 5+55=60>50; 50-46=4<5.
    committee_hi_wei: str = field(
        default_factory=lambda: _envs("SIM_COMMITTEE_HI_WEI", "50000000000000000000"))     # 50e18
    bench_lo_wei: str = field(
        default_factory=lambda: _envs("SIM_BENCH_LO_WEI", "5000000000000000000"))          # 5e18
    member_fund_wei: str = field(
        default_factory=lambda: _envs("SIM_MEMBER_FUND_WEI", "60000000000000000000"))      # 60e18
    bench_stake_wei: str = field(
        default_factory=lambda: _envs("SIM_BENCH_STAKE_WEI", "1000000000000000000"))       # 1e18 (legacy)
    evict_stake_wei: str = field(
        default_factory=lambda: _envs("SIM_EVICT_STAKE_WEI", "55000000000000000000"))      # 55e18
    voluntary_undelegate_wei: str = field(
        default_factory=lambda: _envs("SIM_VOLUNTARY_UNDELEGATE_WEI", "46000000000000000000"))  # 46e18
    rotation_confirm_epochs: int = field(
        default_factory=lambda: _envi("SIM_ROTATION_CONFIRM_EPOCHS", 6))  # ~E+3 horizon + slack
    # ANCHOR band (v61 a26): validator-0 (pinned host-RPC + spec-derive telemetry source) and
    # validator-1 (pinned cert-upstream) must NEVER be rotated out — a25's eviction resolves an
    # EMERGENT victim (the chain drops the weakest incumbent) and rekeyed whichever member left,
    # INCLUDING an anchor. Rekeying validator-0 REBIRTHS its container (kind=Restart) → the fresh
    # node rederives 100% while catching up → the spec-head-lag invariant (which sampled v0) false-
    # fired, and the host RPC blipped. Keep the anchors above the eviction reach (bench_lo+evict_stake
    # = 60e18) so the chain's "weakest incumbent" is always a NON-anchor: anchor_stake > 60e18.
    anchor_stake_wei: str = field(
        default_factory=lambda: _envs("SIM_ANCHOR_STAKE_WEI", "70000000000000000000"))     # 70e18 > 60e18
    anchor_fund_wei: str = field(
        default_factory=lambda: _envs("SIM_ANCHOR_FUND_WEI", "90000000000000000000"))       # 70 stake + gas

    def __post_init__(self):
        # ── SIM_QUICK profile overlay (case-soak.sh:57-79, 138-142) ─────────────
        # validators / initial_committee / identity_pool have PROFILE-DEPENDENT
        # defaults in the bash (set INSIDE the quick/full branches), not flat
        # constants. Porting validators as a flat 14 (the bash VAL_CONTAINERS
        # default — a validators↔val_containers confusion) drove val_containers to
        # 18, so compose_gen emitted phantom containers validator-14..17; the v61.9
        # node-down check then fired on the container-less bench idx validator-16.
        # 0 = "operator did not set the env var" sentinel (mirrors identity_pool).
        if self.validators == 0:
            self.validators = 4 if self.quick else 10
        if self.initial_committee == 0:
            self.initial_committee = 4 if self.quick else 7
        # Derived quantities (byte-identical arithmetic to the bash header).
        self.spare_end = self.validators + self.spares               # first idx PAST the spare band
        self.val_containers = self.validators + self.spares + self.rotation_slots
        # identity_pool derives from val_containers (NOT == it): full +6, quick +2.
        if self.identity_pool == 0:
            self.identity_pool = self.val_containers + (2 if self.quick else 6)
        self.initial_f = (self.initial_committee - 1) // 3
        self.min_committee = 3 * self.initial_f + 1
        if not self.seed:
            # Fresh-seed default = bash `sha256("$(date +%s)-$$") | cut -c1-16` (F18): the epoch
            # SECONDS and the pid, hyphen-joined, first 16 hex chars. pid alone (the old recipe) is
            # near-constant across quick relaunches — two runs a second apart could collide.
            entropy = f"{int(__import__('time').time())}-{os.getpid()}"
            self.seed = hashlib.sha256(entropy.encode()).hexdigest()[:16]

    def stack_spec(self) -> StackSpec:
        """The sim's projection onto the stack layer's explicit contract. `stack/bringup` used to
        be handed this whole object and duck-type seven attributes off it; the conversion lives
        HERE, on the sim side, so `stack/` needs no knowledge of SimConfig at all — that is the
        direction of the dependency, and a classmethod on StackSpec reading a `cfg` would just
        move the duck-typing down a layer."""
        return StackSpec(
            validators=self.validators,
            val_containers=self.val_containers,
            initial_committee=self.initial_committee,
            identity_pool=self.identity_pool,
            no_cascade=self.no_cascade,
            byzantine=self.byzantine,
        )

    def actions_pool(self):
        """ACTIONS (case-soak.sh) — base FIVE, +byzantine pair iff SIM_BYZANTINE, +voluntary_exit
        iff SIM_VOLUNTARY_EXIT. The append ORDER and the pool SIZE are load-bearing: they fix the
        modulus the 2nd draw reduces against, so this list must stay byte-identical to the bash
        ACTIONS array or the two stop replaying the same schedule. The base pool dropped from six
        to five when `liveness_jail` was deleted — pre-deletion seeds do NOT replay against this
        pool, by design (that sixth slot could only log a skip). SIM_ACTIONS override (B' §9 item
        4 rolling restart) honored when set."""
        override = os.environ.get("SIM_ACTIONS")
        if override:
            return override.split()
        acts = ["graceful_stop_restart", "sigkill_restart", "cpu_throttle",
                "dkg_midwindow_restart", "delegate_shift"]
        if self.byzantine == 1:
            acts += ["byzantine_equivocate", "byzantine_forge_pk"]
        if self.voluntary_exit == 1:
            acts += ["voluntary_exit"]
        return acts


# ── the domain-typed state machine (the reviewed-hardest artifact) ────────────
#
# The ~27 declare -A + ~85 globals partition into FOUR keying domains the bash comments treat as
# a load-bearing invariant (mixing them is "house disease #1", documented at EVER_FAULTED):
#
#   CONTAINER domain  — a fact about the docker process/lifecycle ("this container is down"). A
#                       container is RECYCLED and re-tasked to new identities, so these must NOT
#                       survive a re-task with their old meaning.
#   IDENTITY domain   — an on-chain fact about a logical validator identity (idx). Survives
#                       container recycling: a tombstoned identity stays jailed on-chain forever
#                       even after its container is re-tasked onto a fresh mint.
#   SEAT domain       — a committee-seat obligation. Belongs to the IDENTITY that VACATED the
#                       seat (resolved via ident_idx at ENQUEUE time and frozen), never to the
#                       container that happened to host it — the container-aliasing defect class
#                       (three proven multi-hour failures) is killed by keying on the seat.
#   ADDRESS domain    — lowercased owner-address maps.
#
# Grouping the maps into typed sub-dataclasses makes a container key reaching a seat map a type
# error the reviewer can see, where bash could only warn in a comment.


@dataclass
class ContainerState:
    """CONTAINER domain — keyed by service name ("validator-12"). Docker lifecycle facts."""
    disrupt_kind: dict = field(default_factory=dict)      # container -> action kind (restore)
    restore_at: dict = field(default_factory=dict)        # container -> epoch the restore issues
    recovering: dict = field(default_factory=dict)        # container -> ts the restore issued
    recover_over_ticks: dict = field(default_factory=dict)
    recover_leadfin: dict = field(default_factory=dict)
    stop_fin: dict = field(default_factory=dict)          # container -> own finalized before stop
    warm_debt: dict = field(default_factory=dict)         # container -> 1 (restarted, not re-warm)
    warm_debt_base: dict = field(default_factory=dict)    # container -> beacon_seed_active base
    warm_debt_since: dict = field(default_factory=dict)   # container -> epoch its debt began
    warm_debt_over_ticks: dict = field(default_factory=dict)
    self_partitioned: dict = field(default_factory=dict)  # container -> 1 (Bug-A self-partition)
    self_partition_since: dict = field(default_factory=dict)
    self_partition_over_ticks: dict = field(default_factory=dict)
    seed_active_prev: dict = field(default_factory=dict)  # container -> last beacon_seed_active
    seed_flat_ticks: dict = field(default_factory=dict)
    slot_identity: dict = field(default_factory=dict)     # container -> hosted identity idx


@dataclass
class IdentityState:
    """IDENTITY domain — keyed by logical on-chain identity idx. Survives container recycling."""
    ever_faulted: dict = field(default_factory=dict)      # idx -> action (MONOTONIC, never cleared)
    permanently_dead: dict = field(default_factory=dict)  # container -> 1 (burned tombstone)
    tombstone_settle_epoch: dict = field(default_factory=dict)
    promote_warm_tries: dict = field(default_factory=dict)  # idx -> consecutive warming rounds
    self_partition_deferrals: dict = field(default_factory=dict)
    self_partition_warn_next: dict = field(default_factory=dict)
    dkg_barrier_base: dict = field(default_factory=dict)  # incoming idx -> dkg_ceremony_ok base


@dataclass
class SeatState:
    """SEAT domain — keyed by the identity idx that VACATED a committee seat (frozen at enqueue).
    SEAT_CONTAINER is the ONLY bridge back to the container domain, SEVERED at recycle."""
    pending_backfill_by_seat: dict = field(default_factory=dict)  # seat -> epoch enqueued
    seat_container: dict = field(default_factory=dict)            # seat -> hosting container
    backfill_pause_credit: dict = field(default_factory=dict)
    backfill_pause_laste: dict = field(default_factory=dict)


@dataclass
class AddressState:
    """ADDRESS domain — lowercased owner-address maps."""
    addr2idx: dict = field(default_factory=dict)          # addr -> container ("validator-N")
    owner_addr_cache: dict = field(default_factory=dict)  # idx -> lowercased addr
    ever_members: dict = field(default_factory=dict)      # addr -> 1 (monotonic)


@dataclass
class SimState:
    """The full orchestrator state. Sub-dataclasses partition the maps by keying domain; the
    scalar globals live at the top. `prng` carries the replay stream. Everything defaults to its
    bash `${VAR:-...}` value so an unset field behaves as an unset global.

    Exposes ident_idx / mark_disrupted / unmark_disrupted / add_pending — the small mutators the
    dispatch and the seat helpers call, each a 1:1 port of the case-soak.sh helper."""
    cfg: SimConfig = field(default_factory=SimConfig)
    prng: SimPRNG = None

    container: ContainerState = field(default_factory=ContainerState)
    identity: IdentityState = field(default_factory=IdentityState)
    seat: SeatState = field(default_factory=SeatState)
    address: AddressState = field(default_factory=AddressState)

    # scalar globals (case-soak.sh:392-466)
    disrupted: str = ""                # SIM_DISRUPTED (space-set, container domain)
    cur_committee: str = ""            # SIM_CUR_COMMITTEE (space-set of owner addrs)
    cur_epoch: int = 0
    cur_f: int = 1
    pending: str = ""                  # SIM_PENDING (space-set "kind@epoch")
    shareless: int = 0
    round: int = 0
    tick: int = 0
    expected_stall: int = 0
    effective_faults: str = ""         # EFFECTIVE_FAULTS (container-domain union)
    next_joiner: int = 0
    grow_landed: int = -1
    grow_fire_epoch: int = 0
    next_bench_joiner: int = 0
    promotable: str = ""               # unified promotable pool (space-set of idxs)
    promote_cand: str = ""             # select_promote_candidate output (this round's pick)
    promote_cand_kind: str = ""        # "ready" | "warm" | ""
    promote_pending: str = ""          # in-flight promote guard (bench idx)
    promote_share_base: str = ""
    promote_land_epoch: str = ""
    promote_over_ticks: int = 0
    refill_pending: str = ""
    refill_resumed: str = ""
    refill_share_base: str = ""
    refill_land_epoch: str = ""
    refill_over_ticks: int = 0
    last_bench_join_round: int = 0
    settle_until_epoch: int = 0
    next_growth_epoch: int = 3
    tombstones: int = 0
    voluntary_exits: int = 0
    max_minted_idx: int = 0
    committee_read_ok: int = 1
    battery_coverage: int = None       # SIM_BATTERY_COVERAGE; set each tick by compute_self_partitioned
    dkg_window_fragile: int = 0
    promote_nohost_since_epoch: int = 0
    ncap_pin_since_epoch: int = 0
    ncap_pin_lastwarn_e: int = 0
    last_growth_kind: str = ""         # "growth" | "refill"
    # ── chain-driven ROTATION phase (v61 a20) ──
    rotation_phase: int = 0            # LATCH: 1 once growth reached full cap → phase-2 rotation
    bench_provisioned: int = 0         # LATCH: warm bench registered+activated+staked-low once
    provision_cursor: int = 0          # BELT cursor: 0..validators-1 = committee stratify step,
                                       # validators..val_containers = bench-activate step (one/round)
    next_rekey_idx: int = 0            # cursor into the fresh-key supply band (val_containers..pool)
    # rekey_pending — ROTATION domain: container -> {"epoch": marked, "cause": str, "idx": departed
    # identity idx}. A permanent departure (byzantine / voluntary / stake-evict) marks its container
    # here; process_rotations rekeys it under a FRESH pool key ONCE it has actually left committee.
    rekey_pending: dict = field(default_factory=dict)
    # evict_pending — ROTATION domain (FIX-3): bench_idx(str) -> {"epoch", "bench_idx", "snapshot":
    # [seated owner addrs at apply], "addr_idx": {addr: identity idx}}. A stake-eviction boosts a
    # bench and records the committee snapshot; process_evictions resolves the EMERGENT evicted
    # member on landing (the chain drops the weakest, not a drawn victim) and rekeys ITS container.
    evict_pending: dict = field(default_factory=dict)

    def __post_init__(self):
        if self.prng is None:
            self.prng = SimPRNG(self.cfg.seed)
        self.next_joiner = self.cfg.initial_committee
        self.grow_landed = self.cfg.initial_committee - 1
        self.next_bench_joiner = self.cfg.val_containers
        self.next_rekey_idx = self.cfg.val_containers   # fresh-key supply for rotation rekeys
        self.max_minted_idx = self.cfg.identity_pool - 1
        self.cur_f = self.cfg.initial_f

    def event_context(self) -> EventContext:
        """The sim's projection onto `core/events`. Passed to `EventLog` as a CALLABLE (not a
        value): every field below moves each round, and the log re-reads it per event exactly as
        the bash re-read its globals. Living here rather than in `core/` is what lets the event
        log be used by a consumer that has no SimState at all."""
        return EventContext(
            seed=self.cfg.seed,
            round=self.round,
            epoch=self.cur_epoch,
            f=self.cur_f,
            committee=self.cur_committee,
            disrupted=self.disrupted,
            pending=self.pending,
        )

    # --- domain bridges / mutators (1:1 with case-soak.sh / soak-actions.sh helpers) ---
    def ident_idx(self, container: str):
        """_ident_idx: container -> LOGICAL on-chain identity idx. A reborn container serves
        SLOT_IDENTITY[c]; a native one serves its own container-suffix idx. For a native it is
        the container idx → the proven native path is byte-identical.

        ALWAYS returns a STRING — the SINGLE normalization point for the identity-idx domain
        (NEW-1). EVER_FAULTED, OWNER_ADDR_CACHE, and _wrongslash_idx_of are all str-keyed (bash
        array subscripts are strings); a bare int from SLOT_IDENTITY here would miss the cache on
        every reborn identity and false-fire wrongful-slash on a legitimately-faulted rebirth."""
        if container in self.container.slot_identity:
            return str(self.container.slot_identity[container])
        return container.rsplit("-", 1)[-1]

    def mark_disrupted(self, container: str):
        if not setops.in_set(container, self.disrupted):
            self.disrupted = (self.disrupted + " " + container).strip()

    def unmark_disrupted(self, container: str):
        self.disrupted = setops.set_remove(container, self.disrupted).strip()

    def add_pending(self, entry: str):
        """add_pending: append "kind@epoch" to SIM_PENDING (space-set)."""
        self.pending = (self.pending + " " + entry).strip()


def _sweep_generated_overlays() -> None:
    """On-exit sweep of the per-run generated compose overlays (case-soak.sh:515-519).

    Two generators leak files into the smoke dir: act_byzantine writes
    `docker-compose.sim.byz-<victim>.gen.yml` (a hardcoded FLUENT_DPOS_BYZANTINE) and
    hostpool.rebirth_identity writes `docker-compose.sim.reborn-<slot>.gen.yml` (a hardcoded
    IDENTITY_IDX). Bash removed both on EVERY exit; the Python port never did, so they
    accumulated across runs — a latent contamination surface for the next run and, worse, a
    forensic decoy (a stale byz overlay on disk says nothing about whether THIS run had a
    byzantine victim). Unconditional, exactly like bash: it runs even under SIM_KEEP_UP,
    because a container that is already up has its config applied and is unaffected.
    Best-effort — a teardown must never raise over a file."""
    for pattern in ("docker-compose.sim.byz-*.gen.yml",
                    "docker-compose.sim.reborn-*.gen.yml"):
        for stale in glob.glob(pattern):
            try:
                os.unlink(stale)
            except OSError:
                pass


# ── the LIVE-SEAM REGISTRY (the completeness contract for _bind_live_seams) ───
#
# Every row is an IMPURE READ seam that is initialised to a CONSTANT STUB at construction
# (-1 / 0 / "" / `lambda c: False`) so the docker-free unit suite runs without a topology, and that
# a LIVE run MUST rebind to a real nodes.py reader.
#
# Why it is declared, and declared HERE: the failure this exists to prevent is a seam that exists,
# is never bound, and reports nothing. v61 a15 wired `hp._beacon` and forgot `hp._confirm`, so
# sim_warm_ready returned False for nine hours — the committee could not backfill a single
# vacancy and NOTHING errored. Before this registry the only "list" of seams was a hand-written one
# inside a test that mirrored the binder, so a seam added to a constructor and forgotten in the
# binder produced no failure anywhere.
#
# The rows are STRINGS, not classes, on purpose. The reason recorded here used to be that
# dispatch.py imported this module at import time so Dispatcher could not be imported back — that
# has not been true since P3 (dispatch imports actions/rounds/setops/writes and NOT orchestrator).
# The reason that IS true: orchestrator imports all three of these classes LAZILY, inside the
# functions that construct them (HostPool/Reconcilers at :535-536, Dispatcher at :748 and :780),
# so no class object exists at module scope where this registry is declared. Naming them as
# strings keeps the registry a plain declaration of intent that the test resolves on its own,
# instead of a second place that has to import whatever the binder imports. The test
# resolves them and — crucially — cross-checks this list against the constructors themselves
# (test_live_seams.py::test_live_seam_registry_covers_every_construction_stub), so an EIGHTH stub
# added tomorrow and forgotten here fails too. Do not turn either side into a hand-copied mirror
# of the other.
#
#   (binder parameter, module, class, attribute)
LIVE_SEAMS = (
    ("hp",   "hostpool",    "HostPool",    "_confirm"),
    ("hp",   "hostpool",    "HostPool",    "_beacon"),
    ("rec",  "reconcilers", "Reconcilers", "beacon_share_delta"),
    ("rec",  "reconcilers", "Reconcilers", "beacon_seed_delta"),
    ("disp", "dispatch",    "Dispatcher",  "beacon_metric"),
    ("disp", "dispatch",    "Dispatcher",  "refill_spare_attempts"),
    ("disp", "dispatch",    "Dispatcher",  "top_stake_leader"),
)

# The ONLY seams allowed to still hold their construction stub after _bind_live_seams.
# top_stake_leader() is INTENTIONALLY INERT in bash too (case-soak.sh:555) — gate rule 6 tolerates
# "" by design — so the faithful live binding IS the construction default. Anything else turning up
# in the residual set is a forgotten binding, which is the nine-hour wedge.
INERT_SEAMS = frozenset({"top_stake_leader"})


# ── the Orchestrator (ties it together; main loop skeleton) ───────────────────

class Orchestrator:
    """Ties config+state+actuators+prng+battery and drives the churn loop. The pure DECISION
    layer (draw_round / build_gate_inputs / apply_action / the dispatcher-track predicates) is
    fully ported and testable; the IMPURE bring-up + live reads are seams (see INTEGRATION GAPS).
    """

    def __init__(self, cfg: SimConfig = None, actuators=None, dry_run: bool = False):
        self.cfg = cfg or SimConfig()
        self.state = SimState(cfg=self.cfg)
        self.act = actuators or actions.Actuators(env=self._compose_env(), dry_run=dry_run)
        self.calm_permille = int(self.cfg.calm_fraction * 1000)
        self.rt_addrs = {}     # post-deploy contract addresses; populated + checked in _build_chain

    def _compose_env(self):
        return {
            "RPC": os.environ.get("RPC", topology.DEFAULT_RPC_URL),
            "COMPOSE_FILE": os.environ.get("COMPOSE_FILE", ""),
            "STAKING_RT": os.environ.get("STAKING_RT", ""),
            "TOKEN": os.environ.get("TOKEN", ""),
            "GOV_ADDR": os.environ.get("GOV_ADDR", ""),
            "CHAIN_ID": os.environ.get("CHAIN_ID", str(topology.CHAIN_ID)),
        }

    def self_check(self):
        """Speculative dry-run selfcheck (case-soak.sh:2077 subshell probe). Runs the decision +
        dispatch over a DEEP COPY of the state so mutations are discarded — the explicit, tested
        rollback seam that replaces bash's free subshell isolation (analysis §3.2)."""
        from ..chain.writes import Chain
        probe = copy.deepcopy(self.state)
        dry = actions.Actuators(env=self._compose_env(), dry_run=True)
        # A DRY Chain for the two chain-writing arms (delegate_shift / voluntary_exit). This
        # probe has no live chain — it runs before/independently of _build_chain — and
        # apply_action's `chain` is required, so the probe must supply one. Chain(dry=True)
        # builds Runner(dry=True) (chain/writes.py:119): every write records its argv and returns a
        # canned value, and every wait/assert short-circuits on p.dry (gov_action:442,
        # _gov_wait_succeeded:481, _status_assert:743, ChainPaced.wait:122) — so the probe walks
        # both arms end to end without touching the chain or sleeping.
        dry_chain = Chain(runner=self._runner(dry=True))
        pool = self.cfg.actions_pool()
        # exercise every action arm's dispatch (arg-contract flush, like sim_selfcheck).
        for a in pool:
            apply_action(probe, dry, a, topology.validator(2), probe.cur_epoch, chain=dry_chain)
        return True   # the deep copy is discarded — no state mutated

    # -- the differential-testable round decision (docker-free) --
    def decide_round(self, cur_epoch: int, actions_pool=None):
        """One round's DRAW + victim-pool + calm/forced decision, docker-free. This is exactly
        what the differential replay test drives side-by-side with the bash PRNG. Builds the
        victim pool from the CURRENT committee/ADDR2IDX/disrupted (committee_victim_idxs, no
        PRNG), then draw_round (the sacred 4 draws)."""
        pool = actions_pool or self.cfg.actions_pool()
        vpool = setops.committee_victim_idxs(
            self.state.cur_committee, self.state.address.addr2idx, self.state.disrupted)
        return draw_round(self.state, pool, vpool, cur_epoch, self.calm_permille)

    # -- WRITE-layer wiring (bring-up + per-tick reconcilers + the adapter) -----
    def _runner(self, dry=False, echo=False):
        from ..core.proc import Runner
        return Runner(env=self._compose_env(), dry=dry, echo=echo)

    def _build_chain(self, runner, bu):
        from ..chain.writes import Chain, ChainError
        chain = Chain(runner=runner, RPC=os.environ.get("RPC", topology.DEFAULT_RPC_URL),
                      STAKING_RT=bu.staking_rt, CHAIN_CONFIG_RT=bu.chain_config_rt,
                      GOV_ADDR=bu.gov_addr, LIVENESS_RT=bu.liveness_rt, TOKEN=bu.token,
                      CHAIN_ID=os.environ.get("CHAIN_ID", str(topology.CHAIN_ID)))
        # The post-deploy addresses enter the harness HERE and nowhere else, so this is where
        # they are checked. They are also what run() feeds the battery ctx each tick: the sim
        # deploys the staking cluster at run time, and a battery reading the genesis predeploys
        # reads CODELESS addresses — every on-chain detector then evaluates over nothing and
        # reports "holds". An empty field means bring-up never produced the fact; abort now
        # rather than run a sim whose invariant battery cannot see the chain.
        self.rt_addrs = {"STAKING_RT": chain.staking_rt,
                         "CHAIN_CONFIG_RT": chain.chain_config_rt,
                         "LIVENESS_RT": chain.liveness_rt}
        blank = sorted(k for k, v in self.rt_addrs.items() if not v)
        if blank:
            raise ChainError("selfcheck-no-runtime-addrs",
                             f"runtime contract address(es) {', '.join(blank)} empty after "
                             "bring-up — refusing a run whose invariant battery would read the "
                             "codeless predeploys and report every on-chain detector as holding")
        return chain

    def _reconcilers(self, chain):
        from .hostpool import HostPool
        from .reconcilers import Reconcilers
        hp = HostPool(self.state, chain, chain.p,
                      confirm=lambda c: False, beacon=lambda idx: -1)
        rec = Reconcilers(self.state, chain, self.act, hostpool=hp)
        return rec, hp

    def to_ctx(self, **overrides):
        """Build the (battery.Ctx, battery.SimCtx) pair for THIS tick (the SimState adapter)."""
        from . import state_ctx
        return state_ctx.to_ctx(self.state, **overrides)

    @staticmethod
    def _battery_knobs(battery):
        """The battery's effective cadence/threshold knobs, for the run-config record.

        Read OFF THE LIVE INSTANCE rather than copied into a list here: a hand-written mirror of
        battery.py's ~40 knobs is exactly the kind of list that silently stops matching (the
        _bind_live_seams lesson). Battery.__init__ sets the knobs and then calls _reset_rolling
        for the per-run rolling state, so "uppercase attribute _reset_rolling does not write" IS
        the knob set. Depends on _reset_rolling staying assignment-only — it reads no attribute
        today, and this call would raise at once if that changed. A stand-in battery with no
        _reset_rolling (unit-test doubles) simply has no rolling state to subtract.
        The env-sourced Ctx fields (battery.py:100-105) are knobs too and are carried alongside;
        an empty SIM_DATA_ROOT / L3_SPAMMER_ADDR is precisely the "why did that detector never
        run" question this record exists to answer.
        """
        rolling = set()
        reset = getattr(type(battery), "_reset_rolling", None)
        if reset is not None:
            probe = types.SimpleNamespace()
            reset(probe)
            rolling = set(vars(probe))
        knobs = {k: v for k, v in vars(battery).items() if k.isupper() and k not in rolling}
        ctx = getattr(battery, "ctx", None)
        for name in ("L3_SPAMMER_ADDR", "SIM_DATA_ROOT", "SIM_PEER_SPOKE"):
            knobs[name] = getattr(ctx, name, None)
        return knobs

    # -- GAP-1: bind the beacon/metric READ seams to the live nodes.py exec readers ------------
    def _bind_live_seams(self, disp, rec, hp):
        """Bind the dispatcher / reconciler / host-pool beacon+metric seams to live nodes.py exec
        readers. Before this the seams return the -1/0/"" construction defaults (docker-free unit
        tests keep overriding them AFTER construction, unchanged); a live run wires them here so the
        watchdogs read real container metrics. Seam→bash map:
          rec.beacon_share_delta  ← sim_beacon_share_delta (dkg_ceremony_ok, :9100)  → dispatch
                                     inherits it (disp.beacon_share_delta delegates to rec)
          rec.beacon_seed_delta   ← sim_beacon_seed_delta  (beacon_seed_active, :9100)
          hp._beacon              ← sim_beacon_share_delta  (sim_warm_ready's beacon-plane gate)
          hp._confirm             ← rec.recovery_confirmed   (sim_warm_ready's EL-frontier gate)
          disp.beacon_metric      ← sim_beacon_metric       (arbitrary family, :9100)
          disp.refill_spare_attempts ← _refill_spare_attempts (reth_dpos_derive_db_timeout_attempts
                                        _total, :9200)
        disp.top_stake_leader stays "" — top_stake_leader() is INTENTIONALLY INERT in bash
        (case-soak.sh:555; gate rule 6 tolerates "" by design), so the faithful live binding is the
        same inert "" the construction default already returns."""
        from ..core import nodes
        s = self.state
        vc = self.cfg.val_containers

        def _svc(idx):
            return nodes.beacon_service_for_idx(idx, vc, s.container.slot_identity)

        def share_delta(idx):
            sv = _svc(idx)
            return nodes.beacon_metric(sv, "dkg_ceremony_ok") if sv else -1

        def seed_delta(idx):
            sv = _svc(idx)
            return nodes.beacon_metric(sv, "beacon_seed_active") if sv else -1

        def beacon_metric_seam(idx, family):
            sv = _svc(idx)
            return nodes.beacon_metric(sv, family) if sv else -1

        rec.beacon_share_delta = share_delta
        rec.beacon_seed_delta = seed_delta
        hp._beacon = share_delta
        hp._confirm = rec.recovery_confirmed   # WITHOUT this, hp._confirm stays the construction
        # `lambda c: False` stub → sim_warm_ready ALWAYS returns False in a live run → bench-promote
        # never fires → the committee can only grow via native register_activate, never past the
        # native band. (v61 a15: the beacon seam was wired here but the confirm seam was forgotten;
        # unit tests inject confirm= so they never caught it.)
        disp.beacon_metric = beacon_metric_seam
        disp.refill_spare_attempts = lambda v: nodes.refill_spare_attempts(v)
        # disp.top_stake_leader: left inert ("") — see docstring.

    # -- GAP-2a: production-seam liveness self-check (soak-invariants.sh::sim_selfcheck_result_seams)
    def selfcheck_result_seams(self, fin, node_fin_in=None, node_roots_of=None):
        """sim_selfcheck_result_seams (soak-invariants.sh:1410): assert the REAL result-agreement
        READ seams (_node_fin_in / _node_roots_of) return a real (finalized-tag, stateRoot/
        receiptsRoot) pair on >= f+1 committee nodes at a K-lagged height. Returns (ok, msg). The
        node readers are injectable so a test drives both directions without a live topology."""
        from ..core import nodes
        node_fin_in = node_fin_in or nodes.node_fin_in
        node_roots_of = node_roots_of or nodes.node_roots_of
        k = int(os.environ.get("RESULT_LAG_K", "3"))
        s = self.state
        h = fin - k
        if h <= 0:
            self._seam_err_id = "selfcheck-seams-dead"
            return (False, f"self-check called at fin={fin} (<= K={k}) — no K-lagged finalized "
                           "height to probe yet; call it once the chain is past K")
        members = (s.cur_committee or "").split()
        resolved = 0          # committee owner-addrs that mapped to a container via ADDR2IDX
        live = 0
        tried = 0
        detail = ""
        for a in members:
            ss = s.address.addr2idx.get(a.lower())
            if not ss:
                continue
            resolved += 1
            if setops.in_set(ss, s.disrupted) or ss in s.identity.permanently_dead:
                continue
            tried += 1
            ceil = fin if topology.is_pinned_rpc_host(ss) else node_fin_in(ss)
            roots = node_roots_of(ss, h)
            detail += f" {ss}(fin={ceil},roots=[{roots or '<empty>'}])"
            if roots and not str(roots).startswith("null") and str(ceil) != "-1":
                live += 1
        # (a) EMPTY CANDIDATE LIST — a HARNESS resolution failure, NOT dead seams. Bash never runs
        # this loop on an empty ADDR2IDX: build_addr_map (case-soak.sh:2133) populates it right after
        # sim_bring_up, before the self-check resolves committee addresses through it. If NO committee
        # owner-address resolves (0-of-0), the fault is our resolution layer (build_addr_map skipped /
        # ADDR2IDX unpopulated / committee read from a pre-DPoS source), so name it honestly instead
        # of blaming the on-chain _node_roots_of/_node_fin_in seams for "seams DEAD".
        if resolved == 0:
            self._seam_err_id = "selfcheck-resolution-empty"
            return (False, f"result-agreement candidate list EMPTY at height {h}: 0 of "
                           f"{len(members)} committee owner-address(es) resolved through ADDR2IDX — "
                           "a HARNESS RESOLUTION failure (build_addr_map not run / ADDR2IDX "
                           "unpopulated / committee read from a pre-DPoS source), NOT the "
                           "result-agreement seams being dead. Populate ADDR2IDX (build_addr_map) "
                           "from a live committee read before self-checking")
        floor = s.cur_f + 1
        if live >= floor:
            self._seam_err_id = ""
            return (True, f"result-agreement seams LIVE at height {h}: {live} of {tried} committee "
                          f"node(s) returned a real pair (floor f+1={floor})")
        self._seam_err_id = "selfcheck-seams-dead"
        return (False, f"result-agreement seams DEAD at height {h}: only {live} of {tried} committee "
                       f"node(s) returned a real (finalized-tag, roots) pair, below the f+1={floor} "
                       f"floor —{detail}. _node_roots_of/_node_fin_in are not reading the containers. "
                       "Refusing to start a run with no independent result-divergence detection")

    def selfcheck_result_seams_or_abort(self, finalized_dec=None, selfcheck=None, sleep=None):
        """selfcheck_result_seams_or_abort (case-soak.sh:1668): 3 bounded retries (10s apart) over
        the real seam self-check — distinguishes a post-bring-up brown-out from a genuinely dead
        seam. Raises ChainError on 3 failures (the bash `exit 1` — the first fault must never be
        applied on an untrusted battery). Seams injectable for the pass/abort unit tests."""
        from ..core import nodes
        from ..chain.writes import ChainError
        finalized_dec = finalized_dec or nodes.finalized_dec
        selfcheck = selfcheck or self.selfcheck_result_seams
        sleep = sleep if sleep is not None else __import__("time").sleep
        msg = ""
        for attempt in (1, 2, 3):
            ok, msg = selfcheck(finalized_dec())
            if ok:
                return
            if attempt < 3:
                sleep(10)
        # The failure class the last self-check attempt stamped: "selfcheck-resolution-empty" (the
        # 0-of-0 harness resolution failure) vs the default dead-seams verdict. Injected self-checks
        # (the unit tests) never stamp it, so they keep the historical "selfcheck-seams-dead" id.
        raise ChainError(getattr(self, "_seam_err_id", "") or "selfcheck-seams-dead", msg)

    def dry_run_bringup(self):
        """--dry-run-bringup: print the FULL bring-up command sequence (every subprocess argv,
        in order, with the env delta) WITHOUT executing — diff it against the known-good bash
        bring-up offline."""
        from ..stack.bringup import BringUp
        runner = self._runner(dry=True, echo=True)
        print(f"# dpos_harness dry-run-bringup (seed={self.cfg.seed}, N={self.cfg.val_containers})")
        BringUp(self.cfg.stack_spec(), runner).run()
        print(f"# {len(runner.log)} commands")
        return 0

    def dry_run_tick(self):
        """--dry-run-tick: print one synthetic tick's would-be commands given a canned state (the
        reconcilers' docker reads + the round decision), WITHOUT executing."""
        from ..stack.bringup import BringUp
        runner = self._runner(dry=True, echo=True)
        bu = BringUp(self.cfg.stack_spec(), runner)
        bu.run()                                   # populate the post-deploy facts (dry)
        chain = self._build_chain(runner, bu)
        self.act = actions.Actuators(env=self._compose_env(), dry_run=True)  # no live docker
        rec, hp = self._reconcilers(chain)
        # stub the reconciler's own RPC/docker READ seams so a dry tick touches NOTHING live
        # (the chain WRITE reads already go through the dry Runner; these bypass it).
        rec.finalized_dec = lambda: 100
        rec.node_finalized = lambda v: 100
        rec.recovery_confirmed = lambda v: True
        rec.blockhash_of = lambda v, h: "0xabc"
        rec.beacon_share_delta = lambda i: 5
        rec.beacon_seed_delta = lambda i: 5
        # canned reads so owner_addr / committee resolve, and a STEADY-STATE (growth complete,
        # nothing in flight) so the tick exercises the fault-LOTTERY path (the interesting churn).
        addrs = [f"0x{'%040x' % (0xA0 + i)}" for i in range(self.cfg.initial_committee)]
        runner.reads = {
            "docker compose exec": "aa" * 32,
            "cast wallet": addrs[0],
            "cast call": " ".join(addrs),      # getEpochCommittee → the canned committee
        }
        self.state.cur_committee = " ".join(addrs)
        self.state.address.addr2idx = {a: topology.validator(i) for i, a in enumerate(addrs)}
        self.state.cur_epoch = 5
        self.state.next_joiner = self.cfg.validators           # growth complete
        self.state.grow_landed = self.cfg.validators - 1
        self.state.next_bench_joiner = 10 ** 9                 # bench supply idle
        self.state.settle_until_epoch = 0
        from .dispatch import Dispatcher
        disp = Dispatcher(self.state, chain, self.act, hp, rec)
        # stub the dispatch's own beacon/metric seams so a dry tick touches nothing live.
        disp.beacon_metric = lambda i, f: 0
        disp.refill_spare_attempts = lambda v: 0
        disp.top_stake_leader = lambda: ""
        print("# --- one synthetic tick (reconcile → full round dispatch) ---")
        runner.log.clear()
        cur = self.state.cur_epoch
        try:
            rec.compute_effective_faults(cur)
            rec.compute_dkg_barrier(cur)
            rec.process_restores(cur)
            rec.process_tombstone_backfills(cur)
            n = len(self.state.cur_committee.split()) or self.cfg.initial_committee
            verdict = disp.run_round(cur, n)
        except Exception as e:                     # a watchdog trip in a canned tick is expected
            print(f"# (dispatch raised on canned state: {e})")
            verdict = "raised"
        print(f"# round verdict: {verdict}")
        for kind, msg in disp.events[-4:]:
            print(f"#   event[{kind}] {msg[:100]}")
        print(f"# {len(runner.log)} tick commands")
        return 0

    def run(self):
        """The live sim (case-soak.sh:2170-3121): bring-up → self-check → the per-tick loop
        (committee-read visibility → reconcilers → check_resource_pools → the PERSISTENT invariant
        battery → churn cadence → CHECK_SECS sleep) under SIGINT/SIGTERM traps + a fail-bundle
        teardown honoring SIM_KEEP_UP. IMPURE (docker/cast). The decision + state layer is fully
        ported + unit-tested; this method wires the ported WRITE seams around it."""
        from ..stack.bringup import BringUp
        from .dispatch import Dispatcher
        from ..core.events import EventLog
        from ..checks.battery import Battery
        from ..chain.writes import ChainError
        from ..core import nodes

        events = []                                   # ONE sink: rec/disp/battery/act append here
        # The actuators are built in __init__, before this sink exists, so the binding is here —
        # earliest possible point, since act's graceful-stop barriers (actions._shutdown_flushed /
        # _marshal_ack_tripwire) report through it and run inside the churn round.
        self.act.events = events
        elog = EventLog(context=self.state.event_context)
        runner = self._runner()
        bu = BringUp(self.cfg.stack_spec(), runner)

        interrupted = {"v": False}

        def _on_signal(signum, _frame):               # F5: SIGINT/SIGTERM (case-soak.sh:526-533)
            interrupted["v"] = True
            raise KeyboardInterrupt
        prev_int = signal.signal(signal.SIGINT, _on_signal)
        prev_term = signal.signal(signal.SIGTERM, _on_signal)

        def _drain():                                 # F5: route events into events.jsonl each tick
            for kind, note in events:
                elog.sim_event(kind, note)
            events.clear()

        def _teardown():
            try:
                bu.spammers.stop()                    # F11: never leak the tx spammers
            except Exception:  # noqa: BLE001
                pass
            if os.environ.get("SIM_KEEP_UP", "0") != "1":
                runner.run_ok(["docker", "compose", "down", "-v", "--remove-orphans"],
                              timeout=300, note="teardown-down")
            _sweep_generated_overlays()

        try:
            bu.run()
            chain = self._build_chain(runner, bu)
            # ── build_addr_map (case-soak.sh:2133): populate ADDR2IDX FROM the live chain the moment
            # bring-up returns, BEFORE any self-check resolves a committee owner-address through it.
            # Bash runs it here (right after sim_bring_up, before sim_selfcheck / the result-seam
            # self-check) and NEVER lets that self-check iterate an empty map — the 0-of-0 class.
            # Rotation slots serve their OWN native identity until a rebirth re-tasks them (:2134-2137).
            from ..chain.writes import build_addr_map
            for rs in range(self.cfg.spare_end, self.cfg.val_containers):
                self.state.container.slot_identity.setdefault(topology.validator(rs), str(rs))
            build_addr_map(self.state, chain)
            rec, hp = self._reconcilers(chain)
            rec.events = events
            disp = Dispatcher(self.state, chain, self.act, hp, rec, events=events)
            battery = Battery(events=events)          # F1: ONE persistent battery (belts persist)
            self._bind_live_seams(disp, rec, hp)      # GAP-1: live beacon/metric reads
            self.self_check()
            # ── GAP-2a startup self-check (case-soak.sh:2154-2162): AFTER bring-up, BEFORE the loop.
            self.state.cur_epoch = chain.current_epoch()
            self.state.cur_committee = chain.committee(self.state.cur_epoch)
            self.state.cur_f = max((len(self.state.cur_committee.split()) - 1) // 3, 0)
            if not self.state.cur_committee:
                raise ChainError("selfcheck-no-committee",
                                 f"cannot read committee[{self.state.cur_epoch}] at startup — "
                                 "refusing a run whose result-divergence detector is unverifiable")
            self.selfcheck_result_seams_or_abort(finalized_dec=nodes.finalized_dec)

            dur_secs = _to_secs(os.environ.get("SIM_DURATION", "0"))
            churn_secs = _to_secs(os.environ.get("SIM_CHURN_PERIOD", "25s"))
            check_secs = _to_secs(os.environ.get("SIM_CHECK_PERIOD", "10s")) or 10
            stop_round = self.cfg.stop_round
            start = time.time()
            last_churn = 0.0
            committee_read_fail_ticks = 0

            # ── run-config record: the EFFECTIVE knobs this run acts and judges by ──────────
            # A bundle used to carry NO record of the environment its run was launched under —
            # Runner's command log is in-memory only (proc.py:88-90) and nothing else printed
            # the config, so an analysis could not tell whether e.g. a cadence override had
            # taken effect. Emitted here, after every knob is resolved and before the first
            # tick, so every bundle written from the loop carries it.
            # EFFECTIVE values only, never raw os.environ: the environment carries unrelated
            # secrets, and the post-default/post-derivation value is the thing being asked for.
            elog.sim_event("config", json.dumps({
                "cfg": vars(self.cfg),                       # SIM_* knobs + derived geometry
                "loop": {"duration_secs": dur_secs, "churn_secs": churn_secs,
                         "check_secs": check_secs, "epoch_interval": rec.epoch_interval,
                         "calm_permille": self.calm_permille},
                "battery": self._battery_knobs(battery),
                "runtime_addrs": self.rt_addrs,
            }, sort_keys=True, default=str))

            while True:
                now = time.time()
                if dur_secs > 0 and now - start >= dur_secs:      # F4: SIM_DURATION run-end
                    elog.sim_event("end", "duration reached")
                    break

                cur = chain.current_epoch()
                self.state.cur_epoch = cur
                # ── F6: committee read + coverage VISIBILITY (case-soak.sh:2182-2207) ──
                self.state.cur_committee = chain.committee(cur)
                n = len(self.state.cur_committee.split())
                if n == 0 or not self.state.cur_committee:
                    self.state.committee_read_ok = 0
                    committee_read_fail_ticks += 1
                    self.state.battery_coverage = 0    # vacuous tick: no membership claim fabricated
                    elog.sim_event("committee_read_fail",
                                    f"committee[{cur}] UNREADABLE (size={n}) — falling back to "
                                    f"n={self.cfg.initial_committee} for ARITHMETIC ONLY; tick "
                                    f"marked VACUOUS. Consecutive: {committee_read_fail_ticks}")
                    if n == 0:
                        n = self.cfg.initial_committee
                else:
                    self.state.committee_read_ok = 1
                    committee_read_fail_ticks = 0
                self.state.cur_f = (n - 1) // 3

                # ── §0 reconcile in-flight membership (GATED on a READABLE committee, F7) ──
                if self.state.committee_read_ok == 1:
                    rec.reconcile_inflight_membership(cur, n)
                # ── §1 tick-top reconcilers ──
                rec.compute_effective_faults(cur)
                rec.compute_dkg_barrier(cur)
                rec.process_restores(cur)
                rec.process_tombstone_backfills(cur)
                rec.process_rotations(cur)             # a20: chain-driven rekey of departed containers
                rec.process_evictions(cur)             # a22: resolve+rekey the emergent stake-evict victim
                rec.maintain_committee_hi(cur)         # a24: keep seated members at Hi (promoted benches)
                rec.replenish_key_supply(cur)          # a25: lazy fresh-key top-up (>50% band spent)
                rec.check_resource_pools(cur)          # F8: finite-pool headroom + loud exhaustion

                # ── the PERSISTENT invariant battery at the check cadence (F1) ──
                self.state.tick += 1
                # refresh ctx per tick (belts persist); rt_addrs points the battery's cast-read
                # seams at the DEPLOYED contracts instead of the codeless genesis predeploys.
                battery.bind(*self.to_ctx(**self.rt_addrs))
                if not battery.check_invariants():
                    _drain()
                    self._fail_bundle(elog, battery.inv_fail_id, battery.inv_fail_msg, _teardown)
                    return 1
                _drain()

                # ── churn cadence (case-soak.sh:2231, :2165) ──
                if now - last_churn >= churn_secs:
                    last_churn = now
                    # NEW-2: SIM_ROUND is incremented in EXACTLY ONE place — dispatch.run_round
                    # (matching bash's single SIM_ROUND++ at the churn-block top, case-soak.sh:2232).
                    # The stop-round gate predicts that increment (round+1) so it breaks BEFORE the
                    # round body, exactly as the bash `(( SIM_ROUND > STOP )) && break` does.
                    if stop_round and self.state.round + 1 > stop_round:
                        elog.sim_event("end", "SIM_STOP_ROUND reached")
                        break
                    disp.run_round(cur, n)                 # increments state.round
                    _drain()

                time.sleep(check_secs)                  # F3: terminal tick sleep

            elog.sim_event("end", f"sim finished cleanly (seed {self.cfg.seed}, "
                                   f"{self.state.round} rounds)")
            _teardown()
            return 0
        except KeyboardInterrupt:                       # F5: SIGINT/SIGTERM path
            if interrupted["v"]:
                elog.sim_event("end", "interrupted (SIGINT/SIGTERM)")
            _drain()
            _teardown()
            return 130
        except ChainError as e:                         # F5: watchdog trip → bundle + teardown
            _drain()
            self._fail_bundle(elog, e.reason_id, e.message, _teardown)
            return 1
        finally:
            signal.signal(signal.SIGINT, prev_int)
            signal.signal(signal.SIGTERM, prev_term)

    def _fail_bundle(self, elog, inv_id, msg, teardown):
        """fail_bundle (case-soak.sh): record the invariant_fail event, dump the structured bundle,
        then tear down honoring SIM_KEEP_UP."""
        elog.sim_event("invariant_fail", f"{inv_id} {msg}")
        try:
            elog.bundle_dump(msg, inv_id)
        except Exception:  # noqa: BLE001 — a best-effort bundle must never mask the original fault
            pass
        teardown()


# ── PORT STATUS (the code-completable port is DONE) ───────────────────────────
#
# The DECISION + STATE layer and every IMPURE seam are now ported and unit-tested:
#   * sim_bring_up ....................... bringup.py (gen-compose → up → migrate → DeployStaking
#                                           → commitEpochCommittee → cold-restart under --dpos)
#   * the pp_* staking/gov WRITE helpers .. chain/writes.py (Chain: gov_action / fund_eth / ensure_blend
#                                           / owner_addr,key / consensus_keys / runtime_write,cat …)
#   * the per-tick live READS ............. nodes.py (pp_current_epoch/committee/committee_has via
#                                           the Chain adapters; recovery_confirmed / node_finalized)
#   * the beacon/metric READ seams ....... nodes.beacon_metric / refill_spare_attempts, bound live
#                                           by _bind_live_seams (dkg_ceremony_ok / beacon_seed_active
#                                           :9100, reth_dpos_derive_db_timeout_attempts_total :9200);
#                                           top_stake_leader stays inert "" (bash is inert too)
#   * the per-tick reconcilers ........... reconcilers.py (compute_effective_faults / compute_dkg_
#                                           barrier / process_restores / process_tombstone_backfills)
#   * the rebirth host-pool machinery .... hostpool.py (warm_identity / warm_ready / promote_host_
#                                           avail / recyclable_containers)
#   * sim_selfcheck_result_seams ........ selfcheck_result_seams(_or_abort) — the startup gate,
#                                           called in run() post-bring-up / pre-loop
#
# NOTE: the once-per-run quorum-loss probe (deliberately dropping f+1) was REMOVED from the sim
# churn loop — it violated the ≤f-at-source invariant and its docker stop/start proved unreliable
# under live churn (stale COMPOSE_FILE). The f+1 halt-then-recover SAFETY property is now proven by
# the standalone case_quorum.py, which runs on a clean, stable docker set.
#
# WHAT REMAINS is FIRST-LIVE-RUN CALIBRATION ONLY — timings/thresholds that can only be trued
# against a real topology, not code left to write:
#   * track-interleaving timing — the round cadence (SIM_CHURN_PERIOD) vs. how fast growth/refill/
#     promote tracks actually LAND on the live chain (bash tuned these over many sims);
#   * barrier calibration under degraded rate — the DKG-barrier / recover / backfill deadlines when
#     production runs at a degraded (sub-1-blk/s) rate under saturation;
#   * chain-paced pacing — the ChainPaced block/freeze budgets for the gov + activation waits.
