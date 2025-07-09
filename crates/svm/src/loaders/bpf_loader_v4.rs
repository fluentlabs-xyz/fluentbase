use crate::{
    account::BorrowedAccount,
    common::{limited_deserialize_packet_size, rbpf_config_default},
    compute_budget::compute_budget::ComputeBudget,
    context::{InstructionContext, InvokeContext},
    error::Error,
    loaded_programs::{ProgramCacheEntry, ProgramCacheEntryType, DELAY_VISIBILITY_SLOT_OFFSET},
    loaders::execute::execute,
    solana_program::{
        loader_v4,
        loader_v4::{LoaderV4State, LoaderV4Status, DEPLOYMENT_COOLDOWN_IN_SLOTS},
        loader_v4_instruction::LoaderV4Instruction,
    },
};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::sync::atomic::Ordering;
use fluentbase_sdk::SharedAPI;
use solana_instruction::error::InstructionError;
use solana_pubkey::Pubkey;
use solana_rbpf::{
    declare_builtin_function,
    memory_region::MemoryMapping,
    program::{BuiltinProgram, FunctionRegistry},
};

pub const DEFAULT_COMPUTE_UNITS: u64 = 2_000;

pub(crate) fn get_state(data: &[u8]) -> Result<&LoaderV4State, InstructionError> {
    unsafe {
        let data = data
            .get(0..LoaderV4State::program_data_offset())
            .ok_or(InstructionError::AccountDataTooSmall)?
            .try_into()
            .unwrap();
        Ok(core::mem::transmute::<
            &[u8; LoaderV4State::program_data_offset()],
            &LoaderV4State,
        >(data))
    }
}

pub(crate) fn get_state_mut(data: &mut [u8]) -> Result<&mut LoaderV4State, InstructionError> {
    unsafe {
        let data = data
            .get_mut(0..LoaderV4State::program_data_offset())
            .ok_or(InstructionError::AccountDataTooSmall)?
            .try_into()
            .unwrap();
        Ok(core::mem::transmute::<
            &mut [u8; LoaderV4State::program_data_offset()],
            &mut LoaderV4State,
        >(data))
    }
}

pub fn create_program_runtime_environment<'a, SDK: SharedAPI>(
    compute_budget: &ComputeBudget,
    debugging_features: bool,
) -> BuiltinProgram<InvokeContext<'a, SDK>> {
    let mut config = rbpf_config_default(Some(&compute_budget));
    if debugging_features {
        config.enable_instruction_tracing = debugging_features;
        config.enable_symbol_and_section_labels = debugging_features;
    }
    BuiltinProgram::new_loader(config, FunctionRegistry::default())
}

fn check_program_account(
    instruction_context: &InstructionContext,
    program: &BorrowedAccount,
    authority_address: &Pubkey,
) -> Result<LoaderV4State, InstructionError> {
    if !loader_v4::check_id(program.get_owner()) {
        return Err(InstructionError::InvalidAccountOwner);
    }
    if program.get_data().is_empty() {
        return Err(InstructionError::InvalidAccountData);
    }
    let state = get_state(program.get_data())?;
    if !program.is_writable() {
        return Err(InstructionError::InvalidArgument);
    }
    if !instruction_context.is_instruction_account_signer(1)? {
        return Err(InstructionError::MissingRequiredSignature);
    }
    if &state.authority_address_or_next_version != authority_address {
        return Err(InstructionError::IncorrectAuthority);
    }
    if matches!(state.status, LoaderV4Status::Finalized) {
        return Err(InstructionError::Immutable);
    }
    Ok(*state)
}

