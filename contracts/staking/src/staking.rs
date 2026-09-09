//! Staking ownership, epoch reads, and validator lifecycle methods.

use crate::{
    consensus::{self, store_consensus_keys, verify_consensus_keys},
    consts::*,
    events, liveness, math,
    storage::{chain_config_storage, consensus_storage, staking_storage, ValidatorSnapshotStorage},
    types::{
        AddressAmountCommand, AddressCommand, AddressU16Command, RegisterValidatorCommand,
        U64Command, ValidatorBlockCommand, ValidatorDelegatorCommand, ValidatorEpochCommand,
    },
    util::{
        current_epoch, current_epoch_at_block, decode, decode_args, ensure_governance,
        ensure_initialized, ensure_mutable, ensure_non_payable, next_epoch, reserve_available,
        revert, revert_with, safe_transfer, safe_transfer_from, write_abi,
    },
};
use alloc::{vec, vec::Vec};
use fluentbase_sdk::{Address, ContextReader, ExitCode, SharedAPI, U256};

fn address_arg(input: &[u8]) -> Result<Address, ExitCode> {
    Ok(decode::<AddressCommand>(input)?.value)
}

pub(crate) fn validator_status<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
) -> Result<u8, ExitCode> {
    staking_storage()
        .validators_accessor()
        .entry(validator)
        .status_accessor()
        .get_checked(sdk)
}

pub(crate) fn remove_active<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
) -> Result<(), ExitCode> {
    let active = staking_storage().active_validators_accessor();
    let len = active.len_checked(sdk)?;
    for index in 0..len {
        if active.at(index).get_checked(sdk)? != validator {
            continue;
        }
        if index + 1 != len {
            let last = active.at(len - 1).get_checked(sdk)?;
            active.at(index).set_checked(sdk, last)?;
        }
        active.pop_checked(sdk)?;
        break;
    }
    Ok(())
}

fn deactivate_validator_at<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    epoch: u64,
) -> Result<(), ExitCode> {
    remove_active(sdk, validator)?;
    staking_storage()
        .validators_accessor()
        .entry(validator)
        .status_accessor()
        .set_checked(sdk, STATUS_PENDING)?;
    set_selection_visible(sdk, validator, false, epoch)
}

pub(crate) fn set_validator<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    validator_owner: Address,
    validator_status: u8,
    commission_rate: u16,
    stake: U256,
    changed_at: u64,
) -> Result<(), ExitCode> {
    if validator.is_zero() {
        return revert(sdk, ERR_ZERO_VALIDATOR);
    }
    if validator_owner.is_zero() {
        return revert(sdk, ERR_ZERO_OWNER);
    }
    if commission_rate > COMMISSION_RATE_MAX {
        return revert_with(sdk, ERR_BAD_COMMISSION_RATE, &commission_rate);
    }
    let Some(compact_stake) = math::compact_balance(stake) else {
        return revert(sdk, ERR_WRONG_AMOUNT_PRECISION);
    };

    let storage = staking_storage();
    let record = storage.validators_accessor().entry(validator);
    if record.status_accessor().get_checked(sdk)? != STATUS_NOT_FOUND {
        return revert_with(sdk, ERR_VALIDATOR_ALREADY_EXISTS, &validator);
    }
    if !storage
        .owner_validators_accessor()
        .entry(validator_owner)
        .get_checked(sdk)?
        .is_zero()
    {
        return revert_with(sdk, ERR_VALIDATOR_OWNER_ALREADY_IN_USE, &validator_owner);
    }

    record.owner_accessor().set_checked(sdk, validator_owner)?;
    record
        .status_accessor()
        .set_checked(sdk, validator_status)?;
    record.changed_at_accessor().set_checked(sdk, changed_at)?;
    // The reward cursor starts where the validator does. Left at zero, a
    // validator registered at epoch N would have to walk MAX_EPOCHS_PER_CLAIM
    // epochs of empty history per call before reaching its first payable
    // epoch. The delegator side floors itself on read instead
    // (`delegate_claim_start`, `max(cursor, first)`), because its first epoch is
    // per delegator-validator pair; an owner has exactly one birth epoch and it
    // is already known here.
    //
    // Not the same floor, though it was once: the delegator's is a REWARD epoch
    // (`first_reward_epoch_for`), while this one is the stake epoch itself. The
    // owner therefore opens on two epochs that can pay him nothing, which costs
    // him two steps of the claim window and nothing else.
    record.claimed_at_accessor().set_checked(sdk, changed_at)?;
    storage
        .owner_validators_accessor()
        .entry(validator_owner)
        .set_checked(sdk, validator)?;

    let snapshot = storage
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(changed_at);
    snapshot
        .total_delegated_accessor()
        .set_checked(sdk, compact_stake)?;
    snapshot
        .commission_rate_accessor()
        .set_checked(sdk, commission_rate)?;
    insert_snapshot_epoch(sdk, validator, changed_at)?;

    let delegation = storage
        .validator_delegations_accessor()
        .entry(validator)
        .entry(validator_owner);
    let delegation_queue_length = delegation.delegate_queue_accessor().len_checked(sdk)?;
    if delegation_queue_length != 0 {
        return revert_with(
            sdk,
            ERR_DELEGATION_QUEUE_NOT_EMPTY,
            &U256::from(delegation_queue_length),
        );
    }
    let initial = delegation.delegate_queue_accessor().grow_checked(sdk)?;
    initial.amount_accessor().set_checked(sdk, compact_stake)?;
    initial.epoch_accessor().set_checked(sdk, changed_at)?;

    if validator_status == STATUS_ACTIVE {
        storage
            .active_validators_accessor()
            .push_checked(sdk, validator)?;
    }
    seed_selection_membership(
        sdk,
        validator,
        validator_status == STATUS_ACTIVE,
        changed_at,
    )?;
    events::ValidatorAdded {
        validator,
        owner: validator_owner,
        status: validator_status,
        commission_rate,
    }
    .emit(sdk)?;
    Ok(())
}

fn seed_selection_membership<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    visible: bool,
    since_epoch: u64,
) -> Result<(), ExitCode> {
    let membership = staking_storage()
        .selection_membership_accessor()
        .entry(validator);
    membership.visible_accessor().set_checked(sdk, visible)?;
    // Written directly rather than through `set_selection_visible` so the seed
    // takes effect at `since_epoch` itself, with no next-epoch delay: a genesis
    // validator is selectable in epoch 0. The empty history below `since_epoch`
    // is the truthful one — the validator did not exist there.
    membership.prev_visible_accessor().set_checked(sdk, false)?;
    membership.prev_from_accessor().set_checked(sdk, 0)?;
    membership
        .prev2_visible_accessor()
        .set_checked(sdk, false)?;
    membership.prev2_from_accessor().set_checked(sdk, 0)?;
    membership
        .effective_from_accessor()
        .set_checked(sdk, since_epoch)
}

pub(crate) fn set_selection_visible<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    visible: bool,
    at_epoch: u64,
) -> Result<(), ExitCode> {
    let membership = staking_storage()
        .selection_membership_accessor()
        .entry(validator);
    let effective = at_epoch.checked_add(1).ok_or(ExitCode::IntegerOverflow)?;
    let effective_from = membership.effective_from_accessor().get_checked(sdk)?;
    if effective_from == effective {
        // Re-stamped inside the same epoch close: the transition already exists
        // and only its landing value changes. Shifting here would spend a second
        // history slot on one transition — an exclusion and its release within a
        // single close would then push the oldest segment out for nothing.
        membership.visible_accessor().set_checked(sdk, visible)?;
    } else {
        // Read the whole record before writing any of it: each field below is
        // the source for the one above it.
        let previous = membership.visible_accessor().get_checked(sdk)?;
        let previous_visible = membership.prev_visible_accessor().get_checked(sdk)?;
        let previous_from = membership.prev_from_accessor().get_checked(sdk)?;
        membership
            .prev2_visible_accessor()
            .set_checked(sdk, previous_visible)?;
        membership
            .prev2_from_accessor()
            .set_checked(sdk, previous_from)?;
        membership
            .prev_visible_accessor()
            .set_checked(sdk, previous)?;
        membership
            .prev_from_accessor()
            .set_checked(sdk, effective_from)?;
        membership.visible_accessor().set_checked(sdk, visible)?;
        membership
            .effective_from_accessor()
            .set_checked(sdk, effective)?;
    }
    Ok(())
}

pub(crate) fn selection_visible_at<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    epoch: u64,
) -> Result<bool, ExitCode> {
    let membership = staking_storage()
        .selection_membership_accessor()
        .entry(validator);
    if epoch >= membership.effective_from_accessor().get_checked(sdk)? {
        membership.visible_accessor().get_checked(sdk)
    } else if epoch >= membership.prev_from_accessor().get_checked(sdk)? {
        membership.prev_visible_accessor().get_checked(sdk)
    } else if epoch >= membership.prev2_from_accessor().get_checked(sdk)? {
        membership.prev2_visible_accessor().get_checked(sdk)
    } else {
        // The record holds three transitions and no more, so anything older
        // than the oldest recorded one falls off the end. Answering `false`
        // there is the conservative direction: before its first transition a
        // validator was not in the selection view, and excluding a member that
        // should have been seated shrinks a committee, while including one that
        // should not have been seats an address the epoch never authorized.
        Ok(false)
    }
}

