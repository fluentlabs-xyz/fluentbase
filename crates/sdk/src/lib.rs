//! Public SDK for writing Fluentbase contracts, including entrypoints, storage, and syscall helpers.
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
extern crate core;

pub extern crate rwasm as rwasm_core;

mod address;
mod allocator;
pub mod constructor;
pub mod debug;
pub mod entrypoint;
pub mod leb128;
mod macros;
pub mod panic;
pub mod shared;
pub mod storage;
// #[deprecated(note = "Use `fluentbase_sdk::storage` instead", since = "0.4.5-dev")]
pub mod storage_legacy;
pub mod syscall;
pub mod system;
mod types;
#[cfg(feature = "universal-token")]
pub mod universal_token;

pub use address::*;
pub use allocator::*;
pub use fluentbase_codec as codec;
pub use fluentbase_crypto as crypto;
pub use fluentbase_sdk_derive as derive;
pub use fluentbase_types::*;
pub use types::*;

#[cfg(all(not(feature = "std"), not(target_arch = "wasm32")))]
compile_error!("non-std mode is only supported for the wasm32 target");

#[cfg(target_endian = "big")]
compile_error!("fluentbase-sdk is not implemented for big-endian targets");
