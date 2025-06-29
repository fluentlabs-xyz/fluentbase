// use crate::builtins::SyscallSecp256k1Recover;
// use crate::solana_program::bpf_loader;
// use crate::solana_program::blake3;
// use crate::solana_program::bpf_loader_deprecated;
use crate::{
    account::AccountSharedData,
    bpf_loader,
    bpf_loader_deprecated,
    builtins::{
        SyscallAbort,
        SyscallCreateProgramAddress,
        SyscallHash,
        SyscallLog,
        SyscallMemcpy,
        SyscallMemmove,
        SyscallMemset,
        SyscallPanic,
        // SyscallPoseidon,
        SyscallTryFindProgramAddress,
    },
    context::{InstructionContext, InvokeContext, TransactionContext},
    loaded_programs::DELAY_VISIBILITY_SLOT_OFFSET,
};
use crate::{
    clock::Slot,
    hash::{Hash, Hasher},
    solana_program::loader_v4,
};
use alloc::{sync::Arc, vec, vec::Vec};
use core::marker::PhantomData;
use fluentbase_sdk::{Address, SharedAPI, U256};
use solana_bincode::limited_deserialize;
use solana_feature_set::{
    bpf_account_data_direct_mapping,
    error_on_syscall_bpf_function_hash_collisions,
    reject_callx_r10,
    FeatureSet,
};
use solana_instruction::error::InstructionError;
use solana_pubkey::{Pubkey, SVM_ADDRESS_PREFIX};
use solana_rbpf::{
    program::{BuiltinFunction, BuiltinProgram, FunctionRegistry},
    vm::Config,
};

pub const DEFAULT_LOADER_COMPUTE_UNITS: u64 = 570;
pub const DEPRECATED_LOADER_COMPUTE_UNITS: u64 = 1_140;
pub const UPGRADEABLE_LOADER_COMPUTE_UNITS: u64 = 2_370;
/// Maximum over-the-wire size of a Transaction
///   1280 is IPv6 minimum MTU
///   40 bytes is the size of the IPv6 header
///   8 bytes is the size of the fragment header
pub const PACKET_DATA_SIZE: usize = 1280 - 40 - 8;

#[cfg(target_arch = "wasm32")]
#[inline(always)]
pub fn keccak256(input: &[u8]) -> B256 {
    #[link(wasm_import_module = "fluentbase_v1preview")]
    extern "C" {
        fn _keccak256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    }
    let mut result = B256::ZERO;
    unsafe {
        _keccak256(input.as_ptr(), input.len() as u32, result.as_mut_ptr());
    }
    result
}

#[cfg(not(target_arch = "wasm32"))]
pub fn keccak256(data: &[u8]) -> B256 {
    use keccak_hash::keccak;
    B256::new(keccak(data).0)
}

// #[cfg(target_pointer_width = "64")]
// pub(crate) type PtrSizedType = u64;
// #[cfg(target_pointer_width = "32")]
// pub(crate) type PtrSizedType = u32;

use crate::{
    compute_budget::compute_budget::ComputeBudget,
    error::{Error, RuntimeError, SvmError},
    loaded_programs::ProgramCacheEntry,
    solana_program::{bpf_loader_upgradeable, bpf_loader_upgradeable::UpgradeableLoaderState},
};
#[cfg(test)]
use fluentbase_sdk_testing::HostTestingContext;
use fluentbase_types::B256;

#[cfg(test)]
pub type TestSdkType = HostTestingContext;

pub trait HasherImpl {
    const NAME: &'static str;
    type Output: AsRef<[u8]>;

    fn create_hasher() -> Self;
    fn hash(&mut self, val: &[u8]);
    fn result(self) -> Self::Output;
}

pub struct Sha256Hasher(Hasher);
impl HasherImpl for Sha256Hasher {
    const NAME: &'static str = "Sha256";
    type Output = Hash;

    fn create_hasher() -> Self {
        Sha256Hasher(Hasher::default())
    }

