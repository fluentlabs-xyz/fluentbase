use crate::EvmTestingContextWithGenesis;
use fluentbase_codec::SolidityABI;
use fluentbase_contracts::{
    FLUENTBASE_EXAMPLES_CLIENT_SOLIDITY, FLUENTBASE_EXAMPLES_ROUTER_SOLIDITY,
};
use fluentbase_sdk::{address, Address, U256};
use fluentbase_testing::EvmTestingContext;
use hex_literal::hex;

#[test]
fn test_client_solidity() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    ctx.add_balance(DEPLOYER_ADDRESS, U256::from(10e18));

    let contract_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_ROUTER_SOLIDITY.wasm_bytecode.into(),
    );
    let client_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDRESS,
        FLUENTBASE_EXAMPLES_CLIENT_SOLIDITY.wasm_bytecode.into(),
    );

    ctx.add_balance(contract_address, U256::from(10e18));
    println!("contract_address: {:?}", contract_address);
    ctx.add_balance(client_address, U256::from(10e18));

    let client_input = hex!("f60ea708000000000000000000000000f91c20c0cafbfdc150adff51bbfc5808edde7cb5000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000186a00000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000b48656c6c6f20576f726c64000000000000000000000000000000000000000000");

    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        client_address,
        client_input.into(),
        None,
        None,
    );

    println!("result: {:?}", result);
    assert_eq!(result.is_success(), true);
    let msg_b = result.output().unwrap();
    let msg: String = SolidityABI::decode(msg_b, 0).unwrap();
    assert_eq!(msg, "Hello World");
}
