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
- Calls configured BLS verifier and BLEND reserve contracts.

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
There are two validator creation paths: the initializer, which seats its validators active with the genesis stake pulled
from the sponsor, and permissionless registration, which bonds the registrant's own self-stake and leaves the validator
pending until governance activates it. Both require the BLS key, proof of possession, and peer key in the
validator-creation call; there is no separate key-registration phase.

## Accounting Invariants

- Stake and commission changes take effect through epoch snapshots; selection changes become visible in the following
  epoch.
- A newly materialized snapshot copies only the latest state already effective at that epoch. Earlier-effective stake
  and commission changes are carried forward through any scheduled warm-up snapshots, never copied backward from them.
- The configured validator minimum binds where the bond is posted — initialization and permissionless registration — and
  again on a partial owner withdrawal, which may not leave a self-stake remainder below it. Activation requires only that
  the owner's effective self-stake be non-zero, so raising the minimum never strands an earlier registrant while an owner
  who withdrew his whole bond still cannot be seated. Committee selection applies no self-stake minimum of its own.
- A full owner exit moves an active validator to pending in the same transaction and removes its next-epoch selection
  visibility.
- Delegation amounts must use `BALANCE_COMPACT_PRECISION`.
- Undelegated principal is released only after its maturity epoch and is claimed through the reward path.
- Reward claims and views never consume epochs at or beyond the exclusive settled frontier. Matured undelegated
  principal is processed against its own bounded cursor, so delayed reward settlement cannot block withdrawals.
- A validator owner's undelegated principal matures on the ordinary schedule; no separate liability deadline applies.
- Equivocation evidence does not expire. The offender is resolved from the signing key, which is recorded permanently,
  so a report stays valid for as long as there is stake to seize.
- Equivocation seizure consumes both active and pending self-principal.
- Claims, stipend catch-up, and committee pruning are bounded per call.
- BLEND transfers accept ERC-20 tokens that return `true` or no data; explicit `false` reverts.
- The epoch stipend is flat pro-rata over the committee's frozen leader weights and consults no liveness verdict. The
  only exclusions are a permanent equivocation tombstone and a zero frozen weight.
- **Deciding what an epoch owes and paying it are separate acts.** The close splits the pot and writes the
  per-validator credits together with the total it assigned, in one frame; the settlement cursor later pulls that
  total and moves on. Nothing on the payment path reads a committee or a weight.
- The credits and the total the payment pulls are computed from the same figure in the same frame, and that figure is
  the sum of the floored shares, never the pot. The contract therefore never owes more than it computed, and the
  remainder of at most `n − 1` base units stays with the funder.
- **A failed pull defers the epoch; it never forfeits it.** The disbursement is all-or-nothing — reverted calls and
  malformed return values revert settlement — but the entitlement is already recorded, so a refusal costs a delay and
  nothing else. A revoked allowance holds the epochs it stops and pays every one of them once it is restored.
- An accrued epoch is not claimable until it is funded. All three claim walks bound themselves by the settlement
  cursor, which is what keeps the two apart without letting a claim run ahead of the money.
- `getEpochRewards` reports what an epoch was ACCRUED, not what has been PAID for it. It fills in at the close, whether
  or not a token ever moves, so it cannot be used to diagnose a stalled cursor.
- Equivocation tombstones are permanent and prevent key reuse.
- Compressed BLS public keys are stored as three fixed `bytes32` words. Validator creation rejects any verifier output
  that is not exactly 96 bytes, avoiding dynamic-bytes metadata and making malformed stored key lengths unrepresentable.
- Committee selection ranks candidates by stake first and drops those without active, correctly
  shaped consensus keys afterwards. The order matters: filtering before the cut would promote a
  lower-staked keyed validator into the committee, changing which validators the epoch seats.
- `commitEpochCommittee` takes no argument. It derives the committee itself and sorts it ascending by
  peer key, which is the order the consensus index space uses — `recordProduction` credits the member
  at the carried leader index. Producing that order rather than checking a supplied one removes the
  only way the two could have disagreed.
- Every revert reachable inside a system call stops the chain. These calls run before any transaction
  in the block, and the node treats a non-success result as a block-execution error, so there is no
  retry and no transaction can repair the state afterwards. A committee below `MIN_COMMITTEE_LENGTH`
  is therefore an assertion of an assumption — that the chain always has that many eligible
  validators — not a condition the contract expects to meet.
- The committee-size cap is epoch-addressed. Changing it schedules the new value from the next epoch,
  so an epoch that has already started keeps the cap it was selected under. The scalar getter reports
  the latest scheduled value immediately and is not epoch-correct by design.
- Leader weights are frozen at commit time from the selection epoch, and are never recomputed on
  read. An unfrozen weight would depend on the block height each node reads at, and the leader is
  drawn from those weights.
- A seizure has one recipient — the configured slash fund, or the burn sink when none is set. Nobody
  is paid for reporting, so no submitter of a slash can profit from copying another's evidence.
