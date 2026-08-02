"""production_path.py — the THIRD devnet, and the bring-up the five production-path cases share.

`stack/profiles.py` describes two devnets: the sim's GENERATED N-validator pair and the smoke
cases' STATIC four-validator pair. There is a third, and it was invisible from Python until now:
the **production-path** stack — six validators plus a full-node on `docker-compose.production-path
.yml`, with a BARE genesis (no staking contracts baked in) whose staking cluster is deployed at
RUNTIME by `forge script DeployStaking` and whose addresses are DISCOVERED from the deploy
manifest rather than predicted from a create-nonce.

Five bash cases run on it — `case-production-path`, `case-vrf-rotation`, `case-vrf-dkg-halt`,
`case-vrf-dkg-durability`, `case-byzantine-vrf` — and each `export COMPOSE_FILE=…` before sourcing
`lib.sh`, so every bare `docker compose` in the library targets that project. This module holds
the profile that expresses that, and the 14-phase bring-up two of those cases call directly.

═══ WHY THIS IS A PROFILE AND NOT A FLAG ══════════════════════════════════════════════════

It differs from the static profile in every one of the three questions a profile answers, and
from the generated profile in two:

  * **which compose files** — its own pair, and it drives them through the AMBIENT `COMPOSE_FILE`
    (like the generated profile, unlike the static one, which names `-f` flags explicitly). That
    is a bash fact: `case-vrf-rotation.sh:77` exports the base name before sourcing anything, and
    `pp_bring_up_rotation` RE-EXPORTS the colon-joined pair mid-function at the cold restart.
  * **whether they must be generated** — no; they are checked in.
  * **which services are the committee** — `PP_VALS` is SIX validators (lib.sh:605), while the
    COMMITTEE that gets consensus keys is `PP_COMMITTEE_SIZE`, default FIVE (lib.sh:608). The two
    numbers are different on purpose and that difference IS the rotation these cases test:
    validator-5 boots as a follower and joins the committee later, so a profile that collapsed
    them would delete the case.

═══ pp_bring_up_rotation — WHAT SURVIVES THE PORT ═════════════════════════════════════════

`RotationBringUp` is lib.sh:1252-1357, phase for phase. Six things in it are not incidental:

1. **`exit 1`, not `return 1`** (§2.4 item 12). Every failure branch in the bash TERMINATES THE
   PROCESS, which fires the caller's `trap cleanup EXIT` — and that trap is three things, not one:
   `pp_spammer_stop; rm -f "$MANIFEST"; tear_down` (case-vrf-rotation.sh:88). A Python `raise` only
   unwinds, so the teardown has to be re-wired deliberately. It is re-wired in the CASE WRAPPER
   (`cases/smoke/prod.py::run`), not here: this class raises `RotationBringUpError` and the wrapper
   owns the `finally`. Doing it here would give the bring-up a teardown the assertion phase does
   not have, which is the half bash's trap actually covered.
2. **the mid-function `COMPOSE_FILE` re-export** (lib.sh:1348). Everything up to the cold restart
   runs against the BARE compose; from the restart on, against the pair. It goes to `os.environ`
   AND the Runner's env for the same reason `bringup._set_compose` does — the bare `docker compose
   exec` READERS in `core/rpc.py` inherit only `os.environ`, and a Runner env seeded earlier would
   otherwise shadow it.
3. **ACT is computed against a LITERAL 64, not against the epoch interval** (lib.sh:1320:
   `ACT=$(( ((HEAD / 64) + 2) * 64 ))`). `EPOCH_LEN` is not read until the last line of the
   function, so the bash could not have used it even if it wanted to. Substituting the real
   interval here would move the activation block on any stack whose interval is not 64 — i.e.
   change which block the whole case anchors on — so the literal is preserved and named.
4. **the pre-written `staking-reader.json` is ASSERTED, not regenerated.** The sim REGENERATES it
   from the manifest (`bringup.py` step 7, derive-don't-predict); the production path deliberately
   does the opposite — it checks that the file genesis baked from `--staking-reader-create-nonces`
   matches what DeployStaking actually deployed. That assert IS the create-nonce drift detector,
   and replacing it with a regen would delete the only thing that notices when the nonces move.
5. **`setBlsVerifier` MUST precede `setConsensusKeys`.** The keys are PoP-verified on the way in;
   with no verifier set the first `setConsensusKeys` reverts. The bash says so in its banner.
6. **the spammer starts BEFORE the deploys** and keeps user tx pressure on the mempool across
   every transition the case then measures. Its key is mnemonic index 6 — an account that issues
   no other transaction, or its nonce races the deploy txs (`core/spammer.py`).
"""

