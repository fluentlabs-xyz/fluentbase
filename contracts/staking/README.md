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
- Verifies BLS12-381 signatures ITSELF, against the EIP-2537 precompiles at the addresses the
  fork fixes (`0x02`, `0x05`, `0x0b`, `0x0f`, `0x10` — `src/bls.rs`). There is no configured
  verifier and no setter for one: `blsVerifier`, `setBlsVerifier` and `getBlsVerifier` were
  removed on 2026-09-08, so a substituted verifier can no longer accept a forged proof of
  possession.
- Calls the BLEND token on behalf of the configured reserve — reading what the reserve can cover
  at an epoch close, and moving it straight to a claimant at a claim.

## Lifecycle

1. Deployment atomically installs and initializes staking, chain configuration, and external dependencies before public
   transactions can execute.
2. The initializer is permissionless and one-shot. Its `initialStakeOwner` argument is the explicit BLEND sponsor for
   genesis validator stake; it grants no contract authority.
3. Governance manages chain configuration, dependency rotation, and validator status.
4. Validator creation verifies and stores consensus keys atomically; delegators approve and deposit BLEND.
5. The system caller commits epoch committees and closes finished epochs; the close prices an epoch and moves no money.
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
- Undelegated principal is released only after its maturity epoch, through `withdrawDelegatorPrincipal`, which is a
  separate handler from the reward claim and draws on a separate pot.
- **A reward and a matured withdrawal are two claims over two cursors and two pots.** The reward is drawn off the BLEND
  reserve straight to the recipient and never enters this contract; the principal is a deposit this contract holds and
  goes out of its own balance. `claimed_through_epoch` and `undelegate_gap` move independently, so an unfunded reserve
  cannot hold up a withdrawal and an empty withdrawal queue cannot hold up a reward. Reward claims are bounded by the
  current epoch — there is no settlement frontier to bound them by.
- A validator owner's undelegated principal matures on the ordinary schedule; no separate liability deadline applies.
- Equivocation evidence does not expire. The offender is resolved from the signing key, which is recorded permanently,
  so a report stays valid for as long as there is stake to seize.
- Equivocation seizure consumes both active and pending self-principal.
- Claims and committee pruning are bounded per call.
- BLEND transfers accept ERC-20 tokens that return `true` or no data; explicit `false` reverts.
- The epoch stipend is flat pro-rata over the committee's frozen leader weights and consults no liveness verdict. The
  only exclusions are a permanent equivocation tombstone and a zero frozen weight.
- **The stipend never enters this contract.** The close only records what each seat is owed; a claim pulls that amount
  off the BLEND reserve straight to the claimant. Everything this contract holds is somebody's deposit, so a balance
  check on it is a check on deposits and never on stipend solvency.
- The credits an epoch writes are the sum of the floored shares, never the pot, so the remainder of at most `n − 1`
  base units is simply never assigned.
- **An epoch the reserve cannot cover is forfeited, permanently.** Before it prices an epoch, the close reads
  `min(balanceOf(reserve), allowance(reserve, staking))`; below the pot the epoch closes at zero and nothing revisits
  it, so a later top-up funds later epochs and never that one. All-or-nothing, not pro rata: a credit written to a
  snapshot is one the reserve was good for when it was written.
- **Every failure of that read scores zero, never an error.** A missing token, a reverting call, an undecodable answer
  — all read as "the reserve can cover nothing". The close is a pre-execution system call, so the alternative to a
  quiet zero is a chain halt no transaction can repair.
- A zero in `EpochBlendRewardsCommitted` therefore has several causes — zero rate, empty committee, all-zero weights,
  an epoch that recorded no block, weights aged out of the ring, and a reserve that is empty, unapproved or unreadable
  — and the contract does not tell them apart. That is accepted: the reserve cases are a configuration error, checked
  off-chain at deployment.
- A CLAIM, unlike the close, is allowed to revert, and does: a reserve that cannot pay a claim fails it rather than
  paying short.
- `getEpochRewards` reports what an epoch was ACCRUED, which is what is owed. Nothing records what has been claimed
  against it.
- Equivocation tombstones are permanent and prevent key reuse.
- Compressed BLS public keys are stored as three fixed `bytes32` words. Validator creation rejects any verifier output
  that is not exactly 96 bytes, avoiding dynamic-bytes metadata and making malformed stored key lengths unrepresentable.
- Committee selection drops the ineligible FIRST and ranks by stake afterwards. Eligible means all
  three of: status Active, selection-visible (no running production exclusion), and holding a
  consensus key active by the selection epoch. The order matters the other way round: filtering
  after the cut spent a seat on a validator that could not take it and did not pass the seat to the
  next candidate, so a population of twenty eligible validators could seat four.
- `commitEpochCommittee` takes no argument. It derives the committee itself and sorts it ascending by
  peer key, which is the order the consensus index space uses — `recordProduction` credits the member
  at the carried leader index. Producing that order rather than checking a supplied one removes the
  only way the two could have disagreed.
- A revert inside a system call stops the chain — with ONE exception the node makes deliberately.
  These calls run before any transaction in the block and the node treats a non-success result as a
  block-execution error, so for `recordProduction` and `commitEpochCommittee` there is no retry and
  no transaction can repair the state afterwards. `slashEquivocation` is the exception: the node
  logs its revert and does not commit its state, trading a lost slash for a running chain, because
  a slash has a second route that can still land it. A committee below `MIN_COMMITTEE_LENGTH`
  is therefore an assertion of an assumption — that the chain always has that many eligible
  validators — not a condition the contract expects to meet. That assumption is reachable by
  ordinary permissionless action: one owner withdrawing their own self-stake, or one equivocation
  tombstone, removes a validator from the active set IMMEDIATELY, so on a network sitting at the
  floor the very next commit reverts and the chain stops. The alternative — carrying the previous
  committee forward — was removed on 2026-09-07 because it froze the seats, tombstoned members
  included, in the quorum denominator.
