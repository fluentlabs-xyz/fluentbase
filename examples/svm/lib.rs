#![cfg_attr(target_arch = "wasm32", no_std)]
// #![no_std]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::{format, vec, vec::Vec};
use fluentbase_core::debug_log;
use fluentbase_sdk::{basic_entrypoint, derive::Contract, journal::JournalState, SharedAPI};
use hex_literal::hex;
use solana_ee_core::{
    context::ExecContextObject,
    create_vm,
    helpers::{
        serialize_parameters_aligned,
        translate_string_and_do,
        Error as SolanaError,
        SyscallAbort,
        SyscallMemcpy,
        SyscallStubInterceptor,
        INSTRUCTION_METER_BUDGET,
    },
};
use solana_program::{account_info::AccountInfo, clock::Epoch, pubkey::Pubkey};
use solana_rbpf::{
    declare_builtin_function,
    ebpf,
    elf::Executable,
    error::ProgramResult,
    memory_region::{MemoryMapping, MemoryRegion},
    program::{BuiltinFunction, BuiltinProgram, FunctionRegistry},
    verifier::RequisiteVerifier,
    vm::Config,
};

#[derive(Contract)]
struct SVM<SDK> {
    sdk: SDK,
}

declare_builtin_function!(
    /// Log a user's info message
    SyscallLog<SDK: SharedAPI>,
    fn rust(
        invoke_context: &mut ExecContextObject<SDK>,
        addr: u64,
        len: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, SolanaError> {
        // let cost = invoke_context
        //     .get_compute_budget()
        //     .syscall_base_cost
        //     .max(len);
        // consume_compute_meter(invoke_context, cost)?;

        translate_string_and_do(
            memory_mapping,
            addr,
            len,
            // invoke_context.get_check_aligned(),
            true,
            &mut |string: &str| {
                // stable_log::program_log(&invoke_context.get_log_collector(), string);
                #[cfg(all(feature = "std", feature = "debug-print"))]
                println!("Log: {string}");
                debug_log!("SyscallLog: hi, there!");
                panic!("panic in: SyscallLog");
                Ok(0)
            },
        )?;
        Ok(0)
    }
);

impl<SDK: SharedAPI> SVM<SDK> {
    fn deploy(&mut self) {
        // any custom deployment logic here
    }
    fn main(&mut self) {
        // This tests checks that a struct field adjacent to another field
        // which is a relocatable function pointer is not overwritten when
        // the function pointer is relocated at load time.
        let config = Config {
            enable_instruction_tracing: false,
            reject_broken_elfs: true,
            // sanitize_user_provided_values: true,
            ..Config::default()
        };
        let input = self.sdk.input();
        let elf_bytes = input.to_vec();

        let instruction_count = 0;
        let mut context_object = ExecContextObject::new(&mut self.sdk, instruction_count);

        // Holds the function symbols of an Executable
        let mut function_registry =
            FunctionRegistry::<BuiltinFunction<ExecContextObject<SDK>>>::default();
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

        let loader = alloc::sync::Arc::new(BuiltinProgram::new_loader(config, function_registry));
        let mut executable_elf =
            Executable::<ExecContextObject<SDK>>::from_elf(&elf_bytes, loader.clone()).unwrap();

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
            let mut init = serialize_parameters_aligned(&accounts, &instruction_data, &program_id)
                .expect("failed to serialize");
            mem[..init.len()].copy_from_slice(&init);

            let mem_region = MemoryRegion::new_writable(&mut mem, ebpf::MM_INPUT_START);

            create_vm!(
                vm,
                &executable_elf,
                &mut context_object,
                stack,
                heap,
                vec![mem_region],
                None
            );
            vm.registers;

            // println!(
            //     "Executing program with expected result: {}",
            //     expected_result
            // );
            let (interpreter_instruction_count, result) = vm.execute_program(&executable_elf, true);
            // println!("Execution result: {:?}", result);

            assert_eq!(
                expected_result,
                format!("{:?}", result),
                "Unexpected result for executed program"
            );
            (interpreter_instruction_count, vm.registers[11])
        };
    }
}

basic_entrypoint!(SVM);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};

    #[test]
    fn test_contract_works() {
        let native_sdk = TestingContext::empty().with_input("Hello, World");
        let sdk = JournalState::empty(native_sdk.clone());
        let mut swm = SVM::new(sdk);
        swm.deploy();
        swm.main();
        let output = native_sdk.take_output();
    }
}
