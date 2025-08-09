use crate::{HostTestingContext, HostTestingContextNativeAPI};
use core::{borrow::Borrow, mem::take, str::from_utf8};
use fluentbase_revm::{RwasmBuilder, RwasmContext};
use fluentbase_runtime::{Runtime, RuntimeContext};
use fluentbase_sdk::{
    bytes::BytesMut, calc_create_address, compile_wasm_to_rwasm, Address, BytecodeOrHash, Bytes,
    ContextReader, ExitCode, GenesisContract, MetadataAPI, SharedAPI, SharedContextInputV1,
    STATE_MAIN, U256,
};
use revm::{
    context::{
        result::{ExecutionResult, ExecutionResult::Success, Output},
        BlockEnv, CfgEnv, TransactTo, TxEnv,
    },
    database::InMemoryDB,
    handler::MainnetContext,
    primitives::{hardfork::PRAGUE, keccak256, map::DefaultHashBuilder, HashMap},
    state::{Account, AccountInfo, Bytecode},
    DatabaseCommit, ExecuteCommitEvm, MainBuilder,
};
use rwasm::{RwasmModule, Store};

#[allow(dead_code)]
pub struct EvmTestingContext {
    pub sdk: HostTestingContext,
    pub db: InMemoryDB,
    pub cfg: CfgEnv,
    pub disabled_rwasm: bool,
}

impl Default for EvmTestingContext {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl EvmTestingContext {
    pub fn new() -> Self {
        Self {
            sdk: HostTestingContext::default(),
            db: InMemoryDB::default(),
            cfg: CfgEnv::default(),
            disabled_rwasm: false,
        }
    }

    pub fn with_block_number(self, number: u64) -> Self {
        let sdk = self.sdk.with_block_number(number);
        Self {
            sdk,
            db: self.db,
            cfg: self.cfg,
            disabled_rwasm: self.disabled_rwasm,
        }
    }

    // Add smart contracts to the genesis
    pub fn with_contracts(self, contracts: &[GenesisContract]) -> Self {
        let mut db = self.db;
        for contract in contracts.iter() {
            let info: AccountInfo = AccountInfo {
                balance: U256::ZERO,
                nonce: 0,
                code_hash: contract.rwasm_bytecode_hash,
                code: Some(Bytecode::new_raw(contract.rwasm_bytecode.clone())),
            };
            db.insert_account_info(contract.address, info);
        }
        Self {
            sdk: self.sdk,
            db,
            cfg: self.cfg,
            disabled_rwasm: self.disabled_rwasm,
        }
    }

    pub fn commit_storage(&mut self) {
        let storage = self.sdk.dump_storage();
        storage.iter().for_each(|((address, slot), value)| {
            self.db
                .insert_account_storage(*address, *slot, *value)
                .unwrap();
        })
    }

