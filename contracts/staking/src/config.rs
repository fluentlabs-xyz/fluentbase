//! Governance-controlled staking parameters and external dependencies.
//!
//! Setters validate cross-field invariants before updating namespaced storage.

use crate::{
    consts::*,
    events,
    math::compact_balance,
    storage::chain_config_storage,
    types::{AddressCommand, BoolCommand, InitializeCommand, U256Command, U32Command, U64Command},
    util::{
        decode, ensure_governance, ensure_mutable, ensure_non_payable, next_epoch, revert,
        revert_with, write_abi,
    },
};
use alloc::string::String;
use fluentbase_sdk::{Address, ContextReader, ExitCode, SharedAPI, U256};

fn ensure_governance_mutation<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_governance(sdk)
}

fn ensure_dpos_not_active<SDK: SharedAPI>(sdk: &mut SDK, activation: u64) -> Result<(), ExitCode> {
    if activation != 0 && sdk.context().block_number() >= activation {
        return revert(sdk, ERR_DPOS_ALREADY_ACTIVE);
    }
    Ok(())
}

/// Initialize all chain configuration and dependency fields.
pub(crate) fn apply_initial_config<SDK: SharedAPI>(
    sdk: &mut SDK,
    command: &InitializeCommand,
) -> Result<(), ExitCode> {
    validate_initialization(sdk, command)?;
    let initialization_block = sdk.context().block_number();
    let config = chain_config_storage();
    config
        .staking_token_accessor()
        .set_checked(sdk, command.staking_token)?;
    config
        .active_validators_length_accessor()
        .set_checked(sdk, command.active_validators_length as u64)?;
    schedule_cap_checkpoint(sdk, 0, command.active_validators_length)?;
    config
        .epoch_block_interval_accessor()
        .set_checked(sdk, command.epoch_block_interval as u64)?;
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
    config
        .liveness_slashing_accessor()
        .set_checked(sdk, command.liveness_slashing)?;
    config
        .blend_reserve_accessor()
        .set_checked(sdk, command.blend_reserve)?;
    config
        .min_verdict_due_blocks_accessor()
        .set_checked(sdk, DEFAULT_MIN_VERDICT_DUE_BLOCKS)?;
    config
        .exclusion_backoff_cap_accessor()
        .set_checked(sdk, DEFAULT_EXCLUSION_BACKOFF_CAP)?;
    // The tier ships off, and an unwritten slot would ship it on.
    config
        .production_liveness_disabled_accessor()
        .set_checked(sdk, true)?;
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

    events::ActiveValidatorsLengthChanged {
        prev_value: DEFAULT_ACTIVE_VALIDATORS_LENGTH as u32,
        new_value: command.active_validators_length,
        effective_epoch: 0,
    }
    .emit(sdk)?;
    events::EpochBlockIntervalChanged {
        prev_value: DEFAULT_EPOCH_BLOCK_INTERVAL as u32,
        new_value: command.epoch_block_interval,
    }
    .emit(sdk)?;
    events::UndelegatePeriodChanged {
        prev_value: DEFAULT_UNDELEGATE_PERIOD as u32,
        new_value: command.undelegate_period,
    }
    .emit(sdk)?;
    events::MinValidatorStakeAmountChanged {
        prev_value: DEFAULT_MIN_VALIDATOR_STAKE,
        new_value: command.min_validator_stake_amount,
    }
    .emit(sdk)?;
    events::MinStakingAmountChanged {
        prev_value: DEFAULT_MIN_STAKING_AMOUNT,
        new_value: command.min_staking_amount,
    }
    .emit(sdk)?;
    events::DposActivationBlockChanged {
        prev_value: initialization_block,
        new_value: command.dpos_activation_block,
    }
    .emit(sdk)?;
    events::MinVerdictDueBlocksChanged {
        prev_value: 0,
        new_value: DEFAULT_MIN_VERDICT_DUE_BLOCKS,
    }
    .emit(sdk)?;
    events::ExclusionBackoffCapChanged {
        prev_value: 0,
        new_value: DEFAULT_EXCLUSION_BACKOFF_CAP,
    }
    .emit(sdk)?;
    events::ProductionLivenessDisabledChanged {
        prev_value: false,
        new_value: true,
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
    events::LivenessSlashingChanged {
        prev_value: Address::ZERO,
        new_value: command.liveness_slashing,
    }
    .emit(sdk)?;
    events::BlendReserveChanged {
        prev_value: Address::ZERO,
        new_value: command.blend_reserve,
    }
    .emit(sdk)?;
    Ok(())
}

fn validate_initialization<SDK: SharedAPI>(
    sdk: &mut SDK,
    command: &InitializeCommand,
) -> Result<(), ExitCode> {
    if command.staking_token.is_zero() {
        return revert(sdk, ERR_ZERO_STAKING_TOKEN);
    }
    if command.active_validators_length == 0
        || command.active_validators_length as u64 > MAX_ACTIVE_VALIDATORS_LENGTH
        || command.epoch_block_interval == 0
        || command.undelegate_period == 0
        || command.min_validator_stake_amount.is_zero()
        || command.min_staking_amount.is_zero()
        || compact_balance(command.min_validator_stake_amount).is_none()
        || compact_balance(command.min_staking_amount).is_none()
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
    if command.liveness_slashing.is_zero() {
        return revert_with(sdk, ERR_ZERO_VALUE, &String::from("livenessSlashing"));
    }
    if command.blend_reserve.is_zero() {
        return revert_with(sdk, ERR_ZERO_VALUE, &String::from("blendReserve"));
    }
    Ok(())
}

/// Public handler `0x6cc69027` (`DEFAULT_SLASH_REPORTER_BPS`).
///
/// Returns the protocol default slash reporter BPS constant.
pub fn default_slash_reporter_bps<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &DEFAULT_SLASH_REPORTER_REWARD_BPS)
}

