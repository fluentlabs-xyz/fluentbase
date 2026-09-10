use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall};
use fluentbase_codec::SolidityABI;
use fluentbase_sdk::{
    address, Address, DEFAULT_FEE_MANAGER_AUTH, PRECOMPILE_FEE_MANAGER, SYSTEM_ADDRESS, U256,
};
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

sol! {
    function owner() external view returns (address);
    function changeOwner(address new_owner) external;
    function renounceOwnership() external;
    function withdraw(address recipient) external;
}

fn read_owner(ctx: &mut EvmTestingContext) -> Address {
    let result = ctx.call_evm_tx(
        Address::ZERO,
        PRECOMPILE_FEE_MANAGER,
        ownerCall {}.abi_encode().into(),
        None,
        None,
    );
    assert!(result.is_success());
    ownerCall::abi_decode_returns_validate(result.output().unwrap()).unwrap()
}

fn change_owner(ctx: &mut EvmTestingContext, caller: Address, new_owner: Address) -> bool {
    ctx.call_evm_tx(
        caller,
        PRECOMPILE_FEE_MANAGER,
        changeOwnerCall { new_owner }.abi_encode().into(),
        None,
        None,
    )
    .is_success()
}

fn withdraw(ctx: &mut EvmTestingContext, caller: Address, recipient: Address) -> bool {
    ctx.call_evm_tx(
        caller,
        PRECOMPILE_FEE_MANAGER,
        withdrawCall { recipient }.abi_encode().into(),
        None,
        None,
    )
    .is_success()
}

/// Zero is the empty-slot sentinel that `owner()` maps to the genesis bootstrap key, so accepting
/// it as a transfer target would silently reactivate the retired launch authority.
#[test]
fn test_fee_manager_change_owner_to_zero_is_rejected() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let governance = address!("1234567890123456789012345678901234567890");

    // Bootstrap state: an unset slot resolves to the genesis authority.
    assert_eq!(read_owner(&mut ctx), DEFAULT_FEE_MANAGER_AUTH);

    // Nonzero handoff retires the bootstrap key.
    assert!(change_owner(&mut ctx, DEFAULT_FEE_MANAGER_AUTH, governance));
    assert_eq!(read_owner(&mut ctx), governance);

    // The zero transfer must revert before touching storage.
    assert!(!change_owner(&mut ctx, governance, Address::ZERO));
    assert_eq!(
        read_owner(&mut ctx),
        governance,
        "zero transfer restored the bootstrap authority"
    );

    // The retired genesis key stays powerless, and the current owner keeps its authority.
    assert!(!change_owner(
        &mut ctx,
        DEFAULT_FEE_MANAGER_AUTH,
        DEFAULT_FEE_MANAGER_AUTH
    ));
    assert_eq!(read_owner(&mut ctx), governance);
    assert!(change_owner(&mut ctx, governance, governance));
}

/// Withdrawal is the sink the retired key must not regain.
#[test]
fn test_fee_manager_withdraw_authority_survives_rejected_zero_transfer() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let governance = address!("1234567890123456789012345678901234567890");
    let recipient = Address::repeat_byte(0x11);
    let amount = U256::from(1000);

    assert!(change_owner(&mut ctx, DEFAULT_FEE_MANAGER_AUTH, governance));
    assert!(!change_owner(&mut ctx, governance, Address::ZERO));

    ctx.add_balance(PRECOMPILE_FEE_MANAGER, amount);

    // The bootstrap key cannot drain fees after the handoff.
    assert!(!withdraw(&mut ctx, DEFAULT_FEE_MANAGER_AUTH, recipient));
    assert_eq!(ctx.get_balance(PRECOMPILE_FEE_MANAGER), amount);

    // The real owner still can.
    assert!(withdraw(&mut ctx, governance, recipient));
    assert_eq!(ctx.get_balance(recipient), amount);
}

/// Renunciation stays the explicit fork-only exit and is not weakened by the zero-address guard.
#[test]
fn test_fee_manager_renounce_ownership_is_still_terminal() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let governance = address!("1234567890123456789012345678901234567890");

    assert!(change_owner(&mut ctx, DEFAULT_FEE_MANAGER_AUTH, governance));

    let result = ctx.call_evm_tx(
        governance,
        PRECOMPILE_FEE_MANAGER,
        renounceOwnershipCall {}.abi_encode().into(),
        None,
        None,
    );
    assert!(result.is_success());
    assert_eq!(read_owner(&mut ctx), SYSTEM_ADDRESS);

    // Neither the previous owner nor the retired bootstrap key can take it back.
    assert!(!change_owner(&mut ctx, governance, governance));
    assert!(!change_owner(
        &mut ctx,
        DEFAULT_FEE_MANAGER_AUTH,
        DEFAULT_FEE_MANAGER_AUTH
    ));
    assert_eq!(read_owner(&mut ctx), SYSTEM_ADDRESS);
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
