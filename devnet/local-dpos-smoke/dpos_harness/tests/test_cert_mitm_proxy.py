"""`scripts/cert-mitm-proxy.py` — the two tamper modes, at the byte level.

WHY THIS FILE EXISTS. The proxy stopped being a one-liner when phase 4 needed a mode that
corrupts the SEED SLOT ONLY. That mode carries fixed byte offsets read off the Rust wire format
(`crates/dpos/bls/src/combined_scheme.rs`: `CombinedCertificate` is
`VoteCertificate ‖ seed_flag(1 B) ‖ seed_slot(48 B)`), and offsets in a sidecar that runs inside a
container are the classic thing nobody notices going wrong: a proxy that rewrites the wrong bytes
still relays, the follower still refuses (for a different reason, or none), and the phase still
looks like it proved something.

The proxy prints its own readback for that reason, and `verdicts_follow.evaluate_seed_tamper_landed`
gates on the print. This file is the other half: it drives the rewrite directly, so the offsets are
checked against the format rather than against the proxy agreeing with itself.

The module is loaded BY PATH with `websockets` stubbed — it lives in `scripts/`, outside the
package, and its only dependency is one the sidecar pip-installs at boot.
"""

from __future__ import annotations

import importlib.util
import pathlib
import sys
import types

import pytest

_PROXY = (pathlib.Path(__file__).resolve().parents[2] / "scripts" / "cert-mitm-proxy.py")


@pytest.fixture(scope="module")
def proxy():
    stub = types.ModuleType("websockets")
    stub.ConnectionClosed = type("ConnectionClosed", (Exception,), {})
    stub.connect = stub.serve = None
    sys.modules.setdefault("websockets", stub)
    spec = importlib.util.spec_from_file_location("cert_mitm_proxy", _PROXY)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


#: A stand-in `certificate` field: a variable-length prefix (the proposal + the multisig vote
#: certificate) followed by the fixed 49-byte tail the two modes care about.
PREFIX = "ab" * 80
#: flag = 1 (a seed IS present) + 48 non-zero bytes standing in for the compressed G1 point.
LIVE_TAIL = "01" + "cd" * 48
SEEDLESS_TAIL = "00" + "00" * 48


def test_the_tail_span_matches_the_rust_wire_format(proxy):
    """`SEED_FLAG` (1) + `SEED_SLOT` (`SIGNATURE_BYTES` = 48). If `write_seed_slot` ever grows a
    field, this is the constant that has to move with it, and the seed-slot mode would otherwise
    silently start clearing the tail of the multisig aggregate instead."""
    assert proxy.SEED_TAIL_BYTES == 49
    assert proxy.SEED_TAIL_HEX == 98
    assert proxy.CLEARED_TAIL_HEX == "0" * 98
    assert len(LIVE_TAIL) == proxy.SEED_TAIL_HEX


def test_seed_slot_mode_relays_verbatim_until_it_is_armed(proxy, tmp_path):
    """THE PASS-THROUGH WINDOW IS LOAD-BEARING. `observe_cert` — the follower's only trigger for
    fetching the artifact — runs after a certificate VERIFIES, so a proxy that tampered from its
    first frame would keep the follower vote-only forever and the negative could never fire."""
    arm = tmp_path / "arm"
    t = proxy.SeedSlotTamper(str(arm))
    cert = PREFIX + LIVE_TAIL
    assert t(cert) == cert and t.cleared == 0
    arm.write_text("1")
    assert t(cert) != cert and t.cleared == 1


def test_seed_slot_mode_clears_the_flag_and_the_slot_and_NOTHING_else(proxy, tmp_path):
    """The quorum must survive. The multisig aggregate lives inside `VoteCertificate`, ahead of
    the tail, so a cleared certificate still verifies its quorum and is refused ONLY by the arm
    that says a beacon-active epoch must carry a seed. Touching a byte of the prefix would turn
    this back into phase 3's test."""
    arm = tmp_path / "arm"
    arm.write_text("1")
    t = proxy.SeedSlotTamper(str(arm))
    out = t(PREFIX + LIVE_TAIL)
    assert out[:-98] == PREFIX
    assert out[-98:] == proxy.CLEARED_TAIL_HEX
    assert len(out) == len(PREFIX + LIVE_TAIL)


