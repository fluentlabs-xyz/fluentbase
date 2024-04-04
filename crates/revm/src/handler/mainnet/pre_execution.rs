//! Handles related to the main function of the EVM.
//!
//! They handle initial setup of the EVM, call loop and the final return of the EVM

use crate::{
    primitives::{
        db::Database,
        EVMError, Env, Spec,
        SpecId::{CANCUN, SHANGHAI},
        TransactTo, U256,
    },
    Context,
};
use fluentbase_core::Account;

/// Main load handle
#[inline]
pub fn load_accounts<SPEC: Spec, EXT, DB: Database>(
    context: &mut Context<EXT, DB>,
) -> Result<(), EVMError<DB::Error>> {
    // set journaling state flag.
    context.evm.inner.spec_id = SPEC::SPEC_ID;

    // load coinbase
    // EIP-3651: Warm COINBASE. Starts the `COINBASE` address warm
    if SPEC::enabled(SHANGHAI) {
        Account::new_from_jzkt(&context.evm.inner.env.block.coinbase);
    }

    context.evm.load_access_list()?;
    Ok(())
}

/// Helper function that deducts the caller balance.
#[inline]
pub fn deduct_caller_inner<SPEC: Spec>(caller_account: &mut Account, env: &Env) {
    // Subtract gas costs from the caller's account.
    // We need to saturate the gas cost to prevent underflow in case that `disable_balance_check` is enabled.
    let mut gas_cost = U256::from(env.tx.gas_limit).saturating_mul(env.effective_gas_price());

    // EIP-4844
    if SPEC::enabled(CANCUN) {
        let data_fee = env.calc_data_fee().expect("already checked");
        gas_cost = gas_cost.saturating_add(data_fee);
    }

    // set new caller account balance.
    caller_account.sub_balance_saturating(gas_cost);

    // bump the nonce for calls. Nonce for CREATE will be bumped in `handle_create`.
    if matches!(env.tx.transact_to, TransactTo::Call(_)) {
        caller_account.inc_nonce().unwrap();
    }
}

/// Deducts the caller balance to the transaction limit.
#[inline]
pub fn deduct_caller<SPEC: Spec, EXT, DB: Database>(
    context: &mut Context<EXT, DB>,
) -> Result<(), EVMError<DB::Error>> {
    // load caller's account.
    let mut caller_account = Account::new_from_jzkt(&context.evm.inner.env.tx.caller);
    // deduct gas cost from caller's account.
    deduct_caller_inner::<SPEC>(&mut caller_account, &context.evm.inner.env);
    // write account changes
    caller_account.write_to_jzkt();
    Ok(())
}
