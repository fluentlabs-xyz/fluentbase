use crate::{
    account::{AccountSharedData, ReadableAccount, WritableAccount},
    context::InstructionAccount,
    declare_process_instruction,
    loaded_programs::ProgramCacheEntry,
    native_loader,
    solana_program::instruction::{AccountMeta, Instruction},
    system_instruction::{SystemError, SystemInstruction},
    system_processor::Entrypoint,
    system_program,
    test_helpers::{mock_process_instruction, new_test_sdk, prepare_vars_for_tests},
    with_mock_invoke_context,
};
use fluentbase_sdk::SharedAPI;
use serde::{Deserialize, Serialize};
use solana_bincode::{deserialize, serialize};
use solana_instruction::error::InstructionError;
use solana_pubkey::{Pubkey, PUBKEY_BYTES};
use solana_stable_layout::stable_instruction::StableInstruction;

#[derive(Debug, Serialize, Deserialize)]
enum MockInstruction {
    NoopSuccess,
    NoopFail,
    ModifyOwned,
    ModifyNotOwned,
    ModifyReadonly,
    UnbalancedPush,
    UnbalancedPop,
    ConsumeComputeUnits {
        compute_units_to_consume: u64,
        desired_result: Result<(), InstructionError>,
    },
    Resize {
        new_len: u64,
    },
}