/// Public handler `0x5d887462` (`MAX_ACTIVE_VALIDATORS`).
///
/// Returns the protocol max active validators limit.
pub fn max_active_validators<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &(MAX_ACTIVE_VALIDATORS_LENGTH as u32))
}

/// Public handler `0x2bc2fec4` (`MAX_BLEND_STIPEND_PER_EPOCH`).
///
/// Returns the protocol max blend stipend per epoch limit.
pub fn max_blend_stipend_per_epoch<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &MAX_BLEND_STIPEND_PER_EPOCH)
}

/// Public handler `0x0a3a6183` (`MAX_SLASH_REPORTER_BPS`).
///
/// Returns the protocol max slash reporter BPS limit.
pub fn max_slash_reporter_bps<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &MAX_SLASH_REPORTER_REWARD_BPS)
}

/// Public handler `0x6fd3afb7` (`DEFAULT_MIN_VERDICT_DUE_BLOCKS`).
///
/// Returns the protocol default verdict due-block floor.
pub fn default_min_verdict_due_blocks<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &DEFAULT_MIN_VERDICT_DUE_BLOCKS)
}

/// Public handler `0xd4c30c1a` (`DEFAULT_EXCLUSION_BACKOFF_CAP`).
///
/// Returns the protocol default exclusion backoff cap.
pub fn default_exclusion_backoff_cap<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &DEFAULT_EXCLUSION_BACKOFF_CAP)
}

/// Public handler `0x9b9a11ba` (`MAX_MIN_VERDICT_DUE_BLOCKS`).
///
/// Returns the protocol ceiling on the verdict due-block floor.
pub fn max_min_verdict_due_blocks<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &MAX_MIN_VERDICT_DUE_BLOCKS)
}

