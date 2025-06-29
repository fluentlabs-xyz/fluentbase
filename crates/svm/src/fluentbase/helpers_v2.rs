use crate::{
    account::{
        is_executable_account,
        Account,
        AccountSharedData,
        ReadableAccount,
        WritableAccount,
        PROGRAM_OWNERS,
    },
    builtins::register_builtins,
    compute_budget::compute_budget::ComputeBudget,
    context::{EnvironmentConfig, IndexOfAccount, InvokeContext, TransactionContext},
    error::SvmError,
    fluentbase::common::{extract_account_data_or_default, flush_accounts, BatchMessage},
    helpers::storage_read_account_data,
    loaded_programs::{ProgramCacheEntry, ProgramCacheForTxBatch, ProgramRuntimeEnvironments},
    loaders::bpf_loader_v4,
    message_processor::MessageProcessor,
    native_loader,
    saturating_add_assign,
    select_sapi,
    solana_program,
    solana_program::{
        feature_set::feature_set_default,
        loader_v4,
        message::{legacy, LegacyMessage, SanitizedMessage},
        svm_message::SVMMessage,
        sysvar::instructions::{
            construct_instructions_data,
            BorrowedAccountMeta,
            BorrowedInstruction,
        },
    },
    system_processor,
    system_program,
    sysvar_cache::SysvarCache,
};
use alloc::{sync::Arc, vec::Vec};
use fluentbase_sdk::{debug_log_ext, ContextReader, MetadataAPI, SharedAPI};
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
use solana_rent::{sysvar, Rent};
use solana_transaction_error::TransactionError;

pub fn init_config() -> Config {
    Config {
        enable_instruction_tracing: false,
        reject_broken_elfs: true,
        sanitize_user_provided_values: true,
        aligned_memory_mapping: true,
        enable_address_translation: true, /* To be deactivated once we have BTF inference and
                                           * verification */
        enable_stack_frame_gaps: true,
        ..Default::default()
    }
}

