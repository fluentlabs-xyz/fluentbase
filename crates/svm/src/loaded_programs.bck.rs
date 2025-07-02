use crate::{
    bpf_loader,
    bpf_loader_deprecated,
    clock::{Epoch, Slot},
    context::{BuiltinFunctionWithContext, InvokeContext},
    error::SvmError,
    pubkey::Pubkey,
    solana_program::{bpf_loader_upgradeable, loader_v4},
};
use alloc::{boxed::Box, sync::Arc};
use core::{
    fmt::{Debug, Formatter},
    sync::atomic::{AtomicU64, Ordering},
};
use fluentbase_sdk::{HashMap, SharedAPI};
use solana_rbpf::{
    elf::{ElfError, Executable},
    program::{BuiltinProgram, FunctionRegistry},
    verifier::RequisiteVerifier,
    vm::Config,
};

pub type ProgramRuntimeEnvironment<'a, SDK> = Arc<BuiltinProgram<InvokeContext<'a, SDK>>>;
pub const MAX_LOADED_ENTRY_COUNT: usize = 256;
pub const DELAY_VISIBILITY_SLOT_OFFSET: Slot = 1;

#[derive(Default)]
pub enum LoadedProgramType<'a, SDK: SharedAPI> {
    /// Tombstone for programs which currently do not pass the verifier but could if the feature set changed.
    FailedVerification(ProgramRuntimeEnvironment<'a, SDK>),
    /// Tombstone for programs that were either explicitly closed or never deployed.
    ///
    /// It's also used for accounts belonging to program loaders, that don't actually contain program code (e.g. buffer accounts for LoaderV3 programs).
    #[default]
    Closed,
    DelayVisibility,
    /// Successfully verified but not currently compiled, used to track usage statistics when a compiled program is evicted from memory.
    Unloaded(ProgramRuntimeEnvironment<'a, SDK>),
    LegacyV0(Arc<Executable<InvokeContext<'a, SDK>>>),
    LegacyV1(Arc<Executable<InvokeContext<'a, SDK>>>),
    Typed(Arc<Executable<InvokeContext<'a, SDK>>>),
    #[cfg(test)]
    TestLoaded(ProgramRuntimeEnvironment<'a, SDK>),
    Builtin(Arc<BuiltinProgram<InvokeContext<'a, SDK>>>),
}

impl<'a, SDK: SharedAPI> Debug for LoadedProgramType<'a, SDK> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            LoadedProgramType::FailedVerification(_) => {
                write!(f, "LoadedProgramType::FailedVerification")
            }
            LoadedProgramType::Closed => write!(f, "LoadedProgramType::Closed"),
            LoadedProgramType::DelayVisibility => write!(f, "LoadedProgramType::DelayVisibility"),
            LoadedProgramType::Unloaded(_) => write!(f, "LoadedProgramType::Unloaded"),
            LoadedProgramType::LegacyV0(_) => write!(f, "LoadedProgramType::LegacyV0"),
            LoadedProgramType::LegacyV1(_) => write!(f, "LoadedProgramType::LegacyV1"),
            LoadedProgramType::Typed(_) => write!(f, "LoadedProgramType::Typed"),
            #[cfg(test)]
            LoadedProgramType::TestLoaded(_) => write!(f, "LoadedProgramType::TestLoaded"),
            LoadedProgramType::Builtin(_) => write!(f, "LoadedProgramType::Builtin"),
        }
    }
}

impl<'a, SDK: SharedAPI> LoadedProgramType<'a, SDK> {
    /// Returns a reference to its environment if it has one
    pub fn get_environment(&self) -> Option<&ProgramRuntimeEnvironment<'a, SDK>> {
        match self {
            LoadedProgramType::LegacyV0(program)
            | LoadedProgramType::LegacyV1(program)
            | LoadedProgramType::Typed(program) => Some(program.get_loader()),
            LoadedProgramType::FailedVerification(env) | LoadedProgramType::Unloaded(env) => {
                Some(env)
            }
            #[cfg(test)]
            LoadedProgramType::TestLoaded(environment) => Some(environment),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProgramRuntimeEnvironments<'a, SDK: SharedAPI> {
    /// Globally shared RBPF config and syscall registry for runtime V1
    pub program_runtime_v1: ProgramRuntimeEnvironment<'a, SDK>,
    /// Globally shared RBPF config and syscall registry for runtime V2
    pub program_runtime_v2: ProgramRuntimeEnvironment<'a, SDK>,
}

impl<'a, SDK: SharedAPI> Default for ProgramRuntimeEnvironments<'a, SDK> {
    fn default() -> Self {
        let config = Config::default();
        let empty_loader = Arc::new(BuiltinProgram::new_loader(
            config,
            FunctionRegistry::default(),
        ));
        Self {
            program_runtime_v1: empty_loader.clone(),
            program_runtime_v2: empty_loader,
        }
    }
}

#[derive(Debug, Default)]
pub struct LoadedProgram<'a, SDK: SharedAPI> {
    /// The program of this entry
    pub program: Arc<LoadedProgramType<'a, SDK>>,
    /// Size of account that stores the program and program data
    pub account_size: usize,
    /// Slot in which the program was (re)deployed
    pub deployment_slot: Slot,
    /// Slot in which this entry will become active (can be in the future)
    pub effective_slot: Slot,
    /// Optional expiration slot for this entry, after which it is treated as non-existent
    pub maybe_expiration_slot: Option<Slot>,
    /// How often this entry was used by a transaction
    pub tx_usage_counter: AtomicU64,
    /// How often this entry was used by an instruction
    pub ix_usage_counter: AtomicU64,
    /// Latest slot in which the entry was used
    pub latest_access_slot: AtomicU64,
}

impl<'a, 'b, SDK: SharedAPI> LoadedProgram<'a, SDK> {
    /// Creates a new user program
    pub fn new(
        loader_key: &Pubkey,
        program_runtime_environment: ProgramRuntimeEnvironment<'a, SDK>,
        deployment_slot: Slot,
        effective_slot: Slot,
        maybe_expiration_slot: Option<Slot>,
        elf_bytes: &[u8],
        account_size: usize,
        // metrics: &mut LoadProgramMetrics,
    ) -> Result<Self, SvmError> {
        Self::new_internal(
            loader_key,
            program_runtime_environment,
            deployment_slot,
            effective_slot,
            maybe_expiration_slot,
            elf_bytes,
            account_size,
            // metrics,
            false, /* reloading */
        )
    }

    /// Reloads a user program, *without* running the verifier.
    ///
    /// # Safety
    ///
    /// This method is unsafe since it assumes that the program has already been verified. Should
    /// only be called when the program was previously verified and loaded in the cache, but was
    /// unloaded due to inactivity. It should also be checked that the `program_runtime_environment`
    /// hasn't changed since it was unloaded.
    pub unsafe fn reload(
        loader_key: &Pubkey,
        program_runtime_environment: Arc<BuiltinProgram<InvokeContext<'a, SDK>>>,
        deployment_slot: Slot,
        effective_slot: Slot,
        maybe_expiration_slot: Option<Slot>,
        elf_bytes: &[u8],
        account_size: usize,
        // metrics: &mut LoadProgramMetrics,
    ) -> Result<Self, SvmError> {
        Self::new_internal(
            loader_key,
            program_runtime_environment,
            deployment_slot,
            effective_slot,
            maybe_expiration_slot,
            elf_bytes,
            account_size,
            // metrics,
            true, /* reloading */
        )
    }

    fn new_internal(
        loader_key: &Pubkey,
        program_runtime_environment: Arc<BuiltinProgram<InvokeContext<'a, SDK>>>,
        deployment_slot: Slot,
        effective_slot: Slot,
        maybe_expiration_slot: Option<Slot>,
        elf_bytes: &[u8],
        account_size: usize,
        // metrics: &mut LoadProgramMetrics,
        reloading: bool,
    ) -> Result<Self, SvmError> {
        // let mut load_elf_time = Measure::start("load_elf_time");
        // The following unused_mut exception is needed for architectures that do not
        // support JIT compilation.
        let executable = Executable::load(elf_bytes, program_runtime_environment.clone());
        let mut executable = executable?;
        // load_elf_time.stop();
        // metrics.load_elf_us = load_elf_time.as_us();

        if !reloading {
            // let mut verify_code_time = Measure::start("verify_code_time");
            executable.verify::<RequisiteVerifier>()?;
            // verify_code_time.stop();
            // metrics.verify_code_us = verify_code_time.as_us();
        }

        // #[cfg(all(not(target_os = "windows"), target_arch = "x86_64"))]
        // {
        //     // let mut jit_compile_time = Measure::start("jit_compile_time");
        //     executable.jit_compile()?;
        //     // jit_compile_time.stop();
        //     // metrics.jit_compile_us = jit_compile_time.as_us();
        // }

        // Allowing mut here, since it may be needed for jit compile, which is under a config flag
        #[allow(unused_mut)]
        let mut program = if bpf_loader_deprecated::check_id(loader_key) {
            LoadedProgramType::LegacyV0(executable.into())
        } else if bpf_loader::check_id(loader_key) || bpf_loader_upgradeable::check_id(loader_key) {
            LoadedProgramType::LegacyV1(executable.into())
        } else if loader_v4::check_id(loader_key) {
            LoadedProgramType::Typed(executable.into())
        } else {
            panic!();
        };

        Ok(Self {
            deployment_slot,
            account_size,
            effective_slot,
            maybe_expiration_slot,
            tx_usage_counter: AtomicU64::new(0),
            program: program.into(),
            ix_usage_counter: AtomicU64::new(0),
            latest_access_slot: AtomicU64::new(0),
        })
    }

    pub fn to_unloaded(&self) -> Option<Self> {
        Some(Self {
            program: LoadedProgramType::Unloaded(self.program.get_environment()?.clone()).into(),
            account_size: self.account_size,
            deployment_slot: self.deployment_slot,
            effective_slot: self.effective_slot,
            maybe_expiration_slot: self.maybe_expiration_slot,
            tx_usage_counter: AtomicU64::new(self.tx_usage_counter.load(Ordering::Relaxed)),
            ix_usage_counter: AtomicU64::new(self.ix_usage_counter.load(Ordering::Relaxed)),
            latest_access_slot: AtomicU64::new(self.latest_access_slot.load(Ordering::Relaxed)),
        })
    }

    /// Creates a new built-in program
    pub fn new_builtin(
        deployment_slot: Slot,
        account_size: usize,
        builtin_function: BuiltinFunctionWithContext<'a, SDK>,
    ) -> Self {
        let mut function_registry = FunctionRegistry::default();
        function_registry
            .register_function_hashed(*b"entrypoint", builtin_function)
            .unwrap();
        Self {
            deployment_slot,
            account_size,
            effective_slot: deployment_slot,
            maybe_expiration_slot: None,
            tx_usage_counter: AtomicU64::new(0),
            program: LoadedProgramType::Builtin(
                BuiltinProgram::new_builtin(function_registry).into(),
            )
            .into(),
            ix_usage_counter: AtomicU64::new(0),
            latest_access_slot: AtomicU64::new(0),
        }
    }

    pub fn new_tombstone(slot: Slot, reason: LoadedProgramType<'a, SDK>) -> Self {
        let maybe_expiration_slot = matches!(reason, LoadedProgramType::DelayVisibility)
            .then_some(slot.saturating_add(DELAY_VISIBILITY_SLOT_OFFSET));
        let tombstone = Self {
            program: reason.into(),
            account_size: 0,
            deployment_slot: slot,
            effective_slot: slot,
            maybe_expiration_slot,
            tx_usage_counter: AtomicU64::default(),
            ix_usage_counter: AtomicU64::default(),
            latest_access_slot: AtomicU64::new(0),
        };
        debug_assert!(tombstone.is_tombstone());
        tombstone
    }

    pub fn is_tombstone(&self) -> bool {
        matches!(
            self.program.as_ref(),
            LoadedProgramType::FailedVerification(_)
                | LoadedProgramType::Closed
                | LoadedProgramType::DelayVisibility
        )
    }

    fn is_implicit_delay_visibility_tombstone(&self, slot: Slot) -> bool {
        !matches!(self.program.as_ref(), LoadedProgramType::Builtin(_))
            && self.effective_slot.saturating_sub(self.deployment_slot)
                == DELAY_VISIBILITY_SLOT_OFFSET
            && slot >= self.deployment_slot
            && slot < self.effective_slot
    }

    pub fn update_access_slot(&self, slot: Slot) {
        let _ = self.latest_access_slot.fetch_max(slot, Ordering::Relaxed);
    }

    pub fn decayed_usage_counter(&self, now: Slot) -> u64 {
        let last_access = self.latest_access_slot.load(Ordering::Relaxed);
        // Shifting the u64 value for more than 63 will cause an overflow.
        let decaying_for = core::cmp::min(63, now.saturating_sub(last_access));
        self.tx_usage_counter.load(Ordering::Relaxed) >> decaying_for
    }
}

#[derive(Debug)]
pub struct LoadedProgramsForTxBatch<'a, SDK: SharedAPI> {
    /// Pubkey is the address of a program.
    /// LoadedProgram is the corresponding program entry valid for the slot in which a transaction is being executed.
    entries: HashMap<Pubkey, Arc<LoadedProgram<'a, SDK>>>,
    slot: Slot,
    pub environments: ProgramRuntimeEnvironments<'a, SDK>,
    /// Anticipated replacement for `environments` at the next epoch.
    ///
    /// This is `None` during most of an epoch, and only `Some` around the boundaries (at the end and beginning of an epoch).
    /// More precisely, it starts with the recompilation phase a few hundred slots before the epoch boundary,
    /// and it ends with the first rerooting after the epoch boundary.
    /// Needed when a program is deployed at the last slot of an epoch, becomes effective in the next epoch.
    /// So needs to be compiled with the environment for the next epoch.
    pub upcoming_environments: Option<ProgramRuntimeEnvironments<'a, SDK>>,
    /// The epoch of the last rerooting
    pub latest_root_epoch: Epoch,
    pub hit_max_limit: bool,
}

impl<'a, SDK: SharedAPI> LoadedProgramsForTxBatch<'a, SDK> {
    pub fn new(
        slot: Slot,
        environments: ProgramRuntimeEnvironments<'a, SDK>,
        upcoming_environments: Option<ProgramRuntimeEnvironments<'a, SDK>>,
        latest_root_epoch: Epoch,
    ) -> Self {
        Self {
            slot,
            environments,
            upcoming_environments,
            latest_root_epoch,
            hit_max_limit: false,
            entries: Default::default(),
        }
    }
    pub fn partial_default1(
        slot: Slot,
        environments: ProgramRuntimeEnvironments<'a, SDK>,
        upcoming_environments: Option<ProgramRuntimeEnvironments<'a, SDK>>,
    ) -> Self {
        Self {
            entries: HashMap::new(),
            slot,
            environments,
            upcoming_environments,
            latest_root_epoch: Epoch::default(),
            hit_max_limit: false,
        }
    }
    pub fn partial_default2(slot: Slot, environments: ProgramRuntimeEnvironments<'a, SDK>) -> Self {
        Self::partial_default1(slot, environments, None)
    }

    #[cfg(test)]
    pub fn entries(&self) -> &HashMap<Pubkey, Arc<LoadedProgram<'a, SDK>>> {
        &self.entries
    }

    // pub fn new_from_cache<FG: ForkGraph>(
    //     slot: Slot,
    //     epoch: Epoch,
    //     cache: &LoadedPrograms<FG>,
    // ) -> Self {
    //     Self {
    //         entries: HashMap::new(),
    //         slot,
    //         environments: cache.get_environments_for_epoch(epoch).clone(),
    //         upcoming_environments: cache.get_upcoming_environments_for_epoch(epoch),
    //         latest_root_epoch: cache.latest_root_epoch,
    //         hit_max_limit: false,
    //     }
    // }

    /// Returns the current environments depending on the given epoch
    pub fn get_environments_for_epoch(&self, epoch: Epoch) -> &ProgramRuntimeEnvironments<'a, SDK> {
        if epoch != self.latest_root_epoch {
            if let Some(upcoming_environments) = self.upcoming_environments.as_ref() {
                return upcoming_environments;
            }
        }
        &self.environments
    }

    /// Refill the cache with a single entry. It's typically called during transaction loading, and
    /// transaction processing (for program management instructions).
    /// It replaces the existing entry (if any) with the provided entry. The return value contains
    /// `true` if an entry existed.
    /// The function also returns the newly inserted value.
    pub fn replenish(
        &mut self,
        key: Pubkey,
        entry: Arc<LoadedProgram<'a, SDK>>,
    ) -> (bool, Arc<LoadedProgram<'a, SDK>>) {
        (self.entries.insert(key, entry.clone()).is_some(), entry)
    }

    pub fn find(&self, key: &Pubkey) -> Option<Arc<LoadedProgram<'a, SDK>>> {
        self.entries.get(key).map(|entry| {
            if entry.is_implicit_delay_visibility_tombstone(self.slot) {
                // Found a program entry on the current fork, but it's not effective
                // yet. It indicates that the program has delayed visibility. Return
                // the tombstone to reflect that.
                Arc::new(LoadedProgram::new_tombstone(
                    entry.deployment_slot,
                    LoadedProgramType::DelayVisibility,
                ))
            } else {
                entry.clone()
            }
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn slot(&self) -> Slot {
        self.slot
    }

    pub fn set_slot(&mut self, slot: Slot) {
        self.slot = slot;
    }
    pub fn set_slot_for_tests(&mut self, slot: Slot) {
        self.slot = slot;
    }

    pub fn merge(&mut self, other: &Self) {
        other.entries.iter().for_each(|(key, entry)| {
            self.replenish(*key, entry.clone());
        })
    }
}
