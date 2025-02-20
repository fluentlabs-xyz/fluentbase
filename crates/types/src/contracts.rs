use alloy_primitives::{address, Address};

pub const PRECOMPILE_EVM: Address = address!("0000000000000000000000000000000000005210");
pub const PRECOMPILE_WASM: Address = address!("0000000000000000000000000000000000005220");
pub const PRECOMPILE_SVM: Address = address!("0000000000000000000000000000000000005230");

pub const PRECOMPILE_SECP256K1_ECRECOVER: Address =
    address!("0000000000000000000000000000000000000001");
pub const PRECOMPILE_SHA256: Address = address!("0000000000000000000000000000000000000002");
pub const PRECOMPILE_RIPEMD160: Address = address!("0000000000000000000000000000000000000003");
pub const PRECOMPILE_IDENTITY: Address = address!("0000000000000000000000000000000000000004");
pub const PRECOMPILE_MODEXP: Address = address!("0000000000000000000000000000000000000005");
pub const PRECOMPILE_BN128_ADD: Address = address!("0000000000000000000000000000000000000006");
pub const PRECOMPILE_BN128_MUL: Address = address!("0000000000000000000000000000000000000007");
pub const PRECOMPILE_BN128_PAIR: Address = address!("0000000000000000000000000000000000000008");
pub const PRECOMPILE_BLAKE2: Address = address!("0000000000000000000000000000000000000009");
pub const PRECOMPILE_KZG_POINT_EVALUATION: Address =
    address!("000000000000000000000000000000000000000a");
pub const PRECOMPILE_BLS12_381_G1_ADD: Address =
    address!("000000000000000000000000000000000000000b");
pub const PRECOMPILE_BLS12_381_G1_MUL: Address =
    address!("000000000000000000000000000000000000000c");
pub const PRECOMPILE_BLS12_381_G1_MSM: Address =
    address!("000000000000000000000000000000000000000d");
pub const PRECOMPILE_BLS12_381_G2_ADD: Address =
    address!("000000000000000000000000000000000000000e");
pub const PRECOMPILE_BLS12_381_G2_MUL: Address =
    address!("000000000000000000000000000000000000000f");
pub const PRECOMPILE_BLS12_381_G2_MSM: Address =
    address!("0000000000000000000000000000000000000010");
pub const PRECOMPILE_BLS12_381_PAIRING: Address =
    address!("0000000000000000000000000000000000000011");
pub const PRECOMPILE_BLS12_381_MAP_FP_TO_G1: Address =
    address!("0000000000000000000000000000000000000012");
pub const PRECOMPILE_BLS12_381_MAP_FP2_TO_G2: Address =
    address!("0000000000000000000000000000000000000013");
pub const PRECOMPILE_SECP256R1_VERIFY: Address =
    address!("0000000000000000000000000000000000000100");

// keccak256("native_precompile")[..4] + keccak256("multicall(bytes[])")[..4]
pub const PRECOMPILE_NATIVE_MULTICALL: Address =
    address!("e78e5e46000000000000000000000000ac9650d8");

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
pub fn try_resolve_precompile_account(input: &[u8]) -> Option<Address> {
    if input.len() < 4 {
        return None;
    };
    if input[..4] == PRECOMPILE_NATIVE_MULTICALL[16..] {
        Some(PRECOMPILE_NATIVE_MULTICALL)
    } else {
        None
    }
}
