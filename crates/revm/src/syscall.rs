use crate::{
    api::RwasmFrame,
    instruction_result_from_exit_code,
    types::{SystemInterruptionInputs, SystemInterruptionOutcome},
    ExecutionResult, NextAction,
};
use core::cmp::min;
use fluentbase_evm::EthereumMetadata;
use fluentbase_sdk::{
    byteorder::{ByteOrder, LittleEndian, ReadBytesExt},
    bytes::Buf,
    calc_create4_address, is_system_precompile, keccak256, Address, Bytes, ExitCode, Log, LogData,
    B256, FUEL_DENOM_RATE, KECCAK_EMPTY, PRECOMPILE_EVM_RUNTIME, STATE_MAIN, U256,
};
use revm::{
    bytecode::{ownable_account::OwnableAccountBytecode, Bytecode},
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
        wasm::wasm_max_code_size,
    },
    Database, Inspector,
};
use std::{boxed::Box, vec::Vec};

#[tracing::instrument(level = "info", skip_all)]
pub(crate) fn inspect_rwasm_interruption<CTX: ContextTr, INSP: Inspector<CTX>>(
    _frame: &mut RwasmFrame,
    _ctx: &mut CTX,
    _inspector: &mut INSP,
    _inputs: SystemInterruptionInputs,
) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
    // frame.interpreter.gas = Gas::new(inputs.gas.remaining());
    // let prev_bytecode = frame.interpreter.bytecode.clone();
    // let prev_hash = frame.interpreter.bytecode.hash().clone();
    // let evm_opcode = match &inputs.syscall_params.code_hash {
    //     &SYSCALL_ID_CALL => opcode::CALL,
    //     &SYSCALL_ID_STATIC_CALL => opcode::STATICCALL,
    //     &SYSCALL_ID_CALL_CODE => opcode::CALLCODE,
    //     &SYSCALL_ID_DELEGATE_CALL => opcode::DELEGATECALL,
    //     &SYSCALL_ID_CREATE => opcode::CREATE,
    //     &SYSCALL_ID_CREATE2 => opcode::CREATE2,
    //     _ => return execute_rwasm_interruption::<CTX, INSP>(frame, ctx, inputs),
    // };
    // let bytecode = Bytecode::Rwasm([evm_opcode].into());
    // frame.interpreter.bytecode = ExtBytecode::new(bytecode);
    // inspector.step(&mut frame.interpreter, ctx);
    // let result = execute_rwasm_interruption::<CTX, INSP>(frame, ctx, inputs)?;
    // if let Some(prev_hash) = prev_hash {
    //     frame.interpreter.bytecode = ExtBytecode::new_with_hash(prev_bytecode, prev_hash);
    // } else {
    //     frame.interpreter.bytecode = ExtBytecode::new(prev_bytecode);
    // }
    // let gas = if let Some(interrupted_outcome) = frame.interrupted_outcome.as_ref() {
    //     interrupted_outcome.remaining_gas
    // } else {
    //     match &result {
    //         NextAction::Return(result) => result.gas,
    //         _ => unreachable!("frame can't return here"),
    //     }
    // };
    // _ = frame.interpreter.gas.record_cost(gas.spent());
    // frame.interpreter.gas.record_refund(gas.refunded());
    // inspector.step_end(&mut frame.interpreter, ctx);
    // Ok(result)
    todo!()
}

