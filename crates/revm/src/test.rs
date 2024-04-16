use crate::{Evm, InMemoryDB};
use core::mem::take;
use fluentbase_core::{helpers::calc_create_address, Account};
use fluentbase_genesis::devnet::{devnet_genesis_from_file, KECCAK_HASH_KEY, POSEIDON_HASH_KEY};
use fluentbase_genesis::{Genesis, EXAMPLE_GREETING_ADDRESS};
use fluentbase_poseidon::poseidon_hash;
use fluentbase_types::{address, Address, Bytes, KECCAK_EMPTY, POSEIDON_EMPTY, U256};
use revm_primitives::db::DatabaseCommit;
use revm_primitives::{
    hex, keccak256, AccountInfo, Bytecode, CreateScheme, Env, ExecutionResult, HashMap, TransactTo,
};

#[allow(dead_code)]
struct TestingContext {
    genesis: Genesis,
    db: InMemoryDB,
}

impl Default for TestingContext {
    fn default() -> Self {
        Self::load_from_genesis(devnet_genesis_from_file())
    }
}

#[allow(dead_code)]
impl TestingContext {
    fn load_from_genesis(genesis: Genesis) -> Self {
        // create jzkt and put it into testing context
        let mut db = InMemoryDB::default();
        // convert all accounts from genesis into jzkt
        for (k, v) in genesis.alloc.iter() {
            let poseidon_hash = v
                .storage
                .as_ref()
                .and_then(|v| v.get(&POSEIDON_HASH_KEY).cloned())
                .unwrap_or_else(|| {
                    v.code
                        .as_ref()
                        .map(|v| poseidon_hash(&v).into())
                        .unwrap_or(POSEIDON_EMPTY)
                });
            let keccak_hash = v
                .storage
                .as_ref()
                .and_then(|v| v.get(&KECCAK_HASH_KEY).cloned())
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
                source_code_size: v.code.as_ref().map(|v| v.len() as u64).unwrap_or_default(),
                source_code_hash: keccak_hash,
                rwasm_code_size: v.code.as_ref().map(|v| v.len() as u64).unwrap_or_default(),
                rwasm_code_hash: poseidon_hash,
            };
            let mut info: AccountInfo = account.into();
            info.code = v.code.clone().map(Bytecode::new_raw);
            info.rwasm_code = v.code.clone().map(Bytecode::new_raw);
            db.insert_account_info(*k, info);
        }
        Self { genesis, db }
    }

    pub(crate) fn get_balance(&mut self, address: Address) -> U256 {
        let account = self.db.load_account(address).unwrap();
        account.info.balance
    }

    pub(crate) fn add_balance(&mut self, address: Address, value: U256) {
        let account = self.db.load_account(address).unwrap();
        account.info.balance += value;
        let mut revm_account = revm_primitives::Account::from(account.info.clone());
        revm_account.mark_touch();
        self.db.commit(HashMap::from([(address, revm_account)]));
    }
}

struct TxBuilder<'a> {
    ctx: &'a mut TestingContext,
    env: Env,
}

#[allow(dead_code)]
impl<'a> TxBuilder<'a> {
    fn create(ctx: &'a mut TestingContext, deployer: Address, init_code: Bytes) -> Self {
        let mut env = Env::default();
        env.tx.caller = deployer;
        env.tx.transact_to = TransactTo::Create(CreateScheme::Create);
        env.tx.data = init_code;
        env.tx.gas_limit = 300_000_000;
        Self { ctx, env }
    }

    fn call(ctx: &'a mut TestingContext, caller: Address, callee: Address) -> Self {
        let mut env = Env::default();
        env.tx.gas_price = U256::from(1);
        env.tx.caller = caller;
        env.tx.transact_to = TransactTo::Call(callee);
        env.tx.gas_limit = 10_000_000;
        Self { ctx, env }
    }

    fn input(mut self, input: Bytes) -> Self {
        self.env.tx.data = input;
        self
    }

    fn value(mut self, value: U256) -> Self {
        self.env.tx.value = value;
        self
    }

    fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.env.tx.gas_limit = gas_limit;
        self
    }

    fn gas_price(mut self, gas_price: U256) -> Self {
        self.env.tx.gas_price = gas_price;
        self
    }

    fn exec(&mut self) -> ExecutionResult {
        let mut evm = Evm::builder()
            .with_env(Box::new(take(&mut self.env)))
            .with_db(&mut self.ctx.db)
            .build();
        evm.transact_commit().unwrap()
    }
}

