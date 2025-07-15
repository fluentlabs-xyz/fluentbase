mod tests {
    use crate::EvmTestingContextWithGenesis;
    use core::str::from_utf8;
    use fluentbase_sdk::{
        address,
        Address,
        ContextReader,
        ContractContextV1,
        SharedAPI,
        PRECOMPILE_SVM_RUNTIME,
        U256,
    };
    use fluentbase_sdk_testing::EvmTestingContext;
    use fluentbase_svm::{
        account::{AccountSharedData, ReadableAccount, WritableAccount},
        common::{evm_address_from_pubkey, evm_balance_from_lamports, pubkey_from_evm_address},
        fluentbase::common::BatchMessage,
        helpers::storage_read_account_data,
        pubkey::Pubkey,
        solana_program::{
            instruction::{AccountMeta, Instruction},
            loader_v4,
            loader_v4::LoaderV4State,
            message::Message,
        },
        system_program,
    };
    use fluentbase_svm_shared::{
        bincode_helpers::serialize,
        test_structs::{CreateAccountAndModifySomeData1, SolBigModExp},
    };
    use hex_literal::hex;
    use rand::random_range;
    use std::{fs::File, io::Read, time::Instant};

    pub fn load_program_account_from_elf_file(loader_id: &Pubkey, path: &str) -> AccountSharedData {
        let mut file = File::open(path).expect("file open failed");
        let mut elf = Vec::new();
        file.read_to_end(&mut elf).unwrap();
        let mut program_account = AccountSharedData::new(0, 0, loader_id);
        program_account.set_data(elf);
        program_account.set_executable(true);
        program_account
    }

    #[test]
    fn test_svm_deploy() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        ctx.sdk.set_ownable_account_address(PRECOMPILE_SVM_RUNTIME);
        const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
        ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
            ..Default::default()
        });

        // setup

        let loader_id = loader_v4::id();

        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../contracts/examples/svm/assets/fluentbase_examples_svm_solana_program_state_usage.so",
        );

        let program_bytes = account_with_program.data().to_vec();
        ctx.add_balance(DEPLOYER_ADDRESS, U256::from(1e18));

        let measure = Instant::now();
        let (_contract_address, _gas_used) =
            ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS, program_bytes.into());
        println!("deploy took: {:.2?}", measure.elapsed());
    }

    #[test]
    fn test_svm_deploy_exec() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        ctx.sdk.set_ownable_account_address(PRECOMPILE_SVM_RUNTIME);
        assert_eq!(ctx.sdk.context().block_number(), 0);
        const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");

        // setup

        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();

        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../contracts/examples/svm/assets/fluentbase_examples_svm_solana_program_state_usage.so",
        );

        // setup initial accounts

        let payer_lamports = 101;
        let pk_payer = pubkey_from_evm_address(&DEPLOYER_ADDRESS);
        ctx.add_balance(DEPLOYER_ADDRESS, evm_balance_from_lamports(payer_lamports));

        // deploy and get exec contract

        let program_bytes = account_with_program.data().to_vec();
        let measure = Instant::now();
        let (contract_address, _gas) =
            ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS, program_bytes.into());
        println!("deploy took: {:.2?}", measure.elapsed());

        let pk_exec = pubkey_from_evm_address(&contract_address);

        let seed1 = b"seed";
        let seeds = &[seed1.as_slice(), pk_payer.as_ref()];
        let (pk_new, _bump) = Pubkey::find_program_address(seeds, &pk_exec);

        // exec

        let space: u32 = 101;

        let test_command_data = CreateAccountAndModifySomeData1 {
            lamports_to_send: 12,
            space: 101,
            seeds: vec![seed1.to_vec()],
            byte_n_to_set: random_range(0..space),
            byte_n_value: rand::random(),
        };
        let test_command =
            fluentbase_svm_shared::test_structs::TestCommand::CreateAccountAndModifySomeData1(
                test_command_data.clone(),
            );
        let instruction_data = serialize(&test_command).unwrap();
        println!(
            "instruction_data ({}): {:x?}",
            instruction_data.len(),
            &instruction_data
        );

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
        let mut batch_message = BatchMessage::new(None);
        batch_message.clear().append_one(message);
        let input = serialize(&batch_message).unwrap();
        let measure = Instant::now();
        let result =
            ctx.call_evm_tx_simple(DEPLOYER_ADDRESS, contract_address, input.into(), None, None);
        println!("exec took: {:.2?}", measure.elapsed());
        let output = result.output().unwrap();
        if output.len() > 0 {
            let out_text = from_utf8(output).unwrap();
            println!("output.len {} output '{}'", output.len(), out_text);
        }
        let output = result.output().unwrap_or_default();
        assert!(result.is_success());
        let expected_output = hex!("");
        assert_eq!(hex::encode(expected_output), hex::encode(output));

        ctx.db_storage_to_sdk();

        ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
            address: contract_address,
            ..Default::default()
        });

        let exec_account: AccountSharedData = storage_read_account_data(&ctx.sdk, &pk_exec)
            .expect(format!("failed to read exec account data: {}", pk_exec).as_str());
        assert_eq!(exec_account.lamports(), 0);
        assert_eq!(
            exec_account.data().len(),
            LoaderV4State::program_data_offset() + account_with_program.data().len()
        );
        assert_eq!(
            &exec_account.data()[LoaderV4State::program_data_offset()..],
            account_with_program.data()
        );

        let payer_account = storage_read_account_data(&ctx.sdk, &pk_payer).expect(
            format!(
                "failed to read payer {} (address:{}) account data",
                pk_payer,
                evm_address_from_pubkey::<true>(&pk_payer)
                    .expect("pk payer must be evm compatible")
            )
            .as_str(),
        );
        assert_eq!(
            payer_account.lamports(),
            payer_lamports - 1 - test_command_data.lamports_to_send
        );
        assert_eq!(payer_account.data().len(), 0);

        let new_account = storage_read_account_data(&ctx.sdk, &pk_new)
            .expect(format!("failed to read new account data: {}", pk_new).as_str());
        assert_eq!(new_account.lamports(), test_command_data.lamports_to_send);
        assert_eq!(new_account.data().len(), space as usize);
        assert_eq!(
            new_account.data()[test_command_data.byte_n_to_set as usize],
            test_command_data.byte_n_value
        );
    }

    #[test]
    fn test_svm_sol_big_mod_exp() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        ctx.sdk.set_ownable_account_address(PRECOMPILE_SVM_RUNTIME);
        assert_eq!(ctx.sdk.context().block_number(), 0);
        const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");

        // setup

        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();

        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../contracts/examples/svm/assets/fluentbase_examples_svm_solana_program_state_usage.so",
        );

        // setup initial accounts

        let payer_lamports = 101;
        let pk_payer = pubkey_from_evm_address(&DEPLOYER_ADDRESS);
        ctx.add_balance(DEPLOYER_ADDRESS, evm_balance_from_lamports(payer_lamports));

        // deploy and get exec contract

        let program_bytes = account_with_program.data().to_vec();
        let measure = Instant::now();
        let (contract_address, _gas) =
            ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS, program_bytes.into());
        println!("deploy took: {:.2?}", measure.elapsed());

        let pk_exec = pubkey_from_evm_address(&contract_address);

        let seed1 = b"seed";
        let seeds = &[seed1.as_slice(), pk_payer.as_ref()];
        let (pk_new, _bump) = Pubkey::find_program_address(seeds, &pk_exec);

        // exec

        let test_cases = [
            // base, exponent, modulus, expected
            SolBigModExp::new(
                "1111111111111111111111111111111111111111111111111111111111111111",
                "1111111111111111111111111111111111111111111111111111111111111111",
                "111111111111111111111111111111111111111111111111111111111111110A",
                "0A7074864588D6847F33A168209E516F60005A0CEC3F33AAF70E8002FE964BCD",
            ),
            SolBigModExp::new(
                "2222222222222222222222222222222222222222222222222222222222222222",
                "2222222222222222222222222222222222222222222222222222222222222222",
                "1111111111111111111111111111111111111111111111111111111111111111",
                "0000000000000000000000000000000000000000000000000000000000000000",
            ),
            SolBigModExp::new(
                "3333333333333333333333333333333333333333333333333333333333333333",
                "3333333333333333333333333333333333333333333333333333333333333333",
                "2222222222222222222222222222222222222222222222222222222222222222",
                "1111111111111111111111111111111111111111111111111111111111111111",
            ),
            SolBigModExp::new(
                "9874231472317432847923174392874918237439287492374932871937289719",
                "0948403985401232889438579475812347232099080051356165126166266222",
                "25532321a214321423124212222224222b242222222222222222222222222444",
                "220ECE1C42624E98AEE7EB86578B2FE5C4855DFFACCB43CCBB708A3AB37F184D",
            ),
            SolBigModExp::new(
                "3494396663463663636363662632666565656456646566786786676786768766",
                "2324324333246536456354655645656616169896565698987033121934984955",
                "0218305479243590485092843590249879879842313131156656565565656566",
                "012F2865E8B9E79B645FCE3A9E04156483AE1F9833F6BFCF86FCA38FC2D5BEF0",
            ),
            SolBigModExp::new(
                "0000000000000000000000000000000000000000000000000000000000000005",
                "0000000000000000000000000000000000000000000000000000000000000002",
                "0000000000000000000000000000000000000000000000000000000000000007",
                "0000000000000000000000000000000000000000000000000000000000000004",
            ),
            SolBigModExp::new(
                "0000000000000000000000000000000000000000000000000000000000000019",
                "0000000000000000000000000000000000000000000000000000000000000019",
                "0000000000000000000000000000000000000000000000000000000000000064",
                "0000000000000000000000000000000000000000000000000000000000000019",
            ),
            SolBigModExp::new(
                "0000000000000000000000000000000000000000000000000000000000000019",
                "0000000000000000000000000000000000000000000000000000000000000019",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000",
            ),
            SolBigModExp::new(
                "0000000000000000000000000000000000000000000000000000000000000019",
                "0000000000000000000000000000000000000000000000000000000000000019",
                "0000000000000000000000000000000000000000000000000000000000000001",
                "0000000000000000000000000000000000000000000000000000000000000000",
            ),
        ]
        .to_vec();

        for test_case in &test_cases {
            let test_command_data = test_case;
            let test_command = fluentbase_svm_shared::test_structs::TestCommand::SolBigModExp(
                test_command_data.clone(),
            );
            let instruction_data = serialize(&test_command).unwrap();
            println!(
                "instruction_data ({}): {:x?}",
                instruction_data.len(),
                &instruction_data
            );

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
            let mut batch_message = BatchMessage::new(None);
            batch_message.clear().append_one(message);
            let input = serialize(&batch_message).unwrap();
            let measure = Instant::now();
            let result = ctx.call_evm_tx_simple(
                DEPLOYER_ADDRESS,
                contract_address,
                input.into(),
                None,
                None,
            );
            println!("exec took: {:.2?}", measure.elapsed());
            let output = result.output().unwrap();
            if output.len() > 0 {
                let out_text = from_utf8(output).unwrap();
                println!("output.len {} output '{}'", output.len(), out_text);
            }
            let output = result.output().unwrap_or_default();
            assert!(result.is_success());
            let expected_output = hex!("");
            assert_eq!(hex::encode(expected_output), hex::encode(output));
        }
    }
}
