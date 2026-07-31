use super::*;
use crate::{
    consts::{STATUS_ACTIVE, STATUS_JAIL, STATUS_PENDING},
    storage::{
        chain_config_storage, consensus_storage, initializer_storage, staking_storage,
        DelegationOpStorage, UndelegationOpStorage, ValidatorSnapshotStorage,
    },
    types::{
        AddressCommand, AddressU16Command, BoolCommand, ConsensusKeys, EpochSignerCommand,
        EquivocationCommand, InitializeCommand, RegisterValidatorCommand, TwoAddressesCommand,
        U256Command, U32Command, U64Command, ValidatorBlockCommand, ValidatorDelegatorCommand,
        ValidatorEpochCommand,
    },
};
use fluentbase_sdk::{
    bytes::BytesMut,
    codec::SolidityABI,
    hex, is_engine_metered_precompile, is_execute_using_system_runtime, keccak256,
    storage::{StorageDescriptor, StorageLayout},
    Address, Bytes, ContextReader, ContractContextV1, ExitCode, SyscallResult, B256,
    GENESIS_GOVERNANCE, GENESIS_STAKING, U256,
};
use fluentbase_testing::TestingContextImpl;
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

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

#[test]
fn compact_storage_matches_solidity_struct_layouts() {
    assert_eq!(ValidatorSnapshotStorage::SLOTS, 1);
    assert_eq!(<ValidatorSnapshotStorage as StorageLayout>::BYTES, 32);
    assert_eq!(DelegationOpStorage::SLOTS, 1);
    assert_eq!(<DelegationOpStorage as StorageLayout>::BYTES, 22);
    assert_eq!(UndelegationOpStorage::SLOTS, 1);
    assert_eq!(<UndelegationOpStorage as StorageLayout>::BYTES, 22);

    let slot = U256::from(7);
    let snapshot = ValidatorSnapshotStorage::new(slot, 0);
    assert_eq!(snapshot.total_delegated_accessor().slot(), slot);
    assert_eq!(snapshot.total_delegated_accessor().offset(), 18);
    assert_eq!(snapshot.slashes_count_accessor().offset(), 14);
    assert_eq!(snapshot.commission_rate_accessor().offset(), 12);
    assert_eq!(snapshot.total_blend_rewards_accessor().offset(), 0);
}

#[test]
fn contract_storage_uses_separate_erc7201_namespaces() {
    let initializer_slot = initializer_storage().initialized_accessor().slot();
    let chain_config_slot = chain_config_storage().staking_token_accessor().slot();
    let consensus_slot = consensus_storage().consensus_keys_accessor().slot();
    let staking_slot = staking_storage().owner_accessor().slot();

    assert_eq!(initializer_slot, INITIALIZER_STORAGE_SLOT);
    assert_eq!(chain_config_slot, CHAIN_CONFIG_STORAGE_SLOT);
    assert_eq!(consensus_slot, CONSENSUS_STORAGE_SLOT);
    assert_eq!(staking_slot, STAKING_STORAGE_SLOT);
    assert_ne!(initializer_slot, chain_config_slot);
    assert_ne!(initializer_slot, consensus_slot);
    assert_ne!(initializer_slot, staking_slot);
    assert_ne!(chain_config_slot, consensus_slot);
    assert_ne!(chain_config_slot, staking_slot);
    assert_ne!(consensus_slot, staking_slot);
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
        self.set_caller(GENESIS_GOVERNANCE);
        let command = self.initialize_command(owner, validators, stakes, commission_rate);
        self.initialize_with(command)
    }

    fn initialize_command(
        &self,
        owner: Address,
        validators: Vec<Address>,
        stakes: Vec<U256>,
        commission_rate: u16,
    ) -> InitializeCommand {
        InitializeCommand {
            initial_owner: owner,
            validators,
            initial_stakes: stakes,
            commission_rate,
            staking_token: Address::with_last_byte(0xf0),
            active_validators_length: DEFAULT_ACTIVE_VALIDATORS_LENGTH as u32,
            epoch_block_interval: DEFAULT_EPOCH_BLOCK_INTERVAL as u32,
            felony_threshold: DEFAULT_FELONY_THRESHOLD,
            validator_jail_epoch_length: DEFAULT_VALIDATOR_JAIL_EPOCH_LENGTH,
            undelegate_period: DEFAULT_UNDELEGATE_PERIOD as u32,
            min_validator_stake_amount: DEFAULT_MIN_VALIDATOR_STAKE,
            min_staking_amount: DEFAULT_MIN_STAKING_AMOUNT,
            dpos_activation_block: self.sdk.context().block_number(),
            bls_verifier: Address::ZERO,
            evidence_decoder: Address::ZERO,
            min_undelegate_blocks: U256::ZERO,
            liveness_slashing: Address::with_last_byte(0xf1),
            blend_reserve: Address::with_last_byte(0xf2),
        }
    }

    fn initialize_with(&mut self, command: InitializeCommand) -> ExitCode {
        let owner = command.initial_owner;
        let exit = self.call(encode_args_call(SIG_INITIALIZE, &command)).0;
        self.set_caller(owner);
        exit
    }
}

enum MockDisbursement {
    Amount(U256),
    EmptyReturn,
    Revert,
}

struct StipendCallState {
    liveness: Address,
    reserve: Address,
    finalized_epoch_p1: u64,
    reserve_balances: VecDeque<U256>,
    disbursements: VecDeque<MockDisbursement>,
    reserve_balance_reads: usize,
    disburse_calls: Vec<(Address, U256)>,
}

fn encode_mock_return<T>(value: &T) -> Bytes
where
    T: fluentbase_sdk::codec::Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    let mut output = BytesMut::new();
    SolidityABI::<T>::encode(value, &mut output, 0).unwrap();
    output.freeze().into()
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
    assert!(
        result.1.len() >= SIG_LEN_BYTES,
        "revert payload is missing the four-byte selector: {:?}",
        result.1
    );
    assert_eq!(&result.1[..4], &selector.to_be_bytes());
}

fn assert_direct_revert(result: Result<(), ExitCode>, sdk: &TestingContextImpl, selector: u32) {
    assert_revert_selector((result.unwrap_err(), sdk.take_output()), selector);
}

fn stipend_test_sdk(
    reserve_balances: Vec<U256>,
    disbursements: Vec<MockDisbursement>,
) -> (Harness, Rc<RefCell<StipendCallState>>, Address) {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let liveness = Address::with_last_byte(0xb0);
    let reserve = Address::with_last_byte(0xc0);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0,),
        ExitCode::Ok
    );

    let config = chain_config_storage();
    config
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::from(100))
        .unwrap();
    config
        .liveness_slashing_accessor()
        .set_checked(&mut harness.sdk, liveness)
        .unwrap();
    config
        .blend_reserve_accessor()
        .set_checked(&mut harness.sdk, reserve)
        .unwrap();
    consensus_storage()
        .epoch_committees_accessor()
        .entry(0)
        .push_checked(&mut harness.sdk, validator)
        .unwrap();
    harness.set_caller(SYSTEM_CALLER);
    harness.sdk.take_logs();

    let calls = Rc::new(RefCell::new(StipendCallState {
        liveness,
        reserve,
        finalized_epoch_p1: 1,
        reserve_balances: reserve_balances.into(),
        disbursements: disbursements.into(),
        reserve_balance_reads: 0,
        disburse_calls: Vec::new(),
    }));
    let call_state = calls.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            if input.len() < SIG_LEN_BYTES {
                return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams);
            }
            let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
            let mut calls = call_state.borrow_mut();
            match (address, selector) {
                (address, SIG_LAST_FINALIZED_EPOCH_P1) if address == calls.liveness => {
                    SyscallResult::new(
                        encode_mock_return(&calls.finalized_epoch_p1),
                        0,
                        0,
                        ExitCode::Ok,
                    )
                }
                (address, SIG_RESERVE_BALANCE) if address == calls.reserve => {
                    calls.reserve_balance_reads += 1;
                    let balance = calls.reserve_balances.pop_front().unwrap_or(U256::ZERO);
                    SyscallResult::new(encode_mock_return(&balance), 0, 0, ExitCode::Ok)
                }
                (address, SIG_PARTICIPATION) if address == calls.liveness => {
                    SyscallResult::new(encode_mock_return(&(1u32, 1u32)), 0, 0, ExitCode::Ok)
                }
                (address, SIG_RESERVE_DISBURSE) if address == calls.reserve => {
                    let params = &input[SIG_LEN_BYTES..];
                    let (recipient, assigned) =
                        SolidityABI::<(Address, U256)>::decode(&params, 0).unwrap();
                    calls.disburse_calls.push((recipient, assigned));
                    match calls.disbursements.pop_front().unwrap() {
                        MockDisbursement::Amount(sent) => {
                            SyscallResult::new(encode_mock_return(&sent), 0, 0, ExitCode::Ok)
                        }
                        MockDisbursement::EmptyReturn => {
                            SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Ok)
                        }
                        MockDisbursement::Revert => {
                            SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic)
                        }
                    }
                }
                _ => SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic),
            }
        });
    (harness, calls, validator)
}

fn stipend_accounting(sdk: &TestingContextImpl, validator: Address) -> (U256, U256, u64) {
    let storage = staking_storage();
    let validator_reward = U256::from(
        storage
            .validator_snapshots_accessor()
            .entry(validator)
            .entry(0)
            .total_blend_rewards_accessor()
            .get_checked(sdk)
            .unwrap(),
    );
    (
        validator_reward,
        storage.credited_blend_accessor().get_checked(sdk).unwrap(),
        storage
            .last_rewarded_epoch_p1_accessor()
            .get_checked(sdk)
            .unwrap(),
    )
}

