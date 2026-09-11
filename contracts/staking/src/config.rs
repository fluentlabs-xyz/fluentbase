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
        current_epoch, decode, ensure_governance, ensure_mutable, ensure_non_payable, next_epoch,
        revert, revert_with, write_abi, write_returns,
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
    // The same floor the setter enforces, for the same reason: the cap truncates
    // the selection, so a cap below `MIN_COMMITTEE_LENGTH` makes every
    // `commitEpochCommittee` derive fewer members than the floor and revert on a
    // pre-execution system call. At genesis that stops the chain at block zero
    // rather than mid-run, but it stops it either way, and refusing the cap here
    // is the only place a genesis can still be told which value was wrong. The
    // zero case is subsumed: zero is below the floor.
    if (command.active_validators_length as usize) < MIN_COMMITTEE_LENGTH {
        return revert_with(
            sdk,
            ERR_ACTIVE_VALIDATORS_LENGTH_BELOW_COMMITTEE_FLOOR,
            &(
                command.active_validators_length,
                MIN_COMMITTEE_LENGTH as u32,
            ),
        );
    }
    if command.active_validators_length as u64 > MAX_COMMITTEE_SIZE
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
    if command.blend_reserve.is_zero() {
        return revert_with(sdk, ERR_ZERO_VALUE, &String::from("blendReserve"));
    }
    Ok(())
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

/// `(this epoch, the epoch a declaration made now becomes applicable at)`.
fn timelock_effective_epoch<SDK: SharedAPI>(sdk: &SDK) -> Result<(u64, u64), ExitCode> {
    let declared_at = current_epoch(sdk)?;
    Ok((declared_at, timelock_window(declared_at)?.0))
}

/// Refuses unless `declared` names a real declaration whose term has elapsed.
///
/// The zero address is the "nothing declared" sentinel — both declaring setters
/// refuse a zero, so it cannot collide with a real value, while epoch zero is an
/// ordinary epoch to declare in and could not serve as one.
fn ensure_timelock_elapsed<SDK: SharedAPI>(
    sdk: &mut SDK,
    declared: Address,
    declared_at: u64,
) -> Result<(), ExitCode> {
    if declared.is_zero() {
        return revert(sdk, ERR_NO_PENDING_CHANGE);
    }
    let current = current_epoch(sdk)?;
    let (effective_at, expires_at) = timelock_window(declared_at)?;
    if current < effective_at {
        return revert_with(sdk, ERR_TIMELOCK_NOT_ELAPSED, &(current, effective_at));
    }
    // The declaration EXPIRES. Without this a rotation declared and then
    // abandoned stays armed for the life of the chain, and the scheme's own
    // guarantee inverts: a key stolen months later lands it in one block, the
    // notice period having scrolled past while nobody was watching. Past the
    // window governance must declare again and wait again, which puts the notice
    // back in front of the change.
    if current >= expires_at {
        return revert_with(sdk, ERR_TIMELOCK_EXPIRED, &(current, expires_at));
    }
    Ok(())
}

/// `(first epoch a declaration may land, first epoch it may no longer land)`.
fn timelock_window(declared_at: u64) -> Result<(u64, u64), ExitCode> {
    let effective_at = declared_at
        .checked_add(ADDRESS_SETTER_TIMELOCK_EPOCHS)
        .ok_or(ExitCode::IntegerOverflow)?;
    let expires_at = effective_at
        .checked_add(ADDRESS_SETTER_APPLY_WINDOW_EPOCHS)
        .ok_or(ExitCode::IntegerOverflow)?;
    Ok((effective_at, expires_at))
}

