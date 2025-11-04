use crate::{
    api::RwasmFrame,
    inspector::inspect_syscall,
    instruction_result_from_exit_code,
    syscall::execute_rwasm_interruption,
    types::{SystemInterruptionInputs, SystemInterruptionOutcome},
    ExecutionResult, NextAction,
};
use fluentbase_runtime::{
    default_runtime_executor,
    syscall_handler::{syscall_exec_impl, syscall_resume_impl},
    RuntimeContext,
};
use fluentbase_sdk::{
    is_delegated_runtime_address, keccak256, rwasm_core::RwasmModule, BlockContextV1,
    BytecodeOrHash, Bytes, BytesOrRef, ContractContextV1, ExitCode, SharedContextInput,
    SharedContextInputV1, SyscallInvocationParams, TxContextV1, FUEL_DENOM_RATE, STATE_DEPLOY,
    STATE_MAIN, U256,
};
use revm::{
    bytecode::{opcode, Bytecode},
    context::{Block, Cfg, ContextError, ContextTr, JournalTr, Transaction},
    handler::FrameData,
    interpreter::{
        interpreter::ExtBytecode,
        interpreter_types::{InputsTr, RuntimeFlag},
        return_ok, return_revert, CallInput, FrameInput, InstructionResult,
    },
    Database, Inspector,
};

#[tracing::instrument(level = "info", skip_all)]
pub(crate) fn run_rwasm_loop<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    mut inspector: Option<&mut INSP>,
) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
    let next_action = loop {
        let next_action = if let Some(interruption_outcome) = frame.take_interrupted_outcome() {
            execute_rwasm_resume(frame, ctx, interruption_outcome, inspector.as_mut())
        } else {
            execute_rwasm_frame(frame, ctx, inspector.as_mut())
        }?;
        match next_action {
            NextAction::InterruptionResult => continue,
            _ => break next_action,
        }
    };
    let interpreter_result = match &next_action {
        NextAction::NewFrame(_) => {
            return Ok(next_action);
        }
        NextAction::Return(result) => result,
        NextAction::InterruptionResult => unreachable!(),
    };
    let create_frame = match &frame.data {
        FrameData::Call(_) => {
            return Ok(next_action);
        }
        FrameData::Create(frame) => frame,
    };

    // Make sure the error is checked before processing,
    // it's very important or EVM since it can return a bytecode starting from 0xEF
    if !interpreter_result.result.is_ok() {
        return Ok(next_action);
    }

    // Check the resulting bytecode does it match to rWasm signature (bytecode override)
    let overwrite_delegated_bytecode_with_rwasm =
        if interpreter_result.output.first() == Some(&0xEF) {
            let account = ctx
                .journal_mut()
                .load_account_code(create_frame.created_address)?;
            match account.data.info.code.as_ref() {
                Some(Bytecode::OwnableAccount(ownable_account_bytecode)) => {
                    is_delegated_runtime_address(&ownable_account_bytecode.owner_address)
                }
                Some(_) | None => false,
            }
        } else {
            false
        };
    if overwrite_delegated_bytecode_with_rwasm {
        // TODO(dmitry123): "optimize me, store RwasmModule inside Bytecode"
        let (_, bytes_read) = RwasmModule::new(interpreter_result.output.as_ref());
        let (rwasm_module_raw, constructor_params_raw) = (
            interpreter_result.output.slice(..bytes_read),
            interpreter_result.output.slice(bytes_read..),
        );
        let bytecode_hash = keccak256(rwasm_module_raw.as_ref());
        // Rewrite overridden rWasm bytecode
        let bytecode = Bytecode::new_rwasm(rwasm_module_raw);
        ctx.journal_mut()
            .set_code(create_frame.created_address, bytecode.clone());
        // Change input params
        frame.interpreter.input.input = CallInput::Bytes(constructor_params_raw);
        frame.interpreter.input.account_owner = None;
        frame.interpreter.bytecode = ExtBytecode::new_with_hash(bytecode, bytecode_hash);
        frame.interpreter.gas = interpreter_result.gas;
        // Re-run deploy function using rWasm
        return run_rwasm_loop(frame, ctx, inspector);
    }
    Ok(next_action)
}

