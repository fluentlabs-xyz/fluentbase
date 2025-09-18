use crate::common::pubkey_from_evm_address;
use crate::helpers::{extract_accounts, storage_read_account_data_or_default};
use crate::solana_program::rent_collector::RENT_EXEMPT_RENT_EPOCH;
use crate::{
    account::{
        is_executable_by_account, Account, AccountSharedData, ReadableAccount, WritableAccount,
        PROGRAM_OWNERS,
    },
    builtins::register_builtins,
    common::rbpf_config_default,
    compute_budget::compute_budget::ComputeBudget,
    context::{EnvironmentConfig, IndexOfAccount, InvokeContext, TransactionContext},
    error::SvmError,
    fluentbase::common::BatchMessage,
    helpers::storage_read_account_data,
    loaded_programs::{ProgramCacheEntry, ProgramCacheForTxBatch, ProgramRuntimeEnvironments},
    loaders::bpf_loader_v4,
    message_processor::MessageProcessor,
    native_loader, saturating_add_assign, solana_program,
    solana_program::{
        feature_set::feature_set_default,
        loader_v4,
        message::{legacy, LegacyMessage, SanitizedMessage},
        svm_message::SVMMessage,
        sysvar::instructions::{
            construct_instructions_data, BorrowedAccountMeta, BorrowedInstruction,
        },
    },
    system_processor, system_program,
    sysvar_cache::SysvarCache,
};
use alloc::{sync::Arc, vec::Vec};
use fluentbase_sdk::{ContextReader, MetadataAPI, SharedAPI};
use fluentbase_types::MetadataStorageAPI;
use hashbrown::{hash_map::Entry, HashMap, HashSet};
use solana_bincode::deserialize;
use solana_clock::Clock;
use solana_epoch_schedule::EpochSchedule;
use solana_feature_set::{disable_account_loader_special_case, FeatureSet};
use solana_pubkey::Pubkey;
use solana_rbpf::{
    program::{BuiltinFunction, BuiltinProgram, FunctionRegistry},
    vm::Config,
};
use solana_transaction_error::TransactionError;

const INSTRUCTION_STACK_CAPACITY: usize = 100;
const INSTRUCTION_TRACE_CAPACITY: usize = 100;

pub fn init_config() -> Config {
    rbpf_config_default(None)
}

