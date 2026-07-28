use super::*;
use crate::storage::{staking_storage, STATUS_ACTIVE};
use crate::types::{AddressCommand, ConfigureCommand, InitializeCommand};
use fluentbase_sdk::{
    bytes::BytesMut, codec::SolidityABI, derive::derive_keccak256_id, is_engine_metered_precompile,
    is_execute_using_system_runtime, Address, Bytes, ContractContextV1, ExitCode,
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
        let command = InitializeCommand {
            initial_owner: owner,
            validators,
            initial_stakes: stakes,
            commission_rate,
        };
        self.call(encode_call(SIG_INITIALIZE, &command)).0
    }
}

fn decode_output<T>(output: &[u8]) -> T
where
    T: fluentbase_sdk::codec::Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    SolidityABI::<T>::decode(&output, 0).unwrap()
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
        derive_keccak256_id!("configure(address,uint64,uint64,uint256,uint256)")
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
fn stores_reserved_governance_and_chain_configuration() {
    let owner = Address::with_last_byte(0xa0);
    let staking_token = Address::with_last_byte(0xb1);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    let (_, output) = harness.call(encode_empty_call(SIG_GET_GOVERNANCE));
    assert_eq!(decode_output::<Address>(&output), GENESIS_GOVERNANCE);

    let (_, output) = harness.call(encode_empty_call(SIG_GET_STAKING));
    assert_eq!(decode_output::<Address>(&output), GENESIS_STAKING);
    let (_, output) = harness.call(encode_empty_call(SIG_GET_CHAIN_CONFIG));
    assert_eq!(decode_output::<Address>(&output), GENESIS_STAKING);

    let configure = ConfigureCommand {
        staking_token,
        active_validators_length: 50,
        epoch_block_interval: 100,
        min_validator_stake_amount: U256::from(1_000),
        min_staking_amount: U256::from(10),
    };
    assert_eq!(
        harness.call(encode_call(SIG_CONFIGURE, &configure)).0,
        ExitCode::Ok
    );
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
    let (_, output) = harness.call(encode_empty_call(SIG_GET_STAKING_TOKEN));
    assert_eq!(decode_output::<Address>(&output), staking_token);

    harness.set_block_number(1_100);
    let (_, output) = harness.call(encode_empty_call(SIG_CURRENT_EPOCH));
    assert_eq!(decode_output::<u64>(&output), 1);
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
