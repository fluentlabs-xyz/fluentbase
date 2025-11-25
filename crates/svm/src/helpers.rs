extern crate solana_rbpf;

use crate::{native_loader, solana_program, system_program};
use alloc::{boxed::Box, vec, vec::Vec};
use bincode::error::DecodeError;
use hashbrown::HashMap;
use solana_account_info::AccountInfo;
use solana_clock::Epoch;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::{Pubkey, PUBKEY_BYTES};
use solana_rbpf::{
    ebpf,
    elf::Executable,
    memory_region::{MemoryCowCallback, MemoryMapping, MemoryRegion},
    vm::ContextObject,
};

pub type StdResult<T, E> = Result<T, E>;

#[derive(Clone, PartialEq, Eq)]
pub struct AllocErr;

#[derive(Clone)]
pub struct SerializedAccountMetadata {
    pub original_data_len: usize,
    pub vm_data_addr: u64,
    pub vm_key_addr: u64,
    pub vm_lamports_addr: u64,
    pub vm_owner_addr: u64,
}

pub struct SyscallContext {
    pub allocator: BpfAllocator,
    pub accounts_metadata: Vec<SerializedAccountMetadata>,
    pub trace_log: Vec<[u64; 12]>,
}

pub fn address_is_aligned<T>(address: u64) -> bool {
    (address as *mut T as usize)
        .checked_rem(align_of::<T>())
        .map(|rem| rem == 0)
        .expect("T to be non-zero aligned")
}

use crate::{
    account::{
        to_account, Account, AccountSharedData, InheritableAccountFields, ReadableAccount,
        WritableAccount, DUMMY_INHERITABLE_ACCOUNT_FIELDS,
    },
    common::GlobalLamportsBalance,
    context::{BpfAllocator, TransactionContext},
    error::{RuntimeError, SvmError},
    fluentbase::common::{GlobalBalance, SYSTEM_PROGRAMS_KEYS},
    native_loader::create_loadable_account_with_fields2,
    solana_program::{loader_v4, sysvar::Sysvar},
};
use fluentbase_sdk::{calc_create_metadata_address, debug_log, keccak256, Address, Bytes, MetadataAPI, MetadataStorageAPI, SharedAPI, StorageAPI, B256, PRECOMPILE_SVM_RUNTIME, U256};
use solana_rbpf::ebpf::MM_HEAP_START;

pub fn create_memory_mapping<'a, 'b, C: ContextObject>(
    executable: &'a Executable<C>,
    stack: &'b mut [u8],
    heap: &'b mut [u8],
    additional_regions: Vec<MemoryRegion>,
    cow_cb: Option<MemoryCowCallback>,
) -> Result<MemoryMapping<'a>, Box<dyn core::error::Error>> {
    let config = executable.get_config();
    let sbpf_version = executable.get_sbpf_version();
    let regions: Vec<MemoryRegion> = vec![
        executable.get_ro_region(),
        MemoryRegion::new_writable_gapped(
            stack,
            ebpf::MM_STACK_START,
            if !sbpf_version.dynamic_stack_frames() && config.enable_stack_frame_gaps {
                config.stack_frame_size as u64
            } else {
                0
            },
        ),
        MemoryRegion::new_writable(heap, MM_HEAP_START),
    ]
    .into_iter()
    .chain(additional_regions)
    .collect();

    Ok(if let Some(cow_cb) = cow_cb {
        MemoryMapping::new_with_cow(regions, cow_cb, config, sbpf_version)?
    } else {
        MemoryMapping::new(regions, config, sbpf_version)?
    })
}

pub fn is_zeroed(buf: &[u8]) -> bool {
    const ZEROS_LEN: usize = 1024;
    const ZEROS: [u8; ZEROS_LEN] = [0; ZEROS_LEN];
    let mut chunks = buf.chunks_exact(ZEROS_LEN);

    #[allow(clippy::indexing_slicing)]
    {
        chunks.all(|chunk| chunk == &ZEROS[..])
            && chunks.remainder() == &ZEROS[..chunks.remainder().len()]
    }
}

#[macro_export]
macro_rules! saturating_add_assign {
    ($i:expr, $v:expr) => {{
        $i = $i.saturating_add($v)
    }};
}

pub fn create_account_with_fields<S: Sysvar>(
    sysvar: &S,
    (lamports, rent_epoch): InheritableAccountFields,
) -> Account {
    let data_len = S::size_of().max(solana_bincode::serialized_size(sysvar).unwrap());
    let mut account = Account::new(lamports, data_len, &solana_program::sysvar::id());
    to_account::<S, Account>(sysvar, &mut account).unwrap();
    account.rent_epoch = rent_epoch;
    account
}

