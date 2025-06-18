mod tests {
    use crate::{
        account::{AccountSharedData, ReadableAccount},
        common::{calculate_max_chunk_size, pubkey_from_address},
        fluentbase::{
            common::{BatchMessage, MemStorage},
            helpers_v2::{exec_encoded_svm_batch_message, exec_encoded_svm_message},
        },
        helpers::{storage_read_account_data, storage_write_account_data},
        loaders::bpf_loader_v4::get_state,
        native_loader,
        native_loader::create_loadable_account_for_test,
        solana_program::{
            instruction::Instruction,
            loader_v4,
            loader_v4::{create_buffer, LoaderV4State, LoaderV4Status},
            message::Message,
            sysvar,
        },
        system_program,
        test_helpers::load_program_account_from_elf_file,
    };
    use core::str::from_utf8;
    use fluentbase_sdk::{
        address,
        Address,
        BlockContextV1,
        ContractContextV1,
        SharedAPI,
        SharedContextInputV1,
        StorageAPI,
    };
    use fluentbase_sdk_testing::HostTestingContext;
    use hashbrown::HashMap;
    use solana_bincode::serialize;
    use solana_instruction::AccountMeta;
    use solana_pubkey::Pubkey;

    fn main_single_message<SAPI: StorageAPI>(
        mut sdk: impl SharedAPI,
        mut sapi: Option<&mut SAPI>,
    ) -> HashMap<Pubkey, AccountSharedData> {
        let input = sdk.input();

        let result = exec_encoded_svm_message(&mut sdk, input, true, &mut sapi);
        if let Err(err) = result {
            panic!("exec svm message error: {:?}", err);
        }
        result.unwrap()
    }

    fn main_batch_message<SAPI: StorageAPI>(
        mut sdk: impl SharedAPI,
        mut sapi: Option<&mut SAPI>,
    ) -> HashMap<Pubkey, AccountSharedData> {
        let input = sdk.input();

        let result = exec_encoded_svm_batch_message(&mut sdk, input, true, &mut sapi);
        if let Err(err) = result {
            panic!("exec svm message error: {:?}", err);
        }
        result.unwrap()
    }

    #[test]
    fn test_create_fill_deploy_exec() {
        // setup

        let system_program_id = system_program::id();
        let native_loader_id = native_loader::id();
        let loader_id = loader_v4::id();
        let sysvar_clock_id = sysvar::clock::id();
        let sysvar_rent_id = sysvar::rent::id();

        let pk_payer = Pubkey::new_unique();
        let pk_payer_account = AccountSharedData::new(100, 0, &system_program_id);

        let pk_tmp = Pubkey::new_unique();
        let pk_tmp_account = AccountSharedData::new(100, 0, &system_program_id);

        let pk_exec = Pubkey::from([8; 32]);

        // let pk_exec_data = Pubkey::from([3; 32]);
        // let pk_exec_data_account = AccountSharedData::new(0, 0, &pk_exec);

        let pk_authority = Pubkey::from([9; 32]);
        // let pk_authority_account = AccountSharedData::new(100, 0, &system_program_id);

        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../../examples/svm/solana-program/assets/solana_program.so",
            "../../examples/svm/solana-program-transfer-with-cpi/assets/solana_program.so",
            // "./test_elfs/out/noop_aligned.so",
        );

        let program_len = account_with_program.data().len();
        let buffer_len = LoaderV4State::program_data_offset().saturating_add(program_len);

        let shared_context = SharedContextInputV1 {
            block: Default::default(),
            tx: Default::default(),
            contract: ContractContextV1 {
                address: Default::default(),
                bytecode_address: Default::default(),
                caller: Default::default(),
                is_static: false,
                value: Default::default(),
                gas_limit: 0,
            },
        };
        let sdk = HostTestingContext::default().with_shared_context_input(shared_context);
        let mut sapi = MemStorage::new();

        storage_write_account_data(&mut sapi, &pk_payer, &pk_payer_account).unwrap();
        storage_write_account_data(&mut sapi, &pk_tmp, &pk_tmp_account).unwrap();
        storage_write_account_data(
            &mut sapi,
            &system_program_id,
            &create_loadable_account_for_test("system_program_id", &native_loader_id),
        )
        .unwrap();
        storage_write_account_data(
            &mut sapi,
            &loader_id,
            &create_loadable_account_for_test("loader_id", &native_loader_id),
        )
        .unwrap();
        storage_write_account_data(
            &mut sapi,
            &sysvar_clock_id,
            &create_loadable_account_for_test("sysvar_clock_id", &system_program_id),
        )
        .unwrap();
        storage_write_account_data(
            &mut sapi,
            &sysvar_rent_id,
            &create_loadable_account_for_test("sysvar_rent_id", &system_program_id),
        )
        .unwrap();

        // init buffer

        let instructions = create_buffer(
            &pk_payer,
            &pk_exec,
            0,
            &pk_authority,
            program_len as u32,
            &pk_payer,
        );
        let message = Message::new(&instructions, Some(&pk_payer));
        let mut sdk = sdk.with_input(serialize(&message).unwrap());
        main_single_message(sdk.clone(), Some(&mut sapi));
        let output = sdk.take_output();
        assert_eq!(from_utf8(&output).unwrap(), "");

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        assert_eq!(account_data.lamports(), 100);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        // let account_data: AccountSharedData =
        //     storage_read_account_data(&sapi, &pk_authority).unwrap();
        // assert_eq!(account_data.lamports(), 100);
        // assert_eq!(account_data.data().len(), 0);
        // assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), 0);
        assert_eq!(account_data.data().len(), buffer_len);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &system_program_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), 17);
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &loader_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), "loader_id".len());
        assert_eq!(account_data.executable(), true);

        // fill buffer

        let create_msg = |offset: u32, bytes: Vec<u8>| {
            let instruction = loader_v4::write(&pk_exec, &pk_authority, offset, bytes);
            let instructions = vec![instruction];
            Message::new(&instructions, Some(&pk_payer))
        };
        let mut write_messages = vec![];
        let chunk_size = calculate_max_chunk_size(&create_msg);
        for (chunk, i) in account_with_program.data().chunks(chunk_size).zip(0..) {
            let offset = i * chunk_size;
            let msg = create_msg(offset as u32, chunk.to_vec());
            write_messages.push(msg);
        }
        for (_, message) in write_messages.iter().enumerate() {
            sdk = sdk.with_input(serialize(&message).unwrap());
            main_single_message(sdk.clone(), Some(&mut sapi));
        }

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        assert_eq!(account_data.lamports(), 100);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), 0);
        assert_eq!(account_data.data().len(), buffer_len);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &system_program_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), 17);
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &loader_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), "loader_id".len());
        assert_eq!(account_data.executable(), true);

        // deploy

        let instruction = loader_v4::deploy(&pk_exec, &pk_authority);
        let message = Message::new(&[instruction], Some(&pk_payer));
        sdk = sdk.with_input(serialize(&message).unwrap());
        main_single_message(sdk.clone(), Some(&mut sapi));

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        assert_eq!(account_data.lamports(), 100);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), 0);
        assert_eq!(account_data.data().len(), buffer_len);
        assert_eq!(account_data.executable(), false);

        // let account_data: AccountSharedData =
        //     storage_read_account_data(&sapi, &pk_authority).unwrap();
        // assert_eq!(account_data.lamports(), 100);
        // assert_eq!(account_data.data().len(), 0);
        // assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &system_program_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), 17);
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &loader_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), "loader_id".len());
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &sysvar_clock_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), "sysvar_clock_id".len());
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &sysvar_rent_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), "sysvar_rent_id".len());
        assert_eq!(account_data.executable(), true);

        // exec

        let amount = 12u64;
        let instructions = vec![Instruction::new_with_bincode(
            pk_exec.clone(),
            &amount.to_be_bytes(),
            vec![
                AccountMeta::new(pk_tmp, true),
                AccountMeta::new(pk_payer, false),
                AccountMeta::new(system_program_id, false),
            ],
        )];
        let message = Message::new(&instructions, None);
        sdk = sdk
            .with_shared_context_input(SharedContextInputV1 {
                block: BlockContextV1 {
                    number: 1,
                    ..Default::default()
                },
                ..Default::default()
            })
            .with_input(serialize(&message).unwrap());
        let _result_accounts = main_single_message(sdk.clone(), Some(&mut sapi));

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), 0);
        assert_eq!(account_data.data().len(), buffer_len);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        assert_eq!(account_data.lamports(), 112);
    }

    #[test]
    fn test_create_fill_deploy_exec_with_state() {
        // setup

        let system_program_id = system_program::id();
        let native_loader_id = native_loader::id();
        let loader_id = loader_v4::id();
        let sysvar_clock_id = sysvar::clock::id();
        let sysvar_rent_id = sysvar::rent::id();

        const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
        // let pk_payer = Pubkey::new_unique();
        let pk_payer = pubkey_from_address(&DEPLOYER_ADDRESS);
        let pk_payer_account = AccountSharedData::new(101, 0, &system_program_id);

        // let pk_exec = Pubkey::from([8; 32]);
        const CONTRACT_ADDRESS: Address = address!("0xf91c20c0cafbfdc150adff51bbfc5808edde7cb5");
        // let pk_exec = pubkey_from_pubkey(&Pubkey::from([8; 32]));
        let pk_exec = pubkey_from_address(&CONTRACT_ADDRESS);

        // let pk_tmp = Pubkey::new_unique();
        // let pk_tmp_account = AccountSharedData::new(100, 0, &pk_exec);

        let seed1 = b"my_seed";
        let seed2 = pk_payer.as_ref();
        let seeds = &[seed1, seed2];
        let (pk_new, _bump) = Pubkey::find_program_address(seeds, &pk_exec);

        let pk_authority = Pubkey::from([9; 32]);
        // let pk_authority_account = AccountSharedData::new(100, 0, &system_program_id);

        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../../examples/svm/solana-program/assets/solana_program.so",
            "../../examples/svm/solana-program-state-usage/assets/solana_program.so",
            // "./test_elfs/out/noop_aligned.so",
        );

        let program_len = account_with_program.data().len();
        let buffer_len = LoaderV4State::program_data_offset().saturating_add(program_len);

        let shared_context = SharedContextInputV1 {
            block: Default::default(),
            tx: Default::default(),
            contract: ContractContextV1 {
                address: Default::default(),
                bytecode_address: Default::default(),
                caller: Default::default(),
                is_static: false,
                value: Default::default(),
                gas_limit: 0,
            },
        };
        let sdk = HostTestingContext::default().with_shared_context_input(shared_context);
        let mut sapi = MemStorage::new();

        storage_write_account_data(&mut sapi, &pk_payer, &pk_payer_account).unwrap();
        // storage_write_account_data(&mut sapi, &pk_tmp, &pk_tmp_account).unwrap();
        storage_write_account_data(
            &mut sapi,
            &system_program_id,
            &create_loadable_account_for_test("system_program_id", &native_loader_id),
        )
        .unwrap();
        storage_write_account_data(
            &mut sapi,
            &loader_id,
            &create_loadable_account_for_test("loader_id", &native_loader_id),
        )
        .unwrap();
        storage_write_account_data(
            &mut sapi,
            &sysvar_clock_id,
            &create_loadable_account_for_test("sysvar_clock_id", &system_program_id),
        )
        .unwrap();
        storage_write_account_data(
            &mut sapi,
            &sysvar_rent_id,
            &create_loadable_account_for_test("sysvar_rent_id", &system_program_id),
        )
        .unwrap();

        // init buffer

        let instructions = create_buffer(
            &pk_payer,
            &pk_exec,
            0,
            &pk_authority,
            program_len as u32,
            &pk_payer,
        );
        let message = Message::new(&instructions, Some(&pk_payer));
        let mut sdk = sdk.with_input(serialize(&message).unwrap());
        main_single_message(sdk.clone(), Some(&mut sapi));
        let output = sdk.take_output();
        assert_eq!(from_utf8(&output).unwrap(), "");

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        assert_eq!(account_data.lamports(), 101);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        // let account_data: AccountSharedData =
        //     storage_read_account_data(&sapi, &pk_authority).unwrap();
        // assert_eq!(account_data.lamports(), 100);
        // assert_eq!(account_data.data().len(), 0);
        // assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), 0);
        assert_eq!(account_data.data().len(), buffer_len);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &system_program_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), 17);
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &loader_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), "loader_id".len());
        assert_eq!(account_data.executable(), true);

        // fill buffer

        let create_msg = |offset: u32, bytes: Vec<u8>| {
            let instruction = loader_v4::write(&pk_exec, &pk_authority, offset, bytes);
            let instructions = vec![instruction];
            Message::new(&instructions, Some(&pk_payer))
        };
        let mut write_messages = vec![];
        let chunk_size = calculate_max_chunk_size(&create_msg);
        for (chunk, i) in account_with_program.data().chunks(chunk_size).zip(0..) {
            let offset = i * chunk_size;
            let msg = create_msg(offset as u32, chunk.to_vec());
            write_messages.push(msg);
        }
        for (_, message) in write_messages.iter().enumerate() {
            sdk = sdk.with_input(serialize(&message).unwrap());
            main_single_message(sdk.clone(), Some(&mut sapi));
        }

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        assert_eq!(account_data.lamports(), 101);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), 0);
        assert_eq!(account_data.data().len(), buffer_len);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &system_program_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), 17);
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &loader_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), "loader_id".len());
        assert_eq!(account_data.executable(), true);

        // deploy

        let instruction = loader_v4::deploy(&pk_exec, &pk_authority);
        let message = Message::new(&[instruction], Some(&pk_payer));
        sdk = sdk.with_input(serialize(&message).unwrap());
        main_single_message(sdk.clone(), Some(&mut sapi));

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        assert_eq!(account_data.lamports(), 101);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), 0);
        assert_eq!(account_data.data().len(), buffer_len);
        assert_eq!(account_data.executable(), false);

        // let account_data: AccountSharedData =
        //     storage_read_account_data(&sapi, &pk_authority).unwrap();
        // assert_eq!(account_data.lamports(), 100);
        // assert_eq!(account_data.data().len(), 0);
        // assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &system_program_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), 17);
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &loader_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), "loader_id".len());
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &sysvar_clock_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), "sysvar_clock_id".len());
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &sysvar_rent_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), "sysvar_rent_id".len());
        assert_eq!(account_data.executable(), true);

        // // exec contract command '1': simple modifications to 2nd account
        //
        // let instructions = vec![Instruction::new_with_bincode(
        //     pk_exec.clone(),
        //     &[1],
        //     vec![
        //         AccountMeta::new(pk_payer, true),
        //         AccountMeta::new(pk_tmp, false),
        //         // AccountMeta::new(system_program_id, false),
        //     ],
        // )];
        // let message = Message::new(&instructions, None);
        // sdk = sdk
        //     .with_shared_context_input(SharedContextInputV1 {
        //         block: BlockContextV1 {
        //             number: 1,
        //             ..Default::default()
        //         },
        //         ..Default::default()
        //     })
        //     .with_input(serialize(&message).unwrap());
        // let result_accounts = main_single_message(sdk.clone(), Some(&mut sapi));
        //
        // let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_tmp).unwrap();
        // assert_eq!(account_data.lamports(), 100);
        // assert_eq!(account_data.data().len(), MAX_PERMITTED_DATA_INCREASE);
        // assert_eq!(account_data.data()[0], 123);
        // assert_eq!(account_data.executable(), false);
        //
        // // let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        // // assert_eq!(account_data.lamports(), 0);
        // // assert_eq!(account_data.data().len(), buffer_len);
        // // assert_eq!(account_data.executable(), false);
        // //
        // // let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        // // assert_eq!(account_data.lamports(), 100);
        // // assert_eq!(account_data.data().len(), 0);
        // // assert_eq!(account_data.executable(), false);

        // exec contract command '2': create new account

        let mut instruction_data = Vec::<u8>::new();
        let lamports: u64 = 12;
        let space: u32 = 101;
        let seed1 = b"my_seed";
        let seed_len: u8 = seed1.len() as u8;
        let byte_n_to_set: u32 = 14;
        let byte_n_val: u8 = 33;
        instruction_data.push(2);
        instruction_data.extend_from_slice(lamports.to_le_bytes().as_slice());
        instruction_data.extend_from_slice(space.to_le_bytes().as_slice());
        instruction_data.push(seed_len);
        instruction_data.extend_from_slice(seed1);
        instruction_data.extend_from_slice(byte_n_to_set.to_le_bytes().as_slice());
        instruction_data.push(byte_n_val);

        let instructions = vec![Instruction::new_with_bincode(
            pk_exec.clone(),
            &instruction_data,
            vec![
                AccountMeta::new(pk_payer, true),
                AccountMeta::new(pk_new, false),
                AccountMeta::new(system_program_id, false),
            ],
        )];
        let message = Message::new(&instructions, None);
        sdk = sdk
            .with_shared_context_input(SharedContextInputV1 {
                block: BlockContextV1 {
                    number: 1,
                    ..Default::default()
                },
                ..Default::default()
            })
            .with_input(serialize(&message).unwrap());
        let _result_accounts = main_single_message(sdk.clone(), Some(&mut sapi));

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &pk_new).expect("account must exist in storage");
        assert_eq!(account_data.lamports(), lamports);
        assert_eq!(account_data.data().len(), space as usize);
        assert_eq!(account_data.data()[byte_n_to_set as usize], byte_n_val);
        assert_eq!(account_data.executable(), false);
    }

    #[test]
    fn test_create_fill_deploy_exec_messages_batch() {
        let mut sdk = HostTestingContext::default();
        let mut sapi = MemStorage::new();

        let system_program_id = system_program::id();
        let native_loader_id = native_loader::id();
        let loader_id = loader_v4::id();
        let sysvar_clock_id = sysvar::clock::id();
        let sysvar_rent_id = sysvar::rent::id();

        const CONTRACT_CALLER: Address = address!("1231238908230948230948209348203984029834");
        const CONTRACT_ADDRESS: Address = address!("0xF91c20C0Cafbfdc150adFf51BBfC5808EdDE7CB5");

        let pk_payer = pubkey_from_address(&CONTRACT_CALLER);
        let pk_payer_account = AccountSharedData::new(100, 0, &system_program_id);

        // let pk_tmp = Pubkey::new_unique();
        // let pk_tmp_account = AccountSharedData::new(100, 0, &system_program_id);

        let pk_exec = pubkey_from_address(&CONTRACT_ADDRESS);

        let seed1 = b"my_seed";
        let seed2 = pk_payer.as_ref();
        let seeds = &[seed1, seed2];
        let (pk_new, _bump) = Pubkey::find_program_address(seeds, &pk_exec);

        let pk_authority = pk_payer.clone();

        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "./test_elfs/out/noop_aligned.so",
            // "../../examples/svm/solana-program/assets/solana_program.so",
            // "../../examples/svm/solana-program-transfer-with-cpi/assets/solana_program.so",
            "../../examples/svm/solana-program-state-usage/assets/solana_program.so",
        );

        let program_len = account_with_program.data().len();
        let buffer_len = LoaderV4State::program_data_offset().saturating_add(program_len);

        storage_write_account_data(&mut sapi, &pk_payer, &pk_payer_account).unwrap();
        storage_write_account_data(
            &mut sapi,
            &system_program_id,
            &create_loadable_account_for_test("system_program_id", &native_loader_id),
        )
        .unwrap();
        storage_write_account_data(
            &mut sapi,
            &loader_id,
            &create_loadable_account_for_test("loader_id", &native_loader_id),
        )
        .unwrap();
        storage_write_account_data(
            &mut sapi,
            &sysvar_clock_id,
            &create_loadable_account_for_test("sysvar_clock_id", &system_program_id),
        )
        .unwrap();
        storage_write_account_data(
            &mut sapi,
            &sysvar_rent_id,
            &create_loadable_account_for_test("sysvar_rent_id", &system_program_id),
        )
        .unwrap();

        // init buffer, fill buffer, deploy

        let mut batch_message = BatchMessage::new(None);

        let instructions = create_buffer(
            &pk_payer,
            &pk_exec,
            0,
            &pk_authority,
            program_len as u32,
            &pk_payer,
        );
        let message = Message::new(&instructions, Some(&pk_payer));
        batch_message.append_one(message);

        let create_msg = |offset: u32, bytes: Vec<u8>| {
            let instruction = loader_v4::write(&pk_exec, &pk_authority, offset, bytes);
            let instructions = vec![instruction];
            Message::new(&instructions, Some(&pk_payer))
        };
        let mut write_messages = vec![];
        let chunk_size = calculate_max_chunk_size(&create_msg);
        for (chunk, i) in account_with_program.data().chunks(chunk_size).zip(0..) {
            let offset = i * chunk_size;
            let msg = create_msg(offset as u32, chunk.to_vec());
            write_messages.push(msg);
        }
        batch_message.append_many(write_messages);

        let lamports_to_transfer_on_deploy = 0;
        let instruction = loader_v4::deploy(&pk_exec, &pk_authority);
        let message = Message::new(&[instruction], Some(&pk_payer));
        batch_message.append_one(message);

        sdk = sdk.with_input(serialize(&batch_message).unwrap());
        main_batch_message(sdk.clone(), Some(&mut sapi));

        // exec
        // recreate storage to test if we need only specific accounts (other accounts dropped from storage)

        let pk_payer_account = storage_read_account_data(&mut sapi, &pk_payer).unwrap();
        let pk_exec_account = storage_read_account_data(&mut sapi, &pk_exec).unwrap();

        let mut sapi = MemStorage::new();

        storage_write_account_data(&mut sapi, &pk_payer, &pk_payer_account).unwrap();
        storage_write_account_data(&mut sapi, &pk_exec, &pk_exec_account).unwrap();
        // storage_write_account_data(&mut sapi, &pk_tmp, &pk_tmp_account).unwrap();
        storage_write_account_data(
            &mut sapi,
            &system_program_id,
            &create_loadable_account_for_test("system_program_id", &native_loader_id),
        )
        .unwrap();
        storage_write_account_data(
            &mut sapi,
            &loader_id,
            &create_loadable_account_for_test("loader_id", &native_loader_id),
        )
        .unwrap();

        let mut instruction_data = Vec::<u8>::new();
        let lamports: u64 = 12;
        let space: u32 = 101;
        let seed1 = b"my_seed";
        let seed_len: u8 = seed1.len() as u8;
        let byte_n_to_set: u32 = 14;
        let byte_n_val: u8 = 33;
        instruction_data.push(2);
        instruction_data.extend_from_slice(lamports.to_le_bytes().as_slice());
        instruction_data.extend_from_slice(space.to_le_bytes().as_slice());
        instruction_data.push(seed_len);
        instruction_data.extend_from_slice(seed1);
        instruction_data.extend_from_slice(byte_n_to_set.to_le_bytes().as_slice());
        instruction_data.push(byte_n_val);

        let instructions = vec![Instruction::new_with_bincode(
            pk_exec.clone(),
            &instruction_data,
            vec![
                // account_meta1
                AccountMeta::new(pk_payer, true),
                AccountMeta::new(pk_new, false),
                AccountMeta::new(system_program_id, false),
            ],
        )];
        let message = Message::new(&instructions, None);
        batch_message.clear().append_one(message);
        sdk = sdk
            .with_shared_context_input(SharedContextInputV1 {
                block: BlockContextV1 {
                    number: 1,
                    ..Default::default()
                },
                ..Default::default()
            })
            .with_input(serialize(&batch_message).unwrap());
        main_batch_message(sdk.clone(), Some(&mut sapi));

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), lamports_to_transfer_on_deploy);
        assert_eq!(account_data.data().len(), buffer_len);
        assert_eq!(account_data.executable(), false);
        assert_eq!(account_data.owner(), &loader_id);
        let state = get_state(account_data.data()).unwrap();
        matches!(state.status, LoaderV4Status::Deployed);
    }
}
