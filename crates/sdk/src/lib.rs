#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
extern crate core;

pub extern crate rwasm as rwasm_core;

mod address;
mod allocator;
pub mod constructor;
pub mod entrypoint;
pub mod leb128;
mod macros;
pub mod panic;
pub mod shared;
pub mod storage;
#[deprecated(note = "Use `fluentbase_sdk::storage` instead", since = "0.4.5-dev")]
pub mod storage_legacy;
pub mod syscall;
mod types;

pub use address::*;
pub use allocator::*;
pub use fluentbase_codec as codec;
pub use fluentbase_crypto as crypto;
// pub mod crypto {
//     use fluentbase_types::B256;
//
//     pub fn crypto_keccak256(data: &[u8]) -> B256 {
//         unimplemented!()
//     }
//     pub fn crypto_sha256(data: &[u8]) -> B256 {
//         unimplemented!()
//     }
//     pub fn crypto_poseidon(parameters: u32, endianness: u32, data: &[u8]) -> B256 {
//         unimplemented!()
//     }
//     pub fn crypto_blake3(data: &[u8]) -> B256 {
//         unimplemented!()
//     }
// }
pub use fluentbase_sdk_derive as derive;
pub use fluentbase_types::*;
pub use types::*;

#[cfg(feature = "std")]
#[macro_export]
macro_rules! include_this_wasm {
    () => {
        include_bytes!(env!("FLUENTBASE_WASM_ARTIFACT_PATH"))
    };
}
