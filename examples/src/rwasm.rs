use fluentbase_rwasm::rwasm::Compiler;
use fluentbase_sdk::{SysPlatformSDK, SDK};

pub fn main() {
    let mut wasm_bytecode: [u8; 1024] = [0; 1024];
    SDK::sys_read_slice(&mut wasm_bytecode, 0);
    let mut compiler = Compiler::new(&wasm_bytecode).unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();
    SDK::sys_write_slice(rwasm_bytecode.as_slice());
}
