//! Staking ownership, epoch reads, and validator lifecycle methods.

use crate::{
    consts::*,
    events, math,
    storage::{chain_config_storage, consensus_storage, staking_storage, ValidatorSnapshotStorage},
    types::{
        AddressAmountCommand, AddressCommand, AddressU16Command, RegisterValidatorCommand,
        TwoAddressesCommand, U64Command, ValidatorBlockCommand, ValidatorDelegatorCommand,
        ValidatorEpochCommand,
    },
    util::{
        current_epoch, current_epoch_at_block, decode, ensure_governance, ensure_initialized,
        ensure_mutable, ensure_non_payable, next_epoch, revert, revert_with, safe_transfer,
        safe_transfer_from, write_abi,
    },
};
use alloc::{vec, vec::Vec};
use fluentbase_sdk::{
    bytes::BytesMut, codec::SolidityABI, Address, Bytes, ContextReader, ExitCode, SharedAPI, U256,
};

fn address_arg(input: &[u8]) -> Result<Address, ExitCode> {
    Ok(decode::<AddressCommand>(input)?.value)
}

fn validator_status<SDK: SharedAPI>(sdk: &SDK, validator: Address) -> Result<u8, ExitCode> {
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
    record.first_snapshot_epoch_p1_accessor().set_checked(
        sdk,
        changed_at.checked_add(1).ok_or(ExitCode::IntegerOverflow)?,
    )?;
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
    let storage = staking_storage();
    if visible {
        storage
            .selection_roster_accessor()
            .push_checked(sdk, validator)?;
    }
    let membership = storage.selection_membership_accessor().entry(validator);
    membership.visible_accessor().set_checked(sdk, visible)?;
    membership.prev_visible_accessor().set_checked(sdk, false)?;
    membership
        .effective_from_accessor()
        .set_checked(sdk, since_epoch)?;
    membership.rostered_accessor().set_checked(sdk, visible)
}

fn ensure_rostered<SDK: SharedAPI>(sdk: &mut SDK, validator: Address) -> Result<(), ExitCode> {
    let storage = staking_storage();
    let membership = storage.selection_membership_accessor().entry(validator);
    if !membership.rostered_accessor().get_checked(sdk)? {
        storage
            .selection_roster_accessor()
            .push_checked(sdk, validator)?;
        membership.rostered_accessor().set_checked(sdk, true)?;
    }
    Ok(())
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
    if membership.effective_from_accessor().get_checked(sdk)? == effective {
        membership.visible_accessor().set_checked(sdk, visible)?;
    } else {
        let previous = membership.visible_accessor().get_checked(sdk)?;
        membership
            .prev_visible_accessor()
            .set_checked(sdk, previous)?;
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
    } else {
        membership.prev_visible_accessor().get_checked(sdk)
    }
}

pub(crate) fn selected_validators_at<SDK: SharedAPI>(
    sdk: &SDK,
    epoch: u64,
) -> Result<Vec<Address>, ExitCode> {
    let storage = staking_storage();
    let roster = storage.selection_roster_accessor();
    let len = roster.len_checked(sdk)?;
    let mut candidates = Vec::with_capacity(len as usize);
    for index in 0..len {
        let validator = roster.at(index).get_checked(sdk)?;
        if selection_visible_at(sdk, validator, epoch)? {
            candidates.push(validator);
        }
    }
    let cap = chain_config_storage()
        .active_validators_length_accessor()
        .get_checked(sdk)? as usize;
    top_k_by_stake_at(sdk, candidates, epoch, cap)
}

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
    top_k_by_stake_at(sdk, candidates, epoch, cap)
}