pub fn exec_encoded_svm_batch_message<SDK: SharedAPI>(
    sdk: &mut SDK,
    batch_message: &[u8],
) -> Result<HashMap<Pubkey, AccountSharedData>, SvmError> {
    let batch_message = deserialize(batch_message)?;
    exec_svm_batch_message(sdk, batch_message)
}
pub fn exec_svm_batch_message<SDK: SharedAPI>(
    sdk: &mut SDK,
    batch_message: BatchMessage,
) -> Result<HashMap<Pubkey, AccountSharedData>, SvmError> {
    let mut result_accounts: HashMap<Pubkey, AccountSharedData> = Default::default();
    for (idx, message) in batch_message.messages().iter().enumerate() {
        let ra = exec_svm_message(sdk, message.clone())?;
        result_accounts.extend(ra);
    }
    Ok(result_accounts)
}
pub fn exec_encoded_svm_message<SDK: SharedAPI>(
    sdk: &mut SDK,
    message: &[u8],
) -> Result<HashMap<Pubkey, AccountSharedData>, SvmError> {
    let message = deserialize(message)?;
    exec_svm_message(sdk, message)
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct LoadedTransactionAccount {
    pub(crate) account: AccountSharedData,
    pub(crate) loaded_size: usize,
}

fn construct_instructions_account(message: &impl SVMMessage) -> AccountSharedData {
    let account_keys = message.account_keys();
    let mut decompiled_instructions = Vec::with_capacity(message.num_instructions());
    for (program_id, instruction) in message.program_instructions_iter() {
        let accounts = instruction
            .accounts
            .iter()
            .map(|account_index| {
                let account_index = usize::from(*account_index);
                BorrowedAccountMeta {
                    is_signer: message.is_signer(account_index),
                    is_writable: message.is_writable(account_index),
                    pubkey: account_keys.get(account_index).unwrap(),
                }
            })
            .collect();

        decompiled_instructions.push(BorrowedInstruction {
            accounts,
            data: &instruction.data,
            program_id,
        });
    }

    AccountSharedData::from(Account {
        data: construct_instructions_data(&decompiled_instructions),
        owner: system_program::id(),
        ..Account::default()
    })
}

fn account_shared_data_from_program(
    key: &Pubkey,
    program_accounts: &HashMap<Pubkey, (&Pubkey, u64)>,
) -> Result<AccountSharedData, SvmError> {
    // It's an executable program account. The program is already loaded in the cache.
    // So the account data is not needed. Return a dummy AccountSharedData with meta
    // information.
    let mut program_account = AccountSharedData::default();
    let (program_owner, _count) = program_accounts
        .get(key)
        .ok_or(TransactionError::AccountNotFound)?;
    program_account.set_owner(**program_owner);
    program_account.set_executable(true);
    Ok(program_account)
}

#[allow(clippy::too_many_arguments)]
fn load_transaction_account<'a, SDK: SharedAPI, API: MetadataAPI + MetadataStorageAPI>(
    api: &API,
    message: &impl SVMMessage,
    account_key: &Pubkey,
    account_index: usize,
    instruction_accounts: &[&u8],
    feature_set: &FeatureSet,
    program_accounts: &HashMap<Pubkey, (&Pubkey, u64)>,
    loaded_programs: &ProgramCacheForTxBatch<'a, SDK>,
) -> Result<(LoadedTransactionAccount, bool), SvmError> {
    let mut account_found = true;
    let disable_account_loader_special_case =
        feature_set.is_active(&disable_account_loader_special_case::id());
    let is_instruction_account = u8::try_from(account_index)
        .map(|i| instruction_accounts.contains(&&i))
        .unwrap_or(false);
    let is_writable = message.is_writable(account_index);
    let loaded_account = if solana_program::sysvar::instructions::check_id(account_key) {
        // Since the instructions sysvar is constructed by the SVM and modified
        // for each transaction instruction, it cannot be overridden.
        LoadedTransactionAccount {
            loaded_size: 0,
            account: construct_instructions_account(message),
        }
    } else if let Some(program) =
        (!disable_account_loader_special_case && !is_instruction_account && !is_writable)
            .then_some(())
            .and_then(|_| loaded_programs.find(account_key))
    {
        // Optimization to skip loading of accounts which are only used as
        // programs in top-level instructions and not passed as instruction accounts.
        LoadedTransactionAccount {
            loaded_size: program.account_size,
            account: account_shared_data_from_program(account_key, program_accounts)?,
        }
    } else {
        storage_read_account_data(api, account_key)
            .map(|account| {
                // Inspect the account prior to collecting rent, since
                // rent collection can modify the account.

                LoadedTransactionAccount {
                    loaded_size: account.data().len(),
                    account,
                }
            })
            .unwrap_or_else(|_| {
                account_found = false;
                let mut default_account = AccountSharedData::default();

                // All new accounts must be rent-exempt (enforced in
                // Bank::execute_loaded_transaction). Currently, rent collection
                // sets rent_epoch to u64::MAX, but initializing the account
                // with this field already set would allow us to skip rent collection for these
                // accounts.
                default_account.set_rent_epoch(RENT_EXEMPT_RENT_EPOCH);
                LoadedTransactionAccount {
                    loaded_size: default_account.data().len(),
                    account: default_account,
                }
            })
    };

    Ok((loaded_account, account_found))
}

