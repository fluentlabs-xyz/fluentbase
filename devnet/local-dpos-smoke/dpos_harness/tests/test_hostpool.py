"""test_hostpool.py — the rebirth / host-pool machinery. Drives HostPool with a dry Runner + a
stubbed Chain (committee_has / owner_addr seams) and asserts the docker choreography argv + the
domain-typed state mutations (the container-aliasing bridge sever)."""

import os

from dpos_harness.sim.hostpool import HostPool
from dpos_harness.sim.orchestrator import SimConfig, SimState
from dpos_harness.core.proc import Runner


class FakeChain:
    def __init__(self, seated=()):
        self.seated = set(str(s) for s in seated)
        self.p = Runner(dry=True)

    def owner_addr(self, idx):
        return f"0xowner{idx}"

    def committee_has(self, addr, epoch):
        return addr.replace("0xowner", "") in self.seated


def _hp(seated=(), **skw):
    s = SimState(cfg=SimConfig(validators=4, initial_committee=3, spares=1, rotation_slots=1,
                                 identity_pool=8))
    for k, v in skw.items():
        setattr(s, k, v)
    r = Runner(dry=True)
    chain = FakeChain(seated)
    hp = HostPool(s, chain, r, confirm=lambda c: True, beacon=lambda i: 5)
    return hp, s, r


def test_retask_severs_seat_bridge():
    """_retask_slot: re-tasking a container SEVERS every SEAT_CONTAINER bridge naming it (the
    aliasing guard) and writes SLOT_IDENTITY."""
    hp, s, r = _hp()
    s.seat.seat_container = {3: "validator-5", 7: "validator-6"}
    hp.retask_slot("validator-5", 20)
    assert 3 not in s.seat.seat_container      # bridge to the re-tasked container severed
    assert 7 in s.seat.seat_container          # unrelated bridge kept
    assert s.container.slot_identity["validator-5"] == "20"   # NEW-1: stored in the str idx domain


def test_recycle_container_clears_container_tracking():
    """sim_recycle_container: drops the byz overlay + clears PERMANENTLY_DEAD/DISRUPT_KIND/
    TOMBSTONE_SETTLE_EPOCH + the against-f slot + severs the bridge."""
    hp, s, r = _hp()
    s.identity.permanently_dead = {"validator-5": 1}
    s.container.disrupt_kind = {"validator-5": "byzantine_equivocate"}
    s.identity.tombstone_settle_epoch = {"validator-5": 3}
    s.disrupted = "validator-5"
    s.seat.seat_container = {5: "validator-5"}
    hp.recycle_container("validator-5")
    assert "validator-5" not in s.identity.permanently_dead
    assert "validator-5" not in s.container.disrupt_kind
    assert s.disrupted == ""
    assert 5 not in s.seat.seat_container
    lines = [" ".join(a.argv) for a in r.log]
    assert any("rm -f docker-compose.sim.byz-validator-5.gen.yml" in l for l in lines)


def test_recyclable_containers_excludes_seated_and_disrupted():
    """_recyclable_containers: only tombstones whose slash LANDED (not seated) AND not against-f."""
    hp, s, r = _hp(seated={"6"})   # identity 6 still seated
    s.identity.permanently_dead = {"validator-5": 1, "validator-6": 1, "validator-7": 1}
    s.disrupted = "validator-7"    # still against-f
    s.container.slot_identity = {"validator-6": 6}
    out = hp.recyclable_containers()
    assert out == ["validator-5"]  # 6 seated, 7 disrupted → excluded


def test_rebirth_identity_choreography(monkeypatch):
    """sim_rebirth_identity: stop → wipe beacon → write IDENTITY_IDX overlay → up --no-deps
    --force-recreate, in order."""
    hp, s, r = _hp()
    monkeypatch.delenv("COMPOSE_FILE", raising=False)   # no ambient value → the Runner env is base
    hp.p.env["COMPOSE_FILE"] = "base.yml"
    hp.rebirth_identity("validator-5", 20)
    lines = [" ".join(a.argv) for a in r.log]
    stop = next(i for i, l in enumerate(lines) if "docker compose stop --timeout 40 validator-5" in l)
    wipe = next(i for i, l in enumerate(lines) if "rm -rf /runtime/reth-data/v5/beacon" in l)
    up = next(i for i, l in enumerate(lines)
              if "up -d --no-deps --force-recreate validator-5" in l)
    assert stop < wipe < up
    # the persistent overlay was appended to COMPOSE_FILE (idempotent).
    assert "docker-compose.sim.reborn-validator-5.gen.yml" in hp.p.env["COMPOSE_FILE"]


