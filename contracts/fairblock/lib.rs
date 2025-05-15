#[cfg(all(not(target_arch = "wasm32"), feature = "enable_go"))]
pub const WASM_BYTECODE: &[u8] = fluentbase_sdk::include_this_wasm!();

#[cfg(all(not(target_arch = "wasm32"), not(feature = "enable_go")))]
pub const WASM_BYTECODE: &[u8] = &[];
