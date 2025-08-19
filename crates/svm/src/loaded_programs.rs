use crate::{
    // bpf_loader,
    common::rbpf_config_default,
    context::{BuiltinFunctionWithContext, InvokeContext},
    native_loader,
    solana_program::loader_v4,
};
use alloc::{boxed::Box, sync::Arc};
use core::fmt::{Debug, Formatter};
use fluentbase_sdk::SharedAPI;
use hashbrown::HashMap;
use solana_clock::{Epoch, Slot};
use solana_pubkey::Pubkey;
use solana_rbpf::{
    elf::Executable,
    program::{BuiltinProgram, FunctionRegistry},
    verifier::RequisiteVerifier,
};

pub type ProgramRuntimeEnvironment<'a, SDK> = Arc<BuiltinProgram<InvokeContext<'a, SDK>>>;

pub const DELAY_VISIBILITY_SLOT_OFFSET: Slot = 1;

/// Relationship between two fork IDs
#[derive(Copy, Clone, PartialEq)]
pub enum BlockRelation {
    /// The slot is on the same fork and is an ancestor of the other slot
    Ancestor,
    /// The two slots are equal and are on the same fork
    Equal,
    /// The slot is on the same fork and is a descendant of the other slot
    Descendant,
    /// The slots are on two different forks and may have had a common ancestor at some point
    Unrelated,
    /// Either one or both of the slots are either older than the latest root, or are in future
    Unknown,
}

/// Maps relationship between two slots.
pub trait ForkGraph {
    /// Returns the BlockRelation of A to B
    fn relationship(&self, a: Slot, b: Slot) -> BlockRelation;
}

/// The owner of a programs accounts, thus the loader of a program
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum ProgramCacheEntryOwner {
    #[default]
    NativeLoader,
    LoaderV4,
}

impl TryFrom<&Pubkey> for ProgramCacheEntryOwner {
    type Error = ();
    fn try_from(loader_key: &Pubkey) -> Result<Self, ()> {
        if native_loader::check_id(loader_key) {
            Ok(ProgramCacheEntryOwner::NativeLoader)
        // } else if bpf_loader::check_id(loader_key) {
        //     Ok(ProgramCacheEntryOwner::LoaderV2)
        } else if loader_v4::check_id(loader_key) {
            Ok(ProgramCacheEntryOwner::LoaderV4)
        } else {
            Err(())
        }
    }
}

impl From<ProgramCacheEntryOwner> for Pubkey {
    fn from(program_cache_entry_owner: ProgramCacheEntryOwner) -> Self {
        match program_cache_entry_owner {
            ProgramCacheEntryOwner::NativeLoader => native_loader::id(),
            // ProgramCacheEntryOwner::LoaderV2 => bpf_loader::id(),
            ProgramCacheEntryOwner::LoaderV4 => loader_v4::id(),
        }
    }
}

/*
    The possible ProgramCacheEntryType transitions:

    DelayVisibility is special in that it is never stored in the cache.
    It is only returned by ProgramCacheForTxBatch::find() when a Loaded entry
    is encountered which is not effective yet.

    Builtin re/deployment:
    - Empty => Builtin in TransactionBatchProcessor::add_builtin
    - Builtin => Builtin in TransactionBatchProcessor::add_builtin

    Un/re/deployment (with delay and cooldown):
    - Empty / Closed => Loaded in UpgradeableLoaderInstruction::DeployWithMaxDataLen
    - Loaded / FailedVerification => Loaded in UpgradeableLoaderInstruction::Upgrade
    - Loaded / FailedVerification => Closed in UpgradeableLoaderInstruction::Close

    Eviction and unloading (in the same slot):
    - Unloaded => Loaded in ProgramCache::assign_program
    - Loaded => Unloaded in ProgramCache::unload_program_entry

    At epoch boundary (when feature set and environment changes):
    - Loaded => FailedVerification in Bank::_new_from_parent
    - FailedVerification => Loaded in Bank::_new_from_parent

    Through pruning (when on orphan fork or overshadowed on the rooted fork):
    - Closed / Unloaded / Loaded / Builtin => Empty in ProgramCache::prune
*/

