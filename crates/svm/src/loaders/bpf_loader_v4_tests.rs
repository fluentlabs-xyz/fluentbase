#[cfg(test)]
mod tests {
    use crate::{
        account::{AccountSharedData, ReadableAccount, WritableAccount},
        compute_budget::compute_budget::ComputeBudget,
        context::{IndexOfAccount, InvokeContext},
        helpers::create_account_shared_data_for_test,
        loaded_programs::ProgramCacheEntry,
        loaders::{
            bpf_loader_v4,
            bpf_loader_v4::{create_program_runtime_environment, get_state_mut},
        },
        rent,
        solana_program::{
            instruction::AccountMeta,
            loader_v4,
            loader_v4::LoaderV4Status,
            loader_v4_instruction::LoaderV4Instruction,
            sysvar,
        },
        test_helpers::{mock_process_instruction, new_test_sdk},
    };
    use fluentbase_sdk::SharedAPI;
    use solana_bincode::serialize;
    use solana_clock::Slot;
    use solana_instruction::error::InstructionError;
    use solana_pubkey::Pubkey;
    use std::{fs::File, io::Read, path::Path, sync::Arc};

    pub fn load_all_invoked_programs<SDK: SharedAPI>(invoke_context: &mut InvokeContext<SDK>) {
        // let mut load_program_metrics = LoadProgramMetrics::default();
        let num_accounts = invoke_context.transaction_context.get_number_of_accounts();
        for index in 0..num_accounts {
            let account = invoke_context
                .transaction_context
                .get_account_at_index(index)
                .expect("Failed to get the account")
                .borrow();

            let owner = account.owner();
            if loader_v4::check_id(owner) {
                let pubkey = invoke_context
                    .transaction_context
                    .get_key_of_account_at_index(index)
                    .expect("Failed to get account key");

                if let Some(programdata) = account
                    .data()
                    .get(loader_v4::LoaderV4State::program_data_offset()..)
                {
                    if let Ok(loaded_program) = ProgramCacheEntry::new(
                        &loader_v4::id(),
                        invoke_context
                            .program_cache_for_tx_batch
                            .environments
                            .program_runtime_v2
                            .clone(),
                        0,
                        0,
                        programdata,
                        account.data().len(),
                        // &mut load_program_metrics,
                    ) {
                        invoke_context
                            .program_cache_for_tx_batch
                            .set_slot_for_tests(0);
                        invoke_context
                            .program_cache_for_tx_batch
                            .replenish(*pubkey, Arc::new(loaded_program));
                    }
                }
            }
        }
    }

    fn process_instruction<SDK: SharedAPI>(
        sdk: &SDK,
        program_indices: Vec<IndexOfAccount>,
        instruction_data: &[u8],
        transaction_accounts: Vec<(Pubkey, AccountSharedData)>,
        instruction_accounts: &[(IndexOfAccount, bool, bool)],
        expected_result: Result<(), InstructionError>,
    ) -> Vec<AccountSharedData> {
        let instruction_accounts = instruction_accounts
            .iter()
            .map(
                |(index_in_transaction, is_signer, is_writable)| AccountMeta {
                    pubkey: transaction_accounts[*index_in_transaction as usize].0,
                    is_signer: *is_signer,
                    is_writable: *is_writable,
                },
            )
            .collect::<Vec<_>>();
        mock_process_instruction(
            sdk,
            &loader_v4::id(),
            program_indices,
            instruction_data,
            transaction_accounts,
            instruction_accounts,
            expected_result,
            bpf_loader_v4::Entrypoint::vm,
            |invoke_context| {
                invoke_context
                    .program_cache_for_tx_batch
                    .environments
                    .program_runtime_v2 = Arc::new(create_program_runtime_environment(
                    &ComputeBudget::default(),
                    false,
                ));
                load_all_invoked_programs(invoke_context);
            },
            |_invoke_context| {},
        )
    }

