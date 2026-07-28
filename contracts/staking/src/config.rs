use alloc::string::String;

use fluentbase_sdk::{ContextReader, ExitCode, SharedAPI, U256};

use crate::{
    consts::*,
    events,
    storage::staking_storage,
    types::{
        AddressCommand, BoolCommand, ConfigureDependenciesCommand, U256Command, U32Command,
        U64Command,
    },
    util::{
        decode, ensure_governance, ensure_mutable, ensure_non_payable, revert, revert_with,
        write_abi,
    },
};

fn ensure_governance_mutation<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    ensure_governance(sdk)
}

pub fn default_participation_floor_bps<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &DEFAULT_PARTICIPATION_FLOOR_BPS)
}

pub fn default_slash_reporter_bps<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &DEFAULT_SLASH_REPORTER_REWARD_BPS)
}

pub fn max_active_validators<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &(MAX_ACTIVE_VALIDATORS_LENGTH as u32))
}

pub fn max_blend_stipend_per_epoch<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &MAX_BLEND_STIPEND_PER_EPOCH)
}

pub fn max_participation_floor_bps<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &MAX_PARTICIPATION_FLOOR_BPS)
}

pub fn max_slash_reporter_bps<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(sdk, &MAX_SLASH_REPORTER_REWARD_BPS)
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
    let minimum = staking_storage()
        .config_accessor()
        .min_undelegate_blocks_accessor()
        .get_checked(sdk)?;
    if window < minimum {
        return revert_with(sdk, ERR_UNDELEGATE_WINDOW_TOO_SHORT, &(window, minimum));
    }
    Ok(())
}

pub fn configure_dependencies<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let storage = staking_storage();
    let command: ConfigureDependenciesCommand = decode(input)?;
    if command.liveness_slashing.is_zero() {
        return zero_value(sdk, "livenessSlashing");
    }
    if command.blend_reserve.is_zero() {
        return zero_value(sdk, "blendReserve");
    }
    let config = storage.config_accessor();
    config
        .liveness_slashing_accessor()
        .set_checked(sdk, command.liveness_slashing)?;
    config
        .blend_reserve_accessor()
        .set_checked(sdk, command.blend_reserve)
}

pub fn get_felony_threshold<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &staking_storage()
            .config_accessor()
            .felony_threshold_accessor()
            .get_checked(sdk)?,
    )
}

pub fn set_felony_threshold<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "felonyThreshold");
    }
    let field = staking_storage()
        .config_accessor()
        .felony_threshold_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::FelonyThresholdChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

pub fn get_validator_jail_epoch_length<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &staking_storage()
            .config_accessor()
            .validator_jail_epoch_length_accessor()
            .get_checked(sdk)?,
    )
}

pub fn set_validator_jail_epoch_length<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "validatorJailEpochLength");
    }
    let field = staking_storage()
        .config_accessor()
        .validator_jail_epoch_length_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::ValidatorJailEpochLengthChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

pub fn get_slash_reporter_reward_bps<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let stored = staking_storage()
        .config_accessor()
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
    let field = staking_storage()
        .config_accessor()
        .slash_reporter_reward_bps_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::SlashReporterRewardBpsChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

pub fn get_slash_fund_address<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &staking_storage()
            .config_accessor()
            .slash_fund_address_accessor()
            .get_checked(sdk)?,
    )
}

pub fn set_slash_fund_address<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<AddressCommand>(input)?.value;
    if value.is_zero() {
        return zero_value(sdk, "slashFundAddress");
    }
    let field = staking_storage()
        .config_accessor()
        .slash_fund_address_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::SlashFundAddressChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

pub fn get_participation_floor_bps<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let stored = staking_storage()
        .config_accessor()
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
    let field = staking_storage()
        .config_accessor()
        .participation_floor_bps_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::ParticipationFloorBpsChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

pub fn get_participation_jail_disabled<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &staking_storage()
            .config_accessor()
            .participation_jail_disabled_accessor()
            .get_checked(sdk)?,
    )
}

pub fn set_participation_jail_disabled<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<BoolCommand>(input)?.value;
    let field = staking_storage()
        .config_accessor()
        .participation_jail_disabled_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::ParticipationJailDisabledChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

pub fn get_blend_stipend_per_epoch<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &staking_storage()
            .config_accessor()
            .blend_stipend_per_epoch_accessor()
            .get_checked(sdk)?,
    )
}

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
    let field = staking_storage()
        .config_accessor()
        .blend_stipend_per_epoch_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::BlendStipendPerEpochChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

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
    let field = staking_storage()
        .config_accessor()
        .active_validators_length_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value as u64)?;
    events::ActiveValidatorsLengthChanged {
        prev_value: previous as u32,
        new_value: value,
    }
    .emit(sdk)
}

pub fn set_epoch_block_interval<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "epochBlockInterval");
    }
    let config = staking_storage().config_accessor();
    let activation = config.dpos_activation_block_accessor().get_checked(sdk)?;
    if sdk.context().block_number() >= activation {
        return revert(sdk, ERR_DPOS_ALREADY_ACTIVE);
    }
    if activation % value as u64 != 0 {
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

pub fn set_dpos_activation_block<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U64Command>(input)?.value;
    let config = staking_storage().config_accessor();
    let previous = config.dpos_activation_block_accessor().get_checked(sdk)?;
    if sdk.context().block_number() >= previous {
        return revert(sdk, ERR_DPOS_ALREADY_ACTIVE);
    }
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

pub fn set_undelegate_period<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "undelegatePeriod");
    }
    let config = staking_storage().config_accessor();
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

pub fn set_min_validator_stake_amount<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U256Command>(input)?.value;
    require_nonzero(sdk, value, "minValidatorStakeAmount")?;
    let field = staking_storage()
        .config_accessor()
        .min_validator_stake_amount_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::MinValidatorStakeAmountChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

pub fn set_min_staking_amount<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U256Command>(input)?.value;
    require_nonzero(sdk, value, "minStakingAmount")?;
    let field = staking_storage()
        .config_accessor()
        .min_staking_amount_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::MinStakingAmountChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

pub fn get_bls_verifier<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &staking_storage()
            .config_accessor()
            .bls_verifier_accessor()
            .get_checked(sdk)?,
    )
}

pub fn set_bls_verifier<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<AddressCommand>(input)?.value;
    if value.is_zero() {
        return zero_value(sdk, "blsVerifier");
    }
    let field = staking_storage().config_accessor().bls_verifier_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::BlsVerifierChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}

pub fn get_evidence_decoder<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    write_abi(
        sdk,
        &staking_storage()
            .config_accessor()
            .evidence_decoder_accessor()
            .get_checked(sdk)?,
    )
}

pub fn set_evidence_decoder<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<AddressCommand>(input)?.value;
    if value.is_zero() {
        return zero_value(sdk, "evidenceDecoder");
    }
    let field = staking_storage()
        .config_accessor()
        .evidence_decoder_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value)?;
    events::EvidenceDecoderChanged {
        prev_value: previous,
        new_value: value,
    }
    .emit(sdk)
}
