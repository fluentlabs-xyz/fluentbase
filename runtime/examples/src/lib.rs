#![no_std]

#[cfg(feature = "evm")]
mod evm;
#[cfg(feature = "rwasm")]
mod rwasm;
#[cfg(feature = "wasi")]
mod wasi;

use fluentbase_sdk::{evm_keccak256, evm_return_slice, sys_read};

fn greeting() {
    let mut input: [u8; 3] = [0; 3];
    sys_read(input.as_mut_ptr(), 0, 3);
    let sum = input.iter().fold(0u32, |r, v| r + *v as u32);
    let sum_bytes = sum.to_be_bytes();
    evm_return_slice(&sum_bytes)
}

fn keccak256() {
    return;
    let data = b"Hello, World!";
    // sys_read(data.as_mut_ptr(), offset, size);
    // let sum = input.iter().fold(0u32, |r, v| r + *v as u32);
    //let sum_bytes = sum.to_be_bytes();
    evm_keccak256(data)
}

fn panic() {
    panic!("its time to panic");
}

#[no_mangle]
pub extern "C" fn main() {
    #[cfg(feature = "keccak256")]
    keccak256();
    return;
    #[cfg(feature = "greeting")]
    greeting();
    #[cfg(feature = "panic")]
    panic();
    #[cfg(feature = "rwasm")]
    crate::rwasm::rwasm();
    #[cfg(feature = "evm")]
    crate::evm::evm();
    #[cfg(feature = "wasi")]
    crate::wasi::wasi();
}
