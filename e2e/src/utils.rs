use core::{mem::take, str::from_utf8};
use fluentbase_genesis::{devnet_genesis_from_file, Genesis};
use fluentbase_runtime::{Runtime, RuntimeContext};
use fluentbase_sdk::{
    bytes::BytesMut,
    calc_create_address,
    codec::CompactABI,
    create_import_linker,
    testing::{TestingContext, TestingContextNativeAPI},
    Address,
    Bytes,
    ExitCode,
    HashMap,
    SharedContextInputV1,
    SysFuncIdx,
    KECCAK_EMPTY,
    STATE_DEPLOY,
    STATE_MAIN,
    U256,
};
use revm::{
    primitives::{keccak256, AccountInfo, Bytecode, Env, ExecutionResult, TransactTo},
    DatabaseCommit,
    Evm,
    InMemoryDB,
};
use rwasm::{
    engine::{bytecode::Instruction, RwasmConfig, StateRouterConfig},
    rwasm::{instruction::InstructionExtra, BinaryFormat, BinaryFormatWriter, RwasmModule},
    Error,
};

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
            let code_hash = v
                .code
                .as_ref()
                .map(|value| keccak256(&value))
                .unwrap_or(KECCAK_EMPTY);
            let mut info: AccountInfo = AccountInfo {
                balance: v.balance,
                nonce: v.nonce.unwrap_or_default(),
                code_hash,
                code: None,
            };
            info.code = v.code.clone().map(Bytecode::new_raw);
            db.insert_account_info(*k, info);
        }
        Self {
            sdk: TestingContext::default(),
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
        let mut info: AccountInfo = AccountInfo {
            balance: U256::ZERO,
            nonce: 0,
            code_hash: keccak256(&rwasm_binary),
            code: None,
        };
        if !rwasm_binary.is_empty() {
            info.code = Some(Bytecode::new_raw(rwasm_binary.into()));
        }
        self.db.insert_account_info(address, info.clone());
        info
    }

    pub(crate) fn add_bytecode(&mut self, address: Address, bytecode: Bytes) -> AccountInfo {
        let mut info: AccountInfo = AccountInfo {
            balance: U256::ZERO,
            nonce: 0,
            code_hash: keccak256(bytecode.as_ref()),
            code: None,
        };
        info.code = Some(Bytecode::new_raw(bytecode));
        self.db.insert_account_info(address, info.clone());
        info
    }

    pub(crate) fn get_balance(&mut self, address: Address) -> U256 {
        let account = self.db.load_account(address).unwrap();
        account.info.balance
    }

    pub(crate) fn get_nonce(&mut self, address: Address) -> u64 {
        let account = self.db.load_account(address).unwrap();
        account.info.nonce
    }

    pub(crate) fn add_balance(&mut self, address: Address, value: U256) {
        let account = self.db.load_account(address).unwrap();
        account.info.balance += value;
        let mut revm_account = revm::primitives::Account::from(account.info.clone());
        revm_account.mark_touch();
        self.db.commit(HashMap::from([(address, revm_account)]));
    }

    pub(crate) fn deploy_evm_tx(&mut self, deployer: Address, init_bytecode: Bytes) -> Address {
        let (contract_address, _) = self.deploy_evm_tx_with_gas(deployer, init_bytecode);
        contract_address
    }

    pub(crate) fn deploy_evm_tx_with_gas(
        &mut self,
        deployer: Address,
        init_bytecode: Bytes,
    ) -> (Address, u64) {
        let result = TxBuilder::create(self, deployer, init_bytecode.clone().into())
            .enable_rwasm_proxy()
            .exec();
        if !result.is_success() {
            println!("{:?}", result);
            println!(
                "{}",
                from_utf8(result.output().cloned().unwrap_or_default().as_ref()).unwrap_or("")
            );
        }
        if !result.is_success() {
            try_print_utf8_error(result.output().cloned().unwrap_or_default().as_ref())
        }
        println!("deployment gas used: {}", result.gas_used());
        assert!(result.is_success());
        let contract_address = calc_create_address::<TestingContextNativeAPI>(&deployer, 0);
        assert_eq!(contract_address, deployer.create(0));
        (contract_address, result.gas_used())
    }

    pub(crate) fn deploy_evm_tx_with_nonce(
        &mut self,
        deployer: Address,
        init_bytecode: Bytes,
        nonce: u64,
    ) -> (Address, u64) {
        let result = TxBuilder::create(self, deployer, init_bytecode.clone().into()).exec();
        if !result.is_success() {
            println!("{:?}", result);
            println!(
                "{}",
                from_utf8(result.output().cloned().unwrap_or_default().as_ref()).unwrap_or("")
            );
        }
        assert!(result.is_success());
        let contract_address = calc_create_address::<TestingContextNativeAPI>(&deployer, nonce);
        assert_eq!(contract_address, deployer.create(nonce));

        (contract_address, result.gas_used())
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
        env.tx.gas_limit = 30_000_000;
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
        env.tx.gas_limit = 3_000_000;
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

    pub fn timestamp(mut self, timestamp: u64) -> Self {
        self.env.block.timestamp = U256::from(timestamp);
        self
    }

    pub fn enable_rwasm_proxy(mut self) -> Self {
        self.env.cfg.enable_rwasm_proxy = true;
        self
    }

    pub fn exec(&mut self) -> ExecutionResult {
        let db = take(&mut self.ctx.db);
        let mut evm = Evm::builder()
            .with_env(Box::new(take(&mut self.env)))
            .with_db(db)
            .build();
        let result = evm.transact_commit().unwrap();
        (self.ctx.db, _) = evm.into_db_and_env_with_handler_cfg();
        result
    }
}

