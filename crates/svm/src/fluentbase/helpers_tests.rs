mod tests {
    use crate::{
        account::{AccountSharedData, ReadableAccount},
        common::{calculate_max_chunk_size, pubkey_from_address},
        fluentbase::{
            common::{process_svm_result, BatchMessage, MemStorage},
            helpers::{exec_encoded_svm_batch_message, exec_encoded_svm_message},
        },
        helpers::{storage_read_account_data, storage_write_account_data},
        native_loader,
        native_loader::create_loadable_account_for_test,
        solana_program::{
            bpf_loader_upgradeable,
            bpf_loader_upgradeable::UpgradeableLoaderState,
            message::Message,
            sysvar::{clock, rent},
        },
        system_program,
        test_helpers::load_program_account_from_elf_file,
    };
    use core::str::from_utf8;
    use fluentbase_sdk::{
        address,
        Address,
        BlockContextV1,
        Bytes,
        ContractContextV1,
        SharedAPI,
        SharedContextInputV1,
        StorageAPI,
    };
    use fluentbase_sdk_testing::HostTestingContext;
    use solana_bincode::serialize;
    use solana_hash::Hash;
    use solana_instruction::Instruction;
    use solana_pubkey::Pubkey;

    fn main_single_message<SAPI: StorageAPI>(mut sdk: impl SharedAPI, mut sapi: Option<&mut SAPI>) {
        let input = sdk.input();

        let result = exec_encoded_svm_message(&mut sdk, input, true, &mut sapi);
        if let Err(err) = result {
            panic!("exec svm message error: {:?}", err);
        }
        let _ = process_svm_result(result);

        let out = Bytes::new();
        sdk.write(out.as_ref());
    }

    fn main_batch_message<SAPI: StorageAPI>(mut sdk: impl SharedAPI, mut sapi: Option<&mut SAPI>) {
        let input = sdk.input();

        let result = exec_encoded_svm_batch_message(&mut sdk, input, true, &mut sapi);
        if let Err(err) = result {
            panic!("exec svm message error: {:?}", err);
        }
        let _output = process_svm_result(result);

        let out = Bytes::new();
        sdk.write(out.as_ref());
    }

    #[test]
    fn test_create_fill_deploy_exec() {
        // setup

        let system_program_id = system_program::id();
        let native_loader_id = native_loader::id();
        let loader_id = bpf_loader_upgradeable::id();
        let sysvar_clock_id = clock::id();
        let sysvar_rent_id = rent::id();

        let pk_payer = Pubkey::new_unique();
        let pk_payer_account = AccountSharedData::new(100, 0, &system_program_id);

        let pk_buffer = Pubkey::new_unique();

        let pk_exec = Pubkey::from([8; 32]);

        let pk_authority = Pubkey::from([9; 32]);
        let pk_authority_account = AccountSharedData::new(100, 0, &system_program_id);

        let (pk_program_data, _) = Pubkey::find_program_address(&[pk_exec.as_ref()], &loader_id);

        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "./test_elfs/out/noop_aligned.so",
            "../../examples/svm/solana-program/assets/solana_program.so",
        );

        let blockhash = Hash::default();

        let program_len = account_with_program.data().len();
        let programdata_len = UpgradeableLoaderState::size_of_programdata(program_len);
        let buffer_len = UpgradeableLoaderState::size_of_buffer(program_len);

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
        storage_write_account_data(&mut sapi, &pk_authority, &pk_authority_account).unwrap();
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

        let instructions = bpf_loader_upgradeable::create_buffer(
            &pk_payer,
            &pk_buffer,
            &pk_authority,
            0,
            program_len,
        )
        .unwrap();
        let message = Message::new_with_blockhash(&instructions, Some(&pk_payer), &blockhash);
        let mut sdk = sdk.with_input(serialize(&message).unwrap());
        main_single_message(sdk.clone(), Some(&mut sapi));
        let output = sdk.take_output();
        assert_eq!(from_utf8(&output).unwrap(), "");

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        assert_eq!(account_data.lamports(), 100);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &pk_authority).unwrap();
        assert_eq!(account_data.lamports(), 100);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_buffer).unwrap();
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
        for (_, message) in write_messages.iter().enumerate() {
            sdk = sdk.with_input(serialize(&message).unwrap());
            main_single_message(sdk.clone(), Some(&mut sapi));
        }

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        assert_eq!(account_data.lamports(), 100);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_buffer).unwrap();
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

        let lamports_to_transfer_on_deploy = 10;
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
        sdk = sdk.with_input(serialize(&message).unwrap());
        main_single_message(sdk.clone(), Some(&mut sapi));

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_payer).unwrap();
        assert_eq!(account_data.lamports(), 89);
        assert_eq!(account_data.data().len(), 0);
        assert_eq!(account_data.executable(), false);

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), 10);
        assert_eq!(account_data.data().len(), 36);
        assert_eq!(account_data.executable(), true);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &pk_authority).unwrap();
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
        assert_eq!(account_data.data().len(), programdata_len);
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
            .with_input(serialize(&message).unwrap());
        main_single_message(sdk.clone(), Some(&mut sapi));

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), 10);
        assert_eq!(account_data.data().len(), 36);
        assert_eq!(account_data.executable(), true);
    }

    // #[ignore] //
    #[test]
    fn test_create_fill_deploy_exec_messages_batch() {
        let mut sdk = HostTestingContext::default();
        let mut sapi = MemStorage::new();

        let system_program_id = system_program::id();
        let native_loader_id = native_loader::id();
        let loader_id = bpf_loader_upgradeable::id();

        const CONTRACT_CALLER: Address = address!("1231238908230948230948209348203984029834");
        const CONTRACT_ADDRESS: Address = address!("0xF91c20C0Cafbfdc150adFf51BBfC5808EdDE7CB5");

        let pk_payer = pubkey_from_address(&CONTRACT_CALLER);
        let account_payer = AccountSharedData::new(1000000000, 0, &system_program_id);

        let pk_buffer = Pubkey::new_unique();

        let pk_exec = pubkey_from_address(&CONTRACT_ADDRESS);

        // let pk_authority = pk_payer.clone();
        let pk_authority = Pubkey::from([9; 32]);
        let authority_account = AccountSharedData::new(100, 0, &system_program_id);

        let (pk_programdata, _) = Pubkey::find_program_address(&[pk_exec.as_ref()], &loader_id);

        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../../examples/svm/solana-program/assets/solana_program.so",
            // "./test_elfs/out/noop_aligned.so",
        );

        let blockhash = Hash::default();

        let program_len = account_with_program.data().len();
        let programdata_len = UpgradeableLoaderState::size_of_programdata(program_len);

        storage_write_account_data(&mut sapi, &pk_payer, &account_payer).unwrap();
        storage_write_account_data(&mut sapi, &pk_authority, &authority_account).unwrap();
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

        // init buffer, fill buffer, deploy

        let mut batch_message = BatchMessage::new(None);

        let instructions = bpf_loader_upgradeable::create_buffer(
            &pk_payer,
            &pk_buffer,
            &pk_authority,
            0,
            program_len,
        )
        .unwrap();
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

        let lamports_to_transfer_on_deploy = 10;
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

        sdk = sdk.with_input(serialize(&batch_message).unwrap());
        main_batch_message(sdk.clone(), Some(&mut sapi));

        // exec
        // recreate storage to test if we need only specific accounts (other accounts dropped from storage)

        let payer_account = storage_read_account_data(&mut sapi, &pk_payer).unwrap();
        let exec_account = storage_read_account_data(&mut sapi, &pk_exec).unwrap();
        let programdata_account = storage_read_account_data(&mut sapi, &pk_programdata).unwrap();

        let mut sapi = MemStorage::new();

        storage_write_account_data(&mut sapi, &pk_payer, &payer_account).unwrap();
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
            &loader_id,
            &create_loadable_account_for_test("loader_id", &native_loader_id),
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
            .with_input(serialize(&batch_message).unwrap());
        main_batch_message(sdk.clone(), Some(&mut sapi));

        let account_data: AccountSharedData = storage_read_account_data(&sapi, &pk_exec).unwrap();
        assert_eq!(account_data.lamports(), lamports_to_transfer_on_deploy);
        assert_eq!(account_data.data().len(), 36);
        assert_eq!(account_data.executable(), true);
        assert_eq!(account_data.owner(), &loader_id);

        let account_data: AccountSharedData =
            storage_read_account_data(&sapi, &pk_programdata).unwrap();
        assert_eq!(account_data.lamports(), 1);
        assert_eq!(account_data.data().len(), programdata_len);
        assert_eq!(account_data.executable(), false);
        assert_eq!(account_data.owner(), &loader_id);
    }
}