/// Actual payload of [ProgramCacheEntry].
#[derive(Default)]
pub enum ProgramCacheEntryType<'a, SDK: SharedAPI> {
    /// Tombstone for programs which currently do not pass the verifier but could if the feature
    /// set changed.
    FailedVerification(ProgramRuntimeEnvironment<'a, SDK>),
    /// Tombstone for programs that were either explicitly closed or never deployed.
    ///
    /// It's also used for accounts belonging to program loaders, that don't actually contain
    /// program code (e.g. buffer accounts for LoaderV3 programs).
    #[default]
    Closed,
    /// Tombstone for programs which have recently been modified but the new version is not visible
    /// yet.
    DelayVisibility,
    /// Successfully verified but not currently compiled.
    ///
    /// It continues to track usage statistics even when the compiled executable of the program is
    /// evicted from memory.
    Unloaded(ProgramRuntimeEnvironment<'a, SDK>),
    /// Verified and compiled program
    Loaded(Arc<Executable<InvokeContext<'a, SDK>>>),
    /// A built-in program which is not stored on-chain but backed into and distributed with the
    /// validator
    Builtin(Arc<BuiltinProgram<InvokeContext<'a, SDK>>>),
}

impl<'a, SDK: SharedAPI> Debug for ProgramCacheEntryType<'a, SDK> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            ProgramCacheEntryType::FailedVerification(_) => {
                write!(f, "ProgramCacheEntryType::FailedVerification")
            }
            ProgramCacheEntryType::Closed => write!(f, "ProgramCacheEntryType::Closed"),
            ProgramCacheEntryType::DelayVisibility => {
                write!(f, "ProgramCacheEntryType::DelayVisibility")
            }
            ProgramCacheEntryType::Unloaded(_) => write!(f, "ProgramCacheEntryType::Unloaded"),
            ProgramCacheEntryType::Loaded(_) => write!(f, "ProgramCacheEntryType::Loaded"),
            ProgramCacheEntryType::Builtin(_) => write!(f, "ProgramCacheEntryType::Builtin"),
        }
    }
}

impl<'a, SDK: SharedAPI> ProgramCacheEntryType<'a, SDK> {
    /// Returns a reference to its environment if it has one
    pub fn get_environment(&self) -> Option<ProgramRuntimeEnvironment<'a, SDK>> {
        match self {
            ProgramCacheEntryType::Loaded(program) => Some(program.get_loader().clone()),
            ProgramCacheEntryType::FailedVerification(env)
            | ProgramCacheEntryType::Unloaded(env) => Some(env.clone()),
            _ => None,
        }
    }
}

/// Holds a program version at a specific address and on a specific slot / fork.
///
/// It contains the actual program in [ProgramCacheEntryType] and a bunch of meta-data.
#[derive(Default)]
pub struct ProgramCacheEntry<'a, SDK: SharedAPI> {
    /// The program of this entry
    pub program: ProgramCacheEntryType<'a, SDK>,
    /// The loader of this entry
    pub account_owner: ProgramCacheEntryOwner,
    /// Size of account that stores the program and program data
    pub account_size: usize,
    /// Slot in which the program was (re)deployed
    pub deployment_slot: Slot,
    /// Slot in which this entry will become active (can be in the future)
    pub effective_slot: Slot,
}

impl<'a, SDK: SharedAPI> PartialEq for ProgramCacheEntry<'a, SDK> {
    fn eq(&self, other: &Self) -> bool {
        self.effective_slot == other.effective_slot
            && self.deployment_slot == other.deployment_slot
            && self.is_tombstone() == other.is_tombstone()
    }
}

