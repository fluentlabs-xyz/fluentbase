extern crate solana_rbpf;

use crate::{alloc::string::ToString, solana_program};
use alloc::{boxed::Box, rc::Rc, str::Utf8Error, string::String, vec, vec::Vec};
use core::{
    cell::RefCell,
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

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AllocErr;
impl Display for AllocErr {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.write_str("Error: Memory allocation failed")
    }
}

#[derive(Debug, Clone)]
pub struct SerializedAccountMetadata {
    pub original_data_len: usize,
    pub vm_data_addr: u64,
    pub vm_key_addr: u64,
    pub vm_lamports_addr: u64,
    pub vm_owner_addr: u64,
}

#[derive(Debug)]
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

const LOG_MESSAGES_BYTES_LIMIT: usize = 10 * 1000;

pub struct LogCollector {
    messages: Vec<String>,
    bytes_written: usize,
    bytes_limit: Option<usize>,
    limit_warning: bool,
}

impl Default for LogCollector {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            bytes_written: 0,
            bytes_limit: Some(LOG_MESSAGES_BYTES_LIMIT),
            limit_warning: false,
        }
    }
}

impl LogCollector {
    pub fn log(&mut self, message: &str) {
        let Some(limit) = self.bytes_limit else {
            self.messages.push(message.to_string());
            return;
        };

        let bytes_written = self.bytes_written.saturating_add(message.len());
        if bytes_written >= limit {
            if !self.limit_warning {
                self.limit_warning = true;
                self.messages.push(String::from("Log truncated"));
            }
        } else {
            self.bytes_written = bytes_written;
            self.messages.push(message.to_string());
        }
    }

    pub fn get_recorded_content(&self) -> &[String] {
        self.messages.as_slice()
    }

    pub fn new_ref() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self::default()))
    }

    pub fn new_ref_with_limit(bytes_limit: Option<usize>) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            bytes_limit,
            ..Self::default()
        }))
    }

    pub fn into_messages(self) -> Vec<String> {
        self.messages
    }
}

/// Convenience macro to log a message with an `Option<Rc<RefCell<LogCollector>>>`
#[macro_export]
macro_rules! ic_logger_msg {
    ($log_collector:expr, $message:expr) => {
        $crate::log_collector::log::debug!(
            target: "solana_runtime::message_processor::stable_log",
            "{}",
            $message
        );
        if let Some(log_collector) = $log_collector.as_ref() {
            if let Ok(mut log_collector) = log_collector.try_borrow_mut() {
                log_collector.log($message);
            }
        }
    };
    ($log_collector:expr, $fmt:expr, $($arg:tt)*) => {
        $crate::log_collector::log::debug!(
            target: "solana_runtime::message_processor::stable_log",
            $fmt,
            $($arg)*
        );
        if let Some(log_collector) = $log_collector.as_ref() {
            if let Ok(mut log_collector) = log_collector.try_borrow_mut() {
                log_collector.log(&format!($fmt, $($arg)*));
            }
        }
    };
}

/// Convenience macro to log a message with an `InvokeContext`
#[macro_export]
macro_rules! ic_msg {
    ($invoke_context:expr, $message:expr) => {
        $crate::ic_logger_msg!($invoke_context.get_log_collector(), $message)
    };
    ($invoke_context:expr, $fmt:expr, $($arg:tt)*) => {
        $crate::ic_logger_msg!($invoke_context.get_log_collector(), $fmt, $($arg)*)
    };
}

/// Error definitions

