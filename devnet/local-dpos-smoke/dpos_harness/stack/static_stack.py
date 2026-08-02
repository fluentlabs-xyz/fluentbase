"""static_stack.py — the STATIC devnet's lifecycle (`lib.sh` cluster B).

Python port of `bring_up_dpos` (lib.sh:492-498), `_migrate_to_dpos` (:409-471), `tear_down`
(:500) and the two converge readers (`_read_sequencer_nodes` :167, `_read_dpos_nodes` :475)
that sit between them. This is the base the 26 smoke cases stand on: 23 of them call
`bring_up_dpos` and all 26 call `tear_down`, so its fidelity matters more than any one case's.

The choreography, in the order that makes it correct:

    1. `docker compose up --build -d`            — phase A, the bare sequencer chain
    2. converge all five nodes                    — finalized aligned, > 0
    3. wait until finalized >= the ACTIVATION BLOCK, so the swap anchor lands in relative
       epoch 0 — see `_wait_activation`
    4. graceful-stop ALL FOUR validators at once, then verify each really persisted
    5. reject an anchor outside relative epoch 0
    6. recreate the four validators under the DPoS overlay + the per-case extra overlay
    7. converge again, strictly PAST the anchor

Steps 3-6 are `_migrate_to_dpos`, and it is the SINGLE point that honours the per-case extra
compose overlay (`StaticProfile.extra_overlays`). Twenty-three cases inherit that through
`bring_up_dpos` without knowing it exists, so the ordering above is not a suggestion: an
overlay applied before the flush would be recreated away, and a recreate before the flush
would lose the anchor block.

WHY THE FLUSH GATE IS A RETRY LOOP AND NOT AN ASSERT: the anchor block only exists on disk if
every validator's reth persisted its in-memory tail. If one did not, bringing the sequencer
back UP and letting it reconverge gives the laggard another chance to re-obtain the anchor —
four attempts, then fail loud. The loop is why step 5 exists at all: a reconverge can advance
the sequencer PAST relative epoch 0, and a cold start outside that window fails loud in the
node itself, so the harness rejects it here with an actionable message instead.

COMPOSE RESOLUTION IS ASYMMETRIC ON PURPOSE — see `stack/profiles.py`. This module never
exports `COMPOSE_FILE`: phase A, `stop`, `start`, `logs` and `down` are all BARE `docker
compose` calls resolving `docker-compose.yml`, and only the DPoS recreate names its files with
explicit `-f` flags (lib.sh:469). `--remove-orphans` on teardown is what still reaps the
containers a per-case overlay added, which is why the bare `down` is sufficient.
"""

from __future__ import annotations

import os

from . import dataroot
from .profiles import StaticProfile
from ..core import converge, lifecycle, nodes, proc, topology
from ..core.converge import ConvergeError

#: `lib.sh:494` / `:447` — phase-A and flush-retry converge budget.
SEQUENCER_CONVERGE_S = 90
#: `lib.sh:496` — post-swap converge budget. The DPoS cold start at a high anchor needs longer
#: than the sequencer, which converges in seconds.
DPOS_CONVERGE_S = 120
#: `lib.sh:428` — how long the sequencer gets to finalize up to the activation block.
ACTIVATION_WAIT_S = 180
#: `lib.sh:434` — flush attempts. The 4th does not retry; it reports.
FLUSH_ATTEMPTS = 4
#: `lib.sh:436` — graceful stop timeout. Long enough for reth's `on_graceful_shutdown` to
#: persist the in-memory tail to MDBX; a SIGKILL at the ceiling exits 137 instead of 0 and the
#: flush barrier sees it.
STOP_TIMEOUT_S = 40

#: `docker compose logs --tail=N` depths at the three fail-loud sites (lib.sh:430/447, :494,
#: :496). Kept distinct because they are: the post-swap failure needs more history than a
#: phase-A one, and flattening them would quietly shorten a diagnostic.
LOG_TAIL_SEQUENCER = 120
LOG_TAIL_DPOS = 200


