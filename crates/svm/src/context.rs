use crate::{
    account::{AccountSharedData, BorrowedAccount, ReadableAccount},
    account_utils::StateMut,
    bpf_loader,
    bpf_loader_deprecated,
    common::{check_loader_id, load_program_from_bytes},
    helpers::SyscallContext,
    loaded_programs::{
        // LoadedProgram,
        // LoadedProgramType,
        // LoadedProgramsForTxBatch,
        ProgramRuntimeEnvironments,
    },
    loaders::bpf_loader_v4,
    native_loader,
    solana_program::{
        bpf_loader_upgradeable,
        bpf_loader_upgradeable::UpgradeableLoaderState,
        loader_v4,
        loader_v4::{LoaderV4State, LoaderV4Status},
    },
    sysvar_cache::SysvarCache,
};
use crate::{
    // bpf_loader_upgradeable,
    // bpf_loader_upgradeable::UpgradeableLoaderState,
    clock::Slot,
    hash::Hash,
    rent::Rent,
};
use crate::{
    compute_budget::compute_budget::ComputeBudget,
    error::SvmError,
    helpers::{storage_read_account_data, AllocErr, LogCollector},
    loaded_programs::{
        ProgramCacheEntry,
        ProgramCacheEntryOwner,
        ProgramCacheEntryType,
        ProgramCacheForTxBatch,
    },
    precompiles::Precompile,
};
use alloc::{boxed::Box, rc::Rc, sync::Arc, vec, vec::Vec};
use core::{
    alloc::Layout,
    cell::{Ref, RefCell, RefMut},
    pin::Pin,
    sync::atomic::Ordering,
};
use fluentbase_sdk::{debug_log, HashSet, SharedAPI};
use solana_feature_set::{move_precompile_verification_to_svm, FeatureSet};
use solana_instruction::error::InstructionError;
use solana_pubkey::Pubkey;
use solana_rbpf::{
    ebpf::MM_HEAP_START,
    error::{EbpfError, ProgramResult},
    memory_region::MemoryMapping,
    program::{BuiltinFunction, SBPFVersion},
    static_analysis::TraceLogEntry,
    vm::{Config, ContextObject, EbpfVm},
};
use solana_stable_layout::stable_instruction::StableInstruction;

/// Index of an account inside of the TransactionContext or an InstructionContext.
pub type IndexOfAccount = u16;

pub type BuiltinFunctionWithContext<'a, SDK: SharedAPI> = BuiltinFunction<InvokeContext<'a, SDK>>;

/// Contains account meta data which varies between instruction.
///
/// It also contains indices to other structures for faster lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstructionAccount {
    /// Points to the account and its key in the `TransactionContext`
    pub index_in_transaction: IndexOfAccount,
    /// Points to the first occurrence in the parent `InstructionContext`
    ///
    /// This excludes the program accounts.
    pub index_in_caller: IndexOfAccount,
    /// Points to the first occurrence in the current `InstructionContext`
    ///
    /// This excludes the program accounts.
    pub index_in_callee: IndexOfAccount,
    /// Is this account supposed to sign
    pub is_signer: bool,
    /// Is this account allowed to become writable
    pub is_writable: bool,
}

/// An account key and the matching account
pub type TransactionAccount = (Pubkey, AccountSharedData);

// #[derive(Debug)]
pub enum ProgramAccountLoadResult {
    InvalidAccountData(ProgramCacheEntryOwner),
    ProgramOfLoaderV1(AccountSharedData),
    ProgramOfLoaderV2(AccountSharedData),
    ProgramOfLoaderV3(AccountSharedData, AccountSharedData, Slot),
    ProgramOfLoaderV4(AccountSharedData, Slot),
}

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

pub struct EnvironmentConfig {
    pub blockhash: Hash,
    epoch_total_stake: Option<u64>,
    // epoch_vote_accounts: Option<&'a VoteAccountsHashMap>,
    pub feature_set: Arc<FeatureSet>,
    pub lamports_per_signature: u64,
    sysvar_cache: SysvarCache,
}
impl<'a> EnvironmentConfig {
    pub fn new(
        blockhash: Hash,
        epoch_total_stake: Option<u64>,
        // epoch_vote_accounts: Option<&'a VoteAccountsHashMap>,
        feature_set: Arc<FeatureSet>,
        lamports_per_signature: u64,
        sysvar_cache: SysvarCache,
    ) -> Self {
        Self {
            blockhash,
            epoch_total_stake,
            // epoch_vote_accounts,
            feature_set,
            lamports_per_signature,
            sysvar_cache,
        }
    }
}

// #[derive(Debug)]
pub struct InvokeContext<'a, SDK: SharedAPI> {
    pub transaction_context: TransactionContext,
    // sysvar_cache: SysvarCache,
    // pub programs_loaded_for_tx_batch: LoadedProgramsForTxBatch<'a, SDK>,
    // pub programs_modified_by_tx: LoadedProgramsForTxBatch<'a, SDK>,
    /// The local program cache for the transaction batch.
    pub program_cache_for_tx_batch: ProgramCacheForTxBatch<'a, SDK>,
    /// Runtime configurations used to provision the invocation environment.
    pub environment_config: EnvironmentConfig,
    pub compute_budget: ComputeBudget,
    pub compute_meter: RefCell<u64>,
    // log_collector: Option<Rc<RefCell<LogCollector>>>,
    // pub trace_log: Vec<TraceLogEntry>,
    // pub execute_time: Option<Measure>,
    // pub timings: ExecuteDetailsTimings,
    pub syscall_context: Vec<Option<SyscallContext>>,
    traces: Vec<Vec<[u64; 12]>>,
    // pub remaining: u64,
    // pub syscall_context: Vec<Option<SyscallContext>>,
    // pub feature_set: Arc<FeatureSet>,
    // pub blockhash: Hash,
    // pub lamports_per_signature: u64,
    // pub current_compute_budget: ComputeBudget,
    pub sdk: &'a SDK,
}

