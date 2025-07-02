#[cfg(test)]
pub mod tests {
    // use crate::helpers::test_utils;
    use crate::{
        account::{AccountSharedData, ReadableAccount, WritableAccount},
        account_utils::StateMut,
        bpf_loader,
        bpf_loader_deprecated,
        context::InvokeContext,
        deploy_program,
        helpers::{calculate_heap_cost, create_account_shared_data_for_test},
        loaders::bpf_loader_upgradeable::Entrypoint,
        solana_program::{
            bpf_loader_upgradeable,
            bpf_loader_upgradeable::UpgradeableLoaderState,
            loader_upgradeable_instruction::UpgradeableLoaderInstruction,
            sysvar::{clock, epoch_schedule, rent},
        },
        system_program,
        test_helpers::{
            load_program_account_from_elf_file,
            mock_process_instruction,
            new_test_sdk,
            prepare_vars_for_tests,
            process_instruction,
        },
        with_mock_invoke_context,
    };
    use alloc::sync::Arc;
    use core::{
        ops::Range,
        sync::atomic::{AtomicU64, Ordering},
    };
    use fluentbase_sdk::SharedAPI;
    use rand::Rng;
    use serde::Serialize;
    use solana_bincode::serialize;
    use solana_epoch_schedule::EpochSchedule;
    use solana_instruction::AccountMeta;
    use solana_pubkey::Pubkey;
    use solana_rbpf::program::BuiltinProgram;
    use solana_rent::Rent;
    use std::{fs::File, io::Read};

    #[test]
    fn test_bpf_loader_invoke_main() {
        let loader_id = bpf_loader::id();
        let program_id = Pubkey::new_unique();
        let mut program_account =
            load_program_account_from_elf_file(&loader_id, "test_elfs/out/noop_aligned.so");
        let parameter_id = Pubkey::new_unique();
        let parameter_account = AccountSharedData::new(1, 0, &loader_id);
        let parameter_meta = AccountMeta {
            pubkey: parameter_id,
            is_signer: false,
            is_writable: false,
        };

        let sdk = new_test_sdk();

        // Case: No program account
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &[],
            Vec::new(),
            Vec::new(),
            Err(InstructionError::UnsupportedProgramId),
        );

        // Case: Only a program account
        process_instruction(
            &sdk,
            &loader_id,
            &[0],
            &[],
            vec![(program_id, program_account.clone())],
            Vec::new(),
            Ok(()),
        );

        // Case: With program and parameter account
        process_instruction(
            &sdk,
            &loader_id,
            &[0],
            &[],
            vec![
                (program_id, program_account.clone()),
                (parameter_id, parameter_account.clone()),
            ],
            vec![parameter_meta.clone()],
            Ok(()),
        );

        // Case: With duplicate accounts
        process_instruction(
            &sdk,
            &loader_id,
            &[0],
            &[],
            vec![
                (program_id, program_account.clone()),
                (parameter_id, parameter_account),
            ],
            vec![parameter_meta.clone(), parameter_meta],
            Ok(()),
        );

        // Case: limited budget
        mock_process_instruction(
            &sdk,
            &loader_id,
            vec![0],
            &[],
            vec![(program_id, program_account.clone())],
            Vec::new(),
            Err(InstructionError::ProgramFailedToComplete),
            Entrypoint::vm,
            |invoke_context| {
                invoke_context.mock_set_remaining(0);
                test_utils::load_all_invoked_programs(invoke_context);
            },
            |_invoke_context| {},
        );

