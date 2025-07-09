use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall, SolValue};
use fluentbase_sdk::{Address, Bytes};
use fluentbase_sdk_testing::EvmTestingContext;

/// Contract `ContractDeployer.sol` is a smart contract that deploys
/// the given smart contract using the CREATE opcode of the EVM.
/// Through this opcode, we should be able to deploy both WASM
/// and EVM bytecode.
fn deploy_via_deployer(ctx: &mut EvmTestingContext, bytecode: Bytes) -> Address {
    let owner: Address = Address::ZERO;
    let contract_address = ctx.deploy_evm_tx(
        owner,
        hex::decode(include_bytes!("../assets/ContractDeployer.bin"))
            .unwrap()
            .into(),
    );
    sol! {
        function deploy(bytes memory bytecode) public returns (address contractAddress);
    }
    let encoded_call = deployCall { bytecode }.abi_encode();
    let result = ctx.call_evm_tx(owner, contract_address, encoded_call.into(), None, None);
    assert!(
        result.is_success(),
        "call to \"deploy\" method of ContractDeployer.sol failed"
    );
    let address = <Address>::abi_decode_validate(result.output().unwrap()).unwrap();
    address
}

#[test]
fn test_evm_create_evm_contract() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let owner: Address = Address::ZERO;
    let bytecode = hex::decode(include_bytes!("../assets/HelloWorld.bin")).unwrap();
    let contract_address = deploy_via_deployer(&mut ctx, bytecode.into());
    sol! {
        function sayHelloWorld() public pure returns (string memory);
    }
    let encoded_call = sayHelloWorldCall {}.abi_encode();
    let result = ctx.call_evm_tx(owner, contract_address, encoded_call.into(), None, None);
    assert!(result.is_success());
    let string = <String>::abi_decode_validate(result.output().unwrap()).unwrap();
    assert_eq!(string, "Hello, World");
}

#[test]
fn test_evm_create_wasm_contract() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let owner: Address = Address::ZERO;
    let contract_address = deploy_via_deployer(&mut ctx, crate::EXAMPLE_GREETING.into());
    let result = ctx.call_evm_tx(owner, contract_address, Bytes::new(), None, None);
    println!("{:#?}", result);
    assert!(result.is_success());
    let output = result.output().unwrap().to_vec();
    assert_eq!(String::from_utf8(output).unwrap(), "Hello, World");
}

#[test]
fn test_evm_create_large_wasm_contract() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    deploy_via_deployer(&mut ctx, crate::EXAMPLE_ERC20.into());
}
