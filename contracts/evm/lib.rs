#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

extern crate alloc;
extern crate core;

use alloc::vec::Vec;
use core::convert::AsRef;
use fluentbase_evm::{
    bytecode::AnalyzedBytecode, gas, gas::Gas, opcodes::interruptable_instruction_table,
    types::InterruptionOutcome, EthVM, EthereumMetadata, ExecutionResult, InterpreterAction,
};
use fluentbase_sdk::{
    bincode, byteorder,
    byteorder::ByteOrder,
    crypto::crypto_keccak256,
    entrypoint,
    system::{RuntimeInterruptionOutcomeV1, RuntimeNewFrameInputV1},
    Bytes, ContextReader, ExitCode, SharedAPI, SyscallInvocationParams, B256, EVM_MAX_CODE_SIZE,
    FUEL_DENOM_RATE,
};
use spin::MutexGuard;

/// Transforms metadata into analyzed EVM bytecode when possible.
pub(crate) fn evm_bytecode_from_metadata(metadata: &Bytes) -> Option<AnalyzedBytecode> {
    Some(match EthereumMetadata::read_from_bytes(metadata)? {
        EthereumMetadata::Legacy(bytecode) => {
            AnalyzedBytecode::new(bytecode.bytecode, bytecode.hash)
        }
        EthereumMetadata::Analyzed(bytecode) => bytecode,
    })
}

static SAVED_EVM_CONTEXT: spin::Once<spin::Mutex<Vec<EthVM>>> = spin::Once::new();

fn lock_evm_context<'a>() -> MutexGuard<'a, Vec<EthVM>> {
    let cached_state = SAVED_EVM_CONTEXT.call_once(|| spin::Mutex::new(Vec::new()));
    debug_assert!(
        !cached_state.is_locked(),
        "evm: spin mutex is locked, looks like memory corruption"
    );
    cached_state.lock()
}

fn restore_evm_context_or_create<'a>(
    cached_state: &'a mut MutexGuard<Vec<EthVM>>,
    context: impl ContextReader,
    input: Bytes,
    return_data: Bytes,
) -> &'a mut EthVM {
    // If return data is empty, then we create new EVM frame
    if return_data.is_empty() {
        // Decode new frame input
        let (new_frame_input, _) = bincode::decode_from_slice::<RuntimeNewFrameInputV1, _>(
            input.as_ref(),
            bincode::config::legacy(),
        )
        .unwrap();
        // If analyzed, bytecode is not presented then extract it from the input
        // (contract deployment stage)
        let (analyzed_bytecode, contract_input) = if !new_frame_input.metadata.is_empty() {
            let Some(analyzed_bytecode) = evm_bytecode_from_metadata(&new_frame_input.metadata)
            else {
                unreachable!("evm: a valid metadata must be provided")
            };
            (analyzed_bytecode, new_frame_input.input)
        } else {
            let analyzed_bytecode =
                AnalyzedBytecode::new(new_frame_input.input.clone(), B256::ZERO);
            (analyzed_bytecode, Bytes::new())
        };
        let eth_vm = EthVM::new(context, contract_input, analyzed_bytecode);
        // Push new EthVM frame (new frame is created)
        cached_state.push(eth_vm);
        cached_state.last_mut().unwrap()
    } else {
        drop(context);
        let (
            RuntimeInterruptionOutcomeV1 {
                halted_frame,
                output,
                fuel_consumed,
                fuel_refunded,
                exit_code,
            },
            _,
        ) = bincode::decode_from_slice::<RuntimeInterruptionOutcomeV1, _>(
            return_data.as_ref(),
            bincode::config::legacy(),
        )
        .unwrap();
        let Some(eth_vm) = cached_state.last_mut() else {
            unreachable!("evm: missing cached evm state, can't resume execution")
        };
        let mut gas = Gas::new_spent(fuel_consumed / FUEL_DENOM_RATE);
        gas.record_refund(fuel_refunded / FUEL_DENOM_RATE as i64);
        {
            let dirty_gas = &mut eth_vm.interpreter.gas;
            if !dirty_gas.record_cost(gas.spent()) {
                unreachable!(
                    "evm: a fatal gas mis-sync between runtimes, this should never happen"
                );
            }
            eth_vm.interpreter.extend.committed_gas = *dirty_gas;
        }
        let exit_code = ExitCode::from(exit_code);
        _ = eth_vm
            .interpreter
            .extend
            .interruption_outcome
            .insert(InterruptionOutcome {
                output,
                gas,
                exit_code,
                halted_frame,
            });
        eth_vm
    }
}

/// Deploy entry for EVM contracts.
/// Runs init bytecode, enforces EIP-3541 and EIP-170, charges CODEDEPOSIT gas,
/// then commits the resulting runtime bytecode to metadata.
pub fn deploy_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let (output, exit_code) = deploy_inner(&mut sdk, lock_evm_context());
    let mut exit_code_le: [u8; 4] = [0u8; 4];
    byteorder::LE::write_i32(&mut exit_code_le, exit_code as i32);
    let mut result = Vec::with_capacity(4 + output.len());
    result.extend_from_slice(&exit_code_le);
    result.extend_from_slice(&output);
    sdk.write(&result);
}

