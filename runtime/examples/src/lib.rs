#![no_std]
extern crate alloc;

#[cfg(feature = "evm")]
mod evm;
#[cfg(feature = "greeting")]
mod greeting;
#[cfg(feature = "keccak256")]
mod keccak256;
#[cfg(feature = "poseidon")]
mod poseidon;
#[cfg(feature = "rwasm")]
mod rwasm;
#[cfg(feature = "secp256k1")]
mod secp256k1;
#[cfg(feature = "wasi")]
mod wasi;

#[cfg(feature = "panic")]
fn panic() {
    panic!("its time to panic");
}

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
    greeting::main();
    #[cfg(feature = "keccak256")]
    keccak256::main();
    #[cfg(feature = "poseidon")]
    poseidon::main();
    #[cfg(feature = "secp256k1")]
    secp256k1::main();
    #[cfg(feature = "panic")]
    panic();
    #[cfg(feature = "evm_verify_block_rlps")]
    evm_verify_block_rlps();
    #[cfg(feature = "rwasm")]
    rwasm::main();
    #[cfg(feature = "rwasm_compile_with_linker_test")]
    rwasm_compile_with_linker_test();
    #[cfg(feature = "evm")]
    evm::main();
    #[cfg(feature = "wasi")]
    wasi::main();
}
