"""production_path.py — the THIRD devnet, and the bring-up the five production-path cases share.

`stack/profiles.py` describes two devnets: the sim's GENERATED N-validator pair and the smoke
cases' STATIC four-validator pair. There is a third, and it was invisible from Python until now:
the **production-path** stack — six validators plus a full-node on `docker-compose.production-path
.yml`, with a BARE genesis (no staking module baked in) whose staking contract is DELIVERED to the
running chain through the runtime-upgrade precompile and then initialized by ordinary
transactions. That is the whole subject of this stand: every other stand gets its staking state
from `genesis-bootstrap`, and this one has to earn it at runtime, the way a real network would.

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
   `pp_spammer_stop; rm -f "$MANIFEST"; tear_down` (case-vrf-rotation.sh:88) — now two, the
   manifest leg having gone with the manifest. A Python `raise` only unwinds, so the teardown has
   to be re-wired deliberately. It is re-wired in the CASE WRAPPER
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
4. **the pre-written `staking-reader.json` is neither asserted nor regenerated any more.** Both
   halves existed to reconcile a PREDICTED address with a DEPLOYED one, and there is no prediction
   left: `genesis-bootstrap bare` writes the fixed `GENESIS_STAKING` address into that file, and
   the delivery below installs the module at exactly that address. The create-nonce drift detector
   it used to be goes with its subject.
5. **there is no verifier contract at all, and no `setConsensusKeys`.**
   The old ordering rule ("`setBlsVerifier` MUST precede `setConsensusKeys`") described a
   two-contract split: install a verifier by setter, then feed keys in one at a time and have each
   PoP checked against it. Both halves are gone. The module verifies every genesis PoP INSIDE the
   initializer, and it does so itself — against the EIP-2537 precompiles at their fork-fixed
   addresses, with no address in its storage and no setter to move one. `initialize` therefore
   takes SIXTEEN arguments, not seventeen, and its selector changed with the argument list; keys
   are arguments 4-6.
6. **the spammer starts BEFORE the deploys** and keeps user tx pressure on the mempool across
   every transition the case then measures. Its key is mnemonic index 6 — an account that issues
   no other transaction, or its nonce races the deploy txs (`core/spammer.py`).

═══ THE SIX-STEP BRING-UP OF THE STAKING STATE ════════════════════════════════════════════

A runtime-upgrade install places CODE and nothing else: no constructor runs, storage stays
empty. So every piece of state the genesis stands receive from `bootstrap::run` has to be
issued here as ordinary transactions, and the ORDER is not free:

1. `forge create` the BLEND token. It is Solidity, a plain CREATE, and an ARGUMENT to step 4.
2. Deliver the module: `runtime-upgrade install-local --wasm … --target GENESIS_STAKING`.
   Signed with the GOVERNANCE SIGNER, which is the address `genesis-bootstrap bare` seeds into
   the upgrade precompile's owner slot — any other key fails `only_owner`.
3. `BLEND.approve(staking, …)` from the stake sponsor. `initialize` PULLS the genesis stakes
   inside its own call, so the allowance has to exist before it, not after.
4. `initialize`, the 16-argument one-shot, permissionless, sent by the deployer. It seeds the
   FIVE initial validators with their stakes and consensus keys and takes
   `dpos_activation_block = 0`.
5. `setProductionLivenessDisabled(false)` and `setBlendStipendPerEpoch`, both from the Governor
   at `GENESIS_GOVERNANCE` — the initializer writes the tier OFF deliberately and cannot flip
   its own governance-gated setter.
6. Then the pre-existing governance flow, unchanged: `setDposActivationBlock` here, and
   `registerValidator` → `activateValidator` → `delegate` for the SIXTH validator in the case.

Two details of step 4 are load-bearing and neither is a style choice.

**Five seeded validators, not zero.** The env overlay this stand used to pass omitted
`INITIAL_VALIDATORS`, which reads like "this stand bootstraps its whole committee through
governance" — but `DeployStaking.s.sol` fell back to the network JSON with `vm.envOr`, and
`l2.json` supplied five. So the stand has ALWAYS deployed seeded-with-five and registered a
SIXTH through governance, which is exactly what the 6-validator / 5-seat topology is for.
Seeding preserves that, and it is also what keeps the registry at or above the contract's
`MIN_COMMITTEE_LENGTH = 4`.

**`dpos_activation_block = 0`, the unscheduled sentinel.** A real value here would engage the
node's pre-execution section on the very next block, and the ahead-commit driver runs
PRE-activation by design — so `commit_epoch` would system-call `commitEpochCommittee()` against
a registry that is not populated yet, revert `ERR_COMMITTEE_TOO_SMALL` into the fail-loud arm,
and halt block production. Seeding five removes the shortfall; passing `0` removes the window
in which the shortfall could be observed at all. BOTH, not either — step 6 sets the real
activation block once the registry is complete.
"""

