#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
//! Fluent validator staking system contract.
//!
//! This crate preserves the public Solidity selectors while moving
//! consensus-critical validator state into a fixed-address Rust system
//! contract. The first vertical slice implements genesis initialization,
//! epoch reads, validator registry reads, and governance/owner lifecycle
//! operations. Delegation, rewards, and equivocation are intentionally kept
//! out until their former cross-contract token and BLS calls are replaced by
//! explicit native runtime interfaces.

extern crate alloc;

mod consts;
mod math;
mod storage;

#[cfg(test)]
mod tests;

use alloc::vec::Vec;
use consts::*;
use fluentbase_sdk::{
    bytes::BytesMut,
    codec::{Codec, SolidityABI},
    derive::Event,
    evm::write_evm_exit_message,
    storage::{StorageAddress, StorageBool, StorageDescriptor, StorageU64},
    system_entrypoint, Address, ContextReader, ExitCode, SystemAPI, U256,
};
use storage::{
    active_validators, current_epoch, initialized, owner, status, total_delegated, AddressMap,
    U16Map, U256Map, U32Map, U64Map, U8Map, STATUS_ACTIVE, STATUS_NOT_FOUND, STATUS_PENDING,
};

#[derive(Default, Debug, Codec)]
struct InitializeCommand {
    initial_owner: Address,
    validators: Vec<Address>,
    initial_stakes: Vec<U256>,
    commission_rate: u16,
}

#[derive(Default, Debug, Codec)]
struct AddressCommand {
    value: Address,
}

#[derive(Default, Debug, Codec)]
struct AddressU16Command {
    validator: Address,
    value: u16,
}

#[derive(Default, Debug, Codec)]
struct TwoAddressesCommand {
    validator: Address,
    value: Address,
}

mod events {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Event)]
    pub struct ValidatorAdded {
        #[indexed]
        pub validator: Address,
        #[indexed]
        pub owner: Address,
        pub status: u8,
        pub commission_rate: u16,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Event)]
    pub struct ValidatorModified {
        #[indexed]
        pub validator: Address,
        #[indexed]
        pub owner: Address,
        pub status: u8,
        pub commission_rate: u16,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Event)]
    pub struct ValidatorRemoved {
        #[indexed]
        pub validator: Address,
    }
}

fn revert<SDK: SystemAPI>(sdk: &mut SDK, code: u32) -> Result<(), ExitCode> {
    write_evm_exit_message(code, |slice| sdk.write(slice));
    Err(ExitCode::Panic)
}

fn ensure_non_payable<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    if !sdk.context().contract_value().is_zero() {
        return Err(ExitCode::Panic);
    }
    Ok(())
}

fn ensure_mutable<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    if sdk.context().contract_is_static() {
        return Err(ExitCode::StateChangeDuringStaticCall);
    }
    Ok(())
}

fn ensure_initialized<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    if !initialized(sdk)? {
        return revert(sdk, ERR_NOT_INITIALIZED);
    }
    Ok(())
}

fn ensure_governance<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_initialized(sdk)?;
    if sdk.context().contract_caller() != owner(sdk)? {
        return revert(sdk, ERR_ONLY_GOVERNANCE);
    }
    Ok(())
}

fn decode<T>(input: &[u8]) -> Result<T, ExitCode>
where
    T: fluentbase_sdk::codec::Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    SolidityABI::<T>::decode(&input, 0).map_err(|_| ExitCode::MalformedBuiltinParams)
}

fn write_abi<SDK, T>(sdk: &mut SDK, value: &T) -> Result<(), ExitCode>
where
    SDK: SystemAPI,
    T: fluentbase_sdk::codec::Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    let mut output = BytesMut::new();
    SolidityABI::<T>::encode(value, &mut output, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    sdk.write(output.freeze());
    Ok(())
}

