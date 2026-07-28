#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
//! Fluent validator staking rWasm contract.
//!
//! Staking is deployed at a fixed genesis address but executes as a normal
//! rWasm smart contract. Unlike a system precompile, it uses `SharedAPI`, so
//! the delegation/reward implementation can call the canonical BLEND ERC-20.

extern crate alloc;

mod consts;
mod events;
mod handlers;
mod math;
mod storage;
mod types;
mod util;

#[cfg(test)]
mod tests;

use consts::*;
use fluentbase_sdk::{entrypoint, ExitCode, SharedAPI};
use util::revert;

pub fn main_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let input = sdk.bytes_input();
    if input.len() < SIG_LEN_BYTES {
        return Err(ExitCode::MalformedBuiltinParams);
    }
    let (selector, params) = input.split_at(SIG_LEN_BYTES);
    let selector = u32::from_be_bytes(
        selector
            .try_into()
            .map_err(|_| ExitCode::MalformedBuiltinParams)?,
    );
    match selector {
        SIG_INITIALIZE => handlers::initialize(sdk, params),
        SIG_CONFIGURE => handlers::configure(sdk, params),
        SIG_CURRENT_EPOCH => handlers::current_epoch_read(sdk),
        SIG_NEXT_EPOCH => handlers::next_epoch_read(sdk),
        SIG_OWNER => handlers::owner(sdk),
        SIG_GET_STAKING => handlers::get_staking(sdk),
        SIG_GET_CHAIN_CONFIG => handlers::get_chain_config(sdk),
        SIG_GET_GOVERNANCE => handlers::get_governance(sdk),
        SIG_GET_STAKING_TOKEN => handlers::get_staking_token(sdk),
        SIG_IS_VALIDATOR => handlers::is_validator(sdk, params),
        SIG_IS_VALIDATOR_ACTIVE => handlers::is_validator_active(sdk, params),
        SIG_GET_VALIDATOR_STATUS => handlers::get_validator_status(sdk, params),
        SIG_GET_VALIDATOR_BY_OWNER => handlers::get_validator_by_owner(sdk, params),
        SIG_GET_VALIDATORS => handlers::get_validators(sdk),
        SIG_ADD_VALIDATOR => handlers::add_validator(sdk, params),
        SIG_REMOVE_VALIDATOR => handlers::remove_validator(sdk, params),
        SIG_ACTIVATE_VALIDATOR => handlers::activate_validator(sdk, params),
        SIG_DISABLE_VALIDATOR => handlers::disable_validator(sdk, params),
        SIG_CHANGE_VALIDATOR_COMMISSION_RATE => handlers::change_commission(sdk, params),
        SIG_CHANGE_VALIDATOR_OWNER => handlers::change_owner(sdk, params),
        _ => revert(sdk, ERR_UNKNOWN_METHOD),
    }
}

pub fn contract_main<SDK: SharedAPI>(mut sdk: SDK) {
    match main_entry(&mut sdk) {
        Ok(()) => sdk.exit(),
        Err(exit_code) => sdk.native_exit(exit_code),
    }
}

entrypoint!(contract_main);
