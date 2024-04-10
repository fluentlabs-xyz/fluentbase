use crate::Evm;
use fluentbase_core::{helpers::calc_create_address, Account};
use fluentbase_genesis::{devnet::devnet_genesis, Genesis, EXAMPLE_GREETING_ADDRESS};
use fluentbase_runtime::DefaultEmptyRuntimeDatabase;
use fluentbase_sdk::LowLevelSDK;
use fluentbase_types::{Address, Bytes};
use revm_primitives::{hex, CreateScheme, Env, TransactTo};

#[allow(dead_code)]
struct TestingContext {
    genesis: Genesis,
    jzkt: DefaultEmptyRuntimeDatabase,
}

impl Default for TestingContext {
    fn default() -> Self {
        Self::load_from_genesis(devnet_genesis())
    }
}

impl TestingContext {
    fn load_from_genesis(genesis: Genesis) -> Self {
        // create jzkt and put it into testing context
        let jzkt = LowLevelSDK::with_default_jzkt();
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
fn test_genesis_greeting() {
    let _ctx = TestingContext::default();
    let mut env = Env::default();
    env.tx.transact_to = TransactTo::Call(EXAMPLE_GREETING_ADDRESS);
    env.tx.gas_limit = 3_000_000;
    let mut evm = Evm::builder()
        .with_env(Box::new(env))
        .with_empty_db()
        .build();
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
    let mut evm = Evm::builder()
        .with_env(Box::new(env))
        .with_empty_db()
        .build();
    let result = evm.transact().unwrap();
    assert!(result.result.is_success());
    println!("gas used (deploy): {}", result.result.gas_used());
    let contract_address = calc_create_address(&DEPLOYER_ADDRESS, 0);
    // call greeting WASM contract
    let mut env = Env::default();
    env.tx.transact_to = TransactTo::Call(contract_address);
    env.tx.gas_limit = 10_000_000;
    let mut evm = Evm::builder()
        .with_env(Box::new(env))
        .with_empty_db()
        .build();
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
fn test_evm_greeting() {
    // deploy greeting EVM contract
    let _ctx = TestingContext::default();
    let mut env = Env::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    env.tx.caller = DEPLOYER_ADDRESS;
    env.tx.transact_to = TransactTo::Create(CreateScheme::Create);
    env.tx.data = Bytes::from_static(&hex!("60806040526105ae806100115f395ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e01c80633b2e97481461004357806345773e4e1461007357806348b8bcc314610091575b5f80fd5b61005d600480360381019061005891906102e5565b6100af565b60405161006a919061039a565b60405180910390f35b61007b6100dd565b604051610088919061039a565b60405180910390f35b61009961011a565b6040516100a6919061039a565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100d58161012f565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101298161012f565b91505090565b60605f8203610175576040518060400160405280600181526020017f30000000000000000000000000000000000000000000000000000000000000008152509050610282565b5f8290505f5b5f82146101a457808061018d906103f0565b915050600a8261019d9190610464565b915061017b565b5f8167ffffffffffffffff8111156101bf576101be610494565b5b6040519080825280601f01601f1916602001820160405280156101f15781602001600182028036833780820191505090505b5090505b5f851461027b578180610207906104c1565b925050600a8561021791906104e8565b60306102239190610518565b60f81b8183815181106102395761023861054b565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a856102749190610464565b94506101f5565b8093505050505b919050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102b48261028b565b9050919050565b6102c4816102aa565b81146102ce575f80fd5b50565b5f813590506102df816102bb565b92915050565b5f602082840312156102fa576102f9610287565b5b5f610307848285016102d1565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f5b8381101561034757808201518184015260208101905061032c565b5f8484015250505050565b5f601f19601f8301169050919050565b5f61036c82610310565b610376818561031a565b935061038681856020860161032a565b61038f81610352565b840191505092915050565b5f6020820190508181035f8301526103b28184610362565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f6103fa826103e7565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff820361042c5761042b6103ba565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f61046e826103e7565b9150610479836103e7565b92508261048957610488610437565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104cb826103e7565b91505f82036104dd576104dc6103ba565b5b600182039050919050565b5f6104f2826103e7565b91506104fd836103e7565b92508261050d5761050c610437565b5b828206905092915050565b5f610522826103e7565b915061052d836103e7565b9250828201905080821115610545576105446103ba565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea2646970667358221220feebf5ace29c3c3146cb63bf7ca9009c2005f349075639d267cfbd817adde3e564736f6c63430008180033"));
    env.tx.gas_limit = 3_000_000;
    let mut evm = Evm::builder()
        .with_env(Box::new(env))
        .with_empty_db()
        .build();
    let result = evm.transact().unwrap();
    assert!(result.result.is_success());
    println!("gas used (deploy): {}", result.result.gas_used());
    let contract_address = calc_create_address(&DEPLOYER_ADDRESS, 0);
    // call greeting EVM contract
    let mut env = Env::default();
    env.tx.transact_to = TransactTo::Call(contract_address);
    env.tx.data = Bytes::from_static(&hex!("45773e4e"));
    env.tx.gas_limit = 10_000_000;
    let mut evm = Evm::builder()
        .with_env(Box::new(env))
        .with_empty_db()
        .build();
    let result = evm.transact().unwrap();
    assert!(result.result.is_success());
    println!("gas used (call): {}", result.result.gas_used());
    let bytes = result.result.output().unwrap_or_default();
    let bytes = &bytes[64..75];
    assert_eq!("Hello World", core::str::from_utf8(bytes.as_ref()).unwrap());
}
