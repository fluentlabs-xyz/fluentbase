use crate::{alloc_slice, LowLevelSDK, SharedAPI, JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD, U256};
use fluentbase_sdk_derive::client;
use fluentbase_types::{address, Address, Bytes, SovereignAPI};

pub const PRECOMPILE_EVM: Address = address!("5200000000000000000000000000000000000001");
pub const PRECOMPILE_WASM: Address = address!("5200000000000000000000000000000000000002");
pub const PRECOMPILE_SVM: Address = address!("5200000000000000000000000000000000000003");
pub const PRECOMPILE_FLUENT: Address = address!("5200000000000000000000000000000000000004");

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

pub trait EvmAPI {
    fn address<SDK: SharedAPI>(&self) -> Address;
    fn balance<SDK: SharedAPI>(&self, address: Address) -> U256;
    fn calldatacopy<SDK: SharedAPI>(&self, mem_ptr: *mut u8, data_offset: u64, len: u64);
    fn calldataload<SDK: SharedAPI>(&self, offset: u64) -> U256;
    fn calldatasize<SDK: SharedAPI>(&self) -> u64;
    fn sload<SDK: SharedAPI>(&self, index: U256) -> U256;
    fn sstore<SDK: SharedAPI>(&self, index: U256, value: U256);
}

pub trait WasmAPI {}

pub trait SvmAPI {}

pub trait BlendedAPI {
    fn exec_evm_tx<SDK: SharedAPI>(&self, raw_evm_tx: Bytes);
    fn exec_svm_tx<SDK: SharedAPI>(&self, raw_svm_tx: Bytes);
}

pub fn call_system_contract(address: &Address, input: &[u8], mut fuel: u32) -> (Bytes, i32) {
    let mut address32: [u8; 32] = [0u8; 32];
    address32[12..].copy_from_slice(address.as_slice());
    let mut hash32: [u8; 32] = [0u8; 32];
    _ = LowLevelSDK::get_leaf(
        address32.as_ptr(),
        JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
        hash32.as_mut_ptr(),
        false,
    );
    let exit_code = LowLevelSDK::exec(
        hash32.as_ptr(),
        input.as_ptr(),
        input.len() as u32,
        core::ptr::null_mut(),
        0,
        &mut fuel as *mut u32,
    );
    let output_size = LowLevelSDK::output_size();
    let output = alloc_slice(output_size as usize);
    LowLevelSDK::read_output(output.as_mut_ptr(), 0, output_size);
    (Bytes::copy_from_slice(output), exit_code)
}
