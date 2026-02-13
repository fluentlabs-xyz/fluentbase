use crate::{
    api::RwasmFrame,
    eip2935::eip2935_compute_storage_keys,
    inspector::inspect_syscall,
    instruction_result_from_exit_code,
    syscall::execute_rwasm_interruption,
    types::{SystemInterruptionInputs, SystemInterruptionOutcome},
    ExecutionResult, NextAction,
};
use alloy_primitives::{Address, Log, LogData};
use fluentbase_runtime::{
    default_runtime_executor,
    syscall_handler::{syscall_exec_impl, syscall_resume_impl},
    RuntimeContext, RuntimeExecutor,
};
use fluentbase_sdk::{
    bincode, is_delegated_runtime_address, is_execute_using_system_runtime, keccak256,
    rwasm_core::RwasmModule,
    system::{
        JournalLog, RuntimeExecutionOutcomeV1, RuntimeInterruptionOutcomeV1, RuntimeNewFrameInputV1,
    },
    BlockContextV1, BytecodeOrHash, Bytes, BytesOrRef, ContractContextV1, ExitCode, HashMap,
    SharedContextInput, SharedContextInputV1, SyscallInvocationParams, TxContextV1,
    FUEL_DENOM_RATE, PRECOMPILE_EIP2935, PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME, STATE_DEPLOY,
    STATE_MAIN, U256,
};
use fluentbase_universal_token::storage::erc20_compute_storage_keys;
use revm::{
    bytecode::{opcode, ownable_account::OwnableAccountBytecode, rwasm::RwasmBytecode, Bytecode},
    context::{Block, Cfg, ContextError, ContextTr, JournalTr, Transaction},
    context_interface::journaled_state::JournalLoadError,
    handler::FrameData,
    interpreter::{
        interpreter::ExtBytecode,
        interpreter_types::{InputsTr, ReturnData, RuntimeFlag, StackTr},
        return_ok, return_revert, CallInput, FrameInput, Gas, InstructionResult,
    },
    Database, Inspector,
};
use std::vec::Vec;

fn should_overwrite_delegated_bytecode<'a, CTX: ContextTr>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    next_action: &'a NextAction,
) -> Result<Option<&'a ExecutionResult>, ContextError<<CTX::Db as Database>::Error>> {
    // If we’re about to create a new EVM frame (call/create), return to the handler immediately.
    let interpreter_result = match next_action {
        NextAction::NewFrame(_) => return Ok(None),
        NextAction::Return(result) => result,
        NextAction::InterruptionResult => unreachable!(),
    };

    // The bytecode overwrite flow only makes sense on CREATE frames.
    let create_frame = match &frame.data {
        FrameData::Call(_) => return Ok(None),
        FrameData::Create(frame) => frame,
    };

    // Make sure errors are handled before any post-processing.
    //
    // This is especially important for the EVM, because a failing deployment can still "return"
    // arbitrary bytes, and we must not treat them as executable bytecode.
    if !interpreter_result.result.is_ok() {
        return Ok(None);
    }

    // If the deployed "bytecode" starts with 0xEF, treat it as an rWasm signature
    // (bytecode override path). We only allow this when the deployed contract is an
    // OwnableAccount whose owner address is a delegated runtime address.
    let overwrite_delegated_bytecode_with_rwasm =
        if interpreter_result.output.first() == Some(&0xEF) {
            let account = ctx
                .journal_mut()
                .load_account_with_code(create_frame.created_address)?;
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
        Ok(Some(interpreter_result))
    } else {
        Ok(None)
    }
}

