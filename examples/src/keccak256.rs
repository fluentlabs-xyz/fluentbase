use fluentbase_sdk::{evm::ExecutionContext, CryptoPlatformSDK, SDK};

pub fn main() {
    let input = ExecutionContext::contract_input();
    let mut output = [0u8; 32];
    SDK::crypto_keccak256(&input, &mut output);
    let mut ctx = ExecutionContext::default();
    ctx.return_and_exit(output.as_slice(), 0);
}
