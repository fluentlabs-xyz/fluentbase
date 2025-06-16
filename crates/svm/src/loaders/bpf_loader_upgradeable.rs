use crate::{
    bpf_loader,
    bpf_loader_deprecated,
    common::{
        common_close_account,
        write_program_data,
        DEFAULT_LOADER_COMPUTE_UNITS,
        DEPRECATED_LOADER_COMPUTE_UNITS,
        PACKET_DATA_SIZE,
    },
    context::{IndexOfAccount, InvokeContext},
    declare_builtin_function,
    deploy_program,
    // loaded_programs::{LoadedProgram, LoadedProgramType},
    native_loader,
    solana_program::{
        bpf_loader_upgradeable,
        bpf_loader_upgradeable::UpgradeableLoaderState,
        instruction::AccountMeta,
        loader_upgradeable_instruction::UpgradeableLoaderInstruction,
    },
    system_instruction,
    system_instruction::MAX_PERMITTED_DATA_LENGTH,
    sysvar_cache::get_sysvar_with_account_check,
};
use crate::{
    common::UPGRADEABLE_LOADER_COMPUTE_UNITS,
    loaded_programs::{ProgramCacheEntry, ProgramCacheEntryOwner, ProgramCacheEntryType},
    loaders::execute::execute,
};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use fluentbase_sdk::SharedAPI;
use solana_bincode::limited_deserialize;
use solana_feature_set::{
    enable_bpf_loader_extend_program_ix,
    enable_bpf_loader_set_authority_checked_ix,
};
use solana_instruction::error::InstructionError;
use solana_pubkey::{Pubkey, PubkeyError};
use solana_rbpf::memory_region::MemoryMapping;

declare_builtin_function!(
    Entrypoint<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut InvokeContext<SDK>,
        _arg0: u64,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn core::error::Error>> {
        process_instruction_inner(invoke_context)
    }
);

/*pub(crate) fn execute<'a, SDK: SharedAPI>(
    executable: Arc<Executable<InvokeContext<'a, SDK>>>,
    invoke_context: &mut InvokeContext<'a, SDK>,
) -> Result<(), Box<dyn core::error::Error>> {
    // We dropped the lifetime tracking in the Executor by setting it to 'static,
    // thus we need to reintroduce the correct lifetime of InvokeContext here again.
    // let executable = unsafe { mem::transmute::<_, &Executable<InvokeContext<SDK>>>(executable) };
    // let log_collector = invoke_context.get_log_collector();
    // let invoke_context_ref = invoke_context.borrow();
    let instruction_context = invoke_context
        .transaction_context
        .get_current_instruction_context()?;
    let (_program_id, is_loader_deprecated) = {
        let program_account = instruction_context
            .try_borrow_last_program_account(&invoke_context.transaction_context)?;
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
        .environment_config
        .feature_set
        .is_active(&bpf_account_data_direct_mapping::id());

    // let mut serialize_time = Measure::start("serialize");
    let (parameter_bytes, regions, accounts_metadata) = serialization::serialize_parameters(
        &invoke_context.transaction_context,
        instruction_context,
        !direct_mapping,
    )?;
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

    // let mut create_vm_time = Measure::start("create_vm");
    // let mut execute_time;
    let execution_result = {
        // let compute_meter_prev = invoke_context.get_remaining();
        // let mut invoke_context_ref_mut = invoke_context.borrow_mut();
        create_vm!(
            vm,
            executable.as_ref(),
            regions,
            accounts_metadata,
            invoke_context
        );
        let (mut vm, stack, heap) = match vm {
            Ok(info) => info,
            Err(e) => {
                // ic_logger_msg!(log_collector, "Failed to create SBF VM: {}", e);
                return Err(Box::new(InstructionError::ProgramEnvironmentSetupFailure));
            }
        };
        // create_vm_time.stop();

        // execute_time = Measure::start("execute");
        let (_compute_units_consumed, result) = vm.execute_program(executable.as_ref(), !use_jit);
        drop(vm);
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
                            let instruction_context = invoke_context
                                .transaction_context
                                .get_current_instruction_context()?;

                            let account = instruction_context.try_borrow_instruction_account(
                                &invoke_context.transaction_context,
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
    // execute_time.stop();

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
    // let timings = &mut invoke_context.timings;
    // timings.serialize_us = timings.serialize_us.saturating_add(serialize_time.as_us());
    // timings.create_vm_us = timings.create_vm_us.saturating_add(create_vm_time.as_us());
    // timings.execute_us = timings.execute_us.saturating_add(execute_time.as_us());
    // timings.deserialize_us = timings
    //     .deserialize_us
    //     .saturating_add(deserialize_time.as_us());

    execute_or_deserialize_result
}*/

