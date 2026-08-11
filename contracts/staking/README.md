# Staking

The core validator staking contract implemented as a normal rWasm contract and deployed at
`GENESIS_STAKING`. It uses `SharedAPI` for storage, BLEND transfers, logs, and calls to protocol dependencies.

## Scope

- Implements validator lifecycle, delegation, rewards, committees, equivocation slashing, and
  block-production liveness.
- Owns the chain configuration previously read from `ChainConfig`, and the block-production
  accounting previously held by a separate liveness contract.
- Isolates initializer, chain configuration, consensus, staking, and production-liveness state in
  separate ERC-7201 namespaces: `Fluent.storage.Initializer`, `Fluent.storage.ChainConfig`,
  `Fluent.storage.Consensus`, `Fluent.storage.StakingStorage`, and
  `Fluent.storage.ProductionLiveness`.
- Keeps `StakingPool` external and unchanged; this crate does not deploy or replace it.
- Calls configured BLS verifier, evidence decoder, and BLEND reserve contracts.

## Lifecycle

1. Deployment atomically installs and initializes staking, chain configuration, and external dependencies before public
   transactions can execute.
2. The initializer is permissionless and one-shot. Its `initialStakeOwner` argument is the explicit BLEND sponsor for
   genesis validator stake; it grants no contract authority.
3. Governance manages chain configuration, dependency rotation, and validator status.
4. Validator creation verifies and stores consensus keys atomically; delegators approve and deposit BLEND.
5. The system caller commits epoch committees and settles the stipend for epochs that have finished.
6. Verified equivocation permanently tombstones a validator and seizes its self-stake. Block-production liveness never
   jails and never touches stake; it only excludes a validator from selection for a bounded number of epochs.

Governance is fixed at compile time to the `GENESIS_GOVERNANCE` address. Changing it requires a coordinated code/genesis
rebuild. The base genesis builder embeds staking but does not install governance code at the reserved address, so a
production network genesis must provide the governance deployment or equivalent authority there before privileged
staking operations are needed.

The liveness-slashing and BLEND-reserve dependencies are observable and independently rotatable by governance. Every
initial assignment and later rotation emits its previous and new address. Epoch interval, DPoS activation, and
undelegation-period changes are rejected after a non-zero activation has passed; activation zero remains the explicit
unarmed/non-DPoS state used by the Solidity contract.

Registered validator identities are permanent. Governance may disable and reactivate validators, but disabling never
deletes their records, consensus-key state, ownership mappings, or stake history.
Governance-added validators begin pending with zero stake and cannot become active until their self-stake is effective
at or above the configured validator minimum. Genesis, governance, and permissionless registration all require the BLS
key, proof of possession, and peer key in the validator-creation call; there is no separate key-registration phase.

## Accounting Invariants

- Stake and commission changes take effect through epoch snapshots; selection changes become visible in the following
  epoch.
- A newly materialized snapshot copies only the latest state already effective at that epoch. Earlier-effective stake
  and commission changes are carried forward through any scheduled warm-up snapshots, never copied backward from them.
- Initialization, activation, and committee selection each require the validator owner's effective
  self-stake to meet the configured minimum. A full owner exit moves an active validator to pending in the same
  transaction and removes its next-epoch selection visibility.
- Delegation amounts must use `BALANCE_COMPACT_PRECISION`.
- Undelegated principal is released only after its maturity epoch and is claimed through the reward path.
- Reward claims and views never consume epochs at or beyond the exclusive settled frontier. Matured undelegated
  principal is processed against its own bounded cursor, so delayed reward settlement cannot block withdrawals.
- Each validator-owner undelegation snapshots a fixed liability deadline covering the committee lookahead and evidence
  retention window. Later committees secured by the remaining self-stake do not extend queued principal, and stalled
  committee pruning cannot delay release. The same absolute committee deadlines expire equivocation evidence;
  `getValidatorSelfStakeLock` exposes the current lock state and exclusive unlock epoch.
- Equivocation seizure consumes both active and pending self-principal.
- Claims, stipend catch-up, and committee pruning are bounded per call.
- BLEND transfers accept ERC-20 tokens that return `true` or no data; explicit `false` reverts.
- The epoch stipend is flat pro-rata over the committee's frozen leader weights and consults no liveness verdict. The
  only exclusions are a permanent equivocation tombstone and a zero frozen weight.
- Reserve settlement credits rewards only after the exact assigned amount is disbursed. A successful zero or partial
  disbursement skips the epoch with zero credited rewards and advances the cursor; reverted calls and malformed return
  values revert settlement and remain retryable.
- Equivocation tombstones are permanent and prevent key reuse.
- Compressed BLS public keys are stored as three fixed `bytes32` words. Validator creation rejects any verifier output
  that is not exactly 96 bytes, avoiding dynamic-bytes metadata and making malformed stored key lengths unrepresentable.
- Committee selection ranks candidates by stake first and drops those without active, correctly
  shaped consensus keys afterwards, and rejects empty committees without advancing the commit epoch.
  The order matters: the off-chain deriver reads the same ranked view with inactive keys blanked and
  discards the keyless entries itself, so filtering before the cut would promote a lower-staked
  validator and make every honest submission fail.
- The committee-size cap is epoch-addressed. Changing it schedules the new value from the next epoch,
  so an epoch that has already started keeps the cap it was selected under. The scalar getter reports
  the latest scheduled value immediately and is not epoch-correct by design.
- Leader weights are frozen at commit time from the selection epoch, and are never recomputed on
  read. An unfrozen weight would depend on the block height each node reads at, and the leader is
  drawn from those weights.
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
- `consensus.rs`: consensus keys, epoch committees, and equivocation handling.
- `liveness.rs`: the block-production recorder and the epoch close.
- `storage.rs`: separate ERC-7201 roots and epoch snapshots.

## Block-production liveness

The system caller reports every block's producer through `recordProduction`. When the epoch rolls
over, the close runs three legs with three deliberately different failure policies:

1. **Releases** — unconditional, and frozen by the `productionLivenessDisabled` kill switch.
2. **Verdicts** — fail-loud. An epoch whose recorded block count does not match the epoch interval is
   tainted: it emits `PartialEpoch` and is not judged at all, because a partial record cannot
   distinguish an idle validator from a missing report.
3. **Stipend** — tolerant. It runs in a fuel-capped self-call so a failing payment cannot roll back
   the releases and verdicts of the same close; a failure emits `StipendLegSkipped` from the outer
   frame.

The consequence of failing liveness is a temporary, auto-reversing **exclusion** from committee
selection, never a stake penalty and never a jail — equivocation is the only path to `Jail`. An
exclusion is refused outright when no replacement can take the seat, and a refusal leaves no trace,
so a small network shrinks its committee rather than losing quorum.

The stipend is paid only for an epoch that recorded blocks and has finished. A skipped epoch is
forfeited, not deferred.

## Verification

```bash
cargo test --manifest-path contracts/Cargo.toml -p fluentbase-contracts-staking
cargo test -p fluentbase-e2e staking
```