fn assert_stipend_events(
    sdk: &TestingContextImpl,
    epoch: u64,
    committed_amount: U256,
    skipped: bool,
) {
    let logs = sdk.take_logs();
    assert_eq!(logs.len(), if skipped { 2 } else { 1 });
    let committed_signature = keccak256(events::EpochBlendRewardsCommitted::SIGNATURE.as_bytes());
    let committed = logs
        .iter()
        .find(|(_, topics)| topics[0] == committed_signature)
        .expect("EpochBlendRewardsCommitted must be emitted");
    let committed_epoch = committed.1[1].as_slice();
    assert_eq!(
        SolidityABI::<u64>::decode(&committed_epoch, 0).unwrap(),
        epoch
    );
    assert_eq!(decode_output::<U256>(&committed.0), committed_amount);

    let skipped_signature = keccak256(events::StipendSkipped::SIGNATURE.as_bytes());
    let skipped_log = logs
        .iter()
        .find(|(_, topics)| topics[0] == skipped_signature);
    assert_eq!(skipped_log.is_some(), skipped);
    if let Some((data, topics)) = skipped_log {
        assert!(data.is_empty());
        let skipped_epoch = topics[1].as_slice();
        assert_eq!(
            SolidityABI::<u64>::decode(&skipped_epoch, 0).unwrap(),
            epoch
        );
    }
}

#[test]
fn solidity_bytes_calldata_reaches_staking_handlers() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0,),
        ExitCode::Ok
    );
    harness.set_caller(validator);

    // cast calldata
    // "setConsensusKeys(address,bytes,bytes,bytes32)"
    // 0x0000000000000000000000000000000000000001 0xaabbcc 0xddee 0x...01
    let set_consensus_keys = hex!(
        "225cba85
         0000000000000000000000000000000000000000000000000000000000000001
         0000000000000000000000000000000000000000000000000000000000000080
         00000000000000000000000000000000000000000000000000000000000000c0
         0000000000000000000000000000000000000000000000000000000000000001
         0000000000000000000000000000000000000000000000000000000000000003
         aabbcc0000000000000000000000000000000000000000000000000000000000
         0000000000000000000000000000000000000000000000000000000000000002
         ddee000000000000000000000000000000000000000000000000000000000000"
    );
    assert_revert_selector(
        harness.call(set_consensus_keys),
        ERR_INVALID_CONSENSUS_KEY_ENCODING,
    );
    let mut truncated = set_consensus_keys.to_vec();
    truncated.truncate(SIG_LEN_BYTES + 5 * 32);
    assert_eq!(
        harness.call(truncated).0,
        ExitCode::MalformedBuiltinParams,
        "truncated Solidity bytes must return a decode error instead of panicking"
    );

    // cast calldata
    // "slashEquivocationNotarize(bytes,bytes,bytes,bytes,address,bytes32)"
    // 0x01 0x0203 0x04 0x0506 0x0000000000000000000000000000000000000002 0x...03
    let slash_equivocation = hex!(
        "2bc5fb10
         00000000000000000000000000000000000000000000000000000000000000c0
         0000000000000000000000000000000000000000000000000000000000000100
         0000000000000000000000000000000000000000000000000000000000000140
         0000000000000000000000000000000000000000000000000000000000000180
         0000000000000000000000000000000000000000000000000000000000000002
         0000000000000000000000000000000000000000000000000000000000000003
         0000000000000000000000000000000000000000000000000000000000000001
         0100000000000000000000000000000000000000000000000000000000000000
         0000000000000000000000000000000000000000000000000000000000000002
         0203000000000000000000000000000000000000000000000000000000000000
         0000000000000000000000000000000000000000000000000000000000000001
         0400000000000000000000000000000000000000000000000000000000000000
         0000000000000000000000000000000000000000000000000000000000000002
         0506000000000000000000000000000000000000000000000000000000000000"
    );
    assert_revert_selector(
        harness.call(slash_equivocation),
        ERR_NO_EQUIVOCATION_COMMITMENT,
    );
    let command = consensus::decode_equivocation(&slash_equivocation[4..]).unwrap();
    assert_eq!(&command.evidence[..], &[0x01]);
    assert_eq!(&command.pk_uncompressed[..], &[0x02, 0x03]);
    assert_eq!(&command.sig1_uncompressed[..], &[0x04]);
    assert_eq!(&command.sig2_uncompressed[..], &[0x05, 0x06]);
    assert_eq!(command.beneficiary, Address::with_last_byte(0x02));
    assert_eq!(command.salt, B256::with_last_byte(0x03));
}

#[test]
fn set_consensus_keys_cast_calldata_completes_happy_path() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let verifier = Address::with_last_byte(0xb0);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0,),
        ExitCode::Ok
    );
    harness.set_caller(validator);
    chain_config_storage()
        .bls_verifier_accessor()
        .set_checked(&mut harness.sdk, verifier)
        .unwrap();

    let calls = Rc::new(RefCell::new(Vec::new()));
    let recorded_calls = calls.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            assert_eq!(address, verifier);
            recorded_calls.borrow_mut().push(input.to_vec());
            let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
            let output = match selector {
                SIG_BLS_COMPRESS_G2_UNCHECKED => hex!(
                    "0000000000000000000000000000000000000000000000000000000000000020
                     0000000000000000000000000000000000000000000000000000000000000060
                     3333333333333333333333333333333333333333333333333333333333333333
                     3333333333333333333333333333333333333333333333333333333333333333
                     3333333333333333333333333333333333333333333333333333333333333333"
                )
                .to_vec(),
                SIG_BLS_VERIFY => {
                    hex!("0000000000000000000000000000000000000000000000000000000000000001")
                        .to_vec()
                }
                _ => {
                    return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams)
                }
            };
            SyscallResult::new(output.into(), 0, 0, ExitCode::Ok)
        });

    // cast calldata
    // "setConsensusKeys(address,bytes,bytes,bytes32)"
    // 0x...01 0x{11 * 256} 0x{22 * 128} 0x...01
    let calldata = hex!(
        "225cba85
         0000000000000000000000000000000000000000000000000000000000000001
         0000000000000000000000000000000000000000000000000000000000000080
         00000000000000000000000000000000000000000000000000000000000001a0
         0000000000000000000000000000000000000000000000000000000000000001
         0000000000000000000000000000000000000000000000000000000000000100
         1111111111111111111111111111111111111111111111111111111111111111
         1111111111111111111111111111111111111111111111111111111111111111
         1111111111111111111111111111111111111111111111111111111111111111
         1111111111111111111111111111111111111111111111111111111111111111
         1111111111111111111111111111111111111111111111111111111111111111
         1111111111111111111111111111111111111111111111111111111111111111
         1111111111111111111111111111111111111111111111111111111111111111
         1111111111111111111111111111111111111111111111111111111111111111
         0000000000000000000000000000000000000000000000000000000000000080
         2222222222222222222222222222222222222222222222222222222222222222
         2222222222222222222222222222222222222222222222222222222222222222
         2222222222222222222222222222222222222222222222222222222222222222
         2222222222222222222222222222222222222222222222222222222222222222"
    );
    harness.sdk.take_logs();
    assert_eq!(harness.call(calldata), (ExitCode::Ok, Vec::new()));

    let calls = calls.borrow();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        &calls[0][..SIG_LEN_BYTES],
        &SIG_BLS_COMPRESS_G2_UNCHECKED.to_be_bytes()
    );
    assert_eq!(&calls[1][..SIG_LEN_BYTES], &SIG_BLS_VERIFY.to_be_bytes());

    let stored = consensus_storage()
        .consensus_keys_accessor()
        .entry(validator);
    assert_eq!(
        stored.bls_pubkey_accessor().load(&harness.sdk).unwrap(),
        vec![0x33; BLS_PUBKEY_LENGTH]
    );
    assert_eq!(
        stored
            .peer_pubkey_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        B256::with_last_byte(0x01)
    );
    assert_eq!(
        stored
            .activation_epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0
    );
}

