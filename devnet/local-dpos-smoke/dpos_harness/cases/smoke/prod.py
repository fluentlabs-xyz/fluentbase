"""prod.py — the production-path case seam: `ProdCtx`, and the wrapper that owns the EXIT trap.

`SmokeCtx` (`driver.py`) is what `lib.sh` was to the twenty-odd STATIC-stack cases. `ProdCtx` is
what the `pp_*` half of `lib.sh` is to the five RUNTIME-DEPLOY cases — `case-production-path`,
`case-vrf-rotation`, `case-vrf-dkg-halt`, `case-vrf-dkg-durability`, `case-byzantine-vrf`. Same
contract, same dry-run discipline, different devnet:

  * every read goes through here, so `--dry-run` gets a complete behavioural transcript. A read
    whose ARGV the case layer owns is recorded verbatim; a read delegated to `core/nodes.py` is
    recorded as a `[step] read <name>(<args>)` marker, because its argv belongs to the transport
    layer and duplicating the command construction here would be two implementations of one
    command — which is the bug, not the evidence;
  * `poll()` returns immediately under dry, so a dry run walks the choreography ONCE and prints a
    deterministic transcript.

═══ THE THREE-PART EXIT TRAP ══════════════════════════════════════════════════════════════

The static cases trap `tear_down`. These five trap THREE things (case-vrf-rotation.sh:88):

    cleanup() { pp_spammer_stop; rm -f "$MANIFEST"; tear_down; }
    trap cleanup EXIT

and every one of the three matters on the failure path:

  * **`pp_spammer_stop`** reaps a BACKGROUND `cast send` loop. A leaked one keeps sending against
    a torn-down RPC forever — the 2026-07-14 3.5-hour orphan incident. It is also the reason
    `RotationBringUp` does not stop its own spammer: the trap covers the assertion phase too, and
    the bring-up knows nothing about that.
  * **`rm -f "$MANIFEST"`** deletes the deploy manifest. Leaving it makes the NEXT run read a
    stale `runtime-deployment.json` if its own DeployStaking fails — i.e. judge a fresh chain
    through the previous run's contract addresses, which are codeless on it. It is removed on the
    way OUT, not on the way in, precisely so a failed run leaves nothing behind for the next one.
  * **`tear_down`** is `docker compose down -v --remove-orphans` on the production-path project.

§2.4 item 12: bash's `exit 1` inside `pp_bring_up_rotation` TERMINATED THE PROCESS, which fired
that trap. A Python `raise` only unwinds, so `run()` below re-wires all three into a `finally`.
`RotationBringUp` raises and never cleans up after itself — giving the bring-up a teardown the
assertion phase does not have would cover the wrong half.
"""

from __future__ import annotations

import os

from . import verdicts_prod as V
from ...chain.writes import Chain, ChainError
from ...core import converge, nodes, proc, rpc, topology
from ...core.chainpaced import ChainPaced
from ...core.converge import ConvergeError
from ...core.proc import ProcError, Runner
from ...core.spammer import SpammerPool
from ...stack.production_path import (ProductionPathProfile, RotationBringUp,
                                      RotationBringUpError)

#: `SmokeCtx.exec_test`'s bound, for the same reason: the DKG probes are a `find` over a reth
#: datadir on a host running eight containers, and the 15 s RPC ceiling would report "absent" for a
#: file that is there — the answer two of these cases key their kill on.
EXEC_TEST_TIMEOUT = 60
#: The throwaway-container bound. Longer than `EXEC_TEST_TIMEOUT` because `docker compose run`
#: CREATES a container before it runs anything, and on a loaded daemon that alone costs seconds.
VOL_TIMEOUT = 120


class ProdFailure(Exception):
    """An assertion verdict came back false. `case` is the label used in the FAIL line — the same
    string bash puts in `PP_ROT_LABEL`, so a failure reads identically either side of the port."""

    def __init__(self, case: str, message: str):
        self.case = case
        self.message = message
        super().__init__(f"FAIL ({case}): {message}")


