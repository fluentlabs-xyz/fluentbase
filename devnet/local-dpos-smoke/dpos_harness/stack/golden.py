"""golden.py — a "golden DPoS-active" snapshot/restore so smoke cases can SKIP the slow
boot-to-DPoS (phase-A sequencer + activation-wait + sequencer→DPoS migration, ~2-4 min).

THE IDEA (deliberately simple — no daemon, no registry, one tarball + one hash sidecar)
    All post-boot runtime state (reth MDBX + static files + commonware journals + keys)
    lives in ONE docker volume mounted at /runtime (the generated compose's `runtime`
    volume, docker-named `<project>_runtime`). We bring a small stack to DPoS-active ONCE
    via the normal full boot (BringUp.run), graceful-stop it so reth flushes its in-memory
    tail to MDBX, then `tar` that volume into `sim-out/golden/golden-dpos.tgz` keyed by an
    invalidation hash. Per case, we RESTORE the tarball into a clean runtime volume and start
    the validators DIRECTLY under --dpos — reusing the EXISTING populated-/runtime resume
    path (the crash-survivor / full-restart discriminator), NOT inventing a new mechanism.

INVALIDATION (golden_hash)
    The snapshot is only valid for the exact binary + contracts + topology it was baked with:
      * the fluent smoke docker image id (a rebuilt node binary invalidates the state),
      * contracts/.vendor-sha (a redeployed staking cluster invalidates the addresses),
      * the committee/topology/epoch config that determines the on-chain + consensus state
        (validators / initial_committee / val_containers / identity_pool / epoch interval).
    is_golden_fresh() = tarball present AND its sidecar hash == the current golden_hash().
    A stale/absent golden silently falls back to the full boot — the fast path is opt-in.

The pure hashing/freshness core (compute_hash / is_golden_fresh_logic) takes injected inputs
so it is unit-tested docker-free (tests/test_golden.py). The gatherers (_image_id / _vendor_sha
/ _golden_config) and the docker-touching build/restore live below them.
"""

from __future__ import annotations

import hashlib
import json
import os
import time
from dataclasses import dataclass, field
from typing import Callable

from .bringup import StackSpec
from ..core import proc, topology

# Fixed compose project name (compose_gen emits `name: fluent-dpos-sim`). The runtime
# volume is docker-named `<project>_runtime`; we still DETECT it against `docker volume ls`
# rather than trust this blindly (a project rename would surface as a loud detection error).
PROJECT = "fluent-dpos-sim"
IMAGE = "fluent-dpos-smoke:local"

# Stable artifact dir under the smoke dir (sim-out/ is gitignored). Anchored to THIS file's
# location so it resolves regardless of the caller's cwd. DEPTH-SENSITIVE: this file lives at
# <smoke>/dpos_harness/stack/golden.py, so the smoke dir is THREE dirnames up. Changing the
# module's nesting without changing this silently relocates the golden tarball.
_SMOKE_DIR = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
GOLDEN_DIR = os.path.join(_SMOKE_DIR, "sim-out", "golden")
TARBALL = os.path.join(GOLDEN_DIR, "golden-dpos.tgz")
SIDECAR = os.path.join(GOLDEN_DIR, "golden-dpos.hash")
FACTS = os.path.join(GOLDEN_DIR, "golden-dpos.facts.json")


class GoldenError(RuntimeError):
    """A golden build/restore step failed loudly (missing tarball, undetectable volume,
    tar/extract failure) — never a silent bad snapshot."""


