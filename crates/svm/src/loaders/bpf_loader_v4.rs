use crate::{
    account::BorrowedAccount,
    common::limited_deserialize_packet_size,
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
use alloc::{boxed::Box, sync::Arc, vec, vec::Vec};
use core::sync::atomic::Ordering;
use fluentbase_sdk::{debug_log, SharedAPI};
use solana_instruction::error::InstructionError;
use solana_pubkey::Pubkey;
use solana_rbpf::{
    aligned_memory::AlignedMemory,
    declare_builtin_function,
    ebpf,
    elf::Executable,
    error::ProgramResult,
    memory_region::{MemoryMapping, MemoryRegion},
    program::{BuiltinProgram, FunctionRegistry},
    vm::{Config, ContextObject, EbpfVm},
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

pub fn create_program_runtime_environment_v2<'a, SDK: SharedAPI>(
    compute_budget: &ComputeBudget,
    debugging_features: bool,
) -> BuiltinProgram<InvokeContext<'a, SDK>> {
    let config = Config {
        max_call_depth: compute_budget.max_call_depth,
        stack_frame_size: compute_budget.stack_frame_size,
        enable_address_translation: true, // To be deactivated once we have BTF inference and verification
        enable_stack_frame_gaps: false,
        instruction_meter_checkpoint_distance: 10000,
        enable_instruction_meter: true,
        enable_instruction_tracing: debugging_features,
        enable_symbol_and_section_labels: debugging_features,
        reject_broken_elfs: true,
        noop_instruction_rate: 256,
        sanitize_user_provided_values: true,
        external_internal_function_hash_collision: true,
        reject_callx_r10: true,
        enable_sbpf_v1: false,
        enable_sbpf_v2: true,
        optimize_rodata: true,
        aligned_memory_mapping: true,
        // Warning, do not use `Config::default()` so that configuration here is explicit.
    };
    BuiltinProgram::new_loader(config, FunctionRegistry::default())
}

fn calculate_heap_cost(heap_size: u32, heap_cost: u64) -> u64 {
    const KIBIBYTE: u64 = 1024;
    const PAGE_SIZE_KB: u64 = 32;
    u64::from(heap_size)
        .saturating_add(PAGE_SIZE_KB.saturating_mul(KIBIBYTE).saturating_sub(1))
        .checked_div(PAGE_SIZE_KB.saturating_mul(KIBIBYTE))
        .expect("PAGE_SIZE_KB * KIBIBYTE > 0")
        .saturating_sub(1)
        .saturating_mul(heap_cost)
}

// /// Create the SBF virtual machine
// pub fn create_vm<'a, SDK: SharedAPI>(
//     invoke_context: &mut InvokeContext<'a, SDK>,
//     program: Arc<Executable<InvokeContext<'a, SDK>>>,
// ) -> Result<EbpfVm<'a, InvokeContext<'a, SDK>>, Box<dyn std::error::Error>> {
//     let config = program.get_config();
//     let sbpf_version = program.get_sbpf_version();
//     let compute_budget = invoke_context.get_compute_budget();
//     let heap_size = compute_budget.heap_size;
//     invoke_context.consume_checked(calculate_heap_cost(heap_size, compute_budget.heap_cost))?;
//     let mut stack = AlignedMemory::<{ ebpf::HOST_ALIGN }>::zero_filled(config.stack_size());
//     let mut heap = AlignedMemory::<{ ebpf::HOST_ALIGN }>::zero_filled(
//         usize::try_from(compute_budget.heap_size).unwrap(),
//     );
//     let stack_len = stack.len();
//     let regions: Vec<MemoryRegion> = vec![
//         program.get_ro_region(),
//         MemoryRegion::new_writable_gapped(stack.as_slice_mut(), ebpf::MM_STACK_START, 0),
//         MemoryRegion::new_writable(heap.as_slice_mut(), ebpf::MM_HEAP_START),
//     ];
//     // let log_collector = invoke_context.get_log_collector();
//     let memory_mapping = MemoryMapping::new(regions, config, sbpf_version).map_err(|err| {
//         // ic_logger_msg!(log_collector, "Failed to create SBF VM: {}", err);
//         Box::new(InstructionError::ProgramEnvironmentSetupFailure)
//     })?;
//     Ok(EbpfVm::new(
//         program.get_loader().clone(),
//         sbpf_version,
//         invoke_context,
//         memory_mapping,
//         stack_len,
//     ))
// }