from __future__ import annotations

import json
import os

from .profiles import StackProfile
from ..core import converge, nodes, topology
from ..core.spammer import SpammerPool
from ..chain.writes import Chain, ChainError

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

#: The one remaining `forge create` target. It is an ARGUMENT to `initialize` (argument 8) rather
#: than something wired up afterwards by a setter.
#:
#: `BLS12381Verifier` used to be deployed here as well and passed as argument 15. The staking
#: module verifies BLS signatures itself now, so there is nothing to deploy and nothing to pass.
TOKEN_CONTRACT = "contracts/staking/mocks/MockBlendToken.sol:MockBlendToken"

#: The runtime-upgrade delivery. `runtime-upgrade` is a HOST binary, like `forge` and `cast` —
#: the smoke image ships only `fluent` and `genesis-bootstrap`. Unlike those two it is built
#: FROM THIS REPO and is not something a developer has already installed, so resolution prefers
#: the workspace build and falls back to PATH. `RUNTIME_UPGRADE_BIN` overrides both.
#:
#: The fallback is deliberately a bare name and not an error: an operator who has installed it
#: (or a CI image that bakes it in) should not be forced to set an env var. But the common case
#: is a plain `cargo build --release -p fluentbase-runtime-upgrade`, and requiring a PATH entry
#: for that turned all five production-path cases into a spawn error rather than a clear
#: "binary missing" message.
def _resolve_runtime_upgrade_bin() -> str:
    override = os.environ.get("RUNTIME_UPGRADE_BIN")
    if override:
        return override
    repo_root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "..", ".."))
    for profile in ("release", "debug"):
        built = os.path.join(repo_root, "target", profile, "runtime-upgrade")
        if os.path.isfile(built) and os.access(built, os.X_OK):
            return built
    return "runtime-upgrade"


RUNTIME_UPGRADE_BIN = _resolve_runtime_upgrade_bin()
#: The `.wasm` (NOT the `.rwasm`): the precompile compiles the module ON-CHAIN with
#: `compile_rwasm_maybe_system(&target_address, …)`, which is the same function
#: `crates/genesis/build.rs` uses — so the delivered bytes and the genesis-installed bytes agree
#: by construction. The binary enforces this too: it checks the `\0asm` magic and rejects an
#: already-compiled `.rwasm` before it builds a transaction. Path is relative to the smoke dir,
#: which is every entry point's cwd.
STAKING_WASM = os.environ.get(
    "STAKING_WASM", "contracts/fluentbase_contracts_staking.wasm")
#: The line `runtime-upgrade install-local` prints on success, carrying `result` (`upgraded` on a
#: first install, `up_to_date` when the module is already there) and the verified on-chain code
#: hash. Parsed rather than trusted to the exit code, because `up_to_date` is a legitimate
#: success on a repeated bring-up and looks identical from outside.
UPGRADE_RESULT_PREFIX = "RESULT_MANIFEST_JSON="
UPGRADE_OK_RESULTS = ("upgraded", "up_to_date")

