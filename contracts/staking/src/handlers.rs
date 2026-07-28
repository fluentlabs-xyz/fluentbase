use fluentbase_sdk::{Address, ContextReader, ExitCode, SharedAPI, U256};

use crate::{
    consts::*,
    events,
    storage::{
        current_epoch, remove_active, staking_storage, STATUS_ACTIVE, STATUS_NOT_FOUND,
        STATUS_PENDING,
    },
    types::{AddressU16Command, ConfigureCommand, InitializeCommand, TwoAddressesCommand},
    util::{
        address_arg, decode, emit_modified, ensure_governance, ensure_initialized, ensure_mutable,
        ensure_non_payable, next_epoch, revert, selected_validators, set_validator,
        total_delegated, validator_status, write_abi,
    },
};

pub fn initialize<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    let storage = staking_storage();
    if storage.initialized_accessor().get_checked(sdk)? {
        return revert(sdk, ERR_ALREADY_INITIALIZED);
    }
    let command: InitializeCommand = decode(input)?;
    if command.initial_owner.is_zero() {
        return revert(sdk, ERR_ZERO_OWNER);
    }
    if command.validators.len() != command.initial_stakes.len() {
        return revert(sdk, ERR_MALFORMED_INPUT_LENGTH);
    }

    storage
        .owner_accessor()
        .set_checked(sdk, command.initial_owner)?;
    let activation_block = sdk.context().block_number();
    let config = storage.config_accessor();
    config
        .governance_accessor()
        .set_checked(sdk, fluentbase_sdk::GENESIS_GOVERNANCE)?;
    config
        .dpos_activation_block_accessor()
        .set_checked(sdk, activation_block)?;
    config
        .epoch_block_interval_accessor()
        .set_checked(sdk, DEFAULT_EPOCH_BLOCK_INTERVAL)?;
    config
        .active_validators_length_accessor()
        .set_checked(sdk, DEFAULT_ACTIVE_VALIDATORS_LENGTH)?;
    config
        .min_validator_stake_amount_accessor()
        .set_checked(sdk, DEFAULT_MIN_VALIDATOR_STAKE)?;
    config
        .min_staking_amount_accessor()
        .set_checked(sdk, DEFAULT_MIN_STAKING_AMOUNT)?;

    for (validator, stake) in command.validators.into_iter().zip(command.initial_stakes) {
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
    storage.initialized_accessor().set_checked(sdk, true)?;
    Ok(())
}

/// Configure the BLEND token and the chain parameters now owned by staking.
///
/// This additive rWasm ABI replaces reads from the former `ChainConfig`
/// contract. It is owner-gated so genesis can initialize code first and wire
/// the canonical BLEND address once its deterministic address is known.
pub fn configure<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let storage = staking_storage();
    if sdk.context().contract_caller() != storage.owner_accessor().get_checked(sdk)? {
        return revert(sdk, ERR_ONLY_OWNER);
    }
    let command: ConfigureCommand = decode(input)?;
    if command.staking_token.is_zero() {
        return revert(sdk, ERR_ZERO_STAKING_TOKEN);
    }
    if command.active_validators_length == 0 || command.epoch_block_interval == 0 {
        return revert(sdk, ERR_INVALID_CHAIN_CONFIG);
    }

    let config = storage.config_accessor();
    config
        .staking_token_accessor()
        .set_checked(sdk, command.staking_token)?;
    config
        .active_validators_length_accessor()
        .set_checked(sdk, command.active_validators_length)?;
    config
        .epoch_block_interval_accessor()
        .set_checked(sdk, command.epoch_block_interval)?;
    config
        .min_validator_stake_amount_accessor()
        .set_checked(sdk, command.min_validator_stake_amount)?;
    config
        .min_staking_amount_accessor()
        .set_checked(sdk, command.min_staking_amount)?;
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
    let result = (
        record.owner_accessor().get_checked(sdk)?,
        record.status_accessor().get_checked(sdk)?,
        record.total_delegated_accessor().get_checked(sdk)?,
        record.slashes_count_accessor().get_checked(sdk)?,
        record.changed_at_accessor().get_checked(sdk)?,
        record.jailed_before_accessor().get_checked(sdk)?,
        record.claimed_at_accessor().get_checked(sdk)?,
        record.commission_rate_accessor().get_checked(sdk)?,
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
        return revert(sdk, ERR_NOT_PENDING_VALIDATOR);
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
    emit_modified(sdk, validator)
}

pub fn remove_validator<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_governance(sdk)?;
    let validator = address_arg(input)?;
    if validator_status(sdk, validator)? == STATUS_NOT_FOUND {
        return revert(sdk, ERR_VALIDATOR_NOT_FOUND);
    }
    if !total_delegated(sdk, validator)?.is_zero() {
        return revert(sdk, ERR_VALIDATOR_HAS_ACTIVE_DELEGATIONS);
    }
    remove_active(sdk, validator)?;
    let storage = staking_storage();
    let record = storage.validators_accessor().entry(validator);
    let validator_owner = record.owner_accessor().get_checked(sdk)?;
    storage
        .owner_validators_accessor()
        .entry(validator_owner)
        .set_checked(sdk, Address::ZERO)?;
    record.owner_accessor().set_checked(sdk, Address::ZERO)?;
    record
        .status_accessor()
        .set_checked(sdk, STATUS_NOT_FOUND)?;
    events::ValidatorRemoved { validator }.emit(sdk)?;
    Ok(())
}

pub fn change_commission<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let command: AddressU16Command = decode(input)?;
    if command.value > COMMISSION_RATE_MAX {
        return revert(sdk, ERR_BAD_COMMISSION_RATE);
    }
    let record = staking_storage()
        .validators_accessor()
        .entry(command.validator);
    let validator_owner = record.owner_accessor().get_checked(sdk)?;
    if validator_owner.is_zero() {
        return revert(sdk, ERR_VALIDATOR_NOT_FOUND);
    }
    if validator_owner != sdk.context().contract_caller() {
        return revert(sdk, ERR_ONLY_VALIDATOR_OWNER);
    }
    record
        .commission_rate_accessor()
        .set_checked(sdk, command.value)?;
    emit_modified(sdk, command.validator)
}

pub fn change_owner<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let command: TwoAddressesCommand = decode(input)?;
    let storage = staking_storage();
    let record = storage.validators_accessor().entry(command.validator);
    let old_owner = record.owner_accessor().get_checked(sdk)?;
    if old_owner.is_zero() {
        return revert(sdk, ERR_VALIDATOR_NOT_FOUND);
    }
    if old_owner != sdk.context().contract_caller() {
        return revert(sdk, ERR_ONLY_VALIDATOR_OWNER);
    }
    if command.value.is_zero() {
        return revert(sdk, ERR_ZERO_OWNER);
    }
    if !storage
        .owner_validators_accessor()
        .entry(command.value)
        .get_checked(sdk)?
        .is_zero()
    {
        return revert(sdk, ERR_VALIDATOR_OWNER_ALREADY_IN_USE);
    }
    storage
        .owner_validators_accessor()
        .entry(old_owner)
        .set_checked(sdk, Address::ZERO)?;
    storage
        .owner_validators_accessor()
        .entry(command.value)
        .set_checked(sdk, command.validator)?;
    record.owner_accessor().set_checked(sdk, command.value)?;
    emit_modified(sdk, command.validator)
}
