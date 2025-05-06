macro_rules! include_wasm {
    ($crate_name:literal) => {{
        include_bytes!(concat!(
            "../target/target2/wasm32-unknown-unknown/release/deps/",
            $crate_name,
            ".wasm"
        ))
    }};
}

#[cfg(not(target_arch = "wasm32"))]
mod wasm {
    const WASM_BLAKE2F: &[u8] =
        include_bytes!("../../target/target2/wasm32-unknown-unknown/release/blake2f.wasm");
}
#[cfg(not(target_arch = "wasm32"))]
pub use wasm::*;
