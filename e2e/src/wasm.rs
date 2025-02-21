use crate::utils::EvmTestingContext;
use core::str::from_utf8;
use fluentbase_sdk::{Address, Bytes, U256};
use hex_literal::hex;

#[test]
fn test_wasm_greeting() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        include_bytes!("../../examples/greeting/lib.wasm").into(),
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
    println!("Result: {:?}", result);
    assert!(result.is_success());
    assert_eq!("Hello, World", from_utf8(output.as_ref()).unwrap());
}

#[test]
fn test_wasm_keccak256() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        include_bytes!("../../examples/keccak256/lib.wasm").into(),
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
fn test_wasm_panic() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        include_bytes!("../../examples/panic/lib.wasm").into(),
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
    assert_eq!(
        "it's panic time\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        from_utf8(&bytes.as_ref()[68..]).unwrap()
    );
}

#[test]
fn test_wasm_erc20() {
    // deploy greeting WASM contract
    let mut ctx = EvmTestingContext::default();
    const OWNER_ADDRESS: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        OWNER_ADDRESS,
        include_bytes!("../../examples/erc20/lib.wasm").into(),
    );
    // call greeting WASM contract
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