#[test]
fn solidity_bytes_outputs_and_event_match_cast_vectors() {
    // cast abi-encode "f(bytes)" 0xaabbcc
    let encoded_bytes = &hex!(
        "0000000000000000000000000000000000000000000000000000000000000020
         0000000000000000000000000000000000000000000000000000000000000003
         aabbcc0000000000000000000000000000000000000000000000000000000000"
    )[..];
    assert_eq!(
        SolidityABI::<Bytes>::decode(&encoded_bytes, 0).unwrap(),
        Bytes::from_static(&[0xaa, 0xbb, 0xcc])
    );

    assert_eq!(
        encode_args_call(
            SIG_BLS_COMPRESS_G2_UNCHECKED,
            &(Bytes::from_static(&[0xaa, 0xbb, 0xcc]),),
        ),
        hex!(
            "a5d2dd22
             0000000000000000000000000000000000000000000000000000000000000020
             0000000000000000000000000000000000000000000000000000000000000003
             aabbcc0000000000000000000000000000000000000000000000000000000000"
        )
    );
    assert_eq!(
        encode_args_call(
            SIG_BLS_VERIFY,
            &(
                Bytes::from_static(&[0x01]),
                Bytes::from_static(&[0x02, 0x03]),
                Bytes::from_static(&[0x04]),
                Bytes::from_static(&[0x05, 0x06]),
                Bytes::from_static(&[0x07]),
            ),
        ),
        hex!(
            "8bf26133
             00000000000000000000000000000000000000000000000000000000000000a0
             00000000000000000000000000000000000000000000000000000000000000e0
             0000000000000000000000000000000000000000000000000000000000000120
             0000000000000000000000000000000000000000000000000000000000000160
             00000000000000000000000000000000000000000000000000000000000001a0
             0000000000000000000000000000000000000000000000000000000000000001
             0100000000000000000000000000000000000000000000000000000000000000
             0000000000000000000000000000000000000000000000000000000000000002
             0203000000000000000000000000000000000000000000000000000000000000
             0000000000000000000000000000000000000000000000000000000000000001
             0400000000000000000000000000000000000000000000000000000000000000
             0000000000000000000000000000000000000000000000000000000000000002
             0506000000000000000000000000000000000000000000000000000000000000
             0000000000000000000000000000000000000000000000000000000000000001
             0700000000000000000000000000000000000000000000000000000000000000"
        )
    );

    let keys = vec![ConsensusKeys {
        bls_pubkey: Bytes::from_static(&[0xaa, 0xbb, 0xcc]),
        peer_pubkey: B256::with_last_byte(0x01),
        activation_epoch: 7,
    }];
    let mut encoded_keys = BytesMut::new();
    SolidityABI::<Vec<ConsensusKeys>>::encode(&keys, &mut encoded_keys, 0).unwrap();
    // cast abi-encode "f((bytes,bytes32,uint64)[])"
    // "[(0xaabbcc,0x...01,7)]"
    assert_eq!(
        encoded_keys.as_ref(),
        &hex!(
            "0000000000000000000000000000000000000000000000000000000000000020
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000020
             0000000000000000000000000000000000000000000000000000000000000060
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000007
             0000000000000000000000000000000000000000000000000000000000000003
             aabbcc0000000000000000000000000000000000000000000000000000000000"
        )
    );

    let mut sdk = TestingContextImpl::default();
    events::ConsensusKeysSet {
        validator: Address::with_last_byte(0x02),
        bls_pubkey: Bytes::from_static(&[0xaa, 0xbb, 0xcc]),
        peer_pubkey: B256::with_last_byte(0x01),
        activation_epoch: 7,
    }
    .emit(&mut sdk)
    .unwrap();
    let logs = sdk.take_logs();
    assert_eq!(logs.len(), 1);
    assert_eq!(
        events::ConsensusKeysSet::SIGNATURE,
        "ConsensusKeysSet(address,bytes,bytes32,uint64)"
    );
    assert_eq!(logs[0].1.len(), 2);
    assert_eq!(
        logs[0].1[0],
        B256::new(hex!(
            "b0119b4b0cb7a880df8e2f34a5c3a4d23f45d700a23e900c5e5dc9a6fc3e1852"
        ))
    );
    assert_eq!(
        logs[0].1[1],
        B256::new(hex!(
            "0000000000000000000000000000000000000000000000000000000000000002"
        ))
    );
    // cast abi-encode "f(bytes,bytes32,uint64)" 0xaabbcc 0x...01 7
    assert_eq!(
        logs[0].0.as_ref(),
        &hex!(
            "0000000000000000000000000000000000000000000000000000000000000060
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000007
             0000000000000000000000000000000000000000000000000000000000000003
             aabbcc0000000000000000000000000000000000000000000000000000000000"
        )
    );

    events::EpochCommitteeCommitted {
        epoch: 7,
        committee: vec![Address::with_last_byte(0x01), Address::with_last_byte(0x02)],
    }
    .emit(&mut sdk)
    .unwrap();
    let logs = sdk.take_logs();
    assert_eq!(logs.len(), 1);
    assert_eq!(
        events::EpochCommitteeCommitted::SIGNATURE,
        "EpochCommitteeCommitted(uint64,address[])"
    );
    assert_eq!(
        logs[0].1[0],
        B256::new(hex!(
            "015ffbf030c2f06f58cedc968ae2ec9df38a79be1a74f68686ca971ce1994a5d"
        ))
    );
    assert_eq!(
        logs[0].1[1],
        B256::new(hex!(
            "0000000000000000000000000000000000000000000000000000000000000007"
        ))
    );
    // cast abi-encode "f(address[])"
    // "[0x0000000000000000000000000000000000000001,
    //   0x0000000000000000000000000000000000000002]"
    assert_eq!(
        logs[0].0.as_ref(),
        &hex!(
            "0000000000000000000000000000000000000000000000000000000000000020
             0000000000000000000000000000000000000000000000000000000000000002
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000002"
        )
    );
}

#[test]
fn get_consensus_keys_matches_dynamic_struct_return_vectors() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );

    let stored_keys = consensus_storage()
        .consensus_keys_accessor()
        .entry(validator);
    stored_keys
        .peer_pubkey_accessor()
        .set_checked(&mut harness.sdk, B256::with_last_byte(0xff))
        .unwrap();
    stored_keys
        .activation_epoch_accessor()
        .set_checked(&mut harness.sdk, 42)
        .unwrap();

    let (status, empty_output) = harness.call(encode_call(
        SIG_GET_CONSENSUS_KEYS,
        &AddressCommand { value: validator },
    ));
    assert_eq!(status, ExitCode::Ok);
    // cast abi-encode "f((bytes,bytes32,uint64))" "(0x,0x...ff,42)"
    assert_eq!(
        empty_output,
        hex!(
            "0000000000000000000000000000000000000000000000000000000000000020
             0000000000000000000000000000000000000000000000000000000000000060
             00000000000000000000000000000000000000000000000000000000000000ff
             000000000000000000000000000000000000000000000000000000000000002a
             0000000000000000000000000000000000000000000000000000000000000000"
        )
    );

    stored_keys
        .bls_pubkey_accessor()
        .store(&mut harness.sdk, &[0xaa, 0xbb, 0xcc])
        .unwrap();
    stored_keys
        .peer_pubkey_accessor()
        .set_checked(&mut harness.sdk, B256::with_last_byte(0x01))
        .unwrap();
    stored_keys
        .activation_epoch_accessor()
        .set_checked(&mut harness.sdk, 7)
        .unwrap();

    let (status, nonempty_output) = harness.call(encode_call(
        SIG_GET_CONSENSUS_KEYS,
        &AddressCommand { value: validator },
    ));
    assert_eq!(status, ExitCode::Ok);
    // cast abi-encode "f((bytes,bytes32,uint64))" "(0xaabbcc,0x...01,7)"
    assert_eq!(
        nonempty_output,
        hex!(
            "0000000000000000000000000000000000000000000000000000000000000020
             0000000000000000000000000000000000000000000000000000000000000060
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000007
             0000000000000000000000000000000000000000000000000000000000000003
            aabbcc0000000000000000000000000000000000000000000000000000000000"
        )
    );

    let (status, multi_value_output) =
        harness.call(encode_empty_call(SIG_GET_VALIDATORS_WITH_KEYS));
    assert_eq!(status, ExitCode::Ok);
    // cast abi-encode "f(address[],(bytes,bytes32,uint64)[])"
    // "[0x0000000000000000000000000000000000000001]"
    // "[(0xaabbcc,0x...01,7)]"
    assert_eq!(
        multi_value_output,
        hex!(
            "0000000000000000000000000000000000000000000000000000000000000040
             0000000000000000000000000000000000000000000000000000000000000080
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000020
             0000000000000000000000000000000000000000000000000000000000000060
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000007
             0000000000000000000000000000000000000000000000000000000000000003
             aabbcc0000000000000000000000000000000000000000000000000000000000"
        )
    );
}

#[test]
fn parameterized_custom_errors_use_solidity_abi() {
    let owner = Address::with_last_byte(0xa0);
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(owner, Vec::new(), Vec::new(), 0);
    command.liveness_slashing = Address::ZERO;
    let (_, output) = harness.call(encode_args_call(SIG_INITIALIZE, &command));
    assert_eq!(&output[..4], &ERR_ZERO_VALUE.to_be_bytes());
    assert_eq!(decode_output::<String>(&output[4..]), "livenessSlashing");

    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    harness.set_caller(GENESIS_GOVERNANCE);
    let (_, output) = harness.call(encode_call(
        SIG_SET_BLS_VERIFIER,
        &AddressCommand {
            value: Address::ZERO,
        },
    ));
    assert_eq!(&output[..4], &ERR_ZERO_VALUE.to_be_bytes());
    assert_eq!(decode_output::<String>(&output[4..]), "blsVerifier");
}

