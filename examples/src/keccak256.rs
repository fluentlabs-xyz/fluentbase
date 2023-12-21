use fluentbase_sdk::{evm::ExecutionContext, CryptoPlatformSDK, SysPlatformSDK, SDK};

pub fn main() {
    let input = ExecutionContext::contract_input();
    let mut output = [0u8; 32];
    SDK::crypto_keccak256(&input, &mut output);
    SDK::sys_write(&output);
}