/// Stamp `validator` selection-invisible from the next epoch onward.
///
/// Refuses — returning `false`, never reverting — when the validator is already
/// invisible at the bite epoch, or when the eligible population less this
/// validator would no longer reach [`MIN_COMMITTEE_LENGTH`]. Below the floor the
/// commit reverts, and the commit is a pre-execution system call, so an
/// exclusion that pushed the population under it would stop the chain over a
/// liveness verdict. A revert is not available here either: the caller must be
/// able to leave no trace of a refusal.
///
/// Best-effort by construction: the stamp bites two selection epochs after this
/// check, and registrations in between can invalidate it either way.
pub(crate) fn apply_production_exclusion<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
) -> Result<bool, ExitCode> {
    let bite_epoch = next_epoch(sdk)?;
    if !selection_visible_at(sdk, validator, bite_epoch)? {
        return Ok(false);
    }
    // No "stamps already issued this close" term: an earlier stamp in the same
    // close is already invisible at `bite_epoch`, so this read excludes it.
    if !eligible_population_at_least(sdk, bite_epoch, MIN_COMMITTEE_LENGTH as u64 + 1)? {
        return Ok(false);
    }
    set_selection_visible(sdk, validator, false, current_epoch(sdk)?)?;
    events::ProductionExclusionApplied {
        validator,
        bite_epoch,
    }
    .emit(sdk)?;
    Ok(true)
}

/// Restore selection visibility at the end of an exclusion.
///
/// A silent no-op for a tombstoned or non-Active validator. The selection
/// filter is the visibility stamp and nothing else, so a blind re-stamp would
/// permanently re-seat a slashed equivocator.
pub(crate) fn release_production_exclusion<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
) -> Result<(), ExitCode> {
    if consensus_storage()
        .tombstoned_accessor()
        .entry(validator)
        .get_checked(sdk)?
    {
        return Ok(());
    }
    if validator_status(sdk, validator)? != STATUS_ACTIVE {
        return Ok(());
    }
    let bite_epoch = next_epoch(sdk)?;
    set_selection_visible(sdk, validator, true, current_epoch(sdk)?)?;
    events::ProductionExclusionReleased {
        validator,
        bite_epoch,
    }
    .emit(sdk)
}

