#![no_std]

extern crate alloc;

use alloc::vec;
use fluentbase_sdk::{evm_return_slice, sys_read, sys_write, zktrie_open_};

#[cfg(feature = "evm")]
mod evm;
#[cfg(feature = "rwasm")]
mod rwasm;
#[cfg(feature = "wasi")]
mod wasi;

fn greeting() {
    let mut input: [u8; 3] = [0; 3];
    sys_read(input.as_mut_ptr(), 0, 3);
    let sum = input.iter().fold(0u32, |r, v| r + *v as u32);
    let sum_bytes = sum.to_be_bytes();
    evm_return_slice(&sum_bytes)
}

fn panic() {
    panic!("its time to panic");
}

pub const HASHLEN: usize = 32;
pub const FIELDSIZE: usize = 32;
pub const ACCOUNTFIELDS: usize = 5;
pub const ACCOUNTSIZE: usize = FIELDSIZE * ACCOUNTFIELDS;
const ROOTSIZE: usize = FIELDSIZE;
const KEYSIZE: usize = 20;
fn zktrie_open_test() {
    const ACCOUNTS_COUNT: usize = 1;

    let root_offset = 0;
    let keys_offset = root_offset + ROOTSIZE;
    let keys_size = KEYSIZE * ACCOUNTS_COUNT;
    let leafs_offset = keys_offset + keys_size;
    let leafs_size = ACCOUNTSIZE * ACCOUNTS_COUNT;

    let len = ROOTSIZE + keys_size + leafs_size;
    let mut v = vec![0; len];
    sys_read(v.as_mut_ptr(), root_offset as u32, len as u32);
    // sys_write(root_offset as u32, len as u32);

    zktrie_open_(
        v.as_mut_ptr() as i32,
        ROOTSIZE as i32,
        v.as_mut_ptr() as i32 + keys_offset as i32,
        v.as_mut_ptr() as i32 + leafs_offset as i32,
        ACCOUNTS_COUNT as i32,
    );
}

#[no_mangle]
pub extern "C" fn main() {
    #[cfg(feature = "greeting")]
    greeting();
    #[cfg(feature = "zktrie_open_test")]
    zktrie_open_test();
    #[cfg(feature = "panic")]
    panic();
    #[cfg(feature = "rwasm")]
    crate::rwasm::rwasm();
    #[cfg(feature = "evm")]
    crate::evm::evm();
    #[cfg(feature = "wasi")]
    crate::wasi::wasi();
}
