//! Initialization, compatibility getters, and governance validator lifecycle.

use crate::{
    consts::*,
    events,
    storage::{
        current_epoch, remove_active, staking_storage, STATUS_ACTIVE, STATUS_NOT_FOUND,
        STATUS_PENDING,
    },
    types::{AddressU16Command, ConfigureCommand, InitializeCommand, TwoAddressesCommand},
    util::{
        address_arg, decode, decode_args, emit_modified, ensure_governance, ensure_initialized,
        ensure_mutable, ensure_non_payable, ensure_rostered, next_epoch, revert, revert_with,
        selected_validators, set_selection_visible, set_validator, touch_validator_snapshot,
        validator_status, write_abi,
    },
};
use alloc::vec::Vec;
use fluentbase_sdk::{Address, ContextReader, ExitCode, SharedAPI, U256};

pub fn initialize<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    let storage = staking_storage();
    if storage.initialized_accessor().get_checked(sdk)? {
        return revert(sdk, ERR_ALREADY_INITIALIZED);
    }
    let (initial_owner, validators, initial_stakes, commission_rate) =
        decode_args::<(Address, Vec<Address>, Vec<U256>, u16)>(input)?;
    let command = InitializeCommand {
        initial_owner,
        validators,
        initial_stakes,
        commission_rate,
    };
    if command.initial_owner.is_zero() {
        return revert(sdk, ERR_ZERO_OWNER);
    }
    if command.validators.len() != command.initial_stakes.len() {
        return revert(sdk, ERR_MALFORMED_INPUT_LENGTH);
    }
    if command.commission_rate > COMMISSION_RATE_MAX {
        return revert_with(sdk, ERR_BAD_COMMISSION_RATE, &command.commission_rate);
    }
    storage
        .owner_accessor()
        .set_checked(sdk, command.initial_owner)?;
    let config = storage.config_accessor();
    config
        .governance_accessor()
        .set_checked(sdk, fluentbase_sdk::GENESIS_GOVERNANCE)?;
    if !config.configured_accessor().get_checked(sdk)? {
        let activation_block = sdk.context().block_number();
        config
            .dpos_activation_block_accessor()
            .set_checked(sdk, activation_block)?;
        config
            .epoch_block_interval_accessor()
            .set_checked(sdk, DEFAULT_EPOCH_BLOCK_INTERVAL)?;
        config
            .undelegate_period_accessor()
            .set_checked(sdk, DEFAULT_UNDELEGATE_PERIOD)?;
        config
            .active_validators_length_accessor()
            .set_checked(sdk, DEFAULT_ACTIVE_VALIDATORS_LENGTH)?;
        config
            .min_validator_stake_amount_accessor()
            .set_checked(sdk, DEFAULT_MIN_VALIDATOR_STAKE)?;
        config
            .min_staking_amount_accessor()
            .set_checked(sdk, DEFAULT_MIN_STAKING_AMOUNT)?;
        config
            .felony_threshold_accessor()
            .set_checked(sdk, DEFAULT_FELONY_THRESHOLD)?;
        config
            .validator_jail_epoch_length_accessor()
            .set_checked(sdk, DEFAULT_VALIDATOR_JAIL_EPOCH_LENGTH)?;
    }

    let mut total_stakes = U256::ZERO;
    for (validator, stake) in command.validators.into_iter().zip(command.initial_stakes) {
        total_stakes = total_stakes
            .checked_add(stake)
            .ok_or(ExitCode::IntegerOverflow)?;
        set_validator(
            sdk,
            validator,
            validator,
            STATUS_ACTIVE,
            command.commission_rate,
            stake,
            0,
        )?;
    }
    // Match Solidity's `initializer` reentrancy guard before the first
    // external token call. A failed call reverts the transaction and storage.
    storage.initialized_accessor().set_checked(sdk, true)?;
    pull_initial_stakes(sdk, command.initial_owner, total_stakes)?;
    Ok(())
}

#[cfg(not(test))]
fn pull_initial_stakes<SDK: SharedAPI>(
    sdk: &mut SDK,
    owner: Address,
    total_stakes: U256,
) -> Result<(), ExitCode> {
    if total_stakes.is_zero() {
        return Ok(());
    }
    crate::util::safe_transfer_from(sdk, owner, total_stakes)
}

// Unit storage tests use a host without nested-call support. The real genesis
// integration test exercises the ERC-20 pull through the compiled rWasm.
#[cfg(test)]
fn pull_initial_stakes<SDK: SharedAPI>(
    _sdk: &mut SDK,
    _owner: Address,
    _total_stakes: U256,
) -> Result<(), ExitCode> {
    Ok(())
}

