"""writes.py — the WRITE-side chain helpers. Python port of the lib.sh pp_* write helpers +
the soak-actions.sh staking/governance write FLOWS (sim_send, register/activate, gov_action,
mint, delegate, promote, voluntary_exit).

This is the mirror of nodes.py (which ported the lib.sh READ side). Every method is a LITERAL
translation of the bash `cast`/`docker`/`forge`/`jq` command line — the bash line is the
oracle, quoted in each docstring and pinned by the unit tests (test_writes.py records the
exact argv). Commands are issued through a single `proc.Runner` seam so a test / dry-run sees
them without a live chain.

The `Chain` object carries the post-deploy runtime facts (RPC, STAKING_RT/CHAIN_CONFIG_RT/
GOV_ADDR/LIVENESS_RT/TOKEN, CHAIN_ID, PP_PEERS) — deployment facts, not policy toggles
(standards/general.md). The sim orchestrator constructs ONE Chain after the deploy and
threads it into bringup / reconcilers / hostpool / the dispatcher.

PORT-NOTE: the fail-loud `_die`/fail_bundle bash idiom becomes a raised ChainError the caller
catches to write a structured bundle (mirrors battery.py's inv_fail return). A best-effort
`|| true` bash site becomes `run_ok` (rc swallowed).
"""

from __future__ import annotations

import json
import os
import re
import sys
import time

from ..core import topology
from ..core.nodes import hex_to_dec as _hex_to_dec
from ..core.policy import DEMOTED_INVARIANTS, gov_voter_list
from ..core.proc import Runner


class ChainError(Exception):
    """Fail-loud translation of the bash `_die` / `fail_bundle` write-side aborts. Carries a
    (reason_id, message) so the orchestrator writes a bundle instead of a bare exit."""

    def __init__(self, reason_id: str, message: str):
        self.reason_id = reason_id
        self.message = message
        super().__init__(f"{reason_id}: {message}")




def fatal_or_diag(events, fail_id: str, msg: str, seen=None, key=None) -> None:
    """Central liveness-first policy for the reconciler/dispatch watchdogs: a DEMOTED bookkeeping
    watchdog records a diagnostic event and returns (the run continues); anything else raises
    ChainError as before. `events` is the caller's (kind, note) capture list.

    LATCH: a demoted watchdog re-evaluates every tick while its (unresolved) condition holds, so
    without a latch it would flood events.jsonl with the same line. When `seen` (a set) is passed,
    a given (fail_id, key) is recorded ONCE per run — enough signal for future bug-finding, no
    spam. `key` defaults to the full message; pass a stable entity key (e.g. the seat/validator)
    for messages that embed a changing counter."""
    if fail_id in DEMOTED_INVARIANTS:
        if seen is not None:
            latch = (fail_id, key if key is not None else msg)
            if latch in seen:
                return
            seen.add(latch)
        events.append(("diag", f"[{fail_id}] {msg}"))
        return
    raise ChainError(fail_id, msg)


# OZ Governor ProposalState names (state(uint256) byte) — honest failure labels (lib.sh).
_GOV_STATE_NAMES = ["Pending", "Active", "Canceled", "Defeated", "Succeeded",
                    "Queued", "Expired", "Executed"]

#: `pp_gov_wait_succeeded`'s frozen-chain escape (lib.sh:1038) — a LITERAL 120 s in bash, and
#: deliberately not `gov_confirm_frozen_s`. The two look interchangeable and are not: this one
#: bounds the VOTING-WINDOW wait, that one bounds the post-execute nonce confirm, and they are
#: separately tunable in bash only because one of them is not tunable at all. Wiring the knob to
#: both would let a caller widening the confirm budget silently widen the vote wait as well.
GOV_VOTE_FROZEN_S = 120
#: `_pp_gov_sleep` (lib.sh:1000) — the inter-poll pause of the Succeeded wait.
GOV_POLL_S = 2


def gov_state_name(byte) -> str:
    try:
        b = int(byte)
    except (TypeError, ValueError):
        return "Unknown"
    return _GOV_STATE_NAMES[b] if 0 <= b <= 7 else "Unknown"


def _first_token(s: str) -> str:
    """`cast call ...(uint256)` pretty-prints large ints as "<dec> [<sci>]"; take the first
    whitespace token before re-parsing (lib.sh house idiom, `awk '{print $1}'`)."""
    return (s or "").strip().split()[0] if (s or "").strip() else ""


