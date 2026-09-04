#!/usr/bin/env python3
"""A stand-in consensus layer over the engine API: builds a block on a parent.

Version split on this chainspec (Osaka@0): forkchoiceUpdatedV3 (there is no V5),
getPayloadV5, newPayloadV4 (no V5 — checked against error codes -32601/-38005).
"""
import json, time


def build_on(engine, parent, *, ts_bump=1, fee="0x" + "22" * 20, randao="0x" + "11" * 32,
             head_for_attrs=None):
    """Build a block on `parent` (a block object). Returns (payload, newPayload reply)."""
    attrs = {"timestamp": hex(int(parent["timestamp"], 16) + ts_bump),
             "prevRandao": randao, "suggestedFeeRecipient": fee,
             "withdrawals": [], "parentBeaconBlockRoot": "0x" + "00" * 32}
    r = engine.fcu(head_for_attrs or parent["hash"], attrs=attrs, version="V3")
    pid = (r.get("result") or {}).get("payloadId")
    if not pid:
        return None, r
    time.sleep(0.7)
    g = engine.call("engine_getPayloadV5", [pid])
    if "result" not in g:
        return None, g
    env = g["result"]
    payload = env["executionPayload"]
    reqs = env.get("executionRequests", [])
    npr = engine.call("engine_newPayloadV4",
                      [payload, [], "0x" + "00" * 32, reqs])
    return payload, npr


def canonicalize(engine, payload, finalized=None):
    return engine.fcu(payload["blockHash"], finalized=finalized)