pub fn create_account_shared_data_with_fields<S: Sysvar>(
    sysvar: &S,
    fields: InheritableAccountFields,
) -> AccountSharedData {
    AccountSharedData::from(create_account_with_fields(sysvar, fields))
}

pub fn create_account_shared_data_for_test<S: Sysvar>(sysvar: &S) -> AccountSharedData {
    AccountSharedData::from(create_account_with_fields(
        sysvar,
        DUMMY_INHERITABLE_ACCOUNT_FIELDS,
    ))
}

pub fn create_account_for_test<S: Sysvar>(sysvar: &S) -> Account {
    create_account_with_fields(sysvar, DUMMY_INHERITABLE_ACCOUNT_FIELDS)
}

/// Create `AccountInfo`s
pub fn create_is_signer_account_infos<'a>(
    accounts: &'a mut [(&'a Pubkey, bool, &'a mut Account)],
) -> Vec<AccountInfo<'a>> {
    accounts
        .iter_mut()
        .map(|(key, is_signer, account)| {
            AccountInfo::new(
                key,
                *is_signer,
                false,
                &mut account.lamports,
                &mut account.data,
                &account.owner,
                account.executable,
                account.rent_epoch,
            )
        })
        .collect()
}

#[macro_export]
macro_rules! with_mock_invoke_context {
    (
        $invoke_context:ident,
        $transaction_context:ident,
        $sdk:expr,
        $loader:expr,
        $transaction_accounts:expr $(,)?
    ) => {
        use alloc::sync::Arc;
        use $crate::{
            account::ReadableAccount,
            compute_budget::compute_budget::ComputeBudget,
            context::{EnvironmentConfig, InvokeContext, TransactionContext},
            hash::Hash,
            loaded_programs::{ProgramCacheForTxBatch, ProgramRuntimeEnvironments},
            solana_program::feature_set::feature_set_default,
            sysvar_cache::SysvarCache,
        };
        let compute_budget = ComputeBudget::default();
        let $transaction_context = TransactionContext::new(
            $transaction_accounts,
            compute_budget.max_instruction_stack_depth,
            compute_budget.max_instruction_trace_length,
        );
        let mut sysvar_cache = SysvarCache::default();
        sysvar_cache.fill_missing_entries(|pubkey, callback| {
            for index in 0..$transaction_context.get_number_of_accounts() {
                if $transaction_context
                    .get_key_of_account_at_index(index)
                    .unwrap()
                    == pubkey
                {
                    callback(
                        $transaction_context
                            .get_account_at_index(index)
                            .unwrap()
                            .borrow()
                            .data(),
                    );
                }
            }
        });
        let environment_config = EnvironmentConfig::new(
            Hash::default(),
            Arc::new(feature_set_default()),
            sysvar_cache,
        );
        let program_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: $loader.clone(),
                program_runtime_v2: $loader.clone(),
            },
        );
        let $invoke_context = InvokeContext::new(
            $transaction_context,
            program_cache_for_tx_batch,
            environment_config,
            compute_budget,
            $sdk,
        );
    };
}

pub fn derive_metadata_addr(pk: &Pubkey, alt_precompile_address: Option<Address>) -> Address {
    debug_log!();
    let pk_bytes = pk.to_bytes();
    let derived_addr = calc_create_metadata_address(
        &alt_precompile_address.unwrap_or(PRECOMPILE_SVM_RUNTIME),
        &U256::from_be_bytes(pk_bytes),
    );
    debug_log!();
    derived_addr
}

pub fn storage_read_metadata_params<API: MetadataAPI>(
    api: &API,
    pk: &Pubkey,
    alt_precompile_address: Option<Address>,
) -> Result<(Address, u32), SvmError> {
    // let pubkey_hash = keccak256(pubkey.as_ref());
    debug_log!();
    let derived_metadata_address = derive_metadata_addr(pk, alt_precompile_address);
    debug_log!();
    let metadata_size_result = api.metadata_size(&derived_metadata_address);
    // if !metadata_size_result.status.is_ok() {
    //     return Err(metadata_size_result.status.into());
    // }
    debug_log!();
    let metadata_len = metadata_size_result.data.0;
    Ok((derived_metadata_address, metadata_len))
}

pub fn is_program_exists<API: MetadataAPI>(
    api: &API,
    program_id: &Pubkey,
    alt_precompile_address: Option<Address>,
) -> Result<bool, SvmError> {
    let is_exists = if SYSTEM_PROGRAMS_KEYS.contains(program_id) {
        true
    } else {
        let account_metadata =
            storage_read_metadata_params(api, program_id, alt_precompile_address);
        account_metadata.is_ok() && account_metadata?.1 > 0
    };
    Ok(is_exists)
}