/// Does `epoch` have at least `wanted` Active, selection-visible validators?
///
/// A BOUND, not a count, and the scan stops the moment it is met. The only
/// caller asks "would removing one leave the committee floor standing", which
/// needs `MIN_COMMITTEE_LENGTH + 1` and nothing finer — and the exact count is a
/// walk of the whole Active set, two SLOADs per entry, on the epoch close. At the
/// production ceiling of 51 validators that is a hundred reads for an answer that
/// six of them settle. Stopping early is why this is a predicate.
///
/// Mirrors the population `selected_committee_at` ranks, minus its consensus-key
/// term: a validator that is Active and visible but keyless is counted here and
/// dropped there. Counting the wider set is the conservative direction for the
/// only caller — an exclusion refused because the count looked too small never
/// happens, while one allowed on a count that was too large is what the commit's
/// own floor still catches.
///
/// WHICH validators the early exit stops on does not matter: the caller removes
/// exactly one, so `population >= wanted` gives `population - 1 >= wanted - 1`
/// whether or not the one being removed is among those counted.
pub(crate) fn eligible_population_at_least<SDK: SharedAPI>(
    sdk: &SDK,
    epoch: u64,
    wanted: u64,
) -> Result<bool, ExitCode> {
    if wanted == 0 {
        return Ok(true);
    }
    let active = staking_storage().active_validators_accessor();
    let len = active.len_checked(sdk)?;
    let mut population = 0;
    for index in 0..len {
        let validator = active.at(index).get_checked(sdk)?;
        if validator_status(sdk, validator)? == STATUS_ACTIVE
            && selection_visible_at(sdk, validator, epoch)?
        {
            population += 1;
            if population >= wanted {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Live committee view.
///
/// Answers "who would be selected right now" — ranked and cut like the commit,
/// but without the commit's eligibility filter, so it can name a validator the
/// commit would drop. Only `selected_committee_at` feeds committee commits.
pub(crate) fn selected_validators<SDK: SharedAPI>(sdk: &SDK) -> Result<Vec<Address>, ExitCode> {
    let storage = staking_storage();
    let active = storage.active_validators_accessor();
    let active_len = active.len_checked(sdk)?;
    let epoch = current_epoch(sdk)?;
    let mut candidates = Vec::with_capacity(active_len as usize);
    for index in 0..active_len {
        let validator = active.at(index).get_checked(sdk)?;
        if validator_status(sdk, validator)? == STATUS_ACTIVE {
            candidates.push(validator);
        }
    }
    let cap = chain_config_storage()
        .active_validators_length_accessor()
        .get_checked(sdk)? as usize;
    Ok(top_k_by_stake_at(sdk, candidates, epoch, cap)?
        .into_iter()
        .map(|(validator, _)| validator)
        .collect())
}

/// Match Solidity's partial selection sort exactly.
///
/// Equal-stake candidates retain the order of the candidate vector; an address
/// tie-breaker would choose a different committee at the top-k boundary. NB that
/// vector is now `active_validators`, which `remove_active` mutates by
/// swap-remove — so on equal stakes a departure and a return can change WHICH of
/// two tied validators sits above the cut. Deterministic (every node reads the
/// same vector) but history-dependent, where the append-only roster it replaced
/// was not.
/// Returns each selected validator with the stake it was ranked on.
///
/// The weight is returned rather than dropped because the committee commit
/// freezes exactly these numbers, and recomputing them there costs a second
/// `validator_total_at` per member — each of which runs its own binary search
/// over the validator's snapshot epochs.
pub(crate) fn top_k_by_stake_at<SDK: SharedAPI>(
    sdk: &SDK,
    candidates: Vec<Address>,
    epoch: u64,
    cap: usize,
) -> Result<Vec<(Address, U256)>, ExitCode> {
    let mut weighted = Vec::with_capacity(candidates.len());
    for validator in candidates {
        weighted.push((validator, validator_total_at(sdk, validator, epoch)?));
    }
    let mut candidates = weighted;
    let k = core::cmp::min(cap, candidates.len());
    for index in 0..k {
        let mut next = index;
        let mut max_stake = candidates[next].1;
        for (candidate, (_, stake)) in candidates.iter().enumerate().skip(index + 1) {
            if *stake > max_stake {
                next = candidate;
                max_stake = *stake;
            }
        }
        candidates.swap(index, next);
    }
    candidates.truncate(k);
    Ok(candidates)
}

fn emit_modified<SDK: SharedAPI>(sdk: &mut SDK, validator: Address) -> Result<(), ExitCode> {
    let record = staking_storage().validators_accessor().entry(validator);
    let changed_at = record.changed_at_accessor().get_checked(sdk)?;
    let snapshot = staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(changed_at);
    events::ValidatorModified {
        validator,
        owner: record.owner_accessor().get_checked(sdk)?,
        status: record.status_accessor().get_checked(sdk)?,
        commission_rate: snapshot.commission_rate_accessor().get_checked(sdk)?,
    }
    .emit(sdk)
}

fn insert_snapshot_epoch<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    epoch: u64,
) -> Result<(), ExitCode> {
    let epochs = staking_storage()
        .validator_snapshot_epochs_accessor()
        .entry(validator);
    let len = epochs.len_checked(sdk)?;
    let mut low = 0;
    let mut high = len;
    while low < high {
        let middle = low + (high - low) / 2;
        if epochs.at(middle).get_checked(sdk)? < epoch {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    if low < len && epochs.at(low).get_checked(sdk)? == epoch {
        return Ok(());
    }

    epochs.grow_checked(sdk)?;
    let mut index = len;
    while index > low {
        let previous = epochs.at(index - 1).get_checked(sdk)?;
        epochs.at(index).set_checked(sdk, previous)?;
        index -= 1;
    }
    epochs.at(low).set_checked(sdk, epoch)
}

fn latest_snapshot_epoch_at_or_before<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    epoch: u64,
) -> Result<Option<u64>, ExitCode> {
    let epochs = staking_storage()
        .validator_snapshot_epochs_accessor()
        .entry(validator);
    let len = epochs.len_checked(sdk)?;
    let mut low = 0;
    let mut high = len;
    while low < high {
        let middle = low + (high - low) / 2;
        if epochs.at(middle).get_checked(sdk)? <= epoch {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    if low == 0 {
        return Ok(None);
    }
    Ok(Some(epochs.at(low - 1).get_checked(sdk)?))
}

/// The epoch whose snapshot governs what a seat earns at `epoch`.
///
/// `commit_epoch_committee` selects the committee for a target epoch from
/// `target - MAX_COMMITTEE_LOOKAHEAD_EPOCHS` and carries that vintage's weights
/// through as the frozen ring, so the seat's WEIGHT at `epoch` — the thing the
/// stipend is divided by across seats — was fixed there. Dividing the seat's
/// own share by any later vintage lets stake that arrived after the selection
/// dilute the delegators the seat was actually sized on.
///
/// The saturation is the one `commit_epoch_committee` uses, so the two agree on
/// the first two epochs instead of disagreeing about an epoch below zero.
///
/// This is the committee-selection lag and nothing else. `WARMUP_DELAY` happens
/// to be the same number and is a different quantity; see its own note.
fn selection_epoch_for(epoch: u64) -> u64 {
    epoch.saturating_sub(MAX_COMMITTEE_LOOKAHEAD_EPOCHS)
}

/// The first reward epoch a stake epoch governs — the inverse of
/// `selection_epoch_for`, saturation and all.
///
/// Not a bare `+ MAX_COMMITTEE_LOOKAHEAD_EPOCHS`. `selection_epoch_for` clamps
/// at zero, so stake epoch 0 governs reward epochs 0 and 1 as well as 2, and
/// shifting unconditionally would leave the delegator pool of the chain's first
/// two epochs credited to a seat and claimable by nobody.
fn first_reward_epoch_for(stake_epoch: u64) -> Result<u64, ExitCode> {
    if stake_epoch == 0 {
        return Ok(0);
    }
    stake_epoch
        .checked_add(MAX_COMMITTEE_LOOKAHEAD_EPOCHS)
        .ok_or(ExitCode::IntegerOverflow)
}

/// The `(total_delegated, commission_rate)` in force at `epoch`.
///
/// Read-only twin of `touch_snapshot_at_or_before`. The payout and the view have
/// to answer identically and the view holds only `&SDK`, so both go through
/// here; and reading the latest snapshot at or before `epoch` rather than the
/// snapshot AT `epoch` gives an unmaterialized epoch the state that was actually
/// in force — the same answer materializing it would have produced, because
/// materialization copies from exactly this snapshot.
fn snapshot_state_at_or_before<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    epoch: u64,
) -> Result<(math::U112, u16), ExitCode> {
    let Some(snapshot_epoch) = latest_snapshot_epoch_at_or_before(sdk, validator, epoch)? else {
        return Ok((math::U112::ZERO, 0));
    };
    let snapshot = staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(snapshot_epoch);
    Ok((
        snapshot.total_delegated_accessor().get_checked(sdk)?,
        snapshot.commission_rate_accessor().get_checked(sdk)?,
    ))
}

/// Materializes `epoch` from the latest snapshot that was already effective.
///
/// `record.changed_at` is only the highest materialized epoch and can point to
/// a future warm-up checkpoint, so it must never be used as the copy source.
pub(crate) fn touch_snapshot_at_or_before<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    epoch: u64,
) -> Result<ValidatorSnapshotStorage, ExitCode> {
    let storage = staking_storage();
    let snapshots = storage.validator_snapshots_accessor().entry(validator);
    let snapshot = snapshots.entry(epoch);
    let Some(base_epoch) = latest_snapshot_epoch_at_or_before(sdk, validator, epoch)? else {
        return Ok(snapshot);
    };
    if base_epoch == epoch {
        return Ok(snapshot);
    }
    let base = snapshots.entry(base_epoch);
    snapshot
        .total_delegated_accessor()
        .set_checked(sdk, base.total_delegated_accessor().get_checked(sdk)?)?;
    snapshot
        .commission_rate_accessor()
        .set_checked(sdk, base.commission_rate_accessor().get_checked(sdk)?)?;
    insert_snapshot_epoch(sdk, validator, epoch)?;
    let record = storage.validators_accessor().entry(validator);
    if epoch > record.changed_at_accessor().get_checked(sdk)? {
        record.changed_at_accessor().set_checked(sdk, epoch)?;
    }
    Ok(snapshot)
}

fn first_snapshot_index_at_or_after<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    epoch: u64,
) -> Result<u64, ExitCode> {
    let epochs = staking_storage()
        .validator_snapshot_epochs_accessor()
        .entry(validator);
    let len = epochs.len_checked(sdk)?;
    let mut low = 0;
    let mut high = len;
    while low < high {
        let middle = low + (high - low) / 2;
        if epochs.at(middle).get_checked(sdk)? < epoch {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    Ok(low)
}

fn update_total_delegated_from<SDK, F>(
    sdk: &mut SDK,
    validator: Address,
    epoch: u64,
    update: F,
) -> Result<(), ExitCode>
where
    SDK: SharedAPI,
    F: Fn(math::U112) -> Option<math::U112>,
{
    touch_snapshot_at_or_before(sdk, validator, epoch)?;
    // Contract entrypoints can materialize only the current epoch or the
    // E+1/E+2 scheduling horizon, so this future tail remains bounded.
    let storage = staking_storage();
    let epochs = storage
        .validator_snapshot_epochs_accessor()
        .entry(validator);
    let len = epochs.len_checked(sdk)?;
    let start = first_snapshot_index_at_or_after(sdk, validator, epoch)?;
    for index in start..len {
        let snapshot_epoch = epochs.at(index).get_checked(sdk)?;
        let total = storage
            .validator_snapshots_accessor()
            .entry(validator)
            .entry(snapshot_epoch)
            .total_delegated_accessor();
        let next = update(total.get_checked(sdk)?).ok_or(ExitCode::IntegerOverflow)?;
        total.set_checked(sdk, next)?;
    }
    Ok(())
}

/// Removes `delegator`'s bonded position from every materialized snapshot at or
/// after `from_epoch`.
///
/// Each snapshot loses what was bonded *at its own epoch*, not one flat number:
/// `delegate_to` books new stake `WARMUP_DELAY` epochs ahead, so the queue tail
/// can exceed what the nearer snapshots ever counted, and a flat subtraction
/// would underflow and revert the caller. Snapshots before `from_epoch` are
/// deliberately left alone — they denominate rewards that already accrued.
pub(crate) fn remove_delegation_from_totals<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    delegator: Address,
    from_epoch: u64,
) -> Result<(), ExitCode> {
    touch_snapshot_at_or_before(sdk, validator, from_epoch)?;
    let storage = staking_storage();
    let epochs = storage
        .validator_snapshot_epochs_accessor()
        .entry(validator);
    let len = epochs.len_checked(sdk)?;
    let start = first_snapshot_index_at_or_after(sdk, validator, from_epoch)?;
    for index in start..len {
        let snapshot_epoch = epochs.at(index).get_checked(sdk)?;
        let bonded = delegated_amount_at(sdk, validator, delegator, snapshot_epoch)?;
        if bonded.is_zero() {
            continue;
        }
        let total = storage
            .validator_snapshots_accessor()
            .entry(validator)
            .entry(snapshot_epoch)
            .total_delegated_accessor();
        let next = total
            .get_checked(sdk)?
            .checked_sub(bonded)
            .ok_or(ExitCode::IntegerOverflow)?;
        total.set_checked(sdk, next)?;
    }
    Ok(())
}

fn set_commission_from<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    epoch: u64,
    commission_rate: u16,
) -> Result<(), ExitCode> {
    touch_snapshot_at_or_before(sdk, validator, epoch)?;
    // Preserve the new commission in already-materialized warm-up snapshots.
    let storage = staking_storage();
    let epochs = storage
        .validator_snapshot_epochs_accessor()
        .entry(validator);
    let len = epochs.len_checked(sdk)?;
    let start = first_snapshot_index_at_or_after(sdk, validator, epoch)?;
    for index in start..len {
        let snapshot_epoch = epochs.at(index).get_checked(sdk)?;
        storage
            .validator_snapshots_accessor()
            .entry(validator)
            .entry(snapshot_epoch)
            .commission_rate_accessor()
            .set_checked(sdk, commission_rate)?;
    }
    Ok(())
}

fn only_self_stake_remains_after_decrease<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    epoch: u64,
    amount: math::U112,
    remaining_self_stake: math::U112,
) -> Result<bool, ExitCode> {
    let storage = staking_storage();
    let epochs = storage
        .validator_snapshot_epochs_accessor()
        .entry(validator);
    let len = epochs.len_checked(sdk)?;
    let start = first_snapshot_index_at_or_after(sdk, validator, epoch)?;
    for index in start..len {
        let snapshot_epoch = epochs.at(index).get_checked(sdk)?;
        let total = storage
            .validator_snapshots_accessor()
            .entry(validator)
            .entry(snapshot_epoch)
            .total_delegated_accessor()
            .get_checked(sdk)?;
        let next = total.checked_sub(amount).ok_or(ExitCode::IntegerOverflow)?;
        if next != remaining_self_stake {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn validator_total_at<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    epoch: u64,
) -> Result<U256, ExitCode> {
    let storage = staking_storage();
    let record = storage.validators_accessor().entry(validator);
    if record.status_accessor().get_checked(sdk)? == STATUS_NOT_FOUND {
        return Ok(U256::ZERO);
    }

    let Some(snapshot_epoch) = latest_snapshot_epoch_at_or_before(sdk, validator, epoch)? else {
        return Ok(U256::ZERO);
    };
    let compact = storage
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(snapshot_epoch)
        .total_delegated_accessor()
        .get_checked(sdk)?;
    Ok(math::expand_balance(compact))
}

/// The delegator's bonded amount as of `epoch`.
///
/// Queue entries carry the cumulative balance effective from their own epoch, so
/// the answer is the last entry that had already taken effect — or zero when
/// none had.
pub(crate) fn delegated_amount_at<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    delegator: Address,
    epoch: u64,
) -> Result<math::U112, ExitCode> {
    let queue = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator)
        .delegate_queue_accessor();
    let len = queue.len_checked(sdk)?;
    let mut low = 0;
    let mut high = len;
    while low < high {
        let middle = low + (high - low) / 2;
        if queue.at(middle).epoch_accessor().get_checked(sdk)? <= epoch {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    if low == 0 {
        return Ok(math::U112::ZERO);
    }
    queue.at(low - 1).amount_accessor().get_checked(sdk)
}

/// Public handler `0x76671808` (`currentEpoch`).
///
/// Returns the epoch containing the current block.
pub fn current_epoch_read<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    write_abi(sdk, &current_epoch(sdk)?)
}

/// Public handler `0xaea0e78b` (`nextEpoch`).
///
/// Returns the epoch following the current block's epoch.
pub fn next_epoch_read<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    write_abi(sdk, &next_epoch(sdk)?)
}

/// Public handler `0xfacd743b` (`isValidator`).
///
/// Reports whether the address is a registered validator.
pub fn is_validator<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let result = validator_status(sdk, address_arg(input)?)? != STATUS_NOT_FOUND;
    write_abi(sdk, &result)
}

/// Public handler `0x42ad55ac` (`isValidatorActive`).
///
/// Reports whether the validator is active and currently selected.
pub fn is_validator_active<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let validator = address_arg(input)?;
    let result = validator_status(sdk, validator)? == STATUS_ACTIVE
        && selected_validators(sdk)?.contains(&validator);
    write_abi(sdk, &result)
}

