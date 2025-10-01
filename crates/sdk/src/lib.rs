#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
extern crate core;

pub extern crate rwasm as rwasm_core;

mod address;
mod allocator;
#[cfg(not(feature = "std"))]
mod bindings;
pub mod constructor;
pub mod entrypoint;
pub mod leb128;
mod macros;
mod math_api;
mod native_api;
pub mod panic;
#[cfg(not(feature = "std"))]
pub mod rwasm;
pub mod shared;
pub mod storage;
#[deprecated(note = "Use `fluentbase_sdk::storage` instead", since = "0.4.5-dev")]
pub mod storage_legacy;
pub mod syscall;
mod types;

pub use address::*;
pub use allocator::*;
pub use alloy_primitives::*;
pub use fluentbase_codec as codec;
pub use fluentbase_sdk_derive as derive;
pub use hashbrown::{self, hash_map, hash_set, HashMap, HashSet};
pub use math_api::*;
pub use native_api::*;
pub use types::*;

#[cfg(feature = "std")]
#[macro_export]
macro_rules! include_this_wasm {
    () => {
        include_bytes!(env!("FLUENTBASE_WASM_ARTIFACT_PATH"))
    };
}
