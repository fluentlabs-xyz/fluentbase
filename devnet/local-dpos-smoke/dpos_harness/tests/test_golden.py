"""Pure-logic tests for the golden snapshot invalidation + freshness core (no docker / no chain).

Covers the two pure functions golden.py exposes for the hash-keyed cache: compute_hash
(determinism + sensitivity to each state-determining input) and is_golden_fresh_logic (the
present-tarball AND matching-sidecar rule). The live build/restore is exercised by
`python -m dpos_harness stack golden` + `SIM_USE_GOLDEN=1 case growth` against the real devnet — NOT here."""

from __future__ import annotations

from dpos_harness.stack.golden import compute_hash, is_golden_fresh_logic

_IMG = "sha256:1543300f637108e4dd274308c2382c8c9b0b945a6f682b59f59455de5102dc42"
_SHA = "1aaa2576f7a6702246c0792e6270b71112b3aae8"
_CFG = {"validators": 6, "initial_committee": 4, "val_containers": 6,
        "identity_pool": 8, "epoch_interval": 32}


def test_compute_hash_is_deterministic():
    assert compute_hash(_IMG, _SHA, _CFG) == compute_hash(_IMG, _SHA, _CFG)


def test_compute_hash_is_key_order_independent():
    reordered = {"epoch_interval": 32, "val_containers": 6, "validators": 6,
                 "identity_pool": 8, "initial_committee": 4}
    assert compute_hash(_IMG, _SHA, _CFG) == compute_hash(_IMG, _SHA, reordered)


def test_compute_hash_changes_with_image():
    assert compute_hash("sha256:deadbeef", _SHA, _CFG) != compute_hash(_IMG, _SHA, _CFG)


def test_compute_hash_changes_with_vendor_sha():
    assert compute_hash(_IMG, "0000000000000000000000000000000000000000", _CFG) \
        != compute_hash(_IMG, _SHA, _CFG)


def test_compute_hash_changes_with_topology():
    grown = dict(_CFG, validators=7)          # a topology change invalidates the snapshot
    assert compute_hash(_IMG, _SHA, grown) != compute_hash(_IMG, _SHA, _CFG)
    epoch = dict(_CFG, epoch_interval=64)      # an epoch-interval change too
    assert compute_hash(_IMG, _SHA, epoch) != compute_hash(_IMG, _SHA, _CFG)


def test_compute_hash_is_short_hex():
    h = compute_hash(_IMG, _SHA, _CFG)
    assert len(h) == 16
    int(h, 16)  # valid hex


def test_compute_hash_tolerates_vendor_whitespace():
    # the sidecar file may carry a trailing newline — it must not change the key
    assert compute_hash(_IMG, _SHA + "\n", _CFG) == compute_hash(_IMG, _SHA, _CFG)


def test_fresh_requires_tarball_present():
    h = compute_hash(_IMG, _SHA, _CFG)
    assert is_golden_fresh_logic(tarball_exists=False, sidecar_hash=h, current_hash=h) is False


def test_fresh_requires_matching_hash():
    h = compute_hash(_IMG, _SHA, _CFG)
    assert is_golden_fresh_logic(True, sidecar_hash=h, current_hash=h) is True
    assert is_golden_fresh_logic(True, sidecar_hash="stalehash0000000", current_hash=h) is False


def test_fresh_false_on_missing_sidecar():
    h = compute_hash(_IMG, _SHA, _CFG)
    assert is_golden_fresh_logic(True, sidecar_hash=None, current_hash=h) is False
    assert is_golden_fresh_logic(True, sidecar_hash="", current_hash=h) is False


def test_fresh_ignores_surrounding_whitespace():
    h = compute_hash(_IMG, _SHA, _CFG)
    assert is_golden_fresh_logic(True, sidecar_hash=h + "\n", current_hash=h) is True
