use fluentbase_sdk::{
    calc_create4_address,
    keccak256,
    Address,
    Bytes,
    MetadataAPI,
    SyscallResult,
    PRECOMPILE_SVM_RUNTIME,
    SVM_EXECUTABLE_PREIMAGE,
    U256,
};
use keccak_hash::keccak;
use solana_pubkey::Pubkey;

fn derive_salt(pk: &Pubkey) -> U256 {
    let data = [SVM_EXECUTABLE_PREIMAGE.as_slice(), pk.as_ref()].concat();
    U256::from_be_slice(&keccak(data).0)
}

fn derive_address(salt: &U256) -> Address {
    calc_create4_address(&PRECOMPILE_SVM_RUNTIME, &salt, |v| keccak256(v))
}

pub fn write_contract_executable(
    sapi: &mut impl MetadataAPI,
    pk_exec: &Pubkey,
    executable_data: Bytes,
) -> SyscallResult<()> {
    let salt = derive_salt(pk_exec);
    let derived_metadata_address = derive_address(&salt);
    let (metadata_size, _, _) = sapi
        .metadata_size(&derived_metadata_address)
        .expect("metadata size")
        .data;
    let result = if metadata_size == 0 {
        sapi.metadata_create(&salt, executable_data)
    } else {
        sapi.metadata_write(&derived_metadata_address, 0, executable_data)
    };
    result.expect("failed to save contract executable")
}
pub fn read_contract_executable(sapi: &impl MetadataAPI, pk_exec: &Pubkey) -> SyscallResult<Bytes> {
    let salt = derive_salt(pk_exec);
    let derived_metadata_address = derive_address(&salt);
    let data_size = sapi.metadata_size(&derived_metadata_address);
    if !data_size.status.is_ok() {
        return SyscallResult::from_old(data_size, Bytes::default());
    }
    let executable_data = sapi.metadata_copy(&derived_metadata_address, 0, data_size.data.0);
    executable_data
}
