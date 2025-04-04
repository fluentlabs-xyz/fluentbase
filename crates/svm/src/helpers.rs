extern crate solana_rbpf;

use crate::alloc::string::ToString;
use crate::helpers::SyscallError::UnalignedPointer;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::str::Utf8Error;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::cell::RefCell;
use core::fmt::{Display, Formatter, Write};
use core::str::from_utf8;
use core::fmt;
use solana_rbpf::memory_region::AccessType;
use solana_rbpf::{
    aligned_memory::AlignedMemory,
    ebpf,
    ebpf::HOST_ALIGN,
    elf::Executable,
    error::EbpfError,
    memory_region::MemoryCowCallback,
    memory_region::{MemoryMapping, MemoryRegion},
    vm::ContextObject,
};

type StdResult<T, E> = Result<T, E>;

pub const INSTRUCTION_METER_BUDGET: u64 = 1024 * 1024;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AllocErr;
impl fmt::Display for AllocErr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("Error: Memory allocation failed")
    }
}

#[derive(Debug)]
pub struct BpfAllocator {
    len: u64,
    pos: u64,
}

impl BpfAllocator {
    pub fn new(len: u64) -> Self {
        Self { len, pos: 0 }
    }

    pub fn alloc(&mut self, layout: Layout) -> Result<u64, AllocErr> {
        let bytes_to_align = (self.pos as *const u8).align_offset(layout.align()) as u64;
        if self
            .pos
            .saturating_add(bytes_to_align)
            .saturating_add(layout.size() as u64)
            <= self.len
        {
            self.pos = self.pos.saturating_add(bytes_to_align);
            let addr = MM_HEAP_START.saturating_add(self.pos);
            self.pos = self.pos.saturating_add(layout.size() as u64);
            Ok(addr)
        } else {
            Err(AllocErr)
        }
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

fn address_is_aligned<T>(address: u64) -> bool {
    (address as *mut T as usize)
        .checked_rem(align_of::<T>())
        .map(|rem| rem == 0)
        .expect("T to be non-zero aligned")
}

fn translate(
    memory_mapping: &MemoryMapping,
    access_type: AccessType,
    vm_addr: u64,
    len: u64,
) -> StdResult<u64, Box<dyn core::error::Error>> {
    memory_mapping
        .map(access_type, vm_addr, len)
        .map_err(|err| err.into())
        .into()
}

use crate::account::{to_account, Account, AccountSharedData, InheritableAccountFields, DUMMY_INHERITABLE_ACCOUNT_FIELDS};
use crate::context::InvokeContext;
use fluentbase_sdk::{ExitCode, SharedAPI, StorageAPI};
use solana_program::pubkey::{Pubkey, PubkeyError, MAX_SEEDS, MAX_SEED_LEN};
use solana_program::sysvar::Sysvar;
use solana_rbpf::ebpf::MM_HEAP_START;
use solana_rbpf::error::ProgramResult;
use crate::fluentbase_helpers::{MemStorage, SvmError};
use crate::storage_helpers::{ContractPubkeyHelper, StorageChunksWriter, VariableLengthDataWriter};

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

pub type Error = Box<dyn core::error::Error>;

#[derive(Debug, /*ThisError,*/ PartialEq, Eq)]
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
    // #[error("Invoked an instruction with data that is too large ({data_len} > {max_data_len})")]
    MaxInstructionDataLenExceeded { data_len: u64, max_data_len: u64 },
    // #[error("Invoked an instruction with too many accounts ({num_accounts} > {max_accounts})")]
    MaxInstructionAccountsExceeded {
        num_accounts: u64,
        max_accounts: u64,
    },
    // #[error("Invoked an instruction with too many account info's ({num_account_infos} > {max_account_infos})"
    // )]
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
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        todo!()
    }
}

impl core::error::Error for SyscallError {}

