use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::{
    hex,
    universal_token::{ApproveCommand, BalanceOfCommand, InitialSettings, UniversalTokenCommand},
    Address, Bytes, B256, GENESIS_GOVERNANCE, GENESIS_STAKING, U256,
};
use fluentbase_testing::EvmTestingContext;

const OWNER: Address = Address::repeat_byte(0x11);
const VALIDATOR: Address = Address::repeat_byte(0x22);
const TOKEN: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);

sol! {
    struct ConsensusKeys {
        bytes blsPubkey;
        bytes32 peerPubkey;
        uint64 activationEpoch;
    }

    interface IStakingRwasm {
        function initialize(
            address initialOwner,
            address[] validators,
            uint256[] initialStakes,
            uint16 commissionRate,
            address stakingToken,
            uint32 activeValidatorsLength,
            uint32 epochBlockInterval,
            uint32 felonyThreshold,
            uint32 validatorJailEpochLength,
            uint32 undelegatePeriod,
            uint256 minValidatorStakeAmount,
            uint256 minStakingAmount,
            uint64 dposActivationBlock,
            address blsVerifier,
            address evidenceDecoder,
            uint256 minUndelegateBlocks,
            address livenessSlashing,
            address blendReserve
        ) external;
        function delegate(address validator, uint256 amount) external;
        function undelegate(address validator, uint256 amount) external;
        function claimDelegatorFee(address validator) external;
        function setConsensusKeys(
            address validator,
            bytes calldata blsPubkeyUncompressed,
            bytes calldata blsPopUncompressed,
            bytes32 peerPubkey
        ) external;
        function getConsensusKeys(address validator)
            external
            view
            returns (ConsensusKeys memory keys);
        function getValidatorDelegation(address validator, address delegator)
            external
            view
            returns (uint256 delegatedAmount, uint64 atEpoch);
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

fn token_balance(context: &mut EvmTestingContext, token: Address, owner: Address) -> U256 {
    let mut input = Vec::new();
    BalanceOfCommand { owner }.encode_for_send(&mut input);
    let output = call(context, OWNER, token, input);
    U256::try_from_be_slice(&output).expect("ERC-20 balanceOf output")
}

fn initialize_calldata(
    staking_token: Address,
    bls_verifier: Address,
    initial_stakes: Vec<U256>,
) -> Vec<u8> {
    IStakingRwasm::initializeCall {
        initialOwner: OWNER,
        validators: vec![VALIDATOR],
        initialStakes: initial_stakes,
        commissionRate: 0,
        stakingToken: staking_token,
        activeValidatorsLength: 21,
        epochBlockInterval: 200,
        felonyThreshold: 150,
        validatorJailEpochLength: 7,
        undelegatePeriod: 7,
        minValidatorStakeAmount: TOKEN,
        minStakingAmount: TOKEN,
        dposActivationBlock: 1_000,
        blsVerifier: bls_verifier,
        evidenceDecoder: Address::ZERO,
        minUndelegateBlocks: U256::ZERO,
        livenessSlashing: Address::repeat_byte(0x55),
        blendReserve: Address::repeat_byte(0x66),
    }
    .abi_encode()
}

#[test]
fn staking_accepts_solidity_bytes_for_consensus_keys() {
    let mut context = EvmTestingContext::default().with_full_genesis();
    // Init bytecode for a Solidity mock that returns bytes(96) from
    // compressG2Unchecked(bytes) and true from verify(bytes,bytes,bytes,bytes,bytes).
    let verifier = context.deploy_evm_tx(
        OWNER,
        Bytes::from_static(&hex!(
            "6080604052348015600f57600080fd5b506102cd8061001f6000396000f3fe60806040523480156100
             1057600080fd5b50600436106100365760003560e01c80638bf261331461003b578063a5d2dd221461
             006e575b600080fd5b6100596100493660046100fc565b60019a9950505050505050505050565b6040
             5190151581526020015b60405180910390f35b61008161007c366004610207565b61008e565b604051
             6100659190610249565b60408051606080825260808201909252816020820181803683370190505093
             92505050565b60008083601f8401126100c557600080fd5b50813567ffffffffffffffff8111156100
             dd57600080fd5b6020830191508360208285010111156100f557600080fd5b9250929050565b600080
             60008060008060008060008060a08b8d03121561011b57600080fd5b8a3567ffffffffffffffff8111
             1561013257600080fd5b61013e8d828e016100b3565b909b5099505060208b013567ffffffffffffff
             ff81111561015e57600080fd5b61016a8d828e016100b3565b90995097505060408b013567ffffffff
             ffffffff81111561018a57600080fd5b6101968d828e016100b3565b90975095505060608b013567ff
             ffffffffffffff8111156101b657600080fd5b6101c28d828e016100b3565b90955093505060808b01
             3567ffffffffffffffff8111156101e257600080fd5b6101ee8d828e016100b3565b91508093505080
             9150509295989b9194979a5092959850565b6000806020838503121561021a57600080fd5b823567ff
             ffffffffffffff81111561023157600080fd5b61023d858286016100b3565b90969095509350505050
             565b602081526000825180602084015260005b81811015610277576020818601810151604086840101
             520161025a565b506000604082850101526040601f19601f8301168401019150509291505056fea264
             6970667358221220f07b58ae9a9816fe76d1d4ededd0059334efa5a878d8eadfb08a543038e7910364
             736f6c63430008220033"
        )),
    );

    // Keep delegation explicitly pre-activation: current epoch is clamped to
    // zero, so the two-epoch warm-up records the delegation at epoch 2.
    context = context.with_block_number(999);
    call(
        &mut context,
        GENESIS_GOVERNANCE,
        GENESIS_STAKING,
        initialize_calldata(Address::repeat_byte(0x44), verifier, vec![U256::ZERO]),
    );
    call(
        &mut context,
        VALIDATOR,
        GENESIS_STAKING,
        IStakingRwasm::setConsensusKeysCall {
            validator: VALIDATOR,
            blsPubkeyUncompressed: vec![0x11; 256].into(),
            blsPopUncompressed: vec![0x22; 128].into(),
            peerPubkey: B256::with_last_byte(0x01),
        }
        .abi_encode(),
    );
    let output = call(
        &mut context,
        VALIDATOR,
        GENESIS_STAKING,
        IStakingRwasm::getConsensusKeysCall {
            validator: VALIDATOR,
        }
        .abi_encode(),
    );
    let result = IStakingRwasm::getConsensusKeysCall::abi_decode_returns(&output).unwrap();
    assert_eq!(result.blsPubkey.as_ref(), &[0; 96]);
    assert_eq!(result.peerPubkey, B256::with_last_byte(0x01));
    assert_eq!(result.activationEpoch, 0);
}

#[test]
fn genesis_staking_custodies_and_returns_blend_through_real_rwasm_calls() {
    let mut context = EvmTestingContext::default().with_full_genesis();
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
        initialize_calldata(token, Address::ZERO, vec![TOKEN]),
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
        initialize_calldata(token, Address::ZERO, vec![TOKEN]),
    );
    assert_reverts(
        &mut context,
        GENESIS_GOVERNANCE,
        GENESIS_STAKING,
        initialize_calldata(token, Address::ZERO, vec![TOKEN]),
    );
    call(
        &mut context,
        OWNER,
        GENESIS_STAKING,
        IStakingRwasm::delegateCall {
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
        IStakingRwasm::undelegateCall {
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
        IStakingRwasm::claimDelegatorFeeCall {
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
