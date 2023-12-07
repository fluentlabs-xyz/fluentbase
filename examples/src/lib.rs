#![no_std]
extern crate alloc;
extern crate fluentbase_sdk;

#[cfg(feature = "greeting")]
mod greeting;
#[cfg(feature = "keccak256")]
mod keccak256;
#[cfg(feature = "panic")]
mod panic;
#[cfg(feature = "poseidon")]
mod poseidon;
#[cfg(feature = "rwasm")]
mod rwasm;
#[cfg(feature = "secp256k1")]
mod secp256k1;
#[cfg(feature = "state")]
mod state;
#[cfg(feature = "storage")]
mod storage;

#[no_mangle]
pub extern "C" fn deploy() {
    #[cfg(feature = "state")]
    state::deploy();
    #[cfg(feature = "storage")]
    storage::deploy();
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
    panic::main();
    #[cfg(feature = "rwasm")]
    rwasm::main();
    #[cfg(feature = "state")]
    state::main();
    #[cfg(feature = "storage")]
    storage::main();
}
