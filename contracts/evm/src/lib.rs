#![cfg_attr(target_arch = "wasm32", no_std)]

mod instructions;
mod utils;

extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use crate::instructions::exec_evm_bytecode;
use core::ops::Neg;
use fluentbase_sdk::{
    debug_log,
    func_entrypoint,
    Bytes,
    ContractContextReader,
    ExitCode,
    SharedAPI,
    EVM_CODE_HASH_SLOT,
    FUEL_DENOM_RATE,
    KECCAK_EMPTY,
    U256,
};
use revm_interpreter::{
    gas,
    primitives::Bytecode,
    InstructionResult,
    InterpreterResult,
    MAX_CODE_SIZE,
};

fn handle_not_ok_result<SDK: SharedAPI>(mut sdk: SDK, result: InterpreterResult) {
    // calculate the final gas charge for the call
    debug_log!("refund: {}", result.gas.refunded());
    // result.gas.set_final_refund(true);
    // debug_log!("final_refund: {}", result.gas.refunded());
    debug_log!(
        "final_gas: {}",
        result.gas.spent() - result.gas.refunded() as u64
    );
    sdk.sync_evm_gas(result.gas.remaining(), result.gas.refunded());
    // sdk.charge_fuel((result.gas.spent() - result.gas.refunded() as u64) * FUEL_DENOM_RATE);
    sdk.write(result.output.as_ref());
    // we encode EVM error as negative from our error code
    debug_log!("result_code: {:?}", result.result);
    if result.is_revert() {
        sdk.exit(ExitCode::Panic.into_i32());
    }
    sdk.exit((result.result as i32).neg());
}

pub fn deploy<SDK: SharedAPI>(mut sdk: SDK) {
    let input: Bytes = sdk.input().into();
    let evm_bytecode = Bytecode::new_raw(input);

    let gas_limit = sdk.fuel() / FUEL_DENOM_RATE;
    debug_log!("gas_limit: {:?}", gas_limit);

    let mut result = exec_evm_bytecode(&mut sdk, evm_bytecode, Bytes::default(), gas_limit);

    debug_log!("result: {:?}", result.result);
    debug_log!("gas: {:?}", result.gas);
    debug_log!("gas_spent: {:?}", result.gas.spent());
    debug_log!("output: {:?}", result.output);
    debug_log!("output_len: {:?}", result.output.len());

    if !result.is_ok() {
        return handle_not_ok_result(sdk, result);
    }

    // EIP-3541 and EIP-170 checks
    if result.output.first() == Some(&0xEF) {
        sdk.exit((InstructionResult::CreateContractStartingWithEF as i32).neg());
    } else if result.output.len() > MAX_CODE_SIZE {
        sdk.exit((InstructionResult::CreateContractSizeLimit as i32).neg());
    }

    let gas_for_code = result.output.len() as u64 * gas::CODEDEPOSIT;
    debug_log!("gas_for_code: {}", gas_for_code);
    if !result.gas.record_cost(gas_for_code) {
        sdk.charge_fuel(u64::MAX);
    }

    // calculate the final gas charge for the call
    debug_log!("refund: {}", result.gas.refunded());
    // result.gas.set_final_refund(true);
    // debug_log!("final_refund: {}", result.gas.refunded());
    debug_log!(
        "final_gas: {}",
        result.gas.spent() - result.gas.refunded() as u64
    );
    sdk.sync_evm_gas(result.gas.remaining(), result.gas.refunded());
    // sdk.charge_fuel((result.gas.spent() - result.gas.refunded() as u64) * FUEL_DENOM_RATE);

    // we intentionally don't charge gas for these opcodes
    // to keep full compatibility with an EVM deployment process
    let result = sdk.write_preimage(result.output);
    let code_hash = result.data;
    _ = sdk.write_storage(EVM_CODE_HASH_SLOT.into(), code_hash.into());
}