/// Create the SBF virtual machine
pub fn create_vm_exec_program<'a, SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<'a, SDK>,
    program: Arc<Executable<InvokeContext<'a, SDK>>>,
) -> Result<(u64, ProgramResult), Error> {
    let config = program.get_config();
    let sbpf_version = program.get_sbpf_version();
    let compute_budget = invoke_context.get_compute_budget();
    let heap_size = compute_budget.heap_size;
    invoke_context.consume_checked(calculate_heap_cost(heap_size, compute_budget.heap_cost))?;
    let mut stack = AlignedMemory::<{ ebpf::HOST_ALIGN }>::zero_filled(config.stack_size());
    let mut heap = AlignedMemory::<{ ebpf::HOST_ALIGN }>::zero_filled(
        usize::try_from(compute_budget.heap_size).unwrap(),
    );
    let stack_len = stack.len();
    let regions: Vec<MemoryRegion> = vec![
        program.get_ro_region(),
        MemoryRegion::new_writable_gapped(stack.as_slice_mut(), ebpf::MM_STACK_START, 0),
        MemoryRegion::new_writable(heap.as_slice_mut(), ebpf::MM_HEAP_START),
    ];
    // let log_collector = invoke_context.get_log_collector();
    let memory_mapping = MemoryMapping::new(regions, config, sbpf_version).map_err(|err| {
        // ic_logger_msg!(log_collector, "Failed to create SBF VM: {}", err);
        Box::new(InstructionError::ProgramEnvironmentSetupFailure)
    })?;
    let mut vm = EbpfVm::new(
        program.get_loader().clone(),
        sbpf_version,
        invoke_context,
        memory_mapping,
        stack_len,
    );
    let res: (u64, ProgramResult) = vm.execute_program(&program, true);
    Ok(res)
}

/*fn execute<'a, SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<'a, SDK>,
    executable: Arc<Executable<InvokeContext<'a, SDK>>>,
) -> Result<(), Error> {
    // We dropped the lifetime tracking in the Executor by setting it to 'static,
    // thus we need to reintroduce the correct lifetime of InvokeContext here again.
    // let executable =
    //     unsafe { std::mem::transmute::<_, &'a Executable<InvokeContext<'b>>>(executable) };
    // let log_collector = invoke_context.get_log_collector();
    // let stack_height = invoke_context.get_stack_height();
    // let transaction_context = &invoke_context.transaction_context;
    // let instruction_context = transaction_context.get_current_instruction_context()?;
    // let program_id = *instruction_context.get_last_program_key(transaction_context)?;
    #[cfg(any(target_os = "windows", not(target_arch = "x86_64")))]
    let use_jit = false;
    // #[cfg(all(not(target_os = "windows"), target_arch = "x86_64"))]
    // let use_jit = executable.get_compiled_program().is_some();

    // let compute_meter_prev = invoke_context.get_remaining();
    // let mut create_vm_time = Measure::start("create_vm");
    // let mut vm = create_vm(invoke_context, executable.clone())?;
    // let (_, result) = create_vm_exec(invoke_context, executable.clone())?;
    // create_vm_time.stop();

    // let mut execute_time = Measure::start("execute");
    // stable_log::program_invoke(&log_collector, &program_id, stack_height);
    let (_compute_units_consumed, result) =
        create_vm_exec_program(invoke_context, executable.clone())?;
    // drop(vm);
    // ic_logger_msg!(
    //     log_collector,
    //     "Program {} consumed {} of {} compute units",
    //     &program_id,
    //     compute_units_consumed,
    //     compute_meter_prev
    // );
    // execute_time.stop();

    // let timings = &mut invoke_context.timings;
    // timings.create_vm_us = timings.create_vm_us.saturating_add(create_vm_time.as_us());
    // timings.execute_us = timings.execute_us.saturating_add(execute_time.as_us());

    match result {
        ProgramResult::Ok(status) if status != SUCCESS => {
            let error: InstructionError = status.into();
            Err(error.into())
        }
        ProgramResult::Err(error) => Err(error.into()),
        _ => Ok(()),
    }
}*/

