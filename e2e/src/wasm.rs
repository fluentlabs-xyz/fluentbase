use crate::{
    EvmTestingContextWithGenesis,
    EXAMPLE_CHECKMATE,
    EXAMPLE_ERC20,
    EXAMPLE_GREETING,
    EXAMPLE_JSON,
    EXAMPLE_KECCAK256,
    EXAMPLE_PANIC,
    EXAMPLE_RWASM,
    EXAMPLE_SECP256K1,
    EXAMPLE_SIMPLE_STORAGE,
    EXAMPLE_TINY_KECCAK256,
};
use core::str::from_utf8;
use fluentbase_codec::{bytes::BytesMut, SolidityABI};
use fluentbase_sdk::{bytes, Address, Bytes, U256};
use fluentbase_sdk_testing::EvmTestingContext;
use hex_literal::hex;
use revm::bytecode::Bytecode;
use rwasm::RwasmModule;
use std::str::from_utf8_unchecked;
use fluentbase_sdk::constructor::encode_constructor_params;

#[test]
fn test_wasm_greeting() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, EXAMPLE_GREETING.into());
    // call greeting WASM contract
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        Bytes::default(),
        None,
        None,
    );
    let output = result.output().unwrap_or_default();
    assert!(result.is_success());
    assert_eq!("Hello, World", from_utf8(output.as_ref()).unwrap());
    println!("Result: {:?}", result);
}

#[test]
fn test_wasm_tiny_keccak256() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, EXAMPLE_TINY_KECCAK256.into());
    // call greeting WASM contract
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        "Hello, World".into(),
        None,
        None,
    );

    println!("{:?}", result);
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default().as_ref();
    println!("bytes: {:?}", hex::encode(&bytes));
    assert_eq!(
        "a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529",
        hex::encode(&bytes[0..32]),
    );
}

#[test]
fn test_wasm_secp256k1() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, EXAMPLE_SECP256K1.into());
    // call greeting WASM contract
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        bytes!("a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529cf09dd8d0eb3c3968aca8846a249424e5537d3470f979ff902b57914dc77d02316bd29784f668a73cc7a36f4cc5b9ce704481e6cb5b1c2c832af02ca6837ebec044e3b81af9c2234cad09d679ce6035ed1392347ce64ce405f5dcd36228a25de6e47fd35c4215d1edf53e6f83de344615ce719bdb0fd878f6ed76f06dd277956de"),
        None,
        None,
    );
    println!("{:?}", result);
    assert!(result.is_success());
}

#[test]
fn test_wasm_checkmate() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, EXAMPLE_CHECKMATE.into());
    // call greeting WASM contract
    let mut input = BytesMut::new();
    SolidityABI::<(String, String)>::encode(
        &(
            "rnbq1k1r/1p1p3p/5npb/2pQ1p2/p1B1P2P/8/PPP2PP1/RNB1K1NR w KQ - 2 11".to_string(),
            "Qf7".to_string(),
        ),
        &mut input,
        0,
    )
    .unwrap();
    let input = input.freeze().to_vec();
    let result = ctx.call_evm_tx(DEPLOYER_ADDRESS, contract_address, input.into(), None, None);

    println!("{:?}", result);
    assert!(result.is_success());
}

#[test]
fn test_wasm_json() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, EXAMPLE_JSON.into());
    // call greeting WASM contract
    let input = "{\"message\": \"Hello, World\"}".as_bytes().to_vec();
    let result = ctx.call_evm_tx(DEPLOYER_ADDRESS, contract_address, input.into(), None, None);
    println!("{:?}", result);
    assert!(result.is_success());
    assert_eq!(
        result.output().unwrap_or_default().as_ref(),
        "Hello, World".as_bytes()
    );
}

#[test]
fn test_wasm_panic() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, EXAMPLE_PANIC.into());
    // call greeting WASM contract
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        Bytes::default(),
        None,
        None,
    );
    assert!(!result.is_success());
    let bytes = result.output().unwrap_or_default();
    assert_eq!("it's panic time", from_utf8(&bytes[..]).unwrap());
}