declare_process_instruction!(
    MockBuiltin<SDK: SharedAPI>,
    MOCK_BUILTIN_COMPUTE_UNIT_COST,
    |invoke_context| {
        let transaction_context = &invoke_context.transaction_context;
        let instruction_context = transaction_context.get_current_instruction_context()?;
        let instruction_data = instruction_context.get_instruction_data();
        let program_id = instruction_context.get_last_program_key(transaction_context)?;
        let instruction_accounts = (0..4)
            .map(|instruction_account_index| InstructionAccount {
                index_in_transaction: instruction_account_index,
                index_in_caller: instruction_account_index,
                index_in_callee: instruction_account_index,
                is_signer: false,
                is_writable: false,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            program_id,
            instruction_context
                .try_borrow_instruction_account(transaction_context, 0)?
                .get_owner()
        );
        assert_ne!(
            instruction_context
                .try_borrow_instruction_account(transaction_context, 1)?
                .get_owner(),
            instruction_context
                .try_borrow_instruction_account(transaction_context, 0)?
                .get_key()
        );

        if let Ok(instruction) = deserialize(instruction_data) {
            match instruction {
                MockInstruction::NoopSuccess => (),
                MockInstruction::NoopFail => return Err(InstructionError::GenericError),
                MockInstruction::ModifyOwned => instruction_context
                    .try_borrow_instruction_account(transaction_context, 0)?
                    .set_data_from_slice(&[1])?,
                MockInstruction::ModifyNotOwned => instruction_context
                    .try_borrow_instruction_account(transaction_context, 1)?
                    .set_data_from_slice(&[1])?,
                MockInstruction::ModifyReadonly => instruction_context
                    .try_borrow_instruction_account(transaction_context, 2)?
                    .set_data_from_slice(&[1])?,
                MockInstruction::UnbalancedPush => {
                    instruction_context
                        .try_borrow_instruction_account(transaction_context, 0)?
                        .checked_add_lamports(1)?;
                    let program_id = *transaction_context.get_key_of_account_at_index(3)?;
                    let metas = vec![
                        AccountMeta::new_readonly(
                            *transaction_context.get_key_of_account_at_index(0)?,
                            false,
                        ),
                        AccountMeta::new_readonly(
                            *transaction_context.get_key_of_account_at_index(1)?,
                            false,
                        ),
                    ];
                    let inner_instruction = Instruction::new_with_bincode(
                        program_id,
                        &MockInstruction::NoopSuccess,
                        metas,
                    );
                    invoke_context
                        .transaction_context
                        .get_next_instruction_context()?
                        .configure(&[3], &instruction_accounts, &[]);
                    let result = invoke_context.push();
                    assert_eq!(result, Err(InstructionError::UnbalancedInstruction));
                    result?;
                    invoke_context
                        .native_invoke(inner_instruction.into(), &[])
                        .and(invoke_context.pop())?;
                }
                MockInstruction::UnbalancedPop => instruction_context
                    .try_borrow_instruction_account(transaction_context, 0)?
                    .checked_add_lamports(1)?,
                MockInstruction::ConsumeComputeUnits {
                    compute_units_to_consume: _,
                    desired_result,
                } => {
                    return desired_result;
                }
                MockInstruction::Resize { new_len } => instruction_context
                    .try_borrow_instruction_account(transaction_context, 0)?
                    .set_data(vec![0; new_len as usize])?,
            }
        } else {
            return Err(InstructionError::InvalidInstructionData);
        }
        Ok(())
    }
);

#[test]
fn test_process_instruction() {
    let callee_program_id = Pubkey::new_unique();
    let owned_account = AccountSharedData::new(42, 1, &callee_program_id);
    let not_owned_account = AccountSharedData::new(84, 1, &Pubkey::new_unique());
    let readonly_account = AccountSharedData::new(168, 1, &Pubkey::new_unique());
    let loader_account = AccountSharedData::new(0, 1, &native_loader::id());
    let mut program_account = AccountSharedData::new(1, 1, &native_loader::id());
    program_account.set_executable(true);
    let transaction_accounts = vec![
        (Pubkey::new_unique(), owned_account),
        (Pubkey::new_unique(), not_owned_account),
        (Pubkey::new_unique(), readonly_account),
        (callee_program_id, program_account),
        (Pubkey::new_unique(), loader_account),
    ];
    let metas = vec![
        AccountMeta::new(transaction_accounts.first().unwrap().0, false),
        AccountMeta::new(transaction_accounts.get(1).unwrap().0, false),
        AccountMeta::new_readonly(transaction_accounts.get(2).unwrap().0, false),
    ];
    let instruction_accounts = (0..4)
        .map(|instruction_account_index| InstructionAccount {
            index_in_transaction: instruction_account_index,
            index_in_caller: instruction_account_index,
            index_in_callee: instruction_account_index,
            is_signer: false,
            is_writable: instruction_account_index < 2,
        })
        .collect::<Vec<_>>();
    let mut sdk = new_test_sdk();
    let (_config, loader) = prepare_vars_for_tests();
    with_mock_invoke_context!(
        invoke_context,
        transaction_context,
        &mut sdk,
        loader,
        transaction_accounts
    );
    let mut invoke_context = invoke_context;
    let mut program_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
        Default::default(),
        ProgramRuntimeEnvironments {
            program_runtime_v1: loader.clone(),
            program_runtime_v2: loader.clone(),
        },
    );
    program_cache_for_tx_batch.replenish(
        callee_program_id,
        Arc::new(ProgramCacheEntry::new_builtin(0, 1, MockBuiltin::vm)),
    );
    invoke_context.program_cache_for_tx_batch = program_cache_for_tx_batch;

    // Account modification tests
    let cases = vec![
        (MockInstruction::NoopSuccess, Ok(())),
        (
            MockInstruction::NoopFail,
            Err(InstructionError::GenericError),
        ),
        (MockInstruction::ModifyOwned, Ok(())),
        (
            MockInstruction::ModifyNotOwned,
            Err(InstructionError::ExternalAccountDataModified),
        ),
        (
            MockInstruction::ModifyReadonly,
            Err(InstructionError::ReadonlyDataModified),
        ),
        (
            MockInstruction::UnbalancedPush,
            Err(InstructionError::UnbalancedInstruction),
        ),
        (
            MockInstruction::UnbalancedPop,
            Err(InstructionError::UnbalancedInstruction),
        ),
    ];
    for case in cases {
        invoke_context
            .transaction_context
            .get_next_instruction_context()
            .unwrap()
            .configure(&[4], &instruction_accounts, &[]);
        invoke_context.push().unwrap();
        let inner_instruction =
            Instruction::new_with_bincode(callee_program_id, &case.0, metas.clone());
        let result = invoke_context
            .native_invoke(inner_instruction.into(), &[])
            .and(invoke_context.pop());
        assert_eq!(result, case.1);
    }

    // Compute unit consumption tests
    let compute_units_to_consume = 10;
    let expected_results = vec![Ok(()), Err(InstructionError::GenericError)];
    for expected_result in expected_results {
        invoke_context
            .transaction_context
            .get_next_instruction_context()
            .unwrap()
            .configure(&[4], &instruction_accounts, &[]);
        invoke_context.push().unwrap();
        let inner_instruction = Instruction::new_with_bincode(
            callee_program_id,
            &MockInstruction::ConsumeComputeUnits {
                compute_units_to_consume,
                desired_result: expected_result.clone(),
            },
            metas.clone(),
        );
        let inner_instruction = StableInstruction::from(inner_instruction);
        let (inner_instruction_accounts, program_indices) = invoke_context
            .prepare_instruction(&inner_instruction, &[])
            .unwrap();

        // let mut compute_units_consumed = 0;
        let result = invoke_context.process_instruction(
            &inner_instruction.data,
            &inner_instruction_accounts,
            &program_indices,
            // &mut compute_units_consumed,
            // &mut ExecuteTimings::default(),
        );

        // Because the instruction had compute cost > 0, then regardless of the execution result,
        // the number of compute units consumed should be a non-default which is something greater
        // than zero.
        // assert!(compute_units_consumed > 0);
        // assert_eq!(
        //     compute_units_consumed,
        //     compute_units_to_consume.saturating_add(MOCK_BUILTIN_COMPUTE_UNIT_COST),
        // );
        assert_eq!(result, expected_result);

        invoke_context.pop().unwrap();
    }
}

