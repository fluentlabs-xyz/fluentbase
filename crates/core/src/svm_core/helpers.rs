use crate::{
    alloc::string::ToString,
    declare_builtin_function,
    svm_core::helpers::SyscallError::UnalignedPointer,
};
// use thiserror::Error as ThisError;
use alloc::vec;
use alloc::{boxed::Box, rc::Rc, str::Utf8Error, string::String, vec::Vec};
use byteorder::LittleEndian;
use core::{
    cell::RefCell,
    fmt::{Display, Formatter},
    slice::from_raw_parts,
    str::from_utf8,
};
use solana_rbpf::{
    aligned_memory::AlignedMemory,
    ebpf,
    ebpf::HOST_ALIGN,
    elf::Executable,
    error::EbpfError,
    memory_region::{AccessType, MemoryCowCallback, MemoryMapping, MemoryRegion},
    vm,
    vm::EbpfVm,
};

type StdResult<T, E> = Result<T, E>;

pub const INSTRUCTION_METER_BUDGET: u64 = 1024 * 1024;

pub struct SyscallContext {
    // pub allocator: BpfAllocator,
    // pub accounts_metadata: Vec<SerializedAccountMetadata>,
    pub trace_log: Vec<[u64; 12]>,
}

pub struct InvokeContext<'a> {
    // pub transaction_context: &'a mut TransactionContext,
    // sysvar_cache: &'a SysvarCache,
    // log_collector: Option<Rc<RefCell<LogCollector>>>,
    // compute_budget: ComputeBudget,
    // current_compute_budget: ComputeBudget,
    compute_meter: RefCell<u64>,
    // pub programs_loaded_for_tx_batch: &'a LoadedProgramsForTxBatch,
    // pub programs_modified_by_tx: &'a mut LoadedProgramsForTxBatch,
    // pub feature_set: Arc<FeatureSet>,
    // pub timings: ExecuteDetailsTimings,
    // pub blockhash: Hash,
    // pub lamports_per_signature: u64,
    pub syscall_context: Vec<Option<SyscallContext>>,
    // traces: Vec<Vec<[u64; 12]>>,
    _marker: PhantomType<&'a ()>,
}

impl InvokeContext<'_> {
    /// Consume compute units
    fn consume_checked(&self, amount: u64) -> Result<(), Box<dyn core::error::Error>> {
        let mut compute_meter = self.compute_meter.borrow_mut();
        let exceeded = *compute_meter < amount;
        *compute_meter = compute_meter.saturating_sub(amount);
        if exceeded {
            return Err(Box::new(InstructionError::ComputationalBudgetExceeded));
        }
        Ok(())
    }
}

impl<'a> ContextObject for InvokeContext<'a> {
    fn trace(&mut self, state: [u64; 12]) {
        self.syscall_context
            .last_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .trace_log
            .push(state);
    }

    fn consume(&mut self, amount: u64) {
        // 1 to 1 instruction to compute unit mapping
        // ignore overflow, Ebpf will bail if exceeded
        // let mut compute_meter = self.compute_meter.borrow_mut();
        // *compute_meter = compute_meter.saturating_sub(amount);
    }

