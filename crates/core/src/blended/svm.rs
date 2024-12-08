use crate::{
    blended::{
        svm_common::{Keccak256Hasher, PoseidonHasher, Sha256Hasher},
        svm_syscalls::{
            SyscallHash,
            SyscallLog,
            SyscallMemmove,
            SyscallMemset,
            SyscallSecp256k1Recover,
        },
        BlendedRuntime,
    },
    helpers::evm_error_from_exit_code,
};
use alloc::{boxed::Box, format, vec};
use fluentbase_sdk::{Account, Address, Bytes, ContractContext, ExitCode, SovereignAPI, B256};
use revm_interpreter::{gas, CreateInputs, Gas, InstructionResult, InterpreterResult};
use revm_primitives::SOLANA_MAX_CODE_SIZE;
use solana_ee_core::{
    context::ExecContextObject,
    create_vm,
    helpers::{exit_code_from_svm_result, SyscallAbort, SyscallMemcpy, INSTRUCTION_METER_BUDGET},
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

impl<SDK: SovereignAPI> BlendedRuntime<SDK> {
    pub fn load_svm_bytecode(&self, address: &Address) -> (Bytes, B256) {
        let (account, _) = self.sdk.account(&address);
        let bytecode = self
            .sdk
            .preimage(address, &account.code_hash)
            .unwrap_or_default();
        (bytecode, account.code_hash)
    }

    pub fn store_svm_bytecode(&mut self, address: &Address, code_hash: B256, bytecode: Bytes) {
        self.sdk.write_preimage(*address, code_hash, bytecode);
    }

    pub fn exec_svm_bytecode(
        &mut self,
        context: ContractContext,
        _bytecode_account: &Account,
        input: Bytes,
        _gas: &mut Gas,
        _state: u32,
        _call_depth: u32,
    ) -> (Bytes, i32) {
        // take right bytecode depending on context params
        let (svm_bytecode, _code_hash) = self.load_svm_bytecode(&context.bytecode_address);

        // if bytecode is empty, then commit result and return empty buffer
        if svm_bytecode.is_empty() {
            return (Bytes::default(), ExitCode::Ok.into_i32());
        }

        let result = self.exec_svm_contract(&svm_bytecode, &input);
        if result.is_err() {}
        (Bytes::new(), exit_code_from_svm_result(result).into_i32())
    }

    pub fn exec_svm_contract(&mut self, svm_program: &[u8], svm_input: &[u8]) -> ProgramResult {
        let config = Config {
            enable_instruction_tracing: false,
            reject_broken_elfs: true,
            // sanitize_user_provided_values: true,
            ..Config::default()
        };

        let instruction_count = 0;
        let mut context_object = ExecContextObject::new(&mut self.sdk, instruction_count);

        // Holds the function symbols of an Executable
        let mut function_registry =
            FunctionRegistry::<BuiltinFunction<ExecContextObject<SDK>>>::default();
        function_registry
            // .register_function_hashed("sol_log_", SyscallLog::vm)
            .register_function_hashed("sol_log_", SyscallLog::vm)
            .unwrap();
        function_registry
            .register_function_hashed("abort", SyscallAbort::vm)
            .unwrap();
        function_registry
            .register_function_hashed("sol_memset_", SyscallMemset::vm)
            .unwrap();
        function_registry
            .register_function_hashed("sol_memcpy_", SyscallMemcpy::vm)
            .unwrap();
        function_registry
            .register_function_hashed("sol_memmove_", SyscallMemmove::vm)
            .unwrap();
        function_registry
            .register_function_hashed("sol_sha256", SyscallHash::vm::<SDK, Sha256Hasher>)
            .unwrap();
        function_registry
            .register_function_hashed(
                "sol_keccak256",
                SyscallHash::vm::<SDK, Keccak256Hasher<SDK>>,
            )
            .unwrap();
        function_registry
            .register_function_hashed("sol_secp256k1_recover", SyscallSecp256k1Recover::vm)
            .unwrap();
        function_registry
            .register_function_hashed("sol_poseidon", SyscallHash::vm::<SDK, PoseidonHasher<SDK>>)
            .unwrap();

        let loader = alloc::sync::Arc::new(BuiltinProgram::new_loader(config, function_registry));
        let mut executable_elf =
            Executable::<ExecContextObject<SDK>>::from_elf(&svm_program, loader.clone()).unwrap();

        let expected_result = format!("{:?}", ProgramResult::Ok(0x0));
        if !expected_result.contains("ExceededMaxInstructions") {
            context_object.remaining = INSTRUCTION_METER_BUDGET;
        }
        executable_elf.verify::<RequisiteVerifier>().unwrap();

        let mut mem = vec![0u8; 1024 * 1024];
        mem[..svm_input.len()].copy_from_slice(svm_input);

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

        let (_, result) = vm.execute_program(&executable_elf, true);
        result
    }

    pub fn deploy_svm_contract(
        &mut self,
        target_address: Address,
        inputs: Box<CreateInputs>,
        mut gas: Gas,
        _call_depth: u32,
    ) -> InterpreterResult {
        let return_error = |gas: Gas, exit_code: ExitCode| -> InterpreterResult {
            InterpreterResult::new(evm_error_from_exit_code(exit_code), Bytes::new(), gas)
        };

        if inputs.init_code.len() > SOLANA_MAX_CODE_SIZE {
            return return_error(gas, ExitCode::ContractSizeLimit);
        }

        // record gas for each created byte
        let gas_for_code = inputs.init_code.len() as u64 * gas::CODEDEPOSIT;
        if !gas.record_cost(gas_for_code) {
            return return_error(gas, ExitCode::OutOfGas);
        }

        // write callee changes to a database (lets keep rWASM part empty for now since universal
        // loader is not ready yet)
        let (mut contract_account, _) = self.sdk.account(&target_address);
        contract_account.update_bytecode(&mut self.sdk, inputs.init_code, None);

        InterpreterResult {
            result: InstructionResult::Stop,
            output: Bytes::new(),
            gas,
        }
    }
}