/// Public handler `0xa310624f` (`getValidatorStatus`).
///
/// Returns the validator's owner, status, stake, claim, and commission data.
pub fn get_validator_status<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let validator = address_arg(input)?;
    let record = staking_storage().validators_accessor().entry(validator);
    let changed_at = record.changed_at_accessor().get_checked(sdk)?;
    let snapshot = staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(changed_at);
    let result = (
        record.owner_accessor().get_checked(sdk)?,
        record.status_accessor().get_checked(sdk)?,
        math::expand_balance(snapshot.total_delegated_accessor().get_checked(sdk)?),
        changed_at,
        record.claimed_at_accessor().get_checked(sdk)?,
        snapshot.commission_rate_accessor().get_checked(sdk)?,
    );
    write_abi(sdk, &result)
}

/// Public handler `0x30108c22` (`getValidatorByOwner`).
///
/// Returns the validator registered to the supplied owner.
pub fn get_validator_by_owner<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let result = staking_storage()
        .owner_validators_accessor()
        .entry(address_arg(input)?)
        .get_checked(sdk)?;
    write_abi(sdk, &result)
}

/// Public handler `0xb7ab4db5` (`getValidators`).
///
/// Returns the currently selected validators.
pub fn get_validators<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &selected_validators(sdk)?)
}

/// Public handler `0xb46e5520` (`activateValidator`).
///
/// Activates a pending validator under governance control.
pub fn activate_validator<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_governance(sdk)?;
    let validator = address_arg(input)?;
    if validator_status(sdk, validator)? != STATUS_PENDING {
        return revert_with(sdk, ERR_NOT_PENDING_VALIDATOR, &validator);
    }
    let activation_epoch = next_epoch(sdk)?;
    let storage = staking_storage();
    let owner = storage
        .validators_accessor()
        .entry(validator)
        .owner_accessor()
        .get_checked(sdk)?;
    // A full owner exit leaves the validator pending, which is the status a fresh
    // registration also carries, so without this the owner can take the bond back
    // and still be seated with nothing at stake.
    //
    // This is not the `minValidatorStakeAmount` check that used to stand here.
    // That one read a governance-mutable threshold, so raising the bar could
    // refuse a registrant who had already paid the bar in force and still held
    // every token of it. Zero is not a parameter: an owner who kept his bond
    // passes at any threshold. And because a partial withdrawal below the minimum
    // is already refused, self-stake here is either zero or at least the
    // threshold that governed it when it was posted.
    if delegated_amount_at(sdk, validator, owner, activation_epoch)?.is_zero() {
        return revert(sdk, ERR_ZERO_OWNER_SELF_STAKE);
    }
    storage
        .validators_accessor()
        .entry(validator)
        .status_accessor()
        .set_checked(sdk, STATUS_ACTIVE)?;
    storage
        .active_validators_accessor()
        .push_checked(sdk, validator)?;
    // The visibility stamp is the whole selection filter, so re-stamping here
    // would silently cancel a running exclusion. The validator still becomes
    // Active; it stays unselectable until the exclusion's own release path
    // re-stamps it.
    if liveness::readmit_at_epoch_of(sdk, validator)? == 0 {
        set_selection_visible(sdk, validator, true, current_epoch(sdk)?)?;
    }
    touch_snapshot_at_or_before(sdk, validator, activation_epoch)?;
    emit_modified(sdk, validator)
}

/// Public handler `0x1fe97684` (`disableValidator`).
///
/// Moves an active validator back to pending status under governance control.
pub fn disable_validator<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_governance(sdk)?;
    let validator = address_arg(input)?;
    if validator_status(sdk, validator)? != STATUS_ACTIVE {
        return revert(sdk, ERR_NOT_ACTIVE_VALIDATOR);
    }
    deactivate_validator_at(sdk, validator, current_epoch(sdk)?)?;
    touch_snapshot_at_or_before(sdk, validator, next_epoch(sdk)?)?;
    emit_modified(sdk, validator)
}

/// Public handler `0x14f8649f` (`changeValidatorCommissionRate`).
///
/// Schedules a validator commission-rate change for the next epoch.
pub fn change_commission<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let command: AddressU16Command = decode(input)?;
    if command.value > COMMISSION_RATE_MAX {
        return revert_with(sdk, ERR_BAD_COMMISSION_RATE, &command.value);
    }
    let record = staking_storage()
        .validators_accessor()
        .entry(command.validator);
    let validator_owner = record.owner_accessor().get_checked(sdk)?;
    if validator_owner.is_zero() {
        return revert_with(sdk, ERR_VALIDATOR_NOT_FOUND, &command.validator);
    }
    if validator_owner != sdk.context().contract_caller() {
        return revert_with(sdk, ERR_ONLY_VALIDATOR_OWNER, &validator_owner);
    }
    let changed_at = next_epoch(sdk)?;
    set_commission_from(sdk, command.validator, changed_at, command.value)?;
    emit_modified(sdk, command.validator)
}

// Validator registration and BLEND delegation principal accounting.

/// Public handler `0xd951e186` (`getValidatorDelegation`).
///
/// Returns a delegator's effective stake and latest delegation epoch for a validator.
pub fn get_validator_delegation<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let command: ValidatorDelegatorCommand = decode(input)?;
    let queue = staking_storage()
        .validator_delegations_accessor()
        .entry(command.validator)
        .entry(command.delegator)
        .delegate_queue_accessor();
    let len = queue.len_checked(sdk)?;
    if len == 0 {
        return write_abi(sdk, &(U256::ZERO, 0u64));
    }
    let latest = queue.at(len - 1);
    let result = (
        math::expand_balance(latest.amount_accessor().get_checked(sdk)?),
        latest.epoch_accessor().get_checked(sdk)?,
    );
    write_abi(sdk, &result)
}

/// Public handler `0xe8810ea7` (`getValidatorDelegatedStakeAt`).
///
/// Returns a validator's total delegated stake at the requested block.
pub fn get_validator_delegated_stake_at<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let command: ValidatorBlockCommand = decode(input)?;
    if command.block_number > U256::from(u64::MAX) {
        return Err(ExitCode::IntegerOverflow);
    }
    let epoch = current_epoch_at_block(sdk, command.block_number.to::<u64>())?;
    write_abi(sdk, &validator_total_at(sdk, command.validator, epoch)?)
}

/// Public handler `registerValidator(address,uint16,uint256,bytes,bytes,bytes32)`.
///
/// Registers a pending validator and pulls its initial self-stake.
pub fn register_validator<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let command = decode_args::<RegisterValidatorCommand>(input)?;
    if command.commission_rate > COMMISSION_RATE_MAX {
        return revert_with(sdk, ERR_BAD_COMMISSION_RATE, &command.commission_rate);
    }
    if staking_storage()
        .validators_accessor()
        .entry(command.validator)
        .status_accessor()
        .get_checked(sdk)?
        != STATUS_NOT_FOUND
    {
        return revert_with(sdk, ERR_VALIDATOR_ALREADY_EXISTS, &command.validator);
    }
    let minimum = chain_config_storage()
        .min_validator_stake_amount_accessor()
        .get_checked(sdk)?;
    if command.initial_stake < minimum {
        return revert_with(sdk, ERR_INITIAL_STAKE_TOO_LOW, &command.initial_stake);
    }
    let verified = verify_consensus_keys(
        sdk,
        command.validator,
        command.bls_pubkey_uncompressed,
        command.bls_pop_uncompressed,
        command.peer_pubkey,
    )?;
    let owner = sdk.context().contract_caller();
    let since_epoch = next_epoch(sdk)?;
    set_validator(
        sdk,
        command.validator,
        owner,
        STATUS_PENDING,
        command.commission_rate,
        command.initial_stake,
        since_epoch,
    )?;
    store_consensus_keys(sdk, command.validator, verified, since_epoch)?;
    let vault = sdk.context().contract_address();
    safe_transfer_from(sdk, owner, vault, command.initial_stake)
}

/// Public handler `0x026e402b` (`delegate`).
///
/// Delegates staking tokens to a validator.
pub fn delegate<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let command: AddressAmountCommand = decode(input)?;
    let delegator = sdk.context().contract_caller();
    delegate_to(sdk, delegator, command.validator, command.amount, true)
}

