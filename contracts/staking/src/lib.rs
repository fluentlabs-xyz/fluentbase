#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
//! Fluent validator staking rWasm contract.
//!
//! Staking is deployed at a fixed genesis address but executes as a normal
//! rWasm smart contract. Unlike a system precompile, it uses `SharedAPI`, so
//! the delegation/reward implementation can call the canonical BLEND ERC-20.
//!
//! See `README.md` for lifecycle, scope, and accounting invariants.

extern crate alloc;

mod config;
mod consensus;
mod consts;
mod economics;
mod equivocation;
mod events;
mod handlers;
mod liveness;
mod math;
mod rewards;
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
    if let Some(result) = config::dispatch_constant(sdk, selector) {
        return result;
    }
    match selector {
        // Deployment and governance configuration.
        SIG_INITIALIZE => handlers::initialize(sdk, params),
        SIG_CONFIGURE => handlers::configure(sdk, params),
        SIG_CONFIGURE_DEPENDENCIES => config::configure_dependencies(sdk, params),
        // Compatibility getters and governance parameter access.
        SIG_CURRENT_EPOCH => handlers::current_epoch_read(sdk),
        SIG_NEXT_EPOCH => handlers::next_epoch_read(sdk),
        SIG_OWNER => handlers::owner(sdk),
        SIG_GET_STAKING => handlers::get_staking(sdk),
        SIG_GET_CHAIN_CONFIG => handlers::get_chain_config(sdk),
        SIG_GET_GOVERNANCE => handlers::get_governance(sdk),
        SIG_GET_STAKING_TOKEN => handlers::get_staking_token(sdk),
        SIG_GET_ACTIVE_VALIDATORS_LENGTH => handlers::get_active_validators_length(sdk),
        SIG_GET_EPOCH_BLOCK_INTERVAL => handlers::get_epoch_block_interval(sdk),
        SIG_GET_DPOS_ACTIVATION_BLOCK => handlers::get_dpos_activation_block(sdk),
        SIG_GET_UNDELEGATE_PERIOD => handlers::get_undelegate_period(sdk),
        SIG_GET_MIN_VALIDATOR_STAKE_AMOUNT => handlers::get_min_validator_stake_amount(sdk),
        SIG_GET_MIN_STAKING_AMOUNT => handlers::get_min_staking_amount(sdk),
        SIG_GET_FELONY_THRESHOLD => config::get_felony_threshold(sdk),
        SIG_SET_FELONY_THRESHOLD => config::set_felony_threshold(sdk, params),
        SIG_GET_VALIDATOR_JAIL_EPOCH_LENGTH => config::get_validator_jail_epoch_length(sdk),
        SIG_SET_VALIDATOR_JAIL_EPOCH_LENGTH => config::set_validator_jail_epoch_length(sdk, params),
        SIG_GET_SLASH_REPORTER_REWARD_BPS => config::get_slash_reporter_reward_bps(sdk),
        SIG_SET_SLASH_REPORTER_REWARD_BPS => config::set_slash_reporter_reward_bps(sdk, params),
        SIG_GET_SLASH_FUND_ADDRESS => config::get_slash_fund_address(sdk),
        SIG_SET_SLASH_FUND_ADDRESS => config::set_slash_fund_address(sdk, params),
        SIG_GET_PARTICIPATION_FLOOR_BPS => config::get_participation_floor_bps(sdk),
        SIG_SET_PARTICIPATION_FLOOR_BPS => config::set_participation_floor_bps(sdk, params),
        SIG_GET_PARTICIPATION_JAIL_DISABLED => config::get_participation_jail_disabled(sdk),
        SIG_SET_PARTICIPATION_JAIL_DISABLED => config::set_participation_jail_disabled(sdk, params),
        SIG_GET_BLEND_STIPEND_PER_EPOCH => config::get_blend_stipend_per_epoch(sdk),
        SIG_SET_BLEND_STIPEND_PER_EPOCH => config::set_blend_stipend_per_epoch(sdk, params),
        SIG_SET_ACTIVE_VALIDATORS_LENGTH => config::set_active_validators_length(sdk, params),
        SIG_SET_EPOCH_BLOCK_INTERVAL => config::set_epoch_block_interval(sdk, params),
        SIG_SET_DPOS_ACTIVATION_BLOCK => config::set_dpos_activation_block(sdk, params),
        SIG_SET_UNDELEGATE_PERIOD => config::set_undelegate_period(sdk, params),
        SIG_SET_MIN_VALIDATOR_STAKE_AMOUNT => config::set_min_validator_stake_amount(sdk, params),
        SIG_SET_MIN_STAKING_AMOUNT => config::set_min_staking_amount(sdk, params),
        SIG_GET_BLS_VERIFIER => config::get_bls_verifier(sdk),
        SIG_SET_BLS_VERIFIER => config::set_bls_verifier(sdk, params),
        SIG_GET_EVIDENCE_DECODER => config::get_evidence_decoder(sdk),
        SIG_SET_EVIDENCE_DECODER => config::set_evidence_decoder(sdk, params),
        // Validator stake and delegation principal.
        SIG_GET_VALIDATOR_DELEGATION => economics::get_validator_delegation(sdk, params),
        SIG_GET_VALIDATOR_DELEGATED_STAKE_AT => {
            economics::get_validator_delegated_stake_at(sdk, params)
        }
        SIG_REGISTER_VALIDATOR => economics::register_validator(sdk, params),
        SIG_DELEGATE => economics::delegate(sdk, params),
        SIG_UNDELEGATE => economics::undelegate(sdk, params),
        // Validator and delegator rewards.
        SIG_GET_VALIDATOR_FEE => rewards::get_validator_fee(sdk, params),
        SIG_GET_PENDING_VALIDATOR_FEE => rewards::get_pending_validator_fee(sdk, params),
        SIG_CLAIM_VALIDATOR_FEE => rewards::claim_validator_fee(sdk, params),
        SIG_CLAIM_VALIDATOR_FEE_AT_EPOCH => rewards::claim_validator_fee_at_epoch(sdk, params),
        SIG_GET_DELEGATOR_FEE => rewards::get_delegator_fee(sdk, params),
        SIG_GET_PENDING_DELEGATOR_FEE => rewards::get_pending_delegator_fee(sdk, params),
        SIG_CLAIM_DELEGATOR_FEE => rewards::claim_delegator_fee(sdk, params),
        SIG_CLAIM_DELEGATOR_FEE_AT_EPOCH => rewards::claim_delegator_fee_at_epoch(sdk, params),
        SIG_CALC_AVAILABLE_FOR_REDELEGATE_AMOUNT => {
            rewards::calc_available_for_redelegate_amount(sdk, params)
        }
        SIG_REDELEGATE_DELEGATOR_FEE => rewards::redelegate_delegator_fee(sdk, params),
        SIG_GET_EPOCH_REWARDS => rewards::get_epoch_rewards(sdk, params),
        SIG_SETTLE_EPOCH_STIPEND => rewards::settle_epoch_stipend(sdk, params),
        // Consensus identities and committed committees.
        SIG_SET_CONSENSUS_KEYS => consensus::set_consensus_keys(sdk, params),
        SIG_GET_CONSENSUS_KEYS => consensus::get_consensus_keys(sdk, params),
        SIG_GET_VALIDATORS_WITH_KEYS => consensus::get_validators_with_keys(sdk),
        SIG_GET_REGISTRY_WITH_KEYS => consensus::get_registry_with_keys(sdk),
        SIG_GET_VALIDATORS_WITH_KEYS_AT => consensus::get_validators_with_keys_at(sdk, params),
        SIG_NEXT_EPOCH_TO_COMMIT => consensus::next_epoch_to_commit(sdk),
        SIG_COMMITTEE_SELECTION_EPOCH => consensus::committee_selection_epoch(sdk),
        SIG_COMMIT_EPOCH_COMMITTEE => consensus::commit_epoch_committee(sdk, params),
        SIG_GET_DKG_QUAL => consensus::get_dkg_qual(sdk, params),
        SIG_RESOLVE_SIGNER => consensus::resolve_signer(sdk, params),
        SIG_GET_EPOCH_COMMITTEE => consensus::get_epoch_committee(sdk, params),
        SIG_GET_EPOCH_COMMITTEE_LENGTH => consensus::get_epoch_committee_length(sdk, params),
        SIG_GET_EPOCH_COMMITTEE_WITH_STAKES => {
            consensus::get_epoch_committee_with_stakes(sdk, params)
        }
        // Liveness and equivocation penalties.
        SIG_RELEASE_VALIDATOR_FROM_JAIL => liveness::release_validator_from_jail(sdk, params),
        SIG_READMIT_EXPIRED_JAILS => liveness::readmit_expired_jails(sdk, params),
        SIG_SLASH => liveness::slash(sdk, params),
        SIG_COMMIT_EQUIVOCATION_REPORT => equivocation::commit_report(sdk, params),
        SIG_COMPUTE_EQUIVOCATION_REPORT_COMMITMENT => {
            equivocation::compute_report_commitment(sdk, params)
        }
        SIG_GET_EQUIVOCATION_REPORT_COMMITMENT => equivocation::get_report_commitment(sdk, params),
        SIG_SLASH_EQUIVOCATION_NOTARIZE => equivocation::slash_notarize(sdk, params),
        SIG_SLASH_EQUIVOCATION_FINALIZE => equivocation::slash_finalize(sdk, params),
        SIG_SLASH_EQUIVOCATION_NULLIFY_FINALIZE => {
            equivocation::slash_nullify_finalize(sdk, params)
        }
        // Validator lifecycle and ownership.
        SIG_IS_VALIDATOR => handlers::is_validator(sdk, params),
        SIG_IS_VALIDATOR_ACTIVE => handlers::is_validator_active(sdk, params),
        SIG_GET_VALIDATOR_STATUS => handlers::get_validator_status(sdk, params),
        SIG_GET_VALIDATOR_BY_OWNER => handlers::get_validator_by_owner(sdk, params),
        SIG_GET_VALIDATORS => handlers::get_validators(sdk),
        SIG_ADD_VALIDATOR => handlers::add_validator(sdk, params),
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
