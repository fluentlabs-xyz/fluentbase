use crate::{
    account::AccountSharedData,
    clock::Slot,
    context::{InstructionContext, InvokeContext, TransactionContext},
    hash::Hash,
    loaded_programs::DELAY_VISIBILITY_SLOT_OFFSET,
    solana_program::loader_v4,
};
use alloc::{sync::Arc, vec, vec::Vec};
use core::marker::PhantomData;
use fluentbase_sdk::{keccak256, Address, ExitCode, MetadataStorageAPI, SharedAPI, U256};
use fluentbase_svm_common::common::{
    evm_balance_from_lamports, lamports_from_evm_balance, pubkey_to_u256,
};
use solana_instruction::error::InstructionError;
use solana_pubkey::{Pubkey, PUBKEY_BYTES, SVM_ADDRESS_PREFIX};
use solana_rbpf::{
    program::{BuiltinFunction, BuiltinProgram, FunctionRegistry},
    vm::Config,
};

/// Max instruction stack depth. This is the maximum nesting of instructions that can happen during
/// a transaction.
pub const MAX_INSTRUCTION_STACK_DEPTH: usize = 5;

/// Max call depth. This is the maximum nesting of SBF to SBF call that can happen within a program.
pub const MAX_CALL_DEPTH: usize = 64;

/// The size of one SBF stack frame.
pub const STACK_FRAME_SIZE: usize = 4096;

use crate::{
    compute_budget::compute_budget::ComputeBudget,
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

pub struct Sha256Hasher<SDK: SharedAPI> {
    _phantom: PhantomData<SDK>,
    data: Vec<u8>,
    hash: Option<Hash>,
}
impl<SDK: SharedAPI> HasherImpl for Sha256Hasher<SDK> {
    const NAME: &'static str = "Sha256";
    type Output = Hash;

    fn create_hasher() -> Self {
        Self {
            _phantom: Default::default(),
            data: vec![],
            hash: None,
        }
    }

    fn hash(&mut self, val: &[u8]) {
        self.data.extend_from_slice(val);
        self.hash = None;
    }

    fn result(mut self) -> Self::Output {
        if let Some(hash) = self.hash {
            return hash;
        }
        unimplemented!();
        // let hash: Hash = crypto_sha256(&self.data).0.into();
        // self.hash = Some(hash);
        // hash
    }
}

pub struct Keccak256Hasher<SDK: SharedAPI> {
    value: Option<[u8; 32]>,
    acc: Vec<u8>,
    _sdk: PhantomData<SDK>,
}
impl<SDK: SharedAPI> HasherImpl for Keccak256Hasher<SDK> {
    const NAME: &'static str = "Keccak256";
    type Output = [u8; 32];

    fn create_hasher() -> Self {
        Keccak256Hasher {
            value: Default::default(),
            acc: Default::default(),
            _sdk: Default::default(),
        }
    }

    fn hash(&mut self, val: &[u8]) {
        self.acc.extend_from_slice(val);
    }

    fn result(mut self) -> Self::Output {
        if let Some(val) = self.value {
            return val;
        }
        let result = keccak256(&self.acc).0;
        self.value = Some(result);
        result
    }
}

pub struct Blake3Hasher<SDK: SharedAPI> {
    _phantom: PhantomData<SDK>,
    data: Vec<u8>,
    hash: Option<[u8; 32]>,
}
impl<SDK: SharedAPI> HasherImpl for Blake3Hasher<SDK> {
    const NAME: &'static str = "Blake3";
    type Output = [u8; 32];

    fn create_hasher() -> Self {
        Blake3Hasher {
            _phantom: Default::default(),
            data: Default::default(),
            hash: Default::default(),
        }
    }

    fn hash(&mut self, val: &[u8]) {
        self.data.extend_from_slice(val);
        self.hash = None;
    }

    fn result(mut self) -> Self::Output {
        if let Some(hash) = self.hash {
            return hash;
        }
        unimplemented!();
        // let hash = crypto_blake3(&self.data).0;
        // self.hash = Some(hash);
        // hash
    }
}

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
    loader_v4::check_id(id)
}

pub fn rbpf_config_default(compute_budget: Option<&ComputeBudget>) -> Config {
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

pub struct GlobalLamportsBalance<SDK: MetadataStorageAPI> {
    _phantom_data: PhantomData<SDK>,
}

impl<API: MetadataStorageAPI> GlobalLamportsBalance<API> {
    pub fn new() -> Self {
        Self {
            _phantom_data: Default::default(),
        }
    }
    fn get_u256(sdk: &API, pk: &U256) -> U256 {
        sdk.metadata_storage_read(&pk)
            .expect("failed to read balance")
            .data
    }
    pub fn get(sdk: &API, pk: &Pubkey) -> u64 {
        let u256 = pubkey_to_u256(pk);
        let balance = Self::get_u256(sdk, &u256);
        lamports_from_evm_balance(balance)
    }
    fn set_u256(sdk: &mut API, pk: &U256, balance: U256) {
        let balance_current = sdk
            .metadata_storage_write(&pk, balance)
            .expect("failed to write balance");
    }
    pub fn set(sdk: &mut API, pk: &Pubkey, lamports: u64) {
        let u256 = pubkey_to_u256(pk);
        Self::set_u256(sdk, &u256, evm_balance_from_lamports(lamports))
    }
    pub fn change<const ADD_OR_SUB: bool>(
        sdk: &mut API,
        pk: &Pubkey,
        lamports_change: u64,
    ) -> Result<U256, SvmError> {
        let pk_u256 = pubkey_to_u256(pk);
        let balance_current = Self::get_u256(sdk, &pk_u256);
        if lamports_change == 0 {
            return Ok(balance_current);
        }
        let balance_change = evm_balance_from_lamports(lamports_change);

        let balance_new = if ADD_OR_SUB {
            balance_current.checked_add(balance_change)
        } else {
            balance_current.checked_sub(balance_change)
        };
        if let Some(balance_new) = balance_new {
            Self::set_u256(sdk, &pk_u256, balance_new);
            Ok(balance_new)
        } else {
            Err(ExitCode::IntegerOverflow.into())
        }
    }
    pub fn transfer(
        sdk: &mut API,
        pk_from: &Pubkey,
        pk_to: &Pubkey,
        lamports_change: u64,
    ) -> Result<(U256, U256), SvmError> {
        let pk_from_u256 = pubkey_to_u256(pk_from);
        let pk_to_u256 = pubkey_to_u256(pk_to);
        let balance_from_current = Self::get_u256(sdk, &pk_from_u256);
        let balance_to_current = Self::get_u256(sdk, &pk_to_u256);
        if lamports_change == 0 {
            return Ok((balance_from_current, balance_to_current));
        }
        let balance_change = evm_balance_from_lamports(lamports_change);

        let Some(balance_from_new) = balance_from_current.checked_sub(balance_change) else {
            return Err(ExitCode::IntegerOverflow.into());
        };
        let Some(balance_to_new) = balance_to_current.checked_add(balance_change) else {
            return Err(ExitCode::IntegerOverflow.into());
        };
        Self::set_u256(sdk, &pk_from_u256, balance_from_new);
        Self::set_u256(sdk, &pk_to_u256, balance_to_new);

        Ok((balance_from_new, balance_to_new))
    }
}

#[cfg(test)]
mod tests {
    use crate::common::{evm_balance_from_lamports, lamports_from_evm_balance};
    use fluentbase_sdk::U256;
    use fluentbase_svm_common::common::ONE_GWEI;

    #[test]
    fn test_evm_balance_to_lamports_and_back() {
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
