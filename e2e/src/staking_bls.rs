//! The two staking paths that verify BLS signatures, driven end to end with
//! signatures the NODE produced.
//!
//! The staking module verifies BLS itself now (`contracts/staking/src/bls.rs`),
//! so both paths run the contract's own hash-to-curve and the arkworks EIP-2537
//! predeploys against blst-made signatures. The evidence path has no other
//! coverage at this fidelity: the contract's unit tests drive a policy stub, and
//! a stub cannot tell a real signature from a plausible one.
//!
//! It also prints the frame gas of both paths (`--nocapture`), which is what the
//! before/after comparison in `devnet/local-dpos-smoke/contracts/STAKING_ARTEFACT.md`
//! was taken with.

use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::{hex, Address, Bytes, B256, GENESIS_GOVERNANCE, GENESIS_STAKING, U256};
use fluentbase_testing::EvmTestingContext;

const OWNER: Address = Address::repeat_byte(0x11);
const RELAYER: Address = Address::repeat_byte(0xd7);
const TOKEN: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);
const HUGE_GAS: u64 = 30_000_000;

sol! {
    interface IStaking {
        function initialize(
            address initialStakeOwner,
            address[] validators,
            uint256[] initialStakes,
            bytes[] blsPubkeysUncompressed,
            bytes[] blsPopsUncompressed,
            bytes32[] peerPubkeys,
            uint16 commissionRate,
            address stakingToken,
            uint32 activeValidatorsLength,
            uint32 epochBlockInterval,
            uint32 undelegatePeriod,
            uint256 minValidatorStakeAmount,
            uint256 minStakingAmount,
            uint64 dposActivationBlock,
            uint256 minUndelegateBlocks,
            address blendReserve
        ) external;
        function registerValidator(
            address validator,
            uint16 commissionRate,
            uint256 initialStake,
            bytes calldata blsPubkeyUncompressed,
            bytes calldata blsPopUncompressed,
            bytes32 peerPubkey
        ) external;
        function slashEquivocationNotarize(
            bytes evidence,
            bytes pkUncompressed,
            bytes sig1Uncompressed,
            bytes sig2Uncompressed
        ) external;
        function getConsensusKeys(address validator)
            external view returns (bytes blsPubkey, bytes32 peerPubkey, uint64 activationEpoch);
    }
}

/// A token that answers `true` to everything and `2^80-1` to the two reads the
/// epoch close makes. Gas here, not solvency.
fn deploy_token(context: &mut EvmTestingContext) -> Address {
    let runtime = hex!(
        "60003560e01c"
        "80" "6370a08231" "14" "6020" "57"
        "80" "63dd62ed3e" "14" "6020" "57"
        "600160005260206000f3"
        "5b" "69ffffffffffffffffffff" "60005260206000f3"
    );
    let mut init = vec![
        0x60,
        runtime.len() as u8,
        0x80,
        0x60,
        0x0b,
        0x60,
        0x00,
        0x39,
        0x60,
        0x00,
        0xf3,
    ];
    init.extend_from_slice(&runtime);
    context.deploy_evm_tx(OWNER, Bytes::from(init))
}

fn intrinsic(calldata: &[u8]) -> u64 {
    let mut cost = 21_000u64;
    for byte in calldata {
        cost += if *byte == 0 { 4 } else { 16 };
    }
    cost
}

/// Execution gas of the frame, with the EVM transaction intrinsic taken off. A
/// real system call pays no intrinsic, so this is the comparable figure.
fn frame_gas(
    context: &mut EvmTestingContext,
    caller: Address,
    input: Vec<u8>,
    want_ok: bool,
) -> u64 {
    let overhead = intrinsic(&input);
    let result = context.call_evm_tx(caller, GENESIS_STAKING, input.into(), Some(HUGE_GAS), None);
    if result.is_success() != want_ok {
        panic!("unexpected outcome: {result:?}");
    }
    if !want_ok {
        println!("revert output: {:?}", result.output());
    }
    result.gas().total_gas_spent().saturating_sub(overhead)
}

/// The node-side conformance corpus blob, copied from
/// `contracts/staking/src/evidence.rs::tests::CONFLICTING_NOTARIZE`.
const CONFLICTING_NOTARIZE: [u8; 168] = hex!(
    "072a29aa000000000000000000000000000000000000000000000000000000000000aa
     038aa1d24f195fc333878b14744f62a363acf0051249c949c4cc473850991aa708
     41eea2171a333b13de2e61fed4936305
     072a29bb000000000000000000000000000000000000000000000000000000000000bb
     03923c9abd2f0abe63eed5a2d9ac175032b2b48685c61f9e6a7c8b7419d7807782
     1d82a3bfd41a5f10bcfcd8434444f820"
);

/// Real MinSig signatures by validator 0 over the two 35-byte proposals inside
/// the blob, under `FLUENT_DPOS_V1_ ‖ chain_id ‖ _NOTARIZE` and the message DST.
/// Produced by `blst` in the node, same generator as `bls_vectors`.
const SIG1_COMPRESSED: [u8; 48] =
    hex!("b03bfdcb55b779f6f6d1f30bf16d3131e23ea87db3a1d74c6eec39908f539b20fc9531a13ca6444db26dcc870408bbb6");
