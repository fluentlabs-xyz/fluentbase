pub use alloy_genesis::Genesis;
use fluentbase_types::{Address, HashMap};
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

macro_rules! include_wasm {
    ($name:tt) => {{
        include_bytes!(env!(concat!("FLUENTBASE_WASM_BINARY_PATH_", $name)))
    }};
}

lazy_static! {
    #[rustfmt::skip]
    static ref SYSTEM_PRECOMPILES: HashMap<Address, Vec<u8>> = {
        let mut arr = Vec::new();
        arr.extend([
            (fluentbase_types::PRECOMPILE_BIG_MODEXP, Vec::from(include_wasm!("fluentbase-contracts-modexp"))),
            (fluentbase_types::PRECOMPILE_BLAKE2F, Vec::from(include_wasm!("fluentbase-contracts-blake2f"))),
            (fluentbase_types::PRECOMPILE_BN256_ADD, Vec::from(include_wasm!("fluentbase-contracts-bn256"))),
            (fluentbase_types::PRECOMPILE_BN256_MUL, Vec::from(include_wasm!("fluentbase-contracts-bn256"))),
            (fluentbase_types::PRECOMPILE_BN256_PAIR, Vec::from(include_wasm!("fluentbase-contracts-bn256"))),
            (fluentbase_types::PRECOMPILE_ERC20, Vec::from(include_wasm!("fluentbase-contracts-erc20"))),
            (fluentbase_types::PRECOMPILE_EVM_RUNTIME, Vec::from(include_wasm!("fluentbase-contracts-evm"))),
            (fluentbase_types::PRECOMPILE_FAIRBLOCK_VERIFIER, Vec::from(include_wasm!("fluentbase-contracts-fairblock"))),
            (fluentbase_types::PRECOMPILE_IDENTITY, Vec::from(include_wasm!("fluentbase-contracts-identity"))),
            (fluentbase_types::PRECOMPILE_KZG_POINT_EVALUATION, Vec::from(include_wasm!("fluentbase-contracts-kzg"))),
            (fluentbase_types::PRECOMPILE_NATIVE_MULTICALL, Vec::from(include_wasm!("fluentbase-contracts-multicall"))),
            (fluentbase_types::PRECOMPILE_NITRO_VERIFIER, Vec::from(include_wasm!("fluentbase-contracts-nitro"))),
            (fluentbase_types::PRECOMPILE_OAUTH2_VERIFIER, Vec::from(include_wasm!("fluentbase-contracts-oauth2"))),
            (fluentbase_types::PRECOMPILE_RIPEMD160, Vec::from(include_wasm!("fluentbase-contracts-ripemd160"))),
            (fluentbase_types::PRECOMPILE_SECP256K1_RECOVER, Vec::from(include_wasm!("fluentbase-contracts-ecrecover"))),
            (fluentbase_types::PRECOMPILE_SHA256, Vec::from(include_wasm!("fluentbase-contracts-sha256"))),
            (fluentbase_types::PRECOMPILE_WEBAUTHN_VERIFIER, Vec::from(include_wasm!("fluentbase-contracts-webauthn"))),
        ]);

        #[cfg(feature = "bls12")]
        {
            arr.extend([
                (fluentbase_types::PRECOMPILE_BLS12_381_G1_ADD, Vec::from(include_wasm!("fluentbase-contracts-bls12381"))),
                (fluentbase_types::PRECOMPILE_BLS12_381_G1_MSM, Vec::from(include_wasm!("fluentbase-contracts-bls12381"))),
                (fluentbase_types::PRECOMPILE_BLS12_381_G2_ADD, Vec::from(include_wasm!("fluentbase-contracts-bls12381"))),
                (fluentbase_types::PRECOMPILE_BLS12_381_G2_MSM, Vec::from(include_wasm!("fluentbase-contracts-bls12381"))),
                (fluentbase_types::PRECOMPILE_BLS12_381_MAP_G1, Vec::from(include_wasm!("fluentbase-contracts-bls12381"))),
                (fluentbase_types::PRECOMPILE_BLS12_381_MAP_G2, Vec::from(include_wasm!("fluentbase-contracts-bls12381"))),
                (fluentbase_types::PRECOMPILE_BLS12_381_PAIRING, Vec::from(include_wasm!("fluentbase-contracts-bls12381"))),
            ]);
        }
        let mut map = HashMap::new();
        for (addr, data) in arr {
            map.insert(addr, data);
        }
        map
    };
}

/// Checks is contract has self-gas management
pub fn is_self_gas_management_contract(address: &Address) -> bool {
    is_system_precompile(address)
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
    if input[..4] == fluentbase_types::PRECOMPILE_NATIVE_MULTICALL[16..] {
        Some(fluentbase_types::PRECOMPILE_NATIVE_MULTICALL)
    } else {
        None
    }
}
