use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::{keccak256, Address, Bytes, PRECOMPILE_CREATE2_FACTORY, U256};
use fluentbase_testing::EvmTestingContext;

fn minimal_init_code() -> Bytes {
    Bytes::from_static(fluentbase_contracts::FLUENTBASE_EXAMPLES_SIMPLE_STORAGE.wasm_bytecode)
}

#[test]
fn test_create2_factory_deploy_and_predict_create2_address() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let caller = Address::repeat_byte(0x11);
    let salt = U256::from(0xdecafbad_u64);
    let init_code = minimal_init_code();
    let init_code_hash = keccak256(init_code.as_ref());

    sol! {
        function computeCreate2Address(uint256 salt, bytes32 init_code_hash) external view returns (address);
        function deployCreate2(uint256 salt, bytes init_code) external returns (address);
    }

    let predict_input = computeCreate2AddressCall {
        salt,
        init_code_hash,
    }
    .abi_encode();

    let predict_result = ctx.call_evm_tx(
        caller,
        PRECOMPILE_CREATE2_FACTORY,
        predict_input.into(),
        None,
        None,
    );
    assert!(predict_result.is_success(), "predict call failed: {predict_result:?}");
    let predicted: Address =
        computeCreate2AddressCall::abi_decode_returns_validate(predict_result.output().unwrap())
            .unwrap();

    let deploy_input = deployCreate2Call {
        salt,
        init_code: init_code.clone(),
    }
    .abi_encode();

    let deploy_result = ctx.call_evm_tx(
        caller,
        PRECOMPILE_CREATE2_FACTORY,
        deploy_input.into(),
        None,
        None,
    );
    assert!(deploy_result.is_success(), "deploy2 call failed: {deploy_result:?}");
    let deployed: Address =
        deployCreate2Call::abi_decode_returns_validate(deploy_result.output().unwrap()).unwrap();

    assert_eq!(deployed, predicted);
    assert!(ctx.get_code(deployed).is_some(), "deployed account has no code");
}

#[test]
fn test_create2_factory_deploy2_collision_fails() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let caller = Address::repeat_byte(0x22);
    let salt = U256::from(7u64);
    let init_code = minimal_init_code();

    sol! {
        function deployCreate2(uint256 salt, bytes init_code) external returns (address);
    }

    let input = deployCreate2Call {
        salt,
        init_code: init_code.clone(),
    }
    .abi_encode();

    let first = ctx.call_evm_tx(caller, PRECOMPILE_CREATE2_FACTORY, input.clone().into(), None, None);
    assert!(first.is_success(), "first deploy2 failed: {first:?}");

    let second = ctx.call_evm_tx(caller, PRECOMPILE_CREATE2_FACTORY, input.into(), None, None);
    assert!(!second.is_success(), "second deploy2 should fail due to address collision");
}

