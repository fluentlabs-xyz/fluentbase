use alloc::vec::Vec;

use crate::EvmTestingContextWithGenesis;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::{
    crypto::crypto_keccak256, hex, universal_token::*, Address, Bytes, B256, U256,
};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use revm::context::result::ExecutionResult;

const DEPLOYER_ADDR: Address = Address::repeat_byte(1);
const RECIPIENT_ADDR: Address = Address::repeat_byte(3);

fn call_with_sig(
    ctx: &mut EvmTestingContext,
    input: Bytes,
    caller: &Address,
    callee: &Address,
) -> Vec<u8> {
    let result = ctx.call_evm_tx(*caller, *callee, input, None, None);
    assert!(result.is_success(), "call failed: {result:?}");
    result.output().unwrap().to_vec()
}

fn call_with_sig_revert(
    ctx: &mut EvmTestingContext,
    input: Bytes,
    caller: &Address,
    callee: &Address,
) -> Bytes {
    let result = ctx.call_evm_tx(*caller, *callee, input, None, None);
    match result {
        ExecutionResult::Revert { output, .. } => output,
        _ => panic!("expected revert, got: {result:?}"),
    }
}

pub const SECP256K1N: U256 = U256::from_limbs([
    0xbfd2_5e8c_d036_4141,
    0xbaae_dce6_af48_a03b,
    0xffff_ffff_ffff_fffe,
    0xffff_ffff_ffff_ffff,
]);

pub const SECP256K1N_HALF: U256 = U256::from_limbs([
    0xdfe9_2f46_681b_20a0,
    0x5d57_6e73_57a4_501d,
    0xffff_ffff_ffff_ffff,
    0x7fff_ffff_ffff_ffff,
]);

pub fn flip_recovery_id(v: u8) -> u8 {
    match v {
        27 => 28,
        28 => 27,
        0 => 1,
        1 => 0,
        _ => panic!("unexpected recovery id: {v}"),
    }
}

pub struct PermitDigestInput<'a> {
    pub token_name: &'a str,
    pub chain_id: u64,
    pub verifying_contract: Address,
    pub owner: Address,
    pub spender: Address,
    pub value: U256,
    pub nonce: U256,
    pub deadline: U256,
}