# ── the `initialize` arguments this stand seeds ────────────────────────────────────────
#: `initialize(address,address[],uint256[],bytes[],bytes[],bytes32[],uint16,address,uint32,
#: uint32,uint32,uint256,uint256,uint64,uint256,address)` — pinned against the contract's own
#: `SIG_INITIALIZE` (`contracts/staking/src/consts.rs`). Sixteen arguments in one selector, so a
#: re-ordering is a silent wrong-argument call rather than a revert; the keyword-built list below
#: is what keeps the positions honest.
#:
#: This string IS the selector `cast send` computes. When the contract's argument list moves,
#: nothing here fails to build and nothing type-checks — the call simply goes out under a
#: selector the dispatcher does not know and the bring-up dies at the send. The `address
#: blsVerifier` that used to sit at position 15 came out with the external verifier, taking the
#: selector from `0xdfa8efb0` to `0xfecaf0f1`.
INITIALIZE_SIG = ("initialize(address,address[],uint256[],bytes[],bytes[],bytes32[],uint16,"
                  "address,uint32,uint32,uint32,uint256,uint256,uint64,uint256,address)")
#: 1 BLEND. The baseline per-validator genesis stake and `minValidatorStakeAmount`/
#: `minStakingAmount` — the contract rejects a zero minimum, and a stake must be a multiple of
#: `BALANCE_COMPACT_PRECISION` (1e10), which 1e18 is.
INIT_STAKE_WEI = 10 ** 18
#: validator-0 is seeded 5x, exactly as the retired `DeployStaking` config did
#: (`solidity-contracts/scripts/config/local-dpos-smoke/l2.json`: initialStakes
#: `[5e18, 1e18, 1e18, 1e18, 1e18]`). NOT decoration: v0 is the HOST-RPC node every reading in
#: `case-production-path` goes through, and the case's whole subject is a joiner displacing an
#: incumbent at a committee boundary. With six equal stakes the top-5 selection is a tie and the
#: contract's tie-break decides who drops — it dropped v0 once, which leaves the harness
#: measuring the chain through a demoted node, and `evaluate_displaced` fails that outright
#: rather than adapting. The 5x makes "v0 is never the lowest" a property instead of a
#: coincidence.
INIT_STAKE_V0_WEI = 5 * INIT_STAKE_WEI
#: `undelegatePeriod` / `minUndelegateBlocks` / `commissionRate`, mirroring what
#: `genesis-bootstrap` passes on the genesis stands so the two devnets share one economics.
INIT_UNDELEGATE_PERIOD = 16
INIT_MIN_UNDELEGATE_BLOCKS = 0
INIT_COMMISSION_RATE = 0
#: The BLEND the stipend is drawn from (`transferFrom(blendReserve, claimant, …)` — since 2026-09-07
#: the stipend goes straight to the claimant and never lands on the staking contract), approved in
#: the same allowance as the genesis stakes because both draw on the deployer.
STIPEND_BUDGET_WEI = 1_000_000 * 10 ** 18
#: `setBlendStipendPerEpoch` — 10 BLEND, the devnet value `genesis-bootstrap` uses. 0 is the
#: kill-switch, so this must be an explicit non-zero or the stipend leg never runs.
STIPEND_PER_EPOCH_WEI = 10 * 10 ** 18
#: `dposActivationBlock` AT `initialize`. The unscheduled sentinel — see the module header.
INIT_DPOS_ACTIVATION_BLOCK = 0


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
        #: A case's own compose overlay, appended to the DPoS pair at the COLD RESTART and
        #: nowhere else. NO shipping case passes one today — the last that did,
        #: `smoke-byzantine-vrf`, is retired — so the field is currently exercised only by
        #: `tests/test_prod_cases.py`. It is kept, and kept as a PROFILE field, for the reason
        #: `StaticProfile.extra_overlays` (four live users) is one: the compose file list must
        #: have exactly one home, or the next case that needs an overlay starts assembling
        #: `COMPOSE_FILE` strings of its own — which is the shape that silently dropped the base
        #: files and made a `--force-recreate` a no-op.
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


