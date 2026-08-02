"""topology.py — everything the harness KNOWS about the shape of a devnet.

Service naming, the host↔container port map, the docker network and its IP arithmetic,
and the chain identity. One home, so that changing the shape of a devnet is one edit
rather than a grep across six layers.

A pure leaf: imports nothing, reads no environment, performs no IO. Every value here is
a FACT about the compose topology `stack/compose_gen.py` emits — not a tunable. Knobs
stay where they are read (`SIM_*` in their consumers); this module is what those knobs
are read AGAINST.

═══ THE ASYMMETRY IS THE POINT ═══════════════════════════════════════════════════════

Nodes are NOT interchangeable, and flattening them is the way to break this quietly:

  * `validator-0` is the PINNED HOST-RPC NODE. It alone publishes its reth RPC (8545),
    its WS (8546) and BOTH metrics registries (19100→9100, 19200→9200) on the host, and
    it alone mounts `/runtime` for the `runtime_cat`/`runtime_write` seam and the
    funder key. Readers therefore reach it over the HOST while reaching every other
    committee container via `docker compose exec` — a difference in transport, in
    latency, and in what fails when it is down. `cases/quorum.py` and
    `core/setops.py::committee_victim_idxs` refuse to disrupt it for exactly that
    reason.
  * `validator-1` is the pinned CERT-UPSTREAM anchor (v0's `--dpos.follower-upstream`
    and the full-node's second `--cert-upstream`), so it is likewise never a churn
    victim (FLK-8).
  * `full-node` (L2 cert-follower, host 18545) and `downstream` (L3 cascade, host
    28545) publish a host RPC but no metrics, and are never committee members.
  * `genesis-init` is a one-shot: it runs to completion and stays exited. Lifecycle
    one-offs (`compose run --rm --no-deps`) borrow its service definition; it must
    never be treated as a node that is "down".

`HOST_RPC_PORTS` / `HOST_METRICS_PORTS` are that asymmetry as DATA. A service absent
from a map is reachable only in-container — which is a fact about it, not a gap.

The maps are LOAD-BEARING, not documentation: `stack/compose_gen.py` emits every
`ports:` stanza from them (via `host_rpc_publish` / `host_metrics_publish`), so a
service is published on the host IFF it appears here — which is what makes it safe for
`stack/bringup.py` to pick host-vs-`exec` transport off `has_host_rpc`. The corollary
is the hazard: ADDING a service to a map is a topology change, not a note. Adding it to
`HOST_RPC_PORTS` publishes its RPC and reroutes bring-up's height read; adding it to
`HOST_METRICS_PORTS` reroutes `nodes.node_metrics` off `docker compose exec` and onto a
host port. Both are pinned by tests (`test_compose_parity.py`'s port oracle,
`test_rpc.py::test_*metrics_transport*`) precisely so the widening cannot be silent.

═══ PREDEPLOYS ═══════════════════════════════════════════════════════════════════════

The `0x…52xx` addresses below are the genesis predeploy SLOTS. In this configuration
they are CODELESS — `genesis-bootstrap bare` installs an empty predeploy set and the
staking cluster is deployed at runtime by `DeployStaking`, so the live addresses are the
manifest's (`staking_rt` / `chain_config_rt` / `liveness_rt`), injected at the ctx
refresh. Naming them here must NOT make them a default anywhere: a read that falls back
to a codeless address returns empty and judges a live chain through a contract that does
not exist. They are here to be recognised, not to be substituted.
"""

from __future__ import annotations

# ── service naming ────────────────────────────────────────────────────────────────
VALIDATOR_PREFIX = "validator-"
FULL_NODE = "full-node"
DOWNSTREAM = "downstream"
GENESIS_INIT = "genesis-init"

#: The two cascade tiers below the committee (L2 cert-follower, L3 cascade follower).
CASCADE_TIERS = (FULL_NODE, DOWNSTREAM)


def validator(idx) -> str:
    """The committee container serving native index <idx>."""
    return f"{VALIDATOR_PREFIX}{idx}"


def is_validator(service: str) -> bool:
    return bool(service) and service.startswith(VALIDATOR_PREFIX)


