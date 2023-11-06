#![no_std]

extern crate alloc;

use fluentbase_sdk::{mpt_open_, rwasm_compile, sys_read, sys_write, zktrie_open_};

#[cfg(feature = "evm")]
mod evm;
#[cfg(feature = "rwasm")]
mod rwasm;
#[cfg(feature = "wasi")]
mod wasi;

#[cfg(feature = "greeting")]
fn greeting() {
    let mut input: [u8; 3] = [0; 3];
    sys_read(input.as_mut_ptr(), 0, 3);
    let sum = input.iter().fold(0u32, |r, v| r + *v as u32);
    let sum_bytes = sum.to_be_bytes();
    sys_write(sum_bytes.as_ptr() as u32, sum_bytes.len() as u32);
}

#[cfg(feature = "evm_block_number")]
fn evm_block_number() {
    let mut input = [0u8; 11];
    sys_read(input.as_mut_ptr(), 0, input.len() as u32);
    const EXPECTED_LEN: i32 = 32;
    const OUTPUT_OFFSET: i32 = 0;
    let len = evm_block_number_(input.as_mut_ptr() as i32, input.len() as i32, OUTPUT_OFFSET);
    if len != EXPECTED_LEN {
        panic!("output len!={EXPECTED_LEN:?}");
    }
}

fn panic() {
    panic!("its time to panic");
}

#[cfg(feature = "zktrie_open_test")]
fn zktrie_open_test() {
    zktrie_open_();
}
#[cfg(feature = "mpt_open_test")]
fn mpt_open_test() {
    mpt_open_();
}

#[cfg(feature = "revm_db_test")]
fn revm_db_test() {}

#[cfg(feature = "rwasm_compile_with_linker_test")]
pub fn rwasm_compile_with_linker_test() {
    const WB_START_OFFSET: usize = 0;
    const WB_LEN: usize = 628;
    const OUT_LEN_EXPECTED: usize = 954;
    let mut wb = [0u8; WB_START_OFFSET + WB_LEN];
    sys_read(wb.as_mut_ptr(), WB_START_OFFSET as u32, WB_LEN as u32);
    let mut output = [0u8; OUT_LEN_EXPECTED];
    let out_len = rwasm_compile(&wb, &mut output);
    if out_len != OUT_LEN_EXPECTED as i32 {
        panic!("out_len!=OUT_LEN_EXPECTED");
    }
}

#[no_mangle]
pub extern "C" fn main() {
    #[cfg(feature = "greeting")]
    greeting();
    #[cfg(feature = "zktrie_open_test")]
    zktrie_open_test();
    #[cfg(feature = "mpt_open_test")]
    mpt_open_test();
    #[cfg(feature = "panic")]
    panic();
    #[cfg(feature = "evm_verify_block_rlps")]
    evm_verify_block_rlps();
    #[cfg(feature = "rwasm")]
    crate::rwasm::rwasm();
    #[cfg(feature = "rwasm_compile_with_linker_test")]
    rwasm_compile_with_linker_test();
    #[cfg(feature = "evm")]
    crate::evm::evm();
    #[cfg(feature = "wasi")]
    crate::wasi::wasi();
}
