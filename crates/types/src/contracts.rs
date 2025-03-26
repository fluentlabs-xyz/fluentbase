use alloy_primitives::{address, Address};

const fn evm_address(value: u8) -> Address {
    Address::with_last_byte(value)
}

pub const PRECOMPILE_SECP256K1_ECRECOVER: Address = evm_address(0x1);
pub const PRECOMPILE_SHA256: Address = evm_address(0x2);
pub const PRECOMPILE_RIPEMD160: Address = evm_address(0x3);
pub const PRECOMPILE_IDENTITY: Address = evm_address(0x4);
pub const PRECOMPILE_MODEXP: Address = evm_address(0x5);
pub const PRECOMPILE_BN128_ADD: Address = evm_address(0x6);
pub const PRECOMPILE_BN128_MUL: Address = evm_address(0x7);
pub const PRECOMPILE_BN128_PAIR: Address = evm_address(0x8);
pub const PRECOMPILE_BLAKE2: Address = evm_address(0x9);
pub const PRECOMPILE_KZG_POINT_EVALUATION: Address = evm_address(0x0a);
pub const PRECOMPILE_BLS12_381_G1_ADD: Address = evm_address(0x0b);
pub const PRECOMPILE_BLS12_381_G1_MUL: Address = evm_address(0x0c);
pub const PRECOMPILE_BLS12_381_G1_MSM: Address = evm_address(0x0d);
pub const PRECOMPILE_BLS12_381_G2_ADD: Address = evm_address(0x0e);
pub const PRECOMPILE_BLS12_381_G2_MUL: Address = evm_address(0x0f);
pub const PRECOMPILE_BLS12_381_G2_MSM: Address = evm_address(0x10);
pub const PRECOMPILE_BLS12_381_PAIRING: Address = evm_address(0x11);
pub const PRECOMPILE_BLS12_381_MAP_FP_TO_G1: Address = evm_address(0x12);
pub const PRECOMPILE_BLS12_381_MAP_FP2_TO_G2: Address = evm_address(0x13);

// "R native" + keccak256("multicall(bytes[])")[..4]
pub const PRECOMPILE_NATIVE_MULTICALL: Address =
    address!("52206e61746976650000000000000000ac9650d8");

// keccak256("_svm")[..12]
pub const SVM_EE_ADDRESS: Address = address!("9f7f2a4373a3d89a8500e80195683b6f140690cd");

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

/// Example greeting `keccak256("_example_greeting")[12..]`
pub const EXAMPLE_GREETING_ADDRESS: Address = address!("43799b91fb174261ec2950ebb819c2cff2983bdf");

/// Example fairblock `keccak256("_example_fairblock")[12..]`
pub const EXAMPLE_FAIRBLOCK_ADDRESS: Address = address!("d92adea71798aadff13f526556dea230214e0a30");
