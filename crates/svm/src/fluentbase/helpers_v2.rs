use crate::{
    account::{AccountSharedData, ReadableAccount},
    builtins::register_builtins,
    common::compile_accounts_for_tx_ctx,
    compute_budget::ComputeBudget,
    context::{IndexOfAccount, InvokeContext, TransactionContext},
    error::{InstructionError, SvmError},
    fluentbase::common::{
        extract_account_data_or_default,
        flush_accounts,
        load_program_account,
        BatchMessage,
        SYSTEM_PROGRAMS_KEYS,
    },
    loaded_programs::{LoadedProgram, LoadedProgramsForTxBatch, ProgramRuntimeEnvironments},
    loaders::{bpf_loader_v4, bpf_loader_v4::get_state},
    message_processor::MessageProcessor,
    solana_program::{
        loader_v4,
        loader_v4::{LoaderV4State, LoaderV4Status},
        message::{legacy, LegacyMessage, SanitizedMessage},
    },
    system_processor,
    system_program,
    sysvar_cache::SysvarCache,
};
use alloc::{sync::Arc, vec, vec::Vec};
use fluentbase_sdk::{BlockContextReader, SharedAPI, StorageAPI};
use hashbrown::{HashMap, HashSet};
use itertools::Itertools;
use solana_bincode::deserialize;
use solana_clock::Clock;
use solana_epoch_schedule::EpochSchedule;
use solana_feature_set::{bpf_account_data_direct_mapping, FeatureSet};
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
        aligned_memory_mapping: true,
        enable_address_translation: true, // To be deactivated once we have BTF inference and verification
        enable_stack_frame_gaps: true,
        ..Default::default()
    }
}

pub fn exec_encoded_svm_batch_message<SDK: SharedAPI, SAPI: StorageAPI>(
    sdk: &mut SDK,
    batch_message: &[u8],
    flush_result_accounts: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<HashMap<Pubkey, AccountSharedData>, SvmError> {
    let batch_message = deserialize(batch_message)?;
    exec_svm_batch_message(sdk, batch_message, flush_result_accounts, sapi)
}
pub fn exec_svm_batch_message<SDK: SharedAPI, SAPI: StorageAPI>(
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
pub fn exec_encoded_svm_message<SDK: SharedAPI, SAPI: StorageAPI>(
    sdk: &mut SDK,
    message: &[u8],
    flush_result_accounts: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<HashMap<Pubkey, AccountSharedData>, SvmError> {
    let message = deserialize(message)?;
    exec_svm_message(sdk, message, flush_result_accounts, sapi)
}

pub fn exec_svm_message<SDK: SharedAPI, SAPI: StorageAPI>(
    sdk: &mut SDK,
    message: legacy::Message,
    flush_result_accounts: bool,
    sapi: &mut Option<&mut SAPI>,
) -> Result<HashMap<Pubkey, AccountSharedData>, SvmError> {
    let message: SanitizedMessage =
        SanitizedMessage::Legacy(LegacyMessage::new(message, &Default::default()));

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
    let loader_id = loader_v4::id();

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
            let state: Result<&LoaderV4State, InstructionError> = get_state(program_account.data());
            if let Ok(state) = state {
                match state.status {
                    LoaderV4Status::Deployed | LoaderV4Status::Finalized => {
                        program_accounts_to_warmup.push(account_key);
                    }
                    _ => {}
                }
            }
            program_indices
                .last_mut()
                .unwrap()
                .push(program_accounts.len() as IndexOfAccount);
            program_accounts.push((account_key.clone(), program_account));
        }
        // for idx in &instruction.accounts {
        //     let account_key = account_keys.get(*idx as usize).unwrap();
        //     program_accounts_to_warmup.push(account_key);
        // }
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
    let transaction_context = TransactionContext::new(accounts, rent.clone(), 100, 200);

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
        loader_id,
        Arc::new(LoadedProgram::new_builtin(
            0,
            0,
            bpf_loader_v4::Entrypoint::vm,
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
        let mut feature_set = FeatureSet::all_enabled();
        feature_set.deactivate(&bpf_account_data_direct_mapping::id());

        let mut invoke_context = InvokeContext::new(
            transaction_context,
            sysvar_cache.clone(),
            sdk,
            compute_budget.clone(),
            programs_loaded_for_tx_batch,
            programs_modified_by_tx,
            feature_set.into(),
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
