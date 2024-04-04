use crate::types::Gas;
use crate::{
    primitives::{
        db::Database, EVMError, ExecutionResult, ResultAndState, Spec, SpecId::LONDON, U256,
    },
    Context, FrameResult,
};
use fluentbase_core::Account;
use fluentbase_types::ExitCode;
use revm_primitives::{HaltReason, OutOfGasError, SuccessReason};

/// Mainnet end handle does not change the output.
#[inline]
pub fn end<EXT, DB: Database>(
    _context: &mut Context<EXT, DB>,
    evm_output: Result<ResultAndState, EVMError<DB::Error>>,
) -> Result<ResultAndState, EVMError<DB::Error>> {
    evm_output
}

/// Reward beneficiary with gas fee.
#[inline]
pub fn reward_beneficiary<SPEC: Spec, EXT, DB: Database>(
    context: &mut Context<EXT, DB>,
    gas: &Gas,
) -> Result<(), EVMError<DB::Error>> {
    let beneficiary = context.evm.env.block.coinbase;
    let effective_gas_price = context.evm.env.effective_gas_price();

    // transfer fee to coinbase/beneficiary.
    // EIP-1559 discard basefee for coinbase transfer. Basefee amount of gas is discarded.
    let coinbase_gas_price = if SPEC::enabled(LONDON) {
        effective_gas_price.saturating_sub(context.evm.env.block.basefee)
    } else {
        effective_gas_price
    };

    let mut coinbase_account = Account::new_from_jzkt(&beneficiary);

    coinbase_account.add_balance_saturating(
        coinbase_gas_price * U256::from(gas.spend() - gas.refunded() as u64),
    );
    coinbase_account.write_to_jzkt();

    Ok(())
}

#[inline]
pub fn reimburse_caller<SPEC: Spec, EXT, DB: Database>(
    context: &mut Context<EXT, DB>,
    gas: &Gas,
) -> Result<(), EVMError<DB::Error>> {
    let caller = context.evm.env.tx.caller;
    let effective_gas_price = context.evm.env.effective_gas_price();

    // return balance of not spend gas.
    let mut caller_account = Account::new_from_jzkt(&caller);
    caller_account.add_balance_saturating(
        effective_gas_price * U256::from(gas.remaining() + gas.refunded() as u64),
    );
    caller_account.write_to_jzkt();

    Ok(())
}

/// Main return handle, returns the output of the transaction.
#[inline]
pub fn output<EXT, DB: Database>(
    context: &mut Context<EXT, DB>,
    result: FrameResult,
) -> Result<ResultAndState, EVMError<DB::Error>> {
    core::mem::replace(&mut context.evm.error, Ok(()))?;
    // used gas with refund calculated.
    let gas_refunded = result.gas().refunded() as u64;
    let final_gas_used = result.gas().spend() - gas_refunded;
    let output = result.output();
    let instruction_result = result.into_interpreter_result();

    let result = match instruction_result.result.into() {
        ExitCode::Ok => ExecutionResult::Success {
            reason: SuccessReason::Return,
            gas_used: final_gas_used,
            gas_refunded,
            logs: vec![],
            output,
        },
        ExitCode::Panic => ExecutionResult::Revert {
            gas_used: final_gas_used,
            output: output.into_data(),
        },
        _ => ExecutionResult::Halt {
            reason: HaltReason::OutOfGas(OutOfGasError::InvalidOperand),
            gas_used: final_gas_used,
        },
    };

    Ok(ResultAndState {
        result,
        state: Default::default(),
    })
}
