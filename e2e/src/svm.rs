mod tests {
    use crate::EvmTestingContextWithGenesis;
    use core::str::from_utf8;
    use curve25519_dalek::{
        constants::{ED25519_BASEPOINT_POINT, RISTRETTO_BASEPOINT_POINT},
        traits::Identity,
        EdwardsPoint,
    };
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
        test_structs::{
            Blake3,
            CreateAccountAndModifySomeData1,
            CurveGroupOp,
            CurveMultiscalarMultiplication,
            CurvePointValidation,
            Keccak256,
            SetGetReturnData,
            Sha256,
            SolBigModExp,
            SolSecp256k1Recover,
            TestCommand,
        },
    };
    use hex_literal::hex;
    use rand::random_range;
    use solana_curve25519::{
        edwards::{
            add_edwards,
            multiply_edwards,
            multiscalar_multiply_edwards,
            subtract_edwards,
            PodEdwardsPoint,
        },
        scalar::PodScalar,
    };
    use std::{fs::File, io::Read, time::Instant};

    const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");

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

    fn svm_deploy(
        ctx: &mut EvmTestingContext,
        account_with_program: &AccountSharedData,
        seed1: &[u8],
        payer_lamports: u64,
    ) -> (Pubkey, Pubkey, Pubkey, Address) {
        ctx.sdk.set_ownable_account_address(PRECOMPILE_SVM_RUNTIME);
        assert_eq!(ctx.sdk.context().block_number(), 0);

        // setup initial accounts

        let pk_payer = pubkey_from_evm_address(&DEPLOYER_ADDRESS);
        ctx.add_balance(DEPLOYER_ADDRESS, evm_balance_from_lamports(payer_lamports));

        // deploy and get exec contract

        let program_bytes = account_with_program.data().to_vec();
        let measure = Instant::now();
        let (contract_address, _gas) =
            ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS, program_bytes.into());
        println!("deploy took: {:.2?}", measure.elapsed());

        let pk_exec = pubkey_from_evm_address(&contract_address);

        let seeds = &[seed1, pk_payer.as_ref()];
        let (pk_new, _bump) = Pubkey::find_program_address(seeds, &pk_exec);

        (pk_payer, pk_exec, pk_new, contract_address)
    }

    #[test]
    fn test_svm_deploy_exec() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../contracts/examples/svm/assets/fluentbase_examples_svm_solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let space: u32 = 101;

        let test_command_data = CreateAccountAndModifySomeData1 {
            lamports_to_send: 12,
            space,
            seeds: vec![seed1.to_vec()],
            byte_n_to_set: random_range(0..space),
            byte_n_value: rand::random(),
        };
        let test_command: TestCommand = test_command_data.clone().into();
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
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../contracts/examples/svm/assets/fluentbase_examples_svm_solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let test_commands: &[TestCommand] = &[
            SolBigModExp::from_hex(
                "1111111111111111111111111111111111111111111111111111111111111111",
                "1111111111111111111111111111111111111111111111111111111111111111",
                "111111111111111111111111111111111111111111111111111111111111110A",
                "0A7074864588D6847F33A168209E516F60005A0CEC3F33AAF70E8002FE964BCD",
            )
            .into(),
            SolBigModExp::from_hex(
                "2222222222222222222222222222222222222222222222222222222222222222",
                "2222222222222222222222222222222222222222222222222222222222222222",
                "1111111111111111111111111111111111111111111111111111111111111111",
                "0000000000000000000000000000000000000000000000000000000000000000",
            )
            .into(),
            SolBigModExp::from_hex(
                "3333333333333333333333333333333333333333333333333333333333333333",
                "3333333333333333333333333333333333333333333333333333333333333333",
                "2222222222222222222222222222222222222222222222222222222222222222",
                "1111111111111111111111111111111111111111111111111111111111111111",
            )
            .into(),
            SolBigModExp::from_hex(
                "9874231472317432847923174392874918237439287492374932871937289719",
                "0948403985401232889438579475812347232099080051356165126166266222",
                "25532321a214321423124212222224222b242222222222222222222222222444",
                "220ECE1C42624E98AEE7EB86578B2FE5C4855DFFACCB43CCBB708A3AB37F184D",
            )
            .into(),
            SolBigModExp::from_hex(
                "3494396663463663636363662632666565656456646566786786676786768766",
                "2324324333246536456354655645656616169896565698987033121934984955",
                "0218305479243590485092843590249879879842313131156656565565656566",
                "012F2865E8B9E79B645FCE3A9E04156483AE1F9833F6BFCF86FCA38FC2D5BEF0",
            )
            .into(),
            SolBigModExp::from_hex(
                "0000000000000000000000000000000000000000000000000000000000000005",
                "0000000000000000000000000000000000000000000000000000000000000002",
                "0000000000000000000000000000000000000000000000000000000000000007",
                "0000000000000000000000000000000000000000000000000000000000000004",
            )
            .into(),
            SolBigModExp::from_hex(
                "0000000000000000000000000000000000000000000000000000000000000019",
                "0000000000000000000000000000000000000000000000000000000000000019",
                "0000000000000000000000000000000000000000000000000000000000000064",
                "0000000000000000000000000000000000000000000000000000000000000019",
            )
            .into(),
            SolBigModExp::from_hex(
                "0000000000000000000000000000000000000000000000000000000000000019",
                "0000000000000000000000000000000000000000000000000000000000000019",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000",
            )
            .into(),
            SolBigModExp::from_hex(
                "0000000000000000000000000000000000000000000000000000000000000019",
                "0000000000000000000000000000000000000000000000000000000000000019",
                "0000000000000000000000000000000000000000000000000000000000000001",
                "0000000000000000000000000000000000000000000000000000000000000000",
            )
            .into(),
        ];

        for test_command in test_commands {
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

    #[test]
    fn test_svm_sol_secp256k1_recover() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../contracts/examples/svm/assets/fluentbase_examples_svm_solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let test_commands: &[TestCommand] = &[SolSecp256k1Recover {
            message: b"hello world".to_vec(),
            signature_bytes: vec![
                0x93, 0x92, 0xC4, 0x6C, 0x42, 0xF6, 0x31, 0x73, 0x81, 0xD4, 0xB2, 0x44, 0xE9, 0x2F,
                0xFC, 0xE3, 0xF4, 0x57, 0xDD, 0x50, 0xB3, 0xA5, 0x20, 0x26, 0x3B, 0xE7, 0xEF, 0x8A,
                0xB0, 0x69, 0xBB, 0xDE, 0x2F, 0x90, 0x12, 0x93, 0xD7, 0x3F, 0xA0, 0x29, 0x0C, 0x46,
                0x4B, 0x97, 0xC5, 0x00, 0xAD, 0xEA, 0x6A, 0x64, 0x4D, 0xC3, 0x8D, 0x25, 0x24, 0xEF,
                0x97, 0x6D, 0xC6, 0xD7, 0x1D, 0x9F, 0x5A, 0x26,
            ],
            recovery_id: 0,
            pubkey_bytes: vec![
                0x9B, 0xEE, 0x7C, 0x18, 0x34, 0xE0, 0x18, 0x21, 0x7B, 0x40, 0x14, 0x9B, 0x84, 0x2E,
                0xFA, 0x80, 0x96, 0x00, 0x1A, 0x9B, 0x17, 0x88, 0x01, 0x80, 0xA8, 0x46, 0x99, 0x09,
                0xE9, 0xC4, 0x73, 0x6E, 0x39, 0x0B, 0x94, 0x00, 0x97, 0x68, 0xC2, 0x28, 0xB5, 0x55,
                0xD3, 0x0C, 0x0C, 0x42, 0x43, 0xC1, 0xEE, 0xA5, 0x0D, 0xC0, 0x48, 0x62, 0xD3, 0xAE,
                0xB0, 0x3D, 0xA2, 0x20, 0xAC, 0x11, 0x85, 0xEE,
            ],
        }
        .into()];

        for test_command in test_commands {
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

    #[test]
    fn test_svm_sol_keccak256() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../contracts/examples/svm/assets/fluentbase_examples_svm_solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let test_commands: &[TestCommand] = &[Keccak256 {
            data: vec![vec![1u8, 2, 3], vec![4, 5, 6]],
            expected_result: hex!(
                "13a08e3cd39a1bc7bf9103f63f83273cced2beada9f723945176d6b983c65bd2"
            ),
        }
        .into()];

        for test_command in test_commands {
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

    #[test]
    fn test_svm_sol_sha256() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../contracts/examples/svm/assets/fluentbase_examples_svm_solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let test_commands: &[TestCommand] = &[Sha256 {
            data: vec![vec![1u8, 2, 3], vec![4, 5, 6]],
            expected_result: hex!(
                "7192385c3c0605de55bb9476ce1d90748190ecb32a8eed7f5207b30cf6a1fe89"
            ),
        }
        .into()];

        for test_command in test_commands {
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

    #[test]
    fn test_svm_sol_blake3() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../contracts/examples/svm/assets/fluentbase_examples_svm_solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let test_commands: &[TestCommand] = &[Blake3 {
            data: vec![vec![1u8, 2, 3], vec![4, 5, 6]],
            expected_result: hex!(
                "828a8660ae86b86f1ebf951a6f84349520cc1501fb6fcf95b05df01200be9fa2"
            ),
        }
        .into()];

        for test_command in test_commands {
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

    #[test]
    fn test_svm_sol_return_data() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../contracts/examples/svm/assets/fluentbase_examples_svm_solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let test_commands: &[TestCommand] = &[SetGetReturnData {
            data: vec![9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
        }
        .into()];

        for test_command in test_commands {
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
            println!("exec started");
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

    #[test]
    fn test_svm_sol_curve_validate_point() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../contracts/examples/svm/assets/fluentbase_examples_svm_solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let test_commands: &[TestCommand] = &[
            CurvePointValidation {
                curve_id: 0,
                point: ED25519_BASEPOINT_POINT.compress().as_bytes().clone(),
                expected_ret: 0, // OK
            }
            .into(),
            CurvePointValidation {
                curve_id: 0,
                point: [
                    120, 140, 152, 233, 41, 227, 203, 27, 87, 115, 25, 251, 219, 5, 84, 148, 117,
                    38, 84, 60, 87, 144, 161, 146, 42, 34, 91, 155, 158, 189, 121, 79,
                ],
                expected_ret: 1, // ERR
            }
            .into(),
            CurvePointValidation {
                curve_id: 0,
                point: RISTRETTO_BASEPOINT_POINT.compress().as_bytes().clone(),
                expected_ret: 0, // OK
            }
            .into(),
            CurvePointValidation {
                curve_id: 0,
                point: [
                    120, 140, 152, 233, 41, 227, 203, 27, 87, 115, 25, 251, 219, 5, 84, 148, 117,
                    38, 84, 60, 87, 144, 161, 146, 42, 34, 91, 155, 158, 189, 121, 79,
                ],
                expected_ret: 1, // ERR
            }
            .into(),
        ];

        for test_command in test_commands {
            let test_command: TestCommand = test_command.clone().into();
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
            println!("exec started");
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
            assert!(&result.is_success());
            let expected_output = hex!("");
            assert_eq!(hex::encode(expected_output), hex::encode(output));
        }
    }

    #[test]
    fn test_svm_sol_curve_group_op() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../contracts/examples/svm/assets/fluentbase_examples_svm_solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands: Vec<TestCommand> = vec![];

        // identity cases
        let identity = PodEdwardsPoint(EdwardsPoint::identity().compress().to_bytes());
        let point = PodEdwardsPoint([
            201, 179, 241, 122, 180, 185, 239, 50, 183, 52, 221, 0, 153, 195, 43, 18, 22, 38, 187,
            206, 179, 192, 210, 58, 53, 45, 150, 98, 89, 17, 158, 11,
        ]);
        assert_eq!(add_edwards(&point, &identity).unwrap(), point);
        assert_eq!(subtract_edwards(&point, &identity).unwrap(), point);
        test_commands.push(
            CurveGroupOp {
                curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
                group_op: solana_curve25519::curve_syscall_traits::ADD,
                left_input: point.0,
                right_input: identity.0,
                expected_point: point.0,
                expected_ret: 0, // OK
            }
            .into(),
        );
        test_commands.push(
            CurveGroupOp {
                curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
                group_op: solana_curve25519::curve_syscall_traits::SUB,
                left_input: point.0,
                right_input: identity.0,
                expected_point: point.0,
                expected_ret: 0, // OK
            }
            .into(),
        );

        // associativity cases
        let point_a = PodEdwardsPoint([
            33, 124, 71, 170, 117, 69, 151, 247, 59, 12, 95, 125, 133, 166, 64, 5, 2, 27, 90, 27,
            200, 167, 59, 164, 52, 54, 52, 200, 29, 13, 34, 213,
        ]);
        let point_b = PodEdwardsPoint([
            70, 222, 137, 221, 253, 204, 71, 51, 78, 8, 124, 1, 67, 200, 102, 225, 122, 228, 111,
            183, 129, 14, 131, 210, 212, 95, 109, 246, 55, 10, 159, 91,
        ]);
        let point_c = PodEdwardsPoint([
            72, 60, 66, 143, 59, 197, 111, 36, 181, 137, 25, 97, 157, 201, 247, 215, 123, 83, 220,
            250, 154, 150, 180, 192, 196, 28, 215, 137, 34, 247, 39, 129,
        ]);
        assert_eq!(
            add_edwards(&add_edwards(&point_a, &point_b).unwrap(), &point_c),
            add_edwards(&point_a, &add_edwards(&point_b, &point_c).unwrap()),
        );
        test_commands.push(
            CurveGroupOp {
                // a + b
                curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
                group_op: solana_curve25519::curve_syscall_traits::ADD,
                left_input: point_a.0,
                right_input: point_b.0,
                expected_point: add_edwards(&point_a, &point_b).unwrap().0,
                expected_ret: 0, // OK
            }
            .into(),
        );
        test_commands.push(
            CurveGroupOp {
                // (a + b) + c
                curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
                group_op: solana_curve25519::curve_syscall_traits::ADD,
                left_input: add_edwards(&point_a, &point_b).unwrap().0,
                right_input: point_c.0,
                expected_point: add_edwards(&add_edwards(&point_a, &point_b).unwrap(), &point_c)
                    .unwrap()
                    .0,
                expected_ret: 0, // OK
            }
            .into(),
        );
        test_commands.push(
            CurveGroupOp {
                // b + c
                curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
                group_op: solana_curve25519::curve_syscall_traits::ADD,
                left_input: point_b.0,
                right_input: point_c.0,
                expected_point: add_edwards(&point_b, &point_c).unwrap().0,
                expected_ret: 0, // OK
            }
            .into(),
        );
        test_commands.push(
            CurveGroupOp {
                // a + (b + c)
                curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
                group_op: solana_curve25519::curve_syscall_traits::ADD,
                left_input: point_a.0,
                right_input: add_edwards(&point_b, &point_c).unwrap().0,
                expected_point: add_edwards(&point_a, &add_edwards(&point_b, &point_c).unwrap())
                    .unwrap()
                    .0,
                expected_ret: 0, // OK
            }
            .into(),
        );
        test_commands.push(
            CurveGroupOp {
                // (a + b) + c = a + (b + c)
                curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
                group_op: solana_curve25519::curve_syscall_traits::ADD,
                left_input: add_edwards(&point_a, &point_b).unwrap().0,
                right_input: point_c.0,
                expected_point: add_edwards(&point_a, &add_edwards(&point_b, &point_c).unwrap())
                    .unwrap()
                    .0,
                expected_ret: 0, // OK
            }
            .into(),
        );

        // commutativity
        assert_eq!(
            add_edwards(&point_a, &point_b).unwrap(),
            add_edwards(&point_b, &point_a).unwrap(),
        );
        test_commands.push(
            CurveGroupOp {
                // a + b = b + a
                curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
                group_op: solana_curve25519::curve_syscall_traits::ADD,
                left_input: point_a.0,
                right_input: point_b.0,
                expected_point: add_edwards(&point_b, &point_a).unwrap().0,
                expected_ret: 0, // OK
            }
            .into(),
        );
        test_commands.push(
            CurveGroupOp {
                // b + a = a + b
                curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
                group_op: solana_curve25519::curve_syscall_traits::ADD,
                left_input: point_b.0,
                right_input: point_a.0,
                expected_point: add_edwards(&point_a, &point_b).unwrap().0,
                expected_ret: 0, // OK
            }
            .into(),
        );

        // subtraction
        let point = PodEdwardsPoint(ED25519_BASEPOINT_POINT.compress().to_bytes());
        let point_negated = PodEdwardsPoint((-ED25519_BASEPOINT_POINT).compress().to_bytes());
        assert_eq!(point_negated, subtract_edwards(&identity, &point).unwrap(),);
        test_commands.push(
            CurveGroupOp {
                curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
                group_op: solana_curve25519::curve_syscall_traits::SUB,
                left_input: identity.0,
                right_input: point.0,
                expected_point: point_negated.0,
                expected_ret: 0, // OK
            }
            .into(),
        );

        for test_command in &test_commands {
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
            println!("exec started");
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
            assert!(&result.is_success());
            let expected_output = hex!("");
            assert_eq!(hex::encode(expected_output), hex::encode(output));
        }
    }

    #[test]
    fn test_sol_curve_multiscalar_mul() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../contracts/examples/svm/assets/fluentbase_examples_svm_solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands: Vec<TestCommand> = vec![];

        let scalar = PodScalar([
            205, 73, 127, 173, 83, 80, 190, 66, 202, 3, 237, 77, 52, 223, 238, 70, 80, 242, 24, 87,
            111, 84, 49, 63, 194, 76, 202, 108, 62, 240, 83, 15,
        ]);
        let point = PodEdwardsPoint([
            222, 174, 184, 139, 143, 122, 253, 96, 0, 207, 120, 157, 112, 38, 54, 189, 91, 144, 78,
            111, 111, 122, 140, 183, 65, 250, 191, 133, 6, 42, 212, 93,
        ]);
        let basic_product = multiply_edwards(&scalar, &point).unwrap();
        let msm_product = multiscalar_multiply_edwards(&[scalar], &[point]).unwrap();
        assert_eq!(basic_product, msm_product);
        test_commands.push(
            CurveMultiscalarMultiplication {
                curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
                scalars: vec![scalar.0],
                points: vec![point.0],
                expected_point: basic_product.0,
                expected_ret: 0, // OK
            }
            .into(),
        );

        let scalar_a = PodScalar([
            246, 154, 34, 110, 31, 185, 50, 1, 252, 194, 163, 56, 211, 18, 101, 192, 57, 225, 207,
            69, 19, 84, 231, 118, 137, 175, 148, 218, 106, 212, 69, 9,
        ]);
        let scalar_b = PodScalar([
            27, 58, 126, 136, 253, 178, 176, 245, 246, 55, 15, 202, 35, 183, 66, 199, 134, 187,
            169, 154, 66, 120, 169, 193, 75, 4, 33, 241, 126, 227, 59, 3,
        ]);
        let point_x = PodEdwardsPoint([
            252, 31, 230, 46, 173, 95, 144, 148, 158, 157, 63, 10, 8, 68, 58, 176, 142, 192, 168,
            53, 61, 105, 194, 166, 43, 56, 246, 236, 28, 146, 114, 133,
        ]);
        let point_y = PodEdwardsPoint([
            10, 111, 8, 236, 97, 189, 124, 69, 89, 176, 222, 39, 199, 253, 111, 11, 248, 186, 128,
            90, 120, 128, 248, 210, 232, 183, 93, 104, 111, 150, 7, 241,
        ]);
        let ax = multiply_edwards(&scalar_a, &point_x).unwrap();
        let by = multiply_edwards(&scalar_b, &point_y).unwrap();
        let basic_product = add_edwards(&ax, &by).unwrap();
        let msm_product =
            multiscalar_multiply_edwards(&[scalar_a, scalar_b], &[point_x, point_y]).unwrap();
        assert_eq!(basic_product, msm_product);
        test_commands.push(
            CurveMultiscalarMultiplication {
                curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
                scalars: vec![scalar_a.0, scalar_b.0],
                points: vec![point_x.0, point_y.0],
                expected_point: basic_product.0,
                expected_ret: 0, // OK
            }
            .into(),
        );

        for test_command in &test_commands {
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
            println!("exec started");
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
            assert!(&result.is_success());
            let expected_output = hex!("");
            assert_eq!(hex::encode(expected_output), hex::encode(output));
        }
    }
}
