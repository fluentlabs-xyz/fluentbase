//! Syscall interruption handling for rWasm execution inside the EVM handler.
//!
//! The rWasm runtime can "trap" into the host by returning a positive `call_id` and
//! providing syscall parameters via guest memory. This module:
//! - charges EVM gas for syscalls,
//! - performs the corresponding REVM host actions (storage, logs, CALL/CREATE frames, etc.),
//! - and returns a structured outcome so the runtime can resume safely.

use crate::{
    api::RwasmFrame,
    gas::{account_warm_cold_cost, call_cost},
    types::{SystemInterruptionInputs, SystemInterruptionOutcome},
    ExecutionResult, NextAction,
};
use fluentbase_evm::{types::instruction_result_from_exit_code, EthereumMetadata};
use fluentbase_runtime::{default_runtime_executor, RuntimeExecutor};
use fluentbase_sdk::{
    byteorder::{ByteOrder, LittleEndian, ReadBytesExt},
    bytes::Buf,
    calc_create_metadata_address, is_execute_using_system_runtime, is_system_precompile, Address,
    Bytes, ExitCode, Log, LogData, B256, FUEL_DENOM_RATE, KECCAK_EMPTY, PRECOMPILE_EVM_RUNTIME,
    STATE_MAIN, U256,
};
use revm::{
    bytecode::{opcode, ownable_account::OwnableAccountBytecode, Bytecode},
    context::{
        journaled_state::{AccountLoad, JournalLoadError},
        Cfg, ContextError, ContextTr, CreateScheme, JournalTr,
    },
    interpreter::{
        gas, interpreter_types::InputsTr, CallInput, CallInputs, CallScheme, CallValue,
        CreateInputs, FrameInput, Gas, Host, StateLoad,
    },
    primitives::{
        eip3860::MAX_INITCODE_SIZE,
        hardfork::SpecId,
        wasm::{wasm_max_code_size, WASM_MAX_CODE_SIZE},
    },
    Database, Inspector,
};
use rwasm::TrapCode;
use std::{boxed::Box, vec, vec::Vec};