class StaticStack:
    """Drives the static compose pair through the sequencer→DPoS migration.

    Construct with a `StaticProfile` (which carries the per-case overlay and the epoch
    arithmetic) and a `proc.Runner`. `bring_up_dpos()` leaves the stack UP, as bash does —
    every case then does its work and calls `tear_down()`.

    `converge_exclude` drops one validator from the post-swap alignment set. It exists for
    `case-byzantine`, whose validator-3 runs a byzantine build and whose reth therefore never
    finalizes: the honest quorum must align instead, and requiring all five would fail a case
    that is behaving exactly as designed. It defaults from `DPOS_CONVERGE_EXCLUDE` so the bash
    export keeps working.
    """

    def __init__(self, profile: StaticProfile = None, runner=None, converge_exclude=None):
        self.profile = profile if profile is not None else StaticProfile.from_env()
        self.p = runner if runner is not None else proc.Runner()
        self.converge_exclude = (os.environ.get("DPOS_CONVERGE_EXCLUDE", "")
                                 if converge_exclude is None else converge_exclude)
        # `PREV_FIN` (lib.sh:408): the migration anchor height, HEX, and the floor every
        # post-swap alignment gate must finalize strictly past. Starts as the literal string
        # "null" — NOT 0 and not "" — because `converge.hex_floor` treats a non-hex floor as
        # "no floor", and coercing this to 0 would silently turn the anchor guard into
        # `head > 0`, which every live chain satisfies.
        self.prev_fin = "null"

    # -- reads / converge seams (no-op under dry) ------------------------------------
    def _dry(self) -> bool:
        return bool(self.p.dry)

    def read_sequencer_nodes(self):
        """`_read_sequencer_nodes` (lib.sh:167-173): the four genesis validators + the
        full-node, as `(label, "height|hash")` pairs in bash's order. validator-0 and the
        full-node publish a host RPC; validator-1..3 are reached by `docker compose exec`.

        The host-vs-exec choice comes from `topology.HOST_RPC_PORTS` — the same map the compose
        files publish from — so a reader cannot aim at a port the compose file does not expose.
        """
        out = [self._read_host(topology.PINNED_RPC_HOST)]
        for i in range(1, 4):
            out.append((topology.validator(i), nodes.check_node(topology.validator(i))))
        out.append(self._read_host(topology.FULL_NODE))
        return out

    def read_dpos_nodes(self):
        """`_read_dpos_nodes` (lib.sh:475-482): the honest-set reader. Same five nodes, minus
        `converge_exclude`. The full-node is never excluded — it follows the honest quorum's
        certs and is the check that the cascade tier saw the same chain."""
        excl = self.converge_exclude
        out = []
        if excl != topology.PINNED_RPC_HOST:
            out.append(self._read_host(topology.PINNED_RPC_HOST))
        for i in range(1, 4):
            svc = topology.validator(i)
            if excl != svc:
                out.append((svc, nodes.check_node(svc)))
        out.append(self._read_host(topology.FULL_NODE))
        return out

    @staticmethod
    def _read_host(service: str):
        port = topology.host_rpc_port(service)
        return (f"{service}@{port}", nodes.check_external(port))

    def _wait_aligned(self, timeout, floor, reader, what: str):
        """`_wait_aligned` + the caller's fail-loud (lib.sh:172-205 and its call sites).

        Returns the aligned `"height|hash"`. On expiry it prints the per-node readings, dumps
        the container logs, TEARS THE STACK DOWN and raises — the deliberate re-wiring of bash's
        `exit 1`, which fired the case's `trap tear_down EXIT`. A bare Python `raise` only
        unwinds the stack and would leak a running devnet, so the teardown is explicit here.
        Dry → True (the transcript is about the WRITE commands; no reads, no sleeps).
        """
        if self._dry():
            return True
        reading, last = converge.wait_aligned(timeout, floor, reader)
        if reading is not None:
            return reading
        detail = converge.divergence_detail(last)
        floor_msg = f" past anchor {floor}" if converge.hex_floor(floor) is not None else ""
        print(f"FAIL: {what} did not converge{floor_msg} within {timeout}s; last readings: "
              f"{detail}", flush=True)
        tail = LOG_TAIL_DPOS if floor_msg else LOG_TAIL_SEQUENCER
        self.dump_logs(tail)
        self.tear_down()
        raise ConvergeError(what, f"did not converge{floor_msg} within {timeout}s ({detail})")

    def wait_converge(self, timeout=SEQUENCER_CONVERGE_S):
        """`wait_converge` (lib.sh:218): all five nodes past genesis and on the producer's chain
        at their own heights, no floor."""
        return self._wait_aligned(timeout, "", self.read_sequencer_nodes, "phase1-sequencer")

    def wait_dpos_converge(self, timeout=DPOS_CONVERGE_S):
        """`wait_dpos_converge` (lib.sh:487): the honest set aligned strictly past the anchor."""
        return self._wait_aligned(timeout, self.prev_fin, self.read_dpos_nodes, "dpos-chain")

    # -- the two graceful-stop barriers ---------------------------------------------
    def shutdown_flushed(self, service: str) -> bool:
        """`shutdown_flushed` (lib.sh:66-84) — poll `docker inspect` until the container is
        observed exited 0. Reads container METADATA, never logs: the log read raced under
        daemon load and false-reported "flush incomplete" for a node that exited 0 in under a
        second. Dry → True."""
        if self._dry():
            return True
        return lifecycle.shutdown_flushed(service, self._read)

    def marshal_ack_tripwire(self, service: str, since: str) -> str:
        """`marshal_ack_tripwire` (lib.sh:118-138) — the marshal ack-death witness, scoped to
        graceful, unhalted stops. Returns the verdict (`clean` / `scoped-out` / `tripped`) and
        prints the finding on a trip; the CALLER decides whether a trip fails the case, exactly
        as bash's `|| fail` sites do.

        `since` is the UTC RFC3339 cursor captured IMMEDIATELY BEFORE the stop
        (`lifecycle.log_cursor`), so the grep runs only over THIS stop's slice.

        `_migrate_to_dpos` does NOT call this — bash does not either, and adding it would make
        the migration fail on a chain that is merely mid-halt. It is here because three cases
        call the tripwire directly around their own stops.
        """
        if self._dry():
            return lifecycle.CLEAN
        logs = lifecycle.ack_slice(service, since, nodes.logs_since)
        verdict = lifecycle.ack_verdict(logs)
        if verdict == lifecycle.TRIPPED:
            print(lifecycle.ack_tripwire_message(service), flush=True)
        return verdict

    def log_cursor(self) -> str:
        """The pre-stop cursor for `marshal_ack_tripwire`. Re-exported here so a case never has
        to reach into `core` for it, and never invents its own timestamp format (docker
        compares it against DAEMON-side host timestamps — UTC RFC3339 to the second)."""
        return lifecycle.log_cursor()

    # -- lifecycle -------------------------------------------------------------------
    def bring_up_dpos(self):
        """`bring_up_dpos` (lib.sh:492-498). One-shot: bring up the sequencer-era stack,
        converge, migrate to DPoS, and wait until the DPoS chain is live past the anchor.
        LEAVES THE STACK UP — every case is self-contained and tears down itself."""
        self.profile.prepare(self.p)
        self.p.run_checked(["docker", "compose", "up", "--build", "-d"],
                           timeout=1800, note="phase1-up")
        self.wait_converge(SEQUENCER_CONVERGE_S)
        self.migrate_to_dpos()
        self.wait_dpos_converge(DPOS_CONVERGE_S)
        print(f"DPoS stack live (anchor={self.prev_fin})", flush=True)
        return self.prev_fin

    def migrate_to_dpos(self):
        """`_migrate_to_dpos` (lib.sh:409-471). Sets `self.prev_fin` (the anchor, hex).

        The sole honouring point of the per-case extra overlay. Four ordered obligations:
        wait to the activation block, graceful-stop all four validators, verify each flushed,
        recreate under the overlay. Getting any of them out of order breaks 23 cases at once.
        """
        self._wait_activation()
        anchor = self._flush_gate()
        self.prev_fin = anchor.split("|", 1)[0]
        self._assert_anchor_in_epoch_zero()
        # No migration-anchor flags: the nodes read `dposActivationBlock` from the contract and
        # the restart-vs-fresh state from the consensus-archive discriminator. The overlay list
        # is named EXPLICITLY here (bash `-f … -f … $DPOS_EXTRA_COMPOSE`) rather than exported,
        # so a case's overlay applies to this recreate and to nothing else.
        self.p.run_checked(["docker", "compose", *self.profile.compose_args("dpos"),
                            "up", "-d", "--force-recreate", *self.profile.committee()],
                           timeout=600, note="dpos-recreate")
        return self.prev_fin

    def tear_down(self) -> None:
        """`tear_down` (lib.sh:500). Best-effort by contract — a teardown must never turn a
        passing case into a failing one, so both halves swallow their failures (bash
        `2>/dev/null || true` and `soak_data_root_wipe || true`).

        `--remove-orphans` is load-bearing, not hygiene: the `down` is BARE, so it resolves
        only `docker-compose.yml`, and every container a per-case overlay added (cert-follower,
        sentry, the mitm proxy) is an orphan of that project. Without the flag they survive the
        teardown and the next case inherits them.
        """
        self.p.run_ok(["docker", "compose", "down", "-v", "--remove-orphans"],
                      timeout=300, note="teardown-down")
        dataroot.data_root_wipe(self.p)

    # -- migration sub-steps ----------------------------------------------------------
    def _wait_activation(self) -> None:
        """Wait until the sequencer finalizes >= the activation block, so the swap anchor lands
        in RELATIVE EPOCH 0 (lib.sh:416-433).

        Both bounds are hard node behaviour, not margin: below `dposActivationBlock`
        `OriginEpocher` rejects the cold-start height, and at or above
        `activation + interval` the cold-start epoch would be >= 1 and re-hit the empty-marshal
        genesis lookup. This waits for the lower bound; `_assert_anchor_in_epoch_zero` enforces
        the upper one after the flush retries, which are what can overshoot it.
        """
        act = self.profile.activation_block
        print(f"waiting for the sequencer to finalize >= dposActivationBlock={act} "
              "(relative epoch 0)", flush=True)
        if self._dry():
            return
        if converge.wait_finalized_ge(act, ACTIVATION_WAIT_S):
            # Re-read the height purely for the message. Bash has it in hand from its own poll
            # (`$_fin_dec`, lib.sh:424) and printing it is what makes the line useful — how far
            # past activation the anchor will sit is the first thing anyone asks when the
            # epoch-0 window check below rejects it.
            print(f"  sequencer finalized {converge.finalized_dec_pinned()} >= activation "
                  f"{act}; proceeding to swap", flush=True)
            return
        print(f"FAIL: sequencer did not reach dposActivationBlock={act} within "
              f"{ACTIVATION_WAIT_S}s", flush=True)
        self.dump_logs(LOG_TAIL_SEQUENCER)
        self.tear_down()
        raise ConvergeError("activation-wait",
                            f"sequencer did not reach dposActivationBlock={act} within "
                            f"{ACTIVATION_WAIT_S}s")

    def _flush_gate(self) -> str:
        """Graceful-stop the committee and verify every node persisted. Returns the anchor
        reading (`"height|hash"`) captured immediately BEFORE the stop that succeeded.

        The anchor is read inside the loop, before each stop, because a flush-retry brings the
        sequencer back up and advances the chain — the anchor has to be the head of the attempt
        that actually stopped cleanly, not of the first one.
        """
        vals = self.profile.committee()
        anchor = "null|null"
        for attempt in range(1, FLUSH_ATTEMPTS + 1):
            anchor = "0x0|0x0" if self._dry() else nodes.check_external(topology.HOST_RPC_PORT)
            # `run_checked`, not `run_ok`: bash runs these under `set -e`, so a failed stop /
            # restart ABORTS the case (firing its `trap tear_down EXIT`). A Python `raise` only
            # unwinds, so the case layer owns the `finally: tear_down()` — that re-wiring is
            # deliberate and is the one thing a `trap` gave us for free.
            self.p.run_checked(
                ["docker", "compose", "stop", "--timeout", str(STOP_TIMEOUT_S), *vals],
                timeout=300, note="graceful-stop-committee")
            unflushed = [v for v in vals if not self.shutdown_flushed(v)]
            for v in unflushed:
                print(f"  flush incomplete: {v} (attempt {attempt})", flush=True)
            if not unflushed:
                print(f"all validators flushed persistence on shutdown (attempt {attempt})",
                      flush=True)
                return anchor
            if attempt == FLUSH_ATTEMPTS:
                break
            print("  bringing the sequencer network back up to reconverge before retry",
                  flush=True)
            self.p.run_checked(["docker", "compose", "start", *vals], timeout=300,
                               note="flush-retry-start")
            self.wait_converge(SEQUENCER_CONVERGE_S)
        print(f"FAIL: validators did not flush after {FLUSH_ATTEMPTS} attempts — anchor block "
              "would be missing", flush=True)
        self.tear_down()
        raise ConvergeError("flush-gate",
                            f"validators did not flush after {FLUSH_ATTEMPTS} attempts — the "
                            "anchor block would be missing from disk")

    def _assert_anchor_in_epoch_zero(self) -> None:
        """Reject an anchor outside `[activation, activation + interval)` (lib.sh:460-465).

        The nodes self-discover the anchor from `ChainConfig.dposActivationBlock` and anchor the
        consensus genesis there; the fresh-migration cold start fails loud unless reth's
        finalized tip sits in relative epoch 0. A flush-retry that reconverged the sequencer
        past the window would trip that fail-loud inside the node, so it is rejected here with
        an actionable message instead of an opaque node abort.
        """
        if self._dry():
            return
        lo, hi = self.profile.epoch_zero_window()
        anchor_dec = nodes.hex_to_dec(self.prev_fin)
        if lo <= anchor_dec < hi:
            return
        msg = (f"sequencer finalized {anchor_dec} outside relative epoch 0 [{lo}, {hi}) — a "
               "flush-retry likely advanced the sequencer past the window. Re-run, or widen "
               "the window (raise EPOCH_INTERVAL).")
        print(f"FAIL: {msg}", flush=True)
        self.tear_down()
        raise ConvergeError("anchor-window", msg)

    # -- helpers ----------------------------------------------------------------------
    def dump_logs(self, tail: int, *services) -> None:
        """`docker compose logs --tail=N [services]` on a fail-loud path. BARE compose, like
        bash — the per-case overlay's containers are not in this project's config, and a case
        that wants their logs names its own files.

        PRINTS what it captured, matching `SmokeCtx.overlay_dump_logs` (cases/smoke/driver.py):
        `run_ok` only reports rc==0 and discards stdout/stderr, which made every fail-path log
        dump in the ported cases show nothing where bash's `docker compose logs` wrote straight
        to the terminal."""
        if self._dry():
            return
        r = self.p.run_capture(["docker", "compose", "logs", f"--tail={tail}", *services],
                               timeout=120, note="fail-logs")
        if r.stdout or r.stderr:
            print(r.stdout or r.stderr, flush=True)

    def _read(self, argv) -> str:
        """The read seam handed to `core/lifecycle` — the AMBIENT (non-recording) channel, so a
        barrier's `docker inspect` polls never enter the write transcript."""
        return proc.read(argv)
