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
    bytecode::opcode,
    context::{result::ExecutionResult::Revert, transaction::Authorization},
    Database,
};

#[test]
fn test_evm_eip7702_call_delegated_account() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let signer: PrivateKeySigner =
        "0xf0bc949485d112791637d7eb29dea3fd1e0758e8fea3ef542a4245bc896736cc"
            .parse()
            .unwrap();

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

    let authorization = Authorization {
        chain_id: U256::from(ctx.cfg.chain_id),
        address: delegate_to,
        nonce: auth_nonce,
    };
    let auth_sig = signer
        .sign_hash_sync(&authorization.signature_hash())
        .unwrap();
    let signed_auth = authorization.into_signed(auth_sig);

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