fn deploy_evm_tx(ctx: &mut TestingContext, deployer: Address, init_bytecode: Bytes) -> Address {
    // deploy greeting EVM contract
    let result = TxBuilder::create(ctx, deployer, init_bytecode.into()).exec();
    assert!(result.is_success());
    let contract_address = calc_create_address(&deployer, 0);
    let contract_account = ctx.db.accounts.get(&contract_address).unwrap();
    let source_bytecode = ctx
        .db
        .contracts
        .get(&contract_account.info.code_hash)
        .unwrap()
        .bytes()
        .to_vec();
    assert_eq!(contract_account.info.code_hash, keccak256(&source_bytecode));
    assert!(source_bytecode.len() > 0);
    let rwasm_bytecode = ctx
        .db
        .contracts
        .get(&contract_account.info.rwasm_code_hash)
        .unwrap()
        .bytes()
        .to_vec();
    let is_rwasm = rwasm_bytecode.get(0).cloned().unwrap() == 0xef;
    assert!(is_rwasm);
    contract_address
}

fn call_evm_tx(
    ctx: &mut TestingContext,
    caller: Address,
    callee: Address,
    input: Bytes,
) -> ExecutionResult {
    ctx.add_balance(caller, U256::from(1e18));
    // call greeting EVM contract
    TxBuilder::call(ctx, caller, callee).input(input).exec()
}

#[test]
fn test_genesis_greeting() {
    let mut ctx = TestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        EXAMPLE_GREETING_ADDRESS,
        Bytes::default(),
    );
    assert!(result.is_success());
    println!("gas used (call): {}", result.gas_used());
    let bytes = result.output().unwrap_or_default();
    assert_eq!(
        "Hello, World",
        core::str::from_utf8(bytes.as_ref()).unwrap()
    );
}

#[test]
fn test_deploy_greeting() {
    // deploy greeting WASM contract
    let mut ctx = TestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = deploy_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        include_bytes!("../../../examples/bin/greeting.wasm").into(),
    );
    // call greeting WASM contract
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        contract_address,
        Bytes::default(),
    );
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default();
    assert_eq!(
        "Hello, World",
        core::str::from_utf8(bytes.as_ref()).unwrap()
    );
}

#[test]
fn test_deploy_keccak256() {
    // deploy greeting WASM contract
    let mut ctx = TestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = deploy_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        include_bytes!("../../../examples/bin/keccak256.wasm").into(),
    );
    // call greeting WASM contract
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        contract_address,
        "Hello, World".into(),
    );
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default();
    assert_eq!(
        "a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529",
        hex::encode(bytes.as_ref()),
    );
}

#[test]
fn test_deploy_panic() {
    // deploy greeting WASM contract
    let mut ctx = TestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = deploy_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        include_bytes!("../../../examples/bin/panic.wasm").into(),
    );
    // call greeting WASM contract
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        contract_address,
        Bytes::default(),
    );
    assert!(!result.is_success());
    let bytes = result.output().unwrap_or_default();
    assert_eq!(
        "panicked at examples/src/panic.rs:4:5: it is panic time",
        core::str::from_utf8(bytes.as_ref()).unwrap()
    );
}

#[test]
fn test_evm_greeting() {
    // deploy greeting EVM contract
    let mut ctx = TestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = deploy_evm_tx(&mut ctx, DEPLOYER_ADDRESS, hex!("60806040526105ae806100115f395ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e01c80633b2e97481461004357806345773e4e1461007357806348b8bcc314610091575b5f80fd5b61005d600480360381019061005891906102e5565b6100af565b60405161006a919061039a565b60405180910390f35b61007b6100dd565b604051610088919061039a565b60405180910390f35b61009961011a565b6040516100a6919061039a565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100d58161012f565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101298161012f565b91505090565b60605f8203610175576040518060400160405280600181526020017f30000000000000000000000000000000000000000000000000000000000000008152509050610282565b5f8290505f5b5f82146101a457808061018d906103f0565b915050600a8261019d9190610464565b915061017b565b5f8167ffffffffffffffff8111156101bf576101be610494565b5b6040519080825280601f01601f1916602001820160405280156101f15781602001600182028036833780820191505090505b5090505b5f851461027b578180610207906104c1565b925050600a8561021791906104e8565b60306102239190610518565b60f81b8183815181106102395761023861054b565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a856102749190610464565b94506101f5565b8093505050505b919050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102b48261028b565b9050919050565b6102c4816102aa565b81146102ce575f80fd5b50565b5f813590506102df816102bb565b92915050565b5f602082840312156102fa576102f9610287565b5b5f610307848285016102d1565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f5b8381101561034757808201518184015260208101905061032c565b5f8484015250505050565b5f601f19601f8301169050919050565b5f61036c82610310565b610376818561031a565b935061038681856020860161032a565b61038f81610352565b840191505092915050565b5f6020820190508181035f8301526103b28184610362565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f6103fa826103e7565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff820361042c5761042b6103ba565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f61046e826103e7565b9150610479836103e7565b92508261048957610488610437565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104cb826103e7565b91505f82036104dd576104dc6103ba565b5b600182039050919050565b5f6104f2826103e7565b91506104fd836103e7565b92508261050d5761050c610437565b5b828206905092915050565b5f610522826103e7565b915061052d836103e7565b9250828201905080821115610545576105446103ba565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea2646970667358221220feebf5ace29c3c3146cb63bf7ca9009c2005f349075639d267cfbd817adde3e564736f6c63430008180033").into());
    // call greeting EVM contract
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        contract_address,
        hex!("45773e4e").into(),
    );
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default();
    let bytes = &bytes[64..75];
    assert_eq!("Hello World", core::str::from_utf8(bytes.as_ref()).unwrap());
}