pub(crate) fn delegate_to<SDK: SharedAPI>(
    sdk: &mut SDK,
    delegator: Address,
    validator: Address,
    amount: U256,
    pull_tokens: bool,
) -> Result<(), ExitCode> {
    let storage = staking_storage();
    let minimum = chain_config_storage()
        .min_staking_amount_accessor()
        .get_checked(sdk)?;
    if amount.is_zero() || amount < minimum {
        return revert_with(sdk, ERR_AMOUNT_TOO_LOW, &amount);
    }
    let Some(compact_amount) = math::compact_balance(amount) else {
        return revert(sdk, ERR_WRONG_AMOUNT_PRECISION);
    };
    if storage
        .validators_accessor()
        .entry(validator)
        .status_accessor()
        .get_checked(sdk)?
        == STATUS_NOT_FOUND
    {
        return revert_with(sdk, ERR_VALIDATOR_NOT_FOUND, &validator);
    }
    // A tombstone is permanent and the equivocation seizure has already run, so
    // anything delegated from here on can never earn.
    // `claim_delegator_reward_before` reaches this through the redelegate
    // branch, so `redelegateDelegatorFee` reverts here too rather than folding a
    // claim back into a dead validator; the same claim stays payable through
    // `claimDelegatorFee`, which runs the identical path with
    // `redelegate: false` and never calls this. `withdrawDelegatorPrincipal`
    // never comes here at all, so a matured deposit is still withdrawable from a
    // tombstoned validator.
    if consensus_storage()
        .tombstoned_accessor()
        .entry(validator)
        .get_checked(sdk)?
    {
        return revert_with(sdk, ERR_VALIDATOR_TOMBSTONED, &validator);
    }

    // New stake affects accounting only after the warm-up delay.
    let at_epoch = current_epoch(sdk)?
        .checked_add(WARMUP_DELAY)
        .ok_or(ExitCode::IntegerOverflow)?;
    update_total_delegated_from(sdk, validator, at_epoch, |total| {
        total.checked_add(compact_amount)
    })?;

    let queue = storage
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator)
        .delegate_queue_accessor();
    let len = queue.len_checked(sdk)?;
    if len == 0 {
        let operation = queue.grow_checked(sdk)?;
        operation
            .amount_accessor()
            .set_checked(sdk, compact_amount)?;
        operation.epoch_accessor().set_checked(sdk, at_epoch)?;
    } else {
        let latest = queue.at(len - 1);
        let previous_amount = latest.amount_accessor().get_checked(sdk)?;
        let next_amount = previous_amount
            .checked_add(compact_amount)
            .ok_or(ExitCode::IntegerOverflow)?;
        if latest.epoch_accessor().get_checked(sdk)? >= at_epoch {
            latest.amount_accessor().set_checked(sdk, next_amount)?;
        } else {
            let operation = queue.grow_checked(sdk)?;
            operation.amount_accessor().set_checked(sdk, next_amount)?;
            operation.epoch_accessor().set_checked(sdk, at_epoch)?;
        }
    }

    if pull_tokens {
        let vault = sdk.context().contract_address();
        safe_transfer_from(sdk, delegator, vault, amount)?;
    }
    events::Delegated {
        validator,
        staker: delegator,
        amount,
        epoch: at_epoch,
    }
    .emit(sdk)?;
    Ok(())
}

/// Public handler `0x4d99dd16` (`undelegate`).
///
/// Queues delegated stake for release after the undelegation period.
pub fn undelegate<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let command: AddressAmountCommand = decode(input)?;
    let delegator = sdk.context().contract_caller();
    undelegate_from(sdk, delegator, command.validator, command.amount)
}

pub(crate) fn undelegate_from<SDK: SharedAPI>(
    sdk: &mut SDK,
    delegator: Address,
    validator: Address,
    amount: U256,
) -> Result<(), ExitCode> {
    let storage = staking_storage();
    let config = chain_config_storage();
    if amount.is_zero() {
        return revert_with(sdk, ERR_AMOUNT_TOO_LOW, &amount);
    }
    let Some(compact_amount) = math::compact_balance(amount) else {
        return revert(sdk, ERR_WRONG_AMOUNT_PRECISION);
    };
    let record = storage.validators_accessor().entry(validator);
    let status = record.status_accessor().get_checked(sdk)?;
    if status == STATUS_NOT_FOUND {
        return revert_with(sdk, ERR_VALIDATOR_NOT_FOUND, &validator);
    }

    let delegation = storage
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator);
    let queue = delegation.delegate_queue_accessor();
    let len = queue.len_checked(sdk)?;
    if len == 0 {
        return revert(sdk, ERR_DELEGATION_QUEUE_EMPTY);
    }
    let latest = queue.at(len - 1);
    let before_epoch = next_epoch(sdk)?;
    let latest_epoch = latest.epoch_accessor().get_checked(sdk)?;
    if latest_epoch > before_epoch {
        return revert_with(sdk, ERR_PENDING_DELEGATION, &latest_epoch);
    }
    let delegated = latest.amount_accessor().get_checked(sdk)?;
    let Some(next_delegated) = delegated.checked_sub(compact_amount) else {
        return revert(sdk, ERR_INSUFFICIENT_BALANCE);
    };

    let snapshot = touch_snapshot_at_or_before(sdk, validator, before_epoch)?;
    let total = snapshot.total_delegated_accessor().get_checked(sdk)?;
    let Some(_next_total) = total.checked_sub(compact_amount) else {
        return revert(sdk, ERR_INSUFFICIENT_BALANCE);
    };

    let owner = record.owner_accessor().get_checked(sdk)?;
    let owner_self_stake =
        delegator == owner && (status == STATUS_ACTIVE || status == STATUS_PENDING);
    let full_owner_exit = owner_self_stake && next_delegated.is_zero();
    if owner_self_stake {
        let min_validator_stake = config
            .min_validator_stake_amount_accessor()
            .get_checked(sdk)?;
        if math::expand_balance(next_delegated) < min_validator_stake
            && (!next_delegated.is_zero()
                || !only_self_stake_remains_after_decrease(
                    sdk,
                    validator,
                    before_epoch,
                    compact_amount,
                    next_delegated,
                )?)
        {
            return revert(sdk, ERR_OWNER_SELF_STAKE_BELOW_MINIMUM);
        }
    }

    // The minimum binds what is left behind, not what leaves. A full exit is
    // therefore never blocked, which is what keeps a position from being locked
    // when governance raises the minimum; short of that, a partial withdrawal may
    // not strand a remainder below it.
    //
    // Self-stake is exempt because the branch above already governs it, against
    // `minValidatorStakeAmount`. The two minimums are set independently and are
    // not ordered, so running both would hold an owner to whichever is stricter
    // instead of to the one that is meant to govern him.
    if !owner_self_stake {
        let remaining = math::expand_balance(next_delegated);
        let minimum = config.min_staking_amount_accessor().get_checked(sdk)?;
        if !remaining.is_zero() && remaining < minimum {
            return revert_with(sdk, ERR_REMAINING_DELEGATION_TOO_LOW, &(remaining, minimum));
        }
    }

    update_total_delegated_from(sdk, validator, before_epoch, |total| {
        total.checked_sub(compact_amount)
    })?;
    if latest_epoch >= before_epoch {
        latest.amount_accessor().set_checked(sdk, next_delegated)?;
    } else {
        let operation = queue.grow_checked(sdk)?;
        operation
            .amount_accessor()
            .set_checked(sdk, next_delegated)?;
        operation.epoch_accessor().set_checked(sdk, before_epoch)?;
    }

    // Principal remains custodial until the configured undelegation delay ends.
    let maturity_epoch = before_epoch
        .checked_add(config.undelegate_period_accessor().get_checked(sdk)?)
        .ok_or(ExitCode::IntegerOverflow)?;
    let pending = delegation.undelegate_queue_accessor().grow_checked(sdk)?;
    pending.amount_accessor().set_checked(sdk, compact_amount)?;
    pending.epoch_accessor().set_checked(sdk, maturity_epoch)?;
    let pending_undelegated = delegation.pending_undelegated_accessor();
    pending_undelegated.set_checked(
        sdk,
        pending_undelegated
            .get_checked(sdk)?
            .checked_add(amount)
            .ok_or(ExitCode::IntegerOverflow)?,
    )?;

    if full_owner_exit {
        // Stake changes at `before_epoch`; membership changes NOW. `remove_active`
        // is immediate and the selection reads the live active set, so the next
        // commit no longer seats this validator whatever epoch it selects from —
        // there is no historical selection view left to re-derive.
        deactivate_validator_at(sdk, validator, before_epoch - 1)?;
        emit_modified(sdk, validator)?;
    }

    events::Undelegated {
        validator,
        staker: delegator,
        amount,
        epoch: before_epoch,
    }
    .emit(sdk)?;
    Ok(())
}
// Snapshot rewards and bounded claims.

/// Splits `epoch`'s accrued stipend into the delegator pool and the owner's
/// commission.
///
/// The AMOUNT is read at `epoch` — that is what the close credited this seat.
/// Everything that divides it is read at the SELECTION epoch, because that is
/// the vintage the seat's weight was frozen from: see `selection_epoch_for`.
///
/// The rate is the MINIMUM of the selection-epoch rate and the epoch's own, so a
/// rise takes three epochs to reach anyone's money while a cut lands the next
/// epoch. Reading the selection epoch alone would have been symmetrical and
/// wrong in the other direction: an announced discount would miss precisely the
/// delegators it was announced to.
fn snapshot_payout<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    epoch: u64,
) -> Result<(U256, U256), ExitCode> {
    let total_reward = U256::from(
        staking_storage()
            .validator_snapshots_accessor()
            .entry(validator)
            .entry(epoch)
            .total_blend_rewards_accessor()
            .get_checked(sdk)?,
    );
    if total_reward.is_zero() {
        return Ok((U256::ZERO, U256::ZERO));
    }
    let (selection_total, selection_rate) =
        snapshot_state_at_or_before(sdk, validator, selection_epoch_for(epoch))?;
    if selection_total.is_zero() {
        return Ok((U256::ZERO, total_reward));
    }
    let (_, current_rate) = snapshot_state_at_or_before(sdk, validator, epoch)?;
    let rate = core::cmp::min(selection_rate, current_rate);
    let owner_reward = total_reward
        .checked_mul(U256::from(rate))
        .ok_or(ExitCode::IntegerOverflow)?
        / U256::from(BPS_DENOMINATOR);
    Ok((total_reward - owner_reward, owner_reward))
}