/// Execute a single `RwasmFrame` until it:
/// - returns to its caller,
/// - requests a new EVM frame (e.g. an internal CALL/CREATE), or
/// - yields an interruption that must be serviced by the host (syscall trampoline).
///
/// This is the "driver loop" for rWasm execution inside the REVM handler.
///
/// ## Important behaviors
/// - Interruption handling is *re-entrant*: we may resume execution multiple times until the
///   frame settles into a final `Return`/`NewFrame`.
/// - Create frames have a special post-processing path: if the deployed bytecode indicates
///   an ownable/delegated runtime wrapper, we may overwrite the deployed code with a native
///   rWasm module and re-run constructor logic under rWasm.
pub(crate) fn run_rwasm_loop<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    mut inspector: Option<&mut INSP>,
) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
    // Keep executing until the frame produces a stable next action.
    // `NextAction::InterruptionResult` is an internal "continue the loop" signal.
    let next_action = loop {
        let next_action = if let Some(interruption_outcome) = frame.take_interrupted_outcome() {
            // We previously yielded a syscall interruption; resume from the saved host outcome.
            execute_rwasm_resume(frame, ctx, interruption_outcome, inspector.as_mut())
        } else {
            // Normal execution path (no outstanding interruption).
            execute_rwasm_frame(frame, ctx, inspector.as_mut())
        }?;
        match next_action {
            NextAction::InterruptionResult => continue,
            _ => break next_action,
        }
    };

    // Some system contracts (e.g., Wasm) can replace the deployed bytecode after deployment.
    // We use this to translate user-supplied Wasm into rWasm, so the contract runs under the rWasm runtime.
    if let Some(interpreter_result) =
        should_overwrite_delegated_bytecode::<CTX>(frame, ctx, &next_action)?
    {
        // Split the deployment output into:
        // - the rWasm module (prefix)
        // - constructor parameters (suffix)
        // Check `contracts/wasm/lib.rs` for more
        let (rwasm_module, bytes_read) = RwasmModule::new(interpreter_result.output.as_ref());
        let (rwasm_module_raw, constructor_params_raw) = (
            interpreter_result.output.slice(..bytes_read),
            interpreter_result.output.slice(bytes_read..),
        );

        // Note: it never panics, because we check the created address
        //  inside the `should_overwrite_delegated_bytecode` function.
        let created_address = frame.data.created_address().unwrap();

        // Replace the deployed bytecode with rWasm.
        let bytecode_hash = keccak256(rwasm_module_raw.as_ref());
        let bytecode = Bytecode::Rwasm(RwasmBytecode {
            module: rwasm_module,
            raw: rwasm_module_raw,
        });
        ctx.journal_mut()
            .set_code(created_address, bytecode.clone());

        // Reconfigure the interpreter so it re-runs the constructor under rWasm,
        // using the leftover bytes as input.
        frame.interpreter.input.input = CallInput::Bytes(constructor_params_raw);
        frame.interpreter.bytecode = ExtBytecode::new_with_hash(bytecode, bytecode_hash);
        frame.interpreter.gas = interpreter_result.gas;
        // Clear any execution leftovers: we are effectively restarting the frame.
        frame.interpreter.input.bytecode_address = None;
        frame.interpreter.stack.clear();
        frame.interpreter.return_data.clear();
        frame.interpreter.memory.free_child_context();

        // Re-run the `deploy` function using rWasm with new bytecode (for constructor execution).
        // Recursion is checked inside `should_overwrite_delegated_bytecode`, where we verify
        // that delegated ownership is presented (we remove ownership by replacing the bytecode).
        return run_rwasm_loop(frame, ctx, inspector);
    }

    Ok(next_action)
}

