use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::{
    address, hex,
    universal_token::{
        ApproveCommand, BalanceOfCommand, InitialSettings, TransferCommand, UniversalTokenCommand,
    },
    Address, Bytes, B256, GENESIS_GOVERNANCE, GENESIS_STAKING, U256,
};
// Everything with a second caller outside `contracts/staking` — `initialize`,
// `registerValidator`, `delegate`, `undelegate`, `recordProduction`,
// `blocksInEpoch`, and `getConsensusKeys` with the `ConsensusKeys` struct it
// returns — comes from the crate the contract derives its dispatch selectors
// from, so an ABI change to any of them is a compile error here rather than a
// revert against the real blob.
use fluentbase_staking_abi as staking_abi;
use fluentbase_testing::EvmTestingContext;

const OWNER: Address = Address::repeat_byte(0x11);
const VALIDATOR: Address = Address::repeat_byte(0x22);
const TOKEN: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);
const SYSTEM_CALLER: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");

// The three handlers this file is the ONLY place outside `contracts/staking` to
// encode. Under the rule in `fluentbase-staking-abi`'s header they stay here:
// one declaration on each side of one pairing is not a duplicate anyone can
// drift, and the contract derives their selectors with `derive_keccak256_id!`.
// Everything else this file calls comes from the shared crate.
sol! {
    interface IStakingRwasm {
        function withdrawDelegatorPrincipal(address validator) external;
        function getValidatorDelegation(address validator, address delegator)
            external
            view
            returns (uint256 delegatedAmount, uint64 atEpoch);
        function lastProcessedBlock() external view returns (uint64);
    }
}

fn call(
    context: &mut EvmTestingContext,
    caller: Address,
    callee: Address,
    input: Vec<u8>,
) -> Vec<u8> {
    let result = context.call_evm_tx(caller, callee, input.into(), Some(20_000_000), None);
    assert!(result.is_success(), "call failed: {result:?}");
    result.output().cloned().unwrap_or_default().to_vec()
}

fn assert_reverts(
    context: &mut EvmTestingContext,
    caller: Address,
    callee: Address,
    input: Vec<u8>,
) {
    let result = context.call_evm_tx(caller, callee, input.into(), Some(20_000_000), None);
    assert!(
        !result.is_success(),
        "call unexpectedly succeeded: {result:?}"
    );
}

/// Like [`assert_reverts`], but pins WHICH revert. A registration has a dozen
/// ways to fail and most of them are cheaper to hit by accident than the one a
/// test means to reach.
fn assert_reverts_with(
    context: &mut EvmTestingContext,
    caller: Address,
    callee: Address,
    input: Vec<u8>,
    selector: [u8; 4],
) {
    let result = context.call_evm_tx(caller, callee, input.into(), Some(20_000_000), None);
    assert!(
        !result.is_success(),
        "call unexpectedly succeeded: {result:?}"
    );
    let output = result.output().cloned().unwrap_or_default();
    assert_eq!(
        &output[..4],
        &selector,
        "reverted, but not for the reason under test: {output:?}"
    );
}

fn token_balance(context: &mut EvmTestingContext, token: Address, owner: Address) -> U256 {
    let mut input = Vec::new();
    BalanceOfCommand { owner }.encode_for_send(&mut input);
    let output = call(context, OWNER, token, input);
    U256::try_from_be_slice(&output).expect("ERC-20 balanceOf output")
}

fn initialize_calldata(staking_token: Address, initial_stakes: Vec<U256>) -> Vec<u8> {
    let has_initial_validator = !initial_stakes.is_empty();
    let validators = if !has_initial_validator {
        Vec::new()
    } else {
        vec![VALIDATOR]
    };
    staking_abi::initializeCall {
        initialStakeOwner: OWNER,
        validators,
        initialStakes: initial_stakes,
        blsPubkeysUncompressed: if !has_initial_validator {
            Vec::new()
        } else {
            vec![Bytes::copy_from_slice(crate::bls_vectors::pubkey(0))]
        },
        blsPopsUncompressed: if !has_initial_validator {
            Vec::new()
        } else {
            vec![Bytes::copy_from_slice(crate::bls_vectors::pop(0))]
        },
        peerPubkeys: if !has_initial_validator {
            Vec::new()
        } else {
            vec![B256::with_last_byte(1)]
        },
        commissionRate: 0,
        stakingToken: staking_token,
        activeValidatorsLength: 21,
        epochBlockInterval: 200,
        undelegatePeriod: 7,
        minValidatorStakeAmount: TOKEN,
        minStakingAmount: TOKEN,
        dposActivationBlock: 1_000,
        minUndelegateBlocks: U256::ZERO,
        blendReserve: Address::repeat_byte(0x66),
    }
    .abi_encode()
}

