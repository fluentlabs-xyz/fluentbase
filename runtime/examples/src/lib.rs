#![no_std]

// use fluentbase_rwasm::rwasm::Compiler;
use fluentbase_sdk::{evm_return_slice, sys_read};

fn greeting() {
    let mut input: [u8; 3] = [0; 3];
    let n = sys_read(input.as_mut_ptr(), 0, 3);
    let sum = input.iter().fold(0u32, |r, v| r + *v as u32);
    let sum_bytes = sum.to_be_bytes();
    evm_return_slice(&sum_bytes)
}

fn panic() {
    panic!("its time to panic");
}

// fn translator() {
//     let mut wasm_bytecode: [u8; 1024] = [0; 1024];
//     let n = sys_read(wasm_bytecode.as_mut_ptr(), 0, 1024);
//     assert_ne!(n, 1024);
//     let mut compiler = Compiler::new(&wasm_bytecode).unwrap();
//     let rwasm_bytecode = compiler.finalize().unwrap();
//     evm_return_slice(rwasm_bytecode.as_slice());
// }

#[no_mangle]
pub extern "C" fn main() {
    #[cfg(feature = "greeting")]
    greeting();
    #[cfg(feature = "panic")]
    panic();
    // #[cfg(feature = "translator")]
    // translator();
}
