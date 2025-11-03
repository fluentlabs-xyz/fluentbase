#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

extern crate alloc;
extern crate core;

use alloc::vec::Vec;
use core::convert::AsRef;
use fluentbase_evm::{
    bytecode::AnalyzedBytecode, gas, gas::Gas, opcodes::interruptable_instruction_table,
    types::InterruptionOutcome, EthVM, EthereumMetadata, ExecutionResult,
};
use fluentbase_sdk::{
    bincode, debug_log_ext, entrypoint, keccak256, Bytes, ContextReader, ExitCode, SharedAPI, B256,
    EVM_MAX_CODE_SIZE, FUEL_DENOM_RATE,
};
use fluentbase_types::{
    RuntimeInterruptionOutcomeV1, RuntimeNewFrameInputV1, SyscallInvocationParams,
};
use revm_interpreter::InterpreterAction;
use spin::MutexGuard;

/// Store EVM bytecode and its keccak256 hash in contract metadata.
/// Hash is written at offset 0, raw bytecode at offset 32.
pub(crate) fn commit_evm_bytecode<SDK: SharedAPI>(sdk: &mut SDK, evm_bytecode: Bytes) {
    let evm_code_hash = keccak256(evm_bytecode.as_ref());
    let analyzed_bytecode = AnalyzedBytecode::new(evm_bytecode, evm_code_hash);
    let raw_metadata = EthereumMetadata::Analyzed(analyzed_bytecode).write_to_bytes();
    sdk.write(raw_metadata.as_ref());
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

static SAVED_EVM_CONTEXT: spin::Once<spin::Mutex<Vec<EthVM>>> = spin::Once::new();

fn lock_evm_context<'a>() -> MutexGuard<'a, Vec<EthVM>> {
    let cached_state = SAVED_EVM_CONTEXT.call_once(|| {
        let result = Vec::new();
        spin::Mutex::new(result)
    });
    debug_log_ext!("cached_state.len={}", cached_state.lock().len());
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
    debug_log_ext!();
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
            debug_log_ext!(
                "new_frame_input.metadata.len={}",
                new_frame_input.metadata.len(),
            );
            let Some(analyzed_bytecode) = evm_bytecode_from_metadata(&new_frame_input.metadata)
            else {
                unreachable!("evm: a valid metadata must be provided")
            };
            (analyzed_bytecode, new_frame_input.input)
        } else {
            debug_log_ext!();
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
        debug_log_ext!();
        let (
            RuntimeInterruptionOutcomeV1 {
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
        eth_vm.interpreter.extend.interruption_outcome = Option::from(InterruptionOutcome {
            output,
            gas,
            exit_code,
        });
        eth_vm
    }
}

/// Deploy entry for EVM contracts.
/// Runs init bytecode, enforces EIP-3541 and EIP-170, charges CODEDEPOSIT gas,
/// then commits the resulting runtime bytecode to metadata.
pub fn deploy_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let exit_code = deploy_inner(&mut sdk, lock_evm_context());
    sdk.native_exit(exit_code);
}

fn deploy_inner<SDK: SharedAPI>(
    sdk: &mut SDK,
    mut cached_state: MutexGuard<Vec<EthVM>>,
) -> ExitCode {
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
                    return ExitCode::PrecompileError;
                } else if result.output.len() > EVM_MAX_CODE_SIZE {
                    return ExitCode::CreateContractSizeLimit;
                }
                let gas_for_code = result.output.len() as u64 * gas::CODEDEPOSIT;
                if !result.gas.record_cost(gas_for_code) {
                    return ExitCode::OutOfFuel;
                }
                let consumed_diff = result.chargeable_fuel();
                sdk.charge_fuel(consumed_diff);
                // We intentionally don't charge gas for these opcodes
                // to keep full compatibility with an EVM deployment process
                commit_evm_bytecode(sdk, result.output);
                ExitCode::Ok
            } else {
                let consumed_diff = result.chargeable_fuel();
                sdk.charge_fuel(consumed_diff);
                sdk.write(result.output.as_ref());
                if result.result.is_revert() {
                    ExitCode::Panic
                } else {
                    ExitCode::Err
                }
            }
        }
        InterpreterAction::SystemInterruption {
            code_hash,
            input,
            fuel_limit,
            state,
        } => {
            evm.sync_evm_gas(sdk);
            let syscall_params = SyscallInvocationParams {
                code_hash,
                input,
                fuel_limit: fuel_limit.unwrap_or(u64::MAX),
                state,
                fuel16_ptr: 0,
            }
            .encode();
            sdk.write(&syscall_params);
            ExitCode::InterruptionCalled
        }
        InterpreterAction::NewFrame(_) => unreachable!("frames can't be produced"),
    }
}

/// Main entry for executing deployed EVM bytecode.
/// Loads analyzed code from metadata, runs EthVM with call input, settles fuel,
/// and writes the returned data.
pub fn main_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let exit_code = main_inner(&mut sdk, lock_evm_context());
    sdk.native_exit(exit_code);
}