    fn hash(&mut self, val: &[u8]) {
        self.0.hash(val);
    }

    fn result(self) -> Self::Output {
        self.0.result()
    }
}

// pub struct Keccak256Hasher(keccak::Hasher);
//
// impl HasherImpl for Keccak256Hasher {
//     const NAME: &'static str = "Keccak256";
//     type Output = keccak::Hash;
//
//     fn create_hasher() -> Self {
//         Keccak256Hasher(keccak::Hasher::default())
//     }
//
//     fn hash(&mut self, val: &[u8]) {
//         self.0.hash(val);
//     }
//
//     fn result(self) -> Self::Output {
//         self.0.result()
//     }
// }

pub struct Keccak256Hasher<SDK: SharedAPI> {
    initiated: bool,
    value: [u8; 32],
    _sdk: PhantomData<SDK>,
}
impl<SDK: SharedAPI> HasherImpl for Keccak256Hasher<SDK> {
    const NAME: &'static str = "Keccak256";
    type Output = [u8; 32];

    fn create_hasher() -> Self {
        Keccak256Hasher {
            initiated: false,
            value: Default::default(),
            _sdk: Default::default(),
        }
    }

    fn hash(&mut self, val: &[u8]) {
        if self.initiated {
            panic!("accumulation not supported yet")
        } else {
            self.value = keccak256(val).0;
            self.value = Default::default();
            self.initiated = true;
        }
    }

    fn result(self) -> Self::Output {
        self.value
    }
}

pub struct PoseidonHasher<SDK> {
    initiated: bool,
    value: [u8; 32],
    _sdk: PhantomData<SDK>,
}
impl<SDK: SharedAPI> HasherImpl for PoseidonHasher<SDK> {
    const NAME: &'static str = "Poseidon";
    type Output = [u8; 32];

    fn create_hasher() -> Self {
        PoseidonHasher {
            initiated: false,
            value: Default::default(),
            _sdk: Default::default(),
        }
    }

    fn hash(&mut self, _val: &[u8]) {
        if self.initiated {
            panic!("accumulation not supported yet")
        } else {
            // self.value = SDK::poseidon(val).0;
            // TODO
            self.value = Default::default();
            self.initiated = true;
        }
    }

    fn result(self) -> Self::Output {
        self.value
    }
}

// pub struct Blake3Hasher(blake3::Hasher);
// impl HasherImpl for Blake3Hasher {
//     const NAME: &'static str = "Blake3";
//     type Output = blake3::Hash;
//
//     fn create_hasher() -> Self {
//         Blake3Hasher(blake3::Hasher::default())
//     }
//
//     fn hash(&mut self, val: &[u8]) {
//         self.0.hash(val);
//     }
//
//     fn result(self) -> Self::Output {
//         self.0.result()
//     }
// }

// declare_id!("NativeLoader1111111111111111111111111111111");
//
// pub fn create_loadable_account_with_fields(
//     name: &str,
//     owner: Pubkey,
//     (lamports, rent_epoch): InheritableAccountFields,
// ) -> AccountSharedData {
//     Account {
//         lamports,
//         owner,
//         data: name.as_bytes().to_vec(),
//         executable: true,
//         rent_epoch,
//     }
//         .into()
// }
//
// pub fn create_loadable_account_for_test(name: &str, owner: Pubkey) -> AccountSharedData {
//     create_loadable_account_with_fields(name, owner, DUMMY_INHERITABLE_ACCOUNT_FIELDS)
// }

// macro_rules! register_feature_gated_function {
//     ($result:expr, $is_feature_active:expr, $name:expr, $call:expr $(,)?) => {
//         if $is_feature_active {
//             $result.register_function_hashed($name, $call)
//         } else {
//             Ok(0)
//         }
//     };
// }