impl<'a, SDK: SharedAPI> ProgramCacheEntry<'a, SDK> {
    /// Creates a new user program
    pub fn new(
        loader_key: &Pubkey,
        program_runtime_environment: ProgramRuntimeEnvironment<'a, SDK>,
        deployment_slot: Slot,
        effective_slot: Slot,
        elf_bytes: &[u8],
        account_size: usize,
    ) -> Result<Self, Box<dyn core::error::Error>> {
        Self::new_internal(
            loader_key,
            program_runtime_environment,
            deployment_slot,
            effective_slot,
            elf_bytes,
            account_size,
            false,
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
        elf_bytes: &[u8],
        account_size: usize,
    ) -> Result<Self, Box<dyn core::error::Error>> {
        Self::new_internal(
            loader_key,
            program_runtime_environment,
            deployment_slot,
            effective_slot,
            elf_bytes,
            account_size,
            true,
        )
    }

    fn new_internal(
        loader_key: &Pubkey,
        program_runtime_environment: Arc<BuiltinProgram<InvokeContext<'a, SDK>>>,
        deployment_slot: Slot,
        effective_slot: Slot,
        elf_bytes: &[u8],
        account_size: usize,
        reloading: bool,
    ) -> Result<Self, Box<dyn core::error::Error>> {
        // The following unused_mut exception is needed for architectures that do not
        // support JIT compilation.
        #[allow(unused_mut)]
        let mut executable = Executable::load(elf_bytes, program_runtime_environment.clone())?;

        if !reloading {
            executable.verify::<RequisiteVerifier>()?;
        }

        Ok(Self {
            deployment_slot,
            account_owner: ProgramCacheEntryOwner::try_from(loader_key).unwrap(),
            account_size,
            effective_slot,
            program: ProgramCacheEntryType::Loaded(Arc::new(executable)),
        })
    }

    pub fn to_unloaded(&self) -> Option<Self> {
        match &self.program {
            ProgramCacheEntryType::Loaded(_) => {}
            ProgramCacheEntryType::FailedVerification(_)
            | ProgramCacheEntryType::Closed
            | ProgramCacheEntryType::DelayVisibility
            | ProgramCacheEntryType::Unloaded(_)
            | ProgramCacheEntryType::Builtin(_) => {
                return None;
            }
        }
        Some(Self {
            program: ProgramCacheEntryType::Unloaded(self.program.get_environment()?.clone()),
            account_owner: self.account_owner,
            account_size: self.account_size,
            deployment_slot: self.deployment_slot,
            effective_slot: self.effective_slot,
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
            account_owner: ProgramCacheEntryOwner::NativeLoader,
            account_size,
            effective_slot: deployment_slot,
            program: ProgramCacheEntryType::Builtin(Arc::new(BuiltinProgram::new_builtin(
                function_registry,
            ))),
        }
    }

    pub fn new_tombstone(
        slot: Slot,
        account_owner: ProgramCacheEntryOwner,
        reason: ProgramCacheEntryType<'a, SDK>,
    ) -> Self {
        let tombstone = Self {
            program: reason,
            account_owner,
            account_size: 0,
            deployment_slot: slot,
            effective_slot: slot,
        };
        debug_assert!(tombstone.is_tombstone());
        tombstone
    }

    pub fn is_tombstone(&self) -> bool {
        matches!(
            self.program,
            ProgramCacheEntryType::FailedVerification(_)
                | ProgramCacheEntryType::Closed
                | ProgramCacheEntryType::DelayVisibility
        )
    }

    // fn is_implicit_delay_visibility_tombstone(&self, slot: Slot) -> bool {
    //     !matches!(self.program, ProgramCacheEntryType::Builtin(_))
    //         && self.effective_slot.saturating_sub(self.deployment_slot)
    //             == DELAY_VISIBILITY_SLOT_OFFSET
    //         && slot >= self.deployment_slot
    //         && slot < self.effective_slot
    // }

    pub fn account_owner(&self) -> Pubkey {
        self.account_owner.into()
    }
}

/// Globally shared RBPF config and syscall registry
///
/// This is only valid in an epoch range as long as no feature affecting RBPF is activated.
#[derive(Clone)]
pub struct ProgramRuntimeEnvironments<'a, SDK: SharedAPI> {
    /// For program runtime V1
    pub program_runtime_v1: ProgramRuntimeEnvironment<'a, SDK>,
    /// For program runtime V2
    pub program_runtime_v2: ProgramRuntimeEnvironment<'a, SDK>,
}

impl<'a, SDK: SharedAPI> Default for ProgramRuntimeEnvironments<'a, SDK> {
    fn default() -> Self {
        let config = rbpf_config_default(None);
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

/// Local view into [ProgramCache] which was extracted for a specific TX batch.
///
/// This isolation enables the global [ProgramCache] to continue to evolve (e.g. evictions),
/// while the TX batch is guaranteed it will continue to find all the programs it requires.
/// For program management instructions this also buffers them before they are merged back into the
/// global [ProgramCache].
#[derive(Clone, Default)]
pub struct ProgramCacheForTxBatch<'a, SDK: SharedAPI> {
    /// Pubkey is the address of a program.
    /// ProgramCacheEntry is the corresponding program entry valid for the slot in which a
    /// transaction is being executed.
    entries: HashMap<Pubkey, Arc<ProgramCacheEntry<'a, SDK>>>,
    /// Program entries modified during the transaction batch.
    modified_entries: HashMap<Pubkey, Arc<ProgramCacheEntry<'a, SDK>>>,
    slot: Slot,
    pub environments: ProgramRuntimeEnvironments<'a, SDK>,
    /// Anticipated replacement for `environments` at the next epoch.
    ///
    /// This is `None` during most of an epoch, and only `Some` around the boundaries (at the end
    /// and beginning of an epoch). More precisely, it starts with the cache preparation phase
    /// a few hundred slots before the epoch boundary, and it ends with the first rerooting
    /// after the epoch boundary. Needed when a program is deployed at the last slot of an
    /// epoch, becomes effective in the next epoch. So needs to be compiled with the
    /// environment for the next epoch.
    pub upcoming_environments: Option<ProgramRuntimeEnvironments<'a, SDK>>,
    /// The epoch of the last rerooting
    pub latest_root_epoch: Epoch,
    pub hit_max_limit: bool,
    pub loaded_missing: bool,
    pub merged_modified: bool,
}

impl<'a, SDK: SharedAPI> ProgramCacheForTxBatch<'a, SDK> {
    pub fn new(
        slot: Slot,
        environments: ProgramRuntimeEnvironments<'a, SDK>,
        upcoming_environments: Option<ProgramRuntimeEnvironments<'a, SDK>>,
        latest_root_epoch: Epoch,
    ) -> Self {
        Self {
            entries: HashMap::new(),
            modified_entries: HashMap::new(),
            slot,
            environments,
            upcoming_environments,
            latest_root_epoch,
            hit_max_limit: false,
            loaded_missing: false,
            merged_modified: false,
        }
    }
    pub fn new2(slot: Slot, environments: ProgramRuntimeEnvironments<'a, SDK>) -> Self {
        Self::new(slot, environments, None, Epoch::default())
    }

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
        entry: Arc<ProgramCacheEntry<'a, SDK>>,
    ) -> (bool, Arc<ProgramCacheEntry<'a, SDK>>) {
        (self.entries.insert(key, entry.clone()).is_some(), entry)
    }

