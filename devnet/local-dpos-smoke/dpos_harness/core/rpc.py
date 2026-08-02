"""rpc.py — JSON-RPC-over-HTTP transport + read-only cast/docker helpers.

Python port of scripts/rpc.sh (the curl incantation, its FLK-2 timeouts, the
empty-response coercion, the block-field getters, the quarantined cast parse)
plus strip_ansi (lib.sh). This is the ONE place the observer's transport lives.

Transport rules preserved VERBATIM from the bash originals (do not "simplify"
these away):
  - host reads add a hard time bound (FLK-2: an up-but-CPU-starved node accepts
    the socket then never answers; without a bound the whole tick hangs);
  - in-container reads add an OUTER timeout around the whole `docker compose exec`
    (an unresponsive docker daemon/container can hang the exec itself); the inner
    curl bound (10s) < the outer exec timeout (15s) so the inner bound normally
    fires first (clean sentinel), the outer timeout is the backstop;
  - an empty/failed response is coerced to `{}` here so a caller parsing JSON
    always gets a clean object, never a mid-window abort.

PORT-NOTE: bash used `curl --max-time N --connect-timeout M`. urllib has no
separate connect-timeout knob, so the single urllib timeout is set to RPC_MAX_TIME
(the total bound bash's --max-time enforced); the connect phase is covered by the
same overall bound. Behaviorally: same hard ceiling on a hung read.

This module shells out to `docker`/`cast` for in-container and cast reads exactly
as bash does (keeps operator CLI-version parity; each call site stays a literal
translation of the bash command). It performs READS ONLY — the docker verbs used
are `compose exec`/`compose logs` running curl/cat/cast, never up/down/restart/
stop/start/rm/kill/force-recreate.

Every one of those spawns goes through `proc.read` — the package's single exec
seam, on its non-recording ambient channel: same env merge (the ambient
COMPOSE_FILE that resolves the sim project), same timeout discipline, and
nothing added to the write transcript.
"""

from __future__ import annotations

import json
import os
import re
import urllib.error
import urllib.request

from . import proc, topology

RPC_MAX_TIME = float(os.environ.get("RPC_MAX_TIME", "10"))
RPC_CONNECT_TIMEOUT = float(os.environ.get("RPC_CONNECT_TIMEOUT", "3"))
RPC_EXEC_TIMEOUT = float(os.environ.get("RPC_EXEC_TIMEOUT", "15"))

# `docker compose` base — honors the ambient COMPOSE_FILE / project env exactly
# like the bash `docker compose ...` invocations the sim exports.
DOCKER_COMPOSE = ["docker", "compose"]

_ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


def strip_ansi(text: str) -> str:
    """Strip SGR/ANSI escape sequences (lib.sh strip_ansi). MANDATORY before ANY
    machine-parse of container logs: the fluent node emits APPLICATION-EMBEDDED
    ANSI around every structured key=value pair, so `height` ESC `=` ESC `2295`
    would never match a `grep -F "height=2295"`. `docker compose logs --no-color`
    does NOT help (it suppresses only compose's own prefix coloring). Byte-identical
    to the retired bash copy."""
    return _ANSI_RE.sub("", text)


def rpc_body(method: str, params=None) -> str:
    """Build a JSON-RPC request body. params = raw params list (default [])."""
    if params is None:
        params = []
    return json.dumps(
        {"jsonrpc": "2.0", "method": method, "params": params, "id": 1},
        separators=(",", ":"),
    )


def rpc_post_url(url: str, body: str) -> str:
    """POST a JSON-RPC body to a host URL. Returns the raw response JSON, `{}` if
    empty/failed (the empty->{} coercion of rpc_post_url)."""
    req = urllib.request.Request(
        url,
        data=body.encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=RPC_MAX_TIME) as resp:
            out = resp.read().decode(errors="replace")
    except Exception:
        out = ""
    return out if out else "{}"


def _run_read(cmd, timeout: float) -> str:
    """Run a read-only command (docker exec/logs, cast); return stdout, "" on any
    failure/timeout. Mirrors the bash `... 2>/dev/null || true` discipline. Kept as a
    named local alias because every reader in this module and in nodes.py calls it, and
    because the `_run_read`/`RPC_EXEC_TIMEOUT` pairing is the FLK-2 bound the module
    docstring commits to; the exec itself belongs to `proc.read`."""
    return proc.read(cmd, timeout=timeout)


def rpc_post_exec(body: str, exec_prefix) -> str:
    """POST a JSON-RPC body to a node's in-container RPC (:8545) through a command
    prefix (e.g. ["docker","compose","exec","-T","validator-2"]). Outer-bounded by
    the exec timeout. Returns raw JSON, `{}` if empty/failed."""
    cmd = list(exec_prefix) + [
        "curl", "-s",
        "--max-time", str(int(RPC_MAX_TIME)),
        "--connect-timeout", str(int(RPC_CONNECT_TIMEOUT)),
        "-X", "POST",
        "-H", "Content-Type: application/json",
        "--data", body,
        topology.IN_CONTAINER_RPC_URL,
    ]
    out = _run_read(cmd, RPC_EXEC_TIMEOUT)
    return out if out else "{}"


def metrics_get_url(url: str) -> str:
    """GET a plain-text endpoint (Prometheus /metrics) at a host URL. "" if
    unreachable."""
    req = urllib.request.Request(url, method="GET")
    try:
        with urllib.request.urlopen(req, timeout=RPC_MAX_TIME) as resp:
            return resp.read().decode(errors="replace")
    except Exception:
        return ""


