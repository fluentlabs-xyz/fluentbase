mod tests {
    use crate::{
        account::{AccountSharedData, ReadableAccount},
        common::{calculate_max_chunk_size, pubkey_from_address},
        fluentbase_helpers::{
            exec_encoded_svm_batch_message,
            exec_encoded_svm_message,
            process_svm_result,
            BatchMessage,
            MemStorage,
        },
        helpers::{storage_read_account_data, storage_write_account_data},
        native_loader,
        native_loader::create_loadable_account_for_test,
        test_helpers::load_program_account_from_elf_file,
    };
    use core::str::from_utf8;
    use fluentbase_sdk::{
        address,
        testing::TestingContext,
        Address,
        BlockContextV1,
        Bytes,
        ContractContextV1,
        SharedAPI,
        SharedContextInputV1,
        StorageAPI,
        U256,
    };
    use solana_program::{
        bpf_loader_upgradeable,
        bpf_loader_upgradeable::{create_buffer, UpgradeableLoaderState},
        hash::Hash,
        instruction::Instruction,
        message::Message,
        pubkey::Pubkey,
        system_program,
        sysvar,
    };

    fn main_single_message<SAPI: StorageAPI>(mut sdk: impl SharedAPI, mut sapi: Option<&mut SAPI>) {
        let input = sdk.input();

        let result = exec_encoded_svm_message::<_, SAPI>(&mut sdk, input, true, &mut sapi);
        if let Err(err) = result {
            panic!("exec svm message error: {:?}", err);
        }
        let (_output, _) = process_svm_result(result);

        let out = Bytes::new();
        sdk.write(out.as_ref());
    }

    fn main_batch_message<SAPI: StorageAPI>(mut sdk: impl SharedAPI, mut sapi: Option<&mut SAPI>) {
        let input = sdk.input();

        let result = exec_encoded_svm_batch_message::<_, SAPI>(&mut sdk, input, true, &mut sapi);
        if let Err(err) = result {
            panic!("exec svm message error: {:?}", err);
        }
        let (_output, _) = process_svm_result(result);

        let out = Bytes::new();
        sdk.write(out.as_ref());
    }

    #[test]
    fn test_create_fill_deploy_exec() {
        // setup

        let system_program_id = system_program::id();
        let native_loader_id = native_loader::id();
        let bpf_loader_upgradeable_id = bpf_loader_upgradeable::id();
        let sysvar_clock_id = sysvar::clock::id();
        let sysvar_rent_id = sysvar::rent::id();

        let pk_payer = Pubkey::new_unique();
        let account_payer = AccountSharedData::new(100, 0, &system_program_id);

        let pk_buffer = Pubkey::new_unique();

        let pk_exec = Pubkey::from([8; 32]);
        let pk_exec_account = AccountSharedData::new(0, 0, &system_program_id);

        let pk_9 = Pubkey::from([9; 32]);
        let pk_9_account = AccountSharedData::new(100, 0, &system_program_id);

        let (pk_program_data, _) =
            Pubkey::find_program_address(&[pk_exec.as_ref()], &bpf_loader_upgradeable_id);
        let pk_program_data_account = AccountSharedData::new(0, 0, &system_program_id);

        let account_with_program = load_program_account_from_elf_file(
            &bpf_loader_upgradeable_id,
            "./test_elfs/out/solana_ee_hello_world.so",
        );

        let blockhash = Hash::default();

        let program_len = account_with_program.data().len();
        let buffer_space = UpgradeableLoaderState::size_of_buffer(program_len);

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
        let mut sdk = TestingContext::default().with_shared_context_input(shared_context);
        let mut sapi = MemStorage::new();

        storage_write_account_data(&mut sapi, &pk_payer, &account_payer).unwrap();
        storage_write_account_data(&mut sapi, &pk_9, &pk_9_account).unwrap();
        storage_write_account_data(&mut sapi, &pk_exec, &pk_exec_account).unwrap();
        storage_write_account_data(&mut sapi, &pk_program_data, &pk_program_data_account).unwrap();
        storage_write_account_data(
            &mut sapi,
            &system_program_id,
            &create_loadable_account_for_test("system_program_id", &native_loader_id),
        )
        .unwrap();
        storage_write_account_data(
            &mut sapi,
            &bpf_loader_upgradeable_id,
            &create_loadable_account_for_test("bpf_loader_upgradeable_id", &native_loader_id),
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

        let instructions = create_buffer(&pk_payer, &pk_buffer, &pk_9, 0, program_len).unwrap();
        let message = Message::new_with_blockhash(&instructions, Some(&pk_payer), &blockhash);
        let mut sdk = sdk.with_input(bincode::serialize(&message).unwrap());
        main_single_message::<MemStorage>(sdk.clone(), Some(&mut sapi));
        let output = sdk.take_output();
        assert_eq!(from_utf8(&output).unwrap(), "");

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        assert_eq!(account_data.lamports(), 100);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_9).unwrap();
        assert_eq!(account_data.lamports(), 100);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_buffer).unwrap();
        assert_eq!(account_data.lamports(), 0);
        assert_eq!(account_data.data().len(), buffer_space);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &system_program_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), 17);
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &bpf_loader_upgradeable_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), 25);
        assert_eq!(account_data.executable(), true);

        // fill buffer

        let create_msg = |offset: u32, bytes: Vec<u8>| {
            let instruction = bpf_loader_upgradeable::write(&pk_buffer, &pk_9, offset, bytes);
            let instructions = vec![instruction];
            Message::new_with_blockhash(&instructions, Some(&pk_payer), &blockhash)
        };
        let mut write_messages = vec![];
        let chunk_size = calculate_max_chunk_size(&create_msg);
        for (chunk, i) in account_with_program.data().chunks(chunk_size).zip(0..) {
            let offset = i * chunk_size;
            let msg = create_msg(offset as u32, chunk.to_vec());
            write_messages.push(msg);
        }
        for (_, message) in write_messages.iter().enumerate() {
            sdk = sdk.with_input(bincode::serialize(&message).unwrap());
            main_single_message::<MemStorage>(sdk.clone(), Some(&mut sapi));
        }

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        assert_eq!(account_data.lamports(), 100);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_buffer).unwrap();
        assert_eq!(account_data.lamports(), 0);
        assert_eq!(account_data.data().len(), buffer_space);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &system_program_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), 17);
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &bpf_loader_upgradeable_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), 25);
        assert_eq!(account_data.executable(), true);

        // deploy

        let instructions = bpf_loader_upgradeable::deploy_with_max_program_len(
            &pk_payer,
            &pk_exec,
            &pk_buffer,
            &pk_9,
            10,
            account_with_program.data().len(),
        )
        .unwrap();
        let message = Message::new(&instructions, Some(&pk_payer));
        sdk = sdk.with_input(bincode::serialize(&message).unwrap());
        main_single_message::<MemStorage>(sdk.clone(), Some(&mut sapi));

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        assert_eq!(account_data.lamports(), 89);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), 10);
        assert_eq!(account_data.data().len(), 36);
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_9).unwrap();
        assert_eq!(account_data.lamports(), 100);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_buffer).unwrap();
        assert_eq!(account_data.lamports(), 0);
        assert_eq!(account_data.data().len(), 37);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &pk_program_data).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), 72805);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &system_program_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), 17);
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &bpf_loader_upgradeable_id).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), 25);
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

        let instructions = vec![Instruction::new_with_bincode(
            pk_exec.clone(),
            &[0u8; 0],
            vec![],
        )];
        let message = Message::new_with_blockhash(&instructions, Some(&pk_exec), &blockhash);
        sdk = sdk
            .with_shared_context_input(SharedContextInputV1 {
                block: BlockContextV1 {
                    number: 1,
                    ..Default::default()
                },
                ..Default::default()
            })
            .with_input(bincode::serialize(&message).unwrap());
        main_single_message::<MemStorage>(sdk.clone(), Some(&mut sapi));

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), 10);
        assert_eq!(account_data.data().len(), 36);
        assert_eq!(account_data.executable(), true);
    }

    #[test]
    fn test_create_fill_deploy_exec_messages_batch() {
        let mut sdk = TestingContext::default();
        let mut sapi = MemStorage::new();

        let system_program_id = system_program::id();
        let native_loader_id = native_loader::id();
        let bpf_loader_upgradeable_id = bpf_loader_upgradeable::id();

        const CONTRACT_CALLER: Address = address!("1231238908230948230948209348203984029834");
        const CONTRACT_ADDRESS: Address = address!("0xF91c20C0Cafbfdc150adFf51BBfC5808EdDE7CB5");

        let pk_payer = pubkey_from_address(CONTRACT_CALLER);
        let account_payer = AccountSharedData::new(1000000000, 0, &system_program_id);

        let pk_buffer = Pubkey::new_unique();

        let pk_exec = pubkey_from_address(CONTRACT_ADDRESS);

        let pk_authority = pk_payer.clone();

        let (pk_programdata, _) =
            Pubkey::find_program_address(&[pk_exec.as_ref()], &bpf_loader_upgradeable_id);

        let account_with_program = load_program_account_from_elf_file(
            &bpf_loader_upgradeable_id,
            "./test_elfs/out/solana_ee_hello_world.so",
            // "./test_elfs/out/noop_aligned.so",
        );

        let blockhash = Hash::default();

        let program_len = account_with_program.data().len();
        let programdata_len = UpgradeableLoaderState::size_of_programdata(program_len);

        storage_write_account_data(&mut sapi, &pk_payer, &account_payer).unwrap();
        storage_write_account_data(
            &mut sapi,
            &system_program_id,
            &create_loadable_account_for_test("system_program_id", &native_loader_id),
        )
        .unwrap();
        storage_write_account_data(
            &mut sapi,
            &bpf_loader_upgradeable_id,
            &create_loadable_account_for_test("bpf_loader_upgradeable_id", &native_loader_id),
        )
        .unwrap();

        // init buffer, fill buffer, deploy

        let mut batch_message = BatchMessage::new(None);

        let instructions =
            create_buffer(&pk_payer, &pk_buffer, &pk_authority, 0, program_len).unwrap();
        let message = Message::new_with_blockhash(&instructions, Some(&pk_payer), &blockhash);
        batch_message.append_one(message);

        let create_msg = |offset: u32, bytes: Vec<u8>| {
            let instruction =
                bpf_loader_upgradeable::write(&pk_buffer, &pk_authority, offset, bytes);
            let instructions = vec![instruction];
            Message::new_with_blockhash(&instructions, Some(&pk_payer), &blockhash)
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
        let instructions = bpf_loader_upgradeable::deploy_with_max_program_len(
            &pk_payer,
            &pk_exec,
            &pk_buffer,
            &pk_authority,
            lamports_to_transfer_on_deploy,
            account_with_program.data().len(),
        )
        .unwrap();
        let message = Message::new(&instructions, Some(&pk_payer));
        batch_message.append_one(message);

        sdk = sdk.with_input(bincode::serialize(&batch_message).unwrap());
        main_batch_message::<MemStorage>(sdk.clone(), Some(&mut sapi));

        // exec
        // we recreate storage to test if we need only specific accounts (other accounts dropped
        // from storage)

        let exec_account = storage_read_account_data(&mut sapi, &pk_exec).unwrap();
        let programdata_account = storage_read_account_data(&mut sapi, &pk_programdata).unwrap();

        let mut sapi = MemStorage::new();

        storage_write_account_data(&mut sapi, &pk_exec, &exec_account).unwrap();
        storage_write_account_data(&mut sapi, &pk_programdata, &programdata_account).unwrap();
        storage_write_account_data(
            &mut sapi,
            &system_program_id,
            &create_loadable_account_for_test("system_program_id", &native_loader_id),
        )
        .unwrap();
        storage_write_account_data(
            &mut sapi,
            &bpf_loader_upgradeable_id,
            &create_loadable_account_for_test("bpf_loader_upgradeable_id", &native_loader_id),
        )
        .unwrap();

        let instructions = vec![Instruction::new_with_bincode(
            pk_exec.clone(),
            &[0u8; 0],
            vec![],
        )];
        let message = Message::new_with_blockhash(&instructions, Some(&pk_exec), &blockhash);
        batch_message.clear().append_one(message);
        sdk = sdk
            .with_shared_context_input(SharedContextInputV1 {
                block: BlockContextV1 {
                    number: 1,
                    ..Default::default()
                },
                ..Default::default()
            })
            .with_input(bincode::serialize(&batch_message).unwrap());
        main_batch_message::<MemStorage>(sdk.clone(), Some(&mut sapi));

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), lamports_to_transfer_on_deploy);
        assert_eq!(account_data.data().len(), 36);
        assert_eq!(account_data.executable(), true);
        assert_eq!(account_data.owner(), &bpf_loader_upgradeable_id);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &pk_programdata).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), programdata_len);
        assert_eq!(account_data.executable(), false);
        assert_eq!(account_data.owner(), &bpf_loader_upgradeable_id);
    }
}