/// A registration whose proof of possession is REAL, verified by the contract
/// against the real EIP-2537 precompiles.
///
/// This is the honest end of the PoP coverage. The unit tests in
/// `contracts/staking/src/tests.rs` drive a policy stub — they can reach the
/// branches, but they decide nothing about BLS12-381. Here the signature was
/// made by `blst` inside the node, the pairing that accepts it is arkworks
/// inside the predeploy, and the hash-to-curve between them is the contract's
/// own. The last assertion closes the loop from the other side: the 96-byte
/// identity the contract stored is the one `blst` compressed, which is what
/// pins the EIP-2537-to-zcash half-swap and the y-sign rule.
#[test]
fn staking_accepts_a_real_proof_of_possession_and_stores_the_key_blst_compressed() {
    let mut context = EvmTestingContext::default().with_full_genesis();
    context.cfg.chain_id = crate::bls_vectors::CHAIN_ID;
    let token = context.deploy_evm_tx(
        OWNER,
        InitialSettings {
            token_name: "Blend".into(),
            token_symbol: "BLEND".into(),
            decimals: 18,
            initial_supply: TOKEN * U256::from(1_000),
            minter: OWNER,
            pauser: Address::ZERO,
            wrapped: None,
        }
        .encode_with_prefix(),
    );

    // Keep key activation explicitly pre-activation: the current epoch is
    // clamped to zero.
    context = context.with_block_number(999);
    call(
        &mut context,
        GENESIS_GOVERNANCE,
        GENESIS_STAKING,
        initialize_calldata(token, Vec::new()),
    );
    let mut approve = Vec::new();
    ApproveCommand {
        spender: GENESIS_STAKING,
        amount: TOKEN,
    }
    .encode_for_send(&mut approve);
    call(&mut context, OWNER, token, approve);
    call(
        &mut context,
        OWNER,
        GENESIS_STAKING,
        staking_abi::registerValidatorCall {
            validator: VALIDATOR,
            commissionRate: 0,
            initialStake: TOKEN,
            blsPubkeyUncompressed: crate::bls_vectors::pubkey(0).to_vec().into(),
            blsPopUncompressed: crate::bls_vectors::pop(0).to_vec().into(),
            peerPubkey: B256::with_last_byte(0x01),
        }
        .abi_encode(),
    );
    let output = call(
        &mut context,
        VALIDATOR,
        GENESIS_STAKING,
        staking_abi::getConsensusKeysCall {
            validator: VALIDATOR,
        }
        .abi_encode(),
    );
    let result = staking_abi::getConsensusKeysCall::abi_decode_returns(&output).unwrap();
    assert_eq!(
        result.blsPubkey.as_ref(),
        crate::bls_vectors::pubkey_compressed(0)
    );
    assert_eq!(result.peerPubkey, B256::with_last_byte(0x01));
    assert_eq!(result.activationEpoch, 1);

    // One byte of the proof flipped, and the same registration is refused —
    // named, not merely refused: `InvalidProofOfPossession` is the gate under
    // test and a registration has cheaper ways to fail.
    //
    // A second owner, because one account may own only one validator.
    let second_owner = Address::repeat_byte(0x55);
    let second = Address::repeat_byte(0x77);
    let mut transfer = Vec::new();
    TransferCommand {
        to: second_owner,
        amount: TOKEN * U256::from(2),
    }
    .encode_for_send(&mut transfer);
    call(&mut context, OWNER, token, transfer);
    let mut approve = Vec::new();
    ApproveCommand {
        spender: GENESIS_STAKING,
        amount: TOKEN * U256::from(2),
    }
    .encode_for_send(&mut approve);
    call(&mut context, second_owner, token, approve);
    context.add_balance(second_owner, U256::from(10u128).pow(U256::from(20)));

    let registration = |pop: Vec<u8>| {
        staking_abi::registerValidatorCall {
            validator: second,
            commissionRate: 0,
            initialStake: TOKEN,
            blsPubkeyUncompressed: crate::bls_vectors::pubkey(1).to_vec().into(),
            blsPopUncompressed: pop.into(),
            peerPubkey: B256::with_last_byte(0x02),
        }
        .abi_encode()
    };
    // Two forgeries, because they fail at DIFFERENT places and only one of them
    // reaches the pairing's verdict.
    //
    // A flipped bit puts the signature off the curve, and the EIP-2537 pairing
    // precompile REFUSES such input rather than answering "does not verify" — so
    // this one is caught by the precompile-status check.
    let mangled = {
        let mut bytes = crate::bls_vectors::pop(1).to_vec();
        *bytes.last_mut().unwrap() ^= 1;
        bytes
    };
    // Another validator's proof of possession is a perfectly well-formed point
    // in the right subgroup; it just proves possession of a different key. This
    // one reaches the pairing and is rejected by its verdict — the branch that
    // decides whether a validator may register a public key it does not hold the
    // secret for.
    let wrong_key = crate::bls_vectors::pop(2).to_vec();
    for forged in [mangled, wrong_key] {
        assert_reverts_with(
            &mut context,
            second_owner,
            GENESIS_STAKING,
            registration(forged),
            // InvalidProofOfPossession(address)
            hex!("1a4e671e"),
        );
    }
    // The untouched proof for the same key goes through, so what the line above
    // caught is the flipped byte and not some other difference between the calls.
    call(
        &mut context,
        second_owner,
        GENESIS_STAKING,
        registration(crate::bls_vectors::pop(1).to_vec()),
    );
}