#[test]
fn derived_selectors_match_independent_hex_pins() {
    for (actual, pinned) in [
        (SIG_INITIALIZE, 0xdca9ac1b),
        (SIG_CURRENT_EPOCH, 0x76671808),
        (SIG_NEXT_EPOCH, 0xaea0e78b),
        (SIG_OWNER, 0x8da5cb5b),
        (SIG_GET_STAKING_TOKEN, 0x9f9106d1),
        (SIG_DEFAULT_PARTICIPATION_FLOOR_BPS, 0x2c1d88e8),
        (SIG_DEFAULT_SLASH_REPORTER_BPS, 0x6cc69027),
        (SIG_MAX_ACTIVE_VALIDATORS, 0x5d887462),
        (SIG_MAX_BLEND_STIPEND_PER_EPOCH, 0x2bc2fec4),
        (SIG_MAX_PARTICIPATION_FLOOR_BPS, 0x9dbdf12b),
        (SIG_MAX_SLASH_REPORTER_BPS, 0x0a3a6183),
        (SIG_GET_VALIDATOR_DELEGATION, 0xd951e186),
        (SIG_GET_VALIDATOR_DELEGATED_STAKE_AT, 0xe8810ea7),
        (SIG_REGISTER_VALIDATOR, 0xdd0fb5df),
        (SIG_DELEGATE, 0x026e402b),
        (SIG_UNDELEGATE, 0x4d99dd16),
        (SIG_IS_VALIDATOR, 0xfacd743b),
        (SIG_IS_VALIDATOR_ACTIVE, 0x42ad55ac),
        (SIG_GET_VALIDATOR_STATUS, 0xa310624f),
        (SIG_GET_VALIDATOR_BY_OWNER, 0x30108c22),
        (SIG_GET_VALIDATORS, 0xb7ab4db5),
        (SIG_ADD_VALIDATOR, 0x4d238c8e),
        (SIG_ACTIVATE_VALIDATOR, 0xb46e5520),
        (SIG_DISABLE_VALIDATOR, 0x1fe97684),
        (SIG_CHANGE_VALIDATOR_COMMISSION_RATE, 0x14f8649f),
        (SIG_CHANGE_VALIDATOR_OWNER, 0x0052c9e1),
        (SIG_SET_ACTIVE_VALIDATORS_LENGTH, 0xc227a412),
        (SIG_SET_EPOCH_BLOCK_INTERVAL, 0xaf70fa2c),
        (SIG_SET_DPOS_ACTIVATION_BLOCK, 0xf517ca6a),
        (SIG_SET_FELONY_THRESHOLD, 0xfcd6cb3e),
        (SIG_SET_VALIDATOR_JAIL_EPOCH_LENGTH, 0xc8652bd5),
        (SIG_SET_SLASH_REPORTER_REWARD_BPS, 0x58702003),
        (SIG_SET_SLASH_FUND_ADDRESS, 0xa79e7263),
        (SIG_SET_PARTICIPATION_FLOOR_BPS, 0xd0a01007),
        (SIG_SET_PARTICIPATION_JAIL_DISABLED, 0x8664f2e7),
        (SIG_SET_BLEND_STIPEND_PER_EPOCH, 0x2c91b879),
        (SIG_SET_UNDELEGATE_PERIOD, 0x41d8a080),
        (SIG_SET_MIN_VALIDATOR_STAKE_AMOUNT, 0xe1a2e863),
        (SIG_SET_MIN_STAKING_AMOUNT, 0x612d669e),
        (SIG_SET_BLS_VERIFIER, 0x466ae541),
        (SIG_SET_EVIDENCE_DECODER, 0x00857c90),
        (SIG_GET_VALIDATOR_FEE, 0x457179fd),
        (SIG_GET_PENDING_VALIDATOR_FEE, 0xc6fb9065),
        (SIG_CLAIM_VALIDATOR_FEE_AT_EPOCH, 0xadf2a79c),
        (SIG_GET_DELEGATOR_FEE, 0x52b7bea2),
        (SIG_CLAIM_DELEGATOR_FEE_AT_EPOCH, 0xfe38ebef),
        (SIG_CALC_AVAILABLE_FOR_REDELEGATE_AMOUNT, 0x5ef9e8c6),
        (SIG_SETTLE_EPOCH_STIPEND, 0xa631344a),
        (SIG_SET_CONSENSUS_KEYS, 0x225cba85),
        (SIG_GET_VALIDATORS_WITH_KEYS_AT, 0x7cfba9f3),
        (SIG_COMMIT_EPOCH_COMMITTEE, 0x87401d8a),
        (SIG_GET_EPOCH_COMMITTEE_WITH_STAKES, 0xa4d160c1),
        (SIG_RELEASE_VALIDATOR_FROM_JAIL, 0x73a3dda6),
        (SIG_SLASH, 0xc96be4cb),
        (SIG_COMMIT_EQUIVOCATION_REPORT, 0x32890bc0),
        (SIG_COMPUTE_EQUIVOCATION_REPORT_COMMITMENT, 0xc289d76e),
        (SIG_GET_EQUIVOCATION_REPORT_COMMITMENT, 0xa3aae5dd),
        (SIG_SLASH_EQUIVOCATION_NOTARIZE, 0x2bc5fb10),
        (SIG_SLASH_EQUIVOCATION_FINALIZE, 0xb034c58b),
        (SIG_SLASH_EQUIVOCATION_NULLIFY_FINALIZE, 0x337e1437),
    ] {
        assert_eq!(actual, pinned);
    }
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
                U256::from(10) * DEFAULT_MIN_VALIDATOR_STAKE,
                U256::from(20) * DEFAULT_MIN_VALIDATOR_STAKE,
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
            U256::from(10) * DEFAULT_MIN_VALIDATOR_STAKE,
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
    let record = staking_storage().validators_accessor().entry(validator);
    assert_eq!(
        record.status_accessor().get_checked(&harness.sdk).unwrap(),
        STATUS_PENDING,
        "governance additions cannot start active without self-stake"
    );
    let (_, output) = harness.call(encode_call(
        SIG_IS_VALIDATOR_ACTIVE,
        &AddressCommand { value: validator },
    ));
    assert!(!decode_output::<bool>(&output));

    assert_revert_selector(
        harness.call(encode_call(
            SIG_ACTIVATE_VALIDATOR,
            &AddressCommand { value: validator },
        )),
        ERR_OWNER_SELF_STAKE_BELOW_MINIMUM,
    );
    staking::delegate_to(
        &mut harness.sdk,
        validator,
        validator,
        DEFAULT_MIN_VALIDATOR_STAKE,
        false,
    )
    .unwrap();
    assert_revert_selector(
        harness.call(encode_call(
            SIG_ACTIVATE_VALIDATOR,
            &AddressCommand { value: validator },
        )),
        ERR_OWNER_SELF_STAKE_BELOW_MINIMUM,
    );

    harness.set_block_number(1_200);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_ACTIVATE_VALIDATOR,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );
    harness.set_block_number(1_400);
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
    let (_, output) = harness.call(encode_call(
        SIG_IS_VALIDATOR,
        &AddressCommand { value: validator },
    ));
    assert!(
        decode_output::<bool>(&output),
        "disabling must preserve the validator record"
    );
    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATOR_BY_OWNER,
        &AddressCommand { value: validator },
    ));
    assert_eq!(
        decode_output::<Address>(&output),
        validator,
        "disabling must preserve the validator-owner mapping"
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
fn stores_chain_configuration_in_its_own_namespace() {
    let owner = Address::with_last_byte(0xa0);
    let staking_token = Address::with_last_byte(0xb1);
    let mut harness = Harness::new(1_000);
    harness.set_caller(GENESIS_GOVERNANCE);

    let mut command = harness.initialize_command(owner, Vec::new(), Vec::new(), 0);
    command.staking_token = staking_token;
    command.active_validators_length = 50;
    command.epoch_block_interval = 100;
    command.felony_threshold = 150;
    command.validator_jail_epoch_length = 7;
    command.undelegate_period = 7;
    command.min_validator_stake_amount = BALANCE_COMPACT_PRECISION;
    command.min_staking_amount = BALANCE_COMPACT_PRECISION;
    command.dpos_activation_block = 1_000;
    command.bls_verifier = Address::with_last_byte(0xb2);
    command.evidence_decoder = Address::with_last_byte(0xb3);
    command.min_undelegate_blocks = U256::from(701);
    assert_revert_selector(
        harness.call(encode_args_call(SIG_INITIALIZE, &command)),
        ERR_UNDELEGATE_WINDOW_TOO_SHORT,
    );
    command.min_undelegate_blocks = U256::from(700);
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
    assert!(initializer_storage()
        .initialized_accessor()
        .get_checked(&harness.sdk)
        .unwrap());
    let retry = harness.initialize_command(owner, Vec::new(), Vec::new(), 0);
    assert_revert_selector(
        harness.call(encode_args_call(SIG_INITIALIZE, &retry)),
        ERR_ALREADY_INITIALIZED,
    );

    let config = chain_config_storage();
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
    let mut command = harness.initialize_command(owner, Vec::new(), Vec::new(), 0);
    command.liveness_slashing = liveness_slashing;
    command.blend_reserve = blend_reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

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
    for selector in [
        SIG_SET_MIN_VALIDATOR_STAKE_AMOUNT,
        SIG_SET_MIN_STAKING_AMOUNT,
    ] {
        assert_revert_selector(
            harness.call(encode_call(
                selector,
                &U256Command {
                    value: DEFAULT_MIN_STAKING_AMOUNT + U256::from(1),
                },
            )),
            ERR_WRONG_AMOUNT_PRECISION,
        );
    }
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
fn initialize_events_report_defaults_as_previous_values() {
    let owner = Address::with_last_byte(0xa0);
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(owner, Vec::new(), Vec::new(), 0);
    command.staking_token = Address::with_last_byte(0xb0);
    command.active_validators_length = 31;
    command.epoch_block_interval = 200;
    command.felony_threshold = 3;
    command.validator_jail_epoch_length = 4;
    command.undelegate_period = 9;
    command.min_validator_stake_amount = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2);
    command.min_staking_amount = DEFAULT_MIN_STAKING_AMOUNT * U256::from(2);
    command.dpos_activation_block = 1_200;
    assert_eq!(
        harness.call(encode_args_call(SIG_INITIALIZE, &command)).0,
        ExitCode::Ok
    );
    let logs = harness.sdk.take_logs();

    let u32_event = |selector: [u8; 32]| {
        let data = &logs
            .iter()
            .find(|(_, topics)| topics.first() == Some(&B256::new(selector)))
            .expect("u32 configuration event")
            .0;
        decode_output::<(u32, u32)>(data)
    };
    assert_eq!(
        u32_event(events::ActiveValidatorsLengthChanged::SELECTOR),
        (DEFAULT_ACTIVE_VALIDATORS_LENGTH as u32, 31)
    );
    assert_eq!(
        u32_event(events::EpochBlockIntervalChanged::SELECTOR),
        (DEFAULT_EPOCH_BLOCK_INTERVAL as u32, 200)
    );
    assert_eq!(
        u32_event(events::FelonyThresholdChanged::SELECTOR),
        (DEFAULT_FELONY_THRESHOLD, 3)
    );
    assert_eq!(
        u32_event(events::ValidatorJailEpochLengthChanged::SELECTOR),
        (DEFAULT_VALIDATOR_JAIL_EPOCH_LENGTH, 4)
    );
    assert_eq!(
        u32_event(events::UndelegatePeriodChanged::SELECTOR),
        (DEFAULT_UNDELEGATE_PERIOD as u32, 9)
    );

    let u256_event = |selector: [u8; 32]| {
        let data = &logs
            .iter()
            .find(|(_, topics)| topics.first() == Some(&B256::new(selector)))
            .expect("U256 configuration event")
            .0;
        decode_output::<(U256, U256)>(data)
    };
    assert_eq!(
        u256_event(events::MinValidatorStakeAmountChanged::SELECTOR),
        (
            DEFAULT_MIN_VALIDATOR_STAKE,
            command.min_validator_stake_amount
        )
    );
    assert_eq!(
        u256_event(events::MinStakingAmountChanged::SELECTOR),
        (DEFAULT_MIN_STAKING_AMOUNT, command.min_staking_amount)
    );

    let activation_data = &logs
        .iter()
        .find(|(_, topics)| {
            topics.first() == Some(&B256::new(events::DposActivationBlockChanged::SELECTOR))
        })
        .expect("activation configuration event")
        .0;
    assert_eq!(
        decode_output::<(u64, u64)>(activation_data),
        (1_000, command.dpos_activation_block)
    );
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
fn initializer_rejects_subminimum_active_validator() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    let command = harness.initialize_command(
        owner,
        vec![validator],
        vec![DEFAULT_MIN_VALIDATOR_STAKE - BALANCE_COMPACT_PRECISION],
        0,
    );

    assert_revert_selector(
        harness.call(encode_args_call(SIG_INITIALIZE, &command)),
        ERR_INITIAL_STAKE_TOO_LOW,
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validator)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_NOT_FOUND
    );
}

