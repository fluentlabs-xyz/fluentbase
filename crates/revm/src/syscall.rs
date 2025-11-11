use crate::{
    api::RwasmFrame,
    instruction_result_from_exit_code,
    types::{SystemInterruptionInputs, SystemInterruptionOutcome},
    ExecutionResult, NextAction,
};
use core::cmp::min;
use fluentbase_evm::EthereumMetadata;
use fluentbase_runtime::{default_runtime_executor, RuntimeExecutor};
use fluentbase_sdk::{
    byteorder::{ByteOrder, LittleEndian, ReadBytesExt},
    bytes::Buf,
    calc_create4_address, is_system_precompile, Address, Bytes, ExitCode, Log, LogData, B256,
    FUEL_DENOM_RATE, KECCAK_EMPTY, PRECOMPILE_EVM_RUNTIME, STATE_MAIN, U256,
};
use revm::{
    bytecode::{opcode, ownable_account::OwnableAccountBytecode, Bytecode},
    context::{Cfg, ContextError, ContextTr, CreateScheme, JournalTr},
    interpreter::{
        gas,
        gas::{sload_cost, sstore_cost, sstore_refund, warm_cold_cost},
        interpreter_types::InputsTr,
        CallInput, CallInputs, CallScheme, CallValue, CreateInputs, FrameInput, Gas, Host,
        MAX_INITCODE_SIZE,
    },
    primitives::{
        hardfork::{SpecId, BERLIN, ISTANBUL, TANGERINE},
        wasm::{wasm_max_code_size, WASM_MAX_CODE_SIZE},
    },
    Database, Inspector,
};
use revm_helpers::reusable_pool::global::VecU8;
use rwasm::TrapCode;
use std::{boxed::Box, vec, vec::Vec};