pub fn storage_read_metadata<API: MetadataAPI>(
    api: &API,
    pubkey: &Pubkey,
    alt_precompile_address: Option<Address>,
) -> Result<Bytes, SvmError> {
    let ((derived_metadata_address, metadata_len)) =
        storage_read_metadata_params(api, pubkey, alt_precompile_address)?;
    let metadata_copy = api.metadata_copy(&derived_metadata_address, 0, metadata_len);
    if !metadata_copy.status.is_ok() {
        return Err(metadata_copy.status.into());
    }
    let buffer = metadata_copy.data;
    Ok(buffer)
}

pub fn storage_write_metadata<MAPI: MetadataAPI>(
    api: &mut MAPI,
    pubkey: &Pubkey,
    metadata: Bytes,
    alt_precompile_address: Option<Address>,
) -> Result<(), SvmError> {
    let ((derived_metadata_address, metadata_len)) =
        storage_read_metadata_params(api, pubkey, alt_precompile_address)?;
    if metadata_len == 0 {
        api.metadata_create(&U256::from_be_bytes(pubkey.to_bytes()), metadata)
            .expect("metadata creation failed");
    } else {
        api.metadata_write(&derived_metadata_address, 0, metadata)
            .expect("metadata write failed");
    }
    Ok(())
}

pub fn account_data_encode_into(account_data: &AccountSharedData, out: &mut Vec<u8>) {
    let need = 1 + size_of::<Pubkey>() + account_data.data().len();
    out.reserve_exact(need);
    let mut offset = out.len();
    out.push(account_data.executable() as u8);
    offset += 1;
    out.extend_from_slice(account_data.owner().as_ref());
    offset += size_of::<Pubkey>();
    out.extend_from_slice(account_data.data());
}

pub fn account_data_encode_to_vec(account_data: &AccountSharedData) -> Vec<u8> {
    let mut out = vec![];
    account_data_encode_into(account_data, &mut out);
    out
}

pub fn account_data_try_decode(buffer: &[u8]) -> Result<AccountSharedData, SvmError> {
    const MIN_LEN: usize = 1 + size_of::<Pubkey>();
    if buffer.len() < MIN_LEN {
        return Err(SvmError::RuntimeError(RuntimeError::InvalidLength));
    }
    let executable = buffer[0] > 0;
    let owner = Pubkey::new_from_array(buffer[1..1 + size_of::<Pubkey>()].try_into().unwrap());
    let data = &buffer[1 + size_of::<Pubkey>()..];
    // let lamports = GlobalLamportsBalance::get(api, &pk);
    // TODO
    let lamports = 111;
    let account_data = AccountSharedData::create(
        lamports,
        data.to_vec(),
        owner,
        executable,
        Default::default(),
    );
    Ok(account_data)
}

pub fn storage_read_account_data<API: MetadataAPI + MetadataStorageAPI>(
    api: &API,
    pk: &Pubkey,
    alt_precompile_address: Option<Address>,
) -> Result<AccountSharedData, SvmError> {
    if pk == &system_program::id() {
        return Ok(create_loadable_account_with_fields2(
            "system_program_id",
            &native_loader::id(),
        ));
    } else if pk == &loader_v4::id() {
        return Ok(create_loadable_account_with_fields2(
            "loader_v4_id",
            &native_loader::id(),
        ));
    };
    let buffer = storage_read_metadata(api, pk, alt_precompile_address)?;
    if buffer.len() < 1 + size_of::<Pubkey>() {
        return Err(SvmError::RuntimeError(RuntimeError::InvalidLength));
    }
    let executable = buffer[0] > 0;
    let owner = Pubkey::new_from_array(buffer[1..1 + size_of::<Pubkey>()].try_into().unwrap());
    let data = &buffer[1 + size_of::<Pubkey>()..];
    let lamports = GlobalLamportsBalance::get(api, &pk);
    let account_data = AccountSharedData::create(
        lamports,
        data.to_vec(),
        owner,
        executable,
        Default::default(),
    );
    Ok(account_data)
}

pub fn storage_write_account_data<API: MetadataAPI + MetadataStorageAPI>(
    api: &mut API,
    pk: &Pubkey,
    account_data: &AccountSharedData,
    alt_precompile_address: Option<Address>,
) -> Result<(), SvmError> {
    debug_log!();
    let mut buffer = vec![];
    account_data_encode_into(account_data, &mut buffer);
    storage_write_metadata(api, pk, buffer.into(), alt_precompile_address)?;
    debug_log!();
    GlobalLamportsBalance::set(api, &pk, account_data.lamports());
    Ok(())
}