pub fn process_instruction_inner<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<u64, Box<dyn core::error::Error>> {
    // let log_collector = invoke_context.get_log_collector();
    // let invoke_context_ref = invoke_context.borrow();
    let instruction_context = invoke_context
        .transaction_context
        .get_current_instruction_context()?;
    let program_account =
        instruction_context.try_borrow_last_program_account(&invoke_context.transaction_context)?;

    let program_account_key = program_account.get_key();

    // Program Management Instruction
    if native_loader::check_id(program_account.get_owner()) {
        drop(program_account);
        let program_id =
            instruction_context.get_last_program_key(&invoke_context.transaction_context)?;
        return if bpf_loader_upgradeable::check_id(program_id) {
            invoke_context.consume_checked(UPGRADEABLE_LOADER_COMPUTE_UNITS)?;
            process_loader_upgradeable_instruction(invoke_context)
        } else if bpf_loader::check_id(program_id) {
            invoke_context.consume_checked(DEFAULT_LOADER_COMPUTE_UNITS)?;
            // ic_logger_msg!(
            //     log_collector,
            //     "BPF loader management instructions are no longer supported",
            // );

            Err(InstructionError::UnsupportedProgramId)
        } else if bpf_loader_deprecated::check_id(program_id) {
            invoke_context.consume_checked(DEPRECATED_LOADER_COMPUTE_UNITS)?;
            // ic_logger_msg!(log_collector, "Deprecated loader is no longer supported");

            Err(InstructionError::UnsupportedProgramId)
        } else {
            // ic_logger_msg!(log_collector, "Invalid BPF loader id");
            Err(InstructionError::IncorrectProgramId)
        }
        .map(|_| 0)
        .map_err(|error| Box::new(error) as Box<dyn core::error::Error>);
    }

    // Program Invocation
    if !program_account.is_executable() {
        // ic_logger_msg!(log_collector, "Program is not executable");
        return Err(Box::new(InstructionError::IncorrectProgramId));
    }

    // let mut get_or_create_executor_time = Measure::start("get_or_create_executor_time");
    let executor = invoke_context
        .program_cache_for_tx_batch
        .find(program_account_key)
        .ok_or_else(|| {
            // ic_logger_msg!(log_collector, "Program is not cached");
            InstructionError::InvalidAccountData
        })?;
    drop(program_account);
    // get_or_create_executor_time.stop();
    // saturating_add_assign!(
    //     invoke_context.timings.get_or_create_executor_us,
    //     get_or_create_executor_time.as_us()
    // );

    // executor.ix_usage_counter.fetch_add(1, Ordering::Relaxed);
    // let executor_program = &executor.program;
    // let executor_program_ref = executor_program.as_ref();
    let result: Result<(), Box<dyn core::error::Error>> = match &executor.program {
        ProgramCacheEntryType::FailedVerification(_)
        | ProgramCacheEntryType::Closed
        | ProgramCacheEntryType::DelayVisibility => {
            // ic_logger_msg!(log_collector, "Program is not deployed");
            Err(Box::new(InstructionError::InvalidAccountData) as Box<dyn core::error::Error>)
        }
        ProgramCacheEntryType::Loaded(executable) => execute(executable.clone(), invoke_context),
        _ => Err(Box::new(InstructionError::IncorrectProgramId) as Box<dyn core::error::Error>),
    };

    result.map(|_| 0)
}

