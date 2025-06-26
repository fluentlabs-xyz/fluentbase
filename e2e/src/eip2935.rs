use alloc::vec::Vec;
use fluentbase_eip2935::helpers::{slice_from_u256, u256_try_from_slice};
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

fn call_revert(
    ctx: &mut EvmTestingContext,
    input: Bytes,
    caller: &Address,
    callee: &Address,
) -> u32 {
    let result = ctx.call_evm_tx(*caller, *callee, input, None, None);
    match &result {
        ExecutionResult::Revert {
            gas_used: _,
            output,
        } => {
            let error_code = u32::from_be_bytes(output[32..].try_into().unwrap());
            error_code
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

    let input = slice_from_u256(&block_hash1).to_vec().into();
    let output_error = call_revert(&mut ctx, input, &SYSTEM_ADDRESS, &PRECOMPILE_EIP2935);
    assert_eq!(output_error, 1);

    let input = slice_from_u256(&U256::from(0)).to_vec().into();
    let output_error = call_revert(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    assert_eq!(output_error, 1);
    let input = slice_from_u256(&U256::from(1)).to_vec().into();
    let output_error = call_revert(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    assert_eq!(output_error, 1);
    let input = slice_from_u256(&U256::from(3)).to_vec().into();
    let output_error = call_revert(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    assert_eq!(output_error, 1);

    ctx.sdk = ctx.sdk.with_block_number(1);

    let input = slice_from_u256(&block_hash1).to_vec().into();
    let output_data = call_success(&mut ctx, input, &SYSTEM_ADDRESS, &PRECOMPILE_EIP2935);
    let recovered = u256_try_from_slice(&output_data).unwrap();
    let expected = U256::from(0);
    assert_eq!(expected, recovered);

    let input = slice_from_u256(&U256::from(0)).to_vec().into();
    let output_data = call_success(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    let recovered = u256_try_from_slice(&output_data).unwrap();
    let expected = block_hash1;
    assert_eq!(expected, recovered);

    let input = slice_from_u256(&U256::from(1)).to_vec().into();
    let output_error = call_revert(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    assert_eq!(output_error, 1);

    ctx.sdk = ctx.sdk.with_block_number(2);

    let input = slice_from_u256(&block_hash2).to_vec().into();
    let output_data = call_success(&mut ctx, input, &SYSTEM_ADDRESS, &PRECOMPILE_EIP2935);
    let recovered = u256_try_from_slice(&output_data).unwrap();
    assert_eq!(recovered, U256::from(0));

    let input = slice_from_u256(&U256::from(1)).to_vec().into();
    let output_data = call_success(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    let recovered = u256_try_from_slice(&output_data).unwrap();
    assert_eq!(recovered, block_hash2);

    let input = slice_from_u256(&U256::from(2)).to_vec().into();
    let output_error = call_revert(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    assert_eq!(output_error, 1);

    ctx.sdk = ctx.sdk.with_block_number(3);
    let input = slice_from_u256(&block_hash3).to_vec().into();
    let output_data = call_success(&mut ctx, input, &SYSTEM_ADDRESS, &PRECOMPILE_EIP2935);
    let recovered = u256_try_from_slice(&output_data).unwrap();
    assert_eq!(recovered, U256::from(0));

    let input = slice_from_u256(&U256::from(2)).to_vec().into();
    let output_data = call_success(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    let recovered = u256_try_from_slice(&output_data).unwrap();
    assert_eq!(recovered, block_hash3);

    let input = slice_from_u256(&U256::from(3)).to_vec().into();
    let output_error = call_revert(&mut ctx, input, &USER_ADDR, &PRECOMPILE_EIP2935);
    assert_eq!(output_error, 1);
}