class RotationBringUp:
    """`pp_bring_up_rotation` (lib.sh:1252-1357) — the 14-phase runtime-forge bring-up.

    Leaves the stack UP with the DPoS chain live past the anchor, and exposes the ten facts the
    bash exported for the case to read afterwards: `deployer_key`, `deployer_addr`, `token`,
    `staking_rt`, `chain_config_rt`, `gov_addr`, `liveness_rt`, `act`, `anchor`,
    `epoch_len` — plus `epoch_first_block()`, which bash defines at FILE scope (lib.sh:1362)
    precisely so it outlives the helper's `local`s.

    The spammer it starts is the caller's to stop: `spammers` is exposed rather than reaped here,
    because bash's `pp_spammer_stop` runs from the case's EXIT trap and covers the assertion phase
    too, which this object knows nothing about.
    """

    def __init__(self, runner, label: str, profile: ProductionPathProfile = None,
                 contracts_dir: str = None, rpc: str = None,
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
        self.spammers = spammers if spammers is not None else SpammerPool(dry=runner.dry)
        #: `fn(bringup)`, invoked ONCE, after the staking module exists and the `Chain` does, and
        #: BEFORE the first governance write — the slot for a case that must fund or stake before
        #: governance runs. NO shipping case wires one today (the last, `smoke-byzantine-vrf`, is
        #: retired); the POSITION is what the seam is for and it is pinned by
        #: `tests/test_prod_cases.py`. Why exactly here: a hook's writes move tokens through a
        #: `Chain`, so the module must already be installed, and they must precede the first
        #: governance action or the deployer's tx sequence stops matching.
        self.post_manifest = post_manifest
        # Facts the case reads afterwards. The three contract addresses are CONSTANTS, not deploy
        # outcomes — one module at a fixed address, with `chain_config_rt` / `liveness_rt` as
        # aliases for the read surfaces that used to be separate predeploys. `token` is still
        # discovered, because it is still `forge create`d here.
        self.deployer_key = ""
        self.deployer_addr = ""
        self.spammer_addr = ""
        self.token = ""
        self.staking_rt = topology.GENESIS_STAKING
        self.chain_config_rt = topology.GENESIS_STAKING
        self.gov_addr = topology.GENESIS_GOVERNANCE
        self.liveness_rt = topology.GENESIS_STAKING
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
        owns a multi-part cleanup (spammer, compose, teardown) and tearing down half of it here
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

        # -- step 1: forge-create the token ----------------------------------------
        # Solidity, and a plain CREATE, so `forge create` still works and still discovers its
        # address. It is ARGUMENT 8 to `initialize`, not something wired up by a setter
        # afterwards. The BLS verifier used to be deployed here too; the module carries it now.
        print("== runtime deploy: token ==", flush=True)
        self.token = self._forge_create(TOKEN_CONTRACT, "deploy-token",
                                        "0xToKeN0000000000000000000000000000000000")
        if not self.token.startswith("0x"):
            self._fail("MockBlendToken deploy")
        print(f"  token={self.token}", flush=True)

        # The joiner's BLEND, sent before the staking cluster exists. `checked=True` is bash's
        # bare call under `set -e` — see Chain.token_transfer.
        chain0.token_transfer(self.token, chain0.owner_addr(p.val_count - 1), JOINER_BLEND_WEI,
                              checked=True)

        # -- step 2: deliver the staking module to the running chain ---------------
        print(f"== runtime-upgrade: install the staking module at {self.staking_rt} ==",
              flush=True)
        self._deliver_staking_module()

        chain = Chain(runner=self.p, RPC=self.rpc, STAKING_RT=self.staking_rt,
                      CHAIN_CONFIG_RT=self.chain_config_rt, GOV_ADDR=self.gov_addr,
                      LIVENESS_RT=self.liveness_rt, TOKEN=self.token, CHAIN_ID=self.chain_id,
                      PP_PEERS=os.environ.get("PP_PEERS", DEFAULT_PP_PEERS))
        self.chain = chain

        # -- the case's own post-install writes, if it has any ---------------------
        if self.post_manifest is not None:
            self.post_manifest(self)

        # -- steps 3+4: approve the genesis stakes, then initialize -----------------
        print(f"== initialize the staking module (seeding v0..v{p.committee_size - 1}) ==",
              flush=True)
        self._initialize_staking(chain)

        # -- step 5: the two governance-only configuration flips -------------------
        # `apply_initial_config` writes `productionLivenessDisabled = true` DELIBERATELY — the
        # tier ships off and an unwritten slot would ship it on — and the setter is governance-
        # gated, so the initializer cannot flip its own default. Both proposals target the
        # STAKING address explicitly: a proposal into an address with no code executes, emits
        # `ProposalExecuted` and returns 0x1 while doing nothing (OZ's `Address.verifyCallResult`
        # never checks `target.code.length`).
        print("== governance: enable the production-liveness tier + the BLEND stipend ==",
              flush=True)
        self._gov(chain, self.staking_rt,
                  chain.calldata("setProductionLivenessDisabled(bool)", "false"),
                  "setProductionLivenessDisabled", "gov setProductionLivenessDisabled")
        self._gov(chain, self.staking_rt,
                  chain.calldata("setBlendStipendPerEpoch(uint256)", STIPEND_PER_EPOCH_WEI),
                  "setBlendStipendPerEpoch", "gov setBlendStipendPerEpoch")

        # -- step 6: governance sets the REAL activation block ---------------------
        # `initialize` passed 0 (unscheduled) so nothing could engage the node's pre-execution
        # section against a half-built registry. The registry is complete now, so schedule it.
        head = self._head_dec()
        self.act = ((head // ACTIVATION_GRID) + 2) * ACTIVATION_GRID
        print(f"== governance: setDposActivationBlock={self.act} (head={head}) ==", flush=True)
        self._gov(chain, self.staking_rt,
                  chain.calldata("setDposActivationBlock(uint64)", self.act),
                  "setDposActivationBlock", "gov setDposActivationBlock")

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

    def _governance_key(self) -> str:
        """`/runtime/keys/governance.hex`, 0x-prefixed — the ONLY key the runtime-upgrade
        precompile accepts on this stand.

        `genesis-bootstrap bare` seeds this address into the upgrade contract's `owner` storage
        slot (`0x…520010`, slot 0). Without that seed the contract falls back to
        `DEFAULT_UPDATE_GENESIS_AUTH`, a key nobody here holds — which is why the slot is seeded
        rather than the account funded. Any other signer fails `only_owner`."""
        raw = self.p.run(["docker", "compose", "exec", "-T", topology.RUNTIME_MOUNT_HOST,
                          "cat", "/runtime/keys/governance.hex"], timeout=15,
                         note="governance-key")
        raw = (raw or "").strip()
        if raw:
            return raw if raw.startswith("0x") else f"0x{raw}"
        if self.dry:
            return "0x" + "11" * 32
        self._fail("/runtime/keys/governance.hex is empty — the runtime-upgrade owner key is "
                   "the only signer the upgrade precompile accepts on this stand")

    def _deliver_staking_module(self) -> None:
        """Step 2: `runtime-upgrade install-local --wasm <module> --target <staking>`.

        A HOST binary, invoked like `forge` and `cast`. There is deliberately no Python signing
        path: the payload is ~418 KB, far past what fits in an `execve` argument, so no
        `cast send` shape can carry it — this binary builds the calldata in-process, which is
        exactly why it is the tool for this.

        The key goes in the ENVIRONMENT, not the argv, and that is a wedge guard rather than a
        secrets nicety: the binary resolves `--private-key`, then `$PRIVATE_KEY`, then a HIDDEN
        TERMINAL PROMPT. In an unattended harness run that third fallback is not an error, it is
        a bring-up that hangs forever.

        The verdict comes from the `RESULT_MANIFEST_JSON=` line, not from the exit code alone:
        `up_to_date` is a legitimate success on a repeated bring-up (the module is already there
        and the on-chain code hash matches) and is indistinguishable from `upgraded` from
        outside."""
        argv = [RUNTIME_UPGRADE_BIN, "install-local",
                "--wasm", STAKING_WASM,
                "--target", self.staking_rt,
                "--rpc", self.rpc]
        # NOT `self.forge`: that wrapper cd's into the solidity-contracts checkout, and
        # `--wasm` is a path relative to the SMOKE dir (every entry point's cwd), where the
        # vendored artefact lives.
        r = self.p.run_capture(argv, note="install-staking-module",
                               env_overlay={"PRIVATE_KEY": self._governance_key()},
                               timeout=900)
        if self.dry:
            return
        if not r.ok:
            self._fail(f"runtime-upgrade install-local failed: "
                       f"{(r.merged or '').splitlines()[-1] if r.merged else ''}")
        result = _upgrade_result(r.merged)
        if result is None:
            self._fail(f"runtime-upgrade install-local printed no {UPGRADE_RESULT_PREFIX} line — "
                       "cannot confirm the module landed")
        if result.get("result") not in UPGRADE_OK_RESULTS:
            self._fail(f"runtime-upgrade install-local reported result="
                       f"{result.get('result')!r} (want one of {list(UPGRADE_OK_RESULTS)})")
        print(f"  module installed at {self.staking_rt} (result={result.get('result')})",
              flush=True)

    def _initialize_staking(self, chain: Chain) -> None:
        """Steps 3+4: `BLEND.approve(staking, …)` then the 16-argument `initialize`.

        `initialize` PULLS the genesis stakes with `transferFrom` inside its own call, so the
        allowance has to exist first; the same allowance also covers the stipend budget, because
        both draw on the deployer (`MockBlendToken` minted the whole supply to it).

        The consensus keys of all `committee_size` seeded validators ride arguments 4-6 and are
        PoP-verified inside this call, by the module itself against the EIP-2537 precompiles.
        There is no verifier argument and no setter that could supply one.

        `epochBlockInterval` is `ACTIVATION_GRID`, not a separate knob, and the coupling is
        deliberate: the activation block this stand schedules is computed on that grid
        (`ACT = ((HEAD/64)+2)*64`), and an interval that disagreed with it would put the migration
        anchor off an epoch boundary."""
        n = self.profile.committee_size
        keys = [chain.consensus_keys(i) for i in range(n)]
        addrs = [k.get("validatorAddress", "") or chain.owner_addr(i)
                 for i, k in enumerate(keys)]
        if not self.dry and not all(a.startswith("0x") for a in addrs):
            self._fail(f"no owner address for one of v0..v{n - 1}: {addrs}")
        # v0 seeded 5x so it is never the lowest and never the displaced one — see
        # INIT_STAKE_V0_WEI. Mirrors the retired DeployStaking config exactly.
        stakes = [INIT_STAKE_V0_WEI] + [INIT_STAKE_WEI] * (n - 1)
        total_stake = sum(stakes)

        self._send(chain, "BLEND.approve(staking)", self.token,
                   "approve(address,uint256)(bool)",
                   self.staking_rt, total_stake + STIPEND_BUDGET_WEI)

        # Built positionally against INITIALIZE_SIG, one argument per line with the parameter
        # name beside it. Sixteen arguments share one selector, so a transposed pair is a
        # silently wrong call, not a revert.
        args = [
            self.deployer_addr,                                    # 1  initialStakeOwner
            _arr(addrs),                                           # 2  validators
            _arr(stakes),                                          # 3  initialStakes
            _arr([k.get("blsPubkeyUncompressed", "") for k in keys]),   # 4  blsPubkeysUncompressed
            _arr([k.get("blsPoPUncompressed", "") for k in keys]),      # 5  blsPopsUncompressed
            _arr([k.get("peerPubkey", "") for k in keys]),          # 6  peerPubkeys
            INIT_COMMISSION_RATE,                                  # 7  commissionRate
            self.token,                                            # 8  stakingToken
            n,                                                     # 9  activeValidatorsLength
            ACTIVATION_GRID,                                       # 10 epochBlockInterval
            INIT_UNDELEGATE_PERIOD,                                # 11 undelegatePeriod
            INIT_STAKE_WEI,                                        # 12 minValidatorStakeAmount
            INIT_STAKE_WEI,                                        # 13 minStakingAmount
            INIT_DPOS_ACTIVATION_BLOCK,                            # 14 dposActivationBlock
            INIT_MIN_UNDELEGATE_BLOCKS,                            # 15 minUndelegateBlocks
            self.deployer_addr,                                    # 16 blendReserve
        ]
        self._send(chain, "Staking.initialize", self.staking_rt, INITIALIZE_SIG, *args)
        print(f"  initialized: {n} seeded validators, activeValidatorsLength={n}, "
              f"epochBlockInterval={ACTIVATION_GRID}, dposActivationBlock="
              f"{INIT_DPOS_ACTIVATION_BLOCK} (unscheduled)", flush=True)

    def _send(self, chain: Chain, label: str, to: str, sig: str, *args) -> None:
        """One deployer-signed, revert-checked `cast send`. `Chain.send` owns the receipt check,
        the nonce-based confirm and the transient re-send; a raised `ChainError` becomes this
        bring-up's `exit 1` so the caller's `finally` still reaps."""
        try:
            chain.send(label, to, sig, *args, key=self.deployer_key)
        except ChainError as e:
            self._fail(f"{label}: {e.message}")

    def _wait_finalized_ge(self, target, timeout) -> bool:
        if self.dry:
            self.p.step("poll", f"wait_finalized_ge({target}, {timeout}s)")
            return True
        return converge.wait_finalized_ge(target, timeout)

    def _read_epoch_len(self, chain: Chain) -> int:
        """`printf '%d' "$(pp_chainconfig_call 'getEpochBlockInterval()(uint64)')"`.

        Read through `Chain._pp_cfg_read_retry`, which owns the `[sci]` suffix strip (§2.4 item 9)
        and the 3× retry — bash's bare `printf '%d'` here has neither, and this is the one read in
        the bring-up whose failure mode is a silent 0 rather than a loud abort. The `> 0` assert on
        the caller's side is what bash relies on; the retry only makes a transient not reach it."""
        got = chain._pp_cfg_read_retry("getEpochBlockInterval()(uint64)")
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


def _arr(items) -> str:
    """`cast`'s array literal — `[a,b,c]`, or `[]` for an empty one.

    `cast send` parses an array argument from this bracketed, comma-joined form. No spaces: a
    space inside an argument is harmless to `subprocess` (there is no shell) but the recorded
    argv is a test oracle, and one spelling keeps it stable."""
    return "[" + ",".join(str(i) for i in items) + "]"


def _upgrade_result(output: str):
    """The parsed `RESULT_MANIFEST_JSON={…}` object `runtime-upgrade install-local` prints, or
    None when the line is absent or unparseable.

    Scanned from the END: the binary logs progress before it, and the last such line is the one
    describing the transaction that actually settled.

    The verdict is NESTED. The manifest is `{"entries": [ {..., "result": "upgraded"} ]}` — one
    entry per target, because the binary's release path upgrades several at once. We ask for
    exactly one target, so anything other than exactly one entry means the invocation was not
    the one we think it was, and that is a failure rather than something to pick a winner from.
    Returning the single entry keeps every caller reading `result` off a flat object."""
    for line in reversed((output or "").splitlines()):
        line = line.strip()
        if not line.startswith(UPGRADE_RESULT_PREFIX):
            continue
        try:
            obj = json.loads(line[len(UPGRADE_RESULT_PREFIX):])
        except ValueError:
            return None
        if not isinstance(obj, dict):
            return None
        entries = obj.get("entries")
        if not isinstance(entries, list) or len(entries) != 1:
            return None
        entry = entries[0]
        return entry if isinstance(entry, dict) else None
    return None
