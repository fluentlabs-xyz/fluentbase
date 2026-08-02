"""profiles.py — the two devnet STACK PROFILES, behind one interface.

There are two different devnets in this directory and they are not variants of each other:

  * the **generated** profile — `compose_gen` writes a base + `--dpos` overlay pair for N
    validator containers, and `stack/bringup.py` drives it through a runtime contract deploy,
    governance, an activation block and a cold restart. This is what the simulation runs on.
  * the **static** profile — the checked-in `docker-compose.yml` + `docker-compose.dpos.yml`
    pair, four validators and a full-node, genesis-baked contracts, reaching DPoS through a
    graceful-stop migration (`stack/static_stack.py`). This is what all 26 smoke cases run on.

Before this module existed only the first one was expressible in Python, so the case layer had
nowhere to stand and would have been ported as a SECOND parallel harness — the exact situation
this framework exists to remove. A profile answers three questions and nothing else: which
compose files, whether they must be generated first, and which services are the committee.

WHY `compose_files` AND `compose_file_env` BOTH EXIST — they are not two spellings of one
thing, and the difference is bash-faithful:

  * the generated profile drives compose through the ambient `COMPOSE_FILE` env var, which the
    sim exports once per phase (`bringup._set_compose`), so every later BARE `docker compose`
    call — including every read in `core/rpc.py` — resolves the sim project;
  * the static profile does NOT export `COMPOSE_FILE` at all. `lib.sh` runs phase A as a bare
    `docker compose up --build -d` (resolving `docker-compose.yml` by default), issues the DPoS
    recreate with EXPLICIT `-f` flags (lib.sh:469), and leaves every read, `logs` and
    `down` bare. Exporting `COMPOSE_FILE` for the static profile would silently change which
    project a hundred bare read commands resolve, so the port keeps bash's asymmetry.

`compose_file_env` therefore returns None for the static profile, which is a FACT about it
rather than a gap — a caller that must have one is asking the wrong profile.
"""

from __future__ import annotations

import os

from . import compose_gen
from ..core import topology

# ── the generated (simulation) compose pair ───────────────────────────────────────────
# The two ambient COMPOSE_FILE values the sim exports. RELATIVE names with a `:` separator —
# compose resolves each against the process cwd (the smoke dir).
GENERATED_BASE = "docker-compose.sim.gen.yml"
GENERATED_DPOS_OVERLAY = "docker-compose.sim.dpos.gen.yml"

# ── the static (smoke-case) compose pair ──────────────────────────────────────────────
STATIC_BASE = "docker-compose.yml"
STATIC_DPOS_OVERLAY = "docker-compose.dpos.yml"

#: `lib.sh:26` — mirrors genesis-bootstrap's `epochBlockInterval`. 32, NOT the sim's 64: the
#: static cases want quick epoch boundaries and every one of them that does epoch arithmetic
#: (`asserts.sh:71`, `case-liveness.sh:127`, `case-cert-catchup.sh:75`) reads this value.
STATIC_EPOCH_INTERVAL = 32


class StackProfile:
    """One devnet the harness can bring up. Subclasses answer the three questions below.

    Not an ABC with abstractmethods on purpose: a profile is a small value object and the two
    implementations are right here, so `NotImplementedError` is enough and it keeps the class
    usable as a type annotation without dragging `abc` into the layer.
    """

    #: Short label for transcripts and error messages.
    name = "profile"

    def prepare(self, runner) -> None:
        """Do whatever must happen before `docker compose` can resolve this profile's files."""
        raise NotImplementedError

    def compose_files(self, phase: str = "dpos"):
        """The compose files for `phase` ("base" or "dpos"), base-first, overlays after."""
        raise NotImplementedError

    def compose_file_env(self, phase: str = "dpos"):
        """The `COMPOSE_FILE` value for `phase`, or None when this profile does not use one."""
        raise NotImplementedError

    def committee(self):
        """The validator services that come up under `--dpos`."""
        raise NotImplementedError

    # -- shared derivations -----------------------------------------------------------
    def compose_args(self, phase: str = "dpos"):
        """`["-f", <file>, …]` for `phase` — the explicit form, for a command that must name
        its files rather than inherit them (`lib.sh:469`, and every per-case `*_COMPOSE=(-f …)`
        array). Derived from `compose_files`, so a profile cannot disagree with itself."""
        args = []
        for f in self.compose_files(phase):
            args += ["-f", f]
        return args


class GeneratedProfile(StackProfile):
    """The N-validator compose pair `stack/compose_gen.py` writes. The simulation's devnet.

    `prepare` GENERATES the files (unless the runner is dry, where the transcript is the
    artifact and nothing is written). The three sizes are the generator's arguments, and they
    come from the caller's `StackSpec` — this class holds no policy of its own.
    """

    name = "generated"

    def __init__(self, val_containers: int, validators: int, identity_pool: int):
        self.val_containers = int(val_containers)
        self.validators = int(validators)
        self.identity_pool = int(identity_pool)

    @classmethod
    def for_spec(cls, spec) -> "GeneratedProfile":
        """Build the profile a `StackSpec` describes."""
        return cls(spec.val_containers, spec.validators, spec.identity_pool)

    def prepare(self, runner) -> None:
        if runner.dry:
            return
        compose_gen.generate(self.val_containers, self.validators, self.identity_pool)

    def compose_files(self, phase: str = "dpos"):
        if phase == "base":
            return (GENERATED_BASE,)
        return (GENERATED_BASE, GENERATED_DPOS_OVERLAY)

    def compose_file_env(self, phase: str = "dpos"):
        return ":".join(self.compose_files(phase))

    def committee(self):
        return tuple(topology.validator(i) for i in range(self.validators))