class ProdCtx:
    """The case layer's view of a live production-path stack.

    Holds no verdict logic — that is `verdicts_prod.py`. Holds no choreography — that is
    `stack/production_path.py`. What it owns is the read seam and the write helpers, i.e. the
    `pp_*` surface the five cases call.
    """

    def __init__(self, bringup: RotationBringUp):
        self.bu = bringup
        self.p = bringup.p
        self.label = bringup.label
        self.profile = bringup.profile
        self.rpc = bringup.rpc
        self.chain_id = bringup.chain_id
        # The chain-paced primitive, wired to THIS ctx's reads. `pp_wait_epochs` and the gov waits
        # both ride it (lib.sh:904, :1093, :1176), which is why `core/chainpaced.py` is a
        # case-layer dependency and not a sim one.
        self.cp = ChainPaced(block_number=self.block_number, current_epoch=self.current_epoch,
                             head_age_s=self.head_age_s, dry=bool(self.p.dry))

    # -- shape ---------------------------------------------------------------------
    @property
    def dry(self) -> bool:
        return bool(self.p.dry)

    @property
    def chain(self) -> Chain:
        """The write side. Populated by the bring-up, so a case that reaches for it before
        `RotationBringUp.run()` gets a loud AttributeError rather than a `Chain` pointed at four
        empty addresses (which would read every contract as codeless and report "" for
        everything)."""
        if self.bu.chain is None:
            raise ProdFailure(self.label, "no Chain yet — the runtime deploy has not happened")
        return self.bu.chain

    @property
    def spammers(self) -> SpammerPool:
        return self.bu.spammers

    # -- the deploy facts the bash exported ----------------------------------------
    @property
    def token(self) -> str:
        return self.bu.token

    @property
    def staking_rt(self) -> str:
        return self.bu.staking_rt

    @property
    def chain_config_rt(self) -> str:
        return self.bu.chain_config_rt

    @property
    def gov_addr(self) -> str:
        return self.bu.gov_addr

    @property
    def liveness_rt(self) -> str:
        return self.bu.liveness_rt

    @property
    def act(self) -> int:
        return self.bu.act

    @property
    def anchor(self) -> str:
        return self.bu.anchor

    @property
    def epoch_len(self) -> int:
        return self.bu.epoch_len

    def epoch_first_block(self, epoch) -> int:
        """`epoch_first_block <n>` (lib.sh:1362) — `ACT + n * EPOCH_LEN`."""
        return self.bu.epoch_first_block(epoch)

    # -- the read seam -------------------------------------------------------------
    def _read(self, argv, dry_value="", timeout=15) -> str:
        """A read whose ARGV this layer owns. Live: the ambient (non-recording) channel. Dry:
        recorded verbatim, answered with `dry_value`."""
        if self.dry:
            self.p.read(argv, timeout=timeout)
            return dry_value
        return proc.read(argv, timeout=timeout)

    def _read_rc(self, argv, dry_rc: bool = True, timeout=15) -> bool:
        """A read whose whole answer is its EXIT CODE — `find … | grep -q .`, whose stdout is
        empty by construction. `_read` cannot express it (it returns stdout and swallows the
        status), and dropping the `grep -q` to read stdout instead would be a different command
        from the one bash runs, in cases whose kill timing is keyed on exactly this probe."""
        if self.dry:
            self.p.read(argv, timeout=timeout)
            return dry_rc
        return proc.read_capture(argv, timeout=timeout).rc == 0

    def _delegated(self, name: str, args: str, live, dry_value):
        """A read whose argv belongs to `core/nodes.py`. Dry: a transcript marker + a canned
        answer. Live: whatever the core reader says."""
        if self.dry:
            self.p.step("read", f"{name}({args})")
            return dry_value
        return live()

    def poll(self, fn, timeout, poll_s=1, dry_value=True):
        """A wait-for-condition loop; the probe's truthy value, or False on expiry.

        Dry: the probe runs EXACTLY ONCE — so its reads land in the transcript in the right place —
        and its answer is discarded for `dry_value`. Identical to `SmokeCtx.poll`, and it has to
        be: two poll semantics in one harness is one of them being subtly wrong."""
        if self.dry:
            self.p.step("poll", f"{getattr(fn, '__name__', 'condition')} (<= {timeout}s)")
            fn()
            return dry_value
        import time
        deadline = time.monotonic() + timeout
        while True:
            hit = fn()
            if hit:
                return hit
            if time.monotonic() >= deadline:
                return False
            time.sleep(poll_s)

    def sample_until(self, count, fn, sleep_s=0, label="", dry_value=True):
        """`for _ in $(seq 1 N); do …; break; sleep S; done` — the BOUNDED RETRY.

        Distinct from `poll`, which is a wall deadline, and the distinction is the assertion at
        both call sites in this family. `case-production-path.sh:345` takes at most 150 jail
        readings 2 s apart and `case-vrf-dkg-durability.sh:263` at most 30 share readings: the
        NUMBER of readings is what the case bounds, and expressing either as a deadline would make
        it depend on how slow the RPC is that day.

        Returns the first truthy `fn(i)`, or False when the budget ran out. Dry: ONE sample, no
        sleeps — identical to `SmokeCtx.sample_until`, and it has to be."""
        if self.dry:
            self.p.step("sample", f"{label or 'samples'} (<= {count}, {sleep_s}s apart)")
            fn(0)
            return dry_value
        import time
        for i in range(int(count)):
            hit = fn(i)
            if hit:
                return hit
            if i < int(count) - 1 and sleep_s:
                time.sleep(sleep_s)
        return False

    def check(self, ok: bool, message, on_fail=None) -> None:
        """Apply a verdict. `message` may be a CALLABLE and several call sites pass one: bash
        builds its diagnostic inside the `echo` on the FAILURE branch, so the RPCs behind it are
        issued only when the case has already failed. An f-string evaluates eagerly, which would
        issue them on every pass as well — extra load on the chain being measured, and extra reads
        in a transcript whose whole job is to match bash's.

        A DRY run never fails a verdict: its readings are canned, so a verdict computed over them
        is meaningless in both directions. The verdict functions are driven through both
        directions by `tests/test_prod_substrate.py` instead."""
        if self.dry or ok:
            return
        if on_fail is not None:
            on_fail()
        raise ProdFailure(self.label, message() if callable(message) else message)

    def sleep(self, seconds) -> None:
        """A fixed wall wait. Dry: recorded, not slept. NEVER shorten one — §2.4 item 3: cases in
        this family pass BY timeout, and the pacing window is half the assertion."""
        if self.dry:
            self.p.step("sleep", f"{seconds}s")
            return
        import time
        time.sleep(seconds)

    # -- reads: chain geometry -----------------------------------------------------
    def block_number(self, dry_value=None):
        """`_soak_block_number` (lib.sh:764) — the `blocks`-domain cursor. None on an unreadable
        RPC, which the chain-paced primitive reads as `read_fail` — NOT as "no progress"."""
        def live():
            out = proc.read(["cast", "block-number", "--rpc-url", self.rpc]).strip()
            return int(out) if out.isdigit() else None
        return self._delegated("block_number", "", live, dry_value)

    def head_age_s(self, dry_value=None):
        """`_soak_head_age_s` (lib.sh:770) — wall seconds since the TIP block's timestamp, the
        chain-frozen escape's input. None (bash `"?"`) when the timestamp is unreadable, so a
        failed read can never present as a frozen chain."""
        def live():
            import time
            ts = proc.read(["cast", "block", "latest", "--field", "timestamp",
                            "--rpc-url", self.rpc]).strip()
            return int(time.time()) - int(ts) if ts.isdigit() else None
        return self._delegated("head_age_s", "", live, dry_value)

    def finalized_dec(self, dry_value=0) -> int:
        """`finalized_dec` (lib.sh:295) — the host RPC's FINALIZED head as a decimal,
        `check_external 8545 | cut -d'|' -f1`. What `pp_current_epoch` measures against.

        NAMED FOR THE BASH FUNCTION IT IS, and that matters more here than anywhere else in the
        package. The five production-path cases each define a LOCAL `head_dec()` that is a
        different reading — `eth_blockNumber`, the speculative TIP — and use the two for different
        questions in the same file. A ctx method called `head_dec` that answered with FINALIZED
        would silently convert three tip-based negative assertions (the beacon growth probe, and
        both of the halt case's freeze windows) into finalized-based ones, which pass for the
        wrong reason: finalized can climb toward a burst-ahead tip WITHOUT the tip producing a
        single new block."""
        return self._delegated(
            "finalized_dec", "",
            lambda: nodes.hex_to_dec(
                nodes.check_external(topology.HOST_RPC_PORT).split("|", 1)[0]),
            dry_value)

    def tip_dec(self, dry_value=0) -> int:
        """The case-local `head_dec()` (`case-vrf-rotation.sh:100-105`,
        `case-vrf-dkg-halt.sh:56-61`, `case-vrf-dkg-durability.sh:65-70` — three identical
        copies) — `eth_blockNumber`, the SPECULATIVE TIP.

        The counterpart of `finalized_dec` above, and the distinction is the assertion in three
        places: the ACTIVE_LINE growth probe fires at notarize-time derive so it must follow the
        tip, and the halt case's no-progress control keys on the tip because with the DKG outcome
        None the proposer declines to BUILD the boundary block at all — the tip is what freezes."""
        return self._delegated("tip_dec", "", nodes.head_dec, dry_value)

    def current_epoch(self, dry_value=0) -> int:
        """`pp_current_epoch` (lib.sh:743-759) — the decimal RELATIVE DPoS epoch at the host head.

        MEMOIZED, and the memo is load-bearing (§2.4 item 15). The interval and the activation
        block are on-chain CONSTANTS after bring-up, so a transient double-read failure must NOT
        default them to 64/0 — both INFLATE the epoch, and an inflated epoch false-trips the
        growth-landing watchdog. Delegated to `Chain`, which owns the memo, so the ctx and the sim
        cannot end up with two caches that disagree."""
        if self.dry:
            self.p.step("read", "current_epoch()")
            return dry_value
        return self.chain.current_epoch()

    def wait_epochs(self, n) -> bool:
        """`pp_wait_epochs <n>` (lib.sh:901-905) — block until the head crosses <n> more epoch
        boundaries. EPOCH-PACED, which retired the old `60·(n+2)`s wall guess that mis-scaled
        under a degraded block rate: the wait now FOLLOWS the chain and fails only if it FREEZES.

        BASH DYNAMIC SCOPE (§2.4 item 13): `_pp_wait_epochs_reached` is passed BY NAME and reads
        `_cp_wep_start` / `_cp_wep_n` out of `pp_wait_epochs`'s frame. There is no Python
        equivalent, and the naive port — a predicate that re-reads the start epoch each poll —
        would never terminate, because the target would move with the cursor. A closure over the
        start epoch is the equivalent, and the start is captured ONCE, before the loop."""
        n = int(n)
        start = self.current_epoch()

        def reached():
            return self.current_epoch() >= start + n

        return self.cp.wait(f"epochs_{n}", reached, "epochs", n)

    def wait_converge(self, timeout=180, floor: str = ""):
        """`pp_wait_converge [timeout] [floor]` (lib.sh:666) — six validators + the full-node
        aligned at finalized > floor. Returns the aligned `"height|hash"`, or None on expiry.

        Returns rather than raises: bash's call sites each supply their OWN `|| { echo FAIL …;
        exit 1; }` with their own message and their own log-tail depth, and collapsing them into
        one raise here would give five different failures one diagnostic."""
        if self.dry:
            self.p.step("poll", f"pp_wait_converge(<= {timeout}s, floor={floor or 'none'})")
            return "0x0|0xdry"
        reading, _ = converge.wait_aligned(timeout, floor, self.profile.read_nodes)
        return reading

    def wait_finalized_ge(self, target, timeout) -> bool:
        if self.dry:
            self.p.step("poll", f"wait_finalized_ge({target}, {timeout}s)")
            return True
        return converge.wait_finalized_ge(target, timeout)

    def check_external(self, port, dry_value="0x0|0x0") -> str:
        """`check_external <port>` — `"height|hash"`, or the `"null|null"` SENTINEL. The sentinel
        is the contract: an unreachable node must never read as "", because empty strings are
        byte-identical and an all-down poll would false-pass as convergence (§2.4 item 5)."""
        return self._delegated("check_external", str(port),
                               lambda: nodes.check_external(port), dry_value)

    def check_node(self, service: str, dry_value="0x0|0x0") -> str:
        return self._delegated("check_node", service,
                               lambda: nodes.check_node(service), dry_value)

    def baseline_height(self, dry_value=0) -> int:
        """`baseline_height` (lib.sh:402-411) — a SETTLED finalized height for a PRE capture.

        NOT `head_dec`/`finalized_dec`, which map an unreachable RPC onto 0 indistinguishably from
        genesis. All five cases close with `BEFORE=$(baseline_height); sleep 6; AFTER=$(finalized_
        dec); (( AFTER > BEFORE ))`, and a PRE of 0 makes that pass trivially on a chain that has
        stopped — which is precisely what the check exists to catch."""
        return self._delegated("baseline_height", "", converge.baseline_height, dry_value)

    def mixhash_of(self, service: str, block, dry_value="null") -> str:
        """`mixhash_of <svc> <block>` (lib.sh:309) — prev_randao, routed to the host RPC for
        validator-0/full-node and through `docker compose exec` for the rest.

        The cross-node compare in four of the five cases is built on this, and the "null" coercion
        is what keeps a node that does not yet have the block from aborting the read under bash's
        `set -e` — it becomes a READING the verdict can reject, which is the point."""
        return self._delegated("mixhash_of", f"{service}, {block}",
                               lambda: nodes.mixhash_of(service, block), dry_value)

    # -- reads: the runtime volume -------------------------------------------------
    def runtime_cat(self, path: str, dry_value="") -> str:
        """`pp_runtime_cat <path>` (lib.sh:611) — read a file out of validator-0's `/runtime`
        mount, bounded by `timeout 15`. The FLK-2 bound is not optional: this runs per-tick via
        `committee_has`, and a wedged docker daemon must not hang the whole tick (§2.4 item 14)."""
        return self._read(["docker", "compose", "exec", "-T", topology.RUNTIME_MOUNT_HOST,
                           "cat", f"/runtime/{path}"], dry_value=dry_value, timeout=15)

    def owner_key(self, idx, dry_value="0x" + "00" * 32) -> str:
        """`pp_owner_key <idx>` — the 0x-prefixed L2 owner key from `keys/owner-<idx>.hex`."""
        if self.dry:
            self.p.step("read", f"owner_key({idx})")
            return dry_value
        return self.chain.owner_key(idx)

    def owner_addr(self, idx, dry_value=None) -> str:
        """`pp_owner_addr <idx>` (lib.sh:628-636) — the owner ADDRESS, derived on the HOST from the
        durable per-idx private key with `cast wallet address`, lowercased.

        The key-file path is primary and the `addresses.json` lookup is only a FALLBACK, and the
        order is the fix: `keys/owner-N.hex` is written for EVERY idx and survives the wholesale
        rewrites a stray pool-scoped genesis re-run performs, so a MINTED idx no longer resolves to
        null when `addresses.json` is reverted to the pool prefix (geo-soak v53 r572)."""
        if self.dry:
            self.p.step("read", f"owner_addr({idx})")
            return dry_value or f"0x{0xa0 + int(idx):02x}" * 1 + "00" * 19
        return self.chain.owner_addr(idx)

    def consensus_keys(self, idx, dry_value=None) -> dict:
        """`pp_consensus_keys <idx>` (lib.sh:647-651) — re-run the bootstrap binary's
        `consensus-keys` subcommand inside the shared image and parse its JSON.

        `--peers` is the POOL SIZE, not the committee size: `keys::derive()` is peers-INDEPENDENT
        (it hashes only `seed|role|idx`), so the byte output for a given idx is identical at any
        peer count — but `run_consensus_keys` ASSERTS `idx < peers`, so a joiner past the
        production-path 6 must pass the larger pool size or the assert aborts and growth silently
        breaks."""
        if self.dry:
            self.p.step("read", f"consensus_keys({idx})")
            return dict(dry_value or {})
        return self.chain.consensus_keys(idx)

    # -- reads: the runtime staking cluster ----------------------------------------
    def staking_call(self, sig: str, *args, dry_value="") -> str:
        """`pp_staking_call` (lib.sh:669) — `cast call $STAKING_RT …`, the RUNTIME address.

        Deliberately NOT `lib.sh`'s `staking_call`, which reads the genesis PREDEPLOY slot. On this
        stack that slot is CODELESS, so the predeploy read returns empty off a contract that does
        not exist rather than failing — and the caller then has to tell that from a real empty."""
        return self._delegated("staking_call", ", ".join([sig, *map(str, args)]),
                               lambda: nodes.staking_call(sig, *args, addr=self.staking_rt,
                                                          rpc_url=self.rpc), dry_value)

    def chainconfig_call(self, sig: str, *args, dry_value="") -> str:
        """`pp_chainconfig_call` (lib.sh:670) — the same, against `$CHAIN_CONFIG_RT`."""
        return self._delegated("chainconfig_call", ", ".join([sig, *map(str, args)]),
                               lambda: nodes.chainconfig_call(sig, *args,
                                                              addr=self.chain_config_rt,
                                                              rpc_url=self.rpc), dry_value)

    def committee(self, epoch, dry_value="") -> str:
        """`pp_committee <epoch>` (lib.sh:685-692) — the epoch's committee as a sorted, lowercased,
        space-joined address set.

        Never raises on an empty or unreadable answer. bash ends the pipeline in `|| true` because
        an empty committee makes `grep .` exit 1, which under the caller's `pipefail` ABORTS the
        whole run instead of yielding the "" the assertion can report (§2.4 item 8); a Python port
        that raised would invert the same intent."""
        return self._delegated("committee", str(epoch),
                               lambda: V.committee_set(
                                   nodes.staking_call("getEpochCommittee(uint64)(address[])",
                                                      epoch, addr=self.staking_rt,
                                                      rpc_url=self.rpc)),
                               dry_value)

    def committee_has(self, addr: str, epoch) -> bool:
        if not addr:
            return False
        return addr.lower() in self.committee(epoch).split()

    def validator_status(self, addr: str, dry_value="1") -> str:
        """`pp_validator_status <addr>` (lib.sh:677-681) — the status byte over the RUNTIME staking
        cluster. "" when the read FAILED, which the callers treat as a hard error rather than as
        "not jailed"."""
        return self._delegated("validator_status", addr,
                               lambda: V.status_byte(nodes.staking_call(
                                   "getValidatorStatus(address)"
                                   "(address,uint8,uint256,uint64,uint64,uint16)",
                                   addr, addr=self.staking_rt, rpc_url=self.rpc)),
                               dry_value)

    def dkg_qual(self, epoch, dry_value="1") -> str:
        """`pp_dkg_qual <epoch>` (lib.sh:707-714) — the THREE-VALUED on-chain DKG-qualification
        token for the epoch's committee: `"1"` qualified, `"0"` deferred (the incumbent was
        re-committed), `""` UNREADABLE.

        `""` is NOT "deferred" (§2.4 item 6). Deferred is a statement about the chain — the
        incoming committee's DKG did not meet threshold by H_qual, so `committee[E] ==
        committee[E-1]` and the change retries at a later boundary. Unreadable is a statement about
        the RPC and carries no information at all. Branch on `verdicts_prod.dkg_qual_state`."""
        return self._delegated("dkg_qual", str(epoch),
                               lambda: V.dkg_qual_token(nodes.staking_call(
                                   "getDkgQual(uint64)(bool)", epoch, addr=self.staking_rt,
                                   rpc_url=self.rpc)),
                               dry_value)

    # -- reads: logs ---------------------------------------------------------------
    def log_count(self, service: str, pattern: str, dry_value=0) -> int:
        """`log_count <svc> <pattern>` — the whole container log, ANSI-STRIPPED. The strip is
        mandatory and not the reader's discretion (§2.4 item 2): the node writes SGR escapes INSIDE
        its `key=value` pairs, so an unstripped grep for a line that IS present can return zero —
        a silent reader, not a failing one."""
        return self._delegated("log_count", f"{service}, {pattern!r}",
                               lambda: nodes.log_count(service, pattern), dry_value)

    def logs_all(self, service: str, dry_value="") -> str:
        return self._delegated("logs_all", service, lambda: nodes.logs_all(service), dry_value)

    def logs_all_project(self, dry_value="") -> str:
        """`docker compose logs` with NO service — the WHOLE project's logs, ANSI-STRIPPED.

        Three assertions need the project scope rather than one node's: the halt's panic sweep and
        the durability case's slash / equivocation greps. Scoping any of them to the victim would
        search the one node that cannot have logged the event — a slash is dispatched by whichever
        validator OBSERVED the miss."""
        if self.dry:
            self.p.step("read", "logs_all_project()")
            return dry_value
        return rpc.strip_ansi(proc.read(["docker", "compose", "logs"],
                                        timeout=nodes.LOGS_ALL_TIMEOUT))

    def dump_logs(self, tail: int, *services) -> None:
        self.bu.dump_logs(tail, *services)

    # -- reads: in-container probes ------------------------------------------------
    def exec_test(self, service: str, snippet: str, dry_value=False) -> bool:
        """`docker compose exec -T <svc> sh -c '<snippet>'` as a PREDICATE — True iff it exits 0.

        The DKG on-disk gates (`case-vrf-dkg-durability.sh:78-87`, `case-vrf-dkg-halt.sh:65-74`)
        are `find … | grep -q .`: their whole answer is the exit code. Bounded by
        `EXEC_TEST_TIMEOUT` rather than the 15 s RPC ceiling — the probe is a `find` over a reth
        datadir on a host running eight containers, and a timeout here reads as "the file is not
        there", which is the answer two cases key their kill on."""
        argv = ["docker", "compose", "exec", "-T", service, "sh", "-c", snippet]
        return self._read_rc(argv, dry_rc=dry_value, timeout=EXEC_TEST_TIMEOUT)

    def vol_test(self, snippet: str, dry_value=True) -> bool:
        """`docker compose run --rm --no-deps -T --entrypoint sh genesis-init -c '<snippet>'` as a
        PREDICATE (`case-vrf-dkg-durability.sh:93-97`).

        A THROWAWAY container on the SHARED /runtime volume, and the distinction from `exec_test`
        is load-bearing rather than stylistic: `docker compose exec` only works on a RUNNING
        container, and exec-on-STOPPED is a silent no-op. Every file question about a victim that
        is currently down MUST come through here — bash calls that out at `:90-92` because the
        silent no-op is the bug this replaced."""
        argv = ["docker", "compose", "run", "--rm", "--no-deps", "-T", "--entrypoint", "sh",
                topology.GENESIS_INIT, "-c", snippet]
        return self._read_rc(argv, dry_rc=dry_value, timeout=VOL_TIMEOUT)

    def vol_mutate(self, idx, glob: str, snippet: str, note: str, dry_value="") -> str:
        """`vol_mutate_beacon` (`:106-110`) — mutate (or read back) a file under v<idx>'s beacon
        dir from a throwaway container, with `$f` bound to the matched path.

        Recorded through the WRITING runner: it changes bytes on disk, so it is choreography and
        its ORDER relative to the stop/start around it is as much of the evidence as its argv.

        ALWAYS returns the captured stdout and never raises — bash ends the command in `|| true`
        because the inner `sh` exits non-zero on an absent file (the `[ -n "$f" ]` short-circuit),
        which under the caller's `set -e` would silently kill the script. Callers verify success by
        checking the OUTPUT (the corruption's first-bytes readback), never the exit code."""
        inner = (f"f=$(find /runtime/reth-data/v{idx} -type f -name '{glob}' 2>/dev/null "
                 f"| head -1); {snippet}")
        argv = ["docker", "compose", "run", "--rm", "--no-deps", "-T", "--entrypoint", "sh",
                topology.GENESIS_INIT, "-c", inner]
        r = self.p.run_capture(argv, timeout=VOL_TIMEOUT, note=note)
        return (dry_value if self.dry else (r.stdout or "")).strip()

    def proc_cmdline(self, service: str, dry_value="fluent --dpos") -> str:
        """`docker compose exec -T <svc> sh -c 'tr "\\0" " " </proc/1/cmdline'`
        (`case-production-path.sh:191`) — the node's LIVE argv.

        PID 1 is `fluent` because the compose command `exec`s it, so this is what the process is
        ACTUALLY running rather than what the compose file says it should be. That is the whole
        assertion: a regression that re-added `--dpos.follower-upstream` to a committee validator
        would leave the compose file looking fine."""
        return self._read(["docker", "compose", "exec", "-T", service, "sh", "-c",
                           'tr "\\0" " " </proc/1/cmdline'], dry_value=dry_value)

    def peer_gauges_exec(self, service: str, dry_value="") -> str:
        """The IN-CONTAINER commonware registry at its bare root (`:270`), for the join-failure
        diagnostic only. Recorded as a MARKER: bash spells it `wget -qO-` inside the container and
        this goes out as a bounded `curl` through `core/rpc`, so printing bash's argv would be
        describing a process that never runs."""
        return self._delegated("peer_gauges_exec", service,
                               lambda: rpc.metrics_get_exec(
                                   topology.host_url(topology.CONSENSUS_METRICS_PORT),
                                   rpc.compose_exec(service)), dry_value)

    def peer_gauges_host(self, dry_value="") -> str:
        """The same registry on validator-0's HOST port (`:273`)."""
        url = topology.host_url(topology.HOST_CONSENSUS_METRICS_PORT)
        return self._delegated("peer_gauges_host", url,
                               lambda: rpc.metrics_get_url(url), dry_value)

    # -- reads: host wallet --------------------------------------------------------
    def wallet_private_key(self, mnemonic: str, index, dry_value="0x" + "00" * 32) -> str:
        """`cast wallet private-key --mnemonic … --mnemonic-index N` (`case-byzantine-vrf.sh:198`).

        The index is the contract: 6 is the spammer's and 7 the toggle delegator's, both distinct
        from every validator owner key (which come from the bootstrap binary's `derive_32`, not
        from BIP44 at all). A shared index makes the two senders race each other's nonce, which
        surfaces as `replacement transaction underpriced` on a chain where nothing is wrong."""
        return self._read(["cast", "wallet", "private-key", "--mnemonic", mnemonic,
                           "--mnemonic-index", str(index)], dry_value=dry_value).strip()

    def wallet_address(self, mnemonic: str, index, dry_value="0x" + "77" * 20) -> str:
        return self._read(["cast", "wallet", "address", "--mnemonic", mnemonic,
                           "--mnemonic-index", str(index)], dry_value=dry_value).strip()

    # -- writes --------------------------------------------------------------------
    def gov_action(self, target: str, calldata: str, desc: str, voter_idx=None) -> None:
        """`pp_gov_action <target> <calldata> <desc>` (lib.sh:1077-1184) — propose → castVote(For)
        → execute, driven to a confirmed receipt.

        Three things in it are timing requirements rather than style (§2.4 items 9-11): the
        proposal id is `awk '{print $1}'`-stripped of cast's `[sci]` suffix or it fails to re-parse
        as a uint256 argument; the five votes go out `--async` because receipt-awaited sends cost
        1-2 blocks EACH and would overrun the 10-block voting window; and the final `execute` is
        receipt-confirmed because a fire-and-forget send both SWALLOWS a genuine on-chain revert
        and returns before the tx lands, which raced the caller's immediate status read
        (bundle-20260716T030527Z).

        Raises `ProdFailure` with the chain's own reason, so a governance failure reads the same as
        any other assertion failure in this family."""
        try:
            self.chain.gov_action(target, calldata, desc, voter_idx=voter_idx)
        except ChainError as e:
            raise ProdFailure(self.label, f"pp_gov_action: {e.message}") from e

    def calldata(self, sig: str, *args) -> str:
        return self.chain.calldata(sig, *args)

    def token_transfer(self, token: str, to: str, amount, checked: bool = True) -> None:
        """`pp_token_transfer <token> <to> <amount>` (lib.sh:908-911) — BLEND from the deployer.

        `checked=True` by default here, unlike `Chain.token_transfer`: the bash function has no
        `|| true`, so under the case's `set -e` a failed transfer ABORTS. Only `pp_ensure_blend`
        swallows it, and it does so at its own call site."""
        try:
            self.chain.token_transfer(token, to, amount, checked=checked)
        except ChainError as e:
            raise ProdFailure(self.label, e.message) from e

    def ensure_blend(self, token: str, to: str, amount) -> bool:
        """`pp_ensure_blend` (lib.sh:920-930) — IDEMPOTENT BLEND funding, by PROBE.

        The probe is the whole design (§2.4 item 10). A transfer is not idempotent and a timed-out
        receipt does not mean it failed, so a naive re-send double-funds; checking the recipient's
        balance FIRST makes any non-zero balance mean "already landed" (a fresh idx starts at 0).
        Bounded at four attempts. Worst case — a still-pending first send re-sent — merely
        over-funds, which is inert."""
        return self.chain.ensure_blend(token, to, amount)

    def fund_eth(self, to: str, wei) -> int:
        """`pp_fund_eth <to> <wei>` (lib.sh:955-992) — owner-0 → <to>, fee-hardened.

        Returns the THREE-VALUED failure code, not a bool: 0 ok, 1 genuine floor, 2 send/confirm
        failure, 3 balance-READ failure. `verdicts_prod.fund_reason` turns it into the attribution
        line and `fund_is_budget_exhausted` is the only predicate that means "stop minting" —
        `code != 0` is the diagnosed bug (bundle-20260716T203448Z)."""
        return self.chain.fund_eth(to, wei)

    def runtime_write(self, path: str, content: str) -> bool:
        """`pp_runtime_write <path>` (lib.sh:615) — the inverse of `runtime_cat`."""
        return self.chain.runtime_write(path, content)

    def spammer_start(self, from_key: str, to_addr: str, rpc: str = None):
        """`pp_spammer_start <key> <to> [rpc]` — a BACKGROUND value-transfer loop.

        The sender MUST issue no other transaction or its nonce races the deploy txs. A non-default
        `rpc` routes tx pressure THROUGH the cascade (submit to a follower, which gossips
        L3→L2→validator into the proposer pool)."""
        return self.spammers.start(from_key, to_addr, rpc or self.rpc)

    def spammer_stop(self) -> None:
        """`pp_spammer_stop` — reap EVERY spammer this run started, not the last one."""
        self.spammers.stop()

    def forge(self, argv, note: str, env_overlay=None, timeout=600):
        """`forge_l2 <cmd …>` — the `( cd "$SOLIDITY_CONTRACTS_DIR" && "$@" )` wrapper all five
        cases define identically."""
        return self.bu.forge(argv, note=note, env_overlay=env_overlay, timeout=timeout)

    # -- writes: the destructive half ---------------------------------------------
    #
    # Every one of these mutates the chain or the topology, so all go through the RECORDING
    # runner: they are choreography, and in these five cases the ORDER carries as much of the
    # evidence as the argv — a restart issued before the corruption it is supposed to expose is a
    # different test that still passes.

    def compose_stop(self, *services, timeout=None, note="") -> None:
        """`docker compose stop <svc …>` — the GRACEFUL stop, which is what all four kill sites
        here use. `run_checked`: bash runs them bare under `set -e`, so a failed stop aborts the
        case and fires its EXIT trap; here the `ProcError` unwinds to `prod.run`, which prints the
        failure and cleans up in its `finally` (§2.4 item 12)."""
        flag = ["--timeout", str(timeout)] if timeout is not None else []
        self.p.run_checked(["docker", "compose", "stop", *flag, *services],
                           timeout=300, note=note or f"compose-stop {' '.join(services)}")

    def compose_start(self, *services, note="") -> None:
        self.p.run_checked(["docker", "compose", "start", *services],
                           timeout=300, note=note or f"compose-start {' '.join(services)}")

    def _send_argv(self, argv_tail, key):
        """`cast send <to> <sig> <args…> --rpc-url $RPC --private-key <key>`, in BASH'S FLAG ORDER.

        The order is inert to cast and is not inert to the acceptance criterion: the dry-run
        transcript is diffed against the bash argv, and a harness that emitted the two flags the
        other way round would make every one of the ~30 sends in this chunk read as a difference,
        which is how a real difference stops being visible."""
        return ["cast", "send", *[str(a) for a in argv_tail], "--rpc-url", self.rpc,
                *(["--private-key", key] if key else [])]

    def cast_send(self, argv_tail, note: str, key: str = None) -> bool:
        """`cast send … >/dev/null || { echo FAIL …; exit 1; }` — returns whether it succeeded so
        the caller can supply bash's own message.

        NOT `run_checked`: every one of these call sites has an explicit `||` branch with a
        case-specific FAIL line, and collapsing them into one raised `ProcError` would give six
        different failures one diagnostic."""
        r = self.p.run_capture(self._send_argv(argv_tail, key), timeout=180, note=note)
        return True if self.dry else r.ok

    def cast_send_checked(self, argv_tail, note: str, key: str = None) -> None:
        """The BARE spelling — `cast send … >/dev/null` with no `||`, which under the case's
        `set -e` aborts. Every `approve(address,uint256)` in the five is written this way and none
        of them has a message of its own, so a failure here is an infrastructure failure and reads
        as one."""
        self.p.run_checked(self._send_argv(argv_tail, key), timeout=180, note=note)


