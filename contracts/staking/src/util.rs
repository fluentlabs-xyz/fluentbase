use alloc::vec::Vec;
use fluentbase_sdk::{
    bytes::BytesMut,
    codec::{Encoder, SolidityABI},
    evm::write_evm_exit_message,
    Address, ContextReader, ExitCode, SharedAPI, U256,
};

use crate::{
    consts::*,
    events, math,
    storage::{current_epoch, staking_storage, STATUS_ACTIVE, STATUS_NOT_FOUND},
    types::AddressCommand,
};

pub fn revert<SDK: SharedAPI>(sdk: &mut SDK, code: u32) -> Result<(), ExitCode> {
    write_evm_exit_message(code, |slice| sdk.write(slice));
    Err(ExitCode::Panic)
}

pub fn ensure_non_payable<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    if !sdk.context().contract_value().is_zero() {
        return Err(ExitCode::Panic);
    }
    Ok(())
}

pub fn ensure_mutable<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    if sdk.context().contract_is_static() {
        return Err(ExitCode::StateChangeDuringStaticCall);
    }
    Ok(())
}

pub fn ensure_initialized<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    if !staking_storage().initialized_accessor().get_checked(sdk)? {
        return revert(sdk, ERR_NOT_INITIALIZED);
    }
    Ok(())
}

pub fn ensure_governance<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_initialized(sdk)?;
    let governance = staking_storage()
        .config_accessor()
        .governance_accessor()
        .get_checked(sdk)?;
    if sdk.context().contract_caller() != governance {
        return revert(sdk, ERR_ONLY_GOVERNANCE);
    }
    Ok(())
}

pub fn decode<T>(input: &[u8]) -> Result<T, ExitCode>
where
    T: Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    SolidityABI::<T>::decode(&input, 0).map_err(|_| ExitCode::MalformedBuiltinParams)
}

pub fn write_abi<SDK, T>(sdk: &mut SDK, value: &T) -> Result<(), ExitCode>
where
    SDK: SharedAPI,
    T: Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    let mut output = BytesMut::new();
    SolidityABI::<T>::encode(value, &mut output, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    sdk.write(output.freeze());
    Ok(())
}

pub fn address_arg(input: &[u8]) -> Result<Address, ExitCode> {
    Ok(decode::<AddressCommand>(input)?.value)
}

pub fn validator_status<SDK: SharedAPI>(sdk: &SDK, validator: Address) -> Result<u8, ExitCode> {
    staking_storage()
        .validators_accessor()
        .entry(validator)
        .status_accessor()
        .get_checked(sdk)
}

pub fn total_delegated<SDK: SharedAPI>(sdk: &SDK, validator: Address) -> Result<U256, ExitCode> {
    staking_storage()
        .validators_accessor()
        .entry(validator)
        .total_delegated_accessor()
        .get_checked(sdk)
}

pub fn set_validator<SDK: SharedAPI>(
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

    let storage = staking_storage();
    let record = storage.validators_accessor().entry(validator);
    if record.status_accessor().get_checked(sdk)? != STATUS_NOT_FOUND {
        return revert(sdk, ERR_VALIDATOR_ALREADY_EXISTS);
    }
    if !storage
        .owner_validators_accessor()
        .entry(validator_owner)
        .get_checked(sdk)?
        .is_zero()
    {
        return revert(sdk, ERR_VALIDATOR_OWNER_ALREADY_IN_USE);
    }

    record.owner_accessor().set_checked(sdk, validator_owner)?;
    record
        .status_accessor()
        .set_checked(sdk, validator_status)?;
    record.total_delegated_accessor().set_checked(sdk, stake)?;
    record.changed_at_accessor().set_checked(sdk, changed_at)?;
    record
        .commission_rate_accessor()
        .set_checked(sdk, commission_rate)?;
    storage
        .owner_validators_accessor()
        .entry(validator_owner)
        .set_checked(sdk, validator)?;

    if validator_status == STATUS_ACTIVE {
        storage
            .active_validators_accessor()
            .push_checked(sdk, validator)?;
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

pub fn selected_validators<SDK: SharedAPI>(sdk: &SDK) -> Result<Vec<Address>, ExitCode> {
    let storage = staking_storage();
    let active = storage.active_validators_accessor();
    let active_len = active.len_checked(sdk)?;
    let mut ranked = Vec::with_capacity(active_len as usize);
    for index in 0..active_len {
        let validator = active.at(index).get_checked(sdk)?;
        if validator_status(sdk, validator)? == STATUS_ACTIVE {
            ranked.push((validator, total_delegated(sdk, validator)?));
        }
    }
    ranked.sort_unstable_by(|(a_address, a_stake), (b_address, b_stake)| {
        b_stake.cmp(a_stake).then_with(|| a_address.cmp(b_address))
    });
    let cap = storage
        .config_accessor()
        .active_validators_length_accessor()
        .get_checked(sdk)? as usize;
    ranked.truncate(cap);
    Ok(ranked.into_iter().map(|(validator, _)| validator).collect())
}

pub fn emit_modified<SDK: SharedAPI>(sdk: &mut SDK, validator: Address) -> Result<(), ExitCode> {
    let record = staking_storage().validators_accessor().entry(validator);
    events::ValidatorModified {
        validator,
        owner: record.owner_accessor().get_checked(sdk)?,
        status: record.status_accessor().get_checked(sdk)?,
        commission_rate: record.commission_rate_accessor().get_checked(sdk)?,
    }
    .emit(sdk)
}

pub fn next_epoch<SDK: SharedAPI>(sdk: &SDK) -> Result<u64, ExitCode> {
    current_epoch(sdk)?
        .checked_add(1)
        .ok_or(ExitCode::IntegerOverflow)
}
