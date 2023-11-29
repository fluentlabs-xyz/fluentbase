use fluentbase_rwasm::rwasm::Compiler;
use fluentbase_sdk::{SysPlatformSDK, SDK};

pub fn main() {
    let mut wasm_bytecode: [u8; 1024] = [0; 1024];
    SDK::sys_read(&mut wasm_bytecode, 0);
    let mut compiler = Compiler::new(&wasm_bytecode, false).unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();
    SDK::sys_write(rwasm_bytecode.as_slice());
}
