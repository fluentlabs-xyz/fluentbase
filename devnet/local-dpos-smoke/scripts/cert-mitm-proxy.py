#!/usr/bin/env python3
"""WebSocket man-in-the-middle that corrupts finality certificates.

Sits between a `--cert-follow` node and an upstream `consensus` RPC. Relays
every frame verbatim EXCEPT the `certificate` fields (subscription
notifications + getFinalization responses), which it rewrites according to
`--mode`.

═══ THE TWO MODES TEST TWO DIFFERENT ARMS, AND ONE CANNOT SUBSTITUTE FOR THE OTHER ═══

  whole-cert  (default, `smoke-cert-follow` phase 3)
      Flip one hex nibble 4 bytes from the end. The follower must make ZERO
      finalized progress. The rejection lands at DECODE: those trailing bytes
      are the compressed G1 seed slot, and `BlsSignature::read` uncompresses +
      subgroup-checks it, so the flip fails `into_parts()` with probability ~1
      and the certificate never reaches `CertInlet::ingest`.

  seed-slot  (`smoke-cert-follow` phase 4)
      CLEAR the seed slot: write the 1-byte present flag to 0 and the 48-byte
      slot to zeroes. That is a WELL-FORMED, decodable certificate — `Some`
      becomes `None` through `read_seed_slot` — whose multisig quorum is
      untouched and still verifies. It is refused only by the arm that says a
      beacon-active epoch's certificate MUST carry a seed
      (`combined_scheme.rs`'s `verify_certificate`, the `None => false` arm),
      and that arm is reachable on any scheme that carries an ORACLE, i.e. on
      any beacon-active epoch. A follower on a PRE-BEACON epoch accepts this
      certificate. That is what makes it the test for the seed check and what
      makes `whole-cert` useless for it: a decode failure proves nothing about
      the seed check.

      NARROWED BY FLU-1202. The arm used to be reachable only on a scheme
      carrying `cert_seed_pin`, so a cleared slot separated a follower that had
      obtained `PK_epoch` from one that had not. A scheme holds no key material
      now, and `Randomness::oracle_for` attaches an oracle for every
      beacon-active epoch regardless of whether the key resolves, so the
      keyed/keyless distinction is observed by `smoke-cert-keyless` instead.

═══ THE WIRE ═══════════════════════════════════════════════════════════════════

`certificate` is `hex(Finalization.encode())` — bare hex, no `0x`
(`certified_block.rs::from_parts`) — and `Finalization` encodes as
`proposal ‖ CombinedCertificate`. `CombinedCertificate` is
`VoteCertificate ‖ seed_flag(1 B) ‖ seed_slot(48 B)`
(`crates/dpos/bls/src/combined_scheme.rs`: `write_seed_slot`, `SEED_FLAG`,
`SEED_SLOT = SIGNATURE_BYTES = 48`). Everything before the trailing 49 bytes is
variable-length, so the trailing 49 are the only part with a fixed offset — and
they are the only part either mode needs. The multisig aggregate lives inside
`VoteCertificate`, ahead of them, which is why clearing the slot leaves the
quorum valid.

═══ ARMING (seed-slot only) ════════════════════════════════════════════════════

`observe_cert` — the follower's ONLY trigger for fetching `PK_epoch` — runs
AFTER a certificate verifies (`cert_inlet.rs`: "skipped/tampered certs above
never reach here"). A proxy that cleared the seed slot from its first frame
would therefore keep the follower vote-only forever: it would never verify a
certificate, never ask for the artifact, never pin, and so never reject
anything. The negative would pass for the wrong reason, or rather fail to.

So `seed-slot` relays VERBATIM until `--arm-file` appears, and the harness
creates that file only once the follower has logged that it holds `PK_epoch`.
The transition is announced, and every cleared certificate is counted; the
first one prints its before/after bytes. That readback is the proxy's own
proof that its tampering took effect — a mode that silently cleared nothing
would otherwise read as a follower that rejected nothing.

Dependencies: `websockets` (pure-python). stdlib otherwise.
"""

import argparse
import asyncio
import json
import os

import websockets


_HEX = set("0123456789abcdefABCDEF")

#: `combined_scheme.rs` — `SEED_FLAG` (1) + `SEED_SLOT` (48), in BYTES, at the
#: very end of the encoded certificate.
SEED_TAIL_BYTES = 1 + 48
#: The same span in HEX CHARACTERS, which is what the wire field is made of.
SEED_TAIL_HEX = SEED_TAIL_BYTES * 2
#: A cleared slot: flag 0, then 48 zero bytes.
CLEARED_TAIL_HEX = "0" * SEED_TAIL_HEX

#: The lines the harness greps. Changing either one silently disarms
#: `verdicts_follow`'s witnesses, so they are named here and quoted there.
READY_LINE = "cert-mitm: listening"
ARMED_LINE = "cert-mitm: ARMED — clearing the seed slot of every certificate from here on"
CLEARED_LINE = "cert-mitm: seed slot CLEARED"
ALREADY_SEEDLESS_LINE = "cert-mitm: certificate was ALREADY seedless"


def _body(value):
    """The bare hex body of a `certificate` field, or None when it is not one.

    `None` for a non-string, a too-short string, or anything with a non-hex
    character: those are pass-through, never a silent partial rewrite."""
    if not isinstance(value, str):
        return None
    body = value[2:] if value.startswith("0x") else value
    if len(body) < 16 or any(c not in _HEX for c in body):
        return None
    return body


