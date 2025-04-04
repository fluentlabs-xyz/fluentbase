use alloc::vec::Vec;
use alloy_primitives::{address, Address};
use hashbrown::HashMap;
use lazy_static::lazy_static;

/// An address of EVM runtime that is used to execute EVM program
pub const PRECOMPILE_EVM_RUNTIME: Address = address!("0000000000000000000000000000000000520001");
pub const PRECOMPILE_NITRO_VERIFIER: Address = address!("0000000000000000000000000000000000520007");
/// A verifier for Fairblock attestations
pub const PRECOMPILE_FAIRBLOCK_VERIFIER: Address =
    address!("0000000000000000000000000000000000005202");

/// An address for SVM runtime
pub const PRECOMPILE_SVM_RUNTIME: Address = address!("0000000000000000000000000000000000520003");
pub const PRECOMPILE_WRAPPED_ETH: Address = address!("0000000000000000000000000000000000520004");
pub const PRECOMPILE_WEBAUTHN_VERIFIER: Address =
    address!("0000000000000000000000000000000000520005");
pub const PRECOMPILE_OAUTH2_VERIFIER: Address =
    address!("0000000000000000000000000000000000520006");

const fn evm_address(value: u8) -> Address {
    Address::with_last_byte(value)
}

pub const PRECOMPILE_SECP256K1_RECOVER: Address = evm_address(0x01);
pub const PRECOMPILE_SHA256: Address = evm_address(0x02);
pub const PRECOMPILE_RIPEMD160: Address = evm_address(0x03);
pub const PRECOMPILE_IDENTITY: Address = evm_address(0x04);
pub const PRECOMPILE_BIG_MODEXP: Address = evm_address(0x05);
pub const PRECOMPILE_BN256_ADD: Address = evm_address(0x06);
pub const PRECOMPILE_BN256_MUL: Address = evm_address(0x07);
pub const PRECOMPILE_BN256_PAIR: Address = evm_address(0x08);
pub const PRECOMPILE_BLAKE2F: Address = evm_address(0x09);
pub const PRECOMPILE_KZG_POINT_EVALUATION: Address = evm_address(0x0a);
pub const PRECOMPILE_BLS12_381_G1_ADD: Address = evm_address(0x0b);
pub const PRECOMPILE_BLS12_381_G1_MSM: Address = evm_address(0x0c);
pub const PRECOMPILE_BLS12_381_G2_ADD: Address = evm_address(0x0d);
pub const PRECOMPILE_BLS12_381_G2_MSM: Address = evm_address(0x0e);
pub const PRECOMPILE_BLS12_381_PAIRING: Address = evm_address(0x0f);
pub const PRECOMPILE_BLS12_381_MAP_G1: Address = evm_address(0x10);
pub const PRECOMPILE_BLS12_381_MAP_G2: Address = evm_address(0x11);

// "R native" + keccak256("multicall(bytes[])")[..4]
pub const PRECOMPILE_NATIVE_MULTICALL: Address =
    address!("52206e61746976650000000000000000ac9650d8");

pub const WASM_EVM_RUNTIME: &[u8] = include_bytes!("../../../contracts/evm/lib.wasm");
pub const WASM_MULTICALL: &[u8] = include_bytes!("../../../contracts/multicall/lib.wasm");
pub const WASM_FAIRBLOCK_VERIFIER: &[u8] = include_bytes!("../../../contracts/fairblock/lib.wasm");
pub const WASM_ECRECOVER: &[u8] = include_bytes!("../../../contracts/ecrecover/lib.wasm");
pub const WASM_SHA256: &[u8] = include_bytes!("../../../contracts/sha256/lib.wasm");
pub const WASM_RIPEMD160: &[u8] = include_bytes!("../../../contracts/ripemd160/lib.wasm");
pub const WASM_IDENTITY: &[u8] = include_bytes!("../../../contracts/identity/lib.wasm");
pub const WASM_NITRO_VERIFIER: &[u8] = include_bytes!("../../../contracts/nitro/lib.wasm");
pub const WASM_MODEXP: &[u8] = include_bytes!("../../../contracts/modexp/lib.wasm");
pub const WASM_BN256: &[u8] = include_bytes!("../../../contracts/bn256/lib.wasm");
pub const WASM_BLAKE2F: &[u8] = include_bytes!("../../../contracts/blake2f/lib.wasm");
pub const WASM_KZG_POINT_EVALUATION: &[u8] = include_bytes!("../../../contracts/kzg/lib.wasm");
pub const WASM_BLS12381: &[u8] = include_bytes!("../../../contracts/bls12381/lib.wasm");

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