#[test]
fn initializer_is_permissionless_for_atomic_deployment_but_one_shot() {
    let owner = Address::with_last_byte(0xa0);
    let deployer = Address::with_last_byte(0xb0);
    let replacement = Address::with_last_byte(0xc0);
    let mut harness = Harness::new(0);
    harness.set_caller(deployer);

    let command = harness.initialize_command(owner, Vec::new(), Vec::new(), 0);
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
    let (_, output) = harness.call(encode_empty_call(SIG_OWNER));
    assert_eq!(decode_output::<Address>(&output), owner);

    harness.set_caller(replacement);
    let command = harness.initialize_command(replacement, Vec::new(), Vec::new(), 0);
    assert_revert_selector(
        harness.call(encode_args_call(SIG_INITIALIZE, &command)),
        ERR_ALREADY_INITIALIZED,
    );
    let (_, output) = harness.call(encode_empty_call(SIG_OWNER));
    assert_eq!(decode_output::<Address>(&output), owner);
}

#[test]
fn initialize_and_registration_reject_bad_commission_and_duplicate_validator() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(0);
    harness.set_caller(GENESIS_GOVERNANCE);
    let command = harness.initialize_command(
        owner,
        vec![validator],
        vec![DEFAULT_MIN_VALIDATOR_STAKE],
        COMMISSION_RATE_MAX + 1,
    );
    assert_revert_selector(
        harness.call(encode_args_call(SIG_INITIALIZE, &command)),
        ERR_BAD_COMMISSION_RATE,
    );
    assert_eq!(
        harness.initialize(
            owner,
            vec![validator],
            vec![DEFAULT_MIN_VALIDATOR_STAKE],
            COMMISSION_RATE_MAX,
        ),
        ExitCode::Ok
    );

    let input = encode_call(
        SIG_REGISTER_VALIDATOR,
        &RegisterValidatorCommand {
            validator,
            commission_rate: 0,
            initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
        },
    );
    assert_revert_selector(
        (
            staking::register_validator(&mut harness.sdk, &input[SIG_LEN_BYTES..]).unwrap_err(),
            harness.sdk.take_output(),
        ),
        ERR_VALIDATOR_ALREADY_EXISTS,
    );
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
    let mut command = harness.initialize_command(
        owner,
        vec![validator],
        vec![one_token * U256::from(10)],
        500,
    );
    command.staking_token = token;
    command.active_validators_length = 21;
    command.epoch_block_interval = 200;
    command.felony_threshold = 150;
    command.validator_jail_epoch_length = 7;
    command.undelegate_period = 7;
    command.min_validator_stake_amount = one_token;
    command.min_staking_amount = one_token;
    command.dpos_activation_block = 1_000;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    staking::delegate_to(
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
    staking::undelegate_from(&mut harness.sdk, delegator, validator, one_token).unwrap();
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
fn future_delegation_and_noop_commission_do_not_bypass_warmup() {
    let contract_owner = Address::with_last_byte(0xa0);
    let validator_a = Address::with_last_byte(0x01);
    let validator_b = Address::with_last_byte(0x02);
    let delegator = Address::with_last_byte(0xb0);
    let initial_a = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2);
    let initial_b = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(3);
    let delegated = DEFAULT_MIN_STAKING_AMOUNT * U256::from(2);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(
            contract_owner,
            vec![validator_a, validator_b],
            vec![initial_a, initial_b],
            0,
        ),
        ExitCode::Ok
    );
    chain_config_storage()
        .active_validators_length_accessor()
        .set_checked(&mut harness.sdk, 1)
        .unwrap();

    staking::delegate_to(&mut harness.sdk, delegator, validator_a, delegated, false).unwrap();
    harness.set_caller(validator_a);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CHANGE_VALIDATOR_COMMISSION_RATE,
                &AddressU16Command {
                    validator: validator_a,
                    value: 0,
                },
            ))
            .0,
        ExitCode::Ok
    );

    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator_a, 1).unwrap(),
        initial_a,
        "the E+2 delegation must not be copied into the E+1 commission snapshot"
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator_a, 2).unwrap(),
        initial_a + delegated
    );
    assert_eq!(
        staking::selected_validators_at(&harness.sdk, 1).unwrap(),
        vec![validator_b]
    );
    assert_eq!(
        staking::selected_validators_at(&harness.sdk, 2).unwrap(),
        vec![validator_a]
    );

    let reward = DEFAULT_MIN_STAKING_AMOUNT;
    staking_storage()
        .validator_snapshots_accessor()
        .entry(validator_a)
        .entry(1)
        .total_blend_rewards_accessor()
        .set_checked(
            &mut harness.sdk,
            math::narrow_reward(reward).expect("reward fits uint96"),
        )
        .unwrap();
    harness.set_block_number(1_400);
    let (_, output) = harness.call(encode_call(
        SIG_GET_DELEGATOR_FEE,
        &ValidatorDelegatorCommand {
            validator: validator_a,
            delegator: validator_a,
        },
    ));
    assert_eq!(
        decode_output::<U256>(&output),
        reward,
        "future stake must not dilute rewards before its warm-up completes"
    );
}

#[test]
fn commission_change_carries_forward_without_copying_future_stake_backward() {
    let contract_owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let delegator = Address::with_last_byte(0xb0);
    let initial = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2);
    let delegated = DEFAULT_MIN_STAKING_AMOUNT;
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(contract_owner, vec![validator], vec![initial], 500),
        ExitCode::Ok
    );
    staking::delegate_to(&mut harness.sdk, delegator, validator, delegated, false).unwrap();

    harness.set_caller(validator);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CHANGE_VALIDATOR_COMMISSION_RATE,
                &AddressU16Command {
                    validator,
                    value: 1_000,
                },
            ))
            .0,
        ExitCode::Ok
    );

    let snapshots = staking_storage()
        .validator_snapshots_accessor()
        .entry(validator);
    assert_eq!(
        math::expand_balance(
            snapshots
                .entry(1)
                .total_delegated_accessor()
                .get_checked(&harness.sdk)
                .unwrap()
        ),
        initial
    );
    assert_eq!(
        math::expand_balance(
            snapshots
                .entry(2)
                .total_delegated_accessor()
                .get_checked(&harness.sdk)
                .unwrap()
        ),
        initial + delegated
    );
    for epoch in [1, 2] {
        assert_eq!(
            snapshots
                .entry(epoch)
                .commission_rate_accessor()
                .get_checked(&harness.sdk)
                .unwrap(),
            1_000
        );
    }
}

#[test]
fn sparse_snapshot_lookup_uses_sorted_materialized_epochs() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let mut harness = Harness::new(0);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![stake], 0),
        ExitCode::Ok
    );

    let future =
        staking::touch_snapshot_at_or_before(&mut harness.sdk, validator, 1_000_000).unwrap();
    future
        .total_delegated_accessor()
        .set_checked(
            &mut harness.sdk,
            math::compact_balance(stake * U256::from(2)).unwrap(),
        )
        .unwrap();
    staking::touch_snapshot_at_or_before(&mut harness.sdk, validator, 500).unwrap();

    let epochs = staking_storage()
        .validator_snapshot_epochs_accessor()
        .entry(validator);
    assert_eq!(epochs.len_checked(&harness.sdk).unwrap(), 3);
    assert_eq!(epochs.at(0).get_checked(&harness.sdk).unwrap(), 0);
    assert_eq!(epochs.at(1).get_checked(&harness.sdk).unwrap(), 500);
    assert_eq!(epochs.at(2).get_checked(&harness.sdk).unwrap(), 1_000_000);
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 999_999).unwrap(),
        stake
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 1_000_000).unwrap(),
        stake * U256::from(2)
    );
}

