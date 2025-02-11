use crate::utils::EvmTestingContext;
use fluentbase_codec::{CompactABI, SolidityABI};
use fluentbase_sdk::{address, Address, U256};
use hex_literal::hex;

#[test]
fn test_client_solidity() {
    let mut ctx = EvmTestingContext::default();
    const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    ctx.add_balance(DEPLOYER_ADDRESS, U256::from(10e18));

    let (contract_address, _) = ctx.deploy_evm_tx_with_nonce(
        DEPLOYER_ADDRESS,
        include_bytes!("../../examples/router-solidity/lib.wasm").into(),
        0,
    );
    println!("contract_address: {:?}", contract_address);

    let (client_address, _) = ctx.deploy_evm_tx_with_nonce(
        DEPLOYER_ADDRESS,
        include_bytes!("../../examples/client-solidity/lib.wasm").into(),
        1,
    );
    println!("client_address: {:?}", client_address);

    ctx.add_balance(contract_address, U256::from(10e18));
    ctx.add_balance(client_address, U256::from(10e18));

    let client_input = hex!("f60ea708000000000000000000000000f91c20c0cafbfdc150adff51bbfc5808edde7cb5000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000052080000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000b48656c6c6f20576f726c64000000000000000000000000000000000000000000");

    println!("calling client...");
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        client_address,
        client_input.into(),
        None,
        None,
    );

    assert_eq!(result.is_success(), true);

    let output = result.output();
    println!("output: {:?}", output);
    let msg_b = result.output().unwrap();

    let msg: String = SolidityABI::decode(msg_b, 0).unwrap();

    assert_eq!(msg, "Hello World");
}

#[test]
fn test_client_fluent() {
    let mut ctx = EvmTestingContext::default();
    const DEPLOYER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    ctx.add_balance(DEPLOYER_ADDRESS, U256::from(10e18));

    let (contract_address, _) = ctx.deploy_evm_tx_with_nonce(
        DEPLOYER_ADDRESS,
        include_bytes!("../../examples/router-fluent/lib.wasm").into(),
        0,
    );
    println!("contract_address: {:?}", contract_address);

    let (client_address, _) = ctx.deploy_evm_tx_with_nonce(
        DEPLOYER_ADDRESS,
        include_bytes!("../../examples/client-fluent/lib.wasm").into(),
        1,
    );
    println!("client_address: {:?}", client_address);

    ctx.add_balance(contract_address, U256::from(10e18));
    ctx.add_balance(client_address, U256::from(10e18));

    let client_input = hex!("f60ea708f91c20c0cafbfdc150adff51bbfc5808edde7cb500000000000000000000000000000000000000000000000000000000000000000852000000000000440000000b00000048656c6c6f20576f726c6400");

    println!("calling client...");
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDRESS,
        client_address,
        client_input.into(),
        None,
        None,
    );

    assert_eq!(result.is_success(), true);

    let _output = result.output();
    let msg_b = result.output().unwrap();

    let msg: String = CompactABI::decode(msg_b, 0).unwrap();

    assert_eq!(msg, "Hello World");
}

// #[test]
// fn test_codec_case() {
//     let call_method_input = EvmCallMethodInput {
//         caller: Default::default(),
//         address: address!("095e7baea6a6c7c4c2dfeb977efac326af552d87"),
//         bytecode_address: address!("095e7baea6a6c7c4c2dfeb977efac326af552d87"),
//         value: U256::from_be_slice(
//             &hex::decode("0x00000000000000000000000000000000000000000000000000000000000186a0")
//                 .unwrap(),
//         ),
//         apparent_value: Default::default(),
//         input: Bytes::copy_from_slice(&hex::decode("").unwrap()),
//         gas_limit: 9999979000,
//         depth: 0,
//         is_static: false,
//     };
//     let call_method_input_encoded = call_method_input.encode_to_vec(0);
//     let mut buffer = BufferDecoder::new(&call_method_input_encoded);
//     let mut call_method_input_decoded = EvmCallMethodInput::default();
//     EvmCallMethodInput::decode_body(&mut buffer, 0, &mut call_method_input_decoded);
//     assert_eq!(call_method_input_decoded.callee, call_method_input.callee);
// }