/// Configure the BLEND token and the chain parameters now owned by staking.
///
/// This additive rWasm ABI replaces reads from the former `ChainConfig`
/// contract. Genesis may call this before `initialize` so non-zero initial
/// stakes are pulled by the original initializer ABI. Zero-stake deployments
/// may initialize first and configure immediately afterwards.
pub fn configure<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    let storage = staking_storage();
    let caller = sdk.context().contract_caller();
    let owner = storage.owner_accessor().get_checked(sdk)?;
    if owner.is_zero() {
        if caller != fluentbase_sdk::GENESIS_GOVERNANCE {
            return revert_with(sdk, ERR_ONLY_OWNER, &caller);
        }
    } else if caller != owner {
        return revert_with(sdk, ERR_ONLY_OWNER, &caller);
    }
    let config = storage.config_accessor();
    if config.configured_accessor().get_checked(sdk)? {
        return revert(sdk, ERR_ALREADY_CONFIGURED);
    }
    let command: ConfigureCommand = decode(input)?;
    if command.staking_token.is_zero() {
        return revert(sdk, ERR_ZERO_STAKING_TOKEN);
    }
    if command.active_validators_length == 0
        || command.active_validators_length as u64 > MAX_ACTIVE_VALIDATORS_LENGTH
        || command.epoch_block_interval == 0
        || command.felony_threshold == 0
        || command.validator_jail_epoch_length == 0
        || command.undelegate_period == 0
        || command.min_validator_stake_amount.is_zero()
        || command.min_staking_amount.is_zero()
        || crate::math::compact_balance(command.min_validator_stake_amount).is_none()
        || crate::math::compact_balance(command.min_staking_amount).is_none()
    {
        return revert(sdk, ERR_INVALID_CHAIN_CONFIG);
    }
    if !command
        .dpos_activation_block
        .is_multiple_of(command.epoch_block_interval as u64)
    {
        return revert(sdk, ERR_UNALIGNED_ACTIVATION_BLOCK);
    }
    let undelegate_window = U256::from(command.undelegate_period)
        .checked_mul(U256::from(command.epoch_block_interval))
        .ok_or(ExitCode::IntegerOverflow)?;
    if undelegate_window < command.min_undelegate_blocks {
        return revert_with(
            sdk,
            ERR_UNDELEGATE_WINDOW_TOO_SHORT,
            &(undelegate_window, command.min_undelegate_blocks),
        );
    }

    config
        .staking_token_accessor()
        .set_checked(sdk, command.staking_token)?;
    config
        .active_validators_length_accessor()
        .set_checked(sdk, command.active_validators_length as u64)?;
    config
        .epoch_block_interval_accessor()
        .set_checked(sdk, command.epoch_block_interval as u64)?;
    config
        .felony_threshold_accessor()
        .set_checked(sdk, command.felony_threshold)?;
    config
        .validator_jail_epoch_length_accessor()
        .set_checked(sdk, command.validator_jail_epoch_length)?;
    config
        .undelegate_period_accessor()
        .set_checked(sdk, command.undelegate_period as u64)?;
    config
        .min_validator_stake_amount_accessor()
        .set_checked(sdk, command.min_validator_stake_amount)?;
    config
        .min_staking_amount_accessor()
        .set_checked(sdk, command.min_staking_amount)?;
    config
        .min_undelegate_blocks_accessor()
        .set_checked(sdk, command.min_undelegate_blocks)?;
    config
        .dpos_activation_block_accessor()
        .set_checked(sdk, command.dpos_activation_block)?;
    if !command.bls_verifier.is_zero() {
        config
            .bls_verifier_accessor()
            .set_checked(sdk, command.bls_verifier)?;
    }
    if !command.evidence_decoder.is_zero() {
        config
            .evidence_decoder_accessor()
            .set_checked(sdk, command.evidence_decoder)?;
    }
    config.configured_accessor().set_checked(sdk, true)?;

    events::ActiveValidatorsLengthChanged {
        prev_value: 0,
        new_value: command.active_validators_length,
    }
    .emit(sdk)?;
    events::EpochBlockIntervalChanged {
        prev_value: 0,
        new_value: command.epoch_block_interval,
    }
    .emit(sdk)?;
    events::FelonyThresholdChanged {
        prev_value: 0,
        new_value: command.felony_threshold,
    }
    .emit(sdk)?;
    events::ValidatorJailEpochLengthChanged {
        prev_value: 0,
        new_value: command.validator_jail_epoch_length,
    }
    .emit(sdk)?;
    events::UndelegatePeriodChanged {
        prev_value: 0,
        new_value: command.undelegate_period,
    }
    .emit(sdk)?;
    events::MinValidatorStakeAmountChanged {
        prev_value: U256::ZERO,
        new_value: command.min_validator_stake_amount,
    }
    .emit(sdk)?;
    events::MinStakingAmountChanged {
        prev_value: U256::ZERO,
        new_value: command.min_staking_amount,
    }
    .emit(sdk)?;
    events::DposActivationBlockChanged {
        prev_value: 0,
        new_value: command.dpos_activation_block,
    }
    .emit(sdk)?;
    if !command.bls_verifier.is_zero() {
        events::BlsVerifierChanged {
            prev_value: Address::ZERO,
            new_value: command.bls_verifier,
        }
        .emit(sdk)?;
    }
    if !command.evidence_decoder.is_zero() {
        events::EvidenceDecoderChanged {
            prev_value: Address::ZERO,
            new_value: command.evidence_decoder,
        }
        .emit(sdk)?;
    }
    Ok(())
}