    /// Store an entry in `modified_entries` for a program modified during the
    /// transaction batch.
    pub fn store_modified_entry(&mut self, key: Pubkey, entry: Arc<ProgramCacheEntry<'a, SDK>>) {
        self.modified_entries.insert(key, entry);
    }

    /// Drain the program cache's modified entries, returning the owned
    /// collection.
    pub fn drain_modified_entries(&mut self) -> HashMap<Pubkey, Arc<ProgramCacheEntry<'a, SDK>>> {
        core::mem::take(&mut self.modified_entries)
    }

    pub fn find(&self, key: &Pubkey) -> Option<Arc<ProgramCacheEntry<'a, SDK>>> {
        self.modified_entries
            .get(key)
            .or(self.entries.get(key))
            .map(|entry| {
                // if entry.is_implicit_delay_visibility_tombstone(self.slot) {
                //     // Found a program entry on the current fork, but it's not effective
                //     // yet. It indicates that the program has delayed visibility. Return
                //     // the tombstone to reflect that.
                //     Arc::new(ProgramCacheEntry::new_tombstone(
                //         entry.deployment_slot,
                //         entry.account_owner,
                //         ProgramCacheEntryType::DelayVisibility,
                //     ))
                // } else {
                entry.clone()
                // }
            })
    }

    pub fn slot(&self) -> Slot {
        self.slot
    }

    pub fn set_slot_for_tests(&mut self, slot: Slot) {
        self.slot = slot;
    }

    pub fn merge(&mut self, modified_entries: &HashMap<Pubkey, Arc<ProgramCacheEntry<'a, SDK>>>) {
        modified_entries.iter().for_each(|(key, entry)| {
            self.merged_modified = true;
            self.replenish(*key, entry.clone());
        })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub enum ProgramCacheMatchCriteria {
    DeployedOnOrAfterSlot(Slot),
    Tombstone,
    NoCriteria,
}
