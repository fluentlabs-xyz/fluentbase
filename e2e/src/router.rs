use crate::utils::EvmTestingContext;
use fluentbase_codec::SolidityABI;
use fluentbase_sdk::{address, Address, U256};
use hex_literal::hex;

#[test]
fn test_client_solidity() {
    let mut ctx = EvmTestingContext::default();
    const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    ctx.add_balance(DEPLOYER_ADDRESS, U256::from(10e18));

    let (contract_address, _) = ctx.deploy_evm_tx_with_nonce(
        DEPLOYER_ADDRESS,
        crate::examples::EXAMPLE_ROUTER_SOLIDITY.into(),
        0,
    );
    println!("contract_address: {:?}", contract_address);

    let (client_address, _) = ctx.deploy_evm_tx_with_nonce(
        DEPLOYER_ADDRESS,
        crate::examples::EXAMPLE_CLIENT_SOLIDITY.into(),
        1,
    );
    println!("client_address: {:?}", client_address);

    ctx.add_balance(contract_address, U256::from(10e18));
    ctx.add_balance(client_address, U256::from(10e18));

    let client_input = hex!("f60ea708000000000000000000000000f91c20c0cafbfdc150adff51bbfc5808edde7cb50000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a4100000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000b48656c6c6f20576f726c64000000000000000000000000000000000000000000");

    println!("calling client...");
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        client_address,
        client_input.into(),
        None,
        None,
    );

    println!("result: {:?}", result);

    assert_eq!(result.is_success(), true);

    let output = result.output();
    println!("output: {:?}", output);
    let msg_b = result.output().unwrap();

    let msg: String = SolidityABI::decode(msg_b, 0).unwrap();

    assert_eq!(msg, "Hello World");
}
