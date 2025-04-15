pub use alloy_genesis::Genesis;
pub use fluentbase_types::genesis::*;
use fluentbase_types::{compile_wasm_to_rwasm, keccak256, Address, HashMap, B256};
use lazy_static::lazy_static;

#[cfg(feature = "generate-genesis")]
pub fn devnet_genesis_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

pub fn devnet_genesis_v0_1_0_dev10_from_file() -> Genesis {
    let json_file = include_str!("../assets/genesis-devnet-v0.1.0-dev.10.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

#[rustfmt::skip]
mod precompile {
    use fluentbase_types::include_wasm;

    pub const PRECOMPILE_BYTECODE_BIG_MODEXP: &[u8] = include_wasm!("fluentbase-contracts-modexp");
    pub const PRECOMPILE_BYTECODE_BLAKE2F: &[u8] = include_wasm!("fluentbase-contracts-blake2f");
    pub const PRECOMPILE_BYTECODE_BN256_ADD: &[u8] = include_wasm!("fluentbase-contracts-bn256");
    pub const PRECOMPILE_BYTECODE_BN256_MUL: &[u8] = include_wasm!("fluentbase-contracts-bn256");
    pub const PRECOMPILE_BYTECODE_BN256_PAIR: &[u8] = include_wasm!("fluentbase-contracts-bn256");
    pub const PRECOMPILE_BYTECODE_ERC20: &[u8] = include_wasm!("fluentbase-contracts-erc20");
    pub const PRECOMPILE_BYTECODE_EVM_RUNTIME: &[u8] = include_wasm!("fluentbase-contracts-evm");
    pub const PRECOMPILE_BYTECODE_FAIRBLOCK_VERIFIER: &[u8] = include_wasm!("fluentbase-contracts-fairblock");
    pub const PRECOMPILE_BYTECODE_IDENTITY: &[u8] = include_wasm!("fluentbase-contracts-identity");
    pub const PRECOMPILE_BYTECODE_KZG_POINT_EVALUATION: &[u8] = include_wasm!("fluentbase-contracts-kzg");
    pub const PRECOMPILE_BYTECODE_NATIVE_MULTICALL: &[u8] = include_wasm!("fluentbase-contracts-multicall");
    pub const PRECOMPILE_BYTECODE_NITRO_VERIFIER: &[u8] = include_wasm!("fluentbase-contracts-nitro");
    pub const PRECOMPILE_BYTECODE_OAUTH2_VERIFIER: &[u8] = include_wasm!("fluentbase-contracts-oauth2");
    pub const PRECOMPILE_BYTECODE_RIPEMD160: &[u8] = include_wasm!("fluentbase-contracts-ripemd160");
    pub const PRECOMPILE_BYTECODE_SECP256K1_RECOVER: &[u8] = include_wasm!("fluentbase-contracts-ecrecover");
    pub const PRECOMPILE_BYTECODE_SHA256: &[u8] = include_wasm!("fluentbase-contracts-sha256");
    pub const PRECOMPILE_BYTECODE_WEBAUTHN_VERIFIER: &[u8] = include_wasm!("fluentbase-contracts-webauthn");

    #[cfg(feature = "bls12")]
    pub mod bls12 {
        use fluentbase_types::include_wasm;

        pub const PRECOMPILE_BYTECODE_BLS12_381_G1_ADD: &[u8] = include_wasm!("fluentbase-contracts-bls12381");
        pub const PRECOMPILE_BYTECODE_BLS12_381_G1_MSM: &[u8] = include_wasm!("fluentbase-contracts-bls12381");
        pub const PRECOMPILE_BYTECODE_BLS12_381_G2_ADD: &[u8] = include_wasm!("fluentbase-contracts-bls12381");
        pub const PRECOMPILE_BYTECODE_BLS12_381_G2_MSM: &[u8] = include_wasm!("fluentbase-contracts-bls12381");
        pub const PRECOMPILE_BYTECODE_BLS12_381_MAP_G1: &[u8] = include_wasm!("fluentbase-contracts-bls12381");
        pub const PRECOMPILE_BYTECODE_BLS12_381_MAP_G2: &[u8] = include_wasm!("fluentbase-contracts-bls12381");
        pub const PRECOMPILE_BYTECODE_BLS12_381_PAIRING: &[u8] = include_wasm!("fluentbase-contracts-bls12381");
    }
}

#[cfg(feature = "bls12")]
pub use precompile::bls12::*;
pub use precompile::*;

lazy_static! {
    static ref SYSTEM_PRECOMPILES: HashMap<Address, Vec<u8>> = {
        let mut arr = Vec::new();
        #[rustfmt::skip]
        arr.extend([
            (PRECOMPILE_BIG_MODEXP, PRECOMPILE_BYTECODE_BIG_MODEXP.to_vec()),
            (PRECOMPILE_BLAKE2F, PRECOMPILE_BYTECODE_BLAKE2F.to_vec()),
            (PRECOMPILE_BN256_ADD, PRECOMPILE_BYTECODE_BN256_ADD.to_vec()),
            (PRECOMPILE_BN256_MUL, PRECOMPILE_BYTECODE_BN256_MUL.to_vec()),
            (PRECOMPILE_BN256_PAIR, PRECOMPILE_BYTECODE_BN256_PAIR.to_vec()),
            (PRECOMPILE_ERC20, PRECOMPILE_BYTECODE_ERC20.to_vec()),
            (PRECOMPILE_EVM_RUNTIME, PRECOMPILE_BYTECODE_EVM_RUNTIME.to_vec()),
            (PRECOMPILE_FAIRBLOCK_VERIFIER, PRECOMPILE_BYTECODE_FAIRBLOCK_VERIFIER.to_vec()),
            (PRECOMPILE_IDENTITY, PRECOMPILE_BYTECODE_IDENTITY.to_vec()),
            (PRECOMPILE_KZG_POINT_EVALUATION, PRECOMPILE_BYTECODE_KZG_POINT_EVALUATION.to_vec()),
            (PRECOMPILE_NATIVE_MULTICALL, PRECOMPILE_BYTECODE_NATIVE_MULTICALL.to_vec()),
            (PRECOMPILE_NITRO_VERIFIER, PRECOMPILE_BYTECODE_NITRO_VERIFIER.to_vec()),
            (PRECOMPILE_OAUTH2_VERIFIER, PRECOMPILE_BYTECODE_OAUTH2_VERIFIER.to_vec()),
            (PRECOMPILE_RIPEMD160, PRECOMPILE_BYTECODE_RIPEMD160.to_vec()),
            (PRECOMPILE_SECP256K1_RECOVER, PRECOMPILE_BYTECODE_SECP256K1_RECOVER.to_vec()),
            (PRECOMPILE_SHA256, PRECOMPILE_BYTECODE_SHA256.to_vec()),
            (PRECOMPILE_WEBAUTHN_VERIFIER, PRECOMPILE_BYTECODE_WEBAUTHN_VERIFIER.to_vec()),
        ]);
        #[cfg(feature = "bls12")]
        {
            #[rustfmt::skip]
            arr.extend([
                (PRECOMPILE_BLS12_381_G1_ADD, PRECOMPILE_BYTECODE_BLS12_381_G1_ADD.to_vec()),
                (PRECOMPILE_BLS12_381_G1_MSM, PRECOMPILE_BYTECODE_BLS12_381_G1_MSM.to_vec()),
                (PRECOMPILE_BLS12_381_G2_ADD, PRECOMPILE_BYTECODE_BLS12_381_G2_ADD.to_vec()),
                (PRECOMPILE_BLS12_381_G2_MSM, PRECOMPILE_BYTECODE_BLS12_381_G2_MSM.to_vec()),
                (PRECOMPILE_BLS12_381_MAP_G1, PRECOMPILE_BYTECODE_BLS12_381_MAP_G1.to_vec()),
                (PRECOMPILE_BLS12_381_MAP_G2, PRECOMPILE_BYTECODE_BLS12_381_MAP_G2.to_vec()),
                (PRECOMPILE_BLS12_381_PAIRING, PRECOMPILE_BYTECODE_BLS12_381_PAIRING.to_vec()),
            ]);
        }
        let mut map = HashMap::new();
        for (addr, data) in arr {
            map.insert(addr, data);
        }
        map
    };
    static ref SYSTEM_PRECOMPILE_HASHES: HashMap<B256, Address> = {
        let mut map = HashMap::new();
        for (addr, data) in SYSTEM_PRECOMPILES.iter() {
            let rwasm_bytecode = compile_wasm_to_rwasm(data.as_slice())
                .expect("failed to compile system contract to rwasm");
            assert!(rwasm_bytecode.constructor_params.is_empty());
            let rwasm_bytecode: Vec<u8> = rwasm_bytecode.rwasm_bytecode.into();
            map.insert(keccak256(rwasm_bytecode), addr.clone());
        }
        map
    };
}

/// Checks is contract has self-gas management
pub fn is_self_gas_management_contract(address: &Address) -> bool {
    is_system_precompile(address)
}

pub fn get_precompile_wasm_bytecode_by_hash(hash: &B256) -> Option<&'static [u8]> {
    SYSTEM_PRECOMPILE_HASHES
        .get(hash)
        .and_then(|addr| get_precompile_wasm_bytecode(addr))
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

pub fn is_system_precompile_hash(hash: &B256) -> bool {
    SYSTEM_PRECOMPILE_HASHES.contains_key(hash)
}

pub fn get_precompile_wasm_bytecode(address: &Address) -> Option<&[u8]> {
    SYSTEM_PRECOMPILES.get(address).map(Vec::as_ref)
}

pub fn get_all_precompile_addresses() -> Vec<Address> {
    let mut result = Vec::new();
    for key in SYSTEM_PRECOMPILES.keys() {
        result.push(key.clone());
    }
    result
}

pub fn get_all_precompile_hashes() -> Vec<B256> {
    let mut result = Vec::new();
    for key in SYSTEM_PRECOMPILE_HASHES.keys() {
        result.push(key.clone());
    }
    result
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
    if input[..4] == fluentbase_types::PRECOMPILE_NATIVE_MULTICALL[16..] {
        Some(fluentbase_types::PRECOMPILE_NATIVE_MULTICALL)
    } else {
        None
    }
}