    fn get_remaining(&self) -> u64 {
        *self.compute_meter.borrow()
    }
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

use crate::svm_core::{context::ExecContextObject, error::InstructionError};
use fluentbase_sdk::SharedAPI;
use phantom_type::PhantomType;
// use solana_program::account_info::AccountInfo;
// use solana_program::entrypoint::{BPF_ALIGN_OF_U128, MAX_PERMITTED_DATA_INCREASE,
// NON_DUP_MARKER}; use solana_program::pubkey::Pubkey;
use solana_rbpf::ebpf::MM_INPUT_START;
use solana_rbpf::vm::ContextObject;

/// Log collector
// use std::rc::Rc;

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

type Error = Box<dyn core::error::Error>;

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
    // BadSeeds(PubkeyError),
    // #[error("Program {0} not supported by inner instructions")]
    // ProgramNotSupported(Pubkey),
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
        {
            Ok(unsafe { core::mem::transmute::<u64, &mut T>(host_addr) })
        }
        #[cfg(target_pointer_width = "32")]
        {
            Ok(unsafe { core::mem::transmute::<u32, &mut T>(host_addr as u32) })
        }
    } else if !address_is_aligned::<T>(host_addr) {
        // Err(EbpfError::SyscallError::UnalignedPointer.into())
        Err(Box::new(UnalignedPointer))
    } else {
        Ok(unsafe { &mut *(host_addr as *mut T) })
    }
}
fn translate_type_mut<'a, T>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    check_aligned: bool,
) -> Result<&'a mut T, Box<dyn core::error::Error>> {
    translate_type_inner::<T>(memory_mapping, AccessType::Store, vm_addr, check_aligned)
}
fn translate_type<'a, T>(
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

fn translate_slice<'a, T>(
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