fn set_validator<SDK: SystemAPI>(
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
        return revert(sdk, ERR_BAD_COMMISSION_RATE);
    }
    if math::compact_balance(stake).is_none() {
        return revert(sdk, ERR_WRONG_AMOUNT_PRECISION);
    }
    if status(sdk, validator)? != STATUS_NOT_FOUND {
        return revert(sdk, ERR_VALIDATOR_ALREADY_EXISTS);
    }
    if !AddressMap::new(OWNER_VALIDATOR_SLOT)
        .entry(validator_owner)
        .get_checked(sdk)?
        .is_zero()
    {
        return revert(sdk, ERR_VALIDATOR_OWNER_ALREADY_IN_USE);
    }

    AddressMap::new(VALIDATOR_OWNER_SLOT)
        .entry(validator)
        .set_checked(sdk, validator_owner)?;
    AddressMap::new(OWNER_VALIDATOR_SLOT)
        .entry(validator_owner)
        .set_checked(sdk, validator)?;
    U8Map::new(VALIDATOR_STATUS_SLOT)
        .entry(validator)
        .set_checked(sdk, validator_status)?;
    U256Map::new(VALIDATOR_TOTAL_DELEGATED_SLOT)
        .entry(validator)
        .set_checked(sdk, stake)?;
    U64Map::new(VALIDATOR_CHANGED_AT_SLOT)
        .entry(validator)
        .set_checked(sdk, changed_at)?;
    U16Map::new(VALIDATOR_COMMISSION_RATE_SLOT)
        .entry(validator)
        .set_checked(sdk, commission_rate)?;

    if validator_status == STATUS_ACTIVE {
        active_validators().push_checked(sdk, validator)?;
    }
    events::ValidatorAdded {
        validator,
        owner: validator_owner,
        status: validator_status,
        commission_rate,
    }
    .emit(sdk)?;
    Ok(())
}

fn initialize_handler<SDK: SystemAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    if initialized(sdk)? {
        return revert(sdk, ERR_ALREADY_INITIALIZED);
    }
    let command: InitializeCommand = decode(input)?;
    if command.initial_owner.is_zero() {
        return revert(sdk, ERR_ZERO_OWNER);
    }
    if command.validators.len() != command.initial_stakes.len() {
        return revert(sdk, ERR_MALFORMED_INPUT_LENGTH);
    }

    StorageAddress::new(OWNER_SLOT, 12).set_checked(sdk, command.initial_owner)?;
    let activation_block = sdk.context().block_number();
    StorageU64::new(ACTIVATION_BLOCK_SLOT, 24).set_checked(sdk, activation_block)?;
    StorageU64::new(EPOCH_INTERVAL_SLOT, 24).set_checked(sdk, DEFAULT_EPOCH_BLOCK_INTERVAL)?;
    StorageU64::new(ACTIVE_VALIDATORS_LENGTH_SLOT, 24)
        .set_checked(sdk, DEFAULT_ACTIVE_VALIDATORS_LENGTH)?;

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
    // Set this last: a failed initializer cannot leave a partially initialized
    // instance when executed by a host that rolls storage back on errors.
    StorageBool::new(INITIALIZED_SLOT, 31).set_checked(sdk, true)?;
    Ok(())
}

fn current_epoch_handler<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    write_abi(sdk, &current_epoch(sdk)?)
}

fn next_epoch_handler<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let next = current_epoch(sdk)?
        .checked_add(1)
        .ok_or(ExitCode::IntegerOverflow)?;
    write_abi(sdk, &next)
}

fn owner_handler<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &owner(sdk)?)
}

fn address_arg(input: &[u8]) -> Result<Address, ExitCode> {
    Ok(decode::<AddressCommand>(input)?.value)
}

fn is_validator_handler<SDK: SystemAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let result = status(sdk, address_arg(input)?)? != STATUS_NOT_FOUND;
    write_abi(sdk, &result)
}

fn selected_validators<SDK: SystemAPI>(sdk: &SDK) -> Result<Vec<Address>, ExitCode> {
    let active = active_validators();
    let active_len = active.len_checked(sdk)?;
    let mut ranked = Vec::with_capacity(active_len as usize);
    for index in 0..active_len {
        let validator = active.at(index).get_checked(sdk)?;
        if status(sdk, validator)? == STATUS_ACTIVE {
            ranked.push((validator, total_delegated(sdk, validator)?));
        }
    }
    ranked.sort_unstable_by(|(a_address, a_stake), (b_address, b_stake)| {
        b_stake.cmp(a_stake).then_with(|| a_address.cmp(b_address))
    });
    let cap = StorageU64::new(ACTIVE_VALIDATORS_LENGTH_SLOT, 24).get_checked(sdk)? as usize;
    ranked.truncate(cap);
    Ok(ranked.into_iter().map(|(validator, _)| validator).collect())
}

