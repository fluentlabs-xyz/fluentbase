use alloc::vec::Vec;
use fluentbase_sdk::{address, Address, Bytes, U256};
use fluentbase_sdk_testing::EvmTestingContext;
use fluentbase_types::{ContractContextV1, PRECOMPILE_EIP2935, PRECOMPILE_ERC20};
use revm::{context::result::ExecutionResult, handler::SYSTEM_ADDRESS};

fn call_success(
    ctx: &mut EvmTestingContext,
    input: Bytes,
    caller: &Address,
    callee: &Address,
) -> Vec<u8> {
    let result = ctx.call_evm_tx(*caller, *callee, input, None, None);
    println!("result: {:?}", result);
    assert!(result.is_success());
    let output_data = result.output().unwrap().to_vec();
    output_data
}

fn call_revert(ctx: &mut EvmTestingContext, input: Bytes, caller: &Address, callee: &Address) {
    let result = ctx.call_evm_tx(*caller, *callee, input, None, None);
    match &result {
        ExecutionResult::Revert { output, .. } => {
            assert!(output.is_empty());
        }
        _ => {
            panic!("expected revert, got: {:?}", &result)
        }
    }
}

#[test]
fn eip2935_test() {
    let mut ctx = EvmTestingContext::default();
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_ERC20,
        ..Default::default()
    });
    const USER_ADDR: Address = address!("9437947297489237489237461545439472947329");

    let block_hash1: U256 = U256::from(53497298);
    let block_hash2: U256 = U256::from(453465346);
    let block_hash3: U256 = U256::from(12315143);

    let input = block_hash1.as_le_slice().to_vec().into();
    call_revert(&mut ctx, input, &SYSTEM_ADDRESS, &PRECOMPILE_EIP2935);

    let input = U256::from(0).as_le_slice().to_vec().into();
    call_revert(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    let input = U256::from(1).as_le_slice().to_vec().into();
    call_revert(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    let input = U256::from(3).as_le_slice().to_vec().into();
    call_revert(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);

    ctx.sdk = ctx.sdk.with_block_number(1);

    let input = block_hash1.as_le_slice().to_vec().into();
    let output_data = call_success(&mut ctx, input, &SYSTEM_ADDRESS, &PRECOMPILE_EIP2935);
    let recovered = U256::try_from_le_slice(&output_data).unwrap();
    let expected = U256::from(0);
    assert_eq!(expected, recovered);

    let input = U256::from(0).as_le_slice().to_vec().into();
    let output_data = call_success(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    let recovered = U256::try_from_le_slice(&output_data).unwrap();
    let expected = block_hash1;
    assert_eq!(expected, recovered);

    let input = U256::from(1).as_le_slice().to_vec().into();
    call_revert(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);

    ctx.sdk = ctx.sdk.with_block_number(2);

    let input = block_hash2.as_le_slice().to_vec().into();
    let output_data = call_success(&mut ctx, input, &SYSTEM_ADDRESS, &PRECOMPILE_EIP2935);
    let recovered = U256::try_from_le_slice(&output_data).unwrap();
    assert_eq!(recovered, U256::from(0));

    let input = U256::from(1).as_le_slice().to_vec().into();
    let output_data = call_success(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    let recovered = U256::try_from_le_slice(&output_data).unwrap();
    assert_eq!(recovered, block_hash2);

    let input = U256::from(2).as_le_slice().to_vec().into();
    call_revert(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);

    ctx.sdk = ctx.sdk.with_block_number(3);
    let input = block_hash3.as_le_slice().to_vec().into();
    let output_data = call_success(&mut ctx, input, &SYSTEM_ADDRESS, &PRECOMPILE_EIP2935);
    let recovered = U256::try_from_le_slice(&output_data).unwrap();
    assert_eq!(recovered, U256::from(0));

    let input = U256::from(2).as_le_slice().to_vec().into();
    let output_data = call_success(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    let recovered = U256::try_from_le_slice(&output_data).unwrap();
    assert_eq!(recovered, block_hash3);

    let input = U256::from(3).as_le_slice().to_vec().into();
    call_revert(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
}
