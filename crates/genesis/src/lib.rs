pub use alloy_genesis::Genesis;
use fluentbase_types::{
    include_wasm,
    Address,
    HashMap,
    PRECOMPILE_BIG_MODEXP,
    PRECOMPILE_BLAKE2F,
    PRECOMPILE_BLS12_381_G1_ADD,
    PRECOMPILE_BLS12_381_G1_MSM,
    PRECOMPILE_BLS12_381_G2_ADD,
    PRECOMPILE_BLS12_381_G2_MSM,
    PRECOMPILE_BLS12_381_MAP_G1,
    PRECOMPILE_BLS12_381_MAP_G2,
    PRECOMPILE_BLS12_381_PAIRING,
    PRECOMPILE_BN256_ADD,
    PRECOMPILE_BN256_MUL,
    PRECOMPILE_BN256_PAIR,
    PRECOMPILE_EVM_RUNTIME,
    PRECOMPILE_FAIRBLOCK_VERIFIER,
    PRECOMPILE_IDENTITY,
    PRECOMPILE_KZG_POINT_EVALUATION,
    PRECOMPILE_NATIVE_MULTICALL,
    PRECOMPILE_NITRO_VERIFIER,
    PRECOMPILE_RIPEMD160,
    PRECOMPILE_SECP256K1_RECOVER,
    PRECOMPILE_SHA256,
};
use lazy_static::lazy_static;

pub const BLAKE2F_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-blake2f");
pub const BLS12381_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-bls12381");
pub const BN256_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-bn256");
pub const ECRECOVER_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-ecrecover");
pub const ERC20_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-erc20");
pub const EVM_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-evm");
pub const FAIRBLOCK_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-fairblock");
pub const IDENTITY_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-identity");
pub const KZG_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-kzg");
pub const MODEXP_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-modexp");
pub const MULTICALL_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-multicall");
pub const NITRO_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-nitro");
pub const OAUTH2_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-oauth2");
pub const RIPEMD160_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-ripemd160");
pub const SHA256_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-sha256");
pub const WEBAUTHN_WASM_BINARY: &[u8] = include_wasm!("fluentbase-contracts-webauthn");

#[cfg(feature = "generate-genesis")]
pub fn devnet_genesis_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

pub fn devnet_genesis_v0_1_0_dev10_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet-v0.1.0-dev.10.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

/// Checks is contract has self-gas management
pub fn is_self_gas_management_contract(address: &Address) -> bool {
    is_system_precompile(address)
}

lazy_static! {
    static ref SYSTEM_PRECOMPILES: HashMap<Address, Vec<u8>> = {
        let mut m = HashMap::new();
        m.insert(PRECOMPILE_EVM_RUNTIME, Vec::from(EVM_WASM_BINARY));
        m.insert(
            PRECOMPILE_FAIRBLOCK_VERIFIER,
            Vec::from(FAIRBLOCK_WASM_BINARY),
        );
        m.insert(
            PRECOMPILE_NATIVE_MULTICALL,
            Vec::from(MULTICALL_WASM_BINARY),
        );
        m.insert(PRECOMPILE_NITRO_VERIFIER, Vec::from(NITRO_WASM_BINARY));
        m.insert(
            PRECOMPILE_SECP256K1_RECOVER,
            Vec::from(ECRECOVER_WASM_BINARY),
        );
        m.insert(PRECOMPILE_SHA256, Vec::from(SHA256_WASM_BINARY));
        m.insert(PRECOMPILE_RIPEMD160, Vec::from(RIPEMD160_WASM_BINARY));
        m.insert(PRECOMPILE_IDENTITY, Vec::from(IDENTITY_WASM_BINARY));
        m.insert(PRECOMPILE_BIG_MODEXP, Vec::from(MODEXP_WASM_BINARY));
        m.insert(PRECOMPILE_BN256_ADD, Vec::from(BN256_WASM_BINARY));
        m.insert(PRECOMPILE_BN256_MUL, Vec::from(BN256_WASM_BINARY));
        m.insert(PRECOMPILE_BN256_PAIR, Vec::from(BN256_WASM_BINARY));
        m.insert(PRECOMPILE_BLAKE2F, Vec::from(BLAKE2F_WASM_BINARY));
        m.insert(PRECOMPILE_KZG_POINT_EVALUATION, Vec::from(KZG_WASM_BINARY));
        m.insert(PRECOMPILE_BLS12_381_G1_ADD, Vec::from(BLS12381_WASM_BINARY));
        m.insert(PRECOMPILE_BLS12_381_G1_MSM, Vec::from(BLS12381_WASM_BINARY));
        m.insert(PRECOMPILE_BLS12_381_G2_ADD, Vec::from(BLS12381_WASM_BINARY));
        m.insert(PRECOMPILE_BLS12_381_G2_MSM, Vec::from(BLS12381_WASM_BINARY));
        m.insert(
            PRECOMPILE_BLS12_381_PAIRING,
            Vec::from(BLS12381_WASM_BINARY),
        );
        m.insert(PRECOMPILE_BLS12_381_MAP_G1, Vec::from(BLS12381_WASM_BINARY));
        m.insert(PRECOMPILE_BLS12_381_MAP_G2, Vec::from(BLS12381_WASM_BINARY));
        m
    };
}

/// Determines if a given address belongs to the system precompiled set.
///
/// This function checks if the provided `address` exists in the collection
/// of system precompile addresses (`SYSTEM_PRECOMPILES`).
/// This is typically used to differentiate between user-defined addresses and those reserved
/// for EVM precompile contracts.
///
/// # Arguments
/// * `address` - A reference to the `Address` being checked.
///
/// # Returns
/// * `true` - If the `address` is recognized as a system precompile.
/// * `false` - Otherwise.
pub fn is_system_precompile(address: &Address) -> bool {
    // TODO(dmitry123): "add spec verification"
    SYSTEM_PRECOMPILES.contains_key(address)
}

pub fn get_precompile_wasm_bytecode(address: &Address) -> Option<&[u8]> {
    SYSTEM_PRECOMPILES.get(address).map(Vec::as_ref)
}

/// Checks if the function call should be redirected to a native precompiled contract.
///
/// When the first four bytes of the input (function selector) match a precompile's address
/// prefix, returns the corresponding precompiled account that should handle the call.
///
/// # Arguments
/// * `input` - The complete calldata for the function call
///
/// # Returns
/// * `Some(Account)` - The precompiled account if a match is found
/// * `None` - If no matching precompile is found or input is too short
pub fn try_resolve_precompile_account_from_input(input: &[u8]) -> Option<Address> {
    if input.len() < 4 {
        return None;
    };
    if input[..4] == PRECOMPILE_NATIVE_MULTICALL[16..] {
        Some(PRECOMPILE_NATIVE_MULTICALL)
    } else {
        None
    }
}
