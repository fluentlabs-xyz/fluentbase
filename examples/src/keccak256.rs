use fluentbase_sdk::{CryptoPlatformSDK, SysPlatformSDK, SDK};

pub fn main() {
    const MAX_BUFFER: usize = 1024;
    let mut input = [0u8; MAX_BUFFER];
    let input_len = SDK::sys_read(&mut input, 0);
    if input_len as usize > MAX_BUFFER {
        panic!("buffer is limited with {} bytes", MAX_BUFFER)
    }
    let mut output = [0u8; 32];
    SDK::crypto_keccak256(&input[0..(input_len as usize)], &mut output);
    SDK::sys_write(&output);
}
