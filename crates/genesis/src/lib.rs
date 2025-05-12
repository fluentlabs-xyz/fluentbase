pub use alloy_genesis::Genesis;
use fluentbase_types::{
    compile_wasm_to_rwasm_with_config,
    default_compilation_config,
    keccak256,
    Address,
    HashMap,
    B256,
};
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
pub const GENESIS_CONTRACTS: &[(&str, Address, &[u8])] = &[
    ("blake2f", fluentbase_types::PRECOMPILE_BLAKE2F, fluentbase_contracts_blake2f::WASM_BYTECODE),
    ("bls12381", fluentbase_types::PRECOMPILE_KZG_POINT_EVALUATION, fluentbase_contracts_bls12381::WASM_BYTECODE),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_G1_ADD, fluentbase_contracts_bls12381::WASM_BYTECODE),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_G1_MSM, fluentbase_contracts_bls12381::WASM_BYTECODE),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_G2_ADD, fluentbase_contracts_bls12381::WASM_BYTECODE),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_G2_MSM, fluentbase_contracts_bls12381::WASM_BYTECODE),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_PAIRING, fluentbase_contracts_bls12381::WASM_BYTECODE),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_MAP_G1, fluentbase_contracts_bls12381::WASM_BYTECODE),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_MAP_G2, fluentbase_contracts_bls12381::WASM_BYTECODE),
    ("bn256", fluentbase_types::PRECOMPILE_BN256_PAIR, fluentbase_contracts_bn256::WASM_BYTECODE),
    ("ecrecover", fluentbase_types::PRECOMPILE_SECP256K1_RECOVER, fluentbase_contracts_ecrecover::WASM_BYTECODE),
    ("erc20", fluentbase_types::PRECOMPILE_ERC20, fluentbase_contracts_erc20::WASM_BYTECODE),
    ("evm", fluentbase_types::PRECOMPILE_EVM_RUNTIME, fluentbase_contracts_evm::WASM_BYTECODE),
    ("fairblock", fluentbase_types::PRECOMPILE_FAIRBLOCK_VERIFIER, fluentbase_contracts_fairblock::WASM_BYTECODE),
    ("identity", fluentbase_types::PRECOMPILE_IDENTITY, fluentbase_contracts_identity::WASM_BYTECODE),
    ("kzg", fluentbase_types::PRECOMPILE_KZG_POINT_EVALUATION, fluentbase_contracts_kzg::WASM_BYTECODE),
    ("modexp", fluentbase_types::PRECOMPILE_BIG_MODEXP, fluentbase_contracts_modexp::WASM_BYTECODE),
    ("multicall", fluentbase_types::PRECOMPILE_NATIVE_MULTICALL, fluentbase_contracts_multicall::WASM_BYTECODE),
    ("nitro", fluentbase_types::PRECOMPILE_NITRO_VERIFIER, fluentbase_contracts_nitro::WASM_BYTECODE),
    ("oauth2", fluentbase_types::PRECOMPILE_OAUTH2_VERIFIER, fluentbase_contracts_oauth2::WASM_BYTECODE),
    ("ripemd160", fluentbase_types::PRECOMPILE_RIPEMD160, fluentbase_contracts_ripemd160::WASM_BYTECODE),
    ("secp256r1", fluentbase_types::PRECOMPILE_SECP256K1_RECOVER, fluentbase_contracts_secp256r1::WASM_BYTECODE),
    ("sha256", fluentbase_types::PRECOMPILE_SHA256, fluentbase_contracts_sha256::WASM_BYTECODE),
    ("webauthn", fluentbase_types::PRECOMPILE_WEBAUTHN_VERIFIER, fluentbase_contracts_webauthn::WASM_BYTECODE),
];

lazy_static! {
    static ref SYSTEM_PRECOMPILES: HashMap<Address, Vec<u8>> = {
        let mut map = HashMap::new();
        for (_, addr, data) in GENESIS_CONTRACTS {
            map.insert(addr.clone(), data.to_vec());
        }
        map
    };
    static ref SYSTEM_PRECOMPILE_HASHES: HashMap<B256, Address> = {
        let mut map = HashMap::new();
        for (addr, data) in SYSTEM_PRECOMPILES.iter() {
            let mut config = default_compilation_config();
            config.builtins_consume_fuel(false);
            let rwasm_bytecode = compile_wasm_to_rwasm_with_config(data.as_slice(), config)
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