pub fn domain_separator(token_name: &str, chain_id: u64, verifying_contract: Address) -> B256 {
    let domain_typehash = crypto_keccak256(
        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let version_hash = crypto_keccak256("1");
    let name_hash = crypto_keccak256(token_name.as_bytes());

    let mut encoded_domain = Vec::with_capacity(32 * 5);
    encoded_domain.extend_from_slice(domain_typehash.as_slice());
    encoded_domain.extend_from_slice(name_hash.as_slice());
    encoded_domain.extend_from_slice(version_hash.as_slice());
    encoded_domain.extend_from_slice(&U256::from(chain_id).to_be_bytes::<{ U256::BYTES }>());
    encoded_domain.extend_from_slice(verifying_contract.into_word().as_slice());
    crypto_keccak256(encoded_domain)
}

pub fn permit_digest(input: PermitDigestInput<'_>) -> B256 {
    let permit_typehash = crypto_keccak256(
        "Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)",
    );
    let domain_separator =
        domain_separator(input.token_name, input.chain_id, input.verifying_contract);

    let mut encoded_permit = Vec::with_capacity(32 * 6);
    encoded_permit.extend_from_slice(permit_typehash.as_slice());
    encoded_permit.extend_from_slice(input.owner.into_word().as_slice());
    encoded_permit.extend_from_slice(input.spender.into_word().as_slice());
    encoded_permit.extend_from_slice(&input.value.to_be_bytes::<{ U256::BYTES }>());
    encoded_permit.extend_from_slice(&input.nonce.to_be_bytes::<{ U256::BYTES }>());
    encoded_permit.extend_from_slice(&input.deadline.to_be_bytes::<{ U256::BYTES }>());
    let permit_hash = crypto_keccak256(encoded_permit);

    let mut digest_payload = Vec::with_capacity(66);
    digest_payload.extend_from_slice(b"\x19\x01");
    digest_payload.extend_from_slice(domain_separator.as_slice());
    digest_payload.extend_from_slice(permit_hash.as_slice());
    crypto_keccak256(digest_payload)
}

#[test]
fn universal_token_permit_sets_allowance_and_nonce() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let signer: PrivateKeySigner =
        "0xf0bc949485d112791637d7eb29dea3fd1e0758e8fea3ef542a4245bc896736cc"
            .parse()
            .unwrap();
    let owner = Address::from_slice(signer.address().as_ref());
    let spender = RECIPIENT_ADDR;

    let initial_settings = InitialSettings {
        token_name: "Token".into(),
        token_symbol: "TKN".into(),
        decimals: 18,
        initial_supply: U256::ZERO,
        minter: Address::ZERO,
        pauser: Address::ZERO,
        wrapped: None,
    }
    .encode_with_prefix();

    let token = ctx.deploy_evm_tx(DEPLOYER_ADDR, initial_settings);

    sol! {
        function permit(address owner,address spender,uint256 value,uint256 deadline,uint8 v,bytes32 r,bytes32 s) external;
        function allowance(address owner,address spender) external view returns (uint256);
        function nonces(address owner) external view returns (uint256);
        function DOMAIN_SEPARATOR() external view returns (bytes32);
    }

    let domain_out = call_with_sig(
        &mut ctx,
        DOMAIN_SEPARATORCall {}.abi_encode().into(),
        &DEPLOYER_ADDR,
        &token,
    );
    let chain_id = ctx.cfg.chain_id;
    assert_eq!(
        B256::from_slice(domain_out.as_ref()),
        domain_separator("Token", chain_id, token)
    );

    let nonce = U256::ZERO;
    let value = U256::from(777u64);
    let deadline = U256::MAX;

    let digest = permit_digest(PermitDigestInput {
        token_name: "Token",
        chain_id,
        verifying_contract: token,
        owner,
        spender,
        value,
        nonce,
        deadline,
    });
    let sig = signer.sign_hash_sync(&digest).unwrap();
    let sig_bytes = sig.as_bytes();
    let r = B256::from_slice(&sig_bytes[0..32]);
    let s = B256::from_slice(&sig_bytes[32..64]);
    let v = sig_bytes[64];

    let input = permitCall {
        owner,
        spender,
        value,
        deadline,
        v,
        r,
        s,
    }
    .abi_encode();
    assert_eq!(input.len(), 228, "permit calldata must include selector");
    assert_eq!(
        u32::from_be_bytes(input[0..4].try_into().unwrap()),
        SIG_ERC20_PERMIT,
        "permit selector mismatch"
    );

    let result = ctx.call_evm_tx(DEPLOYER_ADDR, token, input.clone().into(), None, None);
    assert!(result.is_success(), "permit failed: {result:?}");

    let allowance_out = call_with_sig(
        &mut ctx,
        allowanceCall { owner, spender }.abi_encode().into(),
        &DEPLOYER_ADDR,
        &token,
    );
    assert_eq!(U256::from_be_slice(allowance_out.as_ref()), value);

    let nonces_out = call_with_sig(
        &mut ctx,
        noncesCall { owner }.abi_encode().into(),
        &DEPLOYER_ADDR,
        &token,
    );
    assert_eq!(U256::from_be_slice(nonces_out.as_ref()), U256::ONE);

    let replay_output = call_with_sig_revert(&mut ctx, input.into(), &DEPLOYER_ADDR, &token);
    assert!(
        replay_output
            .as_ref()
            .ends_with(&ERR_UST_INVALID_SIGNATURE.to_be_bytes()),
        "unexpected replay revert payload: 0x{}",
        hex::encode(replay_output.as_ref())
    );

    let allowance_out = call_with_sig(
        &mut ctx,
        allowanceCall { owner, spender }.abi_encode().into(),
        &DEPLOYER_ADDR,
        &token,
    );
    assert_eq!(U256::from_be_slice(allowance_out.as_ref()), value);

    let nonces_out = call_with_sig(
        &mut ctx,
        noncesCall { owner }.abi_encode().into(),
        &DEPLOYER_ADDR,
        &token,
    );
    assert_eq!(U256::from_be_slice(nonces_out.as_ref()), U256::ONE);
}