/// Match Solidity's partial selection sort exactly.
///
/// Equal-stake candidates retain roster order; an address tie-breaker would
/// choose a different committee at the top-k boundary.
fn top_k_by_stake_at<SDK: SharedAPI>(
    sdk: &SDK,
    candidates: Vec<Address>,
    epoch: u64,
    cap: usize,
) -> Result<Vec<Address>, ExitCode> {
    let mut candidates = candidates
        .into_iter()
        .map(|validator| Ok((validator, validator_total_at(sdk, validator, epoch)?)))
        .collect::<Result<Vec<_>, ExitCode>>()?;
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
    Ok(candidates
        .into_iter()
        .map(|(validator, _)| validator)
        .collect())
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

fn touch_validator_snapshot<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    epoch: u64,
) -> Result<ValidatorSnapshotStorage, ExitCode> {
    let storage = staking_storage();
    let snapshots = storage.validator_snapshots_accessor().entry(validator);
    let snapshot = snapshots.entry(epoch);
    if !snapshot
        .total_delegated_accessor()
        .get_checked(sdk)?
        .is_zero()
    {
        return Ok(snapshot);
    }

    let record = storage.validators_accessor().entry(validator);
    let changed_at = record.changed_at_accessor().get_checked(sdk)?;
    let previous = snapshots.entry(changed_at);
    snapshot
        .total_delegated_accessor()
        .set_checked(sdk, previous.total_delegated_accessor().get_checked(sdk)?)?;
    snapshot
        .commission_rate_accessor()
        .set_checked(sdk, previous.commission_rate_accessor().get_checked(sdk)?)?;
    insert_snapshot_epoch(sdk, validator, epoch)?;
    if epoch > changed_at {
        record.changed_at_accessor().set_checked(sdk, epoch)?;
    }
    Ok(snapshot)
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

/// Public handler `0x8da5cb5b` (`owner`).
///
/// Returns the staking contract owner.
pub fn owner<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &staking_storage().owner_accessor().get_checked(sdk)?)
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
/// Returns the validator's owner, status, stake, slash, jail, claim, and commission data.
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
        snapshot.slashes_count_accessor().get_checked(sdk)?,
        changed_at,
        record.jailed_before_accessor().get_checked(sdk)?,
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

/// Public handler `0x4d238c8e` (`addValidator`).
///
/// Adds an active validator under governance control.
pub fn add_validator<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_governance(sdk)?;
    let validator = address_arg(input)?;
    let changed_at = next_epoch(sdk)?;
    set_validator(
        sdk,
        validator,
        validator,
        STATUS_ACTIVE,
        0,
        U256::ZERO,
        changed_at,
    )
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
    let storage = staking_storage();
    storage
        .validators_accessor()
        .entry(validator)
        .status_accessor()
        .set_checked(sdk, STATUS_ACTIVE)?;
    storage
        .active_validators_accessor()
        .push_checked(sdk, validator)?;
    ensure_rostered(sdk, validator)?;
    set_selection_visible(sdk, validator, true, current_epoch(sdk)?)?;
    touch_validator_snapshot(sdk, validator, next_epoch(sdk)?)?;
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
    remove_active(sdk, validator)?;
    staking_storage()
        .validators_accessor()
        .entry(validator)
        .status_accessor()
        .set_checked(sdk, STATUS_PENDING)?;
    set_selection_visible(sdk, validator, false, current_epoch(sdk)?)?;
    touch_validator_snapshot(sdk, validator, next_epoch(sdk)?)?;
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
    let snapshot = touch_validator_snapshot(sdk, command.validator, changed_at)?;
    snapshot
        .commission_rate_accessor()
        .set_checked(sdk, command.value)?;
    emit_modified(sdk, command.validator)
}

/// Public handler `0x0052c9e1` (`changeValidatorOwner`).
///
/// Rejects validator-owner changes because validator ownership is immutable.
pub fn change_owner<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let command: TwoAddressesCommand = decode(input)?;
    let owner = staking_storage()
        .validators_accessor()
        .entry(command.validator)
        .owner_accessor()
        .get_checked(sdk)?;
    if owner.is_zero() {
        return revert_with(sdk, ERR_VALIDATOR_NOT_FOUND, &command.validator);
    }
    if owner != sdk.context().contract_caller() {
        return revert_with(sdk, ERR_ONLY_VALIDATOR_OWNER, &owner);
    }
    revert(sdk, ERR_VALIDATOR_OWNER_IMMUTABLE)
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

