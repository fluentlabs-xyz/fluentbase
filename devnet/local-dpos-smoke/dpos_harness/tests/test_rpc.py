"""Transport + read-helper tests, and the nodes.py read-only enforcement grep."""

from __future__ import annotations

import json
import os

import pytest

from dpos_harness.core import nodes, rpc


# ── pure transport / parse helpers ───────────────────────────────────────────
def test_strip_ansi():
    raw = "height\x1b[0m\x1b[2m=\x1b[0m2295 seed_round=\x1b[0m17"
    assert rpc.strip_ansi(raw) == "height=2295 seed_round=17"


def test_rpc_body():
    b = rpc.rpc_body("eth_getBlockByNumber", ["finalized", False])
    assert json.loads(b) == {
        "jsonrpc": "2.0", "method": "eth_getBlockByNumber",
        "params": ["finalized", False], "id": 1,
    }
    # default params -> []
    assert json.loads(rpc.rpc_body("net_peerCount"))["params"] == []


@pytest.mark.parametrize("lineno,text,expected", [
    (1, "  123\n  456", "123"),
    (2, "  123\n  456", "456"),
    (2, "\n  0x00ff", "0x00ff"),   # tuple: leading blank line then fields
    (3, "a\nb", ""),               # out of range -> ""
])
def test_cast_field(lineno, text, expected):
    assert rpc.cast_field(lineno, text) == expected


def test_num_hash_coercion():
    out = json.dumps({"result": {"number": "0x10", "hash": "0xabc"}})
    assert nodes._num_hash(out) == ("0x10", "0xabc")
    assert nodes._num_hash("{}") == ("null", "null")
    assert nodes._num_hash("") == ("null", "null")  # empty/garbage -> null|null sentinel


@pytest.mark.parametrize("h,expected", [
    ("0x0000000000000000000000000000000000000000000000000000000000000000", True),
    ("0x0", True),
    ("0x1", False),
    ("0xabc", False),
    ("", False),
])
def test_is_zero_hash(h, expected):
    assert nodes.is_zero_hash(h) is expected


def test_finalized_dec_coercion(monkeypatch):
    monkeypatch.setattr(nodes, "check_external", lambda p: "0x64|0xabc")
    monkeypatch.setattr(nodes, "running_services", lambda: [])   # fast path: v0 fine, no fallback
    assert nodes.finalized_dec() == 100
    # v0 dark AND no other node reachable → whole chain at 0
    monkeypatch.setattr(nodes, "check_external", lambda p: "null|null")
    assert nodes.finalized_dec() == 0


def test_finalized_dec_falls_back_to_committee_when_v0_dark(monkeypatch):
    """LIVENESS-FIRST: v0 (the pinned RPC host) reads 0/unreachable, but the chain is alive on the
    rest of the committee — finalized_dec returns the committee max, so a single node's death
    (even the RPC host's) never fakes a liveness failure. v0 itself is excluded from the fallback."""
    monkeypatch.setattr(nodes, "check_external", lambda p: "null|null")   # v0 dark
    monkeypatch.setattr(nodes, "running_services",
                        lambda: ["validator-0", "validator-1", "validator-3", "full-node"])
    fins = {"validator-0": 999, "validator-1": 7136, "validator-3": 7140}
    monkeypatch.setattr(nodes, "node_fin_in", lambda svc: fins.get(svc, -1))
    assert nodes.finalized_dec() == 7140          # committee max, NOT v0's stale 999 (excluded)


# ── metric parsers (awk-shape) ───────────────────────────────────────────────
METRICS = """# HELP x
dpos_sync_degraded{reason="result_divergence"} 1
dpos_sync_degraded{reason="no_peers"} 0
beacon_seed_active_total 42
some_executor_deferred_height 7
reth_dpos_derive_el_apply_duration_seconds_sum{path="finalized"} 1.5
reth_dpos_derive_el_apply_duration_seconds_sum{path="spec"} 0.5
reth_dpos_derive_el_apply_duration_seconds_count{path="finalized"} 10
reth_dpos_derive_el_apply_duration_seconds_count{path="spec"} 4
"""


def test_metric_val():
    assert nodes.metric_val(METRICS, "dpos_sync_degraded", 'reason="result_divergence"') == "1"
    assert nodes.metric_val(METRICS, "beacon_seed_active", "") == "42"
    assert nodes.metric_val(METRICS, "not_present", "") == ""


def test_gauge_val_suffix_anchored():
    # anchored (^|_)deferred_height$ — matches the actor-prefixed name
    assert nodes.gauge_val(METRICS, "deferred_height") == "7"
    # a bare substring must NOT match a longer metric name
    assert nodes.gauge_val("foo_deferred_height_total 9", "deferred_height") == ""


def test_summary_agg():
    s, c = nodes.summary_agg(METRICS, "reth_dpos_derive_el_apply_duration_seconds")
    assert abs(s - 2.0) < 1e-9  # 1.5 + 0.5 across label sets
    assert c == 14              # 10 + 4