/// Execute the current rWasm frame until it halts or yields an interruption/new-frame.
///
/// This function:
/// - Builds the runtime "shared context" payload (block/tx/contract).
/// - Optionally packs extra state (storage/balances) for system runtime contracts.
/// - Executes rWasm via `syscall_exec_impl`.
/// - Denominates fuel to EVM gas and charges gas/refund.
/// - Routes the exit code into either a normal halt or a syscall interruption trampoline.
fn execute_rwasm_frame<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    inspector: Option<&mut INSP>,
) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
    let interpreter = &mut frame.interpreter;
    let is_create: bool = matches!(frame.input, FrameInput::Create(..));

    // `bytecode_address` is the address from which we fetched code; if not set, fall back to
    // the target address (typical CALL semantics).
    let bytecode_address = interpreter
        .input
        .bytecode_address()
        .cloned()
        .unwrap_or_else(|| interpreter.input.target_address());

    // Special case: OwnableAccount bytecode supports delegation to another runtime.
    //
    // Important: this must run after EIP-7702 resolution; otherwise delegation from EVM accounts
    // in Fluent mode will not work correctly.
    if let Bytecode::OwnableAccount(ownable_bytecode) = &*interpreter.bytecode {
        let delegated_address = ownable_bytecode.owner_address;

        // Expose the delegated owner to downstream components (e.g. inspectors/tracing).
        interpreter.input.account_owner = Some(delegated_address);

        // Replace the executing bytecode with the delegated account's code.
        let account = &ctx
            .journal_mut()
            .load_account_with_code(delegated_address)?
            .info;
        let bytecode = account.code.clone().unwrap_or_default();
        interpreter.bytecode = ExtBytecode::new_with_hash(bytecode, account.code_hash);
    }

    // Load "meta bytecode" from the original bytecode address. This is used to:
    // - detect ownable wrappers (metadata + effective runtime address)
    // - decide whether to pack system-runtime context/state
    let meta_account = ctx.journal_mut().load_account_with_code(bytecode_address)?;
    let meta_bytecode = meta_account.info.code.clone().unwrap_or_default();

    // Encode the shared context (block/tx/contract) that the runtime expects.
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
    let mut contract_input = context_input
        .encode()
        .expect("revm: unable to encode shared context input")
        .to_vec();

    // Extract call data / initcode input bytes.
    let input = interpreter.input.input.bytes(ctx);

    let target_address = interpreter.input.target_address();
    let caller_address = interpreter.input.caller_address();

    // If we're executing an ownable wrapper, system runtime selection is based on the owner,
    // not on the wrapper address itself.
    let (effective_bytecode_metadata, effective_bytecode_address) = match meta_bytecode {
        Bytecode::OwnableAccount(v) => (Some(v.metadata), v.owner_address),
        _ => (None, bytecode_address),
    };

    // Disallow delegated runtime addresses as call targets or bytecode sources.
    //
    // Rationale: allowing direct interaction could enable non-standard state access patterns.
    // If we ever relax this, we should do it with explicit invariants and a security review.
    if is_delegated_runtime_address(&target_address)
        || is_delegated_runtime_address(&bytecode_address)
    {
        return Ok(NextAction::error(
            ExitCode::NotSupportedBytecode,
            interpreter.gas,
        ));
    }

    // System runtime contracts can request preloaded state.
    // For certain known precompiles we compute a deterministic set of storage keys.
    if is_execute_using_system_runtime(&effective_bytecode_address) {
        let block_number = ctx.block().number().as_limbs()[0];

        // Collect EVM access list information (addresses, storage slots).
        // Note: the runtime does not yet fully support EVM access lists; we only use the
        // address list for balance prefetch and use custom per-precompile storage keys below.
        let storage_list: Vec<U256> = match effective_bytecode_address {
            // Override storage keys for known system runtimes, based on calldata and context.
            PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME => {
                erc20_compute_storage_keys(input.as_ref(), &caller_address, is_create)
                    .unwrap_or_default()
            }
            PRECOMPILE_EIP2935 => {
                eip2935_compute_storage_keys(input.as_ref(), &caller_address, block_number)
                    .unwrap_or_default()
            }
            _ => {
                // TODO(dmitry123): Add support for pre-loading for some storage slots to EVM runtime
                /*let mut storage_list: Vec<U256> = vec![];
                if let Some(access_list) = ctx.tx().access_list() {
                    for x in access_list {
                        storage_list.extend(x.storage_slots().map(|v| U256::from_be_bytes(v.0)));
                    }
                };
                storage_list*/

                // Note: we do not use the EVM storage access list for runtime yet.
                vec![]
            }
        };

        // Prefetch storage values.
        let mut storage = HashMap::<U256, U256>::with_capacity(storage_list.len());
        for k in storage_list {
            let skip_cold =
                interpreter.gas.remaining() < ctx.cfg().gas_params().cold_storage_cost();
            let state_load = ctx
                .journal_mut()
                .sload_skip_cold_load(target_address, k, skip_cold);
            match state_load {
                // We have enough gas to execute the cold sload
                Ok(data) => {
                    let gas_cost = if data.is_cold {
                        ctx.cfg().gas_params().cold_storage_cost()
                    } else {
                        ctx.cfg().gas_params().warm_storage_read_cost()
                    };
                    // We charge for gas in advance because we don't charge inside system contracts
                    if !interpreter.gas.record_cost(gas_cost) {
                        return Ok(NextAction::out_of_fuel(Gas::new_spent(
                            interpreter.gas.remaining(),
                        )));
                    }
                    _ = storage.insert(k, data.data)
                }
                // We need more gas to execute the cold sload
                Err(JournalLoadError::ColdLoadSkipped) => {
                    return Ok(NextAction::out_of_fuel(Gas::new_spent(
                        interpreter.gas.remaining(),
                    )))
                }
                // Return database error
                Err(JournalLoadError::DBError(err)) => return Err(ContextError::Db(err)),
            }
        }

        // Wrap everything into the system-runtime new-frame input format.
        let new_frame_input = RuntimeNewFrameInputV1 {
            metadata: effective_bytecode_metadata.unwrap_or_default(),
            input,
            context: contract_input.into(),
            storage: Some(storage),
        };
        let new_frame_input =
            bincode::encode_to_vec(&new_frame_input, bincode::config::legacy()).unwrap();
        contract_input = new_frame_input;
    } else {
        // Non-system runtime: context bytes + raw input bytes.
        contract_input.extend_from_slice(&input);
    }

    // We only expect rWasm bytecode at this point.
    let rwasm_bytecode = match &*interpreter.bytecode {
        Bytecode::Rwasm(bytecode) => bytecode.clone(),
        _ => {
            #[cfg(feature = "std")]
            eprintln!(
                "WARNING: unexpected bytecode type; this should never happen: {:?}",
                interpreter.bytecode
            );
            return Ok(NextAction::error(
                ExitCode::NotSupportedBytecode,
                interpreter.gas,
            ));
        }
    };

    // Pass bytecode by value + hash to the runtime executor.
    let bytecode_hash = BytecodeOrHash::Bytecode {
        address: effective_bytecode_address,
        bytecode: rwasm_bytecode.module,
        hash: interpreter.bytecode.hash().unwrap(),
    };

    // Fuel is denominated later into EVM gas.
    // The multiplication can overflow in pathological cases; saturate to u64::MAX.
    let fuel_limit = interpreter
        .gas
        .remaining()
        .checked_mul(FUEL_DENOM_RATE)
        .unwrap_or(u64::MAX);

    // Execute rWasm entrypoint for this frame.
    let mut runtime_context = RuntimeContext::default();
    let (fuel_consumed, fuel_refunded, exit_code) = syscall_exec_impl(
        &mut runtime_context,
        bytecode_hash,
        BytesOrRef::Bytes(contract_input.into()),
        fuel_limit,
        if is_create { STATE_DEPLOY } else { STATE_MAIN },
    );

    // Convert consumed fuel into gas to charge inside REVM.
    // On some networks we floor vs ceil; keep the behavior feature-gated.
    cfg_if::cfg_if! {
        if #[cfg(feature = "fluent-testnet")] {
            let gas_consumed = fuel_consumed / FUEL_DENOM_RATE;
        } else {
            let gas_consumed = (fuel_consumed + FUEL_DENOM_RATE - 1) / FUEL_DENOM_RATE;
        }
    }

    // Charge gas. If we cannot, halt with out-of-fuel.
    if !interpreter.gas.record_cost(gas_consumed) {
        return Ok(NextAction::error(ExitCode::OutOfFuel, interpreter.gas));
    }

    // Apply refunds. Runtime refund is in fuel; denominate to gas refund units.
    interpreter
        .gas
        .record_refund(fuel_refunded / FUEL_DENOM_RATE as i64);

    // Extract return data from the execution context.
    let return_data: Bytes = runtime_context.execution_result.return_data.into();

    process_exec_result(frame, ctx, inspector, exit_code, return_data)
}

