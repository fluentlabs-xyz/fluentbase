use core::mem::take;
use fluentbase_sdk::{
    basic_entrypoint,
    debug_log,
    derive::{derive_keccak256, Contract},
    env_from_context,
    Address,
    Bytes,
    ContractContextReader,
    ExitCode,
    SharedAPI,
    B256,
    U256,
};
use revm_interpreter::{
    analysis::to_analysed,
    as_u64_saturated,
    gas,
    opcode::make_instruction_table,
    AccountLoad,
    CallOutcome,
    CallScheme,
    Contract,
    CreateOutcome,
    Eip7702CodeLoad,
    Gas,
    Host,
    InstructionResult,
    Interpreter,
    InterpreterAction,
    InterpreterResult,
    SStoreResult,
    SelfDestructResult,
    SharedMemory,
    StateLoad,
};
use revm_precompile::Log;
use revm_primitives::{Bytecode, CancunSpec, CreateScheme, Env, BLOCK_HASH_HISTORY, MAX_CODE_SIZE};

pub struct EvmLoader<'a, SDK> {
    sdk: &'a mut SDK,
    env: Env,
    address: Address,
    last_gas_consumed: u64,
}

fn evm_error_from_exit_code(exit_code: ExitCode) -> InstructionResult {
    match exit_code {
        ExitCode::Ok => InstructionResult::Stop,
        ExitCode::Panic => InstructionResult::Revert,
        ExitCode::CallDepthOverflow => InstructionResult::CallTooDeep,
        ExitCode::InsufficientBalance => InstructionResult::OutOfFunds,
        ExitCode::OutOfGas => InstructionResult::OutOfGas,
        ExitCode::OpcodeNotFound => InstructionResult::OpcodeNotFound,
        ExitCode::WriteProtection => InstructionResult::StateChangeDuringStaticCall,
        ExitCode::InvalidEfOpcode => InstructionResult::InvalidFEOpcode,
        ExitCode::InvalidJump => InstructionResult::InvalidJump,
        // ExitCode::NotActivated => InstructionResult::NotActivated,
        ExitCode::StackUnderflow => InstructionResult::StackUnderflow,
        ExitCode::StackOverflow => InstructionResult::StackOverflow,
        ExitCode::OutputOverflow => InstructionResult::OutOfOffset,
        ExitCode::CreateCollision => InstructionResult::CreateCollision,
        ExitCode::OverflowPayment => InstructionResult::OverflowPayment,
        ExitCode::PrecompileError => InstructionResult::PrecompileError,
        ExitCode::NonceOverflow => InstructionResult::NonceOverflow,
        ExitCode::ContractSizeLimit => InstructionResult::CreateContractSizeLimit,
        ExitCode::CreateContractStartingWithEF => InstructionResult::CreateContractStartingWithEF,
        ExitCode::FatalExternalError => InstructionResult::FatalExternalError,
        // TODO(dmitry123): "what's proper unknown error code mapping?"
        _ => InstructionResult::OutOfGas,
    }
}

fn exit_code_from_evm_error(evm_error: InstructionResult) -> ExitCode {
    match evm_error {
        InstructionResult::Continue
        | InstructionResult::Stop
        | InstructionResult::Return
        | InstructionResult::SelfDestruct
        | InstructionResult::CallOrCreate => ExitCode::Ok,
        InstructionResult::Revert => ExitCode::Panic,
        InstructionResult::CallTooDeep => ExitCode::CallDepthOverflow,
        InstructionResult::OutOfFunds => ExitCode::InsufficientBalance,
        InstructionResult::OutOfGas
        | InstructionResult::MemoryOOG
        | InstructionResult::MemoryLimitOOG
        | InstructionResult::PrecompileOOG
        | InstructionResult::InvalidOperandOOG => ExitCode::OutOfGas,
        InstructionResult::OpcodeNotFound => ExitCode::OpcodeNotFound,
        InstructionResult::CallNotAllowedInsideStatic
        | InstructionResult::StateChangeDuringStaticCall => ExitCode::WriteProtection,
        InstructionResult::InvalidFEOpcode => ExitCode::InvalidEfOpcode,
        InstructionResult::InvalidJump => ExitCode::InvalidJump,
        InstructionResult::StackUnderflow => ExitCode::StackUnderflow,
        InstructionResult::StackOverflow => ExitCode::StackOverflow,
        InstructionResult::OutOfOffset => ExitCode::OutputOverflow,
        InstructionResult::CreateCollision => ExitCode::CreateCollision,
        InstructionResult::OverflowPayment => ExitCode::OverflowPayment,
        InstructionResult::PrecompileError => ExitCode::PrecompileError,
        InstructionResult::NonceOverflow => ExitCode::NonceOverflow,
        InstructionResult::CreateContractSizeLimit | InstructionResult::CreateInitCodeSizeLimit => {
            ExitCode::ContractSizeLimit
        }
        InstructionResult::CreateContractStartingWithEF => ExitCode::CreateContractStartingWithEF,
        InstructionResult::FatalExternalError => ExitCode::FatalExternalError,
        _ => ExitCode::UnknownError,
    }
}

