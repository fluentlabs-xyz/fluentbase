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
        decode, ensure_governance, ensure_mutable, ensure_non_payable, revert, revert_with,
        write_abi,
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
    config
        .liveness_slashing_accessor()
        .set_checked(sdk, command.liveness_slashing)?;
    config
        .blend_reserve_accessor()
        .set_checked(sdk, command.blend_reserve)?;
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
    }
    .emit(sdk)?;
    events::EpochBlockIntervalChanged {
        prev_value: DEFAULT_EPOCH_BLOCK_INTERVAL as u32,
        new_value: command.epoch_block_interval,
    }
    .emit(sdk)?;
    events::FelonyThresholdChanged {
        prev_value: DEFAULT_FELONY_THRESHOLD,
        new_value: command.felony_threshold,
    }
    .emit(sdk)?;
    events::ValidatorJailEpochLengthChanged {
        prev_value: DEFAULT_VALIDATOR_JAIL_EPOCH_LENGTH,
        new_value: command.validator_jail_epoch_length,
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
        || command.felony_threshold == 0
        || command.validator_jail_epoch_length == 0
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

/// Public handler `0x2c1d88e8` (`DEFAULT_PARTICIPATION_FLOOR_BPS`).
///
/// Returns the protocol default participation floor BPS constant.
pub fn default_participation_floor_bps<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &DEFAULT_PARTICIPATION_FLOOR_BPS)
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

/// Public handler `0x9dbdf12b` (`MAX_PARTICIPATION_FLOOR_BPS`).
///
/// Returns the protocol max participation floor BPS limit.
pub fn max_participation_floor_bps<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &MAX_PARTICIPATION_FLOOR_BPS)
}

/// Public handler `0x0a3a6183` (`MAX_SLASH_REPORTER_BPS`).
///
/// Returns the protocol max slash reporter BPS limit.
pub fn max_slash_reporter_bps<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &MAX_SLASH_REPORTER_REWARD_BPS)
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

/// Public handler `0xbe199738` (`getFelonyThreshold`).
///
/// Returns the configured felony threshold.
pub fn get_felony_threshold<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .felony_threshold_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0xfcd6cb3e` (`setFelonyThreshold`).
///
/// Updates the configured felony threshold.
pub fn set_felony_threshold<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "felonyThreshold");
    }
    let field = chain_config_storage().felony_threshold_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::FelonyThresholdChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0x6cbe6cd8` (`getValidatorJailEpochLength`).
///
/// Returns the configured validator jail epoch length.
pub fn get_validator_jail_epoch_length<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .validator_jail_epoch_length_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0xc8652bd5` (`setValidatorJailEpochLength`).
///
/// Updates the configured validator jail epoch length.
pub fn set_validator_jail_epoch_length<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "validatorJailEpochLength");
    }
    let field = chain_config_storage().validator_jail_epoch_length_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::ValidatorJailEpochLengthChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
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

/// Public handler `0x4baffdc4` (`getParticipationFloorBps`).
///
/// Returns the configured participation floor BPS.
pub fn get_participation_floor_bps<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let stored = chain_config_storage()
        .participation_floor_bps_accessor()
        .get_checked(sdk)?;
    write_abi(
        sdk,
        &(if stored == 0 {
            DEFAULT_PARTICIPATION_FLOOR_BPS
        } else {
            stored
        }),
    )
}

/// Public handler `0xd0a01007` (`setParticipationFloorBps`).
///
/// Updates the configured participation floor BPS.
pub fn set_participation_floor_bps<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "participationFloorBps");
    }
    if value > MAX_PARTICIPATION_FLOOR_BPS {
        return revert_with(
            sdk,
            ERR_PARTICIPATION_FLOOR_BPS_TOO_HIGH,
            &(value, MAX_PARTICIPATION_FLOOR_BPS),
        );
    }
    let field = chain_config_storage().participation_floor_bps_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::ParticipationFloorBpsChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

/// Public handler `0x485fd959` (`getParticipationJailDisabled`).
///
/// Returns the configured participation jail disabled.
pub fn get_participation_jail_disabled<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &chain_config_storage()
            .participation_jail_disabled_accessor()
            .get_checked(sdk)?,
    )
}

/// Public handler `0x8664f2e7` (`setParticipationJailDisabled`).
///
/// Updates the configured participation jail disabled.
pub fn set_participation_jail_disabled<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<BoolCommand>(input)?.value;
    let field = chain_config_storage().participation_jail_disabled_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::ParticipationJailDisabledChanged {
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
    field.set_checked(sdk, value as u64)?;
    events::ActiveValidatorsLengthChanged {
        prev_value: previous as u32,
        new_value: value,
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