#[tracing::instrument(level = "info", skip_all)]
fn execute_rwasm_frame<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    inspector: Option<&mut INSP>,
) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
    let interpreter = &mut frame.interpreter;
    let is_create: bool = matches!(frame.input, FrameInput::Create(..));
    let is_static: bool = interpreter.runtime_flag.is_static();
    let bytecode_address = interpreter
        .input
        .bytecode_address()
        .cloned()
        .unwrap_or_else(|| interpreter.input.target_address());
    let effective_bytecode_address = interpreter
        .input
        .account_owner
        .unwrap_or_else(|| bytecode_address);

    // encode input with all related context info
    let context_input = SharedContextInput::V1(SharedContextInputV1 {
        block: BlockContextV1 {
            chain_id: ctx.cfg().chain_id(),
            coinbase: ctx.block().beneficiary(),
            timestamp: ctx.block().timestamp().as_limbs()[0],
            number: ctx.block().number().as_limbs()[0],
            difficulty: ctx.block().difficulty(),
            prev_randao: ctx.block().prevrandao().unwrap(),
            gas_limit: ctx.block().gas_limit(),
            base_fee: U256::from(ctx.block().basefee()),
        },
        tx: TxContextV1 {
            gas_limit: ctx.tx().gas_limit(),
            nonce: ctx.tx().nonce(),
            gas_price: U256::from(ctx.tx().gas_price()),
            gas_priority_fee: ctx.tx().max_priority_fee_per_gas().map(|v| U256::from(v)),
            origin: ctx.tx().caller(),
            value: ctx.tx().value(),
        },
        contract: ContractContextV1 {
            address: interpreter.input.target_address(),
            bytecode_address,
            caller: interpreter.input.caller_address,
            is_static: interpreter.runtime_flag.is_static(),
            value: interpreter.input.call_value,
            gas_limit: interpreter.gas.remaining(),
        },
    });
    let mut context_input = context_input
        .encode()
        .expect("revm: unable to encode shared context input")
        .to_vec();
    let inputs_bytes = interpreter.input.input.bytes(ctx);
    context_input.extend_from_slice(&inputs_bytes);

    let rwasm_code_hash = interpreter.bytecode.hash().unwrap();

    let rwasm_bytecode = match &*interpreter.bytecode {
        Bytecode::Rwasm(bytecode) => bytecode.clone(),
        _ => {
            #[cfg(feature = "std")]
            eprintln!(
                "WARNING: unexpected bytecode type: {:?}, need investigation, this should never happen",
                interpreter.bytecode
            );
            return Ok(NextAction::error(
                ExitCode::NotSupportedBytecode,
                interpreter.gas,
            ));
        }
    };
    let bytecode_hash = BytecodeOrHash::Bytecode {
        address: effective_bytecode_address,
        bytecode: rwasm_bytecode.module,
        hash: rwasm_code_hash,
    };

    // Fuel limit we denominate later to gas
    let fuel_limit = interpreter
        .gas
        .remaining()
        .checked_mul(FUEL_DENOM_RATE)
        .unwrap_or(u64::MAX);

    // Execute function
    let mut runtime_context = RuntimeContext::default();
    let (fuel_consumed, fuel_refunded, exit_code) = syscall_exec_impl(
        &mut runtime_context,
        bytecode_hash,
        BytesOrRef::Bytes(context_input.into()),
        fuel_limit,
        if is_create { STATE_DEPLOY } else { STATE_MAIN },
    );

    // make sure we have enough gas to charge from the call
    // assert_eq!(
    //     (fuel_consumed + FUEL_DENOM_RATE - 1) / FUEL_DENOM_RATE,
    //     fuel_consumed / FUEL_DENOM_RATE
    // );
    if !interpreter.gas.record_denominated_cost(fuel_consumed) {
        return Ok(NextAction::error(ExitCode::OutOfFuel, interpreter.gas));
    }
    interpreter.gas.record_denominated_refund(fuel_refunded);

    // extract return data from the execution context
    let return_data: Bytes;
    return_data = runtime_context.execution_result.return_data.into();

    process_exec_result(
        frame,
        ctx,
        inspector,
        exit_code,
        return_data,
        is_create,
        is_static,
    )
}