pub fn process_instruction_write<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
    offset: u32,
    bytes: Vec<u8>,
) -> Result<(), InstructionError> {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let mut program = instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
    let authority_address = instruction_context
        .get_index_of_instruction_account_in_transaction(1)
        .and_then(|index| transaction_context.get_key_of_account_at_index(index))?;
    let state = check_program_account(instruction_context, &program, authority_address)?;
    if !matches!(state.status, LoaderV4Status::Retracted) {
        return Err(InstructionError::InvalidArgument);
    }
    let end_offset = (offset as usize).saturating_add(bytes.len());
    program
        .get_data_mut()?
        .get_mut(
            LoaderV4State::program_data_offset().saturating_add(offset as usize)
                ..LoaderV4State::program_data_offset().saturating_add(end_offset),
        )
        .ok_or_else(|| InstructionError::AccountDataTooSmall)?
        .copy_from_slice(&bytes);
    Ok(())
}

pub fn process_instruction_truncate<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
    new_size: u32,
) -> Result<(), InstructionError> {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let mut program = instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
    let authority_address = instruction_context
        .get_index_of_instruction_account_in_transaction(1)
        .and_then(|index| transaction_context.get_key_of_account_at_index(index))?;
    let is_initialization =
        new_size > 0 && program.get_data().len() < LoaderV4State::program_data_offset();
    if is_initialization {
        if !loader_v4::check_id(program.get_owner()) {
            return Err(InstructionError::InvalidAccountOwner);
        }
        if !program.is_writable() {
            return Err(InstructionError::InvalidArgument);
        }
        if !program.is_signer() {
            return Err(InstructionError::MissingRequiredSignature);
        }
        if !instruction_context.is_instruction_account_signer(1)? {
            return Err(InstructionError::MissingRequiredSignature);
        }
    } else {
        let state = check_program_account(instruction_context, &program, authority_address);
        let state = state?;
        if !matches!(state.status, LoaderV4Status::Retracted) {
            return Err(InstructionError::InvalidArgument);
        }
    }
    let required_lamports = if new_size == 0 {
        0
    } else {
        let rent = invoke_context.get_sysvar_cache().get_rent()?;
        rent.minimum_balance(LoaderV4State::program_data_offset().saturating_add(new_size as usize))
    };
    match program.get_lamports().cmp(&required_lamports) {
        core::cmp::Ordering::Less => {
            return Err(InstructionError::InsufficientFunds);
        }
        core::cmp::Ordering::Greater => {
            let recipient =
                instruction_context.try_borrow_instruction_account(transaction_context, 2);
            let mut recipient = recipient?;
            if !instruction_context.is_instruction_account_writable(2)? {
                return Err(InstructionError::InvalidArgument);
            }
            let lamports_to_receive = program.get_lamports().saturating_sub(required_lamports);
            program.checked_sub_lamports(lamports_to_receive)?;
            recipient.checked_add_lamports(lamports_to_receive)?;
        }
        core::cmp::Ordering::Equal => {}
    }
    if new_size == 0 {
        program.set_data_length(0)?;
    } else {
        program.set_data_length(
            LoaderV4State::program_data_offset().saturating_add(new_size as usize),
        )?;
        if is_initialization {
            let state = get_state_mut(program.get_data_mut()?)?;
            state.slot = 0;
            state.status = LoaderV4Status::Retracted;
            state.authority_address_or_next_version = *authority_address;
        }
    }
    Ok(())
}

