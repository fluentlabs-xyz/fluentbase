"""nodes.py — READ-ONLY node / metric / docker-log read helpers.

Python port of the READ-SIDE helpers in lib.sh and soak-invariants.sh: the
finalized/height reads (check_node/check_external/finalized_dec), block hash &
prev_randao reads, the consensus two-tier read, the per-tier finalized reads, the
in-container own-finalized + executed-root reads, the metrics scrape + its metric/
gauge/summary parsers, the cast read wrappers (staking/chainconfig/liveness) and
their participation/status/committee derivations, per-node ceilings, and the
docker-logs read (ANSI-stripped).

READ-ONLY BY CONSTRUCTION — module contract:
    This module MUST NOT mutate the topology. It contains NO docker up / down /
    restart / stop / start / rm / kill / force-recreate, no `cast send`, no
    exec-write (every `docker compose exec` runs curl/cat/cast reads only). A
    pytest (test_rpc.py::test_nodes_module_is_read_only) greps this module for the
    forbidden verbs and fails if any appear. Attaching this to a LIVE sim is safe.

PORT-NOTE: bash coerced an unreachable RPC to 0 / "null" / the "null|null"
sentinel; the Python ports preserve those exact sentinels so downstream verdict
logic (finalize-stall baseline, k-lag, beacon reads) behaves identically.
"""

from __future__ import annotations

import json
import os

from . import rpc, topology
from .rpc import (
    block_field_at,
    block_field_of,
    compose_exec,
    metrics_get_exec,
    metrics_get_url,
    rpc_body,
    rpc_post_exec,
    rpc_post_url,
    strip_ansi,
    cast_field,
)

# The ambient host RPC — validator-0's, unless an operator repoints it. The DEFAULT is a
# topology fact (which node publishes 8545); the override is config, so it stays here.
RPC = os.environ.get("RPC", topology.DEFAULT_RPC_URL)

JSONRPC_FINALIZED = rpc_body("eth_getBlockByNumber", ["finalized", False])


# --- height/hash reads -----------------------------------------------------

def _num_hash(out: str):
    """(.result.number, .result.hash) or ("null","null") — the check_node/
    check_external empty->{}->null|null coercion."""
    try:
        obj = json.loads(out)
        res = obj.get("result") or {}
        num = res.get("number")
        h = res.get("hash")
    except Exception:
        num, h = None, None
    return (num if num is not None else "null", h if h is not None else "null")


def check_node(service: str) -> str:
    """"height|hash" as seen INSIDE container <service>, or "null|null" if
    unreachable (the all-nodes-down false-pass guard: an unreachable node MUST
    read the null|null sentinel, never "")."""
    num, h = _num_hash(rpc_post_exec(JSONRPC_FINALIZED, compose_exec(service)))
    return f"{num}|{h}"


def check_external(port: int) -> str:
    """"height|hash" at a host-mapped RPC port, or "null|null" if unreachable."""
    num, h = _num_hash(rpc_post_url(topology.host_url(port), JSONRPC_FINALIZED))
    return f"{num}|{h}"


def hex_to_dec(v) -> int:
    """printf '%d' of a hex/dec string; 0 on non-numeric (bash `|| echo 0`)."""
    try:
        s = str(v).strip()
        if s.startswith("0x") or s.startswith("0X"):
            return int(s, 16)
        return int(s)
    except Exception:
        return 0


def finalized_dec() -> int:
    """Decimal chain-wide finalized height. LIVENESS-FIRST + resilient: reads the pinned host RPC
    (validator-0 : port 8545) first — the fast steady-state path — but if that is 0/unreachable
    (v0 restarting, recreate churn, or dead) it FALLS BACK to the max finalized across the
    OTHER running committee containers. A single node going down — even the pinned RPC host — must
    never zero the chain-wide liveness signal: the chain is alive iff SOME node is finalizing, not
    iff v0's container is up. Only a genuinely dead chain (every node at 0) coerces to 0. An
    unreachable RPC still coerces to 0 (fine for `>= target` polling; 0 is never a valid baseline)."""
    num = check_external(topology.HOST_RPC_PORT).split("|", 1)[0]
    fin = 0 if num == "null" else hex_to_dec(num)
    if fin > 0:
        return fin
    # v0 dark — poll the rest of the committee so one node's death can't fake a liveness failure.
    best = 0
    for svc in running_services():
        if not topology.is_validator(svc) or topology.is_pinned_rpc_host(svc):
            continue
        h = node_fin_in(svc)          # -1 if that node is also unreachable
        if h > best:
            best = h
    return best