@dataclass
class GoldenSpec:
    """Everything golden needs from ABOVE it, handed down by the caller.

    WHY IT EXISTS: this module used to build its own `SimConfig` and call a *case module's*
    env profile to compute the invalidation hash — six imports reaching UP from `stack` into
    `sim` and `cases`. Every one of them is now a field here, so `stack/golden.py` imports
    nothing above its own layer and the caller (which already knows its profile) says what the
    snapshot is keyed to. Nothing about the hash or the build/restore steps changed.

    Fields:
      config       — the state-determining knobs the invalidation hash is computed over. This
                     is the ONLY thing `--check` needs, which is why it has no default: a spec
                     built without it would silently hash `{}` and call every golden fresh.
      stack_spec   — the bring-up contract (`bringup.StackSpec`), passed straight through to
                     `BringUp`. Only `build_golden` needs it. It used to be an `Any`-typed
                     "opaque bring-up config" because bring-up itself took anything with the
                     right seven attributes; now that bring-up states its contract, so does this.
      validators / val_containers / identity_pool — the topology numbers `restore_golden` feeds
                     to `compose_gen.generate` and the `docker compose create` service list.
      await_dpos_active — the caller's "wait until DPoS is live" probe, `(chain, nodes,
                     deadline_s) -> (fin, epoch)`. Only `build_golden` needs it.
    """
    config: dict = field(default_factory=dict)
    stack_spec: StackSpec = None
    validators: int = 0
    val_containers: int = 0
    identity_pool: int = 0
    await_dpos_active: Callable = None


# ── PURE CORE (docker-free, unit-tested) ────────────────────────────────────────

