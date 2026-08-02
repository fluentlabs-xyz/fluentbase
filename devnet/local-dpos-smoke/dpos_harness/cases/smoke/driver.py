"""driver.py — the shared skeleton of the five `case-*.sh` wrappers, and the case READ SEAM.

The bash wrappers are 14-30 lines each and identical apart from which assertions they run:

    set -euo pipefail; cd <smoke>; source lib.sh; source asserts.sh
    bring_up_dpos
    trap tear_down EXIT
    assert_<x> [assert_<y> …]

`run()` below is that shape, with two re-wirings that Python forces and one that it does not:

  * `trap tear_down EXIT` becomes `try/finally`. Bash's `exit 1` inside an assertion terminated
    the PROCESS, which fired the trap; a Python `raise` only unwinds, so a missing `finally` would
    leak a running devnet on every failure (§2.4 item 12). `StaticStack` already tears itself down
    on a failed BRING-UP, so the `finally` covers the assertion phase, which is the half bash's
    trap covered.
  * `set -e` becomes exceptions: `SmokeFailure` for an assertion verdict, `ConvergeError` /
    `ProcError` for the infrastructure underneath. All three exit 1, as bash did.
  * fail-fast across a multi-assertion case is UNCHANGED — the first failure aborts the run and
    the later assertions do not execute. `smoke-base` is ordered by increasing sophistication
    precisely so the first thing to break is the simplest one.

═══ THE READ SEAM ═════════════════════════════════════════════════════════════════════════

`SmokeCtx` is what `lib.sh` was to the case scripts: the ONE place a case reaches the chain.
Every read goes through it, for a reason that is not tidiness — it gives `--dry-run` a complete
behavioural transcript. `stack/static_stack.py` can dry-run on writes alone because its reads are
polls whose only output is "keep waiting"; an ASSERTION is the opposite, it issues no writes worth
speaking of and is defined entirely by what it reads. A write-only transcript of `smoke-epoch`
would be empty.

So under `--dry-run` a read is RECORDED and answered with a canned value:

  * a read whose argv the CASE LAYER owns (`cast balance`, `cast receipt`, the funded-key
    `docker compose exec … cat`, the metrics curl) is recorded as its real argv, through the dry
    Runner — that argv is the thing to diff against bash;
  * a read delegated to `core/nodes.py` (`mixhash_of`, `log_count`, `staking_call`, …) is recorded
    as a `[step] read <name>(<args>)` marker instead. Its argv belongs to the transport layer,
    which has its own oracle tests; duplicating the command construction here to get it into this
    transcript would be two implementations of one command, which is the bug, not the evidence.

And the poll loops collapse: `ctx.poll()` returns immediately under dry. A dry run therefore walks
the choreography ONCE, sleeps never, and prints a deterministic transcript.

═══ THE OVERLAY SEAM (the follower trio) ══════════════════════════════════════════════════

Three cases — `cert-follow`, `cert-cascade`, `tx-cascade` — bring up SERVICES THE BASE COMPOSE
FILE DOES NOT DEFINE (followers, a WS man-in-the-middle, an L2 sentry, an L3 downstream). Bash
gives each of them a per-case file array and drives those services through it:

    CF_COMPOSE=(-f docker-compose.yml -f docker-compose.dpos.yml -f docker-compose.cert-follow.yml)
    docker compose "${CF_COMPOSE[@]}" up -d cert-follower

That array is `StaticProfile.compose_args("dpos")` — base, DPoS overlay, then the per-case
`extra_overlays`. The `overlay_*` methods below are the ONE place that array reaches a command,
so a case never assembles compose flags of its own and the file list has exactly one home.

TWO CONSEQUENCES WORTH STATING, because both are easy to get silently wrong:

  * the profile's overlay list is ALSO honoured by `_migrate_to_dpos`'s recreate (that is the
    mechanism `case-byzantine` uses), so the ported recreate line names the case's overlay where
    the bash line does not. Naming an extra `-f` on a recreate that lists its services explicitly
    creates none of that file's services — the effect is nil and the alternative was a second,
    parallel path for the same list, which is the thing this seam exists to prevent.
  * `tear_down` stays BARE. It reaps these containers as ORPHANS (`--remove-orphans`), exactly as
    bash does; teaching it the overlay would be a different teardown, not a more thorough one.

`export_compose_env` is bash's `export MOCK_ROLLUP_ADDR` — a value the compose FILE interpolates
(`--cert-follow.l1-rollup-address=${MOCK_ROLLUP_ADDR:-0x0…}`), set once and inherited by every
later compose command. It rides as a per-call env overlay so the dry-run transcript prints it in
front of the command that consumed it, which is where the evidence belongs; an ambient
`os.environ` mutation would be invisible in the transcript and would outlive the case.
"""

from __future__ import annotations

import json
import os

from ...core import converge, nodes, proc, rpc, topology
from ...core.converge import ConvergeError
from ...core.proc import ProcError, Runner
from ...stack.profiles import StaticProfile
from ...stack.static_stack import StaticStack

#: The five nodes every cross-node beacon assertion compares (`asserts.sh:112`). The import
#: follower is IN the set deliberately — that membership is the whole of assertion E1
#: (cert-follower mixHash == validators), and dropping it would leave the check committee-only.
BEACON_NODES = (topology.validator(0), topology.validator(1), topology.validator(2),
                topology.validator(3), topology.FULL_NODE)

#: `asserts.sh:344` — validator-0's host-published commonware registry.
CONSENSUS_METRICS_URL = (topology.host_url(topology.HOST_CONSENSUS_METRICS_PORT) + "/metrics")

#: `asserts-fault.sh:188` — the SAME registry read at the BARE ROOT, not at `/metrics`.
#: `assert_peers` writes `METRICS="http://localhost:19100/"` and the two spellings are kept
#: distinct rather than unified: commonware's `Metrics::encode()` handler answers on both paths
#: today, but which URL a case reads is an argv fact about that case, and collapsing them would
#: make the transcript stop matching the bash it is evidence against.
PEERS_METRICS_URL = topology.host_url(topology.HOST_CONSENSUS_METRICS_PORT) + "/"

#: `asserts-fault.sh:50` — `assert_deferred` builds its own `rpc()` helper against a HARDCODED
#: `http://localhost:8545`, deliberately not `$RPC`. Pinned here for the same reason: the
#: K-lag/result-commitment reads are about the PRODUCER's two tiers, and an operator who
#: repointed `$RPC` at a follower would be sampling a node that has no consensus namespace.
DEFERRED_RPC_URL = topology.DEFAULT_RPC_URL

#: `case-cert-catchup.sh:93` — `docker compose exec -T <svc> curl -s http://localhost:9100`, the
#: BARE ROOT of the in-container commonware registry, not `/metrics`. A THIRD spelling of the same
#: registry alongside `CONSENSUS_METRICS_URL` and `PEERS_METRICS_URL`, and kept distinct for the
#: reason stated there: which URL a case reads is an argv fact about that case.
IN_CONTAINER_GAUGE_URL = topology.host_url(topology.CONSENSUS_METRICS_PORT)

