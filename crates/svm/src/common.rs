use crate::{
    account::AccountSharedData,
    bpf_loader,
    bpf_loader_deprecated,
    clock::Slot,
    context::{InstructionContext, InvokeContext, TransactionContext},
    hash::{Hash, Hasher},
    loaded_programs::DELAY_VISIBILITY_SLOT_OFFSET,
    solana_program::loader_v4,
};
use alloc::{sync::Arc, vec, vec::Vec};
use core::marker::PhantomData;
use fluentbase_sdk::{keccak256, Address, SharedAPI, U256};
use solana_bincode::limited_deserialize;
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

use crate::{
    compute_budget::compute_budget::{ComputeBudget, MAX_CALL_DEPTH, STACK_FRAME_SIZE},
    error::{Error, RuntimeError, SvmError},
    loaded_programs::ProgramCacheEntry,
};

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

pub fn check_loader_id(id: &Pubkey) -> bool {
    loader_v4::check_id(id) || bpf_loader::check_id(id) || bpf_loader_deprecated::check_id(id)
}

pub fn rbpf_config_default(compute_budget: Option<&ComputeBudget>) -> Config {
    // TODO validate all config variables usages
    Config {
        enable_instruction_tracing: false,
        reject_broken_elfs: true,
        sanitize_user_provided_values: true,
        enable_instruction_meter: true,
        max_call_depth: compute_budget.map_or_else(|| MAX_CALL_DEPTH, |v| v.max_call_depth),
        stack_frame_size: compute_budget.map_or_else(|| STACK_FRAME_SIZE, |v| v.stack_frame_size),
        ..Default::default()
    }
}

pub fn load_program_from_bytes<'a, SDK: SharedAPI>(
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
                programdata,
                account_size,
            )
        }
    } else {
        ProgramCacheEntry::new(
            loader_key,
            program_runtime_environment,
            deployment_slot,
            effective_slot,
            programdata,
            account_size,
        )
    }
    .map_err(|_err| InstructionError::InvalidAccountData)?;
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

        let deployment_slot: Slot = $slot;
        let environments = $invoke_context.get_environments_for_slot(
            deployment_slot.saturating_add(DELAY_VISIBILITY_SLOT_OFFSET)
        ).map_err(|_e| {
            // This will never fail since the epoch schedule is already configured.
            InstructionError::ProgramEnvironmentSetupFailure
        })?;
        let deployment_program_runtime_environment = morph_into_deployment_environment_v1(
            environments.program_runtime_v1.clone(),
        ).map_err(|_e| {
            InstructionError::ProgramEnvironmentSetupFailure
        })?;
        // Verify using stricter deployment_program_runtime_environment
        let executable = Executable::<InvokeContext<_>>::load(
            $new_programdata,
            Arc::new(deployment_program_runtime_environment),
        ).map_err(|_err| {
            InstructionError::InvalidAccountData
        });
        let executable = executable?;
        executable.verify::<RequisiteVerifier>().map_err(|_err| {
            InstructionError::InvalidAccountData
        })?;
        // Reload but with environments.program_runtime_v1
        let executor = load_program_from_bytes(
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
        $invoke_context.program_cache_for_tx_batch.replenish($program_id, Arc::new(executor));
    }};
}

pub fn common_close_account(
    authority_address: &Option<Pubkey>,
    transaction_context: &TransactionContext,
    instruction_context: &InstructionContext,
) -> Result<(), InstructionError> {
    if authority_address.is_none() {
        return Err(InstructionError::Immutable);
    }
    if *authority_address
        != Some(*transaction_context.get_key_of_account_at_index(
            instruction_context.get_index_of_instruction_account_in_transaction(2)?,
        )?)
    {
        return Err(InstructionError::IncorrectAuthority);
    }
    if !instruction_context.is_instruction_account_signer(2)? {
        return Err(InstructionError::MissingRequiredSignature);
    }

    let mut close_account =
        instruction_context.try_borrow_instruction_account(transaction_context, 0)?;
    let mut recipient_account =
        instruction_context.try_borrow_instruction_account(transaction_context, 1)?;

    recipient_account.checked_add_lamports(close_account.get_lamports())?;
    close_account.set_lamports(0)?;
    Ok(())
}

/// Deserialize with a limit based the maximum amount of data a program can expect to get.
/// This function should be used in place of direct deserialization to help prevent OOM errors
pub fn limited_deserialize_packet_size<T>(instruction_data: &[u8]) -> Result<T, InstructionError>
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
    PACKET_DATA_SIZE
        // TODO fix magic constant
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
