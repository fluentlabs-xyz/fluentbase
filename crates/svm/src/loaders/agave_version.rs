use crate::{
    bpf_loader_deprecated,
    compute_budget::compute_budget::MAX_INSTRUCTION_STACK_DEPTH,
    context::{IndexOfAccount, InvokeContext},
    create_vm,
    macros::MEMORY_POOL,
    serialization,
};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use fluentbase_sdk::{debug_log, SharedAPI};
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
    // We dropped the lifetime tracking in the Executor by setting it to 'static,
    // thus we need to reintroduce the correct lifetime of InvokeContext here again.
    // let executable = unsafe {
    //     mem::transmute::<&'a Executable<InvokeContext<'static>>, &'a Executable<InvokeContext<'b>>>(
    //         executable,
    //     )
    // };
    // let log_collector = invoke_context.get_log_collector();
    debug_log!("agave_version.execute1");
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    debug_log!("agave_version.execute2");
    let (_program_id, is_loader_deprecated) = {
        let program_account =
            instruction_context.try_borrow_last_program_account(transaction_context)?;
        (
            *program_account.get_key(),
            *program_account.get_owner() == bpf_loader_deprecated::id(),
        )
    };
    // #[cfg(any(target_os = "windows", not(target_arch = "x86_64")))]
    let use_jit = false;
    // #[cfg(all(not(target_os = "windows"), target_arch = "x86_64"))]
    // let use_jit = executable.get_compiled_program().is_some();
    let direct_mapping = invoke_context
        .get_feature_set()
        .is_active(&bpf_account_data_direct_mapping::id());
    debug_log!("agave_version.execute3: direct_mapping {}", direct_mapping);

    // let mut serialize_time = Measure::start("serialize");
    let (parameter_bytes, regions, accounts_metadata) = serialization::serialize_parameters(
        &invoke_context.transaction_context,
        instruction_context,
        !direct_mapping,
    )?;
    debug_log!(
        "agave_version.execute4: parameter_bytes.len {} regions {:?} accounts_metadata {:?}",
        parameter_bytes.len(),
        regions,
        accounts_metadata
    );
    // serialize_time.stop();

    // save the account addresses so in case we hit an AccessViolation error we
    // can map to a more specific error
    let account_region_addrs = accounts_metadata
        .iter()
        .map(|m| {
            let vm_end = m
                .vm_data_addr
                .saturating_add(m.original_data_len as u64)
                .saturating_add(if !is_loader_deprecated {
                    MAX_PERMITTED_DATA_INCREASE as u64
                } else {
                    0
                });
            m.vm_data_addr..vm_end
        })
        .collect::<Vec<_>>();
    debug_log!(
        "agave_version.execute5: account_region_addrs {:?}",
        account_region_addrs,
    );

    // let mut create_vm_time = Measure::start("create_vm");
    let execution_result = {
        // let compute_meter_prev = invoke_context.get_remaining();
        create_vm!(
            vm,
            executable.as_ref(),
            regions,
            accounts_metadata,
            invoke_context
        );
        let (mut vm, stack, heap) = match vm {
            // let mut vm = match vm {
            Ok(info) => info,
            Err(_e) => {
                // #[cfg(feature = "std")]
                // println!("Failed to create SBF VM: {}", e);
                return Err(Box::new(InstructionError::ProgramEnvironmentSetupFailure));
            }
        };
        // create_vm_time.stop();

        // vm.context_object_pointer.execute_time = Some(Measure::start("execute"));
        debug_log!(
            "agave_version.execute6: ptr_eq {}",
            Arc::ptr_eq(&vm.loader, executable.get_loader()),
        );
        let (_compute_units_consumed, result) = vm.execute_program(executable.as_ref(), !use_jit);
        debug_log!("agave_version.execute7: result {:x?}", result,);
        {
            let mut memory_pool = MEMORY_POOL.write();
            memory_pool.put_stack(stack);
            memory_pool.put_heap(heap);
            debug_assert!(memory_pool.stack_len() <= MAX_INSTRUCTION_STACK_DEPTH);
            debug_assert!(memory_pool.heap_len() <= MAX_INSTRUCTION_STACK_DEPTH);
        }
        // MEMORY_POOL.with_borrow_mut(|memory_pool| {
        //     memory_pool.put_stack(stack);
        //     memory_pool.put_heap(heap);
        //     debug_assert!(memory_pool.stack_len() <= MAX_INSTRUCTION_STACK_DEPTH);
        //     debug_assert!(memory_pool.heap_len() <= MAX_INSTRUCTION_STACK_DEPTH);
        // });
        drop(vm);
        // if let Some(execute_time) = invoke_context.execute_time.as_mut() {
        //     execute_time.stop();
        //     invoke_context.timings.execute_us += execute_time.as_us();
        // }

        // ic_logger_msg!(
        //     log_collector,
        //     "Program {} consumed {} of {} compute units",
        //     &program_id,
        //     compute_units_consumed,
        //     compute_meter_prev
        // );
        // let (_returned_from_program_id, return_data) =
        //     invoke_context.transaction_context.get_return_data();
        // if !return_data.is_empty() {
        //     stable_log::program_return(&log_collector, &program_id, return_data);
        // }
        match result {
            ProgramResult::Ok(status) if status != SUCCESS => {
                let error: InstructionError = status.into();
                Err(Box::new(error) as Box<dyn core::error::Error>)
            }
            ProgramResult::Err(mut error) => {
                debug_log!("agave_version.execute8");
                // if invoke_context
                //     .get_feature_set()
                //     .is_active(&solana_feature_set::deplete_cu_meter_on_vm_failure::id())
                //     && !matches!(error, EbpfError::SyscallError(_))
                // {
                //     // when an exception is thrown during the execution of a
                //     // Basic Block (e.g., a null memory dereference or other
                //     // faults), determining the exact number of CUs consumed
                //     // up to the point of failure requires additional effort
                //     // and is unnecessary since these cases are rare.
                //     //
                //     // In order to simplify CU tracking, simply consume all
                //     // remaining compute units so that the block cost
                //     // tracker uses the full requested compute unit cost for
                //     // this failed transaction.
                //     invoke_context.consume(invoke_context.get_remaining());
                // }

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
                    debug_log!("agave_version.execute9");
                    err
                } else {
                    debug_log!("agave_version.execute10");
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

    // let mut deserialize_time = Measure::start("deserialize");
    let execute_or_deserialize_result = execution_result.and_then(|_| {
        deserialize_parameters(invoke_context, parameter_bytes.as_slice(), !direct_mapping)
            .map_err(|error| Box::new(error) as Box<dyn core::error::Error>)
    });
    // deserialize_time.stop();

    // Update the timings
    // invoke_context.timings.serialize_us += serialize_time.as_us();
    // invoke_context.timings.create_vm_us += create_vm_time.as_us();
    // invoke_context.timings.deserialize_us += deserialize_time.as_us();

    execute_or_deserialize_result
}