fn translate_type_inner<'a, T>(
    memory_mapping: &MemoryMapping,
    access_type: AccessType,
    vm_addr: u64,
    check_aligned: bool,
) -> Result<&'a mut T, Box<dyn core::error::Error>> {
    let host_addr = translate(memory_mapping, access_type, vm_addr, size_of::<T>() as u64)?;
    if !check_aligned {
        #[cfg(target_pointer_width = "64")]
        { Ok(unsafe { core::mem::transmute::<u64, &mut T>(host_addr) }) }
        #[cfg(target_pointer_width = "32")]
        { Ok(unsafe { core::mem::transmute::<u32, &mut T>(host_addr as u32) }) }
    } else if !address_is_aligned::<T>(host_addr) {
        // Err(EbpfError::SyscallError::UnalignedPointer.into())
        Err(Box::new(UnalignedPointer))
    } else {
        Ok(unsafe { &mut *(host_addr as *mut T) })
    }
}
pub fn translate_type_mut<'a, T>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    check_aligned: bool,
) -> Result<&'a mut T, Box<dyn core::error::Error>> {
    translate_type_inner::<T>(memory_mapping, AccessType::Store, vm_addr, check_aligned)
}
pub fn translate_type<'a, T>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    check_aligned: bool,
) -> Result<&'a T, Box<dyn core::error::Error>> {
    translate_type_inner::<T>(memory_mapping, AccessType::Load, vm_addr, check_aligned)
        .map(|value| &*value)
}

fn translate_slice_inner<'a, T>(
    memory_mapping: &MemoryMapping,
    access_type: AccessType,
    vm_addr: u64,
    len: u64,
    check_aligned: bool,
) -> Result<&'a mut [T], Error> {
    if len == 0 {
        return Ok(&mut []);
    }

    let total_size = len.saturating_mul(size_of::<T>() as u64);
    if isize::try_from(total_size).is_err() {
        return Err(SyscallError::InvalidLength.into());
    }

    let host_addr = translate(memory_mapping, access_type, vm_addr, total_size)?;

    if check_aligned && !address_is_aligned::<T>(host_addr) {
        return Err(SyscallError::UnalignedPointer.into());
    }
    Ok(unsafe { core::slice::from_raw_parts_mut(host_addr as *mut T, len as usize) })
}

pub fn translate_slice<'a, T>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    len: u64,
    check_aligned: bool,
) -> Result<&'a [T], Error> {
    translate_slice_inner::<T>(
        memory_mapping,
        AccessType::Load,
        vm_addr,
        len,
        check_aligned,
    )
        .map(|value| &*value)
}

pub fn translate_slice_mut<'a, T>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    len: u64,
    check_aligned: bool,
) -> Result<&'a mut [T], Error> {
    translate_slice_inner::<T>(
        memory_mapping,
        AccessType::Store,
        vm_addr,
        len,
        check_aligned,
    )
}

/// Take a virtual pointer to a string (points to SBF VM memory space), translate it
/// pass it to a user-defined work function
pub fn translate_string_and_do(
    memory_mapping: &MemoryMapping,
    addr: u64,
    len: u64,
    check_aligned: bool,
    work: &mut dyn FnMut(&str) -> Result<u64, Error>,
) -> Result<u64, Error> {
    let buf = translate_slice::<u8>(memory_mapping, addr, len, check_aligned)?;
    match from_utf8(buf) {
        Ok(message) => work(message),
        Err(err) => Err(SyscallError::InvalidString(err, buf.to_vec()).into()),
    }
}

/// Check that two regions do not overlap.
///
/// Hidden to share with bpf_loader without being part of the API surface.
#[doc(hidden)]
pub fn is_nonoverlapping<N>(src: N, src_len: N, dst: N, dst_len: N) -> bool
where
    N: Ord + num_traits::SaturatingSub,
{
    // If the absolute distance between the ptrs is at least as big as the size of the other,
    // they do not overlap.
    if src > dst {
        src.saturating_sub(&dst) >= dst_len
    } else {
        dst.saturating_sub(&src) >= src_len
    }
}

pub fn memmove<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
    dst_addr: u64,
    src_addr: u64,
    n: u64,
    memory_mapping: &MemoryMapping,
) -> Result<u64, Error> {
    // if invoke_context
    //     .feature_set
    //     .is_active(&feature_set::bpf_account_data_direct_mapping::id())
    // {
    //     memmove_non_contiguous(dst_addr, src_addr, n, memory_mapping)
    // } else {
    let dst_ptr = translate_slice_mut::<u8>(
        memory_mapping,
        dst_addr,
        n,
        // invoke_context.get_check_aligned(),
        true,
    )?
        .as_mut_ptr();
    let src_ptr = translate_slice::<u8>(
        memory_mapping,
        src_addr,
        n,
        // invoke_context.get_check_aligned(),
        true,
    )?
        .as_ptr();

    unsafe { core::ptr::copy(src_ptr, dst_ptr, n as usize) };
    Ok(0)
    // }
}