pub(crate) fn storage_read_account_data_or_default<API: MetadataAPI + MetadataStorageAPI>(
    api: &API,
    pk: &Pubkey,
    space_default: usize,
    owner_default: Option<&Pubkey>,
    alt_precompile_address: Option<Address>,
) -> AccountSharedData {
    storage_read_account_data(api, pk, alt_precompile_address).unwrap_or_else(|_e| {
        let lamports = GlobalBalance::get(api, pk);
        AccountSharedData::new(
            lamports,
            space_default,
            owner_default.unwrap_or(&system_program::id()),
        )
    })
}

pub fn extract_accounts(
    transaction_context: &TransactionContext,
) -> Result<HashMap<Pubkey, AccountSharedData>, SvmError> {
    let mut accounts =
        HashMap::with_capacity(transaction_context.get_number_of_accounts() as usize);
    for account_idx in 0..transaction_context.get_number_of_accounts() {
        let account_key = transaction_context.get_key_of_account_at_index(account_idx)?;
        let account_data = transaction_context.get_account_at_index(account_idx)?;
        accounts.insert(
            account_key.clone(),
            account_data.borrow().to_account_shared_data(),
        );
    }
    Ok(accounts)
}

pub fn update_accounts(
    transaction_context: &mut TransactionContext,
    accounts: &HashMap<Pubkey, AccountSharedData>,
) {
    for (pk, data) in accounts {
        let idx = transaction_context
            .find_index_of_account(pk)
            .expect("each account must be presented");
        let mut account = transaction_context
            .get_account_at_index(idx)
            .expect("each account must be presented");
        // let mut_data = account.borrow_mut().data_as_mut_slice();
        *account.borrow_mut() = data.clone();
    }
}

pub fn serialize_svm_program_params(
    dst: &mut Vec<u8>,
    program_id: &Pubkey,
    account_metas: &[AccountMeta],
    instruction_data: &[u8],
) -> Result<(), SvmError> {
    let additional_capacity =
        PUBKEY_BYTES + 1 + account_metas.len() * size_of::<AccountMeta>() + instruction_data.len();
    dst.reserve(additional_capacity);

    dst.extend_from_slice(&program_id.to_bytes());
    dst.push(account_metas.len() as u8);
    for am in account_metas.iter() {
        let am_vec = solana_bincode::serialize(am)?;
        dst.extend_from_slice(&am_vec);
    }
    dst.extend_from_slice(&instruction_data);
    Ok(())
}

pub fn serialize_svm_program_params_from_instruction(
    dst: &mut Vec<u8>,
    instruction: &Instruction,
) -> Result<(), SvmError> {
    serialize_svm_program_params(
        dst,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
}

pub fn deserialize_svm_program_params(
    src: &[u8],
) -> Result<(Pubkey, Vec<AccountMeta>, &[u8]), SvmError> {
    let mut expected_len = PUBKEY_BYTES + 1;
    if src.len() < expected_len {
        return Err(RuntimeError::InvalidLength.into());
    }
    let account_metas_count = src[PUBKEY_BYTES] as usize;
    expected_len += account_metas_count * size_of::<AccountMeta>();
    if src.len() < expected_len {
        return Err(RuntimeError::InvalidLength.into());
    }

    let mut offset = 0;
    let program_id = Pubkey::new_from_array(src[offset..offset + PUBKEY_BYTES].try_into().unwrap());
    offset += PUBKEY_BYTES + 1;
    let mut account_metas: Vec<AccountMeta> = Vec::with_capacity(account_metas_count);
    for _i in 0..account_metas_count {
        let account_meta_raw = &src[offset..offset + size_of::<AccountMeta>()];
        let account_meta: AccountMeta = solana_bincode::deserialize(account_meta_raw)?;
        account_metas.push(account_meta);
        offset += size_of::<AccountMeta>();
    }
    let input = &src[offset..];
    Ok((program_id, account_metas, input))
}

pub fn deserialize_svm_program_params_into_instruction(
    src: &[u8],
) -> Result<Instruction, SvmError> {
    let (program_id, account_metas, input) = deserialize_svm_program_params(src)?;
    Ok(Instruction {
        program_id,
        accounts: account_metas,
        data: input.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use crate::helpers::{deserialize_svm_program_params, serialize_svm_program_params};
    use solana_instruction::AccountMeta;
    use solana_pubkey::Pubkey;

    #[test]
    fn ser_deser_svm_program_params() {
        let src: (Pubkey, Vec<AccountMeta>, &[u8]) = (
            Pubkey::new_unique(),
            vec![AccountMeta::new(Pubkey::new_unique(), true)],
            &[1, 2, 3, 4, 5],
        );
        let mut serialized = vec![];
        serialize_svm_program_params(&mut serialized, &src.0, &src.1, &src.2).unwrap();
        let deserialized = deserialize_svm_program_params(&serialized).unwrap();
        assert_eq!(src, deserialized);
    }
}