fn main_inner<SDK: SharedAPI>(sdk: &mut SDK, mut cached_state: MutexGuard<Vec<EthVM>>) -> ExitCode {
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
    debug_log_ext!(
        "evm.interpreter.bytecode.bytecode({})={:x?}",
        evm.interpreter.bytecode.bytecode().len(),
        evm.interpreter.bytecode.bytecode(),
    );
    match evm.run_step(&instruction_table, sdk) {
        InterpreterAction::Return(result) => {
            evm.sync_evm_gas(sdk);
            _ = cached_state.pop();
            sdk.write(result.output.as_ref());
            if result.result.is_ok() {
                ExitCode::Ok
            } else if result.result.is_revert() {
                ExitCode::Panic
            } else {
                ExitCode::Err
            }
        }
        InterpreterAction::SystemInterruption {
            code_hash,
            input,
            fuel_limit,
            state,
        } => {
            debug_log_ext!();
            evm.sync_evm_gas(sdk);
            let syscall_params = SyscallInvocationParams {
                code_hash,
                input,
                fuel_limit: fuel_limit.unwrap_or(u64::MAX),
                state,
                fuel16_ptr: 0,
            }
            .encode();
            sdk.write(&syscall_params);
            ExitCode::InterruptionCalled
        }
        InterpreterAction::NewFrame(_) => unreachable!("frames can't be produced"),
    }
}

entrypoint!(main_entry, deploy_entry);

#[cfg(test)]
mod tests {
    use crate::{deploy_entry, main_entry};
    use core::str::from_utf8;
    use fluentbase_sdk::{hex, Address, ContractContextV1, PRECOMPILE_EVM_RUNTIME, U256};
    use fluentbase_testing::HostTestingContext;

    #[ignore]
    #[test]
    fn test_deploy_greeting() {
        const CONTRACT_ADDRESS: Address = Address::new([
            189, 119, 4, 22, 163, 52, 95, 145, 228, 179, 69, 118, 203, 128, 74, 87, 111, 164, 142,
            177,
        ]);
        let mut sdk = HostTestingContext::default()
            .with_contract_context(ContractContextV1 {
                address: CONTRACT_ADDRESS,
                bytecode_address: CONTRACT_ADDRESS,
                caller: Address::ZERO,
                is_static: false,
                value: U256::ZERO,
                gas_limit: 1_000_000,
            })
            .with_gas_limit(1_000_000);
        sdk.set_ownable_account_address(PRECOMPILE_EVM_RUNTIME);
        // deploy
        {
            sdk = sdk.with_input(hex!("60806040526105ae806100115f395ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e01c80633b2e97481461004357806345773e4e1461007357806348b8bcc314610091575b5f80fd5b61005d600480360381019061005891906102e5565b6100af565b60405161006a919061039a565b60405180910390f35b61007b6100dd565b604051610088919061039a565b60405180910390f35b61009961011a565b6040516100a6919061039a565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100d58161012f565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101298161012f565b91505090565b60605f8203610175576040518060400160405280600181526020017f30000000000000000000000000000000000000000000000000000000000000008152509050610282565b5f8290505f5b5f82146101a457808061018d906103f0565b915050600a8261019d9190610464565b915061017b565b5f8167ffffffffffffffff8111156101bf576101be610494565b5b6040519080825280601f01601f1916602001820160405280156101f15781602001600182028036833780820191505090505b5090505b5f851461027b578180610207906104c1565b925050600a8561021791906104e8565b60306102239190610518565b60f81b8183815181106102395761023861054b565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a856102749190610464565b94506101f5565b8093505050505b919050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102b48261028b565b9050919050565b6102c4816102aa565b81146102ce575f80fd5b50565b5f813590506102df816102bb565b92915050565b5f602082840312156102fa576102f9610287565b5b5f610307848285016102d1565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f5b8381101561034757808201518184015260208101905061032c565b5f8484015250505050565b5f601f19601f8301169050919050565b5f61036c82610310565b610376818561031a565b935061038681856020860161032a565b61038f81610352565b840191505092915050565b5f6020820190508181035f8301526103b28184610362565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f6103fa826103e7565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff820361042c5761042b6103ba565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f61046e826103e7565b9150610479836103e7565b92508261048957610488610437565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104cb826103e7565b91505f82036104dd576104dc6103ba565b5b600182039050919050565b5f6104f2826103e7565b91506104fd836103e7565b92508261050d5761050c610437565b5b828206905092915050565b5f610522826103e7565b915061052d836103e7565b9250828201905080821115610545576105446103ba565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea2646970667358221220feebf5ace29c3c3146cb63bf7ca9009c2005f349075639d267cfbd817adde3e564736f6c63430008180033"));
            deploy_entry(sdk.clone());
        }
        // main
        {
            let sdk = sdk.with_input(hex!("45773e4e"));
            main_entry(sdk.clone());
            let bytes = &sdk.take_output()[64..75];
            assert_eq!("Hello World", from_utf8(bytes.as_ref()).unwrap());
        }
    }
}
