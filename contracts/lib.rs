macro_rules! include_wasm {
    ($crate_name:literal) => {{
        include_bytes!(concat!(
            "../target/target2/wasm32-unknown-unknown/release/deps/",
            $crate_name,
            ".wasm"
        ))
    }};
}

pub mod genesis {
    pub const BIG_MODEXP: &[u8] = include_wasm!("fluentbase_contracts_modexp");
    pub const BLAKE2F: &[u8] = include_wasm!("fluentbase_contracts_blake2f");
    pub const BN256_ADD: &[u8] = include_wasm!("fluentbase_contracts_bn256");
    pub const BN256_MUL: &[u8] = include_wasm!("fluentbase_contracts_bn256");
    pub const BN256_PAIR: &[u8] = include_wasm!("fluentbase_contracts_bn256");
    pub const ERC20: &[u8] = include_wasm!("fluentbase_contracts_erc20");
    pub const EVM_RUNTIME: &[u8] = include_wasm!("fluentbase_contracts_evm");
    pub const FAIRBLOCK_VERIFIER: &[u8] = include_wasm!("fluentbase_contracts_fairblock");
    pub const IDENTITY: &[u8] = include_wasm!("fluentbase_contracts_identity");
    pub const KZG_POINT_EVALUATION: &[u8] = include_wasm!("fluentbase_contracts_kzg");
    pub const NATIVE_MULTICALL: &[u8] = include_wasm!("fluentbase_contracts_multicall");
    pub const NITRO_VERIFIER: &[u8] = include_wasm!("fluentbase_contracts_nitro");
    pub const OAUTH2_VERIFIER: &[u8] = include_wasm!("fluentbase_contracts_oauth2");
    pub const RIPEMD160: &[u8] = include_wasm!("fluentbase_contracts_ripemd160");
    pub const SECP256K1_RECOVER: &[u8] = include_wasm!("fluentbase_contracts_ecrecover");
    pub const SHA256: &[u8] = include_wasm!("fluentbase_contracts_sha256");
    pub const WEBAUTHN_VERIFIER: &[u8] = include_wasm!("fluentbase_contracts_webauthn");
}
