use fluentbase_sdk::{CryptoPlatformSDK, SysPlatformSDK, SDK};

pub fn main() {
    let mut input = [0u8; 11]; // "hello world"
    SDK::sys_read(&mut input, 0);
    let mut output = [0u8; 32];
    SDK::crypto_poseidon(&input, &mut output);
}