impl<'a, SDK: SharedAPI> Host for EvmLoader<'a, SDK> {
    fn env(&self) -> &Env {
        &self.env
    }

    fn env_mut(&mut self) -> &mut Env {
        &mut self.env
    }

    fn load_account_delegated(&mut self, _address: Address) -> Option<AccountLoad> {
        Some(AccountLoad {
            load: Eip7702CodeLoad::new_not_delegated((), false),
            is_empty: false,
        })
    }

    fn block_hash(&mut self, number: u64) -> Option<B256> {
        let block_number = as_u64_saturated!(self.env().block.number);
        let Some(diff) = block_number.checked_sub(number) else {
            return Some(B256::ZERO);
        };
        if diff > 0 && diff <= BLOCK_HASH_HISTORY {
            todo!("implement block hash history")
        } else {
            Some(B256::ZERO)
        }
    }

    fn balance(&mut self, address: Address) -> Option<StateLoad<U256>> {
        let (balance, is_cold) = self.sdk.balance(&address);
        Some(StateLoad::new(balance, is_cold))
    }

    fn code(&mut self, address: Address) -> Option<Eip7702CodeLoad<Bytes>> {
        debug_log!("code: address={} self-address={}", address, self.address);
        let result = self.code_hash(address)?;
        let evm_bytecode = self.sdk.preimage(&result.data);
        Some(Eip7702CodeLoad::new_not_delegated(
            evm_bytecode,
            result.is_cold,
        ))
    }

    fn code_hash(&mut self, address: Address) -> Option<Eip7702CodeLoad<B256>> {
        if address == self.address {
            let (evm_code_hash, is_cold) = self.sdk.storage(&EVM_CODE_HASH_SLOT);
            let evm_code_hash = evm_code_hash.to_le_bytes::<32>().into();

            return Some(Eip7702CodeLoad::new_not_delegated(evm_code_hash, is_cold));
        }
        let (evm_code_hash, is_cold) = self.sdk.ext_storage(&address, &EVM_CODE_HASH_SLOT);
        let evm_code_hash = evm_code_hash.to_le_bytes::<32>().into();
        Some(Eip7702CodeLoad::new_not_delegated(evm_code_hash, is_cold))
    }

    fn sload(&mut self, _address: Address, index: U256) -> Option<StateLoad<U256>> {
        let (value, is_cold) = self.sdk.storage(&index);
        Some(StateLoad::new(value, is_cold))
    }

    fn sstore(
        &mut self,
        _address: Address,
        index: U256,
        new_value: U256,
    ) -> Option<StateLoad<SStoreResult>> {
        let (original_value, present_value, is_cold) = self.sdk.write_storage(index, new_value);
        let result = SStoreResult {
            original_value,
            present_value,
            new_value,
        };

        self.last_gas_consumed = self.sdk.last_fuel_consumed();

        Some(StateLoad::new(result, is_cold))
    }

    fn tload(&mut self, _address: Address, index: U256) -> U256 {
        self.sdk.transient_storage(&index)
    }

    fn tstore(&mut self, _address: Address, index: U256, value: U256) {
        self.sdk.write_transient_storage(index, value);
    }

