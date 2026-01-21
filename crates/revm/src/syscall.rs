use crate::{
    api::RwasmFrame,
    instruction_result_from_exit_code,
    types::{SystemInterruptionInputs, SystemInterruptionOutcome},
    ExecutionResult, NextAction,
};
use fluentbase_evm::gas::{COLD_SLOAD_COST_ADDITIONAL};
use fluentbase_evm::EthereumMetadata;
use fluentbase_runtime::{default_runtime_executor, RuntimeExecutor};
use fluentbase_sdk::{
    byteorder::{ByteOrder, LittleEndian, ReadBytesExt},
    bytes::Buf,
    calc_create_metadata_address, is_execute_using_system_runtime, is_system_precompile, Address,
    Bytes, ExitCode, Log, LogData, B256, FUEL_DENOM_RATE, KECCAK_EMPTY, PRECOMPILE_EVM_RUNTIME,
    STATE_MAIN, U256,
};
use revm::context::journaled_state::JournalLoadError;
use revm::{
    bytecode::{opcode, ownable_account::OwnableAccountBytecode, Bytecode},
    context::{Cfg, ContextError, ContextTr, CreateScheme, JournalTr},
    interpreter::{
        gas,
        gas::{sload_cost, sstore_cost, sstore_refund, warm_cold_cost},
        interpreter_types::InputsTr,
        CallInput, CallInputs, CallScheme, CallValue, CreateInputs, FrameInput, Gas, Host,
    },
    primitives::{
        eip3860::MAX_INITCODE_SIZE,
        hardfork::{SpecId, BERLIN, ISTANBUL, TANGERINE},
        wasm::{wasm_max_code_size, WASM_MAX_CODE_SIZE},
    },
    Database, Inspector,
};
use rwasm::TrapCode;
use std::{boxed::Box, vec, vec::Vec};
use revm::interpreter::StateLoad;

pub(crate) trait MemoryReaderTr {
    fn memory_read(&self, call_id: u32, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode>;
}

pub(crate) struct DefaultRuntimeExecutorMemoryReader;
#[cfg(test)]
pub(crate) struct ForwardInputMemoryReader(Bytes);

impl MemoryReaderTr for DefaultRuntimeExecutorMemoryReader {
    fn memory_read(&self, call_id: u32, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        default_runtime_executor().memory_read(call_id, offset, buffer)
    }
}
#[cfg(test)]
impl MemoryReaderTr for ForwardInputMemoryReader {
    fn memory_read(&self, _call_id: u32, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        self.0
            .get(offset..offset + buffer.len())
            .ok_or(TrapCode::MemoryOutOfBounds)?
            .copy_to_slice(buffer);
        Ok(())
    }
}

#[tracing::instrument(level = "info", skip_all)]
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
    let is_system_runtime = account_owner_address
        .filter(is_execute_using_system_runtime)
        .is_some();

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
            // For system runtime contracts, we always forward execution result to synchronize
            // frames, otherwise it can cause memory corruption
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