fn is_validator_active_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let validator = address_arg(input)?;
    let result =
        status(sdk, validator)? == STATUS_ACTIVE && selected_validators(sdk)?.contains(&validator);
    write_abi(sdk, &result)
}

fn get_validator_status_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let validator = address_arg(input)?;
    let result = (
        AddressMap::new(VALIDATOR_OWNER_SLOT)
            .entry(validator)
            .get_checked(sdk)?,
        status(sdk, validator)?,
        total_delegated(sdk, validator)?,
        U32Map::new(VALIDATOR_SLASHES_SLOT)
            .entry(validator)
            .get_checked(sdk)?,
        U64Map::new(VALIDATOR_CHANGED_AT_SLOT)
            .entry(validator)
            .get_checked(sdk)?,
        U64Map::new(VALIDATOR_JAILED_BEFORE_SLOT)
            .entry(validator)
            .get_checked(sdk)?,
        U64Map::new(VALIDATOR_CLAIMED_AT_SLOT)
            .entry(validator)
            .get_checked(sdk)?,
        U16Map::new(VALIDATOR_COMMISSION_RATE_SLOT)
            .entry(validator)
            .get_checked(sdk)?,
    );
    write_abi(sdk, &result)
}

fn get_validator_by_owner_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let result = AddressMap::new(OWNER_VALIDATOR_SLOT)
        .entry(address_arg(input)?)
        .get_checked(sdk)?;
    write_abi(sdk, &result)
}

fn get_validators_handler<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &selected_validators(sdk)?)
}

fn add_validator_handler<SDK: SystemAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_governance(sdk)?;
    let validator = address_arg(input)?;
    let changed_at = current_epoch(sdk)?
        .checked_add(1)
        .ok_or(ExitCode::IntegerOverflow)?;
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

fn activate_validator_handler<SDK: SystemAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_governance(sdk)?;
    let validator = address_arg(input)?;
    if status(sdk, validator)? != STATUS_PENDING {
        return revert(sdk, ERR_NOT_PENDING_VALIDATOR);
    }
    U8Map::new(VALIDATOR_STATUS_SLOT)
        .entry(validator)
        .set_checked(sdk, STATUS_ACTIVE)?;
    active_validators().push_checked(sdk, validator)?;
    emit_modified(sdk, validator)
}

fn disable_validator_handler<SDK: SystemAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_governance(sdk)?;
    let validator = address_arg(input)?;
    if status(sdk, validator)? != STATUS_ACTIVE {
        return revert(sdk, ERR_NOT_ACTIVE_VALIDATOR);
    }
    storage::remove_active(sdk, validator)?;
    U8Map::new(VALIDATOR_STATUS_SLOT)
        .entry(validator)
        .set_checked(sdk, STATUS_PENDING)?;
    emit_modified(sdk, validator)
}

fn remove_validator_handler<SDK: SystemAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_governance(sdk)?;
    let validator = address_arg(input)?;
    if status(sdk, validator)? == STATUS_NOT_FOUND {
        return revert(sdk, ERR_VALIDATOR_NOT_FOUND);
    }
    if !total_delegated(sdk, validator)?.is_zero() {
        return revert(sdk, ERR_VALIDATOR_HAS_ACTIVE_DELEGATIONS);
    }
    storage::remove_active(sdk, validator)?;
    let validator_owner = AddressMap::new(VALIDATOR_OWNER_SLOT)
        .entry(validator)
        .get_checked(sdk)?;
    AddressMap::new(OWNER_VALIDATOR_SLOT)
        .entry(validator_owner)
        .set_checked(sdk, Address::ZERO)?;
    AddressMap::new(VALIDATOR_OWNER_SLOT)
        .entry(validator)
        .set_checked(sdk, Address::ZERO)?;
    U8Map::new(VALIDATOR_STATUS_SLOT)
        .entry(validator)
        .set_checked(sdk, STATUS_NOT_FOUND)?;
    events::ValidatorRemoved { validator }.emit(sdk)?;
    Ok(())
}