from __future__ import annotations

import json
import os

from .profiles import StackProfile
from ..core import converge, nodes, topology
from ..core.spammer import SpammerPool
from ..chain.writes import Chain

# ── the production-path compose pair ──────────────────────────────────────────────────
PRODUCTION_BASE = "docker-compose.production-path.yml"
PRODUCTION_DPOS_OVERLAY = "docker-compose.production-path.dpos.yml"

#: `lib.sh:605` — `PP_VALS=(validator-0 … validator-5)`. SIX containers boot; five of them are
#: the initial committee. Fixed by the checked-in compose file, not by a knob.
PP_VAL_COUNT = 6
#: `lib.sh:608` — `PP_COMMITTEE_SIZE="${PP_COMMITTEE_SIZE:-5}"`. Env-overridable: the n=6
#: byzantine repro sets 6. Default 5 keeps every existing case byte-identical.
DEFAULT_COMMITTEE_SIZE = 5
#: `lib.sh:650` — `--peers "${PP_PEERS:-6}"` for the consensus-keys one-off.
DEFAULT_PP_PEERS = 6

# ── the bring-up's fixed budgets (lib.sh:1258-1352) ───────────────────────────────────
#: phase-A converge. Longer than the static stack's 90 s: six containers stagger their boot.
PHASE_A_CONVERGE_S = 240
#: the activation-block finalize wait (`wait_finalized_ge "$ACT" 400`).
ACTIVATION_WAIT_S = 400
#: the two post-activation converges (at the activation block, and past the anchor).
POST_CONVERGE_S = 180
#: `docker compose logs --tail=N` depths at the five fail-loud sites. Kept distinct because they
#: are: the post-swap failure needs more history than a phase-A one and flattening them would
#: quietly shorten a diagnostic.
LOG_TAIL_CONVERGE = 120
LOG_TAIL_NODE = 80
LOG_TAIL_DPOS = 200

#: `ACT=$(( ((HEAD / 64) + 2) * 64 ))` (lib.sh:1320) — see the module header, item 3. This is NOT
#: the epoch interval; it is a literal that predates `EPOCH_LEN` being read at all.
ACTIVATION_GRID = 64
#: `cast send "$SPAMMER_ADDR" --value 1000000000000000` (lib.sh:1267) — 0.001 ETH of gas budget.
SPAMMER_FUNDING_WEI = "1000000000000000"
#: `pp_token_transfer "$TOKEN" "$(pp_owner_addr 5)" "10000000000000000000"` (lib.sh:1282) — the
#: joiner's BLEND, sent BEFORE the staking cluster exists so v5 can self-delegate into the
#: committee later without a second funding round.
JOINER_BLEND_WEI = "10000000000000000000"
#: `--mnemonic-index 6` (lib.sh:1265) — the spammer's dedicated account.
SPAMMER_MNEMONIC_INDEX = "6"
DEFAULT_MNEMONIC = "test test test test test test test test test test test junk"

#: The three keys the pre-written `/runtime/staking-reader.json` must agree with the manifest on
#: (lib.sh:1328-1330), as `(json key in the runtime file, manifest key)`. `governance` is
#: deliberately absent: the reader does not carry it, and adding it would fail every run.
STAKING_READER_PAIRS = (
    ("staking_address", "staking"),
    ("chain_config_address", "chain_config"),
    ("liveness_slashing_address", "liveness_slashing"),
)

#: The two `forge create` targets, in bash's order. The verifier is deployed SEPARATELY here and
#: wired by governance; the sim's DeployStaking self-deploys one instead.
TOKEN_CONTRACT = "contracts/staking/mocks/MockBlendToken.sol:MockBlendToken"
VERIFIER_CONTRACT = "contracts/libraries/BLS12381Verifier.sol:BLS12381Verifier"


class RotationBringUpError(Exception):
    """A bring-up phase failed. Every branch that raises this is an `exit 1` in the bash — see
    the module header, item 1: the CALLER's `finally` is what replaces the EXIT trap."""

    def __init__(self, label: str, message: str):
        self.label = label
        self.message = message
        super().__init__(f"FAIL ({label}): {message}")


