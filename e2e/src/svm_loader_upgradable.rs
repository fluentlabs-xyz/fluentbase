mod tests {
    use crate::utils::EvmTestingContext;
    use fluentbase_sdk::{
        address,
        Address,
        BlockContextReader,
        ContractContextV1,
        SharedAPI,
        PRECOMPILE_SVM_RUNTIME,
        U256,
    };
    use hex_literal::hex;
    use solana_ee_core::{
        account::{AccountSharedData, ReadableAccount, WritableAccount},
        bincode,
        common::pubkey_from_address,
        fluentbase::fluentbase_common::BatchMessage,
        solana_program::{
            bpf_loader_upgradeable,
            instruction::Instruction,
            loader_v4,
            message::Message,
            pubkey::Pubkey,
            rent::Rent,
        },
    };
    use std::{fs::File, io::Read};

    #[test]
    fn test_svm_deploy() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
        ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
            address: PRECOMPILE_SVM_RUNTIME,
            ..Default::default()
        });

        // setup

        let loader_id = bpf_loader_upgradeable::id();
        // let loader_id = loader_v4::id();

        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            "../solana-ee-core/crates/core/test_elfs/out/noop_aligned.so",
            // "../solana-ee-core/crates/examples/hello-world/assets/solana_program.so",
        );

        let program_bytes = account_with_program.data().to_vec();
        ctx.add_balance(DEPLOYER_ADDRESS, U256::from(1e18));
        let (contract_address, gas_used) =
            ctx.deploy_evm_tx_with_gas(DEPLOYER_ADDRESS, program_bytes.into());
        println!("contract_addr {:x?}", contract_address);
    }

    #[test]
    fn test_svm_deploy_exec() {
        let mut ctx = EvmTestingContext::default().with_full_genesis();
        assert_eq!(ctx.sdk.context().block_number(), 0);
        const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");

        // setup

        let loader_id = bpf_loader_upgradeable::id();
        // let loader_id = loader_v4::id();

        let account_with_program = load_program_account_from_elf_file(
            &loader_id,
            // "../solana-ee-core/crates/examples/hello-world/assets/solana_program.so",
            "../solana-ee-core/crates/core/test_elfs/out/noop_aligned.so",
        );

        // init buffer, fill buffer, deploy

        let program_bytes = account_with_program.data().to_vec();
        ctx.add_balance(DEPLOYER_ADDRESS, U256::from(1e18));
        let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, program_bytes.into());
        println!("contract_address {:x?}", contract_address);

        // exec

        let pk_exec = pubkey_from_address(contract_address);
        println!("test: pk_exec {:x?}", &pk_exec.as_ref());

        let instructions = vec![Instruction::new_with_bincode(
            pk_exec.clone(),
            &[0u8; 0],
            vec![],
        )];
        let message = Message::new(&instructions, Some(&pk_exec));
        let mut batch_message = BatchMessage::new(None);
        batch_message.clear().append_one(message);
        let input = bincode::serialize(&batch_message).unwrap();
        println!("input.len {} input '{:?}'", input.len(), input.as_slice());
        ctx.sdk = ctx.sdk.with_block_number(1);
        assert_eq!(ctx.sdk.context().block_number(), 1);
        let result = ctx.call_evm_tx(DEPLOYER_ADDRESS, contract_address, input.into(), None, None);
        let output = result.output().unwrap_or_default();
        assert!(result.is_success());
        let expected_output = hex!("");
        assert_eq!(hex::encode(expected_output), hex::encode(output));
    }
}
