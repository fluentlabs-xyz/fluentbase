use alloy_primitives::{address, hex, Address};

/// An address of EVM runtime that is used to execute EVM program
pub const PRECOMPILE_EVM_RUNTIME: Address = address!("0000000000000000000000000000000000520001");
/// A verifier for Fairblock attestations
pub const PRECOMPILE_FAIRBLOCK_VERIFIER: Address =
    address!("0000000000000000000000000000000000005202");

/// An address for SVM runtime
pub const PRECOMPILE_SVM_RUNTIME: Address = address!("0000000000000000000000000000000000520003");
pub const SVM_EXECUTABLE_PREIMAGE: Address = address!("0000000000000000000000000000000000520010");
pub const PRECOMPILE_WRAPPED_ETH: Address = address!("0000000000000000000000000000000000520004");
pub const PRECOMPILE_WEBAUTHN_VERIFIER: Address =
    address!("0000000000000000000000000000000000520005");
pub const PRECOMPILE_OAUTH2_VERIFIER: Address =
    address!("0000000000000000000000000000000000520006");
pub const PRECOMPILE_NITRO_VERIFIER: Address = address!("0000000000000000000000000000000000520007");
pub const PRECOMPILE_ERC20_RUNTIME: Address = address!("0000000000000000000000000000000000520008");
pub const PRECOMPILE_WASM_RUNTIME: Address = address!("0000000000000000000000000000000000520009");
pub const PRECOMPILE_EIP2935: Address = address!("0000F90827F1C53a10cb7A02335B175320002935");

pub const SYSTEM_ADDRESS: Address = address!("fffffffffffffffffffffffffffffffffffffffe");

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

pub const PRECOMPILE_ADDRESSES: &[Address] = &[
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
    PRECOMPILE_EIP2935,
    PRECOMPILE_ERC20_RUNTIME,
    PRECOMPILE_EVM_RUNTIME,
    PRECOMPILE_FAIRBLOCK_VERIFIER,
    PRECOMPILE_IDENTITY,
    PRECOMPILE_KZG_POINT_EVALUATION,
    PRECOMPILE_NITRO_VERIFIER,
    PRECOMPILE_OAUTH2_VERIFIER,
    PRECOMPILE_RIPEMD160,
    PRECOMPILE_SECP256K1_RECOVER,
    PRECOMPILE_SHA256,
    PRECOMPILE_SVM_RUNTIME,
    PRECOMPILE_WASM_RUNTIME,
    PRECOMPILE_WEBAUTHN_VERIFIER,
    PRECOMPILE_WRAPPED_ETH,
    SVM_EXECUTABLE_PREIMAGE,
];

pub fn is_system_precompile(address: &Address) -> bool {
    PRECOMPILE_ADDRESSES.contains(address)
}

pub const fn is_resumable_precompile(address: &Address) -> bool {
    match address {
        &PRECOMPILE_EIP2935
        | &PRECOMPILE_ERC20_RUNTIME
        | &PRECOMPILE_EVM_RUNTIME
        | &PRECOMPILE_FAIRBLOCK_VERIFIER
        | &PRECOMPILE_SVM_RUNTIME
        | &PRECOMPILE_WASM_RUNTIME
        | &PRECOMPILE_WEBAUTHN_VERIFIER
        | &PRECOMPILE_WRAPPED_ETH
        | &SVM_EXECUTABLE_PREIMAGE => true,
        _ => false,
    }
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

/// The authority address that is allowed to update the code of arbitrary accounts
pub const UPDATE_GENESIS_AUTH: Address = address!("0xa7bf6a9168fe8a111307b7c94b8883fe02b30934");

/// The prefix that must appear at the beginning of the transaction `call data`
/// to signal that the transaction is intended to perform an account update.
pub const UPDATE_GENESIS_PREFIX: [u8; 4] = hex!("0x69bc6f64");