#[derive(Debug, /* ThisError, */ PartialEq, Eq)]
pub enum SyscallError {
    // #[error("{0}: {1:?}")]
    InvalidString(Utf8Error, Vec<u8>),
    // #[error("SBF program panicked")]
    Abort,
    // #[error("SBF program Panicked in {0} at {1}:{2}")]
    Panic(String, u64, u64),
    // #[error("Cannot borrow invoke context")]
    InvokeContextBorrowFailed,
    // #[error("Malformed signer seed: {0}: {1:?}")]
    MalformedSignerSeed(Utf8Error, Vec<u8>),
    // #[error("Could not create program address with signer seeds: {0}")]
    BadSeeds(PubkeyError),
    // #[error("Program {0} not supported by inner instructions")]
    ProgramNotSupported(Pubkey),
    // #[error("Unaligned pointer")]
    UnalignedPointer,
    // #[error("Too many signers")]
    TooManySigners,
    // #[error("Instruction passed to inner instruction is too large ({0} > {1})")]
    InstructionTooLarge(usize, usize),
    // #[error("Too many accounts passed to inner instruction")]
    TooManyAccounts,
    // #[error("Overlapping copy")]
    CopyOverlapping,
    // #[error("Return data too large ({0} > {1})")]
    ReturnDataTooLarge(u64, u64),
    // #[error("Hashing too many sequences")]
    TooManySlices,
    // #[error("InvalidLength")]
    InvalidLength,
    // #[error("Invoked an instruction with data that is too large ({data_len} >
    // {max_data_len})")]
    MaxInstructionDataLenExceeded {
        data_len: u64,
        max_data_len: u64,
    },
    // #[error("Invoked an instruction with too many accounts ({num_accounts} > {max_accounts})")]
    MaxInstructionAccountsExceeded {
        num_accounts: u64,
        max_accounts: u64,
    },
    // #[error("Invoked an instruction with too many account info's ({num_account_infos} >
    // {max_account_infos})" )]
    MaxInstructionAccountInfosExceeded {
        num_account_infos: u64,
        max_account_infos: u64,
    },
    // #[error("InvalidAttribute")]
    InvalidAttribute,
    // #[error("Invalid pointer")]
    InvalidPointer,
    // #[error("Arithmetic overflow")]
    ArithmeticOverflow,
}

impl Display for SyscallError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            SyscallError::InvalidString(_, _) => write!(f, "InvalidString"),
            SyscallError::Abort => write!(f, "Abort"),
            SyscallError::Panic(_, _, _) => write!(f, "Panic"),
            SyscallError::InvokeContextBorrowFailed => write!(f, "InvokeContextBorrowFailed"),
            SyscallError::MalformedSignerSeed(_, _) => write!(f, "MalformedSignerSeed"),
            SyscallError::BadSeeds(_) => write!(f, "BadSeeds"),
            SyscallError::ProgramNotSupported(_) => write!(f, "ProgramNotSupported"),
            SyscallError::UnalignedPointer => write!(f, "UnalignedPointer"),
            SyscallError::TooManySigners => write!(f, "TooManySigners"),
            SyscallError::InstructionTooLarge(_, _) => write!(f, "InstructionTooLarge"),
            SyscallError::TooManyAccounts => write!(f, "TooManyAccounts"),
            SyscallError::CopyOverlapping => write!(f, "CopyOverlapping"),
            SyscallError::ReturnDataTooLarge(_, _) => write!(f, "ReturnDataTooLarge"),
            SyscallError::TooManySlices => write!(f, "TooManySlices"),
            SyscallError::InvalidLength => write!(f, "InvalidLength"),
            SyscallError::MaxInstructionDataLenExceeded { .. } => {
                write!(f, "MaxInstructionDataLenExceeded")
            }
            SyscallError::MaxInstructionAccountsExceeded { .. } => {
                write!(f, "MaxInstructionAccountsExceeded")
            }
            SyscallError::MaxInstructionAccountInfosExceeded { .. } => {
                write!(f, "MaxInstructionAccountInfosExceeded")
            }
            SyscallError::InvalidAttribute => write!(f, "InvalidAttribute"),
            SyscallError::InvalidPointer => write!(f, "InvalidPointer"),
            SyscallError::ArithmeticOverflow => write!(f, "ArithmeticOverflow"),
        }
    }
}

impl core::error::Error for SyscallError {}

// declare_builtin_function!(
//     SyscallStubInterceptor<SDK: SharedAPI>,
//     fn rust(
//         invoke_context: &mut InvokeContext<SDK>,
//         addr: u64,
//         len: u64,
//         arg3: u64,
//         arg4: u64,
//         arg5: u64,
//         memory_mapping: &mut MemoryMapping,
//     ) -> Result<u64, Error> {
//         Ok(0)
//     }
// );

// declare_builtin_function!(
//     /// Panic syscall function, called when the SBF program calls 'sol_panic_()`
//     /// Causes the SBF program to be halted immediately
//     SyscallPanic<SDK: SharedAPI>,
//     fn rust(
//         _invoke_context: &mut InvokeContext<SDK>,
//         file: u64,
//         len: u64,
//         line: u64,
//         column: u64,
//         _arg5: u64,
//         memory_mapping: &mut MemoryMapping,
//     ) -> Result<u64, Box<dyn core::error::Error>> {
//         // consume_compute_meter(invoke_context, len)?;
//         //
//         // translate_string_and_do(
//         //     memory_mapping,
//         //     file,
//         //     len,
//         //     invoke_context.get_check_aligned(),
//         //     &mut |string: &str| Err(SyscallError::Panic(string.to_string(), line,
// column).into()),         // )
//         let error_message = "Dummy panic due to unimplemented syscall"; // Dummy error message
//         Err(SyscallError::Panic(error_message.to_string(), line, column).into())
//     }
// );

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

