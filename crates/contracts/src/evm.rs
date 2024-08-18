use alloc::vec::Vec;
use core::mem::take;
use fluentbase_core::{
    debug_log,
    helpers::{evm_error_from_exit_code, exit_code_from_evm_error},
};
use fluentbase_sdk::{
    b256,
    basic_entrypoint,
    derive::Contract,
    env_from_context,
    Address,
    Bytes,
    ContractContext,
    ExitCode,
    NativeAPI,
    SharedAPI,
    SovereignAPI,
    SyscallAPI,
    B256,
    U256,
};
use revm_interpreter::{
    analysis::to_analysed,
    as_usize_saturated,
    opcode::make_instruction_table,
    CallOutcome,
    CallScheme,
    Contract,
    Gas,
    Host,
    Interpreter,
    InterpreterAction,
    InterpreterResult,
    LoadAccountResult,
    SStoreResult,
    SelfDestructResult,
    SharedMemory,
};
use revm_precompile::Log;
use revm_primitives::{Bytecode, CancunSpec, Env, BLOCK_HASH_HISTORY, MAX_CALL_STACK_LIMIT};

pub struct EvmLoader2<'a, SDK> {
    sdk: &'a mut SDK,
    env: Env,
}

impl<'a, SDK: SharedAPI> Host for EvmLoader2<'a, SDK> {
    fn env(&self) -> &Env {
        &self.env
    }

    fn env_mut(&mut self) -> &mut Env {
        &mut self.env
    }

    fn load_account(&mut self, _address: Address) -> Option<LoadAccountResult> {
        Some(LoadAccountResult {
            is_cold: false,
            is_empty: false,
        })
    }

    fn block_hash(&mut self, number: U256) -> Option<B256> {
        let block_number = as_usize_saturated!(self.env().block.number);
        let requested_number = as_usize_saturated!(number);
        let Some(diff) = block_number.checked_sub(requested_number) else {
            return Some(B256::ZERO);
        };
        if diff > 0 && diff <= BLOCK_HASH_HISTORY {
            todo!("implement block hash history")
        } else {
            Some(B256::ZERO)
        }
    }

    fn balance(&mut self, _address: Address) -> Option<(U256, bool)> {
        Some((U256::ZERO, false))
    }

    fn code(&mut self, address: Address) -> Option<(Bytes, bool)> {
        // let (account, is_cold) = self.sdk.account(&address);
        // if account.is_empty() {
        //     return Some((Bytes::new(), is_cold));
        // }
        let code_hash_storage_key = self.code_hash_storage_key(&address);
        let value = self.sdk.storage(&code_hash_storage_key);
        let evm_code_hash = B256::from(value.to_le_bytes());
        Some((self.sdk.preimage(&evm_code_hash), false))
    }

    fn code_hash(&mut self, address: Address) -> Option<(B256, bool)> {
        // let (account, is_cold) = self.sdk.account(&address);
        // if account.is_empty() {
        //     return Some((B256::ZERO, is_cold));
        // }
        let code_hash_storage_key = self.code_hash_storage_key(&address);
        let value = self.sdk.storage(&code_hash_storage_key);
        let evm_code_hash = B256::from(value.to_le_bytes());
        Some((evm_code_hash, false))
    }

    fn sload(&mut self, _address: Address, index: U256) -> Option<(U256, bool)> {
        let value = self.sdk.storage(&index);
        Some((value, false))
    }

    fn sstore(&mut self, _address: Address, index: U256, new_value: U256) -> Option<SStoreResult> {
        self.sdk.write_storage(index, new_value);
        Some(SStoreResult {
            original_value: U256::ZERO,
            present_value: U256::ZERO,
            new_value,
            is_cold: false,
        })
    }

    fn tload(&mut self, address: Address, index: U256) -> U256 {
        todo!()
    }

    fn tstore(&mut self, address: Address, index: U256, value: U256) {
        todo!()
    }

    fn log(&mut self, mut log: Log) {
        self.sdk
            .emit_log(take(&mut log.data.data), log.data.topics());
    }

    fn selfdestruct(&mut self, address: Address, target: Address) -> Option<SelfDestructResult> {
        todo!()
    }
}

impl<'a, SDK: SharedAPI> EvmLoader2<'a, SDK> {
    pub fn new(sdk: &'a mut SDK) -> Self {
        Self {
            env: env_from_context(sdk.block_context(), sdk.tx_context()),
            sdk,
        }
    }

    fn code_hash_storage_key(&self, address: &Address) -> U256 {
        let mut buffer64: [u8; 64] = [0u8; 64];
        buffer64[0..32].copy_from_slice(U256::ZERO.as_le_slice());
        buffer64[44..64].copy_from_slice(address.as_slice());
        let code_hash_key = self.sdk.keccak256(&buffer64);
        U256::from_be_bytes(code_hash_key.0)
    }

    pub fn load_evm_bytecode(&self) -> (Bytecode, B256) {
        let evm_bytecode_slot = U256::from_le_bytes(
            // keccak256("_evm_bytecode")
            b256!("464f4fbe3ee516729988523173d04cc6584cc89d0b116c572a1d3defb81ff453").0,
        );
        let bytecode_len: usize = self.sdk.storage(&evm_bytecode_slot).as_limbs()[0] as usize;
        let mut bytecode = Vec::with_capacity(bytecode_len);
        let chunks_num = (bytecode_len + 31) / 32;
        for i in 0..chunks_num {
            let slot_i = evm_bytecode_slot + U256::from(i + 1);
            let chunk = self.sdk.storage(&slot_i);
            bytecode.extend_from_slice(chunk.as_le_slice());
        }
        let evm_bytecode_hash_slot = U256::from_le_bytes(
            // keccak256("_evm_bytecode_hash")
            b256!("fd8a2cf66e0f80fe20ebc0e96c0e08e69c883c792a0409d4f4f92413fb66e980").0,
        );
        let code_hash = self.sdk.storage(&evm_bytecode_hash_slot);
        (
            Bytecode::new_raw(Bytes::from(bytecode)),
            B256::from(code_hash.to_le_bytes::<32>()),
        )
    }