def compute_hash(image_id: str, vendor_sha: str, config: dict) -> str:
    """Deterministic short invalidation key over the three state-determining inputs. Pure:
    same inputs → same 16-hex digest, key order irrelevant (sort_keys)."""
    payload = json.dumps(
        {"image": image_id or "", "vendor": (vendor_sha or "").strip(), "config": config or {}},
        sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(payload.encode()).hexdigest()[:16]


def is_golden_fresh_logic(tarball_exists: bool, sidecar_hash, current_hash: str) -> bool:
    """Fresh iff the tarball is present AND its recorded sidecar hash equals the current
    golden_hash(). A missing tarball, a missing sidecar, or a hash mismatch (rebuilt image /
    bumped contracts / changed topology) is stale. Pure."""
    if not tarball_exists:
        return False
    if not sidecar_hash:
        return False
    return str(sidecar_hash).strip() == str(current_hash).strip()


# ── INPUT GATHERERS ─────────────────────────────────────────────────────────────

def _image_id(image: str = IMAGE) -> str:
    """Docker image id of the fluent smoke image ("" if the image is absent — a golden built
    against no image is meaningless, and "" simply never matches a real sidecar)."""
    out = proc.read_capture(["docker", "image", "inspect", image, "--format", "{{.Id}}"],
                           timeout=30)
    return out.stdout.strip() if out.ok else ""   # docker down → "" (stale by construction)


def _vendor_sha() -> str:
    """contracts/.vendor-sha contents ("" if absent)."""
    path = os.path.join(_SMOKE_DIR, "contracts", ".vendor-sha")
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return ""


def golden_hash(spec: GoldenSpec) -> str:
    """The current invalidation key = compute_hash(image id, vendor sha, topology config).

    The topology config used to be gathered here by applying the case-growth env profile and
    constructing a `SimConfig`; it now arrives as `spec.config`, built by the caller that owns
    that profile. The caller must apply its env profile BEFORE building the spec, exactly as
    `_golden_config` did, so a `--check` from a clean shell hashes what the build baked with."""
    return compute_hash(_image_id(), _vendor_sha(), spec.config)


# ── DISK FRESHNESS ──────────────────────────────────────────────────────────────

def _read_sidecar():
    try:
        with open(SIDECAR) as f:
            return f.read().strip()
    except OSError:
        return None


def is_golden_fresh(spec: GoldenSpec) -> bool:
    """Disk-backed freshness: the tarball exists AND its sidecar hash == golden_hash(spec)."""
    return is_golden_fresh_logic(os.path.exists(TARBALL), _read_sidecar(), golden_hash(spec))


def load_facts() -> dict:
    """Load the deploy-facts sidecar (staking/chain_config/governance/liveness/token addresses)
    captured at build time so a restored stack can drive on-chain actions without re-reading the
    host manifest."""
    with open(FACTS) as f:
        return json.load(f)


# ── VOLUME DETECTION ────────────────────────────────────────────────────────────

def _detect_runtime_volume() -> str:
    """Detect the runtime docker volume (never trust a hardcoded name). The generated compose
    pins project `fluent-dpos-sim` + volume key `runtime`, so the docker name is
    `fluent-dpos-sim_runtime` — but we VERIFY it against `docker volume ls`, and fall back to a
    unique `<project>_runtime` match, raising loudly if it cannot be resolved."""
    out = proc.read_capture(["docker", "volume", "ls", "--format", "{{.Name}}"], timeout=30)
    if not out.ok:
        raise GoldenError(f"could not list docker volumes: rc={out.rc} {out.stderr}")
    vols = out.stdout.split()
    candidate = f"{PROJECT}_runtime"
    if candidate in vols:
        return candidate
    matches = [v for v in vols if v.startswith(PROJECT) and v.endswith("_runtime")]
    if len(matches) == 1:
        return matches[0]
    raise GoldenError(
        f"could not detect the runtime volume (looked for {candidate!r}; "
        f"candidates={matches or 'none'}; all volumes={vols})")


def _tar_volume(runner, volume: str):
    """tar the runtime volume into TARBALL via a throwaway alpine container (reth writes
    root-owned files; the container runs as root and reads the volume mount directly)."""
    os.makedirs(GOLDEN_DIR, exist_ok=True)
    runner.run_checked(
        ["docker", "run", "--rm", "-v", f"{volume}:/data", "-v", f"{GOLDEN_DIR}:/b",
         "alpine", "tar", "czf", "/b/golden-dpos.tgz", "-C", "/data", "."],
        timeout=600, note="golden-tar")


def _untar_into_volume(runner, volume: str):
    """Extract TARBALL into a (freshly-created, empty) runtime volume — the reverse of _tar."""
    if not os.path.exists(TARBALL):
        raise GoldenError(f"golden tarball missing: {TARBALL}")
    runner.run_checked(
        ["docker", "run", "--rm", "-v", f"{volume}:/data", "-v", f"{GOLDEN_DIR}:/b",
         "alpine", "sh", "-c", "rm -rf /data/* /data/.[!.]* 2>/dev/null; "
         "tar xzf /b/golden-dpos.tgz -C /data"],
        timeout=600, note="golden-untar")


# ── BUILD ───────────────────────────────────────────────────────────────────────

def build_golden(spec: GoldenSpec, runner=None) -> str:
    """Bring the caller's stack to DPoS-active ONCE via the normal full boot, graceful-stop it
    (reth flushes to MDBX), tar the runtime volume into TARBALL, and write the hash + facts
    sidecars. Returns the golden hash. Wipes stale volumes/data-root first (a leftover volume
    caused a reth static-file panic in an earlier run).

    The caller has already applied its env profile and built `spec` from it — this function no
    longer reaches up into a case module to do that for itself."""
    from ..core import nodes
    from .bringup import BringUp, DPOS_COMPOSE
    from ..chain.writes import Chain
    from ..core.proc import Runner

    rpc = os.environ.get("RPC", topology.DEFAULT_RPC_URL)
    env = {"RPC": rpc, "COMPOSE_FILE": os.environ.get("COMPOSE_FILE", ""),
           "CHAIN_ID": os.environ.get("CHAIN_ID", str(topology.CHAIN_ID))}
    runner = runner or Runner(env=env)

    t0 = time.time()
    print(f"GOLDEN build: config {spec.config}", flush=True)

    # Data hygiene: nuke any leftover fluent-dpos-sim volume + data-root before the build so a
    # stale runtime tree can't panic reth on the fresh boot. (BringUp.run's own preflight
    # `down -v` + data-root wipe repeats this, but we belt it here against a bind-backed volume
    # or a volume orphaned from a prior project.)
    runner.run_ok(["docker", "compose", "down", "-v", "--remove-orphans"], timeout=300,
                  note="golden-preflight-down")
    for v in _list_project_volumes():
        runner.run_ok(["docker", "volume", "rm", "-f", v], timeout=60, note="golden-rm-vol")

    bu = BringUp(spec.stack_spec, runner)
    print("GOLDEN build: full boot (phase-A sequencer + deploy + activation + --dpos migration)...",
          flush=True)
    bu.run()
    t_boot = time.time()

    chain = Chain(runner=runner, RPC=rpc, STAKING_RT=bu.staking_rt,
                  CHAIN_CONFIG_RT=bu.chain_config_rt, GOV_ADDR=bu.gov_addr,
                  LIVENESS_RT=bu.liveness_rt, TOKEN=bu.token,
                  CHAIN_ID=env["CHAIN_ID"])
    fin, epoch = spec.await_dpos_active(chain, nodes, deadline_s=300)
    print(f"GOLDEN build: DPoS active (fin={fin} epoch={epoch}) after "
          f"{t_boot - t0:.0f}s full boot", flush=True)

    # Stop the tx pressure BEFORE the graceful flush so nothing writes during the tar.
    try:
        bu.spammers.stop()
    except Exception:  # noqa: BLE001
        pass

    # Graceful stop → reth's native on_graceful_shutdown persists the in-memory tail to MDBX.
    os.environ["COMPOSE_FILE"] = DPOS_COMPOSE
    runner.env["COMPOSE_FILE"] = DPOS_COMPOSE
    runner.run_checked(["docker", "compose", "stop", "--timeout", "40"], timeout=180,
                       note="golden-graceful-stop")

    volume = _detect_runtime_volume()
    print(f"GOLDEN build: taring runtime volume {volume} → {TARBALL}", flush=True)
    _tar_volume(runner, volume)

    facts = {"token": bu.token, "staking": bu.staking_rt, "chain_config": bu.chain_config_rt,
             "governance": bu.gov_addr, "liveness_slashing": bu.liveness_rt}
    with open(FACTS, "w") as f:
        json.dump(facts, f, indent=2)
    h = golden_hash(spec)
    with open(SIDECAR, "w") as f:
        f.write(h + "\n")

    # Leave a clean host — the tarball is the artifact.
    runner.run_ok(["docker", "compose", "down", "-v", "--remove-orphans"], timeout=300,
                  note="golden-cleanup-down")

    size = os.path.getsize(TARBALL) if os.path.exists(TARBALL) else 0
    print(f"GOLDEN build: DONE hash={h} tarball={size} bytes total={time.time() - t0:.0f}s",
          flush=True)
    return h


def _list_project_volumes():
    out = proc.read_capture(["docker", "volume", "ls", "--format", "{{.Name}}"], timeout=30)
    return [v for v in out.stdout.split() if v.startswith(PROJECT)]


# ── RESTORE ─────────────────────────────────────────────────────────────────────

def restore_golden(spec: GoldenSpec, runner) -> str:
    """Recreate a clean runtime volume and extract the golden tarball into it, WITHOUT running
    genesis-init (which would rewrite /runtime). Uses `docker compose create` to materialise the
    compose-labelled volume + validator containers (nothing started), then extracts into the
    volume; the caller then `start`s the validators directly under --dpos. Returns the volume
    name. Assumes is_golden_fresh(spec) already checked by the caller.

    The topology numbers arrive on `spec`; the caller applied its env profile before building
    it, which is what the removed `case_growth.apply_case_env_defaults()` call did here."""
    from . import compose_gen
    from .bringup import DPOS_COMPOSE

    if not is_golden_fresh(spec):
        raise GoldenError("restore_golden called but the golden is stale/absent")

    # Regenerate the compose pair for this profile so COMPOSE_FILE resolves and the runtime
    # volume definition matches what the golden was baked against.
    compose_gen.generate(spec.val_containers, spec.validators, spec.identity_pool)
    os.environ["COMPOSE_FILE"] = DPOS_COMPOSE
    runner.env["COMPOSE_FILE"] = DPOS_COMPOSE

    # Clean slate, then let compose CREATE (not start) the validators — this creates the
    # compose-labelled runtime volume without running genesis-init.
    runner.run_ok(["docker", "compose", "down", "-v", "--remove-orphans"], timeout=300,
                  note="golden-restore-down")
    val_list = [topology.validator(i) for i in range(spec.validators)]
    runner.run_checked(["docker", "compose", "create", *val_list], timeout=300,
                       note="golden-restore-create")

    volume = _detect_runtime_volume()
    print(f"GOLDEN restore: extracting {TARBALL} → volume {volume}", flush=True)
    _untar_into_volume(runner, volume)
    return volume
