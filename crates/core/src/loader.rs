use crate::{
    evm::{call::_evm_call, create::_evm_create},
    wasm::{call::_wasm_call, create::_wasm_create},
};
use fluentbase_sdk::{
    types::{EvmCallMethodInput, EvmCallMethodOutput, EvmCreateMethodInput, EvmCreateMethodOutput},
    SovereignAPI,
};
use fluentbase_types::{BytecodeType, ContextReader};

pub fn _loader_call<CTX: ContextReader, SDK: SovereignAPI>(
    ctx: &CTX,
    sdk: &SDK,
    input: EvmCallMethodInput,
) -> EvmCallMethodOutput {
    let (account, _) = sdk.account(&input.callee);
    let source_code = sdk.preimage(&account.source_code_hash);
    match BytecodeType::from_slice(source_code.as_ref()) {
        BytecodeType::EVM => _evm_call(ctx, sdk, input),
        BytecodeType::WASM => _wasm_call(ctx, sdk, input),
    }
}

pub fn _loader_create<CTX: ContextReader, SDK: SovereignAPI>(
    ctx: &CTX,
    sdk: &SDK,
    input: EvmCreateMethodInput,
) -> EvmCreateMethodOutput {
    match BytecodeType::from_slice(input.bytecode.as_ref()) {
        BytecodeType::EVM => _evm_create(ctx, sdk, input),
        BytecodeType::WASM => _wasm_create(ctx, sdk, input),
    }
}
