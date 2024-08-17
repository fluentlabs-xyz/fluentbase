use crate::{
    debug_log,
    evm::EvmLoader,
    fluentbase_sdk::NativeAPI,
    helpers::{evm_error_from_exit_code, exit_code_from_evm_error, wasm2rwasm},
    types::{Frame, NextAction},
};
use alloc::{boxed::Box, vec, vec::Vec};
use alloy_rlp::Encodable;
use core::mem::take;
use fluentbase_codec::Encoder;
use fluentbase_sdk::{Address, Bytes, B256, U256};
use fluentbase_types::{
    contracts::{
        SYSCALL_ID_CALL,
        SYSCALL_ID_CREATE,
        SYSCALL_ID_DELEGATE_CALL,
        SYSCALL_ID_STORAGE_READ,
        SYSCALL_ID_STORAGE_WRITE,
    },
    env_from_context,
    Account,
    AccountStatus,
    BytecodeType,
    ContractContext,
    ExitCode,
    SharedContextInputV1,
    SovereignAPI,
    SyscallInvocationParams,
    STATE_DEPLOY,
    STATE_MAIN,
};
use revm_interpreter::{
    gas,
    gas::{sstore_cost, COLD_SLOAD_COST, WARM_STORAGE_READ_COST},
    CallInputs,
    CallOutcome,
    CallScheme,
    CallValue,
    Contract,
    CreateInputs,
    CreateOutcome,
    Gas,
    Host,
    InterpreterResult,
};
use revm_primitives::{
    Bytecode,
    CreateScheme,
    Env,
    SpecId,
    MAX_CALL_STACK_LIMIT,
    MAX_INITCODE_SIZE,
    WASM_MAX_CODE_SIZE,
};

pub struct BlendedRuntime<'a, SDK> {
    sdk: &'a mut SDK,
    env: Env,
}

impl<'a, SDK: SovereignAPI> BlendedRuntime<'a, SDK> {
    pub fn new(sdk: &'a mut SDK) -> Self {
        Self {
            env: env_from_context(sdk.block_context(), sdk.tx_context()),
            sdk,
        }
    }

    fn syscall_storage_read(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        // read value from storage
        let (value, is_cold) = self.sdk.storage(
            &context.address,
            &U256::from_le_slice(params.input.as_ref()),
        );

        let gas_cost = if is_cold {
            COLD_SLOAD_COST
        } else {
            WARM_STORAGE_READ_COST
        };

        // return value as bytes with success exit code
        NextAction::ExecutionResult(
            value.to_le_bytes::<32>().into(),
            gas_cost,
            ExitCode::Ok.into_i32(),
        )
    }

    fn syscall_storage_write(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 64 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let slot = U256::from_le_slice(&params.input[0..32]);
        let value = U256::from_le_slice(&params.input[32..64]);

        let sstore_result = EvmLoader::new(self.sdk)
            .sstore(context.address, slot, value)
            .unwrap();

        let gas_cost = match sstore_cost(
            SpecId::CANCUN,
            sstore_result.original_value,
            sstore_result.present_value,
            sstore_result.new_value,
            params.fuel_limit,
            sstore_result.is_cold,
        ) {
            Some(gas_cost) => {
                if params.fuel_limit < gas_cost {
                    return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
                }
                gas_cost
            }
            None => return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas),
        };