- A validator's `owner` is its immutable administrative, validator-fee, self-stake, and slashing identity.
  `changeValidatorOwner` remains in the compatibility ABI but always reverts with
  `ValidatorOwnerImmutable()`.

## Equivocation slashing

Two entry points reach the same terminal effects — tombstone, jail, removal from the selection view, and
seizure of the offender's self-stake. They differ only in who established that the offender equivocated.

`slashEquivocation(uint64 epoch, uint32 signerIdx)` is **system-caller only** and carries no evidence. It
records a verdict the committee has already reached: every member verified the charge against the block it
rode in before voting for that block, which is the same trust basis as any other state transition. The
signer index is resolved against `epoch`'s frozen committee, the same lookup `resolveSigner` answers. A
repeat verdict against an already-tombstoned validator returns successfully and changes nothing — two
proposers may carry the same charge, and a system caller must not be able to fail a pre-execution call on a
race.

The three `slashEquivocation{Notarize,Finalize,NullifyFinalize}(bytes,bytes,bytes,bytes)` entry points are
permissionless and carry the evidence itself: the encoded conflict, the uncompressed public key, and the two
uncompressed signatures. Here the contract verifies — it resolves identity from the registered BLS key,
which is write-once and never released, so this route works for any epoch, including one whose committee has
long rotated out. That is why it stays: a charge that fails to reach a block before its epoch ends can no
longer be verified by any live committee, and this is the only way it still lands. A repeat here reverts
with `AlreadySlashedForEquivocation(address)`.

The seizure has a single recipient: the configured slash fund, or `EQUIVOCATION_BURN_SINK` when none is set.
A recipient that refuses the transfer does not roll the slash back — the tombstone, the jail and the
active-set removal are already written, and the recipient is not chosen by whoever submitted the slash.

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
over, the close runs four legs with three deliberately different failure policies:

1. **Releases** — unconditional. Neither a tainted epoch nor the `productionLivenessDisabled` kill
   switch holds an expiring exclusion back: tying releases to either would freeze them during
   exactly the outage they exist for, and would make the exclusion duration depend on when the
   switch was flipped. A release is not a punishment, so the switch has no business stopping it.
2. **Verdicts** — fail-loud, and the one leg the kill switch does hold. An epoch whose recorded
   block count does not match the number of heights it could have recorded is tainted: it emits
   `PartialEpoch` and is not judged at all, because a partial record cannot distinguish an idle
   validator from a missing report. With the switch on, no new verdicts and no new stamps.

   That number is the epoch interval for every epoch but the first. Epoch 0 owns one height fewer:
   the DPoS activation block is produced by the pre-DPoS sequencer, which holds no committee
   position, so its header carries empty `extra_data` and the node issues no record for it. Epoch 0
   is therefore complete at `interval - 1`, and expecting the full interval there would taint a
   healthy first day on every chain and leave it permanently unjudged.
3. **Accrual** — unconditional and fail-loud, in the close's own frame. It decides what the closing
   epoch owes its committee and records it, moving no money. Unconditional because the close only
   ever runs for the epoch of the last recorded block: an epoch it skips here is one nothing will
   ever accrue for, and the settlement cursor walks contiguously and would wait on it forever. So an
   epoch that recorded nothing still gets marked closed, owing nothing. It runs in the main frame,
   not inside the tolerant leg below, because the leg's failure is survivable only while a retrying
   cursor exists to re-do the work — and the payment cursor no longer does that work.
4. **Payment** — tolerant. It runs in a fuel-capped self-call so a failing payment cannot roll back
   the releases, verdicts and accrual of the same close; a failure emits `StipendLegSkipped` from
   the outer frame. What the discarded frame takes with it is the payment and only the payment: the
   epochs it could not fund are still recorded and still owed.

The consequence of failing liveness is a temporary, auto-reversing **exclusion** from committee
selection, never a stake penalty and never a jail — equivocation is the only path to `Jail`. An
exclusion is refused outright when no replacement can take the seat, and a refusal leaves no trace,
so a small network shrinks its committee rather than losing quorum.

An epoch is paid only once it has finished, and only for what its own close assigned it. An epoch
that recorded no blocks is assigned nothing and is forfeited; an epoch whose close has not run yet
but which did record blocks is deferred, because its close is still coming and forfeiting would
throw away a real entitlement. The two are told apart by the recorded block count, which is why the
scalar the close writes cannot decide it alone.

## Verification

```bash
cargo test --manifest-path contracts/Cargo.toml -p fluentbase-contracts-staking
cargo test --manifest-path contracts/Cargo.toml -p fluentbase-contracts-staking --features devnet-views
cargo test -p fluentbase-e2e staking
```

The first run is the shape that ships. Four liveness views — `producedAt`, `blocksInEpoch`,
`pendingExclusions`, `lastProcessedBlock` — have no production consumer and are compiled out unless
`devnet-views` is on, so that run also asserts the contract works with none of them present. The
second run covers the four. The e2e suite always gets them: it activates the feature through
`[dev-dependencies]`, which are built for tests and never for `cargo build`, so no ordinary build —
including the one that produces the published genesis — can pick them up.