fn validator_owner_rewards<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    before_epoch: u64,
) -> Result<U256, ExitCode> {
    let record = staking_storage().validators_accessor().entry(validator);
    if record.status_accessor().get_checked(sdk)? == STATUS_NOT_FOUND {
        return Ok(U256::ZERO);
    }
    // Reads what `claim_validator_before` would pay, and a tombstone makes that
    // nothing. Without this the view would keep quoting a balance no call can
    // ever collect.
    if consensus_storage()
        .tombstoned_accessor()
        .entry(validator)
        .get_checked(sdk)?
    {
        return Ok(U256::ZERO);
    }
    let mut epoch = record.claimed_at_accessor().get_checked(sdk)?;
    // Bound historical work; callers can continue from the stored cursor.
    let before_epoch = core::cmp::min(before_epoch, epoch.saturating_add(MAX_EPOCHS_PER_CLAIM));
    let mut rewards = U256::ZERO;
    while epoch < before_epoch {
        rewards = rewards
            .checked_add(snapshot_payout(sdk, validator, epoch)?.1)
            .ok_or(ExitCode::IntegerOverflow)?;
        epoch = epoch.checked_add(1).ok_or(ExitCode::IntegerOverflow)?;
    }
    Ok(rewards)
}

/// A delegator's unclaimed REWARD, and nothing else.
///
/// The matured principal has its own reader and its own cursor. They are read
/// apart because they are now PAID apart: the reward is pulled off the BLEND
/// reserve straight to the delegator and never touches this contract, while the
/// principal is a deposit this contract still holds. One number over two sources
/// would need a rule for the half that fails.
fn delegator_reward_claimable<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    delegator: Address,
    reward_before_epoch: u64,
) -> Result<U256, ExitCode> {
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator);
    let delegates = delegation.delegate_queue_accessor();
    let delegate_len = delegates.len_checked(sdk)?;
    let mut claimable = U256::ZERO;

    if let Some((mut index, mut epoch)) = delegate_claim_start(sdk, validator, delegator)? {
        while index < delegate_len && epoch < reward_before_epoch {
            // The queue is stamped in STAKE epochs and this walk is in REWARD
            // epochs. An entry that takes effect at stake epoch S governs the
            // rewards of E >= S + MAX_COMMITTEE_LOOKAHEAD_EPOCHS, so the
            // successor's stamp has to be carried forward by the same lag before
            // it can close this entry's reward window.
            let changed_at = if index + 1 < delegate_len {
                first_reward_epoch_for(delegates.at(index + 1).epoch_accessor().get_checked(sdk)?)?
            } else {
                reward_before_epoch
            };
            let end = core::cmp::min(reward_before_epoch, changed_at);
            let delegated = delegates.at(index).amount_accessor().get_checked(sdk)?;
            while epoch < end {
                let (delegator_pool, _) = snapshot_payout(sdk, validator, epoch)?;
                // Numerator and denominator are both the selection vintage, and
                // `snapshot_payout` sized the pool it is dividing on that same
                // vintage. Reading the denominator at `epoch` instead would let a
                // delegation that arrived after the seat was weighed shrink every
                // earlier delegator's share of a pot their stake alone earned.
                let (total, _) =
                    snapshot_state_at_or_before(sdk, validator, selection_epoch_for(epoch))?;
                if !total.is_zero() {
                    claimable = claimable
                        .checked_add(
                            delegator_pool
                                .checked_mul(U256::from(delegated))
                                .ok_or(ExitCode::IntegerOverflow)?
                                / U256::from(total),
                        )
                        .ok_or(ExitCode::IntegerOverflow)?;
                }
                epoch = epoch.checked_add(1).ok_or(ExitCode::IntegerOverflow)?;
            }
            index += 1;
        }
    }
    Ok(claimable)
}

/// A delegator's matured undelegation principal, and nothing else.
fn delegator_principal_claimable<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    delegator: Address,
    principal_before_epoch: u64,
) -> Result<U256, ExitCode> {
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator);
    let undelegates = delegation.undelegate_queue_accessor();
    let undelegate_len = undelegates.len_checked(sdk)?;
    let mut undelegate_gap = delegation.undelegate_gap_accessor().get_checked(sdk)?;
    let mut claimable = U256::ZERO;
    while undelegate_gap < undelegate_len {
        let operation = undelegates.at(undelegate_gap);
        if operation.epoch_accessor().get_checked(sdk)? > principal_before_epoch {
            break;
        }
        claimable = claimable
            .checked_add(math::expand_balance(
                operation.amount_accessor().get_checked(sdk)?,
            ))
            .ok_or(ExitCode::IntegerOverflow)?;
        undelegate_gap += 1;
    }
    Ok(claimable)
}

/// Position to resume a reward claim from: the delegate-queue entry in force at the reward
/// cursor, and the epoch to start accruing at. `None` when the delegator has no delegations.
///
/// The resume epoch is `max(cursor, first entry)` rather than the bare cursor: a delegator who
/// has never claimed carries a zero cursor, and windowing `MAX_EPOCHS_PER_CLAIM` from zero would
/// place the whole window before the first delegation and strand the funds permanently.
fn delegate_claim_start<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    delegator: Address,
) -> Result<Option<(u64, u64)>, ExitCode> {
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator);
    let delegates = delegation.delegate_queue_accessor();
    let len = delegates.len_checked(sdk)?;
    if len == 0 {
        return Ok(None);
    }
    let cursor = delegation
        .claimed_through_epoch_accessor()
        .get_checked(sdk)?;
    // Both halves of the `max` have to be REWARD epochs. The cursor already is;
    // the queue stamp is a stake epoch, and the first reward it can earn is its
    // selection lag later. Without the shift the walk would open on a reward
    // epoch whose selection vintage predates the delegation, spending window
    // budget on epochs that pay this delegator nothing.
    let first = first_reward_epoch_for(delegates.at(0).epoch_accessor().get_checked(sdk)?)?;
    let start = core::cmp::max(cursor, first);
    // `start` is at or after the first reward epoch the opening entry governs,
    // so its selection epoch is at or after that entry's stamp and the search
    // below always lands on a real entry.
    let stake_start = selection_epoch_for(start);

    let mut low = 0;
    let mut high = len;
    while low < high {
        let middle = low + (high - low) / 2;
        if delegates.at(middle).epoch_accessor().get_checked(sdk)? <= stake_start {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    Ok(Some((low - 1, start)))
}

fn capped_delegator_reward_epoch<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    delegator: Address,
    before_epoch: u64,
) -> Result<u64, ExitCode> {
    let start = match delegate_claim_start(sdk, validator, delegator)? {
        Some((_, start)) => start,
        None => return Ok(before_epoch),
    };
    Ok(core::cmp::min(
        before_epoch,
        start
            .checked_add(MAX_EPOCHS_PER_CLAIM)
            .ok_or(ExitCode::IntegerOverflow)?,
    ))
}

fn capped_delegator_principal_epoch<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    delegator: Address,
    before_epoch: u64,
) -> Result<u64, ExitCode> {
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator);
    let undelegates = delegation.undelegate_queue_accessor();
    let undelegate_gap = delegation.undelegate_gap_accessor().get_checked(sdk)?;
    if undelegate_gap >= undelegates.len_checked(sdk)? {
        return Ok(before_epoch);
    }
    let first = undelegates
        .at(undelegate_gap)
        .epoch_accessor()
        .get_checked(sdk)?;
    Ok(core::cmp::min(
        before_epoch,
        first
            .checked_add(MAX_EPOCHS_PER_CLAIM)
            .ok_or(ExitCode::IntegerOverflow)?,
    ))
}

/// Consumes the REWARD cursor only, and returns what it accrued.
///
/// It never reads or writes `undelegate_gap`: the two cursors are independent,
/// and a reward claim that nudged the withdrawal queue would silently mature
/// principal the delegator did not ask for.
fn consume_delegator_reward<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    delegator: Address,
    reward_before_epoch: u64,
) -> Result<U256, ExitCode> {
    let storage = staking_storage();
    let delegation = storage
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator);
    let delegates = delegation.delegate_queue_accessor();
    let delegate_len = delegates.len_checked(sdk)?;
    let mut claimable = U256::ZERO;

    if let Some((mut index, mut epoch)) = delegate_claim_start(sdk, validator, delegator)? {
        while index < delegate_len && epoch < reward_before_epoch {
            // The queue is stamped in STAKE epochs and this walk is in REWARD
            // epochs. An entry that takes effect at stake epoch S governs the
            // rewards of E >= S + MAX_COMMITTEE_LOOKAHEAD_EPOCHS, so the
            // successor's stamp has to be carried forward by the same lag before
            // it can close this entry's reward window.
            let changed_at = if index + 1 < delegate_len {
                first_reward_epoch_for(delegates.at(index + 1).epoch_accessor().get_checked(sdk)?)?
            } else {
                reward_before_epoch
            };
            let end = core::cmp::min(reward_before_epoch, changed_at);
            let delegated = delegates.at(index).amount_accessor().get_checked(sdk)?;
            while epoch < end {
                let (delegator_pool, _) = snapshot_payout(sdk, validator, epoch)?;
                // Numerator and denominator are both the selection vintage, and
                // `snapshot_payout` sized the pool it is dividing on that same
                // vintage. Reading the denominator at `epoch` instead would let a
                // delegation that arrived after the seat was weighed shrink every
                // earlier delegator's share of a pot their stake alone earned.
                let (total, _) =
                    snapshot_state_at_or_before(sdk, validator, selection_epoch_for(epoch))?;
                if !total.is_zero() {
                    claimable = claimable
                        .checked_add(
                            delegator_pool
                                .checked_mul(U256::from(delegated))
                                .ok_or(ExitCode::IntegerOverflow)?
                                / U256::from(total),
                        )
                        .ok_or(ExitCode::IntegerOverflow)?;
                }
                epoch = epoch.checked_add(1).ok_or(ExitCode::IntegerOverflow)?;
            }
            index += 1;
        }
        delegation
            .claimed_through_epoch_accessor()
            .set_checked(sdk, epoch)?;
    }
    Ok(claimable)
}