/// Public handler `0xdd0fb5df` (`registerValidator`).
///
/// Registers a pending validator and pulls its initial self-stake.
pub fn register_validator<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let command: RegisterValidatorCommand = decode(input)?;
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
    safe_transfer_from(sdk, owner, command.initial_stake)
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

    // New stake affects accounting only after the warm-up delay.
    let at_epoch = current_epoch(sdk)?
        .checked_add(WARMUP_DELAY)
        .ok_or(ExitCode::IntegerOverflow)?;
    let snapshot = touch_validator_snapshot(sdk, validator, at_epoch)?;
    let next_total = snapshot
        .total_delegated_accessor()
        .get_checked(sdk)?
        .checked_add(compact_amount)
        .ok_or(ExitCode::IntegerOverflow)?;
    snapshot
        .total_delegated_accessor()
        .set_checked(sdk, next_total)?;

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
        safe_transfer_from(sdk, delegator, amount)?;
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
    let minimum = config.min_staking_amount_accessor().get_checked(sdk)?;
    if amount.is_zero() || amount < minimum {
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

    let snapshot = touch_validator_snapshot(sdk, validator, before_epoch)?;
    let total = snapshot.total_delegated_accessor().get_checked(sdk)?;
    let Some(next_total) = total.checked_sub(compact_amount) else {
        return revert(sdk, ERR_INSUFFICIENT_BALANCE);
    };

    let owner = record.owner_accessor().get_checked(sdk)?;
    if delegator == owner && (status == STATUS_ACTIVE || status == STATUS_PENDING) {
        let min_validator_stake = config
            .min_validator_stake_amount_accessor()
            .get_checked(sdk)?;
        if math::expand_balance(next_delegated) < min_validator_stake
            && (!next_delegated.is_zero() || next_total != next_delegated)
        {
            return revert(sdk, ERR_OWNER_SELF_STAKE_BELOW_MINIMUM);
        }
    }

    snapshot
        .total_delegated_accessor()
        .set_checked(sdk, next_total)?;
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

    events::Undelegated {
        validator,
        staker: delegator,
        amount,
        epoch: before_epoch,
    }
    .emit(sdk)?;
    Ok(())
}
// Snapshot rewards, bounded claims, and finalized stipend settlement.

fn external_call<SDK, T>(
    sdk: &mut SDK,
    target: Address,
    selector: u32,
    params: &T,
) -> Result<Bytes, ExitCode>
where
    SDK: SharedAPI,
    T: fluentbase_sdk::codec::Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    let mut encoded = BytesMut::new();
    SolidityABI::<T>::encode(params, &mut encoded, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    let mut input = selector.to_be_bytes().to_vec();
    input.extend_from_slice(&encoded);
    let result = sdk.call(target, U256::ZERO, &input, None);
    if !result.status.is_ok() {
        sdk.write(result.data);
        return Err(result.status);
    }
    Ok(result.data)
}

fn call_decode<SDK, T, R>(
    sdk: &mut SDK,
    target: Address,
    selector: u32,
    params: &T,
) -> Result<R, ExitCode>
where
    SDK: SharedAPI,
    T: fluentbase_sdk::codec::Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
    R: fluentbase_sdk::codec::Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    let output = external_call(sdk, target, selector, params)?;
    SolidityABI::<R>::decode(&output, 0).map_err(|_| ExitCode::MalformedBuiltinParams)
}

