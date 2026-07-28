use super::*;
use crate::storage::{staking_storage, STATUS_ACTIVE, STATUS_JAIL, STATUS_PENDING};
use crate::types::{
    AddressCommand, BoolCommand, ConfigureCommand, ConfigureDependenciesCommand,
    EpochSignerCommand, TwoAddressesCommand, U256Command, U32Command, U64Command,
    ValidatorBlockCommand, ValidatorDelegatorCommand, ValidatorEpochCommand,
};
use fluentbase_sdk::{
    bytes::BytesMut, codec::SolidityABI, derive::derive_keccak256_id, is_engine_metered_precompile,
    is_execute_using_system_runtime, Address, Bytes, ContractContextV1, ExitCode, B256,
    GENESIS_GOVERNANCE, GENESIS_STAKING, U256,
};
use fluentbase_testing::TestingContextImpl;

fn encode_call<T>(selector: u32, value: &T) -> Vec<u8>
where
    T: fluentbase_sdk::codec::Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    let mut params = BytesMut::new();
    SolidityABI::<T>::encode(value, &mut params, 0).unwrap();
    let mut input = selector.to_be_bytes().to_vec();
    input.extend_from_slice(&params);
    input
}

fn encode_args_call<T>(selector: u32, value: &T) -> Vec<u8>
where
    T: fluentbase_sdk::codec::FunctionArgs<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    let mut params = BytesMut::new();
    SolidityABI::<T>::encode_function_args(value, &mut params).unwrap();
    let mut input = selector.to_be_bytes().to_vec();
    input.extend_from_slice(&params);
    input
}

fn encode_empty_call(selector: u32) -> Vec<u8> {
    selector.to_be_bytes().to_vec()
}

struct Harness {
    sdk: TestingContextImpl,
}

impl Harness {
    fn new(block_number: u64) -> Self {
        let gas_limit = 1_000_000;
        let sdk = TestingContextImpl::default()
            .with_contract_context(ContractContextV1 {
                address: GENESIS_STAKING,
                bytecode_address: GENESIS_STAKING,
                gas_limit,
                ..Default::default()
            })
            .with_block_number(block_number)
            .with_gas_limit(gas_limit);
        Self { sdk }
    }

    fn set_caller(&self, caller: Address) {
        self.sdk.context_mut().caller = caller;
    }

    fn set_block_number(&mut self, block_number: u64) {
        self.sdk = core::mem::take(&mut self.sdk).with_block_number(block_number);
    }

    fn call<I: Into<Bytes>>(&mut self, input: I) -> (ExitCode, Vec<u8>) {
        self.sdk = core::mem::take(&mut self.sdk).with_input(input);
        let storage_before = self.sdk.dump_storage();
        let exit = match main_entry(&mut self.sdk) {
            Ok(()) => ExitCode::Ok,
            Err(exit) => exit,
        };
        if !exit.is_ok() {
            self.sdk.restore_storage(storage_before);
        }
        (exit, self.sdk.take_output())
    }

    fn initialize(
        &mut self,
        owner: Address,
        validators: Vec<Address>,
        stakes: Vec<U256>,
        commission_rate: u16,
    ) -> ExitCode {
        self.call(encode_args_call(
            SIG_INITIALIZE,
            &(owner, validators, stakes, commission_rate),
        ))
        .0
    }
}

fn decode_output<T>(output: &[u8]) -> T
where
    T: fluentbase_sdk::codec::Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    SolidityABI::<T>::decode(&output, 0).unwrap()
}

fn decode_returns<T>(output: &[u8]) -> T
where
    T: fluentbase_sdk::codec::FunctionArgs<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    SolidityABI::<T>::decode_function_args(&output).unwrap()
}

fn assert_revert_selector(result: (ExitCode, Vec<u8>), selector: u32) {
    assert_eq!(result.0, ExitCode::Panic);
    assert_eq!(&result.1[..4], &selector.to_be_bytes());
}

#[test]
fn parameterized_custom_errors_use_solidity_abi() {
    let owner = Address::with_last_byte(0xa0);
    let outsider = Address::with_last_byte(0xb0);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    harness.set_caller(outsider);
    let (_, output) = harness.call(encode_call(
        SIG_CONFIGURE,
        &ConfigureCommand {
            staking_token: Address::with_last_byte(0xc0),
            active_validators_length: 21,
            epoch_block_interval: 200,
            felony_threshold: 150,
            validator_jail_epoch_length: 7,
            undelegate_period: 7,
            min_validator_stake_amount: DEFAULT_MIN_VALIDATOR_STAKE,
            min_staking_amount: DEFAULT_MIN_STAKING_AMOUNT,
            dpos_activation_block: 1_000,
            bls_verifier: Address::ZERO,
            evidence_decoder: Address::ZERO,
            min_undelegate_blocks: U256::ZERO,
        },
    ));
    assert_eq!(&output[..4], &ERR_ONLY_OWNER.to_be_bytes());
    assert_eq!(decode_output::<Address>(&output[4..]), outsider);

    harness.set_caller(GENESIS_GOVERNANCE);
    let (_, output) = harness.call(encode_call(
        SIG_SET_BLS_VERIFIER,
        &AddressCommand {
            value: Address::ZERO,
        },
    ));
    assert_eq!(&output[..4], &ERR_ZERO_VALUE.to_be_bytes());
    assert_eq!(
        decode_output::<alloc::string::String>(&output[4..]),
        "blsVerifier"
    );
}