pub fn process_instruction_deploy<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let mut program = instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
    let authority_address = instruction_context
        .get_index_of_instruction_account_in_transaction(1)
        .and_then(|index| transaction_context.get_key_of_account_at_index(index))?;
    let source_program = instruction_context
        .try_borrow_instruction_account(transaction_context, 2)
        .ok();
    let state = check_program_account(instruction_context, &program, authority_address)?;
    let current_slot = invoke_context.get_sysvar_cache().get_clock()?.slot;

    // Slot = 0 indicates that the program hasn't been deployed yet. So no need to check for the
    // cooldown slots. (Without this check, the program deployment is failing in freshly started
    // test validators. That's  because at startup current_slot is 0, which is < DEPLOYMENT_COOLDOWN_IN_SLOTS).
    if state.slot != 0 && state.slot.saturating_add(DEPLOYMENT_COOLDOWN_IN_SLOTS) > current_slot {
        return Err(InstructionError::InvalidArgument);
    }
    if !matches!(state.status, LoaderV4Status::Retracted) {
        return Err(InstructionError::InvalidArgument);
    }
    let buffer = if let Some(ref source_program) = source_program {
        let source_state =
            check_program_account(instruction_context, source_program, authority_address)?;
        if !matches!(source_state.status, LoaderV4Status::Retracted) {
            return Err(InstructionError::InvalidArgument);
        }
        source_program
    } else {
        &program
    };

    let programdata = buffer
        .get_data()
        .get(LoaderV4State::program_data_offset()..)
        .ok_or(InstructionError::AccountDataTooSmall)?;

    let deployment_slot = state.slot;
    let effective_slot = deployment_slot.saturating_add(DELAY_VISIBILITY_SLOT_OFFSET);

    let environments = invoke_context
        .get_environments_for_slot(effective_slot)
        .map_err(|_err| {
            // This will never fail since the epoch schedule is already configured.
            InstructionError::InvalidArgument
        })?;

    let executor = ProgramCacheEntry::new(
        &loader_v4::id(),
        environments.program_runtime_v2.clone(),
        deployment_slot,
        effective_slot,
        programdata,
        buffer.get_data().len(),
    )
    .map_err(|_err| InstructionError::InvalidAccountData)?;
    if let Some(mut source_program) = source_program {
        let rent = invoke_context.get_sysvar_cache().get_rent()?;
        let required_lamports = rent.minimum_balance(source_program.get_data().len());
        let transfer_lamports = required_lamports.saturating_sub(program.get_lamports());
        program.set_data_from_slice(source_program.get_data())?;
        source_program.set_data_length(0)?;
        source_program.checked_sub_lamports(transfer_lamports)?;
        program.checked_add_lamports(transfer_lamports)?;
    }
    let state = get_state_mut(program.get_data_mut()?)?;
    state.slot = current_slot;
    state.status = LoaderV4Status::Deployed;

    if let Some(old_entry) = invoke_context
        .program_cache_for_tx_batch
        .find(program.get_key())
    {
        executor.tx_usage_counter.store(
            old_entry.tx_usage_counter.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        executor.ix_usage_counter.store(
            old_entry.ix_usage_counter.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
    }
    invoke_context
        .program_cache_for_tx_batch
        .replenish(*program.get_key(), Arc::new(executor));
    Ok(())
}

pub fn process_instruction_retract<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let mut program = instruction_context.try_borrow_instruction_account(transaction_context, 0)?;

    let authority_address = instruction_context
        .get_index_of_instruction_account_in_transaction(1)
        .and_then(|index| transaction_context.get_key_of_account_at_index(index))?;
    let state = check_program_account(instruction_context, &program, authority_address)?;
    let current_slot = invoke_context.get_sysvar_cache().get_clock()?.slot;
    if state.slot.saturating_add(DEPLOYMENT_COOLDOWN_IN_SLOTS) > current_slot {
        return Err(InstructionError::InvalidArgument);
    }
    if matches!(state.status, LoaderV4Status::Retracted) {
        return Err(InstructionError::InvalidArgument);
    }
    let state = get_state_mut(program.get_data_mut()?)?;
    state.status = LoaderV4Status::Retracted;
    Ok(())
}

pub fn process_instruction_transfer_authority<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let mut program = instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
    let authority_address = instruction_context
        .get_index_of_instruction_account_in_transaction(1)
        .and_then(|index| transaction_context.get_key_of_account_at_index(index))?;
    let new_authority_address = instruction_context
        .get_index_of_instruction_account_in_transaction(2)
        .and_then(|index| transaction_context.get_key_of_account_at_index(index))
        .ok()
        .cloned();
    let _state = check_program_account(instruction_context, &program, authority_address)?;
    if new_authority_address.is_some() && !instruction_context.is_instruction_account_signer(2)? {
        return Err(InstructionError::MissingRequiredSignature);
    }
    let state = get_state_mut(program.get_data_mut()?)?;
    if let Some(new_authority_address) = new_authority_address {
        state.authority_address_or_next_version = new_authority_address;
    } else if matches!(state.status, LoaderV4Status::Deployed) {
        state.status = LoaderV4Status::Finalized;
    } else {
        return Err(InstructionError::InvalidArgument);
    }
    Ok(())
}

