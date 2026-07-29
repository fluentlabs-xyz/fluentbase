//! Snapshot-based rewards, bounded claims, and finalized stipend settlement.

use alloc::vec;
use fluentbase_sdk::{
    bytes::BytesMut, codec::SolidityABI, Address, Bytes, ContextReader, ExitCode, SharedAPI, U256,
};

use crate::{
    consts::*,
    economics::delegate_to,
    events,
    storage::{current_epoch, staking_storage, STATUS_NOT_FOUND},
    types::{AddressCommand, U64Command, ValidatorDelegatorCommand, ValidatorEpochCommand},
    util::{
        decode, ensure_initialized, ensure_mutable, ensure_non_payable, next_epoch, revert,
        revert_with, safe_transfer, touch_snapshot_at_or_before, write_abi,
    },
};

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
    let total_reward = U256::from(
        snapshot
            .total_blend_rewards_accessor()
            .get_checked(sdk)?,
    );
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
    let before_epoch = core::cmp::min(
        before_epoch,
        epoch.saturating_add(MAX_EPOCHS_PER_CLAIM),
    );
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
    while undelegate_gap < undelegate_len {
        let operation = undelegates.at(undelegate_gap);
        if operation.epoch_accessor().get_checked(sdk)? > before_epoch {
            break;
        }
        claimable = claimable
            .checked_add(crate::math::expand_balance(
                operation.amount_accessor().get_checked(sdk)?,
            ))
            .ok_or(ExitCode::IntegerOverflow)?;
        undelegate_gap += 1;
    }
    Ok(claimable)
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
    while undelegate_gap < undelegate_len {
        let operation = undelegates.at(undelegate_gap);
        if operation.epoch_accessor().get_checked(sdk)? > before_epoch {
            break;
        }
        claimable = claimable
            .checked_add(crate::math::expand_balance(
                operation.amount_accessor().get_checked(sdk)?,
            ))
            .ok_or(ExitCode::IntegerOverflow)?;
        undelegate_gap += 1;
    }
    delegation
        .undelegate_gap_accessor()
        .set_checked(sdk, undelegate_gap)?;
    Ok(claimable)
}

fn available_for_redelegate<SDK: SharedAPI>(
    sdk: &SDK,
    claimable: U256,
) -> Result<(U256, U256), ExitCode> {
    let amount = claimable / BALANCE_COMPACT_PRECISION * BALANCE_COMPACT_PRECISION;
    let minimum = staking_storage()
        .config_accessor()
        .min_staking_amount_accessor()
        .get_checked(sdk)?;
    if amount < minimum {
        return Ok((U256::ZERO, claimable));
    }
    Ok((amount, claimable - amount))
}

pub fn get_validator_fee<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let validator = decode::<AddressCommand>(input)?.value;
    write_abi(
        sdk,
        &validator_owner_rewards(sdk, validator, current_epoch(sdk)?)?,
    )
}

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

pub fn claim_validator_fee<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let validator = decode::<AddressCommand>(input)?.value;
    claim_validator_before(sdk, validator, current_epoch(sdk)?)
}

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
        &delegator_claimable(
            sdk,
            command.validator,
            command.delegator,
            before_epoch,
        )?,
    )
}

pub fn get_pending_delegator_fee<SDK: SharedAPI>(
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
        next_epoch(sdk)?,
    )?;
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

pub fn claim_delegator_fee<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    let validator = decode::<AddressCommand>(input)?.value;
    let delegator = sdk.context().contract_caller();
    claim_delegator_before(sdk, validator, delegator, current_epoch(sdk)?, false)
}

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
    let claimable = delegator_claimable(
        sdk,
        command.validator,
        command.delegator,
        before_epoch,
    )?;
    write_abi(sdk, &available_for_redelegate(sdk, claimable)?)
}

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

pub fn get_epoch_rewards<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_initialized(sdk)?;
    let epoch = decode::<U64Command>(input)?.value;
    let committee = staking_storage().epoch_committees_accessor().entry(epoch);
    let len = committee.len_checked(sdk)?;
    let mut total = U256::ZERO;
    for index in 0..len {
        let validator = committee.at(index).get_checked(sdk)?;
        total = total
            .checked_add(
                U256::from(
                    staking_storage()
                        .validator_snapshots_accessor()
                        .entry(validator)
                        .entry(epoch)
                        .total_blend_rewards_accessor()
                        .get_checked(sdk)?,
                ),
            )
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
    let committee = storage.epoch_committees_accessor().entry(epoch);
    let len = committee.len_checked(sdk)?;
    let desired = storage
        .config_accessor()
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
            let floor = storage
                .config_accessor()
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
                    || storage
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
                let stake = crate::util::validator_total_at(sdk, validator, epoch)?;
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

    if !assigned.is_zero() {
        let recipient = sdk.context().contract_address();
        let sent =
            call_decode::<_, _, U256>(sdk, reserve, SIG_RESERVE_DISBURSE, &(recipient, assigned))?;
        // Never credit accounting for BLEND the contract did not receive.
        if sent != assigned {
            return revert_with(sdk, ERR_RESERVE_SHORT_DISBURSEMENT, &(sent, assigned));
        }
    }
    if !assigned.is_zero() {
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
        blend_amount: assigned,
    }
    .emit(sdk)
}

pub fn settle_epoch_stipend<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_initialized(sdk)?;
    if sdk.context().contract_caller() != SYSTEM_CALLER {
        return revert(sdk, ERR_ONLY_SYSTEM_CALL);
    }
    let requested = decode::<U64Command>(input)?.value;
    let storage = staking_storage();
    let config = storage.config_accessor();
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