pub fn morph_into_deployment_environment_v1<'a, SDK: SharedAPI>(
    from: Arc<BuiltinProgram<InvokeContext<'a, SDK>>>,
) -> Result<BuiltinProgram<InvokeContext<'a, SDK>>, Error> {
    let mut config = *from.get_config();
    config.reject_broken_elfs = true;

    let mut result = FunctionRegistry::<BuiltinFunction<InvokeContext<'a, SDK>>>::default();

    for (key, (name, value)) in from.get_function_registry().iter() {
        // Deployment of programs with sol_alloc_free is disabled. So do not register the syscall.
        if name != *b"sol_alloc_free_" {
            result.register_function(key, name, value)?;
        }
    }

    Ok(BuiltinProgram::new_loader(config, result))
}

pub fn create_program_runtime_environment_v1<'a, SDK: SharedAPI>(
    feature_set: &FeatureSet,
    compute_budget: &ComputeBudget,
    reject_deployment_of_broken_elfs: bool,
    debugging_features: bool,
) -> Result<BuiltinProgram<InvokeContext<'a, SDK>>, Error> {
    // let enable_alt_bn128_syscall = feature_set.is_active(&enable_alt_bn128_syscall::id());
    // let enable_alt_bn128_compression_syscall =
    //     feature_set.is_active(&enable_alt_bn128_compression_syscall::id());
    // let enable_big_mod_exp_syscall = feature_set.is_active(&enable_big_mod_exp_syscall::id());
    // let blake3_syscall_enabled = feature_set.is_active(&blake3_syscall_enabled::id());
    // let curve25519_syscall_enabled = feature_set.is_active(&curve25519_syscall_enabled::id());
    // let disable_fees_sysvar = feature_set.is_active(&disable_fees_sysvar::id());
    // let epoch_rewards_syscall_enabled =
    //     feature_set.is_active(&enable_partitioned_epoch_reward::id());
    // let disable_deploy_of_alloc_free_syscall = reject_deployment_of_broken_elfs
    //     && feature_set.is_active(&disable_deploy_of_alloc_free_syscall::id());
    // let last_restart_slot_syscall_enabled =
    // feature_set.is_active(&last_restart_slot_sysvar::id()); let enable_poseidon_syscall =
    // feature_set.is_active(&enable_poseidon_syscall::id());
    // let remaining_compute_units_syscall_enabled =
    //     feature_set.is_active(&remaining_compute_units_syscall_enabled::id());
    // !!! ATTENTION !!!
    // When adding new features for RBPF here,
    // also add them to `Bank::apply_builtin_program_feature_transitions()`.

    let config = Config {
        max_call_depth: compute_budget.max_call_depth,
        stack_frame_size: compute_budget.stack_frame_size,
        enable_address_translation: true,
        enable_stack_frame_gaps: !feature_set.is_active(&bpf_account_data_direct_mapping::id()),
        instruction_meter_checkpoint_distance: 10000,
        enable_instruction_meter: true,
        enable_instruction_tracing: debugging_features,
        enable_symbol_and_section_labels: debugging_features,
        reject_broken_elfs: reject_deployment_of_broken_elfs,
        noop_instruction_rate: 256,
        sanitize_user_provided_values: true,
        external_internal_function_hash_collision: feature_set
            .is_active(&error_on_syscall_bpf_function_hash_collisions::id()),
        reject_callx_r10: feature_set.is_active(&reject_callx_r10::id()),
        enable_sbpf_v1: true,
        enable_sbpf_v2: false,
        optimize_rodata: false,
        aligned_memory_mapping: !feature_set.is_active(&bpf_account_data_direct_mapping::id()),
        // Warning, do not use `Config::default()` so that configuration here is explicit.
    };
    let mut result = FunctionRegistry::<BuiltinFunction<InvokeContext<SDK>>>::default();

    // Abort
    result.register_function_hashed(*b"abort", SyscallAbort::vm)?;

    // Panic
    result.register_function_hashed(*b"sol_panic_", SyscallPanic::vm)?;

    // Logging
    result.register_function_hashed(*b"sol_log_", SyscallLog::vm)?;
    // result.register_function_hashed(*b"sol_log_64_", SyscallLogU64::vm)?;
    // result.register_function_hashed(*b"sol_log_compute_units_", SyscallLogBpfComputeUnits::vm)?;
    // result.register_function_hashed(*b"sol_log_pubkey", SyscallLogPubkey::vm)?;

    // Program defined addresses (PDA)
    result.register_function_hashed(
        *b"sol_create_program_address",
        SyscallCreateProgramAddress::vm,
    )?;
    result.register_function_hashed(
        *b"sol_try_find_program_address",
        SyscallTryFindProgramAddress::vm,
    )?;

    // Sha256
    result.register_function_hashed(*b"sol_sha256", SyscallHash::vm::<SDK, Sha256Hasher>)?;

    // Keccak256
    result.register_function_hashed(
        *b"sol_keccak256",
        SyscallHash::vm::<SDK, Keccak256Hasher<SDK>>,
    )?;

    // Secp256k1 Recover
    // result.register_function_hashed(*b"sol_secp256k1_recover", SyscallSecp256k1Recover::vm)?;

    // // Blake3
    // register_feature_gated_function!(
    //     result,
    //     blake3_syscall_enabled,
    //     *b"sol_blake3",
    //     SyscallHash::vm::<SDK, Blake3Hasher>,
    // )?;

    // Elliptic Curve Operations
    // register_feature_gated_function!(
    //     result,
    //     curve25519_syscall_enabled,
    //     *b"sol_curve_validate_point",
    //     SyscallCurvePointValidation::vm,
    // )?;
    // register_feature_gated_function!(
    //     result,
    //     curve25519_syscall_enabled,
    //     *b"sol_curve_group_op",
    //     SyscallCurveGroupOps::vm,
    // )?;
    // register_feature_gated_function!(
    //     result,
    //     curve25519_syscall_enabled,
    //     *b"sol_curve_multiscalar_mul",
    //     SyscallCurveMultiscalarMultiplication::vm,
    // )?;

    // Sysvars
    // result.register_function_hashed(*b"sol_get_clock_sysvar", SyscallGetClockSysvar::vm)?;
    // result.register_function_hashed(
    //     *b"sol_get_epoch_schedule_sysvar",
    //     SyscallGetEpochScheduleSysvar::vm,
    // )?;
    // register_feature_gated_function!(
    //     result,
    //     !disable_fees_sysvar,
    //     *b"sol_get_fees_sysvar",
    //     SyscallGetFeesSysvar::vm,
    // )?;
    // result.register_function_hashed(*b"sol_get_rent_sysvar", SyscallGetRentSysvar::vm)?;

    // register_feature_gated_function!(
    //     result,
    //     last_restart_slot_syscall_enabled,
    //     *b"sol_get_last_restart_slot",
    //     SyscallGetLastRestartSlotSysvar::vm,
    // )?;

    // register_feature_gated_function!(
    //     result,
    //     epoch_rewards_syscall_enabled,
    //     *b"sol_get_epoch_rewards_sysvar",
    //     SyscallGetEpochRewardsSysvar::vm,
    // )?;

    // Memory ops
    result.register_function_hashed(*b"sol_memcpy_", SyscallMemcpy::vm)?;
    result.register_function_hashed(*b"sol_memmove_", SyscallMemmove::vm)?;
    // result.register_function_hashed(*b"sol_memcmp_", SyscallMemcmp::vm)?;
    result.register_function_hashed(*b"sol_memset_", SyscallMemset::vm)?;

    // Processed sibling instructions
    // result.register_function_hashed(
    //     *b"sol_get_processed_sibling_instruction",
    //     SyscallGetProcessedSiblingInstruction::vm,
    // )?;

    // Stack height
    // result.register_function_hashed(*b"sol_get_stack_height", SyscallGetStackHeight::vm)?;

    // Return data
    // result.register_function_hashed(*b"sol_set_return_data", SyscallSetReturnData::vm)?;
    // result.register_function_hashed(*b"sol_get_return_data", SyscallGetReturnData::vm)?;

    // Cross-program invocation
    // result.register_function_hashed(*b"sol_invoke_signed_c", SyscallInvokeSignedC::vm)?;
    // result.register_function_hashed(*b"sol_invoke_signed_rust", SyscallInvokeSignedRust::vm)?;

    // Memory allocator
    // register_feature_gated_function!(
    //     result,
    //     !disable_deploy_of_alloc_free_syscall,
    //     *b"sol_alloc_free_",
    //     SyscallAllocFree::vm,
    // )?;

    // Alt_bn128
    // register_feature_gated_function!(
    //     result,
    //     enable_alt_bn128_syscall,
    //     *b"sol_alt_bn128_group_op",
    //     SyscallAltBn128::vm,
    // )?;

    // Big_mod_exp
    // register_feature_gated_function!(
    //     result,
    //     enable_big_mod_exp_syscall,
    //     *b"sol_big_mod_exp",
    //     SyscallBigModExp::vm,
    // )?;

    // Poseidon
    // register_feature_gated_function!(
    //     result,
    //     enable_poseidon_syscall,
    //     *b"sol_poseidon",
    //     SyscallPoseidon::vm,
    // )?;

    // Accessing remaining compute units
    // register_feature_gated_function!(
    //     result,
    //     remaining_compute_units_syscall_enabled,
    //     *b"sol_remaining_compute_units",
    //     SyscallRemainingComputeUnits::vm
    // )?;

    // Alt_bn128_compression
    // register_feature_gated_function!(
    //     result,
    //     enable_alt_bn128_compression_syscall,
    //     *b"sol_alt_bn128_compression",
    //     SyscallAltBn128Compression::vm,
    // )?;

    // Log data
    // result.register_function_hashed(*b"sol_log_data", SyscallLogData::vm)?;

    Ok(BuiltinProgram::new_loader(config, result))
}