fn check_program_account(
    // log_collector: &Option<Rc<RefCell<LogCollector>>>,
    instruction_context: &InstructionContext,
    program: &BorrowedAccount,
    authority_address: &Pubkey,
) -> Result<LoaderV4State, InstructionError> {
    if !loader_v4::check_id(program.get_owner()) {
        // ic_logger_msg!(log_collector, "Program not owned by loader");
        return Err(InstructionError::InvalidAccountOwner);
    }
    if program.get_data().is_empty() {
        // ic_logger_msg!(log_collector, "Program is uninitialized");
        return Err(InstructionError::InvalidAccountData);
    }
    let state = get_state(program.get_data())?;
    if !program.is_writable() {
        // ic_logger_msg!(log_collector, "Program is not writeable");
        return Err(InstructionError::InvalidArgument);
    }
    if !instruction_context.is_instruction_account_signer(1)? {
        // ic_logger_msg!(log_collector, "Authority did not sign");
        return Err(InstructionError::MissingRequiredSignature);
    }
    if &state.authority_address_or_next_version != authority_address {
        // ic_logger_msg!(log_collector, "Incorrect authority provided");
        return Err(InstructionError::IncorrectAuthority);
    }
    if matches!(state.status, LoaderV4Status::Finalized) {
        // ic_logger_msg!(log_collector, "Program is finalized");
        return Err(InstructionError::Immutable);
    }
    Ok(*state)
}

pub fn process_instruction_write<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
    offset: u32,
    bytes: Vec<u8>,
) -> Result<(), InstructionError> {
    // let log_collector = invoke_context.get_log_collector();
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let mut program = instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
    let authority_address = instruction_context
        .get_index_of_instruction_account_in_transaction(1)
        .and_then(|index| transaction_context.get_key_of_account_at_index(index))?;
    let state = check_program_account(
        // &log_collector,
        instruction_context,
        &program,
        authority_address,
    )?;
    if !matches!(state.status, LoaderV4Status::Retracted) {
        // ic_logger_msg!(log_collector, "Program is not retracted");
        return Err(InstructionError::InvalidArgument);
    }
    let end_offset = (offset as usize).saturating_add(bytes.len());
    program
        .get_data_mut()?
        .get_mut(
            LoaderV4State::program_data_offset().saturating_add(offset as usize)
                ..LoaderV4State::program_data_offset().saturating_add(end_offset),
        )
        .ok_or_else(|| {
            // ic_logger_msg!(log_collector, "Write out of bounds");
            InstructionError::AccountDataTooSmall
        })?
        .copy_from_slice(&bytes);
    Ok(())
}

