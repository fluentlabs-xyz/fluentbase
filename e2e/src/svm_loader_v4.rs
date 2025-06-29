mod tests {
    use core::str::from_utf8;
    use fluentbase_sdk::{
        address,
        debug_log_ext,
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
        rent::Rent,
        solana_bincode::serialize,
        solana_program::{
            instruction::{AccountMeta, Instruction},
            loader_v4,
            loader_v4::LoaderV4State,
            message::Message,
        },
        system_program,
    };
    use hex_literal::hex;
    use rand::random_range;
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
    fn test_svm_deploy() {
        let mut ctx = EvmTestingContext::default();
        ctx.sdk.set_ownable_account_address(PRECOMPILE_SVM_RUNTIME);
        const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
        ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
            // address: PRECOMPILE_SVM_RUNTIME,
            ..Default::default()
        });

        // setup

        let loader_id = loader_v4::id();

        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../examples/svm/solana-program-state-usage/assets/solana_program.so",
        );

        let program_bytes = account_with_program.data().to_vec();
        ctx.add_balance(DEPLOYER_ADDRESS, U256::from(1e18));
        let (_contract_address, _gas_used) =
            ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS, program_bytes.into());
    }

    #[test]
    fn test_svm_deploy_exec() {
        let mut ctx = EvmTestingContext::default();
        ctx.sdk.set_ownable_account_address(PRECOMPILE_SVM_RUNTIME);
        assert_eq!(ctx.sdk.context().block_number(), 0);
        const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");

        // setup

        let loader_id = loader_v4::id();
        let system_program_id = system_program::id();

        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../examples/svm/solana-program/assets/solana_program.so",
            "../examples/svm/solana-program-state-usage/assets/solana_program.so",
        );

        // setup initial accounts

        let payer_lamports = 101;
        let pk_payer = pubkey_from_evm_address(&DEPLOYER_ADDRESS);
        ctx.add_balance(DEPLOYER_ADDRESS, evm_balance_from_lamports(payer_lamports));

        // deploy and get exec contract

        let program_bytes = account_with_program.data().to_vec();
        let (contract_address, _gas) =
            ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS, program_bytes.into());

        let pk_exec = pubkey_from_evm_address(&contract_address);

        let seed1 = b"my_seed";
        let seed2 = pk_payer.as_ref();
        let seeds = &[seed1.as_slice(), seed2];
        let (pk_new, _bump) = Pubkey::find_program_address(seeds, &pk_exec);

        // exec

        let mut instruction_data = Vec::<u8>::new();
        let lamports_to_send: u64 = 12;
        let space: u32 = 101;
        let seed_len: u8 = seed1.len() as u8;
        let byte_n_to_set: u32 = random_range(0..space);
        let byte_n_val: u8 = rand::random();
        instruction_data.push(2);
        instruction_data.extend_from_slice(lamports_to_send.to_le_bytes().as_slice());
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
        let mut batch_message = BatchMessage::new(None);
        batch_message.clear().append_one(message);
        let input = serialize(&batch_message).unwrap();
        ctx.sdk = ctx.sdk.with_block_number(1);
        assert_eq!(ctx.sdk.context().block_number(), 1);
        let result =
            ctx.call_evm_tx_simple(DEPLOYER_ADDRESS, contract_address, input.into(), None, None);
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

        debug_log_ext!();
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

        debug_log_ext!();
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
            payer_lamports - 1 - lamports_to_send
        );
        assert_eq!(payer_account.data().len(), 0);

        debug_log_ext!();
        let new_account = storage_read_account_data(&ctx.sdk, &pk_new)
            .expect(format!("failed to read new account data: {}", pk_new).as_str());
        assert_eq!(new_account.lamports(), lamports_to_send);
        assert_eq!(new_account.data().len(), space as usize);
        assert_eq!(new_account.data()[byte_n_to_set as usize], byte_n_val);
    }
}
