# DPoS Local Smoke (Pipeline 2 — sequencer→DPoS migration mirror)

Two-phase smoke test that mirrors the production migration from a
single sequencer to a 4-validator DPoS BFT set on isolated
`chainId=2026`, deterministic from a BIP39 mnemonic:

- **phase-1** (`make smoke`): validator-0 runs as the sequencer
  (1 block / sec); validators 1-3 and a non-staking full-node follow
  via `--cert-upstream ws://172.20.0.10:8546` (deprecated alias
  `--sequencer-url`). All 5 align finalized
  > 0 within 60 s.
- **phase-2** (`make smoke-swap`): cold-restart validators 0-3 with
  `--dpos` via `docker-compose.dpos.yml` override; chain continues
  past the sequencer's last block via DPoS BFT. All 5 align finalized
  > sequencer_last within 60 s.

Every node passes `--dpos.staking-config=/runtime/staking-reader.json`
in both phases — required so `FluentBlockExecutor::apply_pre_execution_changes`
runs the `commitEpochCommittee` system call at epoch boundaries
identically on every executor (otherwise followers compute a
divergent state-root and reject the sequencer's blocks). This is the same
constraint prod will face during migration.

## Prerequisites

- Docker + docker-compose v2 (`docker compose` subcommand)
- `make`, `jq`, `curl` on host
- Sibling checkout of `fluentlabs-xyz/solidity-contracts` at
  `../../../solidity-contracts/` (or set `SOLIDITY_CONTRACTS_DIR` env)
- Foundry (`forge`) on host — only for `make regen-contracts`
  (genesis-init container does not need it at run time)

## Quick start

    make regen-contracts        # one-time, after a Solidity change
    make smoke                  # phase-1: sequencer + followers; leaves chain UP
    make smoke-swap             # phase-2: cold-restart to DPoS; tears down on success

For phase-1 only (no migration test) run `make smoke` and clean up
with `make down`. For the full end-to-end migration test run both
sequentially.

For interactive observation:

    make up                     # foreground; ^C to stop
    make logs                   # follow logs of all 5 services
    curl -s -X POST -H 'Content-Type: application/json' \
      --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
      http://localhost:8545    # validator-0 block height
    curl ... http://localhost:18545   # full-node block height

## Acceptance check

**phase-1** (`make smoke`) succeeds when within 60 s of `docker
compose up`:

- all 5 nodes' `eth_getBlockByNumber("finalized", false).result.number > 0`
- all 5 nodes' `eth_getBlockByNumber("finalized", false).result.hash` are identical

Chain stays UP on success so phase-2 can take over via compose
override.

**phase-2** (`make smoke-swap`) succeeds when within 60 s of the
cold-restart:

- all 5 nodes' finalized number > the sequencer's last finalized number (chain
  visibly advanced post-swap, not stuck at the swap boundary)
- all 5 nodes' finalized hash identical

On either failure: container logs dumped, `docker compose down -v`
cleans up.

## Teardown

    make down                   # `docker compose down -v` — removes
                                # the `runtime` volume so the next
                                # `up` regenerates a fresh chain

## Troubleshooting

- **`SOLIDITY_CONTRACTS_DIR not found`** — clone the
  `fluentlabs-xyz/solidity-contracts` repo as a sibling of this one,
  or pass `SOLIDITY_CONTRACTS_DIR=/path/to/repo make regen-contracts`.
- **Port 8545 / 18545 already in use** — another `fluent` or `anvil`
  instance is running. `docker compose down -v` here + `lsof -i:8545`
  to find the conflicting process.
- **Genesis-init fails with "invalid BIP39 mnemonic"** — only happens
  if `FLUENT_DPOS_MNEMONIC` is overridden with a malformed phrase.
  Unset the env var to use the default deterministic mnemonic
  (foundry/hardhat-canonical "test test ... junk").
- **validator-0 producing blocks but followers stuck at block 0** —
  one (or more) of validators 1-3 / full-node missing
  `--dpos.staking-config`. State-root mismatch on
  `commitEpochCommittee` system call causes followers to reject
  the sequencer's blocks. All 5 nodes must pass identical
  `--dpos.staking-config` in both phases.
- **phase-2 `make smoke-swap` hangs at PREV_FIN** — the cold-restart
  happened but DPoS BFT didn't make a block past the sequencer's last
  finalized. `docker compose logs validator-0` shows DPoS engine
  state; common cause is a swap fired past the first epoch boundary
  (block ≥ 32) without prior `commitEpochCommittee` for the new
  epoch. Keep swap within epoch 0 in smoke (default `epoch_block_interval = 32`).
- **Contract artefacts stale after Solidity change** — run
  `make regen-contracts` and commit the new JSONs in `contracts/`.

## Network topology (reth devp2p peering)

Under `--dpos`, reth devp2p is the EL-block transport for rejoin/catch-up
(a restarted validator FCU-drives its head toward the consensus tip and
reth bulk-downloads the gap over eth/68 from its trusted peer). In this
smoke, validators 1-3 statically pin validator-0's enode
(`--trusted-peers="$(cat /runtime/v0-enode.txt)" --trusted-only
--disable-discovery`) and validator-0 is the hub
(`--p2p-secret-key` + `--port=30303` give it a deterministic enode).

This v0-hub mesh is a **degenerate sentry topology** — fine for a 4-node
loopback smoke, but NOT the production shape. In production, validators
run **behind sentry nodes** (Cosmos/Tendermint canon): the validator's IP
is never gossiped; it connects only to its own sentries via
`--trusted-peers` + `--disable-discovery`, and the sentries are the public
faces that run discovery and absorb DoS. Network identity is operator
config, off-chain — the on-chain `ConsensusKeys` carry no IP/enode. See
`~/.claude/standards/general.md` ("DPoS/BFT validator networking").

## Production-path smoke (`make smoke-production-path`)

The full prod lifecycle on a chain where the staking cluster is deployed at
**runtime via forge** (not baked into genesis):

1. 6 nodes + a full node boot a **bare** chain (no staking predeploys) — a
   plain sequencer (validator-0) + WS followers. Every node carries
   `--dpos.staking-config` from first boot: `genesis-init` pre-writes
   `staking-reader.json` predicting the runtime CREATE addresses from deployer
   nonces (`--staking-reader-create-nonces`, see the compose comment), so all
   nodes execute the `commitEpochCommittee` system call identically from
   block 1.
2. The host driver deploys `MockBlendToken` + `BLS12381Verifier` (`forge
   create`) and the staking cluster (`forge script DeployStaking`, config
   selected via `NETWORK=local-dpos-smoke/l2`); the driver asserts the deploy
   manifest matches the pre-written `staking-reader.json` (fail-loud on
   deployer-nonce drift).
3. Bootstraps a 5-validator committee: `setBlsVerifier` (governance) **before**
   `setConsensusKeys` (the PoP is verified against the on-chain verifier), then
   `setDposActivationBlock` (governance).
4. The sequencer's **dynamic activation gate** (per-tick on-chain re-read)
   clean-halts sequencer production at exactly `dposActivationBlock` — no
   mid-flight restart, so the followers ride the uninterrupted WS stream to
   the same height; once all nodes align, ALL six validators cold-restart into
   `--dpos` (`--dpos.follower-upstream` set): committee members cold-start as
   signers, while validator-5 (no committee seat yet) stays a verify-only node
   that follows the chain via its cert-inlet.
5. Registers the **external 6th** validator (`registerValidator` →
   `setConsensusKeys` → governance `activateValidator` → `delegate`) while it
   follows via its inlet; once its key appears in the ahead-committed
   `getEpochCommittee(E+1)` and it holds its DKG share, `reconcile_roles`
   **promotes it to Signer in-process** (no restart) — the case asserts
   convergence past the boundary, the `promoted to Signer in-process` log line,
   the committee rotation, AND that the displaced validator **demotes to
   verify-only in-process** and keeps following its inlet (no silent-verifier
   wedge; watchdog WARN absent from v5's entire log).
6. Ejects one committee validator by **liveness** (stopped at an epoch start so
   50 misses fit one 64-block epoch) — asserts jail, then absence from
   `getEpochCommittee` two boundaries later (committee[E+1] was committed
   pre-jail).
7. A background value-transfer spammer runs throughout; the chain must keep
   finalizing across every transition.

Long (~5-8 min) and first-of-its-kind, so it is **NOT** in `make smoke-all` —
run it explicitly. Uses its own 6-node compose project
(`docker-compose.production-path.yml` + `.production-path.dpos.yml`, chainId 2026)
distinct from the genesis-baked cases. Needs `forge`/`cast`/`jq` and a
`solidity-contracts` checkout at `SOLIDITY_CONTRACTS_DIR`.

## Joining a running chain as a new validator

Boot the node in unified `--dpos` with one or more
`--dpos.follower-upstream ws://<upstream>` URLs (a `consensus`-RPC WebSocket
of any validator or follower). The node's supervisor keeps it on an
in-process cert-follow substrate (verifying every upstream certificate
against the on-chain committee) while its key is outside the committee.
Register + activate the validator and delegate stake (see the
production-path case for the exact calls) — once the key appears in the
ahead-committed `getEpochCommittee(E+1)`, the supervisor stops the follower
at the epoch boundary and promotes to signer in-process. Rotation-out later
demotes it back to the follower substrate the same way. No restarts, no
manual timing.

Without `--dpos.follower-upstream` a non-committee `--dpos` node has zero
consensus-plane connectivity (push dissemination is participant-scoped) and
idles behind the committee watchdog WARN until its committee epoch — run
unified mode instead.

## What this is NOT

- Adversarial scenarios (slashing, view-change, equivocation) —
  separate ticket(s)
- Production deployment — uses deprecated plaintext BLS key path,
  devnet-only Dockerfile (`fluent.image.kind=devnet-smoke` label)
- CI integration — pure developer hand-tool

## Simulation — long-running survivability test (`make smoke-sim`)

A seeded, stochastic, **long-running** DPoS production-path sim (task
`dpos_production_soak_smoke`). ONE unified profile, **beacon ON throughout**, a
**dynamic growing+shrinking committee**, churn interleaved with genuinely clean
epochs under continuous tx load. A **DKG-window-aware safety gate** keeps the
chain live, so any *unexpected* finalize-stall is a real bug → a replayable
failure bundle + clean stop.

```sh
make smoke-sim-quick   # ~5-8 min CI sanity gate (4 nodes, byzantine off)
make smoke-sim         # full run (default 10 validators, ~10-min epochs, unbounded)
make smoke-sim-ram     # same, data root on /dev/shm (tmpfs — spares the SSD)
```

All three run the Python simulation (`python3 -m dpos_harness sim run`). The
safety gate's pure unit test used to have its own `make` target pointed at a bash
script; it is now part of the normal suite —
`uvx --with pyyaml pytest dpos_harness/tests -q`.

Needs foundry (`forge`/`cast`) and the smoke image built with the
`dpos-devnet-byzantine` feature (the Dockerfile enables it).

### Knobs (env)

| Knob | Full | Quick | Meaning |
|---|---|---|---|
| `SIM_DURATION` | `0` (unbounded) | `5m` | wall bound; `0` = run until fail/Ctrl-C |
| `SIM_VALIDATORS` | `10` | `4` | target N the cluster grows toward (≤51) |
| `SIM_INITIAL_COMMITTEE` | `4` | `4` | starting committee; `MIN_COMMITTEE` floor derives from THIS |
| `SIM_EPOCH_INTERVAL` | `600` | `32` | blocks/epoch (runtime ChainConfig + env) |
| `SIM_CHURN_PERIOD` | `90s` | `20s` | mean delay between churn actions |
| `SIM_CHECK_PERIOD` | `10s` | `5s` | invariant battery cadence |
| `SIM_CALM_FRACTION` | `0.4` | `0.4` | fraction of epochs with zero churn |
| `SIM_BYZANTINE` / `SIM_QUORUM_PROBE` | `1` / `1` | `0` / `0` | enable byzantine actions / the quorum-loss probe |
| `SIM_SEED` | fresh, printed | fresh, printed | PRNG seed — set it to replay the intent schedule |
| `SIM_EXEC_SAT_THRESHOLD` / `SIM_EXEC_SAT_TICKS` / `SIM_EXEC_SAT_EARLY` | `0.7` / `3` / `0.4` | same | exec-saturation watch (reported-not-asserted): warn event when the chain-wide mean per-block EL derive+import time (`reth_dpos_derive_el_apply_duration_seconds` Δsum/Δcount over the tick window) exceeds THRESHOLD of the 1s block interval for TICKS consecutive measured ticks; one-time info event on first sighting above EARLY |
| `SIM_RATE_REPORT_TICKS` / `SIM_RATE_WARN` | `20` / `0.5` | same | block-rate watch (reported-not-asserted): periodic info timeline event every N measured ticks with `rate=Δfinalized/Δwall blk/s`; warn when rate < `SIM_RATE_WARN` for 2+ consecutive measured ticks (SLOW-without-stalling — a full stall is hard-asserted by finalize-stall) |
| `SIM_ACTIONS` | – (full pool) | – | space-list override of the fault-lottery pool (changes the PRNG action modulus — replay only against the same override). E.g. `SIM_ACTIONS="graceful_stop_restart"` + `SIM_BYZANTINE=0` + `SIM_VALIDATORS`==`SIM_INITIAL_COMMITTEE` = the B′ rolling-restart-on-a-STABLE-committee validation shape (plan dpos_beacon_seed_witness §9 item 4) |
| `SIM_OUT` / `SIM_KEEP_UP` / `SIM_STOP_ROUND` | `./sim-out` / – / – | | output dir / keep stack on fail / stop before a round |
| `SIM_DATA_ROOT` | `/mnt/storage/fluent-sim` (Makefile default) | same | relocate ALL runtime validator data (the shared `/runtime` volume: reth MDBX + static files, consensus/marshal journals, per-validator keys) off the system disk onto a separate disk via a bind-backed volume at `$SIM_DATA_ROOT/runtime`. Empty = historical docker-managed named volume under `/var/lib/docker`. Must be an absolute path with ≥1 component (`/` is refused). `down -v` cannot clear a bind target, so the harness does a root-owned container wipe of `$SIM_DATA_ROOT/runtime` at bring-up/teardown (fresh state each run). Opt-in in the scripts; the value lives in the `make smoke-sim` target — override on the CLI or `SIM_DATA_ROOT= make smoke-sim` to opt out. `make smoke-sim-ram` = the same run with the root on `/dev/shm/fluent-sim` (tmpfs — the validator data lives in RAM, sparing the SSD; no sudo needed) |
| `SIM_DATA_FILL_WARN_PCT` / `SIM_DATA_FILL_FAIL_PCT` | `75` / `90` | same | data-root fill watchdog (active only when `SIM_DATA_ROOT` is set; essential for the RAM-backed `smoke-sim-ram` root): each battery tick reads the used% of the filesystem backing the data root — ≥ WARN → one rate-limited `data_root_filling` watch event per episode (reported-not-asserted, names pct + absolute free; re-arms below the warn line); ≥ FAIL → hard `data-root-full` fail with honest attribution (tmpfs/disk full — NOT a product failure) BEFORE the ENOSPC node deaths get mis-attributed by the liveness checks |
| `SIM_PRUNE_PROFILE` | `full` | same | reth prune profile emitted on every validator/spare + the L3 `downstream` (NOT `full-node`, which stays archive — it carries `--rpc.eth-proof-window=50000` and is the deep-history keeper). `full` adds `--full`: Fluent activates Paris at block 0 so `--full` keeps ALL bodies (Before(0) = nothing pruned) while pinning account/storage-history changesets at Distance(10064) — safe over every DPoS derive bound (replay≤64, K=3, harness windows) — and pruning receipts(10064)/sender-recovery(Full); the devp2p sync source (validator-0) still serves genesis-deep bodies under `--full`. `downstream`'s `--rpc.eth-proof-window` is lowered to `10000` (< 10064) so it never advertises a wider proof window than the history it retains under `--full`. Set to `archive` to emit no prune flag anywhere (escape hatch, byte-identical to pre-knob behavior) |
| `PEER_HOST_MODE` | `ip` | same | how `peers.json` + each `--dpos.dialable` render a validator's ingress host. `ip` = pinned docker IP `172.20.0.$((10+idx)):9000` (byte-identical to before). `dns` = the docker service-name hostname `validator-$idx:9000`, exercising the validator crate's `ALLOW_DNS` path (network-wide invariant, now `true`). Passed through to `gen-soak-compose.sh` |
| `BOOTSTRAP_MODE` | `json` | same | cold-start peer-discovery source for `--dpos.bootstrappers`. `json` = `/runtime/peers.json` file (byte-identical to before). `dns` stands up a CoreDNS service (`coredns/coredns:1.11.1` at `172.20.0.53`) serving a `pubkey@validator-i:9000` TXT zone (`seed.sim.local`) emitted by `genesis-bootstrap --seed-dns-zone`, points every validator's resolver (`dns:` key) at it, and passes `--dpos.bootstrappers=dns:seed.sim.local`. `peers.json` is still written (harmless). Non-seed DNS queries forward to docker's embedded resolver (127.0.0.11) so service-name lookups keep working; validators do NOT gate on CoreDNS (the node's ~2 min DNS retry covers a late/empty zone — an empty seed set is a valid inbound-only startup) |

### Heavy execution load (`python3 -m dpos_harness load start`)

Standalone SSTORE-heavy load blaster — the load-generation half of the
execution-saturation observability pair (the measurement half is the
`exec-saturation` / `block-rate` soft watches above: aim this at a live sim's
v0 RPC and read the events off `$SIM_OUT/events.jsonl`). Deploys the vendored
`contracts/GasBurner.json` (harness-local fixture, source in
`contracts/GasBurner.sol` — regenerate with `forge build` + `jq`, NOT part of
the sibling `make regen-contracts`), funds `LOAD_SENDERS` ephemeral keys from the
harness's funded dev key, and paces per-sender `burn(n)` sends so
`SENDERS×rate×LOAD_TX_GAS ≈ LOAD_TARGET_GAS_FRACTION×block_gas_limit` per 1 s block.
Burn size is calibrated empirically (two small `eth_estimateGas` probes — big
probes hit the node's rWASM `OutOfFuel` RPC cap). Robust: per-sender
BACKPRESSURE WINDOW (`lh_inflight_gate` — the local nonce derives from the
polled mined head and never runs more than `LOAD_MAX_INFLIGHT` ahead, so the
reth 16-per-account pool cap is never hit), EIP-1559 capped-fee sends
(`LOAD_MAX_FEE`/`LOAD_TIP` — base fee over the cap makes the window WAIT and
auto-mine on decay: no repricing, no fee-bid feedback, no sender bankruptcy),
nonce re-sync on desync, hard `timeout` on every cast RPC call with an
unconditional status heartbeat (`rpc-timeout` markers), per-sender loops never
die on a failed send; SIGTERM/Ctrl-C kills senders cleanly.

    python3 -m dpos_harness load start                        # supervise until stop, defaults below
    LOAD_DURATION=60 LOAD_SENDERS=1 LOAD_TARGET_GAS_FRACTION=0.02 python3 -m dpos_harness load start   # gentle poke
    LOAD_WAIT_MARKER_LOG=sim.log python3 -m dpos_harness load start   # gate funding on the sim's DeployStaking marker
    python3 -m dpos_harness load stop [pidfile]               # clean pidfile-based shutdown

The legacy `scripts/load-heavy.sh` is still on disk until the bash tree is
deleted. It reads the OLD `LH_*` names, not the `LOAD_*` ones below, and greps
the OLD `[soak r<N> ` run marker — so it will not gate on a Python sim's log
without `LH_WAIT_MARKER_RE` set. Use the Python blaster with the Python sim.

**Self-supervising lifecycle** (the sender-lifecycle rework — retires the old
external `launch-load.sh` + `blaster-waiter.sh` scratchpad glue): this ONE
process supervises itself. On start it (1) **acquires** the funder key + RPC
readiness with bounded retry/backoff *inside* the script — every
`LOAD_ACQUIRE_INTERVAL` (15 s), each miss logged with its reason, loud FAIL only
after `LOAD_ACQUIRE_TIMEOUT` (30 min) — so a not-yet-readable key never yields a
silent zero-load run; (2) **gates** all funding behind the sim's first
`[sim r<N> ` marker when `LOAD_WAIT_MARKER_LOG` is set (the DeployStaking-complete
signal — funding before it corrupts the shared dev-EOA nonce), replacing the
external waiter; (3) **supervises** the `LOAD_SENDERS` loops, restarting any that
die with a per-sender restart counter + exponential backoff (flap-reset after a
healthy run). It writes a **pidfile** (`LOAD_PIDFILE`, default beside `LOAD_LOG` or
`$TMPDIR/load-heavy.pid`). **Never `pkill` this process** — the self-match traps
and orphaned `cast` grandchildren are exactly the recurring incident class. Stop
it with `python3 -m dpos_harness load stop [pidfile]` (or a plain SIGTERM to the
supervisor): `stop` TERMs the recorded process group, waits, KILL-9 fallback,
verifies identity by pid+etime (never nukes a recycled PID), and removes the
pidfile.

| Knob | Default | Meaning |
|---|---|---|
| `LOAD_RPC` | `http://localhost:8545` | target RPC (the sim's v0 host port) |
| `LOAD_SENDERS` | `4` | ephemeral sender loops |
| `LOAD_TARGET_GAS_FRACTION` | `0.6` | of block gas limit per 1 s block |
| `LOAD_TX_GAS` | `3000000` | per-tx gas budget (burn(n) fills 90% of it) |
| `LOAD_DURATION` | `0` (until signal) | run bound, seconds |
| `LOAD_MAX_FEE` / `LOAD_TIP` | `5000` / `2` wei | EIP-1559 maxFeePerGas cap / maxPriorityFeePerGas. The self-regulator: effective price = baseFee+tip (never the cap); base fee above the cap ⇒ the pending window WAITS and auto-mines on decay. Replaced the legacy `eth_gasPrice×3` bid, whose refresh loop fed back into its own base-fee climb and bankrupted senders into unaffordable peak-priced windows (down-repricing a legacy tx is impossible — same-nonce replacement must bid ≥1.1×) |
| `LOAD_FUND_WEI` / `LOAD_FUNDER_KEY` | window×tx_gas×max_fee×4 (computed) / validator-0 `funded.hex` | sender funding — a full in-flight window at the worst-case cap price, ×4 margin; scales with the knobs |
| `LOAD_STATUS_EVERY` | `30` | status-line cadence (txs sent, fleet in-flight, basefee/max, sample receipt, head) |
| `LOAD_MAX_INFLIGHT` | `12` | per-sender in-flight (pending−mined) cap, < reth's 16 per-account pool cap; senders self-throttle to drain instead of racing nonces |
| `LOAD_UNWEDGE_SKIPS` | `10` | full-window skips with a frozen mined head between "head-of-line waits" diagnostic logs (base-fee-over-cap self-regulation, or an inherited stuck legacy tx — which is loudly reported at startup as topup/decay-required, never futilely re-sent) |
| `LOAD_WAIT_MARKER_LOG` | unset (no gate) | sim log to gate ALL funding behind its first `[sim r<N> ` marker (DeployStaking-complete). Replaces the external `blaster-waiter.sh` |
| `LOAD_ACQUIRE_TIMEOUT` / `LOAD_ACQUIRE_INTERVAL` | `1800` / `15` s | bounded startup acquisition (funder key + RPC + marker): retry every interval, loud FAIL only after the timeout — never die on the first miss |
| `LOAD_PIDFILE` / `LOAD_LOG` | `$TMPDIR/load-heavy.pid` (or beside `LOAD_LOG`) | supervisor pidfile path / its default anchor. `stop [pidfile]` reads it |
| `LOAD_SUPERVISE_TICK` / `LOAD_RESTART_HEALTHY_SECS` | `5` / `120` s | supervisor loop cadence (dead-sender detection latency) / how long a sender must stay up before its restart counter flap-resets |

### Reading a failure + replaying

The run streams a human mirror to stdout and writes machine-readable
`$SIM_OUT/events.jsonl` (one object per scheduled slot AND per invariant tick;
the seed is on every line). After a failure:

```sh
jq 'select(.kind=="invariant_fail")' sim-out/events.jsonl   # broken invariant + round/block/epoch/committee
jq -r '.seed' sim-out/events.jsonl | head -1                # the seed
SIM_SEED=<that seed> SIM_VALIDATORS=<same> SIM_EPOCH_INTERVAL=<same> make smoke-sim
```

The seed replays the **intent** schedule exactly. The *applied* churn is not
bit-identical on replay — the gate reads live on-chain state, so its accept/skip
verdict can differ run-to-run; `events.jsonl` records intent + verdict + applied
per round so a post-mortem reconstructs precisely what ran (this honesty is by
design — modelling committee state in-script to fake determinism would undermine
the gate's live-safety guarantee). The full diagnostic bundle is at
`sim-out/bundle-<ts>/` (summary, the events timeline, every node's logs, RPC
snapshots, the generated compose + runtime topology).
