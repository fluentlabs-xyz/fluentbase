use crate::EvmTestingContextWithGenesis;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{sol, SolCall};
use core::str::from_utf8;
use fluentbase_contracts::{FLUENTBASE_EXAMPLES_ERC20, FLUENTBASE_EXAMPLES_GREETING};
use fluentbase_sdk::{
    address, bytes, calc_create_address, constructor::encode_constructor_params, Address, Bytes,
    PRECOMPILE_BLAKE2F, PRECOMPILE_CREATE2_FACTORY, PRECOMPILE_SECP256K1_RECOVER, U256,
};
use fluentbase_testing::{try_print_utf8_error, EvmTestingContext, TxBuilder};
use hex_literal::hex;
use revm::{
    bytecode::{opcode, Bytecode},
    context::{result::ExecutionResult::Revert, transaction::Authorization},
    Database,
};

fn new_signer() -> PrivateKeySigner {
    "0xf0bc949485d112791637d7eb29dea3fd1e0758e8fea3ef542a4245bc896736cc"
        .parse()
        .unwrap()
}

fn signed_auth(
    signer: &PrivateKeySigner,
    chain_id: U256,
    delegated_address: Address,
    nonce: u64,
) -> revm::context::transaction::SignedAuthorization {
    let authorization = Authorization {
        chain_id,
        address: delegated_address,
        nonce,
    };
    let auth_sig = signer
        .sign_hash_sync(&authorization.signature_hash())
        .unwrap();
    authorization.into_signed(auth_sig)
}

#[test]
fn test_evm_eip7702_call_delegated_account() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let signer: PrivateKeySigner = new_signer();

    println!("-------------------");

    let sender: Address = signer.address();
    println!("sender: {:?}", sender);
    ctx.add_balance(sender, U256::from(10u128.pow(18)));

    // 1. deploy Dummy contract
    let delegate_to = ctx.deploy_evm_tx(
        sender,
        hex::decode(include_str!("../assets/Ping.bin"))
            .unwrap()
            .into(),
    );
    println!("delegate_to: {:?}", delegate_to);
    let ctor_value = ctx.db.storage(delegate_to, U256::ZERO).unwrap();
    assert_eq!(ctor_value, U256::ONE);

    // 2. set delegation via EIP-7702
    // after deploy nonce = 1, setup tx will use nonce=1 and increment to 2
    // so auth_nonce must equal the nonce at the moment of auth check = 2
    let outer_nonce = ctx.nonce(sender);
    let auth_nonce = outer_nonce + 1;
    println!("outer_nonce={outer_nonce}, auth_nonce={auth_nonce}");

    let signed_auth = signed_auth(
        &signer,
        U256::from(ctx.cfg.chain_id),
        delegate_to,
        auth_nonce,
    );

    // setup tx: send to Address::ZERO just to not trigger any code execution,
    // authorization list is processed regardless of callee
    let setup_result =
        TxBuilder::call7702(&mut ctx, sender, Address::ZERO, vec![signed_auth], None)
            .gas_limit(100_000)
            .exec();
    println!("setup_result: {:?}", setup_result);
    assert!(setup_result.is_success(), "7702 setup failed");

    let code = ctx.get_code(sender);
    println!("sender code after setup: {:?}", code);

    // 3. call sender.ping(0x7b) → should run DUMMY code in sender's context
    // ping(uint256) selector = 0x773acdef
    let ping_input = Bytes::from(
        hex!("773acdef000000000000000000000000000000000000000000000000000000000000007b").to_vec(),
    );

    let call_result = TxBuilder::call(&mut ctx, sender, sender, None)
        .input(ping_input)
        .gas_limit(100_000)
        .exec();
    println!("call_result: {:?}", call_result);
    assert!(
        call_result.is_success(),
        "ping call failed: {:?}",
        call_result
    );

    let output = call_result.output().unwrap_or_default();
    assert_eq!(output.len(), 32);
    // ping returns input + 1 = 0x7c
    assert_eq!(
        hex::encode(output),
        "000000000000000000000000000000000000000000000000000000000000007c"
    );

    // 4. call sender.value() → should reflect storage written in sender's context
    // value() selector = 0x3fa4f245
    let value_result = TxBuilder::call(&mut ctx, sender, sender, None)
        .input(Bytes::from(hex!("3fa4f245").to_vec()))
        .gas_limit(100_000)
        .exec();
    println!("value_result: {:?}", value_result);
    assert!(value_result.is_success(), "value() call failed");

    let value_output = value_result.output().unwrap_or_default();
    assert_eq!(value_output.len(), 32);
    // value() returns the stored value = 0x7b (written by ping)
    assert_eq!(
        hex::encode(value_output),
        "000000000000000000000000000000000000000000000000000000000000007b"
    );

    let ctor_value = ctx.db.storage(delegate_to, U256::ZERO).unwrap();
    assert_eq!(ctor_value, U256::ONE);
}