class StaticProfile(StackProfile):
    """The checked-in `docker-compose.yml` + `docker-compose.dpos.yml` pair. The smoke cases'
    devnet: four validators, a full-node, contracts baked into genesis.

    ═══ THE EXTRA OVERLAY ═══════════════════════════════════════════════════════════════

    `extra_overlays` is the per-case compose mechanism, and `stack/static_stack.py`'s
    migration is its SINGLE honouring point — the same single point `_migrate_to_dpos` was,
    inherited by 23 of the 26 cases through `bring_up_dpos`. `case-byzantine.sh` exports
    `DPOS_EXTRA_COMPOSE="-f docker-compose.byzantine.yml"` and nothing else in the case knows
    about it; the overlay reaches the chain because the DPoS recreate names it.

    The overlays apply to the **dpos** phase only, faithfully to bash: phase A is a bare
    `docker compose up --build -d`, so an overlay is not in effect until the migration
    recreate. A case whose overlay must be live during phase A would be a new mechanism, not
    a wider value for this one.

    `from_env` parses the bash spelling (`"-f a.yml -f b.yml"`) so a case that still exports
    the variable, and the Makefile targets that set it, keep working unchanged.
    """

    name = "static"

    def __init__(self, extra_overlays=(), epoch_interval=None, activation_block=None):
        self.extra_overlays = tuple(extra_overlays)
        # lib.sh:26 — `EPOCH_INTERVAL="${EPOCH_INTERVAL:-32}"`.
        self.epoch_interval = int(
            epoch_interval if epoch_interval is not None
            else os.environ.get("EPOCH_INTERVAL", STATIC_EPOCH_INTERVAL))
        # lib.sh:33 — `DPOS_ACTIVATION_BLOCK="${DPOS_ACTIVATION_BLOCK:-$((2 * EPOCH_INTERVAL))}"`.
        # MUST equal genesis-bootstrap's `ChainConfig.dposActivationBlock`. The default DERIVES
        # from the interval (bootstrap's own `2 * epochBlockInterval`), so a tuned interval stays
        # aligned without exporting the activation block separately — ONE derivation rule across
        # compose (empty -> bootstrap derives), here, and bootstrap.
        env_act = os.environ.get("DPOS_ACTIVATION_BLOCK")
        self.activation_block = int(
            activation_block if activation_block is not None
            else (env_act if env_act else 2 * self.epoch_interval))

    @classmethod
    def from_env(cls, extra_overlays=None, **kw) -> "StaticProfile":
        """Build the profile a case's environment describes: `DPOS_EXTRA_COMPOSE` in the bash
        `-f file.yml` spelling, plus `EPOCH_INTERVAL` / `DPOS_ACTIVATION_BLOCK`.

        An explicit `extra_overlays` wins over the env var — a Python case should hand its
        overlay in directly rather than round-tripping it through the environment.
        """
        if extra_overlays is None:
            extra_overlays = parse_extra_compose(os.environ.get("DPOS_EXTRA_COMPOSE", ""))
        return cls(extra_overlays=extra_overlays, **kw)

    def prepare(self, runner) -> None:
        """No-op: the static compose files are checked into the repo. Present so the two
        profiles are interchangeable at the call site rather than only nearly so."""
        return

    def compose_files(self, phase: str = "dpos"):
        if phase == "base":
            return (STATIC_BASE,)
        return (STATIC_BASE, STATIC_DPOS_OVERLAY) + self.extra_overlays

    def compose_file_env(self, phase: str = "dpos"):
        """None, deliberately — see the module header. The static stack drives compose with
        explicit `-f` flags and leaves every bare command resolving `docker-compose.yml`,
        exactly as `lib.sh` does."""
        return None

    def committee(self):
        """`lib.sh:35` — `VALS=(validator-0 validator-1 validator-2 validator-3)`. The set is
        fixed by the checked-in compose file, not by a knob."""
        return tuple(topology.validator(i) for i in range(4))

    def epoch_zero_window(self):
        """`[activation, activation + interval)` — relative epoch 0, the ONLY window a
        fresh-migration anchor may land in. Below activation `OriginEpocher` rejects the
        cold-start height; at or above `activation + interval` the cold-start epoch would be
        >= 1 and re-hit the empty-marshal genesis lookup."""
        return self.activation_block, self.activation_block + self.epoch_interval


def parse_extra_compose(value: str):
    """`"-f a.yml -f b.yml"` -> `("a.yml", "b.yml")`. The bash `DPOS_EXTRA_COMPOSE` spelling.

    Bare filenames are accepted too (`"a.yml b.yml"`), because a Python caller setting the
    variable has no reason to write the flags and a silently-dropped overlay is the failure
    mode this whole mechanism is one point for. Anything that is not a `-f` flag is taken as a
    filename; `-f` tokens are consumed.
    """
    return tuple(tok for tok in (value or "").split() if tok != "-f")
