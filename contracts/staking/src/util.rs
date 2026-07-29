//! Shared ABI, validator-selection, snapshot, and ERC-20 helpers.

use alloc::vec::Vec;
use fluentbase_sdk::{
    byteorder::BE,
    bytes::BytesMut,
    codec::{Encoder, FunctionArgs, SolidityABI},
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
    sdk.write(code.to_be_bytes());
    Err(ExitCode::Panic)
}

pub fn revert_with<SDK, T>(sdk: &mut SDK, code: u32, value: &T) -> Result<(), ExitCode>
where
    SDK: SharedAPI,
    T: Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    let mut params = BytesMut::new();
    SolidityABI::<T>::encode(value, &mut params, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    let mut output = code.to_be_bytes().to_vec();
    output.extend_from_slice(&params);
    sdk.write(output);
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

/// Decode a Solidity function's parameter tuple.
///
/// Dynamic function arguments omit the outer tuple offset used when a
/// dynamic Rust struct is encoded as a standalone ABI value.
pub fn decode_args<T>(input: &[u8]) -> Result<T, ExitCode>
where
    T: FunctionArgs<BE, 32, true, false>,
{
    SolidityABI::<T>::decode_function_args(&input).map_err(|_| ExitCode::MalformedBuiltinParams)
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

/// Encode a Solidity function's return tuple without an outer tuple offset.
pub fn write_returns<SDK, T>(sdk: &mut SDK, value: &T) -> Result<(), ExitCode>
where
    SDK: SharedAPI,
    T: FunctionArgs<BE, 32, true, false>,
{
    let mut output = BytesMut::new();
    SolidityABI::<T>::encode_function_args(value, &mut output)
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
        return revert_with(
            sdk,
            ERR_VALIDATOR_OWNER_ALREADY_IN_USE,
            &validator_owner,
        );
    }

    record.owner_accessor().set_checked(sdk, validator_owner)?;
    record
        .status_accessor()
        .set_checked(sdk, validator_status)?;
    record.changed_at_accessor().set_checked(sdk, changed_at)?;
    record
        .first_snapshot_epoch_p1_accessor()
        .set_checked(
            sdk,
            changed_at
                .checked_add(1)
                .ok_or(ExitCode::IntegerOverflow)?,
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
    initial
        .amount_accessor()
        .set_checked(sdk, compact_stake)?;
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

pub fn seed_selection_membership<SDK: SharedAPI>(
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

pub fn ensure_rostered<SDK: SharedAPI>(sdk: &mut SDK, validator: Address) -> Result<(), ExitCode> {
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

pub fn set_selection_visible<SDK: SharedAPI>(
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

pub fn selection_visible_at<SDK: SharedAPI>(
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

pub fn remove_selection_member<SDK: SharedAPI>(
    sdk: &mut SDK,
    validator: Address,
) -> Result<(), ExitCode> {
    let storage = staking_storage();
    let roster = storage.selection_roster_accessor();
    let len = roster.len_checked(sdk)?;
    for index in 0..len {
        if roster.at(index).get_checked(sdk)? != validator {
            continue;
        }
        if index + 1 != len {
            let last = roster.at(len - 1).get_checked(sdk)?;
            roster.at(index).set_checked(sdk, last)?;
        }
        roster.pop_checked(sdk)?;
        break;
    }
    let membership = storage.selection_membership_accessor().entry(validator);
    membership.visible_accessor().set_checked(sdk, false)?;
    membership.prev_visible_accessor().set_checked(sdk, false)?;
    membership.effective_from_accessor().set_checked(sdk, 0)?;
    membership.rostered_accessor().set_checked(sdk, false)
}

pub fn selected_validators_at<SDK: SharedAPI>(
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
    let cap = storage
        .config_accessor()
        .active_validators_length_accessor()
        .get_checked(sdk)? as usize;
    top_k_by_stake_at(sdk, candidates, epoch, cap)
}

pub fn selected_validators<SDK: SharedAPI>(sdk: &SDK) -> Result<Vec<Address>, ExitCode> {
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
    let cap = storage
        .config_accessor()
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
    if epoch > changed_at {
        record.changed_at_accessor().set_checked(sdk, epoch)?;
    }
    Ok(snapshot)
}

pub fn touch_snapshot_at_or_before<SDK: SharedAPI>(
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
    let first_p1 = record
        .first_snapshot_epoch_p1_accessor()
        .get_checked(sdk)?;
    if first_p1 == 0 || epoch < first_p1 - 1 {
        return Ok(snapshot);
    }
    let floor = first_p1 - 1;
    let mut lookup = core::cmp::min(epoch, record.changed_at_accessor().get_checked(sdk)?);
    loop {
        let base = snapshots.entry(lookup);
        if lookup == record.changed_at_accessor().get_checked(sdk)?
            || !base
                .total_delegated_accessor()
                .get_checked(sdk)?
                .is_zero()
            || !base
                .total_blend_rewards_accessor()
                .get_checked(sdk)?
                .is_zero()
            || base.commission_rate_accessor().get_checked(sdk)? != 0
            || base.slashes_count_accessor().get_checked(sdk)? != 0
        {
            snapshot
                .total_delegated_accessor()
                .set_checked(sdk, base.total_delegated_accessor().get_checked(sdk)?)?;
            snapshot
                .commission_rate_accessor()
                .set_checked(sdk, base.commission_rate_accessor().get_checked(sdk)?)?;
            if epoch > record.changed_at_accessor().get_checked(sdk)? {
                record.changed_at_accessor().set_checked(sdk, epoch)?;
            }
            return Ok(snapshot);
        }
        if lookup == floor {
            return Ok(snapshot);
        }
        lookup -= 1;
    }
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

    let first_p1 = record
        .first_snapshot_epoch_p1_accessor()
        .get_checked(sdk)?;
    if first_p1 == 0 || epoch < first_p1 - 1 {
        return Ok(U256::ZERO);
    }
    let floor = first_p1 - 1;
    let mut lookup = core::cmp::min(epoch, record.changed_at_accessor().get_checked(sdk)?);
    let changed_at = record.changed_at_accessor().get_checked(sdk)?;
    let snapshots = storage.validator_snapshots_accessor().entry(validator);
    loop {
        let snapshot = snapshots.entry(lookup);
        let compact = snapshot.total_delegated_accessor().get_checked(sdk)?;
        if lookup == changed_at
            || !compact.is_zero()
            || !snapshot
                .total_blend_rewards_accessor()
                .get_checked(sdk)?
                .is_zero()
            || snapshot.commission_rate_accessor().get_checked(sdk)? != 0
            || snapshot.slashes_count_accessor().get_checked(sdk)? != 0
        {
            return Ok(math::expand_balance(compact));
        }
        if lookup == floor {
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

pub fn erc20_transfer_input(to: Address, amount: U256) -> Result<Vec<u8>, ExitCode> {
    let mut params = BytesMut::new();
    SolidityABI::<(Address, U256)>::encode(&(to, amount), &mut params, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    let mut input = SIG_ERC20_TRANSFER.to_be_bytes().to_vec();
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

pub fn safe_transfer<SDK: SharedAPI>(
    sdk: &mut SDK,
    recipient: Address,
    amount: U256,
) -> Result<(), ExitCode> {
    if amount.is_zero() {
        return Ok(());
    }
    let token = staking_storage()
        .config_accessor()
        .staking_token_accessor()
        .get_checked(sdk)?;
    if token.is_zero() {
        return revert(sdk, ERR_ZERO_STAKING_TOKEN);
    }
    let input = erc20_transfer_input(recipient, amount)?;
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
