use crate::{
    evm::{call::_evm_call, create::_evm_create},
    wasm::{call::_wasm_call, create::_wasm_create},
};
use fluentbase_sdk::{
    types::{EvmCallMethodInput, EvmCallMethodOutput, EvmCreateMethodInput, EvmCreateMethodOutput},
    SovereignAPI,
};
use fluentbase_types::BytecodeType;

pub fn _loader_call<SDK: SovereignAPI>(
    sdk: &mut SDK,
    input: EvmCallMethodInput,
) -> EvmCallMethodOutput {
    let (account, _) = sdk.account(&input.bytecode_address);
    let bytecode_type = sdk
        .preimage(&account.source_code_hash)
        .map(|v| BytecodeType::from_slice(v.as_ref()))
        .unwrap_or(BytecodeType::EVM);
    match bytecode_type {
        BytecodeType::EVM => _evm_call(sdk, input),
        BytecodeType::WASM => _wasm_call(sdk, input),
    }
}

pub fn _loader_create<SDK: SovereignAPI>(
    sdk: &mut SDK,
    input: EvmCreateMethodInput,
) -> EvmCreateMethodOutput {
    match BytecodeType::from_slice(input.bytecode.as_ref()) {
        BytecodeType::EVM => _evm_create(sdk, input),
        BytecodeType::WASM => _wasm_create(sdk, input),
    }
}