fn process_instruction<SDK: SharedAPI>(
    sdk: &mut SDK,
    instruction_data: &[u8],
    transaction_accounts: Vec<(Pubkey, AccountSharedData)>,
    instruction_accounts: Vec<AccountMeta>,
    expected_result: Result<(), InstructionError>,
) -> Vec<AccountSharedData> {
    mock_process_instruction(
        sdk,
        &system_program::id(),
        Vec::new(),
        instruction_data,
        transaction_accounts,
        instruction_accounts,
        expected_result,
        Entrypoint::vm,
        |_invoke_context| {},
        |_invoke_context| {},
    )
}

#[test]
fn test_transfer_lamports() {
    let mut sdk = new_test_sdk();

    let from = Pubkey::new_unique();
    let from_account = AccountSharedData::new(100, 0, &system_program::id());
    let to = Pubkey::from([3; PUBKEY_BYTES]);
    let to_account = AccountSharedData::new(1, 0, &to); // account owner should not matter
    let transaction_accounts = vec![
        (from.clone(), from_account.clone()),
        (to.clone(), to_account.clone()),
    ];
    let instruction_accounts = vec![
        AccountMeta {
            pubkey: from,
            is_signer: true,
            is_writable: true,
        },
        AccountMeta {
            pubkey: to,
            is_signer: false,
            is_writable: true,
        },
    ];

    // Success case
    let accounts = process_instruction(
        &mut sdk,
        &serialize(&SystemInstruction::Transfer { lamports: 50 }).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );
    assert_eq!(accounts[0].lamports(), 50);
    assert_eq!(accounts[0].data(), from_account.data());
    assert_eq!(accounts[1].lamports(), 51);
    assert_eq!(accounts[0].data(), to_account.data());

    // Attempt to move more lamports than from_account has
    let accounts = process_instruction(
        &mut sdk,
        &serialize(&SystemInstruction::Transfer { lamports: 101 }).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(SystemError::ResultWithNegativeLamports.into()),
    );
    assert_eq!(accounts[0].lamports(), 100);
    assert_eq!(accounts[1].lamports(), 1);

    // test signed transfer of zero
    let accounts = process_instruction(
        &mut sdk,
        &serialize(&SystemInstruction::Transfer { lamports: 0 }).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts,
        Ok(()),
    );
    assert_eq!(accounts[0].lamports(), 100);
    assert_eq!(accounts[1].lamports(), 1);

    // test unsigned transfer of zero
    let accounts = process_instruction(
        &mut sdk,
        &serialize(&SystemInstruction::Transfer { lamports: 0 }).unwrap(),
        transaction_accounts,
        vec![
            AccountMeta {
                pubkey: from,
                is_signer: false,
                is_writable: true,
            },
            AccountMeta {
                pubkey: to,
                is_signer: false,
                is_writable: true,
            },
        ],
        Err(InstructionError::MissingRequiredSignature),
    );
    assert_eq!(accounts[0].lamports(), 100);
    assert_eq!(accounts[1].lamports(), 1);
}
