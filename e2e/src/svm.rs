mod tests {
    use crate::EvmTestingContextWithGenesis;
    use ark_serialize::{CanonicalDeserialize, CanonicalSerialize, Compress, Validate};
    use core::str::from_utf8;
    use curve25519_dalek::{
        constants::{ED25519_BASEPOINT_POINT, RISTRETTO_BASEPOINT_POINT},
        EdwardsPoint, RistrettoPoint,
    };
    use fluentbase_runtime::instruction::weierstrass_compress_decompress::{
        ConfigG1Compress, ConfigG1Decompress, ConfigG2Compress, ConfigG2Decompress,
        SyscallWeierstrassCompressDecompressAssign,
    };
    use fluentbase_sdk::{
        address, Address, ContextReader, ContractContextV1, SharedAPI, PRECOMPILE_SVM_RUNTIME, U256,
    };
    use fluentbase_sdk_testing::EvmTestingContext;
    use fluentbase_svm::helpers::storage_write_metadata;
    use fluentbase_svm::{
        account::{AccountSharedData, ReadableAccount, WritableAccount},
        common::{evm_balance_from_lamports, pubkey_from_evm_address},
        fluentbase::common::BatchMessage,
        helpers::storage_read_account_data,
        pubkey::Pubkey,
        solana_program::{
            instruction::{AccountMeta, Instruction},
            loader_v4,
            loader_v4::LoaderV4State,
            message::Message,
        },
        spl_token_2022, system_program,
    };
    use fluentbase_svm_shared::test_structs::{EvmCall, Transfer};
    use fluentbase_svm_shared::{
        bincode_helpers::serialize,
        test_structs::{
            AltBn128Compression, Blake3, CreateAccountAndModifySomeData1, CurveGroupOp,
            CurveMultiscalarMultiplication, CurvePointValidation, Keccak256, Poseidon,
            SetGetReturnData, Sha256, SolBigModExp, SolSecp256k1Recover, SyscallAltBn128,
            TestCommand, EXPECTED_RET_ERR, EXPECTED_RET_OK,
        },
    };
    use fluentbase_types::{
        helpers::convert_endianness_fixed, BN254_G1_POINT_COMPRESSED_SIZE,
        BN254_G1_POINT_DECOMPRESSED_SIZE, BN254_G2_POINT_COMPRESSED_SIZE,
        BN254_G2_POINT_DECOMPRESSED_SIZE, PRECOMPILE_ERC20_RUNTIME, PRECOMPILE_SHA256,
    };
    use hex_literal::hex;
    use rand::random_range;
    use serde::Deserialize;
    use sha2::Digest;
    use solana_bn254::{
        compression::prelude::{
            alt_bn128_g1_compress, alt_bn128_g1_decompress, alt_bn128_g2_compress,
            alt_bn128_g2_decompress, ALT_BN128_G1_COMPRESS, ALT_BN128_G1_DECOMPRESS,
            ALT_BN128_G2_COMPRESS, ALT_BN128_G2_DECOMPRESS,
        },
        prelude::{alt_bn128_addition, ALT_BN128_ADD, ALT_BN128_MUL, ALT_BN128_PAIRING},
        target_arch::{alt_bn128_multiplication, alt_bn128_pairing},
    };
    use solana_curve25519::{
        edwards::{
            add_edwards, multiply_edwards, multiscalar_multiply_edwards, subtract_edwards,
            PodEdwardsPoint,
        },
        ristretto::{
            add_ristretto, multiply_ristretto, multiscalar_multiply_ristretto, subtract_ristretto,
            PodRistrettoPoint,
        },
        scalar::PodScalar,
    };
    use solana_poseidon::{Endianness, Parameters};
    use std::{fs::File, io::Read, ops::Neg, time::Instant};

    const DEPLOYER_ADDRESS1: Address = address!("1231238908230948230948209348203984029834");
    const DEPLOYER_ADDRESS2: Address = address!("1231238928230949230948209148203584029234");

    pub fn process_test_commands(
        ctx: &mut EvmTestingContext,
        contract_address: &Address,
        pk_exec: &Pubkey,
        pk_payer: &Pubkey,
        pk_new: &Pubkey,
        system_program_id: &Pubkey,
        test_commands: &[TestCommand],
    ) {
        for test_command in test_commands {
            let instruction_data = serialize(&test_command).unwrap();

            let instructions = vec![Instruction::new_with_bincode(
                pk_exec.clone(),
                &instruction_data,
                vec![
                    AccountMeta::new(pk_payer.clone(), true),
                    AccountMeta::new(pk_new.clone(), false),
                    AccountMeta::new(system_program_id.clone(), false),
                ],
            )];
            let message = Message::new(&instructions, None);
            let mut batch_message = BatchMessage::new(None);
            batch_message.clear().append_one(message);
            let input = serialize(&batch_message).unwrap();
            println!("exec started");
            let measure = Instant::now();
            let result = ctx.call_evm_tx_simple(
                DEPLOYER_ADDRESS1,
                contract_address.clone(),
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
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );

        let program_bytes = account_with_program.data().to_vec();
        ctx.add_balance(DEPLOYER_ADDRESS1, U256::from(1e18));

        let measure = Instant::now();
        let (_contract_address, _gas_used) =
            ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS1, program_bytes.into());
        println!("deploy took: {:.2?}", measure.elapsed());
    }

    fn svm_deploy(
        ctx: &mut EvmTestingContext,
        account_with_program: &AccountSharedData,
        seed1: &[u8],
        payer_initial_lamports: u64,
    ) -> (Pubkey, Pubkey, Pubkey, Address) {
        ctx.sdk.set_ownable_account_address(PRECOMPILE_SVM_RUNTIME);
        assert_eq!(ctx.sdk.context().block_number(), 0);

        // setup initial accounts

        let pk_deployer1 = pubkey_from_evm_address(&DEPLOYER_ADDRESS1);
        ctx.add_balance(
            DEPLOYER_ADDRESS1,
            evm_balance_from_lamports(payer_initial_lamports),
        );

        // deploy and get exec contract

        let program_bytes = account_with_program.data().to_vec();
        let measure = Instant::now();
        let (contract_address, _gas) =
            ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS1, program_bytes.into());
        println!("deploy took: {:.2?}", measure.elapsed());

        let pk_contract = pubkey_from_evm_address(&contract_address);

        let seeds = &[seed1, pk_deployer1.as_ref()];
        let (pk_new, _bump) = Pubkey::find_program_address(seeds, &pk_contract);

        (pk_deployer1, pk_contract, pk_new, contract_address)
    }

    #[test]
    fn test_svm_deploy_exec() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_initial_lamports = 101;
        let seed1 = b"seed";

        let (pk_deployer1, pk_contract, pk_new, contract_address) = svm_deploy(
            &mut ctx,
            &account_with_program,
            seed1,
            payer_initial_lamports,
        );
        let pk_deployer2 = pubkey_from_evm_address(&DEPLOYER_ADDRESS2);
        // some balance for gas payment
        ctx.add_balance(DEPLOYER_ADDRESS2, evm_balance_from_lamports(1));

        ctx.commit_db_to_sdk();

        // exec

        let space: u32 = 99;
        let mut deployer1_lamports = 30;
        let deployer1_lamports_to_send = 6;

        let mut new_account_lamports = deployer1_lamports_to_send;
        let test_command_data = CreateAccountAndModifySomeData1 {
            lamports_to_send: deployer1_lamports_to_send,
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
            pk_contract.clone(),
            &instruction_data,
            vec![
                AccountMeta::new(pk_deployer1, true),
                AccountMeta::new(pk_new, false),
                AccountMeta::new(system_program_id, false),
            ],
        )];
        let message = Message::new(&instructions, None);
        let mut batch_message = BatchMessage::new(None);
        batch_message.clear().append_one(message);
        let input = serialize(&batch_message).unwrap();
        let result = ctx.call_evm_tx_simple(
            DEPLOYER_ADDRESS1,
            contract_address,
            input.clone().into(),
            None,
            Some(evm_balance_from_lamports(deployer1_lamports)),
        );
        deployer1_lamports -= deployer1_lamports_to_send;
        let output = result.output().unwrap();
        if output.len() > 0 {
            let out_text = from_utf8(output).unwrap();
            println!("output.len {} output '{}'", output.len(), out_text);
        }
        assert!(result.is_success());

        ctx.commit_db_to_sdk();

        let output = result.output().unwrap_or_default();
        let expected_output = hex!("");
        assert_eq!(hex::encode(expected_output), hex::encode(output));

        let contract_account = storage_read_account_data(&ctx.sdk, &pk_contract)
            .expect(format!("failed to read exec account data: {}", pk_contract).as_str());
        assert_eq!(contract_account.lamports(), 0);
        assert_eq!(
            contract_account.data().len(),
            LoaderV4State::program_data_offset() + account_with_program.data().len()
        );
        assert_eq!(
            &contract_account.data()[LoaderV4State::program_data_offset()..],
            account_with_program.data()
        );

        let deployer1_account = storage_read_account_data(&ctx.sdk, &pk_deployer1)
            .expect("failed to read payer account data");
        assert_eq!(deployer1_lamports, deployer1_account.lamports(),);
        assert_eq!(deployer1_account.data().len(), 0);

        let new_account = storage_read_account_data(&ctx.sdk, &pk_new)
            .expect(format!("failed to read new account data: {}", pk_new).as_str());
        assert_eq!(test_command_data.lamports_to_send, new_account.lamports());
        assert_eq!(new_account.data().len(), space as usize);
        assert_eq!(
            new_account.data()[test_command_data.byte_n_to_set as usize],
            test_command_data.byte_n_value
        );

        // create the same new account (pk_new) inside solana app - must fail

        let test_command_data = CreateAccountAndModifySomeData1 {
            lamports_to_send: deployer1_lamports_to_send,
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
            pk_contract.clone(),
            &instruction_data,
            vec![
                AccountMeta::new(pk_deployer1, true),
                AccountMeta::new(pk_new, false),
                AccountMeta::new(system_program_id, false),
            ],
        )];
        let message = Message::new(&instructions, None);
        let mut batch_message = BatchMessage::new(None);
        batch_message.clear().append_one(message);
        let input = serialize(&batch_message).unwrap();
        let result = ctx.call_evm_tx_simple(
            DEPLOYER_ADDRESS1,
            contract_address,
            input.clone().into(),
            None,
            None,
        );
        assert!(!result.is_success());

        ctx.commit_db_to_sdk();

        // transfer lamports to the previously created account (pk_new)

        let deployer1_lamports_to_send = deployer1_lamports_to_send - 1;
        deployer1_lamports -= deployer1_lamports_to_send;
        new_account_lamports += deployer1_lamports_to_send;
        let test_command_data = Transfer {
            lamports: deployer1_lamports_to_send,
            seeds: vec![seed1.to_vec()],
        };
        let test_command: TestCommand = test_command_data.clone().into();
        let instruction_data = serialize(&test_command).unwrap();
        println!(
            "instruction_data ({}): {:x?}",
            instruction_data.len(),
            &instruction_data
        );
        let instructions = vec![Instruction::new_with_bincode(
            pk_contract.clone(),
            &instruction_data,
            vec![
                AccountMeta::new(pk_deployer1, true),
                AccountMeta::new(pk_new, false),
                AccountMeta::new(system_program_id, false),
            ],
        )];
        let message = Message::new(&instructions, None);
        let mut batch_message = BatchMessage::new(None);
        batch_message.clear().append_one(message);
        let input = serialize(&batch_message).unwrap();
        let result = ctx.call_evm_tx_simple(
            DEPLOYER_ADDRESS1,
            contract_address,
            input.clone().into(),
            None,
            None,
        );
        assert!(result.is_success());

        ctx.commit_db_to_sdk();

        let new_account = storage_read_account_data(&ctx.sdk, &pk_new)
            .expect(format!("failed to read new account data: {}", pk_new).as_str());
        assert_eq!(new_account_lamports, new_account.lamports());
        assert_eq!(new_account.data().len(), space as usize);

        let deployer1_account = storage_read_account_data(&ctx.sdk, &pk_deployer1)
            .expect("failed to read payer account data");
        assert_eq!(deployer1_lamports, deployer1_account.lamports(),);
        assert_eq!(deployer1_account.data().len(), 0);

        // transfer lamports DEPLOYER_ADDRESS1 -> DEPLOYER_ADDRESS2

        let deployer1_lamports_to_send = deployer1_lamports_to_send - 1;
        deployer1_lamports -= deployer1_lamports_to_send;
        let mut deployer2_lamports = deployer1_lamports_to_send;
        let test_command_data = Transfer {
            lamports: deployer1_lamports_to_send,
            seeds: vec![seed1.to_vec()],
        };
        let test_command: TestCommand = test_command_data.clone().into();
        let instruction_data = serialize(&test_command).unwrap();
        println!(
            "instruction_data ({}): {:x?}",
            instruction_data.len(),
            &instruction_data
        );
        let instructions = vec![Instruction::new_with_bincode(
            pk_contract.clone(),
            &instruction_data,
            vec![
                AccountMeta::new(pk_deployer1, true),
                AccountMeta::new(pk_deployer2, false),
                AccountMeta::new(system_program_id, false),
            ],
        )];
        let message = Message::new(&instructions, None);
        let mut batch_message = BatchMessage::new(None);
        batch_message.clear().append_one(message);
        let input = serialize(&batch_message).unwrap();
        let result = ctx.call_evm_tx_simple(
            DEPLOYER_ADDRESS1,
            contract_address,
            input.clone().into(),
            None,
            None,
        );
        assert!(result.is_success());

        ctx.commit_db_to_sdk();

        let deployer2_account = storage_read_account_data(&ctx.sdk, &pk_deployer2)
            .expect(format!("failed to read new account data: {}", pk_deployer2).as_str());
        assert_eq!(deployer2_lamports, deployer2_account.lamports());
        assert_eq!(deployer2_account.data().len(), 0);

        let deployer1_account = storage_read_account_data(&ctx.sdk, &pk_deployer1)
            .expect("failed to read payer account data");
        assert_eq!(deployer1_lamports, deployer1_account.lamports(),);
        assert_eq!(deployer1_account.data().len(), 0);

        // transfer lamports DEPLOYER_ADDRESS2 -> DEPLOYER_ADDRESS1

        let deployer2_lamports_to_send = deployer1_lamports_to_send - 1;
        deployer1_lamports += deployer2_lamports_to_send;
        deployer2_lamports -= deployer2_lamports_to_send;
        let test_command_data = Transfer {
            lamports: deployer2_lamports_to_send,
            seeds: vec![seed1.to_vec()],
        };
        let test_command: TestCommand = test_command_data.clone().into();
        let instruction_data = serialize(&test_command).unwrap();
        println!(
            "instruction_data ({}): {:x?}",
            instruction_data.len(),
            &instruction_data
        );
        let instructions = vec![Instruction::new_with_bincode(
            pk_contract.clone(),
            &instruction_data,
            vec![
                AccountMeta::new(pk_deployer2, true),
                AccountMeta::new(pk_deployer1, false),
                AccountMeta::new(system_program_id, false),
            ],
        )];
        let message = Message::new(&instructions, None);
        let mut batch_message = BatchMessage::new(None);
        batch_message.clear().append_one(message);
        let input = serialize(&batch_message).unwrap();
        let result = ctx.call_evm_tx_simple(
            DEPLOYER_ADDRESS2,
            contract_address,
            input.clone().into(),
            None,
            None,
        );
        assert!(result.is_success());

        ctx.commit_db_to_sdk();

        let deployer1_account = storage_read_account_data(&ctx.sdk, &pk_deployer1)
            .expect("failed to read payer account data");
        assert_eq!(deployer1_lamports, deployer1_account.lamports());
        assert_eq!(deployer1_account.data().len(), 0);

        let deployer2_account = storage_read_account_data(&ctx.sdk, &pk_deployer2)
            .expect(format!("failed to read new account data: {}", pk_deployer2).as_str());
        assert_eq!(deployer2_lamports, deployer2_account.lamports());
        assert_eq!(deployer2_account.data().len(), 0);
    }

    #[test]
    fn test_svm_deploy_exec_cross_call_evm_sha256() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_initial_lamports = 101;
        let seed1 = b"seed";

        let (pk_deployer1, pk_contract, _pk_new, contract_address) = svm_deploy(
            &mut ctx,
            &account_with_program,
            seed1,
            payer_initial_lamports,
        );

        ctx.commit_db_to_sdk();

        // exec

        let deployer1_lamports = 0;

        let address = PRECOMPILE_SHA256;
        let value: U256 = U256::from(0);
        let gas_limit: u64 = u64::MAX;
        let call_data: Vec<u8> = vec![1, 2, 3];
        let call_data_sha256_vec = sha2::Sha256::digest(call_data.as_slice()).to_vec();
        let test_command_data = EvmCall {
            address: address.0 .0,
            value: value.to_le_bytes(),
            gas_limit,
            data: call_data,
            result_data_expected: call_data_sha256_vec,
        };
        let test_command: TestCommand = test_command_data.clone().into();
        let instruction_data = serialize(&test_command).unwrap();
        println!(
            "instruction_data ({}): {:x?}",
            instruction_data.len(),
            &instruction_data
        );

        let instructions = vec![Instruction::new_with_bincode(
            pk_contract.clone(),
            &instruction_data,
            vec![
                AccountMeta::new(pk_deployer1, true),
                AccountMeta::new(system_program_id, false),
            ],
        )];
        let message = Message::new(&instructions, None);
        let mut batch_message = BatchMessage::new(None);
        batch_message.clear().append_one(message);
        let input = serialize(&batch_message).unwrap();
        let deployer1_balance_before = ctx.get_balance(DEPLOYER_ADDRESS1);
        let result = ctx.call_evm_tx_simple(
            DEPLOYER_ADDRESS1,
            contract_address,
            input.clone().into(),
            None,
            None,
        );
        let deployer1_balance_after = ctx.get_balance(DEPLOYER_ADDRESS1);
        let deployer1_balance_spent = deployer1_balance_before - deployer1_balance_after;
        assert_eq!(U256::from(27320), deployer1_balance_spent);
        let output = result.output().unwrap();
        if output.len() > 0 {
            let out_text = from_utf8(output).unwrap();
            println!("output.len {} output '{}'", output.len(), out_text);
        }
        assert!(result.is_success());

        ctx.commit_db_to_sdk();

        let output = result.output().unwrap_or_default();
        let expected_output = hex!("");
        assert_eq!(hex::encode(expected_output), hex::encode(output));

        let contract_account = storage_read_account_data(&ctx.sdk, &pk_contract)
            .expect(format!("failed to read exec account data: {}", pk_contract).as_str());
        assert_eq!(contract_account.lamports(), 0);
        assert_eq!(
            contract_account.data().len(),
            LoaderV4State::program_data_offset() + account_with_program.data().len()
        );
        assert_eq!(
            &contract_account.data()[LoaderV4State::program_data_offset()..],
            account_with_program.data()
        );

        let deployer1_account = storage_read_account_data(&ctx.sdk, &pk_deployer1)
            .expect("failed to read payer account data");
        assert_eq!(deployer1_lamports, deployer1_account.lamports());
        assert_eq!(deployer1_account.data().len(), 0);
    }

    #[ignore]
    #[test]
    fn test_svm_deploy_exec_cross_call_evm_erc20_shared() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_initial_lamports = 101;
        let seed1 = b"seed";

        let (pk_deployer1, pk_contract, _pk_new, contract_address) = svm_deploy(
            &mut ctx,
            &account_with_program,
            seed1,
            payer_initial_lamports,
        );

        ctx.commit_db_to_sdk();

        // exec

        let deployer1_lamports = 0;

        let address = PRECOMPILE_ERC20_RUNTIME;
        let value: U256 = U256::from(0);
        let gas_limit: u64 = u64::MAX;
        let call_data: Vec<u8> = vec![1, 2, 3];
        let call_data_sha256_vec = sha2::Sha256::digest(call_data.as_slice()).to_vec();
        // TODO do not use EvmCall, we must make pure svm invoke()
        let test_command_data = EvmCall {
            address: address.0 .0,
            value: value.to_le_bytes(),
            gas_limit,
            data: call_data,
            result_data_expected: call_data_sha256_vec,
        };
        let test_command: TestCommand = test_command_data.clone().into();
        let instruction_data = serialize(&test_command).unwrap();
        println!(
            "instruction_data ({}): {:x?}",
            instruction_data.len(),
            &instruction_data
        );

        storage_write_metadata(&mut ctx.sdk, &spl_token_2022::id(), vec![1, 2, 3].into()).unwrap();

        let instructions = vec![Instruction::new_with_bincode(
            pk_contract.clone(),
            &instruction_data,
            vec![
                AccountMeta::new(pk_deployer1, true),
                AccountMeta::new(system_program_id, false),
            ],
        )];
        let message = Message::new(&instructions, None);
        let mut batch_message = BatchMessage::new(None);
        batch_message.clear().append_one(message);
        let input = serialize(&batch_message).unwrap();
        let deployer1_balance_before = ctx.get_balance(DEPLOYER_ADDRESS1);
        let result = ctx.call_evm_tx_simple(
            DEPLOYER_ADDRESS1,
            contract_address,
            input.clone().into(),
            None,
            None,
        );
        let deployer1_balance_after = ctx.get_balance(DEPLOYER_ADDRESS1);
        let deployer1_balance_spent = deployer1_balance_before - deployer1_balance_after;
        assert_eq!(U256::from(27350), deployer1_balance_spent);
        let output = result.output().unwrap();
        if output.len() > 0 {
            let out_text = from_utf8(output).unwrap();
            println!("output.len {} output '{}'", output.len(), out_text);
        }
        assert!(result.is_success());

        ctx.commit_db_to_sdk();

        let output = result.output().unwrap_or_default();
        let expected_output = hex!("");
        assert_eq!(hex::encode(expected_output), hex::encode(output));

        let contract_account = storage_read_account_data(&ctx.sdk, &pk_contract)
            .expect(format!("failed to read exec account data: {}", pk_contract).as_str());
        assert_eq!(contract_account.lamports(), 0);
        assert_eq!(
            contract_account.data().len(),
            LoaderV4State::program_data_offset() + account_with_program.data().len()
        );
        assert_eq!(
            &contract_account.data()[LoaderV4State::program_data_offset()..],
            account_with_program.data()
        );

        let deployer1_account = storage_read_account_data(&ctx.sdk, &pk_deployer1)
            .expect("failed to read payer account data");
        assert_eq!(deployer1_lamports, deployer1_account.lamports());
        assert_eq!(deployer1_account.data().len(), 0);
    }

    #[test]
    fn test_svm_sol_big_mod_exp() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands: Vec<TestCommand> = Default::default();
        let test_case = SolBigModExp::from_hex(
            "1111111111111111111111111111111111111111111111111111111111111111",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "111111111111111111111111111111111111111111111111111111111111110A",
            "0A7074864588D6847F33A168209E516F60005A0CEC3F33AAF70E8002FE964BCD",
            0,
        );
        test_commands.push(test_case.into());
        let test_case = SolBigModExp::from_hex(
            "2222222222222222222222222222222222222222222222222222222222222222",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "0000000000000000000000000000000000000000000000000000000000000000",
            0,
        );
        test_commands.push(test_case.into());
        let test_case = SolBigModExp::from_hex(
            "3333333333333333333333333333333333333333333333333333333333333333",
            "3333333333333333333333333333333333333333333333333333333333333333",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "1111111111111111111111111111111111111111111111111111111111111111",
            0,
        );
        test_commands.push(test_case.into());
        let test_case = SolBigModExp::from_hex(
            "9874231472317432847923174392874918237439287492374932871937289719",
            "0948403985401232889438579475812347232099080051356165126166266222",
            "25532321a214321423124212222224222b242222222222222222222222222444",
            "220ECE1C42624E98AEE7EB86578B2FE5C4855DFFACCB43CCBB708A3AB37F184D",
            0,
        );
        test_commands.push(test_case.into());
        let test_case = SolBigModExp::from_hex(
            "3494396663463663636363662632666565656456646566786786676786768766",
            "2324324333246536456354655645656616169896565698987033121934984955",
            "0218305479243590485092843590249879879842313131156656565565656566",
            "012F2865E8B9E79B645FCE3A9E04156483AE1F9833F6BFCF86FCA38FC2D5BEF0",
            0,
        );
        test_commands.push(test_case.into());
        let test_case = SolBigModExp::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000005",
            "0000000000000000000000000000000000000000000000000000000000000002",
            "0000000000000000000000000000000000000000000000000000000000000007",
            "0000000000000000000000000000000000000000000000000000000000000004",
            0,
        );
        test_commands.push(test_case.into());
        let test_case = SolBigModExp::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000019",
            "0000000000000000000000000000000000000000000000000000000000000019",
            "0000000000000000000000000000000000000000000000000000000000000064",
            "0000000000000000000000000000000000000000000000000000000000000019",
            0,
        );
        test_commands.push(test_case.into());
        let test_case = SolBigModExp::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000019",
            "0000000000000000000000000000000000000000000000000000000000000019",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            0,
        );
        test_commands.push(test_case.into());
        let test_case = SolBigModExp::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000019",
            "0000000000000000000000000000000000000000000000000000000000000019",
            "0000000000000000000000000000000000000000000000000000000000000001",
            "0000000000000000000000000000000000000000000000000000000000000000",
            0,
        );
        test_commands.push(test_case.into());

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_svm_sol_secp256k1_recover() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands: Vec<TestCommand> = vec![];

        let test_case = SolSecp256k1Recover {
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
            expected_ret: EXPECTED_RET_OK,
        };
        let fluent_test_case = SolSecp256k1Recover {
            message: test_case.message,
            signature_bytes: test_case.signature_bytes,
            recovery_id: test_case.recovery_id,
            pubkey_bytes: test_case.pubkey_bytes,
            expected_ret: test_case.expected_ret,
        };
        test_commands.push(fluent_test_case.into());

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_svm_sol_keccak256() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
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
            expected_ret: EXPECTED_RET_OK,
        }
        .into()];

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_svm_sol_sha256() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands: Vec<TestCommand> = vec![];
        let test_case = Sha256 {
            data: vec![vec![1u8, 2, 3], vec![4, 5, 6]],
            expected_result: hex!(
                "7192385c3c0605de55bb9476ce1d90748190ecb32a8eed7f5207b30cf6a1fe89"
            ),
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_svm_sol_blake3() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
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
            expected_ret: EXPECTED_RET_OK,
        }
        .into()];

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_svm_sol_poseidon_input_ones_be() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let input = [1u8; 32];
        let hash =
            solana_poseidon::hash(Parameters::Bn254X5, Endianness::BigEndian, &input).unwrap();
        assert_eq!(
            hash.to_bytes(),
            [
                5, 191, 172, 229, 129, 238, 97, 119, 204, 25, 198, 197, 99, 99, 166, 136, 130, 241,
                30, 132, 7, 172, 99, 157, 185, 145, 224, 210, 127, 27, 117, 230
            ]
        );

        let test_commands: &[TestCommand] = &[Poseidon {
            parameters: Parameters::Bn254X5.into(),
            endianness: Endianness::BigEndian.into(),
            data: vec![input.to_vec()],
            expected_result: hash.to_bytes(),
            expected_ret: EXPECTED_RET_OK,
        }
        .into()];

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_svm_sol_poseidon_input_ones_le() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let input = [1u8; 32];
        let hash =
            solana_poseidon::hash(Parameters::Bn254X5, Endianness::LittleEndian, &input).unwrap();
        assert_eq!(
            hash.to_bytes(),
            [
                230, 117, 27, 127, 210, 224, 145, 185, 157, 99, 172, 7, 132, 30, 241, 130, 136,
                166, 99, 99, 197, 198, 25, 204, 119, 97, 238, 129, 229, 172, 191, 5
            ],
        );

        let test_commands: &[TestCommand] = &[Poseidon {
            parameters: Parameters::Bn254X5.into(),
            endianness: Endianness::LittleEndian.into(),
            data: vec![input.to_vec()],
            expected_result: hash.to_bytes(),
            expected_ret: EXPECTED_RET_OK,
        }
        .into()];

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_svm_sol_poseidon_input_ones_twos_be() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let input1 = [1u8; 32];
        let input2 = [2u8; 32];
        let hash = solana_poseidon::hashv(
            Parameters::Bn254X5,
            Endianness::BigEndian,
            &[&input1, &input2],
        )
        .unwrap();
        assert_eq!(
            hash.to_bytes(),
            [
                13, 84, 225, 147, 143, 138, 140, 28, 125, 235, 94, 3, 85, 242, 99, 25, 32, 123,
                132, 254, 156, 162, 206, 27, 38, 231, 53, 200, 41, 130, 25, 144
            ]
        );

        let test_commands: &[TestCommand] = &[Poseidon {
            parameters: Parameters::Bn254X5.into(),
            endianness: Endianness::BigEndian.into(),
            data: vec![input1.to_vec(), input2.to_vec()],
            expected_result: hash.to_bytes(),
            expected_ret: EXPECTED_RET_OK,
        }
        .into()];

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_svm_sol_poseidon_input_ones_twos_le() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let input1 = [1u8; 32];
        let input2 = [2u8; 32];
        let hash = solana_poseidon::hashv(
            Parameters::Bn254X5,
            Endianness::LittleEndian,
            &[&input1, &input2],
        )
        .unwrap();
        assert_eq!(
            hash.to_bytes(),
            [
                144, 25, 130, 41, 200, 53, 231, 38, 27, 206, 162, 156, 254, 132, 123, 32, 25, 99,
                242, 85, 3, 94, 235, 125, 28, 140, 138, 143, 147, 225, 84, 13
            ]
        );

        let test_commands: &[TestCommand] = &[Poseidon {
            parameters: Parameters::Bn254X5.into(),
            endianness: Endianness::LittleEndian.into(),
            data: vec![input1.to_vec(), input2.to_vec()],
            expected_result: hash.to_bytes(),
            expected_ret: EXPECTED_RET_OK,
        }
        .into()];

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_svm_sol_poseidon_input_one() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands = vec![];

        let input = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 1,
        ];

        let expected_hashes = [
            [
                41, 23, 97, 0, 234, 169, 98, 189, 193, 254, 108, 101, 77, 106, 60, 19, 14, 150,
                164, 209, 22, 139, 51, 132, 139, 137, 125, 197, 2, 130, 1, 51,
            ],
            [
                0, 122, 243, 70, 226, 211, 4, 39, 158, 121, 224, 169, 243, 2, 63, 119, 18, 148,
                167, 138, 203, 112, 231, 63, 144, 175, 226, 124, 173, 64, 30, 129,
            ],
            [
                2, 192, 6, 110, 16, 167, 42, 189, 43, 51, 195, 178, 20, 203, 62, 129, 188, 177,
                182, 227, 9, 97, 205, 35, 194, 2, 177, 134, 115, 191, 37, 67,
            ],
            [
                8, 44, 156, 55, 10, 13, 36, 244, 65, 111, 188, 65, 74, 55, 104, 31, 120, 68, 45,
                39, 216, 99, 133, 153, 28, 23, 214, 252, 12, 75, 125, 113,
            ],
            [
                16, 56, 150, 5, 174, 104, 141, 79, 20, 219, 133, 49, 34, 196, 125, 102, 168, 3,
                199, 43, 65, 88, 156, 177, 191, 134, 135, 65, 178, 6, 185, 187,
            ],
            [
                42, 115, 246, 121, 50, 140, 62, 171, 114, 74, 163, 229, 189, 191, 80, 179, 144, 53,
                215, 114, 159, 19, 91, 151, 9, 137, 15, 133, 197, 220, 94, 118,
            ],
            [
                34, 118, 49, 10, 167, 243, 52, 58, 40, 66, 20, 19, 157, 157, 169, 89, 190, 42, 49,
                178, 199, 8, 165, 248, 25, 84, 178, 101, 229, 58, 48, 184,
            ],
            [
                23, 126, 20, 83, 196, 70, 225, 176, 125, 43, 66, 51, 66, 81, 71, 9, 92, 79, 202,
                187, 35, 61, 35, 11, 109, 70, 162, 20, 217, 91, 40, 132,
            ],
            [
                14, 143, 238, 47, 228, 157, 163, 15, 222, 235, 72, 196, 46, 187, 68, 204, 110, 231,
                5, 95, 97, 251, 202, 94, 49, 59, 138, 95, 202, 131, 76, 71,
            ],
            [
                46, 196, 198, 94, 99, 120, 171, 140, 115, 48, 133, 79, 74, 112, 119, 193, 255, 146,
                96, 228, 72, 133, 196, 184, 29, 209, 49, 173, 58, 134, 205, 150,
            ],
            [
                0, 113, 61, 65, 236, 166, 53, 241, 23, 212, 236, 188, 235, 95, 58, 102, 220, 65,
                66, 235, 112, 181, 103, 101, 188, 53, 143, 27, 236, 64, 187, 155,
            ],
            [
                20, 57, 11, 224, 186, 239, 36, 155, 212, 124, 101, 221, 172, 101, 194, 229, 46,
                133, 19, 192, 129, 193, 205, 114, 201, 128, 6, 9, 142, 154, 143, 190,
            ],
        ];

        for (i, expected_hash) in expected_hashes.iter().enumerate() {
            let inputs = vec![&input; i + 1]
                .into_iter()
                .map(|arr| &arr[..])
                .collect::<Vec<_>>();
            let hash = solana_poseidon::hashv(Parameters::Bn254X5, Endianness::BigEndian, &inputs)
                .unwrap();
            assert_eq!(hash.to_bytes(), *expected_hash);

            test_commands.push(
                Poseidon {
                    parameters: Parameters::Bn254X5.into(),
                    endianness: Endianness::BigEndian.into(),
                    data: inputs.iter().map(|v| v.to_vec()).collect(),
                    expected_result: hash.to_bytes(),
                    expected_ret: EXPECTED_RET_OK,
                }
                .into(),
            );
            test_commands.push(
                Poseidon {
                    parameters: Parameters::Bn254X5.into(),
                    endianness: Endianness::BigEndian.into(),
                    data: inputs
                        .iter()
                        .map(|v| {
                            let mut v = v.to_vec();
                            v.push(0xa);
                            v
                        })
                        .collect(),
                    expected_result: hash.to_bytes(),
                    expected_ret: EXPECTED_RET_ERR,
                }
                .into(),
            );
        }

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_svm_sol_return_data() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
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

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_svm_sol_curve_validate_point() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands: Vec<TestCommand> = Default::default();
        let test_case = CurvePointValidation {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            point: ED25519_BASEPOINT_POINT.compress().as_bytes().clone(),
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());
        let test_case = CurvePointValidation {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            point: [
                120, 140, 152, 233, 41, 227, 203, 27, 87, 115, 25, 251, 219, 5, 84, 148, 117, 38,
                84, 60, 87, 144, 161, 146, 42, 34, 91, 155, 158, 189, 121, 79,
            ],
            expected_ret: EXPECTED_RET_ERR,
        };
        test_commands.push(test_case.into());
        let test_case = CurvePointValidation {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_RISTRETTO,
            point: RISTRETTO_BASEPOINT_POINT.compress().as_bytes().clone(),
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());
        let test_case = CurvePointValidation {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_RISTRETTO,
            point: [
                120, 140, 152, 233, 41, 227, 203, 27, 87, 115, 25, 251, 219, 5, 84, 148, 117, 38,
                84, 60, 87, 144, 161, 146, 42, 34, 91, 155, 158, 189, 121, 79,
            ],
            expected_ret: EXPECTED_RET_ERR,
        };
        test_commands.push(test_case.into());

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_svm_sol_curve_group_op() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands: Vec<TestCommand> = vec![];

        // identity cases
        use curve25519_dalek::traits::Identity;
        let identity = PodEdwardsPoint(EdwardsPoint::identity().compress().to_bytes());
        let point = PodEdwardsPoint([
            201, 179, 241, 122, 180, 185, 239, 50, 183, 52, 221, 0, 153, 195, 43, 18, 22, 38, 187,
            206, 179, 192, 210, 58, 53, 45, 150, 98, 89, 17, 158, 11,
        ]);
        assert_eq!(add_edwards(&point, &identity).unwrap(), point);
        assert_eq!(subtract_edwards(&point, &identity).unwrap(), point);
        let test_case = CurveGroupOp {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            group_op: solana_curve25519::curve_syscall_traits::ADD,
            left_input: point.0,
            right_input: identity.0,
            expected_point: point.0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        let test_case = CurveGroupOp {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            group_op: solana_curve25519::curve_syscall_traits::SUB,
            left_input: point.0,
            right_input: identity.0,
            expected_point: point.0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        let scalar: [u8; 32] = [
            254, 198, 23, 138, 67, 243, 184, 110, 236, 115, 236, 205, 205, 215, 79, 114, 45, 250,
            78, 137, 3, 107, 136, 237, 49, 126, 117, 223, 37, 191, 88, 6,
        ];
        let right_point: [u8; 32] = [
            70, 222, 137, 221, 253, 204, 71, 51, 78, 8, 124, 1, 67, 200, 102, 225, 122, 228, 111,
            183, 129, 14, 131, 210, 212, 95, 109, 246, 55, 10, 159, 91,
        ];
        let expected_point: [u8; 32] = [
            64, 150, 40, 55, 80, 49, 217, 209, 105, 229, 181, 65, 241, 68, 2, 106, 220, 234, 211,
            71, 159, 76, 156, 114, 242, 68, 147, 31, 243, 211, 191, 124,
        ];
        let test_case = CurveGroupOp {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            group_op: solana_curve25519::curve_syscall_traits::MUL,
            left_input: scalar,
            right_input: right_point,
            expected_point,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

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
        let test_case = CurveGroupOp {
            // a + b
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            group_op: solana_curve25519::curve_syscall_traits::ADD,
            left_input: point_a.0,
            right_input: point_b.0,
            expected_point: add_edwards(&point_a, &point_b).unwrap().0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());
        let test_case = CurveGroupOp {
            // (a + b) + c
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            group_op: solana_curve25519::curve_syscall_traits::ADD,
            left_input: add_edwards(&point_a, &point_b).unwrap().0,
            right_input: point_c.0,
            expected_point: add_edwards(&add_edwards(&point_a, &point_b).unwrap(), &point_c)
                .unwrap()
                .0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        let test_case = CurveGroupOp {
            // b + c
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            group_op: solana_curve25519::curve_syscall_traits::ADD,
            left_input: point_b.0,
            right_input: point_c.0,
            expected_point: add_edwards(&point_b, &point_c).unwrap().0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        let test_case = CurveGroupOp {
            // a + (b + c)
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            group_op: solana_curve25519::curve_syscall_traits::ADD,
            left_input: point_a.0,
            right_input: add_edwards(&point_b, &point_c).unwrap().0,
            expected_point: add_edwards(&point_a, &add_edwards(&point_b, &point_c).unwrap())
                .unwrap()
                .0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        let test_case = CurveGroupOp {
            // (a + b) + c = a + (b + c)
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            group_op: solana_curve25519::curve_syscall_traits::ADD,
            left_input: add_edwards(&point_a, &point_b).unwrap().0,
            right_input: point_c.0,
            expected_point: add_edwards(&point_a, &add_edwards(&point_b, &point_c).unwrap())
                .unwrap()
                .0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        // commutativity
        assert_eq!(
            add_edwards(&point_a, &point_b).unwrap(),
            add_edwards(&point_b, &point_a).unwrap(),
        );
        let test_case = CurveGroupOp {
            // a + b = b + a
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            group_op: solana_curve25519::curve_syscall_traits::ADD,
            left_input: point_a.0,
            right_input: point_b.0,
            expected_point: add_edwards(&point_b, &point_a).unwrap().0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());
        let test_case = CurveGroupOp {
            // b + a = a + b
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            group_op: solana_curve25519::curve_syscall_traits::ADD,
            left_input: point_b.0,
            right_input: point_a.0,
            expected_point: add_edwards(&point_a, &point_b).unwrap().0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        // subtraction
        let point = PodEdwardsPoint(ED25519_BASEPOINT_POINT.compress().to_bytes());
        let point_negated = PodEdwardsPoint((-ED25519_BASEPOINT_POINT).compress().to_bytes());
        assert_eq!(point_negated, subtract_edwards(&identity, &point).unwrap(),);
        let test_case = CurveGroupOp {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            group_op: solana_curve25519::curve_syscall_traits::SUB,
            left_input: identity.0,
            right_input: point.0,
            expected_point: point_negated.0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        // RISTRETTO
        // identity
        let identity = PodRistrettoPoint(RistrettoPoint::identity().compress().to_bytes());
        let point = PodRistrettoPoint([
            210, 174, 124, 127, 67, 77, 11, 114, 71, 63, 168, 136, 113, 20, 141, 228, 195, 254,
            232, 229, 220, 249, 213, 232, 61, 238, 152, 249, 83, 225, 206, 16,
        ]);
        assert_eq!(add_ristretto(&point, &identity).unwrap(), point);
        let test_case = CurveGroupOp {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_RISTRETTO,
            group_op: solana_curve25519::curve_syscall_traits::ADD,
            left_input: point.0,
            right_input: identity.0,
            expected_point: add_ristretto(&point, &identity).unwrap().0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());
        assert_eq!(subtract_ristretto(&point, &identity).unwrap(), point);
        let test_case = CurveGroupOp {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_RISTRETTO,
            group_op: solana_curve25519::curve_syscall_traits::SUB,
            left_input: point.0,
            right_input: identity.0,
            expected_point: subtract_ristretto(&point, &identity).unwrap().0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        // associativity
        let point_a = PodRistrettoPoint([
            208, 165, 125, 204, 2, 100, 218, 17, 170, 194, 23, 9, 102, 156, 134, 136, 217, 190, 98,
            34, 183, 194, 228, 153, 92, 11, 108, 103, 28, 57, 88, 15,
        ]);
        let point_b = PodRistrettoPoint([
            208, 241, 72, 163, 73, 53, 32, 174, 54, 194, 71, 8, 70, 181, 244, 199, 93, 147, 99,
            231, 162, 127, 25, 40, 39, 19, 140, 132, 112, 212, 145, 108,
        ]);
        let point_c = PodRistrettoPoint([
            250, 61, 200, 25, 195, 15, 144, 179, 24, 17, 252, 167, 247, 44, 47, 41, 104, 237, 49,
            137, 231, 173, 86, 106, 121, 249, 245, 247, 70, 188, 31, 49,
        ]);
        assert_eq!(
            add_ristretto(&add_ristretto(&point_a, &point_b).unwrap(), &point_c),
            add_ristretto(&point_a, &add_ristretto(&point_b, &point_c).unwrap()),
        );
        let test_case = CurveGroupOp {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_RISTRETTO,
            group_op: solana_curve25519::curve_syscall_traits::ADD,
            left_input: add_ristretto(&point_a, &point_b).unwrap().0,
            right_input: point_c.0,
            expected_point: add_ristretto(&point_a, &add_ristretto(&point_b, &point_c).unwrap())
                .unwrap()
                .0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());
        assert_eq!(
            subtract_ristretto(&subtract_ristretto(&point_a, &point_b).unwrap(), &point_c),
            subtract_ristretto(&point_a, &add_ristretto(&point_b, &point_c).unwrap()),
        );
        let test_case = CurveGroupOp {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_RISTRETTO,
            group_op: solana_curve25519::curve_syscall_traits::SUB,
            left_input: subtract_ristretto(&point_a, &point_b).unwrap().0,
            right_input: point_c.0,
            expected_point: subtract_ristretto(
                &point_a,
                &add_ristretto(&point_b, &point_c).unwrap(),
            )
            .unwrap()
            .0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        // commutativity
        assert_eq!(
            add_ristretto(&point_a, &point_b).unwrap(),
            add_ristretto(&point_b, &point_a).unwrap(),
        );
        let test_case = CurveGroupOp {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_RISTRETTO,
            group_op: solana_curve25519::curve_syscall_traits::ADD,
            left_input: point_a.0,
            right_input: point_b.0,
            expected_point: add_ristretto(&point_b, &point_a).unwrap().0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        // subtraction
        let point = PodRistrettoPoint(RISTRETTO_BASEPOINT_POINT.compress().to_bytes());
        let point_negated = PodRistrettoPoint((-RISTRETTO_BASEPOINT_POINT).compress().to_bytes());
        assert_eq!(
            point_negated,
            subtract_ristretto(&identity, &point).unwrap(),
        );
        let test_case = CurveGroupOp {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_RISTRETTO,
            group_op: solana_curve25519::curve_syscall_traits::SUB,
            left_input: identity.0,
            right_input: point.0,
            expected_point: point_negated.0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        let scalar_x = PodScalar([
            254, 198, 23, 138, 67, 243, 184, 110, 236, 115, 236, 205, 205, 215, 79, 114, 45, 250,
            78, 137, 3, 107, 136, 237, 49, 126, 117, 223, 37, 191, 88, 6,
        ]);
        let point_a = PodRistrettoPoint([
            68, 80, 232, 181, 241, 77, 60, 81, 154, 51, 173, 35, 98, 234, 149, 37, 1, 39, 191, 201,
            193, 48, 88, 189, 97, 126, 63, 35, 144, 145, 203, 31,
        ]);
        let point_b = PodRistrettoPoint([
            200, 236, 1, 12, 244, 130, 226, 214, 28, 125, 43, 163, 222, 234, 81, 213, 201, 156, 31,
            4, 167, 132, 240, 76, 164, 18, 45, 20, 48, 85, 206, 121,
        ]);
        let ax = multiply_ristretto(&scalar_x, &point_a).unwrap();
        let bx = multiply_ristretto(&scalar_x, &point_b).unwrap();
        assert_eq!(
            add_ristretto(&ax, &bx),
            multiply_ristretto(&scalar_x, &add_ristretto(&point_a, &point_b).unwrap()),
        );
        let test_case = CurveGroupOp {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_RISTRETTO,
            group_op: solana_curve25519::curve_syscall_traits::MUL,
            left_input: scalar_x.0,
            right_input: add_ristretto(&point_a, &point_b).unwrap().0,
            expected_point: add_ristretto(&ax, &bx).unwrap().0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_sol_curve_multiscalar_mul() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
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
        let test_case = CurveMultiscalarMultiplication {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            scalars: vec![scalar.0],
            points: vec![point.0],
            expected_point: basic_product.0,
            expected_ret: EXPECTED_RET_OK,
        };
        test_commands.push(test_case.into());

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
        let test_case = CurveMultiscalarMultiplication {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_EDWARDS,
            scalars: vec![scalar_a.0, scalar_b.0],
            points: vec![point_x.0, point_y.0],
            expected_point: basic_product.0,
            expected_ret: EXPECTED_RET_OK,
        };

        test_commands.push(test_case.into());

        let scalar = PodScalar([
            123, 108, 109, 66, 154, 185, 88, 122, 178, 43, 17, 154, 201, 223, 31, 238, 59, 215, 71,
            154, 215, 143, 177, 158, 9, 136, 32, 223, 139, 13, 133, 5,
        ]);
        let point = PodRistrettoPoint([
            158, 2, 130, 90, 148, 36, 172, 155, 86, 196, 74, 139, 30, 98, 44, 225, 155, 207, 135,
            111, 238, 167, 235, 67, 234, 125, 0, 227, 146, 31, 24, 113,
        ]);
        let basic_product = multiply_ristretto(&scalar, &point).unwrap();
        let msm_product = multiscalar_multiply_ristretto(&[scalar], &[point]).unwrap();
        assert_eq!(basic_product, msm_product);
        let test_case = CurveMultiscalarMultiplication {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_RISTRETTO,
            scalars: vec![scalar.0],
            points: vec![point.0],
            expected_point: basic_product.0,
            expected_ret: EXPECTED_RET_OK,
        };

        test_commands.push(test_case.into());

        let scalar_a = PodScalar([
            8, 161, 219, 155, 192, 137, 153, 26, 27, 40, 30, 17, 124, 194, 26, 41, 32, 7, 161, 45,
            212, 198, 212, 81, 133, 185, 164, 85, 95, 232, 106, 10,
        ]);
        let scalar_b = PodScalar([
            135, 207, 106, 208, 107, 127, 46, 82, 66, 22, 136, 125, 105, 62, 69, 34, 213, 210, 17,
            196, 120, 114, 238, 237, 149, 170, 5, 243, 54, 77, 172, 12,
        ]);
        let point_x = PodRistrettoPoint([
            130, 35, 97, 25, 18, 199, 33, 239, 85, 143, 119, 111, 49, 51, 224, 40, 167, 185, 240,
            179, 25, 194, 213, 41, 14, 155, 104, 18, 181, 197, 15, 112,
        ]);
        let point_y = PodRistrettoPoint([
            152, 156, 155, 197, 152, 232, 92, 206, 219, 159, 193, 134, 121, 128, 139, 36, 56, 191,
            51, 143, 72, 204, 87, 76, 110, 124, 101, 96, 238, 158, 42, 108,
        ]);
        let ax = multiply_ristretto(&scalar_a, &point_x).unwrap();
        let by = multiply_ristretto(&scalar_b, &point_y).unwrap();
        let basic_product = add_ristretto(&ax, &by).unwrap();
        let msm_product =
            multiscalar_multiply_ristretto(&[scalar_a, scalar_b], &[point_x, point_y]).unwrap();
        assert_eq!(basic_product, msm_product);
        let test_case = CurveMultiscalarMultiplication {
            curve_id: solana_curve25519::curve_syscall_traits::CURVE25519_RISTRETTO,
            scalars: vec![scalar_a.0, scalar_b.0],
            points: vec![point_x.0, point_y.0],
            expected_point: basic_product.0,
            expected_ret: EXPECTED_RET_OK,
        };

        test_commands.push(test_case.into());

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_sol_alt_bn128_group_op_addition() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands: Vec<TestCommand> = vec![];

        let test_data = r#"[
        {
            "Input": "18b18acfb4c2c30276db5411368e7185b311dd124691610c5d3b74034e093dc9063c909c4720840cb5134cb9f59fa749755796819658d32efc0d288198f3726607c2b7f58a84bd6145f00c9c2bc0bb1a187f20ff2c92963a88019e7c6a014eed06614e20c147e940f2d70da3f74c9a17df361706a4485c742bd6788478fa17d7",
            "Expected": "2243525c5efd4b9c3d3c45ac0ca3fe4dd85e830a4ce6b65fa1eeaee202839703301d1d33be6da8e509df21cc35964723180eed7532537db9ae5e7d48f195c915",
            "Name": "chfast1",
            "Gas": 150,
            "NoBenchmark": false
        },{
            "Input": "2243525c5efd4b9c3d3c45ac0ca3fe4dd85e830a4ce6b65fa1eeaee202839703301d1d33be6da8e509df21cc35964723180eed7532537db9ae5e7d48f195c91518b18acfb4c2c30276db5411368e7185b311dd124691610c5d3b74034e093dc9063c909c4720840cb5134cb9f59fa749755796819658d32efc0d288198f37266",
            "Expected": "2bd3e6d0f3b142924f5ca7b49ce5b9d54c4703d7ae5648e61d02268b1a0a9fb721611ce0a6af85915e2f1d70300909ce2e49dfad4a4619c8390cae66cefdb204",
            "Name": "chfast2",
            "Gas": 150,
            "NoBenchmark": false
        },{
            "Input": "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "Expected": "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "Name": "cdetrio1",
            "Gas": 150,
            "NoBenchmark": false
        },{
            "Input": "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "Expected": "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "Name": "cdetrio2",
            "Gas": 150,
            "NoBenchmark": false
        },{
            "Input": "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "Expected": "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "Name": "cdetrio3",
            "Gas": 150,
            "NoBenchmark": false
        },{
            "Input": "",
            "Expected": "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "Name": "cdetrio4",
            "Gas": 150,
            "NoBenchmark": false
        },{
            "Input": "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002",
            "Expected": "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002",
            "Name": "cdetrio5",
            "Gas": 150,
            "NoBenchmark": false
        },{
            "Input": "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002",
            "Expected": "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002",
            "Name": "cdetrio6",
            "Gas": 150,
            "NoBenchmark": false
        },{
            "Input": "0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "Expected": "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002",
            "Gas": 150,
            "Name": "cdetrio7",
            "NoBenchmark": false
        },{
            "Input": "0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002",
            "Expected": "030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd315ed738c0e0a7c92e7845f96b2ae9c0a68a6a449e3538fc7ff3ebf7a5a18a2c4",
            "Name": "cdetrio8",
            "Gas": 150,
            "NoBenchmark": false
        },{
            "Input": "17c139df0efee0f766bc0204762b774362e4ded88953a39ce849a8a7fa163fa901e0559bacb160664764a357af8a9fe70baa9258e0b959273ffc5718c6d4cc7c039730ea8dff1254c0fee9c0ea777d29a9c710b7e616683f194f18c43b43b869073a5ffcc6fc7a28c30723d6e58ce577356982d65b833a5a5c15bf9024b43d98",
            "Expected": "15bf2bb17880144b5d1cd2b1f46eff9d617bffd1ca57c37fb5a49bd84e53cf66049c797f9ce0d17083deb32b5e36f2ea2a212ee036598dd7624c168993d1355f",
            "Name": "cdetrio9",
            "Gas": 150,
            "NoBenchmark": false
        }
        ]"#;

        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct TestCase {
            input: String,
            expected: String,
        }

        let test_cases: Vec<TestCase> = serde_json::from_str(test_data).unwrap();

        test_cases.iter().for_each(|test| {
            let input = array_bytes::hex2bytes_unchecked(&test.input);
            let result = alt_bn128_addition(&input);
            assert!(result.is_ok());

            let expected = array_bytes::hex2bytes_unchecked(&test.expected);

            assert_eq!(result.unwrap(), expected);

            let test_case = SyscallAltBn128 {
                group_op: ALT_BN128_ADD,
                input: input.clone(),
                expected_result: expected,
                expected_ret: EXPECTED_RET_OK,
            };

            test_commands.push(test_case.into());
        });

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_sol_alt_bn128_group_op_multiplication() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands: Vec<TestCommand> = vec![];

        let test_data = r#"[
        {
            "Input": "2bd3e6d0f3b142924f5ca7b49ce5b9d54c4703d7ae5648e61d02268b1a0a9fb721611ce0a6af85915e2f1d70300909ce2e49dfad4a4619c8390cae66cefdb20400000000000000000000000000000000000000000000000011138ce750fa15c2",
            "Expected": "070a8d6a982153cae4be29d434e8faef8a47b274a053f5a4ee2a6c9c13c31e5c031b8ce914eba3a9ffb989f9cdd5b0f01943074bf4f0f315690ec3cec6981afc",
            "Name": "chfast1",
            "Gas": 6000,
            "NoBenchmark": false
        },{
            "Input": "070a8d6a982153cae4be29d434e8faef8a47b274a053f5a4ee2a6c9c13c31e5c031b8ce914eba3a9ffb989f9cdd5b0f01943074bf4f0f315690ec3cec6981afc30644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd46",
            "Expected": "025a6f4181d2b4ea8b724290ffb40156eb0adb514c688556eb79cdea0752c2bb2eff3f31dea215f1eb86023a133a996eb6300b44da664d64251d05381bb8a02e",
            "Name": "chfast2",
            "Gas": 6000,
            "NoBenchmark": false
        },{
            "Input": "025a6f4181d2b4ea8b724290ffb40156eb0adb514c688556eb79cdea0752c2bb2eff3f31dea215f1eb86023a133a996eb6300b44da664d64251d05381bb8a02e183227397098d014dc2822db40c0ac2ecbc0b548b438e5469e10460b6c3e7ea3",
            "Expected": "14789d0d4a730b354403b5fac948113739e276c23e0258d8596ee72f9cd9d3230af18a63153e0ec25ff9f2951dd3fa90ed0197bfef6e2a1a62b5095b9d2b4a27",
            "Name": "chfast3",
            "Gas": 6000,
            "NoBenchmark": false
        },{
            "Input": "1a87b0584ce92f4593d161480614f2989035225609f08058ccfa3d0f940febe31a2f3c951f6dadcc7ee9007dff81504b0fcd6d7cf59996efdc33d92bf7f9f8f6ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "Expected": "2cde5879ba6f13c0b5aa4ef627f159a3347df9722efce88a9afbb20b763b4c411aa7e43076f6aee272755a7f9b84832e71559ba0d2e0b17d5f9f01755e5b0d11",
            "Name": "cdetrio1",
            "Gas": 6000,
            "NoBenchmark": false
        },{
            "Input": "1a87b0584ce92f4593d161480614f2989035225609f08058ccfa3d0f940febe31a2f3c951f6dadcc7ee9007dff81504b0fcd6d7cf59996efdc33d92bf7f9f8f630644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000000",
            "Expected": "1a87b0584ce92f4593d161480614f2989035225609f08058ccfa3d0f940febe3163511ddc1c3f25d396745388200081287b3fd1472d8339d5fecb2eae0830451",
            "Name": "cdetrio2",
            "Gas": 6000,
            "NoBenchmark": true
        },{
            "Input": "1a87b0584ce92f4593d161480614f2989035225609f08058ccfa3d0f940febe31a2f3c951f6dadcc7ee9007dff81504b0fcd6d7cf59996efdc33d92bf7f9f8f60000000000000000000000000000000100000000000000000000000000000000",
            "Expected": "1051acb0700ec6d42a88215852d582efbaef31529b6fcbc3277b5c1b300f5cf0135b2394bb45ab04b8bd7611bd2dfe1de6a4e6e2ccea1ea1955f577cd66af85b",
            "Name": "cdetrio3",
            "Gas": 6000,
            "NoBenchmark": true
        },{
            "Input": "1a87b0584ce92f4593d161480614f2989035225609f08058ccfa3d0f940febe31a2f3c951f6dadcc7ee9007dff81504b0fcd6d7cf59996efdc33d92bf7f9f8f60000000000000000000000000000000000000000000000000000000000000009",
            "Expected": "1dbad7d39dbc56379f78fac1bca147dc8e66de1b9d183c7b167351bfe0aeab742cd757d51289cd8dbd0acf9e673ad67d0f0a89f912af47ed1be53664f5692575",
            "Name": "cdetrio4",
            "Gas": 6000,
            "NoBenchmark": true
        },{
            "Input": "1a87b0584ce92f4593d161480614f2989035225609f08058ccfa3d0f940febe31a2f3c951f6dadcc7ee9007dff81504b0fcd6d7cf59996efdc33d92bf7f9f8f60000000000000000000000000000000000000000000000000000000000000001",
            "Expected": "1a87b0584ce92f4593d161480614f2989035225609f08058ccfa3d0f940febe31a2f3c951f6dadcc7ee9007dff81504b0fcd6d7cf59996efdc33d92bf7f9f8f6",
            "Name": "cdetrio5",
            "Gas": 6000,
            "NoBenchmark": true
        },{
            "Input": "17c139df0efee0f766bc0204762b774362e4ded88953a39ce849a8a7fa163fa901e0559bacb160664764a357af8a9fe70baa9258e0b959273ffc5718c6d4cc7cffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "Expected": "29e587aadd7c06722aabba753017c093f70ba7eb1f1c0104ec0564e7e3e21f6022b1143f6a41008e7755c71c3d00b6b915d386de21783ef590486d8afa8453b1",
            "Name": "cdetrio6",
            "Gas": 6000,
            "NoBenchmark": false
        },{
            "Input": "17c139df0efee0f766bc0204762b774362e4ded88953a39ce849a8a7fa163fa901e0559bacb160664764a357af8a9fe70baa9258e0b959273ffc5718c6d4cc7c30644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000000",
            "Expected": "17c139df0efee0f766bc0204762b774362e4ded88953a39ce849a8a7fa163fa92e83f8d734803fc370eba25ed1f6b8768bd6d83887b87165fc2434fe11a830cb",
            "Name": "cdetrio7",
            "Gas": 6000,
            "NoBenchmark": true
        },{
            "Input": "17c139df0efee0f766bc0204762b774362e4ded88953a39ce849a8a7fa163fa901e0559bacb160664764a357af8a9fe70baa9258e0b959273ffc5718c6d4cc7c0000000000000000000000000000000100000000000000000000000000000000",
            "Expected": "221a3577763877920d0d14a91cd59b9479f83b87a653bb41f82a3f6f120cea7c2752c7f64cdd7f0e494bff7b60419f242210f2026ed2ec70f89f78a4c56a1f15",
            "Name": "cdetrio8",
            "Gas": 6000,
            "NoBenchmark": true
        },{
            "Input": "17c139df0efee0f766bc0204762b774362e4ded88953a39ce849a8a7fa163fa901e0559bacb160664764a357af8a9fe70baa9258e0b959273ffc5718c6d4cc7c0000000000000000000000000000000000000000000000000000000000000009",
            "Expected": "228e687a379ba154554040f8821f4e41ee2be287c201aa9c3bc02c9dd12f1e691e0fd6ee672d04cfd924ed8fdc7ba5f2d06c53c1edc30f65f2af5a5b97f0a76a",
            "Name": "cdetrio9",
            "Gas": 6000,
            "NoBenchmark": true
        },{
            "Input": "17c139df0efee0f766bc0204762b774362e4ded88953a39ce849a8a7fa163fa901e0559bacb160664764a357af8a9fe70baa9258e0b959273ffc5718c6d4cc7c0000000000000000000000000000000000000000000000000000000000000001",
            "Expected": "17c139df0efee0f766bc0204762b774362e4ded88953a39ce849a8a7fa163fa901e0559bacb160664764a357af8a9fe70baa9258e0b959273ffc5718c6d4cc7c",
            "Name": "cdetrio10",
            "Gas": 6000,
            "NoBenchmark": true
        },{
            "Input": "039730ea8dff1254c0fee9c0ea777d29a9c710b7e616683f194f18c43b43b869073a5ffcc6fc7a28c30723d6e58ce577356982d65b833a5a5c15bf9024b43d98ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "Expected": "00a1a234d08efaa2616607e31eca1980128b00b415c845ff25bba3afcb81dc00242077290ed33906aeb8e42fd98c41bcb9057ba03421af3f2d08cfc441186024",
            "Name": "cdetrio11",
            "Gas": 6000,
            "NoBenchmark": false
        },{
            "Input": "039730ea8dff1254c0fee9c0ea777d29a9c710b7e616683f194f18c43b43b869073a5ffcc6fc7a28c30723d6e58ce577356982d65b833a5a5c15bf9024b43d9830644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000000",
            "Expected": "039730ea8dff1254c0fee9c0ea777d29a9c710b7e616683f194f18c43b43b8692929ee761a352600f54921df9bf472e66217e7bb0cee9032e00acc86b3c8bfaf",
            "Name": "cdetrio12",
            "Gas": 6000,
            "NoBenchmark": true
        },{
            "Input": "039730ea8dff1254c0fee9c0ea777d29a9c710b7e616683f194f18c43b43b869073a5ffcc6fc7a28c30723d6e58ce577356982d65b833a5a5c15bf9024b43d980000000000000000000000000000000100000000000000000000000000000000",
            "Expected": "1071b63011e8c222c5a771dfa03c2e11aac9666dd097f2c620852c3951a4376a2f46fe2f73e1cf310a168d56baa5575a8319389d7bfa6b29ee2d908305791434",
            "Name": "cdetrio13",
            "Gas": 6000,
            "NoBenchmark": true
        },{
            "Input": "039730ea8dff1254c0fee9c0ea777d29a9c710b7e616683f194f18c43b43b869073a5ffcc6fc7a28c30723d6e58ce577356982d65b833a5a5c15bf9024b43d980000000000000000000000000000000000000000000000000000000000000009",
            "Expected": "19f75b9dd68c080a688774a6213f131e3052bd353a304a189d7a2ee367e3c2582612f545fb9fc89fde80fd81c68fc7dcb27fea5fc124eeda69433cf5c46d2d7f",
            "Name": "cdetrio14",
            "Gas": 6000,
            "NoBenchmark": true
        },{
            "Input": "039730ea8dff1254c0fee9c0ea777d29a9c710b7e616683f194f18c43b43b869073a5ffcc6fc7a28c30723d6e58ce577356982d65b833a5a5c15bf9024b43d980000000000000000000000000000000000000000000000000000000000000001",
            "Expected": "039730ea8dff1254c0fee9c0ea777d29a9c710b7e616683f194f18c43b43b869073a5ffcc6fc7a28c30723d6e58ce577356982d65b833a5a5c15bf9024b43d98",
            "Name": "cdetrio15",
            "Gas": 6000,
            "NoBenchmark": true
        }
        ]"#;

        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct TestCase {
            input: String,
            expected: String,
        }

        let test_cases: Vec<TestCase> = serde_json::from_str(test_data).unwrap();

        test_cases.iter().for_each(|test| {
            let input = array_bytes::hex2bytes_unchecked(&test.input);
            let result = alt_bn128_multiplication(&input);
            assert!(result.is_ok());

            let expected = array_bytes::hex2bytes_unchecked(&test.expected);

            assert_eq!(result.unwrap(), expected);

            let test_case = SyscallAltBn128 {
                group_op: ALT_BN128_MUL,
                input: input.clone(),
                expected_result: expected,
                expected_ret: EXPECTED_RET_OK,
            };

            test_commands.push(test_case.into());
        });

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    type G1 = ark_bn254::g1::G1Affine;
    type G2 = ark_bn254::g2::G2Affine;

    #[test]
    fn test_sol_alt_bn128_pairing() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands: Vec<TestCommand> = vec![];

        let test_data = r#"[
        {
            "Input": "1c76476f4def4bb94541d57ebba1193381ffa7aa76ada664dd31c16024c43f593034dd2920f673e204fee2811c678745fc819b55d3e9d294e45c9b03a76aef41209dd15ebff5d46c4bd888e51a93cf99a7329636c63514396b4a452003a35bf704bf11ca01483bfa8b34b43561848d28905960114c8ac04049af4b6315a416782bb8324af6cfc93537a2ad1a445cfd0ca2a71acd7ac41fadbf933c2a51be344d120a2a4cf30c1bf9845f20c6fe39e07ea2cce61f0c9bb048165fe5e4de877550111e129f1cf1097710d41c4ac70fcdfa5ba2023c6ff1cbeac322de49d1b6df7c2032c61a830e3c17286de9462bf242fca2883585b93870a73853face6a6bf411198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000001",
            "Name": "jeff1",
            "Gas": 113000,
            "NoBenchmark": false
        },{
            "Input": "2eca0c7238bf16e83e7a1e6c5d49540685ff51380f309842a98561558019fc0203d3260361bb8451de5ff5ecd17f010ff22f5c31cdf184e9020b06fa5997db841213d2149b006137fcfb23036606f848d638d576a120ca981b5b1a5f9300b3ee2276cf730cf493cd95d64677bbb75fc42db72513a4c1e387b476d056f80aa75f21ee6226d31426322afcda621464d0611d226783262e21bb3bc86b537e986237096df1f82dff337dd5972e32a8ad43e28a78a96a823ef1cd4debe12b6552ea5f06967a1237ebfeca9aaae0d6d0bab8e28c198c5a339ef8a2407e31cdac516db922160fa257a5fd5b280642ff47b65eca77e626cb685c84fa6d3b6882a283ddd1198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000001",
            "Name": "jeff2",
            "Gas": 113000,
            "NoBenchmark": false
        },{
            "Input": "0f25929bcb43d5a57391564615c9e70a992b10eafa4db109709649cf48c50dd216da2f5cb6be7a0aa72c440c53c9bbdfec6c36c7d515536431b3a865468acbba2e89718ad33c8bed92e210e81d1853435399a271913a6520736a4729cf0d51eb01a9e2ffa2e92599b68e44de5bcf354fa2642bd4f26b259daa6f7ce3ed57aeb314a9a87b789a58af499b314e13c3d65bede56c07ea2d418d6874857b70763713178fb49a2d6cd347dc58973ff49613a20757d0fcc22079f9abd10c3baee245901b9e027bd5cfc2cb5db82d4dc9677ac795ec500ecd47deee3b5da006d6d049b811d7511c78158de484232fc68daf8a45cf217d1c2fae693ff5871e8752d73b21198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000001",
            "Name": "jeff3",
            "Gas": 113000,
            "NoBenchmark": false
        },{
            "Input": "2f2ea0b3da1e8ef11914acf8b2e1b32d99df51f5f4f206fc6b947eae860eddb6068134ddb33dc888ef446b648d72338684d678d2eb2371c61a50734d78da4b7225f83c8b6ab9de74e7da488ef02645c5a16a6652c3c71a15dc37fe3a5dcb7cb122acdedd6308e3bb230d226d16a105295f523a8a02bfc5e8bd2da135ac4c245d065bbad92e7c4e31bf3757f1fe7362a63fbfee50e7dc68da116e67d600d9bf6806d302580dc0661002994e7cd3a7f224e7ddc27802777486bf80f40e4ca3cfdb186bac5188a98c45e6016873d107f5cd131f3a3e339d0375e58bd6219347b008122ae2b09e539e152ec5364e7e2204b03d11d3caa038bfc7cd499f8176aacbee1f39e4e4afc4bc74790a4a028aff2c3d2538731fb755edefd8cb48d6ea589b5e283f150794b6736f670d6a1033f9b46c6f5204f50813eb85c8dc4b59db1c5d39140d97ee4d2b36d99bc49974d18ecca3e7ad51011956051b464d9e27d46cc25e0764bb98575bd466d32db7b15f582b2d5c452b36aa394b789366e5e3ca5aabd415794ab061441e51d01e94640b7e3084a07e02c78cf3103c542bc5b298669f211b88da1679b0b64a63b7e0e7bfe52aae524f73a55be7fe70c7e9bfc94b4cf0da1213d2149b006137fcfb23036606f848d638d576a120ca981b5b1a5f9300b3ee2276cf730cf493cd95d64677bbb75fc42db72513a4c1e387b476d056f80aa75f21ee6226d31426322afcda621464d0611d226783262e21bb3bc86b537e986237096df1f82dff337dd5972e32a8ad43e28a78a96a823ef1cd4debe12b6552ea5f",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000001",
            "Name": "jeff4",
            "Gas": 147000,
            "NoBenchmark": false
        },{
            "Input": "20a754d2071d4d53903e3b31a7e98ad6882d58aec240ef981fdf0a9d22c5926a29c853fcea789887315916bbeb89ca37edb355b4f980c9a12a94f30deeed30211213d2149b006137fcfb23036606f848d638d576a120ca981b5b1a5f9300b3ee2276cf730cf493cd95d64677bbb75fc42db72513a4c1e387b476d056f80aa75f21ee6226d31426322afcda621464d0611d226783262e21bb3bc86b537e986237096df1f82dff337dd5972e32a8ad43e28a78a96a823ef1cd4debe12b6552ea5f1abb4a25eb9379ae96c84fff9f0540abcfc0a0d11aeda02d4f37e4baf74cb0c11073b3ff2cdbb38755f8691ea59e9606696b3ff278acfc098fa8226470d03869217cee0a9ad79a4493b5253e2e4e3a39fc2df38419f230d341f60cb064a0ac290a3d76f140db8418ba512272381446eb73958670f00cf46f1d9e64cba057b53c26f64a8ec70387a13e41430ed3ee4a7db2059cc5fc13c067194bcc0cb49a98552fd72bd9edb657346127da132e5b82ab908f5816c826acb499e22f2412d1a2d70f25929bcb43d5a57391564615c9e70a992b10eafa4db109709649cf48c50dd2198a1f162a73261f112401aa2db79c7dab1533c9935c77290a6ce3b191f2318d198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000001",
            "Name": "jeff5",
            "Gas": 147000,
            "NoBenchmark": false
        },{
            "Input": "1c76476f4def4bb94541d57ebba1193381ffa7aa76ada664dd31c16024c43f593034dd2920f673e204fee2811c678745fc819b55d3e9d294e45c9b03a76aef41209dd15ebff5d46c4bd888e51a93cf99a7329636c63514396b4a452003a35bf704bf11ca01483bfa8b34b43561848d28905960114c8ac04049af4b6315a416782bb8324af6cfc93537a2ad1a445cfd0ca2a71acd7ac41fadbf933c2a51be344d120a2a4cf30c1bf9845f20c6fe39e07ea2cce61f0c9bb048165fe5e4de877550111e129f1cf1097710d41c4ac70fcdfa5ba2023c6ff1cbeac322de49d1b6df7c103188585e2364128fe25c70558f1560f4f9350baf3959e603cc91486e110936198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000000",
            "Name": "jeff6",
            "Gas": 113000,
            "NoBenchmark": false
        },{
            "Input": "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000000",
            "Name": "one_point",
            "Gas": 79000,
            "NoBenchmark": false
        },{
            "Input": "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000001",
            "Name": "two_point_match_2",
            "Gas": 113000,
            "NoBenchmark": false
        },{
            "Input": "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002203e205db4f19b37b60121b83a7333706db86431c6d835849957ed8c3928ad7927dc7234fd11d3e8c36c59277c3e6f149d5cd3cfa9a62aee49f8130962b4b3b9195e8aa5b7827463722b8c153931579d3505566b4edf48d498e185f0509de15204bb53b8977e5f92a0bc372742c4830944a59b4fe6b1c0466e2a6dad122b5d2e030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd31a76dae6d3272396d0cbe61fced2bc532edac647851e3ac53ce1cc9c7e645a83198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000001",
            "Name": "two_point_match_3",
            "Gas": 113000,
            "NoBenchmark": false
        },{
            "Input": "105456a333e6d636854f987ea7bb713dfd0ae8371a72aea313ae0c32c0bf10160cf031d41b41557f3e7e3ba0c51bebe5da8e6ecd855ec50fc87efcdeac168bcc0476be093a6d2b4bbf907172049874af11e1b6267606e00804d3ff0037ec57fd3010c68cb50161b7d1d96bb71edfec9880171954e56871abf3d93cc94d745fa114c059d74e5b6c4ec14ae5864ebe23a71781d86c29fb8fb6cce94f70d3de7a2101b33461f39d9e887dbb100f170a2345dde3c07e256d1dfa2b657ba5cd030427000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000021a2c3013d2ea92e13c800cde68ef56a294b883f6ac35d25f587c09b1b3c635f7290158a80cd3d66530f74dc94c94adb88f5cdb481acca997b6e60071f08a115f2f997f3dbd66a7afe07fe7862ce239edba9e05c5afff7f8a1259c9733b2dfbb929d1691530ca701b4a106054688728c9972c8512e9789e9567aae23e302ccd75",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000001",
            "Name": "two_point_match_4",
            "Gas": 113000,
            "NoBenchmark": false
        },{
            "Input": "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed275dc4a288d1afb3cbb1ac09187524c7db36395df7be3b99e673b13a075a65ec1d9befcd05a5323e6da4d435f3b617cdb3af83285c2df711ef39c01571827f9d",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000001",
            "Name": "ten_point_match_1",
            "Gas": 385000,
            "NoBenchmark": false
        },{
            "Input": "00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002203e205db4f19b37b60121b83a7333706db86431c6d835849957ed8c3928ad7927dc7234fd11d3e8c36c59277c3e6f149d5cd3cfa9a62aee49f8130962b4b3b9195e8aa5b7827463722b8c153931579d3505566b4edf48d498e185f0509de15204bb53b8977e5f92a0bc372742c4830944a59b4fe6b1c0466e2a6dad122b5d2e030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd31a76dae6d3272396d0cbe61fced2bc532edac647851e3ac53ce1cc9c7e645a83198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002203e205db4f19b37b60121b83a7333706db86431c6d835849957ed8c3928ad7927dc7234fd11d3e8c36c59277c3e6f149d5cd3cfa9a62aee49f8130962b4b3b9195e8aa5b7827463722b8c153931579d3505566b4edf48d498e185f0509de15204bb53b8977e5f92a0bc372742c4830944a59b4fe6b1c0466e2a6dad122b5d2e030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd31a76dae6d3272396d0cbe61fced2bc532edac647851e3ac53ce1cc9c7e645a83198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002203e205db4f19b37b60121b83a7333706db86431c6d835849957ed8c3928ad7927dc7234fd11d3e8c36c59277c3e6f149d5cd3cfa9a62aee49f8130962b4b3b9195e8aa5b7827463722b8c153931579d3505566b4edf48d498e185f0509de15204bb53b8977e5f92a0bc372742c4830944a59b4fe6b1c0466e2a6dad122b5d2e030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd31a76dae6d3272396d0cbe61fced2bc532edac647851e3ac53ce1cc9c7e645a83198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002203e205db4f19b37b60121b83a7333706db86431c6d835849957ed8c3928ad7927dc7234fd11d3e8c36c59277c3e6f149d5cd3cfa9a62aee49f8130962b4b3b9195e8aa5b7827463722b8c153931579d3505566b4edf48d498e185f0509de15204bb53b8977e5f92a0bc372742c4830944a59b4fe6b1c0466e2a6dad122b5d2e030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd31a76dae6d3272396d0cbe61fced2bc532edac647851e3ac53ce1cc9c7e645a83198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002203e205db4f19b37b60121b83a7333706db86431c6d835849957ed8c3928ad7927dc7234fd11d3e8c36c59277c3e6f149d5cd3cfa9a62aee49f8130962b4b3b9195e8aa5b7827463722b8c153931579d3505566b4edf48d498e185f0509de15204bb53b8977e5f92a0bc372742c4830944a59b4fe6b1c0466e2a6dad122b5d2e030644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd31a76dae6d3272396d0cbe61fced2bc532edac647851e3ac53ce1cc9c7e645a83198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000001",
            "Name": "ten_point_match_2",
            "Gas": 385000,
            "NoBenchmark": false
        },{
            "Input": "105456a333e6d636854f987ea7bb713dfd0ae8371a72aea313ae0c32c0bf10160cf031d41b41557f3e7e3ba0c51bebe5da8e6ecd855ec50fc87efcdeac168bcc0476be093a6d2b4bbf907172049874af11e1b6267606e00804d3ff0037ec57fd3010c68cb50161b7d1d96bb71edfec9880171954e56871abf3d93cc94d745fa114c059d74e5b6c4ec14ae5864ebe23a71781d86c29fb8fb6cce94f70d3de7a2101b33461f39d9e887dbb100f170a2345dde3c07e256d1dfa2b657ba5cd030427000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000021a2c3013d2ea92e13c800cde68ef56a294b883f6ac35d25f587c09b1b3c635f7290158a80cd3d66530f74dc94c94adb88f5cdb481acca997b6e60071f08a115f2f997f3dbd66a7afe07fe7862ce239edba9e05c5afff7f8a1259c9733b2dfbb929d1691530ca701b4a106054688728c9972c8512e9789e9567aae23e302ccd75",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000001",
            "Name": "ten_point_match_3",
            "Gas": 113000,
            "NoBenchmark": false
        }
        ]"#;

        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct TestCase {
            input: String,
            expected: String,
        }

        let test_cases: Vec<TestCase> = serde_json::from_str(test_data).unwrap();

        test_cases.iter().for_each(|test| {
            let input = array_bytes::hex2bytes_unchecked(&test.input);
            let result = alt_bn128_pairing(&input);
            assert!(result.is_ok());

            let expected = array_bytes::hex2bytes_unchecked(&test.expected);

            assert_eq!(result.unwrap(), expected);

            let test_case = SyscallAltBn128 {
                group_op: ALT_BN128_PAIRING,
                input: input.clone(),
                expected_result: expected,
                expected_ret: EXPECTED_RET_OK,
            };

            test_commands.push(test_case.into());
        });

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_sol_alt_bn128_compression_g1_compress_decompress() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands: Vec<TestCommand> = vec![];

        let g1_be = [
            45, 206, 255, 166, 152, 55, 128, 138, 79, 217, 145, 164, 25, 74, 120, 234, 234, 217,
            68, 149, 162, 44, 133, 120, 184, 205, 12, 44, 175, 98, 168, 172, 20, 24, 216, 15, 209,
            175, 106, 75, 147, 236, 90, 101, 123, 219, 245, 151, 209, 202, 218, 104, 148, 8, 32,
            254, 243, 191, 218, 122, 42, 81, 193, 84,
        ];
        let g1_le = convert_endianness_fixed::<32, 64>(&g1_be);
        let g1: G1 =
            G1::deserialize_with_mode(g1_le.as_slice(), Compress::No, Validate::No).unwrap();

        let g1_neg = g1.neg();
        let mut g1_neg_be = [0u8; 64];
        g1_neg
            .x
            .serialize_with_mode(&mut g1_neg_be[..32], Compress::No)
            .unwrap();
        g1_neg
            .y
            .serialize_with_mode(&mut g1_neg_be[32..64], Compress::No)
            .unwrap();
        let g1_neg_be: [u8; 64] = convert_endianness_fixed::<32, 64>(&g1_neg_be);

        let points = [(g1, g1_be), (g1_neg, g1_neg_be)];

        for (point, g1_be) in &points {
            let mut compressed_ref = [0u8; 32];
            G1::serialize_with_mode(point, compressed_ref.as_mut_slice(), Compress::Yes).unwrap();
            let compressed_ref: [u8; 32] = convert_endianness_fixed::<32, 32>(&compressed_ref);

            let decompressed = alt_bn128_g1_decompress(compressed_ref.as_slice()).unwrap();

            assert_eq!(
                alt_bn128_g1_compress(&decompressed).unwrap(),
                compressed_ref
            );
            assert_eq!(decompressed, *g1_be);

            let test_case = AltBn128Compression {
                group_op: ALT_BN128_G1_COMPRESS,
                input: decompressed.to_vec(),
                expected_result: compressed_ref.to_vec(),
                expected_ret: EXPECTED_RET_OK,
            };

            test_commands.push(test_case.into());
            let syscall_decompressed =
                SyscallWeierstrassCompressDecompressAssign::<ConfigG1Decompress>::fn_impl(
                    &convert_endianness_fixed::<
                        BN254_G1_POINT_COMPRESSED_SIZE,
                        BN254_G1_POINT_COMPRESSED_SIZE,
                    >(&compressed_ref.try_into().unwrap()),
                )
                .unwrap();
            assert_eq!(
                decompressed,
                convert_endianness_fixed::<32, 64>(&syscall_decompressed.try_into().unwrap(),)
            );

            let test_case = AltBn128Compression {
                group_op: ALT_BN128_G1_DECOMPRESS,
                input: compressed_ref.to_vec(),
                expected_result: decompressed.to_vec(),
                expected_ret: EXPECTED_RET_OK,
            };

            test_commands.push(test_case.into());
            let syscall_compressed =
                SyscallWeierstrassCompressDecompressAssign::<ConfigG1Compress>::fn_impl(
                    &convert_endianness_fixed::<
                        BN254_G1_POINT_COMPRESSED_SIZE,
                        BN254_G1_POINT_DECOMPRESSED_SIZE,
                    >(&decompressed.try_into().unwrap()),
                )
                .unwrap();
            assert_eq!(
                compressed_ref,
                convert_endianness_fixed::<32, 32>(&syscall_compressed.try_into().unwrap(),)
            );
        }

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_sol_alt_bn128_compression_g2_compress_decompress() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands: Vec<TestCommand> = vec![];

        let g2_be = [
            40, 57, 233, 205, 180, 46, 35, 111, 215, 5, 23, 93, 12, 71, 118, 225, 7, 46, 247, 147,
            47, 130, 106, 189, 184, 80, 146, 103, 141, 52, 242, 25, 0, 203, 124, 176, 110, 34, 151,
            212, 66, 180, 238, 151, 236, 189, 133, 209, 17, 137, 205, 183, 168, 196, 92, 159, 75,
            174, 81, 168, 18, 86, 176, 56, 16, 26, 210, 20, 18, 81, 122, 142, 104, 62, 251, 169,
            98, 141, 21, 253, 50, 130, 182, 15, 33, 109, 228, 31, 79, 183, 88, 147, 174, 108, 4,
            22, 14, 129, 168, 6, 80, 246, 254, 100, 218, 131, 94, 49, 247, 211, 3, 245, 22, 200,
            177, 91, 60, 144, 147, 174, 90, 17, 19, 189, 62, 147, 152, 18,
        ];
        let g2_le = convert_endianness_fixed::<64, 128>(&g2_be);
        let g2: G2 =
            G2::deserialize_with_mode(g2_le.as_slice(), Compress::No, Validate::No).unwrap();

        let g2_neg = g2.neg();
        let mut g2_neg_be = [0u8; 128];
        g2_neg
            .x
            .serialize_with_mode(&mut g2_neg_be[..64], Compress::No)
            .unwrap();
        g2_neg
            .y
            .serialize_with_mode(&mut g2_neg_be[64..128], Compress::No)
            .unwrap();
        let g2_neg_be: [u8; 128] = convert_endianness_fixed::<64, 128>(&g2_neg_be);

        let points = [(g2, g2_be), (g2_neg, g2_neg_be)];

        for (point, g2_be) in &points {
            let mut compressed_ref = [0u8; 64];
            G2::serialize_with_mode(point, compressed_ref.as_mut_slice(), Compress::Yes).unwrap();
            let compressed_ref: [u8; 64] = convert_endianness_fixed::<64, 64>(&compressed_ref);

            let decompressed = alt_bn128_g2_decompress(compressed_ref.as_slice()).unwrap();

            assert_eq!(
                alt_bn128_g2_compress(&decompressed).unwrap(),
                compressed_ref
            );
            assert_eq!(decompressed, *g2_be);

            let test_case = AltBn128Compression {
                group_op: ALT_BN128_G2_COMPRESS,
                input: decompressed.to_vec(),
                expected_result: alt_bn128_g2_compress(&decompressed).unwrap().to_vec(),
                expected_ret: EXPECTED_RET_OK,
            };

            test_commands.push(test_case.into());
            let syscall_decompressed =
                SyscallWeierstrassCompressDecompressAssign::<ConfigG2Decompress>::fn_impl(
                    &convert_endianness_fixed::<
                        BN254_G2_POINT_COMPRESSED_SIZE,
                        BN254_G2_POINT_COMPRESSED_SIZE,
                    >(&compressed_ref.try_into().unwrap()),
                )
                .unwrap();
            assert_eq!(
                decompressed,
                convert_endianness_fixed::<64, 128>(&syscall_decompressed.try_into().unwrap(),)
            );

            let test_case = AltBn128Compression {
                group_op: ALT_BN128_G2_DECOMPRESS,
                input: alt_bn128_g2_compress(&decompressed).unwrap().to_vec(),
                expected_result: decompressed.to_vec(),
                expected_ret: EXPECTED_RET_OK,
            };

            test_commands.push(test_case.into());
            let syscall_compressed =
                SyscallWeierstrassCompressDecompressAssign::<ConfigG2Compress>::fn_impl(
                    &convert_endianness_fixed::<
                        BN254_G2_POINT_COMPRESSED_SIZE,
                        BN254_G2_POINT_DECOMPRESSED_SIZE,
                    >(&decompressed.try_into().unwrap()),
                )
                .unwrap();
            assert_eq!(
                compressed_ref,
                convert_endianness_fixed::<64, 64>(&syscall_compressed.try_into().unwrap(),)
            );
        }

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }

    #[test]
    fn test_sol_alt_bn128_compression_pairing() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();
        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../contracts/examples/svm/assets/solana_program_state_usage.so",
        );
        let payer_lamports = 101;
        let seed1 = b"seed";

        let (pk_payer, pk_exec, pk_new, contract_address) =
            svm_deploy(&mut ctx, &account_with_program, seed1, payer_lamports);

        // exec

        let mut test_commands: Vec<TestCommand> = vec![];

        let test_data = r#"[
        {
            "Input": "1c76476f4def4bb94541d57ebba1193381ffa7aa76ada664dd31c16024c43f593034dd2920f673e204fee2811c678745fc819b55d3e9d294e45c9b03a76aef41209dd15ebff5d46c4bd888e51a93cf99a7329636c63514396b4a452003a35bf704bf11ca01483bfa8b34b43561848d28905960114c8ac04049af4b6315a416782bb8324af6cfc93537a2ad1a445cfd0ca2a71acd7ac41fadbf933c2a51be344d120a2a4cf30c1bf9845f20c6fe39e07ea2cce61f0c9bb048165fe5e4de877550111e129f1cf1097710d41c4ac70fcdfa5ba2023c6ff1cbeac322de49d1b6df7c2032c61a830e3c17286de9462bf242fca2883585b93870a73853face6a6bf411198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            "Expected": "0000000000000000000000000000000000000000000000000000000000000001",
            "Name": "jeff1",
            "Gas": 113000,
            "NoBenchmark": false
        }
        ]"#;

        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct TestCase {
            input: String,
        }

        let test_cases: Vec<TestCase> = serde_json::from_str(test_data).unwrap();

        test_cases.iter().for_each(|test| {
            let input = array_bytes::hex2bytes_unchecked(&test.input);
            let g1 = input[0..64].to_vec();
            let g1_compressed = alt_bn128_g1_compress(&g1).unwrap();
            assert_eq!(g1, alt_bn128_g1_decompress(&g1_compressed).unwrap());

            let test_case = AltBn128Compression {
                group_op: ALT_BN128_G1_DECOMPRESS,
                input: g1_compressed.to_vec(),
                expected_result: g1.to_vec(),
                expected_ret: EXPECTED_RET_OK,
            };

            test_commands.push(test_case.into());

            let g2 = input[64..192].to_vec();
            let g2_compressed = alt_bn128_g2_compress(&g2).unwrap();
            assert_eq!(g2, alt_bn128_g2_decompress(&g2_compressed).unwrap());

            let test_case = AltBn128Compression {
                group_op: ALT_BN128_G2_DECOMPRESS,
                input: g2_compressed.to_vec(),
                expected_result: g2.to_vec(),
                expected_ret: EXPECTED_RET_OK,
            };

            test_commands.push(test_case.into());
        });

        process_test_commands(
            &mut ctx,
            &contract_address,
            &pk_exec,
            &pk_payer,
            &pk_new,
            &system_program_id,
            &test_commands,
        );
    }
}