# ══ the wrapper ═══════════════════════════════════════════════════════════════════════

def tear_down(runner: Runner) -> None:
    """`tear_down` on the production-path project — BARE `docker compose`, resolving the ambient
    `COMPOSE_FILE`, exactly as `lib.sh:500` does.

    Best-effort by contract: a teardown must never turn a passing case into a failing one, so it
    swallows (bash `2>/dev/null || true`). `--remove-orphans` is load-bearing rather than hygiene —
    a case's own overlay containers are orphans of the bare project and survive without it."""
    runner.run_ok(["docker", "compose", "down", "-v", "--remove-orphans"],
                  timeout=300, note="teardown-down")


def run(case: str, assertions, argv=None, contracts_dir=None, manifest=None,
        committee_size=None, bring_up=True, overlays=None, post_manifest=None) -> int:
    """Bring up the production-path stack, run `assertions` in order, clean up. 0 pass, 1 fail.

    `assertions` is a list of `fn(ctx)`, run in order, fail-fast — the first failure ends the run
    and the later assertions do not execute, as bash's `set -e` did.

    The `finally` is the EXIT trap, all three parts of it, in bash's order (spammer, manifest,
    teardown) — see the module header. It runs on EVERY path including a failed BRING-UP, which is
    the case bash covered with `exit 1` and a Python `raise` does not.

    `overlays` and `post_manifest` are the two seams `case-byzantine-vrf` needs, and they are the
    reason it does NOT get a bring-up of its own. Its bash builds one inline (`:163-297`), and a
    line-by-line diff against `pp_bring_up_rotation` leaves exactly three differences: a THIRD
    compose file at the cold restart (`overlays`), five further deployer-funded transfers that MUST
    post-date DeployStaking (`post_manifest`), and the staking-reader assert hoisted ahead of
    `setBlsVerifier` — which cannot change its own outcome, since both of its operands (a static
    file and an already-written manifest) are fixed by the time either position is reached. Two
    seams and one provably-inert reordering is a much smaller surface than a second copy of the
    14-phase bring-up, which is where the bug density in this family actually lives.

    `bring_up=False` remains for a case that wants the ctx, the cleanup and the argv contract
    without a stack coming up underneath it.
    """
    argv = list(argv or [])
    dry = "--dry-run" in argv
    unknown = [a for a in argv if a != "--dry-run"]
    if unknown:
        print(f"{case}: unrecognised argument(s) {unknown} (only --dry-run is accepted)",
              flush=True)
        return 2

    runner = Runner(dry=dry, echo=dry)
    profile = ProductionPathProfile(committee_size=committee_size, extra_overlays=overlays)
    bu = RotationBringUp(runner=runner, label=case, profile=profile,
                         contracts_dir=contracts_dir, manifest=manifest,
                         post_manifest=post_manifest)
    ctx = ProdCtx(bu)

    if dry:
        print(f"# dpos_harness {case} --dry-run (profile={profile.name}, "
              f"validators={profile.val_count}, committee={profile.committee_size}, "
              f"overlays={list(profile.extra_overlays)}, manifest={bu.manifest})")

    rc = 0
    # `RotationBringUp` exports `COMPOSE_FILE` into `os.environ`, because the bare `docker compose
    # exec` readers inherit only that. bash's process exits at the end of a case, so nothing there
    # ever observes the leftover; here a second case — or the harness's own test suite — would
    # inherit a compose file pointing at the previous run's project. Saved here and restored AFTER
    # the teardown, which still needs the value to reach the right project. Same reasoning, and
    # the same deviation, as `driver._Exports.restore`.
    saved_compose = os.environ.get("COMPOSE_FILE")
    try:
        if bring_up:
            bu.run()
        for fn in assertions:
            fn(ctx)
    except (ProdFailure, RotationBringUpError) as e:
        print(f"FAIL ({getattr(e, 'case', None) or getattr(e, 'label', case)}): {e.message}",
              flush=True)
        rc = 1
    except (ChainError, ConvergeError, ProcError) as e:
        print(f"FAIL ({case}): {e}", flush=True)
        rc = 1
    except KeyboardInterrupt:
        print(f"{case}: interrupted", flush=True)
        rc = 130
    finally:
        _cleanup(bu, runner)
        _restore_compose(saved_compose)

    if dry:
        print(f"# {len(runner.log)} commands")
    return rc