pub fn check_loader_id(id: &Pubkey) -> bool {
    loader_v4::check_id(id)
        || bpf_loader::check_id(id)
        || bpf_loader_deprecated::check_id(id)
        || bpf_loader_upgradeable::check_id(id)
}

pub fn load_program_from_bytes<'a, SDK: SharedAPI>(
    // log_collector: Option<Rc<RefCell<LogCollector>>>,
    // load_program_metrics: &mut LoadProgramMetrics,
    programdata: &[u8],
    loader_key: &Pubkey,
    account_size: usize,
    deployment_slot: Slot,
    program_runtime_environment: Arc<BuiltinProgram<InvokeContext<'a, SDK>>>,
    reloading: bool,
) -> Result<ProgramCacheEntry<'a, SDK>, InstructionError> {
    let effective_slot = deployment_slot.saturating_add(DELAY_VISIBILITY_SLOT_OFFSET);
    let loaded_program = if reloading {
        // Safety: this is safe because the program is being reloaded in the cache.
        unsafe {
            ProgramCacheEntry::reload(
                loader_key,
                program_runtime_environment,
                deployment_slot,
                effective_slot,
                // None,
                programdata,
                account_size,
                // load_program_metrics,
            )
        }
    } else {
        ProgramCacheEntry::new(
            loader_key,
            program_runtime_environment,
            deployment_slot,
            effective_slot,
            // None,
            programdata,
            account_size,
            // load_program_metrics,
        )
    }
    .map_err(|_err| {
        // ic_logger_msg!(log_collector, "{}", err);
        InstructionError::InvalidAccountData
    })?;
    Ok(loaded_program)
}

