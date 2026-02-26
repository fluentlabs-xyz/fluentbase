use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall};
use fluentbase_codec::SolidityABI;
use fluentbase_sdk::{address, Address, DEFAULT_FEE_MANAGER_AUTH, PRECOMPILE_FEE_MANAGER, U256};
use fluentbase_testing::EvmTestingContext;

#[test]
fn test_fee_manager_owner() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    // Check initial owner
    sol! { function owner() external view returns (address); }
    let owner_input = ownerCall {}.abi_encode();
    let result = ctx.call_evm_tx(
        Address::ZERO,
        PRECOMPILE_FEE_MANAGER,
        owner_input.into(),
        None,
        None,
    );
    assert!(result.is_success());
    let owner: Address = ownerCall::abi_decode_returns_validate(result.output().unwrap()).unwrap();
    assert_eq!(owner, DEFAULT_FEE_MANAGER_AUTH);
}

#[test]
fn test_fee_manager_change_owner() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let new_owner = address!("1234567890123456789012345678901234567890");

    // Change owner (called by DEFAULT_FEE_MANAGER_AUTH)
    sol! { function changeOwner(address new_owner) external; }
    let change_owner_input = changeOwnerCall { new_owner }.abi_encode();
    let result = ctx.call_evm_tx(
        DEFAULT_FEE_MANAGER_AUTH,
        PRECOMPILE_FEE_MANAGER,
        change_owner_input.into(),
        None,
        None,
    );
    assert!(result.is_success());

    // Verify a new owner
    sol! { function owner() external view returns (address); }
    let owner_input = ownerCall {}.abi_encode();
    let result = ctx.call_evm_tx(
        Address::ZERO,
        PRECOMPILE_FEE_MANAGER,
        owner_input.into(),
        None,
        None,
    );
    assert!(result.is_success());
    let owner: Address = ownerCall::abi_decode_returns_validate(result.output().unwrap()).unwrap();
    assert_eq!(owner, new_owner);
}

#[test]
fn test_fee_manager_change_owner_unauthorized() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let unauthorized_caller = address!("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
    let new_owner = address!("1234567890123456789012345678901234567890");

    // Attempt to change owner from unauthorized caller
    sol! { function changeOwner(address new_owner) external; }
    let change_owner_input = changeOwnerCall { new_owner }.abi_encode();
    let result = ctx.call_evm_tx(
        unauthorized_caller,
        PRECOMPILE_FEE_MANAGER,
        change_owner_input.into(),
        None,
        None,
    );
    assert!(!result.is_success());
}

#[test]
fn test_fee_manager_withdraw() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let recipient = Address::repeat_byte(0x11);
    let amount = U256::from(1000);

    // Add balance to the fee manager
    ctx.add_balance(PRECOMPILE_FEE_MANAGER, amount);
    assert_eq!(ctx.get_balance(PRECOMPILE_FEE_MANAGER), amount);

    // Withdraw (called by DEFAULT_FEE_MANAGER_AUTH)
    sol! { function withdraw(address recipient) external; }
    let withdraw_input = withdrawCall { recipient }.abi_encode();
    let result = ctx.call_evm_tx(
        DEFAULT_FEE_MANAGER_AUTH,
        PRECOMPILE_FEE_MANAGER,
        withdraw_input.into(),
        None,
        None,
    );
    assert!(result.is_success());

    let new_balance = ctx.get_balance(recipient);
    assert_eq!(new_balance, amount);

    // Note: The current implementation emits `FeeWithdrawn` and does not transfer; success indicates positive balance and correct auth.
}

#[test]
fn test_fee_manager_withdraw_no_balance() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let recipient = address!("0000000000000000000000000000000000000001");

    // Withdraw without a balance (should fail)
    sol! { function withdraw(address recipient) external; }
    let withdraw_input = withdrawCall { recipient }.abi_encode();
    let result = ctx.call_evm_tx(
        DEFAULT_FEE_MANAGER_AUTH,
        PRECOMPILE_FEE_MANAGER,
        withdraw_input.into(),
        None,
        None,
    );
    assert!(!result.is_success());
}
