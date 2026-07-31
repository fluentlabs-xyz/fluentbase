//! One-shot contract initialization.

use crate::{
    config,
    consts::{
        COMMISSION_RATE_MAX, ERR_ALREADY_INITIALIZED, ERR_BAD_COMMISSION_RATE,
        ERR_MALFORMED_INPUT_LENGTH, ERR_ZERO_OWNER,
    },
    staking::set_validator,
    storage::{initializer_storage, staking_storage, STATUS_ACTIVE},
    types::InitializeCommand,
    util::{decode_args, ensure_mutable, ensure_non_payable, revert, revert_with},
};
use fluentbase_sdk::{Address, ExitCode, SharedAPI, U256};

/// Public handler `0xdca9ac1b` (`initialize`).
///
/// Atomically initializes the owner, chain configuration, dependencies, and genesis validators.
pub fn initialize<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    let initializer = initializer_storage();
    // Initialization is intentionally permissionless: deployment executes this
    // call atomically with contract installation, before public transactions can
    // race to initialize. The stored flag below keeps all later calls one-shot.
    if initializer.initialized_accessor().get_checked(sdk)? {
        return revert(sdk, ERR_ALREADY_INITIALIZED);
    }

    let command = decode_args::<InitializeCommand>(input)?;
    validate(sdk, &command)?;
    let total_stakes = command
        .initial_stakes
        .iter()
        .try_fold(U256::ZERO, |total, stake| {
            total.checked_add(*stake).ok_or(ExitCode::IntegerOverflow)
        })?;

    config::apply_initial_config(sdk, &command)?;
    staking_storage()
        .owner_accessor()
        .set_checked(sdk, command.initial_owner)?;
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

    // Match Solidity's `initializer` reentrancy guard before the first
    // external token call. A failed call reverts the transaction and storage.
    initializer.initialized_accessor().set_checked(sdk, true)?;
    pull_initial_stakes(sdk, command.initial_owner, total_stakes)
}

fn validate<SDK: SharedAPI>(sdk: &mut SDK, command: &InitializeCommand) -> Result<(), ExitCode> {
    if command.initial_owner.is_zero() {
        return revert(sdk, ERR_ZERO_OWNER);
    }
    if command.validators.len() != command.initial_stakes.len() {
        return revert(sdk, ERR_MALFORMED_INPUT_LENGTH);
    }
    if command.commission_rate > COMMISSION_RATE_MAX {
        return revert_with(sdk, ERR_BAD_COMMISSION_RATE, &command.commission_rate);
    }
    Ok(())
}

#[cfg(not(test))]
fn pull_initial_stakes<SDK: SharedAPI>(
    sdk: &mut SDK,
    owner: Address,
    total_stakes: U256,
) -> Result<(), ExitCode> {
    if total_stakes.is_zero() {
        return Ok(());
    }
    crate::util::safe_transfer_from(sdk, owner, total_stakes)
}

// Unit storage tests use a host without nested-call support. The real genesis
// integration test exercises the ERC-20 pull through the compiled rWasm.
#[cfg(test)]
fn pull_initial_stakes<SDK: SharedAPI>(
    _sdk: &mut SDK,
    _owner: Address,
    _total_stakes: U256,
) -> Result<(), ExitCode> {
    Ok(())
}