def is_zero_hash(h: str) -> bool:
    """True iff h is 0x000...0 (beacon fell to digest / stall)."""
    if not h or not h.startswith("0x"):
        return False
    return set(h[2:]) <= {"0"}


def mixhash_of(service: str, block) -> str:
    return block_field_of(service, block, "mixHash", RPC)


def mixhash_at(block, rpc_url: str = None) -> str:
    """mixHash of <block> over the HOST `cast block` (lib.sh `mixhash_at`, whose RPC-url
    argument defaults to $RPC — see ee0cf6a9: the default belongs in THIS frame, and losing it
    broke five make targets). "null" when the block is absent / the RPC is down."""
    return block_field_at(block, "mixHash", rpc_url or RPC)


def blockhash_of(service: str, block) -> str:
    return block_field_of(service, block, "hash", RPC)


def blockhash_at(block, rpc_url: str = None) -> str:
    """Hash of <block> over the HOST `cast block` (lib.sh `blockhash_at`, same $RPC default as
    `mixhash_at`). "null" when the node does not hold that block or the RPC is down — which is
    why `wait_follower_align` can compare it directly: a missing block never matches a hash."""
    return block_field_at(block, "hash", rpc_url or RPC)


def peer_count(service: str) -> int:
    """reth devp2p peer count (decimal) INSIDE a validator container — the EL
    transport plane (distinct from commonware consensus connectivity). 0 if the
    RPC is not answering yet / no peers."""
    out = rpc_post_exec(rpc_body("net_peerCount"), compose_exec(service))
    try:
        res = json.loads(out).get("result")
    except Exception:
        res = None
    if isinstance(res, str) and res.startswith("0x"):
        return hex_to_dec(res)
    return 0


#: Whole-container log reads are not bounded by the per-RPC ceiling: a node that has been up for
#: several minutes emits megabytes, and the 15s read bound would silently truncate the answer to
#: "" — which `log_count` would then report as 0 matches, i.e. a beacon that looks dead.
LOGS_ALL_TIMEOUT = 120


def logs_all(service: str, timeout: float = LOGS_ALL_TIMEOUT) -> str:
    """The WHOLE container log of <service>, ANSI-stripped. READ-ONLY.

    ANSI-stripping is not defensive politeness (§2.4 item 2): the node writes SGR escapes INSIDE
    its `key=value` pairs, `--no-color` removes only compose's own prefix colouring, and a silent
    reader that matches nothing is the exact shape of the v57 run-killer. Stripping can never
    lower a match count, so it is safe on every existing caller and mandatory for new ones."""
    return strip_ansi(rpc._run_read(rpc.DOCKER_COMPOSE + ["logs", service], timeout))


def log_count(service: str, pattern: str) -> int:
    """Count log lines matching <pattern> in <service> (0 on no match). lib.sh `log_count`."""
    out = logs_all(service)
    if not out:
        return 0
    return sum(1 for line in out.splitlines() if pattern in line)


def head_dec() -> int:
    """Decimal HEAD height (`eth_blockNumber`) of the host RPC; 0 when unreachable.

    Deliberately NOT a finalized read: the beacon's "still active" growth check must track the
    SPECULATIVE TIP, because the log line it counts fires at notarize-time derive. Right after the
    sequencer→DPoS migration the tip bursts ahead and finalized catches up to it WITHOUT the tip
    producing new blocks, which froze the old finalized-based wait into a false "beacon stopped"."""
    out = rpc_post_url(RPC, rpc_body("eth_blockNumber"))
    try:
        res = json.loads(out).get("result")
    except Exception:
        res = None
    return hex_to_dec(res) if res is not None else 0


def logs_since(service: str, since: str, no_color: bool = True) -> str:
    """Read <service>'s docker-compose logs since <since> (a Go-duration like
    "30s" or an RFC3339 cursor), ANSI-stripped. READ-ONLY."""
    cmd = rpc.DOCKER_COMPOSE + ["logs"]
    if no_color:
        cmd.append("--no-color")
    cmd += ["--since", since, service]
    return strip_ansi(rpc._run_read(cmd, rpc.RPC_EXEC_TIMEOUT))