pub fn current_epoch_read<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    write_abi(sdk, &current_epoch(sdk)?)
}

pub fn next_epoch_read<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    write_abi(sdk, &next_epoch(sdk)?)
}

pub fn owner<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &staking_storage().owner_accessor().get_checked(sdk)?)
}

pub fn get_staking<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let staking = sdk.context().contract_address();
    write_abi(sdk, &staking)
}

pub fn get_chain_config<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    // Chain configuration is embedded in staking, so the compatibility getter
    // resolves to this contract.
    get_staking(sdk)
}

pub fn get_governance<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let governance = staking_storage()
        .config_accessor()
        .governance_accessor()
        .get_checked(sdk)?;
    write_abi(sdk, &governance)
}

pub fn get_staking_token<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let token = staking_storage()
        .config_accessor()
        .staking_token_accessor()
        .get_checked(sdk)?;
    write_abi(sdk, &token)
}

pub fn get_active_validators_length<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let value = staking_storage()
        .config_accessor()
        .active_validators_length_accessor()
        .get_checked(sdk)?;
    write_abi(sdk, &value)
}

pub fn get_epoch_block_interval<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let value = staking_storage()
        .config_accessor()
        .epoch_block_interval_accessor()
        .get_checked(sdk)?;
    write_abi(sdk, &value)
}

pub fn get_dpos_activation_block<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let value = staking_storage()
        .config_accessor()
        .dpos_activation_block_accessor()
        .get_checked(sdk)?;
    write_abi(sdk, &value)
}

pub fn get_undelegate_period<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let value = staking_storage()
        .config_accessor()
        .undelegate_period_accessor()
        .get_checked(sdk)?;
    write_abi(sdk, &value)
}

pub fn get_min_validator_stake_amount<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let value = staking_storage()
        .config_accessor()
        .min_validator_stake_amount_accessor()
        .get_checked(sdk)?;
    write_abi(sdk, &value)
}

pub fn get_min_staking_amount<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let value = staking_storage()
        .config_accessor()
        .min_staking_amount_accessor()
        .get_checked(sdk)?;
    write_abi(sdk, &value)
}

pub fn is_validator<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let result = validator_status(sdk, address_arg(input)?)? != STATUS_NOT_FOUND;
    write_abi(sdk, &result)
}

pub fn is_validator_active<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let validator = address_arg(input)?;
    let result = validator_status(sdk, validator)? == STATUS_ACTIVE
        && selected_validators(sdk)?.contains(&validator);
    write_abi(sdk, &result)
}

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
        crate::math::expand_balance(snapshot.total_delegated_accessor().get_checked(sdk)?),
        snapshot.slashes_count_accessor().get_checked(sdk)?,
        changed_at,
        record.jailed_before_accessor().get_checked(sdk)?,
        record.claimed_at_accessor().get_checked(sdk)?,
        snapshot.commission_rate_accessor().get_checked(sdk)?,
    );
    write_abi(sdk, &result)
}

pub fn get_validator_by_owner<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let result = staking_storage()
        .owner_validators_accessor()
        .entry(address_arg(input)?)
        .get_checked(sdk)?;
    write_abi(sdk, &result)
}

pub fn get_validators<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &selected_validators(sdk)?)
}

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