/// Public handler `0x9f9106d1` (`getStakingToken`).
///
/// Returns the configured staking token.
pub fn get_staking_token<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .staking_token_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0x32cc6f08` (`getActiveValidatorsLength`).
///
/// Returns the configured active validator count.
pub fn get_active_validators_length<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .active_validators_length_accessor()
            .get_checked(sdk)?,
    )
}

/// Committee size cap in force at `epoch`.
///
/// Backward scan over the checkpoint history. An empty history means the
/// contract is not initialized yet, in which case the scalar is the only
/// answer available.
pub(crate) fn active_validators_length_at<SDK: SharedAPI>(
    sdk: &SDK,
    epoch: u64,
) -> Result<u64, ExitCode> {
    let config = chain_config_storage();
    let checkpoints = config.cap_checkpoints_accessor();
    let mut index = checkpoints.len_checked(sdk)?;
    while index > 0 {
        index -= 1;
        let checkpoint = checkpoints.at(index);
        if checkpoint.from_epoch_accessor().get_checked(sdk)? <= epoch {
            return Ok(checkpoint.value_accessor().get_checked(sdk)? as u64);
        }
    }
    config
        .active_validators_length_accessor()
        .get_checked(sdk)
}

/// Records `value` as the cap from `from_epoch` onward.
///
/// A repeat set inside the same epoch overwrites the pending tail instead of
/// appending, so a scheduled-but-not-yet-effective cap can still be corrected
/// without leaving an unreachable checkpoint behind.
fn schedule_cap_checkpoint<SDK: SharedAPI>(
    sdk: &mut SDK,
    from_epoch: u64,
    value: u32,
) -> Result<(), ExitCode> {
    let checkpoints = chain_config_storage().cap_checkpoints_accessor();
    let len = checkpoints.len_checked(sdk)?;
    if len > 0 {
        let tail = checkpoints.at(len - 1);
        if tail.from_epoch_accessor().get_checked(sdk)? == from_epoch {
            return tail.value_accessor().set_checked(sdk, value);
        }
    }
    // `grow_checked` does not zero the new element, so both fields are written.
    let entry = checkpoints.grow_checked(sdk)?;
    entry.from_epoch_accessor().set_checked(sdk, from_epoch)?;
    entry.value_accessor().set_checked(sdk, value)
}

/// Public handler `0xd9b083ba` (`getActiveValidatorsLengthAt`).
///
/// Returns the committee size cap that was in force at `epoch`.
pub fn get_active_validators_length_at<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let epoch = decode::<U64Command>(input)?.value;
    let value = active_validators_length_at(sdk, epoch)?;
    write_abi(sdk, &value)
}

/// Public handler `0x346c90a8` (`getEpochBlockInterval`).
///
/// Returns the configured epoch block interval.
pub fn get_epoch_block_interval<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .epoch_block_interval_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0xa2a50528` (`getDposActivationBlock`).
///
/// Returns the configured DPoS activation block.
pub fn get_dpos_activation_block<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .dpos_activation_block_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0x5e7b72ad` (`getUndelegatePeriod`).
///
/// Returns the configured undelegate period.
pub fn get_undelegate_period<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .undelegate_period_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0x6f856847` (`getMinValidatorStakeAmount`).
///
/// Returns the configured min validator stake amount.
pub fn get_min_validator_stake_amount<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .min_validator_stake_amount_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0xeea9a01b` (`getMinStakingAmount`).
///
/// Returns the configured min staking amount.
pub fn get_min_staking_amount<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .min_staking_amount_accessor()
            .get_checked(sdk)?,
    )
}

fn require_nonzero<SDK: SharedAPI>(
    sdk: &mut SDK,
    value: U256,
    field: &str,
) -> Result<(), ExitCode> {
    if value.is_zero() {
        return revert_with(sdk, ERR_ZERO_VALUE, &String::from(field));
    }
    Ok(())
}

fn zero_value<SDK: SharedAPI>(sdk: &mut SDK, field: &str) -> Result<(), ExitCode> {
    revert_with(sdk, ERR_ZERO_VALUE, &String::from(field))
}

fn require_undelegate_window<SDK: SharedAPI>(
    sdk: &mut SDK,
    period: u64,
    interval: u64,
) -> Result<(), ExitCode> {
    let window = U256::from(period)
        .checked_mul(U256::from(interval))
        .ok_or(ExitCode::IntegerOverflow)?;
    let minimum = chain_config_storage()
        .min_undelegate_blocks_accessor()
        .get_checked(sdk)?;
    if window < minimum {
        return revert_with(sdk, ERR_UNDELEGATE_WINDOW_TOO_SHORT, &(window, minimum));
    }
    Ok(())
}

/// Public handler `0xce534df5` (`getSlashReporterRewardBps`).
///
/// Returns the configured slash reporter reward BPS.
pub fn get_slash_reporter_reward_bps<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let stored = chain_config_storage()
        .slash_reporter_reward_bps_accessor()
        .get_checked(sdk)?;
    write_abi(
        sdk,
        &(if stored == 0 {
            DEFAULT_SLASH_REPORTER_REWARD_BPS
        } else {
            stored
        }),
    )
}

/// Public handler `0x58702003` (`setSlashReporterRewardBps`).
///
/// Updates the configured slash reporter reward BPS.
pub fn set_slash_reporter_reward_bps<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "slashReporterRewardBps");
    }
    if value > MAX_SLASH_REPORTER_REWARD_BPS {
        return revert_with(
            sdk,
            ERR_SLASH_REPORTER_REWARD_BPS_TOO_HIGH,
            &(value, MAX_SLASH_REPORTER_REWARD_BPS),
        );
    }
    let field = chain_config_storage().slash_reporter_reward_bps_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::SlashReporterRewardBpsChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0xc910df38` (`getSlashFundAddress`).
///
/// Returns the configured slash fund address.
pub fn get_slash_fund_address<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .slash_fund_address_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0xa79e7263` (`setSlashFundAddress`).
///
/// Updates the configured slash fund address.
pub fn set_slash_fund_address<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<AddressCommand>(input)?.value;
    if value.is_zero() {
        return zero_value(sdk, "slashFundAddress");
    }
    let field = chain_config_storage().slash_fund_address_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::SlashFundAddressChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0xc8f45d87` (`getBlendStipendPerEpoch`).