impl<'a, SDK: SharedAPI> InvokeContext<'a, SDK> {
    // /// Initialize with instruction meter
    // pub fn new(
    //     transaction_context: TransactionContext,
    //     sysvar_cache: SysvarCache,
    //     sdk: &'a SDK,
    //     compute_budget: ComputeBudget,
    //     programs_loaded_for_tx_batch: LoadedProgramsForTxBatch<'a, SDK>,
    //     programs_modified_by_tx: LoadedProgramsForTxBatch<'a, SDK>,
    //     feature_set: Arc<FeatureSet>,
    //     blockhash: Hash,
    //     lamports_per_signature: u64,
    // ) -> Self {
    //     Self {
    //         transaction_context,
    //         sysvar_cache,
    //         sdk,
    //         programs_loaded_for_tx_batch,
    //         programs_modified_by_tx,
    //         feature_set,
    //         compute_budget,
    //         blockhash,
    //         lamports_per_signature,
    //         compute_meter: RefCell::new(compute_budget.compute_unit_limit),
    //         current_compute_budget: compute_budget,
    //         syscall_context: Default::default(),
    //         trace_log: Default::default(),
    //     }
    // }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transaction_context: TransactionContext,
        program_cache_for_tx_batch: ProgramCacheForTxBatch<'a, SDK>,
        environment_config: EnvironmentConfig,
        // log_collector: Option<Rc<RefCell<LogCollector>>>,
        compute_budget: ComputeBudget,
        // sysvar_cache: SysvarCache,
        sdk: &'a SDK,
    ) -> Self {
        Self {
            transaction_context,
            program_cache_for_tx_batch,
            environment_config,
            // log_collector,
            compute_budget,
            compute_meter: RefCell::new(compute_budget.compute_unit_limit),
            // execute_time: None,
            // timings: ExecuteDetailsTimings::default(),
            syscall_context: Vec::new(),
            traces: Vec::new(),
            sdk,
            // sysvar_cache,
            // trace_log,
        }
    }

    pub fn get_environments_for_slot(
        &self,
        effective_slot: Slot,
    ) -> Result<&ProgramRuntimeEnvironments<'a, SDK>, InstructionError> {
        let epoch_schedule = self.environment_config.sysvar_cache.get_epoch_schedule()?;
        let epoch = epoch_schedule.get_epoch(effective_slot);
        Ok(self
            .program_cache_for_tx_batch
            .get_environments_for_epoch(epoch))
    }

    // pub fn get_environments_for_slot(
    //     &self,
    //     effective_slot: Slot,
    // ) -> Result<&ProgramRuntimeEnvironments<'a, SDK>, InstructionError> {
    //     let epoch_schedule = self.sysvar_cache.get_epoch_schedule()?;
    //     let epoch = epoch_schedule.get_epoch(effective_slot);
    //     Ok(self
    //         .programs_loaded_for_tx_batch
    //         .get_environments_for_epoch(epoch))
    // }

    /// Push a stack frame onto the invocation stack
    pub fn push(&mut self) -> Result<(), InstructionError> {
        let instruction_context = self
            .transaction_context
            .get_instruction_context_at_index_in_trace(
                self.transaction_context.get_instruction_trace_length(),
            )?;
        let program_id = instruction_context
            .get_last_program_key(&self.transaction_context)
            .map_err(|_| InstructionError::UnsupportedProgramId)?;
        if self
            .transaction_context
            .get_instruction_context_stack_height()
            != 0
        {
            let contains = (0..self
                .transaction_context
                .get_instruction_context_stack_height())
                .any(|level| {
                    self.transaction_context
                        .get_instruction_context_at_nesting_level(level)
                        .and_then(|instruction_context| {
                            instruction_context
                                .try_borrow_last_program_account(&self.transaction_context)
                        })
                        .map(|program_account| program_account.get_key() == program_id)
                        .unwrap_or(false)
                });
            let is_last = self
                .transaction_context
                .get_current_instruction_context()
                .and_then(|instruction_context| {
                    instruction_context.try_borrow_last_program_account(&self.transaction_context)
                })
                .map(|program_account| program_account.get_key() == program_id)
                .unwrap_or(false);
            if contains && !is_last {
                // Reentrancy not allowed unless caller is calling itself
                return Err(InstructionError::ReentrancyNotAllowed);
            }
        }

        self.syscall_context.push(None);
        self.transaction_context.push()
    }

    /// Pop a stack frame from the invocation stack
    pub fn pop(&mut self) -> Result<(), InstructionError> {
        if let Some(Some(syscall_context)) = self.syscall_context.pop() {
            self.traces.push(syscall_context.trace_log);
        }
        self.transaction_context.pop()
    }

    /// Current height of the invocation stack, top level instructions are height
    /// `solana_sdk::instruction::TRANSACTION_LEVEL_STACK_HEIGHT`
    pub fn get_stack_height(&self) -> usize {
        self.transaction_context
            .get_instruction_context_stack_height()
    }

    // /// Entrypoint for a cross-program invocation from a builtin program
    // pub fn native_invoke(
    //     &mut self,
    //     instruction: StableInstruction,
    //     signers: &[Pubkey],
    // ) -> Result<(), InstructionError> {
    //     let (instruction_accounts, program_indices) =
    //         self.prepare_instruction(&instruction, signers)?;
    //     let mut compute_units_consumed = 0;
    //     self.process_instruction(
    //         &instruction.data,
    //         &instruction_accounts,
    //         &program_indices,
    //         &mut compute_units_consumed,
    //         // &mut ExecuteTimings::default(),
    //     )?;
    //     Ok(())
    // }

    /// Helper to prepare for process_instruction()
    #[allow(clippy::type_complexity)]
    pub fn prepare_instruction(
        &mut self,
        instruction: &StableInstruction,
        signers: &[Pubkey],
    ) -> Result<(Vec<InstructionAccount>, Vec<IndexOfAccount>), InstructionError> {
        // Finds the index of each account in the instruction by its pubkey.
        // Then normalizes / unifies the privileges of duplicate accounts.
        // Note: This is an O(n^2) algorithm,
        // but performed on a very small slice and requires no heap allocations.
        let instruction_context = self.transaction_context.get_current_instruction_context()?;
        let mut deduplicated_instruction_accounts: Vec<InstructionAccount> = Vec::new();
        let mut duplicate_indicies = Vec::with_capacity(instruction.accounts.len());
        for (instruction_account_index, account_meta) in instruction.accounts.iter().enumerate() {
            let index_in_transaction = self
                .transaction_context
                .find_index_of_account(&account_meta.pubkey)
                .ok_or_else(|| {
                    // ic_msg!(
                    //     self,
                    //     "Instruction references an unknown account {}",
                    //     account_meta.pubkey,
                    // );
                    InstructionError::MissingAccount
                })?;
            if let Some(duplicate_index) =
                deduplicated_instruction_accounts
                    .iter()
                    .position(|instruction_account| {
                        instruction_account.index_in_transaction == index_in_transaction
                    })
            {
                duplicate_indicies.push(duplicate_index);
                let instruction_account = deduplicated_instruction_accounts
                    .get_mut(duplicate_index)
                    .ok_or(InstructionError::NotEnoughAccountKeys)?;
                instruction_account.is_signer |= account_meta.is_signer;
                instruction_account.is_writable |= account_meta.is_writable;
            } else {
                let index_in_caller = instruction_context
                    .find_index_of_instruction_account(
                        &self.transaction_context,
                        &account_meta.pubkey,
                    )
                    .ok_or_else(|| {
                        // ic_msg!(
                        //     self,
                        //     "Instruction references an unknown account {}",
                        //     account_meta.pubkey,
                        // );
                        InstructionError::MissingAccount
                    })?;
                duplicate_indicies.push(deduplicated_instruction_accounts.len());
                deduplicated_instruction_accounts.push(InstructionAccount {
                    index_in_transaction,
                    index_in_caller,
                    index_in_callee: instruction_account_index as IndexOfAccount,
                    is_signer: account_meta.is_signer,
                    is_writable: account_meta.is_writable,
                });
            }
        }
        for instruction_account in deduplicated_instruction_accounts.iter() {
            let borrowed_account = instruction_context.try_borrow_instruction_account(
                &self.transaction_context,
                instruction_account.index_in_caller,
            )?;

            // Readonly in caller cannot become writable in callee
            if instruction_account.is_writable && !borrowed_account.is_writable() {
                // ic_msg!(
                //     self,
                //     "{}'s writable privilege escalated",
                //     borrowed_account.get_key(),
                // );
                return Err(InstructionError::PrivilegeEscalation);
            }

            // To be signed in the callee,
            // it must be either signed in the caller or by the program
            let borrowed_account_key = borrowed_account.get_key();
            if instruction_account.is_signer
                && !(borrowed_account.is_signer() || signers.contains(borrowed_account.get_key()))
            {
                // ic_msg!(
                //     self,
                //     "{}'s signer privilege escalated",
                //     borrowed_account.get_key()
                // );
                return Err(InstructionError::PrivilegeEscalation);
            }
        }
        let instruction_accounts = duplicate_indicies
            .into_iter()
            .map(|duplicate_index| {
                deduplicated_instruction_accounts
                    .get(duplicate_index)
                    .cloned()
                    .ok_or(InstructionError::NotEnoughAccountKeys)
            })
            .collect::<Result<Vec<InstructionAccount>, InstructionError>>()?;

        // Find and validate executables / program accounts
        let callee_program_id = instruction.program_id;
        let program_account_index = instruction_context
            .find_index_of_instruction_account(&self.transaction_context, &callee_program_id)
            .ok_or_else(|| {
                // ic_msg!(self, "Unknown program {}", callee_program_id);
                InstructionError::MissingAccount
            })?;
        let borrowed_program_account = instruction_context
            .try_borrow_instruction_account(&self.transaction_context, program_account_index)?;
        if !borrowed_program_account.is_executable() {
            // ic_msg!(self, "Account {} is not executable", callee_program_id);
            return Err(InstructionError::AccountNotExecutable);
        }

        Ok((
            instruction_accounts,
            vec![borrowed_program_account.get_index_in_transaction()],
        ))
    }

    /// Processes an instruction and returns how many compute units were used
    pub fn process_instruction(
        &mut self,
        instruction_data: &[u8],
        instruction_accounts: &[InstructionAccount],
        program_indices: &[IndexOfAccount],
        // compute_units_consumed: &mut u64,
        // timings: &mut ExecuteTimings,
    ) -> Result<(), InstructionError> {
        // *compute_units_consumed = 0;
        debug_log!("invoke_context.process_instruction1");
        self.transaction_context
            .get_next_instruction_context()?
            .configure(program_indices, instruction_accounts, instruction_data);
        debug_log!("invoke_context.process_instruction2");
        self.push()?;
        debug_log!("invoke_context.process_instruction3");
        let result = self.process_executable_chain(/*compute_units_consumed , timings*/)
            // MUST pop if and only if `push` succeeded, independent of `result`.
            // Thus, the `.and()` instead of an `.and_then()`.
            .and(self.pop());
        debug_log!("invoke_context.process_instruction4: {:?}", result);
        result
    }

    /// Processes a precompile instruction
    pub fn process_precompile<'ix_data>(
        &mut self,
        precompile: &Precompile,
        instruction_data: &[u8],
        instruction_accounts: &[InstructionAccount],
        program_indices: &[IndexOfAccount],
        message_instruction_datas_iter: impl Iterator<Item = &'ix_data [u8]>,
    ) -> Result<(), InstructionError> {
        self.transaction_context
            .get_next_instruction_context()?
            .configure(program_indices, instruction_accounts, instruction_data);
        self.push()?;

        let feature_set = self.get_feature_set();
        let move_precompile_verification_to_svm =
            feature_set.is_active(&move_precompile_verification_to_svm::id());
        if move_precompile_verification_to_svm {
            let instruction_datas: Vec<_> = message_instruction_datas_iter.collect();
            precompile
                .verify(instruction_data, &instruction_datas, feature_set)
                .map_err(InstructionError::from)
                .and(self.pop())
        } else {
            self.pop()
        }
    }

    /// Calls the instruction's program entrypoint method
    fn process_executable_chain(
        &mut self,
        // compute_units_consumed: &mut u64,
        // timings: &mut ExecuteTimings,
    ) -> Result<(), InstructionError> {
        debug_log!("invoke_context.process_executable_chain1");
        let instruction_context = self.transaction_context.get_current_instruction_context()?;
        // let process_executable_chain_time = Measure::start("process_executable_chain_time");

        let builtin_id = {
            // TODO Stanislav: need this check?
            // debug_assert!(instruction_context.get_number_of_program_accounts() <= 1);
            let borrowed_root_account = instruction_context
                .try_borrow_program_account(&self.transaction_context, 0)
                .map_err(|_| InstructionError::UnsupportedProgramId)?;
            debug_log!("invoke_context.process_executable_chain2");
            let owner_id = borrowed_root_account.get_owner();
            if native_loader::check_id(owner_id) {
                *borrowed_root_account.get_key()
            } else {
                *owner_id
            }
        };

        // The Murmur3 hash value (used by RBPF) of the string "entrypoint"
        const ENTRYPOINT_KEY: u32 = 0x71E3CF81;
        let entry = self
            .program_cache_for_tx_batch
            .find(&builtin_id)
            .ok_or(InstructionError::UnsupportedProgramId)?;
        debug_log!("invoke_context.process_executable_chain3");
        let function = match &entry.program {
            ProgramCacheEntryType::Builtin(program) => program
                .get_function_registry()
                .lookup_by_key(ENTRYPOINT_KEY)
                .map(|(_name, function)| function),
            _ => None,
        }
        .ok_or(InstructionError::UnsupportedProgramId)?;
        debug_log!("invoke_context.process_executable_chain4");
        entry.ix_usage_counter.fetch_add(1, Ordering::Relaxed);

        let program_id = *instruction_context.get_last_program_key(&self.transaction_context)?;
        debug_log!("invoke_context.process_executable_chain5");
        self.transaction_context
            .set_return_data(program_id, Vec::new())?;
        debug_log!("invoke_context.process_executable_chain6");
        // let logger = self.get_log_collector();
        // stable_log::program_invoke(&logger, &program_id, self.get_stack_height());
        // let pre_remaining_units = self.get_remaining();
        // In program-runtime v2 we will create this VM instance only once per transaction.
        // `program_runtime_environment_v2.get_config()` will be used instead of `mock_config`.
        // For now, only built-ins are invoked from here, so the VM and its Config are irrelevant.
        let mock_config = Config::default();
        let empty_memory_mapping =
            MemoryMapping::new(Vec::new(), &mock_config, &SBPFVersion::V1).unwrap();
        debug_log!("invoke_context.process_executable_chain7");
        let mut vm = EbpfVm::new(
            self.program_cache_for_tx_batch
                .environments
                .program_runtime_v2
                .clone(),
            &SBPFVersion::V1,
            // Removes lifetime tracking
            unsafe {
                core::mem::transmute::<&mut InvokeContext<SDK>, &mut InvokeContext<SDK>>(self)
            },
            empty_memory_mapping,
            0,
        );
        debug_log!("invoke_context.process_executable_chain8");
        vm.invoke_function(function);
        debug_log!("invoke_context.process_executable_chain9");
        let result = match vm.program_result {
            ProgramResult::Ok(_) => {
                // stable_log::program_success(&logger, &program_id);
                Ok(())
            }
            ProgramResult::Err(ref err) => {
                if let EbpfError::SyscallError(syscall_error) = err {
                    if let Some(instruction_err) = syscall_error.downcast_ref::<InstructionError>()
                    {
                        // stable_log::program_failure(&logger, &program_id, instruction_err);
                        Err(instruction_err.clone())
                    } else {
                        // stable_log::program_failure(&logger, &program_id, syscall_error);
                        Err(InstructionError::ProgramFailedToComplete)
                    }
                } else {
                    // stable_log::program_failure(&logger, &program_id, err);
                    Err(InstructionError::ProgramFailedToComplete)
                }
            }
        };
        debug_log!("invoke_context.process_executable_chain10");
        // let post_remaining_units = self.get_remaining();
        // *compute_units_consumed = pre_remaining_units.saturating_sub(post_remaining_units);

        // if builtin_id == program_id && result.is_ok() && *compute_units_consumed == 0 {
        //     return Err(InstructionError::BuiltinProgramsMustConsumeComputeUnits);
        // }

        // saturating_add_assign!(
        //     timings
        //         .execute_accessories
        //         .process_instructions
        //         .process_executable_chain_us,
        //     process_executable_chain_time.end_as_us()
        // );
        result
    }

    // /// Get this invocation's LogCollector
    // pub fn get_log_collector(&self) -> Option<Rc<RefCell<LogCollector>>> {
    //     self.log_collector.clone()
    // }

    /// Consume compute units
    pub fn consume_checked(&self, amount: u64) -> Result<(), Box<dyn core::error::Error>> {
        let mut compute_meter = self.compute_meter.borrow_mut();
        let exceeded = *compute_meter < amount;
        *compute_meter = compute_meter.saturating_sub(amount);
        if exceeded {
            return Err(Box::new(InstructionError::ComputationalBudgetExceeded));
        }
        Ok(())
    }

    /// Set compute units
    ///
    /// Only use for tests and benchmarks
    pub fn mock_set_remaining(&self, remaining: u64) {
        *self.compute_meter.borrow_mut() = remaining;
    }

    /// Get this invocation's compute budget
    pub fn get_compute_budget(&self) -> &ComputeBudget {
        &self.compute_budget
    }

    /// Get the current feature set.
    pub fn get_feature_set(&self) -> &FeatureSet {
        &self.environment_config.feature_set
    }

    /// Set feature set.
    ///
    /// Only use for tests and benchmarks.
    pub fn mock_set_feature_set(&mut self, feature_set: Arc<FeatureSet>) {
        self.environment_config.feature_set = feature_set;
    }

    /// Get cached sysvars
    pub fn get_sysvar_cache(&self) -> &SysvarCache {
        &self.environment_config.sysvar_cache
    }

    /// Get cached epoch total stake.
    pub fn get_epoch_total_stake(&self) -> Option<u64> {
        self.environment_config.epoch_total_stake
    }

    // /// Get cached epoch vote accounts.
    // pub fn get_epoch_vote_accounts(&self) -> Option<&VoteAccountsHashMap> {
    //     self.environment_config.epoch_vote_accounts
    // }

    // Should alignment be enforced during user pointer translation
    pub fn get_check_aligned(&self) -> bool {
        self.transaction_context
            .get_current_instruction_context()
            .and_then(|instruction_context| {
                let program_account =
                    instruction_context.try_borrow_last_program_account(&self.transaction_context);
                debug_assert!(program_account.is_ok());
                program_account
            })
            .map(|program_account| *program_account.get_owner() != bpf_loader_deprecated::id())
            .unwrap_or(true)
    }

    // Set this instruction syscall context
    pub fn set_syscall_context(
        &mut self,
        syscall_context: SyscallContext,
    ) -> Result<(), InstructionError> {
        *self
            .syscall_context
            .last_mut()
            .ok_or(InstructionError::CallDepth)? = Some(syscall_context);
        Ok(())
    }

    // Get this instruction's SyscallContext
    pub fn get_syscall_context(&self) -> Result<&SyscallContext, InstructionError> {
        self.syscall_context
            .last()
            .and_then(core::option::Option::as_ref)
            .ok_or(InstructionError::CallDepth)
    }

    // Get this instruction's SyscallContext
    pub fn get_syscall_context_mut(&mut self) -> Result<&mut SyscallContext, InstructionError> {
        self.syscall_context
            .last_mut()
            .and_then(|syscall_context| syscall_context.as_mut())
            .ok_or(InstructionError::CallDepth)
    }

    /// Return a references to traces
    pub fn get_traces(&self) -> &Vec<Vec<[u64; 12]>> {
        &self.traces
    }

    // pub fn try_get_common_slot(&mut self) -> Option<Slot> {
    //     let slot = self.programs_loaded_for_tx_batch.slot();
    //     if slot != self.programs_modified_by_tx.slot() {
    //         return None;
    //     };
    //     Some(slot)
    // }

    pub fn inc_slots(&mut self, step: u64) {
        self.program_cache_for_tx_batch
            .set_slot_for_tests(self.program_cache_for_tx_batch.slot().saturating_add(step));
    }

    pub fn set_slot(&mut self, slot: Slot) {
        self.program_cache_for_tx_batch.set_slot_for_tests(slot);
    }

    // pub fn find_program_in_cache(&self, pubkey: &Pubkey) -> Option<Arc<LoadedProgram<'a, SDK>>> {
    //     // First lookup the cache of the programs modified by the current transaction. If not found, lookup
    //     // the cache of the cache of the programs that are loaded for the transaction batch.
    //     let r = self.programs_modified_by_tx.find(pubkey);
    //     let r = r.or_else(|| self.programs_loaded_for_tx_batch.find(pubkey));
    //     r
    // }

    /// Entrypoint for a cross-program invocation from a builtin program
    pub fn native_invoke(
        &mut self,
        instruction: StableInstruction,
        signers: &[Pubkey],
    ) -> Result<(), InstructionError> {
        let (instruction_accounts, program_indices) =
            self.prepare_instruction(&instruction, signers)?;
        // let mut compute_units_consumed = 0;
        self.process_instruction(
            &instruction.data,
            &instruction_accounts,
            &program_indices,
            // &mut compute_units_consumed,
            // &mut ExecuteTimings::default(),
        )?;
        Ok(())
    }

    // pub fn get_check_aligned(&self) -> bool {
    //     true
    // }

    // /// Helper to prepare for process_instruction()
    // #[allow(clippy::type_complexity)]
    // pub fn prepare_instruction(
    //     &mut self,
    //     instruction: &StableInstruction,
    //     signers: &[Pubkey],
    // ) -> Result<(Vec<InstructionAccount>, Vec<IndexOfAccount>), InstructionError> {
    //     // Finds the index of each account in the instruction by its pubkey.
    //     // Then normalizes / unifies the privileges of duplicate accounts.
    //     // Note: This is an O(n^2) algorithm,
    //     // but performed on a very small slice and requires no heap allocations.
    //     let instruction_context = self.transaction_context.get_current_instruction_context()?;
    //     let mut deduplicated_instruction_accounts: Vec<InstructionAccount> = Vec::new();
    //     let mut duplicate_indices = Vec::with_capacity(instruction.accounts.len());
    //     for (instruction_account_index, account_meta) in instruction.accounts.iter().enumerate() {
    //         let index_in_transaction = self
    //             .transaction_context
    //             .find_index_of_account(&account_meta.pubkey)
    //             .ok_or_else(|| {
    //                 // ic_msg!(
    //                 //     self,
    //                 //     "Instruction references an unknown account {}",
    //                 //     account_meta.pubkey,
    //                 // );
    //                 InstructionError::MissingAccount
    //             })?;
    //         if let Some(duplicate_index) =
    //             deduplicated_instruction_accounts
    //                 .iter()
    //                 .position(|instruction_account| {
    //                     instruction_account.index_in_transaction == index_in_transaction
    //                 })
    //         {
    //             duplicate_indices.push(duplicate_index);
    //             let instruction_account = deduplicated_instruction_accounts
    //                 .get_mut(duplicate_index)
    //                 .ok_or(InstructionError::NotEnoughAccountKeys)?;
    //             instruction_account.is_signer |= account_meta.is_signer;
    //             instruction_account.is_writable |= account_meta.is_writable;
    //         } else {
    //             let index_in_caller = instruction_context
    //                 .find_index_of_instruction_account(
    //                     &self.transaction_context,
    //                     &account_meta.pubkey,
    //                 )
    //                 .ok_or_else(|| {
    //                     // ic_msg!(
    //                     //     self,
    //                     //     "Instruction references an unknown account {}",
    //                     //     account_meta.pubkey,
    //                     // );
    //                     InstructionError::MissingAccount
    //                 })?;
    //             duplicate_indices.push(deduplicated_instruction_accounts.len());
    //             deduplicated_instruction_accounts.push(InstructionAccount {
    //                 index_in_transaction,
    //                 index_in_caller,
    //                 index_in_callee: instruction_account_index as IndexOfAccount,
    //                 is_signer: account_meta.is_signer,
    //                 is_writable: account_meta.is_writable,
    //             });
    //         }
    //     }
    //     for instruction_account in deduplicated_instruction_accounts.iter() {
    //         let borrowed_account = instruction_context.try_borrow_instruction_account(
    //             &self.transaction_context,
    //             instruction_account.index_in_caller,
    //         )?;
    //
    //         // Readonly in caller cannot become writable in callee
    //         if instruction_account.is_writable && !borrowed_account.is_writable() {
    //             // ic_msg!(
    //             //     self,
    //             //     "{}'s writable privilege escalated",
    //             //     borrowed_account.get_key(),
    //             // );
    //             return Err(InstructionError::PrivilegeEscalation);
    //         }
    //
    //         // To be signed in the callee,
    //         // it must be either signed in the caller or by the program
    //         if instruction_account.is_signer
    //             && !(borrowed_account.is_signer() || signers.contains(borrowed_account.get_key()))
    //         {
    //             // ic_msg!(
    //             //     self,
    //             //     "{}'s signer privilege escalated",
    //             //     borrowed_account.get_key()
    //             // );
    //             return Err(InstructionError::PrivilegeEscalation);
    //         }
    //     }
    //     let instruction_accounts = duplicate_indices
    //         .into_iter()
    //         .map(|duplicate_index| {
    //             Ok(deduplicated_instruction_accounts
    //                 .get(duplicate_index)
    //                 .ok_or(InstructionError::NotEnoughAccountKeys)?
    //                 .clone())
    //         })
    //         .collect::<Result<Vec<InstructionAccount>, InstructionError>>()?;
    //
    //     // Find and validate executables / program accounts
    //     let callee_program_id = instruction.program_id;
    //     let program_account_index = instruction_context
    //         .find_index_of_instruction_account(&self.transaction_context, &callee_program_id)
    //         .ok_or_else(|| {
    //             // ic_msg!(self, "Unknown program {}", callee_program_id);
    //             InstructionError::MissingAccount
    //         })?;
    //     let borrowed_program_account = instruction_context
    //         .try_borrow_instruction_account(&self.transaction_context, program_account_index)?;
    //     if !borrowed_program_account.is_executable() {
    //         // ic_msg!(self, "Account {} is not executable", callee_program_id);
    //         return Err(InstructionError::AccountNotExecutable);
    //     }
    //
    //     Ok((
    //         instruction_accounts,
    //         vec![borrowed_program_account.get_index_in_transaction()],
    //     ))
    // }
    //
    // /// Processes an instruction and returns how many compute units were used
    // pub fn process_instruction(
    //     &mut self,
    //     instruction_data: &[u8],
    //     instruction_accounts: &[InstructionAccount],
    //     program_indices: &[IndexOfAccount],
    //     // compute_units_consumed: &mut u64,
    // ) -> Result<(), InstructionError> {
    //     // *compute_units_consumed = 0;
    //     self.transaction_context
    //         .get_next_instruction_context()?
    //         .configure(program_indices, instruction_accounts, instruction_data);
    //     let push_result = self.push();
    //     push_result?;
    //     self.process_executable_chain(/*compute_units_consumed, timings*/)
    //         // MUST pop if and only if `push` succeeded, independent of `result`.
    //         // Thus, the `.and()` instead of an `.and_then()`.
    //         .and(self.pop())
    // }
    //
    // /// Calls the instruction's program entrypoint method
    // fn process_executable_chain(
    //     &mut self,
    //     // compute_units_consumed: &mut u64,
    //     // timings: &mut ExecuteTimings,
    // ) -> Result<(), InstructionError> {
    //     let instruction_context = self.transaction_context.get_current_instruction_context()?;
    //     // let mut process_executable_chain_time = Measure::start("process_executable_chain_time");
    //
    //     let builtin_id = {
    //         // debug_assert!(instruction_context.get_number_of_program_accounts() <= 1);
    //         let borrowed_root_account = instruction_context
    //             .try_borrow_program_account(&self.transaction_context, 0)
    //             .map_err(|_| InstructionError::UnsupportedProgramId)?;
    //         let owner_id = borrowed_root_account.get_owner();
    //         if native_loader::check_id(owner_id) {
    //             *borrowed_root_account.get_key()
    //         } else {
    //             *owner_id
    //         }
    //     };
    //
    //     // The Murmur3 hash value (used by RBPF) of the string "entrypoint"
    //     const ENTRYPOINT_KEY: u32 = 0x71E3CF81;
    //     // #[cfg(test)] {
    //     //     println!("builtin_id: {:x?}", builtin_id.to_bytes());
    //     //     let entries = self.programs_loaded_for_tx_batch.entries();
    //     //     for entry in entries {
    //     //         println!("entry.pubkey: {:x?}", entry.0.to_bytes());
    //     //     }
    //     // }
    //     let entry = self
    //         .programs_loaded_for_tx_batch
    //         .find(&builtin_id)
    //         .ok_or(InstructionError::UnsupportedProgramId);
    //     let entry = entry?;
    //     let function = match entry.program.as_ref() {
    //         LoadedProgramType::Builtin(program) => {
    //             let result = program
    //                 .get_function_registry()
    //                 .lookup_by_key(ENTRYPOINT_KEY)
    //                 .map(|(_name, function)| function);
    //             result
    //         }
    //         _ => None,
    //     }
    //     .ok_or(InstructionError::UnsupportedProgramId)?;
    //     entry.ix_usage_counter.fetch_add(1, Ordering::Relaxed);
    //
    //     let program_id = *instruction_context.get_last_program_key(&self.transaction_context)?;
    //     self.transaction_context
    //         .set_return_data(program_id, Vec::new())?;
    //     // let logger = self.get_log_collector();
    //     // stable_log::program_invoke(&logger, &program_id, self.get_stack_height());
    //     // let pre_remaining_units = self.get_remaining();
    //     // In program-runtime v2 we will create this VM instance only once per transaction.
    //     // `program_runtime_environment_v2.get_config()` will be used instead of `mock_config`.
    //     // For now, only built-ins are invoked from here, so the VM and its Config are irrelevant.
    //     let mock_config = Config {
    //         enable_instruction_tracing: false,
    //         reject_broken_elfs: true,
    //         sanitize_user_provided_values: true,
    //         ..Default::default()
    //     };
    //     let empty_memory_mapping =
    //         MemoryMapping::new(Vec::new(), &mock_config, &SBPFVersion::V1).unwrap();
    //     let mut vm = EbpfVm::new(
    //         self.programs_loaded_for_tx_batch
    //             .environments
    //             .program_runtime_v2
    //             .clone(),
    //         &SBPFVersion::V1,
    //         // Removes lifetime tracking
    //         unsafe {
    //             core::mem::transmute::<&mut InvokeContext<SDK>, &mut InvokeContext<SDK>>(self)
    //         },
    //         empty_memory_mapping,
    //         0,
    //     );
    //     vm.invoke_function(function);
    //     let result = match vm.program_result {
    //         ProgramResult::Ok(_) => {
    //             // stable_log::program_success(&logger, &program_id);
    //             Ok(())
    //         }
    //         ProgramResult::Err(ref err) => {
    //             if let EbpfError::SyscallError(syscall_error) = err {
    //                 if let Some(instruction_err) = syscall_error.downcast_ref::<InstructionError>()
    //                 {
    //                     // stable_log::program_failure(&logger, &program_id, instruction_err);
    //                     // #[cfg(test)]
    //                     // println!("Instruction error: {}", instruction_err);
    //                     Err(instruction_err.clone())
    //                 } else {
    //                     // stable_log::program_failure(&logger, &program_id, syscall_error);
    //                     // #[cfg(test)]{
    //                     //     println!("syscall_error: {:?}", syscall_error);
    //                     // }
    //                     Err(InstructionError::ProgramFailedToComplete)
    //                 }
    //             } else {
    //                 // stable_log::program_failure(&logger, &program_id, err);
    //                 Err(InstructionError::ProgramFailedToComplete)
    //             }
    //         }
    //     };
    //     // let post_remaining_units = self.get_remaining();
    //     // *compute_units_consumed = pre_remaining_units.saturating_sub(post_remaining_units);
    //
    //     // if builtin_id == program_id && result.is_ok() && *compute_units_consumed == 0 {
    //     //     return Err(InstructionError::BuiltinProgramsMustConsumeComputeUnits);
    //     // }
    //
    //     // process_executable_chain_time.stop();
    //     // saturating_add_assign!(
    //     //     timings
    //     //         .execute_accessories
    //     //         .process_instructions
    //     //         .process_executable_chain_us,
    //     //     process_executable_chain_time.as_us()
    //     // );
    //     result
    // }
    //
    // /// Consume compute units
    // pub fn consume_checked(&self, amount: u64) -> Result<(), Box<dyn core::error::Error>> {
    //     let mut compute_meter = self.compute_meter.borrow_mut();
    //     let exceeded = *compute_meter < amount;
    //     *compute_meter = compute_meter.saturating_sub(amount);
    //     if exceeded {
    //         return Err(Box::new(InstructionError::ComputationalBudgetExceeded));
    //     }
    //     Ok(())
    // }
    //
    // /// Set compute units
    // ///
    // /// Only use for tests and benchmarks
    // pub fn mock_set_remaining(&self, remaining: u64) {
    //     *self.compute_meter.borrow_mut() = remaining;
    // }
    //
    // /// Get this invocation's compute budget
    // pub fn get_compute_budget(&self) -> &ComputeBudget {
    //     &self.current_compute_budget
    // }
    //
    // /// Get the current feature set.
    // pub fn get_feature_set(&self) -> &FeatureSet {
    //     &self.feature_set
    // }
    //
    // /// Get cached sysvars
    // pub fn get_sysvar_cache(&self) -> &SysvarCache {
    //     &self.sysvar_cache
    // }

    // /// Compares an interpreter trace and a JIT trace.
    // ///
    // /// The log of the JIT can be longer because it only validates the instruction meter at branches.
    // pub fn compare_trace_log(interpreter: &Self, jit: &Self) -> bool {
    //     let interpreter = interpreter.trace_log.as_slice();
    //     let mut jit = jit.trace_log.as_slice();
    //     if jit.len() > interpreter.len() {
    //         jit = &jit[0..interpreter.len()];
    //     }
    //     interpreter == jit
    // }
    //
    // // Set this instruction syscall context
    // pub fn set_syscall_context(
    //     &mut self,
    //     syscall_context: SyscallContext,
    // ) -> Result<(), InstructionError> {
    //     *self
    //         .syscall_context
    //         .last_mut()
    //         .ok_or(InstructionError::CallDepth)? = Some(syscall_context);
    //     Ok(())
    // }
    //
    // // Get this instruction's SyscallContext
    // pub fn get_syscall_context(&self) -> Result<&SyscallContext, InstructionError> {
    //     self.syscall_context
    //         .last()
    //         .and_then(core::option::Option::as_ref)
    //         .ok_or(InstructionError::CallDepth)
    // }

    // /// Push a stack frame onto the invocation stack
    // pub fn push(&mut self) -> Result<(), InstructionError> {
    //     let instruction_context = self
    //         .transaction_context
    //         .get_instruction_context_at_index_in_trace(
    //             self.transaction_context.get_instruction_trace_length(),
    //         )?;
    //     let program_id = instruction_context
    //         .get_last_program_key(&self.transaction_context)
    //         .map_err(|_| InstructionError::UnsupportedProgramId)?;
    //     if self
    //         .transaction_context
    //         .get_instruction_context_stack_height()
    //         == 0
    //     {
    //         // self.current_compute_budget = self.compute_budget;
    //     } else {
    //         let contains = (0..self
    //             .transaction_context
    //             .get_instruction_context_stack_height())
    //             .any(|level| {
    //                 self.transaction_context
    //                     .get_instruction_context_at_nesting_level(level)
    //                     .and_then(|instruction_context| {
    //                         instruction_context
    //                             .try_borrow_last_program_account(&self.transaction_context)
    //                     })
    //                     .map(|program_account| program_account.get_key() == program_id)
    //                     .unwrap_or(false)
    //             });
    //         let is_last = self
    //             .transaction_context
    //             .get_current_instruction_context()
    //             .and_then(|instruction_context| {
    //                 instruction_context.try_borrow_last_program_account(&self.transaction_context)
    //             })
    //             .map(|program_account| program_account.get_key() == program_id)
    //             .unwrap_or(false);
    //         if contains && !is_last {
    //             // Reentrancy not allowed unless caller is calling itself
    //             return Err(InstructionError::ReentrancyNotAllowed);
    //         }
    //     }
    //
    //     self.syscall_context.push(None);
    //     self.transaction_context.push()
    // }

    // /// Pop a stack frame from the invocation stack
    // pub fn pop(&mut self) -> Result<(), InstructionError> {
    //     // if let Some(Some(syscall_context)) = self.syscall_context.pop() {
    //     //     self.traces.push(syscall_context.trace_log);
    //     // }
    //     self.transaction_context.pop()
    // }

    // /// Current height of the invocation stack, top level instructions are height
    // /// `solana_sdk::instruction::TRANSACTION_LEVEL_STACK_HEIGHT`
    // pub fn get_stack_height(&self) -> usize {
    //     self.transaction_context
    //         .get_instruction_context_stack_height()
    // }
}