#[test]
fn implemented_selectors_match_pinned_solidity_abi() {
    assert_eq!(
        SIG_INITIALIZE,
        derive_keccak256_id!("initialize(address,address[],uint256[],uint16)")
    );
    assert_eq!(SIG_CURRENT_EPOCH, derive_keccak256_id!("currentEpoch()"));
    assert_eq!(SIG_NEXT_EPOCH, derive_keccak256_id!("nextEpoch()"));
    assert_eq!(SIG_OWNER, derive_keccak256_id!("owner()"));
    assert_eq!(SIG_GET_STAKING, derive_keccak256_id!("getStaking()"));
    assert_eq!(SIG_GET_GOVERNANCE, derive_keccak256_id!("getGovernance()"));
    assert_eq!(
        SIG_GET_CHAIN_CONFIG,
        derive_keccak256_id!("getChainConfig()")
    );
    assert_eq!(
        SIG_GET_STAKING_TOKEN,
        derive_keccak256_id!("getStakingToken()")
    );
    assert_eq!(
        SIG_CONFIGURE,
        derive_keccak256_id!(
            "configure(address,uint32,uint32,uint32,uint32,uint32,uint256,uint256,uint64,address,address,uint256)"
        )
    );
    assert_eq!(
        SIG_DEFAULT_PARTICIPATION_FLOOR_BPS,
        derive_keccak256_id!("DEFAULT_PARTICIPATION_FLOOR_BPS()")
    );
    assert_eq!(
        SIG_DEFAULT_SLASH_REPORTER_BPS,
        derive_keccak256_id!("DEFAULT_SLASH_REPORTER_BPS()")
    );
    assert_eq!(
        SIG_MAX_ACTIVE_VALIDATORS,
        derive_keccak256_id!("MAX_ACTIVE_VALIDATORS()")
    );
    assert_eq!(
        SIG_MAX_BLEND_STIPEND_PER_EPOCH,
        derive_keccak256_id!("MAX_BLEND_STIPEND_PER_EPOCH()")
    );
    assert_eq!(
        SIG_MAX_PARTICIPATION_FLOOR_BPS,
        derive_keccak256_id!("MAX_PARTICIPATION_FLOOR_BPS()")
    );
    assert_eq!(
        SIG_MAX_SLASH_REPORTER_BPS,
        derive_keccak256_id!("MAX_SLASH_REPORTER_BPS()")
    );
    assert_eq!(
        SIG_GET_VALIDATOR_DELEGATION,
        derive_keccak256_id!("getValidatorDelegation(address,address)")
    );
    assert_eq!(
        SIG_GET_VALIDATOR_DELEGATED_STAKE_AT,
        derive_keccak256_id!("getValidatorDelegatedStakeAt(address,uint256)")
    );
    assert_eq!(
        SIG_REGISTER_VALIDATOR,
        derive_keccak256_id!("registerValidator(address,uint16,uint256)")
    );
    assert_eq!(
        SIG_DELEGATE,
        derive_keccak256_id!("delegate(address,uint256)")
    );
    assert_eq!(
        SIG_UNDELEGATE,
        derive_keccak256_id!("undelegate(address,uint256)")
    );
    assert_eq!(
        SIG_IS_VALIDATOR,
        derive_keccak256_id!("isValidator(address)")
    );
    assert_eq!(
        SIG_IS_VALIDATOR_ACTIVE,
        derive_keccak256_id!("isValidatorActive(address)")
    );
    assert_eq!(
        SIG_GET_VALIDATOR_STATUS,
        derive_keccak256_id!("getValidatorStatus(address)")
    );
    assert_eq!(
        SIG_GET_VALIDATOR_BY_OWNER,
        derive_keccak256_id!("getValidatorByOwner(address)")
    );
    assert_eq!(SIG_GET_VALIDATORS, derive_keccak256_id!("getValidators()"));
    assert_eq!(
        SIG_ADD_VALIDATOR,
        derive_keccak256_id!("addValidator(address)")
    );
    assert_eq!(
        SIG_REMOVE_VALIDATOR,
        derive_keccak256_id!("removeValidator(address)")
    );
    assert_eq!(
        SIG_ACTIVATE_VALIDATOR,
        derive_keccak256_id!("activateValidator(address)")
    );
    assert_eq!(
        SIG_DISABLE_VALIDATOR,
        derive_keccak256_id!("disableValidator(address)")
    );
    assert_eq!(
        SIG_CHANGE_VALIDATOR_COMMISSION_RATE,
        derive_keccak256_id!("changeValidatorCommissionRate(address,uint16)")
    );
    assert_eq!(
        SIG_CHANGE_VALIDATOR_OWNER,
        derive_keccak256_id!("changeValidatorOwner(address,address)")
    );
    assert_eq!(
        SIG_CONFIGURE_DEPENDENCIES,
        derive_keccak256_id!("configureDependencies(address,address)")
    );
    assert_eq!(
        SIG_SET_ACTIVE_VALIDATORS_LENGTH,
        derive_keccak256_id!("setActiveValidatorsLength(uint32)")
    );
    assert_eq!(
        SIG_SET_EPOCH_BLOCK_INTERVAL,
        derive_keccak256_id!("setEpochBlockInterval(uint32)")
    );
    assert_eq!(
        SIG_SET_DPOS_ACTIVATION_BLOCK,
        derive_keccak256_id!("setDposActivationBlock(uint64)")
    );
    assert_eq!(
        SIG_SET_FELONY_THRESHOLD,
        derive_keccak256_id!("setFelonyThreshold(uint32)")
    );
    assert_eq!(
        SIG_SET_VALIDATOR_JAIL_EPOCH_LENGTH,
        derive_keccak256_id!("setValidatorJailEpochLength(uint32)")
    );
    assert_eq!(
        SIG_SET_SLASH_REPORTER_REWARD_BPS,
        derive_keccak256_id!("setSlashReporterRewardBps(uint32)")
    );
    assert_eq!(
        SIG_SET_SLASH_FUND_ADDRESS,
        derive_keccak256_id!("setSlashFundAddress(address)")
    );
    assert_eq!(
        SIG_SET_PARTICIPATION_FLOOR_BPS,
        derive_keccak256_id!("setParticipationFloorBps(uint32)")
    );
    assert_eq!(
        SIG_SET_PARTICIPATION_JAIL_DISABLED,
        derive_keccak256_id!("setParticipationJailDisabled(bool)")
    );
    assert_eq!(
        SIG_SET_BLEND_STIPEND_PER_EPOCH,
        derive_keccak256_id!("setBlendStipendPerEpoch(uint256)")
    );
    assert_eq!(
        SIG_SET_UNDELEGATE_PERIOD,
        derive_keccak256_id!("setUndelegatePeriod(uint32)")
    );
    assert_eq!(
        SIG_SET_MIN_VALIDATOR_STAKE_AMOUNT,
        derive_keccak256_id!("setMinValidatorStakeAmount(uint256)")
    );
    assert_eq!(
        SIG_SET_MIN_STAKING_AMOUNT,
        derive_keccak256_id!("setMinStakingAmount(uint256)")
    );
    assert_eq!(
        SIG_SET_BLS_VERIFIER,
        derive_keccak256_id!("setBlsVerifier(address)")
    );
    assert_eq!(
        SIG_SET_EVIDENCE_DECODER,
        derive_keccak256_id!("setEvidenceDecoder(address)")
    );
    assert_eq!(
        SIG_GET_VALIDATOR_FEE,
        derive_keccak256_id!("getValidatorFee(address)")
    );
    assert_eq!(
        SIG_GET_PENDING_VALIDATOR_FEE,
        derive_keccak256_id!("getPendingValidatorFee(address)")
    );
    assert_eq!(
        SIG_CLAIM_VALIDATOR_FEE_AT_EPOCH,
        derive_keccak256_id!("claimValidatorFeeAtEpoch(address,uint64)")
    );
    assert_eq!(
        SIG_GET_DELEGATOR_FEE,
        derive_keccak256_id!("getDelegatorFee(address,address)")
    );
    assert_eq!(
        SIG_CLAIM_DELEGATOR_FEE_AT_EPOCH,
        derive_keccak256_id!("claimDelegatorFeeAtEpoch(address,uint64)")
    );
    assert_eq!(
        SIG_CALC_AVAILABLE_FOR_REDELEGATE_AMOUNT,
        derive_keccak256_id!("calcAvailableForRedelegateAmount(address,address)")
    );
    assert_eq!(
        SIG_SETTLE_EPOCH_STIPEND,
        derive_keccak256_id!("settleEpochStipend(uint64)")
    );
    assert_eq!(
        SIG_SET_CONSENSUS_KEYS,
        derive_keccak256_id!("setConsensusKeys(address,bytes,bytes,bytes32)")
    );
    assert_eq!(
        SIG_GET_VALIDATORS_WITH_KEYS_AT,
        derive_keccak256_id!("getValidatorsWithKeysAt(uint64)")
    );
    assert_eq!(
        SIG_COMMIT_EPOCH_COMMITTEE,
        derive_keccak256_id!("commitEpochCommittee(address[])")
    );
    assert_eq!(
        SIG_GET_EPOCH_COMMITTEE_WITH_STAKES,
        derive_keccak256_id!("getEpochCommitteeWithStakes(uint64)")
    );
    assert_eq!(
        SIG_RELEASE_VALIDATOR_FROM_JAIL,
        derive_keccak256_id!("releaseValidatorFromJail(address)")
    );
    assert_eq!(SIG_SLASH, derive_keccak256_id!("slash(address)"));
    assert_eq!(
        SIG_SLASH_EQUIVOCATION_NOTARIZE,
        derive_keccak256_id!("slashEquivocationNotarize(bytes,bytes,bytes,bytes)")
    );
}