        // Case: Account not a program
        program_account.set_executable(false);
        process_instruction(
            &sdk,
            &loader_id,
            &[0],
            &[],
            vec![(program_id, program_account)],
            Vec::new(),
            Err(InstructionError::IncorrectProgramId),
        );
    }

    #[test]
    fn test_bpf_loader_invoke_hello_world() {
        let loader_id = bpf_loader::id();
        let program_id = Pubkey::new_unique();
        let mut program_account = load_program_account_from_elf_file(
            &loader_id,
            "../../examples/svm/solana-program/assets/solana_program.so",
        );
        let parameter_id = Pubkey::new_unique();
        let parameter_account = AccountSharedData::new(1, 0, &loader_id);
        let parameter_meta = AccountMeta {
            pubkey: parameter_id,
            is_signer: false,
            is_writable: false,
        };

        let sdk = new_test_sdk();

        // Case: No program account
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &[],
            Vec::new(),
            Vec::new(),
            Err(InstructionError::UnsupportedProgramId),
        );

        // Case: Only a program account
        process_instruction(
            &sdk,
            &loader_id,
            &[0],
            &[],
            vec![(program_id, program_account.clone())],
            Vec::new(),
            Ok(()),
        );

        // Case: With program and parameter account
        process_instruction(
            &sdk,
            &loader_id,
            &[0],
            &[],
            vec![
                (program_id, program_account.clone()),
                (parameter_id, parameter_account.clone()),
            ],
            vec![parameter_meta.clone()],
            Ok(()),
        );

        // Case: With duplicate accounts
        process_instruction(
            &sdk,
            &loader_id,
            &[0],
            &[],
            vec![
                (program_id, program_account.clone()),
                (parameter_id, parameter_account),
            ],
            vec![parameter_meta.clone(), parameter_meta],
            Ok(()),
        );

        // Case: limited budget
        mock_process_instruction(
            &sdk,
            &loader_id,
            vec![0],
            &[],
            vec![(program_id, program_account.clone())],
            Vec::new(),
            Err(InstructionError::ProgramFailedToComplete),
            Entrypoint::vm,
            |invoke_context| {
                invoke_context.mock_set_remaining(0);
                test_utils::load_all_invoked_programs(invoke_context);
            },
            |_invoke_context| {},
        );

        // Case: Account not a program
        program_account.set_executable(false);
        process_instruction(
            &sdk,
            &loader_id,
            &[0],
            &[],
            vec![(program_id, program_account)],
            Vec::new(),
            Err(InstructionError::IncorrectProgramId),
        );
    }

    #[test]
    fn test_bpf_loader_serialize_unaligned() {
        let loader_id = bpf_loader_deprecated::id();
        let program_id = Pubkey::new_unique();
        let program_account =
            load_program_account_from_elf_file(&loader_id, "test_elfs/out/noop_unaligned.so");
        let parameter_id = Pubkey::new_unique();
        let parameter_account = AccountSharedData::new(1, 0, &loader_id);
        let parameter_meta = AccountMeta {
            pubkey: parameter_id,
            is_signer: false,
            is_writable: false,
        };

        let sdk = new_test_sdk();

        // Case: With program and parameter account
        process_instruction(
            &sdk,
            &loader_id,
            &[0],
            &[],
            vec![
                (program_id, program_account.clone()),
                (parameter_id, parameter_account.clone()),
            ],
            vec![parameter_meta.clone()],
            Ok(()),
        );

        // Case: With duplicate accounts
        process_instruction(
            &sdk,
            &loader_id,
            &[0],
            &[],
            vec![
                (program_id, program_account),
                (parameter_id, parameter_account),
            ],
            vec![parameter_meta.clone(), parameter_meta],
            Ok(()),
        );
    }

    #[test]
    fn test_bpf_loader_serialize_aligned() {
        let loader_id = bpf_loader::id();
        let program_id = Pubkey::new_unique();
        let program_account =
            load_program_account_from_elf_file(&loader_id, "test_elfs/out/noop_aligned.so");
        let parameter_id = Pubkey::new_unique();
        let parameter_account = AccountSharedData::new(1, 0, &loader_id);
        let parameter_meta = AccountMeta {
            pubkey: parameter_id,
            is_signer: false,
            is_writable: false,
        };

        let sdk = new_test_sdk();

        // Case: With program and parameter account
        process_instruction(
            &sdk,
            &loader_id,
            &[0],
            &[],
            vec![
                (program_id, program_account.clone()),
                (parameter_id, parameter_account.clone()),
            ],
            vec![parameter_meta.clone()],
            Ok(()),
        );

        // Case: With duplicate accounts
        process_instruction(
            &sdk,
            &loader_id,
            &[0],
            &[],
            vec![
                (program_id, program_account),
                (parameter_id, parameter_account),
            ],
            vec![parameter_meta.clone(), parameter_meta],
            Ok(()),
        );
    }

    #[test]
    fn test_bpf_loader_upgradeable_initialize_buffer() {
        let loader_id = bpf_loader_upgradeable::id();
        let buffer_address = Pubkey::new_unique();
        let buffer_account =
            AccountSharedData::new(1, UpgradeableLoaderState::size_of_buffer(9), &loader_id);
        let authority_address = Pubkey::new_unique();
        let authority_account =
            AccountSharedData::new(1, UpgradeableLoaderState::size_of_buffer(9), &loader_id);
        let instruction_data = serialize(&UpgradeableLoaderInstruction::InitializeBuffer).unwrap();
        let instruction_accounts = vec![
            AccountMeta {
                pubkey: buffer_address,
                is_signer: false,
                is_writable: true,
            },
            AccountMeta {
                pubkey: authority_address,
                is_signer: false,
                is_writable: false,
            },
        ];

        let sdk = new_test_sdk();

        // Case: Success
        let accounts = process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction_data,
            vec![
                (buffer_address, buffer_account),
                (authority_address, authority_account),
            ],
            instruction_accounts.clone(),
            Ok(()),
        );
        let first_account = accounts.first().unwrap();
        let state: UpgradeableLoaderState = first_account.state().unwrap();
        assert_eq!(
            state,
            UpgradeableLoaderState::Buffer {
                authority_address: Some(authority_address)
            }
        );

        // Case: Already initialized
        let accounts = process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction_data,
            vec![
                (buffer_address, accounts.first().unwrap().clone()),
                (authority_address, accounts.get(1).unwrap().clone()),
            ],
            instruction_accounts,
            Err(InstructionError::AccountAlreadyInitialized),
        );
        let state: UpgradeableLoaderState = accounts.first().unwrap().state().unwrap();
        assert_eq!(
            state,
            UpgradeableLoaderState::Buffer {
                authority_address: Some(authority_address)
            }
        );
    }

    #[test]
    fn test_bpf_loader_upgradeable_write() {
        let loader_id = bpf_loader_upgradeable::id();
        let buffer_address = Pubkey::new_unique();
        let mut buffer_account =
            AccountSharedData::new(1, UpgradeableLoaderState::size_of_buffer(9), &loader_id);
        let instruction_accounts = vec![
            AccountMeta {
                pubkey: buffer_address,
                is_signer: false,
                is_writable: true,
            },
            AccountMeta {
                pubkey: buffer_address,
                is_signer: true,
                is_writable: false,
            },
        ];

        let sdk = new_test_sdk();

        // Case: Not initialized
        let instruction = serialize(&UpgradeableLoaderInstruction::Write {
            offset: 0,
            bytes: vec![42; 9],
        })
        .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![(buffer_address, buffer_account.clone())],
            instruction_accounts.clone(),
            Err(InstructionError::InvalidAccountData),
        );

        // Case: Write entire buffer
        let instruction = serialize(&UpgradeableLoaderInstruction::Write {
            offset: 0,
            bytes: vec![42; 9],
        })
        .unwrap();
        buffer_account
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: Some(buffer_address),
            })
            .unwrap();
        let accounts = process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![(buffer_address, buffer_account.clone())],
            instruction_accounts.clone(),
            Ok(()),
        );
        let state: UpgradeableLoaderState = accounts.first().unwrap().state().unwrap();
        assert_eq!(
            state,
            UpgradeableLoaderState::Buffer {
                authority_address: Some(buffer_address)
            }
        );
        assert_eq!(
            &accounts
                .first()
                .unwrap()
                .data()
                .get(UpgradeableLoaderState::size_of_buffer_metadata()..)
                .unwrap(),
            &[42; 9]
        );

        // Case: Write portion of the buffer
        let instruction = serialize(&UpgradeableLoaderInstruction::Write {
            offset: 3,
            bytes: vec![42; 6],
        })
        .unwrap();
        let mut buffer_account =
            AccountSharedData::new(1, UpgradeableLoaderState::size_of_buffer(9), &loader_id);
        buffer_account
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: Some(buffer_address),
            })
            .unwrap();
        let accounts = process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![(buffer_address, buffer_account.clone())],
            instruction_accounts.clone(),
            Ok(()),
        );
        let state: UpgradeableLoaderState = accounts.first().unwrap().state().unwrap();
        assert_eq!(
            state,
            UpgradeableLoaderState::Buffer {
                authority_address: Some(buffer_address)
            }
        );
        assert_eq!(
            &accounts
                .first()
                .unwrap()
                .data()
                .get(UpgradeableLoaderState::size_of_buffer_metadata()..)
                .unwrap(),
            &[0, 0, 0, 42, 42, 42, 42, 42, 42]
        );

        // Case: overflow size
        let instruction = serialize(&UpgradeableLoaderInstruction::Write {
            offset: 0,
            bytes: vec![42; 10],
        })
        .unwrap();
        buffer_account
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: Some(buffer_address),
            })
            .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![(buffer_address, buffer_account.clone())],
            instruction_accounts.clone(),
            Err(InstructionError::AccountDataTooSmall),
        );

        // Case: overflow offset
        let instruction = serialize(&UpgradeableLoaderInstruction::Write {
            offset: 1,
            bytes: vec![42; 9],
        })
        .unwrap();
        buffer_account
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: Some(buffer_address),
            })
            .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![(buffer_address, buffer_account.clone())],
            instruction_accounts.clone(),
            Err(InstructionError::AccountDataTooSmall),
        );

        // Case: Not signed
        let instruction = serialize(&UpgradeableLoaderInstruction::Write {
            offset: 0,
            bytes: vec![42; 9],
        })
        .unwrap();
        buffer_account
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: Some(buffer_address),
            })
            .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![(buffer_address, buffer_account.clone())],
            vec![
                AccountMeta {
                    pubkey: buffer_address,
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: buffer_address,
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Err(InstructionError::MissingRequiredSignature),
        );

        // Case: wrong authority
        let authority_address = Pubkey::new_unique();
        let instruction = serialize(&UpgradeableLoaderInstruction::Write {
            offset: 1,
            bytes: vec![42; 9],
        })
        .unwrap();
        buffer_account
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: Some(buffer_address),
            })
            .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (buffer_address, buffer_account.clone()),
                (authority_address, buffer_account.clone()),
            ],
            vec![
                AccountMeta {
                    pubkey: buffer_address,
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: authority_address,
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Err(InstructionError::IncorrectAuthority),
        );

        // Case: None authority
        let instruction = serialize(&UpgradeableLoaderInstruction::Write {
            offset: 1,
            bytes: vec![42; 9],
        })
        .unwrap();
        buffer_account
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: None,
            })
            .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![(buffer_address, buffer_account.clone())],
            instruction_accounts,
            Err(InstructionError::Immutable),
        );
    }

    fn truncate_data(account: &mut AccountSharedData, len: usize) {
        let mut data = account.data().to_vec();
        data.truncate(len);
        account.set_data(data);
    }

    #[test]
    fn test_bpf_loader_upgradeable_upgrade() {
        let mut file = File::open("test_elfs/out/noop_aligned.so").expect("file open failed");
        let mut elf_orig = Vec::new();
        file.read_to_end(&mut elf_orig).unwrap();
        let mut file = File::open("test_elfs/out/noop_unaligned.so").expect("file open failed");
        let mut elf_new = Vec::new();
        file.read_to_end(&mut elf_new).unwrap();
        assert_ne!(elf_orig.len(), elf_new.len());
        const SLOT: u64 = 42;
        let buffer_address = Pubkey::new_unique();
        let upgrade_authority_address = Pubkey::new_unique();

        let sdk = new_test_sdk();

        fn get_accounts(
            buffer_address: &Pubkey,
            buffer_authority: &Pubkey,
            upgrade_authority_address: &Pubkey,
            elf_orig: &[u8],
            elf_new: &[u8],
        ) -> (Vec<(Pubkey, AccountSharedData)>, Vec<AccountMeta>) {
            let loader_id = bpf_loader_upgradeable::id();
            let program_address = Pubkey::new_unique();
            let spill_address = Pubkey::new_unique();
            let rent = Rent::default();
            let min_program_balance =
                1.max(rent.minimum_balance(UpgradeableLoaderState::size_of_program()));
            let min_programdata_balance = 1.max(rent.minimum_balance(
                UpgradeableLoaderState::size_of_programdata(elf_orig.len().max(elf_new.len())),
            ));
            let (programdata_address, _) =
                Pubkey::find_program_address(&[program_address.as_ref()], &loader_id);
            let mut buffer_account = AccountSharedData::new(
                1,
                UpgradeableLoaderState::size_of_buffer(elf_new.len()),
                &bpf_loader_upgradeable::id(),
            );
            buffer_account
                .set_state(&UpgradeableLoaderState::Buffer {
                    authority_address: Some(*buffer_authority),
                })
                .unwrap();
            buffer_account
                .data_as_mut_slice()
                .get_mut(UpgradeableLoaderState::size_of_buffer_metadata()..)
                .unwrap()
                .copy_from_slice(elf_new);
            let mut programdata_account = AccountSharedData::new(
                min_programdata_balance,
                UpgradeableLoaderState::size_of_programdata(elf_orig.len().max(elf_new.len())),
                &bpf_loader_upgradeable::id(),
            );
            programdata_account
                .set_state(&UpgradeableLoaderState::ProgramData {
                    slot: SLOT,
                    upgrade_authority_address: Some(*upgrade_authority_address),
                })
                .unwrap();
            let mut program_account = AccountSharedData::new(
                min_program_balance,
                UpgradeableLoaderState::size_of_program(),
                &bpf_loader_upgradeable::id(),
            );
            program_account.set_executable(true);
            program_account
                .set_state(&UpgradeableLoaderState::Program {
                    programdata_address,
                })
                .unwrap();
            let spill_account = AccountSharedData::new(0, 0, &Pubkey::new_unique());
            let rent_account = create_account_shared_data_for_test(&rent);
            let clock_account = create_account_shared_data_for_test(&clock::Clock {
                slot: SLOT.saturating_add(1),
                ..Default::default()
            });
            let upgrade_authority_account = AccountSharedData::new(1, 0, &Pubkey::new_unique());
            let transaction_accounts = vec![
                (programdata_address, programdata_account),
                (program_address, program_account),
                (*buffer_address, buffer_account),
                (spill_address, spill_account),
                (rent::id(), rent_account),
                (clock::id(), clock_account),
                (*upgrade_authority_address, upgrade_authority_account),
            ];
            let instruction_accounts = vec![
                AccountMeta {
                    pubkey: programdata_address,
                    is_signer: false,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: program_address,
                    is_signer: false,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: *buffer_address,
                    is_signer: false,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: spill_address,
                    is_signer: false,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: rent::id(),
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: clock::id(),
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: *upgrade_authority_address,
                    is_signer: true,
                    is_writable: false,
                },
            ];
            (transaction_accounts, instruction_accounts)
        }

        fn process_instruction<SDK: SharedAPI>(
            sdk: &SDK,
            transaction_accounts: Vec<(Pubkey, AccountSharedData)>,
            instruction_accounts: Vec<AccountMeta>,
            expected_result: Result<(), InstructionError>,
        ) -> Vec<AccountSharedData> {
            let instruction_data = serialize(&UpgradeableLoaderInstruction::Upgrade).unwrap();
            mock_process_instruction(
                sdk,
                &bpf_loader_upgradeable::id(),
                Vec::new(),
                &instruction_data,
                transaction_accounts,
                instruction_accounts,
                expected_result,
                Entrypoint::vm,
                |_invoke_context| {},
                |_invoke_context| {},
            )
        }

        // Case: Success
        let (transaction_accounts, instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        let accounts =
            process_instruction(&sdk, transaction_accounts, instruction_accounts, Ok(()));
        let min_programdata_balance = Rent::default().minimum_balance(
            UpgradeableLoaderState::size_of_programdata(elf_orig.len().max(elf_new.len())),
        );
        assert_eq!(
            min_programdata_balance,
            accounts.first().unwrap().lamports()
        );
        assert_eq!(0, accounts.get(2).unwrap().lamports());
        assert_eq!(1, accounts.get(3).unwrap().lamports());
        let state: UpgradeableLoaderState = accounts.first().unwrap().state().unwrap();
        assert_eq!(
            state,
            UpgradeableLoaderState::ProgramData {
                slot: SLOT.saturating_add(1),
                upgrade_authority_address: Some(upgrade_authority_address)
            }
        );
        for (i, byte) in accounts
            .first()
            .unwrap()
            .data()
            .get(
                UpgradeableLoaderState::size_of_programdata_metadata()
                    ..UpgradeableLoaderState::size_of_programdata(elf_new.len()),
            )
            .unwrap()
            .iter()
            .enumerate()
        {
            assert_eq!(*elf_new.get(i).unwrap(), *byte);
        }

        // Case: not upgradable
        let (mut transaction_accounts, instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        transaction_accounts
            .get_mut(0)
            .unwrap()
            .1
            .set_state(&UpgradeableLoaderState::ProgramData {
                slot: SLOT,
                upgrade_authority_address: None,
            })
            .unwrap();
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::Immutable),
        );

        // Case: wrong authority
        let (mut transaction_accounts, mut instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        let invalid_upgrade_authority_address = Pubkey::new_unique();
        transaction_accounts.get_mut(6).unwrap().0 = invalid_upgrade_authority_address;
        instruction_accounts.get_mut(6).unwrap().pubkey = invalid_upgrade_authority_address;
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::IncorrectAuthority),
        );

        // Case: authority did not sign
        let (transaction_accounts, mut instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        instruction_accounts.get_mut(6).unwrap().is_signer = false;
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::MissingRequiredSignature),
        );

        // Case: Buffer account and spill account alias
        let (transaction_accounts, mut instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        *instruction_accounts.get_mut(3).unwrap() = instruction_accounts.get(2).unwrap().clone();
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::AccountBorrowFailed),
        );

        // Case: Programdata account and spill account alias
        let (transaction_accounts, mut instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        *instruction_accounts.get_mut(3).unwrap() = instruction_accounts.first().unwrap().clone();
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::AccountBorrowFailed),
        );

        // Case: Program account not executable
        let (mut transaction_accounts, instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        transaction_accounts
            .get_mut(1)
            .unwrap()
            .1
            .set_executable(false);
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::AccountNotExecutable),
        );

        // Case: Program account now owned by loader
        let (mut transaction_accounts, instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        transaction_accounts
            .get_mut(1)
            .unwrap()
            .1
            .set_owner(Pubkey::new_unique());
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::IncorrectProgramId),
        );

        // Case: Program account not writable
        let (transaction_accounts, mut instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        instruction_accounts.get_mut(1).unwrap().is_writable = false;
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::InvalidArgument),
        );

        // Case: Program account not initialized
        let (mut transaction_accounts, instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        transaction_accounts
            .get_mut(1)
            .unwrap()
            .1
            .set_state(&UpgradeableLoaderState::Uninitialized)
            .unwrap();
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::InvalidAccountData),
        );

        // Case: Program ProgramData account mismatch
        let (mut transaction_accounts, mut instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        let invalid_programdata_address = Pubkey::new_unique();
        transaction_accounts.get_mut(0).unwrap().0 = invalid_programdata_address;
        instruction_accounts.get_mut(0).unwrap().pubkey = invalid_programdata_address;
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::InvalidArgument),
        );

        // Case: Buffer account not initialized
        let (mut transaction_accounts, instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        transaction_accounts
            .get_mut(2)
            .unwrap()
            .1
            .set_state(&UpgradeableLoaderState::Uninitialized)
            .unwrap();
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::InvalidArgument),
        );

        // Case: Buffer account too big
        let (mut transaction_accounts, instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        transaction_accounts.get_mut(2).unwrap().1 = AccountSharedData::new(
            1,
            UpgradeableLoaderState::size_of_buffer(
                elf_orig.len().max(elf_new.len()).saturating_add(1),
            ),
            &bpf_loader_upgradeable::id(),
        );
        transaction_accounts
            .get_mut(2)
            .unwrap()
            .1
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: Some(upgrade_authority_address),
            })
            .unwrap();
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::AccountDataTooSmall),
        );

        // Case: Buffer account too small
        let (mut transaction_accounts, instruction_accounts) = get_accounts(
            &buffer_address,
            &upgrade_authority_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        transaction_accounts
            .get_mut(2)
            .unwrap()
            .1
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: Some(upgrade_authority_address),
            })
            .unwrap();
        truncate_data(&mut transaction_accounts.get_mut(2).unwrap().1, 5);
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::InvalidAccountData),
        );

        // Case: Mismatched buffer and program authority
        let (transaction_accounts, instruction_accounts) = get_accounts(
            &buffer_address,
            &buffer_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::IncorrectAuthority),
        );

        // Case: No buffer authority
        let (mut transaction_accounts, instruction_accounts) = get_accounts(
            &buffer_address,
            &buffer_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        transaction_accounts
            .get_mut(2)
            .unwrap()
            .1
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: None,
            })
            .unwrap();
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::IncorrectAuthority),
        );

        // Case: No buffer and program authority
        let (mut transaction_accounts, instruction_accounts) = get_accounts(
            &buffer_address,
            &buffer_address,
            &upgrade_authority_address,
            &elf_orig,
            &elf_new,
        );
        transaction_accounts
            .get_mut(0)
            .unwrap()
            .1
            .set_state(&UpgradeableLoaderState::ProgramData {
                slot: SLOT,
                upgrade_authority_address: None,
            })
            .unwrap();
        transaction_accounts
            .get_mut(2)
            .unwrap()
            .1
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: None,
            })
            .unwrap();
        process_instruction(
            &sdk,
            transaction_accounts,
            instruction_accounts,
            Err(InstructionError::IncorrectAuthority),
        );
    }

    #[test]
    fn test_bpf_loader_upgradeable_set_upgrade_authority() {
        let instruction = serialize(&UpgradeableLoaderInstruction::SetAuthority).unwrap();
        let loader_id = bpf_loader_upgradeable::id();
        let slot = 0;
        let upgrade_authority_address = Pubkey::new_unique();
        let upgrade_authority_account = AccountSharedData::new(1, 0, &Pubkey::new_unique());
        let new_upgrade_authority_address = Pubkey::new_unique();
        let new_upgrade_authority_account = AccountSharedData::new(1, 0, &Pubkey::new_unique());
        let program_address = Pubkey::new_unique();
        let (programdata_address, _) = Pubkey::find_program_address(
            &[program_address.as_ref()],
            &bpf_loader_upgradeable::id(),
        );
        let mut programdata_account = AccountSharedData::new(
            1,
            UpgradeableLoaderState::size_of_programdata(0),
            &bpf_loader_upgradeable::id(),
        );
        programdata_account
            .set_state(&UpgradeableLoaderState::ProgramData {
                slot,
                upgrade_authority_address: Some(upgrade_authority_address),
            })
            .unwrap();
        let programdata_meta = AccountMeta {
            pubkey: programdata_address,
            is_signer: false,
            is_writable: true,
        };
        let upgrade_authority_meta = AccountMeta {
            pubkey: upgrade_authority_address,
            is_signer: true,
            is_writable: false,
        };
        let new_upgrade_authority_meta = AccountMeta {
            pubkey: new_upgrade_authority_address,
            is_signer: false,
            is_writable: false,
        };

        let sdk = new_test_sdk();

        // Case: Set to new authority
        let accounts = process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (upgrade_authority_address, upgrade_authority_account.clone()),
                (
                    new_upgrade_authority_address,
                    new_upgrade_authority_account.clone(),
                ),
            ],
            vec![
                programdata_meta.clone(),
                upgrade_authority_meta.clone(),
                new_upgrade_authority_meta.clone(),
            ],
            Ok(()),
        );
        let state: UpgradeableLoaderState = accounts.first().unwrap().state().unwrap();
        assert_eq!(
            state,
            UpgradeableLoaderState::ProgramData {
                slot,
                upgrade_authority_address: Some(new_upgrade_authority_address),
            }
        );

        // Case: Not upgradeable
        let accounts = process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (upgrade_authority_address, upgrade_authority_account.clone()),
            ],
            vec![programdata_meta.clone(), upgrade_authority_meta.clone()],
            Ok(()),
        );
        let state: UpgradeableLoaderState = accounts.first().unwrap().state().unwrap();
        assert_eq!(
            state,
            UpgradeableLoaderState::ProgramData {
                slot,
                upgrade_authority_address: None,
            }
        );

        // Case: Authority did not sign
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (upgrade_authority_address, upgrade_authority_account.clone()),
            ],
            vec![
                programdata_meta.clone(),
                AccountMeta {
                    pubkey: upgrade_authority_address,
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Err(InstructionError::MissingRequiredSignature),
        );

        // Case: wrong authority
        let invalid_upgrade_authority_address = Pubkey::new_unique();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (
                    invalid_upgrade_authority_address,
                    upgrade_authority_account.clone(),
                ),
                (new_upgrade_authority_address, new_upgrade_authority_account),
            ],
            vec![
                programdata_meta.clone(),
                AccountMeta {
                    pubkey: invalid_upgrade_authority_address,
                    is_signer: true,
                    is_writable: false,
                },
                new_upgrade_authority_meta,
            ],
            Err(InstructionError::IncorrectAuthority),
        );

        // Case: No authority
        programdata_account
            .set_state(&UpgradeableLoaderState::ProgramData {
                slot,
                upgrade_authority_address: None,
            })
            .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (upgrade_authority_address, upgrade_authority_account.clone()),
            ],
            vec![programdata_meta.clone(), upgrade_authority_meta.clone()],
            Err(InstructionError::Immutable),
        );

        // Case: Not a ProgramData account
        programdata_account
            .set_state(&UpgradeableLoaderState::Program {
                programdata_address: Pubkey::new_unique(),
            })
            .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (upgrade_authority_address, upgrade_authority_account),
            ],
            vec![programdata_meta, upgrade_authority_meta],
            Err(InstructionError::InvalidArgument),
        );
    }

    #[test]
    fn test_bpf_loader_upgradeable_set_upgrade_authority_checked() {
        let instruction = serialize(&UpgradeableLoaderInstruction::SetAuthorityChecked).unwrap();
        let loader_id = bpf_loader_upgradeable::id();
        let slot = 0;
        let upgrade_authority_address = Pubkey::new_unique();
        let upgrade_authority_account = AccountSharedData::new(1, 0, &Pubkey::new_unique());
        let new_upgrade_authority_address = Pubkey::new_unique();
        let new_upgrade_authority_account = AccountSharedData::new(1, 0, &Pubkey::new_unique());
        let program_address = Pubkey::new_unique();
        let (programdata_address, _) = Pubkey::find_program_address(
            &[program_address.as_ref()],
            &bpf_loader_upgradeable::id(),
        );
        let mut programdata_account = AccountSharedData::new(
            1,
            UpgradeableLoaderState::size_of_programdata(0),
            &bpf_loader_upgradeable::id(),
        );
        programdata_account
            .set_state(&UpgradeableLoaderState::ProgramData {
                slot,
                upgrade_authority_address: Some(upgrade_authority_address),
            })
            .unwrap();
        let programdata_meta = AccountMeta {
            pubkey: programdata_address,
            is_signer: false,
            is_writable: true,
        };
        let upgrade_authority_meta = AccountMeta {
            pubkey: upgrade_authority_address,
            is_signer: true,
            is_writable: false,
        };
        let new_upgrade_authority_meta = AccountMeta {
            pubkey: new_upgrade_authority_address,
            is_signer: true,
            is_writable: false,
        };

        let sdk = new_test_sdk();

        // Case: Set to new authority
        let accounts = process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (upgrade_authority_address, upgrade_authority_account.clone()),
                (
                    new_upgrade_authority_address,
                    new_upgrade_authority_account.clone(),
                ),
            ],
            vec![
                programdata_meta.clone(),
                upgrade_authority_meta.clone(),
                new_upgrade_authority_meta.clone(),
            ],
            Ok(()),
        );

        let state: UpgradeableLoaderState = accounts.first().unwrap().state().unwrap();
        assert_eq!(
            state,
            UpgradeableLoaderState::ProgramData {
                slot,
                upgrade_authority_address: Some(new_upgrade_authority_address),
            }
        );

        // Case: set to same authority
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (upgrade_authority_address, upgrade_authority_account.clone()),
            ],
            vec![
                programdata_meta.clone(),
                upgrade_authority_meta.clone(),
                upgrade_authority_meta.clone(),
            ],
            Ok(()),
        );

        // Case: present authority not in instruction
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (upgrade_authority_address, upgrade_authority_account.clone()),
                (
                    new_upgrade_authority_address,
                    new_upgrade_authority_account.clone(),
                ),
            ],
            vec![programdata_meta.clone(), new_upgrade_authority_meta.clone()],
            Err(InstructionError::NotEnoughAccountKeys),
        );

        // Case: new authority not in instruction
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (upgrade_authority_address, upgrade_authority_account.clone()),
                (
                    new_upgrade_authority_address,
                    new_upgrade_authority_account.clone(),
                ),
            ],
            vec![programdata_meta.clone(), upgrade_authority_meta.clone()],
            Err(InstructionError::NotEnoughAccountKeys),
        );

        // Case: present authority did not sign
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (upgrade_authority_address, upgrade_authority_account.clone()),
                (
                    new_upgrade_authority_address,
                    new_upgrade_authority_account.clone(),
                ),
            ],
            vec![
                programdata_meta.clone(),
                AccountMeta {
                    pubkey: upgrade_authority_address,
                    is_signer: false,
                    is_writable: false,
                },
                new_upgrade_authority_meta.clone(),
            ],
            Err(InstructionError::MissingRequiredSignature),
        );

        // Case: New authority did not sign
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (upgrade_authority_address, upgrade_authority_account.clone()),
                (
                    new_upgrade_authority_address,
                    new_upgrade_authority_account.clone(),
                ),
            ],
            vec![
                programdata_meta.clone(),
                upgrade_authority_meta.clone(),
                AccountMeta {
                    pubkey: new_upgrade_authority_address,
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Err(InstructionError::MissingRequiredSignature),
        );

        // Case: wrong present authority
        let invalid_upgrade_authority_address = Pubkey::new_unique();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (
                    invalid_upgrade_authority_address,
                    upgrade_authority_account.clone(),
                ),
                (new_upgrade_authority_address, new_upgrade_authority_account),
            ],
            vec![
                programdata_meta.clone(),
                AccountMeta {
                    pubkey: invalid_upgrade_authority_address,
                    is_signer: true,
                    is_writable: false,
                },
                new_upgrade_authority_meta.clone(),
            ],
            Err(InstructionError::IncorrectAuthority),
        );

        // Case: programdata is immutable
        programdata_account
            .set_state(&UpgradeableLoaderState::ProgramData {
                slot,
                upgrade_authority_address: None,
            })
            .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (upgrade_authority_address, upgrade_authority_account.clone()),
            ],
            vec![
                programdata_meta.clone(),
                upgrade_authority_meta.clone(),
                new_upgrade_authority_meta.clone(),
            ],
            Err(InstructionError::Immutable),
        );

        // Case: Not a ProgramData account
        programdata_account
            .set_state(&UpgradeableLoaderState::Program {
                programdata_address: Pubkey::new_unique(),
            })
            .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (upgrade_authority_address, upgrade_authority_account),
            ],
            vec![
                programdata_meta,
                upgrade_authority_meta,
                new_upgrade_authority_meta,
            ],
            Err(InstructionError::InvalidArgument),
        );
    }

    #[test]
    fn test_bpf_loader_upgradeable_set_buffer_authority() {
        let instruction = serialize(&UpgradeableLoaderInstruction::SetAuthority).unwrap();
        let loader_id = bpf_loader_upgradeable::id();
        let invalid_authority_address = Pubkey::new_unique();
        let authority_address = Pubkey::new_unique();
        let authority_account = AccountSharedData::new(1, 0, &Pubkey::new_unique());
        let new_authority_address = Pubkey::new_unique();
        let new_authority_account = AccountSharedData::new(1, 0, &Pubkey::new_unique());
        let buffer_address = Pubkey::new_unique();
        let mut buffer_account =
            AccountSharedData::new(1, UpgradeableLoaderState::size_of_buffer(0), &loader_id);
        buffer_account
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: Some(authority_address),
            })
            .unwrap();
        let mut transaction_accounts = vec![
            (buffer_address, buffer_account.clone()),
            (authority_address, authority_account.clone()),
            (new_authority_address, new_authority_account.clone()),
        ];
        let buffer_meta = AccountMeta {
            pubkey: buffer_address,
            is_signer: false,
            is_writable: true,
        };
        let authority_meta = AccountMeta {
            pubkey: authority_address,
            is_signer: true,
            is_writable: false,
        };
        let new_authority_meta = AccountMeta {
            pubkey: new_authority_address,
            is_signer: false,
            is_writable: false,
        };

        let sdk = new_test_sdk();

        // Case: New authority required
        let accounts = process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![buffer_meta.clone(), authority_meta.clone()],
            Err(InstructionError::IncorrectAuthority),
        );
        let state: UpgradeableLoaderState = accounts.first().unwrap().state().unwrap();
        assert_eq!(
            state,
            UpgradeableLoaderState::Buffer {
                authority_address: Some(authority_address),
            }
        );

        // Case: Set to new authority
        buffer_account
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: Some(authority_address),
            })
            .unwrap();
        let accounts = process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![
                buffer_meta.clone(),
                authority_meta.clone(),
                new_authority_meta.clone(),
            ],
            Ok(()),
        );
        let state: UpgradeableLoaderState = accounts.first().unwrap().state().unwrap();
        assert_eq!(
            state,
            UpgradeableLoaderState::Buffer {
                authority_address: Some(new_authority_address),
            }
        );

        // Case: Authority did not sign
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![
                buffer_meta.clone(),
                AccountMeta {
                    pubkey: authority_address,
                    is_signer: false,
                    is_writable: false,
                },
                new_authority_meta.clone(),
            ],
            Err(InstructionError::MissingRequiredSignature),
        );

        // Case: wrong authority
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (buffer_address, buffer_account.clone()),
                (invalid_authority_address, authority_account),
                (new_authority_address, new_authority_account),
            ],
            vec![
                buffer_meta.clone(),
                AccountMeta {
                    pubkey: invalid_authority_address,
                    is_signer: true,
                    is_writable: false,
                },
                new_authority_meta.clone(),
            ],
            Err(InstructionError::IncorrectAuthority),
        );

        // Case: No authority
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![buffer_meta.clone(), authority_meta.clone()],
            Err(InstructionError::IncorrectAuthority),
        );

        // Case: Set to no authority
        transaction_accounts
            .get_mut(0)
            .unwrap()
            .1
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: None,
            })
            .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![
                buffer_meta.clone(),
                authority_meta.clone(),
                new_authority_meta.clone(),
            ],
            Err(InstructionError::Immutable),
        );

        // Case: Not a Buffer account
        transaction_accounts
            .get_mut(0)
            .unwrap()
            .1
            .set_state(&UpgradeableLoaderState::Program {
                programdata_address: Pubkey::new_unique(),
            })
            .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![buffer_meta, authority_meta, new_authority_meta],
            Err(InstructionError::InvalidArgument),
        );
    }

    #[test]
    fn test_bpf_loader_upgradeable_set_buffer_authority_checked() {
        let instruction = serialize(&UpgradeableLoaderInstruction::SetAuthorityChecked).unwrap();
        let loader_id = bpf_loader_upgradeable::id();
        let invalid_authority_address = Pubkey::new_unique();
        let authority_address = Pubkey::new_unique();
        let authority_account = AccountSharedData::new(1, 0, &Pubkey::new_unique());
        let new_authority_address = Pubkey::new_unique();
        let new_authority_account = AccountSharedData::new(1, 0, &Pubkey::new_unique());
        let buffer_address = Pubkey::new_unique();
        let mut buffer_account =
            AccountSharedData::new(1, UpgradeableLoaderState::size_of_buffer(0), &loader_id);
        buffer_account
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: Some(authority_address),
            })
            .unwrap();
        let mut transaction_accounts = vec![
            (buffer_address, buffer_account.clone()),
            (authority_address, authority_account.clone()),
            (new_authority_address, new_authority_account.clone()),
        ];
        let buffer_meta = AccountMeta {
            pubkey: buffer_address,
            is_signer: false,
            is_writable: true,
        };
        let authority_meta = AccountMeta {
            pubkey: authority_address,
            is_signer: true,
            is_writable: false,
        };
        let new_authority_meta = AccountMeta {
            pubkey: new_authority_address,
            is_signer: true,
            is_writable: false,
        };

        let sdk = new_test_sdk();

        // Case: Set to new authority
        buffer_account
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: Some(authority_address),
            })
            .unwrap();
        let accounts = process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![
                buffer_meta.clone(),
                authority_meta.clone(),
                new_authority_meta.clone(),
            ],
            Ok(()),
        );
        let state: UpgradeableLoaderState = accounts.first().unwrap().state().unwrap();
        assert_eq!(
            state,
            UpgradeableLoaderState::Buffer {
                authority_address: Some(new_authority_address),
            }
        );

        // Case: set to same authority
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![
                buffer_meta.clone(),
                authority_meta.clone(),
                authority_meta.clone(),
            ],
            Ok(()),
        );

        // Case: Missing current authority
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![buffer_meta.clone(), new_authority_meta.clone()],
            Err(InstructionError::NotEnoughAccountKeys),
        );

        // Case: Missing new authority
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![buffer_meta.clone(), authority_meta.clone()],
            Err(InstructionError::NotEnoughAccountKeys),
        );

        // Case: wrong present authority
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (buffer_address, buffer_account.clone()),
                (invalid_authority_address, authority_account),
                (new_authority_address, new_authority_account),
            ],
            vec![
                buffer_meta.clone(),
                AccountMeta {
                    pubkey: invalid_authority_address,
                    is_signer: true,
                    is_writable: false,
                },
                new_authority_meta.clone(),
            ],
            Err(InstructionError::IncorrectAuthority),
        );

        // Case: present authority did not sign
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![
                buffer_meta.clone(),
                AccountMeta {
                    pubkey: authority_address,
                    is_signer: false,
                    is_writable: false,
                },
                new_authority_meta.clone(),
            ],
            Err(InstructionError::MissingRequiredSignature),
        );

        // Case: new authority did not sign
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![
                buffer_meta.clone(),
                authority_meta.clone(),
                AccountMeta {
                    pubkey: new_authority_address,
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Err(InstructionError::MissingRequiredSignature),
        );

        // Case: Not a Buffer account
        transaction_accounts
            .get_mut(0)
            .unwrap()
            .1
            .set_state(&UpgradeableLoaderState::Program {
                programdata_address: Pubkey::new_unique(),
            })
            .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![
                buffer_meta.clone(),
                authority_meta.clone(),
                new_authority_meta.clone(),
            ],
            Err(InstructionError::InvalidArgument),
        );

        // Case: Buffer is immutable
        transaction_accounts
            .get_mut(0)
            .unwrap()
            .1
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: None,
            })
            .unwrap();
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts.clone(),
            vec![buffer_meta, authority_meta, new_authority_meta],
            Err(InstructionError::Immutable),
        );
    }

    // fatal runtime error: stack overflow
    #[test]
    fn test_bpf_loader_upgradeable_close() {
        let instruction = serialize(&UpgradeableLoaderInstruction::Close).unwrap();
        let loader_id = bpf_loader_upgradeable::id();
        let invalid_authority_address = Pubkey::new_unique();
        let authority_address = Pubkey::new_unique();
        let authority_account = AccountSharedData::new(1, 0, &Pubkey::new_unique());
        let recipient_address = Pubkey::new_unique();
        let recipient_account = AccountSharedData::new(1, 0, &Pubkey::new_unique());
        let buffer_address = Pubkey::new_unique();
        let mut buffer_account =
            AccountSharedData::new(1, UpgradeableLoaderState::size_of_buffer(128), &loader_id);
        buffer_account
            .set_state(&UpgradeableLoaderState::Buffer {
                authority_address: Some(authority_address),
            })
            .unwrap();
        let uninitialized_address = Pubkey::new_unique();
        let mut uninitialized_account = AccountSharedData::new(
            1,
            UpgradeableLoaderState::size_of_programdata(0),
            &loader_id,
        );
        uninitialized_account
            .set_state(&UpgradeableLoaderState::Uninitialized)
            .unwrap();
        let programdata_address = Pubkey::new_unique();
        let mut programdata_account = AccountSharedData::new(
            1,
            UpgradeableLoaderState::size_of_programdata(128),
            &loader_id,
        );
        programdata_account
            .set_state(&UpgradeableLoaderState::ProgramData {
                slot: 0,
                upgrade_authority_address: Some(authority_address),
            })
            .unwrap();
        let program_address = Pubkey::new_unique();
        let mut program_account =
            AccountSharedData::new(1, UpgradeableLoaderState::size_of_program(), &loader_id);
        program_account.set_executable(true);
        program_account
            .set_state(&UpgradeableLoaderState::Program {
                programdata_address,
            })
            .unwrap();
        let clock_account = create_account_shared_data_for_test(&clock::Clock {
            slot: 1,
            ..clock::Clock::default()
        });
        let transaction_accounts = vec![
            (buffer_address, buffer_account.clone()),
            (recipient_address, recipient_account.clone()),
            (authority_address, authority_account.clone()),
        ];
        let buffer_meta = AccountMeta {
            pubkey: buffer_address,
            is_signer: false,
            is_writable: true,
        };
        let recipient_meta = AccountMeta {
            pubkey: recipient_address,
            is_signer: false,
            is_writable: true,
        };
        let authority_meta = AccountMeta {
            pubkey: authority_address,
            is_signer: true,
            is_writable: false,
        };

        let sdk = new_test_sdk();

        // Case: close a buffer account
        let accounts = process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            transaction_accounts,
            vec![
                buffer_meta.clone(),
                recipient_meta.clone(),
                authority_meta.clone(),
            ],
            Ok(()),
        );
        assert_eq!(0, accounts.first().unwrap().lamports());
        assert_eq!(2, accounts.get(1).unwrap().lamports());
        let state: UpgradeableLoaderState = accounts.first().unwrap().state().unwrap();
        assert_eq!(state, UpgradeableLoaderState::Uninitialized);
        assert_eq!(
            UpgradeableLoaderState::size_of_uninitialized(),
            accounts.first().unwrap().data().len()
        );

        // Case: close with wrong authority
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (buffer_address, buffer_account.clone()),
                (recipient_address, recipient_account.clone()),
                (invalid_authority_address, authority_account.clone()),
            ],
            vec![
                buffer_meta,
                recipient_meta.clone(),
                AccountMeta {
                    pubkey: invalid_authority_address,
                    is_signer: true,
                    is_writable: false,
                },
            ],
            Err(InstructionError::IncorrectAuthority),
        );

        // Case: close an uninitialized account
        let accounts = process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (uninitialized_address, uninitialized_account.clone()),
                (recipient_address, recipient_account.clone()),
                (invalid_authority_address, authority_account.clone()),
            ],
            vec![
                AccountMeta {
                    pubkey: uninitialized_address,
                    is_signer: false,
                    is_writable: true,
                },
                recipient_meta.clone(),
                authority_meta.clone(),
            ],
            Ok(()),
        );
        assert_eq!(0, accounts.first().unwrap().lamports());
        assert_eq!(2, accounts.get(1).unwrap().lamports());
        let state: UpgradeableLoaderState = accounts.first().unwrap().state().unwrap();
        assert_eq!(state, UpgradeableLoaderState::Uninitialized);
        assert_eq!(
            UpgradeableLoaderState::size_of_uninitialized(),
            accounts.first().unwrap().data().len()
        );

        // Case: close a program account
        let accounts = process_instruction(
            &sdk,
            &loader_id,
            &[],
            &instruction,
            vec![
                (programdata_address, programdata_account.clone()),
                (recipient_address, recipient_account.clone()),
                (authority_address, authority_account.clone()),
                (program_address, program_account.clone()),
                (clock::id(), clock_account.clone()),
            ],
            vec![
                AccountMeta {
                    pubkey: programdata_address,
                    is_signer: false,
                    is_writable: true,
                },
                recipient_meta,
                authority_meta,
                AccountMeta {
                    pubkey: program_address,
                    is_signer: false,
                    is_writable: true,
                },
            ],
            Ok(()),
        );
        assert_eq!(0, accounts.first().unwrap().lamports());
        assert_eq!(2, accounts.get(1).unwrap().lamports());
        let state: UpgradeableLoaderState = accounts.first().unwrap().state().unwrap();
        assert_eq!(state, UpgradeableLoaderState::Uninitialized);
        assert_eq!(
            UpgradeableLoaderState::size_of_uninitialized(),
            accounts.first().unwrap().data().len()
        );

        // Try to invoke closed account
        programdata_account = accounts.first().unwrap().clone();
        program_account = accounts.get(3).unwrap().clone();
        process_instruction(
            &sdk,
            &loader_id,
            &[0, 1],
            &[],
            vec![
                (programdata_address, programdata_account.clone()),
                (program_address, program_account.clone()),
            ],
            Vec::new(),
            Err(InstructionError::InvalidAccountData),
        );

        // Case: Reopen should fail
        process_instruction(
            &sdk,
            &loader_id,
            &[],
            &serialize(&UpgradeableLoaderInstruction::DeployWithMaxDataLen { max_data_len: 0 })
                .unwrap(),
            vec![
                (recipient_address, recipient_account),
                (programdata_address, programdata_account),
                (program_address, program_account),
                (buffer_address, buffer_account),
                (
                    rent::id(),
                    create_account_shared_data_for_test(&Rent::default()),
                ),
                (clock::id(), clock_account),
                (
                    system_program::id(),
                    AccountSharedData::new(0, 0, &system_program::id()),
                ),
                (authority_address, authority_account),
            ],
            vec![
                AccountMeta {
                    pubkey: recipient_address,
                    is_signer: true,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: programdata_address,
                    is_signer: false,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: program_address,
                    is_signer: false,
                    is_writable: true,
                },
                AccountMeta {
                    pubkey: buffer_address,
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: rent::id(),
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: clock::id(),
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: system_program::id(),
                    is_signer: false,
                    is_writable: false,
                },
                AccountMeta {
                    pubkey: authority_address,
                    is_signer: false,
                    is_writable: false,
                },
            ],
            Err(InstructionError::AccountAlreadyInitialized),
        );
    }

    /// fuzzing utility function
    fn fuzz<F>(
        bytes: &[u8],
        outer_iters: usize,
        inner_iters: usize,
        offset: Range<usize>,
        value: Range<u8>,
        work: F,
    ) where
        F: Fn(&mut [u8]),
    {
        let mut rng = rand::thread_rng();
        for _ in 0..outer_iters {
            let mut mangled_bytes = bytes.to_vec();
            for _ in 0..inner_iters {
                let offset = rng.gen_range(offset.start..offset.end);
                let value = rng.gen_range(value.start..value.end);
                *mangled_bytes.get_mut(offset).unwrap() = value;
                work(&mut mangled_bytes);
            }
        }
    }

    #[ignore] // official version doesnt work too
    #[test]
    fn test_fuzz() {
        let loader_id = bpf_loader::id();
        let program_id = Pubkey::new_unique();

        // Create program account
        let mut file = File::open("test_elfs/out/noop_aligned.so").expect("file open failed");
        let mut elf = Vec::new();
        file.read_to_end(&mut elf).unwrap();

        let sdk = new_test_sdk();

        // Mangle the whole file
        fuzz(
            &elf,
            1_000_000_000,
            100,
            0..elf.len(),
            0..255,
            |bytes: &mut [u8]| {
                let mut program_account = AccountSharedData::new(1, 0, &loader_id);
                program_account.set_data(bytes.to_vec());
                program_account.set_executable(true);
                process_instruction(
                    &sdk,
                    &loader_id,
                    &[],
                    &[],
                    vec![(program_id, program_account)],
                    Vec::new(),
                    Ok(()),
                );
            },
        );
    }

    #[test]
    fn test_calculate_heap_cost() {
        let heap_cost = 8_u64;

        // heap allocations are in 32K block, `heap_cost` of CU is consumed per additional 32k

        // assert less than 32K heap should cost zero unit
        assert_eq!(0, calculate_heap_cost(31 * 1024, heap_cost));

        // assert exact 32K heap should be cost zero unit
        assert_eq!(0, calculate_heap_cost(32 * 1024, heap_cost));

        // assert slightly more than 32K heap should cost 1 * heap_cost
        assert_eq!(heap_cost, calculate_heap_cost(33 * 1024, heap_cost));

        // assert exact 64K heap should cost 1 * heap_cost
        assert_eq!(heap_cost, calculate_heap_cost(64 * 1024, heap_cost));
    }

    fn deploy_test_program<SDK: SharedAPI>(
        invoke_context: &mut InvokeContext<SDK>,
        program_id: Pubkey,
    ) -> Result<(), InstructionError> {
        let mut file = File::open("test_elfs/out/noop_unaligned.so").expect("file open failed");
        let mut elf = Vec::new();
        file.read_to_end(&mut elf).unwrap();
        deploy_program!(
            invoke_context,
            program_id,
            &bpf_loader_upgradeable::id(),
            elf.len(),
            2,
            {},
            &elf
        );
        Ok(())
    }

    #[test]
    fn test_program_usage_count_on_upgrade() {
        let transaction_accounts = vec![(
            epoch_schedule::id(),
            create_account_shared_data_for_test(&EpochSchedule::default()),
        )];

        let sdk = new_test_sdk();
        let (config, loader) = prepare_vars_for_tests();

        with_mock_invoke_context!(
            invoke_context,
            transaction_context,
            &sdk,
            loader,
            transaction_accounts
        );
        let program_id = Pubkey::new_unique();
        let env = Arc::new(BuiltinProgram::new_mock());
        let program = LoadedProgram {
            program: Arc::new(LoadedProgramType::Unloaded(env)),
            account_size: 0,
            deployment_slot: 0,
            effective_slot: 0,
            maybe_expiration_slot: None,
            tx_usage_counter: AtomicU64::new(100),
            ix_usage_counter: AtomicU64::new(100),
            latest_access_slot: AtomicU64::new(0),
        };
        invoke_context
            .programs_modified_by_tx
            .replenish(program_id, Arc::new(program));

        assert_eq!(
            deploy_test_program(&mut invoke_context, program_id,),
            Ok(())
        );

        let updated_program = invoke_context
            .programs_modified_by_tx
            .find(&program_id)
            .expect("Didn't find upgraded program in the cache");

        assert_eq!(updated_program.deployment_slot, 2);
        assert_eq!(
            updated_program.tx_usage_counter.load(Ordering::Relaxed),
            100
        );
        assert_eq!(
            updated_program.ix_usage_counter.load(Ordering::Relaxed),
            100
        );
    }

    #[test]
    fn test_program_usage_count_on_non_upgrade() {
        let transaction_accounts = vec![(
            epoch_schedule::id(),
            create_account_shared_data_for_test(&EpochSchedule::default()),
        )];
        let sdk = new_test_sdk();
        let (config, loader) = prepare_vars_for_tests();

        with_mock_invoke_context!(
            invoke_context,
            transaction_context,
            &sdk,
            loader,
            transaction_accounts
        );
        let program_id = Pubkey::new_unique();
        let env = Arc::new(BuiltinProgram::new_mock());
        let program = LoadedProgram {
            program: Arc::new(LoadedProgramType::Unloaded(env)),
            account_size: 0,
            deployment_slot: 0,
            effective_slot: 0,
            maybe_expiration_slot: None,
            tx_usage_counter: AtomicU64::new(100),
            ix_usage_counter: AtomicU64::new(100),
            latest_access_slot: AtomicU64::new(0),
        };
        invoke_context
            .programs_modified_by_tx
            .replenish(program_id, Arc::new(program));

        let program_id2 = Pubkey::new_unique();
        assert_eq!(
            deploy_test_program(&mut invoke_context, program_id2),
            Ok(())
        );

        let program2 = invoke_context
            .programs_modified_by_tx
            .find(&program_id2)
            .expect("Didn't find upgraded program in the cache");

        assert_eq!(program2.deployment_slot, 2);
        assert_eq!(program2.tx_usage_counter.load(Ordering::Relaxed), 0);
        assert_eq!(program2.ix_usage_counter.load(Ordering::Relaxed), 0);
    }
}
