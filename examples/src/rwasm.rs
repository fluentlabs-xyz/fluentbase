use fluentbase_rwasm::rwasm::{Compiler, CompilerConfig};
use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};

pub fn main() {
    let wasm_bytecode: [u8; 0x600] = [0; 0x600];
    let size = LowLevelSDK::sys_input_size() as usize;
    let mut compiler = Compiler::new(&wasm_bytecode[0..size], CompilerConfig::default()).unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();
    let mut ctx = ExecutionContext::default();
    ctx.return_and_exit(rwasm_bytecode.as_slice(), 0);
}