/// Consumes the WITHDRAWAL cursor only, and returns the principal it matured.
///
/// It never reads or writes `claimed_through_epoch`, so an empty BLEND reserve
/// cannot hold a matured deposit hostage — this path does not touch the reserve
/// at all. The principal is paid out of this contract's own balance, where the
/// delegation put it.
fn consume_delegator_principal<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    delegator: Address,
    principal_before_epoch: u64,
) -> Result<U256, ExitCode> {
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator);
    let undelegates = delegation.undelegate_queue_accessor();
    let undelegate_len = undelegates.len_checked(sdk)?;
    let mut undelegate_gap = delegation.undelegate_gap_accessor().get_checked(sdk)?;
    let pending_undelegated = delegation.pending_undelegated_accessor();
    let mut pending_principal = pending_undelegated.get_checked(sdk)?;
    let mut claimable = U256::ZERO;
    while undelegate_gap < undelegate_len {
        let operation = undelegates.at(undelegate_gap);
        if operation.epoch_accessor().get_checked(sdk)? > principal_before_epoch {
            break;
        }
        let principal = math::expand_balance(operation.amount_accessor().get_checked(sdk)?);
        claimable = claimable
            .checked_add(principal)
            .ok_or(ExitCode::IntegerOverflow)?;
        pending_principal = pending_principal
            .checked_sub(principal)
            .ok_or(ExitCode::IntegerOverflow)?;
        undelegate_gap += 1;
    }
    delegation
        .undelegate_gap_accessor()
        .set_checked(sdk, undelegate_gap)?;
    pending_undelegated.set_checked(sdk, pending_principal)?;
    Ok(claimable)
}

fn available_for_redelegate<SDK: SharedAPI>(
    sdk: &SDK,
    claimable: U256,
) -> Result<(U256, U256), ExitCode> {
    let amount = claimable / BALANCE_COMPACT_PRECISION * BALANCE_COMPACT_PRECISION;
    let minimum = chain_config_storage()
        .min_staking_amount_accessor()
        .get_checked(sdk)?;
    if amount < minimum {
        return Ok((U256::ZERO, claimable));
    }
    Ok((amount, claimable - amount))
}

/// Public handler `0x457179fd` (`getValidatorFee`).
///
/// Returns the validator owner's currently claimable rewards.
pub fn get_validator_fee<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let validator = decode::<AddressCommand>(input)?.value;
    write_abi(
        sdk,
        &validator_owner_rewards(sdk, validator, current_epoch(sdk)?)?,
    )
}

fn claim_validator_before<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    before_epoch: u64,
) -> Result<(), ExitCode> {
    let record = staking_storage().validators_accessor().entry(validator);
    if record.status_accessor().get_checked(sdk)? == STATUS_NOT_FOUND {
        return revert_with(sdk, ERR_VALIDATOR_NOT_FOUND, &validator);
    }
    // A proven equivocator's commission is extinguished, not confiscated. The
    // money was never here to seize: `pay_stipend` pulls it from the BLEND
    // reserve, so refusing the claim simply leaves it in the reserve, and it is
    // deliberately NOT routed to the slash fund.
    //
    // The gate is on the OWNER path alone. `seize_self_stake` takes the owner's
    // own bond and touches no delegator queue, and `assign_epoch_shares` already
    // skips a tombstoned seat, so the only thing left standing was the
    // already-accrued commission on epochs the validator was still seated for.
    // Delegators keep their share of those same epochs and claim it through
    // their own path, which this does not touch.
    if consensus_storage()
        .tombstoned_accessor()
        .entry(validator)
        .get_checked(sdk)?
    {
        return revert_with(sdk, ERR_VALIDATOR_TOMBSTONED, &validator);
    }
    let claimed_at = record.claimed_at_accessor().get_checked(sdk)?;
    // Advancing the cursor before transfer is safe because a failed call
    // reverts the whole contract transaction.
    let capped = core::cmp::min(
        before_epoch,
        claimed_at
            .checked_add(MAX_EPOCHS_PER_CLAIM)
            .ok_or(ExitCode::IntegerOverflow)?,
    );
    let mut epoch = claimed_at;
    let mut amount = U256::ZERO;
    while epoch < capped {
        amount = amount
            .checked_add(snapshot_payout(sdk, validator, epoch)?.1)
            .ok_or(ExitCode::IntegerOverflow)?;
        epoch = epoch.checked_add(1).ok_or(ExitCode::IntegerOverflow)?;
    }
    let owner = record.owner_accessor().get_checked(sdk)?;
    record.claimed_at_accessor().set_checked(sdk, epoch)?;
    pay_stipend(sdk, owner, amount)?;
    events::ValidatorOwnerClaimed {
        validator,
        amount,
        epoch,
    }
    .emit(sdk)
}

/// Public handler `0xff4794fc` (`claimValidatorFee`).
///
/// Claims all currently available validator-owner rewards.
pub fn claim_validator_fee<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let validator = decode::<AddressCommand>(input)?.value;
    claim_validator_before(sdk, validator, current_epoch(sdk)?)
}

/// Public handler `0xadf2a79c` (`claimValidatorFeeAtEpoch`).
///
/// Claims validator-owner rewards through the requested epoch.
pub fn claim_validator_fee_at_epoch<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let command = decode::<ValidatorEpochCommand>(input)?;
    if command.before_epoch > current_epoch(sdk)? {
        return revert(sdk, ERR_INVALID_CLAIM_EPOCH);
    }
    claim_validator_before(sdk, command.validator, command.before_epoch)
}

/// Public handler `0x52b7bea2` (`getDelegatorFee`).
///
/// Returns a delegator's currently claimable rewards for a validator.
pub fn get_delegator_fee<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let command = decode::<ValidatorDelegatorCommand>(input)?;
    let requested_epoch = current_epoch(sdk)?;
    let reward_before_epoch =
        capped_delegator_reward_epoch(sdk, command.validator, command.delegator, requested_epoch)?;
    write_abi(
        sdk,
        &delegator_reward_claimable(
            sdk,
            command.validator,
            command.delegator,
            reward_before_epoch,
        )?,
    )
}

/// The delegator's REWARD claim: the reserve pays the delegator directly.
///
/// The matured withdrawal principal is deliberately not here — it is a separate
/// handler over a separate cursor and a separate source. When `redelegate` is
/// set, what gets re-staked is therefore the reward alone; a delegator who asked
/// to withdraw principal no longer has it silently re-staked under them.
fn claim_delegator_reward_before<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    delegator: Address,
    before_epoch: u64,
    redelegate: bool,
) -> Result<(), ExitCode> {
    let reward_before_epoch =
        capped_delegator_reward_epoch(sdk, validator, delegator, before_epoch)?;
    let claimable = consume_delegator_reward(sdk, validator, delegator, reward_before_epoch)?;
    if redelegate {
        let (amount, dust) = available_for_redelegate(sdk, claimable)?;
        if !amount.is_zero() {
            // Book the stake first, then fetch the money — the same order
            // `delegate_to` uses for an ordinary delegation. It matters here
            // because `delegate_to` is where the tombstone refusal lives: a
            // refused redelegation must make no token call at all, not one it
            // then unwinds.
            //
            // `pull_tokens: false` — the delegator never held this money. It
            // comes off the reserve and into this contract, where every other
            // delegation sits: the stake is a deposit however it was earned.
            delegate_to(sdk, delegator, validator, amount, false)?;
            let vault = sdk.context().contract_address();
            let source = chain_config_storage()
                .blend_reserve_accessor()
                .get_checked(sdk)?;
            safe_transfer_from(sdk, source, vault, amount)?;
        }
        pay_stipend(sdk, delegator, dust)?;
        events::Redelegated {
            validator,
            staker: delegator,
            amount,
            dust,
            epoch: reward_before_epoch,
        }
        .emit(sdk)
    } else {
        pay_stipend(sdk, delegator, claimable)?;
        events::Claimed {
            validator,
            staker: delegator,
            amount: claimable,
            epoch: reward_before_epoch,
        }
        .emit(sdk)
    }
}

/// Moves an owed stipend from the BLEND reserve straight to `recipient`.
///
/// The money never enters this contract. Everything the contract holds is
/// somebody's deposit, and mixing the two made a balance check on this contract
/// look like a solvency check while measuring principal.
fn pay_stipend<SDK: SharedAPI>(
    sdk: &mut SDK,
    recipient: Address,
    amount: U256,
) -> Result<(), ExitCode> {
    if amount.is_zero() {
        return Ok(());
    }
    let source = chain_config_storage()
        .blend_reserve_accessor()
        .get_checked(sdk)?;
    safe_transfer_from(sdk, source, recipient, amount)
}

/// Pays out every undelegation whose term has matured, from this contract's own
/// balance.
fn withdraw_delegator_principal_before<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    delegator: Address,
    before_epoch: u64,
) -> Result<(), ExitCode> {
    let principal_before_epoch =
        capped_delegator_principal_epoch(sdk, validator, delegator, before_epoch)?;
    let principal = consume_delegator_principal(sdk, validator, delegator, principal_before_epoch)?;
    safe_transfer(sdk, delegator, principal)?;
    events::Claimed {
        validator,
        staker: delegator,
        amount: principal,
        epoch: principal_before_epoch,
    }
    .emit(sdk)
}

/// Public handler `0x426594b1` (`claimDelegatorFee`).
///
/// Claims all currently available delegator rewards.
pub fn claim_delegator_fee<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let validator = decode::<AddressCommand>(input)?.value;
    let delegator = sdk.context().contract_caller();
    claim_delegator_reward_before(sdk, validator, delegator, current_epoch(sdk)?, false)
}