def validator_idx(service: str):
    """The native index encoded in a `validator-N` service name, or None for anything
    else (`full-node`, `downstream`, a malformed name)."""
    if not is_validator(service):
        return None
    try:
        return int(service[len(VALIDATOR_PREFIX):])
    except ValueError:
        return None


# ── roles ─────────────────────────────────────────────────────────────────────────
#: The pinned host-RPC node — see the module header. Read over the host, never disrupted.
PINNED_RPC_HOST = validator(0)
#: The pinned cert-upstream anchor (FLK-8). Never a churn victim.
CERT_UPSTREAM_ANCHOR = validator(1)
#: Committee containers that sit ABOVE the churn/eviction reach (v61 a26).
ANCHOR_CONTAINERS = (PINNED_RPC_HOST, CERT_UPSTREAM_ANCHOR)
#: The service whose container mounts /runtime for the cat/write seam and the funder key.
RUNTIME_MOUNT_HOST = PINNED_RPC_HOST


def is_pinned_rpc_host(service: str) -> bool:
    return service == PINNED_RPC_HOST


def is_anchor(service: str) -> bool:
    return service in ANCHOR_CONTAINERS


# ── ports ─────────────────────────────────────────────────────────────────────────
# In-container: identical on EVERY node (each has its own network namespace).
RPC_PORT = 8545                    # reth http
WS_PORT = 8546                     # reth ws (the cert-upstream plane)
CONSENSUS_METRICS_PORT = 9100      # commonware registry (--dpos.metrics-port)
EL_METRICS_PORT = 9200             # reth metrics-rs recorder (--metrics)
DEVP2P_PORT = 30303                # reth devp2p (enode)
DPOS_DIALABLE_PORT = 9000          # commonware p2p (--dpos.dialable)

# Host-published: the map is short BECAUSE most nodes publish nothing.
HOST_RPC_PORT = 8545               # validator-0
HOST_WS_PORT = 8546                # validator-0 (published so an operator can tail the WS plane)
HOST_L2_RPC_PORT = 18545           # full-node
HOST_L3_RPC_PORT = 28545           # downstream
HOST_CONSENSUS_METRICS_PORT = 19100  # validator-0 only
HOST_EL_METRICS_PORT = 19200         # validator-0 only

#: service -> host-side RPC port. Absent = in-container reads only.
HOST_RPC_PORTS = {
    PINNED_RPC_HOST: HOST_RPC_PORT,
    FULL_NODE: HOST_L2_RPC_PORT,
    DOWNSTREAM: HOST_L3_RPC_PORT,
}

#: service -> (host consensus-metrics port, host EL-metrics port). ONE entry, deliberately.
HOST_METRICS_PORTS = {
    PINNED_RPC_HOST: (HOST_CONSENSUS_METRICS_PORT, HOST_EL_METRICS_PORT),
}


def host_url(port) -> str:
    """A host-mapped endpoint. `localhost` (not 127.0.0.1) — the bash used localhost and
    the two differ on a host with IPv6-first resolution."""
    return f"http://localhost:{port}"


#: The default host RPC — validator-0's. What `$RPC` falls back to everywhere.
DEFAULT_RPC_URL = host_url(HOST_RPC_PORT)
#: The IN-CONTAINER reth RPC, reached through `docker compose exec … curl`. Numerically
#: equal to DEFAULT_RPC_URL and semantically unrelated: this one is inside a namespace.
IN_CONTAINER_RPC_URL = host_url(RPC_PORT)
IN_CONTAINER_CONSENSUS_METRICS_URL = host_url(CONSENSUS_METRICS_PORT) + "/metrics"
IN_CONTAINER_EL_METRICS_URL = host_url(EL_METRICS_PORT) + "/metrics"


def has_host_rpc(service: str) -> bool:
    """True iff <service> publishes an RPC on the host (so a reader may skip the exec)."""
    return service in HOST_RPC_PORTS


def host_rpc_port(service: str):
    """<service>'s host-side RPC port, or None when it is in-container only."""
    return HOST_RPC_PORTS.get(service)


