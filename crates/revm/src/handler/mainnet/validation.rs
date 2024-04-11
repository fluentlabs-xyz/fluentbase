use crate::{
    primitives::{EVMError, Env, InvalidTransaction, Spec},
    Context,
};
use core::cmp::Ordering;
use fluentbase_core::Account;
use fluentbase_sdk::evm::{Address, U256};
use fluentbase_types::{ExitCode, IJournaledTrie, POSEIDON_EMPTY};
use revm_primitives::{SpecId, BERLIN, HOMESTEAD, ISTANBUL, SHANGHAI};
use std::boxed::Box;
use std::vec::Vec;

pub(crate) const TRANSACTION_ZERO_DATA: u64 = 4;

// berlin eip2929 constants
pub(crate) const ACCESS_LIST_ADDRESS: u64 = 2400;
pub(crate) const ACCESS_LIST_STORAGE_KEY: u64 = 1900;

/// EIP-3860 : Limit and meter initcode
pub(crate) const INITCODE_WORD_COST: u64 = 2;

/// Validate environment for the mainnet.
pub fn validate_env<SPEC: Spec, DB: IJournaledTrie>(env: &Env) -> Result<(), EVMError<ExitCode>> {
    // Important: validate block before tx.
    env.validate_block_env::<SPEC>()?;
    env.validate_tx::<SPEC>()?;
    Ok(())
}

/// Validates transaction against the state.
pub fn validate_tx_against_state<SPEC: Spec, EXT, DB: IJournaledTrie>(
    context: &mut Context<EXT, DB>,
) -> Result<(), EVMError<ExitCode>> {
    // load acc
    let tx_caller = context.evm.env.tx.caller;
    let mut caller_account = Account::new_from_jzkt(&tx_caller);

    let env = &context.evm.inner.env;

    // EIP-3607: Reject transactions from senders with deployed code
    // This EIP is introduced after london but there was no collision in the past,
    // so we can leave it enabled always
    if !env.cfg.is_eip3607_disabled() && caller_account.rwasm_code_hash != POSEIDON_EMPTY {
        return Err(InvalidTransaction::RejectCallerWithCode.into());
    }

    // Check that the transaction's nonce is correct
    if let Some(tx) = env.tx.nonce {
        let state = caller_account.nonce;
        match tx.cmp(&state) {
            Ordering::Greater => {
                return Err(InvalidTransaction::NonceTooHigh { tx, state }.into());
            }
            Ordering::Less => {
                return Err(InvalidTransaction::NonceTooLow { tx, state }.into());
            }
            _ => {}
        }
    }

    let mut balance_check = U256::from(env.tx.gas_limit)
        .checked_mul(env.tx.gas_price)
        .and_then(|gas_cost| gas_cost.checked_add(env.tx.value))
        .ok_or(InvalidTransaction::OverflowPaymentInTransaction)?;

    if SPEC::enabled(SpecId::CANCUN) {
        // if the tx is not a blob tx, this will be None, so we add zero
        let data_fee = env.calc_max_data_fee().unwrap_or_default();
        balance_check = balance_check
            .checked_add(U256::from(data_fee))
            .ok_or(InvalidTransaction::OverflowPaymentInTransaction)?;
    }

    // Check if account has enough balance for gas_limit*gas_price and value transfer.
    // Transfer will be done inside `*_inner` functions.
    if balance_check > caller_account.balance {
        if !(env.cfg.is_balance_check_disabled()) {
            return Err(InvalidTransaction::LackOfFundForMaxFee {
                fee: Box::new(balance_check),
                balance: Box::new(caller_account.balance),
            }
            .into());
        }
        // Add transaction cost to balance to ensure execution doesn't fail.
        caller_account.balance = balance_check;
    }

    caller_account.write_to_jzkt();

    Ok(())
}

/// Validate initial transaction gas.
pub fn validate_initial_tx_gas<SPEC: Spec, DB: IJournaledTrie>(
    env: &Env,
) -> Result<u64, EVMError<ExitCode>> {
    let input = &env.tx.data;
    let is_create = env.tx.transact_to.is_create();
    let access_list = &env.tx.access_list;

    let initial_gas_spend = validate_initial_tx_gas_inner::<SPEC>(input, is_create, access_list);

    // Additional check to see if limit is big enough to cover initial gas.
    if initial_gas_spend > env.tx.gas_limit {
        return Err(InvalidTransaction::CallGasCostMoreThanGasLimit.into());
    }
    Ok(initial_gas_spend)
}

pub fn validate_initial_tx_gas_inner<SPEC: Spec>(
    input: &[u8],
    is_create: bool,
    access_list: &[(Address, Vec<U256>)],
) -> u64 {
    let mut initial_gas = 0;
    let zero_data_len = input.iter().filter(|v| **v == 0).count() as u64;
    let non_zero_data_len = input.len() as u64 - zero_data_len;

    // initdate stipend
    initial_gas += zero_data_len * TRANSACTION_ZERO_DATA;
    // EIP-2028: Transaction data gas cost reduction
    initial_gas += non_zero_data_len * if SPEC::enabled(ISTANBUL) { 16 } else { 68 };

    // get number of access list account and storages.
    if SPEC::enabled(BERLIN) {
        let accessed_slots = access_list
            .iter()
            .fold(0, |slot_count, (_, slots)| slot_count + slots.len() as u64);
        initial_gas += access_list.len() as u64 * ACCESS_LIST_ADDRESS;
        initial_gas += accessed_slots * ACCESS_LIST_STORAGE_KEY;
    }

    // base stipend
    initial_gas += if is_create {
        if SPEC::enabled(HOMESTEAD) {
            // EIP-2: Homestead Hard-fork Changes
            53000
        } else {
            21000
        }
    } else {
        21000
    };

    // EIP-3860: Limit and meter initcode stipend for bytecode analysis
    if SPEC::enabled(SHANGHAI) && is_create {
        initial_gas += initcode_cost(input.len() as u64)
    }

    initial_gas
}

/// EIP-3860: Limit and meter initcode
///
/// Apply extra gas cost of 2 for every 32-byte chunk of initcode.
///
/// This cannot overflow as the initcode length is assumed to be checked.
#[inline]
fn initcode_cost(len: u64) -> u64 {
    let wordd = len / 32;
    let wordr = len % 32;
    INITCODE_WORD_COST * if wordr == 0 { wordd } else { wordd + 1 }
}