impl<'a, SDK: SharedAPI> InvokeContext<'a, SDK> {
    pub fn get_accounts_keys(&self) -> Vec<Pubkey> {
        let number_of_accounts = self.transaction_context.get_number_of_accounts();
        let account_keys = (0..number_of_accounts)
            .map(|index| {
                *self
                    .transaction_context
                    .get_key_of_account_at_index(index)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        account_keys
    }

    pub fn get_account_with_fixed_root(&self, pubkey: &Pubkey) -> Option<AccountSharedData> {
        // self.load_slow_with_fixed_root(&self.ancestors, pubkey)
        //     .map(|(acc, _slot)| acc)
        let idx = self.transaction_context.find_index_of_account(pubkey)?;
        self.transaction_context
            .accounts
            .get(idx)
            .and_then(|v| Some(v.borrow().clone()))
    }

    pub fn load_program_accounts(
        // callbacks: &CB,
        &self,
        pubkey: &Pubkey,
    ) -> Option<ProgramAccountLoadResult> {
        // let program_account = callbacks.get_account_shared_data(pubkey)?;
        let program_account = self.get_account_with_fixed_root(pubkey)?;

        if loader_v4::check_id(program_account.owner()) {
            return Some(
                bpf_loader_v4::get_state(program_account.data())
                    .ok()
                    .and_then(|state| {
                        (!matches!(state.status, LoaderV4Status::Retracted)).then_some(state.slot)
                    })
                    .map(|slot| ProgramAccountLoadResult::ProgramOfLoaderV4(program_account, slot))
                    .unwrap_or(ProgramAccountLoadResult::InvalidAccountData(
                        ProgramCacheEntryOwner::LoaderV4,
                    )),
            );
        }

        if bpf_loader_deprecated::check_id(program_account.owner()) {
            return Some(ProgramAccountLoadResult::ProgramOfLoaderV1(program_account));
        }

        if bpf_loader::check_id(program_account.owner()) {
            return Some(ProgramAccountLoadResult::ProgramOfLoaderV2(program_account));
        }

        if let Ok(UpgradeableLoaderState::Program {
            programdata_address,
        }) = program_account.state()
        {
            if let Some(programdata_account) =
                self.get_account_with_fixed_root(&programdata_address)
            {
                if let Ok(UpgradeableLoaderState::ProgramData {
                    slot,
                    upgrade_authority_address: _,
                }) = programdata_account.state()
                {
                    return Some(ProgramAccountLoadResult::ProgramOfLoaderV3(
                        program_account,
                        programdata_account,
                        slot,
                    ));
                }
            }
        }
        Some(ProgramAccountLoadResult::InvalidAccountData(
            ProgramCacheEntryOwner::LoaderV3,
        ))
    }

    // fn load_program_accounts(&self, pubkey: &Pubkey) -> Option<ProgramAccountLoadResult> {
    //     let program_account: AccountSharedData = self.get_account_with_fixed_root(pubkey)?;
    //
    //     debug_assert!(check_loader_id(program_account.owner()));
    //
    //     if loader_v4::check_id(program_account.owner()) {
    //         return Some(
    //             bpf_loader_v4::get_state(program_account.data())
    //                 .ok()
    //                 .and_then(|state| {
    //                     (!matches!(state.status, LoaderV4Status::Retracted)).then_some(state.slot)
    //                 })
    //                 .map(|slot| ProgramAccountLoadResult::ProgramOfLoaderV4(program_account, slot))
    //                 .unwrap_or(ProgramAccountLoadResult::InvalidAccountData),
    //         );
    //     }
    //
    //     if !bpf_loader_upgradeable::check_id(program_account.owner()) {
    //         return Some(ProgramAccountLoadResult::ProgramOfLoaderV1orV2(
    //             program_account,
    //         ));
    //     }
    //
    //     if let Ok(UpgradeableLoaderState::Program {
    //         programdata_address,
    //     }) = program_account.state()
    //     {
    //         if let Some(programdata_account) =
    //             self.get_account_with_fixed_root(&programdata_address)
    //         {
    //             if let Ok(UpgradeableLoaderState::ProgramData {
    //                 slot,
    //                 upgrade_authority_address: _,
    //             }) = programdata_account.state()
    //             {
    //                 return Some(ProgramAccountLoadResult::ProgramOfLoaderV3(
    //                     program_account,
    //                     programdata_account,
    //                     slot,
    //                 ));
    //             }
    //         }
    //     }
    //
    //     Some(ProgramAccountLoadResult::InvalidAccountData)
    // }

    /// Loads the program with the given pubkey.
    ///
    /// If the account doesn't exist it returns `None`. If the account does exist, it must be a program
    /// account (belong to one of the program loaders). Returns `Some(InvalidAccountData)` if the program
    /// account is `Closed`, contains invalid data or any of the programdata accounts are invalid.
    pub fn load_program_with_pubkey(
        &self,
        // callbacks: &CB,
        environments: &ProgramRuntimeEnvironments<'a, SDK>,
        pubkey: &Pubkey,
        slot: Slot,
        // execute_timings: &mut ExecuteTimings,
        reload: bool,
    ) -> Option<Arc<ProgramCacheEntry<'a, SDK>>> {
        // let mut load_program_metrics = LoadProgramMetrics {
        //     program_id: pubkey.to_string(),
        //     ..LoadProgramMetrics::default()
        // };

        let loaded_program = match self.load_program_accounts(pubkey)? {
            ProgramAccountLoadResult::InvalidAccountData(owner) => Ok(
                ProgramCacheEntry::new_tombstone(slot, owner, ProgramCacheEntryType::Closed),
            ),

            ProgramAccountLoadResult::ProgramOfLoaderV1(program_account) => {
                load_program_from_bytes(
                    // &mut load_program_metrics,
                    program_account.data(),
                    program_account.owner(),
                    program_account.data().len(),
                    0,
                    environments.program_runtime_v1.clone(),
                    reload,
                )
                .map_err(|_| (0, ProgramCacheEntryOwner::LoaderV1))
            }

            ProgramAccountLoadResult::ProgramOfLoaderV2(program_account) => {
                load_program_from_bytes(
                    // &mut load_program_metrics,
                    program_account.data(),
                    program_account.owner(),
                    program_account.data().len(),
                    0,
                    environments.program_runtime_v1.clone(),
                    reload,
                )
                .map_err(|_| (0, ProgramCacheEntryOwner::LoaderV2))
            }

            ProgramAccountLoadResult::ProgramOfLoaderV3(
                program_account,
                programdata_account,
                slot,
            ) => programdata_account
                .data()
                .get(UpgradeableLoaderState::size_of_programdata_metadata()..)
                .ok_or(InstructionError::InvalidAccountData)
                .and_then(|programdata| {
                    load_program_from_bytes(
                        // &mut load_program_metrics,
                        programdata,
                        program_account.owner(),
                        program_account
                            .data()
                            .len()
                            .saturating_add(programdata_account.data().len()),
                        slot,
                        environments.program_runtime_v1.clone(),
                        reload,
                    )
                })
                .map_err(|_| (slot, ProgramCacheEntryOwner::LoaderV3)),

            ProgramAccountLoadResult::ProgramOfLoaderV4(program_account, slot) => program_account
                .data()
                .get(LoaderV4State::program_data_offset()..)
                .ok_or(InstructionError::InvalidAccountData)
                .and_then(|elf_bytes| {
                    load_program_from_bytes(
                        // &mut load_program_metrics,
                        elf_bytes,
                        &loader_v4::id(),
                        program_account.data().len(),
                        slot,
                        environments.program_runtime_v2.clone(),
                        reload,
                    )
                })
                .map_err(|_| (slot, ProgramCacheEntryOwner::LoaderV4)),
        }
        .unwrap_or_else(|(slot, owner)| {
            let env = if let ProgramCacheEntryOwner::LoaderV4 = &owner {
                environments.program_runtime_v2.clone()
            } else {
                environments.program_runtime_v1.clone()
            };
            ProgramCacheEntry::new_tombstone(
                slot,
                owner,
                ProgramCacheEntryType::FailedVerification(env),
            )
        });

        // load_program_metrics.submit_datapoint(&mut execute_timings.details);
        loaded_program.update_access_slot(slot);
        Some(Arc::new(loaded_program))
    }

    pub fn load_program(
        &self,
        pubkey: &Pubkey,
        reload: bool,
        // recompile: Option<Arc<LoadedProgram<'a, SDK>>>,
    ) -> Option<Arc<ProgramCacheEntry<'a, SDK>>> {
        // let loaded_programs_cache = self.loaded_programs_cache.read().unwrap();
        // let effective_epoch = if recompile.is_some() {
        //     loaded_programs_cache.latest_root_epoch.saturating_add(1)
        // } else {
        //     self.epoch
        // };
        // let environments = loaded_programs_cache.get_environments_for_epoch(effective_epoch);
        // let mut load_program_metrics = LoadProgramMetrics {
        //     program_id: pubkey.to_string(),
        //     ..LoadProgramMetrics::default()
        // };

        // TODO is it correct to mock slot here
        let slot = Slot::default();
        let envs_for_slot = self.get_environments_for_slot(slot).unwrap();
        let pre_v1 = &envs_for_slot.program_runtime_v1;
        let pre_v2 = &envs_for_slot.program_runtime_v2;

        self.load_program_with_pubkey(envs_for_slot, pubkey, slot, reload)

        // let load_result = self.load_program_accounts(pubkey);
        // let loaded_program = match load_result? {
        //     ProgramAccountLoadResult::InvalidAccountData(owner) => Ok(
        //         ProgramCacheEntry::new_tombstone(slot, owner, ProgramCacheEntryType::Closed),
        //     ),
        //
        //     ProgramAccountLoadResult::ProgramOfLoaderV1orV2(program_account) => {
        //         load_program_from_bytes(
        //             // &mut load_program_metrics,
        //             program_account.data(),
        //             program_account.owner(),
        //             program_account.data().len(),
        //             0,
        //             // environments.program_runtime_v1.clone(),
        //             pre_v1.clone(),
        //             reload,
        //         )
        //         .map_err(|_| (0, pre_v1.clone()))
        //     }
        //
        //     ProgramAccountLoadResult::ProgramOfLoaderV3(
        //         program_account,
        //         programdata_account,
        //         slot,
        //     ) => programdata_account
        //         .data()
        //         .get(UpgradeableLoaderState::size_of_programdata_metadata()..)
        //         // .ok_or(Box::new(InstructionError::InvalidAccountData).into())
        //         .ok_or(InstructionError::InvalidAccountData)
        //         .and_then(|programdata| {
        //             /*Self::*/
        //             load_program_from_bytes(
        //                 // &mut load_program_metrics,
        //                 programdata,
        //                 program_account.owner(),
        //                 program_account
        //                     .data()
        //                     .len()
        //                     .saturating_add(programdata_account.data().len()),
        //                 slot,
        //                 pre_v1.clone(),
        //                 reload,
        //             )
        //         })
        //         .map_err(|_| (slot, pre_v1.clone())),
        //
        //     ProgramAccountLoadResult::ProgramOfLoaderV4(program_account, slot) => program_account
        //         .data()
        //         .get(LoaderV4State::program_data_offset()..)
        //         .ok_or(InstructionError::InvalidAccountData)
        //         .and_then(|elf_bytes| {
        //             /*Self::*/
        //             load_program_from_bytes(
        //                 // &mut load_program_metrics,
        //                 elf_bytes,
        //                 &loader_v4::id(),
        //                 program_account.data().len(),
        //                 slot,
        //                 pre_v2.clone(),
        //                 reload,
        //             )
        //         })
        //         .map_err(|_| (slot, pre_v2.clone())),
        // }
        // .unwrap_or_else(|(slot, env)| {
        //     ProgramCacheEntry::new_tombstone(slot, LoadedProgramType::FailedVerification(env))
        // });

        // let mut timings = ExecuteDetailsTimings::default();
        // load_program_metrics.submit_datapoint(&mut timings);
        // if let Some(recompile) = recompile {
        //     loaded_program.tx_usage_counter =
        //         AtomicU64::new(recompile.tx_usage_counter.load(Ordering::Relaxed));
        //     loaded_program.ix_usage_counter =
        //         AtomicU64::new(recompile.ix_usage_counter.load(Ordering::Relaxed));
        // }
        // loaded_program.update_access_slot(self.slot());
        // Some(Arc::new(loaded_program))
    }
}

