#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

extern crate alloc;
extern crate core;

use core::convert::AsRef;
use fluentbase_evm::{
    bytecode::AnalyzedBytecode, gas, gas::Gas, opcodes::interruptable_instruction_table,
    types::InterruptionOutcome, EthVM, EthereumMetadata, ExecutionResult, InterpreterAction,
};
use fluentbase_sdk::{
    crypto::crypto_keccak256, system::RuntimeInterruptionOutcomeV1, system_entrypoint, Bytes,
    ExitCode, HashMap, SystemAPI, B256, EVM_MAX_CODE_SIZE, FUEL_DENOM_RATE,
};
use spin::{Mutex, MutexGuard, Once};

/// A saved EthVM context we store between calls
fn lock_evm_context<'a>() -> MutexGuard<'a, HashMap<u32, EthVM>> {
    static SAVED_EVM_CONTEXT: Once<Mutex<HashMap<u32, EthVM>>> = Once::new();
    let mutex = SAVED_EVM_CONTEXT.call_once(|| Mutex::new(HashMap::new()));
    if mutex.is_locked() {
        unreachable!("runtime corruption, can't restore evm context");
    }
    mutex.lock()
}

/// Transforms metadata into analyzed EVM bytecode when possible.
pub(crate) fn evm_bytecode_from_metadata(metadata: &Bytes) -> Option<AnalyzedBytecode> {
    Some(match EthereumMetadata::read_from_bytes(metadata)? {
        EthereumMetadata::Legacy(bytecode) => {
            AnalyzedBytecode::new(bytecode.bytecode, bytecode.hash)
        }
        EthereumMetadata::Analyzed(bytecode) => bytecode,
    })
}

fn restore_evm_context_or_create<'a, SDK: SystemAPI>(
    context: &'a mut MutexGuard<HashMap<u32, EthVM>>,
    sdk: &mut SDK,
) -> &'a mut EthVM {
    // A special case when runtime returns an interruption with missing runtime data (like balances or storage)
    if let Some(outcome) = sdk.take_interruption_outcome() {
        let RuntimeInterruptionOutcomeV1 {
            halted_frame,
            output,
            fuel_consumed,
            fuel_refunded,
            exit_code,
        } = outcome;
        let Some(eth_vm) = context.get_mut(&sdk.unique_key()) else {
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
    } else {
        let metadata = sdk.contract_metadata();
        let input = sdk.bytes_input();
        // If analyzed, bytecode is not presented, then extract it from the input
        // (contract deployment stage)
        let (analyzed_bytecode, contract_input) = if !metadata.is_empty() {
            let Some(analyzed_bytecode) = evm_bytecode_from_metadata(&metadata) else {
                unreachable!("evm: a valid metadata must be provided")
            };
            (analyzed_bytecode, input)
        } else {
            let analyzed_bytecode = AnalyzedBytecode::new(input.clone(), B256::ZERO);
            (analyzed_bytecode, Bytes::new())
        };
        let eth_vm = EthVM::new(sdk.context(), contract_input, analyzed_bytecode);
        // Push a new EthVM frame (new frame is created)
        _ = context.insert(sdk.unique_key(), eth_vm);
        context.get_mut(&sdk.unique_key()).unwrap()
    }
}

/// Deploy entry for EVM contracts.
/// Runs init bytecode, enforces EIP-3541 and EIP-170, charges CODEDEPOSIT gas,
/// then commits the resulting runtime bytecode to metadata.
pub fn deploy_entry<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let mut cached_state = lock_evm_context();
    let evm = restore_evm_context_or_create(&mut cached_state, sdk);
    let instruction_table = interruptable_instruction_table::<SDK>();
    match evm.run_step(&instruction_table, sdk) {
        InterpreterAction::Return(result) => {
            let committed_gas = evm.interpreter.extend.committed_gas;
            cached_state.remove(&sdk.unique_key());
            let mut result = ExecutionResult {
                result: result.result,
                output: result.output,
                committed_gas,
                gas: result.gas,
            };
            if result.result.is_ok() {
                // EIP-3541 and EIP-170 checks
                if result.output.first() == Some(&0xEF) {
                    return Err(ExitCode::CreateContractStartingWithEF);
                } else if result.output.len() > EVM_MAX_CODE_SIZE {
                    return Err(ExitCode::CreateContractSizeLimit);
                }
                let gas_for_code = result.output.len() as u64 * gas::CODEDEPOSIT;
                if !result.gas.record_cost(gas_for_code) {
                    return Err(ExitCode::OutOfFuel);
                }
                let consumed_diff = result.chargeable_fuel();
                sdk.charge_fuel(consumed_diff);
                // We intentionally don't charge gas for these opcodes
                // to keep full compatibility with an EVM deployment process
                let evm_code_hash = crypto_keccak256(result.output.as_ref());
                let analyzed_bytecode = AnalyzedBytecode::new(result.output, evm_code_hash);
                let evm_bytecode = EthereumMetadata::Analyzed(analyzed_bytecode).write_to_bytes();
                sdk.write_contract_metadata(evm_bytecode);
                Ok(())
            } else {
                let consumed_diff = result.chargeable_fuel();
                sdk.charge_fuel(consumed_diff);
                let exit_code = if result.result.is_revert() {
                    ExitCode::Panic
                } else {
                    ExitCode::Err
                };
                sdk.write(result.output);
                Err(exit_code)
            }
        }
        InterpreterAction::SystemInterruption {
            code_hash,
            input,
            fuel_limit,
            state,
        } => {
            // Always sync gas before doing interruption
            evm.sync_evm_gas(sdk);
            sdk.insert_interruption_income(code_hash, input, fuel_limit, state);
            Err(ExitCode::InterruptionCalled)
        }
        InterpreterAction::NewFrame(_) => unreachable!("frames can't be produced"),
    }
}

/// Main entry for executing deployed EVM bytecode.
/// Loads analyzed code from metadata, runs EthVM with call input, settles fuel,
/// and writes the returned data.
#[inline(never)]
pub fn main_entry<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let mut cached_state = lock_evm_context();
    let evm = restore_evm_context_or_create(&mut cached_state, sdk);
    let instruction_table = interruptable_instruction_table::<SDK>();
    match evm.run_step(&instruction_table, sdk) {
        InterpreterAction::Return(result) => {
            evm.sync_evm_gas(sdk);
            cached_state.remove(&sdk.unique_key());
            let exit_code = if result.result.is_ok() {
                ExitCode::Ok
            } else if result.result.is_revert() {
                ExitCode::Panic
            } else {
                ExitCode::Err
            };
            sdk.write(result.output);
            Err(exit_code)
        }
        InterpreterAction::SystemInterruption {
            code_hash,
            input,
            fuel_limit,
            state,
        } => {
            // Always sync gas before doing interruption
            evm.sync_evm_gas(sdk);
            sdk.insert_interruption_income(code_hash, input, fuel_limit, state);
            Err(ExitCode::InterruptionCalled)
        }
        InterpreterAction::NewFrame(_) => unreachable!("evm: frames can't be produced"),
    }
}

system_entrypoint!(main_entry, deploy_entry);