#[test]
fn genesis_staking_custodies_and_returns_blend_through_real_rwasm_calls() {
    let mut context = EvmTestingContext::default().with_full_genesis();
    context.cfg.chain_id = crate::bls_vectors::CHAIN_ID;
    let initial_supply = TOKEN * U256::from(1_000);
    let token = context.deploy_evm_tx(
        OWNER,
        InitialSettings {
            token_name: "Blend".into(),
            token_symbol: "BLEND".into(),
            decimals: 18,
            initial_supply,
            minter: OWNER,
            pauser: Address::ZERO,
            wrapped: None,
        }
        .encode_with_prefix(),
    );

    assert_reverts(
        &mut context,
        OWNER,
        GENESIS_STAKING,
        initialize_calldata(token, vec![TOKEN]),
    );

    let mut approve = Vec::new();
    ApproveCommand {
        spender: GENESIS_STAKING,
        amount: TOKEN * U256::from(4),
    }
    .encode_for_send(&mut approve);
    call(&mut context, OWNER, token, approve);
    call(
        &mut context,
        GENESIS_GOVERNANCE,
        GENESIS_STAKING,
        initialize_calldata(token, vec![TOKEN]),
    );
    assert_reverts(
        &mut context,
        GENESIS_GOVERNANCE,
        GENESIS_STAKING,
        initialize_calldata(token, vec![TOKEN]),
    );
    call(
        &mut context,
        OWNER,
        GENESIS_STAKING,
        staking_abi::delegateCall {
            validator: VALIDATOR,
            amount: TOKEN * U256::from(3),
        }
        .abi_encode(),
    );
    assert_eq!(
        token_balance(&mut context, token, GENESIS_STAKING),
        TOKEN * U256::from(4)
    );

    let output = call(
        &mut context,
        OWNER,
        GENESIS_STAKING,
        IStakingRwasm::getValidatorDelegationCall {
            validator: VALIDATOR,
            delegator: OWNER,
        }
        .abi_encode(),
    );
    let delegation =
        IStakingRwasm::getValidatorDelegationCall::abi_decode_returns(&output).unwrap();
    assert_eq!(delegation.delegatedAmount, TOKEN * U256::from(3));
    assert_eq!(delegation.atEpoch, 2);

    // The epoch-2 delegation is no longer ahead of nextEpoch at block 1_200.
    context = context.with_block_number(1_200);
    call(
        &mut context,
        OWNER,
        GENESIS_STAKING,
        staking_abi::undelegateCall {
            validator: VALIDATOR,
            amount: TOKEN,
        }
        .abi_encode(),
    );
    context = context.with_block_number(3_000);
    call(
        &mut context,
        OWNER,
        GENESIS_STAKING,
        IStakingRwasm::withdrawDelegatorPrincipalCall {
            validator: VALIDATOR,
        }
        .abi_encode(),
    );

    assert_eq!(
        token_balance(&mut context, token, GENESIS_STAKING),
        TOKEN * U256::from(3)
    );
    assert_eq!(
        token_balance(&mut context, token, OWNER),
        initial_supply - TOKEN * U256::from(3)
    );
}

// The independent ABI oracle for the recorder: alloy builds the calldata from
// its own signatures, so a selector that drifted from the Solidity name would
// land on `UnknownMethod()` here rather than on a passing unit test.
#[test]
fn record_production_drives_the_epoch_close_through_real_rwasm() {
    let mut context = EvmTestingContext::default().with_full_genesis();
    context.cfg.chain_id = crate::bls_vectors::CHAIN_ID;
    context = context.with_block_number(999);
    call(
        &mut context,
        GENESIS_GOVERNANCE,
        GENESIS_STAKING,
        initialize_calldata(Address::repeat_byte(0x44), Vec::new()),
    );

    // The height is the block context now, so the closure keeps its parameter
    // only to name which block each assertion is about.
    let record =
        |_block_number: u64| staking_abi::recordProductionCall { leaderIndex: 0 }.abi_encode();
    assert_reverts(&mut context, OWNER, GENESIS_STAKING, record(1_000));

    context = context.with_block_number(1_000);
    call(&mut context, SYSTEM_CALLER, GENESIS_STAKING, record(1_000));
    call(&mut context, SYSTEM_CALLER, GENESIS_STAKING, record(1_000));
    // Crossing into epoch 1 runs the close, which must not fail the block.
    context = context.with_block_number(1_200);
    call(&mut context, SYSTEM_CALLER, GENESIS_STAKING, record(1_200));

    let output = call(
        &mut context,
        OWNER,
        GENESIS_STAKING,
        IStakingRwasm::lastProcessedBlockCall {}.abi_encode(),
    );
    assert_eq!(
        IStakingRwasm::lastProcessedBlockCall::abi_decode_returns(&output).unwrap(),
        1_200
    );
    let output = call(
        &mut context,
        OWNER,
        GENESIS_STAKING,
        staking_abi::blocksInEpochCall { epoch: 0 }.abi_encode(),
    );
    assert_eq!(
        staking_abi::blocksInEpochCall::abi_decode_returns(&output).unwrap(),
        0,
        "no committee is committed, so every block parks"
    );
}