pub(crate) fn try_print_utf8_error(mut output: &[u8]) {
    if output.starts_with(&[0x08, 0xc3, 0x79, 0xa0]) {
        output = &output[68..];
    }
    println!(
        "output: 0x{} ({})",
        hex::encode(&output),
        from_utf8(output)
            .unwrap_or("can't decode utf-8")
            .trim_end_matches("\0")
    );
}

fn rwasm_module(wasm_binary: &[u8]) -> Result<RwasmModule, Error> {
    let mut config = RwasmModule::default_config(None);
    config.rwasm_config(RwasmConfig {
        state_router: Some(StateRouterConfig {
            states: Box::new([
                ("deploy".to_string(), STATE_DEPLOY),
                ("main".to_string(), STATE_MAIN),
            ]),
            opcode: Instruction::Call(SysFuncIdx::STATE.into()),
        }),
        entrypoint_name: None,
        import_linker: Some(create_import_linker()),
        wrap_import_functions: true,
        translate_drop_keep: false,
    });
    RwasmModule::compile_with_config(wasm_binary, &config)
}

fn wasm2rwasm(wasm_binary: &[u8]) -> Vec<u8> {
    let rwasm_module = rwasm_module(wasm_binary);
    if rwasm_module.is_err() {
        panic!("failed to compile wasm to rwasm: {:?}", rwasm_module.err());
    }
    let rwasm_module = rwasm_module.unwrap();
    let length = rwasm_module.encoded_length();
    let mut rwasm_bytecode = vec![0u8; length];
    let mut binary_format_writer = BinaryFormatWriter::new(&mut rwasm_bytecode);
    rwasm_module
        .write_binary(&mut binary_format_writer)
        .expect("failed to encode rwasm bytecode");
    rwasm_bytecode
}

pub(crate) fn run_with_default_context(wasm_binary: Vec<u8>, input_data: &[u8]) -> (Vec<u8>, i32) {
    let rwasm_binary = if wasm_binary[0] == 0xef {
        wasm_binary
    } else {
        wasm2rwasm(wasm_binary.as_slice())
    };

    let context_input = {
        let shared_ctx = SharedContextInputV1 {
            block: Default::default(),
            tx: Default::default(),
            contract: Default::default(),
        };
        let mut buf = BytesMut::new();
        CompactABI::encode(&shared_ctx, &mut buf, 0).unwrap();
        buf.extend_from_slice(input_data);
        buf.freeze().to_vec()
    };
    let ctx = RuntimeContext::new(Bytes::from(rwasm_binary))
        .with_state(STATE_MAIN)
        .with_fuel_limit(100_000_000_000)
        .with_input(context_input);
    // .with_tracer();
    let mut runtime = Runtime::new(ctx);
    runtime.context_mut().clear_output();
    let result = runtime.call();
    println!(
        "exit_code: {} ({})",
        result.exit_code,
        ExitCode::from(result.exit_code)
    );
    try_print_utf8_error(&result.output[..]);
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