pub fn process_instruction_truncate<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
    new_size: u32,
) -> Result<(), InstructionError> {
    // let log_collector = invoke_context.get_log_collector();
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
            // ic_logger_msg!(log_collector, "Program not owned by loader");
            return Err(InstructionError::InvalidAccountOwner);
        }
        if !program.is_writable() {
            // ic_logger_msg!(log_collector, "Program is not writeable");
            return Err(InstructionError::InvalidArgument);
        }
        if !program.is_signer() {
            // ic_logger_msg!(log_collector, "Program did not sign");
            return Err(InstructionError::MissingRequiredSignature);
        }
        if !instruction_context.is_instruction_account_signer(1)? {
            // ic_logger_msg!(log_collector, "Authority did not sign");
            return Err(InstructionError::MissingRequiredSignature);
        }
    } else {
        let state = check_program_account(
            // &log_collector,
            instruction_context,
            &program,
            authority_address,
        )?;
        if !matches!(state.status, LoaderV4Status::Retracted) {
            // ic_logger_msg!(log_collector, "Program is not retracted");
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
            // ic_logger_msg!(
            //     log_collector,
            //     "Insufficient lamports, {} are required",
            //     required_lamports
            // );
            return Err(InstructionError::InsufficientFunds);
        }
        core::cmp::Ordering::Greater => {
            let mut recipient =
                instruction_context.try_borrow_instruction_account(transaction_context, 2)?;
            if !instruction_context.is_instruction_account_writable(2)? {
                // ic_logger_msg!(log_collector, "Recipient is not writeable");
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
    // let log_collector = invoke_context.get_log_collector();
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let mut program = instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
    let authority_address = instruction_context
        .get_index_of_instruction_account_in_transaction(1)
        .and_then(|index| transaction_context.get_key_of_account_at_index(index))?;
    let source_program = instruction_context
        .try_borrow_instruction_account(transaction_context, 2)
        .ok();
    let state = check_program_account(
        // &log_collector,
        instruction_context,
        &program,
        authority_address,
    )?;
    let current_slot = invoke_context.get_sysvar_cache().get_clock()?.slot;

    // Slot = 0 indicates that the program hasn't been deployed yet. So no need to check for the cooldown slots.
    // (Without this check, the program deployment is failing in freshly started test validators. That's
    //  because at startup current_slot is 0, which is < DEPLOYMENT_COOLDOWN_IN_SLOTS).
    if state.slot != 0 && state.slot.saturating_add(DEPLOYMENT_COOLDOWN_IN_SLOTS) > current_slot {
        // ic_logger_msg!(
        //     log_collector,
        //     "Program was deployed recently, cooldown still in effect"
        // );
        return Err(InstructionError::InvalidArgument);
    }
    if !matches!(state.status, LoaderV4Status::Retracted) {
        // ic_logger_msg!(log_collector, "Destination program is not retracted");
        return Err(InstructionError::InvalidArgument);
    }
    let buffer = if let Some(ref source_program) = source_program {
        let source_state = check_program_account(
            // &log_collector,
            instruction_context,
            source_program,
            authority_address,
        )?;
        if !matches!(source_state.status, LoaderV4Status::Retracted) {
            // ic_logger_msg!(log_collector, "Source program is not retracted");
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
        .map_err(|err| {
            // This will never fail since the epoch schedule is already configured.
            // ic_logger_msg!(log_collector, "Failed to get runtime environment {}", err);
            InstructionError::InvalidArgument
        })?;

    // let mut load_program_metrics = LoadProgramMetrics {
    //     program_id: buffer.get_key().to_string(),
    //     ..LoadProgramMetrics::default()
    // };
    let executor = ProgramCacheEntry::new(
        &loader_v4::id(),
        environments.program_runtime_v2.clone(),
        deployment_slot,
        effective_slot,
        programdata,
        buffer.get_data().len(),
        // &mut load_program_metrics,
    )
    .map_err(|err| {
        // ic_logger_msg!(log_collector, "{}", err);
        // debug_log!("error while LoadedProgram::new: {}", err);
        InstructionError::InvalidAccountData
    })?;
    // load_program_metrics.submit_datapoint(&mut invoke_context.timings);
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
    // program.set_executable(true)?;
    Ok(())
}

pub fn process_instruction_retract<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    // let log_collector = invoke_context.get_log_collector();
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let mut program = instruction_context.try_borrow_instruction_account(transaction_context, 0)?;

    let authority_address = instruction_context
        .get_index_of_instruction_account_in_transaction(1)
        .and_then(|index| transaction_context.get_key_of_account_at_index(index))?;
    let state = check_program_account(
        // &log_collector,
        instruction_context,
        &program,
        authority_address,
    )?;
    let current_slot = invoke_context.get_sysvar_cache().get_clock()?.slot;
    if state.slot.saturating_add(DEPLOYMENT_COOLDOWN_IN_SLOTS) > current_slot {
        // ic_logger_msg!(
        //     log_collector,
        //     "Program was deployed recently, cooldown still in effect"
        // );
        return Err(InstructionError::InvalidArgument);
    }
    if matches!(state.status, LoaderV4Status::Retracted) {
        // ic_logger_msg!(log_collector, "Program is not deployed");
        return Err(InstructionError::InvalidArgument);
    }
    let state = get_state_mut(program.get_data_mut()?)?;
    state.status = LoaderV4Status::Retracted;
    Ok(())
}

pub fn process_instruction_transfer_authority<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    // let log_collector = invoke_context.get_log_collector();
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
    let _state = check_program_account(
        // &log_collector,
        instruction_context,
        &program,
        authority_address,
    )?;
    if new_authority_address.is_some() && !instruction_context.is_instruction_account_signer(2)? {
        // ic_logger_msg!(log_collector, "New authority did not sign");
        return Err(InstructionError::MissingRequiredSignature);
    }
    let state = get_state_mut(program.get_data_mut()?)?;
    if let Some(new_authority_address) = new_authority_address {
        state.authority_address_or_next_version = new_authority_address;
    } else if matches!(state.status, LoaderV4Status::Deployed) {
        state.status = LoaderV4Status::Finalized;
    } else {
        // ic_logger_msg!(log_collector, "Program must be deployed to be finalized");
        return Err(InstructionError::InvalidArgument);
    }
    Ok(())
}

pub fn process_instruction_finalize<SDK: SharedAPI>(
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    // let log_collector = invoke_context.get_log_collector();
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let program = instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
    let authority_address = instruction_context
        .get_index_of_instruction_account_in_transaction(1)
        .and_then(|index| transaction_context.get_key_of_account_at_index(index))?;
    let state = check_program_account(
        // &log_collector,
        instruction_context,
        &program,
        authority_address,
    )?;
    if !matches!(state.status, LoaderV4Status::Deployed) {
        // ic_logger_msg!(log_collector, "Program must be deployed to be finalized");
        return Err(InstructionError::InvalidArgument);
    }
    drop(program);
    let next_version =
        instruction_context.try_borrow_instruction_account(transaction_context, 2)?;
    if !loader_v4::check_id(next_version.get_owner()) {
        // ic_logger_msg!(log_collector, "Next version is not owned by loader");
        return Err(InstructionError::InvalidAccountOwner);
    }
    let state_of_next_version = get_state(next_version.get_data())?;
    if state_of_next_version.authority_address_or_next_version != *authority_address {
        // ic_logger_msg!(log_collector, "Next version has a different authority");
        return Err(InstructionError::IncorrectAuthority);
    }
    if matches!(state_of_next_version.status, LoaderV4Status::Finalized) {
        // ic_logger_msg!(log_collector, "Next version is finalized");
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
    // let log_collector = invoke_context.get_log_collector();
    debug_log!("loader_v4.process_instruction_inner1");
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let instruction_data = instruction_context.get_instruction_data();
    let program_id = instruction_context.get_last_program_key(transaction_context)?;
    debug_log!(
        "loader_v4.process_instruction_inner2 program_id {}",
        program_id
    );
    if loader_v4::check_id(program_id) {
        invoke_context.consume_checked(DEFAULT_COMPUTE_UNITS)?;
        debug_log!(
            "loader_v4.process_instruction_inner3 instruction_data {:x?}",
            instruction_data
        );
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
        debug_log!("instruction_data {:x?}", instruction_data);
        if !loader_v4::check_id(program_owner) {
            debug_log!("Program not owned by loader");
            return Err(Box::new(InstructionError::InvalidAccountOwner));
        }
        if program.get_data().is_empty() {
            debug_log!("Program is uninitialized");
            return Err(Box::new(InstructionError::InvalidAccountData));
        }
        let state = get_state(program.get_data())?;
        if matches!(state.status, LoaderV4Status::Retracted) {
            debug_log!("Program is not deployed");
            return Err(Box::new(InstructionError::InvalidArgument));
        }
        debug_log!();
        // let mut get_or_create_executor_time = Measure::start("get_or_create_executor_time");
        let loaded_program = invoke_context
            .program_cache_for_tx_batch
            .find(program.get_key())
            .ok_or_else(|| {
                debug_log!("Program is not cached");
                InstructionError::InvalidAccountData
            })?;
        debug_log!();
        // get_or_create_executor_time.stop();
        // saturating_add_assign!(
        //     invoke_context.timings.get_or_create_executor_us,
        //     get_or_create_executor_time.as_us()
        // );
        drop(program);
        // loaded_program
        //     .ix_usage_counter
        //     .fetch_add(1, Ordering::Relaxed);
        let loaded_program = &loaded_program.program;
        // let executor_program_ref = executor_program.as_ref();
        match loaded_program {
            ProgramCacheEntryType::FailedVerification(_)
            | ProgramCacheEntryType::Closed
            | ProgramCacheEntryType::DelayVisibility => {
                // ic_logger_msg!(log_collector, "Program is not deployed");
                Err(Box::new(InstructionError::UnsupportedProgramId) as Box<dyn core::error::Error>)
            }
            ProgramCacheEntryType::Loaded(executable) => {
                debug_log!();
                execute(executable.clone(), invoke_context)
            }
            _ => {
                Err(Box::new(InstructionError::UnsupportedProgramId) as Box<dyn core::error::Error>)
            }
        }
    }
    .map(|_| 0)
}