def metrics_get_exec(url: str, exec_prefix) -> str:
    """GET a plain-text endpoint via a command prefix. Outer-bounded like every
    exec read. "" if unreachable."""
    cmd = list(exec_prefix) + [
        "curl", "-s",
        "--max-time", str(int(RPC_MAX_TIME)),
        "--connect-timeout", str(int(RPC_CONNECT_TIMEOUT)),
        url,
    ]
    return _run_read(cmd, RPC_EXEC_TIMEOUT)


_ENODE_PUBKEY_RE = re.compile(r"^enode://([0-9a-fA-F]+)@")


def _enode_pubkey(url: str) -> str:
    """The 128-hex secp256k1 node pubkey of the reth node at host RPC `url`, via
    admin_nodeInfo (faithful port of lib.sh `_enode_pubkey`). Returns "" when the
    RPC is down, admin_nodeInfo is unavailable, or the enode is absent/malformed.

    `cast rpc admin_nodeInfo` unwraps `.result`, so the object's `.enode` is the
    full `enode://<128hex>@<ip>:<port>`; we keep ONLY the 128-hex pubkey (the
    caller rebuilds the enode against the node's fixed compose IP — admin_nodeInfo's
    embedded IP is unreliable inside docker). The length is validated HERE (bash
    asserted `^[0-9a-fA-F]{128}$` at the call site) so callers only test truthiness."""
    out = _run_read(["cast", "rpc", "--rpc-url", url, "admin_nodeInfo"], RPC_EXEC_TIMEOUT)
    try:
        obj = json.loads(out) if out.strip() else {}
    except Exception:
        return ""
    enode = obj.get("enode") if isinstance(obj, dict) else None
    if not enode:
        return ""
    m = _ENODE_PUBKEY_RE.match(enode)
    if not m:
        return ""
    pk = m.group(1)
    return pk if len(pk) == 128 else ""


def compose_exec(service: str):
    """The `docker compose exec -T <service>` prefix used by every in-container
    read (a READ prefix — the caller always appends curl/cat/cast, never a write)."""
    return DOCKER_COMPOSE + ["exec", "-T", service]


def cast_field(lineno: int, text: str) -> str:
    """The <lineno>'th line of a multi-line `cast call ...(typeN)` output, whitespace-
    stripped (CQ-M2 quarantine). `cast` prints one value per line, right-justified
    with leading spaces; a multi-field tuple prints one field per line. One
    definition so a foundry output-format change is fixed in ONE place."""
    lines = text.split("\n")
    if 1 <= lineno <= len(lines):
        return lines[lineno - 1].replace(" ", "").strip()
    return ""


# --- block-field getters (CQ-H4) -------------------------------------------
# GRACEFUL everywhere: a not-yet-synced block / unreachable RPC yields "null"
# (never aborts), so callers handle a lagging node cleanly. All results lowercased.

def _jq_field(obj: dict, path: str):
    cur = obj
    for part in path.split("."):
        if not isinstance(cur, dict) or part not in cur or cur[part] is None:
            return None
        cur = cur[part]
    return cur


def block_field_at(block, field: str, rpc_url: str) -> str:
    """Field of block <block> via the HOST `cast block` (validator-0/full-node
    publish a host RPC). `cast block --json` returns the block object at top level,
    so the field is `.<field>` (no `.result` wrapper). Lowercased; "null" on miss."""
    out = _run_read(
        ["cast", "block", str(block), "--rpc-url", rpc_url, "--json"],
        RPC_EXEC_TIMEOUT,
    )
    try:
        obj = json.loads(out) if out.strip() else {}
    except Exception:
        obj = {}
    val = _jq_field(obj, field) if isinstance(obj, dict) else None
    return (str(val) if val is not None else "null").lower()


def block_field_in(service: str, block_dec: int, field: str) -> str:
    """Field of block <block_dec> as seen INSIDE container <service> (validators
    with no host RPC), via an in-container eth_getBlockByNumber. Result path is
    `.result.<field>`. Lowercased; "null" on miss."""
    hexn = hex(int(block_dec))
    out = rpc_post_exec(
        rpc_body("eth_getBlockByNumber", [hexn, False]), compose_exec(service)
    )
    try:
        obj = json.loads(out)
    except Exception:
        obj = {}
    val = _jq_field(obj, "result." + field) if isinstance(obj, dict) else None
    return (str(val) if val is not None else "null").lower()


def block_field_of(service: str, block, field: str, rpc_url_default: str) -> str:
    """Field of block <block> as seen by node <service> — routes to the host RPC for the
    two services this reader has ever been asked for (the pinned RPC host -> the ambient
    default RPC, which an operator may repoint; the full-node -> its host L2 port), else
    the in-container probe.

    `downstream` also publishes a host RPC (topology.HOST_RPC_PORTS) but is deliberately
    NOT routed to it here: no caller reads block fields off the L3 tier, and widening this
    to the whole map would silently change which transport an L3 read takes."""
    if topology.is_pinned_rpc_host(service):
        return block_field_at(block, field, rpc_url_default)
    if service == topology.FULL_NODE:
        return block_field_at(block, field, topology.host_url(topology.HOST_L2_RPC_PORT))
    return block_field_in(service, block, field)