#[test]
fn undelegation_rejects_a_later_pending_delegation_checkpoint() {
    let owner = Address::with_last_byte(0xa0);
    let delegator = Address::with_last_byte(0xb0);
    let validator = Address::with_last_byte(0x01);
    let delegated = DEFAULT_MIN_STAKING_AMOUNT * U256::from(2);
    let undelegated = DEFAULT_MIN_STAKING_AMOUNT;
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

    staking::delegate_to(&mut harness.sdk, delegator, validator, delegated, false).unwrap();
    assert_direct_revert(
        staking::undelegate_from(&mut harness.sdk, delegator, validator, undelegated),
        &harness.sdk,
        ERR_PENDING_DELEGATION,
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 1).unwrap(),
        DEFAULT_MIN_VALIDATOR_STAKE
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 2).unwrap(),
        DEFAULT_MIN_VALIDATOR_STAKE + delegated
    );

    harness.set_block_number(1_200);
    staking::undelegate_from(&mut harness.sdk, delegator, validator, undelegated).unwrap();
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 1).unwrap(),
        DEFAULT_MIN_VALIDATOR_STAKE
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 2).unwrap(),
        DEFAULT_MIN_VALIDATOR_STAKE + delegated - undelegated
    );
    let latest = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator)
        .delegate_queue_accessor()
        .at(0);
    assert_eq!(
        math::expand_balance(latest.amount_accessor().get_checked(&harness.sdk).unwrap()),
        delegated - undelegated
    );
    assert_eq!(
        latest.epoch_accessor().get_checked(&harness.sdk).unwrap(),
        2
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
    staking::delegate_to(&mut harness.sdk, delegator, validator, ten_tokens, false).unwrap();
    let snapshot = staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(2);
    snapshot
        .total_blend_rewards_accessor()
        .set_checked(
            &mut harness.sdk,
            math::narrow_reward(ten_tokens).expect("reward fits uint96"),
        )
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
    let stake_a = U256::from(10) * DEFAULT_MIN_VALIDATOR_STAKE;
    let stake_b = U256::from(20) * DEFAULT_MIN_VALIDATOR_STAKE;
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
        let keys = consensus_storage()
            .consensus_keys_accessor()
            .entry(validator);
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
    let (validators, keys, stakes): (Vec<Address>, Vec<ConsensusKeys>, Vec<U256>) =
        decode_returns(&output);
    assert_eq!(validators, vec![validator_a, validator_b]);
    assert_eq!(
        keys.iter()
            .map(|value| value.bls_pubkey[0])
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
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
    chain_config_storage()
        .active_validators_length_accessor()
        .set_checked(&mut harness.sdk, 1)
        .unwrap();

    let (_, output) = harness.call(encode_empty_call(SIG_GET_VALIDATORS));
    assert_eq!(decode_output::<Vec<Address>>(&output), vec![first]);
}

#[test]
fn selection_filters_active_validator_below_current_minimum() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0,),
        ExitCode::Ok
    );
    chain_config_storage()
        .min_validator_stake_amount_accessor()
        .set_checked(
            &mut harness.sdk,
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2),
        )
        .unwrap();

    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validator)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_ACTIVE,
        "selection must defend independently from lifecycle status"
    );
    let (_, output) = harness.call(encode_empty_call(SIG_GET_VALIDATORS));
    assert!(decode_output::<Vec<Address>>(&output).is_empty());
    assert!(staking::selected_validators_at(&harness.sdk, 0)
        .unwrap()
        .is_empty());
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
    let keys = consensus_storage()
        .consensus_keys_accessor()
        .entry(validator);
    keys.bls_pubkey_accessor()
        .store(&mut harness.sdk, &[1; BLS_PUBKEY_LENGTH])
        .unwrap();
    keys.peer_pubkey_accessor()
        .set_checked(&mut harness.sdk, B256::with_last_byte(1))
        .unwrap();
    consensus_storage()
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
    assert!(consensus_storage()
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
    let mut command = harness.initialize_command(
        owner,
        validators.clone(),
        vec![DEFAULT_MIN_VALIDATOR_STAKE; validators.len()],
        500,
    );
    command.liveness_slashing = liveness;
    command.blend_reserve = reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

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
        crate::consts::STATUS_JAIL
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
fn liveness_slashing_preserves_fixed_committed_committee_quorum() {
    let owner = Address::with_last_byte(0xa0);
    let liveness = Address::with_last_byte(0xb0);
    let validators = (1..=8)
        .map(Address::with_last_byte)
        .collect::<Vec<Address>>();
    let committee = validators[..7].to_vec();
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(
        owner,
        validators.clone(),
        vec![DEFAULT_MIN_VALIDATOR_STAKE; validators.len()],
        500,
    );
    command.active_validators_length = committee.len() as u32;
    command.liveness_slashing = liveness;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    for (index, validator) in validators.iter().enumerate() {
        let key_byte = (index + 1) as u8;
        let keys = consensus_storage()
            .consensus_keys_accessor()
            .entry(*validator);
        keys.bls_pubkey_accessor()
            .store(&mut harness.sdk, &[key_byte; BLS_PUBKEY_LENGTH])
            .unwrap();
        keys.peer_pubkey_accessor()
            .set_checked(&mut harness.sdk, B256::with_last_byte(key_byte))
            .unwrap();
    }
    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_COMMIT_EPOCH_COMMITTEE,
                &(committee.clone(),),
            ))
            .0,
        ExitCode::Ok
    );

    harness.set_caller(liveness);
    for validator in &committee[..2] {
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
            .active_validators_accessor()
            .len_checked(&harness.sdk),
        Ok(6)
    );

    assert_eq!(
        harness
            .call(encode_call(
                SIG_SLASH,
                &AddressCommand {
                    value: committee[2],
                },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(committee[2])
            .status_accessor()
            .get_checked(&harness.sdk),
        Ok(STATUS_ACTIVE),
        "a seven-member committee must retain its five-member quorum floor"
    );

    let non_committee = validators[7];
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SLASH,
                &AddressCommand {
                    value: non_committee,
                },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(non_committee)
            .status_accessor()
            .get_checked(&harness.sdk),
        Ok(STATUS_JAIL),
        "a non-committee jail must not consume the protected committee quorum"
    );
    assert_eq!(
        staking_storage()
            .active_validators_accessor()
            .len_checked(&harness.sdk),
        Ok(5)
    );
}

#[test]
fn jail_readmission_rejects_validator_below_minimum_self_stake() {
    let owner = Address::with_last_byte(0xa0);
    let liveness = Address::with_last_byte(0xb0);
    let validators = (1..=4)
        .map(Address::with_last_byte)
        .collect::<Vec<Address>>();
    let validator = validators[0];
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(
        owner,
        validators.clone(),
        vec![DEFAULT_MIN_VALIDATOR_STAKE; validators.len()],
        0,
    );
    command.liveness_slashing = liveness;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    harness.set_caller(liveness);
    assert_eq!(
        harness
            .call(encode_call(SIG_SLASH, &AddressCommand { value: validator },))
            .0,
        ExitCode::Ok
    );
    staking::undelegate_from(
        &mut harness.sdk,
        validator,
        validator,
        DEFAULT_MIN_VALIDATOR_STAKE,
    )
    .unwrap();
    harness.set_block_number(1_200);

    harness.set_caller(validator);
    assert_revert_selector(
        harness.call(encode_call(
            SIG_RELEASE_VALIDATOR_FROM_JAIL,
            &AddressCommand { value: validator },
        )),
        ERR_OWNER_SELF_STAKE_BELOW_MINIMUM,
    );

    harness.set_caller(liveness);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_READMIT_EXPIRED_JAILS,
                &U64Command { value: 1 },
            ))
            .0,
        ExitCode::Ok,
        "automated readmission must skip an ineligible validator without blocking the scan"
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validator)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_JAIL
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
    chain_config_storage()
        .dpos_activation_block_accessor()
        .set_checked(&mut harness.sdk, 400)
        .unwrap();
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

    chain_config_storage()
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
fn dpos_activation_at_block_zero_is_already_locked() {
    let owner = Address::with_last_byte(0xa0);
    let mut harness = Harness::new(0);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    harness.set_caller(GENESIS_GOVERNANCE);
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_EPOCH_BLOCK_INTERVAL,
            &U32Command { value: 100 },
        )),
        ERR_DPOS_ALREADY_ACTIVE,
    );
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_DPOS_ACTIVATION_BLOCK,
            &U64Command { value: 200 },
        )),
        ERR_DPOS_ALREADY_ACTIVE,
    );
}

#[test]
fn lifecycle_transitions_preserve_next_epoch_snapshot_frontier() {
    let contract_owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
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
        math::compact_balance(stake).unwrap()
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
    assert_eq!(
        record.status_accessor().get_checked(&harness.sdk).unwrap(),
        STATUS_ACTIVE
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
    staking::delegate_to(
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
            staking::undelegate_from(&mut harness.sdk, validator, validator, stake).unwrap_err(),
            harness.sdk.take_output(),
        ),
        ERR_OWNER_SELF_STAKE_BELOW_MINIMUM,
    );
    harness.sdk.restore_storage(before);

    let withdrawn = stake / U256::from(2);
    staking::undelegate_from(&mut harness.sdk, validator, validator, withdrawn).unwrap();
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 1).unwrap(),
        stake - withdrawn
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 2).unwrap(),
        stake - withdrawn + DEFAULT_MIN_STAKING_AMOUNT * U256::from(2),
        "the earlier owner withdrawal must carry into the future delegation snapshot"
    );
}

#[test]
fn sole_validator_owner_full_exit_deactivates_without_leaving_subminimum_dust() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(10);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![stake], 500),
        ExitCode::Ok
    );

    let dust = DEFAULT_MIN_VALIDATOR_STAKE / U256::from(2);
    let before = harness.sdk.dump_storage();
    assert_revert_selector(
        (
            staking::undelegate_from(&mut harness.sdk, validator, validator, stake - dust)
                .unwrap_err(),
            harness.sdk.take_output(),
        ),
        ERR_OWNER_SELF_STAKE_BELOW_MINIMUM,
    );
    harness.sdk.restore_storage(before);
    staking::undelegate_from(&mut harness.sdk, validator, validator, stake).unwrap();

    let storage = staking_storage();
    assert_eq!(
        storage
            .validators_accessor()
            .entry(validator)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_PENDING
    );
    assert_eq!(
        storage
            .active_validators_accessor()
            .len_checked(&harness.sdk),
        Ok(0)
    );
    let membership = storage.selection_membership_accessor().entry(validator);
    assert!(!membership
        .visible_accessor()
        .get_checked(&harness.sdk)
        .unwrap());
    assert_eq!(
        membership
            .effective_from_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1
    );
    assert_eq!(
        staking::selected_validators_at(&harness.sdk, 0).unwrap(),
        vec![validator],
        "the completed epoch remains available for historical committee validation"
    );
    assert!(staking::selected_validators_at(&harness.sdk, 1)
        .unwrap()
        .is_empty());
}