#[test]
fn universal_token_permit_rejects_expired_deadline_without_state_change() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let signer: PrivateKeySigner =
        "0xf0bc949485d112791637d7eb29dea3fd1e0758e8fea3ef542a4245bc896736cc"
            .parse()
            .unwrap();
    let owner = Address::from_slice(signer.address().as_ref());
    let spender = RECIPIENT_ADDR;

    let initial_settings = InitialSettings {
        token_name: "Token".into(),
        token_symbol: "TKN".into(),
        decimals: 18,
        initial_supply: U256::ZERO,
        minter: Address::ZERO,
        pauser: Address::ZERO,
        wrapped: None,
    }
    .encode_with_prefix();

    let token = ctx.deploy_evm_tx(DEPLOYER_ADDR, initial_settings);

    sol! {
        function permit(address owner,address spender,uint256 value,uint256 deadline,uint8 v,bytes32 r,bytes32 s) external;
        function allowance(address owner,address spender) external view returns (uint256);
        function nonces(address owner) external view returns (uint256);
    }

    let value = U256::from(123u64);
    let deadline = U256::ONE;
    let digest = permit_digest(PermitDigestInput {
        token_name: "Token",
        chain_id: ctx.cfg.chain_id,
        verifying_contract: token,
        owner,
        spender,
        value,
        nonce: U256::ZERO,
        deadline,
    });
    let sig = signer.sign_hash_sync(&digest).unwrap();
    let sig_bytes = sig.as_bytes();

    let input = permitCall {
        owner,
        spender,
        value,
        deadline,
        v: sig_bytes[64],
        r: B256::from_slice(&sig_bytes[0..32]),
        s: B256::from_slice(&sig_bytes[32..64]),
    }
    .abi_encode();

    let mut tx = TxBuilder::call(&mut ctx, DEPLOYER_ADDR, token, None)
        .input(input.into())
        .timestamp(2)
        .gas_price(0);
    let result = tx.exec();
    let ExecutionResult::Revert { output, .. } = result else {
        panic!("expected expired permit to revert, got: {result:?}");
    };
    assert!(
        output
            .as_ref()
            .ends_with(&ERR_UST_EXPIRED_DEADLINE.to_be_bytes()),
        "unexpected expired permit revert payload: 0x{}",
        hex::encode(output.as_ref())
    );

    let allowance_out = call_with_sig(
        &mut ctx,
        allowanceCall { owner, spender }.abi_encode().into(),
        &DEPLOYER_ADDR,
        &token,
    );
    assert_eq!(U256::from_be_slice(allowance_out.as_ref()), U256::ZERO);

    let nonce_out = call_with_sig(
        &mut ctx,
        noncesCall { owner }.abi_encode().into(),
        &DEPLOYER_ADDR,
        &token,
    );
    assert_eq!(U256::from_be_slice(nonce_out.as_ref()), U256::ZERO);
}

#[test]
fn universal_token_permit_rejects_invalid_signature() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let signer: PrivateKeySigner =
        "0xf0bc949485d112791637d7eb29dea3fd1e0758e8fea3ef542a4245bc896736cc"
            .parse()
            .unwrap();
    let owner = Address::from_slice(signer.address().as_ref());
    let spender = RECIPIENT_ADDR;

    let initial_settings = InitialSettings {
        token_name: "Token".into(),
        token_symbol: "TKN".into(),
        decimals: 18,
        initial_supply: U256::ZERO,
        minter: Address::ZERO,
        pauser: Address::ZERO,
        wrapped: None,
    }
    .encode_with_prefix();

    let token = ctx.deploy_evm_tx(DEPLOYER_ADDR, initial_settings);

    sol! {
        function permit(address owner,address spender,uint256 value,uint256 deadline,uint8 v,bytes32 r,bytes32 s) external;
    }

    let chain_id = ctx.cfg.chain_id;
    let value = U256::from(111u64);
    let deadline = U256::MAX;

    // Sign digest for a different spender
    let digest = permit_digest(PermitDigestInput {
        token_name: "Token",
        chain_id,
        verifying_contract: token,
        owner,
        spender: Address::with_last_byte(9),
        value,
        nonce: U256::ZERO,
        deadline,
    });
    let sig = signer.sign_hash_sync(&digest).unwrap();
    let sig_bytes = sig.as_bytes();
    let r = B256::from_slice(&sig_bytes[0..32]);
    let s = B256::from_slice(&sig_bytes[32..64]);
    let v = sig_bytes[64];

    let input = permitCall {
        owner,
        spender,
        value,
        deadline,
        v,
        r,
        s,
    }
    .abi_encode();

    let output = call_with_sig_revert(&mut ctx, input.into(), &DEPLOYER_ADDR, &token);
    assert!(
        output
            .as_ref()
            .ends_with(&ERR_UST_INVALID_SIGNATURE.to_be_bytes()),
        "unexpected revert payload: 0x{}",
        hex::encode(output.as_ref())
    );
}

