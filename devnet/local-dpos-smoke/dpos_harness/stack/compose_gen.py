"""compose_gen.py — generate the N-validator sim compose pair. Port of gen-soak-compose.sh.

Writes docker-compose.sim.gen.yml (phase A bare sequencer chain) and
docker-compose.sim.dpos.gen.yml (phase B --dpos overlay), scaling the production-path topology
past its hardcoded 6. Every IP is arithmetic (172.20.0.$((10+idx))) — the ROOT FIX for the
dialable-IP string-concat bug (`172.20.0.1$IDX` silently breaks at IDX 10).

PORT-NOTE: the bash built these via 12 UNQUOTED heredocs + 27 backticks, whose whole disease
class (a stray backtick once truncated the file to 0 bytes with exit 0) DIES here — Python
f-strings never run embedded words as commands. The generated TEXT is kept structurally
byte-compatible with the bash output (same anchors, same fluent argv, same key order) so
`docker compose config` treats them as equivalent; assert_generated is preserved as the
post-write truncation guard. RUST_LOG_STYLE=never, the json-file log bounds, and every knob
(PRUNE/DATA_ROOT/GEO_LATENCY/NO_CASCADE/PEER_HOST_MODE/BOOTSTRAP_MODE/rotation/identity-pool)
are carried faithfully.
"""

from __future__ import annotations

import os

from ..core import topology

# Symmetric inter-region RTT (ms), row-major 5x5 (research P2 matrix). One-way egress = RTT/2.
GEO_REGIONS = 5
GEO_RTT = [
    2, 60, 80, 200, 120,
    60, 2, 140, 150, 180,
    80, 140, 2, 250, 180,
    200, 150, 250, 2, 300,
    120, 180, 180, 300, 2,
]


class ComposeGenError(Exception):
    pass


def _envs(name, default):
    v = os.environ.get(name)
    return v if v not in (None, "") else default


