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
    storage::{
        current_epoch, staking_storage, ValidatorSnapshotStorage, STATUS_ACTIVE, STATUS_NOT_FOUND,
    },
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
    validator_total_at(sdk, validator, current_epoch(sdk)?)
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
    record.changed_at_accessor().set_checked(sdk, changed_at)?;
    storage
        .owner_validators_accessor()
        .entry(validator_owner)
        .set_checked(sdk, validator)?;

    let snapshot = storage
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(changed_at);
    snapshot.initialized_accessor().set_checked(sdk, true)?;
    snapshot
        .total_delegated_accessor()
        .set_checked(sdk, stake)?;
    snapshot
        .commission_rate_accessor()
        .set_checked(sdk, commission_rate)?;

    let delegation = storage
        .validator_delegations_accessor()
        .entry(validator)
        .entry(validator_owner);
    if delegation.delegate_queue_accessor().len_checked(sdk)? != 0 {
        return revert(sdk, ERR_DELEGATION_QUEUE_NOT_EMPTY);
    }
    let initial = delegation.delegate_queue_accessor().grow_checked(sdk)?;
    initial.amount_accessor().set_checked(sdk, stake)?;
    initial.epoch_accessor().set_checked(sdk, changed_at)?;

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
    let epoch = current_epoch(sdk)?;
    let mut ranked = Vec::with_capacity(active_len as usize);
    for index in 0..active_len {
        let validator = active.at(index).get_checked(sdk)?;
        if validator_status(sdk, validator)? == STATUS_ACTIVE {
            ranked.push((validator, validator_total_at(sdk, validator, epoch)?));
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

pub fn next_epoch<SDK: SharedAPI>(sdk: &SDK) -> Result<u64, ExitCode> {
    current_epoch(sdk)?
        .checked_add(1)
        .ok_or(ExitCode::IntegerOverflow)
}

pub fn touch_validator_snapshot<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
    epoch: u64,
) -> Result<ValidatorSnapshotStorage, ExitCode> {
    let storage = staking_storage();
    let snapshots = storage.validator_snapshots_accessor().entry(validator);
    let snapshot = snapshots.entry(epoch);
    if snapshot.initialized_accessor().get_checked(sdk)? {
        return Ok(snapshot);
    }

    let record = storage.validators_accessor().entry(validator);
    let changed_at = record.changed_at_accessor().get_checked(sdk)?;
    let previous = snapshots.entry(changed_at);
    snapshot.initialized_accessor().set_checked(sdk, true)?;
    snapshot
        .total_delegated_accessor()
        .set_checked(sdk, previous.total_delegated_accessor().get_checked(sdk)?)?;
    snapshot
        .commission_rate_accessor()
        .set_checked(sdk, previous.commission_rate_accessor().get_checked(sdk)?)?;
    snapshot
        .slashes_count_accessor()
        .set_checked(sdk, previous.slashes_count_accessor().get_checked(sdk)?)?;
    if epoch > changed_at {
        record.changed_at_accessor().set_checked(sdk, epoch)?;
    }
    Ok(snapshot)
}

pub fn validator_total_at<SDK: SharedAPI>(
    sdk: &SDK,
    validator: Address,
    epoch: u64,
) -> Result<U256, ExitCode> {
    let storage = staking_storage();
    let record = storage.validators_accessor().entry(validator);
    if record.status_accessor().get_checked(sdk)? == STATUS_NOT_FOUND {
        return Ok(U256::ZERO);
    }

    let mut lookup = core::cmp::min(epoch, record.changed_at_accessor().get_checked(sdk)?);
    let snapshots = storage.validator_snapshots_accessor().entry(validator);
    loop {
        let snapshot = snapshots.entry(lookup);
        if snapshot.initialized_accessor().get_checked(sdk)? {
            return snapshot.total_delegated_accessor().get_checked(sdk);
        }
        if lookup == 0 {
            return Ok(U256::ZERO);
        }
        lookup -= 1;
    }
}

pub fn erc20_transfer_from_input(
    from: Address,
    to: Address,
    amount: U256,
) -> Result<Vec<u8>, ExitCode> {
    let mut params = BytesMut::new();
    SolidityABI::<(Address, Address, U256)>::encode(&(from, to, amount), &mut params, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    let mut input = SIG_ERC20_TRANSFER_FROM.to_be_bytes().to_vec();
    input.extend_from_slice(&params);
    Ok(input)
}

pub fn safe_transfer_from<SDK: SharedAPI>(
    sdk: &mut SDK,
    from: Address,
    amount: U256,
) -> Result<(), ExitCode> {
    let token = staking_storage()
        .config_accessor()
        .staking_token_accessor()
        .get_checked(sdk)?;
    if token.is_zero() {
        return revert(sdk, ERR_ZERO_STAKING_TOKEN);
    }
    let recipient = sdk.context().contract_address();
    let input = erc20_transfer_from_input(from, recipient, amount)?;
    let result = sdk.call(token, U256::ZERO, &input, None);
    if !result.status.is_ok() {
        sdk.write(result.data);
        return Err(result.status);
    }
    if !result.data.is_empty()
        && !SolidityABI::<bool>::decode(&result.data, 0)
            .map_err(|_| ExitCode::MalformedBuiltinParams)?
    {
        return revert(sdk, ERR_STAKING_TOKEN_CALL_FAILED);
    }
    Ok(())
}