#[test]
fn test_evm_eip7702_state_override_like_estimate_gas_case() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let deadbeef = address!("deadbeef00000000000000000000000000000000");

    // 0xef0100 ++ 20-byte delegated address (0x...01 = ecrecover precompile)
    let delegated_code = Bytes::from(
        hex::decode("ef01000000000000000000000000000000000000000001")
            .unwrap()
            .to_vec(),
    );
    ctx.add_bytecode(deadbeef, delegated_code);
    ctx.add_balance(deadbeef, U256::from(10u128.pow(18)));

    let input = Bytes::from(
        hex::decode("0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000")
            .unwrap()
            .to_vec(),
    );

    let result = TxBuilder::call(&mut ctx, deadbeef, deadbeef, Some(U256::ZERO))
        .gas_limit(1_000_000)
        .input(input)
        .exec();

    assert!(result.is_success(), "estimate-like call failed: {result:?}");
}

#[test]
fn test_evm_eip7702_auth_nonce_mismatch_is_ignored() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let signer = new_signer();
    let authority = signer.address();
    let caller = address!("aaaaaaaa00000000000000000000000000000000");
    ctx.add_balance(caller, U256::from(10u128.pow(18)));

    let bad_nonce_auth = signed_auth(
        &signer,
        U256::from(ctx.cfg.chain_id),
        PRECOMPILE_SECP256K1_RECOVER,
        7,
    );

    let result = TxBuilder::call7702(&mut ctx, caller, Address::ZERO, vec![bad_nonce_auth], None)
        .gas_limit(200_000)
        .exec();
    assert!(result.is_success(), "tx itself should still succeed: {result:?}");

    assert_eq!(ctx.get_nonce(authority), 0);
    assert!(!matches!(ctx.get_code(authority), Some(Bytecode::Eip7702(_))));
}

#[test]
fn test_evm_eip7702_auth_chain_id_mismatch_is_ignored() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let signer = new_signer();
    let authority = signer.address();
    let caller = address!("aaaaaaaa00000000000000000000000000000000");
    ctx.add_balance(caller, U256::from(10u128.pow(18)));

    let wrong_chain_auth = signed_auth(
        &signer,
        U256::from(ctx.cfg.chain_id + 1),
        PRECOMPILE_SECP256K1_RECOVER,
        0,
    );

    let result = TxBuilder::call7702(&mut ctx, caller, Address::ZERO, vec![wrong_chain_auth], None)
        .gas_limit(200_000)
        .exec();
    assert!(result.is_success(), "tx itself should still succeed: {result:?}");

    assert_eq!(ctx.get_nonce(authority), 0);
    assert!(!matches!(ctx.get_code(authority), Some(Bytecode::Eip7702(_))));
}

#[test]
fn test_evm_eip7702_zero_address_clears_delegation() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let signer = new_signer();
    let authority = signer.address();
    let caller = address!("aaaaaaaa00000000000000000000000000000000");
    ctx.add_balance(caller, U256::from(10u128.pow(18)));

    // first auth: set delegation to precompile 0x01
    let set_auth = signed_auth(
        &signer,
        U256::from(ctx.cfg.chain_id),
        PRECOMPILE_SECP256K1_RECOVER,
        0,
    );
    let set_result = TxBuilder::call7702(&mut ctx, caller, Address::ZERO, vec![set_auth], None)
        .gas_limit(200_000)
        .exec();
    assert!(set_result.is_success(), "set delegation tx failed: {set_result:?}");

    match ctx.get_code(authority) {
        Some(Bytecode::Eip7702(code)) => assert_eq!(code.address(), PRECOMPILE_SECP256K1_RECOVER),
        other => panic!("expected Eip7702 code, got: {other:?}"),
    }
    assert_eq!(ctx.get_nonce(authority), 1);

    // second auth: clear delegation by authorizing address(0)
    let clear_auth = signed_auth(&signer, U256::from(ctx.cfg.chain_id), Address::ZERO, 1);
    let clear_result =
        TxBuilder::call7702(&mut ctx, caller, Address::ZERO, vec![clear_auth], None)
            .gas_limit(200_000)
            .exec();
    assert!(
        clear_result.is_success(),
        "clear delegation tx failed: {clear_result:?}"
    );

    let code = ctx.get_code(authority);
    assert!(
        matches!(code, None) || matches!(code, Some(c) if c.is_empty()),
        "expected empty/none code after clear, got: {code:?}"
    );
    assert_eq!(ctx.get_nonce(authority), 2);
}