#: `case-liveness.sh:30` — the committee address file. `validators[i] == validator-i`, which is the
#: whole reason the case can map a service name to an on-chain address at all.
RUNTIME_ADDRESSES_PATH = "/runtime/addresses.json"

#: `case-cert-catchup.sh:94` — the awk pattern `$1 ~ /(^|_)deferred_height$/`, i.e. the ANCHORED
#: bare metric-name suffix `nodes.gauge_val` implements. NOT a substring match: `metric_val` would
#: also answer for a `deferred_height_total` or a `..._deferred_height_bucket`, and the case reads
#: this as a height.
DEFERRED_HEIGHT_GAUGE = "deferred_height"

#: `SmokeCtx.exec_test`'s bound. Not `rpc.RPC_EXEC_TIMEOUT` (15 s): the probe is a `find` over a
#: reth datadir on a host running eight containers, and a timeout here reads as "the file is not
#: there", which is the answer the DKG-restart case keys its restart on.
EXEC_TEST_TIMEOUT = 60


class SmokeFailure(Exception):
    """An assertion verdict came back false. `case` is the smoke label used in the FAIL line."""

    def __init__(self, case: str, message: str):
        self.case = case
        self.message = message
        super().__init__(f"FAIL ({case}): {message}")


class SmokeCtx:
    """The case layer's view of a live static stack: the profile's epoch arithmetic, the reads,
    and the two write helpers (`cast send`). Holds no verdict logic — that is `verdicts.py`."""

    def __init__(self, stack: StaticStack, rpc_url: str = None, chain_id=None):
        self.stack = stack
        self.p = stack.p
        self.rpc = rpc_url or os.environ.get("RPC", topology.DEFAULT_RPC_URL)
        self.chain_id = str(chain_id or os.environ.get("CHAIN_ID", topology.CHAIN_ID))
        # bash `export FOO=…` before a compose command that interpolates it. Empty for every case
        # that does not deploy a contract the compose file must be told about.
        self.compose_env = {}

    # -- shape ---------------------------------------------------------------------
    @property
    def dry(self) -> bool:
        return bool(self.p.dry)

    @property
    def profile(self) -> StaticProfile:
        return self.stack.profile

    @property
    def interval(self) -> int:
        return self.profile.epoch_interval

    @property
    def activation_block(self) -> int:
        return self.profile.activation_block

    def anchor_dec(self, case: str) -> int:
        """`PREV_FIN` as a decimal height, FAIL-LOUD when there is no anchor.

        `PREV_FIN` starts life as the literal string `"null"` and bash's
        `PREV_DEC=$(printf '%d' "$PREV_FIN")` aborts on it under `set -e`. Coercing it to 0 here
        would silently retarget `smoke-epoch` at block `2*interval` — a height every live chain
        passes — so the anchor guard would be gone with nothing to show for it."""
        prev = self.stack.prev_fin
        if not (isinstance(prev, str) and prev.startswith("0x")):
            raise SmokeFailure(case, f"no migration anchor to measure from (PREV_FIN={prev!r}) — "
                                     "bring_up_dpos did not run, or did not reach the swap")
        return nodes.hex_to_dec(prev)

    # -- the read seam -------------------------------------------------------------
    def _read(self, argv, dry_value="", timeout=15) -> str:
        """A read whose ARGV this layer owns. Live: the ambient (non-recording) channel, exactly
        as every other observer in the package. Dry: recorded verbatim, answered with
        `dry_value`."""
        if self.dry:
            self.p.read(argv, timeout=timeout)
            return dry_value
        return proc.read(argv, timeout=timeout)

    def _delegated(self, name: str, args: str, live, dry_value):
        """A read whose argv belongs to `core/nodes.py`. Dry: a transcript marker + a canned
        answer. Live: whatever the core reader says."""
        if self.dry:
            self.p.step("read", f"{name}({args})")
            return dry_value
        return live()

    def poll(self, fn, timeout, poll_s=1, dry_value=True):
        """A wait-for-condition loop; returns the probe's truthy value, or False on expiry.

        Dry: the probe runs EXACTLY ONCE — so its reads appear in the transcript in the right
        place — and its answer is then discarded for `dry_value`. Walking the loop zero times
        would hide a whole assertion's reads; walking it to a real verdict would need a canned
        chain, which is a fixture nobody would keep true."""
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

    def check(self, case: str, ok: bool, message, on_fail=None) -> None:
        """Apply a verdict. `on_fail` runs before the raise (the bash log dumps).

        A DRY run never fails a verdict: its readings are canned, so any verdict computed over
        them is meaningless in both directions. The verdict functions are exercised — in both
        directions — by `tests/test_smoke_verdicts.py` and `test_smoke_fault_verdicts.py`
        instead, which is the only place a FAIL branch can be driven at all (a live run only ever
        walks the PASS side).

        `message` may be a CALLABLE, and several fault assertions pass one. Bash builds its
        diagnostic inside the `echo` on the failure branch — `echo "FAIL …: victim=$(check_node
        …), v0=$(check_external 8545)"` — so those RPCs are issued only when the case has already
        failed. An f-string evaluates eagerly, which would issue them on every PASS as well:
        extra load on the chain being measured, and extra reads in a transcript whose whole job
        is to match bash's. Deferring the call is what keeps the failure diagnostic on the
        failure path where bash put it."""
        if self.dry or ok:
            return
        if on_fail is not None:
            on_fail()
        raise SmokeFailure(case, message() if callable(message) else message)

    def sleep(self, seconds) -> None:
        """A fixed wall wait (the pacing window). Dry: recorded, not slept. NEVER shorten one of
        these — §2.4 item 3: three cases in this tree pass BY timeout, and the pacing window is a
        measurement interval whose length is half the assertion."""
        if self.dry:
            self.p.step("sleep", f"{seconds}s")
            return
        import time
        time.sleep(seconds)

    def repeat(self, count, fn, sleep_s=0, label=""):
        """`for _ in 1 2 3 4 5 6; do …; sleep S; done` — a SAMPLE COUNT, not a wall deadline.

        `assert_deferred`'s K-lag check is six readings of a moving chain, and the count is the
        assertion: six samples is what makes "the lag never once sat at exactly K" a statement
        about the derive pipeline rather than about one unlucky read. A `poll`-shaped rewrite
        would turn it into a deadline and quietly change how many times it looked.

        Returns the list of `fn(i)` values. Dry: ONE sample, no sleeps.

        Port delta from bash, deliberate: bash sleeps after the LAST sample too (2s of dead time
        before the loop condition fails); this sleeps only BETWEEN samples. Same as
        `core/lifecycle.shutdown_flushed`'s trailing-sleep note."""
        if self.dry:
            self.p.step("sample", f"{label or 'samples'} (x{count}, {sleep_s}s apart)")
            return [fn(0)]
        import time
        out = []
        for i in range(int(count)):
            out.append(fn(i))
            if i < int(count) - 1 and sleep_s:
                time.sleep(sleep_s)
        return out

    def sample_until(self, count, fn, sleep_s=0, label="", dry_value=True):
        """`for _ in 1..N; do …; if hit; then break; fi; sleep S; done` — the BOUNDED retry.

        Distinct from `poll`, which is a wall deadline. `assert_deferred`'s consensus-tier probe
        takes at most six atomic snapshots and fails on the sixth; expressing it as a deadline
        would make the number of snapshots depend on how slow the RPC is that day.

        Returns the first truthy `fn(i)`, or False when the budget ran out."""
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

    # -- reads: chain state --------------------------------------------------------
    def finalized_dec(self, dry_value=0) -> int:
        """`finalized_dec` (lib.sh:273) — the PINNED producer and nothing else. Deliberately not
        `nodes.finalized_dec`, whose committee-wide fallback would answer with a DIFFERENT node's
        height and let a case compare two chains' heads."""
        return self._delegated("finalized_dec", "", converge.finalized_dec_pinned, dry_value)

    def head_dec(self, dry_value=0) -> int:
        return self._delegated("head_dec", "", nodes.head_dec, dry_value)

    def wait_finalized_ge(self, target, timeout) -> bool:
        if self.dry:
            self.p.step("poll", f"wait_finalized_ge({target}, {timeout}s)")
            return True
        return converge.wait_finalized_ge(target, timeout)

    def wait_follower_align(self, port, floor_dec, timeout, dry_value="0x80|0xdry"):
        """`wait_follower_align <port> <floor> [timeout]` (lib.sh:213-227) — the follower on host
        `port` is above `floor_dec` AND the producer's block AT THE FOLLOWER'S HEIGHT is the
        follower's block. Returns the follower's aligned reading, or None on expiry.

        The hash is what makes this an alignment check rather than a progress check: a follower
        that reached the same height on a different block is the fork the three cascade cases
        exist to catch. It is compared at the follower's OWN height, not between the two tips —
        two independently-advancing tips only read equal by luck (see converge.py)."""
        if self.dry:
            self.p.step("poll", f"wait_follower_align({port}, > {floor_dec}, <= {timeout}s)")
            return dry_value
        return converge.wait_follower_align(port, floor_dec, timeout)

    def read_sequencer_nodes(self, dry_value=None):
        """The five-node reading set of `_read_sequencer_nodes` — what `assert_epoch`'s alignment
        loop polls."""
        if self.dry:
            self.p.step("read", "read_sequencer_nodes()")
            return dry_value if dry_value is not None else []
        return self.stack.read_sequencer_nodes()

    def baseline_height(self, dry_value=0) -> int:
        """`baseline_height` (lib.sh:380-389) — a settled finalized height for a PRE capture.

        NOT `finalized_dec`. That one maps an unreachable RPC onto 0, indistinguishably from
        genesis, and every destructive case below then measures `finalized > PRE` against PRE=0,
        which passes trivially and masks the stalled rejoin the case exists to catch. This one
        retries and then FAILS LOUD (`ConvergeError`), which `driver.run` reports as a failure."""
        return self._delegated("baseline_height", "", converge.baseline_height, dry_value)

    def check_external(self, port, dry_value="0x0|0x0") -> str:
        """`check_external <port>` (lib.sh:54) — `"height|hash"` at a host RPC port, or the
        `"null|null"` sentinel. The sentinel is the contract: an unreachable node must never
        read as "", because five empty strings are byte-identical and an all-down poll would
        false-pass as convergence (§2.4 item 5)."""
        return self._delegated("check_external", str(port),
                               lambda: nodes.check_external(port), dry_value)

    def check_node(self, service: str, dry_value="0x0|0x0") -> str:
        """`check_node docker compose exec -T <svc>` (lib.sh:39) — same reading, in-container."""
        return self._delegated("check_node", service,
                               lambda: nodes.check_node(service), dry_value)

    def peer_count(self, service: str, dry_value=1) -> int:
        """`peer_count <svc>` (lib.sh:394) — reth devp2p `net_peerCount` INSIDE the container.
        The EL transport plane, which is a different question from commonware connectivity."""
        return self._delegated("peer_count", service,
                               lambda: nodes.peer_count(service), dry_value)

    def mixhash_of(self, service: str, block, dry_value="null") -> str:
        return self._delegated("mixhash_of", f"{service}, {block}",
                               lambda: nodes.mixhash_of(service, block), dry_value)

    def mixhash_in(self, service: str, block, dry_value="null") -> str:
        """`mixhash_in <svc> <block>` (lib.sh:286) — the IN-CONTAINER read, unconditionally.

        Deliberately not `mixhash_of`, which routes validator-0 and the full-node to their host
        RPCs. The two fault cases read the RESTARTED victim's own view of the gap blocks, and
        the whole point is that it is the victim's node answering — routing that through a host
        port would, for a victim that happened to be validator-0, silently compare the reference
        chain against itself and pass on any divergence."""
        return self._delegated("mixhash_in", f"{service}, {block}",
                               lambda: rpc.block_field_in(service, block, "mixHash"), dry_value)

    def json_rpc(self, method: str, params=None, dry_value=None) -> dict:
        """`assert_deferred`'s own `rpc()` helper (asserts-fault.sh:52-56) — a JSON-RPC POST to
        the producer, parsed. Recorded as a MARKER, not as the `curl` line bash spells: this goes
        out over urllib like every other host read in the package (`core/rpc.py`'s PORT-NOTE),
        and a transcript printing a curl process that never runs would be describing fiction."""
        params = [] if params is None else params
        if self.dry:
            self.p.step("read", f"json_rpc({method}, {json.dumps(params)})")
            return dict(dry_value or {})
        out = rpc.rpc_post_url(DEFERRED_RPC_URL, rpc.rpc_body(method, params))
        try:
            obj = json.loads(out) if (out or "").strip() else {}
        except ValueError:
            obj = {}
        return obj if isinstance(obj, dict) else {}

    def peers_metrics(self, dry_value="") -> str:
        """validator-0's commonware registry at the bare root (`asserts-fault.sh:188`)."""
        return self._delegated("peers_metrics", PEERS_METRICS_URL,
                               lambda: rpc.metrics_get_url(PEERS_METRICS_URL), dry_value)

    def shutdown_flushed(self, service: str, dry_value=True) -> bool:
        """`shutdown_flushed <svc>` (lib.sh:66-84) — the graceful-stop BARRIER, polled off
        `docker inspect`, never off logs (the log read raced under daemon load and false-reported
        "flush incomplete" for a container that exited 0 in under a second; §2.4 item 4).

        In `assert_full_restart` this is not a barrier but the ASSERTION: every validator must be
        observed exited with code 0, which is what "the persistence flushed" means. A SIGKILL at
        the 40s stop ceiling exits 137 and this returns False."""
        return self._delegated("shutdown_flushed", service,
                               lambda: self.stack.shutdown_flushed(service), dry_value)

    def mixhash_at(self, block, dry_value="null") -> str:
        return self._delegated("mixhash_at", str(block),
                               lambda: nodes.mixhash_at(block, self.rpc), dry_value)

    def blockhash_at(self, block, dry_value="null") -> str:
        """`blockhash_at` (lib.sh:352) — the PRODUCER's block hash at a decimal height, over the
        host `cast block`. This is the second read of the same-height fork check: a rejoin /
        reconverge verdict compares each node's hash against the producer's block AT THAT NODE'S
        OWN HEIGHT, so the two tips never have to coincide.

        It goes through `self.rpc` rather than the module default so the fork check reads the
        SAME producer the rest of the case reads; `"null"` for a block the producer does not
        hold (or an unreachable producer), which never matches a real hash."""
        return self._delegated("blockhash_at", str(block),
                               lambda: nodes.blockhash_at(block, self.rpc), dry_value)

    def staking_call(self, sig: str, *args, dry_value="") -> str:
        return self._delegated("staking_call", ", ".join([sig, *map(str, args)]),
                               lambda: nodes.staking_call(sig, *args, rpc_url=self.rpc), dry_value)

    def chainconfig_call(self, sig: str, *args, dry_value="") -> str:
        """`chainconfig_call` (lib.sh:219) — `cast call $CHAIN_CONFIG_ADDR …`.

        A THIRD contract on the same RPC, not a spelling of `staking_call`: the participation
        FLOOR lives on ChainConfig and the participation COUNTERS on LivenessSlashing, and reading
        the floor off the wrong address returns empty rather than failing (`core/nodes.py`'s
        codeless-predeploy note), which the caller would then have to tell from a real 0."""
        return self._delegated("chainconfig_call", ", ".join([sig, *map(str, args)]),
                               lambda: nodes.chainconfig_call(sig, *args, rpc_url=self.rpc),
                               dry_value)

    def validator_status(self, addr: str, dry_value="2") -> str:
        """`validator_status <addr>` (lib.sh:226-234) — the status byte of `getValidatorStatus`'s
        8-field tuple (0 inactive, 1 pending, 2 active, 3 jailed, 4 exiting).

        Returns "" when the read FAILED, and the two cases that use it both treat that as a hard
        error rather than as "not jailed": an empty-vs-"3" false-green would hide the very jail
        `smoke-byzantine` exists to observe and the very jail the DKG-restart case exists to prove
        was AVOIDED (`case-vrf-dkg-restart-midwindow.sh:246-250`)."""
        return self._delegated("validator_status", addr,
                               lambda: nodes.validator_status(addr, rpc_url=self.rpc), dry_value)

    def production(self, epoch, addr: str, dry_value=(9, 10)):
        """`produced_in_epoch <epoch> <addr>` (lib.sh) — `(produced, blocksInEpoch)`, with the
        THREE-VALUED sentinel contract intact (§2.4 item 5, `test_sentinels.py`):

            (-1, -1)   the address is NOT in committee[epoch]
            (-2, -2)   the getter call FAILED (RPC error / empty / partial parse)
            (n,  m)    real counters

        Successor to the retired `participation` bitmap reader — the chain records who PRODUCED
        each block, not who signed a cert, so the credit is exact rather than first-quorum-tail
        truncated. Collapsing either sentinel onto `(0, 0)` is still a FALSE VERDICT: a 0-of-0
        reading looks like absent-but-live, which satisfies `victim produced < hub produced` for
        free. `verdicts_onchain.credit_state` is the one place the three are told apart."""
        return self._delegated("production", f"{epoch}, {addr}",
                               lambda: nodes.production(epoch, addr, rpc_url=self.rpc),
                               dry_value)

    def runtime_addresses(self, dry_value=None):
        """`docker compose exec -T validator-0 cat /runtime/addresses.json` -> the `validators`
        list (`case-liveness.sh:30`, `case-byzantine.sh:22`).

        bash pipes this into HOST-side `jq` because the runtime image has no jq — running jq
        in-container is a 127. Here the parse is in-process, so only the `cat` is a command; the
        argv is the case's own and is recorded verbatim.

        No `tr -d '[:space:]'`, deliberately: bash's `case-liveness.sh:29` calls that out as a
        trap — stripping whitespace over a multi-line `jq -r '.validators[]'` collapses all four
        addresses into one token. The JSON parse has no such hazard, but the ORDER is still
        load-bearing (`validators[i] == validator-i`) and is preserved as read."""
        # The canned addresses are distinct AND none of them is the zero address: a dry transcript
        # naming `0x000…0` for validator-0 would read as the codeless-predeploy hazard rather than
        # as a placeholder.
        raw = self._read(["docker", "compose", "exec", "-T", topology.RUNTIME_MOUNT_HOST,
                          "cat", RUNTIME_ADDRESSES_PATH],
                         dry_value=json.dumps({"validators": dry_value or
                                               ["0x" + f"{0xa0 + i:02x}" * 20 for i in range(4)]}))
        try:
            obj = json.loads(raw) if (raw or "").strip() else {}
        except ValueError:
            return []
        vals = obj.get("validators") if isinstance(obj, dict) else None
        return [str(a) for a in vals] if isinstance(vals, list) else []

    def exec_test(self, service: str, snippet: str, dry_value=False) -> bool:
        """`docker compose exec -T <svc> sh -c '<snippet>'` as a PREDICATE — True iff it exits 0.

        The two on-disk DKG gates (`case-vrf-dkg-restart-midwindow.sh:92-109`) are `find … |
        grep -q .`, i.e. their whole answer is the EXIT CODE and their stdout is empty. `_read`
        cannot express that (it returns stdout and swallows the status), and reading stdout
        instead would need the `grep -q` dropped — a different command from the one bash runs, in
        a case whose restart timing is keyed on exactly this probe."""
        argv = ["docker", "compose", "exec", "-T", service, "sh", "-c", snippet]
        if self.dry:
            self.p.read(argv, timeout=EXEC_TEST_TIMEOUT)
            return dry_value
        return proc.read_capture(argv, timeout=EXEC_TEST_TIMEOUT).rc == 0

    # -- reads: logs and metrics ---------------------------------------------------
    def log_count(self, service: str, pattern: str, dry_value=0) -> int:
        return self._delegated("log_count", f"{service}, {pattern!r}",
                               lambda: nodes.log_count(service, pattern), dry_value)

    def logs_all(self, service: str, dry_value="") -> str:
        """The whole container log, ANSI-STRIPPED. The stripping is mandatory and is not the
        reader's discretion — see `nodes.logs_all` and §2.4 item 2."""
        return self._delegated("logs_all", service, lambda: nodes.logs_all(service), dry_value)

    def logs_tail(self, *services, tail=None, dry_value="") -> str:
        """`docker compose logs [--tail=N] <svc …>` on the BARE project, ANSI-STRIPPED, as a READ.

        `logs_all` is the whole log of ONE service; this is a bounded tail of several, which is
        what the two diagnostic greps in `case-byzantine.sh:41,44` read (`--tail=600` across
        validator-3, then across validator-0 + validator-1). Distinct from `overlay_dump_logs`,
        which PRINTS and answers nothing: these greps decide what gets printed.

        The strip is unconditional and bash has none of it here — §2.4 item 2. Both call sites
        `grep -iE` for `equivocat|slash|…` in a log whose `key=value` pairs carry SGR escapes; an
        unstripped grep can silently return nothing and the failure diagnostic would then say the
        equivocator logged no byzantine markers, which is a second, wrong bug report on top of the
        real one."""
        tail_flag = [f"--tail={tail}"] if tail is not None else []
        argv = ["docker", "compose", "logs", *tail_flag, *services]
        if self.dry:
            self.p.step("read", f"logs_tail({', '.join(services)}, tail={tail})")
            return dry_value
        return rpc.strip_ansi(proc.read(argv, timeout=nodes.LOGS_ALL_TIMEOUT))

    def logs_all_project(self, dry_value="") -> str:
        """`docker compose logs` with NO service — the WHOLE project's logs, ANSI-stripped.

        `case-vrf-dkg-restart-midwindow.sh:242` greps for slash events across every node, and it
        has to: the slash is dispatched by whichever validator observed the miss, so scoping the
        grep to the victim would search the one node that cannot have logged it."""
        if self.dry:
            self.p.step("read", "logs_all_project()")
            return dry_value
        return rpc.strip_ansi(proc.read(["docker", "compose", "logs"],
                                        timeout=nodes.LOGS_ALL_TIMEOUT))

    def deferred_gauge(self, service: str, dry_value="na") -> str:
        """`deferred_gauge <svc>` (`case-cert-catchup.sh:89-95`) — the victim's `deferred_height`
        gauge off its IN-CONTAINER commonware registry, or the literal `"na"`.

        `"na"` rather than 0 is bash's own choice and it is the right one: the gauge is read while
        the container is mid-restart, and a 0 there is a real value meaning "nothing parked". The
        case tracks the PEAK sampled value, so an unreachable read presenting as 0 would be
        indistinguishable from a node that parked nothing — which is the finding the whole case
        turns on.

        Corroboration only: the park is transient (the gauge clears the instant the cert lands),
        so the persistent park `warn!` count is the authority and this is what gets printed
        alongside it."""
        def live():
            text = rpc.metrics_get_exec(IN_CONTAINER_GAUGE_URL, rpc.compose_exec(service))
            return nodes.gauge_val(text, DEFERRED_HEIGHT_GAUGE) or "na"
        return self._delegated("deferred_gauge", service, live, dry_value)

    def node_metric(self, service: str, name: str, dry_value="0") -> str:
        """One metric SAMPLE off ANY service's IN-CONTAINER commonware registry (:9100), by the
        same ANCHORED bare-name match `deferred_gauge` uses (`nodes.gauge_val`).

        `consensus_metrics` above reads validator-0's HOST-published :19100 and nothing else, so
        it cannot answer for the victim; only validator-0 maps a host port
        (`docker-compose.dpos.yml`'s `19100:9100`) while every validator runs
        `--dpos.metrics-port=9100` in-container. A case that measured a counter on the wrong node
        would be reading the hub's view of the victim's recovery.

        ANCHORED and not substring, for the reason `DEFERRED_HEIGHT_GAUGE` records: a substring
        match for `dpos_jump_boundary_refetch` also answers for `..._refetch_failed_total`, and
        the two are read as a positive and a negative of the same event.

        Returns "" when the metric is absent OR the scrape failed — the caller decides what that
        means (`verdicts_boundary.counter_delta` maps it to 0, which is the safe direction on both
        gate shapes there). Deliberately not `"na"` like `deferred_gauge`: that one tracks a PEAK
        across a restart window where 0 is a real reading, and these are monotone counters read
        exactly twice."""
        def live():
            text = rpc.metrics_get_exec(IN_CONTAINER_GAUGE_URL, rpc.compose_exec(service))
            return nodes.gauge_val(text, name)
        return self._delegated("node_metric", f"{service}, {name}", live, dry_value)

    def consensus_metrics(self, dry_value="") -> str:
        """validator-0's host-published :19100 registry — the COMMONWARE one only.

        `nodes.node_metrics` would also concatenate reth's :19200, which `asserts.sh:344` does not
        read. Widening the text a substring match runs over can only add false hits.

        Recorded as a MARKER and not as `curl -s <url>`, even though that is bash's command: this
        goes out over urllib, like every other host HTTP read in the package (`core/rpc.py`'s
        PORT-NOTE), and a transcript that printed a curl line would be describing a process that
        never runs."""
        return self._delegated("consensus_metrics", CONSENSUS_METRICS_URL,
                               lambda: rpc.metrics_get_url(CONSENSUS_METRICS_URL), dry_value)

    def dump_logs(self, tail: int, *services) -> None:
        """`docker compose logs --tail=N [svc …]` on a fail path (never on a passing one)."""
        self.stack.dump_logs(tail, *services)

    # -- reads: cast --------------------------------------------------------------
    def funded_key(self, dry_value="00" * 32) -> str:
        """The funded account's key off validator-0's `/runtime` mount (`asserts.sh:21`).

        `tr -d '[:space:]'` in bash; `.strip()` here. It must not come back empty — every `cast
        send` below would then be signed by `0x`, which fails in a way that reads like an RPC
        problem rather than a missing key."""
        out = self._read(["docker", "compose", "exec", "-T", topology.RUNTIME_MOUNT_HOST,
                          "cat", "/runtime/keys/funded.hex"], dry_value=dry_value)
        return "".join((out or "").split())

    def wallet_address(self, key: str, dry_value="0x" + "11" * 20) -> str:
        return self._read(["cast", "wallet", "address", "--private-key", f"0x{key}"],
                          dry_value=dry_value).strip()

    def cast_balance(self, addr: str, dry_value="0", rpc_url=None) -> str:
        return self._read(["cast", "balance", addr, "--rpc-url", rpc_url or self.rpc],
                          dry_value=dry_value).strip()

    def cast_receipt(self, txhash: str, dry_value="{}", rpc_url=None) -> dict:
        """`cast receipt <h> --rpc-url <RPC> --json`, parsed ONCE.

        bash issues this command twice per hash — once for `.status`, once for `.blockNumber`
        (`asserts.sh:38,41`, `case-tx-cascade.sh:112,118`). One read answering both is the same
        two questions off the same receipt; the only thing lost is a redundant round-trip.

        `rpc_url` exists for the cascade cases, which ask the SAME receipt of two different nodes
        and mean two different questions by it: on the producer it is "a hidden validator mined
        this", on the L3 follower it is "the cascade synced it back". Defaulting it silently to
        the producer would collapse the round trip into the first half."""
        out = self._read(["cast", "receipt", txhash, "--rpc-url", rpc_url or self.rpc, "--json"],
                         dry_value=dry_value)
        try:
            obj = json.loads(out) if (out or "").strip() else {}
        except ValueError:
            obj = {}
        return obj if isinstance(obj, dict) else {}

    def cast_call(self, addr: str, sig: str, *args, dry_value="", rpc_url=None) -> str:
        return self._read(["cast", "call", addr, sig, *[str(a) for a in args],
                           "--rpc-url", rpc_url or self.rpc], dry_value=dry_value)

    def cast_nonce(self, addr: str, dry_value="0", rpc_url=None) -> str:
        """`cast nonce <addr> --rpc-url <url>` (`case-tx-cascade.sh:100`).

        Read off the node the transactions are SUBMITTED to, not off the producer. Both `cast
        send`s below go out `--async`, so cast's own per-invocation "latest" nonce would hand the
        same number to both and the second would be rejected as an underpriced replacement."""
        return self._read(["cast", "nonce", addr, "--rpc-url", rpc_url or self.rpc],
                          dry_value=dry_value).strip()

    def enode_pubkey(self, rpc_url: str, dry_value="ab" * 64) -> str:
        """`_enode_pubkey <url>` (lib.sh:211-214) — the 128-hex devp2p pubkey via `admin_nodeInfo`.

        Delegated: `core/rpc` already validates the 128-hex length, so a caller tests truthiness
        rather than re-deriving the regex. The IP in `admin_nodeInfo` is unreliable inside docker
        and is deliberately NOT returned — the caller rebuilds the enode against the service's
        fixed compose IP (`topology.enode`)."""
        return self._delegated("enode_pubkey", rpc_url,
                               lambda: rpc._enode_pubkey(rpc_url), dry_value)

    def cast_rpc(self, method: str, *args, rpc_url=None, dry_value="") -> str:
        """`cast rpc --rpc-url <url> <method> [args …]` — a raw JSON-RPC READ (`net_peerCount`).

        A read, so it goes out on the ambient channel. The MUTATING spelling of the same command
        (`admin_addTrustedPeer`) is `cast_rpc_write` below and is deliberately a different method:
        one belongs in the transcript and the other does not."""
        return self._read(["cast", "rpc", "--rpc-url", rpc_url or self.rpc, method,
                           *[str(a) for a in args]], dry_value=dry_value).strip()

    def compose_ps_q(self, service: str, dry_value="dry-container-id") -> str:
        """`docker compose ps -q <svc>` — the RUNNING container id (asserts-fault.sh:151,441).

        `-q`, not the `-aq` `core/lifecycle` uses. The difference is the whole point at both call
        sites: they resolve the id of a container that is UP, in order to reach past compose and
        act on it directly (`docker update`, `docker kill`). `-aq` would also answer for a
        container that had already died, which is exactly the case both must refuse."""
        return self._read(["docker", "compose", "ps", "-q", service],
                          dry_value=dry_value).strip()

    # -- writes: the destructive half ---------------------------------------------
    #
    # Every one of these mutates the topology, so all of them go through the RECORDING runner —
    # they are choreography and they belong in the dry-run transcript, where the ORDER is as much
    # of the evidence as the argv. A restore issued before the reading it is supposed to bracket
    # is a different test that still passes.
    #
    # `run_checked`, not `run_ok`, throughout: bash runs these under `set -e`, so a failed stop
    # or restart ABORTS the case and fires its `trap tear_down EXIT`. Here the `ProcError`
    # unwinds to `driver.run`, which prints the failure and tears down in its `finally` — the
    # same two effects, re-wired (§2.4 item 12).

    def compose_stop(self, *services, timeout=None, note="") -> None:
        """`docker compose stop [--timeout N] <svc …>` — the GRACEFUL stop.

        Graceful matters: reth's `on_graceful_shutdown` persists the in-memory tail to MDBX
        before the container exits 0, which is what makes `shutdown_flushed` a meaningful
        assertion afterwards. `assert_crash_survivor` deliberately does NOT come through here —
        it needs the ungraceful path."""
        flag = ["--timeout", str(timeout)] if timeout is not None else []
        self.p.run_checked(["docker", "compose", "stop", *flag, *services],
                           timeout=300, note=note or "compose-stop")

    def compose_start(self, *services, note="") -> None:
        self.p.run_checked(["docker", "compose", "start", *services],
                           timeout=300, note=note or "compose-start")

    def compose_restart(self, *services, note="") -> None:
        self.p.run_checked(["docker", "compose", "restart", *services],
                           timeout=300, note=note or "compose-restart")

    def docker_kill(self, cid: str, note="") -> None:
        """`docker kill <cid>` — a RAW SIGKILL on the container id, bypassing compose.

        asserts-fault.sh:444 explains why it is raw: `docker compose kill`/`start` re-runs the
        service's dependencies, and re-running `genesis-init` races the ungraceful path and made
        the restart flaky. The point of the case is that NOTHING was flushed, so anything that
        adds a graceful step to this line deletes the case."""
        self.p.run_checked(["docker", "kill", cid], timeout=120, note=note or "crash-sigkill")

    def docker_start(self, cid: str, note="") -> None:
        """`docker start <cid>` — the same raw channel as `docker_kill`, for the same reason."""
        self.p.run_checked(["docker", "start", cid], timeout=300, note=note or "crash-restart")

    def docker_update_cpus(self, cid: str, cpus: str, note="") -> None:
        """`docker update --cpus <n> <cid>` — the CPU throttle and its restore.

        This is the one mutation in the chunk that OUTLIVES a Python exception in a way the
        teardown does not automatically undo mid-case, so its caller brackets it in a `finally`.
        A validator left at 0.15 CPU is not a stopped node — it is a slow one, and every later
        assertion would measure a chain that is quietly degraded rather than one that is
        broken."""
        self.p.run_checked(["docker", "update", "--cpus", str(cpus), cid],
                           timeout=120, note=note or "cpu-throttle")

    def netem_delay(self, service: str, delay_ms, iface: str, note="") -> None:
        """`docker compose exec -T <svc> tc qdisc add dev <iface> root netem delay <N>ms` —
        add a one-way egress delay to ONE service (`case-cert-catchup.sh`'s `netem_apply`).

        `run_checked`, deliberately: bash runs it under `set -e`, and a missing `NET_ADMIN` or a
        missing `tc` must abort the case HERE rather than let it run unshaped and then fail its
        park gate with a message pointing at the gap. `gen-soak-compose.sh:144` states the same
        rule for the soak's prologue — loud, never silently unshaped.

        Like `docker_update_cpus`, this mutation OUTLIVES a Python exception in a way teardown
        does not undo mid-case, so its caller brackets it in a `finally` (see `netem_clear`)."""
        self.p.run_checked(["docker", "compose", "exec", "-T", service,
                            "tc", "qdisc", "add", "dev", iface, "root",
                            "netem", "delay", f"{int(delay_ms)}ms"],
                           timeout=120, note=note or "netem-delay")

    def netem_clear(self, service: str, iface: str, note="") -> None:
        """`… tc qdisc del dev <iface> root` — remove it (`netem_clear`).

        `run_ok`, not `run_checked`, and that asymmetry with `netem_delay` is the point: this is
        also the CLEANUP path. It runs on the failure path too, where the qdisc may never have
        been added or the container may already be gone, and a raise there would replace the real
        failure with a cleanup error. bash spells the same thing `|| true`."""
        self.p.run_ok(["docker", "compose", "exec", "-T", service,
                       "tc", "qdisc", "del", "dev", iface, "root"],
                      timeout=120, note=note or "netem-clear")

    def cast_send(self, argv_tail, note: str, dry_value="{}") -> dict:
        """A `cast send … --json`, through the RECORDING runner (it mutates the chain, so it is
        choreography). Returns the parsed JSON receipt."""
        argv = ["cast", "send", *[str(a) for a in argv_tail], "--json"]
        r = self.p.run_capture(argv, timeout=180, note=note)
        out = r.stdout if not self.dry else dry_value
        try:
            obj = json.loads(out) if (out or "").strip() else {}
        except ValueError:
            obj = {}
        return obj if isinstance(obj, dict) else {}

    def cast_send_async(self, argv_tail, note: str, dry_value="0xasync") -> str:
        """`cast send --async …` — submit and return the tx HASH without waiting for a receipt.

        No `--json`, deliberately: `--async` prints the bare hash on stdout and
        `case-tx-cascade.sh:103,106` reads it as such. Adding `--json` here would be a different
        command whose output the case would then have to parse differently — and, worse, `--async`
        is load-bearing rather than an optimisation: the node these go to is a FOLLOWER, which
        mines nothing, so a synchronous `cast send` would block until a validator three hops away
        happened to include it and the case would time out inside cast instead of at its own
        assertion."""
        argv = ["cast", "send", "--async", *[str(a) for a in argv_tail]]
        r = self.p.run_capture(argv, timeout=180, note=note)
        return (dry_value if self.dry else r.stdout).strip()

    def cast_rpc_write(self, method: str, *args, rpc_url=None, note="") -> None:
        """`cast rpc --rpc-url <url> <method> [args …]` for a MUTATING method.

        `admin_addTrustedPeer` changes a node's peer policy, so it is choreography and belongs in
        the transcript — the same argv as `cast_rpc`, on the other channel. `run_checked`: bash
        fails the case when it does not return 0 (`case-tx-cascade.sh:75`)."""
        self.p.run_checked(["cast", "rpc", "--rpc-url", rpc_url or self.rpc, method,
                            *[str(a) for a in args]],
                           timeout=120, note=note or f"cast-rpc-{method}")

    # -- the per-case compose overlay ---------------------------------------------
    #
    # `docker compose -f base -f dpos -f <case>.yml …`, the array bash builds once per case. See
    # the module header: this is the ONE place that array becomes a command.

    def export_compose_env(self, name: str, value: str) -> None:
        """bash `export NAME=value` — a value the compose FILE interpolates, inherited by every
        LATER compose command in the case. Order matters: exporting after the `up` that needed it
        leaves the service running on the compose file's `:-0x0…0` default, which is a follower
        pointed at a rollup that is not there."""
        self.compose_env[name] = str(value)

    def overlay_args(self):
        """`["-f", …]` for base + DPoS overlay + this case's extra overlays."""
        return self.profile.compose_args("dpos")

    def _overlay_argv(self, *tail):
        return ["docker", "compose", *self.overlay_args(), *tail]

    def overlay_up(self, *services, note="") -> None:
        """`docker compose <files> up -d <svc …>` — MUST succeed (bash runs it under `set -e`, or
        with an explicit `|| { echo FAIL; exit 1; }`)."""
        self.p.run_checked(self._overlay_argv("up", "-d", *services),
                           env_overlay=dict(self.compose_env), timeout=600,
                           note=note or f"overlay-up {' '.join(services)}")

    def overlay_up_ok(self, *services, note="") -> bool:
        """The `2>/dev/null || true` spelling (`case-cert-follow.sh:66`). Only the MITM sidecar
        uses it, and only because its readiness — not its start — is what the case gates on: it
        pip-installs `websockets` at boot, so "started" says nothing and the log probe that
        follows says everything."""
        return self.p.run_ok(self._overlay_argv("up", "-d", *services),
                             env_overlay=dict(self.compose_env), timeout=600,
                             note=note or f"overlay-up-ok {' '.join(services)}")

    def overlay_stop(self, *services, timeout=None, note="") -> None:
        flag = ["--timeout", str(timeout)] if timeout is not None else []
        self.p.run_checked(self._overlay_argv("stop", *flag, *services),
                           env_overlay=dict(self.compose_env), timeout=300,
                           note=note or f"overlay-stop {' '.join(services)}")

    def overlay_start(self, *services, note="") -> None:
        self.p.run_checked(self._overlay_argv("start", *services),
                           env_overlay=dict(self.compose_env), timeout=300,
                           note=note or f"overlay-start {' '.join(services)}")

    def overlay_exec_write(self, service: str, path: str, content: str, note="") -> None:
        """`docker compose <files> exec -T <svc> sh -c "printf '%s' '<content>' > <path>"`
        (`case-tx-cascade.sh:53`) — write a file into a RUNNING overlay container.

        Not `Runner.pipe_to_exec`: that one pipes the content on stdin and hard-codes a
        `/runtime/` prefix, and it goes through the BARE compose project, which cannot resolve an
        overlay service at all. The content here is one enode URL, and it is `printf`-ed inside
        the container exactly as bash does so the transcript carries the value that was written —
        which is the thing to check when L3 comes up peerless."""
        self.p.run_checked(
            self._overlay_argv("exec", "-T", service, "sh", "-c",
                               f"printf '%s' '{content}' > {path}"),
            env_overlay=dict(self.compose_env), timeout=120,
            note=note or f"overlay-write {service}:{path}")

    def overlay_logs(self, *services, tail=None, dry_value="") -> str:
        """`docker compose <files> logs --no-color [--tail=N] <svc …>`, ANSI-STRIPPED, as a READ.

        `--no-color` AND the strip are both unconditional here, and three of the four bash call
        sites have neither (`case-cert-follow.sh:71,90`, `case-tx-cascade.sh:141`). That is §2.4
        item 2 and it is not a style preference: the node writes ANSI escapes INSIDE its
        `key=value` pairs, so an unstripped grep can return no match over a log full of them — a
        silent reader, not a failing one. Every one of these greps decides a verdict, and two of
        them decide it in the direction where "no match" means PASS."""
        tail_flag = [f"--tail={tail}"] if tail is not None else []
        argv = self._overlay_argv("logs", "--no-color", *tail_flag, *services)
        if self.dry:
            self.p.step("read", f"overlay_logs({', '.join(services)}, tail={tail})")
            return dry_value
        return rpc.strip_ansi(proc.read(argv, timeout=nodes.LOGS_ALL_TIMEOUT))

    def overlay_ps_state(self, service: str, dry_value="running") -> str:
        """`docker compose <files> ps --format '{{.State}}' <svc>` (`case-cert-cascade.sh:98`).

        The bogus-checkpoint follower may REFUSE and exit before its refusal line can be read, so
        `exited` is a second, equally valid witness of the same rejection. Without it the case
        would poll a dead container for 240 s and then report that it never refused."""
        return self._delegated("overlay_ps_state", service,
                               lambda: proc.read(self._overlay_argv(
                                   "ps", "--format", "{{.State}}", service)).strip(),
                               dry_value)

    def overlay_dump_logs(self, tail: int, *services) -> None:
        """The fail-path `docker compose <files> logs --tail=N <svc …>`.

        PRINTS what it captured. Recorded (it is part of the failure choreography and its argv is
        evidence), but a dump whose output went nowhere would be a command that looks like a
        diagnostic and is not one."""
        r = self.p.run_capture(self._overlay_argv("logs", f"--tail={tail}", *services),
                               env_overlay=dict(self.compose_env), timeout=180,
                               note=f"overlay-fail-logs {' '.join(services)}")
        if not self.dry and (r.stdout or r.stderr):
            print(r.stdout or r.stderr, flush=True)