class Chain:
    def __init__(self, runner: Runner = None, dry: bool = False, **facts):
        self.p = runner or Runner(dry=dry)
        self.rpc = facts.get("RPC", os.environ.get("RPC", topology.DEFAULT_RPC_URL))
        self.staking_rt = facts.get("STAKING_RT", os.environ.get("STAKING_RT", ""))
        self.chain_config_rt = facts.get("CHAIN_CONFIG_RT", os.environ.get("CHAIN_CONFIG_RT", ""))
        self.gov_addr = facts.get("GOV_ADDR", os.environ.get("GOV_ADDR", ""))
        self.liveness_rt = facts.get("LIVENESS_RT", os.environ.get("LIVENESS_RT", ""))
        self.token = facts.get("TOKEN", os.environ.get("TOKEN", ""))
        self.chain_id = facts.get("CHAIN_ID", os.environ.get("CHAIN_ID", str(topology.CHAIN_ID)))
        self.pp_peers = facts.get("PP_PEERS", os.environ.get("PP_PEERS", "6"))
        # M4 memoized epoch geometry (never default interval to 64 / act to 0 on an RPC blip).
        self._interval_cache = ""
        self._act_cache = ""
        # pp_fund_eth's two OUT-PARAMETERS (bash PP_FUND_ETH_BAL / PP_FUND_ETH_ERR). Declared
        # here, not created on first use: a caller reporting rc 1 read `fund_eth_bal` off an
        # object that had never hit the floor branch and got an AttributeError instead of "".
        self.fund_eth_bal = ""
        self.fund_eth_err = ""
        # gov confirm knobs (unused shapes kept for parity).
        self.max_mint_eth_floor = int(os.environ.get("SIM_MINT_ETH_FLOOR_WEI",
                                                      "50000000000000000"))
        self.mint_gas_floor = int(os.environ.get("SIM_MINT_GAS_FLOOR_WEI", "1000000000"))
        self.gov_confirm_blocks = int(os.environ.get("SIM_GOV_CONFIRM_BLOCKS", "20"))
        self.gov_confirm_frozen_s = int(os.environ.get("SIM_GOV_CONFIRM_FROZEN_S", "120"))
        # the chain-paced primitive for the gov/activation waits (lib.sh wait_chain_paced).
        from ..core.chainpaced import ChainPaced
        self.cp = ChainPaced(block_number=self.block_number, current_epoch=self.current_epoch,
                             head_age_s=self.head_age_s, dry=self.p.dry)

    def block_number(self):
        """_sim_block_number: `cast block-number --rpc-url <RPC>` (the blocks-domain cursor)."""
        out = self.p.run(["cast", "block-number", "--rpc-url", self.rpc], note="block-number")
        return int(out) if out.isdigit() else None

    def head_age_s(self):
        """_sim_head_age_s: wall seconds since the tip block timestamp (chain-frozen escape)."""
        ts = self.p.run(["cast", "block", "latest", "--field", "timestamp", "--rpc-url", self.rpc],
                        note="head-age")
        if not ts.isdigit():
            return None
        return int(time.time()) - int(ts)

    # ── runtime-volume reads/writes (validator-0 mounts /runtime) ─────────────
    def runtime_cat(self, path: str) -> str:
        """pp_runtime_cat: `docker compose exec -T validator-0 cat /runtime/<path>` (bash wraps
        it in `timeout 15` — PORT-NOTE in proc.py drops the binary for subprocess timeout)."""
        return self.p.run(["docker", "compose", "exec", "-T", topology.RUNTIME_MOUNT_HOST,
                           "cat", f"/runtime/{path}"], timeout=15, note="runtime-cat")

    def runtime_write(self, path: str, content: str) -> bool:
        """pp_runtime_write: `docker compose exec -T validator-0 sh -c 'cat > /runtime/<path>'`
        with <content> on stdin."""
        return self.p.pipe_to_exec(topology.RUNTIME_MOUNT_HOST, path, content)

    def owner_key(self, idx) -> str:
        """pp_owner_key: 0x-prefixed L2 owner key from keys/owner-<idx>.hex."""
        raw = self.runtime_cat(f"keys/owner-{idx}.hex")
        return f"0x{raw}" if raw else "0x"

    def owner_addr(self, idx) -> str:
        """pp_owner_addr: derive the owner ADDRESS from the durable per-idx key via
        `cast wallet address --private-key 0x<key>` (lowercased); jq-on-addresses.json fallback
        only if the key file is absent (byte-identical native path)."""
        raw = self.runtime_cat(f"keys/owner-{idx}.hex")
        if raw:
            addr = self.p.run(["cast", "wallet", "address", "--private-key", f"0x{raw}"],
                              note="owner-addr")
            if addr.startswith("0x"):
                return addr.lower()
        # fallback: jq -r ".validators[idx]" over addresses.json
        aj = self.runtime_cat("addresses.json")
        try:
            return (json.loads(aj)["validators"][int(idx)] or "").lower()
        except Exception:
            return ""

    def consensus_keys(self, idx) -> dict:
        """pp_consensus_keys: run genesis-bootstrap consensus-keys one-off; returns the parsed
        {validatorAddress, blsPubkeyUncompressed, blsPoPUncompressed, peerPubkey, ownerKey}.
        `--peers` is the POOL size: derive() is peers-independent (keys.rs hashes only
        seed|role|idx → byte-identical at any peer count) but run_consensus_keys ASSERTS
        idx < peers. bash exports PP_PEERS=<pool ceiling> (case-soak.sh) so joiners past the
        default 6 pass the assert; the port never bumped it, so a growth joiner (idx>=6) with
        PP_PEERS=6 hit `idx<peers`==false → empty stdout → empty bls/pop/peer → cast
        `parser error` on the empty bytes32. Size --peers to cover idx so the write layer
        self-corrects regardless of the env, and FAIL LOUD (like bash `jq -r` under set -e)
        when the run yields no keys instead of silently emitting empty cast args."""
        peers = max(int(self.pp_peers), int(idx) + 1)
        out = self.p.run(["docker", "compose", "run", "--rm", "--no-deps", "-T",
                          "--entrypoint", "/usr/local/bin/genesis-bootstrap", topology.GENESIS_INIT,
                          "consensus-keys", "--idx", str(idx), "--peers", str(peers),
                          "--chain-id", str(self.chain_id)], note="consensus-keys")
        try:
            ck = json.loads(out) if out else {}
        except Exception:
            ck = {}
        if self.p.dry:
            return ck                                   # transcript-only: record argv, don't gate
        if not all(ck.get(f) for f in
                   ("blsPubkeyUncompressed", "blsPoPUncompressed", "peerPubkey")):
            raise ChainError("consensus-keys",
                             f"idx {idx}: genesis-bootstrap emitted no consensus keys "
                             f"(peers={peers}); refusing to send empty setConsensusKeys args")
        return ck

    # ── epoch / committee reads over the RUNTIME staking cluster ──────────────
    def _pp_cfg_read_retry(self, sig: str):
        """_pp_cfg_read_retry (lib.sh:719-735): read a ChainConfig getter up to 3× (1 s apart);
        returns int, or None when all three attempts FAILED (the caller then memoizes).

        Two details of the bash are load-bearing and were both lost in the first port:

        * **it branches on cast's EXIT CODE, not on emptiness** — `if out=$(pp_chainconfig_call …)`.
          A cast that SUCCEEDS and prints nothing is a legitimate answer (bash's
          `printf '%d' "" || echo 0` yields 0) and must NOT be retried; only a non-zero cast is a
          transient. Reading rc needs `run_capture`; `run` cannot see it, so the old port retried
          three times over a successful empty read and then reported a transient failure — which
          the caller answers with a MEMOIZED interval, i.e. a non-zero epoch off a chain whose
          ChainConfig said nothing.
        * **the `[sci]` suffix strip** (§2.4 item 9). `cast call …(uintN)` pretty-prints any value
          >= 10000 as `22920 [2.292e4]`, and `printf '%d'` on the whole string emits the int but
          EXITS NON-ZERO on the trailing ` [..]`, so bash's `|| echo 0` then APPENDS a `0` →
          `229200`: a bogus activation block past the head that pinned the epoch to 0 and silently
          disabled all churn. `_first_token` is that strip.
        """
        for attempt in range(3):
            r = self.p.run_capture(["cast", "call", self.chain_config_rt, sig,
                                    "--rpc-url", self.rpc], note="cfg-read")
            if r.ok:
                tok = _first_token(r.stdout)
                try:
                    return int(tok) if tok else 0     # bash `printf '%d' "" || echo 0`
                except ValueError:
                    return 0
            if attempt < 2:
                time.sleep(1)
        return None

    def current_epoch(self) -> int:
        """pp_current_epoch: (head - activation) / interval, both read from ChainConfig and
        MEMOIZED (M4: never default interval→64 / act→0 on a blip → false growth-stall)."""
        head_hex = self._head_hex()
        head = _hex_to_dec(head_hex)
        interval = self._pp_cfg_read_retry("getEpochBlockInterval()(uint32)")
        if interval is not None:
            self._interval_cache = str(interval)
        else:
            interval = int(self._interval_cache or os.environ.get("EPOCH_INTERVAL", "64"))
        act = self._pp_cfg_read_retry("getDposActivationBlock()(uint64)")
        if act is not None:
            self._act_cache = str(act)
        else:
            act = int(self._act_cache or os.environ.get("DPOS_ACTIVATION_BLOCK", "0"))
        if interval == 0 or head < act:
            return 0
        return (head - act) // interval

    def committee(self, epoch) -> str:
        """pp_committee: getEpochCommittee(epoch) as a sorted, lowercased, space-joined addr
        set (membership comparable regardless of on-chain order); "" when empty/unreadable."""
        out = self.p.run(["cast", "call", self.staking_rt,
                          "getEpochCommittee(uint64)(address[])", str(epoch),
                          "--rpc-url", self.rpc], note="committee")
        addrs = sorted(a.lower() for a in re.findall(r"0x[0-9a-fA-F]{40}", out))
        return " ".join(addrs)

    def committee_has(self, addr: str, epoch) -> bool:
        """committee_has: is <addr> (lowercased) in getEpochCommittee(epoch)?"""
        if not addr:
            return False
        return addr.lower() in self.committee(epoch).split()

    def validator_status(self, addr: str) -> str:
        """pp_validator_status: 2nd tuple field (status byte) of getValidatorStatus over the
        RUNTIME staking cluster (the predeploy 0x..5201 is codeless here)."""
        out = self.p.run(["cast", "call", self.staking_rt,
                          "getValidatorStatus(address)"
                          "(address,uint8,uint256,uint64,uint64,uint16)",
                          addr, "--rpc-url", self.rpc], note="validator-status")
        parts = out.split()
        return parts[1] if len(parts) >= 2 else ""

    def validator_stake(self, addr: str) -> int:
        """3rd tuple field (uint256 stake) of getValidatorStatus — the member's CURRENT total
        delegated stake, the quantity the on-chain top-k committee selection ranks by. 0 when the
        read is unavailable (a stratify top-up then treats it as 'needs the full target')."""
        out = self.p.run(["cast", "call", self.staking_rt,
                          "getValidatorStatus(address)"
                          "(address,uint8,uint256,uint64,uint64,uint16)",
                          addr, "--rpc-url", self.rpc], note="validator-stake")
        parts = out.split()
        try:
            return int(parts[2]) if len(parts) >= 3 else 0
        except ValueError:
            return 0

    def staking_current_epoch(self):
        """pp_staking_call 'currentEpoch()(uint64)' — the STAKING contract's OWN epoch cursor,
        which is the argument domain of the epoch-frozen selection view. Deliberately NOT
        current_epoch() (that one DERIVES the epoch from head/interval off ChainConfig and
        memoizes; a derived value that drifts one epoch would make the purity probe compare a
        different epoch's view before and after). None when the read fails → the probe SKIPS
        rather than judging an unreadable RPC."""
        r = self.p.run_capture(["cast", "call", self.staking_rt, "currentEpoch()(uint64)",
                                "--rpc-url", self.rpc], note="staking-current-epoch")
        tok = _first_token(r.stdout)
        if not r.ok or not tok:
            return None
        try:
            return int(tok)
        except ValueError:
            return None

    def selection_view_at(self, epoch):
        """pp_staking_call 'getValidatorsWithKeysAt(uint64)(address[],(bytes,bytes32,uint64)[])'
        <epoch> — the EPOCH-FROZEN selection view (the candidate set + stake snapshot + cap the
        committee of that epoch is cut with). Returned as the raw `cast` stdout: the probe only
        ever compares it to ITSELF across one write, so the decoded shape is irrelevant and any
        re-encoding would only add a way to lose a difference.

        None when the read failed (rc != 0 / empty) — the bash `|| echo READ_FAILED` sentinel,
        kept as a DISTINCT value from a successfully-read view so an RPC brownout can never be
        reported as a purity violation."""
        r = self.p.run_capture(["cast", "call", self.staking_rt,
                                "getValidatorsWithKeysAt(uint64)"
                                "(address[],(bytes,bytes32,uint64)[])", str(epoch),
                                "--rpc-url", self.rpc], note="selection-view")
        out = (r.stdout or "").strip()
        if not r.ok or not out:
            return None
        return out

    def active_validators_length(self) -> int:
        out = self.p.run(["cast", "call", self.chain_config_rt,
                          "getActiveValidatorsLength()(uint32)", "--rpc-url", self.rpc],
                         note="active-len")
        try:
            return int(_first_token(out) or 0)
        except ValueError:
            return 0

    def _head_hex(self) -> str:
        # check_external 8545 | cut -d'|' -f1 — reuse nodes for the read (READ side is pure).
        from ..core import nodes
        return nodes.check_external(topology.HOST_RPC_PORT).split("|", 1)[0]

    def nonce(self, addr: str):
        out = self.p.run(["cast", "nonce", "--rpc-url", self.rpc, addr], note="nonce")
        return int(out) if out.isdigit() else None

    # ── token / ETH funding ───────────────────────────────────────────────────
    def token_transfer(self, token: str, to: str, amount, checked: bool = False) -> None:
        """pp_token_transfer (lib.sh:908-911): `cast send <token> transfer(address,uint256)(bool)
        <to> <amt> --rpc-url <RPC> --private-key <owner-0 key>`.

        `checked` is where bash's two call sites differ, and the difference is real. The bash
        FUNCTION carries no `|| true`, so under the caller's `set -e` a failed transfer ABORTS —
        which is what `pp_bring_up_rotation`'s bare call (lib.sh:1282) relies on. `pp_ensure_blend`
        swallows it at ITS call site instead, because there a timed-out send may still land and the
        next balance probe is the real verdict.

        The default stays `False` (swallow) so the sim's existing call sites are unchanged; the
        rotation bring-up asks for `True` and gets bash's fail-loud."""
        argv = ["cast", "send", token, "transfer(address,uint256)(bool)", to, str(amount),
                "--rpc-url", self.rpc, "--private-key", self.owner_key(0)]
        if checked:
            r = self.p.run_capture(argv, note="token-transfer")
            if not self.p.dry and not r.ok:
                raise ChainError("token-transfer",
                                 f"transfer {amount} of {token} to {to} failed: "
                                 f"{(r.merged or '').splitlines()[-1] if r.merged else ''}")
            return
        self.p.run_ok(argv, note="token-transfer")

    def balance_of(self, token: str, who: str) -> int:
        out = self.p.run(["cast", "call", token, "balanceOf(address)(uint256)", who,
                          "--rpc-url", self.rpc], note="balanceOf")
        tok = _first_token(out)
        return int(tok) if tok.isdigit() else 0

    def ensure_blend(self, token: str, to: str, amount) -> bool:
        """pp_ensure_blend: idempotent BLEND funding — non-zero balance ⇒ already funded (fresh
        idx starts at 0); else re-send, bounded 4×. A timed-out transfer that still lands is
        caught by the next balance check."""
        for _ in range(4):
            if self.balance_of(token, to) > 0:
                return True
            self.token_transfer(token, to, amount)
            time.sleep(3)
        return self.balance_of(token, to) > 0

    def fund_eth(self, to: str, wei) -> int:
        """pp_fund_eth (lib.sh:955-992): fee-hardened owner-0→<to> ETH grant.

        THREE DISTINCT RETURN CODES, and conflating them was itself the diagnosed bug
        (bundle-20260716T203448Z, where all three were reported as "budget exhausted"):

            0   the grant landed (receipt `.status == 0x1`)
            1   GENUINE PRE-CHECK FLOOR — owner-0's balance is below the mint floor. The balance
                is surfaced in `fund_eth_bal` so the caller reports it honestly.
            2   SEND/CONFIRM FAILURE — cast erred or produced no success receipt (a stalled or
                congested chain). cast's last error line is surfaced in `fund_eth_err`.
            3   BALANCE-READ FAILURE — the `cast balance` read itself failed. It must NOT collapse
                to bal=0, which would fake a floor hit. Retried once, then classified as transient.

        Both out-parameters (`PP_FUND_ETH_BAL` / `PP_FUND_ETH_ERR` in bash) are cleared on entry
        and are attributes rather than returns, exactly as bash's globals are — the caller reads
        whichever the code points at. The three WARN lines go to stderr as bash writes them; a
        code with no diagnostic behind it is how the three collapsed into one in the first place.

        DRY: early-return 0 with NO cast (bash's dry-run guard at lib.sh:957)."""
        self.fund_eth_bal = ""
        self.fund_eth_err = ""
        if self.p.dry:
            return 0
        owner = self.owner_addr(0)
        bal = None
        for attempt in (1, 2):
            b = self.p.run(["cast", "balance", owner, "--rpc-url", self.rpc], note="balance")
            if b.isdigit():
                bal = int(b)
                break
            if attempt < 2:
                time.sleep(2)
        if bal is None:
            print("WARN (fund_eth): owner-0 balance read FAILED after retry (RPC "
                  "unreachable/garbled) — NOT treating as floor", file=sys.stderr, flush=True)
            return 3
        # `${#bal} <= 18` in bash guards its 64-bit int arithmetic; kept because it is part of the
        # predicate, not of the arithmetic: a balance with more than 18 digits is above ANY floor
        # this harness sets, so the comparison is skipped rather than widened.
        if len(str(bal)) <= 18 and bal < self.max_mint_eth_floor:
            self.fund_eth_bal = bal
            print(f"WARN (fund_eth): owner-0 ETH balance {bal} wei below the "
                  f"{self.max_mint_eth_floor} wei floor — mint budget exhausted",
                  file=sys.stderr, flush=True)
            return 1
        gp = self.p.run(["cast", "gas-price", "--rpc-url", self.rpc], note="gas-price")
        gp = int(gp) if gp.isdigit() else 0
        bid = max(gp * 3, self.mint_gas_floor)
        # `cast send … 2>&1` — bash classifies off the MERGED stream, so the port must capture it.
        # `run` returns stdout only, which left `fund_eth_err` with nothing to report and made a
        # spawn failure indistinguishable from a receipt whose status was not 0x1.
        r = self.p.run_capture(["cast", "send", "--json", "--rpc-url", self.rpc,
                                "--gas-price", str(bid), "--private-key", self.owner_key(0),
                                "--value", str(wei), to], note="fund-eth")
        if r.ok and _receipt_ok(r.merged):
            return 0
        self.fund_eth_err = (r.merged or "").splitlines()[-1] if r.merged else ""
        print(f"WARN (fund_eth): funding send to {to} FAILED/unconfirmed: {self.fund_eth_err}",
              file=sys.stderr, flush=True)
        return 2

    # ── the revert-checked send (sim_send) ───────────────────────────────────
    def send(self, label: str, to: str, sig: str, *args, key: str) -> None:
        """sim_send: `cast send --json --rpc-url <RPC> <to> <sig> <args...> --private-key <key>`;
        assert receipt `.status==0x1`. On no receipt, confirm via the sender's nonce advancing
        (real proof, not sleep-and-hope); re-send only on the known transients up to 3×. A
        genuine revert raises ChainError (fail-loud)."""
        argv_tail = [to, sig, *[str(a) for a in args], "--private-key", key]
        sender = self.p.run(["cast", "wallet", "address", "--private-key", key], note="send-sender")
        pre = self.nonce(sender) if sender else None
        for attempt in range(1, 4):
            r = self.p.run_capture(["cast", "send", "--json", "--rpc-url", self.rpc, *argv_tail],
                                   note=f"send:{label}")
            if self.p.dry:
                return          # transcript: record the send, assume it lands (SIM_DRYRUN)
            # bash `cast send … 2>&1`: classify off the MERGED stream. A receipt lands on stdout;
            # a transient ("nonce too low", "already known", "not confirmed within the timeout")
            # and a TimeoutExpired (mapped to the `<timed-out>` sentinel) land on stderr — dropping
            # stderr made every branch below unreachable (F13).
            out = r.merged
            if r.ok and _receipt_ok(out):
                return
            if _receipt_present_but_reverted(out):
                raise ChainError("send-reverted", f"{label}: tx REVERTED on-chain")
            # no receipt — confirm via nonce, else re-send on transients
            if sender and pre is not None and self._nonce_advanced(sender, pre):
                return
            if attempt < 3 and (r.timed_out or _is_transient(out)):
                time.sleep(3)
                continue
            raise ChainError("send-failed", f"{label}: cast send error / not confirmed ({out[-200:]})")

    def _nonce_advanced(self, addr: str, base: int, budget: int = 30) -> bool:
        """sim_nonce_advanced: poll <addr>'s nonce; True the moment it exceeds <base>."""
        deadline = time.time() + budget
        while time.time() < deadline:
            n = self.nonce(addr)
            if n is not None and n > base:
                return True
            time.sleep(2)
        return False

    # ── governance: propose → vote → execute (pp_gov_action + sim_gov_action) ─
    def gov_action(self, target: str, calldata: str, desc: str, voter_idx=None) -> None:
        """pp_gov_action: drive one onlyFromGovernance action through
        propose → castVote(For) → execute. v0 proposes; the voter set is `voter_idx` (the sim
        passes the CURRENT committee's owner idxs via sim_live_voter_idx) else the 0..need-1
        prefix (need = ceil(2/3·5)+1). Votes go --async; the Succeeded poll is the real sync
        point; execute is receipt-confirmed. Raises ChainError on a terminal non-Succeeded /
        revert."""
        desc_hash = self.p.run(["cast", "keccak", desc], note="gov-keccak")
        pid = _first_token(self.p.run(
            ["cast", "call", self.gov_addr,
             "hashProposal(address[],uint256[],bytes[],bytes32)(uint256)",
             f"[{target}]", "[0]", f"[{calldata}]", desc_hash, "--rpc-url", self.rpc],
            note="gov-hash"))
        self.p.run(["cast", "send", self.gov_addr,
                    "propose(address[],uint256[],bytes[],string)(uint256)",
                    f"[{target}]", "[0]", f"[{calldata}]", desc,
                    "--rpc-url", self.rpc, "--private-key", self.owner_key(0)],
                   note="gov-propose")
        # wait Active — BLOCK-PACED (Family 4 straggler): state==Active or the chain advances 3
        # blocks (genuine miss) / freezes. cond_fn reads state(pid) each poll (argv recorded).
        #
        # BASH DYNAMIC SCOPE (§2.4 item 13): `_pp_gov_active` assigns the enclosing `state`, so
        # the caller's FAIL line names the LAST OBSERVED state ("proposal not Active (state=0)").
        # Python has no such thing, and the first port simply dropped it — the message went blank
        # in exactly the case where the state is the whole diagnosis (0=Pending means votingDelay
        # is not 0; 3=Defeated means it never opened). A one-slot list is the closure equivalent.
        seen_state = [""]

        def _active():
            seen_state[0] = self._gov_state(pid)
            return seen_state[0] == "1"

        if not self.cp.wait(f"gov_active_{pid}", _active, "blocks", 3):
            raise ChainError("gov-not-active",
                             f"proposal not Active (state={seen_state[0]}) for: {desc}")
        voters = self._voter_list(voter_idx)
        for j in voters:
            self.p.run(["cast", "send", self.gov_addr, "castVote(uint256,uint8)(uint256)",
                        pid, "1", "--rpc-url", self.rpc,
                        "--private-key", self.owner_key(j), "--async"], note="gov-vote")
        self._gov_wait_succeeded(pid, desc)
        # bash captures the sender's nonce BEFORE the send (lib.sh:1157-1158) so the no-receipt
        # branch has a baseline to compare against. Read it here, not inside the branch: after the
        # send has landed the nonce has ALREADY advanced, and a baseline taken then can never be
        # exceeded — the confirm would time out on a tx that mined perfectly.
        ex = ["cast", "send", "--json", self.gov_addr,
              "execute(address[],uint256[],bytes[],bytes32)(uint256)",
              f"[{target}]", "[0]", f"[{calldata}]", desc_hash,
              "--rpc-url", self.rpc, "--private-key", self.owner_key(0)]
        if self.p.dry:
            self.p.run(ex, note="gov-execute")
            return              # transcript: record propose/vote/execute, assume it lands
        sender = self.owner_addr(0)
        pre = self.nonce(sender)
        r = self.p.run_capture(ex, note="gov-execute")
        out = r.merged
        if r.ok and _receipt_ok(out):
            return
        # A RECEIPT THAT SAYS "reverted" IS NOT "no receipt" (lib.sh:1161-1164). The first port
        # collapsed the two and fell through to the nonce confirm — but a REVERTED tx consumes its
        # nonce just as a successful one does, so the confirm would pass and a governance action
        # that reverted on-chain would be reported as applied. Re-issue the call as a `cast call`
        # to surface the revert reason, exactly as bash does, and fail loud.
        if _receipt_present_but_reverted(out):
            reason = self.p.run_capture(["cast", "call", *ex[3:]], note="gov-execute-reason").merged
            reason = (reason or "").splitlines()[-1] if reason else ""
            raise ChainError("gov-execute-reverted",
                             f"execute REVERTED (status={_receipt_status(out)}) for: {desc}: "
                             f"{reason}")
        # No receipt (RPC hiccup / cast timeout): the tx may still be pending-then-mined. Confirm
        # via the sender's nonce advancing — BLOCK-PACED (gov_confirm_blocks blocks /
        # gov_confirm_frozen_s freeze), NOT a wall poll. `ex_now` is bash dynamic scope again: the
        # cond_fn writes the caller's last-read nonce so the FAIL line can name it.
        if sender and pre is not None:
            ex_now = [pre]

            def _confirmed():
                ex_now[0] = self.nonce(sender)
                return ex_now[0] is not None and ex_now[0] > pre

            if not self.cp.wait("gov_exec_confirm", _confirmed, "blocks",
                                self.gov_confirm_blocks, self.gov_confirm_frozen_s):
                raise ChainError("gov-execute",
                                 f"execute not confirmed (no receipt, sender nonce stuck at "
                                 f"{pre}, last read {ex_now[0]}) for: {desc} — "
                                 f"{self.cp.fail_msg or 'chain-paced budget/stall'}")
            return
        raise ChainError("gov-execute",
                         f"execute send failed for: {desc}: "
                         f"{(out or '').splitlines()[-1] if out else ''}")

    def _voter_list(self, voter_idx):
        """The THREE voter-selection modes of `pp_gov_action` (lib.sh:1105-1132), in bash's
        precedence order. They are not variants of one rule and the third exists because
        FluentGovernance's quorum is STAKE-WEIGHTED:

        1. an EXPLICIT idx list (`PP_GOV_VOTER_IDX`, or the `voter_idx` argument the sim passes)
           wins over everything. The sim's churn callers pass the CURRENT committee's owner idxs:
           once the original pool tombstones and committee power migrates to MINTED identities
           (idx >= POOL, outside any prefix), a fixed prefix under-votes → for-power below the 2/3
           STAKE quorum → the proposal is Defeated, on a chain where nothing is wrong.
        2. `PP_GOV_VOTE_ALL=1` → every listed voter. Same reason, blunter: over a grown/uneven
           committee the equal-weight heuristic below can fall short of 2/3 of the POWER. All
           votes are "for", so this only ever ADDS for-power and can never flip a passing
           proposal. Default-off, so every existing case keeps the byte-identical prefix.
        3. otherwise the `ceil(2/3·voters)+1` PREFIX — the equal-weight heuristic, correct on the
           uniform 5-validator production-path committee every non-sim caller runs on.
        """
        if voter_idx is None:
            env_idx = os.environ.get("PP_GOV_VOTER_IDX", "").strip()
            voter_idx = env_idx or None
        if voter_idx is not None:
            if isinstance(voter_idx, str):
                return [int(x) for x in voter_idx.split()]
            return list(voter_idx)
        return gov_voter_list(int(os.environ.get("PP_GOV_VOTERS", "5")),
                              vote_all=os.environ.get("PP_GOV_VOTE_ALL", "0") == "1")

    def _gov_state(self, pid: str) -> str:
        """`cast call <GOV> state(uint256)(uint8) <pid> --rpc-url <RPC>` → the OZ Governor
        ProposalState byte (the argv the cond_fns record)."""
        return _first_token(self.p.run(
            ["cast", "call", self.gov_addr, "state(uint256)(uint8)", pid, "--rpc-url", self.rpc],
            note="gov-state"))

    def _gov_wait_succeeded(self, pid: str, desc: str):
        """pp_gov_wait_succeeded: wait until Succeeded(4), BLOCK-PACED off proposalDeadline
        (bundle-20260716T232341Z). Read the voting end block ONCE, poll state each ~2s until the
        head passes end+5; a terminal non-Succeeded (Canceled/Defeated/Expired) fails with the
        ACTUAL state name, a chain freeze fails naming the head age (deferred to finalize-stall)."""
        if self.p.dry:
            return
        end = _first_token(self.p.run(
            ["cast", "call", self.gov_addr, "proposalDeadline(uint256)(uint256)", pid,
             "--rpc-url", self.rpc], note="gov-deadline"))
        end = int(end) if end.isdigit() else None
        last_move = time.time()
        last_head = -1
        while True:
            st = self._gov_state(pid)
            if st == "4":
                return
            if st in ("2", "3", "6"):  # Canceled / Defeated / Expired
                raise ChainError("gov-defeated",
                                 f"proposal {gov_state_name(st)} (state={st}) for: {desc}")
            head = self.block_number()
            now = time.time()
            if head is not None and head != last_head:
                last_head = head
                last_move = now
            if end is not None and head is not None and head > end + 5:
                raise ChainError("gov-past-deadline",
                                 f"proposal {gov_state_name(st)} (state={st}) past voting end "
                                 f"block {end} (head={head}, margin 5) for: {desc}")
            if now - last_move >= GOV_VOTE_FROZEN_S:
                raise ChainError("gov-frozen",
                                 f"chain FROZEN waiting out the voting window (head={last_head} "
                                 f"unmoved for {int(now - last_move)}s; voting end block="
                                 f"{end if end is not None else 'unreadable'}) for: {desc}")
            time.sleep(GOV_POLL_S)

    def calldata(self, sig: str, *args) -> str:
        """`cast calldata <sig> <args...>` (returns 0x…). Recorded so the gov calldata shape is
        pinned by the test."""
        return self.p.run(["cast", "calldata", sig, *[str(a) for a in args]], note="calldata")

    # ── staking lifecycle write flows (soak-actions.sh) ───────────────────────
    def register_setkeys(self, idx) -> None:
        """_sim_register_setkeys: approve(stake) → registerValidator(→Pending) → assert
        status==2 → setConsensusKeys. Every step revert-checked; the status==2 post-assert
        catches a swallowed revert."""
        addr = self.owner_addr(idx)
        key = self.owner_key(idx)
        if not addr.startswith("0x"):
            raise ChainError("register", f"no owner addr for joiner idx {idx}")
        self.send(f"approve(stake) v{idx}", self.token, "approve(address,uint256)(bool)",
                  self.staking_rt, "1000000000000000000", key=key)
        self.send(f"registerValidator v{idx}", self.staking_rt,
                  "registerValidator(address,uint16,uint256)", addr, 0, "1000000000000000000",
                  key=key)
        st = self.validator_status(addr)
        if st != "2":
            raise ChainError("register", f"v{idx} ({addr}): status={st} after register (want 2)")
        ck = self.consensus_keys(idx)
        self.send(f"setConsensusKeys v{idx}", self.staking_rt,
                  "setConsensusKeys(address,bytes,bytes,bytes32)", addr,
                  ck.get("blsPubkeyUncompressed", ""), ck.get("blsPoPUncompressed", ""),
                  ck.get("peerPubkey", ""), key=key)

    def register_only(self, idx) -> None:
        """sim_register_only: register→Pending→setConsensusKeys, then STOP (never activates →
        invisible to committee selection; a deliberate promote is the only way in)."""
        self.register_setkeys(idx)

    def register_activate(self, idx, raise_cap: int = 1, voter_idx=None) -> None:
        """sim_register_activate: register_setkeys → activateValidator (gov) → assert status==1
        → approve+delegate → (growth only) setActiveValidatorsLength(cap+1)."""
        self.register_setkeys(idx)
        addr = self.owner_addr(idx)
        key = self.owner_key(idx)
        self.gov_action(self.staking_rt, self.calldata("activateValidator(address)", addr),
                        f"activate-{idx}", voter_idx=voter_idx)
        self._status_assert(addr, "1", f"activate v{idx} (want 1=Active)")
        self.send(f"approve(delegate) v{idx}", self.token, "approve(address,uint256)(bool)",
                  self.staking_rt, "2000000000000000000", key=key)
        self.send(f"delegate v{idx}", self.staking_rt, "delegate(address,uint256)", addr,
                  "2000000000000000000", key=key)
        if raise_cap == 1:
            cap = self.active_validators_length()
            if cap <= 0:
                raise ChainError("grow-cap", "read activeValidatorsLength")
            target = int(os.environ.get("SIM_VALIDATORS", "14"))
            new_cap = min(cap + 1, target)
            self._cap_raise_with_purity_probe(idx, cap, new_cap, voter_idx)

    def _cap_raise_with_purity_probe(self, idx, cap, new_cap, voter_idx) -> None:
        """The governance cap-raise, WRAPPED in the live selection-view epoch-purity probe
        (soak-actions.sh sim_register_activate, growth branch).

        The selection view for an epoch that has ALREADY STARTED must be immutable: the committee
        cap is checkpointed per epoch and setActiveValidatorsLength defers to E+1, so
        getValidatorsWithKeysAt(S) for S <= currentEpoch cannot move. While the cap was read LIVE,
        this exact mutation retroactively RESIZED the view of past epochs — the DKG index space
        then exceeded the committed committee, every member voted false and the chain wedged (the
        proven 2026-07-21 dkg_logs idx-stall). A real governance cap-raise under a live DKG is
        exactly what the sim issues every growth step, so this is where that regression is
        cheapest to catch.

        Capture S and its view BEFORE the write, re-read AFTER: a difference FAILS the run
        (ChainError — the write layer's fail-loud type; NOT a demoted diagnostic, this is a
        proven stall class, not harness bookkeeping). The epoch may roll between the two reads —
        harmless: S is captured once and only ages, and an aged S is still an epoch that has
        started. An unreadable epoch or view WARNS and SKIPS: an RPC brownout reported as a purity
        violation would be a flaky guard, which is worse than none."""
        purity_epoch = self.staking_current_epoch()
        before = None if purity_epoch is None else self.selection_view_at(purity_epoch)
        self.gov_action(self.chain_config_rt,
                        self.calldata("setActiveValidatorsLength(uint32)", new_cap),
                        f"grow-cap-{new_cap}", voter_idx=voter_idx)
        after = None if purity_epoch is None else self.selection_view_at(purity_epoch)
        if before is not None and after is not None and before != after:
            raise ChainError(
                "selection-view-purity",
                f"grow-cap v{idx}: selection view for epoch {purity_epoch} CHANGED across "
                "setActiveValidatorsLength — the cap leg is live again (2026-07-21 idx-stall "
                "class)")
        if before is None or after is None:
            ep = "<unreadable>" if purity_epoch is None else purity_epoch
            verdict = f"selection-view purity probe could not read epoch {ep} — SKIPPED"
        else:
            verdict = f"selection-view purity @epoch {purity_epoch} OK"
        print(f"  register_activate v{idx}: committee cap {cap}->{new_cap} "
              f"(EffBal lands @E+3; {verdict})", flush=True)

    def bench_promote(self, idx, voter_idx=None) -> None:
        """sim_bench_promote: activateValidator(idx) ONLY (fills a cap-1 hole; never over-cap)
        + status==1 assert. The IN half of a deferred departure."""
        addr = self.owner_addr(idx)
        if not addr.startswith("0x"):
            raise ChainError("promote", f"v{idx}: no owner addr (dropped minted entry?)")
        self.gov_action(self.staking_rt, self.calldata("activateValidator(address)", addr),
                        f"promote-{idx}", voter_idx=voter_idx)
        self._status_assert(addr, "1", f"promote v{idx}")

    def voluntary_exit(self, served_idx, voter_idx=None) -> bool:
        """sim_voluntary_exit: disableValidator(addr) (Active→Pending, committee dips cap-1) +
        status==2 assert. Returns True on a landed exit (the apply-arm's success guard)."""
        addr = self.owner_addr(served_idx)
        self.gov_action(self.staking_rt, self.calldata("disableValidator(address)", addr),
                        f"voluntary-exit-{served_idx}", voter_idx=voter_idx)
        self._status_assert(addr, "2", f"voluntary_exit v{served_idx}")
        return True

    def delegate_shift(self, served_idx) -> None:
        """sim_delegate_shift: approve+delegate 3e18 from owner-0 to the victim's owner addr to
        shift EffBal and force a rotation @E+3. Best-effort (rotation PRESSURE, not success-
        critical) → run_ok, NOT the revert-checked send."""
        addr = self.owner_addr(served_idx)
        key0 = self.owner_key(0)
        self.p.run_ok(["cast", "send", self.token, "approve(address,uint256)(bool)",
                       self.staking_rt, "3000000000000000000", "--rpc-url", self.rpc,
                       "--private-key", key0], note="delegate-approve")
        self.p.run_ok(["cast", "send", self.staking_rt, "delegate(address,uint256)", addr,
                       "3000000000000000000", "--rpc-url", self.rpc, "--private-key", key0],
                      note="delegate")

    # ── chain-driven ROTATION primitives (v61 a20) ────────────────────────────
    # The committee is on-chain a stake-weighted top-k over the whole selection roster, re-ranked
    # every epoch and auto-committed (onlySystemCall). Rotation therefore needs NO harness
    # committee orchestration — only a STAKE/STATUS change; the chain re-selects. These four ops
    # are the whole rotation-phase write surface (register_activate stays for GROWTH cap-raises).
    def undelegate(self, served_idx, amount, key=None) -> None:
        """sim_undelegate: undelegate(addr, amount) from the validator's OWN key — the on-chain
        'сам вышел' (voluntary exit) primitive. Dropping own stake below the warm-bench tier
        makes the chain drop the validator from the top-k at the next selection (E+1 → seen at
        committee[E+3]); NO disableValidator, NO governance. Revert-checked send (fail-loud)."""
        addr = self.owner_addr(served_idx)
        k = key or self.owner_key(served_idx)
        self.send(f"undelegate v{served_idx}", self.staking_rt,
                  "undelegate(address,uint256)", addr, str(amount), key=k)

    def stake_delegate(self, served_idx, amount, key=None) -> None:
        """sim_stake_delegate: approve+delegate <amount> onto <served_idx> (owner-0 funds it) —
        the 'вытеснен' (stake eviction) primitive. Enough extra stake makes a warm-bench key
        out-rank the lowest-staked committee member, so the chain drops that incumbent from the
        top-k. Revert-checked send (unlike delegate_shift, this MUST land to move the boundary)."""
        addr = self.owner_addr(served_idx)
        k = key or self.owner_key(0)
        self.send(f"approve(evict) v{served_idx}", self.token, "approve(address,uint256)(bool)",
                  self.staking_rt, str(amount), key=k)
        self.send(f"delegate(evict) v{served_idx}", self.staking_rt, "delegate(address,uint256)",
                  addr, str(amount), key=k)

    def remove_validator(self, served_idx, voter_idx=None) -> None:
        """sim_remove_validator: removeValidator(addr) via governance — the PERMANENT roster
        removal for a byzantine/equivocation exit once its stake is gone. Frees the top-k slot
        durably (a disableValidator would be reversible; removeValidator is terminal)."""
        addr = self.owner_addr(served_idx)
        self.gov_action(self.staking_rt, self.calldata("removeValidator(address)", addr),
                        f"remove-validator-{served_idx}", voter_idx=voter_idx)

    def activate_bench(self, idx, stake_wei, voter_idx=None) -> None:
        """sim_activate_bench: register_setkeys → activateValidator (gov) → approve+delegate a LOW
        (bench-tier) stake, NO cap raise. Puts a fresh identity into the selection roster BELOW the
        committee cut, so the chain holds it as a warm-bench candidate ready to be selected the
        moment a seated stake drops. The rotation-phase counterpart of register_activate (which
        raises the cap for GROWTH).

        IDEMPOTENT (v61 a22): a provision belt re-attempts this every round until bench_provisioned
        latches, and a gov-defeated activate leaves a Pending (already-registered) identity. A blind
        re-register reverts `ValidatorAlreadyExists` (spam + the belt never latching), so DISPATCH on
        the current status byte (0 NotFound, 1 Active, 2 Pending, 3 Jail):
          - Active(1): already a provisioned bench → return (a re-delegate would over-stake it).
          - Pending(2): SKIP register (it exists) → just activate + stake (retries a defeated activate).
          - else (NotFound/unreadable): the full register → activate → stake path."""
        addr = self.owner_addr(idx)
        key = self.owner_key(idx)
        st = self.validator_status(addr)
        if st == "1":
            return                                  # already Active → fully provisioned, no-op
        if st != "2":
            self.register_setkeys(idx)              # NotFound (or unreadable): register first
        # Pending(2): fall through to activate WITHOUT re-registering.
        self.gov_action(self.staking_rt, self.calldata("activateValidator(address)", addr),
                        f"activate-bench-{idx}", voter_idx=voter_idx)
        self._status_assert(addr, "1", f"activate-bench v{idx} (want 1=Active)")
        self.send(f"approve(bench) v{idx}", self.token, "approve(address,uint256)(bool)",
                  self.staking_rt, str(stake_wei), key=key)
        self.send(f"delegate(bench) v{idx}", self.staking_rt, "delegate(address,uint256)", addr,
                  str(stake_wei), key=key)

    def fund_blend(self, idx, amount) -> None:
        """sim_fund_blend: TOP UP <idx>'s owner BLEND balance to at least <amount> from owner-0.
        Unlike ensure_blend (idempotent — skips ANY non-zero balance) this transfers the DEFICIT so a
        genesis member (registered with only ~1e18) reaches the funding floor it needs to self-
        delegate up into the committee Hi band. A member already at/above <amount> is a no-op."""
        addr = self.owner_addr(idx)
        have = self.balance_of(self.token, addr)
        deficit = int(amount) - have
        if deficit > 0:
            self.token_transfer(self.token, addr, deficit)

    def self_delegate_to(self, idx, target_total_wei) -> None:
        """sim_self_delegate_to: bring <idx>'s TOTAL delegated stake up to <target_total_wei> by
        SELF-delegating the deficit from the member's OWN key (approve+delegate). Self-delegation
        (not owner-0) is deliberate: the сам-вышел path later self-undelegates the member's own
        stake, so the Hi-band stake must belong to the member. A member already at/above target is a
        no-op (delegate 0 would be a wasted tx / possible revert)."""
        addr = self.owner_addr(idx)
        key = self.owner_key(idx)
        target = int(target_total_wei)
        delta = target - self.validator_stake(addr)
        if delta <= 0:
            return
        self.send(f"approve(self-stake) v{idx}", self.token, "approve(address,uint256)(bool)",
                  self.staking_rt, str(delta), key=key)
        self.send(f"self-delegate v{idx}->{target}", self.staking_rt, "delegate(address,uint256)",
                  addr, str(delta), key=key)

    def mint_identity(self, idx, mint_eth_wei) -> int:
        """sim_mint_identity: write-keys one-off → fund ETH+BLEND from owner-0 → register→Pending.
        Returns 0 (minted, MAX_MINTED_IDX bumped by caller), or 2 (DEFER: transient fund
        failure — idx NOT consumed). rc 1 (owner-0 ETH floor) raises ChainError (hard)."""
        self.p.run(["docker", "compose", "run", "--rm", "--no-deps", "-T",
                    "--entrypoint", "/usr/local/bin/genesis-bootstrap", topology.GENESIS_INIT,
                    "write-keys", "--idx", str(idx), "--chain-id", str(self.chain_id)],
                   note="write-keys")
        addr = self.owner_addr(idx)
        if not addr.startswith("0x"):
            raise ChainError("mint", f"no owner addr for freshly-minted idx {idx}")
        rc = self.fund_eth(addr, mint_eth_wei)
        if rc == 1:
            raise ChainError("mint", f"pp_fund_eth idx {idx} (owner-0 ETH floor — budget exhausted)")
        if rc != 0:
            return 2  # transient — DEFER (do not consume the idx)
        if not self.ensure_blend(self.token, addr, "100000000000000000000"):
            raise ChainError("mint", f"BLEND transfer to idx {idx} failed")
        self.register_only(idx)
        return 0

    # ── shared assert (chain-paced status; here a bounded poll on the SAME argv) ─
    def _status_assert(self, addr: str, want: str, label: str):
        """sim_status_assert: assert on-chain status reaches <want> (async execute lands late —
        bundle-20260716T030527Z, so NOT a single zero-slack read)."""
        for _ in range(20):
            if self.validator_status(addr) == want:
                return
            if self.p.dry:
                return
            time.sleep(2)
        raise ChainError("status-assert", f"{label}: status never reached {want}")