#[test]
fn initializes_registry_and_preserves_solidity_read_abi() {
    let governance = Address::with_last_byte(0xa0);
    let validator_a = Address::with_last_byte(0x01);
    let validator_b = Address::with_last_byte(0x02);
    let mut harness = Harness::new(1_000);
    harness.set_caller(governance);

    assert_eq!(
        harness.initialize(
            governance,
            vec![validator_a, validator_b],
            vec![
                U256::from(10) * BALANCE_COMPACT_PRECISION,
                U256::from(20) * BALANCE_COMPACT_PRECISION,
            ],
            500,
        ),
        ExitCode::Ok
    );

    let (exit, output) = harness.call(encode_empty_call(SIG_OWNER));
    assert_eq!(exit, ExitCode::Ok);
    assert_eq!(decode_output::<Address>(&output), governance);

    let (_, output) = harness.call(encode_empty_call(SIG_GET_VALIDATORS));
    assert_eq!(
        decode_output::<Vec<Address>>(&output),
        vec![validator_b, validator_a],
        "top validators are ordered by stake descending"
    );

    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATOR_STATUS,
        &AddressCommand { value: validator_a },
    ));
    let status: (Address, u8, U256, u32, u64, u64, u64, u16) = decode_output(&output);
    assert_eq!(
        status,
        (
            validator_a,
            STATUS_ACTIVE,
            U256::from(10) * BALANCE_COMPACT_PRECISION,
            0,
            0,
            0,
            0,
            500,
        )
    );
}