#[tracing::instrument(level = "info", skip_all)]
fn execute_rwasm_resume<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    interruption_outcome: SystemInterruptionOutcome,
    inspector: Option<&mut INSP>,
) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
    let SystemInterruptionOutcome { inputs, result, .. } = interruption_outcome;
    let result = result.unwrap();
    let call_id = inputs.call_id;

    let fuel_consumed = result
        .gas
        .spent()
        .checked_mul(FUEL_DENOM_RATE)
        .unwrap_or(u64::MAX);
    let fuel_refunded = result
        .gas
        .refunded()
        .checked_mul(FUEL_DENOM_RATE as i64)
        .unwrap_or(i64::MAX);

    // we can safely convert the result into i32 here,
    // and we shouldn't worry about negative numbers
    // since the constraints is applied only for resulting exit codes
    let exit_code: ExitCode = match result.result {
        return_ok!() => ExitCode::Ok,
        return_revert!() => ExitCode::Panic,
        // a special case for frame execution where we always return `Err` as a failed call/create
        // _ if is_frame => ExitCode::Err,
        // out of gas error codes
        InstructionResult::OutOfGas
        | InstructionResult::OutOfFuel
        | InstructionResult::MemoryOOG
        | InstructionResult::MemoryLimitOOG
        | InstructionResult::PrecompileOOG
        | InstructionResult::InvalidOperandOOG
        | InstructionResult::ReentrancySentryOOG => ExitCode::OutOfFuel,
        // don't map other error codes
        _ => ExitCode::UnknownError,
    };

    let mut runtime_context = RuntimeContext::default();
    let (fuel_consumed, fuel_refunded, exit_code) = syscall_resume_impl(
        &mut runtime_context,
        inputs.call_id,
        result.output.as_ref(),
        exit_code.into_i32(),
        fuel_consumed,
        fuel_refunded,
        inputs.syscall_params.fuel16_ptr,
    );
    let return_data: Bytes = runtime_context.execution_result.return_data.into();

    // make sure we have enough gas to charge from the call
    if !frame.interpreter.gas.record_denominated_cost(fuel_consumed) {
        return Ok(NextAction::error(
            ExitCode::OutOfFuel,
            frame.interpreter.gas,
        ));
    }
    // accumulate refunds (can be forwarded from an interrupted call)
    frame
        .interpreter
        .gas
        .record_denominated_refund(fuel_refunded);

    let result = process_exec_result::<CTX, INSP>(
        frame,
        ctx,
        inspector,
        exit_code,
        return_data,
        inputs.is_create,
        inputs.is_static,
    )?;
    // If interruption ends with return,
    // then we should forget saved runtime, because otherwise it can cause memory leak
    match &result {
        NextAction::Return(_) => {
            use fluentbase_runtime::RuntimeExecutor;
            default_runtime_executor().forget_runtime(call_id);
        }
        _ => {}
    }
    Ok(result)
}

#[tracing::instrument(level = "info", skip_all)]
fn process_exec_result<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    inspector: Option<&mut INSP>,
    exit_code: i32,
    return_data: Bytes,
    is_create: bool,
    is_static: bool,
) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
    // if we have success or failed exit code
    if exit_code <= 0 {
        return Ok(process_halt(frame, ctx, inspector, exit_code, return_data));
    }

    // otherwise, exit code is a "call_id" that identifies saved context
    let call_id = exit_code as u32;

    // try to parse execution params, if it's not possible, then return an error
    let Some(syscall_params) = SyscallInvocationParams::decode(&return_data) else {
        unreachable!("can't decode invocation params");
    };

    let inputs = SystemInterruptionInputs {
        call_id,
        is_create,
        syscall_params,
        gas: frame.interpreter.gas,
        is_static,
    };

    execute_rwasm_interruption::<CTX, INSP>(frame, inspector, ctx, inputs)
}

#[tracing::instrument(level = "info", skip_all)]
fn process_halt<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    inspector: Option<&mut INSP>,
    exit_code: i32,
    return_data: Bytes,
) -> NextAction {
    let exit_code = ExitCode::from(exit_code);
    let result = instruction_result_from_exit_code(exit_code, return_data.is_empty());
    if let Some(inspector) = inspector {
        let evm_opcode = match result {
            InstructionResult::Stop => Some(opcode::STOP),
            InstructionResult::Return => Some(opcode::RETURN),
            return_revert!() => Some(opcode::REVERT),
            _ => {
                // emh, we can't return anything here, because EVM trace doesn't handle traps...
                None
            }
        };
        if let Some(evm_opcode) = evm_opcode {
            inspect_syscall(frame, ctx, inspector, evm_opcode, []);
        }
    }
    NextAction::Return(ExecutionResult {
        result,
        output: return_data,
        gas: frame.interpreter.gas,
    })
}
