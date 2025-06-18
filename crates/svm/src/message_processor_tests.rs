#[cfg(test)]
pub mod tests {
    use crate::{
        account::{AccountSharedData, ReadableAccount, DUMMY_INHERITABLE_ACCOUNT_FIELDS},
        bpf_loader,
        builtins::register_builtins,
        clock::Clock,
        common::{calculate_max_chunk_size, compile_accounts_for_tx_ctx, TestSdkType},
        compute_budget::compute_budget::ComputeBudget,
        context::{EnvironmentConfig, IndexOfAccount, InvokeContext, TransactionContext},
        declare_process_instruction,
        epoch_schedule::EpochSchedule,
        hash::Hash,
        helpers::load_program_account_from_elf_file,
        loaded_programs::{ProgramCacheEntry, ProgramCacheForTxBatch, ProgramRuntimeEnvironments},
        message_processor::MessageProcessor,
        native_loader,
        native_loader::create_loadable_account_for_test,
        rent::Rent,
        solana_program::{
            bpf_loader_upgradeable,
            bpf_loader_upgradeable::UpgradeableLoaderState,
            feature_set::feature_set_default,
            message::{AccountKeys, LegacyMessage, Message, SanitizedMessage},
            sysvar,
        },
        system_instruction::{SystemError, SystemInstruction},
        system_processor,
        system_program,
        sysvar_cache::SysvarCache,
        test_helpers::journal_state,
    };
    use alloc::{sync::Arc, vec, vec::Vec};
    use fluentbase_sdk::SharedAPI;
    use serde::{Deserialize, Serialize};
    use solana_bincode::deserialize;
    use solana_instruction::{error::InstructionError, AccountMeta, Instruction};
    use solana_pubkey::Pubkey;
    use solana_rbpf::{
        program::{BuiltinFunction, BuiltinProgram, FunctionRegistry},
        vm::Config,
    };
    use solana_transaction_error::TransactionError;

    #[test]
    fn test_process_message_readonly_handling_mocked() {
        let config = Config {
            enable_instruction_tracing: false,
            reject_broken_elfs: true,
            sanitize_user_provided_values: true,
            ..Default::default()
        };

        let sdk = journal_state();

        let writable_pubkey = Pubkey::new_unique();
        let readonly_pubkey = Pubkey::new_unique();
        let mock_system_program_id = Pubkey::new_unique();

        let blockhash = Hash::default();

        let accounts = vec![
            (
                writable_pubkey,
                AccountSharedData::new(100, 1, &mock_system_program_id),
            ),
            (
                readonly_pubkey,
                AccountSharedData::new(0, 1, &mock_system_program_id),
            ),
            (
                mock_system_program_id,
                create_loadable_account_for_test("mock_system_program", &native_loader::id()),
            ),
        ];
        let transaction_context = TransactionContext::new(accounts, Default::default(), 1, 3);
        let program_indices = vec![vec![2]];

        let account_keys = (0..transaction_context.get_number_of_accounts())
            .map(|index| {
                *transaction_context
                    .get_key_of_account_at_index(index)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let account_metas = vec![
            AccountMeta::new(writable_pubkey, true),
            AccountMeta::new_readonly(readonly_pubkey, false),
        ];

        let function_registry =
            FunctionRegistry::<BuiltinFunction<InvokeContext<TestSdkType>>>::default();
        // register_builtins(&mut function_registry);
        let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));

        #[derive(Serialize, Deserialize, bincode::Encode, bincode::Decode)]
        enum MockSystemInstruction {
            Correct,
            Transfer { lamports: u64 },
            ChangeData { data: u8 },
        }

        declare_process_instruction!(MockBuiltin<SDK: SharedAPI>, 1, |invoke_context| {
            let transaction_context = &invoke_context.transaction_context;
            let instruction_context = transaction_context.get_current_instruction_context()?;
            let instruction_data = instruction_context.get_instruction_data();
            if let Ok(instruction) = deserialize(instruction_data) {
                match instruction {
                    MockSystemInstruction::Correct => Ok(()),
                    MockSystemInstruction::Transfer { lamports } => {
                        instruction_context
                            .try_borrow_instruction_account(transaction_context, 0)?
                            .checked_sub_lamports(lamports)?;
                        instruction_context
                            .try_borrow_instruction_account(transaction_context, 1)?
                            .checked_add_lamports(lamports)?;
                        Ok(())
                    }
                    MockSystemInstruction::ChangeData { data } => {
                        instruction_context
                            .try_borrow_instruction_account(transaction_context, 1)?
                            .set_data(vec![data])?;
                        Ok(())
                    }
                }
            } else {
                Err(InstructionError::InvalidInstructionData)
            }
        });

        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            mock_system_program_id,
            Arc::new(ProgramCacheEntry::new_builtin(0, 0, MockBuiltin::vm)),
        );

        let message = SanitizedMessage::Legacy(LegacyMessage::new(
            Message::new_with_compiled_instructions(
                1,
                0,
                2,
                account_keys.clone(),
                blockhash,
                AccountKeys::new(&account_keys, None).compile_instructions(&[
                    Instruction::new_with_bincode(
                        mock_system_program_id,
                        &MockSystemInstruction::Correct,
                        account_metas.clone(),
                    ),
                ]),
            ),
            &Default::default(),
        ));

        let compute_budget = ComputeBudget::default();
        let sysvar_cache = SysvarCache::default();
        let environment_config = EnvironmentConfig::new(
            blockhash,
            None,
            Arc::new(feature_set_default()),
            0,
            sysvar_cache,
        );
        let mut invoke_context = InvokeContext::new(
            transaction_context,
            programs_cache_for_tx_batch,
            environment_config,
            compute_budget,
            &sdk,
        );
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        assert!(result.is_ok());
        assert_eq!(
            invoke_context
                .transaction_context
                .get_account_at_index(0)
                .unwrap()
                .borrow()
                .lamports(),
            100
        );
        assert_eq!(
            invoke_context
                .transaction_context
                .get_account_at_index(1)
                .unwrap()
                .borrow()
                .lamports(),
            0
        );

        let message = SanitizedMessage::Legacy(LegacyMessage::new(
            Message::new_with_compiled_instructions(
                1,
                0,
                2,
                account_keys.clone(),
                blockhash,
                AccountKeys::new(&account_keys, None).compile_instructions(&[
                    Instruction::new_with_bincode(
                        mock_system_program_id,
                        &MockSystemInstruction::Transfer { lamports: 50 },
                        account_metas.clone(),
                    ),
                ]),
            ),
            &Default::default(),
        ));
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        assert_eq!(
            result,
            Err(TransactionError::InstructionError(
                0,
                InstructionError::ReadonlyLamportChange
            ))
        );