#[test]
fn epoch_number_is_rebased_to_initialization_block() {
    let governance = Address::with_last_byte(0xa0);
    let mut harness = Harness::new(1_000);
    harness.set_caller(governance);
    assert_eq!(
        harness.initialize(governance, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    harness.set_block_number(1_399);
    let (_, output) = harness.call(encode_empty_call(SIG_CURRENT_EPOCH));
    assert_eq!(decode_output::<u64>(&output), 1);

    harness.set_block_number(1_400);
    let (_, output) = harness.call(encode_empty_call(SIG_NEXT_EPOCH));
    assert_eq!(decode_output::<u64>(&output), 3);
}

#[test]
fn governance_lifecycle_updates_active_registry() {
    let owner = Address::with_last_byte(0xa0);
    let outsider = Address::with_last_byte(0xb0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    harness.set_caller(outsider);
    let (exit, _) = harness.call(encode_call(
        SIG_ADD_VALIDATOR,
        &AddressCommand { value: validator },
    ));
    assert_eq!(exit, ExitCode::Panic);

    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_ADD_VALIDATOR,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );
    let (_, output) = harness.call(encode_call(
        SIG_IS_VALIDATOR_ACTIVE,
        &AddressCommand { value: validator },
    ));
    assert!(decode_output::<bool>(&output));

    assert_eq!(
        harness
            .call(encode_call(
                SIG_DISABLE_VALIDATOR,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );
    let (_, output) = harness.call(encode_call(
        SIG_IS_VALIDATOR_ACTIVE,
        &AddressCommand { value: validator },
    ));
    assert!(!decode_output::<bool>(&output));

    assert_eq!(
        harness
            .call(encode_call(
                SIG_ACTIVATE_VALIDATOR,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );
}

#[test]
fn staking_is_a_genesis_rwasm_contract_not_a_system_precompile() {
    assert!(!is_execute_using_system_runtime(&GENESIS_STAKING));
    assert!(!is_engine_metered_precompile(&GENESIS_STAKING));
}

#[test]
fn embedded_chain_config_exposes_solidity_public_constants() {
    let mut harness = Harness::new(0);
    for (selector, expected) in [
        (
            SIG_DEFAULT_PARTICIPATION_FLOOR_BPS,
            DEFAULT_PARTICIPATION_FLOOR_BPS,
        ),
        (
            SIG_DEFAULT_SLASH_REPORTER_BPS,
            DEFAULT_SLASH_REPORTER_REWARD_BPS,
        ),
        (
            SIG_MAX_ACTIVE_VALIDATORS,
            MAX_ACTIVE_VALIDATORS_LENGTH as u32,
        ),
        (SIG_MAX_PARTICIPATION_FLOOR_BPS, MAX_PARTICIPATION_FLOOR_BPS),
        (SIG_MAX_SLASH_REPORTER_BPS, MAX_SLASH_REPORTER_REWARD_BPS),
    ] {
        let (_, output) = harness.call(encode_empty_call(selector));
        assert_eq!(decode_output::<u32>(&output), expected);
    }
    let (_, output) = harness.call(encode_empty_call(SIG_MAX_BLEND_STIPEND_PER_EPOCH));
    assert_eq!(decode_output::<U256>(&output), MAX_BLEND_STIPEND_PER_EPOCH);
}

#[test]
fn stores_reserved_governance_and_chain_configuration() {
    let owner = Address::with_last_byte(0xa0);
    let staking_token = Address::with_last_byte(0xb1);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);

    let mut configure = ConfigureCommand {
        staking_token,
        active_validators_length: 50,
        epoch_block_interval: 100,
        felony_threshold: 150,
        validator_jail_epoch_length: 7,
        undelegate_period: 7,
        min_validator_stake_amount: BALANCE_COMPACT_PRECISION,
        min_staking_amount: BALANCE_COMPACT_PRECISION,
        dpos_activation_block: 1_000,
        bls_verifier: Address::with_last_byte(0xb2),
        evidence_decoder: Address::with_last_byte(0xb3),
        min_undelegate_blocks: U256::from(701),
    };
    assert_revert_selector(
        harness.call(encode_call(SIG_CONFIGURE, &configure)),
        ERR_UNDELEGATE_WINDOW_TOO_SHORT,
    );
    configure.min_undelegate_blocks = U256::from(700);
    assert_eq!(
        harness.call(encode_call(SIG_CONFIGURE, &configure)).0,
        ExitCode::Ok
    );
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    assert_revert_selector(
        harness.call(encode_call(SIG_CONFIGURE, &configure)),
        ERR_ALREADY_CONFIGURED,
    );

    let (_, output) = harness.call(encode_empty_call(SIG_GET_GOVERNANCE));
    assert_eq!(decode_output::<Address>(&output), GENESIS_GOVERNANCE);

    let (_, output) = harness.call(encode_empty_call(SIG_GET_STAKING));
    assert_eq!(decode_output::<Address>(&output), GENESIS_STAKING);
    let (_, output) = harness.call(encode_empty_call(SIG_GET_CHAIN_CONFIG));
    assert_eq!(decode_output::<Address>(&output), GENESIS_STAKING);

    let config = staking_storage().config_accessor();
    assert_eq!(
        config
            .active_validators_length_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        50
    );
    assert_eq!(
        config
            .epoch_block_interval_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        100
    );
    assert_eq!(
        config
            .min_undelegate_blocks_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        U256::from(700)
    );
    let (_, output) = harness.call(encode_empty_call(SIG_GET_STAKING_TOKEN));
    assert_eq!(decode_output::<Address>(&output), staking_token);

    harness.set_block_number(1_100);
    let (_, output) = harness.call(encode_empty_call(SIG_CURRENT_EPOCH));
    assert_eq!(decode_output::<u64>(&output), 1);
}

#[test]
fn governance_updates_embedded_chain_configuration() {
    let owner = Address::with_last_byte(0xa0);
    let outsider = Address::with_last_byte(0xb0);
    let slash_fund = Address::with_last_byte(0xc1);
    let bls_verifier = Address::with_last_byte(0xc2);
    let evidence_decoder = Address::with_last_byte(0xc3);
    let liveness_slashing = Address::with_last_byte(0xc4);
    let blend_reserve = Address::with_last_byte(0xc5);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    assert_eq!(
        harness
            .call(encode_call(
                SIG_CONFIGURE_DEPENDENCIES,
                &ConfigureDependenciesCommand {
                    liveness_slashing,
                    blend_reserve,
                },
            ))
            .0,
        ExitCode::Ok
    );

    harness.set_caller(outsider);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_FELONY_THRESHOLD,
                &U32Command { value: 3 },
            ))
            .0,
        ExitCode::Panic
    );

    harness.set_caller(GENESIS_GOVERNANCE);
    for (selector, value) in [
        (SIG_SET_FELONY_THRESHOLD, 3),
        (SIG_SET_VALIDATOR_JAIL_EPOCH_LENGTH, 4),
        (SIG_SET_SLASH_REPORTER_REWARD_BPS, 2_500),
        (SIG_SET_PARTICIPATION_FLOOR_BPS, 1_000),
        (SIG_SET_ACTIVE_VALIDATORS_LENGTH, 31),
        (SIG_SET_UNDELEGATE_PERIOD, 9),
    ] {
        assert_eq!(
            harness.call(encode_call(selector, &U32Command { value })).0,
            ExitCode::Ok
        );
    }
    for (selector, value) in [
        (SIG_SET_SLASH_FUND_ADDRESS, slash_fund),
        (SIG_SET_BLS_VERIFIER, bls_verifier),
        (SIG_SET_EVIDENCE_DECODER, evidence_decoder),
    ] {
        assert_eq!(
            harness
                .call(encode_call(selector, &AddressCommand { value }))
                .0,
            ExitCode::Ok
        );
    }
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_PARTICIPATION_JAIL_DISABLED,
                &BoolCommand { value: true },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_BLEND_STIPEND_PER_EPOCH,
                &U256Command {
                    value: U256::from(42),
                },
            ))
            .0,
        ExitCode::Ok
    );

    for (selector, expected) in [
        (SIG_GET_FELONY_THRESHOLD, 3),
        (SIG_GET_VALIDATOR_JAIL_EPOCH_LENGTH, 4),
        (SIG_GET_SLASH_REPORTER_REWARD_BPS, 2_500),
        (SIG_GET_PARTICIPATION_FLOOR_BPS, 1_000),
        (SIG_GET_ACTIVE_VALIDATORS_LENGTH, 31),
        (SIG_GET_UNDELEGATE_PERIOD, 9),
    ] {
        let (exit, output) = harness.call(encode_empty_call(selector));
        assert_eq!(exit, ExitCode::Ok);
        assert_eq!(decode_output::<u32>(&output), expected);
    }
    for (selector, expected) in [
        (SIG_GET_SLASH_FUND_ADDRESS, slash_fund),
        (SIG_GET_BLS_VERIFIER, bls_verifier),
        (SIG_GET_EVIDENCE_DECODER, evidence_decoder),
    ] {
        let (exit, output) = harness.call(encode_empty_call(selector));
        assert_eq!(exit, ExitCode::Ok);
        assert_eq!(decode_output::<Address>(&output), expected);
    }
    let (_, output) = harness.call(encode_empty_call(SIG_GET_PARTICIPATION_JAIL_DISABLED));
    assert!(decode_output::<bool>(&output));
    let (_, output) = harness.call(encode_empty_call(SIG_GET_BLEND_STIPEND_PER_EPOCH));
    assert_eq!(decode_output::<U256>(&output), U256::from(42));
}