        let _is_cold = self.sdk.write_storage(context.address, slot, value);
        NextAction::ExecutionResult(Bytes::default(), gas_cost, ExitCode::Ok.into_i32())
    }

    fn syscall_call(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        debug_assert_eq!(
            Address::len_bytes(),
            20,
            "address len doesn't match 20 bytes"
        );

        // make sure we have enough bytes inside input params, where:
        // - 20 bytes for the target address
        // - 32 bytes for the call value
        if params.input.len() < 20 + 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let target_address = Address::from_slice(&params.input[0..20]);
        let value = U256::from_le_slice(&params.input[20..52]);
        let contract_input = Bytes::copy_from_slice(&params.input[52..]);

        // execute a nested call to another binary
        let (output, gas, exit_code) = self.call_inner(
            Box::new(CallInputs {
                input: contract_input,
                return_memory_offset: Default::default(),
                gas_limit: params.fuel_limit,
                bytecode_address: target_address,
                target_address,
                caller: context.address,
                value: CallValue::Transfer(value),
                scheme: CallScheme::Call,
                is_static: false,
                is_eof: false,
            }),
            call_depth + 1,
        );

        NextAction::ExecutionResult(output, params.fuel_limit - gas.remaining(), exit_code)
    }

    fn syscall_delegate_call(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        debug_assert_eq!(
            Address::len_bytes(),
            20,
            "address len doesn't match 20 bytes"
        );

        // make sure we have enough bytes inside input params, where:
        // - 20 bytes for the target address
        if params.input.len() < 20 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let target_address = Address::from_slice(&params.input[0..20]);
        let contract_input = Bytes::copy_from_slice(&params.input[20..]);

        // execute a nested call to another binary
        let (output, gas, exit_code) = self.call_inner(
            Box::new(CallInputs {
                input: contract_input,
                return_memory_offset: Default::default(),
                gas_limit: params.fuel_limit,
                bytecode_address: target_address,
                target_address,
                caller: context.address,
                value: CallValue::Apparent(context.value.max(context.apparent_value)),
                scheme: CallScheme::Call,
                is_static: false,
                is_eof: false,
            }),
            call_depth + 1,
        );

        NextAction::ExecutionResult(output, gas.remaining(), exit_code)
    }

    fn syscall_emit_log(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() < 1 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        // read topics from input
        let topics_len = params.input[0] as usize;
        if topics_len > 4 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }
        let mut topics = Vec::new();
        if params.input.len() < 1 + topics_len * B256::len_bytes() {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }
        for i in 0..topics_len {
            let topic = &params.input
                [(1 + i * B256::len_bytes())..(1 + i * B256::len_bytes() + B256::len_bytes())];
            topics.push(B256::from_slice(topic));
        }

        // all remaining bytes are data
        let data = Bytes::copy_from_slice(&params.input[(1 + topics_len * B256::len_bytes())..]);

        // make sure we have enough gas to cover this operation
        let Some(gas_cost) = gas::log_cost(topics_len as u8, data.len() as u64) else {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
        };

        // write new log into the journal
        self.sdk.write_log(context.address, data, topics);

        NextAction::ExecutionResult(Bytes::default(), gas_cost, ExitCode::Ok.into_i32())
    }

    fn syscall_create(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        // make sure we have enough bytes inside input params, where:
        // - 1 byte for salt option flag
        // - 32 bytes for salt
        if params.input.len() < 1 + 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let is_create2 = params.input[0] != 0;
        let create_scheme = if is_create2 {
            CreateScheme::Create2 {
                salt: U256::from_le_slice(&params.input[1..33]),
            }
        } else {
            CreateScheme::Create
        };

        let init_code = Bytes::copy_from_slice(&params.input[33..]);

        // execute a nested call to another binary
        let call_outcome = self.create_inner(
            Box::new(CreateInputs {
                caller: context.address,
                scheme: create_scheme,
                // actually, "apparent" value is not possible for CREATE
                value: context.value,
                init_code,
                gas_limit: params.fuel_limit,
            }),
            call_depth + 1,
        );

        NextAction::ExecutionResult(
            call_outcome.result.output,
            call_outcome.result.gas.remaining(),
            exit_code_from_evm_error(call_outcome.result.result).into_i32(),
        )
    }

    fn process_exec_params(&mut self, exit_code: i32, fuel_spent: u64) -> NextAction {
        // if the exit code is non-positive (stands for termination), then execution is finished
        if exit_code <= 0 {
            let return_data = self.sdk.native_sdk().return_data();
            return NextAction::ExecutionResult(return_data, fuel_spent, exit_code);
        }

        // otherwise, exit code is a "call_id" that identifies saved context
        let call_id = exit_code as u32;

        // try to parse execution params, if it's not possible then return an error
        let exec_params = self.sdk.native_sdk().return_data();
        let Some(params) = SyscallInvocationParams::from_slice(exec_params.as_ref()) else {
            unreachable!("can't decode invocation params");
        };

        NextAction::NestedCall(call_id, params)
    }

    fn syscall_exec(
        &mut self,
        contract_context: &ContractContext,
        params: SyscallInvocationParams,
        return_call_id: u32,
    ) -> NextAction {
        // we don't do all these checks for root level
        // because root level is trusted and can do any calls
        if return_call_id > 0 {
            // only main state can be forwarded to the other contract as a nested call,
            // other states can be only used by root
            if params.state != STATE_MAIN {
                return NextAction::from_exit_code(
                    params.fuel_limit,
                    ExitCode::MalformedSyscallParams,
                );
            }
            // check code hashes for system calls
            match params.code_hash {
                SYSCALL_ID_STORAGE_READ => {
                    return self.syscall_storage_read(contract_context, params)
                }
                SYSCALL_ID_STORAGE_WRITE => {
                    return self.syscall_storage_write(contract_context, params)
                }
                SYSCALL_ID_CALL => return self.syscall_call(contract_context, params, 0),
                SYSCALL_ID_DELEGATE_CALL => {
                    return self.syscall_delegate_call(contract_context, params, 0)
                }
                SYSCALL_ID_CREATE => return self.syscall_create(contract_context, params, 0),
                _ => {}
            }
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        // warmup bytecode before execution,
        // it's a technical limitation we're having right now,
        // planning to solve it in the future
        #[cfg(feature = "std")]
        {
            use fluentbase_runtime::Runtime;
            let bytecode = self.sdk.preimage(&params.code_hash).unwrap_or_default();
            Runtime::warmup_bytecode(params.code_hash, bytecode);
        }

        let mut context_input = SharedContextInputV1 {
            block: self.sdk.block_context().clone(),
            tx: self.sdk.tx_context().clone(),
            contract: contract_context.clone(),
        }
            .encode_to_vec(0);
        context_input.extend_from_slice(params.input.as_ref());

        // execute smart contract
        let fuel_before = self.sdk.native_sdk().fuel();
        let exit_code = self.sdk.native_sdk().exec(
            &params.code_hash,
            &context_input,
            params.fuel_limit,
            params.state,
        );
        let fuel_spent = fuel_before - self.sdk.native_sdk().fuel();

        self.process_exec_params(exit_code, fuel_spent)
    }

    fn syscall_resume(
        &mut self,
        contract_context: &ContractContext,
        call_id: u32,
        return_data: &[u8],
        exit_code: i32,
    ) -> NextAction {
        let fuel_before = self.sdk.native_sdk().fuel();
        let exit_code = self
            .sdk
            .native_sdk()
            .resume(call_id, return_data, exit_code);
        let fuel_spent = fuel_before - self.sdk.native_sdk().fuel();
        self.process_exec_params(exit_code, fuel_spent)
    }

    fn exec_rwasm_bytecode(
        &mut self,
        context: ContractContext,
        account: &Account,
        input: &[u8],
        gas: &mut Gas,
        state: u32,
    ) -> (Bytes, i32) {
        debug_log!(
            self.sdk,
            "ecl(exec_rwasm_bytecode): executing rWASM contract={}, caller={}, gas={} input={}",
            &account.address,
            &context.caller,
            gas.remaining(),
            hex::encode(&input),
        );

        let mut call_stack: Vec<Frame> = vec![Frame::Execute(
            SyscallInvocationParams {
                code_hash: account.rwasm_code_hash,
                input: Bytes::copy_from_slice(input),
                fuel_limit: gas.remaining(),
                state,
            },
            0,
        )];

        let mut stack_frame = call_stack.last_mut().unwrap();
        let (output, gas, exit_code) = loop {
            let next_action = match stack_frame {
                Frame::Execute(params, return_call_id) => {
                    self.syscall_exec(&context, take(params), *return_call_id)
                }
                Frame::Resume(return_call_id, return_data, exit_code) => {
                    self.syscall_resume(&context, *return_call_id, return_data.as_ref(), *exit_code)
                }
            };
            match next_action {
                NextAction::ExecutionResult(return_data, fuel_spent, exit_code) => {
                    let _resumable_frame = call_stack.pop().unwrap();
                    assert!(
                        gas.record_cost(fuel_spent),
                        "not enough gas for nested call"
                    );
                    println!(
                        "result exit_code={} gas_remaining={} fuel_spent={}",
                        exit_code,
                        gas.remaining(),
                        fuel_spent
                    );
                    if call_stack.is_empty() {
                        break (return_data, gas, exit_code);
                    }
                    match call_stack.last_mut().unwrap() {
                        Frame::Resume(_, return_data_result, exit_code_result) => {
                            *return_data_result = return_data;
                            *exit_code_result = exit_code;
                        }
                        _ => unreachable!(),
                    }
                    stack_frame = call_stack.last_mut().unwrap();
                }
                NextAction::NestedCall(call_id, params) => {
                    let last_frame = call_stack.last_mut().unwrap();
                    match last_frame {
                        Frame::Execute(_, _) => {
                            *last_frame = Frame::Resume(call_id, Bytes::default(), i32::MIN);
                        }
                        Frame::Resume(call_id_result, _, _) => {
                            *call_id_result = call_id;
                        }
                    }
                    call_stack.push(Frame::Execute(params, call_id));
                    stack_frame = call_stack.last_mut().unwrap();
                }
            }
        };

        (output, exit_code)
    }

    pub fn deploy_wasm_contract(
        &mut self,
        contract: Contract,
        mut gas: Gas,
        depth: u32,
    ) -> InterpreterResult {
        let return_error = |gas: Gas, exit_code: ExitCode| -> InterpreterResult {
            InterpreterResult::new(evm_error_from_exit_code(exit_code), Bytes::new(), gas)
        };

        // translate WASM to rWASM
        let rwasm_bytecode = match wasm2rwasm(contract.bytecode.original_byte_slice()) {
            Ok(rwasm_bytecode) => rwasm_bytecode,
            Err(exit_code) => {
                return return_error(gas, exit_code);
            }
        };

        // // record gas for each created byte
        let gas_for_code = rwasm_bytecode.len() as u64 * gas::CODEDEPOSIT;
        if !gas.record_cost(gas_for_code) {
            return return_error(gas, ExitCode::OutOfGas);
        }

        // write callee changes to a database (lets keep rWASM part empty for now since universal
        // loader is not ready yet)
        let (mut contract_account, _) = self.sdk.account(&contract.target_address);
        contract_account.update_bytecode(self.sdk, Bytes::new(), None, rwasm_bytecode.into(), None);

        // execute rWASM deploy function
        let context = ContractContext {
            address: contract.target_address,
            bytecode_address: contract.target_address,
            caller: contract.caller,
            value: contract.call_value,
            apparent_value: U256::ZERO,
        };
        let (output, exit_code) =
            self.exec_rwasm_bytecode(context, &contract_account, &[], &mut gas, STATE_DEPLOY);

        InterpreterResult {
            result: evm_error_from_exit_code(ExitCode::from(exit_code)),
            output,
            gas,
        }
    }

    pub fn deploy_evm_contract(&mut self, contract: Contract, gas: Gas) -> InterpreterResult {
        EvmLoader::new(self.sdk).deploy(
            contract.caller,
            contract.target_address,
            contract.bytecode.original_bytes(),
            contract.call_value,
            gas.remaining(),
        )
    }

    fn create_inner(&mut self, inputs: Box<CreateInputs>, call_depth: u32) -> CreateOutcome {
        let return_error = |gas: Gas, exit_code: ExitCode| -> CreateOutcome {
            CreateOutcome::new(
                InterpreterResult::new(evm_error_from_exit_code(exit_code), Bytes::new(), gas),
                None,
            )
        };
        let gas = Gas::new(inputs.gas_limit);
        debug_log!(
            self.sdk,
            "ecl(_evm_create): start. gas_limit {}",
            inputs.gas_limit
        );

        // determine bytecode type
        let bytecode_type = BytecodeType::from_slice(&inputs.init_code);

        // load deployer and contract accounts
        let (mut caller_account, _) = self.sdk.account(&inputs.caller);
        if caller_account.balance < inputs.value {
            return return_error(gas, ExitCode::InsufficientBalance);
        }

        // call depth check
        if call_depth > MAX_CALL_STACK_LIMIT {
            return return_error(gas, ExitCode::CallDepthOverflow);
        }

        // check init max code size for EIP-3860
        if inputs.init_code.len()
            > match bytecode_type {
            BytecodeType::EVM => MAX_INITCODE_SIZE,
            BytecodeType::WASM => WASM_MAX_CODE_SIZE,
        }
        {
            return return_error(gas, ExitCode::ContractSizeLimit);
        }

        // calc source code hash
        let source_code_hash = self.sdk.native_sdk().keccak256(inputs.init_code.as_ref());

        // create an account
        let salt_hash = match inputs.scheme {
            CreateScheme::Create2 { salt } => Some((salt, source_code_hash)),
            CreateScheme::Create => None,
        };
        let (contract_account, checkpoint) = match Account::create_account_checkpoint(
            self.sdk,
            &mut caller_account,
            inputs.value,
            salt_hash,
        ) {
            Ok(result) => result,
            Err(exit_code) => return return_error(gas, exit_code),
        };

        debug_log!(
            self.sdk,
            "ecl(_evm_create): creating account={} balance={}",
            contract_account.address,
            hex::encode(contract_account.balance.to_be_bytes::<32>())
        );

        let contract = Contract {
            input: Bytes::new(),
            bytecode: Bytecode::new_raw(inputs.init_code),
            hash: Some(source_code_hash),
            target_address: contract_account.address,
            caller: inputs.caller,
            call_value: inputs.value,
        };

        let mut result = match bytecode_type {
            BytecodeType::EVM => self.deploy_evm_contract(contract, gas),
            BytecodeType::WASM => self.deploy_wasm_contract(contract, gas, call_depth),
        };

        debug_log!(
            self.sdk,
            "ecl(_evm_create): return: Ok: callee_account.address: {}",
            contract_account.address
        );

        // commit all changes made
        if result.result.is_ok() {
            self.sdk.commit();
        } else {
            self.sdk.rollback(checkpoint);
        }

        CreateOutcome::new(result, Some(contract_account.address))
    }

    pub fn create(&mut self, create_inputs: Box<CreateInputs>) -> CreateOutcome {
        self.create_inner(create_inputs, 0)
    }

    fn call_inner(&mut self, inputs: Box<CallInputs>, depth: u32) -> (Bytes, Gas, i32) {
        let return_error = |gas: Gas, exit_code: ExitCode| -> (Bytes, Gas, i32) {
            (Bytes::default(), gas, exit_code.into_i32())
        };
        let mut gas = Gas::new(inputs.gas_limit);
        debug_log!(
            self.sdk,
            "ecl(_evm_call): start. gas_limit {}",
            gas.remaining()
        );

        // call depth check
        if depth > MAX_CALL_STACK_LIMIT {
            return return_error(gas, ExitCode::CallDepthOverflow);
        }

        // read caller and callee
        let (mut caller_account, _) = self.sdk.account(&inputs.caller);
        let (mut callee_account, _) = self.sdk.account(&inputs.target_address);

        // create a new checkpoint position in the journal
        let checkpoint = self.sdk.checkpoint();

        // transfer funds from caller to callee
        if let Some(value) = inputs.value.transfer() {
            debug_log!(
                self.sdk,
                "ecm(_evm_call): transfer from={} to={} value={}",
                caller_account.address,
                callee_account.address,
                hex::encode(value.to_be_bytes::<32>())
            );
        }

        if caller_account.address != callee_account.address {
            let value = inputs.transfer_value().unwrap_or_default();
            // do transfer from caller to callee
            match self
                .sdk
                .transfer(&mut caller_account, &mut callee_account, value)
            {
                Err(exit_code) => return return_error(gas, exit_code),
                Ok(_) => {}
            }
            // write current account state before doing nested calls
            self.sdk
                .write_account(caller_account.clone(), AccountStatus::Modified);
            self.sdk
                .write_account(callee_account.clone(), AccountStatus::Modified);
        } else {
            let value = inputs.transfer_value().unwrap_or_default();
            // what if self-transfer amount exceeds our balance?
            if value > caller_account.balance {
                return return_error(gas, ExitCode::InsufficientBalance);
            }
            // write only one account's state since caller equals callee
            self.sdk
                .write_account(caller_account.clone(), AccountStatus::Modified);
        }

        // check is it precompile
        if let Some(result) =
            self.sdk
                .precompile(&inputs.bytecode_address, &inputs.input, gas.remaining())
        {
            // calculate total gas consumed by precompile call
            let mut gas = Gas::new(gas.remaining());
            if !gas.record_cost(gas.remaining() - result.gas_remaining) {
                return return_error(gas, ExitCode::OutOfGas);
            };
            gas.record_refund(result.gas_refund);
            // if exit code is no successful, then rollback changes, otherwise commit them
            if result.exit_code.is_ok() {
                self.sdk.commit();
            } else {
                self.sdk.rollback(checkpoint);
            }
            // map precompile execution result into EVM interpreter result
            return (result.output, gas, result.exit_code.into_i32());
        }

        // let (bytecode_account, _) = self.sdk.account(&inputs.bytecode_address);
        // let result = if bytecode_account.source_code_size > 0 {
        //     // take right bytecode depending on context params
        //     let (evm_bytecode, code_hash) = self.load_evm_bytecode(&inputs.bytecode_address);
        //     debug_log!(
        //         self.sdk,
        //         "ecl(_evm_call): source_bytecode: {}",
        //         hex::encode(evm_bytecode.original_byte_slice())
        //     );
        //
        //     // if bytecode is empty, then commit result and return empty buffer
        //     if evm_bytecode.is_empty() {
        //         self.sdk.commit();
        //         debug_log!(self.sdk, "ecl(_evm_call): empty bytecode exit_code=Ok");
        //         return return_error(gas, ExitCode::Ok);
        //     }
        //
        //     // initiate contract instance and pass it to interpreter for and EVM transition
        //     let call_value = inputs.call_value();
        //     let contract = Contract {
        //         input: inputs.input,
        //         hash: Some(code_hash),
        //         bytecode: evm_bytecode,
        //         // we don't take contract callee, because callee refers to address with bytecode
        //         target_address: inputs.target_address,
        //         // inside the contract context, we pass "apparent" value that can be different to
        //         // transfer value (it can happen for DELEGATECALL or CALLCODE opcodes)
        //         call_value,
        //         caller: caller_account.address,
        //     };
        //     self.exec_evm_bytecode(contract, gas, inputs.is_static, depth)
        // } else {
        //     let (account, _) = self.sdk.account(&inputs.target_address);
        //     self.exec_rwasm_bytecode(
        //         ContractContext {
        //             caller: inputs.caller,
        //             target_address: inputs.target_address,
        //             value: inputs.value,
        //             call_depth: depth,
        //         },
        //         &account,
        //         inputs.input.as_ref(),
        //         gas,
        //         STATE_MAIN,
        //     )
        // };

        let (account, _) = self.sdk.account(&inputs.target_address);
        let (output, exit_code) = self.exec_rwasm_bytecode(
            ContractContext {
                address: inputs.target_address,
                bytecode_address: inputs.target_address,
                caller: inputs.caller,
                value: inputs.value.transfer().unwrap_or_default(),
                apparent_value: inputs.value.apparent().unwrap_or_default(),
            },
            &account,
            inputs.input.as_ref(),
            &mut gas,
            STATE_MAIN,
        );

        if ExitCode::from(exit_code).is_ok() {
            self.sdk.commit();
        } else {
            self.sdk.rollback(checkpoint);
        }

        debug_log!(
            self.sdk,
            "ecl(_evm_call): return exit_code={:?} gas_remaining={}",
            exit_code,
            gas.remaining(),
        );

        (output, gas, exit_code)
    }

    pub fn call(&mut self, inputs: Box<CallInputs>) -> CallOutcome {
        let (output, fuel, exit_code) = self.call_inner(inputs, 0);

        let result = InterpreterResult {
            result: evm_error_from_exit_code(ExitCode::from(exit_code)),
            output,
            gas: Gas::new(fuel.remaining()),
        };
        CallOutcome::new(result, Default::default())
    }
}