def host_rpc_url(service: str):
    """<service>'s host RPC URL, or None when it is in-container only."""
    port = HOST_RPC_PORTS.get(service)
    return host_url(port) if port is not None else None


def has_host_metrics(service: str) -> bool:
    return service in HOST_METRICS_PORTS


def host_metrics_urls(service: str):
    """(consensus, EL) host metrics URLs for <service>, or None when it publishes none."""
    ports = HOST_METRICS_PORTS.get(service)
    if ports is None:
        return None
    return (host_url(ports[0]) + "/metrics", host_url(ports[1]) + "/metrics")


# ── compose publish mappings ──────────────────────────────────────────────────────
# The `host:container` strings `stack/compose_gen.py` emits under `ports:`. They exist so
# that the two maps above DECIDE who is reachable off-host instead of merely describing it:
# the generator has no port literals of its own, so a map edit changes the compose file.

def host_rpc_publish(service: str):
    """The compose `ports:` mapping publishing <service>'s reth RPC on the host, or None
    when it publishes none (in-container reads only)."""
    port = HOST_RPC_PORTS.get(service)
    return None if port is None else f"{port}:{RPC_PORT}"


def host_metrics_publish(service: str):
    """The compose `ports:` mappings publishing <service>'s two metrics registries
    (consensus, EL) on the host — () when it publishes neither."""
    ports = HOST_METRICS_PORTS.get(service)
    if ports is None:
        return ()
    return (f"{ports[0]}:{CONSENSUS_METRICS_PORT}", f"{ports[1]}:{EL_METRICS_PORT}")


# ── network ───────────────────────────────────────────────────────────────────────
SUBNET = "172.20.0.0/24"
_NET = "172.20.0"

#: validator-N sits at <net>.(10+N). ARITHMETIC, never string concatenation — the bash
#: `172.20.0.1$IDX` silently produced .110 at IDX 10 and .10 collided with the sequencer.
VALIDATOR_IP_OFFSET = 10

GENESIS_INIT_IP = f"{_NET}.5"
#: coredns, emitted only for BOOTSTRAP_MODE=dns. NOTE: .53 is also validator-43's address,
#: so that profile caps out at 43 containers (docker rejects the duplicate loudly). Pinned by
#: tests/test_topology.py so moving either address stays a deliberate act. Pre-existing.
DNS_IP = f"{_NET}.53"
FULL_NODE_IP = f"{_NET}.250"
DOWNSTREAM_IP = f"{_NET}.251"


def validator_ip(idx) -> str:
    return f"{_NET}.{VALIDATOR_IP_OFFSET + int(idx)}"


#: validator-0's IP, i.e. the phase-A sequencer / the WS cert-upstream every follower dials.
SEQUENCER_IP = validator_ip(0)
#: validator-1's IP — v0's `--dpos.follower-upstream` and the full-node's 2nd cert-upstream.
CERT_UPSTREAM_ANCHOR_IP = validator_ip(1)


def enode(pubkey: str, ip: str, port: int = DEVP2P_PORT) -> str:
    """An enode URL rebuilt against a node's FIXED compose IP. `admin_nodeInfo`'s embedded
    IP is unreliable inside docker, so only the 128-hex pubkey is taken from the node and
    the address comes from here."""
    return f"enode://{pubkey}@{ip}:{port}"


# ── chain identity ────────────────────────────────────────────────────────────────
CHAIN_ID = 2026

# Genesis predeploy SLOTS — codeless in this configuration. Read the module header before
# using any of these; they are never a fallback for a runtime-deployed address.
STAKING_ADDR = "0x0000000000000000000000000000000000005201"
CHAIN_CONFIG_ADDR = "0x0000000000000000000000000000000000005202"
STAKING_POOL_ADDR = "0x0000000000000000000000000000000000005203"
# LivenessSlashing is pinned to the fixed EXECUTOR predeploy (PRECOMPILE_LIVENESS_SLASHING),
# NOT the 0x…520N scheme — see genesis-bootstrap bootstrap.rs LIVENESS_SLASHING_ADDR.
LIVENESS_SLASHING_ADDR = "0x0000000000000000000000000000000000520020"