#[macro_export]
macro_rules! deploy_program {
    ($invoke_context:expr, $program_id:expr, $loader_key:expr,
     $account_size:expr, $slot:expr, $drop:expr, $new_programdata:expr $(,)?) => {{
        use crate::loaded_programs::DELAY_VISIBILITY_SLOT_OFFSET;
        use solana_rbpf::elf::Executable;
        use solana_rbpf::verifier::RequisiteVerifier;
        use crate::common::load_program_from_bytes;
        use crate::common::morph_into_deployment_environment_v1;
        use core::sync::atomic::Ordering;
        use crate::clock::Slot;

        // let mut load_program_metrics = LoadProgramMetrics::default();
        // let mut register_syscalls_time = Measure::start("register_syscalls_time");
        let deployment_slot: Slot = $slot;
        let environments = $invoke_context.get_environments_for_slot(
            deployment_slot.saturating_add(DELAY_VISIBILITY_SLOT_OFFSET)
        ).map_err(|_e| {
            // This will never fail since the epoch schedule is already configured.
            // ic_msg!($invoke_context, "Failed to get runtime environment: {}", e);
            InstructionError::ProgramEnvironmentSetupFailure
        })?;
        let deployment_program_runtime_environment = morph_into_deployment_environment_v1(
            environments.program_runtime_v1.clone(),
        ).map_err(|_e| {
            // ic_msg!($invoke_context, "Failed to register syscalls: {}", e);
            InstructionError::ProgramEnvironmentSetupFailure
        })?;
        // register_syscalls_time.stop();
        // load_program_metrics.register_syscalls_us = register_syscalls_time.as_us();
        // Verify using stricter deployment_program_runtime_environment
        // let mut load_elf_time = Measure::start("load_elf_time");
        let executable = Executable::<InvokeContext<_>>::load(
            $new_programdata,
            Arc::new(deployment_program_runtime_environment),
        ).map_err(|_err| {
            // ic_logger_msg!($invoke_context.get_log_collector(), "{}", err);
            InstructionError::InvalidAccountData
        });
        let executable = executable?;
        // load_elf_time.stop();
        // load_program_metrics.load_elf_us = load_elf_time.as_us();
        // let mut verify_code_time = Measure::start("verify_code_time");
        executable.verify::<RequisiteVerifier>().map_err(|_err| {
            // ic_logger_msg!($invoke_context.get_log_collector(), "{}", err);
            InstructionError::InvalidAccountData
        })?;
        // verify_code_time.stop();
        // load_program_metrics.verify_code_us = verify_code_time.as_us();
        // Reload but with environments.program_runtime_v1
        let executor = load_program_from_bytes(
            // $invoke_context.get_log_collector(),
            // &mut load_program_metrics,
            $new_programdata,
            $loader_key,
            $account_size,
            $slot,
            environments.program_runtime_v1.clone(),
            true,
        )?;
        if let Some(old_entry) = $invoke_context.program_cache_for_tx_batch.find(&$program_id) {
            executor.tx_usage_counter.store(
                old_entry.tx_usage_counter.load(Ordering::Relaxed),
                Ordering::Relaxed
            );
            executor.ix_usage_counter.store(
                old_entry.ix_usage_counter.load(Ordering::Relaxed),
                Ordering::Relaxed
            );
        }
        $drop
        // load_program_metrics.program_id = $program_id.to_string();
        // load_program_metrics.submit_datapoint(&mut $invoke_context.timings);
        $invoke_context.program_cache_for_tx_batch.replenish($program_id, Arc::new(executor));
    }};
}

