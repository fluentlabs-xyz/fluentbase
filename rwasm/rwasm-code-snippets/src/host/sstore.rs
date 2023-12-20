use fluentbase_sdk::{EvmPlatformSDK, SDK};

#[no_mangle]
pub fn sstore(
    key0: u64,
    key1: u64,
    key2: u64,
    key3: u64,
    val0: u64,
    val1: u64,
    val2: u64,
    val3: u64,
) {
    let mut key = [0u8; 32];
    key[0..8].copy_from_slice(&key3.to_be_bytes());
    key[8..16].copy_from_slice(&key2.to_be_bytes());
    key[16..24].copy_from_slice(&key1.to_be_bytes());
    key[24..32].copy_from_slice(&key0.to_be_bytes());
    let mut val = [0u8; 32];
    val[0..8].copy_from_slice(&val3.to_be_bytes());
    val[8..16].copy_from_slice(&val2.to_be_bytes());
    val[16..24].copy_from_slice(&val1.to_be_bytes());
    val[24..32].copy_from_slice(&val0.to_be_bytes());
    SDK::evm_sstore(&key, &val);
}