impl<'a, SDK: SharedAPI> ContextObject for InvokeContext<'a, SDK> {
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
        let mut compute_meter = self.compute_meter.borrow_mut();
        *compute_meter = compute_meter.saturating_sub(amount);
    }

    fn get_remaining(&self) -> u64 {
        *self.compute_meter.borrow()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransactionAccounts {
    accounts: Vec<RefCell<AccountSharedData>>,
    touched_flags: RefCell<Box<[bool]>>,
}

impl TransactionAccounts {
    #[cfg(not(target_os = "solana"))]
    fn new(accounts: Vec<RefCell<AccountSharedData>>) -> TransactionAccounts {
        TransactionAccounts {
            touched_flags: RefCell::new(vec![false; accounts.len()].into_boxed_slice()),
            accounts,
        }
    }

    fn len(&self) -> usize {
        self.accounts.len()
    }

    pub fn get(&self, index: IndexOfAccount) -> Option<&RefCell<AccountSharedData>> {
        self.accounts.get(index as usize)
    }

    #[cfg(not(target_os = "solana"))]
    pub fn touch(&self, index: IndexOfAccount) -> Result<(), InstructionError> {
        *self
            .touched_flags
            .borrow_mut()
            .get_mut(index as usize)
            .ok_or(InstructionError::NotEnoughAccountKeys)? = true;
        Ok(())
    }

    #[cfg(not(target_os = "solana"))]
    pub fn touched_count(&self) -> usize {
        self.touched_flags
            .borrow()
            .iter()
            .fold(0usize, |accumulator, was_touched| {
                accumulator.saturating_add(*was_touched as usize)
            })
    }

    pub fn try_borrow(
        &self,
        index: IndexOfAccount,
    ) -> Result<Ref<'_, AccountSharedData>, InstructionError> {
        self.accounts
            .get(index as usize)
            .ok_or(InstructionError::MissingAccount)?
            .try_borrow()
            .map_err(|_| InstructionError::AccountBorrowFailed)
    }

    pub fn try_borrow_mut(
        &self,
        index: IndexOfAccount,
    ) -> Result<RefMut<'_, AccountSharedData>, InstructionError> {
        self.accounts
            .get(index as usize)
            .ok_or(InstructionError::MissingAccount)?
            .try_borrow_mut()
            .map_err(|_| InstructionError::AccountBorrowFailed)
    }

    pub fn into_accounts(self) -> Vec<AccountSharedData> {
        self.accounts
            .into_iter()
            .map(|account| account.into_inner())
            .collect()
    }
}

