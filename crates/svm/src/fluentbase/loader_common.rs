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
use solana_pubkey::Pubkey;

fn derive_salt(pk: &Pubkey) -> U256 {
    let data = [SVM_EXECUTABLE_PREIMAGE.as_slice(), pk.as_ref()].concat();
    U256::from_be_slice(&keccak256(data).0)
}

fn derive_address(salt: &U256) -> Address {
    calc_create4_address(&PRECOMPILE_SVM_RUNTIME, &salt, |v| keccak256(v))
}

pub fn write_contract_executable(
    api: &mut impl MetadataAPI,
    pk_exec: &Pubkey,
    executable_data: Bytes,
) -> SyscallResult<()> {
    let salt = derive_salt(pk_exec);
    let derived_metadata_address = derive_address(&salt);
    let metadata_size = api.metadata_size(&derived_metadata_address);
    if !metadata_size.status.is_ok() {
        return SyscallResult::from_old_empty(metadata_size);
    }
    let metadata_size = metadata_size.data.0;
    let result = if metadata_size == 0 {
        api.metadata_create(&salt, executable_data)
    } else {
        api.metadata_write(&derived_metadata_address, 0, executable_data)
    };
    result
}
pub fn read_contract_executable(api: &impl MetadataAPI, pk_exec: &Pubkey) -> SyscallResult<Bytes> {
    let salt = derive_salt(pk_exec);
    let derived_metadata_address = derive_address(&salt);
    let data_size = api.metadata_size(&derived_metadata_address);
    if !data_size.status.is_ok() {
        return SyscallResult::from_old(data_size, Default::default());
    }
    let executable_data = api.metadata_copy(&derived_metadata_address, 0, data_size.data.0);
    executable_data
}
