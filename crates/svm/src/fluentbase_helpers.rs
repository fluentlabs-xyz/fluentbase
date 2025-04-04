use crate::{
    account::{AccountSharedData, ReadableAccount, WritableAccount},
    bpf_loader_upgradable,
    builtins::register_builtins,
    common::{compile_accounts_for_tx_ctx, evm_address_from_pubkey},
    compute_budget::ComputeBudget,
    context::{IndexOfAccount, InvokeContext, TransactionContext},
    error::{InstructionError, TransactionError},
    feature_set::FeatureSet,
    loaded_programs::{LoadedProgram, LoadedProgramsForTxBatch, ProgramRuntimeEnvironments},
    message_processor::MessageProcessor,
    native_loader,
    storage_helpers::{ContractPubkeyHelper, StorageChunksWriter, VariableLengthDataWriter},
    system_processor,
    sysvar_cache::SysvarCache,
};
use alloc::{rc::Rc, sync::Arc, vec, vec::Vec};
use fluentbase_sdk::{
    bytes::Bytes,
    Address,
    BlockContextReader,
    ContractContextReader,
    ExitCode,
    SharedAPI,
    SharedContextReader,
    StorageAPI,
    SyscallResult,
    TxContextReader,
    B256,
    U256,
};
use itertools::Itertools;
use lazy_static::lazy_static;
use solana_program::{
    bpf_loader_upgradeable,
    clock::Clock,
    epoch_schedule::EpochSchedule,
    message::{legacy, LegacyMessage, SanitizedMessage},
    pubkey::Pubkey,
    rent::Rent,
    system_program,
    sysvar,
};
use solana_rbpf::{
    program::{BuiltinFunction, BuiltinProgram, FunctionRegistry},
    vm::Config,
};

#[derive(Debug)]
pub enum SvmError {
    TransactionError(TransactionError),
    BincodeError(bincode::Error),
    ExitCode(ExitCode),
    InstructionError(InstructionError),
}

impl From<TransactionError> for SvmError {
    fn from(value: TransactionError) -> Self {
        SvmError::TransactionError(value)
    }
}

impl From<ExitCode> for SvmError {
    fn from(value: ExitCode) -> Self {
        SvmError::ExitCode(value)
    }
}

impl From<bincode::Error> for SvmError {
    fn from(value: bincode::Error) -> Self {
        SvmError::BincodeError(value)
    }
}

