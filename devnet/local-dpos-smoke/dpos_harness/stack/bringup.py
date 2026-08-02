"""bringup.py — the sim_bring_up choreography. Python port of case-soak.sh:sim_bring_up
(668-903) + sim_start_cascade_l3 (909-952).

A single ordered command sequence: gen-compose → preflight teardown → phase-A bare chain →
runtime deploy (token + DeployStaking) → regen staking-reader → fund the pool → governance
(interval/jail/participation/activation) → setConsensusKeys → clean-halt at activation →
--dpos cold-restart → converge → L3 cascade. Every docker/cast/forge/forge-script command is
issued through the shared proc.Runner seam, so `--dry-run-bringup` prints the WHOLE sequence
(in order, with the env delta) without executing, to diff against the known-good bash log.

The read/converge WAITS are seam methods (`_wait_*`) that no-op under dry-run — the transcript
is about the WRITE commands, and the reads are already ported (nodes.py). Live mode delegates
them to nodes.py.

PORT-NOTE: the bash `forge_l2 forge …` wrapper (`( cd $SOLIDITY_CONTRACTS_DIR && … )`) becomes
a `cwd=` on the Runner. The DeployStaking env overrides (INITIAL_VALIDATORS/STAKES/…) ride as
an env_overlay so the recorded argv stays the bare `forge script …` line (the oracle) with the
env delta shown separately.
"""

from __future__ import annotations

import os
import time
from dataclasses import dataclass

from . import dataroot, profiles
from .profiles import GeneratedProfile
from ..core import converge, nodes, topology
from ..core.spammer import SpammerPool
from ..chain.writes import Chain, ChainError, sim_regen_staking_reader

# `SpammerPool` MOVED to `core/spammer.py` in P7 chunk 5a and is re-exported here.
# It is the substrate under `pp_spammer_start`, which SIX production-path cases call, and the
# case layer must not import the whole sim bring-up (a 1000-line module holding the generated
# profile's choreography) to reach a thread pool. The name stays bound here because two tests
# and `golden`/`growth`/`seed_continuity`/`orchestrator` reach it through this module.
__all__ = ["SpammerPool", "StackSpec", "BringUp", "resolve_compose_env",
           "BASE_COMPOSE", "DPOS_COMPOSE"]

# The two ambient COMPOSE_FILE values case-soak.sh exports (its base-compose export :347 /
# its dpos-compose export :348). RELATIVE names with a `:` separator — compose resolves each
# against the process cwd (the smoke dir), exactly as the bash `export COMPOSE_FILE=...`
# did. Single source so bring-up (phase A/B) and the attach path (resolve_compose_env)
# agree byte-for-byte.
#
# They are DERIVED from `GeneratedProfile` rather than restated, so this module and the profile
# layer cannot disagree about which files the sim runs on — the whole point of §3.4's two
# profiles is that the compose identity lives in one place per profile.
BASE_COMPOSE = profiles.GENERATED_BASE
DPOS_COMPOSE = f"{profiles.GENERATED_BASE}:{profiles.GENERATED_DPOS_OVERLAY}"

# Bounded-retry budget for the cascade enode-pubkey reads (30×2s = 60s). Bash reads L2's pubkey
# ONCE (case-soak.sh:914), relying on the cold-restart `_wait_aligned _read_sim_nodes` gate
# (:890, which INCLUDES the full-node @18545) to guarantee the node is RPC-up first. The port now
# runs that same alignment gate (bringup.py:_wait_aligned), but this bounded retry is KEPT as a
# belt (the gate's floor is the finalized head, which can be met a beat before admin_nodeInfo
# answers) — BOTH enode reads use bash's L3 retry discipline (:923-925) at the read site.
_ENODE_PK_RETRIES = 30
_ENODE_PK_SLEEP = 2

# `_wait_aligned` poll cadence — bash sleeps 1s between reads (lib.sh:202).
_ALIGN_POLL_S = 1