#[tracing::instrument(level = "info", skip_all)]
pub(crate) fn execute_rwasm_interruption<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    inputs: SystemInterruptionInputs,
) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
    let mut local_gas = Gas::new(inputs.gas.remaining());
    let spec_id: SpecId = ctx.cfg().spec().into();

    let current_target_address = frame.interpreter.input.target_address();
    let account_owner_address = frame.interpreter.input.account_owner_address();

    macro_rules! return_result {
        ($output:expr, $result:ident) => {{
            let output: Bytes = $output.into();
            let result = ExecutionResult {
                result: instruction_result_from_exit_code(ExitCode::$result, output.is_empty()),
                output,
                gas: local_gas,
            };
            frame.insert_interrupted_outcome(SystemInterruptionOutcome {
                inputs: Box::new(inputs),
                remaining_gas: local_gas,
                result: Some(result),
                is_frame: false,
            });
            return Ok(NextAction::InterruptionResult);
        }};
        ($result:ident) => {{
            return_result!(Bytes::default(), $result)
        }};
    }
    macro_rules! return_frame {
        ($action:expr) => {{
            frame.insert_interrupted_outcome(SystemInterruptionOutcome {
                inputs: Box::new(inputs),
                remaining_gas: local_gas,
                result: None,
                is_frame: true,
            });
            return Ok($action);
        }};
    }
    macro_rules! assert_return {
        ($cond:expr, $error:ident) => {
            if !($cond) {
                return_result!($error);
            }
        };
    }
    macro_rules! charge_gas {
        ($value:expr) => {{
            if !local_gas.record_cost($value) {
                return_result!(OutOfFuel);
            }
        }};
    }
    macro_rules! inspect {
        ($evm_opcode:expr, $inputs:expr, $outputs:expr) => {{
            // if let Some(inspector) = inspector.as_mut() {
            //     inspect_syscall(
            //         frame,
            //         ctx,
            //         inspector,
            //         $evm_opcode,
            //         inputs.gas.remaining(),
            //         local_gas,
            //         $inputs,
            //     );
            // }
        }};
    }

    use fluentbase_sdk::syscall::*;
    match inputs.syscall_params.code_hash {
        SYSCALL_ID_STORAGE_READ => {
            assert_return!(
                inputs.syscall_params.input.len() == 32
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            let slot = U256::from_le_slice(&inputs.syscall_params.input[0..32]);
            // execute sload
            let value = ctx.journal_mut().sload(current_target_address, slot)?;
            charge_gas!(sload_cost(spec_id, value.is_cold));
            // inspect!(opcode::SLOAD, [slot], [value.data]);
            let output: [u8; 32] = value.to_le_bytes();
            return_result!(output, Ok)
        }

        SYSCALL_ID_STORAGE_WRITE => {
            assert_return!(
                inputs.syscall_params.input.len() == 32 + 32
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            // don't allow for static context
            assert_return!(!inputs.is_static, StateChangeDuringStaticCall);
            let slot = U256::from_le_slice(&inputs.syscall_params.input[0..32]);
            let new_value = U256::from_le_slice(&inputs.syscall_params.input[32..64]);
            // execute sstore
            let value = ctx
                .journal_mut()
                .sstore(current_target_address, slot, new_value)?;
            if local_gas.remaining() <= gas::CALL_STIPEND {
                return_result!(OutOfFuel);
            }
            let gas_cost = sstore_cost(spec_id.clone(), &value.data, value.is_cold);
            charge_gas!(gas_cost);
            local_gas.record_refund(sstore_refund(spec_id, &value.data));
            // inspect!(opcode::SSTORE, [slot, new_value], []);
            return_result!(Ok)
        }

        SYSCALL_ID_CALL => {
            assert_return!(
                inputs.syscall_params.input.len() >= 20 + 32
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            let target_address = Address::from_slice(&inputs.syscall_params.input[0..20]);
            let value = U256::from_le_slice(&inputs.syscall_params.input[20..52]);
            let contract_input = inputs.syscall_params.input.slice(52..);
            // for static calls with value greater than 0 - revert
            let has_transfer = !value.is_zero();
            if inputs.is_static && has_transfer {
                return_result!(StateChangeDuringStaticCall);
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
                local_gas.remaining_63_of_64_parts(),
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
            // create call inputs
            let call_inputs = Box::new(CallInputs {
                input: CallInput::Bytes(contract_input),
                gas_limit,
                target_address,
                caller: current_target_address,
                bytecode_address: target_address,
                value: CallValue::Transfer(value),
                scheme: CallScheme::Call,
                is_static: inputs.is_static,
                return_memory_offset: Default::default(),
            });
            return_frame!(NextAction::NewFrame(FrameInput::Call(call_inputs)));
        }

        SYSCALL_ID_STATIC_CALL => {
            assert_return!(
                inputs.syscall_params.input.len() >= 20
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            let target_address = Address::from_slice(&inputs.syscall_params.input[0..20]);
            let contract_input = inputs.syscall_params.input.slice(20..);
            let mut account_load = ctx.journal_mut().load_account_delegated(target_address)?;
            // set is_empty to false as we are not creating this account.
            account_load.is_empty = false;
            // EIP-150: gas cost changes for IO-heavy operations
            charge_gas!(gas::call_cost(spec_id.clone(), false, account_load));
            let gas_limit = if spec_id.is_enabled_in(TANGERINE) {
                min(
                    local_gas.remaining_63_of_64_parts(),
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
            // create call inputs
            let call_inputs = Box::new(CallInputs {
                input: CallInput::Bytes(contract_input),
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
            assert_return!(
                inputs.syscall_params.input.len() >= 20 + 32
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            let target_address = Address::from_slice(&inputs.syscall_params.input[0..20]);
            let value = U256::from_le_slice(&inputs.syscall_params.input[20..52]);
            let contract_input = inputs.syscall_params.input.slice(52..);
            let mut account_load = ctx.journal_mut().load_account_delegated(target_address)?;
            // set is_empty to false as we are not creating this account
            account_load.is_empty = false;
            // EIP-150: gas cost changes for IO-heavy operations
            charge_gas!(gas::call_cost(spec_id, !value.is_zero(), account_load));
            let mut gas_limit = if spec_id.is_enabled_in(TANGERINE) {
                min(
                    local_gas.remaining_63_of_64_parts(),
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
            // create call inputs
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
            let call_inputs = Box::new(CallInputs {
                input: CallInput::Bytes(contract_input),
                gas_limit,
                target_address: current_target_address,
                caller: current_target_address,
                bytecode_address: target_address,
                value: CallValue::Transfer(value),
                scheme: CallScheme::CallCode,
                is_static: inputs.is_static,
                return_memory_offset: Default::default(),
            });
            return_frame!(NextAction::NewFrame(FrameInput::Call(call_inputs)));
        }

        SYSCALL_ID_DELEGATE_CALL => {
            assert_return!(
                inputs.syscall_params.input.len() >= 20
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            let target_address = Address::from_slice(&inputs.syscall_params.input[0..20]);
            let contract_input = inputs.syscall_params.input.slice(20..);
            let mut account_load = ctx.journal_mut().load_account_delegated(target_address)?;
            // set is_empty to false as we are not creating this account.
            account_load.is_empty = false;
            // EIP-150: gas cost changes for IO-heavy operations
            charge_gas!(gas::call_cost(spec_id, false, account_load));
            let gas_limit = if spec_id.is_enabled_in(TANGERINE) {
                min(
                    local_gas.remaining_63_of_64_parts(),
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
            // create call inputs
            let call_inputs = Box::new(CallInputs {
                input: CallInput::Bytes(contract_input),
                gas_limit,
                target_address: current_target_address,
                caller: frame.interpreter.input.caller_address(),
                bytecode_address: target_address,
                value: CallValue::Apparent(frame.interpreter.input.call_value()),
                scheme: CallScheme::DelegateCall,
                is_static: inputs.is_static,
                return_memory_offset: Default::default(),
            });
            return_frame!(NextAction::NewFrame(FrameInput::Call(call_inputs)));
        }

        SYSCALL_ID_CREATE | SYSCALL_ID_CREATE2 => {
            assert_return!(
                inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            // not allowed for static calls
            assert_return!(!inputs.is_static, StateChangeDuringStaticCall);
            // make sure we have enough bytes inside input params
            let is_create2 = inputs.syscall_params.code_hash == SYSCALL_ID_CREATE2;
            let (scheme, value, init_code) = if is_create2 {
                assert_return!(
                    inputs.syscall_params.input.len() >= 32 + 32,
                    MalformedBuiltinParams
                );
                let value = U256::from_le_slice(&inputs.syscall_params.input[0..32]);
                let salt = U256::from_le_slice(&inputs.syscall_params.input[32..64]);
                let init_code = inputs.syscall_params.input.slice(64..);
                (CreateScheme::Create2 { salt }, value, init_code)
            } else {
                assert_return!(
                    inputs.syscall_params.input.len() >= 32,
                    MalformedBuiltinParams
                );
                let value = U256::from_le_slice(&inputs.syscall_params.input[0..32]);
                let init_code = inputs.syscall_params.input.slice(32..);
                (CreateScheme::Create, value, init_code)
            };
            // make sure we don't exceed max possible init code
            // TODO(khasan): take into consideration evm.ctx().cfg().max_init_code
            let max_initcode_size = wasm_max_code_size(&init_code).unwrap_or(MAX_INITCODE_SIZE);
            assert_return!(
                init_code.len() <= max_initcode_size,
                CreateContractSizeLimit
            );
            if !init_code.is_empty() {
                charge_gas!(gas::initcode_cost(init_code.len()));
            }
            if is_create2 {
                let Some(gas) = gas::create2_cost(init_code.len().try_into().unwrap()) else {
                    return_result!(OutOfFuel);
                };
                charge_gas!(gas);
            } else {
                charge_gas!(gas::CREATE);
            };
            let mut gas_limit = local_gas.remaining();
            gas_limit -= gas_limit / 64;
            charge_gas!(gas_limit);
            // create inputs
            match scheme {
                CreateScheme::Create => {
                    inspect!(opcode::CREATE, [value, U256::ZERO, U256::ZERO], []);
                }
                CreateScheme::Create2 { .. } => {
                    inspect!(opcode::CREATE2, [value, U256::ZERO, U256::ZERO, salt], []);
                }
                CreateScheme::Custom { .. } => {}
            }
            let create_inputs = Box::new(CreateInputs {
                caller: current_target_address,
                scheme,
                value,
                init_code,
                gas_limit,
            });
            return_frame!(NextAction::NewFrame(FrameInput::Create(create_inputs)));
        }

        SYSCALL_ID_EMIT_LOG => {
            assert_return!(
                inputs.syscall_params.input.len() >= 1 && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            // not allowed for static calls
            assert_return!(!inputs.is_static, StateChangeDuringStaticCall);
            // read topics from input
            let topics_len = inputs.syscall_params.input[0] as usize;
            assert_return!(topics_len <= 4, MalformedBuiltinParams);
            let mut topics = Vec::with_capacity(topics_len);
            assert_return!(
                inputs.syscall_params.input.len() >= 1 + topics_len * B256::len_bytes(),
                MalformedBuiltinParams
            );
            for i in 0..topics_len {
                let offset = 1 + i * B256::len_bytes();
                let topic =
                    &inputs.syscall_params.input.as_ref()[offset..(offset + B256::len_bytes())];
                topics.push(B256::from_slice(topic));
            }
            // all remaining bytes are data
            let data = inputs
                .syscall_params
                .input
                .slice((1 + topics_len * B256::len_bytes())..);
            // make sure we have enough gas to cover this operation
            let Some(gas_cost) = gas::log_cost(topics_len as u8, data.len() as u64) else {
                return_result!(OutOfFuel);
            };
            charge_gas!(gas_cost);
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
            // write new log into the journal
            ctx.journal_mut().log(Log {
                address: current_target_address,
                // it's safe to go unchecked here because we do topic check upper
                data: LogData::new_unchecked(topics, data),
            });
            return_result!(Ok);
        }

        SYSCALL_ID_DESTROY_ACCOUNT => {
            assert_return!(
                inputs.syscall_params.input.len() == 20
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            // not allowed for static calls
            assert_return!(!inputs.is_static, StateChangeDuringStaticCall);
            // destroy an account
            let target = Address::from_slice(&inputs.syscall_params.input[0..20]);
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
            assert_return!(
                inputs.syscall_params.input.len() == 20
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            let address = Address::from_slice(&inputs.syscall_params.input[0..20]);
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
            assert_return!(
                inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            let value = ctx
                .journal_mut()
                .load_account(current_target_address)
                .map(|acc| acc.map(|a| a.info.balance))?;
            charge_gas!(gas::LOW);
            let output: [u8; 32] = value.data.to_le_bytes();
            return_result!(output, Ok)
        }

        SYSCALL_ID_CODE_SIZE => {
            assert_return!(
                inputs.syscall_params.state == STATE_MAIN
                    && inputs.syscall_params.input.len() == 20,
                MalformedBuiltinParams
            );
            let address = Address::from_slice(&inputs.syscall_params.input[0..20]);

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
            assert_return!(
                inputs.syscall_params.state == STATE_MAIN
                    && inputs.syscall_params.input.len() == 20,
                MalformedBuiltinParams
            );
            let address = Address::from_slice(&inputs.syscall_params.input[0..20]);

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

            return_result!(code_hash, Ok);
        }

        SYSCALL_ID_CODE_COPY => {
            assert_return!(
                inputs.syscall_params.state == STATE_MAIN
                    && inputs.syscall_params.input.len() == 20 + 8 * 2,
                MalformedBuiltinParams
            );
            let address = Address::from_slice(&inputs.syscall_params.input[0..20]);
            let mut reader = inputs.syscall_params.input[20..].reader();
            let _code_offset = reader.read_u64::<LittleEndian>().unwrap();
            let code_length = reader.read_u64::<LittleEndian>().unwrap();

            // Load account with bytecode and charge gas for the call
            let account = ctx.journal_mut().load_account_code(address)?;
            let Some(gas_cost) =
                gas::extcodecopy_cost(spec_id, code_length as usize, account.is_cold)
            else {
                return_result!(OutOfFuel);
            };
            charge_gas!(gas_cost);

            // If requested code length is zero, then there is no need to proceed
            if code_length == 0 {
                return_result!(Bytes::new(), Ok);
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
                    .unwrap_or_default(),
            };

            // we store system precompile bytecode in the state trie,
            // according to evm requirements, we should return empty code
            if is_system_precompile(&address) {
                bytecode = Bytes::new();
            }

            // TODO(dmitry123): Add offset/length checks, otherwise gas can be abused!
            return_result!(bytecode, Ok);
        }

        SYSCALL_ID_METADATA_SIZE => {
            assert_return!(
                inputs.syscall_params.input.len() >= 20
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            // syscall is allowed only for accounts that are owned by somebody
            let Some(account_owner_address) = account_owner_address else {
                return_result!(MalformedBuiltinParams);
            };
            // read an account from its address
            let address = Address::from_slice(&inputs.syscall_params.input[..20]);
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
            assert_return!(
                inputs.syscall_params.input.len() == 20,
                MalformedBuiltinParams
            );
            let mut output = [0u8; 4 + 3];
            LittleEndian::write_u32(&mut output, ownable_account_bytecode.metadata.len() as u32);
            output[4] = 0x01u8; // the account belongs to the same runtime
            output[5] = account.is_cold as u8;
            output[6] = account.is_empty() as u8;
            return_result!(output, Ok)
        }

        SYSCALL_ID_METADATA_ACCOUNT_OWNER => {
            assert_return!(
                inputs.syscall_params.input.len() >= 20
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            // syscall is allowed only for accounts that are owned by somebody
            let Some(_account_owner_address) = account_owner_address else {
                return_result!(MalformedBuiltinParams);
            };
            let address = Address::from_slice(&inputs.syscall_params.input[..Address::len_bytes()]);
            let account = ctx.journal_mut().load_account_code(address)?;
            match account.info.code.as_ref() {
                Some(Bytecode::OwnableAccount(ownable_account_bytecode)) => {
                    return_result!(ownable_account_bytecode.owner_address.0, Ok)
                }
                _ => {}
            };
            return_result!(Address::ZERO.0, Ok)
        }
        SYSCALL_ID_METADATA_CREATE => {
            assert_return!(
                inputs.syscall_params.input.len() >= 32
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            // syscall is allowed only for accounts that are owned by somebody
            let Some(account_owner_address) = account_owner_address else {
                return_result!(MalformedBuiltinParams);
            };
            // read an account from its address
            let salt = U256::from_be_slice(&inputs.syscall_params.input[..32]);
            let metadata = inputs.syscall_params.input.slice(32..);
            let derived_metadata_address =
                calc_create4_address(&account_owner_address, &salt, |v| keccak256(v));
            let account = ctx
                .journal_mut()
                .load_account_code(derived_metadata_address)?;
            // make sure there is no account create collision
            if !account.is_empty() {
                return_result!(CreateContractCollision);
            }
            // create a new derived ownable account
            ctx.journal_mut().set_code(
                derived_metadata_address,
                Bytecode::OwnableAccount(OwnableAccountBytecode::new(
                    account_owner_address,
                    metadata.clone(),
                )),
            );
            return_result!(Bytes::new(), Ok)
        }

        SYSCALL_ID_METADATA_WRITE | SYSCALL_ID_METADATA_COPY => {
            assert_return!(
                inputs.syscall_params.input.len() >= 20
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            // syscall is allowed only for accounts that are owned by somebody
            let Some(account_owner_address) = account_owner_address else {
                return_result!(MalformedBuiltinParams);
            };
            // read an account from its address
            let address = Address::from_slice(&inputs.syscall_params.input[..20]);
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
                    return_result!(Bytes::new(), MalformedBuiltinParams)
                }
            };
            // execute a syscall
            match inputs.syscall_params.code_hash {
                SYSCALL_ID_METADATA_WRITE => {
                    assert_return!(
                        inputs.syscall_params.input.len() >= 20 + 4,
                        MalformedBuiltinParams
                    );
                    let offset =
                        LittleEndian::read_u32(&inputs.syscall_params.input[20..24]) as usize;
                    let length = inputs.syscall_params.input[24..].len();
                    // TODO(dmitry123): "figure out a way how to optimize it"
                    let mut metadata = ownable_account_bytecode.metadata.to_vec();
                    metadata.resize(offset + length, 0);
                    metadata[offset..(offset + length)]
                        .copy_from_slice(&inputs.syscall_params.input[24..]);
                    // code might change, rewrite it with a new hash
                    let new_bytecode = Bytecode::OwnableAccount(OwnableAccountBytecode::new(
                        ownable_account_bytecode.owner_address,
                        metadata.into(),
                    ));
                    ctx.journal_mut().set_code(address, new_bytecode);
                    return_result!(Bytes::new(), Ok)
                }
                SYSCALL_ID_METADATA_COPY => {
                    assert_return!(
                        inputs.syscall_params.input.len() == 28,
                        MalformedBuiltinParams
                    );
                    let offset = LittleEndian::read_u32(&inputs.syscall_params.input[20..24]);
                    let length = LittleEndian::read_u32(&inputs.syscall_params.input[24..28]);
                    // take min
                    let length = length.min(ownable_account_bytecode.metadata.len() as u32);
                    let metadata = ownable_account_bytecode
                        .metadata
                        .slice(offset as usize..(offset + length) as usize);
                    return_result!(metadata, Ok)
                }
                _ => unreachable!(),
            }
        }

        SYSCALL_ID_METADATA_STORAGE_READ => {
            // input: slot
            let Some(account_owner_address) = account_owner_address else {
                return_result!(MalformedBuiltinParams);
            };
            const INPUT_LEN: usize = U256::BYTES;
            let syscall_params = &inputs.syscall_params;
            assert_return!(
                syscall_params.input.len() == INPUT_LEN && syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );

            let Ok(slot): Result<[u8; U256::BYTES], _> =
                syscall_params.input.as_ref()[..U256::BYTES].try_into()
            else {
                return_result!(MalformedBuiltinParams)
            };
            let slot_u256 = U256::from_le_bytes(slot);
            let value = ctx.journal_mut().sload(account_owner_address, slot_u256)?;
            let output: [u8; U256::BYTES] = value.to_le_bytes();
            return_result!(output, Ok);
        }

        SYSCALL_ID_METADATA_STORAGE_WRITE => {
            let Some(account_owner_address) = account_owner_address else {
                return_result!(MalformedBuiltinParams);
            };
            // input: slot + value
            const INPUT_LEN: usize = U256::BYTES + U256::BYTES;
            let syscall_params = &inputs.syscall_params;
            assert_return!(
                syscall_params.input.len() == INPUT_LEN && syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );

            let Ok(slot): Result<[u8; U256::BYTES], _> =
                syscall_params.input.as_ref()[..U256::BYTES].try_into()
            else {
                return_result!(MalformedBuiltinParams)
            };
            let Ok(value): Result<[u8; U256::BYTES], _> =
                syscall_params.input.as_ref()[U256::BYTES..].try_into()
            else {
                return_result!(MalformedBuiltinParams)
            };
            let slot_u256 = U256::from_le_bytes(slot);
            let value_u256 = U256::from_le_bytes(value);
            ctx.journal_mut()
                .sstore(account_owner_address, slot_u256, value_u256)?;
            ctx.journal_mut().touch_account(account_owner_address);

            return_result!(Bytes::default(), Ok);
        }

        SYSCALL_ID_TRANSIENT_READ => {
            assert_return!(
                inputs.syscall_params.input.len() == 32
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            // read value from storage
            let slot = U256::from_le_slice(&inputs.syscall_params.input[0..32].as_ref());
            let value = ctx.journal_mut().tload(current_target_address, slot);
            // charge gas
            charge_gas!(gas::WARM_STORAGE_READ_COST);
            // return value
            let output: [u8; 32] = value.to_le_bytes();
            return_result!(output, Ok);
        }

        SYSCALL_ID_TRANSIENT_WRITE => {
            assert_return!(
                inputs.syscall_params.input.len() == 64
                    && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            assert_return!(!inputs.is_static, StateChangeDuringStaticCall);
            // read input
            let slot = U256::from_le_slice(&inputs.syscall_params.input[0..32]);
            let value = U256::from_le_slice(&inputs.syscall_params.input[32..64]);
            // charge gas
            charge_gas!(gas::WARM_STORAGE_READ_COST);
            ctx.journal_mut()
                .tstore(current_target_address, slot, value);
            // empty result
            return_result!(Bytes::new(), Ok);
        }

        SYSCALL_ID_BLOCK_HASH => {
            assert_return!(
                inputs.syscall_params.input.len() == 8 && inputs.syscall_params.state == STATE_MAIN,
                MalformedBuiltinParams
            );
            charge_gas!(gas::BLOCKHASH);
            let requested_block = LittleEndian::read_u64(&inputs.syscall_params.input[0..8]);
            let current_block = ctx.block_number().as_limbs()[0];
            // Why do we return in big-endian here? :facepalm:
            let hash = match current_block.checked_sub(requested_block) {
                Some(diff) if diff > 0 && diff <= 256 => {
                    ctx.block_hash(requested_block).unwrap_or(B256::ZERO)
                }
                _ => B256::ZERO,
            };
            return_result!(hash, Ok);
        }

        _ => return_result!(MalformedBuiltinParams),
    }
}
