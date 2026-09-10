//! The launch runbook separates the two bootstrap roles (see `docs/06-runtime-upgrade.md`,
//! "Bootstrap authorities"): the runtime-upgrade owner and the fee-manager owner start on the same
//! launch key, and one `changeOwner` per contract has to leave that key without either power while
//! giving each new owner exactly its own role.

use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::{
    address, bytes, Address, Bytes, DEFAULT_FEE_MANAGER_AUTH, DEFAULT_UPDATE_GENESIS_AUTH,
    PRECOMPILE_FEE_MANAGER, PRECOMPILE_RUNTIME_UPGRADE, U256,
};
use fluentbase_testing::EvmTestingContext;
use hex_literal::hex;

sol! {
    function owner() external view returns (address);
    function changeOwner(address new_owner) external;
    function withdraw(address recipient) external;
    function upgradeEvmTo(
        address target_address,
        uint256 genesis_hash,
        string genesis_version,
        bytes evm_bytecode
    );
}

fn read_owner(ctx: &mut EvmTestingContext, contract: Address) -> Address {
    let result = ctx.call_evm_tx(
        Address::ZERO,
        contract,
        ownerCall {}.abi_encode().into(),
        None,
        None,
    );
    assert!(result.is_success(), "{result:?}");
    ownerCall::abi_decode_returns_validate(result.output().unwrap()).unwrap()
}

fn change_owner(
    ctx: &mut EvmTestingContext,
    contract: Address,
    caller: Address,
    new_owner: Address,
) -> bool {
    ctx.call_evm_tx(
        caller,
        contract,
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

/// Replaces the runtime of `target` with one that returns its slot zero.
fn upgrade_evm_runtime(ctx: &mut EvmTestingContext, caller: Address, target: Address) -> bool {
    let new_runtime: Bytes = bytes!("60005460005260206000f3");
    ctx.call_evm_tx(
        caller,
        PRECOMPILE_RUNTIME_UPGRADE,
        upgradeEvmToCall {
            target_address: target,
            genesis_hash: U256::ZERO,
            genesis_version: "v1.0.1".to_string(),
            evm_bytecode: new_runtime.to_vec().into(),
        }
        .abi_encode()
        .into(),
        None,
        None,
    )
    .is_success()
}

#[test]
fn test_bootstrap_key_rotation_separates_upgrade_and_treasury_roles() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    const DEPLOYER: Address = address!("0x7777777777777777777777777777777777777777");
    let upgrade_owner = address!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let treasury_owner = address!("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    let recipient = address!("0xcccccccccccccccccccccccccccccccccccccccc");
    let fees = U256::from(1000);

    // An EVM contract whose runtime stores the first ABI argument in slot zero.
    let (target, _) = ctx.deploy_evm_tx_with_gas(
        DEPLOYER,
        hex!("6007600c60003960076000f360043560005500").into(),
    );

    // Launch state: both roles resolve to the same bootstrap key.
    let bootstrap = DEFAULT_UPDATE_GENESIS_AUTH;
    assert_eq!(read_owner(&mut ctx, PRECOMPILE_RUNTIME_UPGRADE), bootstrap);
    assert_eq!(
        read_owner(&mut ctx, PRECOMPILE_FEE_MANAGER),
        DEFAULT_FEE_MANAGER_AUTH
    );

    // Runbook: hand each role to its own key from the bootstrap key.
    assert!(change_owner(
        &mut ctx,
        PRECOMPILE_RUNTIME_UPGRADE,
        bootstrap,
        upgrade_owner
    ));
    assert!(change_owner(
        &mut ctx,
        PRECOMPILE_FEE_MANAGER,
        DEFAULT_FEE_MANAGER_AUTH,
        treasury_owner
    ));
    assert_eq!(
        read_owner(&mut ctx, PRECOMPILE_RUNTIME_UPGRADE),
        upgrade_owner
    );
    assert_eq!(read_owner(&mut ctx, PRECOMPILE_FEE_MANAGER), treasury_owner);
    assert_ne!(upgrade_owner, treasury_owner);

    // The retired bootstrap key holds neither role any more.
    ctx.add_balance(PRECOMPILE_FEE_MANAGER, fees);
    assert!(!upgrade_evm_runtime(&mut ctx, bootstrap, target));
    assert!(!change_owner(
        &mut ctx,
        PRECOMPILE_RUNTIME_UPGRADE,
        bootstrap,
        bootstrap
    ));
    assert!(!withdraw(&mut ctx, bootstrap, recipient));
    assert!(!change_owner(
        &mut ctx,
        PRECOMPILE_FEE_MANAGER,
        bootstrap,
        bootstrap
    ));

    // The upgrade owner cannot touch the treasury, and the treasury owner cannot install code.
    assert!(!withdraw(&mut ctx, upgrade_owner, recipient));
    assert!(!change_owner(
        &mut ctx,
        PRECOMPILE_FEE_MANAGER,
        upgrade_owner,
        upgrade_owner
    ));
    assert!(!upgrade_evm_runtime(&mut ctx, treasury_owner, target));
    assert!(!change_owner(
        &mut ctx,
        PRECOMPILE_RUNTIME_UPGRADE,
        treasury_owner,
        treasury_owner
    ));
    assert_eq!(ctx.get_balance(PRECOMPILE_FEE_MANAGER), fees);
    assert_eq!(
        read_owner(&mut ctx, PRECOMPILE_RUNTIME_UPGRADE),
        upgrade_owner
    );
    assert_eq!(read_owner(&mut ctx, PRECOMPILE_FEE_MANAGER), treasury_owner);

    // Each owner keeps exactly its own role.
    let old_code = ctx.get_code(target).unwrap().original_bytes();
    assert!(upgrade_evm_runtime(&mut ctx, upgrade_owner, target));
    assert_ne!(ctx.get_code(target).unwrap().original_bytes(), old_code);
    assert!(withdraw(&mut ctx, treasury_owner, recipient));
    assert_eq!(ctx.get_balance(recipient), fees);
}
