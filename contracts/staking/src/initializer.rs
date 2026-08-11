//! One-shot contract initialization.

use crate::{
    config,
    consensus::{store_consensus_keys, verify_consensus_keys},
    consts::{
        COMMISSION_RATE_MAX, ERR_ALREADY_INITIALIZED, ERR_BAD_COMMISSION_RATE,
        ERR_INITIAL_STAKE_TOO_LOW, ERR_MALFORMED_INPUT_LENGTH, ERR_ZERO_OWNER, STATUS_ACTIVE,
    },
    staking::set_validator,
    storage::initializer_storage,
    types::InitializeCommand,
    util::{
        decode_args, ensure_mutable, ensure_non_payable, revert, revert_with, safe_transfer_from,
    },
};
use fluentbase_sdk::{Address, ExitCode, SharedAPI, U256};

/// Public handler `0x4b4b21a5` (`initialize`).
///
/// Atomically initializes chain configuration, dependencies, and genesis validators.
pub fn initialize<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) -> Result<(), ExitCode> {
    ensure_non_payable(sdk)?;
    ensure_mutable(sdk)?;
    let initializer = initializer_storage();
    // Initialization is intentionally permissionless: deployment executes this
    // call atomically with contract installation, before public transactions can
    // race to initialize. The stored flag below keeps all later calls one-shot.
    if initializer.initialized_accessor().get_checked(sdk)?
        || initializer.initializing_accessor().get_checked(sdk)?
    {
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
    // Keep the public initialized flag false while BLS verification calls are
    // in flight, and separately reject a nested initialize attempt.
    initializer.initializing_accessor().set_checked(sdk, true)?;
    for ((((validator, stake), bls_pubkey), bls_pop), peer_pubkey) in command
        .validators
        .into_iter()
        .zip(command.initial_stakes)
        .zip(command.bls_pubkeys_uncompressed)
        .zip(command.bls_pops_uncompressed)
        .zip(command.peer_pubkeys)
    {
        let verified = verify_consensus_keys(sdk, validator, bls_pubkey, bls_pop, peer_pubkey)?;
        set_validator(
            sdk,
            validator,
            validator,
            STATUS_ACTIVE,
            command.commission_rate,
            stake,
            0,
        )?;
        store_consensus_keys(sdk, validator, verified, 0)?;
    }

    // Match Solidity's `initializer` reentrancy guard before the first
    // external token call. A failed call reverts the transaction and storage.
    initializer.initialized_accessor().set_checked(sdk, true)?;
    initializer
        .initializing_accessor()
        .set_checked(sdk, false)?;
    pull_initial_stakes(sdk, command.initial_stake_owner, total_stakes)
}

fn validate<SDK: SharedAPI>(sdk: &mut SDK, command: &InitializeCommand) -> Result<(), ExitCode> {
    if command.initial_stake_owner.is_zero() {
        return revert(sdk, ERR_ZERO_OWNER);
    }
    if command.validators.len() != command.initial_stakes.len()
        || command.validators.len() != command.bls_pubkeys_uncompressed.len()
        || command.validators.len() != command.bls_pops_uncompressed.len()
        || command.validators.len() != command.peer_pubkeys.len()
    {
        return revert(sdk, ERR_MALFORMED_INPUT_LENGTH);
    }
    if command.commission_rate > COMMISSION_RATE_MAX {
        return revert_with(sdk, ERR_BAD_COMMISSION_RATE, &command.commission_rate);
    }
    if let Some(stake) = command
        .initial_stakes
        .iter()
        .find(|stake| **stake < command.min_validator_stake_amount)
    {
        return revert_with(sdk, ERR_INITIAL_STAKE_TOO_LOW, stake);
    }
    Ok(())
}

fn pull_initial_stakes<SDK: SharedAPI>(
    sdk: &mut SDK,
    sponsor: Address,
    total_stakes: U256,
) -> Result<(), ExitCode> {
    if total_stakes.is_zero() {
        return Ok(());
    }
    safe_transfer_from(sdk, sponsor, total_stakes)
}