#[test]
fn initializer_rejects_mismatched_arrays_without_persisting_owner() {
    let governance = Address::with_last_byte(0xa0);
    let mut harness = Harness::new(1_000);
    harness.set_caller(governance);

    assert_eq!(
        harness.initialize(governance, vec![Address::with_last_byte(1)], Vec::new(), 0,),
        ExitCode::Panic
    );

    let (_, output) = harness.call(encode_empty_call(SIG_OWNER));
    assert_eq!(decode_output::<Address>(&output), Address::ZERO);
}

#[test]
fn delegation_and_undelegation_follow_epoch_snapshots() {
    let owner = Address::with_last_byte(0xa0);
    let delegator = Address::with_last_byte(0xb0);
    let validator = Address::with_last_byte(0x01);
    let token = Address::with_last_byte(0xc0);
    let one_token = U256::from(1_000_000_000_000_000_000u64);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(
            owner,
            vec![validator],
            vec![one_token * U256::from(10)],
            500,
        ),
        ExitCode::Ok
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CONFIGURE,
                &ConfigureCommand {
                    staking_token: token,
                    active_validators_length: 21,
                    epoch_block_interval: 200,
                    felony_threshold: 150,
                    validator_jail_epoch_length: 7,
                    undelegate_period: 7,
                    min_validator_stake_amount: one_token,
                    min_staking_amount: one_token,
                    dpos_activation_block: 1_000,
                    bls_verifier: Address::ZERO,
                    evidence_decoder: Address::ZERO,
                    min_undelegate_blocks: U256::ZERO,
                },
            ))
            .0,
        ExitCode::Ok
    );

    economics::delegate_to(
        &mut harness.sdk,
        delegator,
        validator,
        one_token * U256::from(2),
        false,
    )
    .unwrap();
    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATOR_DELEGATION,
        &ValidatorDelegatorCommand {
            validator,
            delegator,
        },
    ));
    assert_eq!(
        decode_output::<(U256, u64)>(&output),
        (one_token * U256::from(2), 2)
    );

    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATOR_DELEGATED_STAKE_AT,
        &ValidatorBlockCommand {
            validator,
            block_number: U256::from(1_399),
        },
    ));
    assert_eq!(decode_output::<U256>(&output), one_token * U256::from(10));
    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATOR_DELEGATED_STAKE_AT,
        &ValidatorBlockCommand {
            validator,
            block_number: U256::from(1_400),
        },
    ));
    assert_eq!(decode_output::<U256>(&output), one_token * U256::from(12));

    harness.set_block_number(1_400);
    economics::undelegate_from(&mut harness.sdk, delegator, validator, one_token).unwrap();
    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATOR_DELEGATION,
        &ValidatorDelegatorCommand {
            validator,
            delegator,
        },
    ));
    assert_eq!(decode_output::<(U256, u64)>(&output), (one_token, 3));

    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATOR_DELEGATED_STAKE_AT,
        &ValidatorBlockCommand {
            validator,
            block_number: U256::from(1_600),
        },
    ));
    assert_eq!(decode_output::<U256>(&output), one_token * U256::from(11));
}

#[test]
fn erc20_transfer_from_calldata_is_standard_abi() {
    let from = Address::with_last_byte(0x11);
    let to = Address::with_last_byte(0x22);
    let amount = U256::from(123);
    let input = util::erc20_transfer_from_input(from, to, amount).unwrap();
    assert_eq!(&input[..4], &SIG_ERC20_TRANSFER_FROM.to_be_bytes());
    assert_eq!(
        SolidityABI::<(Address, Address, U256)>::decode(&&input[4..], 0).unwrap(),
        (from, to, amount)
    );
}

