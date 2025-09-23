#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

extern crate alloc;
extern crate core;

use fluentbase_evm::{bytecode::AnalyzedBytecode, gas, EthVM, EthereumMetadata, ExecutionResult};
use fluentbase_sdk::{
    entrypoint, keccak256, Bytes, ContextReader, ExitCode, SharedAPI, B256, EVM_MAX_CODE_SIZE,
};

/// Store EVM bytecode and its keccak256 hash in contract metadata.
/// Hash is written at offset 0, raw bytecode at offset 32.
pub(crate) fn commit_evm_bytecode<SDK: SharedAPI>(sdk: &mut SDK, evm_bytecode: Bytes) {
    let contract_address = sdk.context().contract_address();
    let evm_code_hash = keccak256(evm_bytecode.as_ref());
    let analyzed_bytecode = AnalyzedBytecode::new(evm_bytecode, evm_code_hash);
    let raw_metadata = EthereumMetadata::Analyzed(analyzed_bytecode).write_to_bytes();
    sdk.metadata_write(&contract_address, 0, raw_metadata)
        .unwrap();
}

/// Load analyzed EVM bytecode from contract metadata.
/// Returns None if metadata is empty or code hash is zero/KECCAK_EMPTY.
pub(crate) fn load_evm_bytecode<SDK: SharedAPI>(sdk: &SDK) -> Option<AnalyzedBytecode> {
    // We use bytecode address because contract can be called using DELEGATECALL
    let bytecode_address = sdk.context().contract_bytecode_address();
    // Read metadata size, if it's zero, then an account is not assigned to the EVM runtime
    let (metadata_size, is_account_ownable, _, _) = sdk.metadata_size(&bytecode_address).unwrap();
    if !is_account_ownable {
        return None;
    }
    let metadata = sdk
        .metadata_copy(&bytecode_address, 0, metadata_size)
        .unwrap();
    // Get EVM bytecode from metadata
    Some(match EthereumMetadata::read_from_bytes(&metadata)? {
        EthereumMetadata::Legacy(bytecode) => {
            AnalyzedBytecode::new(bytecode.bytecode, bytecode.hash)
        }
        EthereumMetadata::Analyzed(bytecode) => bytecode,
    })
}

/// Propagate a non-successful interpreter result to the host:
/// charge final fuel delta, write output, and exit with Err/Panic.
fn handle_not_ok_result<SDK: SharedAPI>(mut sdk: SDK, result: ExecutionResult) {
    let (consumed_diff, refund_diff) = result.chargeable_fuel_and_refund();
    sdk.charge_fuel_manually(consumed_diff, refund_diff);
    sdk.write(result.output.as_ref());
    sdk.native_exit(if result.result.is_revert() {
        ExitCode::Panic
    } else {
        ExitCode::Err
    });
}

/// Deploy entry for EVM contracts.
/// Runs init bytecode, enforces EIP-3541 and EIP-170, charges CODEDEPOSIT gas,
/// then commits the resulting runtime bytecode to metadata.
pub fn deploy_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let input: Bytes = sdk.input().into();
    let analyzed_bytecode = AnalyzedBytecode::new(input, B256::ZERO);

    let mut result =
        EthVM::new(sdk.context(), Bytes::default(), analyzed_bytecode).run_the_loop(&mut sdk);
    if !result.result.is_ok() {
        return handle_not_ok_result(sdk, result);
    }

    // EIP-3541 and EIP-170 checks
    if result.output.first() == Some(&0xEF) {
        sdk.native_exit(ExitCode::PrecompileError);
    } else if result.output.len() > EVM_MAX_CODE_SIZE {
        sdk.native_exit(ExitCode::PrecompileError);
    }
    let gas_for_code = result.output.len() as u64 * gas::CODEDEPOSIT;
    if !result.gas.record_cost(gas_for_code) {
        sdk.native_exit(ExitCode::OutOfFuel);
    }

    let (consumed_diff, refund_diff) = result.chargeable_fuel_and_refund();
    sdk.charge_fuel_manually(consumed_diff, refund_diff);

    // We intentionally don't charge gas for these opcodes
    // to keep full compatibility with an EVM deployment process
    commit_evm_bytecode(&mut sdk, result.output);
}

/// Main entry for executing deployed EVM bytecode.
/// Loads analyzed code from metadata, runs EthVM with call input, settles fuel,
/// and writes the returned data.
pub fn main_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let Some(analyzed_bytecode) = load_evm_bytecode(&sdk) else {
        return;
    };
    let result =
        EthVM::new(sdk.context(), sdk.bytes_input(), analyzed_bytecode).run_the_loop(&mut sdk);
    if !result.result.is_ok() {
        return handle_not_ok_result(sdk, result);
    }
    let (consumed_diff, refund_diff) = result.chargeable_fuel_and_refund();
    sdk.charge_fuel_manually(consumed_diff, refund_diff);
    sdk.write(result.output.as_ref());
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
