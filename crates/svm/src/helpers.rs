extern crate solana_rbpf;

use crate::solana_program;
use alloc::{boxed::Box, str::Utf8Error, string::String, vec, vec::Vec};
use core::{
    fmt,
    fmt::{Display, Formatter},
};
use solana_bincode::{deserialize, serialize, serialized_size};
use solana_pubkey::{Pubkey, PubkeyError};
use solana_rbpf::{
    ebpf,
    elf::Executable,
    memory_region::{MemoryCowCallback, MemoryMapping, MemoryRegion},
    vm::ContextObject,
};

pub type StdResult<T, E> = Result<T, E>;

pub const INSTRUCTION_METER_BUDGET: u64 = 1024 * 1024;

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
        to_account,
        Account,
        AccountSharedData,
        InheritableAccountFields,
        DUMMY_INHERITABLE_ACCOUNT_FIELDS,
    },
    context::BpfAllocator,
    error::SvmError,
    solana_program::sysvar::Sysvar,
};
use fluentbase_sdk::{calc_create4_address, keccak256, MetadataAPI, PRECOMPILE_SVM_RUNTIME};
use solana_rbpf::ebpf::MM_HEAP_START;

/// Error definitions

#[derive(Debug, PartialEq, Eq)]
pub enum SyscallError {
    InvalidString(Utf8Error, Vec<u8>),
    Abort,
    Panic(String, u64, u64),
    InvokeContextBorrowFailed,
    MalformedSignerSeed(Utf8Error, Vec<u8>),
    BadSeeds(PubkeyError),
    ProgramNotSupported(Pubkey),
    UnalignedPointer,
    TooManySigners,
    InstructionTooLarge(usize, usize),
    TooManyAccounts,
    CopyOverlapping,
    ReturnDataTooLarge(u64, u64),
    TooManySlices,
    InvalidLength,
    MaxInstructionDataLenExceeded {
        data_len: u64,
        max_data_len: u64,
    },
    MaxInstructionAccountsExceeded {
        num_accounts: u64,
        max_accounts: u64,
    },
    MaxInstructionAccountInfosExceeded {
        num_account_infos: u64,
        max_account_infos: u64,
    },
    InvalidAttribute,
    InvalidPointer,
    ArithmeticOverflow,
}

impl Display for SyscallError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            SyscallError::InvalidString(_, _) => write!(f, "SyscallError::InvalidString"),
            SyscallError::Abort => write!(f, "SyscallError::Abort"),
            SyscallError::Panic(_, _, _) => write!(f, "SyscallError::Panic"),
            SyscallError::InvokeContextBorrowFailed => {
                write!(f, "SyscallError::InvokeContextBorrowFailed")
            }
            SyscallError::MalformedSignerSeed(_, _) => {
                write!(f, "SyscallError::MalformedSignerSeed")
            }
            SyscallError::BadSeeds(_) => write!(f, "SyscallError::BadSeeds"),
            SyscallError::ProgramNotSupported(_) => write!(f, "SyscallError::ProgramNotSupported"),
            SyscallError::UnalignedPointer => write!(f, "SyscallError::UnalignedPointer"),
            SyscallError::TooManySigners => write!(f, "SyscallError::TooManySigners"),
            SyscallError::InstructionTooLarge(_, _) => {
                write!(f, "SyscallError::InstructionTooLarge")
            }
            SyscallError::TooManyAccounts => write!(f, "SyscallError::TooManyAccounts"),
            SyscallError::CopyOverlapping => write!(f, "SyscallError::CopyOverlapping"),
            SyscallError::ReturnDataTooLarge(_, _) => write!(f, "SyscallError::ReturnDataTooLarge"),
            SyscallError::TooManySlices => write!(f, "SyscallError::TooManySlices"),
            SyscallError::InvalidLength => write!(f, "SyscallError::InvalidLength"),
            SyscallError::MaxInstructionDataLenExceeded { .. } => {
                write!(f, "SyscallError::MaxInstructionDataLenExceeded")
            }
            SyscallError::MaxInstructionAccountsExceeded { .. } => {
                write!(f, "SyscallError::MaxInstructionAccountsExceeded")
            }
            SyscallError::MaxInstructionAccountInfosExceeded { .. } => {
                write!(f, "SyscallError::MaxInstructionAccountInfosExceeded")
            }
            SyscallError::InvalidAttribute => write!(f, "SyscallError::InvalidAttribute"),
            SyscallError::InvalidPointer => write!(f, "SyscallError::InvalidPointer"),
            SyscallError::ArithmeticOverflow => write!(f, "SyscallError::ArithmeticOverflow"),
        }
    }
}

impl core::error::Error for SyscallError {}

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

/// Only used in macro, do not use directly!
pub fn calculate_heap_cost(heap_size: u32, heap_cost: u64) -> u64 {
    const KIBIBYTE: u64 = 1024;
    const PAGE_SIZE_KB: u64 = 32;
    let mut rounded_heap_size = u64::from(heap_size);
    rounded_heap_size =
        rounded_heap_size.saturating_add(PAGE_SIZE_KB.saturating_mul(KIBIBYTE).saturating_sub(1));
    rounded_heap_size
        .checked_div(PAGE_SIZE_KB.saturating_mul(KIBIBYTE))
        .expect("PAGE_SIZE_KB * KIBIBYTE > 0")
        .saturating_sub(1)
        .saturating_mul(heap_cost)
}

pub fn create_account_for_test<S: Sysvar>(sysvar: &S) -> Account {
    create_account_with_fields(sysvar, DUMMY_INHERITABLE_ACCOUNT_FIELDS)
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

#[macro_export]
macro_rules! select_api {
    ($optional:expr, $alt:expr, $callback:expr) => {
        if let Some(v) = $optional {
            $callback(*v)
        } else {
            $callback($alt)
        }
    };
}

pub fn storage_read_account_data<API: MetadataAPI>(
    api: &API,
    pubkey: &Pubkey,
) -> Result<AccountSharedData, SvmError> {
    let pubkey_hash = keccak256(pubkey.as_ref());
    let derived_metadata_address =
        calc_create4_address(&PRECOMPILE_SVM_RUNTIME, &pubkey_hash.into(), |v| {
            keccak256(v)
        });
    let metadata_size_result = api.metadata_size(&derived_metadata_address);
    if !metadata_size_result.status.is_ok() {
        return Err(metadata_size_result.status.into());
    }
    let metadata_len = metadata_size_result.data.0;
    let metadata_copy = api.metadata_copy(&derived_metadata_address, 0, metadata_len);
    if !metadata_copy.status.is_ok() {
        return Err(metadata_copy.status.into());
    }
    let buffer = metadata_copy.data;
    let deserialize_result = deserialize(&buffer);
    Ok(deserialize_result?)
}

pub fn storage_write_account_data<API: MetadataAPI>(
    api: &mut API,
    pubkey: &Pubkey,
    account_data: &AccountSharedData,
) -> Result<(), SvmError> {
    let account_data = serialize(account_data)?;
    let pubkey_hash = keccak256(pubkey.as_ref());
    let derived_metadata_address =
        calc_create4_address(&PRECOMPILE_SVM_RUNTIME, &pubkey_hash.into(), |v| {
            keccak256(v)
        });
    let (metadata_size, _, _, _) = api
        .metadata_size(&derived_metadata_address)
        .expect("metadata size")
        .data;
    if metadata_size == 0 {
        api.metadata_create(&pubkey_hash.into(), account_data.into())
            .expect("metadata creation failed");
    } else {
        api.metadata_write(&derived_metadata_address, 0, account_data.into())
            .expect("metadata write failed");
    }
    Ok(())
}