///
/// Test storage though constructor
///
/// ```solidity
/// // SPDX-License-Identifier: MIT
/// pragma solidity 0.8.24;
/// contract Storage {
///   uint256 private value;
///   constructor() payable {
///     value = 100;
///   }
///   function getValue() public view returns (uint256) {
///     return value;
///   }
/// }
/// ```
///
#[test]
fn test_evm_storage() {
    // deploy greeting EVM contract
    let mut ctx = TestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address_1 = deploy_evm_tx(&mut ctx, DEPLOYER_ADDRESS, hex!("608060405260645f8190555060af806100175f395ff3fe6080604052348015600e575f80fd5b50600436106026575f3560e01c80632096525514602a575b5f80fd5b60306044565b604051603b91906062565b60405180910390f35b5f8054905090565b5f819050919050565b605c81604c565b82525050565b5f60208201905060735f8301846055565b9291505056fea26469706673582212206a2e6da07d41af2063301a33093a60613dd63420518670788aa99d7d8f47625564736f6c63430008180033").into());
    let contract_address_2 = deploy_evm_tx(&mut ctx, DEPLOYER_ADDRESS, hex!("608060405260645f8190555060af806100175f395ff3fe6080604052348015600e575f80fd5b50600436106026575f3560e01c80632096525514602a575b5f80fd5b60306044565b604051603b91906062565b60405180910390f35b5f8054905090565b5f819050919050565b605c81604c565b82525050565b5f60208201905060735f8301846055565b9291505056fea26469706673582212206a2e6da07d41af2063301a33093a60613dd63420518670788aa99d7d8f47625564736f6c63430008180033").into());
    // call greeting EVM contract
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        contract_address_1,
        hex!("20965255").into(),
    );
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default();
    assert_eq!(
        "0000000000000000000000000000000000000000000000000000000000000064",
        hex::encode(bytes)
    );
    // call greeting EVM contract
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        contract_address_2,
        hex!("20965255").into(),
    );
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default().iter().as_slice();
    assert_eq!(
        "0000000000000000000000000000000000000000000000000000000000000064",
        hex::encode(bytes)
    );
}

#[test]
fn test_simple_send() {
    // deploy greeting EVM contract
    let mut ctx = TestingContext::default();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    const RECIPIENT_ADDRESS: Address = address!("1092381297182319023812093812312309123132");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = U256::from(1e9);
    let result = TxBuilder::call(&mut ctx, SENDER_ADDRESS, RECIPIENT_ADDRESS)
        .gas_price(gas_price)
        .value(U256::from(1e18))
        .exec();
    assert!(result.is_success());
    let tx_cost = gas_price * U256::from(result.gas_used());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18) - tx_cost);
    assert_eq!(ctx.get_balance(RECIPIENT_ADDRESS), U256::from(1e18));
}

#[test]
fn test_create_send() {
    // deploy greeting EVM contract
    let mut ctx = TestingContext::default();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = U256::from(2e9);
    let result = TxBuilder::create(
        &mut ctx,
        SENDER_ADDRESS,
        include_bytes!("../../../examples/bin/greeting.wasm").into(),
    )
    .gas_price(gas_price)
    .value(U256::from(1e18))
    .exec();
    let contract_address = calc_create_address(&SENDER_ADDRESS, 0);
    assert!(result.is_success());
    let tx_cost = gas_price * U256::from(result.gas_used());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18) - tx_cost);
    assert_eq!(ctx.get_balance(contract_address), U256::from(1e18));
}

#[test]
fn test_evm_revert() {
    // deploy greeting EVM contract
    let mut ctx = TestingContext::default();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = U256::from(0);
    let result = TxBuilder::create(&mut ctx, SENDER_ADDRESS, hex!("5f5ffd").into())
        .gas_price(gas_price)
        .value(U256::from(1e18))
        .exec();
    let contract_address = calc_create_address(&SENDER_ADDRESS, 0);
    assert!(!result.is_success());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(2e18));
    assert_eq!(ctx.get_balance(contract_address), U256::from(0e18));
}
