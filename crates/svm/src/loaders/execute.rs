use crate::{
    common::MAX_INSTRUCTION_STACK_DEPTH,
    context::InvokeContext,
    create_vm,
    macros::MEMORY_POOL,
    serialization,
};
use alloc::{boxed::Box, sync::Arc};
use fluentbase_sdk::SharedAPI;
use solana_instruction::error::InstructionError;
use solana_program_entrypoint::SUCCESS;
use solana_rbpf::{
    elf::Executable,
    error::{EbpfError, ProgramResult},
};

pub fn execute<'a, SDK: SharedAPI>(
    executable: Arc<Executable<InvokeContext<'a, SDK>>>,
    invoke_context: &mut InvokeContext<'a, SDK>,
) -> Result<(), Box<dyn core::error::Error>> {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;

    let use_jit = false;
    let (parameter_bytes, regions, accounts_metadata) = serialization::serialize_parameters(
        &invoke_context.transaction_context,
        instruction_context,
        true,
    )?;

    let execution_result = {
        create_vm!(
            vm,
            executable.as_ref(),
            regions,
            accounts_metadata,
            invoke_context
        );
        let (mut vm, stack, heap) = match vm {
            Ok(info) => info,
            Err(_e) => {
                return Err(Box::new(InstructionError::ProgramEnvironmentSetupFailure));
            }
        };

        let (_compute_units_consumed, result) = vm.execute_program(executable.as_ref(), !use_jit);
        {
            let mut memory_pool = MEMORY_POOL.write();
            memory_pool.put_stack(stack);
            memory_pool.put_heap(heap);
            debug_assert!(memory_pool.stack_len() <= MAX_INSTRUCTION_STACK_DEPTH);
            debug_assert!(memory_pool.heap_len() <= MAX_INSTRUCTION_STACK_DEPTH);
        }
        drop(vm);

        match result {
            ProgramResult::Ok(status) if status != SUCCESS => {
                let error: InstructionError = status.into();
                Err(Box::new(error) as Box<dyn core::error::Error>)
            }
            ProgramResult::Err(error) => Err(if let EbpfError::SyscallError(err) = error {
                err
            } else {
                error.into()
            }),
            _ => Ok(()),
        }
    };

    fn deserialize_parameters<SDK: SharedAPI>(
        invoke_context: &mut InvokeContext<SDK>,
        parameter_bytes: &[u8],
        copy_account_data: bool,
    ) -> Result<(), InstructionError> {
        serialization::deserialize_parameters(
            &invoke_context.transaction_context,
            invoke_context
                .transaction_context
                .get_current_instruction_context()?,
            copy_account_data,
            parameter_bytes,
            &invoke_context.get_syscall_context()?.accounts_metadata,
        )
    }

    let execute_or_deserialize_result = execution_result.and_then(|_| {
        deserialize_parameters(invoke_context, parameter_bytes.as_slice(), true)
            .map_err(|error| Box::new(error) as Box<dyn core::error::Error>)
    });

    execute_or_deserialize_result
}
