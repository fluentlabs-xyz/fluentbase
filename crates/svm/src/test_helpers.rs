use crate::{
    account::{AccountSharedData, WritableAccount},
    builtins::register_builtins,
    common::rbpf_config_default,
    context::{
        BuiltinFunctionWithContext, IndexOfAccount, InstructionAccount, InvokeContext,
        TransactionAccount,
    },
    helpers::create_account_shared_data_for_test,
    loaded_programs::ProgramCacheEntry,
    native_loader,
    pubkey::Pubkey,
    solana_program::{instruction::AccountMeta, sysvar},
    with_mock_invoke_context,
};
use alloc::sync::Arc;
use fluentbase_sdk::{Address, ContractContextV1, SharedAPI, U256};
use fluentbase_testing::HostTestingContext;
use solana_epoch_schedule::EpochSchedule;
use solana_instruction::error::InstructionError;
use solana_rbpf::{
    program::{BuiltinFunction, BuiltinProgram, FunctionRegistry},
    vm::Config,
};
use std::{fs::File, io::Read};

pub(crate) fn prepare_vars_for_tests<'a, SDK: SharedAPI>(
) -> (Config, Arc<BuiltinProgram<InvokeContext<'a, SDK>>>) {
    let config = rbpf_config_default(None);
    // Holds the function symbols of an Executable
    let mut function_registry: FunctionRegistry<BuiltinFunction<InvokeContext<SDK>>> =
        FunctionRegistry::<BuiltinFunction<InvokeContext<SDK>>>::default();
    register_builtins(&mut function_registry);
    let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));

    (config, loader)
}
pub(crate) fn mock_process_instruction<
    'a,
    SDK: SharedAPI,
    F: FnMut(&mut InvokeContext<SDK>),
    G: FnMut(&mut InvokeContext<SDK>),
>(
    sdk: &'a mut SDK,
    loader_id: &Pubkey,
    mut program_indices: Vec<IndexOfAccount>,
    instruction_data: &[u8],
    mut transaction_accounts: Vec<TransactionAccount>,
    instruction_account_metas: Vec<AccountMeta>,
    expected_result: Result<(), InstructionError>,
    builtin_function: BuiltinFunctionWithContext<'a, SDK>,
    mut pre_adjustments: F,
    mut post_adjustments: G,
) -> Vec<AccountSharedData> {
    let mut instruction_accounts: Vec<InstructionAccount> =
        Vec::with_capacity(instruction_account_metas.len());
    for (instruction_account_index, account_meta) in instruction_account_metas.iter().enumerate() {
        let index_in_transaction = transaction_accounts
            .iter()
            .position(|(key, _account)| *key == account_meta.pubkey)
            .unwrap_or(transaction_accounts.len())
            as IndexOfAccount;
        let index_in_callee = instruction_accounts
            .get(0..instruction_account_index)
            .unwrap()
            .iter()
            .position(|instruction_account| {
                instruction_account.index_in_transaction == index_in_transaction
            })
            .unwrap_or(instruction_account_index) as IndexOfAccount;
        instruction_accounts.push(InstructionAccount {
            index_in_transaction,
            index_in_caller: index_in_transaction,
            index_in_callee,
            is_signer: account_meta.is_signer,
            is_writable: account_meta.is_writable,
        });
    }
    program_indices.insert(0, transaction_accounts.len() as IndexOfAccount);
    let processor_account = AccountSharedData::new(0, 0, &native_loader::id());
    transaction_accounts.push((*loader_id, processor_account));
    let pop_epoch_schedule_account = if !transaction_accounts
        .iter()
        .any(|(key, _)| *key == sysvar::epoch_schedule::id())
    {
        transaction_accounts.push((
            sysvar::epoch_schedule::id(),
            create_account_shared_data_for_test(&EpochSchedule::default()),
        ));
        true
    } else {
        false
    };

    let (_config, loader) = prepare_vars_for_tests();

    with_mock_invoke_context!(
        invoke_context,
        transaction_context,
        sdk,
        loader,
        transaction_accounts
    );
    let mut invoke_context = invoke_context;

    invoke_context.program_cache_for_tx_batch.replenish(
        *loader_id,
        Arc::new(ProgramCacheEntry::new_builtin(0, 0, builtin_function)),
    );
    pre_adjustments(&mut invoke_context);
    let result = invoke_context.process_instruction(
        instruction_data,
        &instruction_accounts,
        &program_indices,
    );
    assert_eq!(result, expected_result);
    post_adjustments(&mut invoke_context);
    let mut transaction_accounts = invoke_context
        .transaction_context
        .deconstruct_without_keys()
        .unwrap();
    if pop_epoch_schedule_account {
        transaction_accounts.pop();
    }
    transaction_accounts.pop();
    transaction_accounts
}

pub(crate) fn contract_context() -> ContractContextV1 {
    ContractContextV1 {
        address: Address::from_slice(&[01; 20]),
        bytecode_address: Address::from_slice(&[00; 20]),
        caller: Address::from_slice(&[00; 20]),
        is_static: false,
        value: U256::default(),
        gas_limit: 0,
    }
}
pub(crate) fn journal_state() -> HostTestingContext {
    let tc = HostTestingContext::default();
    let cc = contract_context();
    tc.with_contract_context(cc)
}

pub(crate) fn new_test_sdk() -> HostTestingContext {
    journal_state()
}
