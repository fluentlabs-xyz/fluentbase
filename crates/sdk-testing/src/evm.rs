use crate::{HostTestingContext, HostTestingContextNativeAPI};
use core::{mem::take, str::from_utf8};
use fluentbase_genesis::{devnet_genesis_from_file, Genesis};
use fluentbase_runtime::{Runtime, RuntimeContext};
use fluentbase_sdk::{
    bytes::BytesMut,
    calc_create_address,
    codec::CompactABI,
    Address,
    Bytes,
    ExitCode,
    HashMap,
    SharedContextInputV1,
    KECCAK_EMPTY,
    STATE_MAIN,
    U256,
};
use fluentbase_types::compile_wasm_to_rwasm;
use revm::{
    primitives::{keccak256, AccountInfo, Bytecode, Env, ExecutionResult, TransactTo},
    DatabaseCommit,
    Evm,
    InMemoryDB,
};
use rwasm::legacy::rwasm::{BinaryFormat, RwasmModule};

#[allow(dead_code)]
pub struct EvmTestingContext {
    pub sdk: HostTestingContext,
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
            sdk: HostTestingContext::default(),
            genesis,
            db,
        }
    }

    pub fn add_wasm_contract<I: Into<RwasmModule>>(
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

    pub fn add_bytecode(&mut self, address: Address, bytecode: Bytes) -> AccountInfo {
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

    pub fn get_balance(&mut self, address: Address) -> U256 {
        let account = self.db.load_account(address).unwrap();
        account.info.balance
    }

    pub fn get_nonce(&mut self, address: Address) -> u64 {
        let account = self.db.load_account(address).unwrap();
        account.info.nonce
    }

    pub fn add_balance(&mut self, address: Address, value: U256) {
        let account = self.db.load_account(address).unwrap();
        account.info.balance += value;
        let mut revm_account = revm::primitives::Account::from(account.info.clone());
        revm_account.mark_touch();
        self.db.commit(HashMap::from([(address, revm_account)]));
    }

    pub fn deploy_evm_tx(&mut self, deployer: Address, init_bytecode: Bytes) -> Address {
        let (contract_address, _) = self.deploy_evm_tx_with_gas(deployer, init_bytecode);
        contract_address
    }

    pub fn deploy_evm_tx_with_gas(
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
        let contract_address = calc_create_address::<HostTestingContextNativeAPI>(&deployer, 0);
        assert_eq!(contract_address, deployer.create(0));
        (contract_address, result.gas_used())
    }

    pub fn deploy_evm_tx_with_nonce(
        &mut self,
        deployer: Address,
        init_bytecode: Bytes,
        nonce: u64,
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
        assert!(result.is_success());
        let contract_address = calc_create_address::<HostTestingContextNativeAPI>(&deployer, nonce);
        assert_eq!(contract_address, deployer.create(nonce));

        (contract_address, result.gas_used())
    }

    pub fn call_evm_tx_simple(
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

    pub fn call_evm_tx(
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

pub struct TxBuilder<'a> {
    pub ctx: &'a mut EvmTestingContext,
    pub env: Env,
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

    pub fn disable_builtins_consume_fuel(mut self) -> Self {
        self.env.cfg.disable_builtins_consume_fuel = true;
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

pub fn try_print_utf8_error(mut output: &[u8]) {
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

pub fn run_with_default_context(wasm_binary: Vec<u8>, input_data: &[u8]) -> (Vec<u8>, i32) {
    let rwasm_binary = if wasm_binary[0] == 0xef {
        wasm_binary
    } else {
        compile_wasm_to_rwasm(&wasm_binary)
            .unwrap()
            .rwasm_bytecode
            .into()
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
            .executor
            .tracer()
            .map(|v| v.logs.clone())
            .unwrap_or_default();
        println!("execution trace ({} steps):", logs.len());
        for log in logs.iter().rev().take(100).rev() {
            println!(" - pc={} opcode={}", log.program_counter, log.opcode);
        }
    } else {
        println!(
            "trace steps: {}",
            runtime
                .executor
                .tracer()
                .map(|v| v.logs.len())
                .unwrap_or_default()
        );
    }
    (result.output.into(), result.exit_code)
}

#[allow(dead_code)]
pub fn catch_panic(ctx: &fluentbase_runtime::ExecutionResult) {
    if ctx.exit_code != -1 {
        return;
    }
    println!("panic with err: {}", from_utf8(&ctx.output).unwrap());
}
