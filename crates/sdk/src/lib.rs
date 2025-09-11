#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
extern crate core;

mod allocator;
#[cfg(not(feature = "std"))]
mod bindings;
pub mod constructor;
pub mod entrypoint;
pub mod leb128;
mod macros;
pub mod panic;
#[cfg(not(feature = "std"))]
pub mod rwasm;
pub mod shared;
#[deprecated(note = "Use `fluentbase_sdk::storage` instead", since = "0.4.5-dev")]
pub mod storage_legacy;

pub use allocator::*;
pub use fluentbase_codec as codec;
pub use fluentbase_sdk_derive as derive;
pub use fluentbase_types::*;
pub use hashbrown;

#[cfg(feature = "std")]
#[macro_export]
macro_rules! include_this_wasm {
    () => {
        include_bytes!(env!("FLUENTBASE_WASM_ARTIFACT_PATH"))
    };
}