    fn log(&mut self, mut log: Log) {
        self.sdk
            .emit_log(take(&mut log.data.data), log.data.topics());
    }

    fn selfdestruct(
        &mut self,
        _address: Address,
        target: Address,
    ) -> Option<StateLoad<SelfDestructResult>> {
        let is_cold = self.sdk.destroy_account(target);
        Some(StateLoad::new(SelfDestructResult::default(), is_cold))
    }

    fn last_gas_consumed(&self) -> u64 {
        self.last_gas_consumed
    }
}

const EVM_CODE_HASH_SLOT: U256 = U256::from_le_bytes(derive_keccak256!("_evm_bytecode_hash"));

impl<'a, SDK: SharedAPI> EvmLoader<'a, SDK> {
    pub fn new(sdk: &'a mut SDK) -> Self {
        let address = sdk.context().contract_address();
        Self {
            env: env_from_context(sdk.context()),
            sdk,
            address,
            last_gas_consumed: 0,
        }
    }

    pub fn load_evm_bytecode(&self) -> (Bytecode, B256) {
        let evm_code_hash = self
            .sdk
            .storage(&EVM_CODE_HASH_SLOT)
            .0
            .to_le_bytes::<32>()
            .into();
        let evm_bytecode = self.sdk.preimage(&evm_code_hash);
        (Bytecode::new_raw(evm_bytecode), evm_code_hash)
    }

    pub fn store_evm_bytecode(&mut self, bytecode: Bytecode) {
        let code_hash = self.sdk.write_preimage(bytecode.original_bytes());
        debug_assert_eq!(code_hash, bytecode.hash_slow());
        self.sdk
            .write_storage(EVM_CODE_HASH_SLOT, U256::from_le_bytes(code_hash.0));
    }

