#[cfg(test)] // TODO recover this tests?
pub(crate) mod tests {
    use crate::{
        account::{AccountSharedData, WritableAccount},
        builtins::register_builtins,
        common::TestSdkType,
        compute_budget::{compute_budget::ComputeBudget, ComputeBudget},
        context::{EnvironmentConfig, InvokeContext, TransactionContext},
        declare_process_instruction,
        error::{InstructionError, TransactionError},
        feature_set::FeatureSet,
        helpers::INSTRUCTION_METER_BUDGET,
        loaded_programs::{
            LoadedProgram,
            LoadedProgramsForTxBatch,
            ProgramCacheForTxBatch,
            ProgramRuntimeEnvironments,
        },
        message_processor::MessageProcessor,
        native_loader,
        native_loader::create_loadable_account_for_test,
        secp256k1_instruction::new_secp256k1_instruction,
        serialization::serialize_parameters_aligned_custom,
        solana_program::{feature_set::feature_set_default, sysvar_cache::SysvarCache},
        sysvar_cache::SysvarCache,
        test_helpers::journal_state,
    };
    use alloc::{format, sync::Arc, vec, vec::Vec};
    use fluentbase_sdk::SharedAPI;
    use serde::Deserialize;
    use solana_program::{
        account_info::AccountInfo,
        clock::{Epoch, Slot},
        entrypoint::deserialize,
        hash::Hash,
        instruction::Instruction,
        message::{LegacyMessage, Message, SanitizedMessage},
        pubkey::Pubkey,
        secp256k1_program,
    };
    use solana_rbpf::{
        ebpf,
        elf::Executable,
        error::ProgramResult,
        memory_region::MemoryRegion,
        program::{BuiltinFunction, BuiltinProgram, FunctionRegistry},
        verifier::RequisiteVerifier,
        vm::Config,
    };

    #[test]
    fn serde_test() {
        let program_id = Pubkey::new_from_array([0xcu8; 32]);
        let instruction_data: &[u8] = &[1, 2, 3, 4];

        let account1_key = Pubkey::new_from_array([1u8; 32]);
        let account1_owner = Pubkey::new_from_array([2u8; 32]);
        let mut account1_lamports = 11;
        let mut account1_data = vec![3, 2, 1];
        let account1_rent_epoch = Epoch::default();
        let account1 = AccountInfo::new(
            &account1_key,
            true,
            false,
            &mut account1_lamports,
            &mut account1_data,
            &account1_owner,
            false,
            account1_rent_epoch,
        );
        let accounts: Vec<AccountInfo> = vec![account1];

        let mut init =
            serialize_parameters_aligned_custom(&accounts, &instruction_data, &program_id)
                .expect("failed to serialize");
        let deser = unsafe { deserialize(init.as_mut_ptr()) };

        assert_eq!(accounts[0].key, deser.1[0].key);
        assert_eq!(accounts[0].owner, deser.1[0].owner);
        assert_eq!(accounts[0].rent_epoch, deser.1[0].rent_epoch);
        assert_eq!(accounts[0].lamports, deser.1[0].lamports);
        assert_eq!(accounts[0].data, deser.1[0].data);
        assert_eq!(accounts[0].executable, deser.1[0].executable);
        assert_eq!(accounts[0].is_signer, deser.1[0].is_signer);
        assert_eq!(accounts[0].is_writable, deser.1[0].is_writable);
    }