    pub fn store_evm_bytecode(&mut self, bytecode: Bytecode) {
        let evm_bytecode_slot = U256::from_le_bytes(
            // keccak256("_evm_bytecode")
            b256!("464f4fbe3ee516729988523173d04cc6584cc89d0b116c572a1d3defb81ff453").0,
        );
        self.sdk
            .write_storage(evm_bytecode_slot, U256::from((bytecode.len() + 31) / 32));
        for (i, chunk) in bytecode.original_byte_slice().chunks(32).enumerate() {
            self.sdk.write_storage(
                evm_bytecode_slot + U256::from(i + 1),
                U256::from_le_slice(chunk),
            );
        }
        let evm_bytecode_hash_slot = U256::from_le_bytes(
            // keccak256("_evm_bytecode_hash")
            b256!("fd8a2cf66e0f80fe20ebc0e96c0e08e69c883c792a0409d4f4f92413fb66e980").0,
        );
        self.sdk.write_storage(
            evm_bytecode_hash_slot,
            U256::from_le_bytes(bytecode.hash_slow().0),
        );
    }

    pub fn exec_evm_bytecode(
        &mut self,
        mut contract: Contract,
        gas: Gas,
        is_static: bool,
        depth: u32,
    ) -> InterpreterResult {
        if depth >= MAX_CALL_STACK_LIMIT {
            debug_log!(self.sdk, "depth limit reached: {}", depth);
        }

        // make sure bytecode is analyzed
        contract.bytecode = to_analysed(contract.bytecode);

        let instruction_table = make_instruction_table::<Self, CancunSpec>();

        let mut interpreter = Interpreter::new(contract, gas.remaining(), is_static);
        let mut shared_memory = SharedMemory::new();

        loop {
            // run EVM bytecode to produce next action
            let next_action = interpreter.run(shared_memory, &instruction_table, self);

            // take memory and cr from interpreter and host back (return later)
            shared_memory = interpreter.take_memory();

            match next_action {
                InterpreterAction::Call { inputs } => {
                    let return_memory_offset = inputs.return_memory_offset.clone();

                    let (output, exit_code) = match inputs.scheme {
                        CallScheme::Call => self.sdk.native_sdk().syscall_call(
                            inputs.gas_limit,
                            inputs.target_address,
                            inputs.value.transfer().unwrap_or_default(),
                            inputs.input.as_ref(),
                        ),
                        CallScheme::CallCode => unreachable!(),
                        CallScheme::DelegateCall => self.sdk.native_sdk().syscall_delegate_call(
                            inputs.gas_limit,
                            inputs.target_address,
                            inputs.input.as_ref(),
                        ),
                        CallScheme::StaticCall => unreachable!(),
                    };

                    let result = InterpreterResult::new(
                        evm_error_from_exit_code(ExitCode::from(exit_code)),
                        output,
                        gas,
                    );
                    let call_outcome = CallOutcome::new(result, return_memory_offset);
                    interpreter.insert_call_outcome(&mut shared_memory, call_outcome);
                }
                InterpreterAction::Create { inputs } => {
                    unreachable!();
                    // let create_outcome = self.create_inner(inputs, depth + 1);
                    // interpreter.insert_create_outcome(create_outcome);
                }
                InterpreterAction::Return { result } => {
                    return result;
                }
                InterpreterAction::None => unreachable!("not supported EVM interpreter state"),
                InterpreterAction::EOFCreate { .. } => {
                    unreachable!("not supported EVM interpreter state: EOF")
                }
            }
        }
    }

    pub fn call(&mut self, contract_context: ContractContext) -> InterpreterResult {
        let input = self.sdk.input();
        let (evm_bytecode, _code_hash) = self.load_evm_bytecode();
        let contract = Contract {
            input,
            bytecode: to_analysed(evm_bytecode),
            hash: None,
            target_address: contract_context.address,
            caller: contract_context.caller,
            call_value: contract_context.value,
        };
        self.exec_evm_bytecode(contract, Gas::new(self.sdk.native_sdk().fuel()), false, 0)
    }

    pub fn deploy(&mut self, contract_context: ContractContext) {
        let init_code = self.sdk.input();
        let contract = Contract {
            input: Bytes::default(),
            bytecode: to_analysed(Bytecode::new_raw(init_code)),
            hash: None,
            target_address: contract_context.address,
            caller: contract_context.caller,
            call_value: contract_context.value,
        };
        let result = self.exec_evm_bytecode(contract, Gas::new(0), false, 0);
        if !result.is_ok() {
            // it might be an error message, have to return
            self.sdk.write(result.output.as_ref());
            // exit with corresponding error code
            let exit_code = exit_code_from_evm_error(result.result);
            self.sdk.exit(exit_code.into_i32());
        }
        self.store_evm_bytecode(Bytecode::new_raw(result.output));
    }
}

#[derive(Contract)]
pub struct EvmLoaderEntrypoint<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> EvmLoaderEntrypoint<SDK> {
    pub fn deploy(&mut self) {
        let contract_context = self.sdk.contract_context().clone();
        EvmLoader2::new(&mut self.sdk).deploy(contract_context);
    }

    pub fn main(&mut self) {
        let contract_context = self.sdk.contract_context().clone();
        EvmLoader2::new(&mut self.sdk).call(contract_context);
    }
}

basic_entrypoint!(EvmLoaderEntrypoint);