#[test]
fn reward_views_split_blend_between_owner_and_delegators() {
    let owner = Address::with_last_byte(0xa0);
    let delegator = Address::with_last_byte(0xb0);
    let validator = Address::with_last_byte(0x01);
    let ten_tokens = DEFAULT_MIN_STAKING_AMOUNT * U256::from(10);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![ten_tokens], 1_000),
        ExitCode::Ok
    );
    economics::delegate_to(&mut harness.sdk, delegator, validator, ten_tokens, false).unwrap();
    let snapshot = staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(2);
    snapshot
        .total_blend_rewards_accessor()
        .set_checked(&mut harness.sdk, ten_tokens)
        .unwrap();

    harness.set_block_number(1_600);
    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATOR_FEE,
        &AddressCommand { value: validator },
    ));
    assert_eq!(decode_output::<U256>(&output), DEFAULT_MIN_STAKING_AMOUNT);

    let (_, output) = harness.call(encode_call(
        SIG_GET_DELEGATOR_FEE,
        &ValidatorDelegatorCommand {
            validator,
            delegator,
        },
    ));
    assert_eq!(
        decode_output::<U256>(&output),
        U256::from(9) * DEFAULT_MIN_STAKING_AMOUNT / U256::from(2)
    );

    let (_, output) = harness.call(encode_call(
        SIG_CALC_AVAILABLE_FOR_REDELEGATE_AMOUNT,
        &ValidatorDelegatorCommand {
            validator,
            delegator,
        },
    ));
    assert_eq!(
        decode_output::<(U256, U256)>(&output),
        (
            U256::from(9) * DEFAULT_MIN_STAKING_AMOUNT / U256::from(2),
            U256::ZERO
        )
    );
}

#[test]
fn committee_commit_is_system_gated_and_returns_epoch_stakes() {
    let owner = Address::with_last_byte(0xa0);
    let validator_a = Address::with_last_byte(0x01);
    let validator_b = Address::with_last_byte(0x02);
    let stake_a = U256::from(10) * BALANCE_COMPACT_PRECISION;
    let stake_b = U256::from(20) * BALANCE_COMPACT_PRECISION;
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(
            owner,
            vec![validator_a, validator_b],
            vec![stake_a, stake_b],
            500,
        ),
        ExitCode::Ok
    );

    for (validator, last_byte) in [(validator_a, 1u8), (validator_b, 2u8)] {
        let keys = staking_storage().consensus_keys_accessor().entry(validator);
        keys.bls_pubkey_accessor()
            .store(&mut harness.sdk, &[last_byte; BLS_PUBKEY_LENGTH])
            .unwrap();
        keys.peer_pubkey_accessor()
            .set_checked(&mut harness.sdk, B256::with_last_byte(last_byte))
            .unwrap();
        keys.activation_epoch_accessor()
            .set_checked(&mut harness.sdk, 0)
            .unwrap();
    }

    let committee = (vec![validator_a, validator_b],);
    assert_eq!(
        harness
            .call(encode_args_call(SIG_COMMIT_EPOCH_COMMITTEE, &committee))
            .0,
        ExitCode::Panic
    );
    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_args_call(SIG_COMMIT_EPOCH_COMMITTEE, &committee))
            .0,
        ExitCode::Ok
    );

    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE,
        &U64Command { value: 0 },
    ));
    assert_eq!(
        decode_output::<Vec<Address>>(&output),
        vec![validator_a, validator_b]
    );
    let (_, output) = harness.call(encode_call(
        SIG_RESOLVE_SIGNER,
        &EpochSignerCommand {
            epoch: 0,
            signer_idx: 1,
        },
    ));
    assert_eq!(decode_output::<Address>(&output), validator_b);
    let (_, output) = harness.call(encode_empty_call(SIG_NEXT_EPOCH_TO_COMMIT));
    assert_eq!(decode_output::<u64>(&output), 1);
    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE_WITH_STAKES,
        &U64Command { value: 0 },
    ));
    let (validators, _, stakes): (Vec<Address>, Vec<crate::types::ConsensusKeys>, Vec<U256>) =
        decode_returns(&output);
    assert_eq!(validators, vec![validator_a, validator_b]);
    assert_eq!(stakes, vec![stake_a, stake_b]);

    let logs = harness.sdk.take_logs();
    let (_, topics) = logs
        .iter()
        .find(|(_, topics)| {
            topics.first() == Some(&B256::new(events::EpochCommitteeCommitted::SELECTOR))
        })
        .expect("committee event");
    assert_eq!(topics.len(), 2);
    let (data, _) = logs
        .iter()
        .find(|(_, topics)| {
            topics.first() == Some(&B256::new(events::EpochCommitteeCommitted::SELECTOR))
        })
        .expect("committee event");
    let (event_committee,): (Vec<Address>,) = decode_returns(data);
    assert_eq!(event_committee, vec![validator_a, validator_b]);
}

#[test]
fn equal_stake_top_k_preserves_solidity_roster_order() {
    let owner = Address::with_last_byte(0xa0);
    let first = Address::with_last_byte(0xf0);
    let second = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(
            owner,
            vec![first, second],
            vec![DEFAULT_MIN_VALIDATOR_STAKE, DEFAULT_MIN_VALIDATOR_STAKE],
            0,
        ),
        ExitCode::Ok
    );
    staking_storage()
        .config_accessor()
        .active_validators_length_accessor()
        .set_checked(&mut harness.sdk, 1)
        .unwrap();

    let (_, output) = harness.call(encode_empty_call(SIG_GET_VALIDATORS));
    assert_eq!(decode_output::<Vec<Address>>(&output), vec![first]);
}

#[test]
fn committee_pruning_keeps_dkg_history() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let activation_block = 1_000;
    let mut harness = Harness::new(activation_block);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0,),
        ExitCode::Ok
    );
    harness.set_block_number(
        activation_block
            + DEFAULT_EPOCH_BLOCK_INTERVAL
                * (DEFAULT_UNDELEGATE_PERIOD + EPOCH_COMMITTEE_RETENTION_MARGIN + 2),
    );
    let keys = staking_storage().consensus_keys_accessor().entry(validator);
    keys.bls_pubkey_accessor()
        .store(&mut harness.sdk, &[1; BLS_PUBKEY_LENGTH])
        .unwrap();
    keys.peer_pubkey_accessor()
        .set_checked(&mut harness.sdk, B256::with_last_byte(1))
        .unwrap();
    staking_storage()
        .dkg_qual_accessor()
        .entry(1)
        .set_checked(&mut harness.sdk, true)
        .unwrap();

    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_COMMIT_EPOCH_COMMITTEE,
                &(vec![validator],),
            ))
            .0,
        ExitCode::Ok
    );
    assert!(staking_storage()
        .dkg_qual_accessor()
        .entry(1)
        .get_checked(&harness.sdk)
        .unwrap());
}