def generate(n: int, target: int = None, identity_pool: int = None, out_dir: str = "."):
    """Generate the compose pair for N validators. Returns (base_path, dpos_path). Knobs are read
    from the environment exactly like the bash (SIM_ROTATION_SLOTS, SIM_NO_CASCADE,
    PEER_HOST_MODE, BOOTSTRAP_MODE, SIM_PRUNE_PROFILE, SIM_DATA_ROOT, SIM_GEO_LATENCY,
    SIM_LOG_MAX_SIZE/FILE)."""
    if target is None:
        target = n
    if identity_pool is None:
        identity_pool = n

    if not (4 <= target <= n):
        raise ComposeGenError(f"bad COMMITTEE_TARGET={target} (need 4..{n})")
    if identity_pool < n:
        raise ComposeGenError(f"bad IDENTITY_POOL={identity_pool} (need >= N={n})")
    if n < 4:
        raise ComposeGenError(f"N={n} < 4 (need INITIAL_F>=1; MIN_COMMITTEE math, D4)")
    if n > 51:
        raise ComposeGenError(f"N={n} > 51 (exceeds MAX_COMMITTEE_SIZE=51, dpos.rs:931)")

    rotation_slots = int(_envs("SIM_ROTATION_SLOTS", "0"))
    no_cascade = _envs("SIM_NO_CASCADE", "0")
    peer_host_mode = _envs("PEER_HOST_MODE", "ip")
    bootstrap_mode = _envs("BOOTSTRAP_MODE", "json")
    prune_profile = _envs("SIM_PRUNE_PROFILE", "full")
    data_root = _envs("SIM_DATA_ROOT", "")
    geo_latency = _envs("SIM_GEO_LATENCY", "0")
    log_max_size = _envs("SIM_LOG_MAX_SIZE", "64m")
    log_max_file = _envs("SIM_LOG_MAX_FILE", "5")

    if peer_host_mode not in ("ip", "dns"):
        raise ComposeGenError(f"bad PEER_HOST_MODE={peer_host_mode} (ip|dns)")
    if bootstrap_mode not in ("json", "dns"):
        raise ComposeGenError(f"bad BOOTSTRAP_MODE={bootstrap_mode} (json|dns)")

    seq_ip = topology.SEQUENCER_IP
    full_ip = topology.FULL_NODE_IP
    seed_dns_zone = "seed.sim.local"
    dns_ip = topology.DNS_IP

    def dialable_host(idx):
        return (topology.validator(idx) if peer_host_mode == "dns"
                else topology.validator_ip(idx))

    if bootstrap_mode == "dns":
        bootstrap_src = f"dns:{seed_dns_zone}"
        seed_dns_cmd = f"\n      - --seed-dns-zone={seed_dns_zone}"
        dns_svc_key = f"\n    dns:\n      - {dns_ip}"
    else:
        bootstrap_src = "/runtime/peers.json"
        seed_dns_cmd = ""
        dns_svc_key = ""

    # --full prune line (Paris@0 keeps all bodies). "archive" emits nothing.
    prune_line = "          --full \\\n" if prune_profile != "archive" else ""

    runtime_device = ""
    if data_root:
        dr = data_root.rstrip("/")
        if dr in ("", "/") or not dr.startswith("/"):
            raise ComposeGenError(
                f"refusing SIM_DATA_ROOT='{data_root}' (must be an absolute path with at least "
                "one component; '/' would wipe the root filesystem)")
        runtime_device = f"{dr}/runtime"
        os.makedirs(runtime_device, exist_ok=True)

    capadd = ("    cap_add:\n      - NET_ADMIN\n" if geo_latency == "1" else "")

    def emit_netem_prologue(i):
        """Shell prologue installing per-destination tc-netem for validator i (8-space indent),
        or "" when the toggle is off. Region(i) = i % GEO_REGIONS."""
        if geo_latency != "1":
            return ""
        ind = "        "
        ri = i % GEO_REGIONS
        lines = [f"{ind}set -e",
                 f"{ind}tc qdisc add dev eth0 root handle 1: prio bands {3 + GEO_REGIONS}"]
        for rj in range(GEO_REGIONS):
            owd = GEO_RTT[ri * GEO_REGIONS + rj] // 2
            jit = owd // 10
            band = 4 + rj
            lines.append(f"{ind}tc qdisc add dev eth0 parent 1:{band} handle {band}0: "
                         f"netem delay {owd}ms {jit}ms")
        for j in range(n):
            if j == i:
                continue
            band = 4 + (j % GEO_REGIONS)
            lines.append(f"{ind}tc filter add dev eth0 parent 1:0 protocol ip u32 match ip dst "
                         f"{topology.validator_ip(j)}/32 flowid 1:{band}")
        lines.append(f"{ind}set +e")
        return "\n".join(lines)

    def body_for(i, args):
        pro = emit_netem_prologue(i)
        b = (pro + "\n") if pro else ""
        return b + "        exec /usr/local/bin/fluent node \\\n" + args

    def ports_block(*maps):
        """A service's `ports:` stanza from already-resolved `host:container` mappings, "" when
        it publishes nothing. The generator owns NO port literal: every mapping comes from
        `topology.host_rpc_publish` / `host_metrics_publish`, so `HOST_RPC_PORTS` /
        `HOST_METRICS_PORTS` decide who is reachable off-host rather than describing it after
        the fact. Publishing a node the readers do not expect (or vice versa) is then a compose
        diff, which test_compose_parity's port oracle catches."""
        maps = [m for m in maps if m]
        if not maps:
            return ""
        return "    ports:\n" + "".join(f'      - "{m}"\n' for m in maps)

    ips = ",".join(topology.validator_ip(i) for i in range(identity_pool))

    base = os.path.join(out_dir, "docker-compose.sim.gen.yml")
    dpos = os.path.join(out_dir, "docker-compose.sim.dpos.gen.yml")

    # ── Phase A base ──────────────────────────────────────────────────────────
    log_opts = (f"    driver: json-file\n    options:\n"
                f'      max-size: "{log_max_size}"\n      max-file: "{log_max_file}"\n')
    parts = []
    parts.append(f"""name: fluent-dpos-sim
# GENERATED by dpos_harness.stack.compose_gen N={n} — do not edit (gitignored).
networks:
  fluent-net:
    driver: bridge
    ipam:
      config:
        - subnet: {topology.SUBNET}

x-fluent-build: &fluent-build
  image: fluent-dpos-smoke:local
  environment:
    RUST_LOG_STYLE: "never"
  logging:
{log_opts}
x-fluent-build-def: &fluent-build-def
  build:
    context: ../..
    dockerfile: devnet/local-dpos-smoke/Dockerfile
  image: fluent-dpos-smoke:local
  environment:
    RUST_LOG_STYLE: "never"
  logging:
{log_opts}
x-cpu-limit: &cpu-limit
  deploy:
    resources:
      limits:
        cpus: "2.0"

x-follower-health: &follower-health
  healthcheck:
    test:
      - "CMD-SHELL"
      - "curl -s -X POST -H 'Content-Type: application/json' --data '{{\\"jsonrpc\\":\\"2.0\\",\\"method\\":\\"eth_blockNumber\\",\\"params\\":[],\\"id\\":1}}' {topology.IN_CONTAINER_RPC_URL} | grep -q result"
    interval: 2s
    timeout: 2s
    retries: 30
    start_period: 5s

services:
  {topology.GENESIS_INIT}:
    <<: *fluent-build-def
    entrypoint: ["/usr/local/bin/genesis-bootstrap"]
    command:
      - bare
      - --peers={identity_pool}
      - --bootstrappers=2
      - --output=/runtime
      - --chain-id={topology.CHAIN_ID}
      - --validator-ips={ips}
      - --peer-host-mode={peer_host_mode}{seed_dns_cmd}
      - --staking-reader-create-nonces=9,15,19
    volumes:
      - runtime:/runtime
    networks:
      fluent-net:
        ipv4_address: {topology.GENESIS_INIT_IP}
""")

    # validator-0 (phase A sequencer)
    v0_args = (f"          --chain=/runtime/genesis-local.json \\\n"
               f"{prune_line}          --validator \\\n"
               f"          --validator.block-time=1s \\\n"
               f"          --dpos.staking-config=/runtime/staking-reader.json \\\n"
               f"          --builder.extradata= \\\n"
               f"          --http --http.addr=0.0.0.0 --http.port={topology.RPC_PORT} --http.api=eth,net,web3 \\\n"
               f"          --ws --ws.addr=0.0.0.0 --ws.port={topology.WS_PORT} \\\n"
               f"          --p2p-secret-key=/runtime/v0-p2p-secret.hex \\\n"
               f"          --port={topology.DEVP2P_PORT} \\\n"
               f"          --datadir=/runtime/reth-data/v0")
    parts.append(f"""
  {topology.PINNED_RPC_HOST}:
    <<: [*fluent-build, *cpu-limit, *follower-health]
{capadd}    depends_on:
      {topology.GENESIS_INIT}:
        condition: service_completed_successfully
    entrypoint: ["/bin/sh", "-c"]
    command:
      - |
{body_for(0, v0_args)}
    volumes:
      - runtime:/runtime
{ports_block(topology.host_rpc_publish(topology.PINNED_RPC_HOST),
             f"{topology.HOST_WS_PORT}:{topology.WS_PORT}")}    networks:
      fluent-net:
        ipv4_address: {seq_ip}
""")

    for i in range(1, n):
        prev = i - 1
        args = (f"          --chain=/runtime/genesis-local.json \\\n"
                f"{prune_line}          --cert-upstream=ws://{seq_ip}:{topology.WS_PORT} \\\n"
                f"          --dpos.staking-config=/runtime/staking-reader.json \\\n"
                f"          --builder.extradata= \\\n"
                f'          --http --http.addr=0.0.0.0 --http.port={topology.RPC_PORT} --http.api=eth,net,web3 \\\n'
                f'          --trusted-peers="$(cat /runtime/v0-enode.txt)" \\\n'
                f"          --trusted-only \\\n"
                f"          --disable-discovery \\\n"
                f"          --datadir=/runtime/reth-data/v{i}")
        parts.append(f"""
  {topology.validator(i)}:
    <<: [*fluent-build, *cpu-limit, *follower-health]
{capadd}    depends_on:
      {topology.validator(prev)}:
        condition: service_healthy
    entrypoint: ["/bin/sh", "-c"]
    command:
      - |
{body_for(i, args)}
    volumes:
      - runtime:/runtime
{ports_block(topology.host_rpc_publish(topology.validator(i)))}    networks:
      fluent-net:
        ipv4_address: {topology.validator_ip(i)}
""")

    if no_cascade != "1":
        parts.append(f"""
  {topology.FULL_NODE}:
    <<: [*fluent-build, *cpu-limit, *follower-health]
    depends_on:
      {topology.validator(target - 1)}:
        condition: service_healthy
    entrypoint: ["/bin/sh", "-c"]
    command:
      - |
        exec /usr/local/bin/fluent node \\
          --chain=/runtime/genesis-local.json \\
          --cert-upstream=ws://{seq_ip}:{topology.WS_PORT} \\
          --dpos.staking-config=/runtime/staking-reader.json \\
          --builder.extradata= \\
          --rpc.eth-proof-window=50000 \\
          --http --http.addr=0.0.0.0 --http.port={topology.RPC_PORT} --http.api=eth,net,web3 \\
          --trusted-peers="$(cat /runtime/v0-enode.txt)" \\
          --trusted-only \\
          --disable-discovery \\
          --datadir=/runtime/reth-data/full-node
    volumes:
      - runtime:/runtime
{ports_block(topology.host_rpc_publish(topology.FULL_NODE))}    networks:
      fluent-net:
        ipv4_address: {full_ip}
""")

    if runtime_device:
        parts.append(f"""
volumes:
  runtime:
    driver: local
    driver_opts:
      type: none
      o: bind
      device: {runtime_device}
""")
    else:
        parts.append("\nvolumes:\n  runtime: {}\n")

    base_text = "".join(parts)
    with open(base, "w") as f:
        f.write(base_text)
    assert_generated(base, "\nvolumes:")

    # ── Phase B --dpos overlay ────────────────────────────────────────────────
    dparts = [f"# GENERATED by dpos_harness.stack.compose_gen N={n} — phase B --dpos overlay (gitignored).\nservices:\n"]

    v0d_args = (f"          --chain=/runtime/genesis-local.json \\\n"
                f"{prune_line}          --datadir=/runtime/reth-data/v0 \\\n"
                f"          --dpos \\\n"
                f"          --dpos.follower-upstream="
                f"ws://{topology.CERT_UPSTREAM_ANCHOR_IP}:{topology.WS_PORT} \\\n"
                f"          --dpos.bls-key-path=/runtime/keys/{topology.validator(0)}/bls.hex \\\n"
                f"          --dpos.peer-key-path=/runtime/keys/{topology.validator(0)}/peer.hex \\\n"
                f"          --dpos.staking-config=/runtime/staking-reader.json \\\n"
                f"          --dpos.bootstrappers={bootstrap_src} \\\n"
                f"          --dpos.dialable={dialable_host(0)}:{topology.DPOS_DIALABLE_PORT} \\\n"
                f"          --dpos.metrics-port={topology.CONSENSUS_METRICS_PORT} \\\n"
                f"          --metrics=0.0.0.0:{topology.EL_METRICS_PORT} \\\n"
                f"          --dpos.slasher-keystore-path="
                f"/runtime/keys/{topology.validator(0)}/slasher.json \\\n"
                f"          --dpos.slasher-keystore-password-file="
                f"/runtime/keys/{topology.validator(0)}/slasher.pwd \\\n"
                f"          --builder.extradata= \\\n"
                f"          --p2p-secret-key=/runtime/v0-p2p-secret.hex \\\n"
                f"          --port={topology.DEVP2P_PORT} \\\n"
                f"          --http --http.addr=0.0.0.0 --http.port={topology.RPC_PORT} --http.api=eth,net,web3 \\\n"
                f"          --ws --ws.addr=0.0.0.0 --ws.port={topology.WS_PORT}")
    dparts.append(f"""  {topology.PINNED_RPC_HOST}:
    entrypoint: ["/bin/sh", "-c"]
    command:
      - |
{body_for(0, v0d_args)}{dns_svc_key}
{ports_block(*topology.host_metrics_publish(topology.PINNED_RPC_HOST))}""")

    for i in range(1, n):
        # ALL containers route key material through the LOGICAL identity ${IDENTITY_IDX:-i}. Emit
        # `$${IDENTITY_IDX:-i}` (double-$ = compose literal-$ escape) so the container's /bin/sh
        # expands it at RUNTIME (a rebirth overlay sets IDENTITY_IDX; unset → defaults to i).
        keydir = f"{topology.VALIDATOR_PREFIX}$${{IDENTITY_IDX:-{i}}}"
        args = (f"          --chain=/runtime/genesis-local.json \\\n"
                f"{prune_line}          --datadir=/runtime/reth-data/v{i} \\\n"
                f"          --dpos \\\n"
                f"          --dpos.follower-upstream=ws://{seq_ip}:{topology.WS_PORT} \\\n"
                f"          --dpos.bls-key-path=/runtime/keys/{keydir}/bls.hex \\\n"
                f"          --dpos.peer-key-path=/runtime/keys/{keydir}/peer.hex \\\n"
                f"          --dpos.staking-config=/runtime/staking-reader.json \\\n"
                f"          --dpos.bootstrappers={bootstrap_src} \\\n"
                f"          --dpos.dialable={dialable_host(i)}:{topology.DPOS_DIALABLE_PORT} \\\n"
                f"          --dpos.metrics-port={topology.CONSENSUS_METRICS_PORT} \\\n"
                f"          --metrics=0.0.0.0:{topology.EL_METRICS_PORT} \\\n"
                f"          --dpos.slasher-keystore-path=/runtime/keys/{keydir}/slasher.json \\\n"
                f"          --dpos.slasher-keystore-password-file=/runtime/keys/{keydir}/slasher.pwd \\\n"
                f"          --builder.extradata= \\\n"
                f"          --http --http.addr=0.0.0.0 --http.port={topology.RPC_PORT} --http.api=eth,net,web3 \\\n"
                f"          --ws --ws.addr=0.0.0.0 --ws.port={topology.WS_PORT} \\\n"
                f'          --trusted-peers="$(cat /runtime/v0-enode.txt)" \\\n'
                f"          --trusted-only \\\n"
                f"          --disable-discovery")
        dparts.append(f"""
  {topology.validator(i)}:
    entrypoint: ["/bin/sh", "-c"]
    command:
      - |
{body_for(i, args)}{dns_svc_key}
""")

    if bootstrap_mode == "dns":
        dparts.append(f"""
  coredns:
    image: coredns/coredns:1.11.1
    depends_on:
      {topology.GENESIS_INIT}:
        condition: service_completed_successfully
    command: ["-conf", "/runtime/Corefile"]
    volumes:
      - runtime:/runtime
    networks:
      fluent-net:
        ipv4_address: {dns_ip}
""")

    if no_cascade != "1":
        dparts.append(f"""
  {topology.FULL_NODE}:
    entrypoint: ["/bin/sh", "-c"]
    command:
      - |
        exec /usr/local/bin/fluent node \\
          --chain=/runtime/genesis-local.json \\
          --cert-follow \\
          --cert-upstream=ws://{seq_ip}:{topology.WS_PORT} \\
          --cert-upstream=ws://{topology.CERT_UPSTREAM_ANCHOR_IP}:{topology.WS_PORT} \\
          --dpos.staking-config=/runtime/staking-reader.json \\
          --builder.extradata= \\
          --rpc.eth-proof-window=50000 \\
          --http --http.addr=0.0.0.0 --http.port={topology.RPC_PORT} --http.api=eth,net,web3,admin \\
          --ws --ws.addr=0.0.0.0 --ws.port={topology.WS_PORT} \\
          --trusted-peers="$(cat /runtime/v0-enode.txt)" \\
          --trusted-only \\
          --disable-discovery \\
          --datadir=/runtime/reth-data/full-node
""")
        dparts.append(f"""
  {topology.DOWNSTREAM}:
    image: fluent-dpos-smoke:local
    environment:
      RUST_LOG_STYLE: "never"
    entrypoint: ["/bin/sh", "-c"]
    command:
      - |
        exec /usr/local/bin/fluent node \\
          --chain=/runtime/genesis-local.json \\
{prune_line}          --cert-follow \\
          --cert-upstream=ws://{full_ip}:{topology.WS_PORT} \\
          --dpos.staking-config=/runtime/staking-reader.json \\
          --builder.extradata= \\
          --rpc.eth-proof-window=10000 \\
          --http --http.addr=0.0.0.0 --http.port={topology.RPC_PORT} --http.api=eth,net,web3,admin \\
          --trusted-peers="$(cat /runtime/l2-enode.txt)" \\
          --trusted-only \\
          --disable-discovery \\
          --datadir=/runtime/reth-data/downstream
    volumes:
      - runtime:/runtime
    networks:
      fluent-net:
        ipv4_address: {topology.DOWNSTREAM_IP}
{ports_block(topology.host_rpc_publish(topology.DOWNSTREAM))}""")

    dpos_text = "".join(dparts)
    with open(dpos, "w") as f:
        f.write(dpos_text)
    assert_generated(dpos, f"{topology.validator(n - 1)}:")
    return base, dpos


def assert_generated(path: str, marker: str):
    """Post-write truncation guard (the heredoc-truncation belt): fail LOUDLY if the generated
    file is empty or missing a marker it must always contain."""
    try:
        with open(path) as f:
            text = f.read()
    except OSError:
        raise ComposeGenError(f"FATAL generated {path} is missing")
    if not text:
        raise ComposeGenError(f"FATAL generated {path} is EMPTY")
    if marker not in text:
        raise ComposeGenError(f"FATAL generated {path} missing required marker '{marker}'")