class ProductionPathProfile(StackProfile):
    """The six-validator runtime-deploy devnet the five production-path cases run on."""

    name = "production-path"

    def __init__(self, committee_size=None, val_count: int = PP_VAL_COUNT, extra_overlays=None):
        self.val_count = int(val_count)
        self.committee_size = int(
            committee_size if committee_size is not None
            else os.environ.get("PP_COMMITTEE_SIZE", DEFAULT_COMMITTEE_SIZE))
        #: The case's own compose overlay, appended to the DPoS pair at the COLD RESTART and
        #: nowhere else. One case uses it: `case-byzantine-vrf.sh:288` re-exports a THREE-file
        #: `COMPOSE_FILE` there so `docker-compose.byzantine-vrf.yml` puts
        #: `FLUENT_DPOS_BYZANTINE=forge-beacon-pk` on one validator. It is a profile field for the
        #: reason `StaticProfile.extra_overlays` is: the file list must have exactly one home, or a
        #: case starts assembling `COMPOSE_FILE` strings of its own.
        self.extra_overlays = tuple(extra_overlays or ())

    @classmethod
    def from_env(cls, extra_overlays=None) -> "ProductionPathProfile":
        return cls(extra_overlays=extra_overlays)

    def prepare(self, runner) -> None:
        """No-op: both compose files are checked into the repo, like the static profile's."""
        return

    def compose_files(self, phase: str = "dpos"):
        """The overlays ride on the DPoS phase ONLY. Phase A is the BARE genesis chain and the
        byzantine overlay's service does not exist in a meaningful form there — bash exports the
        bare name at `:85` and only widens it at the restart (`:288`), so a phase-A `up --build`
        carrying the overlay would build a stack the bash never builds."""
        if phase == "base":
            return (PRODUCTION_BASE,)
        return (PRODUCTION_BASE, PRODUCTION_DPOS_OVERLAY, *self.extra_overlays)

    def compose_file_env(self, phase: str = "dpos"):
        """The AMBIENT `COMPOSE_FILE` value — this profile drives compose through the environment,
        as the generated one does. `case-vrf-rotation.sh:77` exports the base before sourcing
        `lib.sh`; `pp_bring_up_rotation` re-exports the pair at the cold restart."""
        return ":".join(self.compose_files(phase))

    def committee(self):
        """`PP_VALS` — the SIX containers the cold restart recreates, not the five that get
        consensus keys. `keyed_committee` is the other number; see the module header."""
        return tuple(topology.validator(i) for i in range(self.val_count))

    def keyed_committee(self):
        """`v0..v(PP_COMMITTEE_SIZE-1)` — the validators `setConsensusKeys` is sent for, i.e. the
        INITIAL committee. validator-5 is deliberately not in it."""
        return tuple(topology.validator(i) for i in range(self.committee_size))

    def read_nodes(self):
        """`_read_pp_nodes` (lib.sh:654-662): validator-0 over the host RPC, validator-1..5 by
        `docker compose exec`, the full-node over host 18545 — as `(label, "height|hash")` pairs
        in bash's order.

        The host-vs-exec choice comes from `topology.HOST_RPC_PORTS`, the same map the compose
        files publish from, so a reader cannot aim at a port the compose file does not expose."""
        out = [_read_host(topology.PINNED_RPC_HOST)]
        for i in range(1, self.val_count):
            svc = topology.validator(i)
            out.append(_read_host(svc) if topology.has_host_rpc(svc)
                       else (svc, nodes.check_node(svc)))
        out.append(_read_host(topology.FULL_NODE))
        return out


def _read_host(service: str):
    port = topology.host_rpc_port(service)
    return (f"{service}@{port}", nodes.check_external(port))


def default_manifest(contracts_dir: str) -> str:
    """`MANIFEST="$(cd "$SOLIDITY_CONTRACTS_DIR" && pwd)/deployments/runtime-deployment.json"` —
    defined identically and independently by all five cases plus `case-soak.sh`, which is the
    duplication P7 chunk 5b's "cheap win" note is about. ONE definition, here.

    ABSOLUTE, and that is not cosmetic: `DeployStaking`'s `vm.writeJson` runs under
    solidity-contracts' foundry.toml with root=cwd=$SOLIDITY_CONTRACTS_DIR, whose `fs_permissions`
    allow only `./deployments`. A RELATIVE path escapes the allowed dir and is denied; an absolute
    one inside `<contracts>/deployments` both satisfies it and makes the forge writer (cwd =
    contracts dir) and the Python reader (cwd = smoke dir) resolve to ONE file."""
    return os.path.abspath(os.path.join(contracts_dir, "deployments", "runtime-deployment.json"))


