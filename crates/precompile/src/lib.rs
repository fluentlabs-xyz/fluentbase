extern crate alloc;
extern crate core;

use alloc::vec::Vec;
use alloy_primitives::{address, Address};
use fluentbase_types::{
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
use hashbrown::HashMap;
use lazy_static::lazy_static;

const WASM_EVM_RUNTIME: &[u8] = include_bytes!("../../../contracts/evm/lib.wasm");
const WASM_MULTICALL: &[u8] = include_bytes!("../../../contracts/multicall/lib.wasm");
const WASM_FAIRBLOCK_VERIFIER: &[u8] = include_bytes!("../../../contracts/fairblock/lib.wasm");
const WASM_ECRECOVER: &[u8] = include_bytes!("../../../contracts/ecrecover/lib.wasm");
const WASM_SHA256: &[u8] = include_bytes!("../../../contracts/sha256/lib.wasm");
const WASM_RIPEMD160: &[u8] = include_bytes!("../../../contracts/ripemd160/lib.wasm");
const WASM_IDENTITY: &[u8] = include_bytes!("../../../contracts/identity/lib.wasm");
const WASM_NITRO_VERIFIER: &[u8] = include_bytes!("../../../contracts/nitro/lib.wasm");
const WASM_MODEXP: &[u8] = include_bytes!("../../../contracts/modexp/lib.wasm");
const WASM_BN256: &[u8] = include_bytes!("../../../contracts/bn256/lib.wasm");
const WASM_BLAKE2F: &[u8] = include_bytes!("../../../contracts/blake2f/lib.wasm");
const WASM_KZG_POINT_EVALUATION: &[u8] = include_bytes!("../../../contracts/kzg/lib.wasm");
const WASM_BLS12381: &[u8] = include_bytes!("../../../contracts/bls12381/lib.wasm");

/// Checks is contract has self-gas management
pub fn is_self_gas_management_contract(address: &Address) -> bool {
    is_system_precompile(address)
}

lazy_static! {
    static ref SYSTEM_PRECOMPILES: HashMap<Address, Vec<u8>> = {
        let mut m = HashMap::new();
        m.insert(PRECOMPILE_EVM_RUNTIME, Vec::from(WASM_EVM_RUNTIME));
        m.insert(
            PRECOMPILE_FAIRBLOCK_VERIFIER,
            Vec::from(WASM_FAIRBLOCK_VERIFIER),
        );
        m.insert(PRECOMPILE_NATIVE_MULTICALL, Vec::from(WASM_MULTICALL));
        m.insert(PRECOMPILE_NITRO_VERIFIER, Vec::from(WASM_NITRO_VERIFIER));
        m.insert(PRECOMPILE_SECP256K1_RECOVER, Vec::from(WASM_ECRECOVER));
        m.insert(PRECOMPILE_SHA256, Vec::from(WASM_SHA256));
        m.insert(PRECOMPILE_RIPEMD160, Vec::from(WASM_RIPEMD160));
        m.insert(PRECOMPILE_IDENTITY, Vec::from(WASM_IDENTITY));
        m.insert(PRECOMPILE_BIG_MODEXP, Vec::from(WASM_MODEXP));
        m.insert(PRECOMPILE_BN256_ADD, Vec::from(WASM_BN256));
        m.insert(PRECOMPILE_BN256_MUL, Vec::from(WASM_BN256));
        m.insert(PRECOMPILE_BN256_PAIR, Vec::from(WASM_BN256));
        m.insert(PRECOMPILE_BLAKE2F, Vec::from(WASM_BLAKE2F));
        m.insert(
            PRECOMPILE_KZG_POINT_EVALUATION,
            Vec::from(WASM_KZG_POINT_EVALUATION),
        );
        m.insert(PRECOMPILE_BLS12_381_G1_ADD, Vec::from(WASM_BLS12381));
        m.insert(PRECOMPILE_BLS12_381_G1_MSM, Vec::from(WASM_BLS12381));
        m.insert(PRECOMPILE_BLS12_381_G2_ADD, Vec::from(WASM_BLS12381));
        m.insert(PRECOMPILE_BLS12_381_G2_MSM, Vec::from(WASM_BLS12381));
        m.insert(PRECOMPILE_BLS12_381_PAIRING, Vec::from(WASM_BLS12381));
        m.insert(PRECOMPILE_BLS12_381_MAP_G1, Vec::from(WASM_BLS12381));
        m.insert(PRECOMPILE_BLS12_381_MAP_G2, Vec::from(WASM_BLS12381));
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
