use crate::types::{CallInputs, CallOutcome, CreateOutcome, Gas, InterpreterResult};
use crate::{
    db::Database,
    primitives::{EVMError, Env, Spec},
    CallFrame, Context, CreateFrame, FrameOrResult, FrameResult,
};
use fluentbase_types::{ExitCode, IJournaledTrie};
use std::boxed::Box;

/// Helper function called inside [`last_frame_return`]
#[inline]
pub fn frame_return_with_refund_flag<SPEC: Spec>(
    env: &Env,
    frame_result: &mut FrameResult,
    refund_enabled: bool,
) {
    let instruction_result = frame_result.interpreter_result().result;
    let gas = frame_result.gas_mut();
    let remaining = gas.remaining();
    let refunded = gas.refunded();

    // Spend the gas limit. Gas is reimbursed when the tx returns successfully.
    *gas = Gas::new(env.tx.gas_limit);
    gas.record_cost(env.tx.gas_limit);

    match instruction_result {
        ExitCode::Ok => {
            gas.erase_cost(remaining);
            gas.record_refund(refunded);
        }
        ExitCode::Panic => {
            gas.erase_cost(remaining);
        }
        _ => {}
    }

    // Calculate gas refund for transaction.
    // If config is set to disable gas refund, it will return 0.
    // If spec is set to london, it will decrease the maximum refund amount to 5th part of
    // gas spend. (Before london it was 2th part of gas spend)
    if refund_enabled {
        // EIP-3529: Reduction in refunds
        gas.set_final_refund::<SPEC>();
    }
}

/// Handle output of the transaction
#[inline]
pub fn last_frame_return<SPEC: Spec, EXT, DB: IJournaledTrie>(
    context: &mut Context<EXT, DB>,
    frame_result: &mut FrameResult,
) -> Result<(), EVMError<ExitCode>> {
    frame_return_with_refund_flag::<SPEC>(&context.evm.env, frame_result, true);
    Ok(())
}

/// Handle frame sub call.
#[inline]
pub fn call<SPEC: Spec, EXT, DB: IJournaledTrie>(
    context: &mut Context<EXT, DB>,
    inputs: Box<CallInputs>,
) -> Result<FrameOrResult, EVMError<ExitCode>> {
    context.evm.make_call_frame(&inputs)
}

#[inline]
pub fn call_return<EXT, DB: IJournaledTrie>(
    context: &mut Context<EXT, DB>,
    frame: Box<CallFrame>,
    interpreter_result: InterpreterResult,
) -> Result<CallOutcome, EVMError<ExitCode>> {
    context
        .evm
        .call_return(&interpreter_result, frame.frame_data.checkpoint);
    Ok(CallOutcome::new(
        interpreter_result,
        frame.return_memory_range,
    ))
}

#[inline]
pub fn create_return<SPEC: Spec, EXT, DB: IJournaledTrie>(
    context: &mut Context<EXT, DB>,
    frame: Box<CreateFrame>,
    mut interpreter_result: InterpreterResult,
) -> Result<CreateOutcome, EVMError<ExitCode>> {
    context.evm.create_return::<SPEC>(
        &mut interpreter_result,
        frame.created_address,
        frame.frame_data.checkpoint,
    );
    Ok(CreateOutcome::new(
        interpreter_result,
        Some(frame.created_address),
    ))
}

#[cfg(test)]
mod tests {
    use revm_primitives::{Bytes, CancunSpec};

    use super::*;

    /// Creates frame result.
    fn call_last_frame_return(instruction_result: ExitCode, gas: Gas) -> Gas {
        let mut env = Env::default();
        env.tx.gas_limit = 100;

        let mut first_frame = FrameResult::Call(CallOutcome::new(
            InterpreterResult {
                result: instruction_result,
                output: Bytes::new(),
                gas,
            },
            0..0,
        ));
        frame_return_with_refund_flag::<CancunSpec>(&env, &mut first_frame, true);
        *first_frame.gas()
    }

    #[test]
    fn test_consume_gas() {
        let gas = call_last_frame_return(ExitCode::Panic, Gas::new(90));
        assert_eq!(gas.remaining(), 90);
        assert_eq!(gas.spend(), 10);
        assert_eq!(gas.refunded(), 0);
    }

    // TODO
    #[test]
    fn test_consume_gas_with_refund() {
        let mut return_gas = Gas::new(90);
        return_gas.record_refund(30);

        let gas = call_last_frame_return(ExitCode::Ok, return_gas);
        assert_eq!(gas.remaining(), 90);
        assert_eq!(gas.spend(), 10);
        assert_eq!(gas.refunded(), 2);

        let gas = call_last_frame_return(ExitCode::Panic, return_gas);
        assert_eq!(gas.remaining(), 90);
        assert_eq!(gas.spend(), 10);
        assert_eq!(gas.refunded(), 0);
    }

    #[test]
    fn test_revert_gas() {
        let gas = call_last_frame_return(ExitCode::Panic, Gas::new(90));
        assert_eq!(gas.remaining(), 90);
        assert_eq!(gas.spend(), 10);
        assert_eq!(gas.refunded(), 0);
    }
}
