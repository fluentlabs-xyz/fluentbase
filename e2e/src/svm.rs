mod tests {
    use crate::utils::{try_print_utf8_error, EvmTestingContext};
    use fluentbase_sdk::{address, Address, Bytes, ContractContextV1, PRECOMPILE_SVM_RUNTIME};
    use hex_literal::hex;
    use revm::primitives::ExecutionResult;
    use solana_ee_core::{
        account::{AccountSharedData, ReadableAccount, WritableAccount},
        bincode,
        common::calculate_max_chunk_size,
        fluentbase_helpers::BatchMessage,
        helpers::{sdk_storage_read_account_data, sdk_storage_write_account_data},
        native_loader,
        native_loader::create_loadable_account_for_test,
        solana_program::{
            bpf_loader_upgradeable,
            bpf_loader_upgradeable::create_buffer,
            message::Message,
            pubkey::Pubkey,
            rent::Rent,
            system_program,
            sysvar,
        },
    };
    use std::{fs::File, io::Read};

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

    #[test]
    fn test_svm_deploy_exec() {
        let mut ctx = EvmTestingContext::default();
        const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
        ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
            address: PRECOMPILE_SVM_RUNTIME,
            ..Default::default()
        });

        // setup

        let pk_system_program = system_program::id();
        let pk_native_loader = native_loader::id();
        let pk_bpf_loader_upgradeable = bpf_loader_upgradeable::id();
        let pk_sysvar_clock = sysvar::clock::id();
        let pk_sysvar_rent = sysvar::rent::id();

        let system_program_account =
            create_loadable_account_for_test("system_program_id", &pk_native_loader);
        let bpf_loader_upgradeable_account =
            create_loadable_account_for_test("bpf_loader_upgradeable_id", &pk_native_loader);
        let sysvar_clock_account =
            create_loadable_account_for_test("sysvar_clock_id", &pk_system_program);
        let sysvar_rent_account =
            create_loadable_account_for_test("sysvar_rent_id", &pk_system_program);

        let pk_payer = Pubkey::new_unique();
        let payer_account = AccountSharedData::new(100, 0, &pk_system_program);

        let pk_buffer = Pubkey::new_unique();

        let pk_exec = Pubkey::from([8; 32]);
        let exec_account = AccountSharedData::new(0, 0, &pk_system_program);

        let pk_nine = Pubkey::from([9; 32]);
        let nine_account = AccountSharedData::new(100, 0, &pk_system_program);

        let (pk_program_data, _) =
            Pubkey::find_program_address(&[pk_exec.as_ref()], &pk_bpf_loader_upgradeable);
        let program_data_account = AccountSharedData::new(0, 0, &pk_system_program);

        let account_with_program = load_program_account_from_elf_file(
            &pk_bpf_loader_upgradeable,
            "../../solana-ee/crates/examples/hello-world/assets/solana_ee_hello_world.so",
        );

        let program_len = account_with_program.data().len();

        macro_rules! validated_stored_account {
            ($core: tt) => {
                paste::paste! {
                    let account_read = sdk_storage_read_account_data(&mut ctx.sdk, &[<pk_ $core>]).unwrap();
                    assert_eq!(&account_read, &[<$core _account>]);
                }
            };
        }

        sdk_storage_write_account_data(&mut ctx.sdk, &pk_payer, &payer_account).unwrap();
        validated_stored_account!(payer);
        sdk_storage_write_account_data(&mut ctx.sdk, &pk_nine, &nine_account).unwrap();
        validated_stored_account!(nine);
        sdk_storage_write_account_data(&mut ctx.sdk, &pk_exec, &exec_account).unwrap();
        validated_stored_account!(exec);
        sdk_storage_write_account_data(&mut ctx.sdk, &pk_program_data, &program_data_account)
            .unwrap();
        validated_stored_account!(program_data);
        sdk_storage_write_account_data(&mut ctx.sdk, &pk_system_program, &system_program_account)
            .unwrap();
        validated_stored_account!(system_program);
        sdk_storage_write_account_data(
            &mut ctx.sdk,
            &pk_bpf_loader_upgradeable,
            &bpf_loader_upgradeable_account,
        )
        .unwrap();
        validated_stored_account!(bpf_loader_upgradeable);
        sdk_storage_write_account_data(&mut ctx.sdk, &pk_sysvar_clock, &sysvar_clock_account)
            .unwrap();
        validated_stored_account!(sysvar_clock);
        sdk_storage_write_account_data(&mut ctx.sdk, &pk_sysvar_rent, &sysvar_rent_account)
            .unwrap();
        validated_stored_account!(sysvar_rent);
        ctx.commit_storage();

        // init buffer, fill buffer, deploy

        let mut batch_message = BatchMessage::new(None);

        let instructions = create_buffer(&pk_payer, &pk_buffer, &pk_nine, 0, program_len).unwrap();
        let message = Message::new(&instructions, Some(&pk_payer));
        batch_message.append_one(message);

        // let create_msg = |offset: u32, bytes: Vec<u8>| {
        //     let instruction = bpf_loader_upgradeable::write(&pk_buffer, &pk_nine, offset, bytes);
        //     let instructions = vec![instruction];
        //     Message::new(&instructions, Some(&pk_payer))
        // };
        // let mut write_messages = vec![];
        // let chunk_size = calculate_max_chunk_size(&create_msg);
        // for (chunk, i) in account_with_program.data().chunks(chunk_size).zip(0..) {
        //     let offset = i * chunk_size;
        //     let msg = create_msg(offset as u32, chunk.to_vec());
        //     write_messages.push(msg);
        // }
        // batch_message.append_many(write_messages);

        // let instructions = bpf_loader_upgradeable::deploy_with_max_program_len(
        //     &pk_payer,
        //     &pk_exec,
        //     &pk_buffer,
        //     &pk_nine,
        //     10,
        //     account_with_program.data().len(),
        // )
        // .unwrap();
        // let message = Message::new(&instructions, Some(&pk_payer));
        // batch_message.append_one(message);

        let input = bincode::serialize(&batch_message).unwrap();
        let input: Bytes = input.into();
        println!("input.len {} input '{:?}'", input.len(), input.as_ref());
        let result: ExecutionResult = ctx.call_evm_tx(
            DEPLOYER_ADDRESS,
            PRECOMPILE_SVM_RUNTIME,
            input,
            Some(300_000_000),
            None,
        );
        let output = result.output().unwrap_or_default();
        assert!(result.is_success());
        let expected_output = hex!("");
        assert_eq!(hex::encode(expected_output), hex::encode(output));
    }
}
