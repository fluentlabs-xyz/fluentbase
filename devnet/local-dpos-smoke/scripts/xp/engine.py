#!/usr/bin/env python3
"""Minimal engine-API client: HS256 JWT (no deps) + JSON-RPC over HTTP.

Usage as a library:  from engine import Engine; e = Engine("http://localhost:18551")
"""
import base64, hashlib, hmac, json, time, urllib.request

import os
from pathlib import Path

# JWT for the engine API. Defaults to a file beside this script (out/jwt.hex);
# the devnet stand does not create it — whoever runs an engine-API experiment
# puts it there.
JWT_PATH = os.environ.get("XP_JWT_PATH",
                          str(Path(__file__).resolve().parent / "out" / "jwt.hex"))


def _b64(b: bytes) -> str:
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


def jwt_token(secret_hex: str = None) -> str:
    if secret_hex is None:
        secret_hex = open(JWT_PATH).read().strip()
    key = bytes.fromhex(secret_hex)
    hdr = _b64(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
    pay = _b64(json.dumps({"iat": int(time.time())}, separators=(",", ":")).encode())
    sig = _b64(hmac.new(key, f"{hdr}.{pay}".encode(), hashlib.sha256).digest())
    return f"{hdr}.{pay}.{sig}"


class Engine:
    def __init__(self, url, auth=True):
        self.url, self.auth = url, auth
        self._id = 0

    def call(self, method, params=None, timeout=30):
        self._id += 1
        body = json.dumps({"jsonrpc": "2.0", "method": method,
                           "params": params or [], "id": self._id}).encode()
        hdrs = {"Content-Type": "application/json"}
        if self.auth:
            hdrs["Authorization"] = "Bearer " + jwt_token()
        req = urllib.request.Request(self.url, data=body, headers=hdrs)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            return {"http_error": e.code, "body": e.read().decode()[:400]}

    def fcu(self, head, safe=None, finalized=None, attrs=None, version="V3"):
        state = {"headBlockHash": head,
                 "safeBlockHash": safe if safe is not None else head,
                 "finalizedBlockHash": finalized if finalized is not None else head}
        return self.call(f"engine_forkchoiceUpdated{version}", [state, attrs])


ZERO = "0x" + "00" * 32
if __name__ == "__main__":
    import sys
    e = Engine(sys.argv[1])
    print(json.dumps(e.call(sys.argv[2], json.loads(sys.argv[3]) if len(sys.argv) > 3 else []), indent=2))