/// Abstraction over reading guest memory for syscall handling.
///
/// This allows tests to inject deterministic memory sources while production uses the
/// default runtime executor.
pub(crate) trait MemoryReaderTr {
    fn memory_read(&self, call_id: u32, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode>;
}

/// Memory reader that forwards reads to the default runtime executor.
pub(crate) struct DefaultRuntimeExecutorMemoryReader;

impl MemoryReaderTr for DefaultRuntimeExecutorMemoryReader {
    fn memory_read(&self, call_id: u32, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        default_runtime_executor().memory_read(call_id, offset, buffer)
    }
}

/// Execute a single syscall interruption raised by the rWasm runtime.
///
/// In this design, the runtime signals an "interruption" by returning a positive `call_id`
/// and placing syscall parameters into guest memory. The host (REVM handler) then:
/// 1) charges EVM gas according to the syscall semantics,
/// 2) optionally creates a new EVM frame (CALL/DELEGATECALL/CREATE/etc.), or
/// 3) returns a result back into the runtime and resumes execution.
///
/// This function implements step (1) and routes into (2)/(3) via `NextAction`.
pub(crate) fn execute_rwasm_interruption<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    mut inspector: Option<&mut INSP>,
    ctx: &mut CTX,
    inputs: SystemInterruptionInputs,
    mr: impl MemoryReaderTr,
) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
    let spec_id: SpecId = ctx.cfg().spec().into();

    let current_target_address = frame.interpreter.input.target_address();
    let account_owner_address = frame.interpreter.input.account_owner_address();

    let is_static = frame.interpreter.runtime_flag.is_static;

    // Modified `Journal::load_account_delegated()` logic, with extra pre-checks to support.
    // A "skip cold load" fast-path when we are close to running out of gas.
    macro_rules! load_account_with_gas_pre_checks {
        ($target_address:expr) => {{
            let mut cost = 0;
            let warm_storage_read_cost = ctx.cfg().gas_params().warm_storage_read_cost();
            let cold_account_additional_cost =
                ctx.cfg().gas_params().cold_account_additional_cost();
            let skip_cold = frame.interpreter.gas.remaining() < cold_account_additional_cost;
            let is_eip7702_enabled = spec_id.is_enabled_in(SpecId::PRAGUE);
            let result = ctx.journal_mut().load_account_info_skip_cold_load(
                $target_address,
                is_eip7702_enabled,
                skip_cold,
            );
            let account_info_load = unwrap_journal_load_error!(result);

            let is_empty = account_info_load.is_empty();
            if account_info_load.is_cold {
                cost += cold_account_additional_cost;
            }

            let mut account_load = StateLoad::new(
                AccountLoad {
                    is_delegate_account_cold: None,
                    is_empty,
                },
                account_info_load.is_cold,
            );

            // Load delegate code if the account uses EIP-7702 delegation.
            if let Some(Bytecode::Eip7702(code)) = &account_info_load.code {
                let address = code.address();
                cost += warm_storage_read_cost;
                if cost > frame.interpreter.gas.remaining() {
                    return_halt!(OutOfFuel);
                }
                let skip_cold =
                    frame.interpreter.gas.remaining() < cost + cold_account_additional_cost;
                let result = ctx
                    .journal_mut()
                    .load_account_info_skip_cold_load(address, true, skip_cold);
                let delegate_account = unwrap_journal_load_error!(result);
                account_load.data.is_delegate_account_cold = Some(delegate_account.is_cold);
            }

            account_load
        }};
    }
    macro_rules! return_result {
        ($output:expr, $result:ident) => {{
            let output: Bytes = $output.into();
            let result = ExecutionResult {
                result: instruction_result_from_exit_code(ExitCode::$result, output.is_empty()),
                output,
                gas: Gas::new_spent(frame.interpreter.gas.spent() - inputs.gas.spent()),
            };
            frame.insert_interrupted_outcome(SystemInterruptionOutcome {
                inputs,
                result: Some(result),
                halted_frame: false,
            });
            return Ok(NextAction::InterruptionResult);
        }};
        ($result:ident) => {{
            return_result!(Bytes::default(), $result)
        }};
    }
    macro_rules! return_halt {
        ($result:ident) => {{
            // For system runtime contracts, always forward the execution result.
            // Runtime and EVM frames are synchronized; otherwise we risk memory corruption.
            let is_system_runtime = account_owner_address
                .filter(is_execute_using_system_runtime)
                .is_some();
            if is_system_runtime {
                let result = ExecutionResult {
                    result: instruction_result_from_exit_code(ExitCode::$result, true),
                    output: Bytes::new(),
                    gas: Gas::new_spent(frame.interpreter.gas.spent() - inputs.gas.spent()),
                };
                frame.insert_interrupted_outcome(SystemInterruptionOutcome {
                    inputs,
                    result: Some(result),
                    halted_frame: true,
                });
                return Ok(NextAction::InterruptionResult);
            }
            let result = ExecutionResult {
                result: instruction_result_from_exit_code(ExitCode::$result, true),
                output: Bytes::new(),
                gas: Gas::new_spent(frame.interpreter.gas.spent() - inputs.gas.spent()),
            };
            return Ok(NextAction::Return(result));
        }};
    }
    macro_rules! return_frame {
        ($action:expr) => {{
            frame.insert_interrupted_outcome(SystemInterruptionOutcome {
                inputs,
                result: None,
                halted_frame: false,
            });
            return Ok($action);
        }};
    }
    macro_rules! assert_halt {
        ($cond:expr, $error:ident) => {
            if !($cond) {
                return_halt!($error);
            }
        };
    }
    macro_rules! charge_gas {
        ($value:expr) => {{
            if !frame.interpreter.gas.record_cost($value) {
                return_halt!(OutOfFuel);
            }
        }};
    }
    macro_rules! inspect {
        ($evm_opcode:expr, $inputs:expr, $outputs:expr) => {{
            if let Some(inspector) = inspector.as_mut() {
                crate::inspector::inspect_syscall(frame, ctx, inspector, $evm_opcode, $inputs);
            }
        }};
    }

    macro_rules! get_input_validated {
        (== $length:expr) => {{
            assert_halt!(
                inputs.syscall_params.input.len() == $length
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            let mut input = [0u8; $length];
            if mr
                .memory_read(
                    inputs.call_id,
                    inputs.syscall_params.input.start,
                    &mut input,
                )
                .is_err()
            {
                return_result!(MemoryOutOfBounds)
            }
            input
        }};
        (>= $length:expr) => {{
            assert_halt!(
                inputs.syscall_params.input.len() >= $length
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            let mut input = vec![0u8; $length];
            if mr
                .memory_read(
                    inputs.call_id,
                    inputs.syscall_params.input.start,
                    &mut input,
                )
                .is_err()
            {
                return_result!(MemoryOutOfBounds)
            }
            let call_id = inputs.call_id;
            let remaining_offset = inputs.syscall_params.input.start + $length;
            let remaining_length =
                inputs.syscall_params.input.end - inputs.syscall_params.input.start - $length;
            let lazy_contract_input = move || -> Result<Vec<u8>, TrapCode> {
                let mut variable_input = vec![0u8; remaining_length];
                mr.memory_read(call_id, remaining_offset, &mut variable_input)?;
                Ok(variable_input)
            };
            (input, lazy_contract_input)
        }};
    }
    macro_rules! unwrap_journal_load_error {
        ($load_result:expr) => {
            match $load_result {
                Ok(v) => v,
                Err(e) => match e {
                    JournalLoadError::ColdLoadSkipped => return_halt!(OutOfFuel),
                    JournalLoadError::DBError(e) => return Err(ContextError::Db(e)),
                },
            }
        };
    }

    use fluentbase_sdk::syscall::*;
    match inputs.syscall_params.code_hash {
        SYSCALL_ID_STORAGE_READ => {
            let input = get_input_validated!(== 32);
            let slot = U256::from_le_slice(&input[0..32]);
            let skip_cold =
                frame.interpreter.gas.remaining() < ctx.cfg().gas_params().cold_storage_cost();
            let result =
                ctx.journal_mut()
                    .sload_skip_cold_load(current_target_address, slot, skip_cold);
            let value = unwrap_journal_load_error!(result);
            charge_gas!(if value.is_cold {
                ctx.cfg().gas_params().cold_storage_cost()
            } else {
                ctx.cfg().gas_params().warm_storage_read_cost()
            });
            inspect!(opcode::SLOAD, [slot], [value.data]);
            let output: [u8; 32] = value.to_le_bytes();
            return_result!(output, Ok)
        }

        SYSCALL_ID_STORAGE_WRITE => {
            assert_halt!(!is_static, StateChangeDuringStaticCall);
            let input = get_input_validated!(== 64);
            let slot = U256::from_le_slice(&input[0..32]);
            let new_value = U256::from_le_slice(&input[32..64]);
            let skip_cold =
                frame.interpreter.gas.remaining() < ctx.cfg().gas_params().cold_storage_cost();
            let result = ctx.journal_mut().sstore_skip_cold_load(
                current_target_address,
                slot,
                new_value,
                skip_cold,
            );
            let state_load = unwrap_journal_load_error!(result);
            assert_halt!(
                frame.interpreter.gas.remaining() > ctx.cfg().gas_params().call_stipend(),
                OutOfFuel
            );
            let gas_cost = ctx.cfg().gas_params().sstore_static_gas()
                + ctx.cfg().gas_params().sstore_dynamic_gas(
                    true,
                    &state_load.data,
                    state_load.is_cold,
                );
            charge_gas!(gas_cost);
            let gas_refund = ctx.cfg().gas_params().sstore_refund(true, &state_load.data);
            frame.interpreter.gas.record_refund(gas_refund);
            inspect!(opcode::SSTORE, [slot, new_value], []);
            return_result!(Ok)
        }

        SYSCALL_ID_CALL => {
            let (input, lazy_contract_input) = get_input_validated!(>= 20 + 32);
            let target_address = Address::from_slice(&input[0..20]);
            let value = U256::from_le_slice(&input[20..52]);
            // For STATICCALL-like semantics, a non-zero value transfer is forbidden (revert).
            let has_transfer = !value.is_zero();
            if is_static && has_transfer {
                return_halt!(StateChangeDuringStaticCall);
            }
            let mut account_load = load_account_with_gas_pre_checks!(target_address);
            // EVM quirk: precompiles are "preloaded" and typically empty/stately-less.
            // However, a precompile can also be explicitly included in genesis, which changes.
            // Its account states and affects CALL gas accounting.
            // Using CALL to invoke a precompile is usually pointless (precompiles are.
            // Effectively stateless), but some test suites require this edge case.
            // Marking system precompiles as empty improves EVM compatibility, even though it may.
            // Cause certain unit tests to fail. We accept that trade-off.
            if is_system_precompile(&target_address) {
                account_load.is_empty = true;
            }
            // EIP-150: gas cost changes for IO-heavy operations.
            charge_gas!(call_cost(spec_id, has_transfer, account_load));
            let mut gas_limit = core::cmp::min(
                ctx.cfg()
                    .gas_params()
                    .call_stipend_reduction(frame.interpreter.gas.remaining()),
                inputs.syscall_params.fuel_limit / FUEL_DENOM_RATE,
            );
            charge_gas!(gas_limit);
            if has_transfer {
                gas_limit = gas_limit.saturating_add(ctx.cfg().gas_params().call_stipend());
            }
            inspect!(
                opcode::CALL,
                [
                    U256::from(gas_limit),             // gas
                    target_address.into_word().into(), // addr
                    value,                             // value
                    U256::ZERO,                        // argsOffset
                    U256::ZERO,                        // argsLength
                    U256::ZERO,                        // retOffset
                    U256::ZERO                         // retLength
                ],
                []
            );
            // Read contract input only after all gas has been charged (avoid allocation on OOG).
            let Ok(contract_input) = lazy_contract_input() else {
                return_halt!(MemoryOutOfBounds);
            };
            // Build `CallInputs` for the new frame.
            let call_inputs = Box::new(CallInputs {
                input: CallInput::Bytes(contract_input.into()),
                gas_limit,
                target_address,
                caller: current_target_address,
                bytecode_address: target_address,
                value: CallValue::Transfer(value),
                scheme: CallScheme::Call,
                is_static,
                return_memory_offset: Default::default(),
                known_bytecode: None,
            });
            return_frame!(NextAction::NewFrame(FrameInput::Call(call_inputs)));
        }

        SYSCALL_ID_STATIC_CALL => {
            let (input, lazy_contract_input) = get_input_validated!(>= 20);
            let target_address = Address::from_slice(&input[0..20]);
            let mut account_load = load_account_with_gas_pre_checks!(target_address);
            // Force `is_empty = false`: we are not creating this account here.
            account_load.is_empty = false;
            // EIP-150: gas cost changes for IO-heavy operations.
            charge_gas!(call_cost(spec_id.clone(), false, account_load));
            let gas_limit = core::cmp::min(
                ctx.cfg()
                    .gas_params()
                    .call_stipend_reduction(frame.interpreter.gas.remaining()),
                inputs.syscall_params.fuel_limit / FUEL_DENOM_RATE,
            );
            charge_gas!(gas_limit);
            inspect!(
                opcode::STATICCALL,
                [
                    U256::from(gas_limit),             // gas
                    target_address.into_word().into(), // addr
                    U256::ZERO,                        // argsOffset
                    U256::ZERO,                        // argsLength
                    U256::ZERO,                        // retOffset
                    U256::ZERO                         // retLength
                ],
                []
            );
            // Read contract input only after all gas has been charged (avoid allocation on OOG).
            let Ok(contract_input) = lazy_contract_input() else {
                return_halt!(MemoryOutOfBounds);
            };
            // Build `CallInputs` for the new frame.
            let call_inputs = Box::new(CallInputs {
                input: CallInput::Bytes(contract_input.into()),
                gas_limit,
                target_address,
                caller: current_target_address,
                bytecode_address: target_address,
                value: CallValue::Transfer(U256::ZERO),
                scheme: CallScheme::StaticCall,
                is_static: true,
                return_memory_offset: Default::default(),
                known_bytecode: None,
            });
            return_frame!(NextAction::NewFrame(FrameInput::Call(call_inputs)));
        }

        SYSCALL_ID_CALL_CODE => {
            let (input, lazy_contract_input) = get_input_validated!(>= 20 + 32);
            let target_address = Address::from_slice(&input[0..20]);
            let value = U256::from_le_slice(&input[20..52]);
            let mut account_load = load_account_with_gas_pre_checks!(target_address);
            // Set is_empty to false as we are not creating this account.
            account_load.is_empty = false;
            let has_transfer = !value.is_zero();
            // EIP-150: gas cost changes for IO-heavy operations.
            charge_gas!(call_cost(spec_id, has_transfer, account_load));
            let mut gas_limit = core::cmp::min(
                ctx.cfg()
                    .gas_params()
                    .call_stipend_reduction(frame.interpreter.gas.remaining()),
                inputs.syscall_params.fuel_limit / FUEL_DENOM_RATE,
            );
            charge_gas!(gas_limit);
            // Add a call stipend if there is a value to be transferred.
            if !value.is_zero() {
                gas_limit = gas_limit.saturating_add(ctx.cfg().gas_params().call_stipend());
            }
            inspect!(
                opcode::CALLCODE,
                [
                    U256::from(gas_limit),             // gas
                    target_address.into_word().into(), // addr
                    value,                             // value
                    U256::ZERO,                        // argsOffset
                    U256::ZERO,                        // argsLength
                    U256::ZERO,                        // retOffset
                    U256::ZERO                         // retLength
                ],
                []
            );
            // Read contract input only after all gas has been charged (avoid allocation on OOG).
            let Ok(contract_input) = lazy_contract_input() else {
                return_halt!(MemoryOutOfBounds);
            };
            // Build `CallInputs` for the new frame.
            let call_inputs = Box::new(CallInputs {
                input: CallInput::Bytes(contract_input.into()),
                gas_limit,
                target_address: current_target_address,
                caller: current_target_address,
                bytecode_address: target_address,
                value: CallValue::Transfer(value),
                scheme: CallScheme::CallCode,
                is_static,
                return_memory_offset: Default::default(),
                known_bytecode: None,
            });
            return_frame!(NextAction::NewFrame(FrameInput::Call(call_inputs)));
        }

        SYSCALL_ID_DELEGATE_CALL => {
            let (input, lazy_contract_input) = get_input_validated!(>= 20);
            let target_address = Address::from_slice(&input[0..20]);
            let mut account_load = load_account_with_gas_pre_checks!(target_address);
            // Force `is_empty = false`: we are not creating this account here.
            account_load.is_empty = false;
            // EIP-150: gas cost changes for IO-heavy operations.
            charge_gas!(call_cost(spec_id, false, account_load));
            let gas_limit = core::cmp::min(
                ctx.cfg()
                    .gas_params()
                    .call_stipend_reduction(frame.interpreter.gas.remaining()),
                inputs.syscall_params.fuel_limit / FUEL_DENOM_RATE,
            );
            charge_gas!(gas_limit);
            inspect!(
                opcode::DELEGATECALL,
                [
                    U256::from(gas_limit),             // gas
                    target_address.into_word().into(), // addr
                    U256::ZERO,                        // argsOffset
                    U256::ZERO,                        // argsLength
                    U256::ZERO,                        // retOffset
                    U256::ZERO                         // retLength
                ],
                []
            );
            // Read contract input only after all gas has been charged (avoid allocation on OOG).
            let Ok(contract_input) = lazy_contract_input() else {
                return_halt!(MemoryOutOfBounds);
            };
            // Build `CallInputs` for the new frame.
            let call_inputs = Box::new(CallInputs {
                input: CallInput::Bytes(contract_input.into()),
                gas_limit,
                target_address: current_target_address,
                caller: frame.interpreter.input.caller_address(),
                bytecode_address: target_address,
                value: CallValue::Apparent(frame.interpreter.input.call_value()),
                scheme: CallScheme::DelegateCall,
                is_static,
                return_memory_offset: Default::default(),
                known_bytecode: None,
            });
            return_frame!(NextAction::NewFrame(FrameInput::Call(call_inputs)));
        }

        SYSCALL_ID_CREATE | SYSCALL_ID_CREATE2 => {
            assert_halt!(!is_static, StateChangeDuringStaticCall);

            // Enforce a hard upper bound on the syscall input size.
            const HARD_CAP: usize = WASM_MAX_CODE_SIZE + U256::BYTES + U256::BYTES;
            assert_halt!(
                inputs.syscall_params.input.len() <= HARD_CAP,
                MalformedBuiltinParams
            );

            // CREATE2 uses a different address derivation scheme and gas calculation.
            let is_create2 = inputs.syscall_params.code_hash == SYSCALL_ID_CREATE2;

            let (input, lazy_init_code) = get_input_validated!(>= if is_create2 {
                U256::BYTES + U256::BYTES
            } else {
                U256::BYTES
            });

            // Validate that syscall parameters contain the required fixed-size prefix.
            let (scheme, value) = if is_create2 {
                let value = U256::from_le_slice(&input[0..32]);
                let salt = U256::from_le_slice(&input[32..64]);
                (CreateScheme::Create2 { salt }, value)
            } else {
                let value = U256::from_le_slice(&input[0..32]);
                (CreateScheme::Create, value)
            };

            // Enforce initcode size limits (EIP-3860 / Wasm-specific caps).
            let init_code_length = inputs.syscall_params.input.len() - input.len();
            if init_code_length > 0 {
                charge_gas!(ctx.cfg().gas_params().initcode_cost(init_code_length));
            }
            let Ok(init_code) = lazy_init_code() else {
                return_halt!(MemoryOutOfBounds);
            };
            let max_initcode_size = wasm_max_code_size(&init_code).unwrap_or(MAX_INITCODE_SIZE);
            assert_halt!(
                init_code_length <= max_initcode_size,
                CreateContractSizeLimit
            );
            if is_create2 {
                charge_gas!(ctx.cfg().gas_params().create2_cost(init_code_length));
            } else {
                charge_gas!(ctx.cfg().gas_params().create_cost());
            };

            let mut gas_limit = frame.interpreter.gas.remaining();
            gas_limit -= gas_limit / 64;
            charge_gas!(gas_limit);

            match scheme {
                CreateScheme::Create => {
                    inspect!(opcode::CREATE, [value, U256::ZERO, U256::ZERO], []);
                }
                CreateScheme::Create2 { salt } => {
                    inspect!(opcode::CREATE2, [value, U256::ZERO, U256::ZERO, salt], []);
                }
                CreateScheme::Custom { .. } => {}
            }
            let create_inputs = Box::new(CreateInputs::new(
                current_target_address,
                scheme,
                value,
                init_code.into(),
                gas_limit,
            ));
            return_frame!(NextAction::NewFrame(FrameInput::Create(create_inputs)));
        }

        SYSCALL_ID_EMIT_LOG => {
            assert_halt!(!is_static, StateChangeDuringStaticCall);
            // Read the number of topics and ensure it does not exceed 4 (EVM LOG0..LOG4).
            // (The EVM hard-limits log topics to 4.)
            assert_halt!(
                inputs.syscall_params.input.len() >= 1 && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            let mut input = [0u8; 1];
            if mr
                .memory_read(
                    inputs.call_id,
                    inputs.syscall_params.input.start,
                    &mut input,
                )
                .is_err()
            {
                return_result!(MemoryOutOfBounds)
            }
            let topics_len = input[0] as usize;
            assert_halt!(topics_len <= 4, MalformedBuiltinParams);
            // Read topics without reading the data payload yet, so we can charge gas for.
            // The payload length before allocating/reading it.
            // (This avoids extra allocations and prevents cheap OOG/DDoS vectors.)
            let mut topics = Vec::with_capacity(topics_len);
            let (input, lazy_data_input) = get_input_validated!(>= 1 + topics_len * U256::BYTES);
            for i in 0..topics_len {
                let offset = 1 + i * B256::len_bytes();
                let topic = &input[offset..(offset + B256::len_bytes())];
                topics.push(B256::from_slice(topic));
            }
            let data_length = inputs.syscall_params.input.len() - 1 - topics_len * U256::BYTES;
            // Ensure we have enough gas before reading the data payload; otherwise a DDoS vector.
            // Exists via forced allocation/reads.
            charge_gas!(
                gas::LOG
                    + ctx
                        .cfg()
                        .gas_params()
                        .log_cost(topics_len as u8, data_length as u64)
            );
            // All remaining bytes are the log data payload.
            let Ok(data) = lazy_data_input() else {
                return_halt!(MemoryOutOfBounds);
            };
            #[rustfmt::skip]
            match topics_len {
                0 => inspect!(opcode::LOG0, [U256::ZERO, U256::ZERO], []),
                1 => inspect!(opcode::LOG1, [U256::ZERO, U256::ZERO, topics[0].into()], []),
                2 => inspect!(opcode::LOG2, [U256::ZERO, U256::ZERO, topics[0].into(), topics[1].into()], []),
                3 => inspect!(opcode::LOG3, [U256::ZERO, U256::ZERO, topics[0].into(), topics[1].into(), topics[2].into()], []),
                4 => inspect!(opcode::LOG4, [U256::ZERO, U256::ZERO, topics[0].into(), topics[1].into(), topics[2].into(), topics[3].into()], []),
                _ => unreachable!(),
            };
            ctx.journal_mut().log(Log {
                address: current_target_address,
                // SAFETY: `new_unchecked` is safe because we enforce the topic-count upper bound.
                data: LogData::new_unchecked(topics, data.into()),
            });
            return_result!(Ok);
        }

        SYSCALL_ID_DESTROY_ACCOUNT => {
            let input = get_input_validated!(== 20);
            // Not allowed in a static context.
            assert_halt!(!is_static, StateChangeDuringStaticCall);
            // Self-destruct the current contract, sending any remaining balance to `target`.
            let target = Address::from_slice(&input[0..20]);
            let skip_cold = frame.interpreter.gas.remaining()
                < ctx.cfg().gas_params().cold_account_additional_cost()
                    + ctx.cfg().gas_params().warm_storage_read_cost();
            let result = ctx
                .journal_mut()
                .selfdestruct(current_target_address, target, skip_cold);
            let mut result = unwrap_journal_load_error!(result);
            // System precompiles are treated as empty accounts for gas/state semantics.
            if result.data.target_exists && is_system_precompile(&target) {
                result.data.target_exists = false;
            }
            // Charge gas for SELFDESTRUCT based on the current hardfork rules.
            let should_charge_top_up = result.data.had_value && !result.data.target_exists;
            charge_gas!(
                5000 + ctx
                    .cfg()
                    .gas_params()
                    .selfdestruct_cost(should_charge_top_up, result.is_cold)
            );
            // Return success (no output payload).
            return_result!(Ok);
        }

        SYSCALL_ID_BALANCE => {
            let input = get_input_validated!(== 20);
            let address = Address::from_slice(&input[0..20]);
            // Load account info (and optionally code) with Berlin warm/cold accounting.
            let skip_cold = frame.interpreter.gas.remaining()
                < ctx.cfg().gas_params().cold_account_additional_cost();
            let account_info_load = unwrap_journal_load_error!(ctx
                .journal_mut()
                .load_account_info_skip_cold_load(address, false, skip_cold));
            let balance_load = StateLoad::new(account_info_load.balance, account_info_load.is_cold);
            // Charge gas for BALANCE according to the active hardfork.
            charge_gas!(account_warm_cold_cost(balance_load.is_cold));
            // Return the balance as a 32-byte little-endian word.
            let output: [u8; 32] = balance_load.data.to_le_bytes();
            return_result!(output, Ok);
        }

        SYSCALL_ID_SELF_BALANCE => {
            let _ = get_input_validated!(== 0);
            let value = ctx
                .journal_mut()
                .load_account(current_target_address)
                .map(|acc| acc.map(|a| a.info.balance))?;
            charge_gas!(gas::LOW);
            let output: [u8; 32] = value.data.to_le_bytes();
            return_result!(output, Ok)
        }

        SYSCALL_ID_CODE_SIZE => {
            let input = get_input_validated!(== 20);
            let address = Address::from_slice(&input[0..20]);
            let skip_cold = frame.interpreter.gas.remaining()
                < ctx.cfg().gas_params().cold_account_additional_cost();
            // Load account info (and optionally code) with Berlin warm/cold accounting.
            let result = ctx
                .journal_mut()
                .load_account_info_skip_cold_load(address, true, skip_cold);
            let account_info = unwrap_journal_load_error!(result);
            charge_gas!(account_warm_cold_cost(account_info.is_cold));
            // A special case for precompiled runtimes, where the way of extracting bytecode might be different.
            // We keep this condition here and moved away from the runtime because Rust applications.
            // Might also request EVM bytecode and initiating extra interruptions to fetch the data might be redundant.
            let mut code_len = match &account_info.code {
                Some(Bytecode::OwnableAccount(ownable_account_bytecode))
                    if ownable_account_bytecode.owner_address == PRECOMPILE_EVM_RUNTIME =>
                {
                    EthereumMetadata::read_from_bytes(&ownable_account_bytecode.metadata)
                        .as_ref()
                        .map(EthereumMetadata::code_size)
                        .unwrap_or(0)
                }
                code => code.as_ref().map(Bytecode::len).unwrap_or(0),
            };
            // We store system precompile bytecode in the state trie,
            // According to EVM requirements, we should return empty code.
            if is_system_precompile(&address) {
                code_len = 0;
            }
            // Code size we encode as 32-bytes in LE encoding,
            // There is no need to return it as 32-bytes array, but it's more EVM-friendly.
            let code_size = U256::from(code_len);
            return_result!(code_size.to_le_bytes::<32>(), Ok);
        }

        SYSCALL_ID_CODE_HASH => {
            let input = get_input_validated!(== 20);
            let address = Address::from_slice(&input[0..20]);
            let skip_cold = frame.interpreter.gas.remaining()
                < ctx.cfg().gas_params().cold_account_additional_cost();
            // Load account info (and optionally code) with Berlin warm/cold accounting.
            let result = ctx
                .journal_mut()
                .load_account_info_skip_cold_load(address, false, skip_cold);
            let account_info = unwrap_journal_load_error!(result);
            charge_gas!(account_warm_cold_cost(account_info.is_cold));

            // Extract code hash for an account for a delegated account.
            // For EVM, we extract code hash from the metadata to satisfy EVM requirements.
            // It requires the account to be loaded with bytecode.
            let mut code_hash = match &account_info.code {
                Some(Bytecode::OwnableAccount(ownable_account_bytecode))
                    if ownable_account_bytecode.owner_address == PRECOMPILE_EVM_RUNTIME =>
                {
                    EthereumMetadata::read_from_bytes(&ownable_account_bytecode.metadata)
                        .as_ref()
                        .map(EthereumMetadata::code_hash)
                        .unwrap_or(B256::ZERO)
                }
                // We return code hash only if account exists (not empty),
                // This is a requirement from EVM.
                _ if account_info.is_empty() => B256::ZERO,
                _ => account_info.code_hash,
            };

            if is_system_precompile(&address) {
                // We store system precompile bytecode in the state trie,
                // According to EVM requirements, we should return empty code.
                code_hash = B256::ZERO;
            } else if code_hash == B256::ZERO && !account_info.is_empty() {
                // If the delegated code hash is zero, then it might be a contract deployment stage,
                // For non-empty account return KECCAK_EMPTY.
                code_hash = KECCAK_EMPTY;
            }

            return_result!(code_hash, Ok);
        }

        SYSCALL_ID_CODE_COPY => {
            let input = get_input_validated!(== 20 + 8 * 2);
            let address = Address::from_slice(&input[0..20]);
            let mut reader = input[20..].reader();
            let code_offset = reader.read_u64::<LittleEndian>().unwrap();
            let code_length = reader.read_u64::<LittleEndian>().unwrap();

            // Invariant: gas is charged for the requested length, not the actual returned length.
            // This prevents gas abuse where an attacker requests a small length but expects the full bytecode.
            charge_gas!(ctx.cfg().gas_params().extcodecopy(code_length as usize));

            let skip_cold = frame.interpreter.gas.remaining()
                < ctx.cfg().gas_params().cold_account_additional_cost();
            // Load account info (and optionally code) with Berlin warm/cold accounting.
            let result = ctx
                .journal_mut()
                .load_account_info_skip_cold_load(address, true, skip_cold);
            let account_info = unwrap_journal_load_error!(result);
            charge_gas!(account_warm_cold_cost(account_info.is_cold));

            // Early return for zero-length request.
            if code_length == 0 {
                return_result!(Bytes::new(), Ok);
            }

            // Load bytecode from an account.
            let mut bytecode = match &account_info.code {
                Some(Bytecode::OwnableAccount(ownable_account_bytecode))
                    if ownable_account_bytecode.owner_address == PRECOMPILE_EVM_RUNTIME =>
                {
                    EthereumMetadata::read_from_bytes(&ownable_account_bytecode.metadata)
                        .as_ref()
                        .map(EthereumMetadata::code_copy)
                        .unwrap_or_default()
                }
                code => code
                    .as_ref()
                    .map(Bytecode::original_bytes)
                    .unwrap_or_default(),
            };

            // System precompiles return empty code per EVM requirements.
            if is_system_precompile(&address) {
                bytecode = Bytes::new();
            }

            let bytecode_len = bytecode.len();
            let code_offset_usize = code_offset as usize;
            let code_length_usize = code_length as usize;

            // If the offset is beyond bytecode, return all zeros.
            if code_offset_usize >= bytecode_len {
                let mut zeros = Vec::with_capacity(code_length_usize);
                zeros.resize(code_length_usize, 0u8);
                return_result!(Bytes::from(zeros), Ok);
            }

            let start = code_offset_usize;
            let available = bytecode_len - start;
            let to_copy = core::cmp::min(code_length_usize, available);

            // Fast path: If no padding needed, return zero-copy slice.
            if to_copy == code_length_usize {
                let result = bytecode.slice(start..start + to_copy);
                return_result!(result, Ok);
            }

            // Slow path: Padding required to reach the requested length.
            let mut result = Vec::with_capacity(code_length_usize);
            result.resize(code_length_usize, 0u8);
            result[..to_copy].copy_from_slice(&bytecode[start..start + to_copy]);

            return_result!(Bytes::from(result), Ok);
        }

        SYSCALL_ID_METADATA_SIZE => {
            let Some(account_owner_address) = account_owner_address else {
                return_halt!(MalformedBuiltinParams);
            };
            let input = get_input_validated!(== 20);
            // Read an account from its address.
            let address = Address::from_slice(&input[..20]);
            let account = ctx.journal_mut().load_account_with_code(address)?;
            // To make sure this account is ownable and owner by the same runtime, that allows.
            // A runtime to modify any account it owns.
            let (metadata_size, is_same_runtime) = match account.info.code.as_ref() {
                Some(Bytecode::OwnableAccount(ownable_account_bytecode)) => {
                    // If an account is not the same - it's not a malformed building param, runtime might not know it's account.
                    if ownable_account_bytecode.owner_address == account_owner_address {
                        (ownable_account_bytecode.metadata.len() as u32, true)
                    } else {
                        (0, true)
                    }
                }
                _ => (0, false),
            };
            // Execute a syscall.
            let mut output = [0u8; 4 + 3];
            LittleEndian::write_u32(&mut output, metadata_size);
            output[4] = is_same_runtime as u8; // the account belongs to the same runtime
            output[5] = account.is_cold as u8;
            output[6] = account.is_empty() as u8;
            return_result!(output, Ok)
        }

        SYSCALL_ID_METADATA_ACCOUNT_OWNER => {
            let Some(account_owner_address) = account_owner_address else {
                return_halt!(MalformedBuiltinParams);
            };
            let input = get_input_validated!(== Address::len_bytes());
            let address = Address::from_slice(&input[..Address::len_bytes()]);
            if address == current_target_address {
                return_result!(account_owner_address.0, Ok);
            }
            let account = ctx.journal_mut().load_account_with_code(address)?;
            match account.info.code.as_ref() {
                Some(Bytecode::OwnableAccount(ownable_account_bytecode)) => {
                    return_result!(ownable_account_bytecode.owner_address.0, Ok)
                }
                _ => return_result!(Address::ZERO.0, Ok),
            };
        }

        SYSCALL_ID_METADATA_CREATE => {
            let Some(account_owner_address) = account_owner_address else {
                return_halt!(MalformedBuiltinParams);
            };
            assert_halt!(!is_static, StateChangeDuringStaticCall);
            let (input, lazy_metadata_input) = get_input_validated!(>= 32);
            let salt = U256::from_be_slice(&input);
            let derived_metadata_address =
                calc_create_metadata_address(&account_owner_address, &salt);
            let account = ctx
                .journal_mut()
                .load_account_with_code(derived_metadata_address)?;
            // Verify no deployment collision exists at the derived address.
            // Check only code_hash and nonce - intentionally ignore balance to prevent.
            // Front-running DoS where the attacker funds an address before legitimate creation.
            // This matches Ethereum CREATE/CREATE2 behavior: accounts can be pre-funded.
            if account.info.code_hash != KECCAK_EMPTY || account.info.nonce != 0 {
                return_result!(CreateContractCollision);
            }
            // Create a new derived ownable account.
            let Ok(metadata_input) = lazy_metadata_input() else {
                return_halt!(MemoryOutOfBounds);
            };
            ctx.journal_mut().set_code(
                derived_metadata_address,
                Bytecode::OwnableAccount(OwnableAccountBytecode::new(
                    account_owner_address,
                    metadata_input.into(),
                )),
            );
            return_result!(Bytes::new(), Ok)
        }

        SYSCALL_ID_METADATA_WRITE => {
            let Some(account_owner_address) = account_owner_address else {
                return_halt!(MalformedBuiltinParams);
            };
            assert_halt!(!is_static, StateChangeDuringStaticCall);
            let (input, lazy_metadata_input) = get_input_validated!(>= 20 + 4);
            // Read an account from its address.
            let address = Address::from_slice(&input[..20]);
            let _offset = LittleEndian::read_u32(&input[20..24]) as usize;
            let account = ctx.journal_mut().load_account_with_code(address)?;
            // To make sure this account is ownable and owner by the same runtime, that allows.
            // A runtime to modify any account it owns.
            let ownable_account_bytecode = match account.info.code.as_ref() {
                Some(Bytecode::OwnableAccount(ownable_account_bytecode))
                    if ownable_account_bytecode.owner_address == account_owner_address =>
                {
                    ownable_account_bytecode
                }
                _ => {
                    return_halt!(MalformedBuiltinParams)
                }
            };
            let Ok(new_metadata) = lazy_metadata_input() else {
                return_halt!(MemoryOutOfBounds);
            };
            // Code might change, rewrite it with a new hash.
            let new_bytecode = Bytecode::OwnableAccount(OwnableAccountBytecode::new(
                ownable_account_bytecode.owner_address,
                new_metadata.into(),
            ));
            ctx.journal_mut().set_code(address, new_bytecode);
            return_result!(Bytes::new(), Ok)
        }

        SYSCALL_ID_METADATA_COPY => {
            let Some(account_owner_address) = account_owner_address else {
                return_halt!(MalformedBuiltinParams);
            };
            let input = get_input_validated!(== 28);
            // Read an account from its address.
            let address = Address::from_slice(&input[..20]);
            let account = ctx.journal_mut().load_account_with_code(address)?;
            // To make sure this account is ownable and owner by the same runtime, that allows.
            // A runtime to modify any account it owns.
            let ownable_account_bytecode = match account.info.code.as_ref() {
                Some(Bytecode::OwnableAccount(ownable_account_bytecode))
                    if ownable_account_bytecode.owner_address == account_owner_address =>
                {
                    ownable_account_bytecode
                }
                _ => {
                    return_halt!(MalformedBuiltinParams)
                }
            };
            let offset = LittleEndian::read_u32(&input[20..24]) as usize;
            let length = LittleEndian::read_u32(&input[24..28]) as usize;
            // Take min.
            let metadata_len = ownable_account_bytecode.metadata.len();

            // If the offset is beyond the end of metadata, nothing can be copied - return empty.
            if offset >= metadata_len {
                return_result!(Bytes::new(), Ok);
            }

            // Clamp the requested length to the remaining bytes after `offset`.
            let copy_len = core::cmp::min(length, length - offset);
            let metadata = ownable_account_bytecode
                .metadata
                .slice(offset..(offset + copy_len));

            return_result!(metadata, Ok)
        }

        SYSCALL_ID_METADATA_STORAGE_READ => {
            let Some(account_owner_address) = account_owner_address else {
                return_halt!(MalformedBuiltinParams);
            };
            let input = get_input_validated!(== U256::BYTES);

            let Ok(slot): Result<[u8; U256::BYTES], _> = input[..U256::BYTES].try_into() else {
                return_halt!(MalformedBuiltinParams)
            };
            let slot_u256 = U256::from_le_bytes(slot);
            let value = ctx.journal_mut().sload(account_owner_address, slot_u256)?;
            let output: [u8; U256::BYTES] = value.to_le_bytes();
            return_result!(output, Ok);
        }

        SYSCALL_ID_METADATA_STORAGE_WRITE => {
            let Some(account_owner_address) = account_owner_address else {
                return_halt!(MalformedBuiltinParams);
            };
            assert_halt!(!is_static, StateChangeDuringStaticCall);
            // Input: slot + value.
            let input = get_input_validated!(== U256::BYTES + U256::BYTES);

            let Ok(slot): Result<[u8; U256::BYTES], _> = input[..U256::BYTES].try_into() else {
                return_halt!(MalformedBuiltinParams)
            };
            let Ok(value): Result<[u8; U256::BYTES], _> = input[U256::BYTES..].try_into() else {
                return_halt!(MalformedBuiltinParams)
            };

            let slot_u256 = U256::from_le_bytes(slot);
            let value_u256 = U256::from_le_bytes(value);
            ctx.journal_mut()
                .sstore(account_owner_address, slot_u256, value_u256)?;
            ctx.journal_mut().touch_account(account_owner_address);

            return_result!(Bytes::default(), Ok);
        }

        SYSCALL_ID_TRANSIENT_READ => {
            let input = get_input_validated!(== 32);
            // Charge gas.
            charge_gas!(ctx.cfg().gas_params().warm_storage_read_cost());
            // Read value from storage.
            let slot = U256::from_le_slice(&input[0..32].as_ref());
            let value = ctx.journal_mut().tload(current_target_address, slot);
            // Return value.
            let output: [u8; 32] = value.to_le_bytes();
            return_result!(output, Ok);
        }

        SYSCALL_ID_TRANSIENT_WRITE => {
            let input = get_input_validated!(== 64);
            assert_halt!(!is_static, StateChangeDuringStaticCall);
            // Read input.
            let slot = U256::from_le_slice(&input[0..32]);
            let value = U256::from_le_slice(&input[32..64]);
            // Charge gas.
            charge_gas!(ctx.cfg().gas_params().warm_storage_read_cost());
            ctx.journal_mut()
                .tstore(current_target_address, slot, value);
            // Empty result.
            return_result!(Bytes::new(), Ok);
        }

        SYSCALL_ID_BLOCK_HASH => {
            let input = get_input_validated!(== 8);
            charge_gas!(gas::BLOCKHASH);
            let requested_block = LittleEndian::read_u64(&input[0..8]);
            let current_block = ctx.block_number().as_limbs()[0];
            // TODO: Why do we return in big-endian here? :facepalm:
            let hash = match current_block.checked_sub(requested_block) {
                Some(diff) if diff > 0 && diff <= 256 => {
                    ctx.db_mut().block_hash(requested_block)?
                }
                _ => B256::ZERO,
            };
            return_result!(hash, Ok);
        }

        _ => return_halt!(MalformedBuiltinParams),
    }
}