# ══ the wrapper ═══════════════════════════════════════════════════════════════════════

def _keep_up() -> bool:
    """`case-vrf.sh:14` — `trap '[ -n "${SMOKE_KEEP_UP:-}" ] || tear_down' EXIT`. Any non-empty
    value keeps the stack, matching bash's `-n` test rather than inventing a `=1` convention."""
    return bool(os.environ.get("SMOKE_KEEP_UP", ""))


class _Exports:
    """bash's `export FOO=…` at the TOP of a case script, before `source lib.sh`.

    Two cases in this chunk TUNE THE GENESIS — `case-cert-catchup.sh:65-66` and
    `case-vrf-dkg-restart-midwindow.sh:61-64` — and the values do not reach the chain through any
    command the case issues. `docker-compose.yml` interpolates `${EPOCH_BLOCK_INTERVAL:-32}` and
    `${DPOS_ACTIVATION_BLOCK:-}`
    into `genesis-init`'s environment, so the value has to be in the AMBIENT environment of the
    `docker compose up --build -d` that `StaticStack` issues — not in a per-call overlay, which
    would reach that one command and not the recreate, and not in `Runner.env`, which the profile
    (reading `EPOCH_INTERVAL` off `os.environ`) would never see.

    So this mutates `os.environ`, as bash does, and RESTORES it on the way out. The restore is the
    one deviation and it is invisible: bash's process exits at that point, so nothing observes the
    variable afterwards there either. What it buys is that a case cannot leave a tuned genesis
    behind for whatever runs next in the same interpreter — the harness's own test suite included.
    """

    def __init__(self, values: dict):
        self.values = {k: str(v) for k, v in (values or {}).items()}
        self.saved = {}

    def apply(self, runner) -> None:
        for k, v in self.values.items():
            self.saved[k] = os.environ.get(k)
            os.environ[k] = v
            runner.step("export", f"{k}={v}")

    def restore(self) -> None:
        for k, old in self.saved.items():
            if old is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = old