#[test]
fn test_wasm_erc20() {
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const OWNER_ADDRESS: Address = Address::ZERO;

    // Add constructor parameters
    let bytecode: &[u8] = EXAMPLE_ERC20.into();
    // constructor params for ERC20:
    //     name: "TestToken"
    //     symbol: "TST"
    //     initial_supply: 1_000_000
    // use examples/erc20/src/lib.rs print_constructor_params_hex() to regenerate
    let constructor_params = hex!("000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000f4240000000000000000000000000000000000000000000000000000000000000000954657374546f6b656e000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000035453540000000000000000000000000000000000000000000000000000000000");
    let encoded_constructor_params = encode_constructor_params(&constructor_params);
    let mut input: Vec<u8> = Vec::new();
    input.extend(bytecode);
    input.extend(encoded_constructor_params);

    let contract_address = ctx.deploy_evm_tx(OWNER_ADDRESS, input.into());

    // call with empty input (should fail)
    let result = ctx.call_evm_tx(
        OWNER_ADDRESS,
        contract_address,
        Bytes::default(),
        None,
        None,
    );
    assert!(!result.is_success());
    let transfer_coin = |ctx: &mut EvmTestingContext| {
        let result = ctx.call_evm_tx(
            OWNER_ADDRESS,
            contract_address,
            hex!("a9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001").into(),
            None,
            None,
        );
        println!("{:?}", result);
        assert!(result.is_success());
    };
    let check_balance = |ctx: &mut EvmTestingContext, expected: U256| {
        // retrieve balance
        let result = ctx.call_evm_tx(
            OWNER_ADDRESS,
            contract_address,
            hex!("70a082310000000000000000000000001111111111111111111111111111111111111111").into(),
            None,
            None,
        );
        println!("{:?}", result);
        assert!(result.is_success());
        // check balance
        let output = result.output().unwrap_or_default().clone();
        assert_eq!(&expected.to_be_bytes::<32>(), output.as_ref());
    };
    // transfer 1 coin
    transfer_coin(&mut ctx);
    check_balance(&mut ctx, U256::from(1));
    transfer_coin(&mut ctx);
    check_balance(&mut ctx, U256::from(2));
}

#[test]
fn test_wasm_simple_storage() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, EXAMPLE_SIMPLE_STORAGE.into());
    // call greeting WASM contract
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        Bytes::default(),
        None,
        None,
    );
    println!("{:?}", result);
    assert!(result.is_success());
}

#[test]
fn test_wasm_rwasm() {
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, EXAMPLE_RWASM.into());
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        EXAMPLE_GREETING.into(),
        None,
        None,
    );
    println!("{:?}", result);
    assert!(result.is_success());

    let output = result.output().unwrap_or_default();
    let (module, _) = RwasmModule::new(&output);
    assert!(module.code_section.len() > 0);
    assert!(unsafe { from_utf8_unchecked(&module.data_section).contains("Hello, World") })
}

#[test]
fn test_wasm_keccak256_gas_price() {
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    let contract_address = ctx.deploy_evm_tx(Address::ZERO, EXAMPLE_KECCAK256.into());
    let result = ctx.call_evm_tx(
        Address::ZERO,
        contract_address,
        "Hello, World".into(),
        None,
        None,
    );
    println!("{:?}", result);
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default().as_ref();
    println!("bytes: {:?}", hex::encode(&bytes));
    assert_eq!(
        "a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529",
        hex::encode(&bytes[0..32]),
    );
}

#[test]
fn deploy_and_load_wasm_contract() {
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let deployer_account = ctx.db.load_account(DEPLOYER_ADDRESS).unwrap();
    assert_eq!(deployer_account.info.nonce, 0);
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDRESS, EXAMPLE_GREETING.into());
    let deployer_account = ctx.db.load_account(DEPLOYER_ADDRESS).unwrap();
    assert_eq!(
        deployer_account.info.nonce, 1,
        "Nonce was not incremented after deployment, maybe database was not committed?"
    );
    let contract_account = ctx.db.load_account(contract_address).unwrap();
    assert!(contract_account.info.code.is_some());
    match contract_account.info.code.clone().unwrap() {
        Bytecode::Rwasm(bytes) => {
            assert!(!bytes.is_empty());
        }
        other => {
            panic!(
                "Expected Rwasm bytecode, found bytecode with len {}: {:?}",
                other.original_byte_slice().len(),
                other
            )
        }
    }
}