pub fn exec_encoded_svm_batch_message<SDK: SharedAPI, SAPI: MetadataAPI>(
    sdk: &mut SDK,
    batch_message: &[u8],
    flush_result_accounts: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<
    (
        HashMap<Pubkey, AccountSharedData>,
        HashMap<Pubkey, (u64, u64)>,
    ),
    SvmError,
> {
    let batch_message = deserialize(batch_message)?;
    exec_svm_batch_message(sdk, batch_message, flush_result_accounts, sapi)
}
pub fn exec_svm_batch_message<SDK: SharedAPI, SAPI: MetadataAPI>(
    sdk: &mut SDK,
    batch_message: BatchMessage,
    do_flush: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<
    (
        HashMap<Pubkey, AccountSharedData>,
        HashMap<Pubkey, (u64, u64)>,
    ),
    SvmError,
> {
    let mut result_accounts: HashMap<Pubkey, AccountSharedData> = Default::default();
    let mut balance_changes: HashMap<Pubkey, (u64, u64)> = Default::default();
    for message in batch_message.messages() {
        let (ra, bhs) = exec_svm_message(sdk, sapi, message.clone(), do_flush)?;
        result_accounts.extend(ra);
        for (account_key, balance_change) in bhs {
            match balance_changes.entry(account_key) {
                Entry::Occupied(v) => {
                    v.into_mut().1 = balance_change.1;
                }
                Entry::Vacant(v) => {
                    v.insert(balance_change);
                }
            }
        }
    }
    Ok((result_accounts, balance_changes))
}
pub fn exec_encoded_svm_message<SDK: SharedAPI, SAPI: MetadataAPI>(
    sdk: &mut SDK,
    message: &[u8],
    flush_result_accounts: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<
    (
        HashMap<Pubkey, AccountSharedData>,
        HashMap<Pubkey, (u64, u64)>,
    ),
    SvmError,
> {
    let message = deserialize(message)?;
    exec_svm_message(sdk, sapi, message, flush_result_accounts)
}

// pub fn prepare_data_for_tx_ctx1<SDK: SharedAPI, SAPI: MetadataAPI>(
//     sdk: &mut SDK,
//     message: &impl SVMMessage,
//     sapi: &mut Option<&mut SAPI>,
// ) -> Result<
//     ((
//         // SanitizedMessage,
//         Vec<(Pubkey, AccountSharedData)>,
//         Vec<Vec<IndexOfAccount>>,
//         Vec<Pubkey>,
//     )),
//     SvmError,
// > { let mut sysvar_cache = SysvarCache::default(); let rent = Rent::free(); let clock =
// > Clock::default(); let epoch_schedule = EpochSchedule::default();
// > sysvar_cache.set_rent(rent.clone()); sysvar_cache.set_clock(clock);
// > sysvar_cache.set_epoch_schedule(epoch_schedule);
//
//     let mut working_accounts = vec![];
//     let mut program_accounts = vec![];
//     let mut program_indices = vec![];
//     let message_clone = message.clone();
//     let account_keys = message_clone.account_keys();
//
//     let mut program_accounts_to_load: Vec<&Pubkey> = Default::default();
//
//     let mut program_account_found = false;
//     for account_key in account_keys.iter() {
//         let account_key_idx = working_accounts
//             .iter()
//             .position(|(pk, _)| pk == account_key);
//         if account_key_idx.is_some() {
//             continue;
//         }
//         if SYSTEM_PROGRAMS_KEYS.contains(account_key) || program_account_found {
//             program_accounts_to_load.push(account_key);
//             program_account_found = true;
//             continue;
//         }
//         let account_data = if let Some(sapi) = sapi {
//             extract_account_data_or_default(*sapi, account_key)?
//         } else {
//             extract_account_data_or_default(sdk, account_key)?
//         };
//         if account_data.executable() {
//             continue; // this is program account?
//         }
//         // loader-v4 doesn't mark account executable after deploy, so we need to check this
// condition         let state: Result<&LoaderV4State, InstructionError> =
// get_state(account_data.data());         if let Ok(state) = state {
//             match state.status {
//                 LoaderV4Status::Deployed | LoaderV4Status::Finalized => continue,
//                 _ => {}
//             }
//         }
//         working_accounts.push((account_key.clone(), account_data));
//     }
//
//     let mut program_accounts_to_warmup: Vec<&Pubkey> = Default::default();
//     for instruction in message.instructions_iter() {
//         program_indices.push(vec![]);
//         let account_key = account_keys
//             .get(instruction.program_id_index as usize)
//             .unwrap();
//         program_accounts_to_load.push(account_key);
//         let program_account_idx = program_accounts
//             .iter()
//             .position(|(pk, _)| pk == account_key);
//         if let Some(program_account_program_idx) = program_account_idx {
//             program_indices
//                 .last_mut()
//                 .unwrap()
//                 .push(program_account_program_idx as IndexOfAccount);
//         } else {
//             let program_account = if let Some(sapi) = sapi {
//                 extract_account_data_or_default(*sapi, account_key)?
//             } else {
//                 extract_account_data_or_default(sdk, account_key)?
//             };
//             let state: Result<&LoaderV4State, InstructionError> =
// get_state(program_account.data());             if let Ok(state) = state {
//                 match state.status {
//                     LoaderV4Status::Deployed | LoaderV4Status::Finalized => {
//                         program_accounts_to_warmup.push(account_key);
//                     }
//                     _ => {}
//                 }
//             }
//             program_indices
//                 .last_mut()
//                 .unwrap()
//                 .push(program_accounts.len() as IndexOfAccount);
//             program_accounts.push((account_key.clone(), program_account));
//         }
//     }
//     for program_account_key in program_accounts_to_load {
//         load_program_account(sdk, sapi, &mut program_accounts, program_account_key)?;
//     }
//
//     let (accounts, working_accounts_count) =
//         compile_accounts_for_tx_ctx(working_accounts, program_accounts);
//     // rearrange program indices
//     program_indices.iter_mut().for_each(|program_sub_indices| {
//         program_sub_indices
//             .iter_mut()
//             .for_each(|program_sub_index| {
//                 *program_sub_index += working_accounts_count;
//             })
//     });
//
//     Ok((
//         // message,
//         accounts,
//         program_indices,
//         program_accounts_to_warmup.into_iter().cloned().collect(),
//     ))
// }

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct LoadedTransactionAccount {
    pub(crate) account: AccountSharedData,
    pub(crate) loaded_size: usize,
    pub(crate) rent_collected: u64,
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
        owner: sysvar::id(),
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
fn load_transaction_account<'a, SDK: SharedAPI, SAPI: MetadataAPI>(
    // callbacks: &CB,
    sapi: &SAPI,
    message: &impl SVMMessage,
    account_key: &Pubkey,
    account_index: usize,
    instruction_accounts: &[&u8],
    // account_overrides: Option<&AccountOverrides>,
    feature_set: &FeatureSet,
    // rent_collector: &dyn SVMRentCollector,
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
            rent_collected: 0,
        }
    // } else if let Some(account_override) =
    //     account_overrides.and_then(|overrides| overrides.get(account_key))
    // {
    //     LoadedTransactionAccount {
    //         loaded_size: account_override.data().len(),
    //         account: account_override.clone(),
    //         rent_collected: 0,
    //     }
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
            rent_collected: 0,
        }
    } else {
        // callbacks
        //     .get_account_shared_data(account_key)
        storage_read_account_data(sapi, account_key)
            .map(|account| {
                // Inspect the account prior to collecting rent, since
                // rent collection can modify the account.
                // callbacks.inspect_account(account_key, AccountState::Alive(&account),
                // is_writable);

                // let rent_collected = if is_writable {
                //     collect_rent_from_account(
                //         feature_set,
                //         rent_collector,
                //         account_key,
                //         &mut account,
                //     )
                //     .rent_amount
                // } else {
                //     0
                // };

                LoadedTransactionAccount {
                    loaded_size: account.data().len(),
                    account,
                    rent_collected: 0,
                }
            })
            .unwrap_or_else(|_| {
                // callbacks.inspect_account(account_key, AccountState::Dead, is_writable);

                account_found = false;
                let mut default_account = AccountSharedData::default();

                // All new accounts must be rent-exempt (enforced in
                // Bank::execute_loaded_transaction). Currently, rent collection
                // sets rent_epoch to u64::MAX, but initializing the account
                // with this field already set would allow us to skip rent collection for these
                // accounts.
                default_account
                    .set_rent_epoch(solana_program::rent_collector::RENT_EXEMPT_RENT_EPOCH);
                LoadedTransactionAccount {
                    loaded_size: default_account.data().len(),
                    account: default_account,
                    rent_collected: 0,
                }
            })
    };

    Ok((loaded_account, account_found))
}

