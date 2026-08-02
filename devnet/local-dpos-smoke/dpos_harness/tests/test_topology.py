"""The devnet shape, pinned.

`core/topology.py` is the single home for what the harness knows about the shape of a
devnet, so a wrong value there is wrong EVERYWHERE at once — which is the trade this
extraction makes: one place to change, one place to get wrong. Three things are pinned
here because each has a silent failure mode:

  * the PORT MAP — a wrong host port reads a different node, or nothing, and the sentinel
    discipline turns "nothing" into a benign-looking `null|null` rather than an error;
  * the IP ARITHMETIC — the bug this replaced (`172.20.0.1$IDX`) produced a *valid* IP at
    every index, so the chain came up and simply never peered past IDX 9;
  * the `validator-0` ROLE ASYMMETRY — it is the pinned host-RPC node, and a refactor that
    makes it indistinguishable from `validator-3` is the one this module could invite. The
    asymmetry lives here as DATA (which services publish host ports), so it is testable.
"""

from __future__ import annotations

from dpos_harness.core import topology as t


# ── service naming ──────────────────────────────────────────────────────────────

def test_validator_names_are_index_arithmetic():
    assert t.validator(0) == "validator-0"
    assert t.validator(13) == "validator-13"
    assert t.validator(10) == "validator-10"
    # `validator-1` is a PREFIX of `validator-10`. Every membership test on these names
    # must therefore be whole-word (core/setops.py exists for that); the name scheme is
    # not going to change, so the hazard is pinned here rather than assumed away.
    assert t.validator(10).startswith(t.validator(1))


def test_validator_idx_round_trips_and_rejects_non_validators():
    for i in (0, 1, 9, 10, 50):
        assert t.validator_idx(t.validator(i)) == i
    for svc in (t.FULL_NODE, t.DOWNSTREAM, t.GENESIS_INIT, "", "validator-x"):
        assert t.validator_idx(svc) is None
        assert not (t.is_validator(svc) and t.validator_idx(svc) is not None)


def test_cascade_tiers_are_not_validators():
    assert t.CASCADE_TIERS == (t.FULL_NODE, t.DOWNSTREAM)
    for svc in t.CASCADE_TIERS:
        assert not t.is_validator(svc)


# ── the port map ────────────────────────────────────────────────────────────────

def test_host_rpc_port_map():
    """Exactly three services publish an RPC on the host, at three distinct ports."""
    assert t.HOST_RPC_PORTS == {
        "validator-0": 8545,
        "full-node": 18545,
        "downstream": 28545,
    }
    assert len(set(t.HOST_RPC_PORTS.values())) == 3


def test_in_container_ports_are_uniform():
    """Every node serves the same ports INSIDE its own namespace; only the host side is
    remapped. Conflating the two is how a reader ends up curling 19100 in-container."""
    assert (t.RPC_PORT, t.WS_PORT) == (8545, 8546)
    assert (t.CONSENSUS_METRICS_PORT, t.EL_METRICS_PORT) == (9100, 9200)
    assert t.IN_CONTAINER_RPC_URL == "http://localhost:8545"
    assert t.IN_CONTAINER_CONSENSUS_METRICS_URL == "http://localhost:9100/metrics"
    assert t.IN_CONTAINER_EL_METRICS_URL == "http://localhost:9200/metrics"


def test_host_metrics_are_the_pinned_node_alone():
    """19100/19200 exist because ONE node publishes its two registries on the host. If a
    second service ever appears here, every `_node_metrics` caller changes transport for
    it — which is a decision, not a detail."""
    assert list(t.HOST_METRICS_PORTS) == [t.PINNED_RPC_HOST]
    assert t.HOST_METRICS_PORTS[t.PINNED_RPC_HOST] == (19100, 19200)
    assert t.host_metrics_urls(t.PINNED_RPC_HOST) == (
        "http://localhost:19100/metrics", "http://localhost:19200/metrics")


def test_host_and_container_metrics_ports_never_collide():
    """The host ports are the container ports + 10000 precisely so a scrape aimed at the
    wrong side fails loudly instead of hitting the other registry."""
    host = set(t.HOST_METRICS_PORTS[t.PINNED_RPC_HOST])
    assert host.isdisjoint({t.CONSENSUS_METRICS_PORT, t.EL_METRICS_PORT})


def test_host_url_and_default_rpc():
    assert t.host_url(18545) == "http://localhost:18545"
    assert t.DEFAULT_RPC_URL == t.host_url(t.HOST_RPC_PORT)
    assert t.host_rpc_url(t.FULL_NODE) == "http://localhost:18545"
    assert t.host_rpc_url(t.validator(3)) is None
    assert t.host_rpc_port(t.validator(3)) is None


# ── the role asymmetry ──────────────────────────────────────────────────────────

def test_validator_0_is_addressed_differently_from_every_other_validator():
    """THE invariant this module must not flatten. v0 is read over the HOST; v1..vN are
    read in-container. Several detectors, `runtime_cat`/`runtime_write` and the funder-key
    read depend on that difference, and a gate rule refuses to disrupt v0 because of it."""
    assert t.is_pinned_rpc_host(t.PINNED_RPC_HOST)
    assert t.has_host_rpc(t.PINNED_RPC_HOST)
    assert t.has_host_metrics(t.PINNED_RPC_HOST)
    assert t.RUNTIME_MOUNT_HOST == t.PINNED_RPC_HOST
    for i in range(1, 14):
        other = t.validator(i)
        assert not t.is_pinned_rpc_host(other)
        assert not t.has_host_rpc(other), f"{other} must be in-container only"
        assert not t.has_host_metrics(other), f"{other} must be in-container only"
        assert t.host_metrics_urls(other) is None