def _restore_compose(saved) -> None:
    """Put `COMPOSE_FILE` back the way the case found it. A case that leaves a tuned compose
    environment behind is how the NEXT thing in the same interpreter ends up driving the wrong
    docker project — including the harness's own test suite, which is where this surfaced."""
    if saved is None:
        os.environ.pop("COMPOSE_FILE", None)
    else:
        os.environ["COMPOSE_FILE"] = saved


def _cleanup(bu: RotationBringUp, runner: Runner) -> None:
    """`cleanup() { pp_spammer_stop; rm -f "$MANIFEST"; tear_down; }`, in that order.

    Each step is independently guarded. bash's trap runs under `set -e` too, but every one of its
    three commands already swallows its own failure (`kill … || true`, `rm -f`, `down … || true`);
    here a raise in the first would skip the other two, so each is wrapped. A cleanup that stops
    half-way is how a leaked spammer outlives the stack it was pressuring."""
    for step, fn in (("spammer-stop", bu.spammers.stop),
                     ("manifest-rm", lambda: _rm_manifest(bu, runner)),
                     ("tear-down", lambda: tear_down(runner))):
        try:
            fn()
        except Exception as e:  # noqa: BLE001 — a teardown must never mask the case's verdict
            print(f"  cleanup {step} failed (ignored): {e}", flush=True)


