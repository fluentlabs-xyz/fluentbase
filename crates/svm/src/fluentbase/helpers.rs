use crate::{
    account::{AccountSharedData, ReadableAccount},
    account_utils::StateMut,
    builtins::register_builtins,
    common::{
        compile_accounts_for_tx_ctx,
        evm_address_from_pubkey,
        evm_balance_from_lamports,
        is_evm_pubkey,
    },
    compute_budget::compute_budget::ComputeBudget,
    context::{EnvironmentConfig, IndexOfAccount, InvokeContext, TransactionContext},
    error::SvmError,
    fluentbase::common::{
        extract_account_data_or_default,
        flush_accounts,
        load_program_account,
        BatchMessage,
        SYSTEM_PROGRAMS_KEYS,
    },
    loaded_programs::{ProgramCacheEntry, ProgramCacheForTxBatch, ProgramRuntimeEnvironments},
    message_processor::MessageProcessor,
    select_sapi,
    solana_program::{
        bpf_loader_upgradeable,
        bpf_loader_upgradeable::UpgradeableLoaderState,
        feature_set::feature_set_default,
        message::{legacy, LegacyMessage, SanitizedMessage},
    },
    system_processor,
    system_program,
    sysvar_cache::SysvarCache,
};
use alloc::{sync::Arc, vec, vec::Vec};
use fluentbase_sdk::{ContextReader, MetadataAPI, SharedAPI};
use hashbrown::HashMap;
use itertools::Itertools;
use solana_bincode::deserialize;
use solana_clock::Clock;
use solana_epoch_schedule::EpochSchedule;
use solana_hash::Hash;
use solana_instruction::error::InstructionError;
use solana_pubkey::Pubkey;
use solana_rbpf::{
    program::{BuiltinFunction, BuiltinProgram, FunctionRegistry},
    vm::Config,
};
use solana_rent::Rent;

pub fn init_config() -> Config {
    Config {
        enable_instruction_tracing: false,
        reject_broken_elfs: true,
        sanitize_user_provided_values: true,
        ..Default::default()
    }
}