    fn load_program_account_from_elf(
        authority_address: Pubkey,
        status: LoaderV4Status,
        path: &str,
    ) -> AccountSharedData {
        let path = Path::new("test_elfs/out/").join(path).with_extension("so");
        let mut file = File::open(path).expect("file open failed");
        let mut elf_bytes = Vec::new();
        file.read_to_end(&mut elf_bytes).unwrap();
        let rent = rent::Rent::default();
        let account_size =
            loader_v4::LoaderV4State::program_data_offset().saturating_add(elf_bytes.len());
        let mut program_account = AccountSharedData::new(
            rent.minimum_balance(account_size),
            account_size,
            &loader_v4::id(),
        );
        let state = get_state_mut(program_account.data_as_mut_slice()).unwrap();
        state.slot = 0;
        state.authority_address_or_next_version = authority_address;
        state.status = status;
        program_account.data_as_mut_slice()[loader_v4::LoaderV4State::program_data_offset()..]
            .copy_from_slice(&elf_bytes);
        program_account
    }

    fn clock(slot: Slot) -> AccountSharedData {
        let clock = sysvar::clock::Clock {
            slot,
            ..sysvar::clock::Clock::default()
        };
        create_account_shared_data_for_test(&clock)
    }

    fn test_loader_instruction_general_errors(instruction: LoaderV4Instruction) {
        let sdk = new_test_sdk();

        let instruction = serialize(&instruction).unwrap();
        let authority_address = Pubkey::new_unique();
        let transaction_accounts = vec![
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Deployed,
                    "relative_call",
                ),
            ),
            (
                authority_address,
                AccountSharedData::new(0, 0, &Pubkey::new_unique()),
            ),
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Finalized,
                    "relative_call",
                ),
            ),
            (
                sysvar::clock::id(),
                create_account_shared_data_for_test(&sysvar::clock::Clock::default()),
            ),
            (
                sysvar::rent::id(),
                create_account_shared_data_for_test(&rent::Rent::default()),
            ),
        ];

        // Error: Missing program account
        process_instruction(
            &sdk,
            vec![],
            &instruction,
            transaction_accounts.clone(),
            &[],
            Err(InstructionError::NotEnoughAccountKeys),
        );

        // Error: Missing authority account
        process_instruction(
            &sdk,
            vec![],
            &instruction,
            transaction_accounts.clone(),
            &[(0, false, true)],
            Err(InstructionError::NotEnoughAccountKeys),
        );

        // Error: Program not owned by loader
        process_instruction(
            &sdk,
            vec![],
            &instruction,
            transaction_accounts.clone(),
            &[(1, false, true), (1, true, false), (2, true, true)],
            Err(InstructionError::InvalidAccountOwner),
        );

        // Error: Program is not writeable
        process_instruction(
            &sdk,
            vec![],
            &instruction,
            transaction_accounts.clone(),
            &[(0, false, false), (1, true, false), (2, true, true)],
            Err(InstructionError::InvalidArgument),
        );

        // Error: Authority did not sign
        process_instruction(
            &sdk,
            vec![],
            &instruction,
            transaction_accounts.clone(),
            &[(0, false, true), (1, false, false), (2, true, true)],
            Err(InstructionError::MissingRequiredSignature),
        );

        // Error: Program is finalized
        process_instruction(
            &sdk,
            vec![],
            &instruction,
            transaction_accounts.clone(),
            &[(2, false, true), (1, true, false), (0, true, true)],
            Err(InstructionError::Immutable),
        );

        // Error: Incorrect authority provided
        process_instruction(
            &sdk,
            vec![],
            &instruction,
            transaction_accounts,
            &[(0, false, true), (2, true, false), (2, true, true)],
            Err(InstructionError::IncorrectAuthority),
        );
    }

    #[test]
    fn test_loader_instruction_write() {
        let sdk = new_test_sdk();

        let authority_address = Pubkey::new_unique();
        let transaction_accounts = vec![
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Retracted,
                    "relative_call",
                ),
            ),
            (
                authority_address,
                AccountSharedData::new(0, 0, &Pubkey::new_unique()),
            ),
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Deployed,
                    "relative_call",
                ),
            ),
            (
                sysvar::clock::id(),
                create_account_shared_data_for_test(&sysvar::clock::Clock::default()),
            ),
            (
                sysvar::rent::id(),
                create_account_shared_data_for_test(&rent::Rent::default()),
            ),
        ];

        // Overwrite existing data
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Write {
                offset: 2,
                bytes: vec![8, 8, 8, 8],
            })
            .unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false)],
            Ok(()),
        );

        // Empty write
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Write {
                offset: 2,
                bytes: Vec::new(),
            })
            .unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false)],
            Ok(()),
        );

        // Error: Program is not retracted
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Write {
                offset: 8,
                bytes: vec![8, 8, 8, 8],
            })
            .unwrap(),
            transaction_accounts.clone(),
            &[(2, false, true), (1, true, false)],
            Err(InstructionError::InvalidArgument),
        );

        // Error: Write out of bounds
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Write {
                offset: transaction_accounts[0]
                    .1
                    .data()
                    .len()
                    .saturating_sub(loader_v4::LoaderV4State::program_data_offset())
                    .saturating_sub(3) as u32,
                bytes: vec![8, 8, 8, 8],
            })
            .unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false)],
            Err(InstructionError::AccountDataTooSmall),
        );

        test_loader_instruction_general_errors(
            // &sdk,
            LoaderV4Instruction::Write {
                offset: 0,
                bytes: Vec::new(),
            },
        );
    }

    #[test]
    fn test_loader_instruction_truncate() {
        let sdk = new_test_sdk();

        let authority_address = Pubkey::new_unique();
        let mut transaction_accounts = vec![
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Retracted,
                    "relative_call",
                ),
            ),
            (
                authority_address,
                AccountSharedData::new(0, 0, &Pubkey::new_unique()),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(0, 0, &loader_v4::id()),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(40000000, 0, &loader_v4::id()),
            ),
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Retracted,
                    "rodata_section",
                ),
            ),
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Deployed,
                    "relative_call",
                ),
            ),
            (
                sysvar::clock::id(),
                create_account_shared_data_for_test(&sysvar::clock::Clock::default()),
            ),
            (
                sysvar::rent::id(),
                create_account_shared_data_for_test(&rent::Rent::default()),
            ),
        ];

        // No change
        let accounts = process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate {
                new_size: transaction_accounts[0]
                    .1
                    .data()
                    .len()
                    .saturating_sub(loader_v4::LoaderV4State::program_data_offset())
                    as u32,
            })
            .unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false)],
            Ok(()),
        );
        assert_eq!(
            accounts[0].data().len(),
            transaction_accounts[0].1.data().len(),
        );
        assert_eq!(accounts[2].lamports(), transaction_accounts[2].1.lamports());
        let lamports = transaction_accounts[4].1.lamports();
        transaction_accounts[0].1.set_lamports(lamports);

        // Initialize program account
        let accounts = process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate {
                new_size: transaction_accounts[0]
                    .1
                    .data()
                    .len()
                    .saturating_sub(loader_v4::LoaderV4State::program_data_offset())
                    as u32,
            })
            .unwrap(),
            transaction_accounts.clone(),
            &[(3, true, true), (1, true, false), (2, false, true)],
            Ok(()),
        );
        assert_eq!(
            accounts[3].data().len(),
            transaction_accounts[0].1.data().len(),
        );

        // Increase program account size
        let accounts = process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate {
                new_size: transaction_accounts[4]
                    .1
                    .data()
                    .len()
                    .saturating_sub(loader_v4::LoaderV4State::program_data_offset())
                    as u32,
            })
            .unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false)],
            Ok(()),
        );
        assert_eq!(
            accounts[0].data().len(),
            transaction_accounts[4].1.data().len(),
        );

        // Decrease program account size
        let accounts = process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate {
                new_size: transaction_accounts[0]
                    .1
                    .data()
                    .len()
                    .saturating_sub(loader_v4::LoaderV4State::program_data_offset())
                    as u32,
            })
            .unwrap(),
            transaction_accounts.clone(),
            &[(4, false, true), (1, true, false), (2, false, true)],
            Ok(()),
        );
        assert_eq!(
            accounts[4].data().len(),
            transaction_accounts[0].1.data().len(),
        );
        assert_eq!(
            accounts[2].lamports(),
            transaction_accounts[2].1.lamports().saturating_add(
                transaction_accounts[4]
                    .1
                    .lamports()
                    .saturating_sub(accounts[4].lamports())
            ),
        );

        // Close program account
        let accounts = process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate { new_size: 0 }).unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false), (2, false, true)],
            Ok(()),
        );
        assert_eq!(accounts[0].data().len(), 0);
        assert_eq!(
            accounts[2].lamports(),
            transaction_accounts[2].1.lamports().saturating_add(
                transaction_accounts[0]
                    .1
                    .lamports()
                    .saturating_sub(accounts[0].lamports())
            ),
        );

        // Error: Program not owned by loader
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate { new_size: 8 }).unwrap(),
            transaction_accounts.clone(),
            &[(1, false, true), (1, true, false), (2, true, true)],
            Err(InstructionError::InvalidAccountOwner),
        );

        // Error: Program is not writeable
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate { new_size: 8 }).unwrap(),
            transaction_accounts.clone(),
            &[(3, false, false), (1, true, false), (2, true, true)],
            Err(InstructionError::InvalidArgument),
        );

        // Error: Program did not sign
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate { new_size: 8 }).unwrap(),
            transaction_accounts.clone(),
            &[(3, false, true), (1, true, false), (2, true, true)],
            Err(InstructionError::MissingRequiredSignature),
        );

        // Error: Authority did not sign
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate { new_size: 8 }).unwrap(),
            transaction_accounts.clone(),
            &[(3, true, true), (1, false, false), (2, true, true)],
            Err(InstructionError::MissingRequiredSignature),
        );

        // Error: Program is and stays uninitialized
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate { new_size: 0 }).unwrap(),
            transaction_accounts.clone(),
            &[(3, false, true), (1, true, false), (2, true, true)],
            Err(InstructionError::InvalidAccountData),
        );

        // Error: Program is not retracted
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate { new_size: 8 }).unwrap(),
            transaction_accounts.clone(),
            &[(5, false, true), (1, true, false), (2, false, true)],
            Err(InstructionError::InvalidArgument),
        );

        // Error: Missing recipient account
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate { new_size: 0 }).unwrap(),
            transaction_accounts.clone(),
            &[(0, true, true), (1, true, false)],
            Err(InstructionError::NotEnoughAccountKeys),
        );

        // Error: Recipient is not writeable
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate { new_size: 0 }).unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false), (2, false, false)],
            Err(InstructionError::InvalidArgument),
        );

        // Error: Insufficient funds
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Truncate {
                new_size: transaction_accounts[4]
                    .1
                    .data()
                    .len()
                    .saturating_sub(loader_v4::LoaderV4State::program_data_offset())
                    .saturating_add(1) as u32,
            })
            .unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false)],
            Err(InstructionError::InsufficientFunds),
        );

        test_loader_instruction_general_errors(
            // &sdk,
            LoaderV4Instruction::Truncate { new_size: 0 },
        );
    }

    #[test]
    fn test_loader_instruction_deploy() {
        let sdk = new_test_sdk();

        let authority_address = Pubkey::new_unique();
        let mut transaction_accounts = vec![
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Retracted,
                    "rodata_section",
                ),
            ),
            (
                authority_address,
                AccountSharedData::new(0, 0, &Pubkey::new_unique()),
            ),
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Retracted,
                    "relative_call",
                ),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(0, 0, &loader_v4::id()),
            ),
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Retracted,
                    "invalid",
                ),
            ),
            (sysvar::clock::id(), clock(1000)),
            (
                sysvar::rent::id(),
                create_account_shared_data_for_test(&rent::Rent::default()),
            ),
        ];

        // Deploy from its own data
        let accounts = process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Deploy).unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false)],
            Ok(()),
        );
        transaction_accounts[0].1 = accounts[0].clone();
        transaction_accounts[5].1 = clock(2000);
        assert_eq!(
            accounts[0].data().len(),
            transaction_accounts[0].1.data().len(),
        );
        assert_eq!(accounts[0].lamports(), transaction_accounts[0].1.lamports());

        // Error: Source program is not writable
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Deploy).unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false), (2, false, false)],
            Err(InstructionError::InvalidArgument),
        );

        // Error: Source program is not retracted
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Deploy).unwrap(),
            transaction_accounts.clone(),
            &[(2, false, true), (1, true, false), (0, false, true)],
            Err(InstructionError::InvalidArgument),
        );

        // Redeploy: Retract, then replace data by other source
        let accounts = process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Retract).unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false)],
            Ok(()),
        );
        transaction_accounts[0].1 = accounts[0].clone();
        let accounts = process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Deploy).unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false), (2, false, true)],
            Ok(()),
        );
        transaction_accounts[0].1 = accounts[0].clone();
        assert_eq!(
            accounts[0].data().len(),
            transaction_accounts[2].1.data().len(),
        );
        assert_eq!(accounts[2].data().len(), 0,);
        assert_eq!(
            accounts[2].lamports(),
            transaction_accounts[2].1.lamports().saturating_sub(
                accounts[0]
                    .lamports()
                    .saturating_sub(transaction_accounts[0].1.lamports())
            ),
        );

        // Error: Program was deployed recently, cooldown still in effect
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Deploy).unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false)],
            Err(InstructionError::InvalidArgument),
        );
        transaction_accounts[5].1 = clock(3000);

        // Error: Program is uninitialized
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Deploy).unwrap(),
            transaction_accounts.clone(),
            &[(3, false, true), (1, true, false)],
            Err(InstructionError::InvalidAccountData),
        );

        // Error: Program fails verification
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Deploy).unwrap(),
            transaction_accounts.clone(),
            &[(4, false, true), (1, true, false)],
            Err(InstructionError::InvalidAccountData),
        );

        // Error: Program is deployed already
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Deploy).unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false)],
            Err(InstructionError::InvalidArgument),
        );

        test_loader_instruction_general_errors(
            // &sdk,
            LoaderV4Instruction::Deploy,
        );
    }

    #[test]
    fn test_loader_instruction_retract() {
        let sdk = new_test_sdk();

        let authority_address = Pubkey::new_unique();
        let mut transaction_accounts = vec![
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Deployed,
                    "rodata_section",
                ),
            ),
            (
                authority_address,
                AccountSharedData::new(0, 0, &Pubkey::new_unique()),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(0, 0, &loader_v4::id()),
            ),
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Retracted,
                    "rodata_section",
                ),
            ),
            (sysvar::clock::id(), clock(1000)),
            (
                sysvar::rent::id(),
                create_account_shared_data_for_test(&rent::Rent::default()),
            ),
        ];

        // Retract program
        let accounts = process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Retract).unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false)],
            Ok(()),
        );
        assert_eq!(
            accounts[0].data().len(),
            transaction_accounts[0].1.data().len(),
        );
        assert_eq!(accounts[0].lamports(), transaction_accounts[0].1.lamports());

        // Error: Program is uninitialized
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Retract).unwrap(),
            transaction_accounts.clone(),
            &[(2, false, true), (1, true, false)],
            Err(InstructionError::InvalidAccountData),
        );

        // Error: Program is not deployed
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Retract).unwrap(),
            transaction_accounts.clone(),
            &[(3, false, true), (1, true, false)],
            Err(InstructionError::InvalidArgument),
        );

        // Error: Program was deployed recently, cooldown still in effect
        transaction_accounts[4].1 = clock(0);
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::Retract).unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (1, true, false)],
            Err(InstructionError::InvalidArgument),
        );

        test_loader_instruction_general_errors(/*&sdk, */ LoaderV4Instruction::Retract);
    }

    #[test]
    fn test_loader_instruction_transfer_authority() {
        let sdk = new_test_sdk();

        let authority_address = Pubkey::new_unique();
        let transaction_accounts = vec![
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Deployed,
                    "rodata_section",
                ),
            ),
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Retracted,
                    "rodata_section",
                ),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(0, 0, &loader_v4::id()),
            ),
            (
                authority_address,
                AccountSharedData::new(0, 0, &Pubkey::new_unique()),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(0, 0, &Pubkey::new_unique()),
            ),
            (
                sysvar::clock::id(),
                create_account_shared_data_for_test(&sysvar::clock::Clock::default()),
            ),
            (
                sysvar::rent::id(),
                create_account_shared_data_for_test(&rent::Rent::default()),
            ),
        ];

        // Transfer authority
        let accounts = process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::TransferAuthority).unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (3, true, false), (4, true, false)],
            Ok(()),
        );
        assert_eq!(
            accounts[0].data().len(),
            transaction_accounts[0].1.data().len(),
        );
        assert_eq!(accounts[0].lamports(), transaction_accounts[0].1.lamports());

        // Finalize program
        let accounts = process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::TransferAuthority).unwrap(),
            transaction_accounts.clone(),
            &[(0, false, true), (3, true, false)],
            Ok(()),
        );
        assert_eq!(
            accounts[0].data().len(),
            transaction_accounts[0].1.data().len(),
        );
        assert_eq!(accounts[0].lamports(), transaction_accounts[0].1.lamports());

        // Error: Program must be deployed to be finalized
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::TransferAuthority).unwrap(),
            transaction_accounts.clone(),
            &[(1, false, true), (3, true, false)],
            Err(InstructionError::InvalidArgument),
        );

        // Error: Program is uninitialized
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::TransferAuthority).unwrap(),
            transaction_accounts.clone(),
            &[(2, false, true), (3, true, false), (4, true, false)],
            Err(InstructionError::InvalidAccountData),
        );

        // Error: New authority did not sign
        process_instruction(
            &sdk,
            vec![],
            &serialize(&LoaderV4Instruction::TransferAuthority).unwrap(),
            transaction_accounts,
            &[(0, false, true), (3, true, false), (4, false, false)],
            Err(InstructionError::MissingRequiredSignature),
        );

        test_loader_instruction_general_errors(
            /*&sdk, */ LoaderV4Instruction::TransferAuthority,
        );
    }

    #[test]
    fn test_execute_program() {
        let sdk = new_test_sdk();

        let program_address = Pubkey::new_unique();
        let authority_address = Pubkey::new_unique();
        let transaction_accounts = vec![
            (
                program_address,
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Finalized,
                    "rodata_section",
                ),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(10000000, 32, &program_address),
            ),
            (
                Pubkey::new_unique(),
                AccountSharedData::new(0, 0, &loader_v4::id()),
            ),
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Retracted,
                    "rodata_section",
                ),
            ),
            (
                Pubkey::new_unique(),
                load_program_account_from_elf(
                    authority_address,
                    LoaderV4Status::Finalized,
                    "invalid",
                ),
            ),
        ];

        // Execute program
        process_instruction(
            &sdk,
            vec![0],
            &[0, 1, 2, 3],
            transaction_accounts.clone(),
            &[(1, false, true)],
            Err(InstructionError::Custom(42)),
        );

        // Error: Program not owned by loader
        process_instruction(
            &sdk,
            vec![1],
            &[0, 1, 2, 3],
            transaction_accounts.clone(),
            &[(1, false, true)],
            Err(InstructionError::InvalidAccountOwner),
        );

        // Error: Program is uninitialized
        process_instruction(
            &sdk,
            vec![2],
            &[0, 1, 2, 3],
            transaction_accounts.clone(),
            &[(1, false, true)],
            Err(InstructionError::InvalidAccountData),
        );

        // Error: Program is not deployed
        process_instruction(
            &sdk,
            vec![3],
            &[0, 1, 2, 3],
            transaction_accounts.clone(),
            &[(1, false, true)],
            Err(InstructionError::InvalidArgument),
        );

        // Error: Program fails verification
        process_instruction(
            &sdk,
            vec![4],
            &[0, 1, 2, 3],
            transaction_accounts,
            &[(1, false, true)],
            Err(InstructionError::InvalidAccountData),
        );
    }
}
