#[cfg(not(target_arch = "wasm32"))]
pub const WASM_BYTECODE: &[u8] = fluentbase_sdk::include_this_wasm!();
