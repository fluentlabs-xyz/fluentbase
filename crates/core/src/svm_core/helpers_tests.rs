#[cfg(test)]
mod tests {
    // use crate::helpers::{/*serialize_parameters_aligned, */SyscallAbort, SyscallLog,
    // SyscallMemcpy, SyscallStubInterceptor, INSTRUCTION_METER_BUDGET};
    use fluentbase_sdk::{
        journal::{JournalState, JournalStateBuilder},
        runtime::TestingContext,
        Address,
        ContractContext,
        SharedAPI,
        U256,
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
    // use std::{io::Read, sync::Arc};
    // use solana_program::account_info::AccountInfo;
    // use solana_program::clock::Epoch;
    // use solana_program::entrypoint::deserialize;
    // use solana_program::pubkey::Pubkey;

    type SdkType = JournalState<TestingContext>;

    // fn test_builder() -> TestBuilder {
    //     TestBuilder::new(1234u64)
    // }
    fn contract_context() -> ContractContext {
        ContractContext {
            address: Address::from_slice(&[01; 20]),
            bytecode_address: Address::from_slice(&[00; 20]),
            caller: Address::from_slice(&[00; 20]),
            is_static: false,
            value: U256::default(),
        }
    }
    fn journal_state() -> JournalState<TestingContext> {
        let mut journal_state_builder = JournalStateBuilder::default();
        journal_state_builder.add_contract_context(contract_context());
        JournalState::builder(TestingContext::empty(), journal_state_builder)
    }

    /*#[test]
    fn serde_test() {
        let program_id = Pubkey::new_from_array([0xcu8; 32]);
        let instruction_data: &[u8] = &[1, 2, 3, 4];

        let account1_key = Pubkey::new_from_array([1u8; 32]);
        let account1_owner = Pubkey::new_from_array([2u8; 32]);
        let mut account1_lamports = 11;
        let mut account1_data = vec![3, 2, 1];
        let account1_rent_epoch = Epoch::default();
        let account1 = AccountInfo::new(&account1_key, true, false, &mut account1_lamports, &mut account1_data, &account1_owner, false, account1_rent_epoch);
        let accounts: Vec<AccountInfo> = vec![account1];

        let mut init = serialize_parameters_aligned(&accounts, &instruction_data, &program_id).expect("failed to serialize");
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
            // sanitize_user_provided_values: true,
            ..Config::default()
        };
        // let solana_elf_file_name = "hello_world";
        let solana_elf_file_name = "solana_ee_hello_world";
        let mut elf_bytes = std::fs::read(format!(
            "../examples/hello-world/assets/{}.so",
            solana_elf_file_name
        ))
            .unwrap();

        println!("ELF file loaded, size: {}", elf_bytes.len());

        let instruction_count = 0;
        let sdk = journal_state();
        let mut context_object = ExecContextObject::new(sdk, instruction_count);

        // Holds the function symbols of an Executable
        let mut function_registry = FunctionRegistry::<BuiltinFunction<ExecContextObject<SdkType>>>::default();
        let shunted_funcs: &[&str] = &[];
        shunted_funcs.iter().for_each(|&v| {
            function_registry
                .register_function_hashed(v, SyscallStubInterceptor::vm)
                .unwrap();
        });
        function_registry
            .register_function_hashed("sol_log_", SyscallLog::vm)
            .unwrap();
        function_registry
            .register_function_hashed("abort", SyscallAbort::vm)
            .unwrap();
        function_registry
            .register_function_hashed("sol_memcpy_", SyscallMemcpy::vm)
            .unwrap();

        // Constructs a loader built-in program
        let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));
        // Creates an executable from an ELF file
        let mut executable_elf =
            Executable::<ExecContextObject<SdkType>>::from_elf(&elf_bytes, loader.clone()).unwrap();

        let expected_result = format!("{:?}", ProgramResult::Ok(0x0));
        if !expected_result.contains("ExceededMaxInstructions") {
            context_object.remaining = INSTRUCTION_METER_BUDGET;
        }
        executable_elf.verify::<RequisiteVerifier>().unwrap();

        let account1_key = Pubkey::new_from_array([1u8; 32]);
        let account1_owner = Pubkey::new_from_array([2u8; 32]);
        let mut account1_lamports = 11;
        let mut account1_data = vec![3, 2, 1];
        let account1_rent_epoch = Epoch::default();
        let account1 = AccountInfo::new(&account1_key, true, false, &mut account1_lamports, &mut account1_data, &account1_owner, false, account1_rent_epoch);
        let accounts: Vec<AccountInfo> = vec![account1];

        let program_id = Pubkey::new_from_array([0xcu8; 32]);
        let instruction_data: &[u8] = &[1, 2, 3, 4];

        let (interpreter_instruction_count, interpreter_final_pct) = {
            let mut mem = vec![0u8; 1024 * 1024];
            let mut init = serialize_parameters_aligned(&accounts, &instruction_data, &program_id).expect("failed to serialize");
            mem[..init.len()].copy_from_slice(&init);

            let mem_region = MemoryRegion::new_writable(&mut mem, ebpf::MM_INPUT_START);

            println!("Memory region for input: {:?}", mem_region);

            crate::create_vm!(
                vm,
                &executable_elf,
                &mut context_object,
                stack,
                heap,
                vec![mem_region],
                None
            );
            vm.registers;

            println!(
                "Executing program with expected result: {}",
                expected_result
            );
            let (interpreter_instruction_count, result) = vm.execute_program(&executable_elf, true);
            println!("Execution result: {:?}", result);

            assert_eq!(
                expected_result,
                format!("{:?}", result),
                "Unexpected result for executed program"
            );
            (
                interpreter_instruction_count,
                vm.registers[11],
            )
        };
        // if executable_elf.get_config().enable_instruction_meter {
        //     assert_eq!(
        //         instruction_count, instruction_count_interpreter,
        //         "Instruction meter did not consume expected amount"
        //     );
        // }
    }*/

    // #[test]
    // fn example_syscall() {
    //     test_interpreter_and_jit_asm!(
    //         "
    //         mov r6, r1
    //         add r1, 2
    //         mov r2, 4
    //         syscall bpf_mem_frob
    //         ldxdw r0, [r6]
    //         be64 r0
    //         exit",
    //         [
    //             0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, //
    //         ],
    //         (
    //             "bpf_mem_frob" => SyscallMemFrob::vm,
    //         ),
    //         SolanaContextObject::new(7),
    //         ProgramResult::Ok(0x102292e2f2c0708),
    //     );
    // }
}
