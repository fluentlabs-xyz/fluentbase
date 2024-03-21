use crate::EVM;
use fluentbase_core::{helpers::calc_create_address, Account};
use fluentbase_genesis::{devnet::devnet_genesis, Genesis, EXAMPLE_GREETING_ADDRESS};
use fluentbase_runtime::{types::InMemoryTrieDb, zktrie::ZkTrieStateDb, JournaledTrie};
use fluentbase_sdk::LowLevelSDK;
use fluentbase_types::Address;
use revm_primitives::{CreateScheme, Env, TransactTo};
use std::{cell::RefCell, rc::Rc};

struct TestingContext {
    genesis: Genesis,
    jzkt: Rc<RefCell<JournaledTrie<ZkTrieStateDb<InMemoryTrieDb>>>>,
}

impl Default for TestingContext {
    fn default() -> Self {
        Self::load_from_genesis(devnet_genesis())
    }
}

impl TestingContext {
    fn load_from_genesis(genesis: Genesis) -> Self {
        // create jzkt and put it into testing context
        let jzkt = Rc::new(RefCell::new(JournaledTrie::new(ZkTrieStateDb::new(
            InMemoryTrieDb::default(),
        ))));
        LowLevelSDK::with_jzkt(jzkt.clone());
        // convert all accounts from genesis into jzkt
        for (k, v) in genesis.alloc.iter() {
            let mut account = Account {
                address: *k,
                balance: v.balance,
                nonce: v.nonce.unwrap_or_default(),
                ..Default::default()
            };
            if let Some(code) = &v.code {
                account.update_rwasm_bytecode(code);
            } else {
                account.write_to_jzkt();
            }
        }
        Self { genesis, jzkt }
    }
}

#[test]
fn test_simple_greeting() {
    let _ctx = TestingContext::default();
    let mut env = Env::default();
    env.tx.transact_to = TransactTo::Call(EXAMPLE_GREETING_ADDRESS);
    env.tx.gas_limit = 3_000_000;
    let mut evm = EVM::with_env(env);
    let result = evm.transact().unwrap();
    assert!(result.result.is_success());
    println!("gas used (call): {}", result.result.gas_used());
    let bytes = result.result.output().unwrap_or_default();
    assert_eq!(
        "Hello, World",
        core::str::from_utf8(bytes.as_ref()).unwrap()
    );
}

#[test]
fn test_deploy_greeting() {
    // deploy greeting WASM contract
    let _ctx = TestingContext::default();
    let mut env = Env::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    env.tx.caller = DEPLOYER_ADDRESS;
    env.tx.transact_to = TransactTo::Create(CreateScheme::Create);
    env.tx.data = include_bytes!("../../../examples/bin/greeting.wasm").into();
    env.tx.gas_limit = 3_000_000;
    let mut evm = EVM::with_env(env);
    let result = evm.transact().unwrap();
    assert!(result.result.is_success());
    println!("gas used (deploy): {}", result.result.gas_used());
    let contract_address = calc_create_address(&DEPLOYER_ADDRESS, 0);
    // call greeting WASM contract
    let mut env = Env::default();
    env.tx.transact_to = TransactTo::Call(contract_address);
    env.tx.gas_limit = 10_000_000;
    let mut evm = EVM::with_env(env);
    let result = evm.transact().unwrap();
    assert!(result.result.is_success());
    println!("gas used (call): {}", result.result.gas_used());
    let bytes = result.result.output().unwrap_or_default();
    assert_eq!(
        "Hello, World",
        core::str::from_utf8(bytes.as_ref()).unwrap()
    );
}