#[test]
fn universal_token_permit_rejects_malleable_high_s_signature_without_state_change() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let signer: PrivateKeySigner =
        "0xf0bc949485d112791637d7eb29dea3fd1e0758e8fea3ef542a4245bc896736cc"
            .parse()
            .unwrap();
    let owner = Address::from_slice(signer.address().as_ref());
    let spender = RECIPIENT_ADDR;

    let initial_settings = InitialSettings {
        token_name: "Token".into(),
        token_symbol: "TKN".into(),
        decimals: 18,
        initial_supply: U256::ZERO,
        minter: Address::ZERO,
        pauser: Address::ZERO,
        wrapped: None,
    }
    .encode_with_prefix();

    let token = ctx.deploy_evm_tx(DEPLOYER_ADDR, initial_settings);

    sol! {
        function permit(address owner,address spender,uint256 value,uint256 deadline,uint8 v,bytes32 r,bytes32 s) external;
        function allowance(address owner,address spender) external view returns (uint256);
        function nonces(address owner) external view returns (uint256);
    }

    let value = U256::from(31337u64);
    let deadline = U256::MAX;
    let digest = permit_digest(PermitDigestInput {
        token_name: "Token",
        chain_id: ctx.cfg.chain_id,
        verifying_contract: token,
        owner,
        spender,
        value,
        nonce: U256::ZERO,
        deadline,
    });
    let sig = signer.sign_hash_sync(&digest).unwrap();
    let sig_bytes = sig.as_bytes();
    let r = B256::from_slice(&sig_bytes[0..32]);
    let low_s = U256::from_be_slice(&sig_bytes[32..64]);
    assert!(
        low_s <= SECP256K1N_HALF,
        "test signer must produce EIP-2 low-s signatures"
    );
    let high_s = SECP256K1N - low_s;
    assert!(
        high_s > SECP256K1N_HALF,
        "malleated signature must cross the low-s boundary"
    );
    let high_s = B256::from(high_s.to_be_bytes::<{ U256::BYTES }>());
    let malleated_v = flip_recovery_id(sig_bytes[64]);

    let input = permitCall {
        owner,
        spender,
        value,
        deadline,
        v: malleated_v,
        r,
        s: high_s,
    }
    .abi_encode();

    let output = call_with_sig_revert(&mut ctx, input.into(), &DEPLOYER_ADDR, &token);
    assert!(
        output
            .as_ref()
            .ends_with(&ERR_UST_INVALID_SIGNATURE.to_be_bytes()),
        "unexpected revert payload: 0x{}",
        hex::encode(output.as_ref())
    );

    let allowance_out = call_with_sig(
        &mut ctx,
        allowanceCall { owner, spender }.abi_encode().into(),
        &DEPLOYER_ADDR,
        &token,
    );
    assert_eq!(U256::from_be_slice(allowance_out.as_ref()), U256::ZERO);

    let nonce_out = call_with_sig(
        &mut ctx,
        noncesCall { owner }.abi_encode().into(),
        &DEPLOYER_ADDR,
        &token,
    );
    assert_eq!(U256::from_be_slice(nonce_out.as_ref()), U256::ZERO);
}