    pub fn exec_evm_bytecode(&mut self, mut contract: Contract) -> InterpreterResult {
        let gas = Gas::new(self.sdk.fuel());

        // make sure bytecode is analyzed
        contract.bytecode = to_analysed(contract.bytecode);

        debug_log!("gas: {}", gas.remaining());

        let instruction_table = make_instruction_table::<Self, CancunSpec>();

        let mut interpreter = Interpreter::new(contract, gas.remaining(), false);
        let mut shared_memory = SharedMemory::new();

        loop {
            // run EVM bytecode to produce next action
            let next_action = interpreter.run(shared_memory, &instruction_table, self);

            // take memory and cr from interpreter and host back (return later)
            shared_memory = interpreter.take_memory();

            debug_log!("next_action: {:?}", &next_action);

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
                        CallScheme::CallCode => self.sdk.call_code(
                            inputs.target_address,
                            inputs.value.transfer().unwrap_or_default(),
                            inputs.input.as_ref(),
                            inputs.gas_limit,
                        ),
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
                        _ => unreachable!("unexpected call scheme"),
                    };
                    let consumed_gas = self.sdk.last_fuel_consumed();
                    assert!(interpreter.gas.record_cost(consumed_gas), "not enough gas");

                    let result = InterpreterResult::new(
                        evm_error_from_exit_code(ExitCode::from(exit_code)),
                        output,
                        interpreter.gas,
                    );
                    let call_outcome = CallOutcome::new(result, return_memory_offset);
                    interpreter.insert_call_outcome(&mut shared_memory, call_outcome);
                }
                InterpreterAction::Create { inputs } => {
                    let create_outcome = match self.sdk.create(
                        inputs.gas_limit,
                        match inputs.scheme {
                            CreateScheme::Create2 { salt } => Some(salt),
                            CreateScheme::Create => None,
                        },
                        &inputs.value,
                        inputs.init_code.as_ref(),
                    ) {
                        Ok(created_address) => {
                            let result = InterpreterResult::new(
                                InstructionResult::Stop,
                                Bytes::default(),
                                interpreter.gas,
                            );
                            CreateOutcome::new(result, Some(created_address))
                        }
                        Err(exit_code) => {
                            let error = evm_error_from_exit_code(ExitCode::from(exit_code));
                            let result =
                                InterpreterResult::new(error, Bytes::default(), interpreter.gas);
                            CreateOutcome::new(result, None)
                        }
                    };
                    let consumed_gas = self.sdk.last_fuel_consumed();
                    assert!(interpreter.gas.record_cost(consumed_gas), "not enough gas");
                    debug_log!("create_outcome: {:?}", &create_outcome);
                    interpreter.insert_create_outcome(create_outcome);
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

    pub fn call(&mut self) -> InterpreterResult {
        let input = self.sdk.input();
        let (evm_bytecode, _code_hash) = self.load_evm_bytecode();
        let contract = Contract {
            input,
            bytecode: to_analysed(evm_bytecode),
            hash: None,
            target_address: self.sdk.context().contract_address(),
            bytecode_address: None,
            caller: self.sdk.context().contract_caller(),
            call_value: self.sdk.context().contract_value(),
        };
        let result = self.exec_evm_bytecode(contract);
        self.sdk.charge_fuel(result.gas.spent());
        result
    }

    pub fn deploy(&mut self) -> ExitCode {
        let init_code = self.sdk.input();
        let contract = Contract {
            input: Bytes::default(),
            bytecode: to_analysed(Bytecode::new_raw(init_code)),
            hash: None,
            target_address: self.sdk.context().contract_address(),
            bytecode_address: None,
            caller: self.sdk.context().contract_caller(),
            call_value: self.sdk.context().contract_value(),
        };
        let mut result = self.exec_evm_bytecode(contract);

        if !result.is_ok() {
            // it might be an error message, have to return
            self.sdk.write(result.output.as_ref());
            // exit with corresponding error code
            return exit_code_from_evm_error(result.result);
        }

        // if bytecode starts with 0xEF or exceeds MAX_CODE_SIZE then return the corresponding error
        if !result.output.is_empty() && result.output.first() == Some(&0xEF) {
            return ExitCode::CreateContractStartingWithEF;
        } else if result.output.len() > MAX_CODE_SIZE {
            return ExitCode::ContractSizeLimit;
        }

        // record gas for each created byte
        let gas_for_code = result.output.len() as u64 * gas::CODEDEPOSIT;
        if !result.gas.record_cost(gas_for_code) {
            return ExitCode::OutOfGas;
        }

        self.sdk.charge_fuel(result.gas.spent());

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

    pub fn deploy_inner(&mut self) -> ExitCode {
        EvmLoader::new(&mut self.sdk).deploy()
    }

    pub fn main(&mut self) {
        let exit_code = self.main_inner();
        self.sdk.exit(exit_code.into_i32());
    }

    pub fn main_inner(&mut self) -> ExitCode {
        let result = EvmLoader::new(&mut self.sdk).call();
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
                bytecode_address: Default::default(),
                caller: Address::ZERO,
                is_static: false,
                value: U256::ZERO,
            })
            .build(native_sdk.clone());
        let mut evm_loader = EvmLoader::new(&mut sdk);
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
        let mut native_sdk = TestingContext::empty().with_fuel(100_000);
        let sdk = JournalStateBuilder::default()
            .with_contract_context(ContractContext {
                address: Address::from([
                    189, 119, 4, 22, 163, 52, 95, 145, 228, 179, 69, 118, 203, 128, 74, 87, 111,
                    164, 142, 177,
                ]),
                bytecode_address: Default::default(),
                caller: Address::ZERO,
                is_static: false,
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

    #[test]
    fn test_deploy_tstore() {
        let mut native_sdk = TestingContext::empty().with_fuel(100_000);
        let sdk = JournalStateBuilder::default()
            .with_contract_context(ContractContext {
                address: Address::from([
                    189, 119, 4, 22, 163, 52, 95, 145, 228, 179, 69, 118, 203, 128, 74, 87, 111,
                    164, 142, 177,
                ]),
                bytecode_address: Default::default(),
                caller: Address::ZERO,
                is_static: false,
                value: U256::ZERO,
            })
            .build(native_sdk.clone());
        let mut app = EvmLoaderEntrypoint::new(sdk);
        // deploy
        {
            native_sdk.set_input(hex!("600460025f5d5a5f5c505a9003035f5500"));
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
