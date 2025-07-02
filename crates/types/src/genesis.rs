use alloy_primitives::{address, Address};

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

pub struct GenesisContractBuildOutput {
    pub name: &'static str,
    pub wasm_bytecode: &'static [u8],
    pub rwasm_bytecode: &'static [u8],
    pub rwasm_bytecode_hash: [u8; 32],
    pub wasmtime_module_bytes: &'static [u8],
}