        let message = SanitizedMessage::Legacy(LegacyMessage::new(
            Message::new_with_compiled_instructions(
                1,
                0,
                2,
                account_keys.clone(),
                blockhash,
                AccountKeys::new(&account_keys, None).compile_instructions(&[
                    Instruction::new_with_bincode(
                        mock_system_program_id,
                        &MockSystemInstruction::ChangeData { data: 50 },
                        account_metas,
                    ),
                ]),
            ),
            &Default::default(),
        ));
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        assert_eq!(
            result,
            Err(TransactionError::InstructionError(
                0,
                InstructionError::ReadonlyDataModified
            ))
        );
    }

    #[test]
    fn test_process_message_duplicate_accounts_mocked() {
        let config = Config {
            enable_instruction_tracing: false,
            reject_broken_elfs: true,
            sanitize_user_provided_values: true,
            ..Default::default()
        };
        let sdk = journal_state();

        let blockhash = Hash::default();

        let function_registry =
            FunctionRegistry::<BuiltinFunction<InvokeContext<TestSdkType>>>::default();
        // register_builtins(&mut function_registry);
        let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));

        #[derive(Serialize, Deserialize, bincode::Encode, bincode::Decode)]
        enum MockSystemInstruction {
            BorrowFail,
            MultiBorrowMut,
            DoWork { lamports: u64, data: u8 },
        }

        declare_process_instruction!(MockBuiltin<SDK: SharedAPI>, 1, |invoke_context| {
            let transaction_context = &invoke_context.transaction_context;
            let instruction_context = transaction_context.get_current_instruction_context()?;
            let instruction_data = instruction_context.get_instruction_data();
            let mut to_account =
                instruction_context.try_borrow_instruction_account(transaction_context, 1)?;
            if let Ok(instruction) = deserialize(instruction_data) {
                match instruction {
                    MockSystemInstruction::BorrowFail => {
                        let from_account = instruction_context
                            .try_borrow_instruction_account(transaction_context, 0)?;
                        let dup_account = instruction_context
                            .try_borrow_instruction_account(transaction_context, 2)?;
                        if from_account.get_lamports() != dup_account.get_lamports() {
                            return Err(InstructionError::InvalidArgument);
                        }
                        Ok(())
                    }
                    MockSystemInstruction::MultiBorrowMut => {
                        let lamports_a = instruction_context
                            .try_borrow_instruction_account(transaction_context, 0)?
                            .get_lamports();
                        let lamports_b = instruction_context
                            .try_borrow_instruction_account(transaction_context, 2)?
                            .get_lamports();
                        if lamports_a != lamports_b {
                            return Err(InstructionError::InvalidArgument);
                        }
                        Ok(())
                    }
                    MockSystemInstruction::DoWork { lamports, data } => {
                        let mut dup_account = instruction_context
                            .try_borrow_instruction_account(transaction_context, 2)?;
                        dup_account.checked_sub_lamports(lamports)?;
                        to_account.checked_add_lamports(lamports)?;
                        dup_account.set_data(vec![data])?;
                        drop(dup_account);
                        let mut from_account = instruction_context
                            .try_borrow_instruction_account(transaction_context, 0)?;
                        from_account.checked_sub_lamports(lamports)?;
                        to_account.checked_add_lamports(lamports)?;
                        Ok(())
                    }
                }
            } else {
                Err(InstructionError::InvalidInstructionData)
            }
        });
        let mock_program_id = Pubkey::from([2u8; 32]);
        let accounts = vec![
            (
                Pubkey::from(rand::random::<[u8; solana_pubkey::PUBKEY_BYTES]>()),
                AccountSharedData::new(100, 1, &mock_program_id),
            ),
            (
                Pubkey::from(rand::random::<[u8; solana_pubkey::PUBKEY_BYTES]>()),
                AccountSharedData::new(0, 1, &mock_program_id),
            ),
            (
                mock_program_id,
                create_loadable_account_for_test("mock_system_program", &native_loader::id()),
            ),
        ];
        let transaction_context = TransactionContext::new(accounts, Rent::default(), 1, 3);
        let program_indices = vec![vec![2]];
        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            mock_program_id,
            Arc::new(ProgramCacheEntry::new_builtin(0, 0, MockBuiltin::vm)),
        );
        let account_metas = vec![
            AccountMeta::new(
                *transaction_context.get_key_of_account_at_index(0).unwrap(),
                true,
            ),
            AccountMeta::new(
                *transaction_context.get_key_of_account_at_index(1).unwrap(),
                false,
            ),
            AccountMeta::new(
                *transaction_context.get_key_of_account_at_index(0).unwrap(),
                false,
            ),
        ];

        let message = SanitizedMessage::Legacy(LegacyMessage::new(
            Message::new(
                &[Instruction::new_with_bincode(
                    mock_program_id,
                    &MockSystemInstruction::BorrowFail,
                    account_metas.clone(),
                )],
                Some(transaction_context.get_key_of_account_at_index(0).unwrap()),
            ),
            &Default::default(),
        ));

        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            mock_program_id,
            Arc::new(ProgramCacheEntry::new_builtin(0, 0, MockBuiltin::vm)),
        );
        let compute_budget = ComputeBudget::default();
        let sysvar_cache = SysvarCache::default();
        let environment_config = EnvironmentConfig::new(
            blockhash,
            None,
            Arc::new(feature_set_default()),
            0,
            sysvar_cache,
        );
        let mut invoke_context = InvokeContext::new(
            transaction_context,
            programs_cache_for_tx_batch,
            environment_config,
            compute_budget,
            &sdk,
        );
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        assert_eq!(
            result,
            Err(TransactionError::InstructionError(
                0,
                InstructionError::AccountBorrowFailed
            ))
        );

        let message = SanitizedMessage::Legacy(LegacyMessage::new(
            Message::new(
                &[Instruction::new_with_bincode(
                    mock_program_id,
                    &MockSystemInstruction::MultiBorrowMut,
                    account_metas.clone(),
                )],
                Some(
                    invoke_context
                        .transaction_context
                        .get_key_of_account_at_index(0)
                        .unwrap(),
                ),
            ),
            &Default::default(),
        ));
        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            mock_program_id,
            Arc::new(ProgramCacheEntry::new_builtin(0, 0, MockBuiltin::vm)),
        );
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        assert!(result.is_ok());

        let message = SanitizedMessage::Legacy(LegacyMessage::new(
            Message::new(
                &[Instruction::new_with_bincode(
                    mock_program_id,
                    &MockSystemInstruction::DoWork {
                        lamports: 10,
                        data: 42,
                    },
                    account_metas,
                )],
                Some(
                    invoke_context
                        .transaction_context
                        .get_key_of_account_at_index(0)
                        .unwrap(),
                ),
            ),
            &Default::default(),
        ));
        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            mock_program_id,
            Arc::new(ProgramCacheEntry::new_builtin(0, 0, MockBuiltin::vm)),
        );
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        assert!(result.is_ok());
        assert_eq!(
            invoke_context
                .transaction_context
                .get_account_at_index(0)
                .unwrap()
                .borrow()
                .lamports(),
            80
        );
        assert_eq!(
            invoke_context
                .transaction_context
                .get_account_at_index(1)
                .unwrap()
                .borrow()
                .lamports(),
            20
        );
        assert_eq!(
            invoke_context
                .transaction_context
                .get_account_at_index(0)
                .unwrap()
                .borrow()
                .data(),
            &vec![42]
        );
    }

    #[test]
    fn test_create_account() {
        let config = Config {
            enable_instruction_tracing: false,
            reject_broken_elfs: true,
            sanitize_user_provided_values: true,
            ..Default::default()
        };

        let blockhash = Hash::default();

        let sdk = journal_state();

        let new_owner = Pubkey::from([9; 32]);
        let from = Pubkey::new_unique();
        let from_account = AccountSharedData::new(100, 0, &system_program::id());
        let to = Pubkey::new_unique();
        let to_account = AccountSharedData::new(0, 0, &system_program::id());

        let non_program_accounts_count = 2;

        let system_program_id = system_program::id();
        let native_loader_id = native_loader::id();

        let accounts = vec![
            (from, from_account),
            (to, to_account),
            (
                system_program_id,
                create_loadable_account_for_test("system_program_id", &native_loader_id),
            ),
        ];
        let transaction_context = TransactionContext::new(accounts, Default::default(), 1, 3);
        let program_indices = vec![vec![non_program_accounts_count]];

        let account_keys = (0..transaction_context.get_number_of_accounts())
            .map(|index| {
                *transaction_context
                    .get_key_of_account_at_index(index)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let account_metas = vec![AccountMeta::new(from, true), AccountMeta::new(to, true)];

        let function_registry =
            FunctionRegistry::<BuiltinFunction<InvokeContext<TestSdkType>>>::default();
        // register_builtins(&mut function_registry);
        let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));

        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            system_program_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                system_processor::Entrypoint::vm,
            )),
        );

        let compute_budget = ComputeBudget::default();
        let sysvar_cache = SysvarCache::default();
        let environment_config = EnvironmentConfig::new(
            blockhash,
            None,
            Arc::new(feature_set_default()),
            0,
            sysvar_cache,
        );
        let mut invoke_context = InvokeContext::new(
            transaction_context,
            programs_cache_for_tx_batch,
            environment_config,
            compute_budget,
            &sdk,
        );

        let message = SanitizedMessage::Legacy(LegacyMessage::new(
            Message::new_with_compiled_instructions(
                2,
                0,
                0,
                account_keys.clone(),
                blockhash,
                AccountKeys::new(&account_keys, None).compile_instructions(&[
                    Instruction::new_with_bincode(
                        system_program_id,
                        &SystemInstruction::CreateAccount {
                            lamports: 50,
                            space: 2,
                            owner: new_owner,
                        },
                        account_metas.clone(),
                    ),
                ]),
            ),
            &Default::default(),
        ));
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        {
            assert!(result.is_ok());
            let accounts_count = invoke_context.transaction_context.get_number_of_accounts();
            assert_eq!(accounts_count, 3);
            let account1 = invoke_context
                .transaction_context
                .get_account_at_index(0)
                .unwrap()
                .borrow();
            assert_eq!(50, account1.lamports());
            let account2 = invoke_context
                .transaction_context
                .get_account_at_index(1)
                .unwrap()
                .borrow();
            assert_eq!(50, account2.lamports());
            let account3 = invoke_context
                .transaction_context
                .get_account_at_index(2)
                .unwrap()
                .borrow();
            assert_eq!(DUMMY_INHERITABLE_ACCOUNT_FIELDS.0, account3.lamports());
        }
    }

    #[test]
    fn test_transfer_lamports() {
        let config = Config {
            enable_instruction_tracing: false,
            reject_broken_elfs: true,
            sanitize_user_provided_values: true,
            ..Default::default()
        };

        let blockhash = Hash::default();

        let sdk = journal_state();

        let from = Pubkey::new_unique();
        let to = Pubkey::from([3; 32]);
        let system_program_id = system_program::id();
        let native_loader_id = native_loader::id();

        let accounts = vec![
            (from, AccountSharedData::new(100, 0, &system_program_id)),
            (to, AccountSharedData::new(1, 0, &system_program_id)),
            (
                system_program_id,
                create_loadable_account_for_test("system_program_id", &native_loader_id),
            ),
        ];
        let transaction_context = TransactionContext::new(accounts, Default::default(), 1, 3);
        let program_indices = vec![vec![2]];

        let account_keys = (0..transaction_context.get_number_of_accounts())
            .map(|index| {
                *transaction_context
                    .get_key_of_account_at_index(index)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let account_metas = vec![AccountMeta::new(from, true), AccountMeta::new(to, false)];

        let function_registry =
            FunctionRegistry::<BuiltinFunction<InvokeContext<TestSdkType>>>::default();
        // register_builtins(&mut function_registry);
        let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));

        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            system_program_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                system_processor::Entrypoint::vm,
            )),
        );

        let compute_budget = ComputeBudget::default();
        let sysvar_cache = SysvarCache::default();
        let environment_config = EnvironmentConfig::new(
            blockhash,
            None,
            Arc::new(feature_set_default()),
            0,
            sysvar_cache,
        );
        let mut invoke_context = InvokeContext::new(
            transaction_context,
            programs_cache_for_tx_batch,
            environment_config,
            compute_budget,
            &sdk,
        );

        let message = SanitizedMessage::Legacy(LegacyMessage::new(
            Message::new_with_compiled_instructions(
                1,
                0,
                1,
                account_keys.clone(),
                blockhash,
                AccountKeys::new(&account_keys, None).compile_instructions(&[
                    Instruction::new_with_bincode(
                        system_program_id,
                        &SystemInstruction::Transfer { lamports: 50 },
                        account_metas.clone(),
                    ),
                ]),
            ),
            &Default::default(),
        ));
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        {
            assert!(result.is_ok());
            assert_eq!(result, Ok(()));
            let account1 = invoke_context
                .transaction_context
                .get_account_at_index(0)
                .unwrap()
                .borrow();
            assert_eq!(50, account1.lamports());
            let account2 = invoke_context
                .transaction_context
                .get_account_at_index(1)
                .unwrap()
                .borrow();
            assert_eq!(51, account2.lamports());
            let account3 = invoke_context
                .transaction_context
                .get_account_at_index(2)
                .unwrap()
                .borrow();
            assert_eq!(DUMMY_INHERITABLE_ACCOUNT_FIELDS.0, account3.lamports());
        }

        let message = SanitizedMessage::Legacy(LegacyMessage::new(
            Message::new_with_compiled_instructions(
                1,
                0,
                1,
                account_keys.clone(),
                blockhash,
                AccountKeys::new(&account_keys, None).compile_instructions(&[
                    Instruction::new_with_bincode(
                        system_program_id,
                        &SystemInstruction::Transfer { lamports: 10 },
                        account_metas.clone(),
                    ),
                ]),
            ),
            &Default::default(),
        ));
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        {
            assert!(result.is_ok());
            assert_eq!(result, Ok(()));
            let account1 = invoke_context
                .transaction_context
                .get_account_at_index(0)
                .unwrap()
                .borrow();
            assert_eq!(40, account1.lamports());
            let account2 = invoke_context
                .transaction_context
                .get_account_at_index(1)
                .unwrap()
                .borrow();
            assert_eq!(61, account2.lamports());
            let account3 = invoke_context
                .transaction_context
                .get_account_at_index(2)
                .unwrap()
                .borrow();
            assert_eq!(DUMMY_INHERITABLE_ACCOUNT_FIELDS.0, account3.lamports());
        }

        let message = SanitizedMessage::Legacy(LegacyMessage::new(
            Message::new_with_compiled_instructions(
                1,
                0,
                1,
                account_keys.clone(),
                blockhash,
                AccountKeys::new(&account_keys, None).compile_instructions(&[
                    Instruction::new_with_bincode(
                        system_program_id,
                        &SystemInstruction::Transfer { lamports: 101 },
                        account_metas.clone(),
                    ),
                ]),
            ),
            &Default::default(),
        ));
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        {
            assert!(result.is_err());
            assert_eq!(
                result.err().unwrap(),
                TransactionError::InstructionError(
                    0,
                    SystemError::ResultWithNegativeLamports.into()
                )
            );
            let account1 = invoke_context
                .transaction_context
                .get_account_at_index(0)
                .unwrap()
                .borrow();
            assert_eq!(40, account1.lamports());
            let account2 = invoke_context
                .transaction_context
                .get_account_at_index(1)
                .unwrap()
                .borrow();
            assert_eq!(61, account2.lamports());
            let account3 = invoke_context
                .transaction_context
                .get_account_at_index(2)
                .unwrap()
                .borrow();
            assert_eq!(DUMMY_INHERITABLE_ACCOUNT_FIELDS.0, account3.lamports());
        }
    }

    #[test]
    fn test_create_account_extend_data_section_change_owner() {
        let config = Config {
            enable_instruction_tracing: false,
            reject_broken_elfs: true,
            sanitize_user_provided_values: true,
            ..Default::default()
        };

        let blockhash = Hash::default();

        let sdk = journal_state();

        let native_loader_id = native_loader::id();
        let system_program_id = system_program::id();
        let new_owner = Pubkey::from([9; 32]);
        let from = Pubkey::new_unique();
        let to = Pubkey::new_unique();
        let account_from = AccountSharedData::new(100, 0, &system_program_id);
        let account_to = AccountSharedData::new(0, 0, &system_program_id);
        // let mut account_with_elf =
        //     load_program_account_from_elf(&bpf_loader_id,
        // "../examples/solana-program/assets/solana_program.so"); account_with_elf.
        // set_lamports(0);

        let accounts = vec![
            (from, account_from),
            (to, account_to),
            (
                system_program_id,
                create_loadable_account_for_test("system_program_id", &native_loader_id),
            ),
        ];
        let transaction_context = TransactionContext::new(accounts, Default::default(), 1, 3);
        let program_indices = vec![vec![2]];

        let function_registry =
            FunctionRegistry::<BuiltinFunction<InvokeContext<TestSdkType>>>::default();
        // register_builtins(&mut function_registry);
        let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));

        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            system_program_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                system_processor::Entrypoint::vm,
            )),
        );

        let compute_budget = ComputeBudget::default();
        let sysvar_cache = SysvarCache::default();
        let environment_config = EnvironmentConfig::new(
            blockhash,
            None,
            Arc::new(feature_set_default()),
            0,
            sysvar_cache,
        );
        let mut invoke_context = InvokeContext::new(
            transaction_context,
            programs_cache_for_tx_batch,
            environment_config,
            compute_budget,
            &sdk,
        );

        let number_of_accounts = invoke_context.transaction_context.get_number_of_accounts();
        let account_keys = (0..number_of_accounts)
            .map(|index| {
                *invoke_context
                    .transaction_context
                    .get_key_of_account_at_index(index)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let account_metas = vec![AccountMeta::new(from, true), AccountMeta::new(to, true)];
        let message = SanitizedMessage::Legacy(LegacyMessage::new(
            Message::new_with_compiled_instructions(
                2,
                0,
                0,
                account_keys.clone(),
                blockhash,
                AccountKeys::new(&account_keys, None).compile_instructions(&[
                    Instruction::new_with_bincode(
                        system_program_id,
                        &SystemInstruction::CreateAccount {
                            lamports: 30,
                            space: 0,
                            owner: system_program_id,
                        },
                        account_metas.clone(),
                    ),
                ]),
            ),
            &Default::default(),
        ));
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        assert!(result.is_ok());
        let account1 = invoke_context
            .transaction_context
            .get_account_at_index(0)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(70, account1.lamports());
        assert_eq!(0, account1.data().len());
        let account2 = invoke_context
            .transaction_context
            .get_account_at_index(1)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(30, account2.lamports());
        assert_eq!(0, account2.data().len());
        let account3 = invoke_context
            .transaction_context
            .get_account_at_index(2)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(DUMMY_INHERITABLE_ACCOUNT_FIELDS.0, account3.lamports());

        // allocate more account data for 2nd account

        let account_metas = vec![AccountMeta::new(to, true)];
        let message = Message::new_with_compiled_instructions(
            2,
            0,
            0,
            account_keys.clone(),
            blockhash,
            AccountKeys::new(&account_keys, None).compile_instructions(&[
                Instruction::new_with_bincode(
                    system_program_id,
                    &SystemInstruction::Allocate { space: 3 },
                    account_metas.clone(),
                ),
            ]),
        );
        let message = LegacyMessage::new(message, &Default::default());
        let message = SanitizedMessage::Legacy(message);
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        {
            assert!(result.is_ok());
            let account1 = invoke_context
                .transaction_context
                .get_account_at_index(0)
                .unwrap()
                .borrow();
            assert_eq!(70, account1.lamports());
            assert_eq!(0, account1.data().len());
            assert_eq!(&system_program_id, account1.owner());
            let account2 = invoke_context
                .transaction_context
                .get_account_at_index(1)
                .unwrap()
                .borrow();
            assert_eq!(30, account2.lamports());
            assert_eq!(3, account2.data().len() as u64);
            assert_eq!(&[0, 0, 0], account2.data());
            assert_eq!(&system_program_id, account2.owner());
            let account3 = invoke_context
                .transaction_context
                .get_account_at_index(2)
                .unwrap()
                .borrow();
            assert_eq!(DUMMY_INHERITABLE_ACCOUNT_FIELDS.0, account3.lamports());
        }

        // assign ownership of the 2nd account to some new owner

        let account_metas = vec![AccountMeta::new(to, true)];
        let message = Message::new_with_compiled_instructions(
            2,
            0,
            0,
            account_keys.clone(),
            blockhash,
            AccountKeys::new(&account_keys, None).compile_instructions(&[
                Instruction::new_with_bincode(
                    system_program_id,
                    &SystemInstruction::Assign { owner: new_owner },
                    account_metas.clone(),
                ),
            ]),
        );
        let message = LegacyMessage::new(message, &Default::default());
        let message = SanitizedMessage::Legacy(message);
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        {
            assert!(result.is_ok());
            let account1 = invoke_context
                .transaction_context
                .get_account_at_index(0)
                .unwrap()
                .borrow();
            assert_eq!(70, account1.lamports());
            assert_eq!(0, account1.data().len());
            let account2 = invoke_context
                .transaction_context
                .get_account_at_index(1)
                .unwrap()
                .borrow();
            assert_eq!(30, account2.lamports());
            assert_eq!(3, account2.data().len() as u64);
            assert_eq!(&[0, 0, 0], account2.data());
            let account3 = invoke_context
                .transaction_context
                .get_account_at_index(2)
                .unwrap()
                .borrow();
            assert_eq!(DUMMY_INHERITABLE_ACCOUNT_FIELDS.0, account3.lamports());
        }
    }

    #[test]
    fn test_create_account_extend_data_section_change_owner_many_in_one() {
        let config = Config {
            enable_instruction_tracing: false,
            reject_broken_elfs: true,
            sanitize_user_provided_values: true,
            ..Default::default()
        };

        let blockhash = Hash::default();

        let sdk = journal_state();

        let native_loader_id = native_loader::id();
        let system_program_id = system_program::id();

        let new_owner = Pubkey::from([9; 32]);
        let from = Pubkey::new_unique();
        let to = Pubkey::new_unique();
        let account_from = AccountSharedData::new(100, 0, &system_program_id);
        let account_to = AccountSharedData::new(0, 0, &system_program_id);
        // let mut account_with_elf =
        //     load_program_account_from_elf(&bpf_loader_id,
        // "../examples/solana-program/assets/solana_program.so"); account_with_elf.
        // set_lamports(0);

        let accounts = vec![
            (from, account_from),
            (to, account_to),
            (
                system_program_id,
                create_loadable_account_for_test("system_program_id", &native_loader_id),
            ),
        ];
        let transaction_context = TransactionContext::new(accounts, Default::default(), 1, 3);
        let mut program_indices = vec![];

        let function_registry =
            FunctionRegistry::<BuiltinFunction<InvokeContext<TestSdkType>>>::default();
        // register_builtins(&mut function_registry);
        let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));

        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            system_program_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                system_processor::Entrypoint::vm,
            )),
        );

        let compute_budget = ComputeBudget::default();
        let sysvar_cache = SysvarCache::default();
        let environment_config = EnvironmentConfig::new(
            blockhash,
            None,
            Arc::new(feature_set_default()),
            0,
            sysvar_cache,
        );
        let mut invoke_context = InvokeContext::new(
            transaction_context,
            programs_cache_for_tx_batch,
            environment_config,
            compute_budget,
            &sdk,
        );

        let number_of_accounts = invoke_context.transaction_context.get_number_of_accounts();
        let account_keys = (0..number_of_accounts)
            .map(|index| {
                *invoke_context
                    .transaction_context
                    .get_key_of_account_at_index(index)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        program_indices.push(vec![2]);
        program_indices.push(vec![2]);
        program_indices.push(vec![2]);
        let message = SanitizedMessage::Legacy(LegacyMessage::new(
            Message::new_with_compiled_instructions(
                2,
                0,
                0,
                account_keys.clone(),
                blockhash,
                AccountKeys::new(&account_keys, None).compile_instructions(&[
                    Instruction::new_with_bincode(
                        system_program_id,
                        &SystemInstruction::CreateAccount {
                            lamports: 30,
                            space: 0,
                            owner: system_program_id,
                        },
                        vec![AccountMeta::new(from, true), AccountMeta::new(to, true)],
                    ),
                    Instruction::new_with_bincode(
                        system_program_id,
                        &SystemInstruction::Allocate { space: 3 },
                        vec![AccountMeta::new(to, true)],
                    ),
                    Instruction::new_with_bincode(
                        system_program_id,
                        &SystemInstruction::Assign { owner: new_owner },
                        vec![AccountMeta::new(to, true)],
                    ),
                ]),
            ),
            &Default::default(),
        ));
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        {
            assert!(result.is_ok());
            let account1 = invoke_context
                .transaction_context
                .get_account_at_index(0)
                .unwrap()
                .borrow();
            assert_eq!(70, account1.lamports());
            assert_eq!(0, account1.data().len());
            let account2 = invoke_context
                .transaction_context
                .get_account_at_index(1)
                .unwrap()
                .borrow();
            assert_eq!(30, account2.lamports());
            assert_eq!(3, account2.data().len() as u64);
            assert_eq!(&[0, 0, 0], account2.data());
            let account3 = invoke_context
                .transaction_context
                .get_account_at_index(2)
                .unwrap()
                .borrow();
            assert_eq!(DUMMY_INHERITABLE_ACCOUNT_FIELDS.0, account3.lamports());
        }
    }

    #[test]
    fn test_create_account_deploy_elf_program_and_exec() {
        let config = Config {
            enable_instruction_tracing: false,
            reject_broken_elfs: true,
            sanitize_user_provided_values: true,
            ..Default::default()
        };

        let blockhash = Hash::default();

        let sdk = journal_state();

        let rent = Rent::free();

        let compute_budget = ComputeBudget::default();
        let mut sysvar_cache = SysvarCache::default();
        sysvar_cache.set_rent(rent.clone());
        sysvar_cache.set_clock(Clock::default());
        sysvar_cache.set_epoch_schedule(EpochSchedule::default());

        let system_program_id = system_program::id();
        let native_loader_id = native_loader::id();
        let bpf_loader_upgradeable_id = bpf_loader_upgradeable::id();
        let bpf_loader_id = bpf_loader::id();

        let pk_exec = Pubkey::from([8; 32]);
        let pk_9 = Pubkey::from([9; 32]);
        let pk_9_account = AccountSharedData::new(100, 0, &system_program_id);
        let (pk_program_data, _) =
            Pubkey::find_program_address(&[pk_exec.as_ref()], &bpf_loader_upgradeable_id);

        let new_accs = vec![
            (
                pk_exec.clone(),
                AccountSharedData::new(0, 0, &system_program_id),
            ),
            (pk_9.clone(), pk_9_account.clone()),
            (
                pk_program_data,
                AccountSharedData::new(0, 0, &system_program_id),
            ),
        ];

        let pk_payer = Pubkey::new_unique();
        let account_payer = AccountSharedData::new(100, 0, &system_program_id);
        let pk_buffer = Pubkey::new_unique();
        let account_buffer = AccountSharedData::new(0, 0, &system_program_id);
        let account_with_program = load_program_account_from_elf_file(
            &bpf_loader_upgradeable_id,
            "../../examples/svm/solana-program/assets/solana_program.so",
        );
        let program_len = account_with_program.data().len();
        let programdata_len = UpgradeableLoaderState::size_of_programdata(program_len);

        let program_signers = vec![&new_accs[0].0, &new_accs[1].0];

        let (accounts, _working_accounts_count) = compile_accounts_for_tx_ctx(
            vec![(pk_payer, account_payer), (pk_buffer, account_buffer)],
            vec![
                (
                    system_program_id,
                    create_loadable_account_for_test("system_program_id", &native_loader_id),
                ),
                (
                    bpf_loader_upgradeable_id,
                    create_loadable_account_for_test(
                        "bpf_loader_upgradeable_id",
                        &native_loader_id,
                    ),
                ),
            ],
        );
        let transaction_context = TransactionContext::new(accounts, rent.clone(), 10, 200);

        let mut function_registry =
            FunctionRegistry::<BuiltinFunction<InvokeContext<TestSdkType>>>::default();
        register_builtins(&mut function_registry);
        let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));
        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            system_program_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                system_processor::Entrypoint::vm,
            )),
        );
        programs_cache_for_tx_batch.replenish(
            bpf_loader_upgradeable_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                crate::loaders::bpf_loader_upgradeable::Entrypoint::vm,
            )),
        );
        let environment_config = EnvironmentConfig::new(
            blockhash,
            None,
            Arc::new(feature_set_default()),
            0,
            sysvar_cache.clone(),
        );
        let invoke_context = InvokeContext::new(
            transaction_context,
            programs_cache_for_tx_batch,
            environment_config,
            compute_budget,
            &sdk,
        );

        // INIT BUFFER

        let account_payer = invoke_context
            .transaction_context
            .get_account_at_index(0)
            .unwrap()
            .borrow()
            .clone();
        let account_buffer = invoke_context
            .transaction_context
            .get_account_at_index(1)
            .unwrap()
            .borrow()
            .clone();

        let (accounts, working_accounts_count) = compile_accounts_for_tx_ctx(
            vec![(pk_payer, account_payer), (pk_buffer, account_buffer)],
            vec![
                (
                    system_program_id,
                    create_loadable_account_for_test("system_program_id", &native_loader_id),
                ),
                (
                    bpf_loader_upgradeable_id,
                    create_loadable_account_for_test(
                        "bpf_loader_upgradeable_id",
                        &native_loader_id,
                    ),
                ),
                (pk_9, pk_9_account.clone()),
            ],
        );
        let transaction_context = TransactionContext::new(accounts, rent.clone(), 10, 200);

        let mut function_registry =
            FunctionRegistry::<BuiltinFunction<InvokeContext<TestSdkType>>>::default();
        register_builtins(&mut function_registry);
        let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));
        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            system_program_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                system_processor::Entrypoint::vm,
            )),
        );
        programs_cache_for_tx_batch.replenish(
            bpf_loader_upgradeable_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                crate::loaders::bpf_loader_upgradeable::Entrypoint::vm,
            )),
        );
        let environment_config = EnvironmentConfig::new(
            blockhash,
            None,
            Arc::new(feature_set_default()),
            0,
            sysvar_cache.clone(),
        );
        let mut invoke_context = InvokeContext::new(
            transaction_context,
            programs_cache_for_tx_batch,
            environment_config,
            compute_budget,
            &sdk,
        );

        let program_indices = vec![
            vec![working_accounts_count],
            vec![working_accounts_count + 1],
        ];

        let program_len = account_with_program.data().len();
        let buffer_space = UpgradeableLoaderState::size_of_buffer(program_len);

        let instructions =
            bpf_loader_upgradeable::create_buffer(&pk_payer, &pk_buffer, &pk_9, 0, program_len)
                .unwrap();
        let message = Message::new_with_blockhash(&instructions, Some(&pk_payer), &blockhash);
        let message = SanitizedMessage::Legacy(LegacyMessage::new(message, &Default::default()));
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        assert!(result.is_ok());
        assert_eq!(
            invoke_context.transaction_context.get_number_of_accounts(),
            5
        );

        let mut idx = 0;
        let pk = invoke_context
            .transaction_context
            .get_key_of_account_at_index(idx)
            .unwrap();
        assert_eq!(pk, &pk_payer);
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(100, account_data.lamports());
        assert_eq!(0, account_data.data().len());
        assert_eq!(false, account_data.executable());

        idx += 1;
        let pk = invoke_context
            .transaction_context
            .get_key_of_account_at_index(idx)
            .unwrap();
        assert_eq!(pk, &pk_buffer);
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(0, account_data.lamports());
        assert_eq!(buffer_space, account_data.data().len());
        assert_eq!(false, account_data.executable());

        idx += 1;
        let pk = invoke_context
            .transaction_context
            .get_key_of_account_at_index(idx)
            .unwrap();
        assert_eq!(pk, &system_program_id);
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(DUMMY_INHERITABLE_ACCOUNT_FIELDS.0, account_data.lamports());
        assert_eq!(17, account_data.data().len());
        assert_eq!(true, account_data.executable());

        idx += 1;
        let pk = invoke_context
            .transaction_context
            .get_key_of_account_at_index(idx)
            .unwrap();
        assert_eq!(pk, &bpf_loader_upgradeable_id);
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(DUMMY_INHERITABLE_ACCOUNT_FIELDS.0, account_data.lamports());
        assert_eq!(25, account_data.data().len());
        assert_eq!(true, account_data.executable());

        idx += 1;
        let pk = invoke_context
            .transaction_context
            .get_key_of_account_at_index(idx)
            .unwrap();
        assert_eq!(pk, &pk_9);
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(100, account_data.lamports());
        assert_eq!(0, account_data.data().len());
        assert_eq!(false, account_data.executable());

        // FILL BUFFER

        let account_payer = invoke_context
            .transaction_context
            .get_account_at_index(0)
            .unwrap()
            .borrow();
        let account_buffer = invoke_context
            .transaction_context
            .get_account_at_index(1)
            .unwrap()
            .borrow();
        let pk_9_account = invoke_context
            .transaction_context
            .get_account_at_index(4)
            .unwrap()
            .borrow();
        let (accounts, working_accounts_count) = compile_accounts_for_tx_ctx(
            vec![
                (pk_payer, account_payer.clone()),
                (pk_9, pk_9_account.clone()),
                (pk_buffer, account_buffer.clone()),
            ],
            vec![(
                bpf_loader_upgradeable_id,
                create_loadable_account_for_test("bpf_loader_upgradeable_id", &native_loader_id),
            )],
        );
        let transaction_context = TransactionContext::new(accounts, rent.clone(), 10, 200);
        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            system_program::id(),
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                system_processor::Entrypoint::vm,
            )),
        );
        programs_cache_for_tx_batch.replenish(
            bpf_loader_upgradeable::id(),
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                crate::loaders::bpf_loader_upgradeable::Entrypoint::vm,
            )),
        );
        let environment_config = EnvironmentConfig::new(
            blockhash,
            None,
            Arc::new(feature_set_default()),
            0,
            sysvar_cache.clone(),
        );
        let mut invoke_context = InvokeContext::new(
            transaction_context,
            programs_cache_for_tx_batch,
            environment_config,
            compute_budget,
            &sdk,
        );

        let program_indices = vec![vec![working_accounts_count]];
        let buffer_space =
            UpgradeableLoaderState::size_of_buffer(account_with_program.data().len());
        let create_msg = |offset: u32, bytes: Vec<u8>| {
            let instruction = bpf_loader_upgradeable::write(&pk_buffer, &pk_9, offset, bytes);
            let instructions = vec![instruction];
            // set_compute_budget_ixs_if_needed(&mut instructions, compute_unit_price);
            Message::new_with_blockhash(&instructions, Some(&pk_payer), &blockhash)
        };
        let mut write_messages = vec![];
        let chunk_size = calculate_max_chunk_size(&create_msg);
        for (chunk, i) in account_with_program.data().chunks(chunk_size).zip(0..) {
            let offset = i * chunk_size;
            let msg = create_msg(offset as u32, chunk.to_vec());
            write_messages.push(msg);
        }
        for (_mn, m) in write_messages.iter().enumerate() {
            let message =
                SanitizedMessage::Legacy(LegacyMessage::new(m.clone(), &Default::default()));
            let result =
                MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
            assert!(result.is_ok());
        }
        assert_eq!(
            invoke_context.transaction_context.get_number_of_accounts(),
            4
        );

        let mut idx = 0;
        let account = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(100, account.lamports());
        assert_eq!(0, account.data().len());
        assert!(!account.executable());

        idx += 1;
        let account = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(100, account.lamports());
        assert_eq!(0, account.data().len());
        assert!(!account.executable());

        idx += 1;
        let account = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(0, account.lamports());
        assert_eq!(buffer_space, account.data().len());
        assert!(!account.executable());

        idx += 1;
        let account = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(1, account.lamports());
        assert_eq!("bpf_loader_upgradeable_id".len(), account.data().len());
        assert!(account.executable());

        // DEPLOY

        let account_payer = invoke_context
            .transaction_context
            .get_account_at_index(0)
            .unwrap()
            .borrow();
        let account_9 = invoke_context
            .transaction_context
            .get_account_at_index(1)
            .unwrap()
            .borrow()
            .clone();
        let account_buffer = invoke_context
            .transaction_context
            .get_account_at_index(2)
            .unwrap()
            .borrow();
        let (accounts, working_accounts_count) = compile_accounts_for_tx_ctx(
            vec![
                (
                    // 0 // authority_address
                    pk_payer,
                    account_payer.clone(),
                ),
                new_accs[0].clone(), // 1 // acc 8... // becomes executable
                (
                    // 2 // acc 9...
                    pk_9, account_9,
                ),
                (
                    // 3
                    pk_buffer,
                    account_buffer.clone(),
                ),
                new_accs[2].clone(), // 4 // program data account
            ],
            vec![
                (
                    system_program_id,
                    create_loadable_account_for_test("system_program_id", &native_loader_id),
                ),
                (
                    bpf_loader_upgradeable_id,
                    create_loadable_account_for_test(
                        "bpf_loader_upgradeable_id",
                        &native_loader_id,
                    ),
                ),
                (
                    sysvar::clock::id(),
                    create_loadable_account_for_test("sysvar_clock_id", &sysvar::id()),
                ),
                (
                    sysvar::rent::id(),
                    create_loadable_account_for_test("sysvar_rent_id", &sysvar::id()),
                ),
            ],
        );
        let transaction_context = TransactionContext::new(accounts, rent.clone(), 10, 200);
        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            system_program_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                system_processor::Entrypoint::vm,
            )),
        );
        programs_cache_for_tx_batch.replenish(
            bpf_loader_upgradeable_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                crate::loaders::bpf_loader_upgradeable::Entrypoint::vm,
            )),
        );
        let environment_config = EnvironmentConfig::new(
            blockhash,
            None,
            Arc::new(feature_set_default()),
            0,
            sysvar_cache.clone(),
        );
        let mut invoke_context = InvokeContext::new(
            transaction_context,
            programs_cache_for_tx_batch,
            environment_config,
            compute_budget,
            &sdk,
        );

        let instructions = bpf_loader_upgradeable::deploy_with_max_program_len(
            &pk_payer,
            program_signers[0],
            &pk_buffer,
            program_signers[1],
            10,
            account_with_program.data().len(),
        )
        .unwrap();
        let message = Message::new_with_blockhash(&instructions, Some(&pk_payer), &blockhash);
        let message = LegacyMessage::new(message, &Default::default());
        let message = SanitizedMessage::Legacy(message);
        let program_indices = vec![
            vec![working_accounts_count],
            vec![working_accounts_count + 1],
        ];
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        assert!(result.is_ok());

        let mut idx = 0;
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(89, account_data.lamports());
        assert_eq!(0, account_data.data().len());
        assert_eq!(true, account.executable());

        idx += 1;
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(10, account_data.lamports());
        assert_eq!(36, account_data.data().len());
        assert_eq!(true, account_data.executable());

        idx += 1;
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(100, account_data.lamports());
        assert_eq!(0, account_data.data().len());
        assert_eq!(false, account_data.executable());

        idx += 1;
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(0, account_data.lamports());
        assert_eq!(37, account_data.data().len());
        assert_eq!(false, account_data.executable());

        idx += 1;
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(1, account_data.lamports());
        assert_eq!(programdata_len, account_data.data().len());
        assert_eq!(&bpf_loader_upgradeable_id, account_data.owner());
        assert_eq!(false, account_data.executable());

        idx += 1;
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(1, account_data.lamports());
        assert_eq!(17, account_data.data().len());
        assert_eq!(true, account_data.executable());

        idx += 1;
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(1, account_data.lamports());
        assert_eq!(25, account_data.data().len());
        assert_eq!(true, account_data.executable());

        idx += 1;
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(1, account_data.lamports());
        assert_eq!("sysvar_clock_id".len(), account_data.data().len());
        assert_eq!(true, account_data.executable());

        idx += 1;
        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(idx)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(1, account_data.lamports());
        assert_eq!("sysvar_rent_id".len(), account_data.data().len());
        assert_eq!(true, account_data.executable());

        assert_eq!(
            invoke_context.transaction_context.get_number_of_accounts(),
            idx + 1
        );

        // EXEC

        let account_exec_pk = invoke_context
            .transaction_context
            .get_key_of_account_at_index(1)
            .unwrap()
            .clone();
        let account_exec = invoke_context
            .transaction_context
            .get_account_at_index(1)
            .unwrap()
            .borrow()
            .clone();
        // account_exec.set_owner(bpf_loader_id.clone());
        // assert_eq!(&bpf_loader_id, account_exec.owner());
        assert_eq!(&bpf_loader_upgradeable_id, account_exec.owner());
        let account_program_data_pk = invoke_context
            .transaction_context
            .get_key_of_account_at_index(4)
            .unwrap()
            .clone();
        let account_program_data = invoke_context
            .transaction_context
            .get_account_at_index(4)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(&bpf_loader_upgradeable_id, account_program_data.owner());
        let (accounts, working_accounts_count) = compile_accounts_for_tx_ctx(
            vec![
                (
                    account_program_data_pk.clone(),
                    account_program_data.clone(),
                ), // acc data
            ],
            vec![
                (
                    bpf_loader_upgradeable_id,
                    create_loadable_account_for_test(
                        "bpf_loader_upgradeable_id",
                        &native_loader_id,
                    ),
                ),
                (account_exec_pk.clone(), account_exec.clone()), // acc exec
            ],
        );
        let accounts_count = accounts.len();
        let transaction_context = TransactionContext::new(accounts, rent.clone(), 10, 200);
        let mut programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_cache_for_tx_batch.replenish(
            system_program_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                system_processor::Entrypoint::vm,
            )),
        );
        programs_cache_for_tx_batch.replenish(
            bpf_loader_upgradeable_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                crate::loaders::bpf_loader_upgradeable::Entrypoint::vm,
            )),
        );
        programs_cache_for_tx_batch.replenish(
            bpf_loader_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0,
                0,
                crate::loaders::bpf_loader_upgradeable::Entrypoint::vm,
            )),
        );

        let environment_config = EnvironmentConfig::new(
            blockhash,
            None,
            Arc::new(feature_set_default()),
            0,
            sysvar_cache,
        );
        let mut invoke_context = InvokeContext::new(
            transaction_context,
            programs_cache_for_tx_batch,
            environment_config,
            compute_budget,
            &sdk,
        );
        let loaded_program = invoke_context.load_program(&account_exec_pk, false);
        if let Some(v) = loaded_program {
            invoke_context
                .program_cache_for_tx_batch
                .replenish(account_exec_pk.clone(), v);
        };

        invoke_context.inc_slots(1);

        let instructions = vec![Instruction::new_with_bincode(
            account_exec_pk.clone(),
            &[0u8; 0],
            vec![],
        )];
        let message = Message::new_with_blockhash(&instructions, Some(&pk_exec), &blockhash);
        let message = LegacyMessage::new(message, &Default::default());
        let message = SanitizedMessage::Legacy(message);
        let program_indices = vec![vec![working_accounts_count, working_accounts_count + 1]];
        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        assert!(result.is_ok());
        let number_of_accounts = invoke_context.transaction_context.get_number_of_accounts();
        assert_eq!(number_of_accounts, accounts_count as IndexOfAccount);

        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(0)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(1, account_data.lamports());
        assert_eq!(programdata_len, account_data.data().len());

        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(1)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(1, account_data.lamports());
        assert_eq!(25, account_data.data().len());

        let account_data = invoke_context
            .transaction_context
            .get_account_at_index(2)
            .unwrap()
            .borrow()
            .clone();
        assert_eq!(10, account_data.lamports());
        assert_eq!(36, account_data.data().len());
    }
}