// pub fn create_memory_mapping<'a, C: ContextObject>(
//     executable: &'a Executable<C>,
//     stack: &mut AlignedMemory<{ HOST_ALIGN }>,
//     heap: &mut AlignedMemory<{ HOST_ALIGN }>,
//     additional_regions: Vec<MemoryRegion>,
//     cow_cb: Option<MemoryCowCallback>,
// ) -> Result<MemoryMapping<'a>, EbpfError> {
//     let config = executable.get_config();
//     let sbpf_version = executable.get_sbpf_version();
//
//     let regions: Vec<MemoryRegion> = vec![
//         executable.get_ro_region(),
//         MemoryRegion::new_writable_gapped(
//             stack.as_slice_mut(),
//             ebpf::MM_STACK_START,
//             if !sbpf_version.dynamic_stack_frames() && config.enable_stack_frame_gaps {
//                 config.stack_frame_size as u64
//             } else {
//                 0
//             },
//         ),
//         MemoryRegion::new_writable(heap.as_slice_mut(), ebpf::MM_HEAP_START),
//     ]
//     .into_iter()
//     .chain(additional_regions.into_iter())
//     .collect();
//
//     // Program code starts at `0x100000000`
//     // Stack data starts at `0x200000000`
//     // Heap data starts at `0x300000000`
//     // Program input parameters start at `0x400000000`
//     // Solana offers 4KB of stack frame space and 32KB of heap space by default
//
//     Ok(if let Some(cow_cb) = cow_cb {
//         MemoryMapping::new_with_cow(regions, cow_cb, config, sbpf_version)?
//     } else {
//         MemoryMapping::new(regions, config, sbpf_version)?
//     })
// }

#[derive(Debug, Clone)]
pub struct SvmTransactResult {
    pub reverted: bool,
    // pub program_state: ProgramState,
    // pub tx: Tx,
    // pub receipts: Vec<Receipt>,
    // pub changes: Changes,
}

// pub fn execute_generated_program<SDK: SharedAPI>(sdk: SDK, prog: &[u8], mem: &mut [u8]) ->
// Option<Vec<u8>> {     let max_instruction_count = 1024;
//     let executable = Executable::<ExecContextObject<SDK>>::from_text_bytes(
//         prog,
//         Arc::new(BuiltinProgram::new_loader(
//             Config {
//                 enable_instruction_tracing: true,
//                 ..Config::default()
//             },
//             FunctionRegistry::default(),
//         )),
//         SBPFVersion::V2,
//         FunctionRegistry::default(),
//     );
//
//     let mut executable = if let Ok(executable) = executable {
//         executable
//     } else {
//         return None;
//     };
//
//     if executable.verify::<RequisiteVerifier>().is_err() || executable.jit_compile().is_err() {
//         return None;
//     }
//
//     let (instruction_count_interpreter, tracer_interpreter, result_interpreter) = {
//         let mut context_object = ExecContextObject::new(sdk, max_instruction_count);
//         let mem_region = MemoryRegion::new_writable(mem, ebpf::MM_INPUT_START);
//         crate::create_vm!(
//             vm,
//             &executable,
//             &mut context_object,
//             stack,
//             heap,
//             vec![mem_region],
//             None
//         );
//
//         let (instruction_count_interpreter, result_interpreter) =
//             vm.execute_program(&executable, true);
//
//         let tracer_interpreter = vm.context_object_pointer;
//         (
//             instruction_count_interpreter,
//             tracer_interpreter,
//             result_interpreter,
//         )
//     };
//
//     // JIT
//
//     let mut context_object = ExecContextObject::new(sdk, max_instruction_count);
//     let mem_region = MemoryRegion::new_writable(mem, ebpf::MM_INPUT_START);
//
//     crate::create_vm!(
//         vm,
//         &executable,
//         &mut context_object,
//         stack,
//         heap,
//         vec![mem_region],
//         None
//     );
//
//     let (instruction_count_jit, result_jit) = vm.execute_program(&executable, true);
//     let tracer_jit = &vm.context_object_pointer;
//
//     if format!("{result_interpreter:?}") != format!("{result_jit:?}")
//         || !ExecContextObject::compare_trace_log(&tracer_interpreter, tracer_jit)
//     {
//         let analysis =
//             solana_rbpf::static_analysis::Analysis::from_executable(&executable).unwrap();
//         let stdout = std::io::stdout();
//         analysis
//             .disassemble_trace_log(&mut stdout.lock(), &tracer_interpreter.trace_log)
//             .unwrap();
//         analysis
//             .disassemble_trace_log(&mut stdout.lock(), &tracer_jit.trace_log)
//             .unwrap();
//         panic!();
//     }
//     if executable.get_config().enable_instruction_meter {
//         assert_eq!(instruction_count_interpreter, instruction_count_jit);
//     }
//
//     Some(mem.to_vec())
// }

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
        use solana_rent::Rent;
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
            Rent::default(),
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
            None,
            Arc::new(feature_set_default()),
            0,
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
            // Some(LogCollector::new_ref()),
            compute_budget,
            $sdk,
        );
    };
}