def test_seed_slot_mode_counts_an_ALREADY_seedless_certificate_apart(proxy, tmp_path):
    """A fallback (no-beacon) epoch's certificate has nothing to clear. Rewriting it would be a
    NO-OP dressed as a tamper, and a phase that concluded "rejected" from one of these would be
    concluding it from an untouched certificate."""
    arm = tmp_path / "arm"
    arm.write_text("1")
    t = proxy.SeedSlotTamper(str(arm))
    cert = PREFIX + SEEDLESS_TAIL
    assert t(cert) == cert
    assert t.cleared == 0 and t.already_seedless == 1


def test_seed_slot_mode_prints_the_first_rewrite_as_its_own_readback(proxy, tmp_path, capsys):
    """THE TAMPER MUST WITNESS ITS OWN TAMPERING (the `tear_journal_to_torn` rule). The harness
    gates on these two lines, so a run in which nothing was cleared is a red case rather than a
    still follower nobody can explain."""
    arm = tmp_path / "arm"
    t = proxy.SeedSlotTamper(str(arm))
    t(PREFIX + LIVE_TAIL)
    arm.write_text("1")
    t(PREFIX + LIVE_TAIL)
    out = capsys.readouterr().out
    assert proxy.ARMED_LINE in out
    assert proxy.CLEARED_LINE in out
    # The before/after bytes, so the print can be audited without trusting the counter.
    assert "cd" * 48 in out and proxy.CLEARED_TAIL_HEX in out


@pytest.mark.parametrize("value", [None, 42, "", "zz" * 60, "abc"])
def test_neither_mode_rewrites_something_that_is_not_a_certificate(proxy, tmp_path, value):
    """Non-strings, short strings and non-hex are PASS-THROUGH, never a silent partial rewrite:
    a proxy that mangled an unrelated field would break the stream for a reason no verdict names."""
    arm = tmp_path / "arm"
    arm.write_text("1")
    assert proxy.SeedSlotTamper(str(arm))(value) == value
    assert proxy._flip_cert(value) == value


def test_whole_cert_mode_still_flips_the_nibble_phase_3_depends_on(proxy):
    """Phase 3 is untouched. Its flip lands 4 bytes from the end — inside the compressed G1 point
    — so `BlsSignature::read`'s uncompress + subgroup check fails and the certificate never
    reaches `CertInlet::ingest`. That is a DECODE failure and it is why phase 3 cannot test the
    seed arm, which is the whole reason the second mode exists."""
    cert = PREFIX + LIVE_TAIL
    out = proxy._flip_cert(cert)
    assert out != cert and len(out) == len(cert)
    diff = [i for i, (a, b) in enumerate(zip(cert, out)) if a != b]
    assert diff == [len(cert) - 8]


def test_the_transform_reaches_every_certificate_field_in_a_decoded_frame(proxy, tmp_path):
    """`_corrupt` walks the JSON: a `certificate` nested under `params.result` (the subscription
    notification) and one under `result` (the by-height pull) are two different shapes and the
    follower reads both."""
    import json
    arm = tmp_path / "arm"
    arm.write_text("1")
    t = proxy.SeedSlotTamper(str(arm))
    frame = json.dumps({"params": {"result": {"certificate": PREFIX + LIVE_TAIL,
                                              "block": "dead"}},
                        "result": [{"certificate": PREFIX + LIVE_TAIL}]})
    out = json.loads(proxy._corrupt(frame, t))
    assert t.cleared == 2
    assert out["params"]["result"]["certificate"].endswith(proxy.CLEARED_TAIL_HEX)
    assert out["params"]["result"]["block"] == "dead"
    assert out["result"][0]["certificate"].endswith(proxy.CLEARED_TAIL_HEX)


def test_a_non_json_frame_is_relayed_untouched(proxy, tmp_path):
    arm = tmp_path / "arm"
    arm.write_text("1")
    assert proxy._corrupt("not json at all", proxy.SeedSlotTamper(str(arm))) == "not json at all"
