# Staking

The core validator staking contract implemented as a normal rWasm contract and deployed at
`GENESIS_STAKING`. It uses `SharedAPI` for storage, BLEND transfers, logs, and calls to protocol
dependencies.

## Scope

- Implements validator lifecycle, delegation, rewards, committees, liveness jail, and equivocation
  slashing.
- Owns the chain configuration previously read from `ChainConfig`.
- Keeps `StakingPool` external and unchanged; this crate does not deploy or replace it.
- Calls configured BLS verifier, evidence decoder, liveness, and BLEND reserve contracts.

## Lifecycle

1. Genesis governance calls `configure` and `initialize` once each; non-zero initial stake requires
   configuration first.
2. Governance configures external dependencies and manages validator status.
3. Validators register consensus keys; delegators approve and deposit BLEND.
4. The system caller commits epoch committees and settles finalized epoch stipends.
5. Liveness and equivocation paths jail or permanently tombstone validators.

## Accounting Invariants

- Stake and commission changes take effect through epoch snapshots; selection changes become visible
  in the following epoch.
- Delegation amounts must use `BALANCE_COMPACT_PRECISION`.
- Undelegated principal is released only after its maturity epoch and is claimed through the reward
  path.
- Claims, stipend catch-up, committee pruning, and jail scans are bounded per call.
- BLEND transfers accept ERC-20 tokens that return `true` or no data; explicit `false` reverts.
- Reserve settlement credits rewards only after the exact assigned amount is disbursed.
- Equivocation tombstones are permanent and prevent key reuse or jail release.

## Source Layout

- `handlers.rs`: initialization, compatibility getters, and validator administration.
- `economics.rs`: registration, delegation, and undelegation.
- `rewards.rs`: reward claims, redelegation, and stipend settlement.
- `consensus.rs`: consensus keys and epoch committees.
- `liveness.rs` / `equivocation.rs`: jail and slashing paths.
- `storage.rs`: ERC-7201 layout and epoch snapshots.
- `config.rs`: governance-controlled parameters and dependencies.

## Verification

```bash
cargo test --manifest-path contracts/Cargo.toml -p fluentbase-contracts-staking
cargo test -p fluentbase-e2e staking
```
