use fluentbase_sdk::{EvmPlatformSDK, SDK};

#[no_mangle]
pub fn sload(a0: u64, a1: u64, a2: u64, a3: u64) {
    let mut key = [0u8; 32];
    key[0..8].copy_from_slice(&a3.to_be_bytes());
    key[8..16].copy_from_slice(&a2.to_be_bytes());
    key[16..24].copy_from_slice(&a1.to_be_bytes());
    key[24..32].copy_from_slice(&a0.to_be_bytes());
    let mut value = [0u8; 32];
    SDK::evm_sload(&key, value.as_mut_slice());
}
