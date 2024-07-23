use crate::{
    evm::{call::_evm_call, create::_evm_create},
    wasm::{call::_wasm_call, create::_wasm_create},
};
use fluentbase_sdk::{
    types::{
        EvmCallMethodInput,
        EvmCallMethodOutput,
        EvmCreateMethodInput,
        EvmCreateMethodOutput,
        FvmCallMethodInput,
        FvmCallMethodOutput,
        FvmCreateMethodInput,
        FvmCreateMethodOutput,
    },
    AccountManager,
    ContextReader,
};
use fluentbase_types::BytecodeType;

pub fn _loader_call<CR: ContextReader, AM: AccountManager>(
    cr: &CR,
    am: &AM,
    input: EvmCallMethodInput,
) -> EvmCallMethodOutput {
    let (account, _) = am.account(input.callee);
    let source_code = am.preimage(&account.source_code_hash);
    match BytecodeType::from_slice(source_code.as_ref()) {
        BytecodeType::EVM => _evm_call(cr, am, input),
        BytecodeType::WASM => _wasm_call(cr, am, input),
        t => panic!("unsupported bytecode type {:?} for evm call loader", &t),
    }
}

pub fn _loader_create<CR: ContextReader, AM: AccountManager>(
    cr: &CR,
    am: &AM,
    input: EvmCreateMethodInput,
) -> EvmCreateMethodOutput {
    match BytecodeType::from_slice(input.bytecode.as_ref()) {
        BytecodeType::EVM => _evm_create(cr, am, input),
        BytecodeType::WASM => _wasm_create(cr, am, input),
        t => panic!("unsupported bytecode type {:?} for evm create loader", &t),
    }
}