/// Resume execution after a host-serviced interruption.
///
/// The interruption outcome contains:
/// - the saved syscall inputs (call_id, params, gas snapshot),
/// - the REVM "halted frame" state for system-runtime v2,
/// - the call result (output + REVM instruction result + gas accounting).
///
/// This function re-enters the runtime via `syscall_resume_impl`, charges gas, and then
/// routes the final result through the normal halt path.
fn execute_rwasm_resume<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    interruption_outcome: SystemInterruptionOutcome,
    inspector: Option<&mut INSP>,
) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
    let SystemInterruptionOutcome {
        inputs,
        result,
        halted_frame,
        ..
    } = interruption_outcome;

    // `result` is expected to exist if we got here; interruption plumbing guarantees it.
    let result = result.unwrap();
    let call_id = inputs.call_id;

    // Convert REVM gas accounting into runtime fuel units.
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

    // Map REVM instruction result to runtime exit codes.
    //
    // Safety notes:
    // - We can safely convert into i32 here (exit codes are constrained by design).
    // - We avoid passing through arbitrary REVM error codes except OOG-class errors.
    let exit_code: ExitCode = match result.result {
        return_ok!() => ExitCode::Ok,
        return_revert!() => ExitCode::Panic,

        // Out-of-gas families map to a single exit code.
        InstructionResult::OutOfGas
        | InstructionResult::OutOfFuel
        | InstructionResult::MemoryOOG
        | InstructionResult::MemoryLimitOOG
        | InstructionResult::PrecompileOOG
        | InstructionResult::InvalidOperandOOG
        | InstructionResult::ReentrancySentryOOG => ExitCode::OutOfFuel,

        _ => ExitCode::UnknownError,
    };

    // If we are resuming a system runtime v2 contract, the resume payload must be wrapped
    // in `RuntimeInterruptionOutcomeV1` so the runtime can reconcile state changes/logs/etc.
    let bytecode_address = frame
        .interpreter
        .input
        .bytecode_address()
        .cloned()
        .unwrap_or_else(|| frame.interpreter.input.target_address());
    let meta_account = ctx.journal_mut().load_account_with_code(bytecode_address)?;
    let meta_bytecode = meta_account.info.code.clone().unwrap_or_default();

    let outcome: Bytes = match meta_bytecode {
        Bytecode::OwnableAccount(v) if is_execute_using_system_runtime(&v.owner_address) => {
            let outcome = RuntimeInterruptionOutcomeV1 {
                halted_frame,
                output: result.output,
                fuel_consumed,
                fuel_refunded,
                exit_code,
            };
            bincode::encode_to_vec(&outcome, bincode::config::legacy())
                .unwrap()
                .into()
        }
        _ => result.output,
    };

    // Resume inside the runtime.
    let mut runtime_context = RuntimeContext::default();
    let (fuel_consumed, fuel_refunded, exit_code) = syscall_resume_impl(
        &mut runtime_context,
        inputs.call_id,
        outcome.as_ref(),
        exit_code.into_i32(),
        fuel_consumed,
        fuel_refunded,
        inputs.syscall_params.fuel16_ptr,
    );

    let return_data: Bytes = runtime_context.execution_result.return_data.into();

    // Convert consumed fuel into gas for REVM.
    cfg_if::cfg_if! {
        if #[cfg(feature = "fluent-testnet")] {
            let gas_consumed = fuel_consumed / FUEL_DENOM_RATE;
        } else {
            let gas_consumed = (fuel_consumed + FUEL_DENOM_RATE - 1) / FUEL_DENOM_RATE;
        }
    }

    // Charge gas for the resumed segment.
    if !frame.interpreter.gas.record_cost(gas_consumed) {
        return Ok(NextAction::error(
            ExitCode::OutOfFuel,
            frame.interpreter.gas,
        ));
    }

    // Accumulate refunds (may be forwarded through interruptions).
    frame
        .interpreter
        .gas
        .record_refund(fuel_refunded / FUEL_DENOM_RATE as i64);

    let result = process_exec_result::<CTX, INSP>(frame, ctx, inspector, exit_code, return_data)?;

    // If the interruption resolves to a final return, forget saved runtime state;
    // otherwise we risk retaining contexts longer than necessary.
    if matches!(&result, NextAction::Return(_)) {
        default_runtime_executor().forget_runtime(call_id);
    }

    Ok(result)
}