fn snapshot_payout<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    epoch: u64,
) -> Result<(U256, U256), ExitCode> {
    let snapshot = staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(epoch);
    let total_reward = U256::from(snapshot.total_blend_rewards_accessor().get_checked(sdk)?);
    if total_reward.is_zero() {
        return Ok((U256::ZERO, U256::ZERO));
    }
    if snapshot
        .total_delegated_accessor()
        .get_checked(sdk)?
        .is_zero()
    {
        return Ok((U256::ZERO, total_reward));
    }
    let owner_reward = total_reward
        .checked_mul(U256::from(
            snapshot.commission_rate_accessor().get_checked(sdk)?,
        ))
        .ok_or(ExitCode::IntegerOverflow)?
        / U256::from(10_000);
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

fn delegator_claimable<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    delegator: Address,
    before_epoch: u64,
) -> Result<U256, ExitCode> {
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator);
    let delegates = delegation.delegate_queue_accessor();
    let delegate_len = delegates.len_checked(sdk)?;
    let mut delegate_gap = delegation.delegate_gap_accessor().get_checked(sdk)?;
    let mut claimable = U256::ZERO;

    while delegate_gap < delegate_len {
        let operation = delegates.at(delegate_gap);
        let mut epoch = operation.epoch_accessor().get_checked(sdk)?;
        if epoch >= before_epoch {
            break;
        }
        let changed_at = if delegate_gap + 1 < delegate_len {
            delegates
                .at(delegate_gap + 1)
                .epoch_accessor()
                .get_checked(sdk)?
        } else {
            before_epoch
        };
        let end = core::cmp::min(before_epoch, changed_at);
        let delegated = operation.amount_accessor().get_checked(sdk)?;
        while epoch < end {
            let (delegator_pool, _) = snapshot_payout(sdk, validator, epoch)?;
            let snapshot = staking_storage()
                .validator_snapshots_accessor()
                .entry(validator)
                .entry(epoch);
            let total = snapshot.total_delegated_accessor().get_checked(sdk)?;
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
        delegate_gap += 1;
    }

    let undelegates = delegation.undelegate_queue_accessor();
    let undelegate_len = undelegates.len_checked(sdk)?;
    let mut undelegate_gap = delegation.undelegate_gap_accessor().get_checked(sdk)?;
    let self_stake_locked = validator_self_stake_is_locked(sdk, validator, delegator)?;
    while undelegate_gap < undelegate_len {
        let operation = undelegates.at(undelegate_gap);
        if self_stake_locked || operation.epoch_accessor().get_checked(sdk)? > before_epoch {
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

fn validator_self_stake_is_locked<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    delegator: Address,
) -> Result<bool, ExitCode> {
    let record = staking_storage().validators_accessor().entry(validator);
    if record.owner_accessor().get_checked(sdk)? != delegator {
        return Ok(false);
    }
    let committee_epoch_p1 = record.last_committee_epoch_p1_accessor().get_checked(sdk)?;
    if committee_epoch_p1 == 0 {
        return Ok(false);
    }
    Ok(consensus_storage()
        .epoch_committees_accessor()
        .entry(committee_epoch_p1 - 1)
        .len_checked(sdk)?
        != 0)
}

fn capped_delegator_claim_epoch<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    delegator: Address,
    before_epoch: u64,
) -> Result<u64, ExitCode> {
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator);
    let delegates = delegation.delegate_queue_accessor();
    let delegate_gap = delegation.delegate_gap_accessor().get_checked(sdk)?;
    let first = if delegate_gap < delegates.len_checked(sdk)? {
        Some(
            delegates
                .at(delegate_gap)
                .epoch_accessor()
                .get_checked(sdk)?,
        )
    } else {
        let undelegates = delegation.undelegate_queue_accessor();
        let undelegate_gap = delegation.undelegate_gap_accessor().get_checked(sdk)?;
        if undelegate_gap < undelegates.len_checked(sdk)? {
            Some(
                undelegates
                    .at(undelegate_gap)
                    .epoch_accessor()
                    .get_checked(sdk)?,
            )
        } else {
            None
        }
    };
    let Some(first) = first else {
        return Ok(before_epoch);
    };
    Ok(core::cmp::min(
        before_epoch,
        first
            .checked_add(MAX_EPOCHS_PER_CLAIM)
            .ok_or(ExitCode::IntegerOverflow)?,
    ))
}