def run(case: str, assertions, argv=None, converge_exclude=None, honours_keep_up=False,
        overlays=None, exports=None) -> int:
    """Bring up the static DPoS stack, run `assertions` in order, tear down. 0 pass, 1 fail.

    `assertions` is a list of `fn(ctx)`, run in order. Fail-fast: the first `SmokeFailure` ends the
    run and the rest do not execute.

    `honours_keep_up` is per-case on purpose. ONE of the five bash wrappers reads `SMOKE_KEEP_UP`
    (`case-vrf.sh:14`) and the other four tear down unconditionally; widening it here would be a
    behaviour change wearing the clothes of a tidy-up, and the four unconditional teardowns are
    what keeps one case's leftover stack out of the next case's bring-up.

    `overlays` is the case's extra compose files — the file array bash keeps in a per-case
    variable. Handed to the PROFILE, which is the single home for the list (see `SmokeCtx`'s
    overlay seam); passing it here rather than through `DPOS_EXTRA_COMPOSE` keeps a case's stack
    shape in the case instead of in the environment it happened to be launched with.

    `exports` is the case's genesis TUNING — see `_Exports`. Applied BEFORE the profile is built,
    because the profile reads `EPOCH_INTERVAL` / `DPOS_ACTIVATION_BLOCK` out of the environment
    and a tuned interval that reached compose but not the profile would give the case a stack on
    64-block epochs and epoch arithmetic on 32-block ones.
    """
    argv = list(argv or [])
    dry = "--dry-run" in argv
    unknown = [a for a in argv if a != "--dry-run"]
    if unknown:
        print(f"{case}: unrecognised argument(s) {unknown} (only --dry-run is accepted)",
              flush=True)
        return 2

    runner = Runner(dry=dry, echo=dry)
    env = _Exports(exports)
    env.apply(runner)
    try:
        return _run(case, assertions, runner, dry, converge_exclude, honours_keep_up, overlays)
    finally:
        env.restore()