fn translate_slice_mut<'a, T>(
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
fn translate_string_and_do(
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

declare_builtin_function!(
    /// Prints a NULL-terminated UTF-8 string.
    SyscallString<SDK: SharedAPI>,
    fn rust(
        context_object: &mut ExecContextObject<SDK>,
        vm_addr: u64,
        len: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn core::error::Error>> {
        let host_addr: Result<u64, EbpfError> =
            memory_mapping.map(AccessType::Load, vm_addr, len).into();
        let host_addr = host_addr?;
        unsafe {
            let c_buf = from_raw_parts(host_addr as *const u8, len as usize);
            let len = c_buf.iter().position(|c| *c == 0).unwrap_or(len as usize);
            let message = from_utf8(&c_buf[0..len]).unwrap_or("Invalid UTF-8 String");
        }
        Ok(0)
    }
);

declare_builtin_function!(
    /// Log a user's info message
    SyscallLog<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
        addr: u64,
        len: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        // let cost = invoke_context
        //     .get_compute_budget()
        //     .syscall_base_cost
        //     .max(len);
        // consume_compute_meter(invoke_context, cost)?;

        translate_string_and_do(
            memory_mapping,
            addr,
            len,
            // invoke_context.get_check_aligned(),
            true,
            &mut |string: &str| {
                // stable_log::program_log(&invoke_context.get_log_collector(), string);
                #[cfg(all(feature = "std", feature = "debug-print"))]
                println!("Log: {string}");
                Ok(0)
            },
        )?;
        Ok(0)
    }
);

declare_builtin_function!(
    /// Abort syscall functions, called when the SBF program calls `abort()`
    /// LLVM will insert calls to `abort()` if it detects an untenable situation,
    /// `abort()` is not intended to be called explicitly by the program.
    /// Causes the SBF program to be halted immediately
    SyscallAbort<SDK: SharedAPI>,
    fn rust(
        _invoke_context: &mut ExecContextObject<SDK>,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        Err(SyscallError::Abort.into())
    }
);

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

fn memmove<SDK: SharedAPI>(
    invoke_context: &mut ExecContextObject<SDK>,
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

declare_builtin_function!(
    /// memcpy
    SyscallMemcpy<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
        dst_addr: u64,
        src_addr: u64,
        n: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        // mem_op_consume(invoke_context, n)?;

        if !is_nonoverlapping(src_addr, n, dst_addr, n) {
            return Err(SyscallError::CopyOverlapping.into());
        }

        // host addresses can overlap so we always invoke memmove
        memmove(invoke_context, dst_addr, src_addr, n, memory_mapping)
    }
);

declare_builtin_function!(
    SyscallStubInterceptor<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
        addr: u64,
        len: u64,
        arg3: u64,
        arg4: u64,
        arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Error> {
        #[cfg(all(feature = "std", feature = "debug-print"))]
            println!(
                "SyscallStubInterceptor: addr {}; len {}; arg3 {}; arg4 {}; arg5 {};",
                addr, len, arg3, arg4, arg5
            );

        Ok(0)
    }
);

declare_builtin_function!(
    /// Panic syscall function, called when the SBF program calls 'sol_panic_()`
    /// Causes the SBF program to be halted immediately
    SyscallPanic,
    fn rust(
        invoke_context: &mut InvokeContext,
        file: u64,
        len: u64,
        line: u64,
        column: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn core::error::Error>> {
        // consume_compute_meter(invoke_context, len)?;
        //
        // translate_string_and_do(
        //     memory_mapping,
        //     file,
        //     len,
        //     invoke_context.get_check_aligned(),
        //     &mut |string: &str| Err(SyscallError::Panic(string.to_string(), line, column).into()),
        // )
        let error_message = "Dummy panic due to unimplemented syscall"; // Dummy error message
        Err(SyscallError::Panic(error_message.to_string(), line, column).into())
    }
);

pub fn create_memory_mapping<'a, C: ContextObject>(
    executable: &'a Executable<C>,
    stack: &'a mut AlignedMemory<{ HOST_ALIGN }>,
    heap: &'a mut AlignedMemory<{ HOST_ALIGN }>,
    additional_regions: Vec<MemoryRegion>,
    cow_cb: Option<MemoryCowCallback>,
) -> Result<MemoryMapping<'a>, EbpfError> {
    let config = executable.get_config();
    let sbpf_version = executable.get_sbpf_version();

    #[cfg(all(feature = "std", feature = "debug-print"))]
    {
        println!("Creating memory mapping:");
        println!("Stack size: {}", stack.len());
        println!("Heap size: {}", heap.len());
    }

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

    #[cfg(all(feature = "std", feature = "debug-print"))]
    println!("Memory regions created: {:?}", regions);
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

/*pub fn serialize_parameters_aligned(
    accounts: &Vec<AccountInfo>,
    instruction_data: &[u8],
    program_id: &Pubkey,
    // copy_account_data: bool,
) -> Result<Vec<u8>, InstructionError> {
    type BO = LittleEndian;
    let mut s = Vec::<u8>::new();
    // Serialize into the buffer
    s.write_u64::<BO>((accounts.len() as u64).to_le()).map_err(|v| InstructionError::InvalidError)?;
    for account in accounts {
        // match account {
        //     SerializeAccount::Account(_, mut borrowed_account) => {
        s.write_u8(NON_DUP_MARKER).map_err(|v| InstructionError::InvalidError)?;
        s.write_u8(account.is_signer as u8).map_err(|v| InstructionError::InvalidError)?;
        s.write_u8(account.is_writable as u8).map_err(|v| InstructionError::InvalidError)?;
        s.write_u8(account.executable as u8).map_err(|v| InstructionError::InvalidError)?;
        s.extend_from_slice(&[0u8, 0, 0, 0]);
        s.extend_from_slice(account.key.as_ref());
        s.extend_from_slice(account.owner.as_ref());
        s.write_u64::<BO>(account.lamports.borrow().to_le()).map_err(|v| InstructionError::InvalidError)?;
        s.write_u64::<BO>((account.data.borrow().len() as u64).to_le()).map_err(|v| InstructionError::InvalidError)?;
        s.extend_from_slice(account.data.borrow().as_ref());
        s.extend(core::iter::repeat(0).take(MAX_PERMITTED_DATA_INCREASE));
        let align = BPF_ALIGN_OF_U128 - (s.len() - s.len() / BPF_ALIGN_OF_U128 * BPF_ALIGN_OF_U128);
        s.extend(core::iter::repeat(0).take(align));
        s.write_u64::<BO>(account.rent_epoch.to_le()).map_err(|v| InstructionError::InvalidError)?;
    }
    s.write_u64::<BO>((instruction_data.len() as u64).to_le()).map_err(|v| InstructionError::InvalidError)?;
    s.extend_from_slice(instruction_data);
    s.extend_from_slice(program_id.as_ref());

    // let (mem, regions) = s.finish();
    // Ok((mem, regions))
    Ok(s)
}*/