/// If the currently executing bytecode address holds an `OwnableAccount` that delegates to the
/// system runtime, return its decoded bytecode so we can mutate/commit metadata updates.
///
/// Note: this reads code from journal, so callers should be careful to keep journal ordering
/// consistent with REVM semantics.
fn get_ownable_account_mut<'a, CTX: ContextTr + 'a, INSP: Inspector<CTX>>(
    frame: &'a mut RwasmFrame,
    ctx: &'a mut CTX,
) -> Result<Option<OwnableAccountBytecode>, ContextError<<CTX::Db as Database>::Error>> {
    let bytecode_address = frame
        .interpreter
        .input
        .bytecode_address()
        .cloned()
        .unwrap_or_else(|| frame.interpreter.input.target_address());

    let bytecode_account = ctx
        .journal_mut()
        .load_account_with_code(bytecode_address)?
        .data;

    let bytecode_account = bytecode_account.info.code.clone();
    Ok(bytecode_account.and_then(|bytecode| match bytecode {
        Bytecode::OwnableAccount(account)
            if is_execute_using_system_runtime(&account.owner_address) =>
        {
            Some(account)
        }
        _ => None,
    }))
}

/// Apply the decoded `RuntimeExecutionOutcomeV1` to the EVM journal:
/// - replace `return_data` with the "user output"
/// - commit storage writes (only if the runtime exit code is ok)
/// - emit logs
/// - update ownable account metadata if present
///
/// This is effectively the bridge from system runtime semantics back into REVM journal writes.
fn process_runtime_execution_outcome<CTX: ContextTr>(
    target_address: &Address,
    ctx: &mut CTX,
    return_data: &mut Bytes,
    exit_code: ExitCode,
    ownable_account_bytecode: Option<OwnableAccountBytecode>,
) -> Result<(), ContextError<<CTX::Db as Database>::Error>> {
    // If we have a fatal exit code, execution halted/panicked and output may be corrupted,
    // so we must not attempt to decode structured outcome bytes.
    if exit_code.is_fatal_exit_code() {
        return Ok(());
    }

    let (runtime_output, _): (RuntimeExecutionOutcomeV1, usize) =
        bincode::decode_from_slice(return_data, bincode::config::legacy())
            .expect("runtime execution outcome");

    // Replace the raw runtime bytes with the contract-visible output.
    *return_data = runtime_output.output.into();

    // Optimization: if the runtime reported a non-ok exit code, we intentionally skip writing
    // state changes into the journal (they would be rolled back anyway).
    if !runtime_output.exit_code.is_ok() {
        return Ok(());
    }

    for (k, v) in runtime_output.storage.unwrap_or_default() {
        ctx.journal_mut().sstore(*target_address, k, v)?;
    }

    for JournalLog { topics, data } in runtime_output.logs {
        ctx.journal_mut().log(Log {
            address: *target_address,
            data: LogData::new_unchecked(topics, data),
        });
    }

    if let Some(new_metadata) = runtime_output.new_metadata {
        // Safety: `new_metadata` should only be set by ownable accounts. If a non-ownable system
        // contract sets it, that indicates a severe invariant break.
        let mut ownable_account_bytecode = ownable_account_bytecode.unwrap();
        ownable_account_bytecode.metadata = new_metadata;
        let bytecode = Bytecode::OwnableAccount(ownable_account_bytecode);
        ctx.journal_mut().set_code(*target_address, bytecode);
    }

    Ok(())
}

