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

pub const WASM_BLAKE2F: &[u8] = include_bytes!("../../../contracts/blake2f/lib.wasm");
pub const WASM_BLS12381: &[u8] = include_bytes!("../../../contracts/bls12381/lib.wasm");
pub const WASM_BN256: &[u8] = include_bytes!("../../../contracts/bn256/lib.wasm");
pub const WASM_ECRECOVER: &[u8] = include_bytes!("../../../contracts/ecrecover/lib.wasm");
pub const WASM_ERC20: &[u8] = include_bytes!("../../../contracts/erc20/lib.wasm");
pub const WASM_EVM: &[u8] = include_bytes!("../../../contracts/evm/lib.wasm");
pub const WASM_FAIRBLOCK: &[u8] = include_bytes!("../../../contracts/fairblock/lib.wasm");
pub const WASM_IDENTITY: &[u8] = include_bytes!("../../../contracts/identity/lib.wasm");
pub const WASM_KZG: &[u8] = include_bytes!("../../../contracts/kzg/lib.wasm");
pub const WASM_MODEXP: &[u8] = include_bytes!("../../../contracts/modexp/lib.wasm");
pub const WASM_MULTICALL: &[u8] = include_bytes!("../../../contracts/multicall/lib.wasm");
pub const WASM_NITRO: &[u8] = include_bytes!("../../../contracts/nitro/lib.wasm");
pub const WASM_OAUTH2: &[u8] = include_bytes!("../../../contracts/oauth2/lib.wasm");
pub const WASM_RIPEMD160: &[u8] = include_bytes!("../../../contracts/ripemd160/lib.wasm");
pub const WASM_SECP256R1: &[u8] = include_bytes!("../../../contracts/secp256r1/lib.wasm");
pub const WASM_SHA256: &[u8] = include_bytes!("../../../contracts/sha256/lib.wasm");
pub const WASM_WEBAUTHN: &[u8] = include_bytes!("../../../contracts/webauthn/lib.wasm");

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
    ("blake2f", fluentbase_types::PRECOMPILE_BLAKE2F, WASM_BLAKE2F),
    ("bls12381", fluentbase_types::PRECOMPILE_KZG_POINT_EVALUATION, WASM_BLS12381),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_G1_ADD, WASM_BLS12381),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_G1_MSM, WASM_BLS12381),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_G2_ADD, WASM_BLS12381),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_G2_MSM, WASM_BLS12381),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_PAIRING, WASM_BLS12381),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_MAP_G1, WASM_BLS12381),
    ("bls12381", fluentbase_types::PRECOMPILE_BLS12_381_MAP_G2, WASM_BLS12381),
    ("bn256", fluentbase_types::PRECOMPILE_BN256_PAIR, WASM_BN256),
    ("ecrecover", fluentbase_types::PRECOMPILE_SECP256K1_RECOVER, WASM_ECRECOVER),
    ("erc20", fluentbase_types::PRECOMPILE_ERC20, WASM_ERC20),
    ("evm", fluentbase_types::PRECOMPILE_EVM_RUNTIME, WASM_EVM),
    ("fairblock", fluentbase_types::PRECOMPILE_FAIRBLOCK_VERIFIER, WASM_FAIRBLOCK),
    ("identity", fluentbase_types::PRECOMPILE_IDENTITY, WASM_IDENTITY),
    ("kzg", fluentbase_types::PRECOMPILE_KZG_POINT_EVALUATION, WASM_KZG),
    ("modexp", fluentbase_types::PRECOMPILE_BIG_MODEXP, WASM_MODEXP),
    ("multicall", fluentbase_types::PRECOMPILE_NATIVE_MULTICALL, WASM_MULTICALL),
    ("nitro", fluentbase_types::PRECOMPILE_NITRO_VERIFIER, WASM_NITRO),
    ("oauth2", fluentbase_types::PRECOMPILE_OAUTH2_VERIFIER, WASM_OAUTH2),
    ("ripemd160", fluentbase_types::PRECOMPILE_RIPEMD160, WASM_RIPEMD160),
    ("secp256r1", fluentbase_types::PRECOMPILE_SECP256K1_RECOVER, WASM_SECP256R1),
    ("sha256", fluentbase_types::PRECOMPILE_SHA256, WASM_SHA256),
    ("webauthn", fluentbase_types::PRECOMPILE_WEBAUTHN_VERIFIER, WASM_WEBAUTHN),
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
