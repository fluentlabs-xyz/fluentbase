# Staking

The core validator staking contract implemented as a normal rWasm contract and deployed at
`GENESIS_STAKING`. It uses `SharedAPI` for storage, BLEND transfers, logs, and calls to protocol dependencies.

## Scope

- Implements validator lifecycle, delegation, rewards, committees, liveness jail, and equivocation slashing.
- Owns the chain configuration previously read from `ChainConfig`.
- Isolates initializer, chain configuration, consensus, and staking state in separate ERC-7201 namespaces:
  `Fluent.storage.Initializer`, `Fluent.storage.ChainConfig`,
  `Fluent.storage.Consensus`, and `Fluent.storage.StakingStorage`.
- Keeps `StakingPool` external and unchanged; this crate does not deploy or replace it.
- Calls configured BLS verifier, evidence decoder, liveness, and BLEND reserve contracts.

## Lifecycle

1. Deployment atomically installs and initializes staking, chain configuration, and external dependencies before public
   transactions can execute.
2. Governance manages chain configuration and validator status.
3. Validators register consensus keys; delegators approve and deposit BLEND.
4. The system caller commits epoch committees and settles finalized epoch stipends.
5. Liveness and equivocation paths jail or permanently tombstone validators.

Registered validator identities are permanent. Governance may disable and reactivate validators, but disabling never
deletes their records, consensus-key state, ownership mappings, or stake history.
Governance-added validators begin pending with zero stake and cannot become active until their self-stake is effective
at or above the configured validator minimum.

## Accounting Invariants

- Stake and commission changes take effect through epoch snapshots; selection changes become visible in the following
  epoch.
- A newly materialized snapshot copies only the latest state already effective at that epoch. Earlier-effective stake
  and commission changes are carried forward through any scheduled warm-up snapshots, never copied backward from them.
- Initialization, activation, jail readmission, and committee selection each require the validator owner's effective
  self-stake to meet the configured minimum. A full owner exit moves an active validator to pending in the same
  transaction and removes its next-epoch selection visibility.
- Delegation amounts must use `BALANCE_COMPACT_PRECISION`.
- Undelegated principal is released only after its maturity epoch and is claimed through the reward path.
- Validator-owner principal remains custodial through the latest committed committee's evidence window, and equivocation
  seizure consumes both active and pending self-principal.
- Claims, stipend catch-up, committee pruning, and jail scans are bounded per call.
- BLEND transfers accept ERC-20 tokens that return `true` or no data; explicit `false` reverts.
- Reserve settlement credits rewards only after the exact assigned amount is disbursed. A successful zero or partial
  disbursement skips the epoch with zero credited rewards and advances the cursor; reverted calls and malformed return
  values revert settlement and remain retryable.
- Equivocation tombstones are permanent and prevent key reuse or jail release.
- Liveness jailing protects the fixed committed committee for the current epoch (or its selected
  pre-commit fallback); sequential reports cannot ratchet down the quorum floor.
- Equivocation reporter rewards use a beneficiary-owned commit/reveal flow; the transaction sender that reveals evidence
  is never used as the reward recipient.
- A validator's `owner` is its immutable administrative, validator-fee, self-stake, and slashing identity.
  `changeValidatorOwner` remains in the compatibility ABI but always reverts with
  `ValidatorOwnerImmutable()`.

## Equivocation reporting

Reporting remains permissionless, but requires two transactions so an observer cannot copy public evidence from the
mempool and redirect the reporter reward:

1. The reward beneficiary computes
   `keccak256(abi.encode(domainHash, chainId, staking, proofKind, keccak256(evidence), beneficiary, salt))`, where
   `domainHash = keccak256("FluentStakingEquivocationReportV1")` and proof kinds are
   `0 = notarize`, `1 = finalize`, and `2 = nullify-finalize`. The
   `computeEquivocationReportCommitment` view returns this value without duplicating the encoding. The salt must be an
   unpredictable 32-byte value and must remain private until reveal.
2. The beneficiary sends `commitEquivocationReport(bytes32)`. One active commitment is stored per beneficiary, so a
   later commit from that beneficiary replaces its earlier one.
3. In a later block, any account may call the matching `slashEquivocation*` method with the four evidence values plus
   the beneficiary and salt. A copied reveal still resolves and pays the original beneficiary; changing the beneficiary,
   proof kind, evidence, or salt no longer matches the prior commitment.
4. A successful slash consumes the commitment and permanently tombstones the validator. Failed evidence verification
   leaves the commitment available for retry.

The repository does not contain a node-side equivocation submitter. Integrations only need an ordinary beneficiary
account for the commit transaction; the reveal may be sent by any funded account after observing that the commit is
included in an earlier block.

## Solidity parity

The Solidity staking source is not checked into this repository. Its mutable-owner self-stake lookup remains affected
and must also disable validator ownership changes before it is deployed or used as the canonical implementation.

## Event ABI audit

The SDK event derive encodes non-indexed fields as Solidity function arguments. This is the canonical event-data shape:
the tuple head starts at byte zero, without the extra outer offset used when a dynamic tuple is encoded as a standalone
value.

The repository-wide `#[derive(Event)]` audit found four events whose emitted data changes:

| Contract        | Event                                                        | Dynamic non-indexed fields                                                       | Byte-shape change                                                                                                          |
|-----------------|--------------------------------------------------------------|----------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------|
| staking         | `ConsensusKeysSet(address,bytes,bytes32,uint64)`             | `bytes blsPubkey`                                                                | Drops the standalone tuple's leading outer-offset word; the first data word is now the `bytes` offset (`0x60`).            |
| staking         | `EpochCommitteeCommitted(uint64,address[])`                  | `address[] committee`                                                            | Drops the leading outer-offset word; event data begins with the array offset (`0x20`).                                     |
| runtime-upgrade | `RuntimeUpgraded(address,bytes32,string,bytes32)`            | `string genesisVersion`                                                          | Drops the leading outer-offset word; the event-data head contains the string offset and `codeHash` directly.               |
| runtime-upgrade | `UpgradePlanned(bytes32,string,address[],bytes32[],address)` | `string genesisVersion`, `address[] targetAddresses`, `bytes32[] wasmCodeHashes` | Drops the leading outer-offset word; all three dynamic offsets are now relative to the event-data tuple head at byte zero. |

All other repository events contain only static non-indexed fields (or no non-indexed fields), so their emitted data
bytes are unchanged.

## Source Layout

- `initializer.rs`: atomic one-shot initialization.
- `config.rs`: chain configuration initialization, getters, setters, and dependencies.
- `staking.rs`: epoch reads, validator administration, delegation, and rewards.
- `consensus.rs`: consensus keys, epoch committees, liveness, and equivocation handling.
- `storage.rs`: separate ERC-7201 roots and epoch snapshots.

## Verification

```bash
cargo test --manifest-path contracts/Cargo.toml -p fluentbase-contracts-staking
cargo test -p fluentbase-e2e staking
```