/// Loaded transaction shared between runtime and programs.
///
/// This context is valid for the entire duration of a transaction being processed.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionContext {
    account_keys: Pin<Box<[Pubkey]>>,
    accounts: Rc<TransactionAccounts>,
    instruction_stack_capacity: usize,
    instruction_trace_capacity: usize,
    instruction_stack: Vec<usize>,
    instruction_trace: Vec<InstructionContext>,
    return_data: TransactionReturnData,
    pub(crate) accounts_resize_delta: RefCell<i64>,
    #[cfg(not(target_os = "solana"))]
    pub(crate) rent: Rent,
    // /// Useful for debugging to filter by or to look it up on the explorer
    // #[cfg(all(not(target_os = "solana"), debug_assertions))]
    // signature: Signature,
}

impl TransactionContext {
    /// Constructs a new TransactionContext
    #[cfg(not(target_os = "solana"))]
    pub fn new(
        transaction_accounts: Vec<TransactionAccount>,
        rent: Rent,
        instruction_stack_capacity: usize,
        instruction_trace_capacity: usize,
    ) -> Self {
        let (account_keys, accounts): (Vec<_>, Vec<_>) = transaction_accounts
            .into_iter()
            .map(|(key, account)| (key, RefCell::new(account)))
            .unzip();
        Self {
            account_keys: Pin::new(account_keys.into_boxed_slice()),
            accounts: Rc::new(TransactionAccounts::new(accounts)),
            instruction_stack_capacity,
            instruction_trace_capacity,
            instruction_stack: Vec::with_capacity(instruction_stack_capacity),
            instruction_trace: vec![InstructionContext::default()],
            return_data: TransactionReturnData::default(),
            accounts_resize_delta: RefCell::new(0),
            rent,
            // #[cfg(all(not(target_os = "solana"), debug_assertions))]
            // signature: Signature::default(),
        }
    }

