#[cfg(test)]
mod tests {
    use crate::{
        account::{Account, AccountSharedData, ReadableAccount},
        context::InvokeContext,
        hash::{hash, Hash},
        helpers::create_account_shared_data_for_test,
        native_loader,
        nonce_account,
        pubkey::Pubkey,
        recent_blockhashes_account,
        rent::Rent,
        solana_program::{
            instruction::{AccountMeta, Instruction},
            nonce,
            nonce::state::{
                Data as NonceData,
                DurableNonce,
                State as NonceState,
                Versions as NonceVersions,
            },
            sysvar,
            sysvar::{recent_blockhashes, recent_blockhashes::IterItem},
        },
        system_instruction,
        system_instruction::{SystemError, SystemInstruction, MAX_PERMITTED_DATA_LENGTH},
        system_processor::{get_system_account_kind, Address, Entrypoint, SystemAccountKind},
        system_program,
        test_helpers::{mock_process_instruction, new_test_sdk, prepare_vars_for_tests},
        with_mock_invoke_context,
    };
    use fluentbase_sdk::SharedAPI;
    use fluentbase_sdk_testing::HostTestingContext;
    use solana_bincode::serialize;
    use solana_fee_calculator::FeeCalculator;
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
    fn create_default_recent_blockhashes_account() -> AccountSharedData {
        #[allow(deprecated)]
        recent_blockhashes_account::create_account_with_data_for_test(
            vec![IterItem(0u64, &Hash::default(), 0); recent_blockhashes::MAX_ENTRIES],
        )
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
    fn test_create_from_account_is_nonce_fail() {
        let sdk = new_test_sdk();

        let nonce = Pubkey::new_unique();
        let nonce_account = AccountSharedData::new_data(
            42,
            &nonce::state::Versions::new(nonce::State::Initialized(nonce::state::Data::default())),
            &system_program::id(),
        )
        .unwrap();
        let new = Pubkey::new_unique();
        let new_account = AccountSharedData::new(0, 0, &system_program::id());

        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::CreateAccount {
                lamports: 42,
                space: 0,
                owner: Pubkey::new_unique(),
            })
            .unwrap(),
            vec![(nonce, nonce_account), (new, new_account)],
            vec![
                AccountMeta {
                    pubkey: nonce,
                    is_signer: true,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: new,
                    is_signer: true,
                    is_writable: true,
                },
            ],
            Err(InstructionError::InvalidArgument),
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
    fn test_transfer_lamports_from_nonce_account_fail() {
        let sdk = new_test_sdk();

        let from = Pubkey::new_unique();
        let from_account = AccountSharedData::new_data(
            100,
            &nonce::state::Versions::new(nonce::State::Initialized(nonce::state::Data {
                authority: from,
                ..nonce::state::Data::default()
            })),
            &system_program::id(),
        )
        .unwrap();
        assert_eq!(
            get_system_account_kind(&from_account),
            Some(SystemAccountKind::Nonce)
        );
        let to = Pubkey::from([3; 32]);
        let to_account = AccountSharedData::new(1, 0, &to); // account owner should not matter

        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::Transfer { lamports: 50 }).unwrap(),
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
            Err(InstructionError::InvalidArgument),
        );
    }

    fn process_nonce_instruction<SDK: SharedAPI>(
        sdk: &SDK,
        instruction: Instruction,
        expected_result: Result<(), InstructionError>,
    ) -> Vec<AccountSharedData> {
        let transaction_accounts = instruction
            .accounts
            .iter()
            .map(|meta| {
                #[allow(deprecated)]
                (
                    meta.pubkey,
                    if recent_blockhashes::check_id(&meta.pubkey) {
                        create_default_recent_blockhashes_account()
                    } else if sysvar::rent::check_id(&meta.pubkey) {
                        create_account_shared_data_for_test(&Rent::free())
                    } else {
                        AccountSharedData::new(0, 0, &Pubkey::new_unique())
                    },
                )
            })
            .collect();
        process_instruction(
            sdk,
            &instruction.data,
            transaction_accounts,
            instruction.accounts,
            expected_result,
        )
    }

    #[test]
    fn test_process_nonce_ix_no_acc_data_fail() {
        let sdk = new_test_sdk();

        let none_address = Pubkey::new_unique();
        process_nonce_instruction(
            &sdk,
            system_instruction::advance_nonce_account(&none_address, &none_address),
            Err(InstructionError::InvalidAccountData),
        );
    }

    #[test]
    fn test_process_nonce_ix_no_keyed_accs_fail() {
        let sdk = new_test_sdk();

        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::AdvanceNonceAccount).unwrap(),
            Vec::new(),
            Vec::new(),
            Err(InstructionError::NotEnoughAccountKeys),
        );
    }

    #[test]
    fn test_process_nonce_ix_only_nonce_acc_fail() {
        let sdk = new_test_sdk();

        let pubkey = Pubkey::new_unique();
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::AdvanceNonceAccount).unwrap(),
            vec![(pubkey, create_default_account())],
            vec![AccountMeta {
                pubkey,
                is_signer: true,
                is_writable: true,
            }],
            Err(InstructionError::NotEnoughAccountKeys),
        );
    }

    #[test]
    fn test_process_nonce_ix_ok() {
        let sdk = new_test_sdk();

        let nonce_address = Pubkey::new_unique();
        let nonce_account = nonce_account::create_account(1_000_000).into_inner();
        #[allow(deprecated)]
        let blockhash_id = recent_blockhashes::id();
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::InitializeNonceAccount(nonce_address)).unwrap(),
            vec![
                (nonce_address, nonce_account),
                (blockhash_id, create_default_recent_blockhashes_account()),
                (sysvar::rent::id(), create_default_rent_account()),
            ],
            vec![
                AccountMeta {
                    pubkey: nonce_address,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: blockhash_id,
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: sysvar::rent::id(),
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Ok(()),
        );
        let blockhash = hash(&serialize(&0).unwrap());
        #[allow(deprecated)]
        let new_recent_blockhashes_account =
            recent_blockhashes_account::create_account_with_data_for_test(vec![
                IterItem(0u64, &blockhash, 0);
                recent_blockhashes::MAX_ENTRIES
            ]);
        mock_process_instruction(
            &sdk,
            &system_program::id(),
            Vec::new(),
            &serialize(&SystemInstruction::AdvanceNonceAccount).unwrap(),
            vec![
                (nonce_address, accounts[0].clone()),
                (blockhash_id, new_recent_blockhashes_account),
            ],
            vec![
                AccountMeta {
                    pubkey: nonce_address,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: blockhash_id,
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Ok(()),
            Entrypoint::vm,
            |invoke_context: &mut InvokeContext<HostTestingContext>| {
                invoke_context.environment_config.blockhash = hash(&serialize(&0).unwrap());
            },
            |_invoke_context| {},
        );
    }

    #[test]
    fn test_process_withdraw_ix_no_acc_data_fail() {
        let sdk = new_test_sdk();

        let nonce_address = Pubkey::new_unique();
        process_nonce_instruction(
            &sdk,
            system_instruction::withdraw_nonce_account(
                &nonce_address,
                &Pubkey::new_unique(),
                &nonce_address,
                1,
            ),
            Err(InstructionError::InvalidAccountData),
        );
    }

    #[test]
    fn test_process_withdraw_ix_no_keyed_accs_fail() {
        let sdk = new_test_sdk();

        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::WithdrawNonceAccount(42)).unwrap(),
            Vec::new(),
            Vec::new(),
            Err(InstructionError::NotEnoughAccountKeys),
        );
    }

    #[test]
    fn test_process_withdraw_ix_only_nonce_acc_fail() {
        let sdk = new_test_sdk();

        let nonce_address = Pubkey::new_unique();
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::WithdrawNonceAccount(42)).unwrap(),
            vec![(nonce_address, create_default_account())],
            vec![AccountMeta {
                pubkey: nonce_address,
                is_signer: true,
                is_writable: true,
            }],
            Err(InstructionError::NotEnoughAccountKeys),
        );
    }

    #[test]
    fn test_process_withdraw_ix_ok() {
        let sdk = new_test_sdk();

        let nonce_address = Pubkey::new_unique();
        let nonce_account = nonce_account::create_account(1_000_000).into_inner();
        let pubkey = Pubkey::new_unique();
        #[allow(deprecated)]
        let blockhash_id = recent_blockhashes::id();
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::WithdrawNonceAccount(42)).unwrap(),
            vec![
                (nonce_address, nonce_account),
                (pubkey, create_default_account()),
                (blockhash_id, create_default_recent_blockhashes_account()),
                (sysvar::rent::id(), create_default_rent_account()),
            ],
            vec![
                AccountMeta {
                    pubkey: nonce_address,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: blockhash_id,
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: sysvar::rent::id(),
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Ok(()),
        );
    }

    #[test]
    fn test_process_initialize_ix_no_keyed_accs_fail() {
        let sdk = new_test_sdk();

        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::InitializeNonceAccount(Pubkey::default())).unwrap(),
            Vec::new(),
            Vec::new(),
            Err(InstructionError::NotEnoughAccountKeys),
        );
    }

    #[test]
    fn test_process_initialize_ix_only_nonce_acc_fail() {
        let sdk = new_test_sdk();

        let nonce_address = Pubkey::new_unique();
        let nonce_account = nonce_account::create_account(1_000_000).into_inner();
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::InitializeNonceAccount(nonce_address)).unwrap(),
            vec![(nonce_address, nonce_account)],
            vec![AccountMeta {
                pubkey: nonce_address,
                is_signer: true,
                is_writable: true,
            }],
            Err(InstructionError::NotEnoughAccountKeys),
        );
    }

    #[test]
    fn test_process_initialize_ix_ok() {
        let sdk = new_test_sdk();

        let nonce_address = Pubkey::new_unique();
        let nonce_account = nonce_account::create_account(1_000_000).into_inner();
        #[allow(deprecated)]
        let blockhash_id = recent_blockhashes::id();
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::InitializeNonceAccount(nonce_address)).unwrap(),
            vec![
                (nonce_address, nonce_account),
                (blockhash_id, create_default_recent_blockhashes_account()),
                (sysvar::rent::id(), create_default_rent_account()),
            ],
            vec![
                AccountMeta {
                    pubkey: nonce_address,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: blockhash_id,
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: sysvar::rent::id(),
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Ok(()),
        );
    }

    #[test]
    fn test_process_authorize_ix_ok() {
        let sdk = new_test_sdk();

        let nonce_address = Pubkey::new_unique();
        let nonce_account = nonce_account::create_account(1_000_000).into_inner();
        #[allow(deprecated)]
        let blockhash_id = recent_blockhashes::id();
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::InitializeNonceAccount(nonce_address)).unwrap(),
            vec![
                (nonce_address, nonce_account),
                (blockhash_id, create_default_recent_blockhashes_account()),
                (sysvar::rent::id(), create_default_rent_account()),
            ],
            vec![
                AccountMeta {
                    pubkey: nonce_address,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: blockhash_id,
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: sysvar::rent::id(),
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Ok(()),
        );
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::AuthorizeNonceAccount(nonce_address)).unwrap(),
            vec![(nonce_address, accounts[0].clone())],
            vec![AccountMeta {
                pubkey: nonce_address,
                is_signer: true,
                is_writable: true,
            }],
            Ok(()),
        );
    }

    #[test]
    fn test_process_authorize_bad_account_data_fail() {
        let sdk = new_test_sdk();

        let nonce_address = Pubkey::new_unique();
        process_nonce_instruction(
            &sdk,
            system_instruction::authorize_nonce_account(
                &nonce_address,
                &Pubkey::new_unique(),
                &nonce_address,
            ),
            Err(InstructionError::InvalidAccountData),
        );
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
    fn test_get_system_account_kind_nonce_ok() {
        let nonce_account = AccountSharedData::new_data(
            42,
            &nonce::state::Versions::new(nonce::State::Initialized(nonce::state::Data::default())),
            &system_program::id(),
        )
        .unwrap();
        assert_eq!(
            get_system_account_kind(&nonce_account),
            Some(SystemAccountKind::Nonce)
        );
    }

    #[test]
    fn test_get_system_account_kind_uninitialized_nonce_account_fail() {
        assert_eq!(
            get_system_account_kind(&nonce_account::create_account(42).borrow()),
            None
        );
    }

    #[test]
    fn test_get_system_account_kind_system_owner_nonzero_nonnonce_data_fail() {
        let other_data_account =
            AccountSharedData::new_data(42, b"other", &Pubkey::default()).unwrap();
        assert_eq!(get_system_account_kind(&other_data_account), None);
    }

    #[test]
    fn test_get_system_account_kind_nonsystem_owner_with_nonce_data_fail() {
        let nonce_account = AccountSharedData::new_data(
            42,
            &nonce::state::Versions::new(nonce::State::Initialized(nonce::state::Data::default())),
            &Pubkey::new_unique(),
        )
        .unwrap();
        assert_eq!(get_system_account_kind(&nonce_account), None);
    }

    #[test]
    fn test_nonce_initialize_with_empty_recent_blockhashes_fail() {
        let sdk = new_test_sdk();

        let nonce_address = Pubkey::new_unique();
        let nonce_account = nonce_account::create_account(1_000_000).into_inner();
        #[allow(deprecated)]
        let blockhash_id = recent_blockhashes::id();
        #[allow(deprecated)]
        let new_recent_blockhashes_account =
            recent_blockhashes_account::create_account_with_data_for_test(vec![]);
        process_instruction(
            &sdk,
            &serialize(&SystemInstruction::InitializeNonceAccount(nonce_address)).unwrap(),
            vec![
                (nonce_address, nonce_account),
                (blockhash_id, new_recent_blockhashes_account),
                (sysvar::rent::id(), create_default_rent_account()),
            ],
            vec![
                AccountMeta {
                    pubkey: nonce_address,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: blockhash_id,
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: sysvar::rent::id(),
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Err(SystemError::NonceNoRecentBlockhashes.into()),
        );
    }

    #[test]
    fn test_nonce_advance_with_empty_recent_blockhashes_fail() {
        let sdk = new_test_sdk();

        let nonce_address = Pubkey::new_unique();
        let nonce_account = nonce_account::create_account(1_000_000).into_inner();
        #[allow(deprecated)]
        let blockhash_id = recent_blockhashes::id();
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::InitializeNonceAccount(nonce_address)).unwrap(),
            vec![
                (nonce_address, nonce_account),
                (blockhash_id, create_default_recent_blockhashes_account()),
                (sysvar::rent::id(), create_default_rent_account()),
            ],
            vec![
                AccountMeta {
                    pubkey: nonce_address,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: blockhash_id,
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: sysvar::rent::id(),
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Ok(()),
        );
        #[allow(deprecated)]
        let new_recent_blockhashes_account =
            recent_blockhashes_account::create_account_with_data_for_test(vec![]);
        mock_process_instruction(
            &sdk,
            &system_program::id(),
            Vec::new(),
            &serialize(&SystemInstruction::AdvanceNonceAccount).unwrap(),
            vec![
                (nonce_address, accounts[0].clone()),
                (blockhash_id, new_recent_blockhashes_account),
            ],
            vec![
                AccountMeta {
                    pubkey: nonce_address,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: blockhash_id,
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Err(SystemError::NonceNoRecentBlockhashes.into()),
            Entrypoint::vm,
            |invoke_context: &mut InvokeContext<HostTestingContext>| {
                invoke_context.environment_config.blockhash = hash(&serialize(&0).unwrap());
            },
            |_invoke_context| {},
        );
    }

    #[test]
    fn test_nonce_account_upgrade_check_owner() {
        let sdk = new_test_sdk();

        let nonce_address = Pubkey::new_unique();
        let versions = NonceVersions::Legacy(Box::new(NonceState::Uninitialized));
        let nonce_account = AccountSharedData::new_data(
            1_000_000,             // lamports
            &versions,             // state
            &Pubkey::new_unique(), // owner
        )
        .unwrap();
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::UpgradeNonceAccount).unwrap(),
            vec![(nonce_address, nonce_account.clone())],
            vec![AccountMeta {
                pubkey: nonce_address,
                is_signer: false,
                is_writable: true,
            }],
            Err(InstructionError::InvalidAccountOwner),
        );
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0], nonce_account);
    }

    fn new_nonce_account(versions: NonceVersions) -> AccountSharedData {
        let nonce_account = AccountSharedData::new_data(
            1_000_000,             // lamports
            &versions,             // state
            &system_program::id(), // owner
        )
        .unwrap();
        assert_eq!(
            nonce_account.deserialize_data::<NonceVersions>().unwrap(),
            versions
        );
        nonce_account
    }

    #[test]
    fn test_nonce_account_upgrade() {
        let sdk = new_test_sdk();

        let nonce_address = Pubkey::new_unique();
        let versions = NonceVersions::Legacy(Box::new(NonceState::Uninitialized));
        let nonce_account = new_nonce_account(versions);
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::UpgradeNonceAccount).unwrap(),
            vec![(nonce_address, nonce_account.clone())],
            vec![AccountMeta {
                pubkey: nonce_address,
                is_signer: false,
                is_writable: true,
            }],
            Err(InstructionError::InvalidArgument),
        );
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0], nonce_account);
        let versions = NonceVersions::Current(Box::new(NonceState::Uninitialized));
        let nonce_account = new_nonce_account(versions);
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::UpgradeNonceAccount).unwrap(),
            vec![(nonce_address, nonce_account.clone())],
            vec![AccountMeta {
                pubkey: nonce_address,
                is_signer: false,
                is_writable: true,
            }],
            Err(InstructionError::InvalidArgument),
        );
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0], nonce_account);
        let blockhash = Hash::from([171; 32]);
        let durable_nonce = DurableNonce::from_blockhash(&blockhash);
        let data = NonceData {
            authority: Pubkey::new_unique(),
            durable_nonce,
            fee_calculator: FeeCalculator {
                lamports_per_signature: 2718,
            },
        };
        let versions = NonceVersions::Legacy(Box::new(NonceState::Initialized(data.clone())));
        let nonce_account = new_nonce_account(versions);
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::UpgradeNonceAccount).unwrap(),
            vec![(nonce_address, nonce_account.clone())],
            vec![AccountMeta {
                pubkey: nonce_address,
                is_signer: false,
                is_writable: false, // Should fail!
            }],
            Err(InstructionError::InvalidArgument),
        );
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0], nonce_account);
        let mut accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::UpgradeNonceAccount).unwrap(),
            vec![(nonce_address, nonce_account)],
            vec![AccountMeta {
                pubkey: nonce_address,
                is_signer: false,
                is_writable: true,
            }],
            Ok(()),
        );
        assert_eq!(accounts.len(), 1);
        let nonce_account = accounts.remove(0);
        let durable_nonce = DurableNonce::from_blockhash(durable_nonce.as_hash());
        assert_ne!(data.durable_nonce, durable_nonce);
        let data = NonceData {
            durable_nonce,
            ..data
        };
        let upgraded_nonce_account =
            NonceVersions::Current(Box::new(NonceState::Initialized(data)));
        assert_eq!(
            nonce_account.deserialize_data::<NonceVersions>().unwrap(),
            upgraded_nonce_account
        );
        let accounts = process_instruction(
            &sdk,
            &serialize(&SystemInstruction::UpgradeNonceAccount).unwrap(),
            vec![(nonce_address, nonce_account)],
            vec![AccountMeta {
                pubkey: nonce_address,
                is_signer: false,
                is_writable: true,
            }],
            Err(InstructionError::InvalidArgument),
        );
        assert_eq!(accounts.len(), 1);
        assert_eq!(
            accounts[0].deserialize_data::<NonceVersions>().unwrap(),
            upgraded_nonce_account
        );
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