/// Public handler `0xa789083d` (`getDelegatorPrincipal`).
///
/// Returns the undelegated principal a delegator can withdraw right now. The
/// other half of what `getDelegatorFee` used to return in one number, split with
/// the claim it mirrors — without it a delegator can see the reward owed but not
/// the deposit waiting for them.
pub fn get_delegator_principal<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let command = decode::<ValidatorDelegatorCommand>(input)?;
    let principal_before_epoch = capped_delegator_principal_epoch(
        sdk,
        command.validator,
        command.delegator,
        current_epoch(sdk)?,
    )?;
    write_abi(
        sdk,
        &delegator_principal_claimable(
            sdk,
            command.validator,
            command.delegator,
            principal_before_epoch,
        )?,
    )
}

/// Public handler `0xe75f359c` (`withdrawDelegatorPrincipal`).
///
/// Pays out undelegations whose term has matured. Separate from the reward claim
/// because the two draw on different money: this one on the deposits the
/// contract holds, the reward on the BLEND reserve. An unfunded reserve
/// therefore cannot block a withdrawal, and an empty withdrawal queue cannot
/// block a reward.
pub fn withdraw_delegator_principal<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let validator = decode::<AddressCommand>(input)?.value;
    let delegator = sdk.context().contract_caller();
    withdraw_delegator_principal_before(sdk, validator, delegator, current_epoch(sdk)?)
}

/// Public handler `0x8ecb3fc9` (`redelegateDelegatorFee`).
///
/// Claims available delegator rewards and redelegates them as stake.
pub fn redelegate_delegator_fee<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let validator = decode::<AddressCommand>(input)?.value;
    let delegator = sdk.context().contract_caller();
    claim_delegator_reward_before(sdk, validator, delegator, current_epoch(sdk)?, true)
}

/// Public handler `0x54c3e84b` (`getEpochRewards`).
///
/// Returns the aggregate staking rewards recorded for an epoch.
pub fn get_epoch_rewards<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let epoch = decode::<U64Command>(input)?.value;
    let (record, len) = consensus::committee_at(sdk, epoch)?;
    let seated = consensus_storage()
        .committee_records_accessor()
        .entry(record);
    let mut total = U256::ZERO;
    for index in 0..len {
        let validator = seated.at(index).get_checked(sdk)?;
        total = total
            .checked_add(U256::from(
                staking_storage()
                    .validator_snapshots_accessor()
                    .entry(validator)
                    .entry(epoch)
                    .total_blend_rewards_accessor()
                    .get_checked(sdk)?,
            ))
            .ok_or(ExitCode::IntegerOverflow)?;
    }
    write_abi(sdk, &total)
}

/// Records what `epoch` owes its committee, at its close, and moves no money.
///
/// The credit is an assignment, not an accumulation. `close_epoch` runs once per
/// epoch, and an overwrite makes a re-entry idempotent rather than resting on
/// that.
///
/// Nothing here settles, and there is no cursor behind it that could come back
/// to an epoch later: what this frame assigns is what the epoch is worth, for
/// good. The stipend is drawn off the BLEND reserve by each claim, one recipient
/// at a time.
pub(crate) fn accrue_epoch<SDK: SharedAPI>(
    sdk: &mut SDK,
    epoch: u64,
    recorded: u32,
) -> Result<(), ExitCode> {
    // An epoch that was never recorded at all is reachable — a stalled recorder,
    // a pre-activation prefix — and must not draw a full pot for no work.
    let assigned = if recorded == 0 {
        U256::ZERO
    } else {
        assign_epoch_shares(sdk, epoch)?
    };
    events::EpochBlendRewardsCommitted {
        epoch,
        blend_amount: assigned,
    }
    .emit(sdk)
}

/// Splits the epoch's pot across its committee and returns the total assigned.
///
/// Priced from the live config because the close *is* the moment the epoch is
/// priced. The pinned rate existed to stop a change between the epoch ending and
/// the epoch being paid from rewriting what it earned; there is no longer a gap
/// between those two for a change to land in.
///
/// The total is the sum of the floored shares, never the pot, so the remainder
/// of at most `n − 1` base units is simply never assigned.
///
/// The second loop re-reads each seat's validator rather than carrying it out of
/// the first, and that is measured rather than overlooked: carrying it saves 100
/// gas per seat, 5_100 across a full committee, 0.16% of this function
/// (`e2e/src/staking_cost.rs`, 2026-08-17). The slot is warm by then. Not worth
/// widening the intermediate vector for.
fn assign_epoch_shares<SDK: SharedAPI>(sdk: &mut SDK, epoch: u64) -> Result<U256, ExitCode> {
    let config = chain_config_storage();
    let pot = config.blend_stipend_per_epoch_accessor().get_checked(sdk)?;
    if pot.is_zero() {
        return Ok(U256::ZERO);
    }
    // The reserve is asked once, here, whether it can cover this epoch — and the
    // answer is final. Below the pot the epoch closes at zero FOREVER: nothing
    // revisits a closed epoch, so a reserve topped up afterwards funds later
    // epochs and never this one. All-or-nothing rather than pro rata.
    //
    // READ WHAT THIS DOES AND DOES NOT GUARANTEE. It is a SPOT reading of a pool
    // that is also the source of every unpaid claim from every earlier epoch,
    // and nothing here reserves, escrows or debits anything — the whole point of
    // this design is that the stipend never enters this contract, so there is no
    // place to hold it. So:
    //
    //   * It does NOT prove the epoch will be payable. Whatever it measures can
    //     leave the reserve before anyone claims, either by the holder moving it
    //     or by claims against older epochs. The credit written below is a
    //     record of what is owed, not a guarantee it can be collected.
    //   * It does NOT prove the reserve covers what is already owed. Two epochs
    //     can pass this test against the same tokens; the second to be claimed
    //     then finds the pull refused and waits for a top-up.
    //   * Because `claimValidatorFee` is permissionless and pays the OWNER,
    //     ANY caller can drain the reserve toward its floor on demand. A run of
    //     such calls immediately before a boundary makes the closing epoch score
    //     short and burn, at the cost of gas alone.
    //
    // The operational rule that makes the gate mean something is therefore held
    // off-chain: fund the reserve well above one epoch's pot. A gate that
    // measured "unpaid obligations plus this epoch" instead would need the
    // contract to track what has been claimed against every open epoch, which is
    // the accounting this design exists to remove.
    //
    // A read that fails counts as zero, which merges "the reserve is empty",
    // "the approval was revoked" and "the token did not answer" into one
    // outcome, and merges all three with the `pot.is_zero()` arm above and the
    // two below. That indistinguishability is accepted deliberately: the close
    // is a pre-execution system call, so the alternative to a quiet zero is a
    // chain halt, and the configuration that produces it is checked off-chain at
    // deployment.
    let reserve = config.blend_reserve_accessor().get_checked(sdk)?;
    if reserve_available(sdk, reserve)? < pot {
        return Ok(U256::ZERO);
    }
    let state = consensus_storage();
    let (record, len) = consensus::committee_at(sdk, epoch)?;
    if len == 0 {
        return Ok(U256::ZERO);
    }
    // Ring miss: forfeit this epoch's stipend and announce it. Never an error —
    // this runs inside the close, a pre-execution system call, so a propagated
    // one is an unrepairable chain halt. Never a silent zero either: three arms
    // in this function already return zero for legitimate reasons, and a fourth
    // that means "we lost the weights" must not be indistinguishable from them.
    let Some(frozen) = consensus::read_weights(sdk, epoch)? else {
        events::EpochWeightsUnavailable {
            epoch,
            members: len as u32,
        }
        .emit(sdk)?;
        return Ok(U256::ZERO);
    };
    let seated = state.committee_records_accessor().entry(record);
    // Weights are the ones frozen at commit time, not a live stake walk: the
    // committee was ranked and the leader drawn from this same vector, so a
    // stake change after the commit must not move anyone's share.
    let mut weights = vec![U256::ZERO; len as usize];
    let mut total_weight = U256::ZERO;
    for (index, frozen_weight) in frozen.into_iter().enumerate() {
        let validator = seated.at(index as u64).get_checked(sdk)?;
        if state
            .tombstoned_accessor()
            .entry(validator)
            .get_checked(sdk)?
        {
            continue;
        }
        // The RAW compact weight, not `expand_balance`, unlike `judge` and
        // `get_epoch_committee_with_stakes`. Harmless because the split is
        // pro-rata and the expansion is a uniform multiply that cancels — but a
        // real difference between functions that look alike, and tidying it into
        // consistency would move the magnitudes inside the overflow-checked
        // multiply below. Preserved deliberately.
        let weight = U256::from(frozen_weight);
        if weight.is_zero() {
            continue;
        }
        weights[index] = weight;
        total_weight = total_weight
            .checked_add(weight)
            .ok_or(ExitCode::IntegerOverflow)?;
    }
    if total_weight.is_zero() {
        return Ok(U256::ZERO);
    }
    let mut assigned = U256::ZERO;
    for (index, weight) in weights.into_iter().enumerate() {
        if weight.is_zero() {
            continue;
        }
        let share = pot.checked_mul(weight).ok_or(ExitCode::IntegerOverflow)? / total_weight;
        if share.is_zero() {
            continue;
        }
        let validator = seated.at(index as u64).get_checked(sdk)?;
        let snapshot = touch_snapshot_at_or_before(sdk, validator, epoch)?;
        snapshot.total_blend_rewards_accessor().set_checked(
            sdk,
            math::narrow_reward(share).ok_or(ExitCode::IntegerOverflow)?,
        )?;
        assigned = assigned
            .checked_add(share)
            .ok_or(ExitCode::IntegerOverflow)?;
    }
    Ok(assigned)
}