///
/// Returns the configured blend stipend per epoch.
pub fn get_blend_stipend_per_epoch<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .blend_stipend_per_epoch_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0x2c91b879` (`setBlendStipendPerEpoch`).
///
/// Updates the configured blend stipend per epoch.
pub fn set_blend_stipend_per_epoch<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U256Command>(input)?.value;
    if value > MAX_BLEND_STIPEND_PER_EPOCH {
        return revert_with(
            sdk,
            ERR_BLEND_STIPEND_PER_EPOCH_TOO_HIGH,
            &(value, MAX_BLEND_STIPEND_PER_EPOCH),
        );
    }
    let field = chain_config_storage().blend_stipend_per_epoch_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::BlendStipendPerEpochChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0xc227a412` (`setActiveValidatorsLength`).
///
/// Updates the configured active validator count.
pub fn set_active_validators_length<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "activeValidatorsLength");
    }
    if value as u64 > MAX_ACTIVE_VALIDATORS_LENGTH {
        return revert_with(
            sdk,
            ERR_MAX_ACTIVE_VALIDATORS_EXCEEDED,
            &(value, MAX_ACTIVE_VALIDATORS_LENGTH as u32),
        );
    }
    let field = chain_config_storage().active_validators_length_accessor();
    let previous = field.get_checked(sdk)?;
    // The scalar reports the latest SCHEDULED value immediately; the checkpoint
    // governs epoch-correct reads. Splitting the two is what keeps an epoch that
    // has already started immutable.
    field.set_checked(sdk, value as u64)?;
    let effective_epoch = next_epoch(sdk)?;
    schedule_cap_checkpoint(sdk, effective_epoch, value)?;
    events::ActiveValidatorsLengthChanged {
        prev_value: previous as u32,
        new_value: value,
        effective_epoch,
    }
    .emit(sdk)
}