# ── the metrics TRANSPORT switch (topology.HOST_METRICS_PORTS) ───────────────
# `node_metrics` used to dispatch on `if service == "validator-0"` — a switch nothing could
# widen by accident. It now dispatches on membership in `topology.HOST_METRICS_PORTS`, so
# ADDING a service to that map silently moves its scrape off `docker compose exec` and onto a
# host port that the compose file does not publish for it — a scrape that then reads "" and
# makes every metric-backed detector go quiet. These two tests pin the ARGV of both branches:
# widening the map fails the exec test, and narrowing it fails the host test.

def _metrics_transport(monkeypatch, service):
    """Run `nodes.node_metrics(<service>)` with both transports stubbed at the lowest level.
    Returns (exec_argvs, host_urls) — the real `docker compose exec … curl …` command lines,
    and the host URLs urllib would have been pointed at."""
    argvs, urls = [], []
    monkeypatch.setattr(rpc, "_run_read", lambda cmd, timeout: argvs.append(list(cmd)) or "")
    monkeypatch.setattr(nodes, "metrics_get_url", lambda url: urls.append(url) or "")
    nodes.node_metrics(service)
    return argvs, urls


def _exec_curl(service, port):
    return ["docker", "compose", "exec", "-T", service,
            "curl", "-s",
            "--max-time", str(int(rpc.RPC_MAX_TIME)),
            "--connect-timeout", str(int(rpc.RPC_CONNECT_TIMEOUT)),
            f"http://localhost:{port}/metrics"]


@pytest.mark.parametrize("service", ["full-node", "downstream", "validator-1", "validator-13"])
def test_non_publishing_nodes_are_scraped_through_compose_exec(monkeypatch, service):
    """Every node but the pinned RPC host is scraped IN-CONTAINER, on :9100 and :9200, through
    `docker compose exec`. Asserted as argv, so putting any of these services into
    HOST_METRICS_PORTS breaks this test instead of silently changing transport in production."""
    argvs, urls = _metrics_transport(monkeypatch, service)
    assert urls == [], f"{service} must not be scraped over a host port: {urls}"
    assert argvs == [_exec_curl(service, 9100), _exec_curl(service, 9200)]


def test_the_pinned_rpc_host_is_scraped_over_its_published_host_ports(monkeypatch):
    """The other side of the same switch — v0 alone publishes :19100/:19200, and is scraped
    there with NO exec. (Positive control: without it the exec test above would also pass if
    node_metrics stopped scraping altogether.)"""
    argvs, urls = _metrics_transport(monkeypatch, "validator-0")
    assert urls == ["http://localhost:19100/metrics", "http://localhost:19200/metrics"]
    assert argvs == []


# ── _enode_pubkey (lib.sh port; v61.7 L3 downstream-crash root) ──────────────
_PK = "a" * 128


@pytest.mark.parametrize("enode,expect", [
    (f"enode://{_PK}@172.20.0.250:30303", _PK),           # well-formed → 128-hex pubkey
    (f"enode://{_PK}@10.0.0.1:9999", _PK),                # embedded IP/port irrelevant
    ("enode://@172.20.0.250:30303", ""),                  # THE v61.7 shape: empty id → ""
    (f"enode://{'a' * 64}@1.2.3.4:30303", ""),            # too-short id (64) → ""
    ("not-an-enode", ""),                                 # malformed → ""
])
def test_enode_pubkey_extract_and_validate(monkeypatch, enode, expect):
    monkeypatch.setattr(rpc, "_run_read", lambda cmd, t: json.dumps({"enode": enode}))
    assert rpc._enode_pubkey("http://localhost:18545") == expect


def test_enode_pubkey_empty_on_rpc_down(monkeypatch):
    """admin_nodeInfo unreachable → `_run_read` returns "" → coerced to "" (caller hard-fails)."""
    monkeypatch.setattr(rpc, "_run_read", lambda cmd, t: "")
    assert rpc._enode_pubkey("http://localhost:18545") == ""


def test_enode_pubkey_empty_on_missing_field(monkeypatch):
    monkeypatch.setattr(rpc, "_run_read", lambda cmd, t: json.dumps({"id": "x"}))
    assert rpc._enode_pubkey("http://localhost:18545") == ""


def test_enode_pubkey_queries_admin_nodeinfo(monkeypatch):
    """Faithful to lib.sh: `cast rpc --rpc-url <url> admin_nodeInfo`."""
    seen = {}

    def spy(cmd, t):
        seen["cmd"] = cmd
        return ""

    monkeypatch.setattr(rpc, "_run_read", spy)
    rpc._enode_pubkey("http://host:18545")
    assert seen["cmd"] == ["cast", "rpc", "--rpc-url", "http://host:18545", "admin_nodeInfo"]


# ── read-only enforcement (module contract) ──────────────────────────────────
def test_nodes_module_is_read_only():
    """nodes.py MUST NOT contain any topology-mutating docker/cast verb. Greps the
    source (minus the module docstring, which legitimately NAMES the forbidden
    verbs in its read-only contract) for the forbidden subcommands."""
    import ast

    src = open(nodes.__file__).read()
    doc = ast.get_docstring(ast.parse(src))
    body = src.replace(doc, "") if doc else src
    forbidden = ['"up"', '"down"', '"restart"', '"stop"', '"start"',
                 '"rm"', '"kill"', '"send"', "force-recreate", "--force-recreate"]
    hits = [tok for tok in forbidden if tok in body]
    assert not hits, f"nodes.py contains topology-mutating verb(s): {hits}"
