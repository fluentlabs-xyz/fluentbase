//! Validator registration and BLEND delegation principal accounting.
//!
//! Balance changes are recorded as epoch-stamped checkpoints.

use crate::{
    consts::*,
    events, math,
    storage::{
        current_epoch, current_epoch_at_block, staking_storage, STATUS_ACTIVE, STATUS_NOT_FOUND,
        STATUS_PENDING,
    },
    types::{
        AddressAmountCommand, RegisterValidatorCommand, ValidatorBlockCommand,
        ValidatorDelegatorCommand,
    },
    util::{
        decode, ensure_initialized, ensure_mutable, ensure_non_payable, next_epoch, revert,
        revert_with, safe_transfer_from, set_validator, touch_validator_snapshot,
        validator_total_at, write_abi,
    },
};
use fluentbase_sdk::{Address, ContextReader, ExitCode, SharedAPI, U256};

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
    let minimum = staking_storage()
        .config_accessor()
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

pub fn delegate<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let command: AddressAmountCommand = decode(input)?;
    let delegator = sdk.context().contract_caller();
    delegate_to(sdk, delegator, command.validator, command.amount, true)
}

pub fn delegate_to<SDK: SharedAPI>(
    sdk: &mut SDK,
    delegator: Address,
    validator: Address,
    amount: U256,
    pull_tokens: bool,
) -> Result<(), ExitCode> {
    let storage = staking_storage();
    let minimum = storage
        .config_accessor()
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

pub fn undelegate<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let command: AddressAmountCommand = decode(input)?;
    let delegator = sdk.context().contract_caller();
    undelegate_from(sdk, delegator, command.validator, command.amount)
}

pub fn undelegate_from<SDK: SharedAPI>(
    sdk: &mut SDK,
    delegator: Address,
    validator: Address,
    amount: U256,
) -> Result<(), ExitCode> {
    let storage = staking_storage();
    let config = storage.config_accessor();
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

    let before_epoch = next_epoch(sdk)?;
    let snapshot = touch_validator_snapshot(sdk, validator, before_epoch)?;
    let total = snapshot.total_delegated_accessor().get_checked(sdk)?;
    let Some(next_total) = total.checked_sub(compact_amount) else {
        return revert(sdk, ERR_INSUFFICIENT_BALANCE);
    };

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
    let delegated = latest.amount_accessor().get_checked(sdk)?;
    let Some(next_delegated) = delegated.checked_sub(compact_amount) else {
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
    if latest.epoch_accessor().get_checked(sdk)? >= before_epoch {
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

    events::Undelegated {
        validator,
        staker: delegator,
        amount,
        epoch: before_epoch,
    }
    .emit(sdk)?;
    Ok(())
}