fn process_loader_upgradeable_instruction<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    // let log_collector = invoke_context.get_log_collector();
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let instruction_data = instruction_context.get_instruction_data();
    let program_id = instruction_context.get_last_program_key(transaction_context)?;

    match limited_deserialize::<PACKET_DATA_SIZE, _>(instruction_data)
        .map_err(|_| InstructionError::InvalidInstructionData)?
    {
        UpgradeableLoaderInstruction::InitializeBuffer => {
            instruction_context.check_number_of_instruction_accounts(2)?;
            let mut buffer =
                instruction_context.try_borrow_instruction_account(transaction_context, 0)?;

            let buffer_state = buffer.get_state();
            if UpgradeableLoaderState::Uninitialized != buffer_state? {
                // ic_logger_msg!(log_collector, "Buffer account already initialized");
                return Err(InstructionError::AccountAlreadyInitialized);
            }

            let authority_key = Some(*transaction_context.get_key_of_account_at_index(
                instruction_context.get_index_of_instruction_account_in_transaction(1)?,
            )?);

            buffer.set_state(&UpgradeableLoaderState::Buffer {
                authority_address: authority_key,
            })?;
        }
        UpgradeableLoaderInstruction::Write { offset, bytes } => {
            instruction_context.check_number_of_instruction_accounts(2)?;
            let buffer =
                instruction_context.try_borrow_instruction_account(transaction_context, 0)?;

            if let UpgradeableLoaderState::Buffer { authority_address } = buffer.get_state()? {
                if authority_address.is_none() {
                    // ic_logger_msg!(log_collector, "Buffer is immutable");
                    return Err(InstructionError::Immutable); // TODO better error code
                }
                let authority_key = Some(*transaction_context.get_key_of_account_at_index(
                    instruction_context.get_index_of_instruction_account_in_transaction(1)?,
                )?);
                if authority_address != authority_key {
                    // ic_logger_msg!(log_collector, "Incorrect buffer authority provided");
                    return Err(InstructionError::IncorrectAuthority);
                }
                if !instruction_context.is_instruction_account_signer(1)? {
                    // ic_logger_msg!(log_collector, "Buffer authority did not sign");
                    return Err(InstructionError::MissingRequiredSignature);
                }
            } else {
                // ic_logger_msg!(log_collector, "Invalid Buffer account");
                return Err(InstructionError::InvalidAccountData);
            }
            drop(buffer);
            write_program_data(
                UpgradeableLoaderState::size_of_buffer_metadata().saturating_add(offset as usize),
                &bytes,
                invoke_context,
            )?;
        }
        UpgradeableLoaderInstruction::DeployWithMaxDataLen { max_data_len } => {
            instruction_context.check_number_of_instruction_accounts(4)?;
            let payer_key = *transaction_context.get_key_of_account_at_index(
                instruction_context.get_index_of_instruction_account_in_transaction(0)?,
            )?;
            let programdata_key = *transaction_context.get_key_of_account_at_index(
                instruction_context.get_index_of_instruction_account_in_transaction(1)?,
            )?;
            let rent = get_sysvar_with_account_check::rent(invoke_context, instruction_context, 4)?;
            let clock =
                get_sysvar_with_account_check::clock(invoke_context, instruction_context, 5)?;
            instruction_context.check_number_of_instruction_accounts(8)?;
            let authority_key = Some(*transaction_context.get_key_of_account_at_index(
                instruction_context.get_index_of_instruction_account_in_transaction(7)?,
            )?);

            // Verify Program account

            let program =
                instruction_context.try_borrow_instruction_account(transaction_context, 2)?;
            if UpgradeableLoaderState::Uninitialized != program.get_state()? {
                // ic_logger_msg!(log_collector, "Program account already initialized");
                return Err(InstructionError::AccountAlreadyInitialized);
            }
            if program.get_data().len() < UpgradeableLoaderState::size_of_program() {
                // ic_logger_msg!(log_collector, "Program account too small");
                return Err(InstructionError::AccountDataTooSmall);
            }
            if program.get_lamports() < rent.minimum_balance(program.get_data().len()) {
                // ic_logger_msg!(log_collector, "Program account not rent-exempt");
                return Err(InstructionError::ExecutableAccountNotRentExempt);
            }
            let new_program_id = *program.get_key();
            drop(program);

            // Verify Buffer account

            let buffer =
                instruction_context.try_borrow_instruction_account(transaction_context, 3)?;
            if let UpgradeableLoaderState::Buffer { authority_address } = buffer.get_state()? {
                if authority_address != authority_key {
                    // ic_logger_msg!(log_collector, "Buffer and upgrade authority don't match");
                    return Err(InstructionError::IncorrectAuthority);
                }
                if !instruction_context.is_instruction_account_signer(7)? {
                    // ic_logger_msg!(log_collector, "Upgrade authority did not sign");
                    return Err(InstructionError::MissingRequiredSignature);
                }
            } else {
                // ic_logger_msg!(log_collector, "Invalid Buffer account");
                return Err(InstructionError::InvalidArgument);
            }
            let buffer_key = *buffer.get_key();
            let buffer_data_offset = UpgradeableLoaderState::size_of_buffer_metadata();
            let buffer_data_len = buffer.get_data().len().saturating_sub(buffer_data_offset);
            let programdata_data_offset = UpgradeableLoaderState::size_of_programdata_metadata();
            let programdata_len = UpgradeableLoaderState::size_of_programdata(max_data_len);
            if buffer.get_data().len() < UpgradeableLoaderState::size_of_buffer_metadata()
                || buffer_data_len == 0
            {
                // ic_logger_msg!(log_collector, "Buffer account too small");
                return Err(InstructionError::InvalidAccountData);
            }
            drop(buffer);
            if max_data_len < buffer_data_len {
                // ic_logger_msg!(
                //     log_collector,
                //     "Max data length is too small to hold Buffer data"
                // );
                return Err(InstructionError::AccountDataTooSmall);
            }
            if programdata_len > MAX_PERMITTED_DATA_LENGTH as usize {
                // ic_logger_msg!(log_collector, "Max data length is too large");
                return Err(InstructionError::InvalidArgument);
            }

            // Create ProgramData account
            let (derived_address, bump_seed) =
                Pubkey::find_program_address(&[new_program_id.as_ref()], program_id);
            if derived_address != programdata_key {
                // ic_logger_msg!(log_collector, "ProgramData address is not derived");
                return Err(InstructionError::InvalidArgument);
            }

            // Drain the Buffer account to payer before paying for programdata account
            {
                let mut buffer =
                    instruction_context.try_borrow_instruction_account(transaction_context, 3)?;
                let mut payer =
                    instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
                payer.checked_add_lamports(buffer.get_lamports())?;
                buffer.set_lamports(0)?;
            }

            let owner_id = *program_id;
            let mut instruction = system_instruction::create_account(
                &payer_key,
                &programdata_key,
                1.max(rent.minimum_balance(programdata_len)),
                programdata_len as u64,
                program_id,
            );

            // pass an extra account to avoid the overly strict UnbalancedInstruction error
            instruction
                .accounts
                .push(AccountMeta::new(buffer_key, false));

            let transaction_context = &invoke_context.transaction_context;
            let instruction_context = transaction_context.get_current_instruction_context()?;
            let caller_program_id =
                instruction_context.get_last_program_key(transaction_context)?;
            let signers = [[new_program_id.as_ref(), &[bump_seed]]]
                .iter()
                .map(|seeds| Pubkey::create_program_address(seeds, caller_program_id))
                .collect::<Result<Vec<Pubkey>, PubkeyError>>()?;
            invoke_context.native_invoke(instruction.into(), signers.as_slice())?;

            // Load and verify the program bits
            let transaction_context = &invoke_context.transaction_context;
            let instruction_context = transaction_context.get_current_instruction_context()?;
            let buffer =
                instruction_context.try_borrow_instruction_account(transaction_context, 3)?;
            deploy_program!(
                invoke_context,
                new_program_id,
                &owner_id,
                UpgradeableLoaderState::size_of_program().saturating_add(programdata_len),
                clock.slot,
                {
                    drop(buffer);
                },
                buffer
                    .get_data()
                    .get(buffer_data_offset..)
                    .ok_or(InstructionError::AccountDataTooSmall)?,
            );

            let transaction_context = &invoke_context.transaction_context;
            let instruction_context = transaction_context.get_current_instruction_context()?;

            // Update the ProgramData account and record the program bits
            {
                let mut programdata =
                    instruction_context.try_borrow_instruction_account(transaction_context, 1)?;
                programdata.set_state(&UpgradeableLoaderState::ProgramData {
                    slot: clock.slot,
                    upgrade_authority_address: authority_key,
                })?;
                let dst_slice = programdata
                    .get_data_mut()?
                    .get_mut(
                        programdata_data_offset
                            ..programdata_data_offset.saturating_add(buffer_data_len),
                    )
                    .ok_or(InstructionError::AccountDataTooSmall)?;
                let mut buffer =
                    instruction_context.try_borrow_instruction_account(transaction_context, 3)?;
                let src_slice = buffer
                    .get_data()
                    .get(buffer_data_offset..)
                    .ok_or(InstructionError::AccountDataTooSmall)?;
                dst_slice.copy_from_slice(src_slice);
                buffer.set_data_length(UpgradeableLoaderState::size_of_buffer(0))?;
            }

            // Update the Program account
            let mut program =
                instruction_context.try_borrow_instruction_account(transaction_context, 2)?;
            program.set_state(&UpgradeableLoaderState::Program {
                programdata_address: programdata_key,
            })?;
            program.set_executable(true)?;
            drop(program);

            // ic_logger_msg!(log_collector, "Deployed program {:?}", new_program_id);
        }
        UpgradeableLoaderInstruction::Upgrade => {
            instruction_context.check_number_of_instruction_accounts(3)?;
            let programdata_key = *transaction_context.get_key_of_account_at_index(
                instruction_context.get_index_of_instruction_account_in_transaction(0)?,
            )?;
            let rent = get_sysvar_with_account_check::rent(invoke_context, instruction_context, 4)?;
            let clock =
                get_sysvar_with_account_check::clock(invoke_context, instruction_context, 5)?;
            instruction_context.check_number_of_instruction_accounts(7)?;
            let authority_key = Some(*transaction_context.get_key_of_account_at_index(
                instruction_context.get_index_of_instruction_account_in_transaction(6)?,
            )?);

            // Verify Program account

            let program =
                instruction_context.try_borrow_instruction_account(transaction_context, 1)?;
            if !program.is_executable() {
                // ic_logger_msg!(log_collector, "Program account not executable");
                return Err(InstructionError::AccountNotExecutable);
            }
            if !program.is_writable() {
                // ic_logger_msg!(log_collector, "Program account not writeable");
                return Err(InstructionError::InvalidArgument);
            }
            if program.get_owner() != program_id {
                // ic_logger_msg!(log_collector, "Program account not owned by loader");
                return Err(InstructionError::IncorrectProgramId);
            }
            if let UpgradeableLoaderState::Program {
                programdata_address,
            } = program.get_state()?
            {
                if programdata_address != programdata_key {
                    // ic_logger_msg!(log_collector, "Program and ProgramData account mismatch");
                    return Err(InstructionError::InvalidArgument);
                }
            } else {
                // ic_logger_msg!(log_collector, "Invalid Program account");
                return Err(InstructionError::InvalidAccountData);
            }
            let new_program_id = *program.get_key();
            drop(program);

            // Verify Buffer account

            let buffer =
                instruction_context.try_borrow_instruction_account(transaction_context, 2)?;
            if let UpgradeableLoaderState::Buffer { authority_address } = buffer.get_state()? {
                if authority_address != authority_key {
                    // ic_logger_msg!(log_collector, "Buffer and upgrade authority don't match");
                    return Err(InstructionError::IncorrectAuthority);
                }
                if !instruction_context.is_instruction_account_signer(6)? {
                    // ic_logger_msg!(log_collector, "Upgrade authority did not sign");
                    return Err(InstructionError::MissingRequiredSignature);
                }
            } else {
                // ic_logger_msg!(log_collector, "Invalid Buffer account");
                return Err(InstructionError::InvalidArgument);
            }
            let buffer_lamports = buffer.get_lamports();
            let buffer_data_offset = UpgradeableLoaderState::size_of_buffer_metadata();
            let buffer_data_len = buffer.get_data().len().saturating_sub(buffer_data_offset);
            if buffer.get_data().len() < UpgradeableLoaderState::size_of_buffer_metadata()
                || buffer_data_len == 0
            {
                // ic_logger_msg!(log_collector, "Buffer account too small");
                return Err(InstructionError::InvalidAccountData);
            }
            drop(buffer);

            // Verify ProgramData account

            let programdata =
                instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
            let programdata_data_offset = UpgradeableLoaderState::size_of_programdata_metadata();
            let programdata_balance_required =
                1.max(rent.minimum_balance(programdata.get_data().len()));
            if programdata.get_data().len()
                < UpgradeableLoaderState::size_of_programdata(buffer_data_len)
            {
                // ic_logger_msg!(log_collector, "ProgramData account not large enough");
                return Err(InstructionError::AccountDataTooSmall);
            }
            if programdata.get_lamports().saturating_add(buffer_lamports)
                < programdata_balance_required
            {
                // ic_logger_msg!(
                //     log_collector,
                //     "Buffer account balance too low to fund upgrade"
                // );
                return Err(InstructionError::InsufficientFunds);
            }
            if let UpgradeableLoaderState::ProgramData {
                slot,
                upgrade_authority_address,
            } = programdata.get_state()?
            {
                if clock.slot == slot {
                    // ic_logger_msg!(log_collector, "Program was deployed in this block already");
                    return Err(InstructionError::InvalidArgument);
                }
                if upgrade_authority_address.is_none() {
                    // ic_logger_msg!(log_collector, "Program not upgradeable");
                    return Err(InstructionError::Immutable);
                }
                if upgrade_authority_address != authority_key {
                    // ic_logger_msg!(log_collector, "Incorrect upgrade authority provided");
                    return Err(InstructionError::IncorrectAuthority);
                }
                if !instruction_context.is_instruction_account_signer(6)? {
                    // ic_logger_msg!(log_collector, "Upgrade authority did not sign");
                    return Err(InstructionError::MissingRequiredSignature);
                }
            } else {
                // ic_logger_msg!(log_collector, "Invalid ProgramData account");
                return Err(InstructionError::InvalidAccountData);
            };
            let programdata_len = programdata.get_data().len();
            drop(programdata);

            // Load and verify the program bits
            let buffer =
                instruction_context.try_borrow_instruction_account(transaction_context, 2)?;
            deploy_program!(
                invoke_context,
                new_program_id,
                program_id,
                UpgradeableLoaderState::size_of_program().saturating_add(programdata_len),
                clock.slot,
                {
                    drop(buffer);
                },
                buffer
                    .get_data()
                    .get(buffer_data_offset..)
                    .ok_or(InstructionError::AccountDataTooSmall)?,
            );

            let transaction_context = &invoke_context.transaction_context;
            let instruction_context = transaction_context.get_current_instruction_context()?;

            // Update the ProgramData account, record the upgraded data, and zero
            // the rest
            let mut programdata =
                instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
            {
                programdata.set_state(&UpgradeableLoaderState::ProgramData {
                    slot: clock.slot,
                    upgrade_authority_address: authority_key,
                })?;
                let dst_slice = programdata
                    .get_data_mut()?
                    .get_mut(
                        programdata_data_offset
                            ..programdata_data_offset.saturating_add(buffer_data_len),
                    )
                    .ok_or(InstructionError::AccountDataTooSmall)?;
                let buffer =
                    instruction_context.try_borrow_instruction_account(transaction_context, 2)?;
                let src_slice = buffer
                    .get_data()
                    .get(buffer_data_offset..)
                    .ok_or(InstructionError::AccountDataTooSmall)?;
                dst_slice.copy_from_slice(src_slice);
            }
            programdata
                .get_data_mut()?
                .get_mut(programdata_data_offset.saturating_add(buffer_data_len)..)
                .ok_or(InstructionError::AccountDataTooSmall)?
                .fill(0);

            // Fund ProgramData to rent-exemption, spill the rest
            let mut buffer =
                instruction_context.try_borrow_instruction_account(transaction_context, 2)?;
            let mut spill =
                instruction_context.try_borrow_instruction_account(transaction_context, 3)?;
            spill.checked_add_lamports(
                programdata
                    .get_lamports()
                    .saturating_add(buffer_lamports)
                    .saturating_sub(programdata_balance_required),
            )?;
            buffer.set_lamports(0)?;
            programdata.set_lamports(programdata_balance_required)?;
            buffer.set_data_length(UpgradeableLoaderState::size_of_buffer(0))?;

            // ic_logger_msg!(log_collector, "Upgraded program {:?}", new_program_id);
        }
        UpgradeableLoaderInstruction::SetAuthority => {
            instruction_context.check_number_of_instruction_accounts(2)?;
            let mut account =
                instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
            let present_authority_key = transaction_context.get_key_of_account_at_index(
                instruction_context.get_index_of_instruction_account_in_transaction(1)?,
            )?;
            let new_authority = instruction_context
                .get_index_of_instruction_account_in_transaction(2)
                .and_then(|index_in_transaction| {
                    transaction_context.get_key_of_account_at_index(index_in_transaction)
                })
                .ok();

            match account.get_state()? {
                UpgradeableLoaderState::Buffer { authority_address } => {
                    if new_authority.is_none() {
                        // ic_logger_msg!(log_collector, "Buffer authority is not optional");
                        return Err(InstructionError::IncorrectAuthority);
                    }
                    if authority_address.is_none() {
                        // ic_logger_msg!(log_collector, "Buffer is immutable");
                        return Err(InstructionError::Immutable);
                    }
                    if authority_address != Some(*present_authority_key) {
                        // ic_logger_msg!(log_collector, "Incorrect buffer authority provided");
                        return Err(InstructionError::IncorrectAuthority);
                    }
                    if !instruction_context.is_instruction_account_signer(1)? {
                        // ic_logger_msg!(log_collector, "Buffer authority did not sign");
                        return Err(InstructionError::MissingRequiredSignature);
                    }
                    account.set_state(&UpgradeableLoaderState::Buffer {
                        authority_address: new_authority.cloned(),
                    })?;
                }
                UpgradeableLoaderState::ProgramData {
                    slot,
                    upgrade_authority_address,
                } => {
                    if upgrade_authority_address.is_none() {
                        // ic_logger_msg!(log_collector, "Program not upgradeable");
                        return Err(InstructionError::Immutable);
                    }
                    if upgrade_authority_address != Some(*present_authority_key) {
                        // ic_logger_msg!(log_collector, "Incorrect upgrade authority provided");
                        return Err(InstructionError::IncorrectAuthority);
                    }
                    if !instruction_context.is_instruction_account_signer(1)? {
                        // ic_logger_msg!(log_collector, "Upgrade authority did not sign");
                        return Err(InstructionError::MissingRequiredSignature);
                    }
                    account.set_state(&UpgradeableLoaderState::ProgramData {
                        slot,
                        upgrade_authority_address: new_authority.cloned(),
                    })?;
                }
                _ => {
                    // ic_logger_msg!(log_collector, "Account does not support authorities");
                    return Err(InstructionError::InvalidArgument);
                }
            }

            // ic_logger_msg!(log_collector, "New authority {:?}", new_authority);
        }
        UpgradeableLoaderInstruction::SetAuthorityChecked => {
            if !invoke_context
                .environment_config
                .feature_set
                .is_active(&enable_bpf_loader_set_authority_checked_ix::id())
            {
                return Err(InstructionError::InvalidInstructionData);
            }

            instruction_context.check_number_of_instruction_accounts(3)?;
            let mut account =
                instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
            let present_authority_key = transaction_context.get_key_of_account_at_index(
                instruction_context.get_index_of_instruction_account_in_transaction(1)?,
            )?;
            let new_authority_key = transaction_context.get_key_of_account_at_index(
                instruction_context.get_index_of_instruction_account_in_transaction(2)?,
            )?;

            match account.get_state()? {
                UpgradeableLoaderState::Buffer { authority_address } => {
                    if authority_address.is_none() {
                        // ic_logger_msg!(log_collector, "Buffer is immutable");
                        return Err(InstructionError::Immutable);
                    }
                    if authority_address != Some(*present_authority_key) {
                        // ic_logger_msg!(log_collector, "Incorrect buffer authority provided");
                        return Err(InstructionError::IncorrectAuthority);
                    }
                    if !instruction_context.is_instruction_account_signer(1)? {
                        // ic_logger_msg!(log_collector, "Buffer authority did not sign");
                        return Err(InstructionError::MissingRequiredSignature);
                    }
                    if !instruction_context.is_instruction_account_signer(2)? {
                        // ic_logger_msg!(log_collector, "New authority did not sign");
                        return Err(InstructionError::MissingRequiredSignature);
                    }
                    account.set_state(&UpgradeableLoaderState::Buffer {
                        authority_address: Some(*new_authority_key),
                    })?;
                }
                UpgradeableLoaderState::ProgramData {
                    slot,
                    upgrade_authority_address,
                } => {
                    if upgrade_authority_address.is_none() {
                        // ic_logger_msg!(log_collector, "Program not upgradeable");
                        return Err(InstructionError::Immutable);
                    }
                    if upgrade_authority_address != Some(*present_authority_key) {
                        // ic_logger_msg!(log_collector, "Incorrect upgrade authority provided");
                        return Err(InstructionError::IncorrectAuthority);
                    }
                    if !instruction_context.is_instruction_account_signer(1)? {
                        // ic_logger_msg!(log_collector, "Upgrade authority did not sign");
                        return Err(InstructionError::MissingRequiredSignature);
                    }
                    if !instruction_context.is_instruction_account_signer(2)? {
                        // ic_logger_msg!(log_collector, "New authority did not sign");
                        return Err(InstructionError::MissingRequiredSignature);
                    }
                    account.set_state(&UpgradeableLoaderState::ProgramData {
                        slot,
                        upgrade_authority_address: Some(*new_authority_key),
                    })?;
                }
                _ => {
                    // ic_logger_msg!(log_collector, "Account does not support authorities");
                    return Err(InstructionError::InvalidArgument);
                }
            }

            // ic_logger_msg!(log_collector, "New authority {:?}", new_authority_key);
        }
        UpgradeableLoaderInstruction::Close => {
            instruction_context.check_number_of_instruction_accounts(2)?;
            if instruction_context.get_index_of_instruction_account_in_transaction(0)?
                == instruction_context.get_index_of_instruction_account_in_transaction(1)?
            {
                // ic_logger_msg!(
                //     log_collector,
                //     "Recipient is the same as the account being closed"
                // );
                return Err(InstructionError::InvalidArgument);
            }
            let mut close_account =
                instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
            let close_key = *close_account.get_key();
            let close_account_state = close_account.get_state()?;
            close_account.set_data_length(UpgradeableLoaderState::size_of_uninitialized())?;
            match close_account_state {
                UpgradeableLoaderState::Uninitialized => {
                    let mut recipient_account = instruction_context
                        .try_borrow_instruction_account(transaction_context, 1)?;
                    recipient_account.checked_add_lamports(close_account.get_lamports())?;
                    close_account.set_lamports(0)?;

                    // ic_logger_msg!(log_collector, "Closed Uninitialized {}", close_key);
                }
                UpgradeableLoaderState::Buffer { authority_address } => {
                    instruction_context.check_number_of_instruction_accounts(3)?;
                    drop(close_account);
                    common_close_account(
                        &authority_address,
                        transaction_context,
                        instruction_context,
                        // &log_collector,
                    )?;

                    // ic_logger_msg!(log_collector, "Closed Buffer {}", close_key);
                }
                UpgradeableLoaderState::ProgramData {
                    slot,
                    upgrade_authority_address: authority_address,
                } => {
                    instruction_context.check_number_of_instruction_accounts(4)?;
                    drop(close_account);
                    let program_account = instruction_context
                        .try_borrow_instruction_account(transaction_context, 3)?;
                    let program_key = *program_account.get_key();

                    if !program_account.is_writable() {
                        // ic_logger_msg!(log_collector, "Program account is not writable");
                        return Err(InstructionError::InvalidArgument);
                    }
                    if program_account.get_owner() != program_id {
                        // ic_logger_msg!(log_collector, "Program account not owned by loader");
                        return Err(InstructionError::IncorrectProgramId);
                    }
                    let clock = invoke_context.get_sysvar_cache().get_clock()?;
                    if clock.slot == slot {
                        // ic_logger_msg!(log_collector, "Program was deployed in this block already");
                        return Err(InstructionError::InvalidArgument);
                    }

                    match program_account.get_state()? {
                        UpgradeableLoaderState::Program {
                            programdata_address,
                        } => {
                            if programdata_address != close_key {
                                // ic_logger_msg!(
                                //     log_collector,
                                //     "ProgramData account does not match ProgramData account"
                                // );
                                return Err(InstructionError::InvalidArgument);
                            }

                            drop(program_account);
                            common_close_account(
                                &authority_address,
                                transaction_context,
                                instruction_context,
                                // &log_collector,
                            )?;
                            let clock = invoke_context.get_sysvar_cache().get_clock()?;
                            invoke_context
                                .program_cache_for_tx_batch
                                .store_modified_entry(
                                    program_key,
                                    Arc::new(ProgramCacheEntry::new_tombstone(
                                        clock.slot,
                                        ProgramCacheEntryOwner::LoaderV3,
                                        ProgramCacheEntryType::Closed,
                                    )),
                                );
                        }
                        _ => {
                            // ic_logger_msg!(log_collector, "Invalid Program account");
                            return Err(InstructionError::InvalidArgument);
                        }
                    }

                    // ic_logger_msg!(log_collector, "Closed Program {}", program_key);
                }
                _ => {
                    // ic_logger_msg!(log_collector, "Account does not support closing");
                    return Err(InstructionError::InvalidArgument);
                }
            }
        }
        UpgradeableLoaderInstruction::ExtendProgram { additional_bytes } => {
            if !invoke_context
                .environment_config
                .feature_set
                .is_active(&enable_bpf_loader_extend_program_ix::ID)
            {
                return Err(InstructionError::InvalidInstructionData);
            }

            if additional_bytes == 0 {
                // ic_logger_msg!(log_collector, "Additional bytes must be greater than 0");
                return Err(InstructionError::InvalidInstructionData);
            }

            const PROGRAM_DATA_ACCOUNT_INDEX: IndexOfAccount = 0;
            const PROGRAM_ACCOUNT_INDEX: IndexOfAccount = 1;
            #[allow(dead_code)]
            // System program is only required when a CPI is performed
            const OPTIONAL_SYSTEM_PROGRAM_ACCOUNT_INDEX: IndexOfAccount = 2;
            const OPTIONAL_PAYER_ACCOUNT_INDEX: IndexOfAccount = 3;

            let programdata_account = instruction_context
                .try_borrow_instruction_account(transaction_context, PROGRAM_DATA_ACCOUNT_INDEX)?;
            let programdata_key = *programdata_account.get_key();

            if program_id != programdata_account.get_owner() {
                // ic_logger_msg!(log_collector, "ProgramData owner is invalid");
                return Err(InstructionError::InvalidAccountOwner);
            }
            if !programdata_account.is_writable() {
                // ic_logger_msg!(log_collector, "ProgramData is not writable");
                return Err(InstructionError::InvalidArgument);
            }

            let program_account = instruction_context
                .try_borrow_instruction_account(transaction_context, PROGRAM_ACCOUNT_INDEX)?;
            if !program_account.is_writable() {
                // ic_logger_msg!(log_collector, "Program account is not writable");
                return Err(InstructionError::InvalidArgument);
            }
            if program_account.get_owner() != program_id {
                // ic_logger_msg!(log_collector, "Program account not owned by loader");
                return Err(InstructionError::InvalidAccountOwner);
            }
            let program_key = *program_account.get_key();
            match program_account.get_state()? {
                UpgradeableLoaderState::Program {
                    programdata_address,
                } => {
                    if programdata_address != programdata_key {
                        // ic_logger_msg!(
                        //     log_collector,
                        //     "Program account does not match ProgramData account"
                        // );
                        return Err(InstructionError::InvalidArgument);
                    }
                }
                _ => {
                    // ic_logger_msg!(log_collector, "Invalid Program account");
                    return Err(InstructionError::InvalidAccountData);
                }
            }
            drop(program_account);

            let old_len = programdata_account.get_data().len();
            let new_len = old_len.saturating_add(additional_bytes as usize);
            if new_len > MAX_PERMITTED_DATA_LENGTH as usize {
                // ic_logger_msg!(
                //     log_collector,
                //     "Extended ProgramData length of {} bytes exceeds max account data length of {} bytes",
                //     new_len,
                //     MAX_PERMITTED_DATA_LENGTH
                // );
                return Err(InstructionError::InvalidRealloc);
            }

            let clock_slot = invoke_context
                .get_sysvar_cache()
                .get_clock()
                .map(|clock| clock.slot)?;

            let upgrade_authority_address = if let UpgradeableLoaderState::ProgramData {
                slot,
                upgrade_authority_address,
            } = programdata_account.get_state()?
            {
                if clock_slot == slot {
                    // ic_logger_msg!(log_collector, "Program was extended in this block already");
                    return Err(InstructionError::InvalidArgument);
                }

                if upgrade_authority_address.is_none() {
                    // ic_logger_msg!(
                    //     log_collector,
                    //     "Cannot extend ProgramData accounts that are not upgradeable"
                    // );
                    return Err(InstructionError::Immutable);
                }
                upgrade_authority_address
            } else {
                // ic_logger_msg!(log_collector, "ProgramData state is invalid");
                return Err(InstructionError::InvalidAccountData);
            };

            let required_payment = {
                let balance = programdata_account.get_lamports();
                let rent = invoke_context.get_sysvar_cache().get_rent()?;
                let min_balance = rent.minimum_balance(new_len).max(1);
                min_balance.saturating_sub(balance)
            };

            // Borrowed accounts need to be dropped before native_invoke
            drop(programdata_account);

            // Dereference the program ID to prevent overlapping mutable/immutable borrow of invoke context
            let program_id = *program_id;
            if required_payment > 0 {
                let payer_key = *transaction_context.get_key_of_account_at_index(
                    instruction_context.get_index_of_instruction_account_in_transaction(
                        OPTIONAL_PAYER_ACCOUNT_INDEX,
                    )?,
                )?;

                invoke_context.native_invoke(
                    system_instruction::transfer(&payer_key, &programdata_key, required_payment)
                        .into(),
                    &[],
                )?;
            }

            let transaction_context = &invoke_context.transaction_context;
            let instruction_context = transaction_context.get_current_instruction_context()?;
            let mut programdata_account = instruction_context
                .try_borrow_instruction_account(transaction_context, PROGRAM_DATA_ACCOUNT_INDEX)?;
            programdata_account.set_data_length(new_len)?;

            let programdata_data_offset = UpgradeableLoaderState::size_of_programdata_metadata();

            deploy_program!(
                invoke_context,
                program_key,
                &program_id,
                UpgradeableLoaderState::size_of_program().saturating_add(new_len),
                clock_slot,
                {
                    drop(programdata_account);
                },
                programdata_account
                    .get_data()
                    .get(programdata_data_offset..)
                    .ok_or(InstructionError::AccountDataTooSmall)?,
            );

            let mut programdata_account = instruction_context
                .try_borrow_instruction_account(transaction_context, PROGRAM_DATA_ACCOUNT_INDEX)?;
            programdata_account.set_state(&UpgradeableLoaderState::ProgramData {
                slot: clock_slot,
                upgrade_authority_address,
            })?;

            // ic_logger_msg!(
            //     log_collector,
            //     "Extended ProgramData account by {} bytes",
            //     additional_bytes
            // );
        }
    }

    Ok(())
}
