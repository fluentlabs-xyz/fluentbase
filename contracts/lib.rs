macro_rules! include_wasm {
    ($crate_name:literal) => {{
        include_bytes!(concat!(
            "../target/target2/wasm32-unknown-unknown/release/deps/",
            $crate_name,
            ".wasm"
        ))
    }};
}

#[rustfmt::skip]
mod system_contracts {
    pub const WASM_BIG_MODEXP: &[u8] = include_wasm!("fluentbase_contracts_modexp");
    pub const WASM_BLAKE2F: &[u8] = include_wasm!("fluentbase_contracts_blake2f");
    pub const WASM_BN256_ADD: &[u8] = include_wasm!("fluentbase_contracts_bn256");
    pub const WASM_BN256_MUL: &[u8] = include_wasm!("fluentbase_contracts_bn256");
    pub const WASM_BN256_PAIR: &[u8] = include_wasm!("fluentbase_contracts_bn256");
    pub const WASM_ERC20: &[u8] = include_wasm!("fluentbase_contracts_erc20");
    pub const WASM_EVM_RUNTIME: &[u8] = include_wasm!("fluentbase_contracts_evm");
    pub const WASM_FAIRBLOCK_VERIFIER: &[u8] = include_wasm!("fluentbase_contracts_fairblock");
    pub const WASM_IDENTITY: &[u8] = include_wasm!("fluentbase_contracts_identity");
    pub const WASM_KZG_POINT_EVALUATION: &[u8] = include_wasm!("fluentbase_contracts_kzg");
    pub const WASM_NATIVE_MULTICALL: &[u8] = include_wasm!("fluentbase_contracts_multicall");
    pub const WASM_NITRO_VERIFIER: &[u8] = include_wasm!("fluentbase_contracts_nitro");
    pub const WASM_OAUTH2_VERIFIER: &[u8] = include_wasm!("fluentbase_contracts_oauth2");
    pub const WASM_RIPEMD160: &[u8] = include_wasm!("fluentbase_contracts_ripemd160");
    pub const WASM_SECP256K1_RECOVER: &[u8] = include_wasm!("fluentbase_contracts_ecrecover");
    pub const WASM_SHA256: &[u8] = include_wasm!("fluentbase_contracts_sha256");
    pub const WASM_WEBAUTHN_VERIFIER: &[u8] = include_wasm!("fluentbase_contracts_webauthn");
}

pub use system_contracts::*;
