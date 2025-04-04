use crate::account::{AccountSharedData, WritableAccount};
use crate::bpf_loader_upgradable::Entrypoint;
use crate::builtins::register_builtins;
use crate::common::TestSdkType;
use crate::context::{BuiltinFunctionWithContext, IndexOfAccount, InstructionAccount, InvokeContext, TransactionAccount};
use crate::error::InstructionError;
use crate::helpers::{create_account_shared_data_for_test, test_utils};
use crate::loaded_programs::LoadedProgram;
use crate::{native_loader, with_mock_invoke_context};
use core::cell::RefCell;
use fluentbase_sdk::testing::TestingContext;
use fluentbase_sdk::{Address, ContractContextV1, SharedAPI, U256};
use solana_program::epoch_schedule::EpochSchedule;
use solana_program::instruction::AccountMeta;
use solana_program::pubkey::Pubkey;
use solana_program::rent::Rent;
use solana_program::sysvar;
use solana_rbpf::program::{BuiltinFunction, BuiltinProgram, FunctionRegistry};
use solana_rbpf::vm::Config;
use std::fs::File;
use std::io::Read;
use alloc::sync::Arc;

pub(crate) fn prepare_vars_for_tests<'a, SDK: SharedAPI>() -> (Config, Arc<BuiltinProgram<InvokeContext<'a, SDK>>>) {
    let config = Config {
        enable_instruction_tracing: false,
        reject_broken_elfs: true,
        sanitize_user_provided_values: true,
        ..Default::default()
    };
    // Holds the function symbols of an Executable
    let mut function_registry: FunctionRegistry<BuiltinFunction<InvokeContext<SDK>>> = FunctionRegistry::<BuiltinFunction<InvokeContext<SDK>>>::default();
    register_builtins(&mut function_registry);
    let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));

    (config, loader)
}
pub fn load_program_account_from_elf_file(loader_id: &Pubkey, path: &str) -> AccountSharedData {
    let mut file = File::open(path).expect("file open failed");
    let mut elf = Vec::new();
    file.read_to_end(&mut elf).unwrap();
    let rent = Rent::default();
    let minimum_balance = rent.minimum_balance(elf.len());
    let mut program_account = AccountSharedData::new(minimum_balance, 0, loader_id);
    program_account.set_data(elf);
    program_account.set_executable(true);
    program_account
}
pub(crate) fn mock_process_instruction<'a, SDK: SharedAPI, F: FnMut(&mut InvokeContext<SDK>), G: FnMut(&mut InvokeContext<SDK>)>(
    sdk: &'a SDK,
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
    let mut instruction_accounts: Vec<InstructionAccount> = Vec::with_capacity(instruction_account_metas.len());
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

    // let config = Config {
    //     enable_instruction_tracing: false,
    //     reject_broken_elfs: true,
    //     sanitize_user_provided_values: true,
    //     ..Default::default()
    // };
    // // Holds the function symbols of an Executable
    // let mut function_registry: FunctionRegistry<BuiltinFunction<InvokeContext<SDK>>> = FunctionRegistry::<BuiltinFunction<InvokeContext<SDK>>>::default();
    // register_builtins(&mut function_registry);
    // let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));

    with_mock_invoke_context!(invoke_context, transaction_context, sdk, loader, transaction_accounts);

    invoke_context.programs_loaded_for_tx_batch.replenish(
        *loader_id,
        Arc::new(LoadedProgram::new_builtin(0, 0, builtin_function)),
    );
    pre_adjustments(&mut invoke_context);
    let result = invoke_context.process_instruction(
        instruction_data,
        &instruction_accounts,
        &program_indices,
        // &mut 0,
        // &mut ExecuteTimings::default(),
    );
    assert_eq!(result, expected_result);
    post_adjustments(&mut invoke_context);
    let mut transaction_accounts = invoke_context.transaction_context.deconstruct_without_keys().unwrap();
    if pop_epoch_schedule_account {
        transaction_accounts.pop();
    }
    transaction_accounts.pop();
    transaction_accounts
}

pub(crate) fn process_instruction<SDK: SharedAPI>(
    sdk: &SDK,
    loader_id: &Pubkey,
    program_indices: &[IndexOfAccount],
    instruction_data: &[u8],
    transaction_accounts: Vec<(Pubkey, AccountSharedData)>,
    instruction_accounts: Vec<AccountMeta>,
    expected_result: Result<(), InstructionError>,
) -> Vec<AccountSharedData> {
    mock_process_instruction(
        sdk,
        loader_id,
        program_indices.to_vec(),
        instruction_data,
        transaction_accounts,
        instruction_accounts,
        expected_result,
        Entrypoint::vm,
        |invoke_context| {
            test_utils::load_all_invoked_programs(invoke_context);
        },
        |_invoke_context| {},
    )
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
pub(crate) fn journal_state() -> TestingContext {
    let mut tc = TestingContext::default();
    let cc = contract_context();
    tc.with_contract_context(cc)
}

pub(crate) fn new_test_sdk_rc() -> Arc<RefCell<TestSdkType>> {
    Arc::new(RefCell::new(new_test_sdk()))
}

pub(crate) fn new_test_sdk() -> TestSdkType {
    journal_state()
}