pub fn common_close_account(
    authority_address: &Option<Pubkey>,
    transaction_context: &TransactionContext,
    instruction_context: &InstructionContext,
    // log_collector: &Option<Rc<RefCell<LogCollector>>>,
) -> Result<(), InstructionError> {
    if authority_address.is_none() {
        // ic_logger_msg!(log_collector, "Account is immutable");
        return Err(InstructionError::Immutable);
    }
    if *authority_address
        != Some(*transaction_context.get_key_of_account_at_index(
            instruction_context.get_index_of_instruction_account_in_transaction(2)?,
        )?)
    {
        // ic_logger_msg!(log_collector, "Incorrect authority provided");
        return Err(InstructionError::IncorrectAuthority);
    }
    if !instruction_context.is_instruction_account_signer(2)? {
        // ic_logger_msg!(log_collector, "Authority did not sign");
        return Err(InstructionError::MissingRequiredSignature);
    }

    let mut close_account =
        instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
    let mut recipient_account =
        instruction_context.try_borrow_instruction_account(transaction_context, 1)?;

    recipient_account.checked_add_lamports(close_account.get_lamports())?;
    close_account.set_lamports(0)?;
    close_account.set_state(&UpgradeableLoaderState::Uninitialized)?;
    Ok(())
}

// /// Deserialize with a limit based the maximum amount of data a program can expect to get.
// /// This function should be used in place of direct deserialization to help prevent OOM errors
// pub fn limited_deserialize<T, const LIMIT: usize>(
//     instruction_data: &[u8],
// ) -> Result<T, InstructionError>
// where
//     T: serde::de::DeserializeOwned,
// {
//     BINCODE_DEFAULT_CONFIG
//         .with_limit::<LIMIT>()
//         .with_fixint_encoding() // As per https://github.com/servo/bincode/issues/333, these two options are needed
//         .allow_trailing_bytes() // to retain the behavior of bincode_deserialize with the new
// `options()` method         .deserialize_from(instruction_data)
//         .map_err(|_| InstructionError::InvalidInstructionData)
// }