    /// Used in mock_process_instruction
    #[cfg(not(target_os = "solana"))]
    pub fn deconstruct_without_keys(self) -> Result<Vec<AccountSharedData>, InstructionError> {
        if !self.instruction_stack.is_empty() {
            return Err(InstructionError::CallDepth);
        }

        Ok(Rc::try_unwrap(self.accounts)
            .expect("transaction_context.accounts has unexpected outstanding refs")
            .into_accounts())
    }

    #[cfg(not(target_os = "solana"))]
    pub fn accounts(&self) -> &Rc<TransactionAccounts> {
        &self.accounts
    }

    // /// Stores the signature of the current transaction
    // #[cfg(all(not(target_os = "solana"), debug_assertions))]
    // pub fn set_signature(&mut self, signature: &Signature) {
    //     self.signature = *signature;
    // }

    // /// Returns the signature of the current transaction
    // #[cfg(all(not(target_os = "solana"), debug_assertions))]
    // pub fn get_signature(&self) -> &Signature {
    //     &self.signature
    // }

    /// Returns the total number of accounts loaded in this Transaction
    pub fn get_number_of_accounts(&self) -> IndexOfAccount {
        self.accounts.len() as IndexOfAccount
    }

    /// Searches for an account by its key
    pub fn get_key_of_account_at_index(
        &self,
        index_in_transaction: IndexOfAccount,
    ) -> Result<&Pubkey, InstructionError> {
        self.account_keys
            .get(index_in_transaction as usize)
            .ok_or(InstructionError::NotEnoughAccountKeys)
    }

