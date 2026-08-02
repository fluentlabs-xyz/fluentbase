"""compose_gen — generated topology, knobs, and the assert_generated truncation guard.

Validates the port of gen-soak-compose.sh: correct services/IPs, RUST_LOG_STYLE=never, log
bounds, and each knob (NO_CASCADE / rotation identity-indirection / PRUNE / DATA_ROOT / GEO). YAML
parse validity is asserted with PyYAML (skipped if absent)."""

from __future__ import annotations

import os

import pytest

from dpos_harness.stack import compose_gen
from dpos_harness.stack.compose_gen import ComposeGenError

try:
    import yaml
    HAVE_YAML = True
except ImportError:
    HAVE_YAML = False


def _clear_env():
    for k in ("SIM_ROTATION_SLOTS", "SIM_NO_CASCADE", "PEER_HOST_MODE", "BOOTSTRAP_MODE",
              "SIM_PRUNE_PROFILE", "SIM_DATA_ROOT", "SIM_GEO_LATENCY"):
        os.environ.pop(k, None)


def test_generate_basic(tmp_path):
    _clear_env()
    base, dpos = compose_gen.generate(6, out_dir=str(tmp_path))
    bt = open(base).read()
    dt = open(dpos).read()
    for i in range(6):
        assert f"validator-{i}:" in bt and f"validator-{i}:" in dt
    assert "full-node:" in bt and "downstream:" in dt
    assert 'RUST_LOG_STYLE: "never"' in bt
    assert 'max-size: "64m"' in bt and 'max-file: "5"' in bt
    # arithmetic IPs (the root-fix): validator-5 at 172.20.0.15, never 172.20.0.15-via-concat
    assert "172.20.0.15" in bt


@pytest.mark.skipif(not HAVE_YAML, reason="PyYAML not installed")
def test_generate_arity_oracle_no_phantom_containers(tmp_path):
    """The v61.9 regression: N=val_containers=14 (target=10, pool=20) must define EXACTLY
    validator-0..13 — NO validator-14..19 (identities >=14 are container-less bench idents,
    NOT containers). A phantom validator-16 is what the live node-down check tripped on."""
    _clear_env()
    base, dpos = compose_gen.generate(14, target=10, identity_pool=20, out_dir=str(tmp_path))
    bsvc = set(yaml.safe_load(open(base))["services"])
    dsvc = set(yaml.safe_load(open(dpos))["services"])
    assert bsvc == {"genesis-init", "full-node", *(f"validator-{i}" for i in range(14))}
    assert dsvc == {"full-node", "downstream", *(f"validator-{i}" for i in range(14))}
    for phantom in range(14, 20):                # pool 20 is an IDENTITY bound, not a container count
        assert f"validator-{phantom}" not in bsvc
        assert f"validator-{phantom}" not in dsvc


@pytest.mark.skipif(not HAVE_YAML, reason="PyYAML not installed")
def test_simconfig_full_defaults_generate_no_phantom(tmp_path):
    """End-to-end guard tying SimConfig defaults to generation (the seam the parity fixture
    bypassed by hardcoding N=14): clean-env SimConfig() → generate(val_containers,...) must
    NOT emit validator-14..19."""
    for k in ("SIM_QUICK", "SIM_VALIDATORS", "SIM_INITIAL_COMMITTEE", "SIM_IDENTITY_POOL",
              "SIM_SPARES", "SIM_ROTATION_SLOTS", "SIM_GEO_LATENCY", "SIM_NO_CASCADE"):
        os.environ.pop(k, None)
    from dpos_harness.sim.orchestrator import SimConfig
    cfg = SimConfig()
    assert (cfg.val_containers, cfg.validators, cfg.identity_pool) == (14, 10, 20)
    base, _ = compose_gen.generate(cfg.val_containers, cfg.validators, cfg.identity_pool,
                                   out_dir=str(tmp_path))
    bsvc = set(yaml.safe_load(open(base))["services"])
    assert {f"validator-{i}" for i in range(14)} <= bsvc
    assert not any(f"validator-{i}" in bsvc for i in range(14, 20))


@pytest.mark.skipif(not HAVE_YAML, reason="PyYAML not installed")
def test_generated_yaml_parses(tmp_path):
    _clear_env()
    base, dpos = compose_gen.generate(6, out_dir=str(tmp_path))
    for p in (base, dpos):
        docs = list(yaml.safe_load_all(open(p)))
        assert docs and "services" in docs[0]


def test_no_cascade_omits_fullnode_and_downstream(tmp_path):
    _clear_env()
    os.environ["SIM_NO_CASCADE"] = "1"
    base, dpos = compose_gen.generate(6, out_dir=str(tmp_path))
    assert "full-node:" not in open(base).read()
    assert "downstream:" not in open(dpos).read()


def test_rotation_identity_indirection(tmp_path):
    _clear_env()
    os.environ["SIM_ROTATION_SLOTS"] = "2"
    _, dpos = compose_gen.generate(6, out_dir=str(tmp_path))
    dt = open(dpos).read()
    # the compose literal-$ escape so the container's /bin/sh expands IDENTITY_IDX at RUNTIME
    assert "${IDENTITY_IDX:-1}" in dt          # non-v0 validators are indirected
    assert "keys/validator-0/bls.hex" in dt    # v0 is DELIBERATELY NOT indirected (hardcoded)


def test_data_root_bind_volume(tmp_path):
    _clear_env()
    root = tmp_path / "simdata"
    os.environ["SIM_DATA_ROOT"] = str(root)
    base, _ = compose_gen.generate(6, out_dir=str(tmp_path))
    bt = open(base).read()
    assert "driver: local" in bt and f"device: {root}/runtime" in bt
    assert (root / "runtime").is_dir()


def test_data_root_refuses_root_fs():
    _clear_env()
    os.environ["SIM_DATA_ROOT"] = "/"
    with pytest.raises(ComposeGenError):
        compose_gen.generate(6)
    os.environ.pop("SIM_DATA_ROOT")


def test_prune_archive_omits_full_flag(tmp_path):
    _clear_env()
    os.environ["SIM_PRUNE_PROFILE"] = "archive"
    base, _ = compose_gen.generate(6, out_dir=str(tmp_path))
    assert "--full" not in open(base).read()


def test_bounds_validation():
    _clear_env()
    with pytest.raises(ComposeGenError):
        compose_gen.generate(3)          # N < 4
    with pytest.raises(ComposeGenError):
        compose_gen.generate(52)         # N > 51
    with pytest.raises(ComposeGenError):
        compose_gen.generate(6, target=3)   # target < 4


def test_assert_generated_catches_truncation(tmp_path):
    p = tmp_path / "empty.yml"
    p.write_text("")
    with pytest.raises(ComposeGenError):
        compose_gen.assert_generated(str(p), "volumes:")
    p.write_text("services:\n  x: {}\n")
    with pytest.raises(ComposeGenError):
        compose_gen.assert_generated(str(p), "MISSING-MARKER")