fn change_commission_handler<SDK: SystemAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let command: AddressU16Command = decode(input)?;
    if command.value > COMMISSION_RATE_MAX {
        return revert(sdk, ERR_BAD_COMMISSION_RATE);
    }
    let validator_owner = AddressMap::new(VALIDATOR_OWNER_SLOT)
        .entry(command.validator)
        .get_checked(sdk)?;
    if validator_owner.is_zero() {
        return revert(sdk, ERR_VALIDATOR_NOT_FOUND);
    }
    if validator_owner != sdk.context().contract_caller() {
        return revert(sdk, ERR_ONLY_VALIDATOR_OWNER);
    }
    U16Map::new(VALIDATOR_COMMISSION_RATE_SLOT)
        .entry(command.validator)
        .set_checked(sdk, command.value)?;
    emit_modified(sdk, command.validator)
}

fn change_owner_handler<SDK: SystemAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let command: TwoAddressesCommand = decode(input)?;
    let old_owner = AddressMap::new(VALIDATOR_OWNER_SLOT)
        .entry(command.validator)
        .get_checked(sdk)?;
    if old_owner.is_zero() {
        return revert(sdk, ERR_VALIDATOR_NOT_FOUND);
    }
    if old_owner != sdk.context().contract_caller() {
        return revert(sdk, ERR_ONLY_VALIDATOR_OWNER);
    }
    if command.value.is_zero() {
        return revert(sdk, ERR_ZERO_OWNER);
    }
    if !AddressMap::new(OWNER_VALIDATOR_SLOT)
        .entry(command.value)
        .get_checked(sdk)?
        .is_zero()
    {
        return revert(sdk, ERR_VALIDATOR_OWNER_ALREADY_IN_USE);
    }
    AddressMap::new(OWNER_VALIDATOR_SLOT)
        .entry(old_owner)
        .set_checked(sdk, Address::ZERO)?;
    AddressMap::new(OWNER_VALIDATOR_SLOT)
        .entry(command.value)
        .set_checked(sdk, command.validator)?;
    AddressMap::new(VALIDATOR_OWNER_SLOT)
        .entry(command.validator)
        .set_checked(sdk, command.value)?;
    emit_modified(sdk, command.validator)
}

fn emit_modified<SDK: SystemAPI>(sdk: &mut SDK, validator: Address) -> Result<(), ExitCode> {
    events::ValidatorModified {
        validator,
        owner: AddressMap::new(VALIDATOR_OWNER_SLOT)
            .entry(validator)
            .get_checked(sdk)?,
        status: status(sdk, validator)?,
        commission_rate: U16Map::new(VALIDATOR_COMMISSION_RATE_SLOT)
            .entry(validator)
            .get_checked(sdk)?,
    }
    .emit(sdk)
}

pub fn main_entry<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let input = sdk.bytes_input();
    if input.len() < SIG_LEN_BYTES {
        return Err(ExitCode::MalformedBuiltinParams);
    }
    let (selector, params) = input.split_at(SIG_LEN_BYTES);
    let selector = u32::from_be_bytes(
        selector
            .try_into()
            .map_err(|_| ExitCode::MalformedBuiltinParams)?,
    );
    match selector {
        SIG_INITIALIZE => initialize_handler(sdk, params),
        SIG_CURRENT_EPOCH => current_epoch_handler(sdk),
        SIG_NEXT_EPOCH => next_epoch_handler(sdk),
        SIG_OWNER => owner_handler(sdk),
        SIG_IS_VALIDATOR => is_validator_handler(sdk, params),
        SIG_IS_VALIDATOR_ACTIVE => is_validator_active_handler(sdk, params),
        SIG_GET_VALIDATOR_STATUS => get_validator_status_handler(sdk, params),
        SIG_GET_VALIDATOR_BY_OWNER => get_validator_by_owner_handler(sdk, params),
        SIG_GET_VALIDATORS => get_validators_handler(sdk),
        SIG_ADD_VALIDATOR => add_validator_handler(sdk, params),
        SIG_REMOVE_VALIDATOR => remove_validator_handler(sdk, params),
        SIG_ACTIVATE_VALIDATOR => activate_validator_handler(sdk, params),
        SIG_DISABLE_VALIDATOR => disable_validator_handler(sdk, params),
        SIG_CHANGE_VALIDATOR_COMMISSION_RATE => change_commission_handler(sdk, params),
        SIG_CHANGE_VALIDATOR_OWNER => change_owner_handler(sdk, params),
        _ => revert(sdk, ERR_UNKNOWN_METHOD),
    }
}

system_entrypoint!(main_entry);