pub fn prepare_data_for_tx_ctx<SDK: SharedAPI>(
    sdk: &mut SDK,
    message: &impl SVMMessage,
    feature_set: &FeatureSet,
    program_accounts: &HashMap<Pubkey, (&Pubkey, u64)>,
    loaded_programs: &ProgramCacheForTxBatch<SDK>,
) -> Result<(Vec<(Pubkey, AccountSharedData)>, Vec<Vec<IndexOfAccount>>), SvmError> {
    let account_keys = message.account_keys();
    let mut accounts: Vec<(Pubkey, AccountSharedData)> = Vec::with_capacity(account_keys.len());
    let mut accounts_found = Vec::with_capacity(account_keys.len());

    let count = message
        .instructions_iter()
        .fold(0, |accum, instr| accum + instr.accounts.len());
    let mut unique_items = HashSet::with_capacity(count);
    let instruction_accounts = message
        .instructions_iter()
        .flat_map(|instruction| instruction.accounts)
        .filter(|v| unique_items.insert(*v))
        .collect::<Vec<&u8>>();

    let mut collect_loaded_account =
        |key: &Pubkey, (loaded_account, found)| -> Result<(), SvmError> {
            accounts.push((*key, loaded_account));
            accounts_found.push(found);
            Ok(())
        };

    // Since the fee payer is always the first account, collect it first. Note
    // that account overrides are already applied during fee payer validation so
    // it's fine to use the fee payer directly here rather than checking account
    // overrides again.
    let fee_payer = message.fee_payer();
    let loaded_fee_payer_account = storage_read_account_data_or_default(sdk, fee_payer, 0, None);
    collect_loaded_account(fee_payer, (loaded_fee_payer_account, true))?;

    // Attempt to load and collect remaining non-fee payer accounts
    for (account_index, account_key) in account_keys.iter().enumerate().skip(1) {
        let (loaded_account, account_found) = load_transaction_account(
            sdk,
            message,
            account_key,
            account_index,
            &instruction_accounts[..],
            feature_set,
            program_accounts,
            loaded_programs,
        )?;
        collect_loaded_account(account_key, (loaded_account.account, account_found))?;
    }

    let builtins_start_index = accounts.len();
    let program_indices = message
        .instructions_iter()
        .map(|instruction| {
            let mut account_indices = Vec::with_capacity(1);
            let program_index = instruction.program_id_index as usize;
            // This command may never return error, because the transaction is sanitized
            let (program_id, program_account) = accounts
                .get(program_index)
                .ok_or(TransactionError::ProgramAccountNotFound)?;
            if native_loader::check_id(program_id) {
                return Ok(account_indices);
            }

            let account_found = accounts_found.get(program_index).unwrap_or(&true);
            if !account_found {
                return Err(TransactionError::ProgramAccountNotFound);
            }

            if !is_executable_by_account(&program_account) && !program_account.executable() {
                return Err(TransactionError::InvalidProgramForExecution);
            }
            account_indices.insert(0, program_index as IndexOfAccount);
            let owner_id = program_account.owner();
            if native_loader::check_id(owner_id) {
                return Ok(account_indices);
            }
            if !accounts
                .get(builtins_start_index..)
                .ok_or(TransactionError::ProgramAccountNotFound)?
                .iter()
                .any(|(key, _)| key == owner_id)
            {
                if let Ok(owner_account) = storage_read_account_data(sdk, owner_id) {
                    if !native_loader::check_id(owner_account.owner())
                        || !owner_account.executable()
                    {
                        return Err(TransactionError::InvalidProgramForExecution);
                    }
                    accounts.push((*owner_id, owner_account));
                } else {
                    return Err(TransactionError::ProgramAccountNotFound);
                }
            }
            Ok(account_indices)
        })
        .collect::<Result<Vec<Vec<IndexOfAccount>>, TransactionError>>()?;

    Ok((accounts, program_indices))
}

