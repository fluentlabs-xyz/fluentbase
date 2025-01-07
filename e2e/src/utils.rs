use core::{mem::take, str::from_utf8};
use fluentbase_genesis::{
    devnet_genesis_from_file,
    Genesis,
    GENESIS_KECCAK_HASH_SLOT,
    GENESIS_POSEIDON_HASH_SLOT,
};
use fluentbase_poseidon::poseidon_hash;
use fluentbase_runtime::{Runtime, RuntimeContext};
use fluentbase_rwasm::{RwasmExecutor, SimpleCallHandler};
use fluentbase_sdk::{
    bytes::BytesMut,
    calc_create_address,
    codec::FluentABI,
    create_import_linker,
    runtime::TestingContext,
    Account,
    Address,
    Bytes,
    ExitCode,
    HashMap,
    SharedContextInputV1,
    SysFuncIdx::STATE,
    KECCAK_EMPTY,
    POSEIDON_EMPTY,
    STATE_DEPLOY,
    STATE_MAIN,
    U256,
};
use revm::{
    primitives::{keccak256, AccountInfo, Bytecode, Env, ExecutionResult, TransactTo},
    DatabaseCommit,
    InMemoryDB,
    Rwasm,
};
use rwasm::{
    engine::{bytecode::Instruction, RwasmConfig, StateRouterConfig},
    rwasm::{instruction::InstructionExtra, BinaryFormat, BinaryFormatWriter, RwasmModule},
    Error,
};
use std::u64;

#[allow(dead_code)]
pub(crate) struct EvmTestingContext {
    pub sdk: TestingContext,
    pub genesis: Genesis,
    pub db: InMemoryDB,
}

impl Default for EvmTestingContext {
    fn default() -> Self {
        Self::load_from_genesis(devnet_genesis_from_file())
    }
}

#[allow(dead_code)]
impl EvmTestingContext {
    fn load_from_genesis(genesis: Genesis) -> Self {
        // create jzkt and put it into testing context
        let mut db = InMemoryDB::default();
        // convert all accounts from genesis into jzkt
        for (k, v) in genesis.alloc.iter() {
            let poseidon_hash = v
                .storage
                .as_ref()
                .and_then(|v| v.get(&GENESIS_POSEIDON_HASH_SLOT).cloned())
                .unwrap_or_else(|| {
                    v.code
                        .as_ref()
                        .map(|v| poseidon_hash(&v).into())
                        .unwrap_or(POSEIDON_EMPTY)
                });
            let _keccak_hash = v
                .storage
                .as_ref()
                .and_then(|v| v.get(&GENESIS_KECCAK_HASH_SLOT).cloned())
                .unwrap_or_else(|| {
                    v.code
                        .as_ref()
                        .map(|v| keccak256(&v))
                        .unwrap_or(KECCAK_EMPTY)
                });
            let account = Account {
                address: *k,
                balance: v.balance,
                nonce: v.nonce.unwrap_or_default(),
                // it makes not much sense to fill these fields, but it reduces hash calculation
                // time a bit
                code_size: v.code.as_ref().map(|v| v.len() as u64).unwrap_or_default(),
                code_hash: poseidon_hash,
            };
            let mut info: AccountInfo = account.into();
            info.code = v.code.clone().map(Bytecode::new_raw);
            db.insert_account_info(*k, info);
        }
        Self {
            sdk: TestingContext::new(RuntimeContext::default()),
            genesis,
            db,
        }
    }

    pub(crate) fn add_wasm_contract<I: Into<RwasmModule>>(
        &mut self,
        address: Address,
        rwasm_module: I,
    ) -> AccountInfo {
        let rwasm_binary = {
            let rwasm_module: RwasmModule = rwasm_module.into();
            let mut result = Vec::new();
            rwasm_module.write_binary_to_vec(&mut result).unwrap();
            result
        };
        let account = Account {
            address,
            balance: U256::ZERO,
            nonce: 0,
            // it makes not much sense to fill these fields, but it optimizes hash calculation a bit
            code_size: rwasm_binary.len() as u64,
            code_hash: poseidon_hash(&rwasm_binary).into(),
        };
        let mut info: AccountInfo = account.into();
        if !rwasm_binary.is_empty() {
            info.code = Some(Bytecode::new_raw(rwasm_binary.into()));
        }
        self.db.insert_account_info(address, info.clone());
        info
    }

    pub(crate) fn get_balance(&mut self, address: Address) -> U256 {
        let account = self.db.load_account(address).unwrap();
        account.info.balance
    }

    pub(crate) fn add_balance(&mut self, address: Address, value: U256) {
        let account = self.db.load_account(address).unwrap();
        account.info.balance += value;
        let mut revm_account = revm::primitives::Account::from(account.info.clone());
        revm_account.mark_touch();
        self.db.commit(HashMap::from([(address, revm_account)]));
    }