    /// Searches for an account by its key
    #[cfg(not(target_os = "solana"))]
    pub fn get_account_at_index(
        &self,
        index_in_transaction: IndexOfAccount,
    ) -> Result<&RefCell<AccountSharedData>, InstructionError> {
        self.accounts
            .get(index_in_transaction)
            .ok_or(InstructionError::NotEnoughAccountKeys)
    }

    /// Searches for an account by its key
    pub fn find_index_of_account(&self, pubkey: &Pubkey) -> Option<IndexOfAccount> {
        self.account_keys
            .iter()
            .position(|key| key == pubkey)
            .map(|index| index as IndexOfAccount)
    }

    /// Searches for a program account by its key
    pub fn find_index_of_program_account(&self, pubkey: &Pubkey) -> Option<IndexOfAccount> {
        self.account_keys
            .iter()
            .rposition(|key| key == pubkey)
            .map(|index| index as IndexOfAccount)
    }

    /// Gets the max length of the InstructionContext trace
    pub fn get_instruction_trace_capacity(&self) -> usize {
        self.instruction_trace_capacity
    }

    /// Returns the instruction trace length.
    ///
    /// Not counting the last empty InstructionContext which is always pre-reserved for the next instruction.
    /// See also `get_next_instruction_context()`.
    pub fn get_instruction_trace_length(&self) -> usize {
        self.instruction_trace.len().saturating_sub(1)
    }

    /// Gets an InstructionContext by its index in the trace
    pub fn get_instruction_context_at_index_in_trace(
        &self,
        index_in_trace: usize,
    ) -> Result<&InstructionContext, InstructionError> {
        self.instruction_trace
            .get(index_in_trace)
            .ok_or(InstructionError::CallDepth)
    }

    /// Gets an InstructionContext by its nesting level in the stack
    pub fn get_instruction_context_at_nesting_level(
        &self,
        nesting_level: usize,
    ) -> Result<&InstructionContext, InstructionError> {
        let index_in_trace = *self
            .instruction_stack
            .get(nesting_level)
            .ok_or(InstructionError::CallDepth)?;
        let instruction_context = self.get_instruction_context_at_index_in_trace(index_in_trace)?;
        debug_assert_eq!(instruction_context.nesting_level, nesting_level);
        Ok(instruction_context)
    }

    /// Gets the max height of the InstructionContext stack
    pub fn get_instruction_stack_capacity(&self) -> usize {
        self.instruction_stack_capacity
    }

    /// Gets instruction stack height, top-level instructions are height
    /// `solana_sdk::instruction::TRANSACTION_LEVEL_STACK_HEIGHT`
    pub fn get_instruction_context_stack_height(&self) -> usize {
        self.instruction_stack.len()
    }

    /// Returns the current InstructionContext
    pub fn get_current_instruction_context(&self) -> Result<&InstructionContext, InstructionError> {
        let level = self
            .get_instruction_context_stack_height()
            .checked_sub(1)
            .ok_or(InstructionError::CallDepth)?;
        self.get_instruction_context_at_nesting_level(level)
    }

    /// Returns the InstructionContext to configure for the next invocation.
    ///
    /// The last InstructionContext is always empty and pre-reserved for the next instruction.
    pub fn get_next_instruction_context(
        &mut self,
    ) -> Result<&mut InstructionContext, InstructionError> {
        self.instruction_trace
            .last_mut()
            .ok_or(InstructionError::CallDepth)
    }

    /// Pushes the next InstructionContext
    #[cfg(not(target_os = "solana"))]
    pub fn push(&mut self) -> Result<(), InstructionError> {
        let nesting_level = self.get_instruction_context_stack_height();
        let caller_instruction_context = self
            .instruction_trace
            .last()
            .ok_or(InstructionError::CallDepth)?;
        let callee_instruction_accounts_lamport_sum =
            self.instruction_accounts_lamport_sum(caller_instruction_context)?;
        if !self.instruction_stack.is_empty() {
            let caller_instruction_context = self.get_current_instruction_context()?;
            let original_caller_instruction_accounts_lamport_sum =
                caller_instruction_context.instruction_accounts_lamport_sum;
            let current_caller_instruction_accounts_lamport_sum =
                self.instruction_accounts_lamport_sum(caller_instruction_context)?;
            if original_caller_instruction_accounts_lamport_sum
                != current_caller_instruction_accounts_lamport_sum
            {
                return Err(InstructionError::UnbalancedInstruction);
            }
        }
        {
            let instruction_context = self.get_next_instruction_context()?;
            instruction_context.nesting_level = nesting_level;
            instruction_context.instruction_accounts_lamport_sum =
                callee_instruction_accounts_lamport_sum;
        }
        let index_in_trace = self.get_instruction_trace_length();
        if index_in_trace >= self.instruction_trace_capacity {
            return Err(InstructionError::MaxInstructionTraceLengthExceeded);
        }
        self.instruction_trace.push(InstructionContext::default());
        if nesting_level >= self.instruction_stack_capacity {
            return Err(InstructionError::CallDepth);
        }
        self.instruction_stack.push(index_in_trace);
        Ok(())
    }

    /// Pops the current InstructionContext
    #[cfg(not(target_os = "solana"))]
    pub fn pop(&mut self) -> Result<(), InstructionError> {
        if self.instruction_stack.is_empty() {
            return Err(InstructionError::CallDepth);
        }
        // Verify (before we pop) that the total sum of all lamports in this instruction did not change
        let detected_an_unbalanced_instruction =
            self.get_current_instruction_context()
                .and_then(|instruction_context| {
                    // Verify all executable accounts have no outstanding refs
                    for account_index in instruction_context.program_accounts.iter() {
                        self.get_account_at_index(*account_index)?
                            .try_borrow_mut()
                            .map_err(|_| InstructionError::AccountBorrowOutstanding)?;
                    }
                    self.instruction_accounts_lamport_sum(instruction_context)
                        .map(|instruction_accounts_lamport_sum| {
                            instruction_context.instruction_accounts_lamport_sum
                                != instruction_accounts_lamport_sum
                        })
                });
        // Always pop, even if we `detected_an_unbalanced_instruction`
        self.instruction_stack.pop();
        if detected_an_unbalanced_instruction? {
            Err(InstructionError::UnbalancedInstruction)
        } else {
            Ok(())
        }
    }

    /// Gets the return data of the current InstructionContext or any above
    pub fn get_return_data(&self) -> (&Pubkey, &[u8]) {
        (&self.return_data.program_id, &self.return_data.data)
    }

    /// Set the return data of the current InstructionContext
    pub fn set_return_data(
        &mut self,
        program_id: Pubkey,
        data: Vec<u8>,
    ) -> Result<(), InstructionError> {
        self.return_data = TransactionReturnData { program_id, data };
        Ok(())
    }