impl From<InstructionError> for SvmError {
    fn from(value: InstructionError) -> Self {
        SvmError::InstructionError(value)
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct BatchMessage {
    messages: Vec<legacy::Message>,
}

impl BatchMessage {
    pub fn new(cap: Option<usize>) -> Self {
        BatchMessage {
            messages: Vec::with_capacity(cap.unwrap_or(0)),
        }
    }

    pub fn clear(&mut self) -> &mut Self {
        self.messages.clear();
        self
    }

    pub fn append_one(&mut self, msg: legacy::Message) -> &mut Self {
        self.messages.push(msg);
        self
    }

    pub fn append_many(&mut self, msgs: Vec<legacy::Message>) -> &mut Self {
        self.messages.extend(msgs);
        self
    }
}

pub fn init_config() -> Config {
    Config {
        enable_instruction_tracing: false,
        reject_broken_elfs: true,
        sanitize_user_provided_values: true,
        ..Default::default()
    }
}

use crate::{
    account_utils::StateMut,
    helpers::{storage_read_account_data, storage_write_account_data},
};
use hashbrown::{HashMap, HashSet};
use serde::{Deserialize, Serialize};
use solana_program::bpf_loader_upgradeable::UpgradeableLoaderState;

lazy_static! {
    pub static ref SYSTEM_PROGRAMS_KEYS: HashSet<Pubkey> = {
        let mut set = HashSet::new();
        set.insert(system_program::id());
        set.insert(native_loader::id());
        set.insert(bpf_loader_upgradeable::id());
        set.insert(sysvar::clock::id());
        set.insert(sysvar::rent::id());
        set
    };
}

pub fn extract_account_data_or_default<SAPI: StorageAPI>(
    sapi: &SAPI,
    account_key: &Pubkey,
) -> Result<AccountSharedData, SvmError> {
    let account_data = storage_read_account_data(sapi, account_key);
    if let Ok(account_data) = account_data {
        return Ok(account_data);
    }
    Ok(AccountSharedData::new(0, 0, &system_program::id()))
}

fn load_program_account<SDK: SharedAPI, SAPI: StorageAPI>(
    sdk: &SDK,
    sapi: &Option<&mut SAPI>,
    program_accounts: &mut Vec<(Pubkey, AccountSharedData)>,
    account_key: &Pubkey,
) -> Result<bool, SvmError> {
    let program_account_idx = program_accounts
        .iter()
        .position(|(pk, _)| pk == account_key);
    if program_account_idx.is_some() {
        return Ok(false);
    }
    let program_account = if let Some(sapi) = sapi {
        extract_account_data_or_default(*sapi, account_key)?
    } else {
        extract_account_data_or_default(sdk, account_key)?
    };
    // TODO do we need this check?
    // if !program_account.executable() {
    //     return Err(TransactionError::InvalidProgramForExecution.into());
    // }
    program_accounts.push((account_key.clone(), program_account));
    Ok(true)
}

pub fn exec_encoded_svm_batch_message<SDK: SharedAPI, SAPI: StorageAPI>(
    sdk: &mut SDK,
    batch_message: &[u8],
    flush_result_accounts: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<Vec<(Pubkey, AccountSharedData)>, SvmError> {
    let batch_message: BatchMessage = bincode::deserialize(batch_message)?;
    exec_svm_batch_message(sdk, batch_message, flush_result_accounts, sapi)
}
pub fn exec_svm_batch_message<SDK: SharedAPI, SAPI: StorageAPI>(
    sdk: &mut SDK,
    batch_message: BatchMessage,
    flush_result_accounts: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<Vec<(Pubkey, AccountSharedData)>, SvmError> {
    let mut result_accounts = vec![];
    for message in &batch_message.messages {
        result_accounts.extend(exec_svm_message(
            sdk,
            message.clone(),
            flush_result_accounts,
            sapi,
        )?)
    }
    Ok(result_accounts)
}
pub fn exec_encoded_svm_message<SDK: SharedAPI, SAPI: StorageAPI>(
    sdk: &mut SDK,
    message: &[u8],
    flush_result_accounts: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<Vec<(Pubkey, AccountSharedData)>, SvmError> {
    let message: legacy::Message = bincode::deserialize(message)?;
    exec_svm_message(sdk, message, flush_result_accounts, sapi)
}
pub fn flush_accounts<SDK: SharedAPI, SAPI: StorageAPI>(
    sdk: &mut SDK,
    sapi: &mut Option<&mut SAPI>,
    accounts: &Vec<(Pubkey, AccountSharedData)>,
) -> Result<(), SvmError> {
    for (key, data) in accounts {
        if let Some(sapi) = sapi {
            storage_write_account_data(*sapi, key, data)?;
        } else {
            storage_write_account_data(sdk, key, data)?;
        }
    }
    Ok(())
}

pub fn exec_svm_message<SDK: SharedAPI, SAPI: StorageAPI>(
    sdk: &mut SDK,
    message: legacy::Message,
    flush_result_accounts: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<Vec<(Pubkey, AccountSharedData)>, SvmError> {
    let message: SanitizedMessage = SanitizedMessage::Legacy(LegacyMessage::new(message));

    let config = init_config();

    // TODO validate blockhash?
    let blockhash = message.recent_blockhash();
    let block_number = sdk.context().block_number();

    let compute_budget = ComputeBudget::default();
    let mut sysvar_cache = SysvarCache::default();
    let rent = Rent::free();
    let clock = Clock::default();
    let epoch_schedule = EpochSchedule::default();
    sysvar_cache.set_rent(rent.clone());
    sysvar_cache.set_clock(clock);
    sysvar_cache.set_epoch_schedule(epoch_schedule);

    let system_program_id = system_program::id();
    let bpf_loader_upgradeable_id = bpf_loader_upgradeable::id();

    let mut working_accounts = vec![];
    let mut program_accounts: Vec<(Pubkey, AccountSharedData)> = vec![];
    let mut program_indices = vec![];
    let account_keys = message.account_keys();

    let mut program_accounts_to_load: Vec<&Pubkey> = vec![];

    let mut program_account_found = false;
    for account_key in account_keys.iter() {
        let account_key_idx = working_accounts
            .iter()
            .position(|(pk, _)| pk == account_key);
        if account_key_idx.is_some() {
            continue;
        }
        if SYSTEM_PROGRAMS_KEYS.contains(account_key) || program_account_found {
            program_accounts_to_load.push(account_key);
            // TODO need a better solution for splitting working accounts and program accounts
            program_account_found = true;
            continue;
        }
        let account_data = if let Some(sapi) = sapi {
            extract_account_data_or_default(*sapi, account_key)?
        } else {
            extract_account_data_or_default(sdk, account_key)?
        };
        if account_data.executable() {
            continue; // this is program account?
        }
        working_accounts.push((account_key.clone(), account_data));
    }

    let mut program_accounts_to_warmup: Vec<&Pubkey> = vec![];
    for instruction in message.instructions() {
        program_indices.push(vec![]);
        let account_key = account_keys
            .get(instruction.program_id_index as usize)
            .unwrap();
        program_accounts_to_load.push(account_key);
        let program_account_idx = program_accounts
            .iter()
            .position(|(pk, _)| pk == account_key);
        if let Some(program_account_program_idx) = program_account_idx {
            program_indices
                .last_mut()
                .unwrap()
                .push(program_account_program_idx as IndexOfAccount);
        } else {
            let program_account = if let Some(sapi) = sapi {
                extract_account_data_or_default(*sapi, account_key)?
            } else {
                extract_account_data_or_default(sdk, account_key)?
            };
            let state: Result<UpgradeableLoaderState, InstructionError> = program_account.state();
            if let Ok(state) = state {
                if let UpgradeableLoaderState::Program {
                    programdata_address,
                } = state
                {
                    program_accounts_to_warmup.push(account_key);
                    // TODO it must be executable, should we validate?
                    let program_account = if let Some(sapi) = sapi {
                        extract_account_data_or_default(*sapi, &programdata_address)?
                    } else {
                        extract_account_data_or_default(sdk, &programdata_address)?
                    };
                    // if !program_account.executable() {
                    //     return
                    // Err(SvmError::TransactionError(TransactionError::InvalidProgramForExecution))
                    // }
                    let program_account_owner = program_account.owner().clone();
                    working_accounts.push((programdata_address, program_account));
                    program_indices
                        .last_mut()
                        .unwrap()
                        .push(program_accounts.len() as IndexOfAccount);
                    load_program_account(sdk, sapi, &mut program_accounts, &program_account_owner)?;
                    // load_program_account(sdk, &mut program_accounts,
                    // &bpf_loader_upgradeable_id)?;
                }
            }
            program_indices
                .last_mut()
                .unwrap()
                .push(program_accounts.len() as IndexOfAccount);
            program_accounts.push((account_key.clone(), program_account));
        }
    }
    for program_account_key in program_accounts_to_load {
        load_program_account(sdk, sapi, &mut program_accounts, program_account_key)?;
    }

    let (accounts, working_accounts_count) =
        compile_accounts_for_tx_ctx(working_accounts, program_accounts);
    // rearrange program indices
    program_indices.iter_mut().for_each(|program_sub_indices| {
        program_sub_indices
            .iter_mut()
            .for_each(|program_sub_index| {
                *program_sub_index += working_accounts_count;
            })
    });

    // TODO compute hardcoded parameters
    let transaction_context = TransactionContext::new(accounts, rent.clone(), 10, 200);

    let mut function_registry = FunctionRegistry::<BuiltinFunction<InvokeContext<SDK>>>::default();
    register_builtins(&mut function_registry);
    let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));
    let mut programs_loaded_for_tx_batch = LoadedProgramsForTxBatch::partial_default2(
        block_number,
        ProgramRuntimeEnvironments {
            program_runtime_v1: loader.clone(),
            program_runtime_v2: loader.clone(),
        },
    );
    programs_loaded_for_tx_batch.replenish(
        system_program_id,
        Arc::new(LoadedProgram::new_builtin(
            0,
            0,
            system_processor::Entrypoint::vm,
        )),
    );
    programs_loaded_for_tx_batch.replenish(
        bpf_loader_upgradeable_id,
        Arc::new(LoadedProgram::new_builtin(
            0,
            0,
            bpf_loader_upgradable::Entrypoint::vm,
        )),
    );
    let programs_modified_by_tx = LoadedProgramsForTxBatch::partial_default2(
        block_number,
        ProgramRuntimeEnvironments {
            program_runtime_v1: loader.clone(),
            program_runtime_v2: loader.clone(),
        },
    );
    let transaction_context = {
        let mut invoke_context = InvokeContext::new(
            transaction_context,
            sysvar_cache.clone(),
            sdk,
            compute_budget.clone(),
            programs_loaded_for_tx_batch,
            programs_modified_by_tx,
            Arc::new(FeatureSet::all_enabled()),
            *blockhash,
            0,
        );
        for pk in program_accounts_to_warmup {
            let loaded_program = invoke_context.load_program(pk, false);
            if let Some(v) = loaded_program {
                invoke_context
                    .programs_loaded_for_tx_batch
                    .replenish(pk.clone(), v);
            };
        }

        MessageProcessor::process_message(&message, &program_indices, &mut invoke_context)?;
        invoke_context.transaction_context
    };

    // TODO optimize accounts saving
    let mut result_accounts: Vec<(Pubkey, AccountSharedData)> =
        Vec::with_capacity(transaction_context.get_number_of_accounts() as usize);
    if flush_result_accounts {
        for account_idx in 0..transaction_context.get_number_of_accounts() {
            let account_key = transaction_context.get_key_of_account_at_index(account_idx)?;
            let account_data = transaction_context.get_account_at_index(account_idx)?;
            result_accounts.push((
                account_key.clone(),
                account_data.borrow().to_account_shared_data(),
            ));
        }
        flush_accounts(sdk, sapi, &result_accounts)?;
    }

    Ok(result_accounts)
}

pub fn process_svm_error(svm_error: SvmError) -> (Vec<(Pubkey, AccountSharedData)>, i32) {
    match svm_error {
        SvmError::TransactionError(err) => (Vec::new(), ExitCode::UnknownError.into_i32()),
        SvmError::BincodeError(err) => (Vec::new(), ExitCode::UnknownError.into_i32()),
        SvmError::ExitCode(err) => (Vec::new(), ExitCode::UnknownError.into_i32()),
        SvmError::InstructionError(err) => (Vec::new(), ExitCode::UnknownError.into_i32()),
    }
}

pub fn process_svm_result(
    result: Result<Vec<(Pubkey, AccountSharedData)>, SvmError>,
) -> (Vec<(Pubkey, AccountSharedData)>, i32) {
    match result {
        Ok(v) => (v, ExitCode::Ok.into_i32()),
        Err(err) => process_svm_error(err),
    }
}

pub struct MemStorage {
    in_memory_storage: HashMap<U256, U256>,
}

impl MemStorage {
    pub fn new() -> Self {
        Self {
            in_memory_storage: HashMap::new(),
        }
    }
}
impl StorageAPI for MemStorage {
    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
        self.in_memory_storage.insert(slot, value);
        SyscallResult::new((), 0, 0, ExitCode::Ok)
    }

    fn storage(&self, slot: &U256) -> SyscallResult<U256> {
        let result = self.in_memory_storage.get(slot).cloned();
        SyscallResult::new(result.unwrap_or_default(), 0, 0, ExitCode::Ok)
    }
}