const SIG1_UNCOMPRESSED: [u8; 128] = hex!(
    "00000000000000000000000000000000103bfdcb55b779f6f6d1f30bf16d3131e23ea87db3a1d74c6eec39908f539b20fc9531a13ca6444db26dcc870408bbb6"
    "000000000000000000000000000000000d38a4f6f9579d3f4a577b7f99661f19a0ae32f791c721e29827c135c1fa9746ef69517c12a18f0b6c19aa1b46727716"
);
const SIG2_COMPRESSED: [u8; 48] =
    hex!("a2712a7610ddae5313e076eb35e2a42a74525e94e275341be94d7a30db58abfb63fa8fce9773da394faacb8f8fdb29d3");
const SIG2_UNCOMPRESSED: [u8; 128] = hex!(
    "0000000000000000000000000000000002712a7610ddae5313e076eb35e2a42a74525e94e275341be94d7a30db58abfb63fa8fce9773da394faacb8f8fdb29d3"
    "00000000000000000000000000000000131707c48a58f1fa171446b243201b7797066eb9d13131c2d948673152f4ace45876827351ad64296e7f1c9d7d77f579"
);

/// The corpus blob with validator 0's real signatures spliced into the two
/// attestation slots, so the slash SUCCEEDS and the measured frame is the whole
/// path rather than a prefix of it.
fn signed_evidence() -> Bytes {
    let mut blob = CONFLICTING_NOTARIZE;
    blob[36..84].copy_from_slice(&SIG1_COMPRESSED);
    blob[120..168].copy_from_slice(&SIG2_COMPRESSED);
    Bytes::from(blob.to_vec())
}

/// Both BLS paths accept what the node signs, and the evidence path tombstones.
#[test]
fn both_bls_paths_accept_node_made_signatures() {
    let mut context = EvmTestingContext::default().with_full_genesis();
    context.cfg.chain_id = crate::bls_vectors::CHAIN_ID;
    let token = deploy_token(&mut context);
    context.add_balance(OWNER, U256::from(10u128).pow(U256::from(20)));
    context.add_balance(RELAYER, U256::from(10u128).pow(U256::from(20)));

    // One genesis validator, so the slash below finds a registered key owner.
    let init = IStaking::initializeCall {
        initialStakeOwner: OWNER,
        validators: vec![Address::repeat_byte(0xa1)],
        initialStakes: vec![TOKEN * U256::from(10)],
        blsPubkeysUncompressed: vec![crate::bls_vectors::pubkey(0).to_vec().into()],
        blsPopsUncompressed: vec![crate::bls_vectors::pop(0).to_vec().into()],
        peerPubkeys: vec![B256::with_last_byte(1)],
        commissionRate: 0,
        stakingToken: token,
        activeValidatorsLength: 51,
        epochBlockInterval: 200,
        undelegatePeriod: 7,
        minValidatorStakeAmount: TOKEN,
        minStakingAmount: TOKEN,
        dposActivationBlock: 200_000,
        minUndelegateBlocks: U256::ZERO,
        blendReserve: OWNER,
    }
    .abi_encode();
    frame_gas(&mut context, GENESIS_GOVERNANCE, init, true);

    // registerValidator: one compressG2 + one verify, plus the registration
    // itself. Measured with a second owner because one account owns one
    // validator.
    let second_owner = Address::repeat_byte(0x55);
    context.add_balance(second_owner, U256::from(10u128).pow(U256::from(20)));
    let register = IStaking::registerValidatorCall {
        validator: Address::repeat_byte(0xa2),
        commissionRate: 0,
        initialStake: TOKEN,
        blsPubkeyUncompressed: crate::bls_vectors::pubkey(1).to_vec().into(),
        blsPopUncompressed: crate::bls_vectors::pop(1).to_vec().into(),
        peerPubkey: B256::with_last_byte(2),
    }
    .abi_encode();
    let register_gas = frame_gas(&mut context, second_owner, register, true);

    // The evidence path: one compressG2, two compressG1, two verifies, then the
    // tombstone and the seizure. Succeeds, because a reverted frame in this
    // harness reports the whole gas limit as spent and would measure nothing.
    let slash = IStaking::slashEquivocationNotarizeCall {
        evidence: signed_evidence(),
        pkUncompressed: crate::bls_vectors::pubkey(0).to_vec().into(),
        sig1Uncompressed: SIG1_UNCOMPRESSED.to_vec().into(),
        sig2Uncompressed: SIG2_UNCOMPRESSED.to_vec().into(),
    }
    .abi_encode();
    let slash_gas = frame_gas(&mut context, RELAYER, slash, true);

    // The slash landed: the same evidence a second time is refused as a re-slash
    // rather than accepted again, which is the state change a successful
    // tombstone leaves behind.
    let repeat = IStaking::slashEquivocationNotarizeCall {
        evidence: signed_evidence(),
        pkUncompressed: crate::bls_vectors::pubkey(0).to_vec().into(),
        sig1Uncompressed: SIG1_UNCOMPRESSED.to_vec().into(),
        sig2Uncompressed: SIG2_UNCOMPRESSED.to_vec().into(),
    }
    .abi_encode();
    let result = context.call_evm_tx(
        RELAYER,
        GENESIS_STAKING,
        repeat.into(),
        Some(HUGE_GAS),
        None,
    );
    assert!(!result.is_success());
    let output = result.output().cloned().unwrap_or_default();
    // AlreadySlashedForEquivocation(address)
    assert_eq!(&output[..4], &hex!("8300031d"));

    println!("registerValidator frame gas: {register_gas}");
    println!("slashEquivocationNotarize frame gas: {slash_gas}");
}
