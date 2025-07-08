use crate::{
    common::MAX_INSTRUCTION_STACK_DEPTH,
    context::{IndexOfAccount, InvokeContext},
    create_vm,
    macros::MEMORY_POOL,
    serialization,
};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use fluentbase_sdk::SharedAPI;
use solana_account_info::MAX_PERMITTED_DATA_INCREASE;
use solana_feature_set::bpf_account_data_direct_mapping;
use solana_instruction::error::InstructionError;
use solana_program_entrypoint::SUCCESS;
use solana_rbpf::{
    elf::Executable,
    error::{EbpfError, ProgramResult},
    memory_region::AccessType,
};

pub fn execute<'a, SDK: SharedAPI>(
    executable: Arc<Executable<InvokeContext<'a, SDK>>>,
    invoke_context: &mut InvokeContext<'a, SDK>,
) -> Result<(), Box<dyn core::error::Error>> {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;

    let use_jit = false;
    let direct_mapping = invoke_context
        .get_feature_set()
        .is_active(&bpf_account_data_direct_mapping::id());

    let (parameter_bytes, regions, accounts_metadata) = serialization::serialize_parameters(
        &invoke_context.transaction_context,
        instruction_context,
        !direct_mapping,
    )?;

    // save the account addresses so in case we hit an AccessViolation error we
    // can map to a more specific error
    let account_region_addrs = accounts_metadata
        .iter()
        .map(|m| {
            let vm_end = m
                .vm_data_addr
                .saturating_add(m.original_data_len as u64)
                .saturating_add(MAX_PERMITTED_DATA_INCREASE as u64);
            m.vm_data_addr..vm_end
        })
        .collect::<Vec<_>>();

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
            ProgramResult::Err(mut error) => {
                if direct_mapping {
                    if let EbpfError::AccessViolation(
                        AccessType::Store,
                        address,
                        _size,
                        _section_name,
                    ) = error
                    {
                        // If direct_mapping is enabled and a program tries to write to a readonly
                        // region we'll get a memory access violation. Map it to a more specific
                        // error so it's easier for developers to see what happened.
                        if let Some((instruction_account_index, _)) = account_region_addrs
                            .iter()
                            .enumerate()
                            .find(|(_, vm_region)| vm_region.contains(&address))
                        {
                            let transaction_context = &invoke_context.transaction_context;
                            let instruction_context =
                                transaction_context.get_current_instruction_context()?;

                            let account = instruction_context.try_borrow_instruction_account(
                                transaction_context,
                                instruction_account_index as IndexOfAccount,
                            )?;

                            error = EbpfError::SyscallError(Box::new(if account.is_executable() {
                                InstructionError::ExecutableDataModified
                            } else if account.is_writable() {
                                InstructionError::ExternalAccountDataModified
                            } else {
                                InstructionError::ReadonlyDataModified
                            }));
                        }
                    }
                }
                Err(if let EbpfError::SyscallError(err) = error {
                    err
                } else {
                    error.into()
                })
            }
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
        deserialize_parameters(invoke_context, parameter_bytes.as_slice(), !direct_mapping)
            .map_err(|error| Box::new(error) as Box<dyn core::error::Error>)
    });

    execute_or_deserialize_result
}
