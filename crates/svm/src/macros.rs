use crate::{
    account::WritableAccount,
    context::{BpfAllocator, IndexOfAccount, InvokeContext},
    helpers::{create_memory_mapping, SerializedAccountMetadata, SyscallContext},
    solana_program::runtime::mem_pool::VmMemoryPool,
};
use alloc::{boxed::Box, rc::Rc, vec::Vec};
use fluentbase_sdk::SharedAPI;
use lazy_static::lazy_static;
use solana_account_info::MAX_PERMITTED_DATA_INCREASE;
pub use solana_rbpf::vm::ContextObject;
use solana_rbpf::{elf::Executable, memory_region::MemoryRegion, vm::EbpfVm};
use spin::RwLock;

lazy_static! {
    pub static ref MEMORY_POOL: RwLock<VmMemoryPool> = RwLock::new(VmMemoryPool::new());
}

/// Only used in macro, do not use directly!
pub fn create_vm<'a, 'b, SDK: SharedAPI>(
    program: &'a Executable<InvokeContext<'b, SDK>>,
    regions: Vec<MemoryRegion>,
    accounts_metadata: Vec<SerializedAccountMetadata>,
    invoke_context: &'a mut InvokeContext<'b, SDK>,
    stack: &mut [u8],
    heap: &mut [u8],
) -> Result<EbpfVm<'a, InvokeContext<'b, SDK>>, Box<dyn core::error::Error>> {
    let stack_size = stack.len();
    let heap_size = heap.len();
    let accounts = Rc::clone(invoke_context.transaction_context.accounts());
    let memory_mapping = create_memory_mapping(
        program,
        stack,
        heap,
        regions,
        Some(Box::new(move |index_in_transaction| {
            // The two calls below can't really fail. If they fail because of a bug,
            // whatever is writing will trigger an EbpfError::AccessViolation like
            // if the region was readonly, and the transaction will fail gracefully.
            let mut account = accounts
                .try_borrow_mut(index_in_transaction as IndexOfAccount)
                .map_err(|_| ())?;
            accounts
                .touch(index_in_transaction as IndexOfAccount)
                .map_err(|_| ())?;

            if account.is_shared() {
                // See BorrowedAccount::make_data_mut() as to why we reserve extra
                // MAX_PERMITTED_DATA_INCREASE bytes here.
                account.reserve(MAX_PERMITTED_DATA_INCREASE);
            }
            Ok(account.data_as_mut_slice().as_mut_ptr() as u64)
        })),
    )?;
    invoke_context.set_syscall_context(SyscallContext {
        allocator: BpfAllocator::new(heap_size as u64),
        accounts_metadata,
        trace_log: Vec::new(),
    })?;
    Ok(EbpfVm::new(
        program.get_loader().clone(),
        program.get_sbpf_version(),
        invoke_context,
        memory_mapping,
        stack_size,
    ))
}

#[macro_export]
macro_rules! create_vm {
    ($vm:ident, $program:expr, $regions:expr, $accounts_metadata:expr, $invoke_context:expr $(,)?) => {
        let stack_size = $program.get_config().stack_size();
        let heap_size = $invoke_context.get_compute_budget().heap_size;
        let $vm = {
            let (mut stack, mut heap) = {
                let mut pool = crate::macros::MEMORY_POOL.write();
                (pool.get_stack(stack_size), pool.get_heap(heap_size))
            };
            let vm = $crate::macros::create_vm(
                $program,
                $regions,
                $accounts_metadata,
                $invoke_context,
                stack
                    .as_slice_mut()
                    .get_mut(..stack_size)
                    .expect("invalid stack size"),
                heap.as_slice_mut()
                    .get_mut(..heap_size as usize)
                    .expect("invalid heap size"),
            );
            vm.map(|vm| (vm, stack, heap))
        };
    };
}

/// Generates an adapter for a BuiltinFunction between the Rust and the VM interface
#[macro_export]
macro_rules! declare_builtin_function {
    ($(#[$attr:meta])* $name:ident $(<$($generic_ident:tt : $generic_type:tt),+>)?, fn rust(
        $vm:ident : &mut $ContextObject:ty,
        $arg_a:ident : u64,
        $arg_b:ident : u64,
        $arg_c:ident : u64,
        $arg_d:ident : u64,
        $arg_e:ident : u64,
        $memory_mapping:ident : &mut $MemoryMapping:ty,
    ) -> $Result:ty { $($rust:tt)* }) => {
        $(#[$attr])*
        pub struct $name {}
        impl $name {
            /// Rust interface
            pub fn rust $(<$($generic_ident : $generic_type),+>)? (
                $vm: &mut $ContextObject,
                $arg_a: u64,
                $arg_b: u64,
                $arg_c: u64,
                $arg_d: u64,
                $arg_e: u64,
                $memory_mapping: &mut $MemoryMapping,
            ) -> $Result {
                $($rust)*
            }
            /// VM interface
            #[allow(clippy::too_many_arguments)]
            pub fn vm $(<$($generic_ident : $generic_type),+>)? (
                $vm: *mut solana_rbpf::vm::EbpfVm<$ContextObject>,
                $arg_a: u64,
                $arg_b: u64,
                $arg_c: u64,
                $arg_d: u64,
                $arg_e: u64,
            ) {
                use solana_rbpf::vm::ContextObject;
                let vm = unsafe {
                    &mut *($vm.cast::<u64>().offset(-(solana_rbpf::vm::get_runtime_environment_key() as isize)).cast::<solana_rbpf::vm::EbpfVm<$ContextObject>>())
                };
                let config = vm.loader.get_config();
                if config.enable_instruction_meter {
                    vm.context_object_pointer.consume(vm.previous_instruction_meter - vm.due_insn_count);
                }
                let converted_result: solana_rbpf::error::ProgramResult = Self::rust ::$(<$($generic_ident),+>)?(
                    vm.context_object_pointer, $arg_a, $arg_b, $arg_c, $arg_d, $arg_e, &mut vm.memory_mapping,
                ).map_err(|err| solana_rbpf::error::EbpfError::SyscallError(err)).into();
                vm.program_result = converted_result;
                if config.enable_instruction_meter {
                    vm.previous_instruction_meter = vm.context_object_pointer.get_remaining();
                }
            }
        }
    };
}

/// Adapter so we can unify the interfaces of built-in programs and syscalls
#[macro_export]
macro_rules! declare_process_instruction {
    ($process_instruction:ident <$($generic_ident:ident : $generic_type:tt),+>, $cu_to_consume:expr, |$invoke_context:ident| $inner:tt) => {
        $crate::declare_builtin_function!(
            $process_instruction <$($generic_ident : $generic_type),+>,
            fn rust(
                invoke_context: &mut $crate::context::InvokeContext <$($generic_ident),+>,
                _arg0: u64,
                _arg1: u64,
                _arg2: u64,
                _arg3: u64,
                _arg4: u64,
                _memory_mapping: &mut solana_rbpf::memory_region::MemoryMapping,
            ) -> core::result::Result<u64, Box<dyn core::error::Error>> {
                fn process_instruction_inner <$($generic_ident : $generic_type),+>(
                    $invoke_context: &mut $crate::context::InvokeContext <$($generic_ident),+>,
                ) -> core::result::Result<(), solana_instruction::error::InstructionError>
                    $inner

                process_instruction_inner(invoke_context)
                    .map(|_| 0)
                    .map_err(|err| Box::new(err) as Box<dyn core::error::Error>).into()
            }
        );
    };
}
