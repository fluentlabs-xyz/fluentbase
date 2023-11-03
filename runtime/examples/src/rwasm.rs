use fluentbase_rwasm::rwasm::Compiler;
use fluentbase_sdk::{rwasm_compile, sys_read, sys_write};

pub fn main() {
    let mut wasm_bytecode: [u8; 1024] = [0; 1024];
    sys_read(wasm_bytecode.as_mut_ptr(), 0, 1024);
    let mut compiler = Compiler::new(&wasm_bytecode).unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();
    sys_write(rwasm_bytecode.as_ptr() as u32, rwasm_bytecode.len() as u32);
}
