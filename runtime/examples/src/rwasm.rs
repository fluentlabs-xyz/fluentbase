use fluentbase_rwasm::rwasm::Compiler;
use fluentbase_sdk::{evm_return_slice, sys_read};

pub fn rwasm() {
    let mut wasm_bytecode: [u8; 1024] = [0; 1024];
    let n = sys_read(wasm_bytecode.as_mut_ptr(), 0, 1024);
    assert_ne!(n, 1024);
    let mut compiler = Compiler::new(&wasm_bytecode).unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();
    evm_return_slice(rwasm_bytecode.as_slice());
}
