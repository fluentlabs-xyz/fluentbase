#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
//! Fluent validator staking rWasm contract.
//!
//! Staking is deployed at a fixed genesis address but executes as a normal
//! rWasm smart contract. Unlike a system precompile, it uses `SharedAPI`, so
//! the delegation/reward implementation can call the canonical BLEND ERC-20.
//!
//! See `README.md` for lifecycle, scope, and accounting invariants.

extern crate alloc;

mod bls;
mod config;
mod consensus;
mod consts;
mod events;
mod evidence;
mod initializer;
mod liveness;
mod math;
mod staking;
mod storage;
mod types;
mod util;

#[cfg(test)]
mod tests;

use consts::*;
use fluentbase_sdk::{entrypoint, ExitCode, SharedAPI};
use util::revert;

/// Decode Solidity calldata and route it to the matching staking operation.
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
        // Initializer
        SIG_INITIALIZE => initializer::initialize(sdk, params),

        // ChainConfig
        SIG_GET_STAKING_TOKEN => config::get_staking_token(sdk),
        SIG_GET_ACTIVE_VALIDATORS_LENGTH => config::get_active_validators_length(sdk),
        SIG_GET_EPOCH_BLOCK_INTERVAL => config::get_epoch_block_interval(sdk),
        SIG_GET_DPOS_ACTIVATION_BLOCK => config::get_dpos_activation_block(sdk),
        SIG_GET_UNDELEGATE_PERIOD => config::get_undelegate_period(sdk),
        SIG_GET_MIN_VALIDATOR_STAKE_AMOUNT => config::get_min_validator_stake_amount(sdk),
        SIG_GET_MIN_STAKING_AMOUNT => config::get_min_staking_amount(sdk),
        SIG_GET_SLASH_FUND_ADDRESS => config::get_slash_fund_address(sdk),
        SIG_SET_SLASH_FUND_ADDRESS => config::set_slash_fund_address(sdk, params),
        SIG_GET_BLEND_STIPEND_PER_EPOCH => config::get_blend_stipend_per_epoch(sdk),
        SIG_SET_BLEND_STIPEND_PER_EPOCH => config::set_blend_stipend_per_epoch(sdk, params),
        SIG_SET_ACTIVE_VALIDATORS_LENGTH => config::set_active_validators_length(sdk, params),
        SIG_SET_EPOCH_BLOCK_INTERVAL => config::set_epoch_block_interval(sdk, params),
        SIG_SET_DPOS_ACTIVATION_BLOCK => config::set_dpos_activation_block(sdk, params),
        SIG_SET_UNDELEGATE_PERIOD => config::set_undelegate_period(sdk, params),
        SIG_SET_MIN_VALIDATOR_STAKE_AMOUNT => config::set_min_validator_stake_amount(sdk, params),
        SIG_SET_MIN_STAKING_AMOUNT => config::set_min_staking_amount(sdk, params),
        SIG_GET_BLEND_RESERVE => config::get_blend_reserve(sdk),
        SIG_SET_BLEND_RESERVE => config::set_blend_reserve(sdk, params),
        SIG_APPLY_BLEND_RESERVE => config::apply_blend_reserve(sdk),
        SIG_CANCEL_BLEND_RESERVE => config::cancel_blend_reserve(sdk),
        SIG_GET_PENDING_BLEND_RESERVE => config::get_pending_blend_reserve(sdk),
        SIG_GET_MIN_VERDICT_DUE_BLOCKS => config::get_min_verdict_due_blocks(sdk),
        SIG_SET_MIN_VERDICT_DUE_BLOCKS => config::set_min_verdict_due_blocks(sdk, params),
        SIG_GET_EXCLUSION_BACKOFF_CAP => config::get_exclusion_backoff_cap(sdk),
        SIG_SET_EXCLUSION_BACKOFF_CAP => config::set_exclusion_backoff_cap(sdk, params),
        SIG_GET_PRODUCTION_LIVENESS_DISABLED => config::get_production_liveness_disabled(sdk),
        SIG_SET_PRODUCTION_LIVENESS_DISABLED => {
            config::set_production_liveness_disabled(sdk, params)
        }

        // ProductionLiveness
        #[cfg(feature = "devnet-views")]
        SIG_BLOCKS_IN_EPOCH => liveness::blocks_in_epoch(sdk, params),
        #[cfg(feature = "devnet-views")]
        SIG_PRODUCED_AT => liveness::produced_at(sdk, params),
        #[cfg(feature = "devnet-views")]
        SIG_PENDING_EXCLUSIONS => liveness::pending_exclusions(sdk),
        #[cfg(feature = "devnet-views")]
        SIG_LAST_PROCESSED_BLOCK => liveness::last_processed_block(sdk),
        SIG_RECORD_PRODUCTION => liveness::record_production(sdk, params),

        // Staking
        SIG_CURRENT_EPOCH => staking::current_epoch_read(sdk),
        SIG_NEXT_EPOCH => staking::next_epoch_read(sdk),
        SIG_IS_VALIDATOR => staking::is_validator(sdk, params),
        SIG_IS_VALIDATOR_ACTIVE => staking::is_validator_active(sdk, params),
        SIG_GET_VALIDATOR_STATUS => staking::get_validator_status(sdk, params),
        SIG_GET_VALIDATOR_BY_OWNER => staking::get_validator_by_owner(sdk, params),
        SIG_GET_VALIDATORS => staking::get_validators(sdk),
        SIG_ACTIVATE_VALIDATOR => staking::activate_validator(sdk, params),
        SIG_DISABLE_VALIDATOR => staking::disable_validator(sdk, params),
        SIG_CHANGE_VALIDATOR_COMMISSION_RATE => staking::change_commission(sdk, params),
        SIG_GET_VALIDATOR_DELEGATION => staking::get_validator_delegation(sdk, params),
        SIG_GET_VALIDATOR_DELEGATED_STAKE_AT => {
            staking::get_validator_delegated_stake_at(sdk, params)
        }
        SIG_REGISTER_VALIDATOR => staking::register_validator(sdk, params),
        SIG_DELEGATE => staking::delegate(sdk, params),
        SIG_UNDELEGATE => staking::undelegate(sdk, params),
        SIG_GET_VALIDATOR_FEE => staking::get_validator_fee(sdk, params),
        SIG_CLAIM_VALIDATOR_FEE => staking::claim_validator_fee(sdk, params),
        SIG_GET_DELEGATOR_FEE => staking::get_delegator_fee(sdk, params),
        SIG_CLAIM_DELEGATOR_FEE => staking::claim_delegator_fee(sdk, params),
        SIG_GET_DELEGATOR_PRINCIPAL => staking::get_delegator_principal(sdk, params),
        SIG_WITHDRAW_DELEGATOR_PRINCIPAL => staking::withdraw_delegator_principal(sdk, params),
        SIG_REDELEGATE_DELEGATOR_FEE => staking::redelegate_delegator_fee(sdk, params),
        SIG_GET_EPOCH_REWARDS => staking::get_epoch_rewards(sdk, params),

        // Consensus
        SIG_GET_CONSENSUS_KEYS => consensus::get_consensus_keys(sdk, params),
        SIG_GET_REGISTRY_WITH_KEYS => consensus::get_registry_with_keys(sdk),
        SIG_NEXT_EPOCH_TO_COMMIT => consensus::next_epoch_to_commit(sdk),
        SIG_COMMIT_EPOCH_COMMITTEE => consensus::commit_epoch_committee(sdk),
        SIG_GET_DKG_QUAL => consensus::get_dkg_qual(sdk, params),
        SIG_GET_EPOCH_COMMITTEE => consensus::get_epoch_committee(sdk, params),
        SIG_GET_EPOCH_COMMITTEE_WITH_STAKES => {
            consensus::get_epoch_committee_with_stakes(sdk, params)
        }
        SIG_SLASH_EQUIVOCATION => consensus::slash_equivocation(sdk, params),
        SIG_SLASH_EQUIVOCATION_NOTARIZE => consensus::slash_notarize(sdk, params),
        SIG_SLASH_EQUIVOCATION_FINALIZE => consensus::slash_finalize(sdk, params),
        SIG_SLASH_EQUIVOCATION_NULLIFY_FINALIZE => consensus::slash_nullify_finalize(sdk, params),

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