/// Public handler `0xaf70fa2c` (`setEpochBlockInterval`).
///
/// Updates the configured epoch block interval.
pub fn set_epoch_block_interval<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "epochBlockInterval");
    }
    let config = chain_config_storage();
    let activation = config.dpos_activation_block_accessor().get_checked(sdk)?;
    ensure_dpos_not_active(sdk, activation)?;
    if activation != 0 && activation % value as u64 != 0 {
        return revert(sdk, ERR_UNALIGNED_ACTIVATION_BLOCK);
    }
    let undelegate = config.undelegate_period_accessor().get_checked(sdk)?;
    require_undelegate_window(sdk, undelegate, value as u64)?;
    let field = config.epoch_block_interval_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value as u64)?;
    events::EpochBlockIntervalChanged {
        prev_value: previous as u32,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0xf517ca6a` (`setDposActivationBlock`).
///
/// Updates the configured DPoS activation block.
pub fn set_dpos_activation_block<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U64Command>(input)?.value;
    let config = chain_config_storage();
    let previous = config.dpos_activation_block_accessor().get_checked(sdk)?;
    ensure_dpos_not_active(sdk, previous)?;
    let interval = config.epoch_block_interval_accessor().get_checked(sdk)?;
    if interval == 0 || value % interval != 0 {
        return revert(sdk, ERR_UNALIGNED_ACTIVATION_BLOCK);
    }
    if value < sdk.context().block_number() {
        return revert(sdk, ERR_ACTIVATION_BLOCK_IN_PAST);
    }
    config
        .dpos_activation_block_accessor()
        .set_checked(sdk, value)?;
    events::DposActivationBlockChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0x41d8a080` (`setUndelegatePeriod`).
///
/// Updates the configured undelegate period.
pub fn set_undelegate_period<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "undelegatePeriod");
    }
    let config = chain_config_storage();
    let activation = config.dpos_activation_block_accessor().get_checked(sdk)?;
    ensure_dpos_not_active(sdk, activation)?;
    require_undelegate_window(
        sdk,
        value as u64,
        config.epoch_block_interval_accessor().get_checked(sdk)?,
    )?;
    let field = config.undelegate_period_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value as u64)?;
    events::UndelegatePeriodChanged {
        prev_value: previous as u32,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0xe1a2e863` (`setMinValidatorStakeAmount`).
///
/// Updates the configured min validator stake amount.
pub fn set_min_validator_stake_amount<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U256Command>(input)?.value;
    require_nonzero(sdk, value, "minValidatorStakeAmount")?;
    if crate::math::compact_balance(value).is_none() {
        return revert(sdk, ERR_WRONG_AMOUNT_PRECISION);
    }
    let field = chain_config_storage().min_validator_stake_amount_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::MinValidatorStakeAmountChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0x612d669e` (`setMinStakingAmount`).
///
/// Updates the configured min staking amount.
pub fn set_min_staking_amount<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U256Command>(input)?.value;
    require_nonzero(sdk, value, "minStakingAmount")?;
    if crate::math::compact_balance(value).is_none() {
        return revert(sdk, ERR_WRONG_AMOUNT_PRECISION);
    }
    let field = chain_config_storage().min_staking_amount_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::MinStakingAmountChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0xee3ad0e7` (`getMinVerdictDueBlocks`).
///
/// Returns the configured verdict due-block floor.
pub fn get_min_verdict_due_blocks<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .min_verdict_due_blocks_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0x4fae9dea` (`setMinVerdictDueBlocks`).
///
/// Updates the due-block floor below which a committee member holds no verdict.
///
/// Zero would judge a member that was never due a single slot; the ceiling
/// keeps the floor from silently disabling the tier for every member at once.
pub fn set_min_verdict_due_blocks<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "minVerdictDueBlocks");
    }
    if value > MAX_MIN_VERDICT_DUE_BLOCKS {
        return revert_with(
            sdk,
            ERR_MIN_VERDICT_DUE_BLOCKS_TOO_HIGH,
            &(value, MAX_MIN_VERDICT_DUE_BLOCKS),
        );
    }
    let field = chain_config_storage().min_verdict_due_blocks_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::MinVerdictDueBlocksChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0x6bed0322` (`getExclusionBackoffCap`).
///
/// Returns the configured exclusion backoff cap.
pub fn get_exclusion_backoff_cap<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .exclusion_backoff_cap_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0x3b543e1c` (`setExclusionBackoffCap`).
///
/// Updates the ceiling on the linear exclusion ladder, in selection epochs.
pub fn set_exclusion_backoff_cap<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "exclusionBackoffCap");
    }
    let field = chain_config_storage().exclusion_backoff_cap_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::ExclusionBackoffCapChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0x9a4c46bb` (`getProductionLivenessDisabled`).
///
/// Returns whether the production-liveness tier is switched off.
pub fn get_production_liveness_disabled<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .production_liveness_disabled_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0x8fc07556` (`setProductionLivenessDisabled`).
///
/// Switches the production-liveness tier on or off.
pub fn set_production_liveness_disabled<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<BoolCommand>(input)?.value;
    let field = chain_config_storage().production_liveness_disabled_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::ProductionLivenessDisabledChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0xc6b904ad` (`getBlsVerifier`).
///
/// Returns the configured BLS verifier.
pub fn get_bls_verifier<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .bls_verifier_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0x466ae541` (`setBlsVerifier`).
///
/// Updates the configured BLS verifier.
pub fn set_bls_verifier<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<AddressCommand>(input)?.value;
    if value.is_zero() {
        return zero_value(sdk, "blsVerifier");
    }
    let field = chain_config_storage().bls_verifier_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::BlsVerifierChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0xe2cf72f9` (`getEvidenceDecoder`).
///
/// Returns the configured evidence decoder.
pub fn get_evidence_decoder<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .evidence_decoder_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0x00857c90` (`setEvidenceDecoder`).
///
/// Updates the configured evidence decoder.
pub fn set_evidence_decoder<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<AddressCommand>(input)?.value;
    if value.is_zero() {
        return zero_value(sdk, "evidenceDecoder");
    }
    let field = chain_config_storage().evidence_decoder_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::EvidenceDecoderChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0xdb2366b4` (`getLivenessSlashing`).
///
/// Returns the configured liveness-slashing authority.
pub fn get_liveness_slashing<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .liveness_slashing_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0xbb32522a` (`setLivenessSlashing`).
///
/// Rotates the liveness-slashing authority under governance control.
pub fn set_liveness_slashing<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<AddressCommand>(input)?.value;
    if value.is_zero() {
        return zero_value(sdk, "livenessSlashing");
    }
    let field = chain_config_storage().liveness_slashing_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::LivenessSlashingChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0x37dff538` (`getBlendReserve`).
///
/// Returns the configured BLEND reserve.
pub fn get_blend_reserve<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .blend_reserve_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0x7899ae8f` (`setBlendReserve`).
///
/// Rotates the BLEND reserve under governance control.
pub fn set_blend_reserve<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<AddressCommand>(input)?.value;
    if value.is_zero() {
        return zero_value(sdk, "blendReserve");
    }
    let field = chain_config_storage().blend_reserve_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::BlendReserveChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}