#[test]
fn validator_owner_is_immutable_and_cannot_detach_self_stake() {
    let contract_owner = Address::with_last_byte(0xa0);
    let owner = Address::with_last_byte(0x01);
    let attempted_owner = Address::with_last_byte(0x02);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(contract_owner, vec![owner], vec![stake], 500),
        ExitCode::Ok
    );

    let before = harness.sdk.dump_storage();
    harness.set_caller(owner);
    assert_revert_selector(
        harness.call(encode_call(
            SIG_CHANGE_VALIDATOR_OWNER,
            &TwoAddressesCommand {
                validator: owner,
                value: attempted_owner,
            },
        )),
        ERR_VALIDATOR_OWNER_IMMUTABLE,
    );
    assert_eq!(harness.sdk.dump_storage(), before);

    let record = staking_storage().validators_accessor().entry(owner);
    assert_eq!(
        record.owner_accessor().get_checked(&harness.sdk).unwrap(),
        owner
    );
    assert_eq!(
        staking_storage()
            .owner_validators_accessor()
            .entry(owner)
            .get_checked(&harness.sdk)
            .unwrap(),
        owner
    );
    assert_eq!(
        staking_storage()
            .owner_validators_accessor()
            .entry(attempted_owner)
            .get_checked(&harness.sdk)
            .unwrap(),
        Address::ZERO
    );
}

#[test]
fn reserve_balance_caps_full_disbursement_and_credited_rewards() {
    let funded = U256::from(60);
    let (mut harness, calls, validator) =
        stipend_test_sdk(vec![funded], vec![MockDisbursement::Amount(funded)]);

    assert_eq!(
        harness
            .call(encode_call(
                SIG_SETTLE_EPOCH_STIPEND,
                &U64Command { value: 0 },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        stipend_accounting(&harness.sdk, validator),
        (funded, funded, 1)
    );
    assert_eq!(calls.borrow().reserve_balance_reads, 1);
    assert_eq!(
        calls.borrow().disburse_calls,
        vec![(GENESIS_STAKING, funded)]
    );
    assert_stipend_events(&harness.sdk, 0, funded, false);
}

#[test]
fn zero_and_partial_disbursements_skip_epoch_without_retry() {
    for sent in [U256::ZERO, U256::from(40)] {
        let assigned = U256::from(100);
        let (mut harness, calls, validator) =
            stipend_test_sdk(vec![assigned], vec![MockDisbursement::Amount(sent)]);

        assert_eq!(
            harness
                .call(encode_call(
                    SIG_SETTLE_EPOCH_STIPEND,
                    &U64Command { value: 0 },
                ))
                .0,
            ExitCode::Ok
        );
        assert_eq!(
            stipend_accounting(&harness.sdk, validator),
            (U256::ZERO, U256::ZERO, 1)
        );
        assert_eq!(calls.borrow().disburse_calls.len(), 1);
        assert_stipend_events(&harness.sdk, 0, U256::ZERO, true);

        assert_eq!(
            harness
                .call(encode_call(
                    SIG_SETTLE_EPOCH_STIPEND,
                    &U64Command { value: 0 },
                ))
                .0,
            ExitCode::Ok
        );
        assert_eq!(
            calls.borrow().disburse_calls.len(),
            1,
            "a skipped epoch must not be retried"
        );
        assert!(harness.sdk.take_logs().is_empty());
    }
}

#[test]
fn zero_reserve_balance_skips_without_disbursement_call() {
    let (mut harness, calls, validator) = stipend_test_sdk(vec![U256::ZERO], Vec::new());

    assert_eq!(
        harness
            .call(encode_call(
                SIG_SETTLE_EPOCH_STIPEND,
                &U64Command { value: 0 },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        stipend_accounting(&harness.sdk, validator),
        (U256::ZERO, U256::ZERO, 1)
    );
    assert!(calls.borrow().disburse_calls.is_empty());
    assert_stipend_events(&harness.sdk, 0, U256::ZERO, true);
}

#[test]
fn failed_or_malformed_disbursement_remains_retryable() {
    for (first_response, expected_exit) in [
        (
            MockDisbursement::EmptyReturn,
            ExitCode::MalformedBuiltinParams,
        ),
        (MockDisbursement::Revert, ExitCode::Panic),
    ] {
        let assigned = U256::from(100);
        let (mut harness, calls, validator) = stipend_test_sdk(
            vec![assigned, assigned],
            vec![first_response, MockDisbursement::Amount(assigned)],
        );

        assert_eq!(
            harness
                .call(encode_call(
                    SIG_SETTLE_EPOCH_STIPEND,
                    &U64Command { value: 0 },
                ))
                .0,
            expected_exit
        );
        assert_eq!(
            stipend_accounting(&harness.sdk, validator),
            (U256::ZERO, U256::ZERO, 0)
        );
        assert!(harness.sdk.take_logs().is_empty());

        assert_eq!(
            harness
                .call(encode_call(
                    SIG_SETTLE_EPOCH_STIPEND,
                    &U64Command { value: 0 },
                ))
                .0,
            ExitCode::Ok
        );
        assert_eq!(
            stipend_accounting(&harness.sdk, validator),
            (assigned, assigned, 1)
        );
        assert_eq!(calls.borrow().disburse_calls.len(), 2);
        assert_stipend_events(&harness.sdk, 0, assigned, false);
    }
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
    let mut command = harness.initialize_command(
        owner,
        validators.clone(),
        vec![DEFAULT_MIN_VALIDATOR_STAKE; validators.len()],
        500,
    );
    command.liveness_slashing = liveness;
    command.blend_reserve = Address::with_last_byte(0xc0);
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
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

    let jailed_record = staking_storage().validators_accessor().entry(validators[0]);
    let initial_deadline = jailed_record
        .jailed_before_accessor()
        .get_checked(&harness.sdk)
        .unwrap();
    harness.set_block_number(1_200);
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
    let extended_deadline = jailed_record
        .jailed_before_accessor()
        .get_checked(&harness.sdk)
        .unwrap();
    assert!(
        extended_deadline > initial_deadline,
        "re-slashing a jailed validator must extend its deadline despite the quorum guard"
    );

    chain_config_storage()
        .validator_jail_epoch_length_accessor()
        .set_checked(&mut harness.sdk, 0)
        .unwrap();
    harness.set_block_number(1_300);
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
        jailed_record
            .jailed_before_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        extended_deadline,
        "a shorter jail configuration must not reduce an existing deadline"
    );

    consensus_storage()
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
        let keys = consensus_storage()
            .consensus_keys_accessor()
            .entry(validator);
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
                validator,
                B256::ZERO,
            ),
        )),
        ERR_NO_EQUIVOCATION_COMMITMENT,
    );
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SETTLE_EPOCH_STIPEND,
            &U64Command { value: 0 },
        )),
        ERR_ONLY_SYSTEM_CALL,
    );
}

#[test]
fn equivocation_commitments_bind_every_reward_domain_field() {
    let beneficiary = Address::with_last_byte(0xa1);
    let staking = GENESIS_STAKING;
    let evidence_hash = keccak256(b"equivocation evidence");
    let salt = B256::with_last_byte(0x51);
    let commitment = consensus::report_commitment_hash(
        1337,
        staking,
        EQUIVOCATION_PROOF_KIND_NOTARIZE,
        evidence_hash,
        beneficiary,
        salt,
    );

    for changed in [
        consensus::report_commitment_hash(
            1338,
            staking,
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
            evidence_hash,
            beneficiary,
            salt,
        ),
        consensus::report_commitment_hash(
            1337,
            Address::with_last_byte(0x99),
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
            evidence_hash,
            beneficiary,
            salt,
        ),
        consensus::report_commitment_hash(
            1337,
            staking,
            EQUIVOCATION_PROOF_KIND_FINALIZE,
            evidence_hash,
            beneficiary,
            salt,
        ),
        consensus::report_commitment_hash(
            1337,
            staking,
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
            keccak256(b"modified evidence"),
            beneficiary,
            salt,
        ),
        consensus::report_commitment_hash(
            1337,
            staking,
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
            evidence_hash,
            Address::with_last_byte(0xa2),
            salt,
        ),
        consensus::report_commitment_hash(
            1337,
            staking,
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
            evidence_hash,
            beneficiary,
            B256::with_last_byte(0x52),
        ),
    ] {
        assert_ne!(commitment, changed);
    }
}

