use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::{
    universal_token::{ApproveCommand, BalanceOfCommand, InitialSettings, UniversalTokenCommand},
    Address, GENESIS_GOVERNANCE, GENESIS_STAKING, U256,
};
use fluentbase_testing::EvmTestingContext;

const OWNER: Address = Address::repeat_byte(0x11);
const VALIDATOR: Address = Address::repeat_byte(0x22);
const TOKEN: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);

sol! {
    interface IStakingRwasm {
        function initialize(
            address initialOwner,
            address[] validators,
            uint256[] initialStakes,
            uint16 commissionRate
        ) external;
        function configure(
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
            uint256 minUndelegateBlocks
        ) external;
        function delegate(address validator, uint256 amount) external;
        function undelegate(address validator, uint256 amount) external;
        function claimDelegatorFee(address validator) external;
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

    call(
        &mut context,
        GENESIS_GOVERNANCE,
        GENESIS_STAKING,
        IStakingRwasm::configureCall {
            stakingToken: token,
            activeValidatorsLength: 21,
            epochBlockInterval: 200,
            felonyThreshold: 150,
            validatorJailEpochLength: 7,
            undelegatePeriod: 7,
            minValidatorStakeAmount: TOKEN,
            minStakingAmount: TOKEN,
            dposActivationBlock: 1_000,
            blsVerifier: Address::ZERO,
            evidenceDecoder: Address::ZERO,
            minUndelegateBlocks: U256::ZERO,
        }
        .abi_encode(),
    );
    assert_reverts(
        &mut context,
        GENESIS_GOVERNANCE,
        GENESIS_STAKING,
        IStakingRwasm::configureCall {
            stakingToken: token,
            activeValidatorsLength: 21,
            epochBlockInterval: 200,
            felonyThreshold: 150,
            validatorJailEpochLength: 7,
            undelegatePeriod: 7,
            minValidatorStakeAmount: TOKEN,
            minStakingAmount: TOKEN,
            dposActivationBlock: 1_000,
            blsVerifier: Address::ZERO,
            evidenceDecoder: Address::ZERO,
            minUndelegateBlocks: U256::ZERO,
        }
        .abi_encode(),
    );
    assert_reverts(
        &mut context,
        OWNER,
        GENESIS_STAKING,
        IStakingRwasm::initializeCall {
            initialOwner: OWNER,
            validators: vec![VALIDATOR],
            initialStakes: vec![TOKEN],
            commissionRate: 0,
        }
        .abi_encode(),
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
        IStakingRwasm::initializeCall {
            initialOwner: OWNER,
            validators: vec![VALIDATOR],
            initialStakes: vec![TOKEN],
            commissionRate: 0,
        }
        .abi_encode(),
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
    // Block 1_400 is epoch 2: (1_400 - activation 1_000) / interval 200.
    assert_eq!(delegation.atEpoch, 2);

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