pub fn main<SDK: SharedAPI>(mut sdk: SDK) {
    debug_log!("contract_address: {:?}", sdk.context().contract_address());
    let bytecode_address = sdk.context().contract_bytecode_address();
    debug_log!("contract_bytecode_address: {:?}", bytecode_address);
    debug_log!(
        "contract_is_static: {:?}",
        sdk.context().contract_is_static()
    );
    let code_hash =
        sdk.delegated_storage(&bytecode_address, &Into::<U256>::into(EVM_CODE_HASH_SLOT));
    // TODO(dmitry123): "do we want to have this optimized during the creation of the frame?"
    if code_hash.data == U256::ZERO || Into::<U256>::into(KECCAK_EMPTY) == code_hash.data {
        debug_log!(
            "skipping EVM execution due to empty code: {}",
            code_hash.data
        );
        return;
    }
    debug_log!("code_hash: {:?}", code_hash.data);
    let evm_bytecode = sdk.preimage(&code_hash.data.into());
    debug_log!("preimage_size: {:?}", evm_bytecode.len());
    let evm_bytecode = Bytecode::new_raw(evm_bytecode);
    let input: Bytes = sdk.input().into();
    debug_log!("input_size: {:?}", input.len());

    let gas_limit = sdk.fuel() / FUEL_DENOM_RATE;
    debug_log!("gas_limit: {:?}", gas_limit);

    let result = exec_evm_bytecode(&mut sdk, evm_bytecode, input, gas_limit);

    debug_log!("result: {:?}", result.result);
    debug_log!("gas: {:?}", result.gas);
    debug_log!("gas_spent: {:?}", result.gas.spent());
    debug_log!("output: {:?}", result.output);

    if !result.is_ok() {
        return handle_not_ok_result(sdk, result);
    }

    // calculate the final gas charge for the call
    debug_log!("refund: {}", result.gas.refunded());
    // result.gas.set_final_refund(true);
    // debug_log!("final_refund: {}", result.gas.refunded());
    debug_log!(
        "final_gas: {}",
        result.gas.spent() - result.gas.refunded() as u64
    );

    sdk.sync_evm_gas(result.gas.remaining(), result.gas.refunded());
    // sdk.charge_fuel((result.gas.spent() - result.gas.refunded() as u64) * FUEL_DENOM_RATE);
    sdk.write(result.output.as_ref());
}

func_entrypoint!(main, deploy);

#[cfg(test)]
mod tests {
    use super::*;
    use core::str::from_utf8;
    use fluentbase_sdk::{testing::TestingContext, Address, ContractContextV1, U256};
    use revm_interpreter::primitives::hex;

    #[test]
    fn test_deploy_greeting() {
        let mut sdk = TestingContext::default().with_contract_context(ContractContextV1 {
            address: Address::from([
                189, 119, 4, 22, 163, 52, 95, 145, 228, 179, 69, 118, 203, 128, 74, 87, 111, 164,
                142, 177,
            ]),
            bytecode_address: Default::default(),
            caller: Address::ZERO,
            is_static: false,
            value: U256::ZERO,
        });
        // deploy
        {
            sdk = sdk.with_input(hex!("60806040526105ae806100115f395ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e01c80633b2e97481461004357806345773e4e1461007357806348b8bcc314610091575b5f80fd5b61005d600480360381019061005891906102e5565b6100af565b60405161006a919061039a565b60405180910390f35b61007b6100dd565b604051610088919061039a565b60405180910390f35b61009961011a565b6040516100a6919061039a565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100d58161012f565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101298161012f565b91505090565b60605f8203610175576040518060400160405280600181526020017f30000000000000000000000000000000000000000000000000000000000000008152509050610282565b5f8290505f5b5f82146101a457808061018d906103f0565b915050600a8261019d9190610464565b915061017b565b5f8167ffffffffffffffff8111156101bf576101be610494565b5b6040519080825280601f01601f1916602001820160405280156101f15781602001600182028036833780820191505090505b5090505b5f851461027b578180610207906104c1565b925050600a8561021791906104e8565b60306102239190610518565b60f81b8183815181106102395761023861054b565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a856102749190610464565b94506101f5565b8093505050505b919050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102b48261028b565b9050919050565b6102c4816102aa565b81146102ce575f80fd5b50565b5f813590506102df816102bb565b92915050565b5f602082840312156102fa576102f9610287565b5b5f610307848285016102d1565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f5b8381101561034757808201518184015260208101905061032c565b5f8484015250505050565b5f601f19601f8301169050919050565b5f61036c82610310565b610376818561031a565b935061038681856020860161032a565b61038f81610352565b840191505092915050565b5f6020820190508181035f8301526103b28184610362565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f6103fa826103e7565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff820361042c5761042b6103ba565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f61046e826103e7565b9150610479836103e7565b92508261048957610488610437565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104cb826103e7565b91505f82036104dd576104dc6103ba565b5b600182039050919050565b5f6104f2826103e7565b91506104fd836103e7565b92508261050d5761050c610437565b5b828206905092915050565b5f610522826103e7565b915061052d836103e7565b9250828201905080821115610545576105446103ba565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea2646970667358221220feebf5ace29c3c3146cb63bf7ca9009c2005f349075639d267cfbd817adde3e564736f6c63430008180033"));
            deploy(sdk.clone());
        }
        // main
        {
            let sdk = sdk.with_input(hex!("45773e4e"));
            main(sdk.clone());
            let bytes = &sdk.take_output()[64..75];
            assert_eq!("Hello World", from_utf8(bytes.as_ref()).unwrap());
        }
    }
}