def _rm_manifest(bu: RotationBringUp, runner: Runner) -> None:
    """`rm -f "$MANIFEST"`. Recorded as a transcript step rather than run as a `rm` process: it is
    an in-process file operation, and a transcript printing a `rm` that never runs would be
    describing fiction (`core/proc.py`'s read-marker note)."""
    runner.step("rm", bu.manifest)
    if runner.dry:
        return
    try:
        os.remove(bu.manifest)
    except FileNotFoundError:
        pass


def dry_run_transcript(label: str = "smoke-vrf-rotation", contracts_dir=None) -> int:
    """Print `pp_bring_up_rotation`'s command transcript and exit — the fidelity oracle for the
    one piece of this substrate that has no prior art in Python.

    Reachable as `python -m dpos_harness stack dry-run-rotation`, mirroring `dry-run-static`. The
    static one exists because "its command sequence matches lib.sh argv for argv" is the whole
    claim; the same is true here and there is no other way to check it without a docker daemon."""
    runner = Runner(dry=True, echo=True)
    profile = ProductionPathProfile()
    bu = RotationBringUp(runner=runner, label=label, profile=profile,
                         contracts_dir=contracts_dir)
    print(f"# dpos_harness dry-run-rotation (profile={profile.name}, "
          f"validators={profile.val_count}, committee={profile.committee_size}, "
          f"manifest={bu.manifest})")
    saved_compose = os.environ.get("COMPOSE_FILE")
    try:
        bu.run()
    finally:
        _restore_compose(saved_compose)
    print(f"# {len(runner.log)} commands")
    return 0