    #[test]
    fn test_elf_execution() {
        // This tests checks that a struct field adjacent to another field
        // which is a relocatable function pointer is not overwritten when
        // the function pointer is relocated at load time.
        let config = Config {
            enable_instruction_tracing: false,
            reject_broken_elfs: true,
            sanitize_user_provided_values: true,
            ..Default::default()
        };
        let solana_elf_file_name = "solana_ee_hello_world";
        let elf_bytes = std::fs::read(format!(
            "../examples/solana-program/assets/{}.so",
            solana_elf_file_name
        ))
        .unwrap();

        let sdk = journal_state();

        let writable_pubkey = Pubkey::new_unique();
        let readonly_pubkey = Pubkey::new_unique();
        let mock_system_program_id = Pubkey::new_unique();

        let accounts = vec![
            (
                writable_pubkey,
                AccountSharedData::new(100, 1, &mock_system_program_id),
            ),
            (
                readonly_pubkey,
                AccountSharedData::new(0, 1, &mock_system_program_id),
            ),
            (
                mock_system_program_id,
                create_loadable_account_for_test("mock_system_program", &native_loader::id()),
            ),
        ];
        let transaction_context = TransactionContext::new(
            accounts,
            #[allow(clippy::clone_on_copy)]
            Default::default(),
            1,
            3,
        );

        let mut function_registry =
            FunctionRegistry::<BuiltinFunction<InvokeContext<TestSdkType>>>::default();
        register_builtins(&mut function_registry);
        let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));

        let executable_elf =
            Executable::<InvokeContext<TestSdkType>>::from_elf(&elf_bytes, loader.clone()).unwrap();

        let programs_cache_for_tx_batch = ProgramCacheForTxBatch::new2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        let compute_budget = ComputeBudget::default();
        let sysvar_cache = SysvarCache::default();
        let environment_config = EnvironmentConfig::new(
            blockhash,
            None,
            Arc::new(feature_set_default()),
            0,
            sysvar_cache,
        );

        let mut invoke_context = InvokeContext::new(
            transaction_context,
            programs_cache_for_tx_batch,
            environment_config,
            compute_budget,
            &sdk,
        );

        let expected_result = format!("{:?}", ProgramResult::Ok(0x0));
        if !expected_result.contains("ExceededMaxInstructions") {
            invoke_context.mock_set_remaining(INSTRUCTION_METER_BUDGET);
        }
        executable_elf.verify::<RequisiteVerifier>().unwrap();

        let account1_key = Pubkey::new_from_array([1u8; 32]);
        let account1_owner = Pubkey::new_from_array([2u8; 32]);
        let mut account1_lamports = 11;
        let mut account1_data = vec![3, 2, 1];
        let account1_rent_epoch = Epoch::default();
        let account1 = AccountInfo::new(
            &account1_key,
            true,
            false,
            &mut account1_lamports,
            &mut account1_data,
            &account1_owner,
            false,
            account1_rent_epoch,
        );
        let accounts: Vec<AccountInfo> = vec![account1];

        let program_id = Pubkey::new_from_array([0xcu8; 32]);
        let instruction_data: &[u8] = &[1, 2, 3, 4];

        let (interpreter_instruction_count, interpreter_final_pct) = {
            let mut mem = vec![0u8; 1024 * 1024];
            let mut init =
                serialize_parameters_aligned_custom(&accounts, &instruction_data, &program_id)
                    .expect("failed to serialize");
            mem[..init.len()].copy_from_slice(&init);

            let mem_region = MemoryRegion::new_writable(&mut mem, ebpf::MM_INPUT_START);

            crate::create_vm!(vm, &executable_elf, mem_region, &mut invoke_context,);
            vm.registers;

            let (interpreter_instruction_count, result) = vm.execute_program(&executable_elf, true);

            assert_eq!(
                expected_result,
                format!("{:?}", result),
                "Unexpected result for executed program"
            );
            (interpreter_instruction_count, vm.registers[11])
        };
    }

    #[test]
    fn test_precompile() {
        let sdk = journal_state();

        let config = Config {
            enable_instruction_tracing: false,
            reject_broken_elfs: true,
            sanitize_user_provided_values: true,
            ..Default::default()
        };

        let function_registry =
            FunctionRegistry::<BuiltinFunction<InvokeContext<TestSdkType>>>::default();
        let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));

        let mock_program_id = Pubkey::new_unique();

        declare_process_instruction!(MockBuiltin<SDK: SharedAPI>, 1, |_invoke_context| {
            Err(InstructionError::Custom(0xbabb1e))
        });

        let mut secp256k1_account = AccountSharedData::new(1, 0, &native_loader::id());
        secp256k1_account.set_executable(true);
        let mut mock_program_account = AccountSharedData::new(1, 0, &native_loader::id());
        mock_program_account.set_executable(true);
        let accounts = vec![
            (secp256k1_program::id(), secp256k1_account),
            (mock_program_id, mock_program_account),
        ];
        let transaction_context = TransactionContext::new(accounts, 1, 2);

        // Since libsecp256k1 is still using the old version of rand, this test
        // copies the `random` implementation at:
        // https://docs.rs/libsecp256k1/latest/src/libsecp256k1/lib.rs.html#430
        let secret_key = {
            use rand::RngCore;
            let mut rng = rand::thread_rng();
            loop {
                let mut ret = [0u8; libsecp256k1::util::SECRET_KEY_SIZE];
                rng.fill_bytes(&mut ret);
                if let Ok(key) = libsecp256k1::SecretKey::parse(&ret) {
                    break key;
                }
            }
        };
        let message = SanitizedMessage::Legacy(LegacyMessage::new(Message::new(
            &[
                new_secp256k1_instruction(&secret_key, b"hello"),
                Instruction::new_with_bytes(mock_program_id, &[], vec![]),
            ],
            None,
        )));
        let mut programs_loaded_for_tx_batch = LoadedProgramsForTxBatch::partial_default2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        programs_loaded_for_tx_batch.replenish(
            mock_program_id,
            Arc::new(LoadedProgram::new_builtin(0, 0, MockBuiltin::vm)),
        );
        let programs_modified_by_tx = LoadedProgramsForTxBatch::partial_default2(
            Default::default(),
            ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            },
        );
        let feature_set = FeatureSet::all_enabled();
        let compute_budget = ComputeBudget::default();
        let sysvar_cache = SysvarCache::default();
        let mut invoke_context = InvokeContext::new(
            transaction_context,
            sysvar_cache,
            &sdk,
            compute_budget,
            programs_loaded_for_tx_batch,
            programs_modified_by_tx,
            Arc::new(feature_set),
            Hash::default(),
            0,
        );
        let result =
            MessageProcessor::process_message(&message, &[vec![0], vec![1]], &mut invoke_context);

        assert_eq!(
            result,
            Err(TransactionError::InstructionError(
                1,
                InstructionError::Custom(0xbabb1e)
            ))
        );
        // assert_eq!(transaction_context.get_instruction_trace_length(), 2);
    }
}