    use fluentbase_sdk::syscall::*;
    match inputs.syscall_params.code_hash {
        SYSCALL_ID_STORAGE_READ => {
            let input = get_input_validated!(== 32);
            let slot = U256::from_le_slice(&input[0..32]);
            let skip_cold = frame.interpreter.gas.remaining() < COLD_SLOAD_COST_ADDITIONAL;
            let value = ctx.journal_mut()
                .sload_skip_cold_load(current_target_address, slot, skip_cold);
            let value = match value {
                Ok(v) => { v }
                Err(e) => {
                    match e {
                        JournalLoadError::ColdLoadSkipped => return_halt!(OutOfFuel),
                        JournalLoadError::DBError(_) => return_halt!(Err),
                    }
                }
            };
            charge_gas!(sload_cost(spec_id, value.is_cold));
            inspect!(opcode::SLOAD, [slot], [value.data]);
            let output: [u8; 32] = value.to_le_bytes();
            return_result!(output, Ok)
        }

        SYSCALL_ID_STORAGE_WRITE => {
            assert_halt!(!is_static, StateChangeDuringStaticCall);
            let input = get_input_validated!(== 64);
            let slot = U256::from_le_slice(&input[0..32]);
            let new_value = U256::from_le_slice(&input[32..64]);
            let skip_cold = frame.interpreter.gas.remaining() < COLD_SLOAD_COST_ADDITIONAL;
            let res = ctx
                .journal_mut()
                .sstore_skip_cold_load(current_target_address, slot, new_value, skip_cold);
            let value = match res {
                Ok(v) => v,
                Err(e) => {
                    match e {
                        JournalLoadError::ColdLoadSkipped => return_halt!(OutOfFuel),
                        JournalLoadError::DBError(_) => return_halt!(Err),
                    }
                }
            };
            assert_halt!(
                frame.interpreter.gas.remaining() > gas::CALL_STIPEND,
                OutOfFuel
            );
            let gas_cost = sstore_cost(spec_id.clone(), &value.data, value.is_cold);
            charge_gas!(gas_cost);
            frame
                .interpreter
                .gas
                .record_refund(sstore_refund(spec_id, &value.data));
            inspect!(opcode::SSTORE, [slot, new_value], []);
            return_result!(Ok)
        }

        SYSCALL_ID_CALL => {
            let (input, lazy_contract_input) = get_input_validated!(>= 20 + 32);
            let target_address = Address::from_slice(&input[0..20]);
            let value = U256::from_le_slice(&input[20..52]);
            // for static calls with value greater than 0 - revert
            let has_transfer = !value.is_zero();
            if is_static && has_transfer {
                return_halt!(StateChangeDuringStaticCall);
            }
            if frame.interpreter.gas.remaining() < gas::calc_call_static_gas(spec_id, has_transfer) {
                return_halt!(OutOfFuel)
            }
            let mut account_load = ctx.journal_mut().load_account_delegated(target_address)?;
            // In EVM, there exists an issue with precompiled contracts.
            // These contracts are preloaded and initially empty.
            // However, a precompiled contract can also be explicitly added
            // inside the genesis file, which affects its state and the gas
            // price for the CALL opcode.
            //
            // Using the CALL opcode to invoke a precompiled contract typically
            // has no practical use, as the contract is stateless.
            // Despite this, there are unit tests that require this condition
            // to be supported.
            //
            // While addressing this, improves compatibility with the EVM,
            // it also breaks several unit tests.
            // Nevertheless, the added compatibility is deemed to outweigh these issues.
            if is_system_precompile(&target_address) {
                account_load.is_empty = true;
            }
            // charge_gas!(gas::calc_call_static_gas(spec_id, has_transfer));
            // EIP-150: gas cost changes for IO-heavy operations
            charge_gas!(gas::call_cost(spec_id, has_transfer, account_load));
            let mut gas_limit = core::cmp::min(
                frame.interpreter.gas.remaining_63_of_64_parts(),
                inputs.syscall_params.fuel_limit / FUEL_DENOM_RATE,
            );
            charge_gas!(gas_limit);
            if has_transfer {
                gas_limit = gas_limit.saturating_add(gas::CALL_STIPEND);
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
            // Read contract inputs after all gas charging
            let Ok(contract_input) = lazy_contract_input() else {
                return_halt!(MemoryOutOfBounds);
            };
            // Create call inputs
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
            const TRANSFERS_VALUE: bool = false;
            if frame.interpreter.gas.remaining() < gas::calc_call_static_gas(spec_id, TRANSFERS_VALUE) {
                return_halt!(OutOfFuel)
            }
            let mut account_load = ctx.journal_mut().load_account_delegated(target_address)?;
            // set is_empty to false as we are not creating this account.
            account_load.is_empty = false;
            // EIP-150: gas cost changes for IO-heavy operations
            charge_gas!(gas::call_cost(spec_id.clone(), TRANSFERS_VALUE, account_load));
            let gas_limit = if spec_id.is_enabled_in(TANGERINE) {
                core::cmp::min(
                    frame.interpreter.gas.remaining_63_of_64_parts(),
                    inputs.syscall_params.fuel_limit / FUEL_DENOM_RATE,
                )
            } else {
                inputs.syscall_params.fuel_limit / FUEL_DENOM_RATE
            };
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
            // Read contract inputs after all gas charging
            let Ok(contract_input) = lazy_contract_input() else {
                return_halt!(MemoryOutOfBounds);
            };
            // Create call inputs
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
            let has_transfer = !value.is_zero();
            if frame.interpreter.gas.remaining() < gas::calc_call_static_gas(spec_id, has_transfer) {
                return_halt!(OutOfFuel)
            }
            let mut account_load = ctx.journal_mut().load_account_delegated(target_address)?;
            // set is_empty to false as we are not creating this account
            account_load.is_empty = false;
            // EIP-150: gas cost changes for IO-heavy operations
            charge_gas!(gas::call_cost(spec_id, has_transfer, account_load));
            let mut gas_limit = if spec_id.is_enabled_in(TANGERINE) {
                core::cmp::min(
                    frame.interpreter.gas.remaining_63_of_64_parts(),
                    inputs.syscall_params.fuel_limit / FUEL_DENOM_RATE,
                )
            } else {
                inputs.syscall_params.fuel_limit / FUEL_DENOM_RATE
            };
            charge_gas!(gas_limit);
            // add call stipend if there is a value to be transferred
            if !value.is_zero() {
                gas_limit = gas_limit.saturating_add(gas::CALL_STIPEND);
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
            // Read contract inputs after all gas charging
            let Ok(contract_input) = lazy_contract_input() else {
                return_halt!(MemoryOutOfBounds);
            };
            // Create call inputs
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
            const TRANSFERS_VALUE: bool = false;
            if frame.interpreter.gas.remaining() < gas::calc_call_static_gas(spec_id, TRANSFERS_VALUE) {
                return_halt!(OutOfFuel)
            }
            let mut account_load = ctx.journal_mut().load_account_delegated(target_address)?;
            // set is_empty to false as we are not creating this account.
            account_load.is_empty = false;
            // EIP-150: gas cost changes for IO-heavy operations
            charge_gas!(gas::call_cost(spec_id, TRANSFERS_VALUE, account_load));
            let gas_limit = if spec_id.is_enabled_in(TANGERINE) {
                core::cmp::min(
                    frame.interpreter.gas.remaining_63_of_64_parts(),
                    inputs.syscall_params.fuel_limit / FUEL_DENOM_RATE,
                )
            } else {
                inputs.syscall_params.fuel_limit / FUEL_DENOM_RATE
            };
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
            // Read contract inputs after all gas charging
            let Ok(contract_input) = lazy_contract_input() else {
                return_halt!(MemoryOutOfBounds);
            };
            // Create call inputs
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

            // Make sure input doesn't exceed hard cap at least
            const HARD_CAP: usize = WASM_MAX_CODE_SIZE + U256::BYTES + U256::BYTES;
            assert_halt!(
                inputs.syscall_params.input.len() <= HARD_CAP,
                MalformedBuiltinParams
            );

            // We have different derivation scheme and gas calculation for CREATE2
            let is_create2 = inputs.syscall_params.code_hash == SYSCALL_ID_CREATE2;

            let (input, lazy_init_code) = get_input_validated!(>= if is_create2 {
                U256::BYTES + U256::BYTES
            } else {
                U256::BYTES
            });

            // Make sure we have enough bytes inside input params
            let (scheme, value) = if is_create2 {
                let value = U256::from_le_slice(&input[0..32]);
                let salt = U256::from_le_slice(&input[32..64]);
                (CreateScheme::Create2 { salt }, value)
            } else {
                let value = U256::from_le_slice(&input[0..32]);
                (CreateScheme::Create, value)
            };

            // Make sure we don't exceed max possible init code
            let init_code_length = inputs.syscall_params.input.len() - input.len();
            if init_code_length > 0 {
                charge_gas!(gas::initcode_cost(init_code_length));
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
                let Some(gas) = gas::create2_cost(init_code_length) else {
                    return_halt!(OutOfFuel);
                };
                charge_gas!(gas);
            } else {
                charge_gas!(gas::CREATE);
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

            let create_inputs = Box::new(CreateInputs {
                caller: current_target_address,
                scheme,
                value,
                init_code: init_code.into(),
                gas_limit,
            });
            return_frame!(NextAction::NewFrame(FrameInput::Create(create_inputs)));
        }

        SYSCALL_ID_EMIT_LOG => {
            assert_halt!(!is_static, StateChangeDuringStaticCall);
            // Read the number of topics from the input and make sure the total numbers of
            // topics don't exceed 4 elements
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
            // Read topics from the input w/o data to make sure gas is charged for data len
            // before it's read from the memory (to avoid extra memory allocation)
            let mut topics = Vec::with_capacity(topics_len);
            let (input, lazy_data_input) = get_input_validated!(>= 1 + topics_len * U256::BYTES);
            for i in 0..topics_len {
                let offset = 1 + i * B256::len_bytes();
                let topic = &input[offset..(offset + B256::len_bytes())];
                topics.push(B256::from_slice(topic));
            }
            // Charge the gas based on the number of topics and remaining data length, we subtract
            // topics and 1 (for topics length)
            let data_length = inputs.syscall_params.input.len() - 1 - topics_len * U256::BYTES;
            // Make sure we have enough gas before reading data input, otherwise a ddos attack can
            // be applied
            let Some(gas_cost) = gas::log_cost(topics_len as u8, data_length as u64) else {
                return_halt!(OutOfFuel);
            };
            charge_gas!(gas_cost);
            // all remaining bytes are data
            let Ok(data) = lazy_data_input() else {
                return_halt!(MemoryOutOfBounds);
            };
            match topics_len {
                0 => inspect!(opcode::LOG0, [U256::ZERO, U256::ZERO], []),
                1 => inspect!(opcode::LOG1, [U256::ZERO, U256::ZERO, topics[0].into()], []),
                2 => inspect!(
                    opcode::LOG2,
                    [U256::ZERO, U256::ZERO, topics[0].into(), topics[1].into()],
                    []
                ),
                3 => inspect!(
                    opcode::LOG3,
                    [
                        U256::ZERO,
                        U256::ZERO,
                        topics[0].into(),
                        topics[1].into(),
                        topics[2].into()
                    ],
                    []
                ),
                4 => inspect!(
                    opcode::LOG4,
                    [
                        U256::ZERO,
                        U256::ZERO,
                        topics[0].into(),
                        topics[1].into(),
                        topics[2].into(),
                        topics[3].into()
                    ],
                    []
                ),
                _ => unreachable!(),
            }
            ctx.journal_mut().log(Log {
                address: current_target_address,
                // SAFETY: It's safe to go unchecked here because we do topic check upper
                data: LogData::new_unchecked(topics, data.into()),
            });
            return_result!(Ok);
        }

        SYSCALL_ID_DESTROY_ACCOUNT => {
            let input = get_input_validated!(== 20);
            // not allowed for static calls
            assert_halt!(!is_static, StateChangeDuringStaticCall);
            // destroy an account
            let target = Address::from_slice(&input[0..20]);
            let skip_cold = frame.interpreter.gas.remaining() < gas::COLD_ACCOUNT_ACCESS_COST_ADDITIONAL + gas::WARM_STORAGE_READ_COST;
            let mut result = ctx
                .journal_mut()
                .selfdestruct(current_target_address, target, skip_cold);
            let mut result = match result {
                Ok(v) => v,
                Err(e) => {
                    match e {
                        JournalLoadError::ColdLoadSkipped => return_halt!(OutOfFuel),
                        JournalLoadError::DBError(_) => return_halt!(Err),
                    }
                }
            };
            // system precompiles are always empty...
            if result.data.target_exists && is_system_precompile(&target) {
                result.data.target_exists = false;
            }
            // charge gas cost
            charge_gas!(gas::selfdestruct_cost(spec_id, result));
            // return value as bytes with success exit code
            return_result!(Ok);
        }

        SYSCALL_ID_BALANCE => {
            let input = get_input_validated!(== 20);
            let address = Address::from_slice(&input[0..20]);
            let skip_cold = frame.interpreter.gas.remaining() < gas::COLD_ACCOUNT_ACCESS_COST_ADDITIONAL;
            // Load an account with the bytecode
            let res = ctx.journal_mut()
                .load_account_info_skip_cold_load(
                    address,
                    false,
                    skip_cold
                );
            let account_info = match res {
                Ok(v) => v,
                Err(e) => {
                    match e {
                        JournalLoadError::ColdLoadSkipped => return_halt!(OutOfFuel),
                        JournalLoadError::DBError(_) => return_halt!(Err),
                    }
                }
            };
            let balance_load = StateLoad::new(account_info.balance, account_info.is_cold);
            // make sure we have enough gas for this op
            charge_gas!(if spec_id.is_enabled_in(BERLIN) {
                warm_cold_cost(balance_load.is_cold)
            } else if spec_id.is_enabled_in(ISTANBUL) {
                700
            } else if spec_id.is_enabled_in(TANGERINE) {
                400
            } else {
                20
            });
            // write the result
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
            let skip_cold = frame.interpreter.gas.remaining() < gas::COLD_ACCOUNT_ACCESS_COST_ADDITIONAL;
            // Load an account with the bytecode
            let res = ctx.journal_mut()
                .load_account_info_skip_cold_load(
                    address,
                    true,
                    skip_cold
                );
            let account_info = match res {
                Ok(v) => v,
                Err(e) => {
                    match e {
                        JournalLoadError::ColdLoadSkipped => return_halt!(OutOfFuel),
                        JournalLoadError::DBError(_) => return_halt!(Err),
                    }
                }
            };
            charge_gas!(warm_cold_cost(account_info.is_cold));

            // A special case for precompiled runtimes, where the way of extracting bytecode might be different.
            // We keep this condition here and moved away from the runtime because Rust applications
            // might also request EVM bytecode and initiating extra interruptions to fetch the data might be redundant.
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
            // according to EVM requirements, we should return empty code
            if is_system_precompile(&address) {
                code_len = 0;
            }

            // Code size we encode as 32-bytes in LE encoding,
            // there is no need to return it as 32-bytes array, but it's more EVM friendly
            let code_size = U256::from(code_len);
            return_result!(code_size.to_le_bytes::<32>(), Ok);
        }

        SYSCALL_ID_CODE_HASH => {
            let input = get_input_validated!(== 20);
            let address = Address::from_slice(&input[0..20]);
            let skip_cold = frame.interpreter.gas.remaining() < gas::COLD_ACCOUNT_ACCESS_COST_ADDITIONAL;
            // Load an account with the bytecode
            let res = ctx.journal_mut()
                .load_account_info_skip_cold_load(
                    address,
                    false,
                    skip_cold
                );
            let account_info = match res {
                Ok(v) => v,
                Err(e) => {
                    match e {
                        JournalLoadError::ColdLoadSkipped => return_halt!(OutOfFuel),
                        JournalLoadError::DBError(_) => return_halt!(Err),
                    }
                }
            };
            charge_gas!(warm_cold_cost(account_info.is_cold));

            // Extract code hash for an account for delegated account.
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
                // this is a requirement from EVM
                _ if account_info.is_empty() => B256::ZERO,
                _ => account_info.code_hash,
            };

            if is_system_precompile(&address) {
                // We store system precompile bytecode in the state trie,
                // according to evm requirements, we should return empty code
                code_hash = B256::ZERO;
            } else if code_hash == B256::ZERO && !account_info.is_empty() {
                // If the delegated code hash is zero, then it might be a contract deployment stage,
                // for non-empty account return KECCAK_EMPTY
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
            let skip_cold = frame.interpreter.gas.remaining() < gas::COLD_ACCOUNT_ACCESS_COST_ADDITIONAL;
            // Load an account with the bytecode
            let res = ctx.journal_mut()
                .load_account_info_skip_cold_load(
                    address,
                    true,
                    skip_cold
                );
            let account_info = match res {
                Ok(v) => v,
                Err(e) => {
                    match e {
                        JournalLoadError::ColdLoadSkipped => return_halt!(OutOfFuel),
                        JournalLoadError::DBError(_) => return_halt!(Err),
                    }
                }
            };

            // CRITICAL: Gas is charged for REQUESTED length, not actual returned length
            // This prevents gas abuse where attacker requests small length but expects full bytecode
            let Some(gas_cost) =
                gas::extcodecopy_cost(spec_id, code_length as usize, account_info.is_cold)
            else {
                return_halt!(OutOfFuel);
            };
            charge_gas!(gas_cost);

            // Early return for zero-length request
            if code_length == 0 {
                return_result!(Bytes::new(), Ok);
            }

            // Load bytecode from account
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

            // System precompiles return empty code per EVM requirements
            if is_system_precompile(&address) {
                bytecode = Bytes::new();
            }

            let bytecode_len = bytecode.len();
            let code_offset_usize = code_offset as usize;
            let code_length_usize = code_length as usize;

            // If offset is beyond bytecode, return all zeros
            if code_offset_usize >= bytecode_len {
                let mut zeros = Vec::with_capacity(code_length_usize);
                zeros.resize(code_length_usize, 0u8);
                return_result!(Bytes::from(zeros), Ok);
            }

            let start = code_offset_usize;
            let available = bytecode_len - start;
            let to_copy = core::cmp::min(code_length_usize, available);

            // Fast path: If no padding needed, return zero-copy slice
            if to_copy == code_length_usize {
                let result = bytecode.slice(start..start + to_copy);
                return_result!(result, Ok);
            }

            // Slow path: Padding required to reach the requested length
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
            // read an account from its address
            let address = Address::from_slice(&input[..20]);
            let account = ctx.journal_mut().load_account_with_code(address)?;
            // to make sure this account is ownable and owner by the same runtime, that allows
            // a runtime to modify any account it owns
            let Some(ownable_account_bytecode) = (match account.info.code.as_ref() {
                Some(Bytecode::OwnableAccount(ownable_account_bytecode)) => {
                    // if an account is not the same - it's not a malformed building param, runtime might not know it's account
                    if ownable_account_bytecode.owner_address == account_owner_address {
                        Some(ownable_account_bytecode)
                    } else {
                        None
                    }
                }
                _ => None,
            }) else {
                let output = Bytes::from([
                    // metadata length is 0 in this case
                    0x00,
                    0x00,
                    0x00,
                    0x00,
                    // pass info about an account (is_account_ownable, is_cold, is_empty)
                    0x00u8,
                    account.is_cold as u8,
                    account.is_empty() as u8,
                ]);
                return_result!(output, Ok);
            };
            // execute a syscall
            let mut output = [0u8; 4 + 3];
            LittleEndian::write_u32(&mut output, ownable_account_bytecode.metadata.len() as u32);
            output[4] = 0x01u8; // the account belongs to the same runtime
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
            // Verify no deployment collision exists at derived address.
            // Check only code_hash and nonce - intentionally ignore balance to prevent
            // front-running DoS where attacker funds address before legitimate creation.
            // This matches Ethereum CREATE2 behavior: accounts can be pre-funded.
            if account.info.code_hash != KECCAK_EMPTY || account.info.nonce != 0 {
                return_result!(CreateContractCollision);
            }
            // create a new derived ownable account
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
            // read an account from its address
            let address = Address::from_slice(&input[..20]);
            let _offset = LittleEndian::read_u32(&input[20..24]) as usize;
            let account = ctx.journal_mut().load_account_with_code(address)?;
            // to make sure this account is ownable and owner by the same runtime, that allows
            // a runtime to modify any account it owns
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
            // code might change, rewrite it with a new hash
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
            // read an account from its address
            let address = Address::from_slice(&input[..20]);
            let account = ctx.journal_mut().load_account_with_code(address)?;
            // to make sure this account is ownable and owner by the same runtime, that allows
            // a runtime to modify any account it owns
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
            // take min
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
            // input: slot + value
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
            // charge gas
            charge_gas!(gas::WARM_STORAGE_READ_COST);
            // read value from storage
            let slot = U256::from_le_slice(&input[0..32].as_ref());
            let value = ctx.journal_mut().tload(current_target_address, slot);
            // return value
            let output: [u8; 32] = value.to_le_bytes();
            return_result!(output, Ok);
        }

        SYSCALL_ID_TRANSIENT_WRITE => {
            let input = get_input_validated!(== 64);
            assert_halt!(!is_static, StateChangeDuringStaticCall);
            // read input
            let slot = U256::from_le_slice(&input[0..32]);
            let value = U256::from_le_slice(&input[32..64]);
            // charge gas
            charge_gas!(gas::WARM_STORAGE_READ_COST);
            ctx.journal_mut()
                .tstore(current_target_address, slot, value);
            // empty result
            return_result!(Bytes::new(), Ok);
        }
        //
        SYSCALL_ID_BLOCK_HASH => {
            let input = get_input_validated!(== 8);
            charge_gas!(gas::BLOCKHASH);
            let requested_block = LittleEndian::read_u64(&input[0..8]);
            let current_block = ctx.block_number().as_limbs()[0];
            // Why do we return in big-endian here? :facepalm:
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

#[cfg(test)]
mod code_copy_tests {
    use super::*;
    use crate::{RwasmContext, RwasmSpecId};
    use fluentbase_sdk::{syscall::SYSCALL_ID_CODE_COPY, SyscallInvocationParams, STATE_MAIN};
    use revm::{
        context::{BlockEnv, CfgEnv, TxEnv},
        database::InMemoryDB,
        inspector::NoOpInspector,
        primitives::{Address, Bytes},
    };

    /// Helper function to test code_copy syscall
    /// Returns (output_data, gas_used)
    fn test_code_copy_helper(
        bytecode: Bytes,
        code_offset: u64,
        code_length: u64,
        initial_gas: u64,
    ) -> (Bytes, u64) {
        // === Setup: Initialize context and database ===
        let db = InMemoryDB::default();
        let mut ctx: RwasmContext<InMemoryDB> = RwasmContext::new(db, RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();

        let mut frame = RwasmFrame::default();

        // === Setup: Create target account with known bytecode ===
        let target_address = Address::from([0x42; 20]);

        {
            let mut account = ctx
                .journal_mut()
                .load_account_with_code_mut(target_address)
                .unwrap();
            account.set_code_and_hash_slow(Bytecode::new_raw(bytecode.clone()));
            account.set_balance(U256::ONE); // Non-empty account
        }

        // === Prepare syscall input ===
        let mut syscall_input = vec![0u8; 20 + 8 + 8];
        syscall_input[0..20].copy_from_slice(target_address.as_slice());
        syscall_input[20..28].copy_from_slice(&code_offset.to_le_bytes());
        syscall_input[28..36].copy_from_slice(&code_length.to_le_bytes());
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let syscall_params = SyscallInvocationParams {
            code_hash: SYSCALL_ID_CODE_COPY,
            input: 0..mr.0.len(),
            state: STATE_MAIN,
            ..Default::default()
        };

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params,
            gas: Gas::new(initial_gas),
        };

        // === Execute: Call the syscall ===
        let result = execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        );

        assert!(
            result.is_ok(),
            "Syscall execution failed: {:?}",
            result.err()
        );

        // === Extract results ===
        let returned_data = frame.interrupted_outcome.as_ref().unwrap();
        let output_data = returned_data.result.as_ref().unwrap().output.clone();

        // Get gas_used from the interruption result directly
        let gas_used = returned_data.result.as_ref().unwrap().gas.spent();

        (output_data, gas_used)
    }

    fn expected_gas(length: usize, is_cold: bool) -> u64 {
        // Base cost depends on warm/cold access
        // After EIP-2929 (BERLIN fork):
        // - Cold access: 2600 gas (first access to account)
        // - Warm access: 100 gas (subsequent accesses)
        let base_gas = if is_cold {
            2600 // COLD_ACCOUNT_ACCESS_COST
        } else {
            100 // WARM_STORAGE_READ_COST
        };

        // Copy cost: 3 gas per 32-byte word
        // Formula: 3 * ceil(length / 32)
        let words = (length + 31) / 32; // ceil(length / 32)
        let copy_gas = words as u64 * 3;

        base_gas + copy_gas
    }

    #[test]
    fn test_code_copy_basic_slice() {
        // Test: Basic slice from middle of bytecode
        let bytecode = Bytes::from(vec![
            0x60, 0x80, 0x60, 0x40, 0x52, 0x33, 0x90, 0x81, 0x01, 0x02,
        ]); // 10 bytes

        let (output, gas) = test_code_copy_helper(bytecode.clone(), 3, 4, 10_000_000);

        assert_eq!(output.len(), 4, "Should return 4 bytes");
        assert_eq!(
            &output[..],
            &bytecode[3..7],
            "Should return bytes at offset 3-6"
        );
        assert_eq!(gas, expected_gas(4, false));
    }

    #[test]
    fn test_code_copy_request_more_than_available() {
        // Test: Request more bytes than available (no padding in WASM)
        let bytecode = Bytes::from(vec![0xAA, 0xBB, 0xCC]); // Only 3 bytes

        let (output, gas) = test_code_copy_helper(bytecode.clone(), 1, 10, 10_000_000);

        assert_eq!(output.len(), 10, "Should return only 2 available bytes");
        assert_eq!(
            &output[..],
            &[0xBB, 0xCC, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(gas, expected_gas(10, false));
    }

    #[test]
    fn test_code_copy_offset_beyond_bytecode() {
        // Test: Offset completely beyond bytecode length
        let bytecode = Bytes::from(vec![0xAA, 0xBB, 0xCC]); // 3 bytes

        let (output, gas) = test_code_copy_helper(bytecode.clone(), 100, 5, 10_000_000);
        // Should return empty bytes
        assert_eq!(
            output.len(),
            5,
            "Should return empty bytes when offset > bytecode.len()"
        );
        assert_eq!(gas, expected_gas(5, false));
    }

    #[test]
    fn test_code_copy_empty_bytecode() {
        // Test: Copy from account with empty/no bytecode
        let bytecode = Bytes::new();

        let (output, gas) = test_code_copy_helper(bytecode.clone(), 0, 10, 10_000_000);

        assert_eq!(
            output.len(),
            10,
            "Should return empty bytes for empty bytecode"
        );
        assert_eq!(
            &output[..],
            &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );

        assert_eq!(gas, expected_gas(10, false));
    }

    #[test]
    fn test_code_copy_gas_calculation() {
        // Test: Verify gas is calculated correctly according to EVM rules
        let bytecode = Bytes::from(vec![0xFF; 200]); // 100 bytes

        let (output, gas) = test_code_copy_helper(bytecode.clone(), 0, 200, 10_000_000);

        assert_eq!(output.len(), 200);
        assert_eq!(output[..], bytecode);
        assert_eq!(gas, expected_gas(200, false));
    }
}

#[cfg(test)]
mod metadata_write_tests {
    use super::*;
    use crate::{RwasmContext, RwasmSpecId};
    use fluentbase_sdk::{
        syscall::{SYSCALL_ID_METADATA_CREATE, SYSCALL_ID_METADATA_WRITE},
        SyscallInvocationParams, PRECOMPILE_WASM_RUNTIME,
    };
    use revm::{
        bytecode::{ownable_account::OwnableAccountBytecode, Bytecode},
        context::{BlockEnv, CfgEnv, TxEnv},
        database::InMemoryDB,
        inspector::NoOpInspector,
    };

    #[test]
    fn test_metadata_write_truncates_existing_data() {
        // === Setup: Initialize context and database ===
        let db = InMemoryDB::default();
        let mut ctx: RwasmContext<InMemoryDB> = RwasmContext::new(db, RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();

        // === Setup: Create frame with owner address ===
        let owner_address = PRECOMPILE_WASM_RUNTIME;
        let mut frame = RwasmFrame::default();
        frame.interpreter.input.account_owner = Some(owner_address);

        // === Step 1: Create an account with initial metadata ===
        let test_address = Address::from_slice(&[0x42; 20]);
        let initial_metadata = Bytes::from(&[0xFF; 100]);

        let _ = ctx.journal_mut().load_account(test_address);

        // Pre-create the ownable account with initial metadata
        ctx.journal_mut().set_code(
            test_address,
            Bytecode::OwnableAccount(OwnableAccountBytecode::new(
                owner_address,
                initial_metadata.clone(),
            )),
        );

        let new_data = vec![0x11, 0x22, 0x33, 0x44];
        let offset = 2u32;

        // Prepare syscall input: address (20 bytes) + offset (4 bytes) + data
        let mut syscall_input = Vec::new();
        syscall_input.extend_from_slice(&test_address.0[..]);
        syscall_input.extend_from_slice(&offset.to_le_bytes());
        syscall_input.extend_from_slice(&new_data);
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let syscall_params = SyscallInvocationParams {
            code_hash: SYSCALL_ID_METADATA_WRITE,
            input: 0..mr.0.len(),
            state: STATE_MAIN,
            fuel_limit: 1_000_000,
            ..Default::default()
        };

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params,
            gas: Gas::new(1_000_000),
        };

        let result = execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        );

        assert!(
            result.is_ok(),
            "Syscall execution failed: {:?}",
            result.err()
        );

        let acc = ctx
            .journal_mut()
            .load_account_with_code(test_address)
            .unwrap();
        match &acc.info.code {
            Some(Bytecode::OwnableAccount(ownable)) => {
                assert_eq!(ownable.metadata[..], new_data);
            }
            _ => panic!("Expected OwnableAccount bytecode"),
        }
    }

    #[test]
    fn test_metadata_create() {
        // === Setup: Initialize context and database ===
        let db = InMemoryDB::default();
        let mut ctx: RwasmContext<InMemoryDB> = RwasmContext::new(db, RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();

        // === Setup: Create frame with owner address ===
        let owner_address = PRECOMPILE_WASM_RUNTIME;
        let mut frame = RwasmFrame::default();
        frame.interpreter.input.account_owner = Some(owner_address);

        // === Prepare: Calculate derived address and ensure it's not empty ===
        let salt = U256::from(123456789u64);
        let metadata = Bytes::from(vec![0x01, 0x02, 0x03]);

        let derived_address = calc_create_metadata_address(&owner_address, &salt);

        // Preload the account to avoid empty account check
        // (In real scenario, this would be done by previous transactions)
        {
            let mut account = ctx
                .journal_mut()
                .load_account_with_code_mut(derived_address)
                .unwrap();
            account.set_balance(U256::ONE);
        }

        // === Execute: Prepare syscall input (salt + metadata) ===
        let mut syscall_input = Vec::with_capacity(32 + metadata.len());
        syscall_input.extend_from_slice(&salt.to_be_bytes::<32>());
        syscall_input.extend_from_slice(&metadata);
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let syscall_params = SyscallInvocationParams {
            code_hash: SYSCALL_ID_METADATA_CREATE,
            input: 0..mr.0.len(),
            state: STATE_MAIN,
            ..Default::default()
        };

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params,
            gas: Gas::new(1_000_000), // Sufficient gas for the operation
        };

        // === Execute: Call the syscall ===
        let result = execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        );

        assert!(
            result.is_ok(),
            "Syscall execution failed: {:?}",
            result.err()
        );
        assert!(
            matches!(result.unwrap(), NextAction::InterruptionResult),
            "Expected InterruptionResult"
        );

        // === Verify: Check the created account ===
        let created_account = ctx
            .journal_mut()
            .load_account_with_code(derived_address)
            .expect("Failed to load created account");

        match &created_account.info.code {
            Some(Bytecode::OwnableAccount(ownable)) => {
                assert_eq!(
                    ownable.owner_address, owner_address,
                    "Owner address mismatch"
                );
                assert_eq!(ownable.metadata, metadata, "Metadata mismatch");
            }
            other => panic!("Expected OwnableAccount bytecode, got {:?}", other),
        }
    }
}

#[cfg(test)]
mod block_hash_tests {
    use super::*;
    use crate::{RwasmContext, RwasmFrame, RwasmSpecId};
    use alloy_primitives::{address, bytes, Address, StorageValue, B256};
    use core::error::Error;
    use fluentbase_sdk::{
        byteorder::LE,
        syscall::{SYSCALL_ID_BLOCK_HASH, SYSCALL_ID_METADATA_COPY, SYSCALL_ID_METADATA_WRITE},
        Bytes, SyscallInvocationParams, STATE_MAIN, U256,
    };
    use revm::{
        bytecode::Bytecode,
        context::{BlockEnv, CfgEnv, TxEnv},
        database::{DBErrorMarker, InMemoryDB},
        inspector::NoOpInspector,
        interpreter::{Gas, InstructionResult},
        state::AccountInfo,
        Database,
    };
    use std::fmt;

    #[derive(Debug, Clone)]
    pub(super) struct MockDbError(pub String);

    impl fmt::Display for MockDbError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl Error for MockDbError {}
    impl DBErrorMarker for MockDbError {}

    struct FailingMockDatabase;

    impl Database for FailingMockDatabase {
        type Error = MockDbError;

        fn basic(&mut self, _address: Address) -> Result<Option<AccountInfo>, Self::Error> {
            Ok(None)
        }

        fn code_by_hash(&mut self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
            Ok(Bytecode::default())
        }

        fn storage(
            &mut self,
            _address: Address,
            _index: U256,
        ) -> Result<StorageValue, Self::Error> {
            Ok(StorageValue::default())
        }

        fn block_hash(&mut self, _number: u64) -> Result<B256, Self::Error> {
            Err(MockDbError("Database I/O error".to_string()))
        }
    }

    /// Helper function to test block_hash syscall
    fn test_block_hash_helper<DB: Database>(
        db: DB,
        current_block: u64,
        requested_block: u64,
    ) -> Result<Bytes, ContextError<DB::Error>> {
        let mut ctx: RwasmContext<DB> = RwasmContext::new(db, RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();
        ctx.block.number = U256::from(current_block);

        let mut frame = RwasmFrame::default();

        let mut syscall_input = vec![0u8; 8];
        syscall_input[0..8].copy_from_slice(&requested_block.to_le_bytes());
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let syscall_params = SyscallInvocationParams {
            code_hash: SYSCALL_ID_BLOCK_HASH,
            input: 0..mr.0.len(),
            state: STATE_MAIN,
            ..Default::default()
        };

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params,
            gas: Gas::new(10_000_000),
        };

        execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        )?;

        Ok(frame
            .interrupted_outcome
            .as_ref()
            .unwrap()
            .result
            .as_ref()
            .unwrap()
            .output
            .clone())
    }

    #[test]
    fn test_block_hash_database_error_is_propagated() {
        // This test verifies that database errors are propagated through execute_rwasm_interruption
        // instead of being silently converted to zero hash

        let db = FailingMockDatabase;
        let mut ctx: RwasmContext<FailingMockDatabase> = RwasmContext::new(db, RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();
        ctx.block.number = U256::from(1000);

        let mut frame = RwasmFrame::default();

        // Request a valid block (within the last 256 blocks)
        let requested_block: u64 = 900;
        let mut syscall_input = vec![0u8; 8];
        syscall_input[0..8].copy_from_slice(&requested_block.to_le_bytes());
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params: SyscallInvocationParams {
                code_hash: SYSCALL_ID_BLOCK_HASH,
                input: 0..mr.0.len(),
                state: STATE_MAIN,
                ..Default::default()
            },
            gas: Gas::new(10_000_000),
        };

        // Execute the syscall - should return Err, not Ok
        let result = execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        );

        // Assert that database error was propagated
        assert!(
            result.is_err(),
            "Database error must be propagated, not hidden"
        );
        println!("result: {:?}", result);
    }

    #[test]
    fn test_block_hash_database_error_propagation() {
        let db = FailingMockDatabase;
        let result = test_block_hash_helper(db, 1000, 900);

        assert!(
            result.is_err(),
            "Expected database error to be propagated, but got Ok"
        );

        let error_msg = format!("{:?}", result.unwrap_err());
        assert!(
            error_msg.contains("Database I/O error") || error_msg.contains("MockDbError"),
            "Error should contain database error message, got: {}",
            error_msg
        );
    }

    #[test]
    fn test_block_hash_out_of_range_returns_zero() {
        let db = InMemoryDB::default();
        let output =
            test_block_hash_helper(db, 1000, 1001).expect("Should succeed for out of range block");

        assert_eq!(output.len(), 32, "Should return 32 bytes");
        assert_eq!(
            &output[..],
            B256::ZERO.as_slice(),
            "Should return zero hash for future block"
        );
    }

    #[test]
    fn test_block_hash_too_old_returns_zero() {
        let db = InMemoryDB::default();
        let output =
            test_block_hash_helper(db, 1000, 743).expect("Should succeed for too old block");

        assert_eq!(
            &output[..],
            B256::ZERO.as_slice(),
            "Should return zero hash for block older than 256"
        );
    }

    #[test]
    fn test_metadata_copy_out_of_bounds() {
        let mut ctx: RwasmContext<InMemoryDB> =
            RwasmContext::new(InMemoryDB::default(), RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();
        let mut frame = RwasmFrame::default();
        frame.interpreter.input.account_owner = Some(Address::ZERO);

        const ADDRESS: Address = address!("1111111111111111111111111111111111111111");
        _ = ctx.load_account_delegated(ADDRESS).unwrap();
        ctx.journal_mut().set_code(
            ADDRESS,
            Bytecode::new_ownable_account(Address::ZERO, bytes!("112233445566")),
        );

        let mut syscall_input = vec![0u8; 28];
        syscall_input[0..20].copy_from_slice(ADDRESS.as_slice());
        LE::write_u32(&mut syscall_input[20..24], 100); // offset
        LE::write_u32(&mut syscall_input[24..28], 0); // length
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params: SyscallInvocationParams {
                code_hash: SYSCALL_ID_METADATA_COPY,
                input: 0..mr.0.len(),
                state: STATE_MAIN,
                ..Default::default()
            },
            gas: Gas::new(10_000_000),
        };
        execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        )
        .unwrap();
        let output = frame
            .interrupted_outcome
            .as_ref()
            .unwrap()
            .result
            .as_ref()
            .unwrap()
            .output
            .clone();
        assert_eq!(output, Bytes::new());
    }

    #[test]
    fn test_metadata_write_static_context() {
        let mut ctx: RwasmContext<InMemoryDB> =
            RwasmContext::new(InMemoryDB::default(), RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();
        let mut frame = RwasmFrame::default();
        frame.interpreter.input.account_owner = Some(Address::ZERO);
        frame.interpreter.runtime_flag.is_static = true;

        const ADDRESS: Address = address!("1111111111111111111111111111111111111111");
        _ = ctx.load_account_delegated(ADDRESS).unwrap();

        let mut syscall_input = vec![0u8; 24];
        syscall_input[0..20].copy_from_slice(ADDRESS.as_slice());
        LE::write_u32(&mut syscall_input[20..24], 0); // _offset
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params: SyscallInvocationParams {
                code_hash: SYSCALL_ID_METADATA_WRITE,
                input: 0..mr.0.len(),
                state: STATE_MAIN,
                ..Default::default()
            },
            gas: Gas::new(10_000_000),
        };
        let result = execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        )
        .unwrap();
        assert_eq!(
            result
                .into_interpreter_action()
                .instruction_result()
                .unwrap(),
            InstructionResult::StateChangeDuringStaticCall
        );
    }
}