    /// Calculates the sum of all lamports within an instruction
    #[cfg(not(target_os = "solana"))]
    fn instruction_accounts_lamport_sum(
        &self,
        instruction_context: &InstructionContext,
    ) -> Result<u128, InstructionError> {
        let mut instruction_accounts_lamport_sum: u128 = 0;
        for instruction_account_index in 0..instruction_context.get_number_of_instruction_accounts()
        {
            if instruction_context
                .is_instruction_account_duplicate(instruction_account_index)?
                .is_some()
            {
                continue; // Skip duplicate account
            }
            let index_in_transaction = instruction_context
                .get_index_of_instruction_account_in_transaction(instruction_account_index)?;
            instruction_accounts_lamport_sum = (self
                .get_account_at_index(index_in_transaction)?
                .try_borrow()
                .map_err(|_| InstructionError::AccountBorrowOutstanding)?
                .lamports() as u128)
                .checked_add(instruction_accounts_lamport_sum)
                .ok_or(InstructionError::ArithmeticOverflow)?;
        }
        Ok(instruction_accounts_lamport_sum)
    }

    /// Returns the accounts resize delta
    pub fn accounts_resize_delta(&self) -> Result<i64, InstructionError> {
        self.accounts_resize_delta
            .try_borrow()
            .map_err(|_| InstructionError::GenericError)
            .map(|value_ref| *value_ref)
    }
}

/// Return data at the end of a transaction
#[derive(Clone, Debug, Default, PartialEq, Eq /*, Deserialize, Serialize*/)]
pub struct TransactionReturnData {
    pub program_id: Pubkey,
    pub data: Vec<u8>,
}

/// Loaded instruction shared between runtime and programs.
///
/// This context is valid for the entire duration of a (possibly cross program) instruction being processed.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct InstructionContext {
    nesting_level: usize,
    instruction_accounts_lamport_sum: u128,
    program_accounts: Vec<IndexOfAccount>,
    instruction_accounts: Vec<InstructionAccount>,
    instruction_data: Vec<u8>,
}

impl InstructionContext {
    /// Used together with TransactionContext::get_next_instruction_context()
    #[cfg(not(target_os = "solana"))]
    pub fn configure(
        &mut self,
        program_accounts: &[IndexOfAccount],
        instruction_accounts: &[InstructionAccount],
        instruction_data: &[u8],
    ) {
        self.program_accounts = program_accounts.to_vec();
        self.instruction_accounts = instruction_accounts.to_vec();
        self.instruction_data = instruction_data.to_vec();
    }

    /// How many Instructions were on the stack after this one was pushed
    ///
    /// That is the number of nested parent Instructions plus one (itself).
    pub fn get_stack_height(&self) -> usize {
        self.nesting_level.saturating_add(1)
    }

    /// Number of program accounts
    pub fn get_number_of_program_accounts(&self) -> IndexOfAccount {
        self.program_accounts.len() as IndexOfAccount
    }

    /// Number of accounts in this Instruction (without program accounts)
    pub fn get_number_of_instruction_accounts(&self) -> IndexOfAccount {
        self.instruction_accounts.len() as IndexOfAccount
    }

    /// Assert that enough accounts were supplied to this Instruction
    pub fn check_number_of_instruction_accounts(
        &self,
        expected_at_least: IndexOfAccount,
    ) -> Result<(), InstructionError> {
        if self.get_number_of_instruction_accounts() < expected_at_least {
            Err(InstructionError::NotEnoughAccountKeys)
        } else {
            Ok(())
        }
    }

    /// Data parameter for the programs `process_instruction` handler
    pub fn get_instruction_data(&self) -> &[u8] {
        &self.instruction_data
    }

    /// Searches for a program account by its key
    pub fn find_index_of_program_account(
        &self,
        transaction_context: &TransactionContext,
        pubkey: &Pubkey,
    ) -> Option<IndexOfAccount> {
        self.program_accounts
            .iter()
            .position(|index_in_transaction| {
                transaction_context
                    .account_keys
                    .get(*index_in_transaction as usize)
                    == Some(pubkey)
            })
            .map(|index| index as IndexOfAccount)
    }

    /// Searches for an instruction account by its key
    pub fn find_index_of_instruction_account(
        &self,
        transaction_context: &TransactionContext,
        pubkey: &Pubkey,
    ) -> Option<IndexOfAccount> {
        self.instruction_accounts
            .iter()
            .position(|instruction_account| {
                transaction_context
                    .account_keys
                    .get(instruction_account.index_in_transaction as usize)
                    == Some(pubkey)
            })
            .map(|index| index as IndexOfAccount)
    }

    /// Translates the given instruction wide program_account_index into a transaction wide index
    pub fn get_index_of_program_account_in_transaction(
        &self,
        program_account_index: IndexOfAccount,
    ) -> Result<IndexOfAccount, InstructionError> {
        Ok(*self
            .program_accounts
            .get(program_account_index as usize)
            .ok_or(InstructionError::NotEnoughAccountKeys)?)
    }

    /// Translates the given instruction wide instruction_account_index into a transaction wide index
    pub fn get_index_of_instruction_account_in_transaction(
        &self,
        instruction_account_index: IndexOfAccount,
    ) -> Result<IndexOfAccount, InstructionError> {
        Ok(self
            .instruction_accounts
            .get(instruction_account_index as usize)
            .ok_or(InstructionError::NotEnoughAccountKeys)?
            .index_in_transaction as IndexOfAccount)
    }

    /// Returns `Some(instruction_account_index)` if this is a duplicate
    /// and `None` if it is the first account with this key
    pub fn is_instruction_account_duplicate(
        &self,
        instruction_account_index: IndexOfAccount,
    ) -> Result<Option<IndexOfAccount>, InstructionError> {
        let index_in_callee = self
            .instruction_accounts
            .get(instruction_account_index as usize)
            .ok_or(InstructionError::NotEnoughAccountKeys)?
            .index_in_callee;
        Ok(if index_in_callee == instruction_account_index {
            None
        } else {
            Some(index_in_callee)
        })
    }

    /// Gets the key of the last program account of this Instruction
    pub fn get_last_program_key<'a, 'b: 'a>(
        &'a self,
        transaction_context: &'b TransactionContext,
    ) -> Result<&'b Pubkey, InstructionError> {
        self.get_index_of_program_account_in_transaction(
            self.get_number_of_program_accounts().saturating_sub(1),
        )
        .and_then(|index_in_transaction| {
            transaction_context.get_key_of_account_at_index(index_in_transaction)
        })
    }

    fn try_borrow_account<'a, 'b: 'a>(
        &'a self,
        transaction_context: &'b TransactionContext,
        index_in_transaction: IndexOfAccount,
        index_in_instruction: IndexOfAccount,
    ) -> Result<BorrowedAccount<'a>, InstructionError> {
        let account = transaction_context
            .accounts
            .get(index_in_transaction)
            .ok_or(InstructionError::MissingAccount)?
            .try_borrow_mut()
            .map_err(|_| InstructionError::AccountBorrowFailed)?;
        Ok(BorrowedAccount {
            transaction_context,
            instruction_context: self,
            index_in_transaction,
            index_in_instruction,
            account,
        })
    }

    /// Gets the last program account of this Instruction
    pub fn try_borrow_last_program_account<'a, 'b: 'a>(
        &'a self,
        transaction_context: &'b TransactionContext,
    ) -> Result<BorrowedAccount<'a>, InstructionError> {
        let number_of_program_accounts = self.get_number_of_program_accounts();
        let number_of_program_accounts = number_of_program_accounts.saturating_sub(1);
        let result =
            self.try_borrow_program_account(transaction_context, number_of_program_accounts);
        debug_assert!(result.is_ok());
        result
    }

    /// Tries to borrow a program account from this Instruction
    pub fn try_borrow_program_account<'a, 'b: 'a>(
        &'a self,
        transaction_context: &'b TransactionContext,
        program_account_index: IndexOfAccount,
    ) -> Result<BorrowedAccount<'a>, InstructionError> {
        let index_in_transaction =
            self.get_index_of_program_account_in_transaction(program_account_index)?;
        self.try_borrow_account(
            transaction_context,
            index_in_transaction,
            program_account_index,
        )
    }

    /// Gets an instruction account of this Instruction
    pub fn try_borrow_instruction_account<'a, 'b: 'a>(
        &'a self,
        transaction_context: &'b TransactionContext,
        instruction_account_index: IndexOfAccount,
    ) -> Result<BorrowedAccount<'a>, InstructionError> {
        let index_in_transaction =
            self.get_index_of_instruction_account_in_transaction(instruction_account_index)?;
        self.try_borrow_account(
            transaction_context,
            index_in_transaction,
            self.get_number_of_program_accounts()
                .saturating_add(instruction_account_index),
        )
    }

    /// Returns whether an instruction account is a signer
    pub fn is_instruction_account_signer(
        &self,
        instruction_account_index: IndexOfAccount,
    ) -> Result<bool, InstructionError> {
        Ok(self
            .instruction_accounts
            .get(instruction_account_index as usize)
            .ok_or(InstructionError::MissingAccount)?
            .is_signer)
    }

    /// Returns whether an instruction account is writable
    pub fn is_instruction_account_writable(
        &self,
        instruction_account_index: IndexOfAccount,
    ) -> Result<bool, InstructionError> {
        Ok(self
            .instruction_accounts
            .get(instruction_account_index as usize)
            .ok_or(InstructionError::MissingAccount)?
            .is_writable)
    }

    /// Calculates the set of all keys of signer instruction accounts in this Instruction
    pub fn get_signers(
        &self,
        transaction_context: &TransactionContext,
    ) -> Result<HashSet<Pubkey>, InstructionError> {
        let mut result = HashSet::new();
        for instruction_account in self.instruction_accounts.iter() {
            if instruction_account.is_signer {
                result.insert(
                    *transaction_context
                        .get_key_of_account_at_index(instruction_account.index_in_transaction)?,
                );
            }
        }
        Ok(result)
    }
}