def _run(case, assertions, runner, dry, converge_exclude, honours_keep_up, overlays) -> int:
    profile = StaticProfile.from_env(extra_overlays=tuple(overlays) if overlays else None)
    stack = StaticStack(profile=profile, runner=runner, converge_exclude=converge_exclude)
    ctx = SmokeCtx(stack)

    if dry:
        print(f"# dpos_harness {case} --dry-run (profile={profile.name}, "
              f"interval={profile.epoch_interval}, activation={profile.activation_block}, "
              f"overlays={list(profile.extra_overlays)})")

    try:
        stack.bring_up_dpos()
    except (ConvergeError, ProcError) as e:
        print(f"FAIL ({case}): bring-up failed: {e}", flush=True)
        return 1

    rc = 0
    try:
        for fn in assertions:
            fn(ctx)
    except SmokeFailure as e:
        print(f"FAIL ({e.case}): {e.message}", flush=True)
        rc = 1
    except (ConvergeError, ProcError) as e:
        print(f"FAIL ({case}): {e}", flush=True)
        rc = 1
    except KeyboardInterrupt:
        print(f"{case}: interrupted", flush=True)
        rc = 130
    finally:
        if honours_keep_up and _keep_up():
            print(f"{case}: SMOKE_KEEP_UP set — leaving the stack up", flush=True)
        else:
            stack.tear_down()

    if dry:
        print(f"# {len(runner.log)} commands")
    return rc
