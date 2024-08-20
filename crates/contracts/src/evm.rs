use alloc::vec::Vec;
use core::mem::take;
use fluentbase_core::{
    debug_log,
    helpers::{evm_error_from_exit_code, exit_code_from_evm_error},
};
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{derive_keccak256, Contract},
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
        unreachable!("not supported");
        Some((U256::ZERO, false))
    }

    fn code(&mut self, address: Address) -> Option<(Bytes, bool)> {
        unreachable!("not supported");
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
        unreachable!("not supported");
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

    fn tload(&mut self, _address: Address, _index: U256) -> U256 {
        unreachable!("not supported");
    }

    fn tstore(&mut self, _address: Address, _index: U256, _value: U256) {
        unreachable!("not supported");
    }

    fn log(&mut self, mut log: Log) {
        self.sdk
            .emit_log(take(&mut log.data.data), log.data.topics());
    }

    fn selfdestruct(&mut self, _address: Address, target: Address) -> Option<SelfDestructResult> {
        self.sdk.destroy_account(target);
        Some(SelfDestructResult::default())
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
        let evm_bytecode_slot = U256::from_le_bytes(derive_keccak256!(keccak256("_evm_bytecode")));
        let bytecode_len: usize = self.sdk.storage(&evm_bytecode_slot).as_limbs()[0] as usize;
        let mut bytecode = Vec::with_capacity(bytecode_len);
        let chunks_num = (bytecode_len + 31) / 32;
        for i in 0..chunks_num {
            let slot_i = evm_bytecode_slot + U256::from(i + 1);
            let chunk = self.sdk.storage(&slot_i);
            bytecode.extend_from_slice(chunk.to_le_bytes::<32>().as_ref());
        }
        bytecode.resize(bytecode_len, 0);
        let evm_bytecode_hash_slot =
            U256::from_le_bytes(derive_keccak256!(keccak256("_evm_bytecode_hash")));
        let code_hash = self.sdk.storage(&evm_bytecode_hash_slot);
        let evm_bytecode = Bytecode::new_raw(Bytes::from(bytecode));
        let code_hash = B256::from(code_hash.to_le_bytes::<32>());
        debug_assert_eq!(evm_bytecode.hash_slow(), code_hash);
        (evm_bytecode, code_hash)
    }

    pub fn store_evm_bytecode(&mut self, bytecode: Bytecode) {
        let evm_bytecode_slot = U256::from_le_bytes(derive_keccak256!(keccak256("_evm_bytecode")));
        self.sdk
            .write_storage(evm_bytecode_slot, U256::from(bytecode.len()));
        for (i, chunk) in bytecode.original_byte_slice().chunks(32).enumerate() {
            if chunk.len() < 32 {
                let mut padded_chunk = [0u8; 32];
                padded_chunk[0..chunk.len()].copy_from_slice(chunk);
                self.sdk.write_storage(
                    evm_bytecode_slot + U256::from(i + 1),
                    U256::from_le_slice(&padded_chunk),
                );
            } else {
                self.sdk.write_storage(
                    evm_bytecode_slot + U256::from(i + 1),
                    U256::from_le_slice(chunk),
                );
            }
        }
        let evm_bytecode_hash_slot =
            U256::from_le_bytes(derive_keccak256!(keccak256("_evm_bytecode_hash")));
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
                        CallScheme::Call => self.sdk.call(
                            inputs.target_address,
                            inputs.value.transfer().unwrap_or_default(),
                            inputs.input.as_ref(),
                            inputs.gas_limit,
                        ),
                        CallScheme::CallCode => unreachable!(),
                        CallScheme::DelegateCall => self.sdk.delegate_call(
                            inputs.target_address,
                            inputs.input.as_ref(),
                            inputs.gas_limit,
                        ),
                        CallScheme::StaticCall => self.sdk.static_call(
                            inputs.target_address,
                            inputs.input.as_ref(),
                            inputs.gas_limit,
                        ),
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
        self.exec_evm_bytecode(contract, Gas::new(self.sdk.fuel()), false, 0)
    }

    pub fn deploy(&mut self, contract_context: ContractContext) -> ExitCode {
        let init_code = self.sdk.input();
        let contract = Contract {
            input: Bytes::default(),
            bytecode: to_analysed(Bytecode::new_raw(init_code)),
            hash: None,
            target_address: contract_context.address,
            caller: contract_context.caller,
            call_value: contract_context.value,
        };
        let result = self.exec_evm_bytecode(contract, Gas::new(self.sdk.fuel()), false, 0);
        if !result.is_ok() {
            // it might be an error message, have to return
            self.sdk.write(result.output.as_ref());
            // exit with corresponding error code
            return exit_code_from_evm_error(result.result);
        }
        self.store_evm_bytecode(Bytecode::new_raw(result.output));
        ExitCode::Ok
    }
}

#[derive(Contract)]
pub struct EvmLoaderEntrypoint<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> EvmLoaderEntrypoint<SDK> {
    pub fn deploy(&mut self) {
        let exit_code = self.deploy_inner();
        self.sdk.exit(exit_code.into_i32());
    }