/// Public handler `0xa79e7263` (`setSlashFundAddress`).
///
/// Updates the configured slash fund address IMMEDIATELY, and deliberately not
/// through the two-step timelock its sibling `setBlendReserve` carries.
///
/// It WAS timelocked, on 2026-09-11, and exempted again the same day for a reason
/// the first pass had not seen. A timelock protects a setting a stolen key could
/// profit from; this address is where a seizure GOES, not a pot it can drain, so
/// the protection is thin. What it costs is not: `seize_self_stake` reverts the
/// whole penalty when the recipient refuses the transfer, so a token that
/// blacklists the configured fund makes equivocation unslashable until the
/// address moves — and behind a seven-epoch timelock that is a week of an
/// offender keeping its seat, its bond and its rewards, free to go on
/// equivocating. It cannot fall back to the burn sink either: `seize_self_stake`
/// reaches `EQUIVOCATION_BURN_SINK` only when the STORED address is zero, and
/// this setter refuses a zero. Immediate rotation IS the repair path that revert
/// depends on, so the two have to agree, and the revert is the one that cannot
/// move.
///
/// `setBlendReserve` keeps its timelock: it names the account the stipend is
/// pulled from, which a stolen key genuinely can point at itself.
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
    // The cap truncates the selection, so a cap below the committee floor makes
    // every commit derive fewer members than `MIN_COMMITTEE_LENGTH` and revert —
    // on a pre-execution system call, which stops the chain with no transaction
    // able to put the cap back. Refusing it here is the only place the two can
    // still be reconciled by a transaction.
    if (value as usize) < MIN_COMMITTEE_LENGTH {
        return revert_with(
            sdk,
            ERR_ACTIVE_VALIDATORS_LENGTH_BELOW_COMMITTEE_FLOOR,
            &(value, MIN_COMMITTEE_LENGTH as u32),
        );
    }
    if value as u64 > MAX_COMMITTEE_SIZE {
        return revert_with(
            sdk,
            ERR_MAX_ACTIVE_VALIDATORS_EXCEEDED,
            &(value, MAX_COMMITTEE_SIZE as u32),
        );
    }
    let field = chain_config_storage().active_validators_length_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, value as u64)?;
    let effective_epoch = next_epoch(sdk)?;
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
/// The floor is a minimum stake share in disguise — a member is judged once its
/// share reaches `minVerdictDueBlocks / epochBlockInterval`. Zero would judge a
/// member that was never due a single slot, and the shipped default is the
/// ceiling: it already sits where the verdict test is conclusive, so lowering it
/// trades confidence for reach and raising it would buy nothing.
///
/// That ceiling is an absolute bound on the parameter, not a guarantee that any
/// member ends up judgeable: which shares are met is decided by the live stake
/// distribution, not by this setting.
pub fn set_min_verdict_due_blocks<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<U32Command>(input)?.value;
    if value == 0 {
        return zero_value(sdk, "minVerdictDueBlocks");
    }
    if value > DEFAULT_MIN_VERDICT_DUE_BLOCKS {
        return revert_with(
            sdk,
            ERR_MIN_VERDICT_DUE_BLOCKS_TOO_HIGH,
            &(value, DEFAULT_MIN_VERDICT_DUE_BLOCKS),
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

/// Public handler `0x37dff538` (`getBlendReserve`).
///
/// Returns the address the epoch stipend is drawn from.
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
/// DECLARES a new address for the epoch stipend to be drawn from. It does not
/// become the reserve until `applyBlendReserve` lands it,
/// `ADDRESS_SETTER_TIMELOCK_EPOCHS` later; a second declaration overwrites the
/// first and restarts the clock.
///
/// What the timelock does NOT change: when the rotation does land there is still
/// no grace period. The first epoch to close after it reads the new address, and
/// an address that cannot cover the pot forfeits that epoch permanently, so
/// landing on an unfunded or unapproved holder burns every epoch until it is
/// funded — the same way a fresh chain burns the epochs that run before its
/// treasury approves. The notice period is time to fund the new holder, and it
/// is worth using for that.
pub fn set_blend_reserve<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let value = decode::<AddressCommand>(input)?.value;
    if value.is_zero() {
        return zero_value(sdk, "blendReserve");
    }
    let (declared_at, effective_at) = timelock_effective_epoch(sdk)?;
    let config = chain_config_storage();
    config
        .pending_blend_reserve_accessor()
        .set_checked(sdk, value)?;
    config
        .pending_blend_reserve_epoch_accessor()
        .set_checked(sdk, declared_at)?;
    events::BlendReserveDeclared {
        new_value: value,
        declared_at_epoch: declared_at,
        effective_at_epoch: effective_at,
    }
    .emit(sdk)
}

/// Public handler `applyBlendReserve()`.
///
/// Lands the declared reserve and clears the declaration.
pub fn apply_blend_reserve<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let config = chain_config_storage();
    let declared = config.pending_blend_reserve_accessor().get_checked(sdk)?;
    let declared_at = config
        .pending_blend_reserve_epoch_accessor()
        .get_checked(sdk)?;
    ensure_timelock_elapsed(sdk, declared, declared_at)?;
    let field = config.blend_reserve_accessor();
    let previous = field.get_checked(sdk)?;
    field.set_checked(sdk, declared)?;
    config
        .pending_blend_reserve_accessor()
        .set_checked(sdk, Address::ZERO)?;
    config
        .pending_blend_reserve_epoch_accessor()
        .set_checked(sdk, 0)?;
    events::BlendReserveChanged {
        prev_value: previous,
        new_value: declared,
    }
    .emit(sdk)
}

/// Public handler `cancelBlendReserve()`.
///
/// Withdraws an outstanding declaration without landing it.
///
/// The declaring setter refuses the zero address, so before this existed the
/// ONLY way to clear the pending pair was to apply it: governance that changed
/// its mind could replace the declared address but never return to "nothing
/// pending". Together with the expiry in `ensure_timelock_elapsed` this is what
/// keeps an abandoned rotation from sitting armed — the expiry bounds how long a
/// forgotten one stays dangerous, and this is how a remembered one is put down
/// immediately.
pub fn cancel_blend_reserve<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_governance_mutation(sdk)?;
    let config = chain_config_storage();
    let declared = config.pending_blend_reserve_accessor().get_checked(sdk)?;
    if declared.is_zero() {
        return revert(sdk, ERR_NO_PENDING_CHANGE);
    }
    config
        .pending_blend_reserve_accessor()
        .set_checked(sdk, Address::ZERO)?;
    config
        .pending_blend_reserve_epoch_accessor()
        .set_checked(sdk, 0)?;
    events::BlendReserveDeclarationCancelled {
        cancelled_value: declared,
    }
    .emit(sdk)
}

/// Public handler `getPendingBlendReserve()`.
///
/// The outstanding declaration: `(address, declared-at epoch, first epoch it may
/// land, first epoch it may no longer land)`, or four zeroes when none is
/// outstanding.
///
/// Without it "is a rotation armed, and when does it land?" is answerable only by
/// replaying `BlendReserveDeclared` from genesis — and a declaration that has
/// neither landed nor been cancelled is exactly the state most worth being able
/// to query.
pub fn get_pending_blend_reserve<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    let config = chain_config_storage();
    let declared = config.pending_blend_reserve_accessor().get_checked(sdk)?;
    if declared.is_zero() {
        return write_returns(sdk, &(Address::ZERO, 0u64, 0u64, 0u64));
    }
    let declared_at = config
        .pending_blend_reserve_epoch_accessor()
        .get_checked(sdk)?;
    let (effective_at, expires_at) = timelock_window(declared_at)?;
    write_returns(sdk, &(declared, declared_at, effective_at, expires_at))
}