# --- two-tier consensus / eth reads ----------------------------------------

def sim_consensus_latest():
    """consensus_getLatest -> (latestFinalized.height, latestResultFinalized),
    both decimal, (0, 0) if unreachable. COV-3: latestFinalized serializes as a
    BLOCK OBJECT (its height is `.latestFinalized.height`); latestResultFinalized
    IS a scalar."""
    out = rpc_post_url(RPC, rpc_body("consensus_getLatest"))
    try:
        res = json.loads(out).get("result") or {}
    except Exception:
        res = {}
    lf = res.get("latestFinalized") or res.get("latest_finalized") or {}
    lf_h = lf.get("height") if isinstance(lf, dict) else None
    lrf = res.get("latestResultFinalized")
    if lrf is None:
        lrf = res.get("latest_result_finalized")
    return (hex_to_dec(lf_h) if lf_h is not None else 0,
            hex_to_dec(lrf) if lrf is not None else 0)


def sim_eth_latest() -> int:
    """eth latest height (decimal) from host RPC, 0 if unreachable."""
    out = rpc_post_url(RPC, rpc_body("eth_getBlockByNumber", ["latest", False]))
    try:
        num = (json.loads(out).get("result") or {}).get("number")
    except Exception:
        num = None
    return hex_to_dec(num) if num is not None else 0


def sim_fin_at(port: int) -> int:
    """Finalized height (decimal) at a host-mapped RPC port (18545=L2, 28545=L3);
    0 if unreachable."""
    out = rpc_post_url(topology.host_url(port), JSONRPC_FINALIZED)
    try:
        num = (json.loads(out).get("result") or {}).get("number")
    except Exception:
        num = None
    return hex_to_dec(num) if num is not None else 0


def node_fin_in(service: str) -> int:
    """Own finalized height (decimal) of an in-container validator, -1 if
    unreachable/not serving (the ceiling sentinel — distinct from 0)."""
    out = rpc_post_exec(JSONRPC_FINALIZED, compose_exec(service))
    try:
        num = (json.loads(out).get("result") or {}).get("number")
    except Exception:
        num = None
    if isinstance(num, str) and num.startswith("0x"):
        return hex_to_dec(num)
    return -1


def node_roots_of(service: str, block_dec: int) -> str:
    """"<stateRoot> <receiptsRoot>" of block <block_dec> as seen by <service>,
    lowercased; "null null" when unreachable / not holding the block. v0 goes via
    the host `cast block`, others via in-container eth_getBlockByNumber."""
    if topology.is_pinned_rpc_host(service):
        out = rpc._run_read(
            ["cast", "block", str(block_dec), "--rpc-url", RPC, "--json"],
            rpc.RPC_EXEC_TIMEOUT,
        )
        try:
            obj = json.loads(out) if out.strip() else {}
        except Exception:
            obj = {}
    else:
        raw = rpc_post_exec(
            rpc_body("eth_getBlockByNumber", [hex(int(block_dec)), False]),
            compose_exec(service),
        )
        try:
            obj = json.loads(raw).get("result") or {}
        except Exception:
            obj = {}
    if not isinstance(obj, dict):
        return "null null"
    sr = obj.get("stateRoot")
    rr = obj.get("receiptsRoot")
    return f"{(sr if sr is not None else 'null')} {(rr if rr is not None else 'null')}".lower()


# --- metrics scrape + parsers ----------------------------------------------

def node_metrics(service: str) -> str:
    """Raw metrics text for a node — the commonware registry (:9100) CONCATENATED
    with reth's metrics-rs recorder (:9200). v0 publishes both host-mapped
    (:19100/:19200); every other committee container is reachable only in-container.
    "" if both unreachable; family substrings are disjoint across the two registries
    so consumers keep matching on the merged text."""
    host = topology.host_metrics_urls(service)
    if host is not None:
        return metrics_get_url(host[0]) + metrics_get_url(host[1])
    return (metrics_get_exec(topology.IN_CONTAINER_CONSENSUS_METRICS_URL, compose_exec(service))
            + metrics_get_exec(topology.IN_CONTAINER_EL_METRICS_URL, compose_exec(service)))