pub fn exec_encoded_svm_batch_message<SDK: SharedAPI, SAPI: MetadataAPI>(
    sdk: &mut SDK,
    batch_message: &[u8],
    flush_result_accounts: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<HashMap<Pubkey, AccountSharedData>, SvmError> {
    let batch_message = deserialize::<BatchMessage>(batch_message)?;
    exec_svm_batch_message(sdk, batch_message, flush_result_accounts, sapi)
}
pub fn exec_svm_batch_message<SDK: SharedAPI, SAPI: MetadataAPI>(
    sdk: &mut SDK,
    batch_message: BatchMessage,
    flush_result_accounts: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<HashMap<Pubkey, AccountSharedData>, SvmError> {
    let mut result_accounts = HashMap::new();
    for message in batch_message.messages() {
        for acc in exec_svm_message(sdk, message.clone(), flush_result_accounts, sapi)? {
            result_accounts.insert(acc.0, acc.1);
        }
    }
    Ok(result_accounts)
}
pub fn exec_encoded_svm_message<SDK: SharedAPI, SAPI: MetadataAPI>(
    sdk: &mut SDK,
    message: &[u8],
    flush_result_accounts: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<HashMap<Pubkey, AccountSharedData>, SvmError> {
    let message = deserialize(message)?;
    exec_svm_message(sdk, message, flush_result_accounts, sapi)
}

pub fn exec_svm_message<SDK: SharedAPI, SAPI: MetadataAPI>(
    sdk: &mut SDK,
    message: legacy::Message,
    flush_result_accounts: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<HashMap<Pubkey, AccountSharedData>, SvmError> {
    let message: SanitizedMessage =
        SanitizedMessage::Legacy(LegacyMessage::new(message, &Default::default()));

    let config = init_config();

    // TODO validate blockhash?
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
            program_account_found = true;
            continue;
        }
        let account_data = select_sapi!(sapi, sdk, |s| {
            extract_account_data_or_default(s, account_key)
        })?;
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
                    // TODO it should be executable, should we validate?
                    let program_account = select_sapi!(sapi, sdk, |s| {
                        extract_account_data_or_default(s, &programdata_address)
                    })?;
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
        bpf_loader_upgradeable_id,
        Arc::new(ProgramCacheEntry::new_builtin(
            0,
            0,
            crate::loaders::bpf_loader_upgradeable::Entrypoint::vm,
        )),
    );
    // let programs_modified_by_tx = ProgramCacheForTxBatch::new2(
    //     block_number,
    //     ProgramRuntimeEnvironments {
    //         program_runtime_v1: loader.clone(),
    //         program_runtime_v2: loader.clone(),
    //     },
    // );
    let transaction_context = {
        let feature_set = Arc::new(feature_set_default());

        let environment_config =
            EnvironmentConfig::new(Hash::default(), None, feature_set, 0, sysvar_cache);

        let mut invoke_context = InvokeContext::new(
            transaction_context,
            program_cache_for_tx_batch,
            environment_config,
            compute_budget.clone(),
            sdk,
        );
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
        flush_accounts(sdk, sapi, &result_accounts)?;
    }

    Ok(result_accounts)
}

#[allow(unused)]
pub(crate) fn settle_balances<SDK: SharedAPI>(
    sdk: &mut SDK,
    balance_changes: HashMap<Pubkey, (u64, u64)>,
) {
    let contract_caller = sdk.context().contract_caller();
    let balance_receivers = balance_changes
        .iter()
        .filter(|(_pk, (before, after))| after > before)
        .map(|(pk, (before, after))| (pk, *after - *before))
        .collect_vec();
    let balance_senders = balance_changes
        .iter()
        .filter(|(_pk, (before, after))| before > after)
        .map(|(pk, (before, after))| (pk, *before - *after))
        .collect_vec();
    let balance_to_receive = balance_receivers
        .iter()
        .fold(0u64, |accum, (_, next)| accum + next);
    let balance_to_send = balance_senders
        .iter()
        .fold(0u64, |accum, (_, next)| accum + next);
    assert_eq!(balance_to_receive, balance_to_send);
    let mut run = balance_receivers.len() > 0;
    if run {
        let mut receiver_idx = 0;
        let mut sender_idx = 0;
        let (mut sender_pk, mut sender_delta) = balance_senders[sender_idx];
        let (mut receiver_pk, mut receiver_delta) = balance_receivers[receiver_idx];
        assert!(is_evm_pubkey(sender_pk));
        assert!(is_evm_pubkey(receiver_pk));
        while run {
            // TODO can be optimised
            let evm_address_from = evm_address_from_pubkey::<true>(sender_pk)
                .expect("sender pk must be evm compatible");
            // TODO can be optimised
            let address_to = evm_address_from_pubkey::<true>(receiver_pk)
                .expect("receiver pk must be evm compatible");
            assert_eq!(evm_address_from, contract_caller);

            let amount;
            if sender_delta > receiver_delta {
                amount = receiver_delta;
                sender_delta -= receiver_delta;
                receiver_idx += 1;
                (receiver_pk, receiver_delta) = balance_receivers[receiver_idx];
                assert!(is_evm_pubkey(receiver_pk));
            } else if sender_delta < receiver_delta {
                amount = sender_delta;
                receiver_delta -= sender_delta;
                sender_idx += 1;
                (sender_pk, sender_delta) = balance_senders[sender_idx];
                assert!(is_evm_pubkey(sender_pk));
            } else {
                sender_idx += 1;
                receiver_idx += 1;
                amount = sender_delta;
                if sender_idx < balance_senders.len() {
                    (receiver_pk, receiver_delta) = balance_receivers[receiver_idx];
                    (sender_pk, sender_delta) = balance_senders[sender_idx];
                    assert!(is_evm_pubkey(receiver_pk));
                    assert!(is_evm_pubkey(sender_pk));
                } else {
                    run = false;
                }
            }

            sdk.call(address_to, evm_balance_from_lamports(amount), &[], None)
                .expect("failed to send while settling evm-svm balances");
        }
    }
}