    pub(crate) fn deploy_inner(&mut self) -> ExitCode {
        let contract_context = self.sdk.contract_context().clone();
        EvmLoader2::new(&mut self.sdk).deploy(contract_context)
    }

    pub fn main(&mut self) {
        let exit_code = self.main_inner();
        self.sdk.exit(exit_code.into_i32());
    }

    pub(crate) fn main_inner(&mut self) -> ExitCode {
        let contract_context = self.sdk.contract_context().clone();
        let result = EvmLoader2::new(&mut self.sdk).call(contract_context);
        self.sdk.write(result.output.as_ref());
        exit_code_from_evm_error(result.result)
    }
}

basic_entrypoint!(EvmLoaderEntrypoint);

#[cfg(test)]
mod tests {
    use super::*;
    use core::str::from_utf8;
    use fluentbase_sdk::{
        journal::JournalStateBuilder,
        runtime::TestingContext,
        Address,
        ContractContext,
        U256,
    };
    use revm_primitives::hex;

    #[test]
    fn test_evm_store_load() {
        let native_sdk = TestingContext::empty();
        let mut sdk = JournalStateBuilder::default()
            .with_contract_context(ContractContext {
                address: Address::from([
                    189, 119, 4, 22, 163, 52, 95, 145, 228, 179, 69, 118, 203, 128, 74, 87, 111,
                    164, 142, 177,
                ]),
                caller: Address::ZERO,
                value: U256::ZERO,
            })
            .build(native_sdk.clone());
        let mut evm_loader = EvmLoader2::new(&mut sdk);
        let bytecode = hex!("60806040526105ae806100115f395ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e0");
        let bytecode = Bytecode::new_raw(bytecode.into());
        evm_loader.store_evm_bytecode(bytecode.clone());
        let (bytecode2, code_hash) = evm_loader.load_evm_bytecode();
        assert_eq!(bytecode.clone(), bytecode2);
        assert_eq!(bytecode.hash_slow(), code_hash);
        assert_eq!(bytecode2.hash_slow(), code_hash);
    }

    #[test]
    fn test_deploy_greeting() {
        let mut native_sdk = TestingContext::empty();
        let sdk = JournalStateBuilder::default()
            .with_contract_context(ContractContext {
                address: Address::from([
                    189, 119, 4, 22, 163, 52, 95, 145, 228, 179, 69, 118, 203, 128, 74, 87, 111,
                    164, 142, 177,
                ]),
                caller: Address::ZERO,
                value: U256::ZERO,
            })
            .build(native_sdk.clone());
        let mut app = EvmLoaderEntrypoint::new(sdk);
        // deploy
        {
            native_sdk.set_input(hex!("60806040526105ae806100115f395ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e01c80633b2e97481461004357806345773e4e1461007357806348b8bcc314610091575b5f80fd5b61005d600480360381019061005891906102e5565b6100af565b60405161006a919061039a565b60405180910390f35b61007b6100dd565b604051610088919061039a565b60405180910390f35b61009961011a565b6040516100a6919061039a565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100d58161012f565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101298161012f565b91505090565b60605f8203610175576040518060400160405280600181526020017f30000000000000000000000000000000000000000000000000000000000000008152509050610282565b5f8290505f5b5f82146101a457808061018d906103f0565b915050600a8261019d9190610464565b915061017b565b5f8167ffffffffffffffff8111156101bf576101be610494565b5b6040519080825280601f01601f1916602001820160405280156101f15781602001600182028036833780820191505090505b5090505b5f851461027b578180610207906104c1565b925050600a8561021791906104e8565b60306102239190610518565b60f81b8183815181106102395761023861054b565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a856102749190610464565b94506101f5565b8093505050505b919050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102b48261028b565b9050919050565b6102c4816102aa565b81146102ce575f80fd5b50565b5f813590506102df816102bb565b92915050565b5f602082840312156102fa576102f9610287565b5b5f610307848285016102d1565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f5b8381101561034757808201518184015260208101905061032c565b5f8484015250505050565b5f601f19601f8301169050919050565b5f61036c82610310565b610376818561031a565b935061038681856020860161032a565b61038f81610352565b840191505092915050565b5f6020820190508181035f8301526103b28184610362565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f6103fa826103e7565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff820361042c5761042b6103ba565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f61046e826103e7565b9150610479836103e7565b92508261048957610488610437565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104cb826103e7565b91505f82036104dd576104dc6103ba565b5b600182039050919050565b5f6104f2826103e7565b91506104fd836103e7565b92508261050d5761050c610437565b5b828206905092915050565b5f610522826103e7565b915061052d836103e7565b9250828201905080821115610545576105446103ba565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea2646970667358221220feebf5ace29c3c3146cb63bf7ca9009c2005f349075639d267cfbd817adde3e564736f6c63430008180033"));
            let exit_code = app.deploy_inner();
            assert_eq!(exit_code, ExitCode::Ok);
        }
        // main
        {
            native_sdk.set_input(hex!("45773e4e"));
            let exit_code = app.main_inner();
            assert_eq!(exit_code, ExitCode::Ok);
            let bytes = &native_sdk.take_output()[64..75];
            assert_eq!("Hello World", from_utf8(bytes.as_ref()).unwrap());
        }
    }
}