def metric_val(text: str, family: str, label: str) -> str:
    """Value ($NF, last whitespace field) of the LAST non-comment line matching
    BOTH substrings <family> and <label> (label="" matches family alone). ""
    if absent."""
    v = ""
    for line in text.splitlines():
        if line.startswith("#"):
            continue
        if family in line and (label == "" or label in line):
            parts = line.split()
            if parts:
                v = parts[-1]
    return v


def metric_first_val(text: str, family: str) -> str:
    """Value ($NF) of the FIRST non-comment line containing <family>. "" if absent.

    FIRST, not last — `metric_val` above takes the last. The two are different parsers and the
    smoke VRF case needs this one: `asserts.sh`'s `metric()` is
    `grep <family> | grep -v '^#' | awk '{print $NF}' | head -1`. On a family that renders under
    several label sets the two answers differ, and silently swapping them would change what a
    "did it grow" comparison is comparing."""
    for line in text.splitlines():
        if line.startswith("#"):
            continue
        if family in line:
            parts = line.split()
            if parts:
                return parts[-1]
    return ""


def beacon_metric_value(text: str, family: str) -> int:
    """sim_beacon_metric's awk parse (soak-actions.sh:1276): the LAST non-comment line whose
    text CONTAINS <family> (substring, e.g. "dkg_ceremony_ok" matches "dkg_ceremony_ok_total_total")
    → its last whitespace field as an int; -1 if absent/garbled. This is the parser the dispatch /
    reconciler beacon seams (share-delta / seed-delta / dkg_ceremony_fail / demote) call live."""
    v = metric_val(text, family, "")
    if v == "":
        return -1
    try:
        return int(float(v))
    except ValueError:
        return -1


def beacon_service_for_idx(idx, val_containers, slot_identity):
    """sim_beacon_metric's container resolution (soak-actions.sh:1273-1274): idx < val_containers
    → validator-<idx> (its own native container); else the SLOT_IDENTITY container currently
    serving that logical idx (reverse-lookup). None when no live container hosts idx — the caller
    turns that into the -1 unreachable sentinel, exactly as bash's empty `$slot` echoes -1."""
    try:
        i = int(idx)
    except (TypeError, ValueError):
        return None
    if i < int(val_containers):
        return topology.validator(i)
    for host, served in (slot_identity or {}).items():
        try:
            if int(served) == i:
                return host
        except (TypeError, ValueError):
            continue
    return None


def beacon_metric(service: str, family: str) -> int:
    """sim_beacon_metric (soak-actions.sh:1270): current value of a commonware DKG counter
    <family> (SUBSTRING) on <service>'s :9100 beacon plane, or -1 if the endpoint is unreachable /
    the family is absent (a down/booting container never false-confirms). Reads :9100 ONLY (the
    commonware registry — NOT reth's :9200) via an in-container curl, byte-for-byte as bash resolves
    validator-$slot and execs (never host-mapped, even for validator-0). Parsed by
    beacon_metric_value (the awk substring / $NF / int shape the production-bytes fixtures pin)."""
    text = metrics_get_exec(topology.IN_CONTAINER_CONSENSUS_METRICS_URL, compose_exec(service))
    return beacon_metric_value(text, family)


def refill_spare_attempts(service: str) -> int:
    """_refill_spare_attempts (soak-invariants.sh:2564): the spare's convergent-retry counter
    reth_dpos_derive_db_timeout_attempts_total off <service>'s in-container reth :9200 exporter.
    0 if the family is absent (metrics-rs registers lazily — never retried = never rendered) or the
    endpoint is unreachable, exactly like bash's `[[ "$_v" =~ ^[0-9]+$ ]] && echo "$_v" || echo 0`."""
    text = metrics_get_exec(topology.IN_CONTAINER_EL_METRICS_URL, compose_exec(service))
    v = metric_val(text, "reth_dpos_derive_db_timeout_attempts_total", "").strip()
    return int(v) if v.isdigit() else 0


def gauge_val(text: str, suffix: str) -> str:
    """Value ($2) of the LAST non-comment line whose metric name ($1) ENDS in the
    bare suffix (anchored `(^|_)suffix$`, NOT substring). "" if absent."""
    import re

    pat = re.compile(r"(^|_)" + re.escape(suffix) + r"$")
    v = ""
    for line in text.splitlines():
        if line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) >= 2 and pat.search(parts[0]):
            v = parts[1]
    return v


