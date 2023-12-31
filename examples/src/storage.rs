use fluentbase_sdk::{EvmPlatformSDK, SysPlatformSDK, SDK};

const STORAGE_KEY: [u8; 32] = [1; 32];

pub fn deploy() {
    let mut value: [u8; 32] = [0; 32];
    value[0..14].copy_from_slice("Hello, Storage".as_bytes());
    SDK::evm_sstore(&STORAGE_KEY, &value);
}

pub fn main() {
    let mut value: [u8; 32] = [0; 32];
    SDK::evm_sload(&STORAGE_KEY, &mut value);
    SDK::sys_write(&value);
}