/// Deserialize with a limit based the maximum amount of data a program can expect to get.
/// This function should be used in place of direct deserialization to help prevent OOM errors
pub fn limited_deserialize_packet_size<T: bincode::de::Decode<()>>(
    instruction_data: &[u8],
) -> Result<T, InstructionError>
where
    T: serde::de::DeserializeOwned,
{
    limited_deserialize::<PACKET_DATA_SIZE, _>(instruction_data)
        .map_err(|_| InstructionError::InvalidInstructionData)
}

pub fn write_program_data<SDK: SharedAPI>(
    program_data_offset: usize,
    bytes: &[u8],
    invoke_context: &mut InvokeContext<SDK>,
) -> Result<(), InstructionError> {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let mut program = instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
    let data = program.get_data_mut()?;
    let write_offset = program_data_offset.saturating_add(bytes.len());
    if data.len() < write_offset {
        // ic_msg!(
        //     invoke_context,
        //     "Write overflow: {} < {}",
        //     data.len(),
        //     write_offset,
        // );
        return Err(InstructionError::AccountDataTooSmall);
    }
    data.get_mut(program_data_offset..write_offset)
        .ok_or(InstructionError::AccountDataTooSmall)?
        .copy_from_slice(bytes);
    Ok(())
}

/// Addition that returns [`InstructionError::InsufficientFunds`] on overflow.
///
/// This is an internal utility function.
#[doc(hidden)]
pub fn checked_add(a: u64, b: u64) -> Result<u64, InstructionError> {
    a.checked_add(b).ok_or(InstructionError::InsufficientFunds)
}

pub fn calculate_max_chunk_size<F>(_create_msg: &F) -> usize
where
    F: Fn(u32, Vec<u8>) -> crate::solana_program::message::legacy::Message,
{
    // let baseline_msg = create_msg(0, Vec::new());
    // let tx_size = bincode_serialized_size(&Transaction {
    //     signatures: vec![
    //         solana_sdk::Signature::default();
    //         baseline_msg.header.num_required_signatures as usize
    //     ],
    //     message: baseline_msg,
    // })
    //     .unwrap() as usize;
    // add 1 byte buffer to account for shortvec encoding
    // PACKET_DATA_SIZE
    //     .saturating_sub(tx_size)
    //     .saturating_sub(1)
    // heuristic calculation
    PACKET_DATA_SIZE
        // .saturating_sub(tx_size)
        .saturating_sub(16)
}