@dataclass(frozen=True)
class StackSpec:
    """Everything BringUp needs from ABOVE it, handed down by the caller.

    WHY IT EXISTS: bring-up used to be handed the simulation's `SimConfig` and DUCK-TYPE seven
    attributes off it. That one habit is what stopped `stack/` from being usable by anything but
    the sim — a smoke case with no interest in calm fractions or stake bands still had to
    construct the sim's ~45-knob config object to boot a chain. The fields below are exactly the
    ones the choreography reads and nothing else; anything a future consumer needs must be ADDED
    here deliberately, which is the point.

      validators        — the COMMITTEE target: how many containers come up under `--dpos` at
                          the cold restart, and how many nodes the converge gate must align.
      val_containers    — total validator CONTAINERS (committee + spares + rotation slots).
                          Containers in [validators, val_containers) are stopped before the
                          --dpos restart, and this is the N compose is generated for.
      initial_committee — the genesis on-chain set: DeployStaking's INITIAL_VALIDATORS /
                          INITIAL_STAKES, the setConsensusKeys loop, and PP_GOV_VOTERS.
      identity_pool     — total funded IDENTITIES (>= val_containers): the post-deploy token
                          grant covers [initial_committee, identity_pool).
      no_cascade        — 1 omits the L2 full-node + L3 downstream tiers. Compared against 1
                          (not truthiness), faithfully to the env contract it came from.
      byzantine         — 1 runs the `--dpos.byzantine equivocate --help` flag-parse assert
                          BEFORE anything schedules a byzantine action. Same ==1 comparison.
    """
    validators: int
    val_containers: int
    initial_committee: int
    identity_pool: int
    no_cascade: int = 0
    byzantine: int = 0


def _geo_wait_margin(val_containers: int) -> int:
    """SIM_GEO_WAIT_MARGIN (case-soak.sh:129-130): 0 unless SIM_GEO_LATENCY=1, in which case
    120 + val_containers*15 — widens the event-driven observation windows under injected latency.
    Every converge/L3 timeout below adds it so the port's budgets match bash byte-for-byte."""
    if os.environ.get("SIM_GEO_LATENCY", "0") == "1":
        return 120 + val_containers * 15
    return 0


def resolve_compose_env(out_dir: str = ".") -> str:
    """Attach path (shadow / status — no bring-up): mirror the process-global COMPOSE_FILE
    that sim_bring_up would have exported, so every BARE `docker compose exec` read (rpc.py /
    nodes.py) resolves the sim project instead of the default docker-compose.yml (the fin=-1 /
    roots=null sentinel class). Honors an already-exported COMPOSE_FILE (operator / Makefile);
    else DISCOVERS the generated compose files on disk the way soak-status.sh discovers a running
    sim — the dpos overlay chain when both gen files exist (a running sim is in phase B), else
    the bare base. Returns the resolved value (\"\" if nothing generated yet). Idempotent."""
    existing = os.environ.get("COMPOSE_FILE")
    if existing:
        return existing
    base_path = os.path.join(out_dir, "docker-compose.sim.gen.yml")
    dpos_path = os.path.join(out_dir, "docker-compose.sim.dpos.gen.yml")
    if os.path.exists(base_path) and os.path.exists(dpos_path):
        val = DPOS_COMPOSE
    elif os.path.exists(base_path):
        val = BASE_COMPOSE
    else:
        return ""
    os.environ["COMPOSE_FILE"] = val
    return val