fn consume_delegator_claim<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    delegator: Address,
    before_epoch: u64,
) -> Result<U256, ExitCode> {
    let storage = staking_storage();
    let delegation = storage
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator);
    let delegates = delegation.delegate_queue_accessor();
    let delegate_len = delegates.len_checked(sdk)?;
    let mut delegate_gap = delegation.delegate_gap_accessor().get_checked(sdk)?;
    let mut claimable = U256::ZERO;

    while delegate_gap < delegate_len {
        let operation = delegates.at(delegate_gap);
        let mut epoch = operation.epoch_accessor().get_checked(sdk)?;
        if epoch >= before_epoch {
            break;
        }
        let has_next = delegate_gap + 1 < delegate_len;
        let changed_at = if has_next {
            delegates
                .at(delegate_gap + 1)
                .epoch_accessor()
                .get_checked(sdk)?
        } else {
            before_epoch
        };
        let end = core::cmp::min(before_epoch, changed_at);
        let delegated = operation.amount_accessor().get_checked(sdk)?;
        while epoch < end {
            let (delegator_pool, _) = snapshot_payout(sdk, validator, epoch)?;
            let snapshot = storage
                .validator_snapshots_accessor()
                .entry(validator)
                .entry(epoch);
            let total = snapshot.total_delegated_accessor().get_checked(sdk)?;
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
        if !has_next || epoch < changed_at {
            operation.epoch_accessor().set_checked(sdk, epoch)?;
            break;
        }
        delegate_gap += 1;
    }
    delegation
        .delegate_gap_accessor()
        .set_checked(sdk, delegate_gap)?;

    let undelegates = delegation.undelegate_queue_accessor();
    let undelegate_len = undelegates.len_checked(sdk)?;
    let mut undelegate_gap = delegation.undelegate_gap_accessor().get_checked(sdk)?;
    let pending_undelegated = delegation.pending_undelegated_accessor();
    let mut pending_principal = pending_undelegated.get_checked(sdk)?;
    let self_stake_locked = validator_self_stake_is_locked(sdk, validator, delegator)?;
    while undelegate_gap < undelegate_len {
        let operation = undelegates.at(undelegate_gap);
        if self_stake_locked || operation.epoch_accessor().get_checked(sdk)? > before_epoch {
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

/// Public handler `0xc6fb9065` (`getPendingValidatorFee`).
///
/// Returns the validator owner's rewards including the pending epoch.
pub fn get_pending_validator_fee<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let validator = decode::<AddressCommand>(input)?.value;
    write_abi(
        sdk,
        &validator_owner_rewards(sdk, validator, next_epoch(sdk)?)?,
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
    safe_transfer(sdk, owner, amount)?;
    events::ValidatorOwnerClaimed {
        validator,
        amount,
        epoch: capped,
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
    let before_epoch = capped_delegator_claim_epoch(
        sdk,
        command.validator,
        command.delegator,
        current_epoch(sdk)?,
    )?;
    write_abi(
        sdk,
        &delegator_claimable(sdk, command.validator, command.delegator, before_epoch)?,
    )
}

/// Public handler `0xc2fd58fc` (`getPendingDelegatorFee`).
///
/// Returns a delegator's rewards including the pending epoch.
pub fn get_pending_delegator_fee<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let command = decode::<ValidatorDelegatorCommand>(input)?;
    let before_epoch =
        capped_delegator_claim_epoch(sdk, command.validator, command.delegator, next_epoch(sdk)?)?;
    write_abi(
        sdk,
        &delegator_claimable(sdk, command.validator, command.delegator, before_epoch)?,
    )
}

fn claim_delegator_before<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    delegator: Address,
    before_epoch: u64,
    redelegate: bool,
) -> Result<(), ExitCode> {
    let capped = capped_delegator_claim_epoch(sdk, validator, delegator, before_epoch)?;
    let claimable = consume_delegator_claim(sdk, validator, delegator, capped)?;
    if redelegate {
        let (amount, dust) = available_for_redelegate(sdk, claimable)?;
        if !amount.is_zero() {
            delegate_to(sdk, delegator, validator, amount, false)?;
        }
        safe_transfer(sdk, delegator, dust)?;
        events::Redelegated {
            validator,
            staker: delegator,
            amount,
            dust,
            epoch: capped,
        }
        .emit(sdk)
    } else {
        safe_transfer(sdk, delegator, claimable)?;
        events::Claimed {
            validator,
            staker: delegator,
            amount: claimable,
            epoch: capped,
        }
        .emit(sdk)
    }
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
    claim_delegator_before(sdk, validator, delegator, current_epoch(sdk)?, false)
}

/// Public handler `0xfe38ebef` (`claimDelegatorFeeAtEpoch`).
///
/// Claims delegator rewards through the requested epoch.
pub fn claim_delegator_fee_at_epoch<SDK: SharedAPI>(
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
    let delegator = sdk.context().contract_caller();
    claim_delegator_before(
        sdk,
        command.validator,
        delegator,
        command.before_epoch,
        false,
    )
}

/// Public handler `0x5ef9e8c6` (`calcAvailableForRedelegateAmount`).
///
/// Returns the claimable rewards that can be redelegated.
pub fn calc_available_for_redelegate_amount<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let command = decode::<ValidatorDelegatorCommand>(input)?;
    let before_epoch = capped_delegator_claim_epoch(
        sdk,
        command.validator,
        command.delegator,
        current_epoch(sdk)?,
    )?;
    let claimable = delegator_claimable(sdk, command.validator, command.delegator, before_epoch)?;
    write_abi(sdk, &available_for_redelegate(sdk, claimable)?)
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
    claim_delegator_before(sdk, validator, delegator, current_epoch(sdk)?, true)
}

/// Public handler `0x54c3e84b` (`getEpochRewards`).
///
/// Returns the aggregate staking rewards recorded for an epoch.
pub fn get_epoch_rewards<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let epoch = decode::<U64Command>(input)?.value;
    let committee = consensus_storage().epoch_committees_accessor().entry(epoch);
    let len = committee.len_checked(sdk)?;
    let mut total = U256::ZERO;
    for index in 0..len {
        let validator = committee.at(index).get_checked(sdk)?;
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

fn fault_tolerance(n: usize) -> usize {
    if n == 0 {
        0
    } else {
        (n - 1) / 3
    }
}

fn settle_one<SDK: SharedAPI>(
    sdk: &mut SDK,
    epoch: u64,
    liveness: Address,
    reserve: Address,
) -> Result<(), ExitCode> {
    let storage = staking_storage();
    let consensus = consensus_storage();
    let committee = consensus.epoch_committees_accessor().entry(epoch);
    let len = committee.len_checked(sdk)?;
    let desired = chain_config_storage()
        .blend_stipend_per_epoch_accessor()
        .get_checked(sdk)?;
    if desired.is_zero() {
        events::EpochBlendRewardsCommitted {
            epoch,
            blend_amount: U256::ZERO,
        }
        .emit(sdk)?;
        return Ok(());
    }
    let reserve_balance = call_decode::<_, _, U256>(sdk, reserve, SIG_RESERVE_BALANCE, &())?;
    let pot = core::cmp::min(desired, reserve_balance);
    let mut assigned = U256::ZERO;
    let mut shares = vec![U256::ZERO; len as usize];

    if !pot.is_zero() && len != 0 {
        let (_, certs) =
            call_decode::<_, _, (u32, u32)>(sdk, liveness, SIG_PARTICIPATION, &(epoch, 0u32))?;
        if certs != 0 {
            let floor = chain_config_storage()
                .participation_floor_bps_accessor()
                .get_checked(sdk)?;
            let floor = if floor == 0 {
                DEFAULT_PARTICIPATION_FLOOR_BPS
            } else {
                floor
            };
            let mut passed = vec![false; len as usize];
            let mut below = 0usize;
            for index in 0..len {
                let (seen, _) = call_decode::<_, _, (u32, u32)>(
                    sdk,
                    liveness,
                    SIG_PARTICIPATION,
                    &(epoch, index as u32),
                )?;
                if U256::from(seen) * U256::from(10_000) < U256::from(certs) * U256::from(floor) {
                    below += 1;
                } else {
                    passed[index as usize] = true;
                }
            }
            // During a partition, keep stake-weighted rewards available to
            // participating and non-participating committee members alike.
            let partition = below > fault_tolerance(len as usize);
            let mut stakes = vec![U256::ZERO; len as usize];
            let mut total_stake = U256::ZERO;
            for index in 0..len {
                let validator = committee.at(index).get_checked(sdk)?;
                if (!partition && !passed[index as usize])
                    || consensus
                        .tombstoned_accessor()
                        .entry(validator)
                        .get_checked(sdk)?
                    || storage
                        .validators_accessor()
                        .entry(validator)
                        .status_accessor()
                        .get_checked(sdk)?
                        == STATUS_NOT_FOUND
                {
                    continue;
                }
                let stake = validator_total_at(sdk, validator, epoch)?;
                if stake.is_zero() {
                    continue;
                }
                stakes[index as usize] = stake;
                total_stake = total_stake
                    .checked_add(stake)
                    .ok_or(ExitCode::IntegerOverflow)?;
            }
            if !total_stake.is_zero() {
                for (index, stake) in stakes.into_iter().enumerate() {
                    if stake.is_zero() {
                        continue;
                    }
                    let share =
                        pot.checked_mul(stake).ok_or(ExitCode::IntegerOverflow)? / total_stake;
                    shares[index] = share;
                    assigned = assigned
                        .checked_add(share)
                        .ok_or(ExitCode::IntegerOverflow)?;
                }
            }
        }
    }

    let mut credited_amount = assigned;
    if !assigned.is_zero() {
        let recipient = sdk.context().contract_address();
        let sent =
            call_decode::<_, _, U256>(sdk, reserve, SIG_RESERVE_DISBURSE, &(recipient, assigned))?;
        if sent != assigned {
            credited_amount = U256::ZERO;
        }
    }
    if !credited_amount.is_zero() {
        let mut credited_this_epoch = U256::ZERO;
        for (index, share) in shares.into_iter().enumerate() {
            if share.is_zero() {
                continue;
            }
            let validator = committee.at(index as u64).get_checked(sdk)?;
            let snapshot = touch_snapshot_at_or_before(sdk, validator, epoch)?;
            let share = crate::math::narrow_reward(share).ok_or(ExitCode::IntegerOverflow)?;
            let next = snapshot
                .total_blend_rewards_accessor()
                .get_checked(sdk)?
                .checked_add(share)
                .ok_or(ExitCode::IntegerOverflow)?;
            snapshot
                .total_blend_rewards_accessor()
                .set_checked(sdk, next)?;
            credited_this_epoch = credited_this_epoch
                .checked_add(U256::from(share))
                .ok_or(ExitCode::IntegerOverflow)?;
        }
        let credited = storage.credited_blend_accessor().get_checked(sdk)?;
        storage.credited_blend_accessor().set_checked(
            sdk,
            credited
                .checked_add(credited_this_epoch)
                .ok_or(ExitCode::IntegerOverflow)?,
        )?;
    } else {
        events::StipendSkipped { epoch }.emit(sdk)?;
    }
    events::EpochBlendRewardsCommitted {
        epoch,
        blend_amount: credited_amount,
    }
    .emit(sdk)
}

/// Public handler `0xa631344a` (`settleEpochStipend`).
///
/// Settles and distributes the finalized epoch stipend.
pub fn settle_epoch_stipend<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    if sdk.context().contract_caller() != SYSTEM_CALLER {
        return revert(sdk, ERR_ONLY_SYSTEM_CALL);
    }
    let requested = decode::<U64Command>(input)?.value;
    let storage = staking_storage();
    let config = chain_config_storage();
    let liveness = config.liveness_slashing_accessor().get_checked(sdk)?;
    let reserve = config.blend_reserve_accessor().get_checked(sdk)?;
    let finalized_p1 = call_decode::<_, _, u64>(sdk, liveness, SIG_LAST_FINALIZED_EPOCH_P1, &())?;
    if finalized_p1 == 0 {
        return Ok(());
    }
    let up_to = core::cmp::min(requested, finalized_p1 - 1);
    let first = storage.last_rewarded_epoch_p1_accessor().get_checked(sdk)?;
    if first != 0 && up_to.checked_add(1).ok_or(ExitCode::IntegerOverflow)? <= first {
        return Ok(());
    }
    let mut epoch = first;
    let mut settled = 0;
    while epoch <= up_to && settled < MAX_SETTLE_CATCHUP {
        settle_one(sdk, epoch, liveness, reserve)?;
        storage
            .last_rewarded_epoch_p1_accessor()
            .set_checked(sdk, epoch.checked_add(1).ok_or(ExitCode::IntegerOverflow)?)?;
        epoch = epoch.checked_add(1).ok_or(ExitCode::IntegerOverflow)?;
        settled += 1;
    }
    Ok(())
}