- The committee-size cap is one live scalar. Changing it governs the next commit, not a future epoch:
  the per-epoch checkpoint history was removed with the rest of the epoch-addressed selection surface.
  What is still frozen per epoch is the COMMITTED committee itself, written once by the commit and
  never rewritten — that, and not a reconstructable selection view, is the record of what an epoch
  seated. The `effectiveEpoch` field of `ActiveValidatorsLengthChanged` is a leftover of the old
  scheduling and no longer names the first epoch the new cap governs.
- Leader weights are frozen at commit time from the selection epoch, and are never recomputed on
  read. An unfrozen weight would depend on the block height each node reads at, and the leader is
  drawn from those weights.
- A seizure has one recipient — the configured slash fund, or the burn sink when none is set. Nobody
  is paid for reporting, so no submitter of a slash can profit from copying another's evidence.
- **The two address setters are not symmetric, and the asymmetry is the point.** `setBlendReserve` names the
  account the stipend is PULLED FROM, which a stolen governance key could point at itself, so it is two-step:
  `setBlendReserve` declares, `applyBlendReserve` lands it after seven epochs and before the window closes
  seven epochs later, `cancelBlendReserve` withdraws it, and `getPendingBlendReserve` shows what is armed. The
  expiry and the withdrawal exist because a declaration that could neither lapse nor be taken back would sit
  armed for the life of the chain, and a key stolen long afterwards would land it in one block with the notice
  period long past. `setSlashFundAddress` names where a seizure GOES, which a stolen key cannot drain, and it
  is the repair path a refused seizure depends on — so it stays immediate. `setBlendStipendPerEpoch` is
  immediate too, decided separately.
- A validator's `owner` is its immutable administrative, validator-fee, self-stake, and slashing identity.
  There is no ABI point that changes it.

## Equivocation slashing

Two entry points reach the same terminal effects — tombstone, jail, removal from the selection view, and
seizure of the offender's self-stake. They differ only in who established that the offender equivocated.

`slashEquivocation(uint64 epoch, uint32 signerIdx)` is **system-caller only** and carries no evidence. It
records a verdict the committee has already reached: every member verified the charge against the block it
rode in before voting for that block, which is the same trust basis as any other state transition. The
signer index is resolved against `epoch`'s frozen committee via `committee_member_at` (`resolveSigner`,
its former public wrapper, is deleted — it had no callers). A
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
A recipient that refuses the transfer **reverts the whole penalty**: the tombstone, the jail, the active-set
removal and the selection-invisibility stamp roll back with the payout, and the charge can be brought again
once the recipient accepts. The alternative — swallowing the refusal — left the bond on this contract with no
path off it and reported a seizure of nothing, which is half a penalty with the missing half unrecoverable.

The cost is that a fund which refuses makes equivocation unslashable *while it refuses*, and the whole repair
is `setSlashFundAddress`. That is why that setter is the one address setter with NO timelock: it was given one
on 2026-09-11 and exempted again the same day, because seven epochs of notice on the repair path is seven
epochs of an offender keeping its seat, its bond and its rewards. Nor can the fund fall back to the burn sink
— the sink is reached only when the stored address is zero and the setter refuses a zero — so an immediate
rotation is the only exit. Survivable because the node soft-folds a revert of the system-call route (below)
rather than halting, and because the rotation lands in one block.

## Solidity parity

The Solidity staking source is not checked into this repository. Its mutable-owner self-stake lookup remains affected
and must make validator ownership immutable before it is deployed or used as the canonical implementation. This
implementation carries no ABI point that changes an owner — the `changeValidatorOwner` entry that used to reject the
attempt was deleted 2026-09-08, so there is nothing here for the Solidity side to mirror.

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

- `lib.rs`: the selector dispatcher, and the whole list of public entry points.
- `initializer.rs`: atomic one-shot initialization.
- `config.rs`: chain configuration initialization, getters, setters, and dependencies.
- `staking.rs`: epoch reads, validator administration, delegation, and rewards.
- `consensus.rs`: consensus keys, epoch committees, and equivocation handling.
- `evidence.rs`: the equivocation evidence wire format and its decoder.
- `bls.rs`: the inlined BLS12-381 verifier over the EIP-2537 precompiles.
- `liveness.rs`: the block-production recorder and the epoch close.
- `storage.rs`: separate ERC-7201 roots and epoch snapshots.
- `consts.rs`: selectors, error ids, protocol limits, defaults.
- `events.rs`, `types.rs`, `math.rs`, `util.rs`: event shapes, command structs, pure arithmetic,
  and the ABI/ERC-20/guard helpers.

## Block-production liveness

The system caller reports every block's producer through `recordProduction`. When the epoch rolls
over, the close runs three legs with three deliberately different failure policies:

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
3. **Accrual** — last, and it decides what the closing epoch owes its committee and records it,
   moving no money. It cannot fail: its one outward call asks the BLEND reserve what it can cover,
   and reads every failure as zero rather than raising it. What that produces is a forfeit — the
   epoch closes owing nothing, for good, because nothing ever revisits a closed epoch.

   There is no fourth leg. The close used to end in a fuel-capped self-call that pulled the epoch's
   pot onto this contract behind a global payment cursor; the reserve pays each claimant directly
   now, so there is nothing left for the close to move and nothing to be tolerant about.

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