#[test]
fn equivocation_commit_reveal_prevents_copied_reveal_reward_redirection() {
    let owner = Address::with_last_byte(0xa0);
    let beneficiary = Address::with_last_byte(0xa1);
    let competing_beneficiary = Address::with_last_byte(0xa2);
    let front_runner = Address::with_last_byte(0xf0);
    let evidence = Bytes::from_static(b"public equivocation evidence");
    let salt = B256::with_last_byte(0x51);
    let competing_salt = B256::with_last_byte(0x52);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    harness.set_caller(Address::ZERO);
    assert_revert_selector(
        harness.call(encode_args_call(
            SIG_COMMIT_EQUIVOCATION_REPORT,
            &(B256::with_last_byte(1),),
        )),
        ERR_ZERO_EQUIVOCATION_BENEFICIARY,
    );
    harness.set_caller(beneficiary);
    assert_revert_selector(
        harness.call(encode_args_call(
            SIG_COMMIT_EQUIVOCATION_REPORT,
            &(B256::ZERO,),
        )),
        ERR_ZERO_EQUIVOCATION_COMMITMENT,
    );
    assert_revert_selector(
        harness.call(encode_args_call(
            SIG_COMPUTE_EQUIVOCATION_REPORT_COMMITMENT,
            &(
                beneficiary,
                EQUIVOCATION_PROOF_KIND_COUNT,
                keccak256(&evidence),
                salt,
            ),
        )),
        ERR_INVALID_EQUIVOCATION_PROOF_KIND,
    );

    let (_, commitment_output) = harness.call(encode_args_call(
        SIG_COMPUTE_EQUIVOCATION_REPORT_COMMITMENT,
        &(
            beneficiary,
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
            keccak256(&evidence),
            salt,
        ),
    ));
    let (commitment,) = decode_returns::<(B256,)>(&commitment_output);
    assert_eq!(
        commitment,
        consensus::report_commitment_hash(
            0,
            GENESIS_STAKING,
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
            keccak256(&evidence),
            beneficiary,
            salt,
        )
    );
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_COMMIT_EQUIVOCATION_REPORT,
                &(commitment,),
            ))
            .0,
        ExitCode::Ok
    );

    let competing_commitment = consensus::report_commitment_hash(
        0,
        GENESIS_STAKING,
        EQUIVOCATION_PROOF_KIND_NOTARIZE,
        keccak256(&evidence),
        competing_beneficiary,
        competing_salt,
    );
    harness.set_caller(competing_beneficiary);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_COMMIT_EQUIVOCATION_REPORT,
                &(competing_commitment,),
            ))
            .0,
        ExitCode::Ok
    );

    // Copying the first transaction creates only a front-runner-owned entry.
    // It cannot replace or authenticate the beneficiary's commitment.
    harness.set_caller(front_runner);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_COMMIT_EQUIVOCATION_REPORT,
                &(commitment,),
            ))
            .0,
        ExitCode::Ok
    );
    let (_, stored_output) = harness.call(encode_args_call(
        SIG_GET_EQUIVOCATION_REPORT_COMMITMENT,
        &(beneficiary,),
    ));
    assert_eq!(
        decode_returns::<(B256, u64)>(&stored_output),
        (commitment, 1_000)
    );

    let command = EquivocationCommand {
        evidence: evidence.clone(),
        pk_uncompressed: Bytes::new(),
        sig1_uncompressed: Bytes::new(),
        sig2_uncompressed: Bytes::new(),
        beneficiary,
        salt,
    };
    assert_direct_revert(
        consensus::verify_report_commitment(
            &mut harness.sdk,
            &command,
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
        ),
        &harness.sdk,
        ERR_EQUIVOCATION_COMMITMENT_NOT_MATURE,
    );

    harness.set_block_number(1_001);
    harness.set_caller(front_runner);

    // A copied reveal may execute, but the authenticated reward beneficiary
    // remains the account that made the mature commitment.
    assert_eq!(
        consensus::verify_report_commitment(
            &mut harness.sdk,
            &command,
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
        ),
        Ok(())
    );

    let redirected = EquivocationCommand {
        beneficiary: front_runner,
        ..EquivocationCommand {
            evidence: evidence.clone(),
            pk_uncompressed: Bytes::new(),
            sig1_uncompressed: Bytes::new(),
            sig2_uncompressed: Bytes::new(),
            beneficiary,
            salt,
        }
    };
    assert_direct_revert(
        consensus::verify_report_commitment(
            &mut harness.sdk,
            &redirected,
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
        ),
        &harness.sdk,
        ERR_EQUIVOCATION_COMMITMENT_MISMATCH,
    );

    let wrong_salt = EquivocationCommand {
        salt: B256::with_last_byte(0x53),
        ..EquivocationCommand {
            evidence: evidence.clone(),
            pk_uncompressed: Bytes::new(),
            sig1_uncompressed: Bytes::new(),
            sig2_uncompressed: Bytes::new(),
            beneficiary,
            salt,
        }
    };
    assert_direct_revert(
        consensus::verify_report_commitment(
            &mut harness.sdk,
            &wrong_salt,
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
        ),
        &harness.sdk,
        ERR_EQUIVOCATION_COMMITMENT_MISMATCH,
    );

    let wrong_evidence = EquivocationCommand {
        evidence: Bytes::from_static(b"modified evidence"),
        ..EquivocationCommand {
            evidence: evidence.clone(),
            pk_uncompressed: Bytes::new(),
            sig1_uncompressed: Bytes::new(),
            sig2_uncompressed: Bytes::new(),
            beneficiary,
            salt,
        }
    };
    assert_direct_revert(
        consensus::verify_report_commitment(
            &mut harness.sdk,
            &wrong_evidence,
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
        ),
        &harness.sdk,
        ERR_EQUIVOCATION_COMMITMENT_MISMATCH,
    );

    consensus::consume_report_commitment(&mut harness.sdk, beneficiary).unwrap();
    assert_direct_revert(
        consensus::verify_report_commitment(
            &mut harness.sdk,
            &command,
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
        ),
        &harness.sdk,
        ERR_NO_EQUIVOCATION_COMMITMENT,
    );

    let competing_command = EquivocationCommand {
        evidence,
        pk_uncompressed: Bytes::new(),
        sig1_uncompressed: Bytes::new(),
        sig2_uncompressed: Bytes::new(),
        beneficiary: competing_beneficiary,
        salt: competing_salt,
    };
    assert_eq!(
        consensus::verify_report_commitment(
            &mut harness.sdk,
            &competing_command,
            EQUIVOCATION_PROOF_KIND_NOTARIZE,
        ),
        Ok(())
    );
}

#[test]
fn equivocation_seizes_active_and_pending_self_delegation() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let reporter = Address::with_last_byte(0xb0);
    let token = Address::with_last_byte(0xc0);
    let active_stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let pending_operation = DEFAULT_MIN_VALIDATOR_STAKE;
    let pending_stake = pending_operation * U256::from(2);
    let total_stake = active_stake + pending_stake;
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![total_stake], 500),
        ExitCode::Ok
    );
    chain_config_storage()
        .staking_token_accessor()
        .set_checked(&mut harness.sdk, token)
        .unwrap();

    for _ in 0..2 {
        staking::undelegate_from(&mut harness.sdk, validator, validator, pending_operation)
            .unwrap();
    }
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(validator);
    let delegates = delegation.delegate_queue_accessor();
    let undelegates = delegation.undelegate_queue_accessor();
    assert_eq!(
        delegates
            .at(delegates.len_checked(&harness.sdk).unwrap() - 1)
            .amount_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        math::compact_balance(active_stake).unwrap()
    );
    assert_eq!(undelegates.len_checked(&harness.sdk).unwrap(), 2);
    assert_eq!(
        delegation
            .pending_undelegated_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        pending_stake
    );

    let transfers = Rc::new(RefCell::new(Vec::<(Address, U256)>::new()));
    let recorded = transfers.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            assert_eq!(address, token);
            assert_eq!(
                u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap()),
                SIG_ERC20_TRANSFER
            );
            let transfer =
                SolidityABI::<(Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0).unwrap();
            recorded.borrow_mut().push(transfer);
            SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Ok)
        });

    consensus::seize_self_stake(&mut harness.sdk, validator, validator, reporter).unwrap();

    let reporter_reward =
        total_stake * U256::from(DEFAULT_SLASH_REPORTER_REWARD_BPS) / U256::from(10_000);
    assert_eq!(
        transfers.borrow().as_slice(),
        &[
            (reporter, reporter_reward),
            (EQUIVOCATION_BURN_SINK, total_stake - reporter_reward),
        ]
    );
    assert_eq!(delegates.len_checked(&harness.sdk).unwrap(), 0);
    assert_eq!(undelegates.len_checked(&harness.sdk).unwrap(), 0);
    assert_eq!(
        delegation
            .pending_undelegated_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        U256::ZERO
    );
    assert_eq!(
        delegation
            .delegate_gap_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0
    );
    assert_eq!(
        delegation
            .undelegate_gap_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0
    );

    harness.set_block_number(10_000);
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
    assert_eq!(transfers.borrow().len(), 2);
}

#[test]
fn committee_liability_locks_pending_self_stake_through_evidence_window() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let token = Address::with_last_byte(0xc0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let activation_block = 1_000;
    let mut harness = Harness::new(activation_block);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![stake], 500),
        ExitCode::Ok
    );
    chain_config_storage()
        .staking_token_accessor()
        .set_checked(&mut harness.sdk, token)
        .unwrap();

    let keys = consensus_storage()
        .consensus_keys_accessor()
        .entry(validator);
    keys.bls_pubkey_accessor()
        .store(&mut harness.sdk, &[1; BLS_PUBKEY_LENGTH])
        .unwrap();
    keys.peer_pubkey_accessor()
        .set_checked(&mut harness.sdk, B256::with_last_byte(1))
        .unwrap();
    keys.activation_epoch_accessor()
        .set_checked(&mut harness.sdk, 0)
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
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validator)
            .last_committee_epoch_p1_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1
    );

    staking::undelegate_from(&mut harness.sdk, validator, validator, stake).unwrap();
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(validator);
    let undelegates = delegation.undelegate_queue_accessor();
    assert_eq!(
        undelegates
            .at(0)
            .epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        DEFAULT_UNDELEGATE_PERIOD + 1
    );

    let transfers = Rc::new(RefCell::new(Vec::<(Address, U256)>::new()));
    let recorded = transfers.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            assert_eq!(address, token);
            assert_eq!(
                u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap()),
                SIG_ERC20_TRANSFER
            );
            let transfer =
                SolidityABI::<(Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0).unwrap();
            recorded.borrow_mut().push(transfer);
            SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Ok)
        });

    harness.set_caller(validator);
    harness.set_block_number(
        activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * (DEFAULT_UNDELEGATE_PERIOD + 1),
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CLAIM_DELEGATOR_FEE,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );
    assert!(transfers.borrow().is_empty());
    assert_eq!(
        delegation
            .undelegate_gap_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0
    );

    let evidence_window_end = DEFAULT_UNDELEGATE_PERIOD + EPOCH_COMMITTEE_RETENTION_MARGIN;
    harness.set_block_number(
        activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * (evidence_window_end + 1),
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CLAIM_DELEGATOR_FEE,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );
    assert!(transfers.borrow().is_empty());

    consensus_storage()
        .epoch_committees_accessor()
        .entry(0)
        .clear_checked(&mut harness.sdk)
        .unwrap();
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CLAIM_DELEGATOR_FEE,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(transfers.borrow().as_slice(), &[(validator, stake)]);
    assert_eq!(
        delegation
            .pending_undelegated_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        U256::ZERO
    );
    assert_eq!(
        delegation
            .undelegate_gap_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1
    );
}
