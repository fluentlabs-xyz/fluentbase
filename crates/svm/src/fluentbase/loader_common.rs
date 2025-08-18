use fluentbase_sdk::{
    calc_create4_address, keccak256, syscall::SyscallResult, Address, Bytes, MetadataAPI,
    PRECOMPILE_SVM_RUNTIME, SVM_EXECUTABLE_PREIMAGE, U256,
};
use solana_pubkey::Pubkey;

fn derive_salt(pk: &Pubkey) -> U256 {
    let data = [SVM_EXECUTABLE_PREIMAGE.as_slice(), pk.as_ref()].concat();
    U256::from_be_slice(&keccak256(data).0)
}

fn derive_address(salt: &U256) -> Address {
    calc_create4_address(&PRECOMPILE_SVM_RUNTIME, &salt, |v| keccak256(v))
}