/// Handle exit codes that are produced by the system runtime boundary.
///
/// For system runtime v2, the return data is a structured envelope that must be decoded and
/// committed into the journal. For non-system contracts we also ensure fatal exit codes are
/// not user-controllable.
fn process_system_runtime_result<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    inspector: Option<&mut INSP>,
    exit_code: i32,
    mut return_data: Bytes,
) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
    let target_address = frame.interpreter.input.target_address();
    let bytecode_address = frame
        .interpreter
        .input
        .bytecode_address
        .unwrap_or(target_address);

    let ownable_account = get_ownable_account_mut::<CTX, INSP>(frame, ctx)?;
    let effective_bytecode_address = ownable_account
        .as_ref()
        .map(|v| v.owner_address)
        .unwrap_or(bytecode_address);

    let mut exit_code = ExitCode::from(exit_code);
    match exit_code {
        // System runtime v2 path: decode and apply structured outcome.
        exit_code if is_execute_using_system_runtime(&effective_bytecode_address) => {
            process_runtime_execution_outcome(
                &target_address,
                ctx,
                &mut return_data,
                exit_code,
                ownable_account,
            )?;
        }

        // Do not allow fatal exit codes to be surfaced by non-system runtime contracts.
        //
        // Note: we intentionally do not expose an API for user code to produce these; callers
        // will be punished for attempting it.
        ExitCode::UnexpectedFatalExecutionFailure | ExitCode::MissingStorageSlot => {
            exit_code = ExitCode::UnknownError;
        }

        // Default behavior: nothing special to do.
        _ => {}
    }

    Ok(process_halt(frame, ctx, inspector, exit_code, return_data))
}

