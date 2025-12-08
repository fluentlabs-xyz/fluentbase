use crate::EvmTestingContextWithGenesis;
use core::str::from_utf8;
use fluentbase_codec::{bytes::BytesMut, SolidityABI};
use fluentbase_contracts::{
    FLUENTBASE_EXAMPLES_BALANCE, FLUENTBASE_EXAMPLES_CHECKMATE, FLUENTBASE_EXAMPLES_ERC20,
    FLUENTBASE_EXAMPLES_GREETING, FLUENTBASE_EXAMPLES_JSON, FLUENTBASE_EXAMPLES_KECCAK,
    FLUENTBASE_EXAMPLES_PANIC, FLUENTBASE_EXAMPLES_RWASM, FLUENTBASE_EXAMPLES_SECP256K1,
    FLUENTBASE_EXAMPLES_SHA256, FLUENTBASE_EXAMPLES_SIMPLE_STORAGE,
    FLUENTBASE_EXAMPLES_TINY_KECCAK, FLUENTBASE_EXAMPLES_UNWIPED_OUTPUT,
};
use fluentbase_sdk::{
    address, bytes, constructor::encode_constructor_params, Address, Bytes, U256,
};
use fluentbase_testing::EvmTestingContext;
use hex_literal::hex;
use revm::bytecode::Bytecode;
use rwasm::RwasmModule;
use std::str::from_utf8_unchecked;

#[test]
fn test_wasm_greeting() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_GREETING.wasm_bytecode.into(),
    );
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
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_TINY_KECCAK.wasm_bytecode.into(),
    );
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
fn test_wasm_sha256() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_SHA256.wasm_bytecode.into(),
    );
    // call greeting WASM contract
    let result = ctx.call_evm_tx(DEPLOYER_ADDRESS, contract_address, "abc".into(), None, None);
    println!("{:?}", result);
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default().as_ref();
    assert_eq!(
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        hex::encode(&bytes[0..32]),
    );
}

#[test]
fn test_wasm_secp256k1() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_SECP256K1.wasm_bytecode.into(),
    );
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
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_CHECKMATE.wasm_bytecode.into(),
    );
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
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_JSON.wasm_bytecode.into(),
    );
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
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_PANIC.wasm_bytecode.into(),
    );
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
    let bytecode: &[u8] = FLUENTBASE_EXAMPLES_ERC20.wasm_bytecode.into();
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
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_SIMPLE_STORAGE.wasm_bytecode.into(),
    );
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
#[ignore] // the pass passes, but it's very slow in rwasm mode
fn test_wasm_rwasm() {
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_RWASM.wasm_bytecode.into(),
    );
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        FLUENTBASE_EXAMPLES_GREETING.wasm_bytecode.into(),
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
    let contract_address = ctx.deploy_evm_tx(
        Address::ZERO,
        FLUENTBASE_EXAMPLES_KECCAK.wasm_bytecode.into(),
    );
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
fn test_wasm_deploy_and_load_wasm_contract() {
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let deployer_account = ctx.db.load_account(DEPLOYER_ADDRESS).unwrap();
    assert_eq!(deployer_account.info.nonce, 0);
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_GREETING.wasm_bytecode.into(),
    );
    let deployer_account = ctx.db.load_account(DEPLOYER_ADDRESS).unwrap();
    assert_eq!(
        deployer_account.info.nonce, 1,
        "Nonce was not incremented after deployment, maybe database was not committed?"
    );
    let contract_account = ctx.db.load_account(contract_address).unwrap();
    assert!(contract_account.info.code.is_some());
    match contract_account.info.code.clone().unwrap() {
        Bytecode::Rwasm(_code) => {}
        other => {
            panic!(
                "Expected Rwasm bytecode, found bytecode with len {}: {:?}",
                other.original_byte_slice().len(),
                other
            )
        }
    }
}

#[test]
fn test_wasm_balance_should_fail_on_oog() {
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    ctx.add_balance(
        address!("0x0000000000000000000000000000000000000001"),
        U256::from(123),
    );
    const DEPLOYER_ADDRESS: Address = address!("0x1111111111111111111111111111111111111111");
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_BALANCE.wasm_bytecode.into(),
    );
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        Bytes::new(),
        // 21k tx + 100 balance
        Some(21_090),
        None,
    );
    // all gas must be charged
    assert_eq!(result.gas_used(), 21_090);
    // it should halt, not revert or ok
    assert!(result.is_halt());
}

#[test]
fn test_wasm_balance_charge() {
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    ctx.add_balance(
        address!("0x0000000000000000000000000000000000000001"),
        U256::from(123),
    );
    const DEPLOYER_ADDRESS: Address = address!("0x1111111111111111111111111111111111111111");
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_BALANCE.wasm_bytecode.into(),
    );
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        Bytes::new(), // 0x01 -> 21237 0x0102 -> 21269
        Some(22_000),
        None,
    );
    let balance = U256::from_le_slice(result.output().unwrap_or_default().as_ref());
    assert_eq!(balance, U256::from(123));
}

#[test]
fn test_wasm_output_remains_unwiped_after_interruption() {
    let mut ctx = EvmTestingContext::default().with_minimal_genesis();
    ctx.add_balance(
        address!("0x0000000000000000000000000000000000000001"),
        U256::from(123),
    );
    const DEPLOYER_ADDRESS: Address = address!("0x1111111111111111111111111111111111111111");
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_UNWIPED_OUTPUT.wasm_bytecode.into(),
    );
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        contract_address,
        Bytes::new(),
        Some(22_000),
        None,
    );
    assert_eq!(result.output().unwrap_or_default().as_ref(), &[0x1]);
}
