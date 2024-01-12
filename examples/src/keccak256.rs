use fluentbase_sdk::{evm::ExecutionContext, LowLevelCryptoSDK, LowLevelSDK};

pub fn main() {
    let input = ExecutionContext::contract_input();
    let mut output = [0u8; 32];
    LowLevelSDK::crypto_keccak256(&input, &mut output);
    let mut ctx = ExecutionContext::default();
    ctx.return_and_exit(output.as_slice(), 0);
}