    pub fn db_storage_to_sdk(&mut self) {
        for (address, db_account) in &mut self.db.cache.accounts {
            self.sdk.visit_inner_storage_mut(|storage| {
                for (k, v) in &db_account.storage {
                    storage.insert((*address, *k), *v);
                }
            });
            if let Some(code) = db_account.info.code.as_mut() {
                match code {
                    Bytecode::OwnableAccount(account) => {
                        self.sdk
                            .metadata_write(address, 0, account.metadata.clone());
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn add_wasm_contract<I: Into<RwasmModule>>(
        &mut self,
        address: Address,
        rwasm_module: I,
    ) -> AccountInfo {
        let rwasm_binary = {
            let rwasm_module: RwasmModule = rwasm_module.into();
            rwasm_module.serialize()
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

    pub fn get_code(&mut self, address: Address) -> Option<&Bytecode> {
        let account = self.db.load_account(address).unwrap();
        account.info.code.as_ref()
    }

    pub fn add_balance(&mut self, address: Address, value: U256) {
        let account = self.db.load_account(address).unwrap();
        account.info.balance += value;
        let mut revm_account = Account::from(account.info.clone());
        revm_account.mark_touch();
        let changes: HashMap<Address, Account, DefaultHashBuilder> =
            HashMap::from([(address, revm_account)]);
        self.db.commit(changes);
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
        let nonce = self.nonce(deployer);
        let result = TxBuilder::create(self, deployer, init_bytecode.clone().into()).exec();
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
        #[cfg(feature = "debug-print")]
        println!("deployment gas used: {}", result.gas_used());
        assert!(result.is_success());
        let contract_address = calc_create_address::<HostTestingContextNativeAPI>(&deployer, nonce);
        assert_eq!(contract_address, deployer.create(nonce));

        assert!(
            matches!(result, Success { output: Output::Create(_, Some(addr)), .. } if addr == contract_address),
            "deploy transaction didn't return expected address: {:?}",
            result
        );
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

    pub fn nonce(&mut self, caller: Address) -> u64 {
        let account = self.db.load_account(caller).unwrap();
        account.info.nonce
    }
}

pub struct TxBuilder<'a> {
    pub ctx: &'a mut EvmTestingContext,
    pub tx: TxEnv,
    pub block: BlockEnv,
}

#[allow(dead_code)]
impl<'a> TxBuilder<'a> {
    pub fn create(ctx: &'a mut EvmTestingContext, deployer: Address, init_code: Bytes) -> Self {
        let mut tx = TxEnv::default();
        tx.caller = deployer;
        tx.kind = TransactTo::Create;
        tx.data = init_code;
        tx.gas_limit = 30_000_000;
        let block = Self::block_env(ctx);
        Self { ctx, tx, block }
    }

    pub fn call(
        ctx: &'a mut EvmTestingContext,
        caller: Address,
        callee: Address,
        value: Option<U256>,
    ) -> Self {
        let mut tx = TxEnv::default();
        if let Some(value) = value {
            tx.value = value;
        }
        tx.gas_price = 1;
        tx.caller = caller;
        tx.kind = TransactTo::Call(callee);
        tx.gas_limit = 3_000_000;
        let block = Self::block_env(ctx);
        Self { ctx, tx, block }
    }

    fn block_env(ctx: &EvmTestingContext) -> BlockEnv {
        let mut block_env = BlockEnv::default();
        let ctx = ctx.sdk.borrow().context();
        block_env.number = U256::from(ctx.block_number());
        block_env
    }

    pub fn input(mut self, input: Bytes) -> Self {
        self.tx.data = input;
        self
    }

    pub fn value(mut self, value: U256) -> Self {
        self.tx.value = value;
        self
    }

    pub fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.tx.gas_limit = gas_limit;
        self
    }

    pub fn nonce(mut self, nonce: u64) -> Self {
        self.tx.nonce = nonce;
        self
    }

    pub fn gas_price(mut self, gas_price: u128) -> Self {
        self.tx.gas_price = gas_price;
        self
    }

    pub fn timestamp(mut self, timestamp: u64) -> Self {
        self.block.timestamp = U256::from(timestamp);
        self
    }

    pub fn exec(&mut self) -> ExecutionResult {
        self.tx.nonce = self.ctx.nonce(self.tx.caller);
        let db = take(&mut self.ctx.db);
        if self.ctx.disabled_rwasm {
            let mut context: MainnetContext<InMemoryDB> = MainnetContext::new(db, PRAGUE);
            context.cfg = self.ctx.cfg.clone();
            context.block = self.block.clone();
            context.tx = self.tx.clone();
            let mut evm = context.build_mainnet();
            let result = evm.transact_commit(self.tx.clone()).unwrap();
            let new_db = &mut evm.journaled_state.database;
            self.ctx.db = take(new_db);
            result
        } else {
            let mut context: RwasmContext<InMemoryDB> = RwasmContext::new(db, PRAGUE);
            context.cfg = self.ctx.cfg.clone();
            context.block = self.block.clone();
            context.tx = self.tx.clone();
            let mut evm = context.build_rwasm();
            let result = evm.transact_commit(self.tx.clone()).unwrap();
            let new_db = &mut evm.0.journaled_state.database;
            self.ctx.db = take(new_db);
            result
        }
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
            .rwasm_module
            .serialize()
            .into()
    };
    let context_input = {
        let shared_ctx = SharedContextInputV1 {
            block: Default::default(),
            tx: Default::default(),
            contract: Default::default(),
        };
        let mut buf = BytesMut::new();
        buf.extend(shared_ctx.encode_to_vec().unwrap());
        buf.extend_from_slice(input_data);
        buf.freeze().to_vec()
    };
    let code_hash = keccak256(&rwasm_binary);
    let bytecode_or_hash = BytecodeOrHash::Bytecode {
        address: Address::ZERO,
        rwasm_module: Bytes::from(rwasm_binary),
        code_hash,
    };
    let ctx = RuntimeContext::new(bytecode_or_hash)
        .with_state(STATE_MAIN)
        .with_fuel_limit(100_000_000_000)
        .with_input(context_input);
    // .with_tracer();
    let mut runtime = Runtime::new(ctx);
    runtime.store.context_mut(|ctx| ctx.clear_output());
    let result = runtime.call();
    println!(
        "exit_code: {} ({})",
        result.exit_code,
        ExitCode::from(result.exit_code)
    );
    try_print_utf8_error(&result.output[..]);
    println!("fuel consumed: {}", result.fuel_consumed);
    // if result.exit_code != 0 {
    //     let logs = &runtime
    //         .executor
    //         .tracer()
    //         .map(|v| v.logs.clone())
    //         .unwrap_or_default();
    //     println!("execution trace ({} steps):", logs.len());
    //     for log in logs.iter().rev().take(100).rev() {
    //         println!(" - pc={} opcode={}", log.program_counter, log.opcode);
    //     }
    // } else {
    //     println!(
    //         "trace steps: {}",
    //         runtime
    //             .executor
    //             .tracer()
    //             .map(|v| v.logs.len())
    //             .unwrap_or_default()
    //     );
    // }
    (result.output.into(), result.exit_code)
}

#[allow(dead_code)]
pub fn catch_panic(ctx: &fluentbase_runtime::ExecutionResult) {
    if ctx.exit_code != -1 {
        return;
    }
    println!("panic with err: {}", from_utf8(&ctx.output).unwrap());
}
