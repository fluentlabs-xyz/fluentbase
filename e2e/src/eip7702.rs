use crate::EvmTestingContextWithGenesis;
use alloy_consensus::{SignableTransaction, TxEip7702, TxEnvelope};
use alloy_network::eip2718::Encodable2718;
use alloy_network::TxSignerSync;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::SolCall;
use fluentbase_sdk::{address, keccak256, Address, Bytes, U256};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use hex_literal::hex;
use revm::{
    context::transaction::Authorization,
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

/// Builds a raw EIP-7702 tx that delegates EOA code to a deployed Ping contract,
/// then calls ping(x) on the EOA itself.
///
/// Steps before running:
///   1. Create a new wallet:        cast wallet new
///   2. Fund it on testnet
///   3. Deploy Ping:                forge create e2e/assets/Ping.sol:Ping --rpc-url https://rpc.testnet.fluent.xyz --private-key <KEY>
///   4. Get EOA nonce:              cast nonce <EOA_ADDR> --rpc-url https://rpc.testnet.fluent.xyz
///   5. Fill TODOs below, run test, copy printed hex
///   6. Send:  cast rpc --rpc-url https://rpc.testnet.fluent.xyz eth_sendRawTransaction '<0x...>'
#[test]
fn build_raw_eip7702_ping_tx() {
    // TODO: paste your new wallet private key
    // 0xAFeC91d439750c5866998ad261E3c1665C584e68
    let signer: PrivateKeySigner =
        "0x76db4f24b30ccb392feaa35c628de261096cf077d18f44d81e03b0cc75099f22"
            .parse()
            .unwrap();

    // TODO: paste deployed Ping contract address
    let ping_contract = address!("0x66b4c3654193f6fd1B9331c1169C72b33EE1b4a8");

    // TODO: set to current EOA nonce (from cast nonce above)
    let current_nonce: u64 = 0;

    // EIP-7702 authorization: delegate this EOA's code to Ping contract
    let authorization = Authorization {
        chain_id: U256::from(20994),
        address: ping_contract,
        nonce: current_nonce + 1, // nonce of the EOA being authorized
    };

    let auth_sig = signer
        .sign_hash_sync(&authorization.signature_hash())
        .unwrap();
    let signed_auth = authorization.into_signed(auth_sig);

    // Encode ping(uint256) calldata with x = 42
    // ❯ cast sig "ping(uint256)"
    // 0x773acdef
    let selector = &keccak256(b"ping(uint256)")[..4];
    let mut calldata = selector.to_vec();
    let x = U256::from(42u64);
    calldata.extend_from_slice(&x.to_be_bytes::<32>());

    // `to` is the EOA itself — EIP-7702 makes it execute Ping's code in EOA context
    let eoa_address = signer.address();

    let mut tx = TxEip7702 {
        chain_id: 20994,
        nonce: current_nonce,
        gas_limit: 200_000,
        max_fee_per_gas: 5_000_000_007,
        max_priority_fee_per_gas: 1_000_000_000,
        to: eoa_address.into(),
        value: U256::ZERO,
        input: Bytes::from(calldata),
        access_list: Default::default(),
        authorization_list: vec![signed_auth],
    };

    let sig = signer.sign_transaction_sync(&mut tx).unwrap();
    let signed = tx.into_signed(sig);
    let envelope = TxEnvelope::Eip7702(signed);
    let raw = envelope.encoded_2718();

    let hex_tx = format!("0x{}", hex::encode(&raw));

    println!("EOA address : {eoa_address}");
    println!("Ping contract: {ping_contract}");
    println!("Nonce used   : {current_nonce}");
    println!("\n--- raw tx ---\n{hex_tx}\n");
    println!(
        "cast rpc --rpc-url https://rpc.testnet.fluent.xyz eth_sendRawTransaction '{hex_tx}'"
    );
}