#[tracing::instrument(level = "info", skip_all)]
pub(crate) fn execute_rwasm_interruption<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    inspector: &mut Option<&mut INSP>,
    ctx: &mut CTX,
    inputs: SystemInterruptionInputs,
) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
    let spec_id: SpecId = ctx.cfg().spec().into();

    let current_target_address = frame.interpreter.input.target_address();
    let account_owner_address = frame.interpreter.input.account_owner_address();

    let is_static = frame.interpreter.runtime_flag.is_static;

    macro_rules! return_result {
        ($output:expr, $result:ident) => {{
            let output: VecU8 = $output.into();
            let result = ExecutionResult {
                result: instruction_result_from_exit_code(ExitCode::$result, output.is_empty()),
                output,
                gas: Gas::new_spent(frame.interpreter.gas.spent() - inputs.gas.spent()),
            };
            frame.insert_interrupted_outcome(SystemInterruptionOutcome {
                inputs: Box::new(inputs),
                result: Some(result),
                is_frame: false,
            });
            return Ok(NextAction::InterruptionResult);
        }};
        ($result:ident) => {{
            return_result!(VecU8::default_for_reuse(), $result)
        }};
    }
    macro_rules! return_halt {
        ($result:ident) => {{
            let result = ExecutionResult {
                result: instruction_result_from_exit_code(ExitCode::$result, true),
                output: VecU8::default_for_reuse(),
                gas: Gas::new_spent(frame.interpreter.gas.spent() - inputs.gas.spent()),
            };
            return Ok(NextAction::Return(result));
        }};
    }
    macro_rules! return_frame {
        ($action:expr) => {{
            frame.insert_interrupted_outcome(SystemInterruptionOutcome {
                inputs: Box::new(inputs),
                result: None,
                is_frame: true,
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
            if default_runtime_executor()
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
            if default_runtime_executor()
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
                default_runtime_executor().memory_read(
                    call_id,
                    remaining_offset,
                    &mut variable_input,
                )?;
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
            let value = ctx.journal_mut().sload(current_target_address, slot)?;
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
            let value = ctx
                .journal_mut()
                .sstore(current_target_address, slot, new_value)?;
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
            // EIP-150: gas cost changes for IO-heavy operations
            charge_gas!(gas::call_cost(spec_id, has_transfer, account_load));
            let mut gas_limit = min(
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
                input: CallInput::Bytes(
                    VecU8::try_from_slice(&contract_input).expect("enough cap"),
                ),
                gas_limit,
                target_address,
                caller: current_target_address,
                bytecode_address: target_address,
                value: CallValue::Transfer(value),
                scheme: CallScheme::Call,
                is_static,
                return_memory_offset: Default::default(),
            });
            return_frame!(NextAction::NewFrame(FrameInput::Call(call_inputs)));
        }

        SYSCALL_ID_STATIC_CALL => {
            let (input, lazy_contract_input) = get_input_validated!(>= 20);
            let target_address = Address::from_slice(&input[0..20]);
            let mut account_load = ctx.journal_mut().load_account_delegated(target_address)?;
            // set is_empty to false as we are not creating this account.
            account_load.is_empty = false;
            // EIP-150: gas cost changes for IO-heavy operations
            charge_gas!(gas::call_cost(spec_id.clone(), false, account_load));
            let gas_limit = if spec_id.is_enabled_in(TANGERINE) {
                min(
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
                input: CallInput::Bytes(
                    VecU8::try_from_slice(&contract_input).expect("enough cap"),
                ),
                gas_limit,
                target_address,
                caller: current_target_address,
                bytecode_address: target_address,
                value: CallValue::Transfer(U256::ZERO),
                scheme: CallScheme::StaticCall,
                is_static: true,
                return_memory_offset: Default::default(),
            });
            return_frame!(NextAction::NewFrame(FrameInput::Call(call_inputs)));
        }

        SYSCALL_ID_CALL_CODE => {
            let (input, lazy_contract_input) = get_input_validated!(>= 20 + 32);
            let target_address = Address::from_slice(&input[0..20]);
            let value = U256::from_le_slice(&input[20..52]);
            let mut account_load = ctx.journal_mut().load_account_delegated(target_address)?;
            // set is_empty to false as we are not creating this account
            account_load.is_empty = false;
            // EIP-150: gas cost changes for IO-heavy operations
            charge_gas!(gas::call_cost(spec_id, !value.is_zero(), account_load));
            let mut gas_limit = if spec_id.is_enabled_in(TANGERINE) {
                min(
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
                input: CallInput::Bytes(
                    VecU8::try_from_slice(&contract_input).expect("enough cap"),
                ),
                gas_limit,
                target_address: current_target_address,
                caller: current_target_address,
                bytecode_address: target_address,
                value: CallValue::Transfer(value),
                scheme: CallScheme::CallCode,
                is_static,
                return_memory_offset: Default::default(),
            });
            return_frame!(NextAction::NewFrame(FrameInput::Call(call_inputs)));
        }

        SYSCALL_ID_DELEGATE_CALL => {
            let (input, lazy_contract_input) = get_input_validated!(>= 20);
            let target_address = Address::from_slice(&input[0..20]);
            let mut account_load = ctx.journal_mut().load_account_delegated(target_address)?;
            // set is_empty to false as we are not creating this account.
            account_load.is_empty = false;
            // EIP-150: gas cost changes for IO-heavy operations
            charge_gas!(gas::call_cost(spec_id, false, account_load));
            let gas_limit = if spec_id.is_enabled_in(TANGERINE) {
                min(
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
                input: CallInput::Bytes(
                    VecU8::try_from_slice(&contract_input).expect("enough cap"),
                ),
                gas_limit,
                target_address: current_target_address,
                caller: frame.interpreter.input.caller_address(),
                bytecode_address: target_address,
                value: CallValue::Apparent(frame.interpreter.input.call_value()),
                scheme: CallScheme::DelegateCall,
                is_static,
                return_memory_offset: Default::default(),
            });
            return_frame!(NextAction::NewFrame(FrameInput::Call(call_inputs)));
        }

        SYSCALL_ID_CREATE | SYSCALL_ID_CREATE2 => {
            assert_halt!(!is_static, StateChangeDuringStaticCall);

            // Make sure input doesn't exceed hard cap at least
            const HARD_CAP: usize = WASM_MAX_CODE_SIZE + U256::BYTES + U256::BYTES;
            assert_halt!(
                inputs.syscall_params.input.len() < HARD_CAP,
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
            let (input, _) = get_input_validated!(>= 1);
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
            let mut result = ctx
                .journal_mut()
                .selfdestruct(current_target_address, target)?;
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
            let value = ctx
                .journal_mut()
                .load_account(address)
                .map(|acc| acc.map(|a| a.info.balance))?;
            // make sure we have enough gas for this op
            charge_gas!(if spec_id.is_enabled_in(BERLIN) {
                warm_cold_cost(value.is_cold)
            } else if spec_id.is_enabled_in(ISTANBUL) {
                700
            } else if spec_id.is_enabled_in(TANGERINE) {
                400
            } else {
                20
            });
            // write the result
            let output: [u8; 32] = value.data.to_le_bytes();
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

            // Load an account with the bytecode
            let account = ctx.journal_mut().load_account_code(address)?;
            charge_gas!(warm_cold_cost(account.is_cold));

            // A special case for precompiled runtimes, where the way of extracting bytecode might be different.
            // We keep this condition here and moved away from the runtime because Rust applications
            // might also request EVM bytecode and initiating extra interruptions to fetch the data might be redundant.
            let mut code_len = match &account.data.info.code {
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

            // Load an account from database
            let account = ctx.journal_mut().load_account_code(address)?;
            charge_gas!(warm_cold_cost(account.is_cold));

            // Extract code hash for an account for delegated account.
            // For EVM, we extract code hash from the metadata to satisfy EVM requirements.
            // It requires the account to be loaded with bytecode.
            let mut code_hash = match &account.data.info.code {
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
                _ if account.is_empty() => B256::ZERO,
                _ => account.info.code_hash,
            };

            if is_system_precompile(&address) {
                // We store system precompile bytecode in the state trie,
                // according to evm requirements, we should return empty code
                code_hash = B256::ZERO;
            } else if code_hash == B256::ZERO && !account.is_empty() {
                // If the delegated code hash is zero, then it might be a contract deployment stage,
                // for non-empty account return KECCAK_EMPTY
                code_hash = KECCAK_EMPTY;
            }

            return_result!(code_hash.0, Ok);
        }

        SYSCALL_ID_CODE_COPY => {
            let input = get_input_validated!(== 20 + 8 * 2);
            let address = Address::from_slice(&input[0..20]);
            let mut reader = input[20..].reader();
            let _code_offset = reader.read_u64::<LittleEndian>().unwrap();
            let code_length = reader.read_u64::<LittleEndian>().unwrap();

            // Load account with bytecode and charge gas for the call
            let account = ctx.journal_mut().load_account_code(address)?;
            let Some(gas_cost) =
                gas::extcodecopy_cost(spec_id, code_length as usize, account.is_cold)
            else {
                return_halt!(OutOfFuel);
            };
            charge_gas!(gas_cost);

            // If requested code length is zero, then there is no need to proceed
            if code_length == 0 {
                return_result!(VecU8::default_for_reuse(), Ok);
            }

            let mut bytecode = match &account.data.info.code {
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
                    .map(|v| VecU8::try_from_slice(&v).expect("enough cap"))
                    .unwrap_or(VecU8::default_for_reuse()),
            };

            // we store system precompile bytecode in the state trie,
            // according to evm requirements, we should return empty code
            if is_system_precompile(&address) {
                bytecode = VecU8::default_for_reuse();
            }

            // TODO(dmitry123): Add offset/length checks, otherwise gas can be abused!
            return_result!(bytecode, Ok);
        }

        SYSCALL_ID_METADATA_SIZE => {
            let Some(account_owner_address) = account_owner_address else {
                return_halt!(MalformedBuiltinParams);
            };
            let input = get_input_validated!(== 20);
            // read an account from its address
            let address = Address::from_slice(&input[..20]);
            let mut account = ctx.journal_mut().load_account_code(address)?;
            // to make sure this account is ownable and owner by the same runtime, that allows
            // a runtime to modify any account it owns
            let Some(ownable_account_bytecode) = (match account.info.code.as_mut() {
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
                let output = VecU8::try_from_slice_unwrap(&[
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
                return_result!(account_owner_address.0 .0, Ok);
            }
            let account = ctx.journal_mut().load_account_code(address)?;
            match account.info.code.as_ref() {
                Some(Bytecode::OwnableAccount(ownable_account_bytecode)) => {
                    return_result!(ownable_account_bytecode.owner_address.0 .0, Ok)
                }
                _ => return_result!(Address::ZERO.0 .0, Ok),
            };
        }

        SYSCALL_ID_METADATA_CREATE => {
            let Some(account_owner_address) = account_owner_address else {
                return_halt!(MalformedBuiltinParams);
            };
            let (input, lazy_metadata_input) = get_input_validated!(>= 32);
            let salt = U256::from_be_slice(&input);
            let derived_metadata_address = calc_create4_address(&account_owner_address, &salt);
            let account = ctx
                .journal_mut()
                .load_account_code(derived_metadata_address)?;
            // make sure there is no account create collision
            assert_halt!(account.is_empty(), CreateContractCollision);
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
            return_result!(VecU8::default_for_reuse(), Ok)
        }

        SYSCALL_ID_METADATA_WRITE => {
            let Some(account_owner_address) = account_owner_address else {
                return_halt!(MalformedBuiltinParams);
            };
            let (input, lazy_metadata_input) = get_input_validated!(>= 20 + 4);
            // read an account from its address
            let address = Address::from_slice(&input[..20]);
            let offset = LittleEndian::read_u32(&input[20..24]) as usize;
            if offset != 0 {
                return_halt!(MalformedBuiltinParams);
            }
            let mut account = ctx.journal_mut().load_account_code(address)?;
            // to make sure this account is ownable and owner by the same runtime, that allows
            // a runtime to modify any account it owns
            let ownable_account_bytecode = match account.info.code.as_mut() {
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
            return_result!(VecU8::default_for_reuse(), Ok)
        }

        SYSCALL_ID_METADATA_COPY => {
            let Some(account_owner_address) = account_owner_address else {
                return_halt!(MalformedBuiltinParams);
            };
            let input = get_input_validated!(== 20);
            // read an account from its address
            let address = Address::from_slice(&input[..20]);
            let mut account = ctx.journal_mut().load_account_code(address)?;
            // to make sure this account is ownable and owner by the same runtime, that allows
            // a runtime to modify any account it owns
            let ownable_account_bytecode = match account.info.code.as_mut() {
                Some(Bytecode::OwnableAccount(ownable_account_bytecode))
                    if ownable_account_bytecode.owner_address == account_owner_address =>
                {
                    ownable_account_bytecode
                }
                _ => {
                    return_halt!(MalformedBuiltinParams)
                }
            };
            assert_halt!(input.len() == 28, MalformedBuiltinParams);
            let offset = LittleEndian::read_u32(&input[20..24]);
            let length = LittleEndian::read_u32(&input[24..28]);
            // take min
            let length = length.min(ownable_account_bytecode.metadata.len() as u32);
            let metadata = ownable_account_bytecode
                .metadata
                .slice(offset as usize..(offset + length) as usize);
            return_result!(metadata.as_ref(), Ok)
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

            return_result!(VecU8::default_for_reuse(), Ok);
        }

        SYSCALL_ID_TRANSIENT_READ => {
            let input = get_input_validated!(== 32);
            // read value from storage
            let slot = U256::from_le_slice(&input[0..32].as_ref());
            let value = ctx.journal_mut().tload(current_target_address, slot);
            // charge gas
            charge_gas!(gas::WARM_STORAGE_READ_COST);
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
            return_result!(VecU8::default_for_reuse(), Ok);
        }

        SYSCALL_ID_BLOCK_HASH => {
            let input = get_input_validated!(== 8);
            charge_gas!(gas::BLOCKHASH);
            let requested_block = LittleEndian::read_u64(&input[0..8]);
            let current_block = ctx.block_number().as_limbs()[0];
            // Why do we return in big-endian here? :facepalm:
            let hash = match current_block.checked_sub(requested_block) {
                Some(diff) if diff > 0 && diff <= 256 => {
                    ctx.block_hash(requested_block).unwrap_or(B256::ZERO)
                }
                _ => B256::ZERO,
            };
            return_result!(hash.0, Ok);
        }

        _ => return_halt!(MalformedBuiltinParams),
    }
}