fn filter_executable_program_accounts<'a, SDK: SharedAPI>(
    sdk: &SDK,
    txs: &[&impl SVMMessage],
    program_owners: &'a [Pubkey],
) -> HashMap<Pubkey, (&'a Pubkey, u64)> {
    let mut result: HashMap<Pubkey, (&'a Pubkey, u64)> =
        HashMap::with_capacity(txs.iter().fold(0usize, |a, v| a + v.account_keys().len()));

    txs.iter().for_each(|etx| {
        etx.account_keys()
            .iter()
            .for_each(|key| match result.entry(*key) {
                Entry::Occupied(mut entry) => {
                    let (_, count) = entry.get_mut();
                    saturating_add_assign!(*count, 1);
                }
                Entry::Vacant(entry) => {
                    let account = storage_read_account_data(sdk, key);
                    if let Ok(acc) = account {
                        if let Some(index) = program_owners.iter().position(|k| k == acc.owner()) {
                            if let Some(owner) = program_owners.get(index) {
                                entry.insert((owner, 1));
                            }
                        }
                    }
                }
            });
    });
    result
}

pub fn exec_svm_message<SDK: SharedAPI>(
    sdk: &mut SDK,
    message: legacy::Message,
) -> Result<HashMap<Pubkey, AccountSharedData>, SvmError> {
    let message: SanitizedMessage =
        SanitizedMessage::Legacy(LegacyMessage::new(message, &Default::default()));

    let contract_caller = sdk.context().contract_caller();
    let pk_caller = pubkey_from_evm_address::<true>(&contract_caller);
    let signer_count = message.num_total_signatures();
    if signer_count > 1 {
        panic!("max number of signers can be 1, given {}", signer_count);
    }
    for i in 0..message.account_keys().len() {
        if message.is_signer(i) {
            let pk_account = message.account_keys().get(i).unwrap();
            if &pk_caller != pk_account {
                panic!("only caller account can be signer: {}", pk_account);
            }
        }
    }

    let config = init_config();

    let block_number = sdk.context().block_number();

    let compute_budget = ComputeBudget::default();
    let mut sysvar_cache = SysvarCache::default();
    let clock = Clock::default();
    let epoch_schedule = EpochSchedule::default();
    sysvar_cache.set_clock(clock);
    sysvar_cache.set_epoch_schedule(epoch_schedule);

    let system_program_id = system_program::id();
    let loader_id = loader_v4::id();

    let mut function_registry = FunctionRegistry::<BuiltinFunction<InvokeContext<SDK>>>::default();
    register_builtins(&mut function_registry);
    let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));
    let mut program_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
        block_number,
        ProgramRuntimeEnvironments {
            program_runtime_v1: loader.clone(),
            program_runtime_v2: loader.clone(),
        },
    );
    program_cache_for_tx_batch.replenish(
        system_program_id,
        Arc::new(ProgramCacheEntry::new_builtin(
            0,
            0,
            system_processor::Entrypoint::vm,
        )),
    );
    program_cache_for_tx_batch.replenish(
        loader_id,
        Arc::new(ProgramCacheEntry::new_builtin(
            0,
            0,
            bpf_loader_v4::Entrypoint::vm,
        )),
    );

    let feature_set = feature_set_default();
    let program_accounts = filter_executable_program_accounts(sdk, &[&message], &PROGRAM_OWNERS);
    let result = prepare_data_for_tx_ctx(
        sdk,
        &message,
        &feature_set,
        &program_accounts,
        &program_cache_for_tx_batch,
    );
    let (accounts, program_indices) = result?;

    let transaction_context = TransactionContext::new(
        accounts,
        INSTRUCTION_STACK_CAPACITY,
        INSTRUCTION_TRACE_CAPACITY,
    );

    let transaction_context = {
        let environment_config = EnvironmentConfig::new(
            *message.recent_blockhash(),
            Arc::new(feature_set),
            sysvar_cache,
        );

        let mut invoke_context = InvokeContext::new(
            transaction_context,
            program_cache_for_tx_batch,
            environment_config,
            compute_budget.clone(),
            sdk,
        );
        let program_accounts_to_warmup: Vec<&Pubkey> = program_accounts.keys().collect();
        for pk in program_accounts_to_warmup {
            let loaded_program = invoke_context.load_program(pk, false);
            if let Some(v) = loaded_program {
                invoke_context
                    .program_cache_for_tx_batch
                    .replenish(pk.clone(), v);
            };
        }

        MessageProcessor::process_message(&message, &program_indices, &mut invoke_context)?;
        invoke_context.transaction_context
    };

    let mut result_accounts = extract_accounts(&transaction_context)?;

    Ok(result_accounts)
}
