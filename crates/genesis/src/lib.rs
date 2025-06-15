pub use alloy_genesis::Genesis;
use fluentbase_types::{Address, Bytes, GenesisContractBuildOutput, HashMap, B256};
use lazy_static::lazy_static;

pub fn devnet_genesis_from_file() -> Genesis {
    let json_file = include_str!("../genesis-devnet.json");
    serde_json::from_str::<Genesis>(json_file).expect("failed to parse genesis json file")
}

#[rustfmt::skip]
const GENESIS_CONTRACTS: &[(Address, GenesisContractBuildOutput)] = &[
    (fluentbase_types::PRECOMPILE_BIG_MODEXP, fluentbase_contracts_modexp::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLAKE2F, fluentbase_contracts_blake2f::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BN256_ADD, fluentbase_contracts_bn256::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BN256_MUL, fluentbase_contracts_bn256::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BN256_PAIR, fluentbase_contracts_bn256::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_ERC20, fluentbase_contracts_erc20::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_EVM_RUNTIME, fluentbase_contracts_evm::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_SVM_RUNTIME, fluentbase_contracts_evm::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_FAIRBLOCK_VERIFIER, fluentbase_contracts_fairblock::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_IDENTITY, fluentbase_contracts_identity::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_KZG_POINT_EVALUATION, fluentbase_contracts_kzg::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_G1_ADD, fluentbase_contracts_bls12381::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_G1_MSM, fluentbase_contracts_bls12381::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_G2_ADD, fluentbase_contracts_bls12381::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_G2_MSM, fluentbase_contracts_bls12381::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_PAIRING, fluentbase_contracts_bls12381::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_MAP_G1, fluentbase_contracts_bls12381::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_BLS12_381_MAP_G2, fluentbase_contracts_bls12381::BUILD_OUTPUT),

    (fluentbase_types::PRECOMPILE_NATIVE_MULTICALL, fluentbase_contracts_multicall::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_NITRO_VERIFIER, fluentbase_contracts_nitro::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_OAUTH2_VERIFIER, fluentbase_contracts_oauth2::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_RIPEMD160, fluentbase_contracts_ripemd160::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_SECP256K1_RECOVER, fluentbase_contracts_ecrecover::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_SHA256, fluentbase_contracts_sha256::BUILD_OUTPUT),
    (fluentbase_types::PRECOMPILE_WEBAUTHN_VERIFIER, fluentbase_contracts_webauthn::BUILD_OUTPUT),
];

pub struct GenesisContract {
    pub name: String,
    pub hash: B256,
    pub address: Address,
    pub wasm_bytecode: Bytes,
    pub rwasm_bytecode: Bytes,
    pub wasmtime_module_bytes: Bytes,
}

impl GenesisContract {
    pub fn from_build_output(address: &Address, build_output: &GenesisContractBuildOutput) -> Self {
        Self {
            name: build_output.name.to_string(),
            hash: B256::new(build_output.rwasm_bytecode_hash),
            address: address.clone(),
            wasm_bytecode: Bytes::from_static(build_output.wasm_bytecode),
            rwasm_bytecode: Bytes::from_static(build_output.rwasm_bytecode),
            wasmtime_module_bytes: Bytes::from_static(build_output.wasmtime_module_bytes),
        }
    }
}

lazy_static! {
    pub static ref GENESIS_CONTRACTS_BY_ADDRESS: HashMap<Address, GenesisContract> = {
        let mut map = HashMap::new();
        for (addr, contract_build_output) in GENESIS_CONTRACTS {
            let contract = GenesisContract::from_build_output(addr, contract_build_output);
            println!(
                "genesis contract address={} hash={} name={}",
                contract.address, contract.hash, contract.name
            );
            map.insert(addr.clone(), contract);
        }
        map
    };
    pub static ref GENESIS_CONTRACTS_BY_HASH: HashMap<B256, GenesisContract> = {
        let mut map = HashMap::new();
        for (addr, contract_build_output) in GENESIS_CONTRACTS {
            let contract = GenesisContract::from_build_output(addr, contract_build_output);
            map.insert(contract.hash, contract);
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
    GENESIS_CONTRACTS_BY_ADDRESS.contains_key(address)
}

pub fn is_system_precompile_hash(hash: &B256) -> bool {
    GENESIS_CONTRACTS_BY_HASH.contains_key(hash)
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