pub fn compile_accounts_for_tx_ctx(
    working_accounts: Vec<(Pubkey, AccountSharedData)>,
    program_accounts: Vec<(Pubkey, AccountSharedData)>,
) -> (Vec<(Pubkey, AccountSharedData)>, u16) {
    let working_accounts_len = working_accounts.len() as u16;
    let mut accounts = vec![];
    accounts.extend(working_accounts);
    accounts.extend(program_accounts);

    (accounts, working_accounts_len)
}

pub fn pubkey_from_evm_address(value: &Address) -> Pubkey {
    let mut new_pk = [0u8; 32];
    new_pk[0..SVM_ADDRESS_PREFIX.len()].copy_from_slice(&SVM_ADDRESS_PREFIX);
    new_pk[SVM_ADDRESS_PREFIX.len()..].copy_from_slice(value.as_slice());
    Pubkey::new_from_array(new_pk)
}

#[inline(always)]
pub fn is_evm_pubkey(pk: &Pubkey) -> bool {
    pk.as_ref().starts_with(&SVM_ADDRESS_PREFIX)
}

pub fn evm_address_from_pubkey<const VALIDATE_PREFIX: bool>(
    pk: &Pubkey,
) -> Result<Address, SvmError> {
    if VALIDATE_PREFIX && !is_evm_pubkey(pk) {
        return Err(SvmError::RuntimeError(RuntimeError::InvalidPrefix));
    }
    Ok(Address::from_slice(
        &pk.as_ref()[SVM_ADDRESS_PREFIX.len()..],
    ))
}

const SIZE_OF_U64: usize = size_of::<u64>();
const ONE_GWEI: u64 = 1_000_000_000;
pub fn lamports_from_evm_balance(value: U256) -> u64 {
    let value = value / U256::from(ONE_GWEI);
    let bytes: [u8; SIZE_OF_U64] = value.to_be_bytes::<{ U256::BYTES }>().as_ref()
        [U256::BYTES - SIZE_OF_U64..U256::BYTES]
        .try_into()
        .unwrap();
    u64::from_be_bytes(bytes)
}

pub fn evm_balance_from_lamports(value: u64) -> U256 {
    let mut bytes = [0u8; U256::BYTES];
    bytes[U256::BYTES - SIZE_OF_U64..U256::BYTES].copy_from_slice(&value.to_be_bytes());
    U256::from_be_bytes(bytes) * U256::from(ONE_GWEI)
}

#[cfg(test)]
mod tests {
    use crate::common::{evm_balance_from_lamports, lamports_from_evm_balance, ONE_GWEI};
    use fluentbase_sdk::U256;

    #[test]
    fn test_evm_balance_to_lamports_and_vice_versa() {
        let evm_balance = U256::from(ONE_GWEI);
        let lamports_balance = lamports_from_evm_balance(evm_balance);
        assert_eq!(lamports_balance, 1);
        let evm_balance = U256::from(9 * ONE_GWEI);
        let lamports_balance = lamports_from_evm_balance(evm_balance);
        assert_eq!(lamports_balance, 9);
        let evm_balance = U256::from(1_000_000_000 * ONE_GWEI);
        let lamports_balance = lamports_from_evm_balance(evm_balance);
        assert_eq!(lamports_balance, ONE_GWEI);
        let evm_balance = U256::from(101e9);
        let lamports_balance = lamports_from_evm_balance(evm_balance);
        assert_eq!(lamports_balance, 101);

        let lamports_balance = 1;
        let evm_balance = evm_balance_from_lamports(lamports_balance);
        assert_eq!(evm_balance, U256::from(ONE_GWEI));
        let lamports_balance = 3;
        let evm_balance = evm_balance_from_lamports(lamports_balance);
        assert_eq!(evm_balance, U256::from(3 * ONE_GWEI));
        let lamports_balance = 1_000_000_000;
        let evm_balance = evm_balance_from_lamports(lamports_balance);
        assert_eq!(evm_balance, U256::from(1_000_000_000 * ONE_GWEI));
    }
}