def summary_agg(text: str, family: str):
    """Aggregate a metrics-rs SUMMARY family: (sum, count) over every
    `<family>_sum{...}` / `_sum` line and `<family>_count{...}` / `_count` line
    across ALL label sets. (0.0, 0) when absent."""
    s = 0.0
    c = 0
    sum_prefix = family + "_sum{"
    sum_exact = family + "_sum"
    cnt_prefix = family + "_count{"
    cnt_exact = family + "_count"
    for line in text.splitlines():
        if line.startswith("#"):
            continue
        parts = line.split()
        if not parts:
            continue
        name = parts[0]
        val = parts[-1]
        if name.startswith(sum_prefix) or name == sum_exact:
            try:
                s += float(val)
            except ValueError:
                pass
        elif name.startswith(cnt_prefix) or name == cnt_exact:
            try:
                c += int(float(val))
            except ValueError:
                pass
    return s, c


# --- cast read wrappers (host -> validator-0 RPC) --------------------------

def _cast_call(addr: str, sig: str, *args, rpc_url: str = None) -> str:
    cmd = ["cast", "call", addr, sig, *[str(a) for a in args],
           "--rpc-url", rpc_url or RPC]
    return rpc._run_read(cmd, rpc.RPC_EXEC_TIMEOUT)


# PRE-EXISTING HAZARD, preserved as-is (P4.1 is an extraction, not a fix): the `addr or …`
# fallbacks below reach the genesis PREDEPLOY slot, which is CODELESS in this configuration
# (see core/topology.py). A read that lands there returns an empty answer off a contract that
# does not exist, rather than failing.
#
# "Every live caller passes addr=" is TRUE but is not the guard — passing the field is not the
# same as the field being set. The runtime addresses default to "" (battery.Ctx.STAKING_RT /
# CHAIN_CONFIG_RT / LIVENESS_RT), and `"" or <predeploy>` IS the predeploy. What actually keeps
# this fallback dark is that both entry points refuse to run without the deployed addresses:
#   * sim/orchestrator.py::_build_chain — ChainError("selfcheck-no-runtime-addrs") when any of
#     the three is blank after bring-up;
#   * sim/shadow.py::resolve_runtime_addrs — ShadowAttachError on an unreadable/zero/malformed
#     address, with no empty-dict return path.
# Relax either into a warn-and-continue (or add a third entry point without the same abort) and
# the fallback fires SILENTLY: every on-chain detector then evaluates over a codeless address
# and reports "holds" for a chain it cannot see. Do not add callers that rely on the fallback,
# and do not introduce this pattern anywhere else.
def staking_call(sig: str, *args, addr: str = None, rpc_url: str = None) -> str:
    return _cast_call(addr or topology.STAKING_ADDR, sig, *args, rpc_url=rpc_url)


def chainconfig_call(sig: str, *args, addr: str = None, rpc_url: str = None) -> str:
    return _cast_call(addr or topology.CHAIN_CONFIG_ADDR, sig, *args, rpc_url=rpc_url)


def liveness_call(sig: str, *args, addr: str = None, rpc_url: str = None) -> str:
    return _cast_call(addr or topology.LIVENESS_SLASHING_ADDR, sig, *args, rpc_url=rpc_url)


def validator_status(addr: str, staking_addr: str = None, rpc_url: str = None) -> str:
    """On-chain validator status byte (2nd tuple field of getValidatorStatus):
    NotFound=0, Active/Pending/etc. The 6-field ABI tuple has ONE definition — it lost
    `slashesCount` and `jailedBefore` with the liveness jail, and a STALE signature does
    not error (cast decodes garbage or truncates), so it must never be copied per-case."""
    out = staking_call(
        "getValidatorStatus(address)(address,uint8,uint256,uint64,uint64,uint16)",
        addr, addr=staking_addr, rpc_url=rpc_url,
    )
    return cast_field(2, out)


# Runtime-deploy variants (production-path / sim deploy staking at runtime).
def pp_validator_status(addr: str, staking_rt: str, rpc_url: str = None) -> str:
    return validator_status(addr, staking_addr=staking_rt, rpc_url=rpc_url)