pub fn process_instruction_finalize<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let program = instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
    let authority_address = instruction_context
        .get_index_of_instruction_account_in_transaction(1)
        .and_then(|index| transaction_context.get_key_of_account_at_index(index))?;
    let state = check_program_account(instruction_context, &program, authority_address)?;
    if !matches!(state.status, LoaderV4Status::Deployed) {
        return Err(InstructionError::InvalidArgument);
    }
    drop(program);
    let next_version =
        instruction_context.try_borrow_instruction_account(transaction_context, 2)?;
    if !loader_v4::check_id(next_version.get_owner()) {
        return Err(InstructionError::InvalidAccountOwner);
    }
    let state_of_next_version = get_state(next_version.get_data())?;
    if state_of_next_version.authority_address_or_next_version != *authority_address {
        return Err(InstructionError::IncorrectAuthority);
    }
    if matches!(state_of_next_version.status, LoaderV4Status::Finalized) {
        return Err(InstructionError::Immutable);
    }
    let address_of_next_version = *next_version.get_key();
    drop(next_version);
    let mut program = instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
    let state = get_state_mut(program.get_data_mut()?)?;
    state.authority_address_or_next_version = address_of_next_version;
    state.status = LoaderV4Status::Finalized;
    Ok(())
}

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
    ) -> Result<u64, Error> {
        process_instruction_inner(invoke_context)
    }
);

pub fn process_instruction_inner<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<u64, Error> {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let instruction_data = instruction_context.get_instruction_data();
    let program_id = instruction_context.get_last_program_key(transaction_context)?;
    if loader_v4::check_id(program_id) {
        match limited_deserialize_packet_size(instruction_data)? {
            LoaderV4Instruction::Write { offset, bytes } => {
                process_instruction_write(invoke_context, offset, bytes)
            }
            LoaderV4Instruction::Truncate { new_size } => {
                process_instruction_truncate(invoke_context, new_size)
            }
            LoaderV4Instruction::Deploy => process_instruction_deploy(invoke_context),
            LoaderV4Instruction::Retract => process_instruction_retract(invoke_context),
            LoaderV4Instruction::TransferAuthority => {
                process_instruction_transfer_authority(invoke_context)
            }
            LoaderV4Instruction::Finalize => process_instruction_finalize(invoke_context),
        }
        .map_err(|err| Box::new(err) as Error)
    } else {
        let program = instruction_context.try_borrow_last_program_account(transaction_context)?;
        let program_owner = program.get_owner();
        if !loader_v4::check_id(program_owner) {
            return Err(Box::new(InstructionError::InvalidAccountOwner));
        }
        if program.get_data().is_empty() {
            return Err(Box::new(InstructionError::InvalidAccountData));
        }
        let state = get_state(program.get_data())?;
        if matches!(state.status, LoaderV4Status::Retracted) {
            return Err(Box::new(InstructionError::InvalidArgument));
        }

        let program_key = program.get_key().clone();
        let loaded_program = invoke_context
            .program_cache_for_tx_batch
            .find(&program_key)
            .ok_or_else(|| InstructionError::InvalidAccountData)?;

        drop(program);

        let loaded_program = &loaded_program.program;

        match loaded_program {
            ProgramCacheEntryType::FailedVerification(_)
            | ProgramCacheEntryType::Closed
            | ProgramCacheEntryType::DelayVisibility => {
                Err(Box::new(InstructionError::UnsupportedProgramId) as Box<dyn core::error::Error>)
            }
            ProgramCacheEntryType::Loaded(executable) => {
                execute(executable.clone(), invoke_context)
            }
            _ => {
                Err(Box::new(InstructionError::UnsupportedProgramId) as Box<dyn core::error::Error>)
            }
        }
    }
    .map(|_| 0)
}