def _rewrap(value, body):
    return ("0x" + body) if value.startswith("0x") else body


def _flip_cert(value):
    """`whole-cert` — flip one hex nibble in the SIGNATURE region.

    A nibble 8 chars from the end, well inside the trailing 48-byte G1 slot.
    NOT offset 0: that is the proposal's epoch uvarint, and corrupting it hits
    epoch reinterpretation rather than a clean verification failure."""
    body = _body(value)
    if body is None:
        return value
    idx = len(body) - 8
    flipped = "1" if body[idx].lower() != "1" else "2"
    return _rewrap(value, body[:idx] + flipped + body[idx + 1:])


class SeedSlotTamper:
    """`seed-slot` — clear the trailing flag+slot once armed, and SAY SO.

    Stateful because the mode is: it counts what it changed and what it could
    not change, and it prints the first rewrite's before/after so a run can be
    audited without trusting the counter."""

    def __init__(self, arm_file):
        self.arm_file = arm_file
        self.armed = arm_file is None
        self.cleared = 0
        self.already_seedless = 0
        self.printed_witness = False

    def _check_arm(self):
        if self.armed:
            return True
        if self.arm_file and os.path.exists(self.arm_file):
            self.armed = True
            print(ARMED_LINE, flush=True)
        return self.armed

    def __call__(self, value):
        if not self._check_arm():
            return value
        body = _body(value)
        if body is None or len(body) < SEED_TAIL_HEX:
            return value
        before = body[-SEED_TAIL_HEX:]
        if before == CLEARED_TAIL_HEX:
            # A fallback (no-beacon) epoch's certificate carries no seed to
            # clear. Rewriting it would be a NO-OP dressed as a tamper, and a
            # phase that concluded "rejected" from one of these would be
            # concluding it from an untouched certificate.
            self.already_seedless += 1
            if self.already_seedless == 1:
                print(f"{ALREADY_SEEDLESS_LINE} (flag+slot were already zero) — nothing to "
                      "clear on this one; it is relayed unchanged", flush=True)
            return value
        after = body[:-SEED_TAIL_HEX] + CLEARED_TAIL_HEX
        # THE READBACK IS THE ASSERTION. Re-slice the string that will actually
        # go on the wire rather than trusting the concatenation above.
        tail = after[-SEED_TAIL_HEX:]
        if tail != CLEARED_TAIL_HEX or len(after) != len(body):
            print(f"cert-mitm: REFUSING to relay a botched rewrite (tail={tail!r}, "
                  f"len {len(body)} -> {len(after)}) — relaying the original", flush=True)
            return value
        self.cleared += 1
        if not self.printed_witness:
            self.printed_witness = True
            print(f"{CLEARED_LINE} (first): {before} -> {tail} (multisig quorum untouched)",
                  flush=True)
        elif self.cleared % 25 == 0:
            print(f"{CLEARED_LINE}: {self.cleared} so far", flush=True)
        return _rewrap(value, after)


def _tamper(node, on_cert):
    """Recursively rewrite every `certificate` field in a decoded JSON value."""
    if isinstance(node, dict):
        return {
            k: (on_cert(v) if k == "certificate" else _tamper(v, on_cert))
            for k, v in node.items()
        }
    if isinstance(node, list):
        return [_tamper(v, on_cert) for v in node]
    return node


def _corrupt(raw, on_cert):
    try:
        msg = json.loads(raw)
    except (ValueError, TypeError):
        return raw  # not JSON — pass through untouched
    return json.dumps(_tamper(msg, on_cert))


async def _relay(name, src, dst, transform):
    try:
        async for frame in src:
            await dst.send(transform(frame))
    except websockets.ConnectionClosed:
        pass


async def _handle(client, upstream_url, on_cert):
    async with websockets.connect(upstream_url, max_size=128 * 1024 * 1024) as upstream:
        await asyncio.gather(
            _relay("c2u", client, upstream, lambda f: f),
            _relay("u2c", upstream, client, lambda f: _corrupt(f, on_cert)),
        )


async def _main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--listen", default="0.0.0.0:8546")
    parser.add_argument("--upstream", required=True)
    parser.add_argument("--mode", choices=("whole-cert", "seed-slot"), default="whole-cert",
                        help="whole-cert: flip a nibble (fails DECODE). "
                             "seed-slot: clear flag+slot (fails the SEED arm, quorum intact).")
    parser.add_argument("--arm-file", default=None,
                        help="seed-slot only: relay verbatim until this path exists.")
    args = parser.parse_args()
    host, port = args.listen.rsplit(":", 1)

    on_cert = _flip_cert if args.mode == "whole-cert" else SeedSlotTamper(args.arm_file)
    arming = f", armed by {args.arm_file}" if args.mode == "seed-slot" and args.arm_file else ""

    async def handler(client):
        await _handle(client, args.upstream, on_cert)

    print(f"{READY_LINE} {args.listen} -> {args.upstream} (mode={args.mode}{arming})", flush=True)
    async with websockets.serve(handler, host, int(port), max_size=128 * 1024 * 1024):
        await asyncio.Future()  # run forever


if __name__ == "__main__":
    asyncio.run(_main())
