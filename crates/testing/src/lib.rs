mod evm;
mod host;

pub use evm::*;
pub use fluentbase_sdk::include_this_wasm;
pub use host::*;

extern crate alloc;
