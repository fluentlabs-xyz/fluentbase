use fluentbase_rwasm::rwasm::{Compiler, CompilerConfig};
use fluentbase_sdk::{SysPlatformSDK, SDK};

pub fn main() {
    let mut wasm_bytecode: [u8; 0x600] = [0; 0x600];
    let size = SDK::sys_read(&mut wasm_bytecode, 0) as usize;
    let mut compiler = Compiler::new(&wasm_bytecode[0..size], CompilerConfig::default()).unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();
    SDK::sys_write(rwasm_bytecode.as_slice());
}