class BringUp:
    """Drives sim_bring_up. Construct with a `StackSpec` and a proc.Runner (dry or live); call
    run(). In dry mode the deploy manifest reads return canned addresses so the sequence walks
    to the end. Exposes the post-deploy facts (staking_rt, …) the orchestrator threads onward."""

    def __init__(self, spec: StackSpec, runner, chain: Chain = None, profile=None):
        # Enforced, not merely annotated: a `StackSpec` that anything with the right seven
        # attributes could stand in for is the duck-type it replaced, and this codebase's
        # recurring failure is exactly the contract nobody checks. The sim converts its own
        # config with `SimConfig.stack_spec()`; a case builds a StackSpec directly.
        if not isinstance(spec, StackSpec):
            raise TypeError(
                f"BringUp needs a StackSpec, got {type(spec).__name__} — build one explicitly "
                "(SimConfig.stack_spec() on the sim side)")
        self.spec = spec
        self.p = runner
        # WHICH DEVNET this bring-up drives. `GeneratedProfile` is the only one that fits this
        # choreography (a runtime contract deploy against N generated containers); the static
        # compose pair has its own lifecycle in `stack/static_stack.py`. The parameter exists so
        # the compose identity is asked for rather than hardcoded here, which is what lets both
        # profiles be inspected/compared by the same tooling.
        self.profile = profile if profile is not None else GeneratedProfile.for_spec(spec)
        self.rpc = os.environ.get("RPC", topology.DEFAULT_RPC_URL)
        self.contracts_dir = os.environ.get("SOLIDITY_CONTRACTS_DIR", "../../../solidity-contracts")
        # MUST be ABSOLUTE, mirroring bash `MANIFEST="$(cd "$SOLIDITY_CONTRACTS_DIR" && pwd)/…"`
        # (case-production-path.sh:32). DeployStaking's `vm.writeJson(out, OUTPUT_PATH)` runs under
        # solidity-contracts' foundry.toml with root=cwd=$SOLIDITY_CONTRACTS_DIR, whose fs_permissions
        # only allow `./deployments`. Forge resolves a RELATIVE OUTPUT_PATH against that root, so a
        # relative `../../../solidity-contracts/deployments/…` escapes the allowed dir and is denied.
        # An absolute path inside <contracts>/deployments both satisfies fs_permissions and makes the
        # forge writer (cwd=contracts_dir) and the Python readers (cwd=smoke dir) resolve to ONE file.
        self.manifest = os.path.abspath(os.environ.get(
            "MANIFEST",
            os.path.join(self.contracts_dir, "deployments", "runtime-deployment.json")))
        self.base_compose = self.profile.compose_file_env("base")
        self.dpos_compose = self.profile.compose_file_env("dpos")
        # facts populated by run()
        self.token = ""
        self.staking_rt = ""
        self.chain_config_rt = ""
        self.gov_addr = ""
        self.liveness_rt = ""
        self.chain = chain  # set after deploy
        # ANCHOR = the finalized head captured right after the activation converge (case-soak.sh:864);
        # the FLOOR every post-activation alignment gate (cold-restart :890, L3 :934) must finalize
        # strictly PAST. Dry stays "0x0" (the poll no-ops under dry anyway).
        self.anchor = "0x0"
        self.spammers = SpammerPool(dry=runner.dry)  # F11: main + L3 tx pressure; stopped in teardown

    def _set_compose(self, value: str):
        """Set the ambient COMPOSE_FILE for EVERY subsequent subprocess — mirroring the bash
        `export COMPOSE_FILE=...` (case-soak.sh :675 phase A / :877 phase B). It goes to BOTH:
          * os.environ — so the BARE `docker compose exec` READ helpers (rpc.py::_run_read,
            nodes.py), which inherit only os.environ (no Runner env), resolve the sim project;
          * the Runner's own env — the write side merges self.env AFTER os.environ, so a stale
            \"\" seeded there at construction (Orchestrator._compose_env, COMPOSE_FILE unset at
            startup) would otherwise SHADOW os.environ and re-break the write path.
        Setting both is bash-faithful (one process-global value) and keeps writes+reads in lockstep."""
        os.environ["COMPOSE_FILE"] = value
        self.p.env["COMPOSE_FILE"] = value

    # -- read/converge seams (no-op under dry; delegate to nodes.py live) --------
    def _wait_aligned(self, timeout, floor, reader):
        """Multi-node alignment poll — faithful port of bash `_wait_aligned` (lib.sh:172-205).

        Every `_ALIGN_POLL_S` seconds, read ALL nodes via `reader` (a bound method returning a
        list of `(label, "height|hash")` pairs, THE PINNED PRODUCER FIRST). Succeed only when, for
        EVERY reading: (a) the head is non-null, non-empty and past genesis (not "null"/"0x0"),
        (b) when `floor` is a hex string, the head is NUMERICALLY GREATER than it (a head merely
        DIFFERENT from the floor could be a regressed chain — bash's exact guard), and (c) the
        producer's block AT THAT NODE'S OWN HEIGHT is that node's block.

        (c) is SAME-HEIGHT identity, not tip identity, and this docstring used to say the
        opposite — "all readings are byte-identical" — which is the rule 3f47bb91 removed and the
        reason it had to be removed: the nodes are read serially on a chain producing a block a
        second, so ragged heights are the healthy steady state and demanding coincident tips is a
        race. The fork check is what the compare existed for and it survives intact; only the
        demand that the tips coincide is gone. A byte-identical set still short-circuits before
        any producer read. Returns the producer's "height|hash" reading. Bounded by `timeout`
        seconds; on expiry raises a LOUD
        ChainError naming every node's last reading so the laggard/divergent set is visible (bash
        returns 1 and its caller dumps the divergent nodes + logs). Dry → no-op True (the transcript
        is about the WRITE commands; no reads, no sleeps).

        The POLL itself lives in `core/converge.py`, shared with the static stack: the sentinel
        discipline it encodes (a `"null"` head is not convergence, `"0x0"` is not convergence, a
        floor is a STRICT numeric floor and a non-hex floor means no floor) is the same
        discipline in both devnets, and two copies of it is two chances to lose one."""
        if self.p.dry:
            return True
        reading, last = converge.wait_aligned(timeout, floor, reader, poll_s=_ALIGN_POLL_S)
        if reading is not None:
            return reading
        floor_msg = (f" past floor {floor}" if converge.hex_floor(floor) is not None else "")
        raise ChainError("wait-aligned",
                         f"nodes did not converge{floor_msg} within {timeout}s; "
                         f"last readings: {converge.divergence_detail(last)}")

    @staticmethod
    def _read_host_node(service):
        """(label, "height|hash") for a service that publishes its RPC on the host — the port
        comes from topology.HOST_RPC_PORTS, the same map compose_gen publishes from, so the
        reader cannot aim at a port the compose file does not expose."""
        port = topology.host_rpc_port(service)
        return (f"{service}@{port}", nodes.check_external(port))

    def _read_sim_nodes(self):
        """Bash `_read_sim_nodes` (case-soak.sh:358-363): the committee's finalized heads —
        validator-0 via the host RPC (8545), validator-1..validators-1 via `docker compose exec`,
        plus the full-node via host 18545 (absent when the cascade is omitted). One (label,
        "height|hash") pair per node.

        The host-vs-exec choice is `topology.has_host_rpc`, not a hardcoded validator-0: today
        only v0 publishes, so this is the same read it always was, but publishing a second
        committee node would change BOTH the compose file and this reader from the one edit."""
        out = [self._read_host_node(topology.PINNED_RPC_HOST)]
        for i in range(1, self.spec.validators):
            svc = topology.validator(i)
            out.append(self._read_host_node(svc) if topology.has_host_rpc(svc)
                       else (svc, nodes.check_node(svc)))
        if self.spec.no_cascade != 1:
            out.append(self._read_host_node(topology.FULL_NODE))
        return out

    def _read_cascade_node(self):
        """Bash `_read_cascade_node` (case-soak.sh:907): the L3 downstream reth RPC (host 28545)."""
        return [self._read_host_node(topology.DOWNSTREAM)]

    def _finalized_dec(self):
        return 0 if self.p.dry else nodes.finalized_dec()

    def _head_hex(self):
        return ("0x0" if self.p.dry
                else nodes.check_external(topology.HOST_RPC_PORT).split("|", 1)[0])

    def _capture_enode_pubkey(self, url):
        """Bounded-retry read of a reth node's 128-hex enode pubkey (rpc._enode_pubkey via
        admin_nodeInfo), "" if it never appears within _ENODE_PK_RETRIES×_ENODE_PK_SLEEP. Used for
        BOTH cascade tiers: bash waits for the full-node via the cold-restart converge gate before
        its single L2 read, but that gate is a no-op seam in this port, so we poll here exactly as
        bash polls for L3's pubkey. Dry → canned 128 zeros (no RPC, no sleep)."""
        if self.p.dry:
            return "0" * 128
        pk = ""
        for _ in range(_ENODE_PK_RETRIES):
            pk = nodes.rpc._enode_pubkey(url)
            if pk:
                break
            time.sleep(_ENODE_PK_SLEEP)
        return pk

    # -- the choreography --------------------------------------------------------
    def run(self):
        spec = self.spec
        # 1. generate compose for N containers (COMPOSE_FILE scopes every later compose call).
        # Transcript-only marker: compose_gen.generate() writes its own two files, so there is
        # nothing for the Runner to write here — only a line to record so the dry-run sequence
        # shows WHERE generation happens. This used to be a write_file() whose PATH argument was
        # the label `<compose-gen>`, which is how a 19-byte file of that name kept reappearing
        # in the smoke directory on every live run.
        self.p.step("compose-gen",
                    f"N={spec.val_containers} target={spec.validators} pool={spec.identity_pool}")
        self.profile.prepare(self.p)
        self._set_compose(self.base_compose)

        # 2. UNCONDITIONAL preflight teardown (FLK-1) + data-root wipe. F12: checked + generous
        # ceiling — a failed teardown leaves stale volumes that shift the deploy CREATEs (the
        # v51/v54 nonce-war class), so it must abort loudly, never swallow a False.
        self.p.run_checked(["docker", "compose", "down", "-v", "--remove-orphans"],
                           timeout=300, note="preflight-down")
        self._data_root_wipe()

        # 3. byzantine flag parse assert (BEFORE scheduling byz). Lifecycle `compose run --rm`:
        # checked so a flag that no longer parses aborts loudly instead of being swallowed.
        if spec.byzantine == 1:
            self.p.run_checked(["docker", "compose", "run", "--rm", "--no-deps", "-T",
                                "--entrypoint", "/usr/local/bin/fluent", topology.GENESIS_INIT,
                                "node", "--dpos.byzantine", "equivocate", "--help"],
                               timeout=120, note="byz-flag-assert")

        # 4. phase A: bare sequencer chain. F12: `up --build` is the slow image build — checked +
        # generous ceiling so a build failure aborts the run loudly instead of proceeding on nothing.
        self.p.run_checked(["docker", "compose", "up", "--build", "-d"], timeout=1800,
                           note="phaseA-up")
        # Converge timeout scales with the container count (staggered boot), not the epoch interval
        # (phase A only needs the first few blocks aligned) — bash `converge_wait` (case-soak.sh:703),
        # reused at the activation converge (:862) and, +90, at the cold-restart converge (:889).
        converge_wait = 180 + spec.val_containers * 20 + _geo_wait_margin(spec.val_containers)
        self._wait_aligned(converge_wait, "", self._read_sim_nodes)

        # 5. deployer + spammer accounts.
        chain0 = Chain(runner=self.p, RPC=self.rpc, CHAIN_ID=os.environ.get("CHAIN_ID", str(topology.CHAIN_ID)))
        deployer_key = chain0.owner_key(0)
        deployer_addr = chain0.owner_addr(0)
        mnem = os.environ.get(
            "FLUENT_DPOS_MNEMONIC",
            "test test test test test test test test test test test junk")
        spammer_addr = self.p.run(["cast", "wallet", "address", "--mnemonic", mnem,
                                    "--mnemonic-index", "6"], note="spammer-addr")
        spammer_key = self.p.run(["cast", "wallet", "private-key", "--mnemonic", mnem,
                                  "--mnemonic-index", "6"], note="spammer-key")
        # leaked-state tripwire (one RPC before the first deployer tx).
        self._assert_fresh_deployer(deployer_addr)
        self.p.run_ok(["cast", "send", spammer_addr, "--value", "1000000000000000",
                       "--rpc-url", self.rpc, "--private-key", deployer_key], note="fund-spammer")
        # F11 (pp_spammer_start, case-soak.sh:731): start the main tx spammer so the proposer pool
        # is never empty for the rest of the run. Spammer key issues NO other txns (else its nonce
        # races the deploy/registration txns). Stopped by the orchestrator teardown (bu.spammers).
        self.spammers.start(spammer_key, deployer_addr, self.rpc, note="main")

        # 6. runtime deploy: token + DeployStaking (self-deploys verifier + decoder).
        self.token = self._deploy_token(deployer_key)
        self._deploy_staking(deployer_addr, deployer_key)
        self._read_manifest()

        # 7. derive-don't-predict: regen staking-reader.json from the manifest.
        chain = Chain(runner=self.p, RPC=self.rpc, STAKING_RT=self.staking_rt,
                      CHAIN_CONFIG_RT=self.chain_config_rt, GOV_ADDR=self.gov_addr,
                      LIVENESS_RT=self.liveness_rt, TOKEN=self.token,
                      CHAIN_ID=os.environ.get("CHAIN_ID", str(topology.CHAIN_ID)))
        self.chain = chain
        if not self.p.dry:
            body = sim_regen_staking_reader(self.manifest)
        else:
            body = "{}"
        chain.runtime_write("staking-reader.json", body)

        # 8. fund ALL joiners/spares/bench identities (post-deploy).
        for i in range(spec.initial_committee, spec.identity_pool):
            chain.token_transfer(self.token, chain.owner_addr(i), "100000000000000000000")

        os.environ["PP_GOV_VOTERS"] = str(spec.initial_committee)

        # 9. governance: epoch block interval.
        interval = os.environ.get("SIM_EPOCH_INTERVAL", "64")
        chain.gov_action(self.chain_config_rt,
                         chain.calldata("setEpochBlockInterval(uint32)", interval),
                         "setEpochBlockInterval")
        # (removed) the jail-length and participation-jail-kill-switch gov calls: both setters
        # are DELETED from ChainConfig with the liveness jail, so they would hit dead selectors.
        # The production-liveness tier that replaces it ships FLAG-OFF from genesis and stays off
        # — arming it is Phase-5 work gated on the pre-enable evidence.

        # 10. setConsensusKeys for the INITIAL committee.
        for i in range(spec.initial_committee):
            ck = chain.consensus_keys(i)
            chain.send(f"setConsensusKeys v{i}", self.staking_rt,
                       "setConsensusKeys(address,bytes,bytes,bytes32)",
                       ck.get("validatorAddress", chain.owner_addr(i)),
                       ck.get("blsPubkeyUncompressed", ""), ck.get("blsPoPUncompressed", ""),
                       ck.get("peerPubkey", ""), key=chain.owner_key(i))

        # 11. compute activation (2 epochs ahead) + gov setDposActivationBlock.
        head = _hex(self._head_hex())
        act = ((head // int(interval)) + 2) * int(interval)
        os.environ["DPOS_ACTIVATION_BLOCK"] = str(act)
        chain.gov_action(self.chain_config_rt,
                         chain.calldata("setDposActivationBlock(uint64)", act),
                         "setDposActivationBlock")

        # 12. CHAIN-PACED clean-halt wait at activation (case-soak.sh:854-863): FOLLOW finalized to
        # ACT before touching the committee — the sequencer clean-halts at the activation block, so
        # bring-up must not return / cold-restart until finalized >= ACT (else run() self-checks a
        # still-bare, pre-activation chain, the fin=216-vs-act=360 premature-return class). Budget =
        # (ACT-HEAD) tip blocks + one interval of K-lag/settle margin (bash `ACT-HEAD+interval`);
        # cp.wait no-ops under dry (transcript is about the WRITE commands). Then converge + stop spares.
        self.chain.cp.wait("activation", lambda: self._finalized_dec() >= act, "blocks",
                           act - head + int(interval))
        self._wait_aligned(converge_wait, "", self._read_sim_nodes)
        # Capture the aligned finalized head as the ANCHOR (case-soak.sh:864) — the floor every
        # post-activation gate below must finalize strictly PAST (dry stays "0x0").
        self.anchor = self._head_hex()
        for s in range(spec.validators, spec.val_containers):
            self.p.run_ok(["docker", "compose", "stop", topology.validator(s)],
                          note="stop-spare")

        # 13. phase B: --dpos overlay + cold-restart of the committee (+ full-node).
        self._set_compose(self.dpos_compose)
        up_list = [topology.validator(i) for i in range(spec.validators)]
        if spec.no_cascade != 1:
            up_list.append(topology.FULL_NODE)
        # F12: the --dpos cold-restart is a lifecycle command — checked so a failed force-recreate
        # aborts loudly rather than leaving the committee half-restarted.
        self.p.run_checked(["docker", "compose", "up", "-d", "--force-recreate", *up_list],
                           timeout=600, note="dpos-cold-restart")
        # Cold-restart converge: boot N nodes under --dpos + first DKG + finalize PAST the anchor
        # (bash `cold_wait = converge_wait + 90`, case-soak.sh:889-890; ANCHOR floor).
        self._wait_aligned(converge_wait + 90, self.anchor, self._read_sim_nodes)

        # 14. L3 cascade (unless SIM_NO_CASCADE).
        if spec.no_cascade != 1:
            self._start_cascade_l3()
        return True

    # -- golden fast path --------------------------------------------------------
    def run_from_golden(self, restore, load_facts):
        """FAST PATH: restore the golden DPoS-active snapshot and start the validators DIRECTLY
        under --dpos — skipping phase-A / activation / migration entirely. Reuses the EXISTING
        populated-/runtime resume path (crash-survivor / full-restart): the restored volume holds
        reth's MDBX + static files + commonware journals + keys, so the nodes resume finalizing.

        The caller must have confirmed the golden is fresh. Populates the deploy facts
        (staking_rt/…/token) from the golden facts sidecar so the case can drive on-chain actions
        without a re-deploy. Returns True once the committee re-converges (aligned finalized > 0).

        `restore(runner) -> volume` and `load_facts() -> dict` are passed IN rather than imported:
        `golden` already imports this module to build a snapshot, so importing it back made a
        same-layer cycle out of what is really a caller's decision — golden-vs-fresh. Both stay
        CALLABLES, not eager values, so `load_facts` is still read AFTER the restore, exactly
        where it was: a missing facts sidecar must fail at the same point it always did."""
        t0 = time.time()
        restore(self.p)
        self._set_compose(self.dpos_compose)
        # `start` (not `up`) the pre-created validators: start does NOT evaluate depends_on, so
        # genesis-init stays created-but-unrun and never rewrites the restored /runtime.
        val_list = [topology.validator(i) for i in range(self.spec.validators)]
        self.p.run_checked(["docker", "compose", "start", *val_list], timeout=300,
                           note="golden-start")
        # Recover the deploy facts captured at build time (no re-deploy on the fast path).
        facts = load_facts()
        self.token = facts["token"]
        self.staking_rt = facts["staking"]
        self.chain_config_rt = facts["chain_config"]
        self.gov_addr = facts["governance"]
        self.liveness_rt = facts["liveness_slashing"]
        self.chain = Chain(runner=self.p, RPC=self.rpc, STAKING_RT=self.staking_rt,
                           CHAIN_CONFIG_RT=self.chain_config_rt, GOV_ADDR=self.gov_addr,
                           LIVENESS_RT=self.liveness_rt, TOKEN=self.token,
                           CHAIN_ID=os.environ.get("CHAIN_ID", str(topology.CHAIN_ID)))
        os.environ["PP_GOV_VOTERS"] = str(self.spec.initial_committee)
        # Cold-resume converge: nodes re-peer + finalize past their restored head (aligned, >0).
        converge_wait = 120 + self.spec.val_containers * 15
        self._wait_aligned(converge_wait, "", self._read_sim_nodes)
        print(f"GOLDEN restore: DPoS-active via snapshot in {time.time() - t0:.0f}s "
              "(skipped phase-A/activation/migration)", flush=True)
        return True

    # -- deploy sub-steps --------------------------------------------------------
    def _deploy_token(self, deployer_key) -> str:
        out = self.p.run(["forge", "create", "--rpc-url", self.rpc, "--private-key", deployer_key,
                          "--broadcast", "--json",
                          "contracts/staking/mocks/MockBlendToken.sol:MockBlendToken"],
                         cwd=self.contracts_dir, note="deploy-token")
        if self.p.dry:
            return "0xToKeN0000000000000000000000000000000000"
        try:
            import json
            return json.loads(out).get("deployedTo", "")
        except Exception:
            return ""

    def _deploy_staking(self, deployer_addr, deployer_key):
        spec = self.spec
        iv = ",".join(_pp_addr_dry(i) if self.p.dry else Chain(runner=self.p, RPC=self.rpc)
                      .owner_addr(i) for i in range(spec.initial_committee))
        ist = ",".join("5000000000000000000" if j == 0 else "1000000000000000000"
                       for j in range(spec.initial_committee))
        overlay = {
            "NETWORK": "local-dpos-smoke/l2", "DEPLOYER": deployer_addr,
            "INITIAL_OWNER": deployer_addr, "STAKING_TOKEN": self.token,
            "OUTPUT_PATH": self.manifest, "INITIAL_VALIDATORS": iv, "INITIAL_STAKES": ist,
            "ACTIVE_VALIDATORS_LENGTH": str(spec.initial_committee),
        }
        # F12: DeployStaking's broadcast is the deploy the whole chain depends on — checked +
        # generous ceiling so a broadcast failure aborts loudly (a swallowed False here is the
        # v51/v54 nonce-war class: the run continues on a chain with no staking cluster).
        self.p.run_checked(
            ["forge", "script", "scripts/deploy/DeployStaking.s.sol:DeployStaking",
             "--rpc-url", self.rpc, "--private-key", deployer_key,
             "--broadcast", "--skip-simulation"],
            env_overlay=overlay, timeout=600, cwd=self.contracts_dir, note="DeployStaking")

    def _read_manifest(self):
        if self.p.dry:
            # DRY-RUN STAND-INS ONLY — the transcript needs *some* address so the sequence
            # walks to the end. They are the genesis predeploy SLOTS, which are CODELESS
            # here (core/topology.py); nothing may read them on a live path. `gov_addr`
            # borrows the staking-pool slot: the deploy manifest's governance address has no
            # predeploy of its own, and a distinct-looking placeholder is all dry mode needs.
            self.staking_rt = topology.STAKING_ADDR
            self.chain_config_rt = topology.CHAIN_CONFIG_ADDR
            self.gov_addr = topology.STAKING_POOL_ADDR
            self.liveness_rt = topology.LIVENESS_SLASHING_ADDR
            return
        import json
        with open(self.manifest) as f:
            m = json.load(f)
        self.staking_rt = m["staking"]
        self.chain_config_rt = m["chain_config"]
        self.gov_addr = m["governance"]
        self.liveness_rt = m["liveness_slashing"]

    def _data_root_wipe(self):
        """HARD-guarded busybox wipe of `$SIM_DATA_ROOT/runtime` (no-op if unset). CONTENTS
        only; runs as root inside a throwaway container (reth writes root-owned files).

        Shared with the static stack's teardown via `stack/dataroot.py` — the guard set (unset
        root, non-absolute path, verify-empty-in-the-same-container) is what keeps this from
        being pointed at `/` or silently not wiping, and one copy of it is enough."""
        dataroot.data_root_wipe(self.p)

    def _assert_fresh_deployer(self, addr):
        """sim_assert_fresh_deployer: deployer nonce MUST be 0 on a fresh chain (a leaked reth
        datadir shifts the deploy CREATEs → staking-address drift, v54)."""
        if self.p.dry:
            return
        n = self.p.run(["cast", "nonce", "--rpc-url", self.rpc, addr], note="fresh-deployer")
        if n != "0":
            raise ChainError("leaked-state",
                             f"deployer {addr} nonce={n or '<unreadable>'} (want 0)")

    def _start_cascade_l3(self):
        """sim_start_cascade_l3: capture L2 enode → pin for L3 → up downstream → mutual trust
        via admin_addTrustedPeer → wait L3 finalize through L2 → start the L3 write-path spammer.
        admin_nodeInfo's embedded IP is unreliable inside docker, so each enode keeps ONLY the
        128-hex pubkey and is rebuilt against the node's fixed compose IP + default devp2p 30303.
        The enode reads are seams; the WRITE commands (up/exec-write/cast rpc) are recorded."""
        # 1. capture L2's (full-node) enode pubkey and pin it for L3's --trusted-peers. Bounded-retry
        #    (the _wait_aligned gate finalizes the head but can't guarantee admin_nodeInfo answers on
        #    the same beat — v61.8: empty read at fin=306). An empty pubkey after the budget is the
        #    v61.7 downstream-crash
        #    class (`--trusted-peers=enode://@… → clap 'Failed to parse id', exit 2): hard-fail
        #    loudly (bash asserts ^[0-9a-fA-F]{128}$) rather than write a malformed l2-enode.txt.
        l2_url = topology.host_rpc_url(topology.FULL_NODE)
        pk2 = self._capture_enode_pubkey(l2_url)
        if not pk2:
            raise ChainError("cascade-l3",
                             "bad L2 enode pubkey (full-node RPC / admin_nodeInfo not up within "
                             "retry budget)")
        l2_enode = topology.enode(pk2, topology.FULL_NODE_IP)
        self.p.run_ok(["docker", "compose", "exec", "-T", topology.FULL_NODE, "sh", "-c",
                       f"printf '%s' '{l2_enode}' > /runtime/l2-enode.txt"], note="l2-enode-write")
        self.p.run_ok(["docker", "compose", "up", "-d", topology.DOWNSTREAM],
                      note="up-downstream")
        # 2. capture L3's enode ONCE its RPC is up (30×2s), then admin_addTrustedPeer so L2 ACCEPTS
        #    L3 — mutual trust so neither tier drops --trusted-only. A zero/absent pubkey (the old
        #    hardcoded '0'*128) makes the trust a no-op → L3 never peers and never syncs.
        pk3 = self._capture_enode_pubkey(topology.host_rpc_url(topology.DOWNSTREAM))
        if not pk3:
            raise ChainError("cascade-l3", "bad L3 enode pubkey (downstream RPC up?)")
        l3_enode = topology.enode(pk3, topology.DOWNSTREAM_IP)
        self.p.run_ok(["cast", "rpc", "--rpc-url", l2_url,
                       "admin_addTrustedPeer", l3_enode], note="l2-trusts-l3")
        # L3 must finalize THROUGH L2 past the anchor (its only peer), bash `l3_wait`
        # (case-soak.sh:933-934; anchor floor, _read_cascade_node reader).
        l3_wait = 150 + self.spec.val_containers * 15 + _geo_wait_margin(self.spec.val_containers)
        self._wait_aligned(l3_wait, self.anchor, self._read_cascade_node)
        # 3. F11: the L3 write-path spammer — submit to L3's RPC (28545); the write-path gossips
        # L3→L2→validator into the proposer pool, exercising the sentry cascade. DISTINCT key
        # (mnemonic index 7) so it never nonce-races the v0 spammer (index 6); funded via owner 0.
        mnem = os.environ.get(
            "FLUENT_DPOS_MNEMONIC",
            "test test test test test test test test test test test junk")
        l3_key = self.p.run(["cast", "wallet", "private-key", "--mnemonic", mnem,
                             "--mnemonic-index", "7"], note="l3-spammer-key")
        l3_addr = self.p.run(["cast", "wallet", "address", "--mnemonic", mnem,
                              "--mnemonic-index", "7"], note="l3-spammer-addr")
        # EXPORT it (bash: `export L3_SPAMMER_ADDR`, case-soak.sh:945-946). battery.Ctx sources this
        # field from the environment (battery.py:100) and to_ctx rebuilds a Ctx every tick, so the
        # write-path reporter (_inv_write_path — "L3-submitted txs not landing, sender nonce flat")
        # is INERT unless this lands in os.environ. Keeping it a local was the whole reason that
        # detector never ran in the Python port. Only a real address is exported: a dry-run/unread
        # value must not clobber an operator-set L3_SPAMMER_ADDR with "".
        if l3_addr.startswith("0x"):
            os.environ["L3_SPAMMER_ADDR"] = l3_addr
        self.p.run_ok(["cast", "send", l3_addr, "--value", "100000000000000",
                       "--rpc-url", self.rpc, "--private-key", self.chain.owner_key(0)],
                      note="fund-l3-spammer")
        self.spammers.start(l3_key, self.chain.owner_addr(0),
                            topology.host_rpc_url(topology.DOWNSTREAM), note="l3")


def _hex(v) -> int:
    try:
        s = str(v).strip()
        return int(s, 16) if s.startswith(("0x", "0X")) else int(s or 0)
    except Exception:
        return 0


def _pp_addr_dry(i) -> str:
    return f"0x{'%040x' % (0xA0 + int(i))}"