#[test]
fn liveness_slash_jails_and_readmits_without_breaking_quorum() {
    let owner = Address::with_last_byte(0xa0);
    let liveness = Address::with_last_byte(0xb0);
    let reserve = Address::with_last_byte(0xb1);
    let validators = (1..=4)
        .map(Address::with_last_byte)
        .collect::<Vec<Address>>();
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(
            owner,
            validators.clone(),
            vec![BALANCE_COMPACT_PRECISION; validators.len()],
            500,
        ),
        ExitCode::Ok
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CONFIGURE_DEPENDENCIES,
                &ConfigureDependenciesCommand {
                    liveness_slashing: liveness,
                    blend_reserve: reserve,
                },
            ))
            .0,
        ExitCode::Ok
    );

    harness.set_caller(liveness);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SLASH,
                &AddressCommand {
                    value: validators[0],
                },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validators[0])
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        crate::storage::STATUS_JAIL
    );

    harness.set_block_number(1_200);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_READMIT_EXPIRED_JAILS,
                &U64Command { value: 1 },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validators[0])
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_ACTIVE
    );
}

#[test]
fn chain_config_guards_match_solidity_boundaries() {
    let owner = Address::with_last_byte(0xa0);
    let mut harness = Harness::new(0);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    harness.set_caller(GENESIS_GOVERNANCE);

    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_ACTIVE_VALIDATORS_LENGTH,
            &U32Command { value: 52 },
        )),
        ERR_MAX_ACTIVE_VALIDATORS_EXCEEDED,
    );
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_SLASH_REPORTER_REWARD_BPS,
            &U32Command { value: 0 },
        )),
        ERR_ZERO_VALUE,
    );
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_PARTICIPATION_FLOOR_BPS,
            &U32Command { value: 2_001 },
        )),
        ERR_PARTICIPATION_FLOOR_BPS_TOO_HIGH,
    );
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_BLEND_STIPEND_PER_EPOCH,
            &U256Command {
                value: MAX_BLEND_STIPEND_PER_EPOCH + U256::from(1),
            },
        )),
        ERR_BLEND_STIPEND_PER_EPOCH_TOO_HIGH,
    );
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_DPOS_ACTIVATION_BLOCK,
            &U64Command { value: 401 },
        )),
        ERR_UNALIGNED_ACTIVATION_BLOCK,
    );

    staking_storage()
        .config_accessor()
        .min_undelegate_blocks_accessor()
        .set_checked(&mut harness.sdk, U256::from(1_000))
        .unwrap();
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_UNDELEGATE_PERIOD,
            &U32Command { value: 4 },
        )),
        ERR_UNDELEGATE_WINDOW_TOO_SHORT,
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_DPOS_ACTIVATION_BLOCK,
                &U64Command { value: 1_200 },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_EPOCH_BLOCK_INTERVAL,
                &U32Command { value: 300 },
            ))
            .0,
        ExitCode::Ok
    );
    harness.set_block_number(1_200);
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_EPOCH_BLOCK_INTERVAL,
            &U32Command { value: 200 },
        )),
        ERR_DPOS_ALREADY_ACTIVE,
    );
}

#[test]
fn lifecycle_transitions_preserve_next_epoch_snapshot_frontier() {
    let contract_owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let new_owner = Address::with_last_byte(0x02);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let mut harness = Harness::new(1_000);
    harness.set_caller(contract_owner);
    assert_eq!(
        harness.initialize(contract_owner, vec![validator], vec![stake], 500),
        ExitCode::Ok
    );

    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_DISABLE_VALIDATOR,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );
    let record = staking_storage().validators_accessor().entry(validator);
    assert_eq!(
        record.status_accessor().get_checked(&harness.sdk).unwrap(),
        STATUS_PENDING
    );
    assert_eq!(
        record
            .changed_at_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1
    );
    assert_eq!(
        staking_storage()
            .validator_snapshots_accessor()
            .entry(validator)
            .entry(1)
            .total_delegated_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        stake
    );

    assert_eq!(
        harness
            .call(encode_call(
                SIG_ACTIVATE_VALIDATOR,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );
    harness.set_caller(validator);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CHANGE_VALIDATOR_OWNER,
                &TwoAddressesCommand {
                    validator,
                    value: new_owner,
                },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        record
            .changed_at_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1
    );
    assert_eq!(
        staking_storage()
            .owner_validators_accessor()
            .entry(new_owner)
            .get_checked(&harness.sdk)
            .unwrap(),
        validator
    );
}

#[test]
fn validator_owner_cannot_drop_below_minimum_while_delegators_remain() {
    let owner = Address::with_last_byte(0xa0);
    let delegator = Address::with_last_byte(0xb0);
    let validator = Address::with_last_byte(0x01);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(10);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![stake], 500),
        ExitCode::Ok
    );
    economics::delegate_to(
        &mut harness.sdk,
        delegator,
        validator,
        DEFAULT_MIN_STAKING_AMOUNT * U256::from(2),
        false,
    )
    .unwrap();

    let before = harness.sdk.dump_storage();
    assert_revert_selector(
        (
            economics::undelegate_from(&mut harness.sdk, validator, validator, stake).unwrap_err(),
            harness.sdk.take_output(),
        ),
        ERR_OWNER_SELF_STAKE_BELOW_MINIMUM,
    );
    harness.sdk.restore_storage(before);
}