    pub(crate) fn deploy_evm_tx(&mut self, deployer: Address, init_bytecode: Bytes) -> Address {
        // let bytecode_type = BytecodeType::from_slice(init_bytecode.as_ref());
        // deploy greeting EVM contract
        let result = TxBuilder::create(self, deployer, init_bytecode.clone().into()).exec();
        if !result.is_success() {
            println!("{:?}", result);
            println!(
                "{}",
                from_utf8(result.output().cloned().unwrap_or_default().as_ref()).unwrap_or("")
            );
        }
        assert!(result.is_success());
        let contract_address = calc_create_address::<TestingContext>(&deployer, 0);
        assert_eq!(contract_address, deployer.create(0));
        // let contract_account = ctx.db.accounts.get(&contract_address).unwrap();
        // if bytecode_type == BytecodeType::EVM {
        //     let source_bytecode = ctx
        //         .db
        //         .contracts
        //         .get(&contract_account.info.code_hash)
        //         .unwrap()
        //         .original_bytes()
        //         .to_vec();
        //     assert_eq!(contract_account.info.code_hash, keccak256(&source_bytecode));
        //     assert!(source_bytecode.len() > 0);
        // }
        // if bytecode_type == BytecodeType::WASM {
        //     let rwasm_bytecode = ctx
        //         .db
        //         .contracts
        //         .get(&contract_account.info.rwasm_code_hash)
        //         .unwrap()
        //         .bytes()
        //         .to_vec();
        //     assert_eq!(
        //         contract_account.info.rwasm_code_hash.0,
        //         poseidon_hash(&rwasm_bytecode)
        //     );
        //     let is_rwasm = rwasm_bytecode.get(0).cloned().unwrap() == 0xef;
        //     assert!(is_rwasm);
        // }
        contract_address
    }

    pub(crate) fn deploy_evm_tx_with_nonce(
        &mut self,
        deployer: Address,
        init_bytecode: Bytes,
        nonce: u64,
    ) -> Address {
        let result = TxBuilder::create(self, deployer, init_bytecode.clone().into()).exec();
        if !result.is_success() {
            println!("{:?}", result);
            println!(
                "{}",
                from_utf8(result.output().cloned().unwrap_or_default().as_ref()).unwrap_or("")
            );
        }
        assert!(result.is_success());
        let contract_address = calc_create_address::<TestingContext>(&deployer, nonce);
        assert_eq!(contract_address, deployer.create(nonce));

        contract_address
    }

    pub(crate) fn call_evm_tx_simple(
        &mut self,
        caller: Address,
        callee: Address,
        input: Bytes,
        gas_limit: Option<u64>,
        value: Option<U256>,
    ) -> ExecutionResult {
        // call greeting EVM contract
        let mut tx_builder = TxBuilder::call(self, caller, callee, value).input(input);
        if let Some(gas_limit) = gas_limit {
            tx_builder = tx_builder.gas_limit(gas_limit);
        }
        tx_builder.exec()
    }

    pub(crate) fn call_evm_tx(
        &mut self,
        caller: Address,
        callee: Address,
        input: Bytes,
        gas_limit: Option<u64>,
        value: Option<U256>,
    ) -> ExecutionResult {
        self.add_balance(caller, U256::from(1e18));
        self.call_evm_tx_simple(caller, callee, input, gas_limit, value)
    }
}

pub(crate) struct TxBuilder<'a> {
    pub(crate) ctx: &'a mut EvmTestingContext,
    pub(crate) env: Env,
}

#[allow(dead_code)]
impl<'a> TxBuilder<'a> {
    pub fn create(ctx: &'a mut EvmTestingContext, deployer: Address, init_code: Bytes) -> Self {
        let mut env = Env::default();
        env.tx.caller = deployer;
        env.tx.transact_to = TransactTo::Create;
        env.tx.data = init_code;
        env.tx.gas_limit = 300_000_000;
        Self { ctx, env }
    }

    pub fn call(
        ctx: &'a mut EvmTestingContext,
        caller: Address,
        callee: Address,
        value: Option<U256>,
    ) -> Self {
        let mut env = Env::default();
        if let Some(value) = value {
            env.tx.value = value;
        }
        env.tx.gas_price = U256::from(1);
        env.tx.caller = caller;
        env.tx.transact_to = TransactTo::Call(callee);
        env.tx.gas_limit = 300_000_000;
        Self { ctx, env }
    }

    pub fn input(mut self, input: Bytes) -> Self {
        self.env.tx.data = input;
        self
    }

    pub fn value(mut self, value: U256) -> Self {
        self.env.tx.value = value;
        self
    }

    pub fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.env.tx.gas_limit = gas_limit;
        self
    }

    pub fn gas_price(mut self, gas_price: U256) -> Self {
        self.env.tx.gas_price = gas_price;
        self
    }

    pub fn exec(&mut self) -> ExecutionResult {
        let db = take(&mut self.ctx.db);
        let mut evm = Rwasm::builder()
            .with_env(Box::new(take(&mut self.env)))
            .with_db(db)
            .build();
        let result = evm.transact_commit().unwrap();
        self.ctx.db = evm.into_db();
        result
    }
}

