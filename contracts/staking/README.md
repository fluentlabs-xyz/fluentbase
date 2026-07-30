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

Registered validator identities are permanent. Governance may disable and reactivate validators,
but disabling never deletes their records, consensus-key state, ownership mappings, or stake
history.

## Accounting Invariants

- Stake and commission changes take effect through epoch snapshots; selection changes become visible
  in the following epoch.
- Delegation amounts must use `BALANCE_COMPACT_PRECISION`.
- Undelegated principal is released only after its maturity epoch and is claimed through the reward
  path.
- Claims, stipend catch-up, committee pruning, and jail scans are bounded per call.
- BLEND transfers accept ERC-20 tokens that return `true` or no data; explicit `false` reverts.
- Reserve settlement credits rewards only after the exact assigned amount is disbursed. A successful
  zero or partial disbursement skips the epoch with zero credited rewards and advances the cursor;
  reverted calls and malformed return values revert settlement and remain retryable.
- Equivocation tombstones are permanent and prevent key reuse or jail release.
- Equivocation reporter rewards use a beneficiary-owned commit/reveal flow; the transaction sender
  that reveals evidence is never used as the reward recipient.
- A validator's `owner` is its immutable administrative, validator-fee, self-stake, and slashing
  identity. `changeValidatorOwner` remains in the compatibility ABI but always reverts with
  `ValidatorOwnerImmutable()`.

## Equivocation reporting

Reporting remains permissionless, but requires two transactions so an observer cannot copy public
evidence from the mempool and redirect the reporter reward:

1. The reward beneficiary computes
   `keccak256(abi.encode(domainHash, chainId, staking, proofKind, keccak256(evidence), beneficiary, salt))`,
   where `domainHash = keccak256("FluentStakingEquivocationReportV1")` and proof kinds are
   `0 = notarize`, `1 = finalize`, and `2 = nullify-finalize`. The
   `computeEquivocationReportCommitment` view returns this value without duplicating the encoding.
   The salt must be an unpredictable 32-byte value and must remain private until reveal.
2. The beneficiary sends `commitEquivocationReport(bytes32)`. One active commitment is stored per
   beneficiary, so a later commit from that beneficiary replaces its earlier one.
3. In a later block, any account may call the matching `slashEquivocation*` method with the four
   evidence values plus the beneficiary and salt. A copied reveal still resolves and pays the
   original beneficiary; changing the beneficiary, proof kind, evidence, or salt no longer matches
   the prior commitment.
4. A successful slash consumes the commitment and permanently tombstones the validator. Failed
   evidence verification leaves the commitment available for retry.

The repository does not contain a node-side equivocation submitter. Integrations only need an
ordinary beneficiary account for the commit transaction; the reveal may be sent by any funded
account after observing that the commit is included in an earlier block.

## Solidity parity

The Solidity staking source is not checked into this repository. Its mutable-owner self-stake lookup
remains affected and must also disable validator ownership changes before it is deployed or used as
the canonical implementation.

## Event ABI audit

The SDK event derive encodes non-indexed fields as Solidity function arguments. This is the canonical
event-data shape: the tuple head starts at byte zero, without the extra outer offset used when a
dynamic tuple is encoded as a standalone value.

The repository-wide `#[derive(Event)]` audit found four events whose emitted data changes:

| Contract | Event | Dynamic non-indexed fields | Byte-shape change |
| --- | --- | --- | --- |
| staking | `ConsensusKeysSet(address,bytes,bytes32,uint64)` | `bytes blsPubkey` | Drops the standalone tuple's leading outer-offset word; the first data word is now the `bytes` offset (`0x60`). |
| staking | `EpochCommitteeCommitted(uint64,address[])` | `address[] committee` | Drops the leading outer-offset word; event data begins with the array offset (`0x20`). |
| runtime-upgrade | `RuntimeUpgraded(address,bytes32,string,bytes32)` | `string genesisVersion` | Drops the leading outer-offset word; the event-data head contains the string offset and `codeHash` directly. |
| runtime-upgrade | `UpgradePlanned(bytes32,string,address[],bytes32[],address)` | `string genesisVersion`, `address[] targetAddresses`, `bytes32[] wasmCodeHashes` | Drops the leading outer-offset word; all three dynamic offsets are now relative to the event-data tuple head at byte zero. |

All other repository events contain only static non-indexed fields (or no non-indexed fields), so
their emitted data bytes are unchanged.

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
