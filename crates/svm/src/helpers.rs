extern crate solana_rbpf;

use crate::{solana_program, system_program};
use alloc::{boxed::Box, vec, vec::Vec};
use bincode::error::DecodeError;
use solana_bincode::{deserialize, serialize, serialized_size};
use solana_clock::Epoch;
use solana_pubkey::Pubkey;
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

use crate::account::{ReadableAccount, WritableAccount};
use crate::common::GlobalLamportsBalance;
use crate::error::RuntimeError;
use crate::fluentbase::common::GlobalBalance;
use crate::{
    account::{
        to_account, Account, AccountSharedData, InheritableAccountFields,
        DUMMY_INHERITABLE_ACCOUNT_FIELDS,
    },
    context::BpfAllocator,
    error::SvmError,
    solana_program::sysvar::Sysvar,
};
use fluentbase_sdk::{
    calc_create4_address, debug_log_ext, keccak256, Bytes, MetadataAPI, PRECOMPILE_SVM_RUNTIME,
};
use fluentbase_types::{Address, MetadataStorageAPI, SharedAPI, StorageAPI, B256};
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
    let data_len = S::size_of().max(serialized_size(sysvar).unwrap());
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

pub fn storage_metadata_params<API: MetadataAPI>(
    api: &API,
    pubkey: &Pubkey,
) -> Result<(B256, Address, u32), SvmError> {
    // let pubkey_hash = keccak256(pubkey.as_ref());
    let pubkey_hash: B256 = pubkey.to_bytes().into();
    let derived_metadata_address =
        calc_create4_address(&PRECOMPILE_SVM_RUNTIME, &pubkey_hash.into(), |v| {
            keccak256(v)
        });
    let metadata_size_result = api.metadata_size(&derived_metadata_address);
    if !metadata_size_result.status.is_ok() {
        return Err(metadata_size_result.status.into());
    }
    let metadata_len = metadata_size_result.data.0;
    Ok((pubkey_hash, derived_metadata_address, metadata_len))
}

pub fn storage_read_metadata<API: MetadataAPI>(
    api: &API,
    pubkey: &Pubkey,
) -> Result<Bytes, SvmError> {
    let ((_, derived_metadata_address, metadata_len)) = storage_metadata_params(api, pubkey)?;
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
) -> Result<(), SvmError> {
    let ((pubkey_hash, derived_metadata_address, metadata_len)) =
        storage_metadata_params(api, pubkey)?;
    if metadata_len == 0 {
        api.metadata_create(&pubkey_hash.into(), metadata)
            .expect("metadata creation failed");
    } else {
        api.metadata_write(&derived_metadata_address, 0, metadata)
            .expect("metadata write failed");
    }
    Ok(())
}

pub fn storage_read_account_data<API: MetadataAPI + MetadataStorageAPI>(
    api: &API,
    pk: &Pubkey,
) -> Result<AccountSharedData, SvmError> {
    let buffer = storage_read_metadata(api, pk)?;
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
) -> Result<(), SvmError> {
    let mut buffer = vec![0u8; 1 + size_of::<Pubkey>() + account_data.data().len()];
    buffer[0] = account_data.executable() as u8;
    buffer[1..1 + size_of::<Pubkey>()].copy_from_slice(account_data.owner().as_ref());
    buffer[1 + size_of::<Pubkey>()..].copy_from_slice(account_data.data());
    storage_write_metadata(api, pk, buffer.into())?;
    GlobalLamportsBalance::set(api, &pk, account_data.lamports());
    Ok(())
}

pub(crate) fn storage_read_account_data_or_default<API: MetadataAPI + MetadataStorageAPI>(
    api: &API,
    pk: &Pubkey,
    space_default: usize,
    owner_default: Option<&Pubkey>,
) -> AccountSharedData {
    storage_read_account_data(api, pk).unwrap_or_else(|_e| {
        let lamports = GlobalBalance::get(api, pk);
        AccountSharedData::new(
            lamports,
            space_default,
            owner_default.unwrap_or(&system_program::id()),
        )
    })
}
