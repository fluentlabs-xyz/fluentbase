#!/usr/bin/env python3
"""WebSocket man-in-the-middle that corrupts finality certificates.

Sits between a `--cert-follow` node and an upstream `consensus` RPC. Relays
every frame verbatim EXCEPT it flips one hex nibble in every `certificate`
field (subscription notifications + getFinalization responses). A correct
follower must reject the tampered cert (its BLS multisig no longer verifies
against the on-chain committee) and make ZERO finalized progress — the proof
that the follower verifies rather than trusts.

Dependencies: `websockets` (pure-python). stdlib otherwise.
"""

import argparse
import asyncio
import json

import websockets


_HEX = set("0123456789abcdefABCDEF")


def _flip_cert(value):
    """Flip one hex nibble in the SIGNATURE region of a certificate hex string.

    The cert wire form is `hex(Finalization.encode())` — bare hex, NO `0x` prefix
    (see `certified_block.rs::from_parts`). The encoding is `proposal ‖ certificate`,
    so the BLS aggregate signature sits in the trailing bytes. Flip a nibble 8 chars
    from the end (well inside the 48-byte MinSig signature), NOT offset 0 — offset 0
    is the proposal's epoch uvarint, and corrupting that hits a different code path
    (epoch reinterpretation) rather than a clean signature-verification failure.
    """
    if not isinstance(value, str):
        return value
    body = value[2:] if value.startswith("0x") else value
    if len(body) < 16 or any(c not in _HEX for c in body):
        return value
    idx = len(body) - 8
    flipped = "1" if body[idx].lower() != "1" else "2"
    body = body[:idx] + flipped + body[idx + 1 :]
    return ("0x" + body) if value.startswith("0x") else body


def _tamper(node):
    """Recursively rewrite every `certificate` field in a decoded JSON value."""
    if isinstance(node, dict):
        return {
            k: (_flip_cert(v) if k == "certificate" else _tamper(v))
            for k, v in node.items()
        }
    if isinstance(node, list):
        return [_tamper(v) for v in node]
    return node


def _corrupt(raw):
    try:
        msg = json.loads(raw)
    except (ValueError, TypeError):
        return raw  # not JSON — pass through untouched
    return json.dumps(_tamper(msg))


async def _relay(name, src, dst, transform):
    try:
        async for frame in src:
            await dst.send(transform(frame))
    except websockets.ConnectionClosed:
        pass


async def _handle(client, upstream_url):
    async with websockets.connect(upstream_url, max_size=128 * 1024 * 1024) as upstream:
        await asyncio.gather(
            _relay("c2u", client, upstream, lambda f: f),
            _relay("u2c", upstream, client, _corrupt),
        )


async def _main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--listen", default="0.0.0.0:8546")
    parser.add_argument("--upstream", required=True)
    args = parser.parse_args()
    host, port = args.listen.rsplit(":", 1)

    async def handler(client):
        await _handle(client, args.upstream)

    print(f"cert-mitm: listening {args.listen} -> {args.upstream} (corrupting certs)", flush=True)
    async with websockets.serve(handler, host, int(port), max_size=128 * 1024 * 1024):
        await asyncio.Future()  # run forever


if __name__ == "__main__":
    asyncio.run(_main())