def test_anchors_are_v0_and_v1_and_nothing_else():
    """v0 (host RPC) and v1 (cert-upstream) are both pinned out of the churn pool. The set
    is duplicated nowhere: `sim/reconcilers.ANCHOR_CONTAINERS` aliases this one."""
    from dpos_harness.sim import reconcilers
    assert t.ANCHOR_CONTAINERS == (t.validator(0), t.validator(1))
    assert reconcilers.ANCHOR_CONTAINERS is t.ANCHOR_CONTAINERS
    assert t.is_anchor(t.validator(0)) and t.is_anchor(t.validator(1))
    assert not t.is_anchor(t.validator(2))
    assert not t.is_anchor(t.FULL_NODE)


def test_committee_victim_draw_never_yields_an_anchor():
    """The asymmetry as BEHAVIOUR, not just data: the victim draw must skip both anchors
    whatever the committee looks like."""
    from dpos_harness.core import setops
    addr2idx = {f"0xa{i}": t.validator(i) for i in range(6)}
    committee = " ".join(addr2idx)
    victims = setops.committee_victim_idxs(committee, addr2idx)
    assert victims == [t.validator(i) for i in range(2, 6)]
    assert not any(t.is_anchor(v) for v in victims)


# ── IP derivation ───────────────────────────────────────────────────────────────

def test_validator_ip_is_arithmetic_not_concatenation():
    """`172.20.0.1$IDX` (the bug this replaced) yields .110 at IDX 10 and collides .10
    with the sequencer at IDX 0 — a valid address every time, so nothing fails loudly."""
    assert t.validator_ip(0) == "172.20.0.10"
    assert t.validator_ip(1) == "172.20.0.11"
    assert t.validator_ip(9) == "172.20.0.19"
    assert t.validator_ip(10) == "172.20.0.20"
    assert t.validator_ip(40) == "172.20.0.50"


def test_validator_ips_are_unique_and_inside_the_subnet_up_to_the_committee_cap():
    """51 is MAX_COMMITTEE_SIZE (a product constant, checked in compose_gen) — the IP plan
    has to survive a full-size committee without leaving the /24 or repeating itself."""
    ips = [t.validator_ip(i) for i in range(51)]
    assert len(set(ips)) == 51
    assert set(ips).isdisjoint({t.GENESIS_INIT_IP, t.FULL_NODE_IP, t.DOWNSTREAM_IP})
    for ip in ips:
        head, last = ip.rsplit(".", 1)
        assert head == "172.20.0" and 0 < int(last) < 255


def test_the_dns_address_collides_with_validator_43():
    """A PRE-EXISTING latent collision, pinned rather than silently inherited.

    `coredns` sits at 172.20.0.53 and validator-N sits at .（10+N), so index 43 wants the
    same address. It has never fired: coredns is only emitted for BOOTSTRAP_MODE=dns, and
    no run has combined that with a 44+ container pool. Docker would reject the second
    `ipv4_address`, so the failure is a loud compose error rather than a silent misroute —
    but it caps the DNS-bootstrap profile at 43 containers, which is worth knowing before
    someone raises the committee size and blames the product.

    This test does not sanction the collision; it makes moving either address a deliberate
    act. Fixing it (moving coredns out of the validator band) is a topology change, not an
    extraction, so it is not done here."""
    assert t.DNS_IP == t.validator_ip(43)


def test_named_ips_agree_with_the_derivation():
    assert t.SEQUENCER_IP == t.validator_ip(0)
    assert t.CERT_UPSTREAM_ANCHOR_IP == t.validator_ip(1)
    assert t.SUBNET == "172.20.0.0/24"
    assert (t.FULL_NODE_IP, t.DOWNSTREAM_IP) == ("172.20.0.250", "172.20.0.251")


def test_enode_rebuilds_against_the_compose_ip():
    """admin_nodeInfo's embedded IP is unreliable inside docker, so only the pubkey comes
    from the node — the address must come from here, on the default devp2p port."""
    pk = "a" * 128
    assert t.enode(pk, t.FULL_NODE_IP) == f"enode://{pk}@172.20.0.250:30303"
    assert t.DEVP2P_PORT == 30303


# ── chain identity ──────────────────────────────────────────────────────────────

def test_chain_id():
    assert t.CHAIN_ID == 2026


def test_predeploy_slots_are_not_defaults_in_the_ctx():
    """The predeploy addresses are CODELESS in this configuration — that is why the
    runtime addresses are injected at the ctx refresh. Naming them in topology must not
    resurrect them as a `Ctx` default; that defect was fixed and must stay fixed."""
    from dpos_harness.checks.battery import Ctx
    ctx = Ctx()
    predeploys = {t.STAKING_ADDR, t.CHAIN_CONFIG_ADDR, t.STAKING_POOL_ADDR,
                  t.LIVENESS_SLASHING_ADDR}
    for field in ("STAKING_RT", "CHAIN_CONFIG_RT", "LIVENESS_RT"):
        assert getattr(ctx, field) == "", f"Ctx.{field} must default empty, not to a predeploy"
        assert getattr(ctx, field) not in predeploys


def test_topology_is_a_pure_leaf():
    """It reads no environment and imports nothing from the package: a module every layer
    depends on must not carry configuration or a dependency of its own."""
    import ast
    import pathlib
    src = pathlib.Path(t.__file__).read_text()
    tree = ast.parse(src)
    imports = [n for n in ast.walk(tree) if isinstance(n, (ast.Import, ast.ImportFrom))]
    names = []
    for n in imports:
        if isinstance(n, ast.Import):
            names += [a.name for a in n.names]
        else:
            names.append(n.module or "")
    assert names == ["__future__"], f"topology must import nothing but __future__: {names}"
    assert "os.environ" not in src and "getenv" not in src