pub unsafe fn memcmp(s1: &[u8], s2: &[u8], n: usize) -> i32 {
    for i in 0..n {
        let a = *s1.get_unchecked(i);
        let b = *s2.get_unchecked(i);
        if a != b {
            return (a as i32).saturating_sub(b as i32);
        };
    }

    0
}

pub fn translate_and_check_program_address_inputs<'a>(
    seeds_addr: u64,
    seeds_len: u64,
    program_id_addr: u64,
    memory_mapping: &mut MemoryMapping,
    check_aligned: bool,
) -> Result<(Vec<&'a [u8]>, &'a Pubkey), Error> {
    let untranslated_seeds =
        translate_slice::<&[u8]>(memory_mapping, seeds_addr, seeds_len, check_aligned)?;
    if untranslated_seeds.len() > MAX_SEEDS {
        return Err(SyscallError::BadSeeds(PubkeyError::MaxSeedLengthExceeded).into());
    }
    let seeds = untranslated_seeds
        .iter()
        .map(|untranslated_seed| {
            if untranslated_seed.len() > MAX_SEED_LEN {
                return Err(SyscallError::BadSeeds(PubkeyError::MaxSeedLengthExceeded).into());
            }
            translate_slice::<u8>(
                memory_mapping,
                untranslated_seed.as_ptr() as *const _ as u64,
                untranslated_seed.len() as u64,
                check_aligned,
            )
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let program_id = translate_type::<Pubkey>(memory_mapping, program_id_addr, check_aligned)?;
    Ok((seeds, program_id))
}

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
//         #[cfg(all(feature = "std", feature = "debug-print"))] {
//             // println!(
//             //     "SyscallStubInterceptor: addr {}; len {}; arg3 {}; arg4 {}; arg5 {};",
//             //     addr, len, arg3, arg4, arg5
//             // );
//         }
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
//         //     &mut |string: &str| Err(SyscallError::Panic(string.to_string(), line, column).into()),
//         // )
//         let error_message = "Dummy panic due to unimplemented syscall"; // Dummy error message
//         Err(SyscallError::Panic(error_message.to_string(), line, column).into())
//     }
// );

pub fn create_memory_mapping<'a, C: ContextObject>(
    executable: &'a Executable<C>,
    stack: &mut AlignedMemory<{ HOST_ALIGN }>,
    heap: &mut AlignedMemory<{ HOST_ALIGN }>,
    additional_regions: Vec<MemoryRegion>,
    cow_cb: Option<MemoryCowCallback>,
) -> Result<MemoryMapping<'a>, EbpfError> {
    let config = executable.get_config();
    let sbpf_version = executable.get_sbpf_version();

    // #[cfg(feature = "debug-print")] {
    //     println!("Creating memory mapping:");
    //     println!("Stack size: {}", stack.len());
    //     println!("Heap size: {}", heap.len());
    // }

    let regions: Vec<MemoryRegion> = vec![
        executable.get_ro_region(),
        MemoryRegion::new_writable_gapped(
            stack.as_slice_mut(),
            ebpf::MM_STACK_START,
            if !sbpf_version.dynamic_stack_frames() && config.enable_stack_frame_gaps {
                config.stack_frame_size as u64
            } else {
                0
            },
        ),
        MemoryRegion::new_writable(heap.as_slice_mut(), ebpf::MM_HEAP_START),
    ]
        .into_iter()
        .chain(additional_regions.into_iter())
        .collect();

    // #[cfg(feature = "debug-print")]
    // println!("Memory regions created: {:?}", regions);
    // Program code starts at `0x100000000`
    // Stack data starts at `0x200000000`
    // Heap data starts at `0x300000000`
    // Program input parameters start at `0x400000000`
    // Solana offers 4KB of stack frame space and 32KB of heap space by default

    Ok(if let Some(cow_cb) = cow_cb {
        MemoryMapping::new_with_cow(regions, cow_cb, config, sbpf_version)?
    } else {
        MemoryMapping::new(regions, config, sbpf_version)?
    })
}

#[derive(Debug, Clone)]
pub struct SvmTransactResult {
    pub reverted: bool,
    // pub program_state: ProgramState,
    // pub tx: Tx,
    // pub receipts: Vec<Receipt>,
    // pub changes: Changes,
}