pub(crate) fn run_with_default_context(wasm_binary: Vec<u8>, input_data: &[u8]) -> (Vec<u8>, i32) {
    let rwasm_binary = if wasm_binary[0] == 0xef {
        wasm_binary
    } else {
        wasm2rwasm(wasm_binary.as_slice()).unwrap()
    };

    let context_input = {
        let shared_ctx = SharedContextInputV1 {
            block: Default::default(),
            tx: Default::default(),
            contract: Default::default(),
        };
        let mut buf = BytesMut::new();
        FluentABI::encode(&shared_ctx, &mut buf, 0).unwrap();
        buf.extend_from_slice(input_data);
        buf.freeze().to_vec()
    };
    let ctx = RuntimeContext::new(rwasm_binary)
        .with_state(STATE_MAIN)
        .with_fuel_limit(100_000_000_000)
        .with_input(context_input)
        .with_is_test();
    // .with_tracer();
    let mut runtime = Runtime::new(ctx);
    runtime.data_mut().clear_output();
    let result = runtime.call();
    println!(
        "exit_code: {} ({})",
        result.exit_code,
        ExitCode::from(result.exit_code)
    );
    println!(
        "output: 0x{} ({})",
        hex::encode(&result.output),
        from_utf8(&result.output).unwrap_or("can't decode utf-8")
    );
    println!("fuel consumed: {}", result.fuel_consumed);
    if result.exit_code != 0 {
        let logs = &runtime
            .store()
            .tracer()
            .map(|v| v.logs.clone())
            .unwrap_or_default();
        println!("execution trace ({} steps):", logs.len());
        for log in logs.iter().rev().take(100).rev() {
            if let Some(value) = log.opcode.aux_value() {
                println!(
                    " - pc={} opcode={}({})",
                    log.program_counter, log.opcode, value
                );
            } else {
                println!(" - pc={} opcode={}", log.program_counter, log.opcode);
            }
        }
    } else {
        println!(
            "trace steps: {}",
            runtime
                .store()
                .tracer()
                .map(|v| v.logs.len())
                .unwrap_or_default()
        );
    }
    (result.output.into(), result.exit_code)
}

pub(crate) fn run_with_default_context2(wasm_binary: Vec<u8>, input_data: &[u8]) -> (Vec<u8>, i32) {
    let rwasm_binary = if wasm_binary[0] == 0xef {
        wasm_binary
    } else {
        wasm2rwasm(wasm_binary.as_slice()).unwrap()
    };

    let context_input = {
        let shared_ctx = SharedContextInputV1 {
            block: Default::default(),
            tx: Default::default(),
            contract: Default::default(),
        };
        let mut buf = BytesMut::new();
        FluentABI::encode(&shared_ctx, &mut buf, 0).unwrap();
        buf.extend_from_slice(input_data);
        buf.freeze().to_vec()
    };

    let mut simple_call_handler = SimpleCallHandler::default();
    simple_call_handler.state = STATE_MAIN;
    simple_call_handler.input = context_input;
    let exit_code = RwasmExecutor::parse(&rwasm_binary, Some(&mut simple_call_handler), None)
        .unwrap()
        .run()
        .unwrap();

    println!("exit_code: {} ({})", exit_code, ExitCode::from(exit_code));
    println!(
        "output: 0x{} ({})",
        hex::encode(&simple_call_handler.output),
        from_utf8(&simple_call_handler.output).unwrap_or("can't decode utf-8")
    );

    (simple_call_handler.output, exit_code)
}

#[allow(dead_code)]
pub(crate) fn catch_panic(ctx: &fluentbase_runtime::ExecutionResult) {
    if ctx.exit_code != -71 {
        return;
    }
    println!(
        "panic with err: {}",
        std::str::from_utf8(&ctx.output).unwrap()
    );
}

#[inline(always)]
pub fn rwasm_module(wasm_binary: &[u8]) -> Result<RwasmModule, Error> {
    let mut config = RwasmModule::default_config(None);
    config.rwasm_config(RwasmConfig {
        state_router: Some(StateRouterConfig {
            states: Box::new([
                ("deploy".to_string(), STATE_DEPLOY),
                ("main".to_string(), STATE_MAIN),
            ]),
            opcode: Instruction::Call(STATE.into()),
        }),
        entrypoint_name: None,
        import_linker: Some(create_import_linker()),
        wrap_import_functions: true,
        translate_drop_keep: false,
    });
    RwasmModule::compile_with_config(wasm_binary, &config)
}

#[inline(always)]
pub fn wasm2rwasm(wasm_binary: &[u8]) -> Result<Vec<u8>, ExitCode> {
    let rwasm_module = rwasm_module(wasm_binary);
    if rwasm_module.is_err() {
        return Err(ExitCode::CompilationError);
    }
    let rwasm_module = rwasm_module.unwrap();
    let length = rwasm_module.encoded_length();
    let mut rwasm_bytecode = vec![0u8; length];
    let mut binary_format_writer = BinaryFormatWriter::new(&mut rwasm_bytecode);
    rwasm_module
        .write_binary(&mut binary_format_writer)
        .expect("failed to encode rwasm bytecode");
    Ok(rwasm_bytecode)
}
