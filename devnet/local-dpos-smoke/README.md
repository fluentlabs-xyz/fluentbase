# DPoS Local Smoke (Pipeline 2 — sequencer→DPoS migration mirror)

Two-phase smoke test that mirrors the production migration from a
single Tempo sequencer to a 4-validator DPoS BFT set on isolated
`chainId=2026`, deterministic from a BIP39 mnemonic:

- **phase-1** (`make smoke`): validator-0 runs as a Tempo sequencer
  (1 block / sec); validators 1-3 and a non-staking full-node follow
  via `--sequencer-url ws://172.20.0.10:8546`. All 5 align finalized
  > 0 within 60 s.
- **phase-2** (`make smoke-swap`): cold-restart validators 0-3 with
  `--dpos` via `docker-compose.dpos.yml` override; chain continues
  past Tempo's last block via DPoS BFT. All 5 align finalized
  > tempo_last within 60 s.

Every node passes `--dpos.staking-config=/runtime/staking-reader.json`
in both phases — required so `FluentBlockExecutor::apply_pre_execution_changes`
runs the `commitEpochCommittee` system call at epoch boundaries
identically on every executor (otherwise followers compute a
divergent state-root and reject Tempo's blocks). This is the same
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
    make smoke                  # phase-1: Tempo + followers; leaves chain UP
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

- all 5 nodes' finalized number > Tempo's last finalized number (chain
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
  Tempo's blocks. All 5 nodes must pass identical
  `--dpos.staking-config` in both phases.
- **phase-2 `make smoke-swap` hangs at PREV_FIN** — the cold-restart
  happened but DPoS BFT didn't make a block past Tempo's last
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

## What this is NOT

- Adversarial scenarios (slashing, view-change, equivocation) —
  separate ticket(s)
- Production deployment — uses deprecated plaintext BLS key path,
  devnet-only Dockerfile (`fluent.image.kind=devnet-smoke` label)
- Hot in-process swap (pipeline 3) — requires per-engine Shutdown
  channel + FCU ordering invariants, not implemented; cold-restart
  swap above is the supported migration mechanism today
- CI integration — pure developer hand-tool