def test_rebirth_exports_overlay_to_the_ambient_environ(monkeypatch):
    """The reborn IDENTITY_IDX overlay must reach os.environ, not only Runner.env.

    Actuators and Runner hold two SEPARATE env dicts (orchestrator.py:711 / :760), and
    Actuators.act_byzantine deliberately rebuilds its compose list from os.environ
    (actions.py:682) because Actuators.env is a stale pre-bring-up snapshot. With a Runner-only
    append, `up -d --no-deps --force-recreate <slot>` for a byzantine action on a REBORN slot ran
    without the overlay, so keydir fell back to the NATIVE index (compose_gen.py:345-346): the
    container returned as an already-burned identity, the intended identity went silent and
    nothing was slashable (byzantine-slash-not-landed). Bash exports for exactly this reason
    (soak-actions.sh:644-656). The conftest fixture snapshots/restores COMPOSE_FILE per test."""
    hp, s, r = _hp()
    base = "docker-compose.sim.gen.yml:docker-compose.sim.dpos.gen.yml"
    monkeypatch.setenv("COMPOSE_FILE", base)
    hp.p.env["COMPOSE_FILE"] = base
    overlay = "docker-compose.sim.reborn-validator-5.gen.yml"

    hp.rebirth_identity("validator-5", 20)
    assert os.environ["COMPOSE_FILE"] == f"{base}:{overlay}"      # ambient carries the overlay
    assert hp.p.env["COMPOSE_FILE"] == os.environ["COMPOSE_FILE"]  # both envs in lockstep

    # A re-rebirth rewrites the SAME overlay file, so the PATH-style guard must not double-append.
    hp.rebirth_identity("validator-5", 21)
    assert os.environ["COMPOSE_FILE"].split(":").count(overlay) == 1


def test_rebirth_bases_on_ambient_not_the_empty_runner_sentinel(monkeypatch):
    """Runner.env is seeded with an EMPTY COMPOSE_FILE at Orchestrator.__init__ (present-but-empty),
    so a defaulted read of it would yield "" and publish an overlay-ONLY COMPOSE_FILE — dropping the
    base sim files and making every later compose call a silent no-op (the actions.py:682 defect).
    Base on the ambient value and re-sync the Runner from it."""
    hp, s, r = _hp()
    monkeypatch.setenv("COMPOSE_FILE", "base.yml")
    hp.p.env["COMPOSE_FILE"] = ""
    hp.rebirth_identity("validator-5", 20)
    want = "base.yml:docker-compose.sim.reborn-validator-5.gen.yml"
    assert os.environ["COMPOSE_FILE"] == want
    assert hp.p.env["COMPOSE_FILE"] == want


def test_warm_ready_needs_confirm_and_beacon():
    hp, s, r = _hp()
    hp._confirm = lambda c: True
    hp._beacon = lambda i: 5
    assert hp.warm_ready(2) is True          # native idx, EL-confirmed + beacon serving
    hp._beacon = lambda i: -1                # metrics plane cold
    assert hp.warm_ready(2) is False


def test_rebirth_returns_up_result():
    """Fix A (v61 a13): rebirth_identity returns the resume `up` result, NOT an unconditional True.
    A resume that fails to leave the slot running must report False so the caller never records a
    false-HOSTED slot (the desync that deferred the promote track for 9h)."""
    hp, s, r = _hp()
    hp.p.env["COMPOSE_FILE"] = "base.yml"
    hp.p.run_ok = lambda argv, **kw: not ("up" in argv and "--force-recreate" in argv)  # only up fails
    assert hp.rebirth_identity("validator-5", 20) is False
    hp.p.run_ok = lambda argv, **kw: True                         # a clean resume that comes up
    assert hp.rebirth_identity("validator-5", 20) is True


def test_warm_identity_no_false_host_when_resume_up_fails():
    """Fix A+B end-to-end (v61 a13): when the chosen rotation slot's resume `up` fails, warm_identity
    returns False and writes NO slot_identity entry. Before the fix rebirth returned True
    unconditionally and retask_slot recorded a host that was never running → warm_identity(idx)
    short-circuited HOSTED forever, wedging the promote track (423 'HOSTED but not yet warm_ready'
    over 766 epochs while docker had no such container)."""
    hp, s, r = _hp(next_joiner=99)           # refill exhausted → rotation slots not reserved
    hp.p.run_ok = lambda argv, **kw: not ("up" in argv and "--force-recreate" in argv)  # only up fails
    assert hp.warm_identity(6) is False      # bench idx 6 (>= val_containers=6) — needs a rebirth host
    assert s.container.slot_identity == {}   # NO false-HOSTED entry recorded


def test_warm_identity_records_host_when_resume_up_succeeds():
    """The success path still works: a resume that comes up IS recorded (retask), so a healthy
    rebirth continues to host the bench identity (guards against Fix B over-rejecting)."""
    hp, s, r = _hp(next_joiner=99)
    hp.p.run_ok = lambda argv, **kw: True    # every resume comes up cleanly
    assert hp.warm_identity(6) is True
    assert "6" in [str(v) for v in s.container.slot_identity.values()]
