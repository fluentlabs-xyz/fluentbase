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
    #[cfg(feature = "rwasm")]
    rwasm::main();
    #[cfg(feature = "evm")]
    evm::main();
    #[cfg(feature = "wasi")]
    wasi::main();
}