// pub fn execute_generated_program<SDK: SharedAPI>(sdk: SDK, prog: &[u8], mem: &mut [u8]) -> Option<Vec<u8>> {
//     let max_instruction_count = 1024;
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
//         #[cfg(feature = "debug-print")] {
//             println!("result_interpreter={result_interpreter:?}");
//             println!("result_jit={result_jit:?}");
//         }
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
    let data_len = S::size_of().max(bincode::serialized_size(sysvar).unwrap() as usize);
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
        use {
            solana_program::hash::Hash,
            solana_program::rent::Rent,
            alloc::sync::Arc,
            $crate::{
                feature_set::FeatureSet,
                context::TransactionContext,
                sysvar_cache::SysvarCache,
                account::ReadableAccount,
            },
            $crate::{
                context::InvokeContext,
                loaded_programs::LoadedProgramsForTxBatch,
                loaded_programs::ProgramRuntimeEnvironments,
            },
        };
        let compute_budget = $crate::compute_budget::ComputeBudget::default();
        let $transaction_context = TransactionContext::new(
            $transaction_accounts,
            Rent::default(),
            compute_budget.max_invoke_stack_height,
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
        let programs_loaded_for_tx_batch = LoadedProgramsForTxBatch::partial_default2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: $loader.clone(),
                program_runtime_v2: $loader.clone(),
            }
        );
        let programs_modified_by_tx = LoadedProgramsForTxBatch::partial_default2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: $loader.clone(),
                program_runtime_v2: $loader.clone(),
            }
        );
        let mut $invoke_context = InvokeContext::new(
            $transaction_context,
            sysvar_cache,
            $sdk,
            // Some(LogCollector::new_ref()),
            compute_budget,
            programs_loaded_for_tx_batch,
            programs_modified_by_tx,
            Arc::new(FeatureSet::all_enabled()),
            Hash::default(),
            0,
        );
    };
}

pub fn storage_read_account_data<SAPI: StorageAPI>(
    sapi: &SAPI,
    pubkey: &Pubkey,
) -> Result<AccountSharedData, SvmError> {
    let mut buffer = vec![];
    let mut storage_writer = StorageChunksWriter {
        slot_calc: Rc::new(ContractPubkeyHelper { pubkey: &pubkey }),
        _phantom: Default::default(),
    };
    storage_writer.read_data(sapi, &mut buffer)?;
    Ok(bincode::deserialize(&buffer)?)
}

pub fn storage_write_account_data<SAPI: StorageAPI>(
    sapi: &mut SAPI,
    pubkey: &Pubkey,
    account_data: &AccountSharedData,
) -> Result<(), SvmError> {
    let storage_writer = StorageChunksWriter {
        slot_calc: Rc::new(ContractPubkeyHelper { pubkey: &pubkey }),
        _phantom: Default::default(),
    };
    storage_writer.write_data(sapi, &bincode::serialize(account_data)?);
    Ok(())
}


pub mod test_utils {
    use crate::account::ReadableAccount;
    use crate::common::{check_loader_id, create_program_runtime_environment_v1, load_program_from_bytes};
    use crate::context::InvokeContext;
    use crate::loaded_programs::DELAY_VISIBILITY_SLOT_OFFSET;
    use alloc::sync::Arc;
    use fluentbase_sdk::SharedAPI;

    pub fn load_all_invoked_programs<SDK: SharedAPI>(invoke_context: &mut InvokeContext<SDK>) {
        // let mut load_program_metrics = LoadProgramMetrics::default();
        let program_runtime_environment = create_program_runtime_environment_v1(
            &invoke_context.feature_set,
            invoke_context.get_compute_budget(),
            false,
            false,
        ).unwrap();
        let program_runtime_environment = Arc::new(program_runtime_environment);
        let num_accounts = invoke_context.transaction_context.get_number_of_accounts();
        for index in 0..num_accounts {
            let account = invoke_context
                .transaction_context
                .get_account_at_index(index)
                .expect("Failed to get the account")
                .borrow();

            let owner = account.owner();
            if check_loader_id(owner) {
                let pubkey = invoke_context
                    .transaction_context
                    .get_key_of_account_at_index(index)
                    .expect("Failed to get account key");

                if let Ok(loaded_program) = load_program_from_bytes(
                    // None,
                    // &mut load_program_metrics,
                    account.data(),
                    owner,
                    account.data().len(),
                    0,
                    program_runtime_environment.clone(),
                    false,
                ) {
                    invoke_context
                        .programs_modified_by_tx
                        .set_slot(DELAY_VISIBILITY_SLOT_OFFSET);
                    invoke_context
                        .programs_modified_by_tx
                        .replenish(*pubkey, Arc::new(loaded_program));
                }
            }
        }
    }
}