# ── module helpers ────────────────────────────────────────────────────────────

def _receipt_ok(out: str) -> bool:
    """jq -r '.status // .receipt.status' == 0x1|1 over a --json receipt."""
    st = _receipt_status(out)
    return st in ("0x1", "1")


def _receipt_present_but_reverted(out: str) -> bool:
    st = _receipt_status(out)
    return st not in ("0x1", "1", None)


def _receipt_status(out: str):
    try:
        obj = json.loads(out)
    except Exception:
        return None
    if not isinstance(obj, dict):
        return None
    return obj.get("status") or (obj.get("receipt") or {}).get("status")


_TRANSIENT = ("not confirmed within the timeout", "timed out", "already known",
              "nonce too low", "replacement transaction underpriced")


def _is_transient(out: str) -> bool:
    return any(t in (out or "") for t in _TRANSIENT)


def build_addr_map(state, chain) -> None:
    """build_addr_map (case-soak.sh:608): rebuild ADDR2IDX (owner-addr → serving container) FROM
    EMPTY over the whole materialized identity pool (0..MAX_MINTED_IDX), keying off the LOGICAL
    identity each slot currently serves. A native/spare/rotation slot serves its OWN idx unless
    SLOT_IDENTITY re-tasked it (reborn); a container-less bench idx is served by whichever slot
    SLOT_IDENTITY points at it (reverse lookup); an un-hosted idx is skipped.

    The map REBUILDS every call (a rebirth changes which slot serves an identity), but the address
    RESOLUTION is MEMOIZED in OWNER_ADDR_CACHE — an owner address is immutable once the durable
    owner-N.hex exists (v53-r572), so the docker+cast spawn pair is paid once per identity ever.

    KEY DOMAIN (F2): OWNER_ADDR_CACHE is keyed by the identity idx as a STRING (bash-faithful — a
    bash array subscript is always a string, and EVER_FAULTED / the battery reverse-lookup key on
    the SAME str(idx) domain). Only a derived 0x* address is cached; a not-yet-materialized mint
    resolves to ""/None and is retried on the next rebuild, never frozen."""
    addr = state.address
    slot_identity = state.container.slot_identity
    val_containers = state.cfg.val_containers
    addr.addr2idx = {}
    for idx in range(0, state.max_minted_idx + 1):
        served = ""
        # NEW-1: SLOT_IDENTITY values are STRINGS in production; compare in the str domain so a
        # reborn slot (str value) still matches the int loop cursor.
        mapped = slot_identity.get(topology.validator(idx))
        if idx < val_containers and (mapped is None or str(mapped) == str(idx)):
            served = topology.validator(idx)
        else:
            for slot, sidx in slot_identity.items():
                if str(sidx) == str(idx):
                    served = slot
                    break
        if not served:
            continue
        key = str(idx)
        a = addr.owner_addr_cache.get(key)
        if not a:
            resolved = chain.owner_addr(idx)
            a = (resolved or "").lower()
            if a.startswith("0x"):
                addr.owner_addr_cache[key] = a
        if a:
            addr.addr2idx[a] = served


def sim_regen_staking_reader(manifest_path: str) -> str:
    """sim_regen_staking_reader: derive staking-reader.json from the DeployStaking manifest's
    ACTUAL deployed addresses, lowercased (kills the create-nonce PREDICTION fragility class).
    Pure (json only, no docker) so it is gate-testable; `stack.bringup` writes the result to
    /runtime. THE only copy — `sim/actions.py` carried an identical one with no production
    caller and it is gone."""
    with open(manifest_path) as f:
        m = json.load(f)
    return json.dumps({
        "staking_address": str(m["staking"]).lower(),
        "chain_config_address": str(m["chain_config"]).lower(),
        "liveness_slashing_address": str(m["liveness_slashing"]).lower(),
    })