// pub fn mock_process_instruction<F: FnMut(&mut InvokeContext), G: FnMut(&mut InvokeContext)>(
//     loader_id: &Pubkey,
//     mut program_indices: Vec<IndexOfAccount>,
//     instruction_data: &[u8],
//     mut transaction_accounts: Vec<TransactionAccount>,
//     instruction_account_metas: Vec<AccountMeta>,
//     expected_result: Result<(), InstructionError>,
//     builtin_function: BuiltinFunctionWithContext,
//     mut pre_adjustments: F,
//     mut post_adjustments: G,
// ) -> Vec<AccountSharedData> {
//     let mut instruction_accounts: Vec<InstructionAccount> =
//         Vec::with_capacity(instruction_account_metas.len());
//     for (instruction_account_index, account_meta) in instruction_account_metas.iter().enumerate()
// {         let index_in_transaction = transaction_accounts
//             .iter()
//             .position(|(key, _account)| *key == account_meta.pubkey)
//             .unwrap_or(transaction_accounts.len())
//             as IndexOfAccount;
//         let index_in_callee = instruction_accounts
//             .get(0..instruction_account_index)
//             .unwrap()
//             .iter()
//             .position(|instruction_account| {
//                 instruction_account.index_in_transaction == index_in_transaction
//             })
//             .unwrap_or(instruction_account_index) as IndexOfAccount;
//         instruction_accounts.push(InstructionAccount {
//             index_in_transaction,
//             index_in_caller: index_in_transaction,
//             index_in_callee,
//             is_signer: account_meta.is_signer,
//             is_writable: account_meta.is_writable,
//         });
//     }
//     if program_indices.is_empty() {
//         program_indices.insert(0, transaction_accounts.len() as IndexOfAccount);
//         let processor_account = AccountSharedData::new(0, 0, &native_loader::id());
//         transaction_accounts.push((*loader_id, processor_account));
//     }
//     let pop_epoch_schedule_account = if !transaction_accounts
//         .iter()
//         .any(|(key, _)| *key == sysvar::epoch_schedule::id())
//     {
//         transaction_accounts.push((
//             sysvar::epoch_schedule::id(),
//             create_account_shared_data_for_test(&EpochSchedule::default()),
//         ));
//         true
//     } else {
//         false
//     };
//     with_mock_invoke_context!(invoke_context, transaction_context, transaction_accounts);
//     let mut program_cache_for_tx_batch = ProgramCacheForTxBatch::default();
//     program_cache_for_tx_batch.replenish(
//         *loader_id,
//         Arc::new(ProgramCacheEntry::new_builtin(0, 0, builtin_function)),
//     );
//     invoke_context.program_cache_for_tx_batch = &mut program_cache_for_tx_batch;
//     pre_adjustments(&mut invoke_context);
//     let result = invoke_context.process_instruction(
//         instruction_data,
//         &instruction_accounts,
//         &program_indices,
//         &mut 0,
//         &mut ExecuteTimings::default(),
//     );
//     assert_eq!(result, expected_result);
//     post_adjustments(&mut invoke_context);
//     let mut transaction_accounts = transaction_context.deconstruct_without_keys().unwrap();
//     if pop_epoch_schedule_account {
//         transaction_accounts.pop();
//     }
//     transaction_accounts.pop();
//     transaction_accounts
// }

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
    let (metadata_size, _, _) = api
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
