#[cfg(test)]
mod tests {
    use crate::{
        account::{Account, AccountSharedData, ReadableAccount},
        context::InvokeContext,
        hash::Hash,
        helpers::create_account_shared_data_for_test,
        native_loader,
        pubkey::Pubkey,
        rent::Rent,
        solana_program::{instruction::AccountMeta, sysvar},
        system_instruction::{SystemError, SystemInstruction, MAX_PERMITTED_DATA_LENGTH},
        system_processor::{get_system_account_kind, Address, Entrypoint, SystemAccountKind},
        system_program,
        test_helpers::{mock_process_instruction, new_test_sdk, prepare_vars_for_tests},
        with_mock_invoke_context,
    };
    use fluentbase_sdk::SharedAPI;
    use fluentbase_sdk_testing::HostTestingContext;
    use solana_bincode::serialize;
    use solana_instruction::error::InstructionError;

    fn process_instruction<SDK: SharedAPI>(
        sdk: &SDK,
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

    fn create_default_account() -> AccountSharedData {
        AccountSharedData::new(0, 0, &Pubkey::new_unique())
    }
    fn create_default_rent_account() -> AccountSharedData {
        create_account_shared_data_for_test(&Rent::free())
    }

    #[test]
    fn test_create_account() {
        let sdk = new_test_sdk();

        let new_owner = Pubkey::from([9; 32]);
        let from = Pubkey::new_unique();
        let to = Pubkey::new_unique();
        let from_account = AccountSharedData::new(100, 0, &system_program::id());
        let to_account = AccountSharedData::new(0, 0, &Pubkey::default());

        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 50,
                space: 2,
                owner: new_owner,
            })
            .unwrap(),
            vec![(from, from_account), (to, to_account)],
            vec![
                AccountMeta {
                    pubkey: from,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: to,
                    is_signer: true,
                    is_writable: true,
                },
            ],
            Ok(()),
        );
        assert_eq!(accounts[0].lamports(), 50);
        assert_eq!(accounts[1].lamports(), 50);
        assert_eq!(accounts[1].owner(), &new_owner);
        assert_eq!(accounts[1].data(), &[0, 0]);
    }

    #[test]
    fn test_create_account_with_seed() {
        let sdk = new_test_sdk();

        let new_owner = Pubkey::from([9; 32]);
        let from = Pubkey::new_unique();
        let seed = "shiny pepper";
        let to = Pubkey::create_with_seed(&from, seed, &new_owner).unwrap();
        let from_account = AccountSharedData::new(100, 0, &system_program::id());
        let to_account = AccountSharedData::new(0, 0, &Pubkey::default());

        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccountWithSeed {
                base: from,
                seed: seed.to_string(),
                lamports: 50,
                space: 2,
                owner: new_owner,
            })
            .unwrap(),
            vec![(from, from_account), (to, to_account)],
            vec![
                AccountMeta {
                    pubkey: from,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: to,
                    is_signer: true,
                    is_writable: true,
                },
            ],
            Ok(()),
        );
        assert_eq!(accounts[0].lamports(), 50);
        assert_eq!(accounts[1].lamports(), 50);
        assert_eq!(accounts[1].owner(), &new_owner);
        assert_eq!(accounts[1].data(), &[0, 0]);
    }

    #[test]
    fn test_create_account_with_seed_separate_base_account() {
        let sdk = new_test_sdk();

        let new_owner = Pubkey::from([9; 32]);
        let from = Pubkey::new_unique();
        let base = Pubkey::new_unique();
        let seed = "shiny pepper";
        let to = Pubkey::create_with_seed(&base, seed, &new_owner).unwrap();
        let from_account = AccountSharedData::new(100, 0, &system_program::id());
        let to_account = AccountSharedData::new(0, 0, &Pubkey::default());
        let base_account = AccountSharedData::new(0, 0, &Pubkey::default());

        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccountWithSeed {
                base,
                seed: seed.to_string(),
                lamports: 50,
                space: 2,
                owner: new_owner,
            })
            .unwrap(),
            vec![(from, from_account), (to, to_account), (base, base_account)],
            vec![
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
                AccountMeta {
                    pubkey: base,
                    is_signer: true,
                    is_writable: false,
                },
            ],
            Ok(()),
        );
        assert_eq!(accounts[0].lamports(), 50);
        assert_eq!(accounts[1].lamports(), 50);
        assert_eq!(accounts[1].owner(), &new_owner);
        assert_eq!(accounts[1].data(), &[0, 0]);
    }

    #[test]
    fn test_address_create_with_seed_mismatch() {
        let sdk = new_test_sdk();

        let (_config, loader) = prepare_vars_for_tests();
        with_mock_invoke_context!(
            invoke_context,
            transaction_context,
            &sdk,
            loader,
            Vec::new()
        );
        let from = Pubkey::new_unique();
        let seed = "dull boy";
        let to = Pubkey::new_unique();
        let owner = Pubkey::new_unique();

        assert_eq!(
            Address::create(&to, Some((&from, seed, &owner)), &invoke_context),
            Err(SystemError::AddressWithSeedMismatch.into())
        );
    }

    #[test]
    fn test_create_account_with_seed_missing_sig() {
        let sdk = new_test_sdk();

        let new_owner = Pubkey::from([9; 32]);
        let from = Pubkey::new_unique();
        let seed = "dull boy";
        let to = Pubkey::create_with_seed(&from, seed, &new_owner).unwrap();
        let from_account = AccountSharedData::new(100, 0, &system_program::id());
        let to_account = AccountSharedData::new(0, 0, &Pubkey::default());

        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 50,
                space: 2,
                owner: new_owner,
            })
            .unwrap(),
            vec![(from, from_account), (to, to_account)],
            vec![
                AccountMeta {
                    pubkey: from,
                    is_signer: true,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: to,
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Err(InstructionError::MissingRequiredSignature),
        );
        assert_eq!(accounts[0].lamports(), 100);
        assert_eq!(accounts[1], AccountSharedData::default());
    }

    #[test]
    fn test_create_with_zero_lamports() {
        let sdk = new_test_sdk();

        // create account with zero lamports transferred
        let new_owner = Pubkey::from([9; 32]);
        let from = Pubkey::new_unique();
        let from_account = AccountSharedData::new(100, 0, &Pubkey::new_unique()); // not from system account
        let to = Pubkey::new_unique();
        let to_account = AccountSharedData::new(0, 0, &Pubkey::default());

        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 0,
                space: 2,
                owner: new_owner,
            })
            .unwrap(),
            vec![(from, from_account), (to, to_account)],
            vec![
                AccountMeta {
                    pubkey: from,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: to,
                    is_signer: true,
                    is_writable: true,
                },
            ],
            Ok(()),
        );
        assert_eq!(accounts[0].lamports(), 100);
        assert_eq!(accounts[1].lamports(), 0);
        assert_eq!(*accounts[1].owner(), new_owner);
        assert_eq!(accounts[1].data(), &[0, 0]);
    }

    #[test]
    fn test_create_negative_lamports() {
        let sdk = new_test_sdk();

        // Attempt to create account with more lamports than from_account has
        let new_owner = Pubkey::from([9; 32]);
        let from = Pubkey::new_unique();
        let from_account = AccountSharedData::new(100, 0, &Pubkey::new_unique());
        let to = Pubkey::new_unique();
        let to_account = AccountSharedData::new(0, 0, &Pubkey::default());

        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 150,
                space: 2,
                owner: new_owner,
            })
            .unwrap(),
            vec![(from, from_account), (to, to_account)],
            vec![
                AccountMeta {
                    pubkey: from,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: to,
                    is_signer: true,
                    is_writable: true,
                },
            ],
            Err(SystemError::ResultWithNegativeLamports.into()),
        );
    }

    #[test]
    fn test_request_more_than_allowed_data_length() {
        let sdk = new_test_sdk();

        let from = Pubkey::new_unique();
        let from_account = AccountSharedData::new(100, 0, &system_program::id());
        let to = Pubkey::new_unique();
        let to_account = AccountSharedData::new(0, 0, &Pubkey::default());
        let instruction_accounts = vec![
            AccountMeta {
                pubkey: from,
                is_signer: true,
                is_writable: true,
            },
            AccountMeta {
                pubkey: to,
                is_signer: true,
                is_writable: true,
            },
        ];

        // Trying to request more data length than permitted will result in failure
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 50,
                space: MAX_PERMITTED_DATA_LENGTH + 1,
                owner: system_program::id(),
            })
            .unwrap(),
            vec![(from, from_account.clone()), (to, to_account.clone())],
            instruction_accounts.clone(),
            Err(SystemError::InvalidAccountDataLength.into()),
        );

        // Trying to request equal or less data length than permitted will be successful
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 50,
                space: MAX_PERMITTED_DATA_LENGTH,
                owner: system_program::id(),
            })
            .unwrap(),
            vec![(from, from_account), (to, to_account)],
            instruction_accounts,
            Ok(()),
        );
        assert_eq!(accounts[1].lamports(), 50);
        assert_eq!(accounts[1].data().len() as u64, MAX_PERMITTED_DATA_LENGTH);
    }

    #[test]
    fn test_create_already_in_use() {
        let sdk = new_test_sdk();

        let new_owner = Pubkey::from([9; 32]);
        let from = Pubkey::new_unique();
        let from_account = AccountSharedData::new(100, 0, &system_program::id());
        let owned_key = Pubkey::new_unique();

        // Attempt to create system account in account already owned by another program
        let original_program_owner = Pubkey::from([5; 32]);
        let owned_account = AccountSharedData::new(0, 0, &original_program_owner);
        let unchanged_account = owned_account.clone();
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 50,
                space: 2,
                owner: new_owner,
            })
            .unwrap(),
            vec![(from, from_account.clone()), (owned_key, owned_account)],
            vec![
                AccountMeta {
                    pubkey: from,
                    is_signer: true,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: owned_key,
                    is_signer: true,
                    is_writable: false,
                },
            ],
            Err(SystemError::AccountAlreadyInUse.into()),
        );
        assert_eq!(accounts[0].lamports(), 100);
        assert_eq!(accounts[1], unchanged_account);

        // Attempt to create system account in account that already has data
        let owned_account = AccountSharedData::new(0, 1, &Pubkey::default());
        let unchanged_account = owned_account.clone();
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 50,
                space: 2,
                owner: new_owner,
            })
            .unwrap(),
            vec![(from, from_account.clone()), (owned_key, owned_account)],
            vec![
                AccountMeta {
                    pubkey: from,
                    is_signer: true,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: owned_key,
                    is_signer: true,
                    is_writable: false,
                },
            ],
            Err(SystemError::AccountAlreadyInUse.into()),
        );
        assert_eq!(accounts[0].lamports(), 100);
        assert_eq!(accounts[1], unchanged_account);

        // Attempt to create an account that already has lamports
        let owned_account = AccountSharedData::new(1, 0, &Pubkey::default());
        let unchanged_account = owned_account.clone();
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 50,
                space: 2,
                owner: new_owner,
            })
            .unwrap(),
            vec![(from, from_account), (owned_key, owned_account)],
            vec![
                AccountMeta {
                    pubkey: from,
                    is_signer: true,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: owned_key,
                    is_signer: true,
                    is_writable: false,
                },
            ],
            Err(SystemError::AccountAlreadyInUse.into()),
        );
        assert_eq!(accounts[0].lamports(), 100);
        assert_eq!(accounts[1], unchanged_account);
    }

    #[test]
    fn test_create_unsigned() {
        let sdk = new_test_sdk();

        // Attempt to create an account without signing the transfer
        let new_owner = Pubkey::from([9; 32]);
        let from = Pubkey::new_unique();
        let from_account = AccountSharedData::new(100, 0, &system_program::id());
        let owned_key = Pubkey::new_unique();
        let owned_account = AccountSharedData::new(0, 0, &Pubkey::default());

        // Haven't signed from account
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 50,
                space: 2,
                owner: new_owner,
            })
            .unwrap(),
            vec![
                (from, from_account.clone()),
                (owned_key, owned_account.clone()),
            ],
            vec![
                AccountMeta {
                    pubkey: from,
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: owned_key,
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Err(InstructionError::MissingRequiredSignature),
        );

        // Haven't signed to account
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 50,
                space: 2,
                owner: new_owner,
            })
            .unwrap(),
            vec![(from, from_account.clone()), (owned_key, owned_account)],
            vec![
                AccountMeta {
                    pubkey: from,
                    is_signer: true,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: owned_key,
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Err(InstructionError::MissingRequiredSignature),
        );

        // Don't support unsigned creation with zero lamports (ephemeral account)
        let owned_account = AccountSharedData::new(0, 0, &Pubkey::default());
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 50,
                space: 2,
                owner: new_owner,
            })
            .unwrap(),
            vec![(from, from_account), (owned_key, owned_account)],
            vec![
                AccountMeta {
                    pubkey: from,
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: owned_key,
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Err(InstructionError::MissingRequiredSignature),
        );
    }

    #[test]
    fn test_create_sysvar_invalid_id_with_feature() {
        let sdk = new_test_sdk();

        // Attempt to create system account in account already owned by another program
        let from = Pubkey::new_unique();
        let from_account = AccountSharedData::new(100, 0, &system_program::id());
        let to = Pubkey::new_unique();
        let to_account = AccountSharedData::new(0, 0, &system_program::id());

        // fail to create a sysvar::id() owned account
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 50,
                space: 2,
                owner: sysvar::id(),
            })
            .unwrap(),
            vec![(from, from_account), (to, to_account)],
            vec![
                AccountMeta {
                    pubkey: from,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: to,
                    is_signer: true,
                    is_writable: true,
                },
            ],
            Ok(()),
        );
    }

    #[test]
    fn test_create_data_populated() {
        let sdk = new_test_sdk();

        // Attempt to create system account in account with populated data
        let new_owner = Pubkey::from([9; 32]);
        let from = Pubkey::new_unique();
        let from_account = AccountSharedData::new(100, 0, &system_program::id());
        let populated_key = Pubkey::new_unique();
        let populated_account = AccountSharedData::from(Account {
            data: vec![0, 1, 2, 3],
            ..Account::default()
        });

        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 50,
                space: 2,
                owner: new_owner,
            })
            .unwrap(),
            vec![(from, from_account), (populated_key, populated_account)],
            vec![
                AccountMeta {
                    pubkey: from,
                    is_signer: true,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: populated_key,
                    is_signer: true,
                    is_writable: false,
                },
            ],
            Err(SystemError::AccountAlreadyInUse.into()),
        );
    }

    #[test]
    fn test_assign() {
        let sdk = new_test_sdk();

        let new_owner = Pubkey::from([9; 32]);
        let pubkey = Pubkey::new_unique();
        let account = AccountSharedData::new(100, 0, &system_program::id());

        // owner does not change, no signature needed
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::Assign {
                owner: system_program::id(),
            })
            .unwrap(),
            vec![(pubkey, account.clone())],
            vec![AccountMeta {
                pubkey,
                is_signer: false,
                is_writable: true,
            }],
            Ok(()),
        );

        // owner does change, signature needed
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::Assign { owner: new_owner }).unwrap(),
            vec![(pubkey, account.clone())],
            vec![AccountMeta {
                pubkey,
                is_signer: false,
                is_writable: true,
            }],
            Err(InstructionError::MissingRequiredSignature),
        );

        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::Assign { owner: new_owner }).unwrap(),
            vec![(pubkey, account.clone())],
            vec![AccountMeta {
                pubkey,
                is_signer: true,
                is_writable: true,
            }],
            Ok(()),
        );

        // assign to sysvar instead of system_program
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::Assign {
                owner: sysvar::id(),
            })
            .unwrap(),
            vec![(pubkey, account)],
            vec![AccountMeta {
                pubkey,
                is_signer: true,
                is_writable: true,
            }],
            Ok(()),
        );
    }

    #[test]
    fn test_process_bogus_instruction() {
        let sdk = new_test_sdk();

        // Attempt to assign with no accounts
        let instruction = SystemInstruction::Assign {
            owner: Pubkey::new_unique(),
        };
        let data = serialize(&instruction).unwrap();
        process_instruction(
            &sdk,
            &data,
            Vec::new(),
            Vec::new(),
            Err(InstructionError::NotEnoughAccountKeys),
        );

        // Attempt to transfer with no destination
        let from = Pubkey::new_unique();
        let from_account = AccountSharedData::new(100, 0, &system_program::id());
        let instruction = SystemInstruction::Transfer { lamports: 0 };
        let data = serialize(&instruction).unwrap();
        process_instruction(
            &sdk,
            &data,
            vec![(from, from_account)],
            vec![AccountMeta {
                pubkey: from,
                is_signer: true,
                is_writable: false,
            }],
            Err(InstructionError::NotEnoughAccountKeys),
        );
    }

    #[test]
    fn test_transfer_lamports() {
        let sdk = new_test_sdk();

        let from = Pubkey::new_unique();
        let from_account = AccountSharedData::new(100, 0, &system_program::id());
        let to = Pubkey::from([3; 32]);
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
            &sdk,
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
            &sdk,
            &serialize(&SystemInstruction::Transfer { lamports: 101 }).unwrap(),
            transaction_accounts.clone(),
            instruction_accounts.clone(),
            Err(SystemError::ResultWithNegativeLamports.into()),
        );
        assert_eq!(accounts[0].lamports(), 100);
        assert_eq!(accounts[1].lamports(), 1);

        // test signed transfer of zero
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::Transfer { lamports: 0 }).unwrap(),
            transaction_accounts.clone(),
            instruction_accounts,
            Ok(()),
        );
        assert_eq!(accounts[0].lamports(), 100);
        assert_eq!(accounts[1].lamports(), 1);

        // test unsigned transfer of zero
        let accounts = process_instruction(
            &sdk,
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

    #[test]
    fn test_transfer_with_seed() {
        let sdk = new_test_sdk();

        let base = Pubkey::new_unique();
        let base_account = AccountSharedData::new(100, 0, &Pubkey::from([2; 32])); // account owner should not matter
        let from_seed = "42".to_string();
        let from_owner = system_program::id();
        let from = Pubkey::create_with_seed(&base, from_seed.as_str(), &from_owner).unwrap();
        let from_account = AccountSharedData::new(100, 0, &system_program::id());
        let to = Pubkey::from([3; 32]);
        let to_account = AccountSharedData::new(1, 0, &to); // account owner should not matter
        let transaction_accounts =
            vec![(from, from_account), (base, base_account), (to, to_account)];
        let instruction_accounts = vec![
            AccountMeta {
                pubkey: from,
                is_signer: true,
                is_writable: true,
            },
            AccountMeta {
                pubkey: base,
                is_signer: true,
                is_writable: false,
            },
            AccountMeta {
                pubkey: to,
                is_signer: false,
                is_writable: true,
            },
        ];

        // Success case
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::TransferWithSeed {
                lamports: 50,
                from_seed: from_seed.clone(),
                from_owner,
            })
            .unwrap(),
            transaction_accounts.clone(),
            instruction_accounts.clone(),
            Ok(()),
        );
        assert_eq!(accounts[0].lamports(), 50);
        assert_eq!(accounts[2].lamports(), 51);

        // Attempt to move more lamports than from_account has
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::TransferWithSeed {
                lamports: 101,
                from_seed: from_seed.clone(),
                from_owner,
            })
            .unwrap(),
            transaction_accounts.clone(),
            instruction_accounts.clone(),
            Err(SystemError::ResultWithNegativeLamports.into()),
        );
        assert_eq!(accounts[0].lamports(), 100);
        assert_eq!(accounts[2].lamports(), 1);

        // Test unsigned transfer of zero
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::TransferWithSeed {
                lamports: 0,
                from_seed,
                from_owner,
            })
            .unwrap(),
            transaction_accounts,
            instruction_accounts,
            Ok(()),
        );
        assert_eq!(accounts[0].lamports(), 100);
        assert_eq!(accounts[2].lamports(), 1);
    }

    #[test]
    fn test_get_system_account_kind_system_ok() {
        let system_account = AccountSharedData::default();
        assert_eq!(
            get_system_account_kind(&system_account),
            Some(SystemAccountKind::System)
        );
    }

    #[test]
    fn test_get_system_account_kind_system_owner_nonzero_nonnonce_data_fail() {
        let other_data_account =
            AccountSharedData::new_data(42, b"other", &Pubkey::default()).unwrap();
        assert_eq!(get_system_account_kind(&other_data_account), None);
    }

    #[test]
    fn test_assign_native_loader_and_transfer() {
        let sdk = new_test_sdk();

        for size in [0, 10] {
            let pubkey = Pubkey::new_unique();
            let account = AccountSharedData::new(100, size, &system_program::id());
            let accounts = process_instruction(
                &sdk,
                &serialize(&SystemInstruction::Assign {
                    owner: native_loader::id(),
                })
                .unwrap(),
                vec![(pubkey, account.clone())],
                vec![AccountMeta {
                    pubkey,
                    is_signer: true,
                    is_writable: true,
                }],
                Ok(()),
            );
            assert_eq!(accounts[0].owner(), &native_loader::id());
            assert_eq!(accounts[0].lamports(), 100);

            let pubkey2 = Pubkey::new_unique();
            let accounts = process_instruction(
                &sdk,
                &serialize(&SystemInstruction::Transfer { lamports: 50 }).unwrap(),
                vec![
                    (
                        pubkey2,
                        AccountSharedData::new(100, 0, &system_program::id()),
                    ),
                    (pubkey, accounts[0].clone()),
                ],
                vec![
                    AccountMeta {
                        pubkey: pubkey2,
                        is_signer: true,
                        is_writable: true,
                    },
                    AccountMeta {
                        pubkey,
                        is_signer: false,
                        is_writable: true,
                    },
                ],
                Ok(()),
            );
            assert_eq!(accounts[1].owner(), &native_loader::id());
            assert_eq!(accounts[1].lamports(), 150);
        }
    }
}