pub fn prepare_data_for_tx_ctx2<SDK: SharedAPI, SAPI: MetadataAPI>(
    sdk: &mut SDK,
    message: &impl SVMMessage,
    sapi: &mut Option<&mut SAPI>,
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
            // let LoadedTransactionAccount {
            //     account,
            //     loaded_size,
            //     rent_collected,
            // } = loaded_account;

            // accumulate_and_check_loaded_account_data_size(
            //     &mut accumulated_accounts_data_size,
            //     loaded_size,
            //     compute_budget_limits.loaded_accounts_bytes,
            //     error_metrics,
            // )?;

            // tx_rent += rent_collected;
            // rent_debits.insert(key, rent_collected, account.lamports());

            accounts.push((*key, loaded_account));
            accounts_found.push(found);
            Ok(())
        };

    // Since the fee payer is always the first account, collect it first. Note
    // that account overrides are already applied during fee payer validation so
    // it's fine to use the fee payer directly here rather than checking account
    // overrides again.
    let fee_payer = message.fee_payer();
    let loaded_fee_payer_account =
        // select_sapi!(sapi, sdk, |s| { storage_read_account_data(s, fee_payer) })
        select_sapi!(sapi, sdk, |s| { extract_account_data_or_default(s, fee_payer) }) // TODO should we throw error?
            .expect("fee payer expected");
    collect_loaded_account(fee_payer, (loaded_fee_payer_account, true))?;

    // Attempt to load and collect remaining non-fee payer accounts
    for (account_index, account_key) in account_keys.iter().enumerate().skip(1) {
        // let (loaded_account, account_found) = load_transaction_account(
        //     // callbacks,
        //     sapi.unwrap(),
        //     &message,
        //     account_key,
        //     account_index,
        //     &instruction_accounts[..],
        //     // account_overrides,
        //     feature_set,
        //     // rent_collector,
        //     program_accounts,
        //     loaded_programs,
        // )?;
        let (loaded_account, account_found) = select_sapi!(sapi, sdk, |s| {
            load_transaction_account(
                // callbacks,
                s,
                message,
                account_key,
                account_index,
                &instruction_accounts[..],
                // account_overrides,
                feature_set,
                // rent_collector,
                program_accounts,
                loaded_programs,
            )
        })?;
        collect_loaded_account(account_key, (loaded_account.account, account_found))?;
    }

    let builtins_start_index = accounts.len();
    let program_indices = message
        .instructions_iter()
        .map(|instruction| {
            let mut account_indices = Vec::with_capacity(2);
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
                // error_metrics.account_not_found += 1;
                return Err(TransactionError::ProgramAccountNotFound);
            }

            // if !program_account.executable() {
            if !is_executable_account(&program_account) && !program_account.executable() {
                // error_metrics.invalid_program_for_execution += 1;
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
                if let Ok(owner_account) =
                    select_sapi!(sapi, sdk, |s| { storage_read_account_data(s, owner_id) })
                {
                    if !native_loader::check_id(owner_account.owner())
                        || !owner_account.executable()
                    {
                        return Err(TransactionError::InvalidProgramForExecution);
                    }
                    // accumulate_and_check_loaded_account_data_size(
                    //     &mut accumulated_accounts_data_size,
                    //     owner_account.data().len(),
                    //     compute_budget_limits.loaded_accounts_bytes,
                    //     error_metrics,
                    // )?;
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

fn filter_executable_program_accounts<'a, SDK: SharedAPI, SAPI: MetadataAPI>(
    sdk: &SDK,
    sapi: &mut Option<&mut SAPI>,
    txs: &[&impl SVMMessage],
    program_owners: &'a [Pubkey],
) -> HashMap<Pubkey, (&'a Pubkey, u64)> {
    let mut result: HashMap<Pubkey, (&'a Pubkey, u64)> = HashMap::new();

    txs.iter().for_each(|etx| {
        etx.account_keys()
            .iter()
            .for_each(|key| match result.entry(*key) {
                Entry::Occupied(mut entry) => {
                    let (_, count) = entry.get_mut();
                    saturating_add_assign!(*count, 1);
                }
                Entry::Vacant(entry) => {
                    // if let Some(index) = callbacks.account_matches_owners(key, program_owners) {
                    let account =
                        select_sapi!(sapi, sdk, |s| { storage_read_account_data(s, key) });
                    if let Ok(acc) = account {
                        // if acc.lamports() <= 0 {
                        //     return;
                        // }
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

pub fn exec_svm_message<SDK: SharedAPI, SAPI: MetadataAPI>(
    sdk: &mut SDK,
    sapi: &mut Option<&mut SAPI>,
    message: legacy::Message,
    flush_result_accounts: bool,
) -> Result<
    (
        HashMap<Pubkey, AccountSharedData>,
        HashMap<Pubkey, (u64, u64)>,
    ),
    SvmError,
> {
    let config = init_config();

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
    let loader_id = loader_v4::id();

    let message: SanitizedMessage =
        SanitizedMessage::Legacy(LegacyMessage::new(message, &Default::default()));

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
    let program_accounts =
        filter_executable_program_accounts(sdk, sapi, &[&message], &PROGRAM_OWNERS);
    let result = prepare_data_for_tx_ctx2(
        sdk,
        &message,
        sapi,
        &feature_set,
        &program_accounts,
        &program_cache_for_tx_batch,
    );
    let (accounts, program_indices) = result?;

    // TODO compute hardcoded parameters
    let transaction_context = TransactionContext::new(accounts, rent.clone(), 100, 200);

    let (transaction_context, balance_changes) = {
        let feature_set = feature_set_default();

        // TODO need specific blockhash?
        let environment_config = EnvironmentConfig::new(
            *message.recent_blockhash(),
            None,
            Arc::new(feature_set),
            0,
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

        let balance_changes =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context)?;
        (invoke_context.transaction_context, balance_changes)
    };

    // TODO optimize accounts saving
    let mut result_accounts =
        HashMap::with_capacity(transaction_context.get_number_of_accounts() as usize);

    for account_idx in 0..transaction_context.get_number_of_accounts() {
        let account_key = transaction_context.get_key_of_account_at_index(account_idx)?;

        let account_data = transaction_context.get_account_at_index(account_idx)?;

        result_accounts.insert(
            account_key.clone(),
            account_data.borrow().to_account_shared_data(),
        );
    }
    if flush_result_accounts {
        debug_log_ext!();
        flush_accounts(sdk, sapi, &result_accounts)?;
    }

    Ok((result_accounts, balance_changes))
}