/// Route an execution result:
/// - `exit_code <= 0` means "final halt code" (Ok/Revert/Panic/…).
/// - `exit_code > 0` means "call_id" for an interruption trampoline.
///
/// For interruptions, we decode invocation params from return bytes and delegate to the
/// host interruption executor.
fn process_exec_result<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    inspector: Option<&mut INSP>,
    exit_code: i32,
    return_data: Bytes,
) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
    // Final result (success/failure) path.
    if exit_code <= 0 {
        return process_system_runtime_result(frame, ctx, inspector, exit_code, return_data);
    }

    // Interruption path: exit code is a call_id that identifies the saved context.
    let call_id = exit_code as u32;

    // Decode syscall invocation parameters from return data.
    let Some(syscall_params) = SyscallInvocationParams::decode(&return_data) else {
        unreachable!("revm: can't decode invocation params");
    };

    let gas = frame.interpreter.gas;
    let inputs = SystemInterruptionInputs {
        call_id,
        syscall_params,
        gas,
    };

    execute_rwasm_interruption::<CTX, INSP>(
        frame,
        inspector,
        ctx,
        inputs,
        crate::syscall::DefaultRuntimeExecutorMemoryReader {},
    )
}

/// Convert an rWasm/runtime `ExitCode` + return bytes into a REVM `ExecutionResult`,
/// optionally emitting an inspector syscall event to keep EVM tracing consistent.
///
/// Note: EVM traces don't natively model "traps" (interruptions), so we only emit events
/// for STOP/RETURN/REVERT-compatible outcomes.
fn process_halt<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    inspector: Option<&mut INSP>,
    exit_code: ExitCode,
    return_data: Bytes,
) -> NextAction {
    let result = instruction_result_from_exit_code(exit_code, return_data.is_empty());

    if let Some(inspector) = inspector {
        let evm_opcode = match result {
            InstructionResult::Stop => Some(opcode::STOP),
            InstructionResult::Return => Some(opcode::RETURN),
            return_revert!() => Some(opcode::REVERT),
            _ => {
                // We cannot reliably map other outcomes into a single EVM opcode for tracing.
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