def pp_committee(epoch, staking_rt: str, rpc_url: str = None) -> str:
    """Committee member set at <epoch> as a sorted, lowercased, space-joined
    string (set membership comparable regardless of on-chain ordering). "" when
    empty/unreadable."""
    import re

    out = staking_call("getEpochCommittee(uint64)(address[])", epoch,
                       addr=staking_rt, rpc_url=rpc_url)
    addrs = sorted(a.lower() for a in re.findall(r"0x[0-9a-fA-F]{40}", out))
    return " ".join(addrs)


def pp_dkg_qual(epoch, staking_rt: str, rpc_url: str = None) -> str:
    """getDkgQual(epoch) normalized to a THREE-valued token: "1" (qualified),
    "0" (deferred/incumbent re-committed), "" (unreadable/garbled). The pure seams
    treat "" as UNKNOWN via dkg_qual_state, never as empty=deferred."""
    out = staking_call("getDkgQual(uint64)(bool)", epoch,
                       addr=staking_rt, rpc_url=rpc_url).strip()
    out = "".join(out.split())
    if out == "true":
        return "1"
    if out == "false":
        return "0"
    return ""


def signer_idx(addr: str, epoch, staking_addr: str = None, rpc_url: str = None) -> int:
    """0-based peer-pubkey-sorted committee index of <addr> in <epoch>, or -1 if
    absent (getEpochCommittee enumeration position)."""
    import re

    want = addr.lower()
    out = staking_call("getEpochCommittee(uint64)(address[])", epoch,
                       addr=staking_addr, rpc_url=rpc_url)
    for i, a in enumerate(re.findall(r"0x[0-9a-fA-F]{40}", out)):
        if a.lower() == want:
            return i
    return -1


def production(epoch, addr: str, rpc_url: str = None,
               staking_rt: str = None, liveness_rt: str = None):
    """Blocks PRODUCED by <addr> in <epoch>, as (produced, blocksInEpoch).

    Successor to the retired `participation()` bitmap reader. The bitmap was
    proposer-asserted and never crypto-checked, and commonware stops verifying a view's
    votes at first quorum — so a fully-live geo-distant member was credited a fraction of
    the certs it actually signed. `producedAt` counts a 2-byte record every honest voter
    checked against the consensus-supplied round leader, so it is exact.

    Sentinels (both fields together): (-1,-1)=not in committee; (-2,-2)=getter call FAILED
    (RPC error / empty / partial parse) — a -2 must be retried, NEVER treated as real 0s.
    Both reads take the RUNTIME contract address: the committee enumeration comes from the
    deployed staking cluster, the counters from the deployed liveness one."""
    idx = signer_idx(addr, epoch, staking_addr=staking_rt, rpc_url=rpc_url)
    if idx == -1:
        return -1, -1
    out = liveness_call("producedAt(uint64,uint32)(uint32)", epoch, idx,
                        addr=liveness_rt, rpc_url=rpc_url)
    produced = cast_field(1, out) if out else ""
    out = liveness_call("blocksInEpoch(uint64)(uint32)", epoch,
                        addr=liveness_rt, rpc_url=rpc_url)
    total = cast_field(1, out) if out else ""
    if not (produced and total):
        return -2, -2
    try:
        return int(produced), int(total)
    except ValueError:
        return -2, -2


# --- live-topology discovery (shadow runner; read-only) --------------------

def running_services():
    """Running sim compose services (one per line) — the panic-scan enumeration,
    scoped to the ambient COMPOSE_FILE. READ-ONLY (`docker compose ps`)."""
    out = rpc._run_read(
        rpc.DOCKER_COMPOSE + ["ps", "--status", "running", "--format", "{{.Service}}"],
        rpc.RPC_EXEC_TIMEOUT,
    )
    return [s for s in out.splitlines() if s.strip()]


def down_services():
    """Sim compose services that are EXITED/DEAD as `service|state|exitcode`
    lines (the node-down complement of running_services). READ-ONLY."""
    out = rpc._run_read(
        rpc.DOCKER_COMPOSE + ["ps", "-a", "--status", "exited", "--status", "dead",
                              "--format", "{{.Service}}|{{.State}}|{{.ExitCode}}"],
        rpc.RPC_EXEC_TIMEOUT,
    )
    return [s for s in out.splitlines() if s.strip()]