class RotationBringUp:
    """`pp_bring_up_rotation` (lib.sh:1252-1357) — the 14-phase runtime-forge bring-up.

    Leaves the stack UP with the DPoS chain live past the anchor, and exposes the eleven facts the
    bash exported for the case to read afterwards: `deployer_key`, `deployer_addr`, `token`,
    `verifier`, `staking_rt`, `chain_config_rt`, `gov_addr`, `liveness_rt`, `act`, `anchor`,
    `epoch_len` — plus `epoch_first_block()`, which bash defines at FILE scope (lib.sh:1362)
    precisely so it outlives the helper's `local`s.

    The spammer it starts is the caller's to stop: `spammers` is exposed rather than reaped here,
    because bash's `pp_spammer_stop` runs from the case's EXIT trap and covers the assertion phase
    too, which this object knows nothing about.
    """

    def __init__(self, runner, label: str, profile: ProductionPathProfile = None,
                 contracts_dir: str = None, manifest: str = None, rpc: str = None,
                 chain_id=None, spammers: SpammerPool = None, post_manifest=None):
        # `PP_ROT_LABEL` (lib.sh:1253) is `${PP_ROT_LABEL:?…}` — REQUIRED, and bash aborts on an
        # unset one rather than defaulting. It is the label in every FAIL line, so a default would
        # make five cases' failures indistinguishable.
        if not label:
            raise ValueError("RotationBringUp needs a label (bash PP_ROT_LABEL) — it names every "
                             "FAIL line, and five cases share this bring-up")
        self.p = runner
        self.label = label
        self.profile = profile if profile is not None else ProductionPathProfile.from_env()
        self.rpc = rpc or os.environ.get("RPC", topology.DEFAULT_RPC_URL)
        self.chain_id = str(chain_id or os.environ.get("CHAIN_ID", topology.CHAIN_ID))
        self.contracts_dir = contracts_dir or os.environ.get("SOLIDITY_CONTRACTS_DIR",
                                                             "../../../solidity-contracts")
        self.manifest = manifest or os.environ.get("MANIFEST") or default_manifest(
            self.contracts_dir)
        self.spammers = spammers if spammers is not None else SpammerPool(dry=runner.dry)
        #: `fn(bringup)`, invoked ONCE, after the deploy manifest is read and the `Chain` exists,
        #: and BEFORE the first governance write. One case needs it: `case-byzantine-vrf.sh:235-250`
        #: sends five further DEPLOYER-funded transfers (the byzantine owner's BLEND, the toggle
        #: delegator's gas and BLEND, and BLEND for the three floor-bumped owners) and the comment
        #: at `:190-197` is emphatic about WHY they sit exactly here: sending them EARLIER would
        #: advance the deployer nonce and shift DeployStaking's CREATE addresses off the prediction
        #: baked into `staking-reader.json`, and the node would then read ChainConfig at the wrong
        #: address and the cold start would fail. The hook is the post-DeployStaking half of that
        #: contract, expressed as a seam rather than as a second bring-up.
        self.post_manifest = post_manifest
        # the eleven exported facts (lib.sh:1247-1249), empty until run().
        self.deployer_key = ""
        self.deployer_addr = ""
        self.spammer_addr = ""
        self.token = ""
        self.verifier = ""
        self.staking_rt = ""
        self.chain_config_rt = ""
        self.gov_addr = ""
        self.liveness_rt = ""
        self.act = 0
        self.anchor = "0x0"
        self.epoch_len = 0
        self.chain = None

    # -- seams ---------------------------------------------------------------------
    @property
    def dry(self) -> bool:
        return bool(self.p.dry)

    def _fail(self, message: str, tail: int = None, *services):
        """A bash `{ echo "FAIL ($L): …"; docker compose logs …; exit 1; }` branch.

        The log dump goes out FIRST, as bash's does — a diagnostic printed after the exception has
        already unwound to the wrapper is a diagnostic in the wrong place in the output."""
        if tail is not None:
            self.dump_logs(tail, *services)
        raise RotationBringUpError(self.label, message)

    def dump_logs(self, tail: int, *services) -> None:
        if self.dry:
            return
        r = self.p.run_capture(["docker", "compose", "logs", f"--tail={tail}", *services],
                               timeout=120, note="fail-logs")
        if r.stdout or r.stderr:
            print(r.stdout or r.stderr, flush=True)

    def _set_compose(self, value: str) -> None:
        """bash `export COMPOSE_FILE=…` — process-global, so BOTH the ambient environment (which
        the bare `docker compose exec` readers inherit) and the Runner's own env (which is merged
        AFTER `os.environ` and would otherwise shadow it). `bringup._set_compose`'s note applies
        verbatim; the two must not drift."""
        os.environ["COMPOSE_FILE"] = value
        self.p.env["COMPOSE_FILE"] = value
        self.p.step("export", f"COMPOSE_FILE={value}")

    def wait_converge(self, timeout=PHASE_A_CONVERGE_S, floor: str = ""):
        """`pp_wait_converge [timeout] [floor]` (lib.sh:666) — all six validators plus the
        full-node aligned at finalized > floor. Returns the aligned `"height|hash"`.

        Unlike `StaticStack._wait_aligned` this does NOT tear the stack down on expiry: the caller
        owns a three-part cleanup (spammer, manifest, compose) and tearing down half of it here
        would leave the other half leaked. It raises; `cases/smoke/prod.py::run` reaps."""
        if self.dry:
            # Recorded as a marker, not skipped silently: the three converge gates are WHERE this
            # bring-up can hang, and a transcript that jumps from the `up` to the next write says
            # nothing about the wait between them. The argv belongs to the transport layer
            # (`core/nodes.py`), so it is a marker rather than a fabricated `docker compose exec`.
            self.p.step("poll", f"pp_wait_converge(<= {timeout}s, floor={floor or 'none'})")
            return True
        reading, last = converge.wait_aligned(timeout, floor, self.profile.read_nodes)
        if reading is not None:
            return reading
        return converge.divergence_detail(last), last

    def _converge_or_fail(self, timeout, floor, what: str, tail: int):
        got = self.wait_converge(timeout, floor)
        if self.dry or not isinstance(got, tuple):
            return got
        detail, _ = got
        floor_msg = f" past anchor {floor}" if converge.hex_floor(floor) is not None else ""
        self._fail(f"{what}{floor_msg} (last readings: {detail})", tail)

    def _head_dec(self) -> int:
        if self.dry:
            self.p.step("read", f"check_external({topology.HOST_RPC_PORT}) -> head")
            return 0
        return nodes.hex_to_dec(nodes.check_external(topology.HOST_RPC_PORT).split("|", 1)[0])

    def _head_hex(self) -> str:
        if self.dry:
            self.p.step("read", f"check_external({topology.HOST_RPC_PORT}) -> anchor")
            return "0x0"
        return nodes.check_external(topology.HOST_RPC_PORT).split("|", 1)[0]

    def forge(self, argv, note: str, env_overlay=None, timeout=600):
        """`forge_l2 forge …` — the `( cd "$SOLIDITY_CONTRACTS_DIR" && "$@" )` wrapper the five
        cases each define identically (chunk 5b's "cheap win"). It is a `cwd=` on the Runner, so
        the RECORDED argv stays the bare `forge …` line (the oracle) with the cwd shown around
        it."""
        return self.p.run_capture(argv, cwd=self.contracts_dir, note=note,
                                  env_overlay=env_overlay, timeout=timeout)

    # -- the choreography ----------------------------------------------------------
    def run(self):
        """Phases A..cold-restart, in bash's order. Raises `RotationBringUpError` on any failure
        branch bash spells `exit 1`."""
        p = self.profile
        # Phase A runs against the BARE compose file. The bash cases export this before sourcing
        # lib.sh; setting it here makes the bring-up self-contained and keeps the value in the
        # transcript where the `up` that consumed it can be seen next to it.
        self._set_compose(p.compose_file_env("base"))

        print("== phase A: bare sequencer chain ==", flush=True)
        self.p.run_checked(["docker", "compose", "up", "--build", "-d"], timeout=1800,
                           note="phaseA-up")
        self._converge_or_fail(PHASE_A_CONVERGE_S, "", "bare chain did not converge",
                               LOG_TAIL_CONVERGE)
        print("  converged plain chain", flush=True)

        chain0 = Chain(runner=self.p, RPC=self.rpc, CHAIN_ID=self.chain_id)
        self.deployer_key = chain0.owner_key(0)
        self.deployer_addr = chain0.owner_addr(0)

        # -- the tx spammer, started BEFORE the deploys (module header item 6) ------
        mnem = os.environ.get("FLUENT_DPOS_MNEMONIC", DEFAULT_MNEMONIC)
        spammer_key = self.p.run(["cast", "wallet", "private-key", "--mnemonic", mnem,
                                  "--mnemonic-index", SPAMMER_MNEMONIC_INDEX], note="spammer-key")
        self.spammer_addr = self.p.run(["cast", "wallet", "address", "--mnemonic", mnem,
                                        "--mnemonic-index", SPAMMER_MNEMONIC_INDEX],
                                       note="spammer-addr")
        if not self.p.run_capture(["cast", "send", self.spammer_addr, "--value",
                                   SPAMMER_FUNDING_WEI, "--rpc-url", self.rpc,
                                   "--private-key", self.deployer_key],
                                  note="fund-spammer").ok and not self.dry:
            self._fail("fund spammer account")
        self.spammers.start(spammer_key, self.deployer_addr, self.rpc, note="production-path")
        print(f"  tx spammer started (from {self.spammer_addr})", flush=True)

        # -- runtime deploy: token + BLS verifier ----------------------------------
        print("== runtime deploy: token + BLS verifier ==", flush=True)
        self.token = self._forge_create(TOKEN_CONTRACT, "deploy-token",
                                        "0xToKeN0000000000000000000000000000000000")
        if not self.token.startswith("0x"):
            self._fail("MockBlendToken deploy")
        self.verifier = self._forge_create(VERIFIER_CONTRACT, "deploy-verifier",
                                           "0xVeRiFiEr00000000000000000000000000000000")
        if not self.verifier.startswith("0x"):
            self._fail("BLS12381Verifier deploy")
        print(f"  token={self.token} verifier={self.verifier}", flush=True)

        # The joiner's BLEND, sent before the staking cluster exists. `checked=True` is bash's
        # bare call under `set -e` — see Chain.token_transfer.
        chain0.token_transfer(self.token, chain0.owner_addr(p.val_count - 1), JOINER_BLEND_WEI,
                              checked=True)

        # -- runtime deploy: the staking cluster -----------------------------------
        print("== runtime deploy: staking cluster (DeployStaking) ==", flush=True)
        overlay = {"NETWORK": "local-dpos-smoke/l2", "DEPLOYER": self.deployer_addr,
                   "INITIAL_OWNER": self.deployer_addr, "STAKING_TOKEN": self.token,
                   "OUTPUT_PATH": self.manifest}
        # NO INITIAL_VALIDATORS / INITIAL_STAKES / ACTIVE_VALIDATORS_LENGTH, unlike the sim's
        # deploy. The production path registers and activates its committee through governance
        # afterwards — that IS the path it exists to exercise — so seeding the deploy with a
        # committee would skip the thing under test.
        if not self.forge(["forge", "script",
                           "scripts/deploy/DeployStaking.s.sol:DeployStaking",
                           "--rpc-url", self.rpc, "--private-key", self.deployer_key,
                           "--broadcast", "--skip-simulation"],
                          note="DeployStaking", env_overlay=overlay).ok and not self.dry:
            self._fail("DeployStaking (EIP-170? see prereqs)")
        self._read_manifest()
        print(f"  staking={self.staking_rt} chainConfig={self.chain_config_rt} "
              f"gov={self.gov_addr} liveness={self.liveness_rt}", flush=True)

        chain = Chain(runner=self.p, RPC=self.rpc, STAKING_RT=self.staking_rt,
                      CHAIN_CONFIG_RT=self.chain_config_rt, GOV_ADDR=self.gov_addr,
                      LIVENESS_RT=self.liveness_rt, TOKEN=self.token, CHAIN_ID=self.chain_id,
                      PP_PEERS=os.environ.get("PP_PEERS", DEFAULT_PP_PEERS))
        self.chain = chain

        # -- the case's own post-DeployStaking writes, if it has any ---------------
        if self.post_manifest is not None:
            self.post_manifest(self)

        # -- governance: setBlsVerifier MUST precede setConsensusKeys --------------
        print("== governance: setBlsVerifier (MUST precede setConsensusKeys) ==", flush=True)
        self._gov(chain, self.chain_config_rt,
                  chain.calldata("setBlsVerifier(address)", self.verifier),
                  "setBlsVerifier", "gov setBlsVerifier")

        # -- setConsensusKeys for the INITIAL committee ---------------------------
        print(f"== setConsensusKeys for committee v0..v{p.committee_size - 1} ==", flush=True)
        for i in range(p.committee_size):
            ck = chain.consensus_keys(i)
            argv = ["cast", "send", self.staking_rt,
                    "setConsensusKeys(address,bytes,bytes,bytes32)",
                    ck.get("validatorAddress", ""), ck.get("blsPubkeyUncompressed", ""),
                    ck.get("blsPoPUncompressed", ""), ck.get("peerPubkey", ""),
                    "--rpc-url", self.rpc, "--private-key", chain.owner_key(i)]
            if not self.p.run_capture(argv, note=f"setConsensusKeys-v{i}").ok and not self.dry:
                self._fail(f"setConsensusKeys v{i}")
        print(f"  consensus keys set for {p.committee_size} validators", flush=True)

        # -- governance: the activation block -------------------------------------
        head = self._head_dec()
        self.act = ((head // ACTIVATION_GRID) + 2) * ACTIVATION_GRID
        print(f"== governance: setDposActivationBlock={self.act} (head={head}) ==", flush=True)
        self._gov(chain, self.chain_config_rt,
                  chain.calldata("setDposActivationBlock(uint64)", self.act),
                  "setDposActivationBlock", "gov setDposActivationBlock")

        # -- the create-nonce drift detector (module header item 4) ---------------
        print("== assert pre-written staking-reader.json matches the deploy manifest ==",
              flush=True)
        self._assert_staking_reader(chain)
        print("  pre-written config matches manifest", flush=True)

        # -- clean-halt at the activation block -----------------------------------
        print(f"== wait for sequencer (validator-0) to clean-halt at activation block "
              f"{self.act} ==", flush=True)
        if not self._wait_finalized_ge(self.act, ACTIVATION_WAIT_S):
            self._fail(f"sequencer did not reach activation block {self.act} "
                       f"(head={converge.finalized_dec_pinned()})",
                       LOG_TAIL_NODE, topology.PINNED_RPC_HOST)
        self._converge_or_fail(POST_CONVERGE_S, "",
                               "followers did not align at the activation block",
                               LOG_TAIL_CONVERGE)
        print(f"  all nodes aligned at {self.act}; proceeding to --dpos cold-restart", flush=True)

        # -- the cold restart into --dpos -----------------------------------------
        print("== cold-restart: all validators into unified --dpos "
              "(+ full-node into --cert-follow) ==", flush=True)
        self.anchor = self._head_hex()
        self._set_compose(p.compose_file_env("dpos"))
        if not self.p.run_capture(["docker", "compose", "up", "-d", "--force-recreate",
                                   *p.committee(), topology.FULL_NODE],
                                  timeout=600, note="dpos-cold-restart").ok and not self.dry:
            self._fail("cold-restart into --dpos (a validator exited)", LOG_TAIL_NODE,
                       topology.PINNED_RPC_HOST)
        self._converge_or_fail(POST_CONVERGE_S, self.anchor, "DPoS chain did not converge",
                               LOG_TAIL_DPOS)
        print(f"  DPoS chain live past anchor {self.anchor}", flush=True)

        # -- the epoch geometry the case does its arithmetic in -------------------
        self.epoch_len = self._read_epoch_len(chain)
        if self.epoch_len <= 0:
            self._fail("getEpochBlockInterval()=0")
        return self

    # -- sub-steps -----------------------------------------------------------------
    def _forge_create(self, contract: str, note: str, dry_addr: str) -> str:
        """`forge_l2 forge create … --json <contract> | jq -r '.deployedTo'`."""
        r = self.forge(["forge", "create", "--rpc-url", self.rpc,
                        "--private-key", self.deployer_key, "--broadcast", "--json", contract],
                       note=note)
        if self.dry:
            return dry_addr
        try:
            return json.loads(r.stdout).get("deployedTo", "") or ""
        except (ValueError, AttributeError):
            return ""

    def _read_manifest(self) -> None:
        """`jq -r '.staking' "$MANIFEST"` ×4, then assert each is `0x…` (else `cat $MANIFEST`).

        The dry stand-ins are the genesis predeploy SLOTS, which are CODELESS here
        (`core/topology.py`) — the transcript needs *some* address so the sequence walks to the
        end, and nothing may read them on a live path."""
        if self.dry:
            self.staking_rt = topology.STAKING_ADDR
            self.chain_config_rt = topology.CHAIN_CONFIG_ADDR
            self.gov_addr = topology.STAKING_POOL_ADDR
            self.liveness_rt = topology.LIVENESS_SLASHING_ADDR
            return
        try:
            with open(self.manifest) as fh:
                m = json.load(fh)
        except (OSError, ValueError) as e:
            self._fail(f"manifest unreadable at {self.manifest}: {e}")
            return
        self.staking_rt = str(m.get("staking", ""))
        self.chain_config_rt = str(m.get("chain_config", ""))
        self.gov_addr = str(m.get("governance", ""))
        self.liveness_rt = str(m.get("liveness_slashing", ""))
        for name, value in (("STAKING_RT", self.staking_rt),
                            ("CHAIN_CONFIG_RT", self.chain_config_rt),
                            ("GOV_ADDR", self.gov_addr),
                            ("LIVENESS_RT", self.liveness_rt)):
            if not value.startswith("0x"):
                print(json.dumps(m, indent=2), flush=True)   # bash `cat "$MANIFEST"`
                self._fail(f"manifest missing {name}")

    def _assert_staking_reader(self, chain: Chain) -> None:
        """`docker compose exec -T validator-0 cat /runtime/staking-reader.json`, then compare
        three lowercased addresses against the manifest (lib.sh:1327-1334).

        The FAIL message names the fix, as bash's does: a mismatch means the deployer's create
        nonces drifted and `--staking-reader-create-nonces` needs updating. Dropping that half of
        the message leaves the reader with an address mismatch and no idea what produced it."""
        raw = chain.runtime_cat("staking-reader.json")
        if self.dry:
            return
        try:
            pre = json.loads(raw) if (raw or "").strip() else {}
        except ValueError:
            pre = {}
        manifest = {"staking": self.staking_rt, "chain_config": self.chain_config_rt,
                    "liveness_slashing": self.liveness_rt}
        for reader_key, manifest_key in STAKING_READER_PAIRS:
            want = str(manifest[manifest_key]).lower()
            got = str(pre.get(reader_key, "")).lower()
            if got != want:
                self._fail(f"pre-written {reader_key}={got} != deployed {want} (deployer nonce "
                           "drift — update --staking-reader-create-nonces)")

    def _wait_finalized_ge(self, target, timeout) -> bool:
        if self.dry:
            self.p.step("poll", f"wait_finalized_ge({target}, {timeout}s)")
            return True
        return converge.wait_finalized_ge(target, timeout)

    def _read_epoch_len(self, chain: Chain) -> int:
        """`printf '%d' "$(pp_chainconfig_call 'getEpochBlockInterval()(uint32)')"`.

        Read through `Chain._pp_cfg_read_retry`, which owns the `[sci]` suffix strip (§2.4 item 9)
        and the 3× retry — bash's bare `printf '%d'` here has neither, and this is the one read in
        the bring-up whose failure mode is a silent 0 rather than a loud abort. The `> 0` assert on
        the caller's side is what bash relies on; the retry only makes a transient not reach it."""
        got = chain._pp_cfg_read_retry("getEpochBlockInterval()(uint32)")
        if self.dry:
            # The read above was still ISSUED so its argv reaches the transcript; the canned answer
            # is 0 (an empty dry stdout), which would then trip the `epoch_len > 0` assert and end
            # the transcript one phase early. The stand-in is the activation grid — the same number
            # a default devnet answers with, and it feeds `epoch_first_block` so a case's dry
            # arithmetic lands on plausible block heights instead of on `ACT + 0`.
            return ACTIVATION_GRID
        return got or 0

    def _gov(self, chain: Chain, target: str, calldata: str, desc: str, fail_label: str) -> None:
        """One `pp_gov_action … || { echo "FAIL ($L): …"; exit 1; }` branch."""
        from ..chain.writes import ChainError
        try:
            chain.gov_action(target, calldata, desc)
        except ChainError as e:
            self._fail(f"{fail_label}: {e.message}")

    # -- the arithmetic the case does afterwards -----------------------------------
    def epoch_first_block(self, epoch) -> int:
        """`epoch_first_block <n>` (lib.sh:1362) — `ACT + n * EPOCH_LEN`.

        bash defines this at FILE scope, not inside `pp_bring_up_rotation`, precisely so it
        survives the helper's `local` scope and can read the globals the bring-up set. Here it is
        a method on the object that owns those two facts, which is the same guarantee without the
        global."""
        return int(self.act) + int(epoch) * int(self.epoch_len)