fn deploy_inner<SDK: SharedAPI>(
    sdk: &mut SDK,
    mut cached_state: MutexGuard<Vec<EthVM>>,
) -> (Bytes, ExitCode) {
    let evm = restore_evm_context_or_create(
        &mut cached_state,
        sdk.context(),
        sdk.bytes_input(),
        sdk.return_data(),
    );
    let instruction_table = interruptable_instruction_table::<SDK>();
    match evm.run_step(&instruction_table, sdk) {
        InterpreterAction::Return(result) => {
            let committed_gas = evm.interpreter.extend.committed_gas;
            _ = cached_state.pop();
            let mut result = ExecutionResult {
                result: result.result,
                output: result.output,
                committed_gas,
                gas: result.gas,
            };
            if result.result.is_ok() {
                // EIP-3541 and EIP-170 checks
                if result.output.first() == Some(&0xEF) {
                    return (Bytes::new(), ExitCode::CreateContractStartingWithEF);
                } else if result.output.len() > EVM_MAX_CODE_SIZE {
                    return (Bytes::new(), ExitCode::CreateContractSizeLimit);
                }
                let gas_for_code = result.output.len() as u64 * gas::CODEDEPOSIT;
                if !result.gas.record_cost(gas_for_code) {
                    return (Bytes::new(), ExitCode::OutOfFuel);
                }
                let consumed_diff = result.chargeable_fuel();
                sdk.charge_fuel(consumed_diff);
                // We intentionally don't charge gas for these opcodes
                // to keep full compatibility with an EVM deployment process
                let evm_code_hash = crypto_keccak256(result.output.as_ref());
                let analyzed_bytecode = AnalyzedBytecode::new(result.output, evm_code_hash);
                let evm_bytecode = EthereumMetadata::Analyzed(analyzed_bytecode).write_to_bytes();
                (evm_bytecode, ExitCode::Ok)
            } else {
                let consumed_diff = result.chargeable_fuel();
                sdk.charge_fuel(consumed_diff);
                let exit_code = if result.result.is_revert() {
                    ExitCode::Panic
                } else {
                    ExitCode::Err
                };
                (result.output, exit_code)
            }
        }
        InterpreterAction::SystemInterruption {
            code_hash,
            input,
            fuel_limit,
            state,
        } => {
            let input_offset = input.as_ptr() as usize;
            evm.sync_evm_gas(sdk);
            let syscall_params = SyscallInvocationParams {
                code_hash,
                input: input_offset..(input_offset + input.len()),
                fuel_limit: fuel_limit.unwrap_or(u64::MAX),
                state,
                fuel16_ptr: 0,
            }
            .encode();
            (syscall_params.into(), ExitCode::InterruptionCalled)
        }
        InterpreterAction::NewFrame(_) => unreachable!("frames can't be produced"),
    }
}

/// Main entry for executing deployed EVM bytecode.
/// Loads analyzed code from metadata, runs EthVM with call input, settles fuel,
/// and writes the returned data.
#[inline(never)]
pub fn main_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let mut cached_state = lock_evm_context();
    let evm = restore_evm_context_or_create(
        &mut cached_state,
        // Pass information about execution context (contract address, caller) into the EthVM,
        // but it's used only if EthVM is not created (aka first call, not resume)
        sdk.context(),
        // Input of the smart contract
        sdk.bytes_input(),
        // Return data indicates the existence of interrupted state,
        // if we have return data not empty,
        // then we've executed this frame before and need to resume
        sdk.return_data(),
    );
    let instruction_table = interruptable_instruction_table::<SDK>();
    let (output, exit_code) = match evm.run_step(&instruction_table, &mut sdk) {
        InterpreterAction::Return(result) => {
            evm.sync_evm_gas(&mut sdk);
            _ = cached_state.pop();
            let exit_code = if result.result.is_ok() {
                ExitCode::Ok
            } else if result.result.is_revert() {
                ExitCode::Panic
            } else {
                ExitCode::Err
            };
            (result.output, exit_code)
        }
        InterpreterAction::SystemInterruption {
            code_hash,
            input,
            fuel_limit,
            state,
        } => {
            let input_offset = input.as_ptr() as usize;
            evm.sync_evm_gas(&mut sdk);
            let syscall_params = SyscallInvocationParams {
                code_hash,
                input: input_offset..(input_offset + input.len()),
                fuel_limit: fuel_limit.unwrap_or(u64::MAX),
                state,
                fuel16_ptr: 0,
            }
            .encode();
            (syscall_params.into(), ExitCode::InterruptionCalled)
        }
        InterpreterAction::NewFrame(_) => unreachable!("evm: frames can't be produced"),
    };
    let mut exit_code_le: [u8; 4] = [0u8; 4];
    byteorder::LE::write_i32(&mut exit_code_le, exit_code as i32);
    let mut result = Vec::with_capacity(4 + output.len());
    result.extend_from_slice(&exit_code_le);
    result.extend_from_slice(&output);
    sdk.write(&result);
}

entrypoint!(main_entry, deploy_entry);