#[test]
fn reward_claims_are_bounded_to_one_thousand_epochs() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(0);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(
            owner,
            vec![validator],
            vec![DEFAULT_MIN_VALIDATOR_STAKE],
            500,
        ),
        ExitCode::Ok
    );
    harness.set_block_number(DEFAULT_EPOCH_BLOCK_INTERVAL * (MAX_EPOCHS_PER_CLAIM + 1));
    harness.set_caller(validator);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CLAIM_DELEGATOR_FEE,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        staking_storage()
            .validator_delegations_accessor()
            .entry(validator)
            .entry(validator)
            .delegate_queue_accessor()
            .at(0)
            .epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        MAX_EPOCHS_PER_CLAIM
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CLAIM_VALIDATOR_FEE,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validator)
            .claimed_at_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        MAX_EPOCHS_PER_CLAIM
    );
    assert_revert_selector(
        harness.call(encode_call(
            SIG_CLAIM_VALIDATOR_FEE_AT_EPOCH,
            &ValidatorEpochCommand {
                validator,
                before_epoch: MAX_EPOCHS_PER_CLAIM + 2,
            },
        )),
        ERR_INVALID_CLAIM_EPOCH,
    );
}

#[test]
fn liveness_halt_guard_and_equivocation_tombstone_are_enforced() {
    let owner = Address::with_last_byte(0xa0);
    let liveness = Address::with_last_byte(0xb0);
    let validators = (1..=4)
        .map(Address::with_last_byte)
        .collect::<Vec<Address>>();
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(
            owner,
            validators.clone(),
            vec![DEFAULT_MIN_VALIDATOR_STAKE; validators.len()],
            500,
        ),
        ExitCode::Ok
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CONFIGURE_DEPENDENCIES,
                &ConfigureDependenciesCommand {
                    liveness_slashing: liveness,
                    blend_reserve: Address::with_last_byte(0xc0),
                },
            ))
            .0,
        ExitCode::Ok
    );
    harness.set_caller(liveness);
    for validator in &validators[..2] {
        assert_eq!(
            harness
                .call(encode_call(
                    SIG_SLASH,
                    &AddressCommand { value: *validator },
                ))
                .0,
            ExitCode::Ok
        );
    }
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validators[0])
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_JAIL
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validators[1])
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_ACTIVE,
        "the second jail would drop the active set below Simplex quorum"
    );

    staking_storage()
        .tombstoned_accessor()
        .entry(validators[0])
        .set_checked(&mut harness.sdk, true)
        .unwrap();
    harness.set_caller(validators[0]);
    assert_revert_selector(
        harness.call(encode_call(
            SIG_RELEASE_VALIDATOR_FROM_JAIL,
            &AddressCommand {
                value: validators[0],
            },
        )),
        ERR_ALREADY_SLASHED_FOR_EQUIVOCATION,
    );
}

#[test]
fn committee_validation_is_canonical_and_membership_changes_mint_dkg_bit() {
    let owner = Address::with_last_byte(0xa0);
    let validator_a = Address::with_last_byte(0x01);
    let validator_b = Address::with_last_byte(0x02);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(
            owner,
            vec![validator_a, validator_b],
            vec![DEFAULT_MIN_VALIDATOR_STAKE, DEFAULT_MIN_VALIDATOR_STAKE,],
            500,
        ),
        ExitCode::Ok
    );
    for (validator, last_byte) in [(validator_a, 1u8), (validator_b, 2u8)] {
        let keys = staking_storage().consensus_keys_accessor().entry(validator);
        keys.bls_pubkey_accessor()
            .store(&mut harness.sdk, &[last_byte; BLS_PUBKEY_LENGTH])
            .unwrap();
        keys.peer_pubkey_accessor()
            .set_checked(&mut harness.sdk, B256::with_last_byte(last_byte))
            .unwrap();
    }

    harness.set_caller(SYSTEM_CALLER);
    assert_revert_selector(
        harness.call(encode_args_call(
            SIG_COMMIT_EPOCH_COMMITTEE,
            &(vec![validator_b, validator_a],),
        )),
        ERR_COMMITTEE_NOT_STRICTLY_ASCENDING,
    );
    assert_revert_selector(
        harness.call(encode_args_call(
            SIG_COMMIT_EPOCH_COMMITTEE,
            &(vec![validator_a],),
        )),
        ERR_COMMITTEE_LENGTH_MISMATCH,
    );
    let both = (vec![validator_a, validator_b],);
    assert_eq!(
        harness
            .call(encode_args_call(SIG_COMMIT_EPOCH_COMMITTEE, &both))
            .0,
        ExitCode::Ok
    );

    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_DISABLE_VALIDATOR,
                &AddressCommand { value: validator_b },
            ))
            .0,
        ExitCode::Ok
    );
    harness.set_caller(SYSTEM_CALLER);
    for _ in 0..2 {
        assert_eq!(
            harness
                .call(encode_args_call(SIG_COMMIT_EPOCH_COMMITTEE, &both))
                .0,
            ExitCode::Ok
        );
    }
    harness.set_block_number(1_200);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_COMMIT_EPOCH_COMMITTEE,
                &(vec![validator_a],),
            ))
            .0,
        ExitCode::Ok
    );
    let (_, output) = harness.call(encode_call(SIG_GET_DKG_QUAL, &U64Command { value: 3 }));
    assert!(decode_output::<bool>(&output));
}

#[test]
fn external_dependency_flows_fail_closed_before_calls() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(
            owner,
            vec![validator],
            vec![DEFAULT_MIN_VALIDATOR_STAKE],
            500,
        ),
        ExitCode::Ok
    );

    harness.set_caller(validator);
    assert_revert_selector(
        harness.call(encode_args_call(
            SIG_SET_CONSENSUS_KEYS,
            &(validator, Vec::<u8>::new(), Vec::<u8>::new(), B256::ZERO),
        )),
        ERR_INVALID_CONSENSUS_KEY_ENCODING,
    );
    assert_revert_selector(
        harness.call(encode_args_call(
            SIG_SLASH_EQUIVOCATION_NOTARIZE,
            &(
                Vec::<u8>::new(),
                Vec::<u8>::new(),
                Vec::<u8>::new(),
                Vec::<u8>::new(),
            ),
        )),
        ERR_EVIDENCE_DECODER_NOT_CONFIGURED,
    );
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SETTLE_EPOCH_STIPEND,
            &U64Command { value: 0 },
        )),
        ERR_ONLY_SYSTEM_CALL,
    );
}
