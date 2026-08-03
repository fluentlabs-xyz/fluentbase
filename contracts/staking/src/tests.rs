use super::*;
use crate::{
    consts::{STATUS_ACTIVE, STATUS_PENDING},
    storage::{
        chain_config_storage, consensus_storage, initializer_storage, production_liveness_storage,
        staking_storage, CapCheckpointStorage, ConsensusKeysStorage, DelegationOpStorage,
        ProductionValidatorStorage, UndelegationOpStorage, ValidatorSnapshotStorage,
        ValidatorStorage,
    },
    types::{
        AddValidatorCommand, AddressCommand, AddressU16Command, BoolCommand, ConsensusKeys,
        EpochSignerCommand, EquivocationCommand, InitializeCommand, RegisterValidatorCommand,
        RecordProductionCommand, TwoAddressesCommand, U256Command, U32Command, U64Command,
        ValidatorBlockCommand, ValidatorDelegatorCommand, ValidatorEpochCommand,
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
    assert_eq!(<ValidatorSnapshotStorage as StorageLayout>::BYTES, 28);
    assert_eq!(DelegationOpStorage::SLOTS, 1);
    assert_eq!(<DelegationOpStorage as StorageLayout>::BYTES, 22);
    assert_eq!(UndelegationOpStorage::SLOTS, 1);
    assert_eq!(<UndelegationOpStorage as StorageLayout>::BYTES, 30);
    assert_eq!(ConsensusKeysStorage::SLOTS, 5);

    let slot = U256::from(7);
    let snapshot = ValidatorSnapshotStorage::new(slot, 0);
    assert_eq!(snapshot.total_delegated_accessor().slot(), slot);
    assert_eq!(snapshot.total_delegated_accessor().offset(), 18);
    assert_eq!(snapshot.commission_rate_accessor().offset(), 16);
    assert_eq!(snapshot.total_blend_rewards_accessor().offset(), 4);

    // Removing a field from the middle of a packed word relocates everything
    // below it, so the surviving offsets are pinned rather than assumed.
    assert_eq!(ValidatorStorage::SLOTS, 2);
    let validator = ValidatorStorage::new(slot, 0);
    assert_eq!(validator.owner_accessor().offset(), 12);
    assert_eq!(validator.status_accessor().offset(), 11);
    assert_eq!(validator.changed_at_accessor().offset(), 3);
    assert_eq!(validator.claimed_at_accessor().slot(), slot + U256::from(1));

    assert_eq!(CapCheckpointStorage::SLOTS, 1);
    assert_eq!(<CapCheckpointStorage as StorageLayout>::BYTES, 12);

    // The per-block credit writes `total_produced` and `last_produced_epoch_p1`
    // together; both must stay inside the first slot or every recorded block
    // costs a second store.
    assert_eq!(ProductionValidatorStorage::SLOTS, 2);
    let production = ProductionValidatorStorage::new(slot, 0);
    assert_eq!(production.total_produced_accessor().slot(), slot);
    assert_eq!(production.total_produced_accessor().offset(), 24);
    assert_eq!(production.last_produced_epoch_p1_accessor().slot(), slot);
    assert_eq!(production.last_produced_epoch_p1_accessor().offset(), 16);
    assert_eq!(production.last_failed_epoch_p1_accessor().offset(), 8);
    assert_eq!(production.readmit_at_epoch_accessor().offset(), 0);
    assert_eq!(
        production.kick_count_accessor().slot(),
        slot + U256::from(1)
    );
    assert_eq!(production.kick_count_accessor().offset(), 28);
}

#[test]
fn contract_storage_uses_separate_erc7201_namespaces() {
    let initializer_slot = initializer_storage().initialized_accessor().slot();
    let chain_config_slot = chain_config_storage().staking_token_accessor().slot();
    let consensus_slot = consensus_storage().consensus_keys_accessor().slot();
    let staking_slot = staking_storage().validators_accessor().slot();
    let liveness_slot = production_liveness_storage()
        .last_processed_block_accessor()
        .slot();

    assert_eq!(initializer_slot, INITIALIZER_STORAGE_SLOT);
    assert_eq!(chain_config_slot, CHAIN_CONFIG_STORAGE_SLOT);
    assert_eq!(consensus_slot, CONSENSUS_STORAGE_SLOT);
    assert_eq!(staking_slot, STAKING_STORAGE_SLOT);
    assert_eq!(liveness_slot, PRODUCTION_LIVENESS_STORAGE_SLOT);
    let roots = [
        initializer_slot,
        chain_config_slot,
        consensus_slot,
        staking_slot,
        liveness_slot,
    ];
    for (index, root) in roots.iter().enumerate() {
        assert!(
            !roots[index + 1..].contains(root),
            "storage root {index} aliases a later namespace"
        );
    }
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
        sdk.set_call_handler(|_address, _value, input, _fuel_limit| {
            if input.len() < SIG_LEN_BYTES {
                return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams);
            }
            let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
            let output = match selector {
                SIG_BLS_COMPRESS_G2_UNCHECKED => {
                    let args = &input[SIG_LEN_BYTES..];
                    let (uncompressed,) =
                        SolidityABI::<(Bytes,)>::decode_function_args(&args).unwrap();
                    encode_mock_return(&Bytes::from(vec![
                        uncompressed[0].wrapping_add(0x22);
                        BLS_PUBKEY_LENGTH
                    ]))
                }
                SIG_BLS_VERIFY => encode_mock_return(&true),
                SIG_ERC20_TRANSFER_FROM | SIG_ERC20_TRANSFER => encode_mock_return(&true),
                _ => {
                    return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams)
                }
            };
            SyscallResult::new(output, 0, 0, ExitCode::Ok)
        });
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
        let validator_count = validators.len();
        InitializeCommand {
            initial_stake_owner: owner,
            validators,
            initial_stakes: stakes,
            bls_pubkeys_uncompressed: (0..validator_count)
                .map(|index| {
                    Bytes::from(vec![
                        0x11u8.wrapping_add(index as u8);
                        BLS_PUBKEY_UNCOMPRESSED_LENGTH
                    ])
                })
                .collect(),
            bls_pops_uncompressed: (0..validator_count)
                .map(|index| {
                    Bytes::from(vec![
                        0x22u8.wrapping_add(index as u8);
                        BLS_POP_UNCOMPRESSED_LENGTH
                    ])
                })
                .collect(),
            peer_pubkeys: (1..=validator_count)
                .map(|index| B256::with_last_byte(index as u8))
                .collect(),
            commission_rate,
            staking_token: Address::with_last_byte(0xf0),
            active_validators_length: DEFAULT_ACTIVE_VALIDATORS_LENGTH as u32,
            epoch_block_interval: DEFAULT_EPOCH_BLOCK_INTERVAL as u32,
            undelegate_period: DEFAULT_UNDELEGATE_PERIOD as u32,
            min_validator_stake_amount: DEFAULT_MIN_VALIDATOR_STAKE,
            min_staking_amount: DEFAULT_MIN_STAKING_AMOUNT,
            dpos_activation_block: self.sdk.context().block_number(),
            bls_verifier: if validator_count == 0 {
                Address::ZERO
            } else {
                Address::with_last_byte(0xb0)
            },
            evidence_decoder: Address::ZERO,
            min_undelegate_blocks: U256::ZERO,
            liveness_slashing: Address::with_last_byte(0xf1),
            blend_reserve: Address::with_last_byte(0xf2),
        }
    }

    fn initialize_with(&mut self, command: InitializeCommand) -> ExitCode {
        let owner = command.initial_stake_owner;
        let exit = self.call(encode_args_call(SIG_INITIALIZE, &command)).0;
        self.set_caller(owner);
        exit
    }
}

fn store_test_consensus_keys(
    sdk: &mut TestingContextImpl,
    validator: Address,
    key_byte: u8,
    peer_pubkey: B256,
    activation_epoch: u64,
) {
    let keys = consensus_storage()
        .consensus_keys_accessor()
        .entry(validator);
    let parts = keys.bls_pubkey_accessor();
    for index in 0..3 {
        parts
            .at(index)
            .set_checked(sdk, B256::repeat_byte(key_byte))
            .unwrap();
    }
    keys.peer_pubkey_accessor()
        .set_checked(sdk, peer_pubkey)
        .unwrap();
    keys.activation_epoch_accessor()
        .set_checked(sdk, activation_epoch)
        .unwrap();
    consensus_storage()
        .peer_pubkey_owner_accessor()
        .entry(peer_pubkey)
        .set_checked(sdk, validator)
        .unwrap();
    consensus_storage()
        .bls_pubkey_owner_accessor()
        .entry(keccak256(vec![key_byte; BLS_PUBKEY_LENGTH]))
        .set_checked(sdk, validator)
        .unwrap();
}

/// Writes an epoch committee and its frozen leader weights, the pair
/// `commitEpochCommittee` appends together and the stipend reads back.
fn commit_test_committee(sdk: &mut TestingContextImpl, epoch: u64, members: &[(Address, U256)]) {
    let consensus = consensus_storage();
    let committee = consensus.epoch_committees_accessor().entry(epoch);
    let stakes = consensus.leader_stakes_accessor().entry(epoch);
    for (validator, stake) in members {
        committee.push_checked(sdk, *validator).unwrap();
        stakes
            .push_checked(sdk, crate::math::compact_balance(*stake).unwrap())
            .unwrap();
    }
}

fn record_test_production(sdk: &mut TestingContextImpl, epoch: u64, blocks: u32) {
    production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(epoch)
        .set_checked(sdk, blocks)
        .unwrap();
}

enum MockDisbursement {
    Amount(U256),
    EmptyReturn,
    Revert,
}

struct StipendCallState {
    reserve: Address,
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
        .blend_reserve_accessor()
        .set_checked(&mut harness.sdk, reserve)
        .unwrap();
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[(validator, DEFAULT_MIN_VALIDATOR_STAKE)],
    );
    // Epoch 0 is only settleable once it is over and only if it recorded blocks.
    record_test_production(&mut harness.sdk, 0, DEFAULT_EPOCH_BLOCK_INTERVAL as u32);
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL);
    harness.set_caller(SYSTEM_CALLER);
    harness.sdk.take_logs();

    let calls = Rc::new(RefCell::new(StipendCallState {
        reserve,
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
                (address, SIG_RESERVE_BALANCE) if address == calls.reserve => {
                    calls.reserve_balance_reads += 1;
                    let balance = calls.reserve_balances.pop_front().unwrap_or(U256::ZERO);
                    SyscallResult::new(encode_mock_return(&balance), 0, 0, ExitCode::Ok)
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
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    let register_validator = encode_args_call(
        SIG_REGISTER_VALIDATOR,
        &RegisterValidatorCommand {
            validator,
            commission_rate: 0,
            initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
            bls_pubkey_uncompressed: Bytes::from_static(&[0xaa, 0xbb, 0xcc]),
            bls_pop_uncompressed: Bytes::from_static(&[0xdd, 0xee]),
            peer_pubkey: B256::with_last_byte(1),
        },
    );
    assert_revert_selector(
        harness.call(register_validator.clone()),
        ERR_INVALID_CONSENSUS_KEY_ENCODING,
    );
    let mut truncated = register_validator.to_vec();
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
fn add_validator_cast_calldata_registers_consensus_keys_atomically() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let verifier = Address::with_last_byte(0xb0);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    harness.set_caller(GENESIS_GOVERNANCE);
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
    // "addValidator(address,bytes,bytes,bytes32)"
    // 0x...01 0x{11 * 256} 0x{22 * 128} 0x...01
    let calldata = hex!(
        "fff952d5
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
        consensus::read_bls_pubkey(&harness.sdk, validator).unwrap(),
        Bytes::from(vec![0x33; BLS_PUBKEY_LENGTH])
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
        1
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

    let (status, empty_output) = harness.call(encode_call(
        SIG_GET_CONSENSUS_KEYS,
        &AddressCommand {
            value: Address::with_last_byte(0xff),
        },
    ));
    assert_eq!(status, ExitCode::Ok);
    assert_eq!(
        decode_output::<ConsensusKeys>(&empty_output),
        ConsensusKeys::default()
    );

    let (status, nonempty_output) = harness.call(encode_call(
        SIG_GET_CONSENSUS_KEYS,
        &AddressCommand { value: validator },
    ));
    assert_eq!(status, ExitCode::Ok);
    assert_eq!(
        decode_output::<ConsensusKeys>(&nonempty_output),
        ConsensusKeys {
            bls_pubkey: Bytes::from(vec![0x33; BLS_PUBKEY_LENGTH]),
            peer_pubkey: B256::with_last_byte(1),
            activation_epoch: 0,
        }
    );

    let (status, multi_value_output) =
        harness.call(encode_empty_call(SIG_GET_VALIDATORS_WITH_KEYS));
    assert_eq!(status, ExitCode::Ok);
    assert_eq!(
        decode_returns::<(Vec<Address>, Vec<ConsensusKeys>)>(&multi_value_output),
        (
            vec![validator],
            vec![ConsensusKeys {
                bls_pubkey: Bytes::from(vec![0x33; BLS_PUBKEY_LENGTH]),
                peer_pubkey: B256::with_last_byte(1),
                activation_epoch: 0,
            }],
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
    for (selector, field) in [
        (SIG_SET_BLS_VERIFIER, "blsVerifier"),
        (SIG_SET_LIVENESS_SLASHING, "livenessSlashing"),
        (SIG_SET_BLEND_RESERVE, "blendReserve"),
    ] {
        let (_, output) = harness.call(encode_call(
            selector,
            &AddressCommand {
                value: Address::ZERO,
            },
        ));
        assert_eq!(&output[..4], &ERR_ZERO_VALUE.to_be_bytes());
        assert_eq!(decode_output::<String>(&output[4..]), field);
    }
}

#[test]
fn derived_selectors_match_independent_hex_pins() {
    for (actual, pinned) in [
        (SIG_INITIALIZE, 0xd86555fe),
        (SIG_CURRENT_EPOCH, 0x76671808),
        (SIG_NEXT_EPOCH, 0xaea0e78b),
        (SIG_GET_STAKING_TOKEN, 0x9f9106d1),
        (SIG_DEFAULT_SLASH_REPORTER_BPS, 0x6cc69027),
        (SIG_MAX_ACTIVE_VALIDATORS, 0x5d887462),
        (SIG_MAX_BLEND_STIPEND_PER_EPOCH, 0x2bc2fec4),
        (SIG_MAX_SLASH_REPORTER_BPS, 0x0a3a6183),
        (SIG_GET_VALIDATOR_DELEGATION, 0xd951e186),
        (SIG_GET_VALIDATOR_DELEGATED_STAKE_AT, 0xe8810ea7),
        (SIG_REGISTER_VALIDATOR, 0x8d6067ed),
        (SIG_DELEGATE, 0x026e402b),
        (SIG_UNDELEGATE, 0x4d99dd16),
        (SIG_IS_VALIDATOR, 0xfacd743b),
        (SIG_IS_VALIDATOR_ACTIVE, 0x42ad55ac),
        (SIG_GET_VALIDATOR_STATUS, 0xa310624f),
        (SIG_GET_VALIDATOR_BY_OWNER, 0x30108c22),
        (SIG_GET_VALIDATORS, 0xb7ab4db5),
        (SIG_ADD_VALIDATOR, 0xfff952d5),
        (SIG_ACTIVATE_VALIDATOR, 0xb46e5520),
        (SIG_DISABLE_VALIDATOR, 0x1fe97684),
        (SIG_CHANGE_VALIDATOR_COMMISSION_RATE, 0x14f8649f),
        (SIG_CHANGE_VALIDATOR_OWNER, 0x0052c9e1),
        (SIG_SET_ACTIVE_VALIDATORS_LENGTH, 0xc227a412),
        (SIG_GET_ACTIVE_VALIDATORS_LENGTH_AT, 0xd9b083ba),
        (SIG_SET_EPOCH_BLOCK_INTERVAL, 0xaf70fa2c),
        (SIG_SET_DPOS_ACTIVATION_BLOCK, 0xf517ca6a),
        (SIG_SET_SLASH_REPORTER_REWARD_BPS, 0x58702003),
        (SIG_SET_SLASH_FUND_ADDRESS, 0xa79e7263),
        (SIG_SET_BLEND_STIPEND_PER_EPOCH, 0x2c91b879),
        (SIG_SET_UNDELEGATE_PERIOD, 0x41d8a080),
        (SIG_SET_MIN_VALIDATOR_STAKE_AMOUNT, 0xe1a2e863),
        (SIG_SET_MIN_STAKING_AMOUNT, 0x612d669e),
        (SIG_SET_BLS_VERIFIER, 0x466ae541),
        (SIG_SET_EVIDENCE_DECODER, 0x00857c90),
        (SIG_GET_LIVENESS_SLASHING, 0xdb2366b4),
        (SIG_SET_LIVENESS_SLASHING, 0xbb32522a),
        (SIG_GET_BLEND_RESERVE, 0x37dff538),
        (SIG_SET_BLEND_RESERVE, 0x7899ae8f),
        (SIG_GET_VALIDATOR_FEE, 0x457179fd),
        (SIG_GET_PENDING_VALIDATOR_FEE, 0xc6fb9065),
        (SIG_CLAIM_VALIDATOR_FEE_AT_EPOCH, 0xadf2a79c),
        (SIG_GET_DELEGATOR_FEE, 0x52b7bea2),
        (SIG_GET_VALIDATOR_SELF_STAKE_LOCK, 0xc72e0d73),
        (SIG_CLAIM_DELEGATOR_FEE_AT_EPOCH, 0xfe38ebef),
        (SIG_CALC_AVAILABLE_FOR_REDELEGATE_AMOUNT, 0x5ef9e8c6),
        (SIG_SETTLE_EPOCH_STIPEND, 0xa631344a),
        (SIG_GET_VALIDATORS_WITH_KEYS_AT, 0x7cfba9f3),
        (SIG_COMMIT_EPOCH_COMMITTEE, 0x87401d8a),
        (SIG_GET_EPOCH_COMMITTEE_WITH_STAKES, 0xa4d160c1),
        (SIG_COMMIT_EQUIVOCATION_REPORT, 0x32890bc0),
        (SIG_COMPUTE_EQUIVOCATION_REPORT_COMMITMENT, 0xc289d76e),
        (SIG_GET_EQUIVOCATION_REPORT_COMMITMENT, 0xa3aae5dd),
        (SIG_SLASH_EQUIVOCATION_NOTARIZE, 0x2bc5fb10),
        (SIG_SLASH_EQUIVOCATION_FINALIZE, 0xb034c58b),
        (SIG_SLASH_EQUIVOCATION_NULLIFY_FINALIZE, 0x337e1437),
        (SIG_DEFAULT_MIN_VERDICT_DUE_BLOCKS, 0x6fd3afb7),
        (SIG_DEFAULT_EXCLUSION_BACKOFF_CAP, 0xd4c30c1a),
        (SIG_MAX_MIN_VERDICT_DUE_BLOCKS, 0x9b9a11ba),
        (SIG_GET_MIN_VERDICT_DUE_BLOCKS, 0xee3ad0e7),
        (SIG_SET_MIN_VERDICT_DUE_BLOCKS, 0x4fae9dea),
        (SIG_GET_EXCLUSION_BACKOFF_CAP, 0x6bed0322),
        (SIG_SET_EXCLUSION_BACKOFF_CAP, 0x3b543e1c),
        (SIG_GET_PRODUCTION_LIVENESS_DISABLED, 0x9a4c46bb),
        (SIG_SET_PRODUCTION_LIVENESS_DISABLED, 0x8fc07556),
        (SIG_GET_PRODUCTION_STATS, 0x8e948ac1),
        (SIG_BLOCKS_IN_EPOCH, 0xf06be669),
        (SIG_PRODUCED_AT, 0x91c7d453),
        (SIG_PENDING_EXCLUSIONS, 0xaef690f9),
        (SIG_READMIT_AT_EPOCH, 0x32066046),
        (SIG_LAST_PROCESSED_BLOCK, 0x33de61d2),
        (SIG_RECORD_PRODUCTION, 0x8244a2c2),
        (SIG_SETTLE_EPOCH_STIPEND_FROM, 0x92d321ab),
        (ERR_MIN_VERDICT_DUE_BLOCKS_TOO_HIGH, 0xb1776ed0),
        (ERR_ONLY_SELF_CALL, 0xff54bf4b),
    ] {
        assert_eq!(actual, pinned);
    }
}

#[test]
fn production_liveness_event_signatures_match_the_solidity_abi() {
    // Event field types are mapped by type name and an unrecognised path
    // degrades silently to `tuple`, so the signatures are pinned as strings
    // rather than compared against the same derivation that produced them.
    assert_eq!(
        events::MinVerdictDueBlocksChanged::SIGNATURE,
        "MinVerdictDueBlocksChanged(uint32,uint32)"
    );
    assert_eq!(
        events::ExclusionBackoffCapChanged::SIGNATURE,
        "ExclusionBackoffCapChanged(uint32,uint32)"
    );
    assert_eq!(
        events::ProductionLivenessDisabledChanged::SIGNATURE,
        "ProductionLivenessDisabledChanged(bool,bool)"
    );
    assert_eq!(
        events::ProductionExclusionApplied::SIGNATURE,
        "ProductionExclusionApplied(address,uint64)"
    );
    assert_eq!(
        events::ProductionExclusionReleased::SIGNATURE,
        "ProductionExclusionReleased(address,uint64)"
    );
    assert_eq!(
        events::PartialEpoch::SIGNATURE,
        "PartialEpoch(uint64,uint32,uint32)"
    );
    assert_eq!(
        events::ProductionVerdictFailed::SIGNATURE,
        "ProductionVerdictFailed(uint64,address,uint32,uint256)"
    );
    assert_eq!(
        events::CorrelatedFailureEpoch::SIGNATURE,
        "CorrelatedFailureEpoch(uint64,uint256,uint256)"
    );
    assert_eq!(
        events::StipendLegSkipped::SIGNATURE,
        "StipendLegSkipped(uint64)"
    );
    assert_eq!(
        events::PartialEpoch::SELECTOR,
        hex!("0e3a2b176af3f126559b647eec7cf85052c5cf6239a5fb6869c57d8416225690")
    );
    assert_eq!(
        events::ProductionVerdictFailed::SELECTOR,
        hex!("4d49874ac1e640f94f7ab435dd2305f79014052c7c659aac7f36742d043f8bd8")
    );
    assert_eq!(
        events::CorrelatedFailureEpoch::SELECTOR,
        hex!("3a7c10dc4c9367950614ebeed14db659b710db26c303e92aaf4ac4cdd5b10925")
    );
    assert_eq!(
        events::StipendLegSkipped::SELECTOR,
        hex!("d4266dfa609215f824cf7ef1953a79620625b0e0595260ffd393280da7285dbd")
    );
    assert_eq!(
        events::ProductionExclusionApplied::SELECTOR,
        hex!("b8336509c4e8c35e4348c3c1666d3687b7ba430c61e7d929d7f50010984da426")
    );
    assert_eq!(
        events::ProductionExclusionReleased::SELECTOR,
        hex!("1ce26a6c35478b6335c4d63746f3728de9394747163281c4383bf3ac95137644")
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
                U256::from(10) * DEFAULT_MIN_VALIDATOR_STAKE,
                U256::from(20) * DEFAULT_MIN_VALIDATOR_STAKE,
            ],
            500,
        ),
        ExitCode::Ok
    );

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
    let status: (Address, u8, U256, u64, u64, u16) = decode_output(&output);
    assert_eq!(
        status,
        (
            validator_a,
            STATUS_ACTIVE,
            U256::from(10) * DEFAULT_MIN_VALIDATOR_STAKE,
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
    chain_config_storage()
        .bls_verifier_accessor()
        .set_checked(&mut harness.sdk, Address::with_last_byte(0xb0))
        .unwrap();

    let add_validator_command = || AddValidatorCommand {
        validator,
        bls_pubkey_uncompressed: Bytes::from(vec![0x11; BLS_PUBKEY_UNCOMPRESSED_LENGTH]),
        bls_pop_uncompressed: Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]),
        peer_pubkey: B256::with_last_byte(1),
    };

    harness.set_caller(outsider);
    let (exit, _) = harness.call(encode_args_call(
        SIG_ADD_VALIDATOR,
        &add_validator_command(),
    ));
    assert_eq!(exit, ExitCode::Panic);

    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_ADD_VALIDATOR,
                &add_validator_command(),
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
            SIG_DEFAULT_SLASH_REPORTER_BPS,
            DEFAULT_SLASH_REPORTER_REWARD_BPS,
        ),
        (
            SIG_MAX_ACTIVE_VALIDATORS,
            MAX_ACTIVE_VALIDATORS_LENGTH as u32,
        ),
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
    let replacement_liveness = Address::with_last_byte(0xd4);
    let replacement_reserve = Address::with_last_byte(0xd5);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    let mut command = harness.initialize_command(owner, Vec::new(), Vec::new(), 0);
    command.dpos_activation_block = 2_000;
    command.liveness_slashing = liveness_slashing;
    command.blend_reserve = blend_reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
    harness.sdk.take_logs();

    harness.set_caller(outsider);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_SLASH_REPORTER_REWARD_BPS,
                &U32Command { value: 3 },
            ))
            .0,
        ExitCode::Panic
    );
    for selector in [SIG_SET_LIVENESS_SLASHING, SIG_SET_BLEND_RESERVE] {
        assert_revert_selector(
            harness.call(encode_call(
                selector,
                &AddressCommand {
                    value: Address::with_last_byte(0xee),
                },
            )),
            ERR_ONLY_GOVERNANCE,
        );
    }

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
        (SIG_SET_SLASH_REPORTER_REWARD_BPS, 2_500),
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
        (SIG_SET_LIVENESS_SLASHING, replacement_liveness),
        (SIG_SET_BLEND_RESERVE, replacement_reserve),
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
                SIG_SET_BLEND_STIPEND_PER_EPOCH,
                &U256Command {
                    value: U256::from(42),
                },
            ))
            .0,
        ExitCode::Ok
    );

    for (selector, expected) in [
        (SIG_GET_SLASH_REPORTER_REWARD_BPS, 2_500),
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
        (SIG_GET_LIVENESS_SLASHING, replacement_liveness),
        (SIG_GET_BLEND_RESERVE, replacement_reserve),
    ] {
        let (exit, output) = harness.call(encode_empty_call(selector));
        assert_eq!(exit, ExitCode::Ok);
        assert_eq!(decode_output::<Address>(&output), expected);
    }
    let (_, output) = harness.call(encode_empty_call(SIG_GET_BLEND_STIPEND_PER_EPOCH));
    assert_eq!(decode_output::<U256>(&output), U256::from(42));

    let logs = harness.sdk.take_logs();
    for (selector, expected) in [
        (
            events::LivenessSlashingChanged::SELECTOR,
            (liveness_slashing, replacement_liveness),
        ),
        (
            events::BlendReserveChanged::SELECTOR,
            (blend_reserve, replacement_reserve),
        ),
    ] {
        let data = &logs
            .iter()
            .find(|(_, topics)| topics.first() == Some(&B256::new(selector)))
            .expect("dependency change event")
            .0;
        assert_eq!(decode_output::<(Address, Address)>(data), expected);
    }
}

#[test]
fn initialize_events_report_defaults_as_previous_values() {
    let owner = Address::with_last_byte(0xa0);
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(owner, Vec::new(), Vec::new(), 0);
    command.staking_token = Address::with_last_byte(0xb0);
    command.active_validators_length = 31;
    command.epoch_block_interval = 200;
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

    for (signature, expected_signature, selector, pinned, expected) in [
        (
            events::LivenessSlashingChanged::SIGNATURE,
            "LivenessSlashingChanged(address,address)",
            events::LivenessSlashingChanged::SELECTOR,
            hex!("60722009eccddf6548f3d2699ac687292d370409c60e81feb25ceebd1c236c37"),
            (Address::ZERO, command.liveness_slashing),
        ),
        (
            events::BlendReserveChanged::SIGNATURE,
            "BlendReserveChanged(address,address)",
            events::BlendReserveChanged::SELECTOR,
            hex!("58bf6b15bd5404c0ab55a8db9e88ce5c154feb8c288266925ea26421253a6390"),
            (Address::ZERO, command.blend_reserve),
        ),
    ] {
        assert_eq!(signature, expected_signature);
        assert_eq!(selector, pinned);
        let data = &logs
            .iter()
            .find(|(_, topics)| topics.first() == Some(&B256::new(selector)))
            .expect("initial dependency event")
            .0;
        assert_eq!(decode_output::<(Address, Address)>(data), expected);
    }
}

#[test]
fn initializer_rejects_mismatched_arrays_without_persisting_state() {
    let governance = Address::with_last_byte(0xa0);
    let mut harness = Harness::new(1_000);
    harness.set_caller(governance);

    assert_eq!(
        harness.initialize(governance, vec![Address::with_last_byte(1)], Vec::new(), 0,),
        ExitCode::Panic
    );

    let mut command = harness.initialize_command(
        governance,
        vec![Address::with_last_byte(1)],
        vec![DEFAULT_MIN_VALIDATOR_STAKE],
        0,
    );
    command.bls_pops_uncompressed.clear();
    assert_revert_selector(
        harness.call(encode_args_call(SIG_INITIALIZE, &command)),
        ERR_MALFORMED_INPUT_LENGTH,
    );

    assert!(!initializer_storage()
        .initialized_accessor()
        .get_checked(&harness.sdk)
        .unwrap());
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
    assert!(initializer_storage()
        .initialized_accessor()
        .get_checked(&harness.sdk)
        .unwrap());

    harness.set_caller(replacement);
    let command = harness.initialize_command(replacement, Vec::new(), Vec::new(), 0);
    assert_revert_selector(
        harness.call(encode_args_call(SIG_INITIALIZE, &command)),
        ERR_ALREADY_INITIALIZED,
    );
    assert!(initializer_storage()
        .initialized_accessor()
        .get_checked(&harness.sdk)
        .unwrap());
}

#[test]
fn initializer_pulls_genesis_stake_from_declared_sponsor() {
    let sponsor = Address::with_last_byte(0xa0);
    let deployer = Address::with_last_byte(0xb0);
    let validator = Address::with_last_byte(0x01);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let mut harness = Harness::new(0);
    let token = Address::with_last_byte(0xf0);
    let captured = Rc::new(RefCell::new(None));
    let captured_call = captured.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
            let output = match selector {
                SIG_BLS_COMPRESS_G2_UNCHECKED => {
                    let args = &input[SIG_LEN_BYTES..];
                    let (uncompressed,) =
                        SolidityABI::<(Bytes,)>::decode_function_args(&args).unwrap();
                    encode_mock_return(&Bytes::from(vec![
                        uncompressed[0].wrapping_add(0x22);
                        BLS_PUBKEY_LENGTH
                    ]))
                }
                SIG_BLS_VERIFY => encode_mock_return(&true),
                SIG_ERC20_TRANSFER_FROM => {
                    let (from, to, amount) =
                        decode_output::<(Address, Address, U256)>(&input[SIG_LEN_BYTES..]);
                    *captured_call.borrow_mut() = Some((address, from, to, amount));
                    encode_mock_return(&true)
                }
                _ => {
                    return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams)
                }
            };
            SyscallResult::new(output, 0, 0, ExitCode::Ok)
        });
    harness.set_caller(deployer);

    let command = harness.initialize_command(sponsor, vec![validator], vec![stake], 0);
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
    assert_eq!(
        *captured.borrow(),
        Some((token, sponsor, GENESIS_STAKING, stake))
    );
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

    let input = encode_args_call(
        SIG_REGISTER_VALIDATOR,
        &RegisterValidatorCommand {
            validator,
            commission_rate: 0,
            initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
            bls_pubkey_uncompressed: Bytes::from(vec![0x11; BLS_PUBKEY_UNCOMPRESSED_LENGTH]),
            bls_pop_uncompressed: Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]),
            peer_pubkey: B256::with_last_byte(0xff),
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
fn register_validator_verifies_and_stores_consensus_keys_in_one_call() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let verifier = Address::with_last_byte(0xb0);
    let peer_pubkey = B256::with_last_byte(0x11);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    chain_config_storage()
        .bls_verifier_accessor()
        .set_checked(&mut harness.sdk, verifier)
        .unwrap();
    harness.set_caller(owner);

    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_REGISTER_VALIDATOR,
                &RegisterValidatorCommand {
                    validator,
                    commission_rate: 500,
                    initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                    bls_pubkey_uncompressed: Bytes::from(vec![
                        0x11;
                        BLS_PUBKEY_UNCOMPRESSED_LENGTH
                    ]),
                    bls_pop_uncompressed: Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]),
                    peer_pubkey,
                },
            ))
            .0,
        ExitCode::Ok
    );

    let record = staking_storage().validators_accessor().entry(validator);
    assert_eq!(
        record.owner_accessor().get_checked(&harness.sdk).unwrap(),
        owner
    );
    assert_eq!(
        record.status_accessor().get_checked(&harness.sdk).unwrap(),
        STATUS_PENDING
    );
    assert_eq!(
        consensus::read_bls_pubkey(&harness.sdk, validator).unwrap(),
        Bytes::from(vec![0x33; BLS_PUBKEY_LENGTH])
    );
    let keys = consensus_storage()
        .consensus_keys_accessor()
        .entry(validator);
    assert_eq!(
        keys.peer_pubkey_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        peer_pubkey
    );
    assert_eq!(
        keys.activation_epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1
    );
    assert_eq!(
        consensus_storage()
            .bls_pubkey_owner_accessor()
            .entry(keccak256(vec![0x33; BLS_PUBKEY_LENGTH]))
            .get_checked(&harness.sdk)
            .unwrap(),
        validator
    );
}

#[test]
fn registration_rejects_replayed_bls_key_and_pop_without_partial_state() {
    let first_owner = Address::with_last_byte(0xa0);
    let second_owner = Address::with_last_byte(0xa1);
    let first_validator = Address::with_last_byte(0x01);
    let second_validator = Address::with_last_byte(0x02);
    let verifier = Address::with_last_byte(0xb0);
    let first_peer_pubkey = B256::with_last_byte(0x11);
    let second_peer_pubkey = B256::with_last_byte(0x12);
    let bls_pubkey_uncompressed = Bytes::from(vec![0x11; BLS_PUBKEY_UNCOMPRESSED_LENGTH]);
    let bls_pop_uncompressed = Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]);
    let bls_pubkey_hash = keccak256(vec![0x33; BLS_PUBKEY_LENGTH]);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(first_owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    chain_config_storage()
        .bls_verifier_accessor()
        .set_checked(&mut harness.sdk, verifier)
        .unwrap();

    harness.set_caller(first_owner);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_REGISTER_VALIDATOR,
                &RegisterValidatorCommand {
                    validator: first_validator,
                    commission_rate: 0,
                    initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                    bls_pubkey_uncompressed: bls_pubkey_uncompressed.clone(),
                    bls_pop_uncompressed: bls_pop_uncompressed.clone(),
                    peer_pubkey: first_peer_pubkey,
                },
            ))
            .0,
        ExitCode::Ok
    );

    harness.set_caller(second_owner);
    assert_revert_selector(
        harness.call(encode_args_call(
            SIG_REGISTER_VALIDATOR,
            &RegisterValidatorCommand {
                validator: second_validator,
                commission_rate: 0,
                initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                bls_pubkey_uncompressed,
                bls_pop_uncompressed,
                peer_pubkey: second_peer_pubkey,
            },
        )),
        ERR_BLS_PUBKEY_ALREADY_IN_USE,
    );

    assert_eq!(
        consensus_storage()
            .bls_pubkey_owner_accessor()
            .entry(bls_pubkey_hash)
            .get_checked(&harness.sdk)
            .unwrap(),
        first_validator
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(second_validator)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_NOT_FOUND
    );
    assert!(staking_storage()
        .owner_validators_accessor()
        .entry(second_owner)
        .get_checked(&harness.sdk)
        .unwrap()
        .is_zero());
    assert!(consensus_storage()
        .peer_pubkey_owner_accessor()
        .entry(second_peer_pubkey)
        .get_checked(&harness.sdk)
        .unwrap()
        .is_zero());
}

#[test]
fn registration_rejects_non_96_byte_compressed_key_without_partial_state() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let verifier = Address::with_last_byte(0xb0);
    let peer_pubkey = B256::with_last_byte(0x11);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    chain_config_storage()
        .bls_verifier_accessor()
        .set_checked(&mut harness.sdk, verifier)
        .unwrap();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            assert_eq!(address, verifier);
            assert_eq!(
                u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap()),
                SIG_BLS_COMPRESS_G2_UNCHECKED
            );
            SyscallResult::new(
                encode_mock_return(&Bytes::from(vec![0x33; BLS_PUBKEY_LENGTH - 1])),
                0,
                0,
                ExitCode::Ok,
            )
        });
    harness.set_caller(owner);

    assert_revert_selector(
        harness.call(encode_args_call(
            SIG_REGISTER_VALIDATOR,
            &RegisterValidatorCommand {
                validator,
                commission_rate: 0,
                initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                bls_pubkey_uncompressed: Bytes::from(vec![0x11; BLS_PUBKEY_UNCOMPRESSED_LENGTH]),
                bls_pop_uncompressed: Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]),
                peer_pubkey,
            },
        )),
        ERR_INVALID_CONSENSUS_KEY_ENCODING,
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
    assert!(consensus_storage()
        .peer_pubkey_owner_accessor()
        .entry(peer_pubkey)
        .get_checked(&harness.sdk)
        .unwrap()
        .is_zero());
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

// Leader weight is drawn from the SELECTION epoch (target - 2), the same vintage
// that ranked membership. Stamping `target` instead yields a contract that is
// internally consistent and still wrong: nothing reverts, the leader is just
// weighted by an epoch the committee was not chosen under.
#[test]
fn leader_weights_are_frozen_at_the_selection_epoch_vintage() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let delegator = Address::with_last_byte(0xb0);
    let initial = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2);
    let added = DEFAULT_MIN_STAKING_AMOUNT * U256::from(5);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![initial], 0),
        ExitCode::Ok
    );

    // Effective at epoch 2, so epoch 0 and epoch 2 hold different stakes.
    staking::delegate_to(&mut harness.sdk, delegator, validator, added, false).unwrap();
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 0).unwrap(),
        initial
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 2).unwrap(),
        initial + added
    );

    harness.set_caller(SYSTEM_CALLER);
    for _ in 0..3 {
        assert_eq!(
            harness
                .call(encode_args_call(
                    SIG_COMMIT_EPOCH_COMMITTEE,
                    &(vec![validator],),
                ))
                .0,
            ExitCode::Ok
        );
    }

    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE_WITH_STAKES,
        &U64Command { value: 2 },
    ));
    let (_, _, stakes): (Vec<Address>, Vec<ConsensusKeys>, Vec<U256>) = decode_returns(&output);
    assert_eq!(
        stakes,
        vec![initial],
        "epoch 2 was selected from epoch 0 and must carry epoch 0's weight"
    );

    staking::delegate_to(&mut harness.sdk, delegator, validator, added, false).unwrap();
    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE_WITH_STAKES,
        &U64Command { value: 2 },
    ));
    let (_, _, stakes): (Vec<Address>, Vec<ConsensusKeys>, Vec<U256>) = decode_returns(&output);
    assert_eq!(
        stakes,
        vec![initial],
        "a committed epoch's weights do not move when stake changes afterwards"
    );
}

#[test]
fn pruning_drops_leader_weights_with_their_committee() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );

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

    harness.set_block_number(1_000 + 60 * DEFAULT_EPOCH_BLOCK_INTERVAL);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_COMMIT_EPOCH_COMMITTEE,
                &(vec![validator],),
            ))
            .0,
        ExitCode::Ok
    );

    let (exit, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE_WITH_STAKES,
        &U64Command { value: 0 },
    ));
    assert_eq!(exit, ExitCode::Ok);
    let (validators, _, stakes): (Vec<Address>, Vec<ConsensusKeys>, Vec<U256>) =
        decode_returns(&output);
    assert!(
        validators.is_empty() && stakes.is_empty(),
        "a pruned epoch answers empty, not a length mismatch"
    );
}

// The committee cap was the last input of the epoch-frozen selection view still
// read live: raising it used to retroactively enlarge the committee of an epoch
// that had already been committed, which desynchronises the DKG index space
// from the committed committee.
#[test]
fn raising_the_cap_leaves_already_started_epochs_untouched() {
    let owner = Address::with_last_byte(0xa0);
    let big = Address::with_last_byte(0x01);
    let small = Address::with_last_byte(0x02);
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(
        owner,
        vec![big, small],
        vec![
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(5),
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2),
        ],
        500,
    );
    command.active_validators_length = 1;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
    assert_eq!(
        staking::selected_validators_at(&harness.sdk, 0).unwrap(),
        vec![big]
    );

    harness.set_caller(GENESIS_GOVERNANCE);
    harness.sdk.take_logs();
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_ACTIVE_VALIDATORS_LENGTH,
                &U32Command { value: 2 },
            ))
            .0,
        ExitCode::Ok
    );
    let logs = harness.sdk.take_logs();
    let (data, _) = logs
        .iter()
        .find(|(_, topics)| {
            topics.first() == Some(&B256::new(events::ActiveValidatorsLengthChanged::SELECTOR))
        })
        .expect("cap change event");
    assert_eq!(
        decode_output::<(u32, u32, u64)>(data),
        (1, 2, 1),
        "the event must announce the epoch the new cap first governs, not the current one"
    );

    assert_eq!(
        staking::selected_validators_at(&harness.sdk, 0).unwrap(),
        vec![big],
        "epoch 0 has already started and keeps the cap it was selected under"
    );
    assert_eq!(
        staking::selected_validators_at(&harness.sdk, 1).unwrap(),
        vec![big, small]
    );

    let (_, output) = harness.call(encode_call(
        SIG_GET_ACTIVE_VALIDATORS_LENGTH_AT,
        &U64Command { value: 0 },
    ));
    assert_eq!(decode_output::<u64>(&output), 1);
    let (_, output) = harness.call(encode_call(
        SIG_GET_ACTIVE_VALIDATORS_LENGTH_AT,
        &U64Command { value: 1 },
    ));
    assert_eq!(decode_output::<u64>(&output), 2);
    let (_, output) = harness.call(encode_empty_call(SIG_GET_ACTIVE_VALIDATORS_LENGTH));
    assert_eq!(
        decode_output::<u64>(&output),
        2,
        "the scalar reports the latest scheduled value immediately"
    );
}

// The other half of the key filter: keys that exist but activate later must be
// treated exactly like absent keys on both legs.
#[test]
fn keys_activating_after_the_selection_epoch_are_filtered_after_the_cut() {
    let owner = Address::with_last_byte(0xa0);
    let future = Address::with_last_byte(0x01);
    let keyed = Address::with_last_byte(0x02);
    let spare = Address::with_last_byte(0x03);
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(
        owner,
        vec![future, keyed, spare],
        vec![
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(9),
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(3),
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2),
        ],
        500,
    );
    command.active_validators_length = 2;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    consensus_storage()
        .consensus_keys_accessor()
        .entry(future)
        .activation_epoch_accessor()
        .set_checked(&mut harness.sdk, 7)
        .unwrap();

    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATORS_WITH_KEYS_AT,
        &U64Command { value: 0 },
    ));
    let (view, view_keys): (Vec<Address>, Vec<ConsensusKeys>) = decode_returns(&output);
    assert_eq!(
        view,
        vec![future, keyed],
        "the not-yet-activated validator still occupies its top-k slot"
    );
    assert!(view_keys[0].bls_pubkey.is_empty());

    harness.set_caller(SYSTEM_CALLER);
    assert_revert_selector(
        harness.call(encode_args_call(
            SIG_COMMIT_EPOCH_COMMITTEE,
            &(vec![keyed, spare],),
        )),
        ERR_COMMITTEE_LENGTH_MISMATCH,
    );
    assert_eq!(
        harness
            .call(encode_args_call(SIG_COMMIT_EPOCH_COMMITTEE, &(vec![keyed],)))
            .0,
        ExitCode::Ok
    );
}

#[test]
fn committee_stake_read_rejects_a_committee_without_matching_frozen_weights() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );

    // A committee whose weights were never stamped: the reader must refuse it
    // rather than fall back to a live, height-dependent walk.
    consensus_storage()
        .epoch_committees_accessor()
        .entry(4)
        .push_checked(&mut harness.sdk, validator)
        .unwrap();

    assert_revert_selector(
        harness.call(encode_call(
            SIG_GET_EPOCH_COMMITTEE_WITH_STAKES,
            &U64Command { value: 4 },
        )),
        ERR_LEADER_STAKES_LENGTH_MISMATCH,
    );
}

#[test]
fn repeated_cap_changes_in_one_epoch_collapse_into_a_single_checkpoint() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );

    harness.set_caller(GENESIS_GOVERNANCE);
    for value in [2u32, 3, 4] {
        assert_eq!(
            harness
                .call(encode_call(
                    SIG_SET_ACTIVE_VALIDATORS_LENGTH,
                    &U32Command { value },
                ))
                .0,
            ExitCode::Ok
        );
    }

    assert_eq!(
        chain_config_storage()
            .cap_checkpoints_accessor()
            .len_checked(&harness.sdk)
            .unwrap(),
        2,
        "the genesis checkpoint plus one pending entry, not one entry per call"
    );
    let (_, output) = harness.call(encode_call(
        SIG_GET_ACTIVE_VALIDATORS_LENGTH_AT,
        &U64Command { value: 1 },
    ));
    assert_eq!(decode_output::<u64>(&output), 4);
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
    // Through governance, not a raw storage poke: committee selection reads the
    // cap checkpoint, and only the setter schedules one.
    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_ACTIVE_VALIDATORS_LENGTH,
                &U32Command { value: 1 },
            ))
            .0,
        ExitCode::Ok
    );

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
    staking_storage()
        .last_rewarded_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, 2)
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
    staking_storage()
        .last_rewarded_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, 3)
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
        vec![0x33, 0x34]
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

// A keyless validator outranking a keyed one by stake occupies a top-k slot and
// is dropped afterwards; it is not skipped over. Ranking before filtering is what
// keeps `getValidatorsWithKeysAt` — the array the off-chain deriver builds from —
// in agreement with the committee `commitEpochCommittee` will accept.
#[test]
fn committee_verify_matches_the_selection_view_the_deriver_reads() {
    let owner = Address::with_last_byte(0xa0);
    let keyless = Address::with_last_byte(0x01);
    let keyed_a = Address::with_last_byte(0x02);
    let keyed_b = Address::with_last_byte(0x03);
    let validators = vec![keyless, keyed_a, keyed_b];
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(
        owner,
        validators.clone(),
        vec![
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(100),
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(3),
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2),
        ],
        500,
    );
    command.active_validators_length = 2;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    let keyless_keys = consensus_storage().consensus_keys_accessor().entry(keyless);
    keyless_keys
        .peer_pubkey_accessor()
        .set_checked(&mut harness.sdk, B256::ZERO)
        .unwrap();

    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATORS_WITH_KEYS_AT,
        &U64Command { value: 0 },
    ));
    let (view, view_keys): (Vec<Address>, Vec<ConsensusKeys>) = decode_returns(&output);
    assert_eq!(
        view,
        vec![keyless, keyed_a],
        "the selection view ranks by stake before any key filtering"
    );
    assert!(
        view_keys[0].bls_pubkey.is_empty(),
        "the keyless top-ranked validator is surfaced with blank keys, not omitted"
    );

    harness.set_caller(SYSTEM_CALLER);
    assert_revert_selector(
        harness.call(encode_args_call(
            SIG_COMMIT_EPOCH_COMMITTEE,
            &(vec![keyed_a, keyed_b],),
        )),
        ERR_COMMITTEE_LENGTH_MISMATCH,
    );
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_COMMIT_EPOCH_COMMITTEE,
                &(vec![keyed_a],),
            ))
            .0,
        ExitCode::Ok
    );
    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE,
        &U64Command { value: 0 },
    ));
    assert_eq!(decode_output::<Vec<Address>>(&output), vec![keyed_a]);
}

#[test]
fn fully_keyless_committee_reverts_without_advancing_commit_pointer() {
    let owner = Address::with_last_byte(0xa0);
    let validators = vec![Address::with_last_byte(0x01), Address::with_last_byte(0x02)];
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(
            owner,
            validators.clone(),
            vec![DEFAULT_MIN_VALIDATOR_STAKE; validators.len()],
            500,
        ),
        ExitCode::Ok
    );
    for validator in &validators {
        consensus_storage()
            .consensus_keys_accessor()
            .entry(*validator)
            .peer_pubkey_accessor()
            .set_checked(&mut harness.sdk, B256::ZERO)
            .unwrap();
    }
    harness.set_caller(SYSTEM_CALLER);
    assert_revert_selector(
        harness.call(encode_args_call(
            SIG_COMMIT_EPOCH_COMMITTEE,
            &(Vec::<Address>::new(),),
        )),
        ERR_COMMITTEE_TOO_SMALL,
    );

    let (_, output) = harness.call(encode_empty_call(SIG_NEXT_EPOCH_TO_COMMIT));
    assert_eq!(decode_output::<u64>(&output), 0);
    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE_LENGTH,
        &U64Command { value: 0 },
    ));
    assert_eq!(decode_output::<U256>(&output), U256::ZERO);

    for (index, validator) in validators.iter().enumerate() {
        let key_byte = (index + 1) as u8;
        store_test_consensus_keys(
            &mut harness.sdk,
            *validator,
            key_byte,
            B256::with_last_byte(key_byte),
            0,
        );
    }
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_COMMIT_EPOCH_COMMITTEE,
                &(validators.clone(),),
            ))
            .0,
        ExitCode::Ok
    );
    let (_, output) = harness.call(encode_empty_call(SIG_NEXT_EPOCH_TO_COMMIT));
    assert_eq!(decode_output::<u64>(&output), 1);
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
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_UNDELEGATE_PERIOD,
            &U32Command { value: 10 },
        )),
        ERR_DPOS_ALREADY_ACTIVE,
    );
}

#[test]
fn dpos_activation_at_block_zero_remains_configurable() {
    let owner = Address::with_last_byte(0xa0);
    let mut harness = Harness::new(0);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    harness.set_caller(GENESIS_GOVERNANCE);
    chain_config_storage()
        .min_undelegate_blocks_accessor()
        .set_checked(&mut harness.sdk, U256::from(1_000))
        .unwrap();
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_EPOCH_BLOCK_INTERVAL,
            &U32Command { value: 100 },
        )),
        ERR_UNDELEGATE_WINDOW_TOO_SHORT,
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_EPOCH_BLOCK_INTERVAL,
                &U32Command { value: 250 },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_DPOS_ACTIVATION_BLOCK,
                &U64Command { value: 500 },
            ))
            .0,
        ExitCode::Ok
    );
}

#[test]
fn undelegate_period_change_does_not_shorten_queued_self_stake_lock() {
    let sponsor = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let mut harness = Harness::new(0);
    assert_eq!(
        harness.initialize(sponsor, vec![validator], vec![stake], 0),
        ExitCode::Ok
    );

    staking::undelegate_from(&mut harness.sdk, validator, validator, stake).unwrap();
    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATOR_SELF_STAKE_LOCK,
        &AddressCommand { value: validator },
    ));
    let before = decode_output::<(bool, u64)>(&output);
    assert!(before.0);

    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_UNDELEGATE_PERIOD,
                &U32Command { value: 1 },
            ))
            .0,
        ExitCode::Ok
    );
    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATOR_SELF_STAKE_LOCK,
        &AddressCommand { value: validator },
    ));
    assert_eq!(decode_output::<(bool, u64)>(&output), before);
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

fn install_solvent_reserve(sdk: &TestingContextImpl, reserve: Address, balance: U256) {
    sdk.set_call_handler(move |address, _value, input, _fuel_limit| {
        if input.len() < SIG_LEN_BYTES {
            return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams);
        }
        let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
        match (address, selector) {
            (address, SIG_RESERVE_BALANCE) if address == reserve => {
                SyscallResult::new(encode_mock_return(&balance), 0, 0, ExitCode::Ok)
            }
            (address, SIG_RESERVE_DISBURSE) if address == reserve => {
                let (_, assigned) =
                    SolidityABI::<(Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0).unwrap();
                SyscallResult::new(encode_mock_return(&assigned), 0, 0, ExitCode::Ok)
            }
            _ => SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic),
        }
    });
}

fn epoch_reward(sdk: &TestingContextImpl, validator: Address, epoch: u64) -> U256 {
    U256::from(
        staking_storage()
            .validator_snapshots_accessor()
            .entry(validator)
            .entry(epoch)
            .total_blend_rewards_accessor()
            .get_checked(sdk)
            .unwrap(),
    )
}

#[test]
fn stipend_pays_the_frozen_weights_not_the_stake_at_settlement_time() {
    let owner = Address::with_last_byte(0xa0);
    let delegator = Address::with_last_byte(0xd0);
    let reserve = Address::with_last_byte(0xc0);
    let validator_a = Address::with_last_byte(0x01);
    let validator_b = Address::with_last_byte(0x02);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let mut harness = Harness::new(1_000);
    let mut command =
        harness.initialize_command(owner, vec![validator_a, validator_b], vec![stake, stake], 0);
    command.blend_reserve = reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    harness.set_caller(SYSTEM_CALLER);
    for _ in 0..3 {
        assert_eq!(
            harness
                .call(encode_args_call(
                    SIG_COMMIT_EPOCH_COMMITTEE,
                    &(vec![validator_a, validator_b],),
                ))
                .0,
            ExitCode::Ok
        );
    }
    // Effective from epoch 2, so a live walk at the settled epoch would weight
    // the committee 4:1 where the epoch-0 freeze weights it 1:1.
    staking::delegate_to(
        &mut harness.sdk,
        delegator,
        validator_a,
        stake * U256::from(3),
        false,
    )
    .unwrap();

    chain_config_storage()
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::from(100))
        .unwrap();
    staking_storage()
        .last_rewarded_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, 2)
        .unwrap();
    install_solvent_reserve(&harness.sdk, reserve, U256::from(100));
    record_test_production(&mut harness.sdk, 2, DEFAULT_EPOCH_BLOCK_INTERVAL as u32);

    harness.set_block_number(1_000 + 3 * DEFAULT_EPOCH_BLOCK_INTERVAL);
    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SETTLE_EPOCH_STIPEND,
                &U64Command { value: 2 },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(epoch_reward(&harness.sdk, validator_a, 2), U256::from(50));
    assert_eq!(epoch_reward(&harness.sdk, validator_b, 2), U256::from(50));
}

// A committee may be committed two epochs ahead, so the weights an unfinished
// epoch would be paid on already exist. Paying it draws a full pot for an epoch
// with no production and advances the cursor past it for good.
#[test]
fn an_epoch_that_has_not_finished_cannot_be_settled() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let reserve = Address::with_last_byte(0xc0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(owner, vec![validator], vec![stake], 0);
    command.blend_reserve = reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    commit_test_committee(&mut harness.sdk, 0, &[(validator, stake)]);
    commit_test_committee(&mut harness.sdk, 1, &[(validator, stake)]);
    chain_config_storage()
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::from(100))
        .unwrap();
    install_solvent_reserve(&harness.sdk, reserve, U256::from(1_000));
    record_test_production(&mut harness.sdk, 0, DEFAULT_EPOCH_BLOCK_INTERVAL as u32);
    record_test_production(&mut harness.sdk, 1, DEFAULT_EPOCH_BLOCK_INTERVAL as u32);

    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SETTLE_EPOCH_STIPEND,
                &U64Command { value: 1 },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(epoch_reward(&harness.sdk, validator, 0), U256::ZERO);
    assert_eq!(epoch_reward(&harness.sdk, validator, 1), U256::ZERO);
    assert_eq!(
        staking_storage()
            .last_rewarded_epoch_p1_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "the cursor must not advance past an epoch that was never paid"
    );

    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL);
    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SETTLE_EPOCH_STIPEND,
                &U64Command { value: 1 },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(epoch_reward(&harness.sdk, validator, 0), U256::from(100));
    assert_eq!(
        epoch_reward(&harness.sdk, validator, 1),
        U256::ZERO,
        "epoch 1 is still running and stays unpaid"
    );
}

// The cursor walks epochs contiguously, so a gap in production — a stalled
// recorder, a pre-activation prefix — is passed over by a LATER epoch's close.
// Without a per-epoch belt each skipped epoch draws a full pot for no blocks.
#[test]
fn an_epoch_that_recorded_no_blocks_is_skipped_when_a_later_one_settles() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let reserve = Address::with_last_byte(0xc0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(owner, vec![validator], vec![stake], 0);
    command.blend_reserve = reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    for epoch in 0..3 {
        commit_test_committee(&mut harness.sdk, epoch, &[(validator, stake)]);
    }
    chain_config_storage()
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::from(100))
        .unwrap();
    install_solvent_reserve(&harness.sdk, reserve, U256::from(1_000));
    // Epochs 0 and 1 recorded nothing; only epoch 2 produced blocks.
    record_test_production(&mut harness.sdk, 2, DEFAULT_EPOCH_BLOCK_INTERVAL as u32);

    harness.set_block_number(1_000 + 3 * DEFAULT_EPOCH_BLOCK_INTERVAL);
    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SETTLE_EPOCH_STIPEND,
                &U64Command { value: 2 },
            ))
            .0,
        ExitCode::Ok
    );

    assert_eq!(epoch_reward(&harness.sdk, validator, 0), U256::ZERO);
    assert_eq!(epoch_reward(&harness.sdk, validator, 1), U256::ZERO);
    assert_eq!(
        epoch_reward(&harness.sdk, validator, 2),
        U256::from(100),
        "only the epoch that recorded blocks is paid"
    );
    assert_eq!(
        staking_storage()
            .credited_blend_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        U256::from(100),
        "the skipped epochs must not have drawn a pot each"
    );
}

// Truncating to the shorter of the two arrays would hand the whole pot to a
// committee prefix and then advance the cursor past the epoch for good, so the
// settle path must refuse a mismatch exactly as the reader does.
#[test]
fn settlement_rejects_a_committee_without_matching_frozen_weights() {
    let owner = Address::with_last_byte(0xa0);
    let seated = Address::with_last_byte(0x01);
    let unweighted = Address::with_last_byte(0x02);
    let reserve = Address::with_last_byte(0xc0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let mut harness = Harness::new(1_000);
    let mut command =
        harness.initialize_command(owner, vec![seated, unweighted], vec![stake, stake], 0);
    command.blend_reserve = reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    // Two committee members, one frozen weight.
    let consensus = consensus_storage();
    let committee = consensus.epoch_committees_accessor().entry(0);
    committee.push_checked(&mut harness.sdk, seated).unwrap();
    committee.push_checked(&mut harness.sdk, unweighted).unwrap();
    consensus
        .leader_stakes_accessor()
        .entry(0)
        .push_checked(&mut harness.sdk, crate::math::compact_balance(stake).unwrap())
        .unwrap();
    chain_config_storage()
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::from(100))
        .unwrap();
    install_solvent_reserve(&harness.sdk, reserve, U256::from(1_000));
    record_test_production(&mut harness.sdk, 0, DEFAULT_EPOCH_BLOCK_INTERVAL as u32);

    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL);
    harness.set_caller(SYSTEM_CALLER);
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SETTLE_EPOCH_STIPEND,
            &U64Command { value: 0 },
        )),
        ERR_LEADER_STAKES_LENGTH_MISMATCH,
    );
    assert_eq!(
        staking_storage()
            .last_rewarded_epoch_p1_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "a refused settlement must not advance the cursor past the epoch"
    );
}

#[test]
fn tombstoned_committee_member_earns_no_stipend_share() {
    let owner = Address::with_last_byte(0xa0);
    let reserve = Address::with_last_byte(0xc0);
    let validator_a = Address::with_last_byte(0x01);
    let validator_b = Address::with_last_byte(0x02);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let mut harness = Harness::new(1_000);
    let mut command =
        harness.initialize_command(owner, vec![validator_a, validator_b], vec![stake, stake], 0);
    command.blend_reserve = reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    commit_test_committee(
        &mut harness.sdk,
        0,
        &[(validator_a, stake), (validator_b, stake)],
    );
    consensus_storage()
        .tombstoned_accessor()
        .entry(validator_a)
        .set_checked(&mut harness.sdk, true)
        .unwrap();
    chain_config_storage()
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::from(100))
        .unwrap();
    install_solvent_reserve(&harness.sdk, reserve, U256::from(100));
    record_test_production(&mut harness.sdk, 0, DEFAULT_EPOCH_BLOCK_INTERVAL as u32);

    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL);
    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SETTLE_EPOCH_STIPEND,
                &U64Command { value: 0 },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(epoch_reward(&harness.sdk, validator_a, 0), U256::ZERO);
    assert_eq!(epoch_reward(&harness.sdk, validator_b, 0), U256::from(100));
    assert_eq!(
        staking_storage()
            .credited_blend_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        U256::from(100)
    );
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
fn permissionless_validator_claim_waits_for_stipend_settlement() {
    let assigned = U256::from(100);
    let attacker = Address::with_last_byte(0xd0);
    let token = Address::with_last_byte(0xf0);
    let (mut harness, _calls, validator) =
        stipend_test_sdk(vec![assigned], vec![MockDisbursement::Amount(assigned)]);
    staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(0)
        .commission_rate_accessor()
        .set_checked(&mut harness.sdk, 1_000)
        .unwrap();
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL);

    harness.set_caller(attacker);
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
        0,
        "an unsettled epoch must remain claimable after a permissionless call"
    );

    harness.set_caller(SYSTEM_CALLER);
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
        staking_storage()
            .last_rewarded_epoch_p1_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1
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
    harness.set_caller(attacker);
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
        transfers.borrow().as_slice(),
        &[(validator, U256::from(10))]
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validator)
            .claimed_at_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1
    );
}

#[test]
fn delayed_reward_settlement_does_not_block_matured_principal() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let delegator = Address::with_last_byte(0x02);
    let token = Address::with_last_byte(0xf0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let reward = DEFAULT_MIN_STAKING_AMOUNT;
    let activation_block = 1_000;
    let mut harness = Harness::new(activation_block);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![stake], 0),
        ExitCode::Ok
    );
    // Keep this regression focused on the independent reward/principal cursors.
    // Validator-owner principal is deliberately subject to the separate
    // self-stake liability deadline covered by the bounded-lock tests.
    staking::delegate_to(&mut harness.sdk, delegator, validator, stake, false).unwrap();
    harness.set_block_number(activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * WARMUP_DELAY);
    staking::undelegate_from(&mut harness.sdk, delegator, validator, stake).unwrap();
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator);

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

    harness.set_caller(delegator);
    let maturity_epoch = WARMUP_DELAY + 1 + DEFAULT_UNDELEGATE_PERIOD;
    harness.set_block_number(activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * maturity_epoch);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CLAIM_DELEGATOR_FEE,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(transfers.borrow().as_slice(), &[(delegator, stake)]);
    assert_eq!(
        delegation
            .delegate_gap_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "principal maturity must not consume the unsettled reward cursor"
    );
    assert_eq!(
        delegation
            .undelegate_gap_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1
    );

    staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(WARMUP_DELAY)
        .total_blend_rewards_accessor()
        .set_checked(
            &mut harness.sdk,
            math::narrow_reward(reward * U256::from(2)).expect("reward fits uint96"),
        )
        .unwrap();
    staking_storage()
        .last_rewarded_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, WARMUP_DELAY + 1)
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
    assert_eq!(
        transfers.borrow().as_slice(),
        &[(delegator, stake), (delegator, reward)]
    );
    assert_eq!(
        delegation
            .delegate_gap_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1
    );
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
    staking_storage()
        .last_rewarded_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, MAX_EPOCHS_PER_CLAIM + 1)
        .unwrap();
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
            SIG_REGISTER_VALIDATOR,
            &RegisterValidatorCommand {
                validator: Address::with_last_byte(2),
                commission_rate: 0,
                initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                bls_pubkey_uncompressed: Bytes::new(),
                bls_pop_uncompressed: Bytes::new(),
                peer_pubkey: B256::ZERO,
            },
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
fn continuously_seated_validator_self_stake_unlocks_at_bounded_liability_deadline() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let token = Address::with_last_byte(0xc0);
    let withdrawn = DEFAULT_MIN_VALIDATOR_STAKE;
    let stake = withdrawn * U256::from(2);
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

    staking::undelegate_from(&mut harness.sdk, validator, validator, withdrawn).unwrap();
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(validator);
    let undelegates = delegation.undelegate_queue_accessor();
    let maturity_epoch = DEFAULT_UNDELEGATE_PERIOD + 1;
    let liability_end_epoch =
        maturity_epoch + EPOCH_COMMITTEE_RETENTION_MARGIN + MAX_COMMITTEE_LOOKAHEAD_EPOCHS;
    assert_eq!(
        undelegates
            .at(0)
            .epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        maturity_epoch
    );
    assert_eq!(
        undelegates
            .at(0)
            .self_stake_unlock_epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        liability_end_epoch
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validator)
            .self_stake_unlock_epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        liability_end_epoch
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

    // Keep committing this still-active validator after its partial self-exit.
    // New committees are secured by the remaining self-stake and must not keep
    // extending the already-queued principal's fixed liability deadline.
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
        harness
            .call(encode_args_call(
                SIG_COMMIT_EPOCH_COMMITTEE,
                &(vec![validator],),
            ))
            .0,
        ExitCode::Ok
    );
    for epoch in 1..liability_end_epoch {
        harness.set_block_number(activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * epoch);
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

        if epoch == maturity_epoch {
            harness.set_caller(validator);
            let (exit, output) = harness.call(encode_call(
                SIG_GET_VALIDATOR_SELF_STAKE_LOCK,
                &AddressCommand { value: validator },
            ));
            assert_eq!(exit, ExitCode::Ok);
            assert_eq!(
                decode_output::<(bool, u64)>(&output),
                (true, liability_end_epoch)
            );
            consensus::ensure_equivocation_evidence_unexpired(&mut harness.sdk, 2).unwrap();
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
        }
    }

    harness.set_block_number(activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * liability_end_epoch);
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validator)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_ACTIVE
    );
    assert!(
        staking::selected_validators_at(&harness.sdk, liability_end_epoch)
            .unwrap()
            .contains(&validator)
    );
    assert_eq!(
        consensus_storage()
            .epoch_committees_accessor()
            .entry(2)
            .len_checked(&harness.sdk)
            .unwrap(),
        1,
        "time-based release must not depend on a later committee commit pruning storage"
    );
    harness.set_caller(validator);
    let (exit, output) = harness.call(encode_call(
        SIG_GET_VALIDATOR_SELF_STAKE_LOCK,
        &AddressCommand { value: validator },
    ));
    assert_eq!(exit, ExitCode::Ok);
    assert_eq!(
        decode_output::<(bool, u64)>(&output),
        (false, liability_end_epoch)
    );
    assert_direct_revert(
        consensus::ensure_equivocation_evidence_unexpired(&mut harness.sdk, 2),
        &harness.sdk,
        ERR_EQUIVOCATION_EVIDENCE_EXPIRED,
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
    assert_eq!(transfers.borrow().as_slice(), &[(validator, withdrawn)]);
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
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validator)
            .self_stake_unlock_epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0
    );
}

#[test]
fn production_liveness_ships_disabled_on_a_fresh_chain() {
    let owner = Address::with_last_byte(0xa0);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    let (exit, output) = harness.call(encode_empty_call(SIG_GET_PRODUCTION_LIVENESS_DISABLED));
    assert_eq!(exit, ExitCode::Ok);
    assert!(
        decode_output::<bool>(&output),
        "an unwritten slot reads false, so the tier would ship ON unless init seeds it"
    );

    for (selector, expected) in [
        (SIG_GET_MIN_VERDICT_DUE_BLOCKS, DEFAULT_MIN_VERDICT_DUE_BLOCKS),
        (SIG_GET_EXCLUSION_BACKOFF_CAP, DEFAULT_EXCLUSION_BACKOFF_CAP),
        (
            SIG_DEFAULT_MIN_VERDICT_DUE_BLOCKS,
            DEFAULT_MIN_VERDICT_DUE_BLOCKS,
        ),
        (
            SIG_DEFAULT_EXCLUSION_BACKOFF_CAP,
            DEFAULT_EXCLUSION_BACKOFF_CAP,
        ),
        (SIG_MAX_MIN_VERDICT_DUE_BLOCKS, MAX_MIN_VERDICT_DUE_BLOCKS),
    ] {
        let (exit, output) = harness.call(encode_empty_call(selector));
        assert_eq!(exit, ExitCode::Ok);
        assert_eq!(decode_output::<u32>(&output), expected);
    }

    let logs = harness.sdk.take_logs();
    let disabled_data = &logs
        .iter()
        .find(|(_, topics)| {
            topics.first() == Some(&B256::new(events::ProductionLivenessDisabledChanged::SELECTOR))
        })
        .expect("kill-switch seed event")
        .0;
    assert_eq!(decode_output::<(bool, bool)>(disabled_data), (false, true));
}

#[test]
fn production_liveness_setters_enforce_their_bounds() {
    let owner = Address::with_last_byte(0xa0);
    let outsider = Address::with_last_byte(0xb0);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    harness.set_caller(outsider);
    for selector in [
        SIG_SET_MIN_VERDICT_DUE_BLOCKS,
        SIG_SET_EXCLUSION_BACKOFF_CAP,
    ] {
        assert_revert_selector(
            harness.call(encode_call(selector, &U32Command { value: 5 })),
            ERR_ONLY_GOVERNANCE,
        );
    }
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_PRODUCTION_LIVENESS_DISABLED,
            &BoolCommand { value: false },
        )),
        ERR_ONLY_GOVERNANCE,
    );

    harness.set_caller(GENESIS_GOVERNANCE);
    for selector in [
        SIG_SET_MIN_VERDICT_DUE_BLOCKS,
        SIG_SET_EXCLUSION_BACKOFF_CAP,
    ] {
        assert_revert_selector(
            harness.call(encode_call(selector, &U32Command { value: 0 })),
            ERR_ZERO_VALUE,
        );
    }
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_MIN_VERDICT_DUE_BLOCKS,
            &U32Command {
                value: MAX_MIN_VERDICT_DUE_BLOCKS + 1,
            },
        )),
        ERR_MIN_VERDICT_DUE_BLOCKS_TOO_HIGH,
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_MIN_VERDICT_DUE_BLOCKS,
                &U32Command {
                    value: MAX_MIN_VERDICT_DUE_BLOCKS,
                },
            ))
            .0,
        ExitCode::Ok,
        "the ceiling is inclusive"
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_EXCLUSION_BACKOFF_CAP,
                &U32Command { value: u32::MAX },
            ))
            .0,
        ExitCode::Ok,
        "the ladder ceiling is bounded away from zero only"
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_PRODUCTION_LIVENESS_DISABLED,
                &BoolCommand { value: false },
            ))
            .0,
        ExitCode::Ok
    );

    let (_, output) = harness.call(encode_empty_call(SIG_GET_MIN_VERDICT_DUE_BLOCKS));
    assert_eq!(decode_output::<u32>(&output), MAX_MIN_VERDICT_DUE_BLOCKS);
    let (_, output) = harness.call(encode_empty_call(SIG_GET_EXCLUSION_BACKOFF_CAP));
    assert_eq!(decode_output::<u32>(&output), u32::MAX);
    let (_, output) = harness.call(encode_empty_call(SIG_GET_PRODUCTION_LIVENESS_DISABLED));
    assert!(!decode_output::<bool>(&output));
}

// A roster member that is selection-visible but below the minimum self-stake can
// never be seated, so it is not a replacement. Counting visibility alone
// over-reports the pool and hands out a stamp whose seat then simply vanishes.
// The Solidity has no such filter, so this gap exists only in the port.
#[test]
fn exclusion_does_not_count_a_validator_that_cannot_be_seated_as_a_replacement() {
    let owner = Address::with_last_byte(0xa0);
    let rich = Address::with_last_byte(0x01);
    let middle = Address::with_last_byte(0x02);
    let poor = Address::with_last_byte(0x03);
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(
        owner,
        vec![rich, middle, poor],
        vec![
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(5),
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(3),
            DEFAULT_MIN_VALIDATOR_STAKE,
        ],
        0,
    );
    command.active_validators_length = 2;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    // `poor` stays visible but drops below the bar, so the eligible pool is 2 —
    // exactly the cap, leaving nothing to replace an excluded member with.
    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_MIN_VALIDATOR_STAKE_AMOUNT,
                &U256Command {
                    value: DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2),
                },
            ))
            .0,
        ExitCode::Ok
    );

    let next = crate::util::next_epoch(&harness.sdk).unwrap();
    assert_eq!(
        staking::selected_validators_at(&harness.sdk, next).unwrap(),
        vec![rich, middle],
        "the pool that can actually be seated is already at the cap"
    );

    harness.sdk.take_logs();
    assert!(
        !staking::apply_production_exclusion(&mut harness.sdk, middle).unwrap(),
        "an unseatable roster member is not a replacement"
    );
    assert!(harness.sdk.take_logs().is_empty());
    assert_eq!(
        staking::selected_validators_at(&harness.sdk, next).unwrap(),
        vec![rich, middle],
        "the refused stamp must not have shrunk the committee"
    );
}

#[test]
fn production_exclusion_refuses_without_a_replacement_and_leaves_no_trace() {
    let owner = Address::with_last_byte(0xa0);
    let first = Address::with_last_byte(0x01);
    let second = Address::with_last_byte(0x02);
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(
        owner,
        vec![first, second],
        vec![
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(3),
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2),
        ],
        0,
    );
    command.active_validators_length = 2;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
    harness.sdk.take_logs();

    assert!(
        !staking::apply_production_exclusion(&mut harness.sdk, second).unwrap(),
        "the visible pool only equals the cap, so excluding a member shrinks the committee"
    );
    assert!(staking::selection_visible_at(&harness.sdk, second, 1).unwrap());
    assert_eq!(
        staking::selected_validators_at(&harness.sdk, 1).unwrap(),
        vec![first, second]
    );
    assert!(
        harness.sdk.take_logs().is_empty(),
        "a refused stamp must leave no trace at all"
    );
}

#[test]
fn production_exclusion_bites_at_the_next_epoch_and_not_before() {
    let owner = Address::with_last_byte(0xa0);
    let first = Address::with_last_byte(0x01);
    let second = Address::with_last_byte(0x02);
    let third = Address::with_last_byte(0x03);
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(
        owner,
        vec![first, second, third],
        vec![
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(3),
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2),
            DEFAULT_MIN_VALIDATOR_STAKE,
        ],
        0,
    );
    command.active_validators_length = 2;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
    harness.sdk.take_logs();

    assert!(staking::apply_production_exclusion(&mut harness.sdk, second).unwrap());
    assert!(
        staking::selection_visible_at(&harness.sdk, second, 0).unwrap(),
        "epoch 0 has already started and its selection view must not be rewritten"
    );
    assert!(!staking::selection_visible_at(&harness.sdk, second, 1).unwrap());
    assert_eq!(
        staking::selected_validators_at(&harness.sdk, 0).unwrap(),
        vec![first, second]
    );
    assert_eq!(
        staking::selected_validators_at(&harness.sdk, 1).unwrap(),
        vec![first, third],
        "the freed seat is taken by the replacement the refusal rule required"
    );

    let logs = harness.sdk.take_logs();
    let (data, topics) = logs
        .iter()
        .find(|(_, topics)| {
            topics.first() == Some(&B256::new(events::ProductionExclusionApplied::SELECTOR))
        })
        .expect("exclusion event");
    assert_eq!(&topics[1].0[12..], second.as_slice());
    assert_eq!(decode_output::<u64>(data), 1);

    harness.sdk.take_logs();
    assert!(
        !staking::apply_production_exclusion(&mut harness.sdk, second).unwrap(),
        "a member already invisible at the bite epoch is refused rather than re-stamped"
    );
    assert!(
        harness.sdk.take_logs().is_empty(),
        "a repeat stamp leaves no trace either"
    );
}

#[test]
fn exclusion_release_skips_tombstoned_and_non_active_validators() {
    let owner = Address::with_last_byte(0xa0);
    let keeper = Address::with_last_byte(0x01);
    let tombstoned = Address::with_last_byte(0x02);
    let demoted = Address::with_last_byte(0x03);
    let healthy = Address::with_last_byte(0x04);
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(
        owner,
        vec![keeper, tombstoned, demoted, healthy],
        vec![DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4); 4],
        0,
    );
    command.active_validators_length = 1;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    for validator in [tombstoned, demoted, healthy] {
        assert!(staking::apply_production_exclusion(&mut harness.sdk, validator).unwrap());
    }
    consensus_storage()
        .tombstoned_accessor()
        .entry(tombstoned)
        .set_checked(&mut harness.sdk, true)
        .unwrap();
    staking_storage()
        .validators_accessor()
        .entry(demoted)
        .status_accessor()
        .set_checked(&mut harness.sdk, STATUS_PENDING)
        .unwrap();
    harness.sdk.take_logs();

    for validator in [tombstoned, demoted] {
        staking::release_production_exclusion(&mut harness.sdk, validator).unwrap();
        assert!(
            !staking::selection_visible_at(&harness.sdk, validator, 1).unwrap(),
            "a blind re-stamp would re-seat a validator the selection filter has no other check for"
        );
    }
    assert!(
        harness.sdk.take_logs().is_empty(),
        "a skipped release must not announce one"
    );

    staking::release_production_exclusion(&mut harness.sdk, healthy).unwrap();
    assert!(staking::selection_visible_at(&harness.sdk, healthy, 1).unwrap());
    let logs = harness.sdk.take_logs();
    let (data, topics) = logs
        .iter()
        .find(|(_, topics)| {
            topics.first() == Some(&B256::new(events::ProductionExclusionReleased::SELECTOR))
        })
        .expect("release event");
    assert_eq!(&topics[1].0[12..], healthy.as_slice());
    assert_eq!(decode_output::<u64>(data), 1);
}

#[test]
fn governance_activation_does_not_cancel_a_running_exclusion() {
    let owner = Address::with_last_byte(0xa0);
    let keeper = Address::with_last_byte(0x01);
    let subject = Address::with_last_byte(0x02);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(
            owner,
            vec![keeper, subject],
            vec![
                DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2),
                DEFAULT_MIN_VALIDATOR_STAKE,
            ],
            0,
        ),
        ExitCode::Ok
    );

    let readmit = production_liveness_storage()
        .validators_accessor()
        .entry(subject)
        .readmit_at_epoch_accessor();
    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_DISABLE_VALIDATOR,
                &AddressCommand { value: subject },
            ))
            .0,
        ExitCode::Ok
    );
    readmit.set_checked(&mut harness.sdk, 5).unwrap();
    assert_eq!(
        harness
            .call(encode_call(
                SIG_ACTIVATE_VALIDATOR,
                &AddressCommand { value: subject },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(subject)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_ACTIVE
    );
    assert!(
        !staking::selection_visible_at(&harness.sdk, subject, 1).unwrap(),
        "re-activation must not re-stamp visibility while an exclusion is running"
    );
    assert_eq!(
        staking::selected_validators_at(&harness.sdk, 1).unwrap(),
        vec![keeper]
    );

    assert_eq!(
        harness
            .call(encode_call(
                SIG_DISABLE_VALIDATOR,
                &AddressCommand { value: subject },
            ))
            .0,
        ExitCode::Ok
    );
    readmit.set_checked(&mut harness.sdk, 0).unwrap();
    assert_eq!(
        harness
            .call(encode_call(
                SIG_ACTIVATE_VALIDATOR,
                &AddressCommand { value: subject },
            ))
            .0,
        ExitCode::Ok
    );
    assert!(
        staking::selection_visible_at(&harness.sdk, subject, 1).unwrap(),
        "with no exclusion recorded the activation stamp is unchanged"
    );
}

#[test]
fn production_liveness_views_read_the_new_namespace() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let stranger = Address::with_last_byte(0x0f);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(
            owner,
            vec![validator],
            vec![DEFAULT_MIN_VALIDATOR_STAKE],
            0
        ),
        ExitCode::Ok
    );
    commit_test_committee(&mut harness.sdk, 4, &[(validator, DEFAULT_MIN_VALIDATOR_STAKE)]);

    let storage = production_liveness_storage();
    storage
        .last_processed_block_accessor()
        .set_checked(&mut harness.sdk, 1_234)
        .unwrap();
    storage
        .blocks_in_epoch_accessor()
        .entry(4u64)
        .set_checked(&mut harness.sdk, 200)
        .unwrap();
    storage
        .produced_accessor()
        .entry(4u64)
        .entry(0u32)
        .set_checked(&mut harness.sdk, 7)
        .unwrap();
    storage
        .pending_exclusions_accessor()
        .push_checked(&mut harness.sdk, validator)
        .unwrap();
    let record = storage.validators_accessor().entry(validator);
    record
        .total_produced_accessor()
        .set_checked(&mut harness.sdk, 99)
        .unwrap();
    record
        .last_produced_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, 5)
        .unwrap();
    record
        .last_failed_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, 3)
        .unwrap();
    record
        .readmit_at_epoch_accessor()
        .set_checked(&mut harness.sdk, 9)
        .unwrap();
    record
        .kick_count_accessor()
        .set_checked(&mut harness.sdk, 2)
        .unwrap();

    let (exit, output) = harness.call(encode_empty_call(SIG_LAST_PROCESSED_BLOCK));
    assert_eq!(exit, ExitCode::Ok);
    assert_eq!(decode_output::<u64>(&output), 1_234);

    let (exit, output) = harness.call(encode_call(SIG_BLOCKS_IN_EPOCH, &U64Command { value: 4 }));
    assert_eq!(exit, ExitCode::Ok);
    assert_eq!(decode_output::<u32>(&output), 200);

    let (exit, output) = harness.call(encode_call(
        SIG_PRODUCED_AT,
        &EpochSignerCommand {
            epoch: 4,
            signer_idx: 0,
        },
    ));
    assert_eq!(exit, ExitCode::Ok);
    assert_eq!(decode_output::<u32>(&output), 7);

    let (exit, output) = harness.call(encode_empty_call(SIG_PENDING_EXCLUSIONS));
    assert_eq!(exit, ExitCode::Ok);
    assert_eq!(decode_output::<Vec<Address>>(&output), vec![validator]);

    let (exit, output) = harness.call(encode_call(
        SIG_READMIT_AT_EPOCH,
        &AddressCommand { value: validator },
    ));
    assert_eq!(exit, ExitCode::Ok);
    assert_eq!(decode_output::<u64>(&output), 9);

    let (exit, output) = harness.call(encode_call(
        SIG_GET_PRODUCTION_STATS,
        &ValidatorEpochCommand {
            validator,
            before_epoch: 4,
        },
    ));
    assert_eq!(exit, ExitCode::Ok);
    assert_eq!(
        decode_output::<(u32, u64, u64, u64, u32, u64)>(&output),
        (7, 99, 4, 2, 2, 9)
    );

    let (exit, output) = harness.call(encode_call(
        SIG_GET_PRODUCTION_STATS,
        &ValidatorEpochCommand {
            validator: stranger,
            before_epoch: 4,
        },
    ));
    assert_eq!(exit, ExitCode::Ok);
    assert_eq!(
        decode_output::<(u32, u64, u64, u64, u32, u64)>(&output),
        (0, 0, 0, 0, 0, 0),
        "a non-member has no committee index and must not alias index 0"
    );
}

/// Boots the tier with one active validator per stake, the committee cap set to
/// `cap`, the kill switch off and the system caller installed.
fn liveness_harness(stakes: &[U256], cap: u32) -> (Harness, Vec<Address>) {
    let owner = Address::with_last_byte(0xa0);
    let members: Vec<Address> = (1..=stakes.len())
        .map(|index| Address::with_last_byte(index as u8))
        .collect();
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(owner, members.clone(), stakes.to_vec(), 0);
    command.active_validators_length = cap;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_PRODUCTION_LIVENESS_DISABLED,
                &BoolCommand { value: false },
            ))
            .0,
        ExitCode::Ok
    );
    harness.set_caller(SYSTEM_CALLER);
    harness.sdk.take_logs();
    (harness, members)
}

fn record_production(harness: &mut Harness, block_number: u64, leader_index: u8) -> ExitCode {
    harness.set_caller(SYSTEM_CALLER);
    harness
        .call(encode_call(
            SIG_RECORD_PRODUCTION,
            &RecordProductionCommand {
                block_number,
                leader_index,
            },
        ))
        .0
}

fn seed_epoch_production(
    sdk: &mut TestingContextImpl,
    epoch: u64,
    produced: &[u32],
    recorded: u32,
) {
    let storage = production_liveness_storage();
    for (index, count) in produced.iter().enumerate() {
        storage
            .produced_accessor()
            .entry(epoch)
            .entry(index as u32)
            .set_checked(sdk, *count)
            .unwrap();
    }
    storage
        .blocks_in_epoch_accessor()
        .entry(epoch)
        .set_checked(sdk, recorded)
        .unwrap();
}

/// Drives the close of `epoch` by recording the first block of `epoch + 1`.
fn close_epoch_via_record(harness: &mut Harness, epoch: u64) -> ExitCode {
    let boundary = 1_000 + (epoch + 1) * DEFAULT_EPOCH_BLOCK_INTERVAL;
    production_liveness_storage()
        .last_processed_block_accessor()
        .set_checked(&mut harness.sdk, boundary - 1)
        .unwrap();
    harness.set_block_number(boundary);
    record_production(harness, boundary, 0)
}

fn production_record(
    sdk: &TestingContextImpl,
    validator: Address,
) -> (u64, u64, u64, u64, u32) {
    let record = production_liveness_storage()
        .validators_accessor()
        .entry(validator);
    (
        record.total_produced_accessor().get_checked(sdk).unwrap(),
        record
            .last_produced_epoch_p1_accessor()
            .get_checked(sdk)
            .unwrap(),
        record
            .last_failed_epoch_p1_accessor()
            .get_checked(sdk)
            .unwrap(),
        record.readmit_at_epoch_accessor().get_checked(sdk).unwrap(),
        record.kick_count_accessor().get_checked(sdk).unwrap(),
    )
}

fn pending_exclusion_set(sdk: &TestingContextImpl) -> Vec<Address> {
    let entries = production_liveness_storage().pending_exclusions_accessor();
    (0..entries.len_checked(sdk).unwrap())
        .map(|index| entries.at(index).get_checked(sdk).unwrap())
        .collect()
}

fn logs_of(logs: &[(Bytes, Vec<B256>)], selector: [u8; 32]) -> Vec<(Bytes, Vec<B256>)> {
    logs.iter()
        .filter(|(_, topics)| topics.first() == Some(&B256::new(selector)))
        .cloned()
        .collect()
}

// A repeated block number must not be counted twice, and the epoch cursor must
// come from the block already stored rather than from the one arriving: reading
// it after the overwrite collapses `previous_epoch` onto `epoch` and the close
// never runs.
#[test]
fn record_production_belt_holds_and_the_epoch_cursor_precedes_the_overwrite() {
    let (mut harness, members) = liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE], 21);
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[(members[0], DEFAULT_MIN_VALIDATOR_STAKE)],
    );

    assert_eq!(record_production(&mut harness, 1_000, 0), ExitCode::Ok);
    assert_eq!(record_production(&mut harness, 1_000, 0), ExitCode::Ok);
    assert_eq!(record_production(&mut harness, 999, 0), ExitCode::Ok);

    let storage = production_liveness_storage();
    assert_eq!(
        storage
            .blocks_in_epoch_accessor()
            .entry(0u64)
            .get_checked(&harness.sdk)
            .unwrap(),
        1,
        "a replayed block number must be counted exactly once"
    );
    assert_eq!(production_record(&harness.sdk, members[0]).0, 1);
    assert_eq!(
        storage
            .last_processed_block_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1_000
    );

    harness.set_block_number(1_200);
    harness.sdk.take_logs();
    assert_eq!(record_production(&mut harness, 1_200, 0), ExitCode::Ok);
    let logs = harness.sdk.take_logs();
    let partial = logs_of(&logs, events::PartialEpoch::SELECTOR);
    assert_eq!(
        partial.len(),
        1,
        "crossing into epoch 1 must close epoch 0, which the stored cursor identifies"
    );
    assert_eq!(
        SolidityABI::<u64>::decode(&partial[0].1[1].as_slice(), 0).unwrap(),
        0
    );
    assert_eq!(
        decode_output::<(u32, u32)>(&partial[0].0),
        (1, DEFAULT_EPOCH_BLOCK_INTERVAL as u32)
    );
}

// The close names the epoch that ended, and the block that triggered it belongs
// to the epoch that started: closing `epoch` instead of `previous_epoch` reports
// an epoch nothing has recorded yet.
#[test]
fn the_close_reports_the_epoch_that_ended_not_the_one_the_block_starts() {
    let (mut harness, members) = liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE], 21);
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[(members[0], DEFAULT_MIN_VALIDATOR_STAKE)],
    );
    commit_test_committee(
        &mut harness.sdk,
        1,
        &[(members[0], DEFAULT_MIN_VALIDATOR_STAKE)],
    );
    seed_epoch_production(&mut harness.sdk, 0, &[7], 7);

    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);

    let storage = production_liveness_storage();
    assert_eq!(
        storage
            .blocks_in_epoch_accessor()
            .entry(0u64)
            .get_checked(&harness.sdk)
            .unwrap(),
        7,
        "the boundary block must not land in the epoch being closed"
    );
    assert_eq!(
        storage
            .blocks_in_epoch_accessor()
            .entry(1u64)
            .get_checked(&harness.sdk)
            .unwrap(),
        1
    );
    let logs = harness.sdk.take_logs();
    let partial = logs_of(&logs, events::PartialEpoch::SELECTOR);
    assert_eq!(partial.len(), 1);
    assert_eq!(
        SolidityABI::<u64>::decode(&partial[0].1[1].as_slice(), 0).unwrap(),
        0
    );
    assert_eq!(
        decode_output::<(u32, u32)>(&partial[0].0).0,
        7,
        "the taint must be measured before the boundary block is credited"
    );
}

// An uncommitted epoch parks the block: nothing counted, nothing credited, and
// no revert — a revert here is a per-block system call failing, i.e. a halt.
#[test]
fn an_uncommitted_committee_parks_the_block_instead_of_reverting() {
    let (mut harness, members) = liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE], 21);

    assert_eq!(record_production(&mut harness, 1_000, 0), ExitCode::Ok);
    let storage = production_liveness_storage();
    assert_eq!(
        storage
            .last_processed_block_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1_000,
        "the idempotency belt still advances over a parked block"
    );
    assert_eq!(
        storage
            .blocks_in_epoch_accessor()
            .entry(0u64)
            .get_checked(&harness.sdk)
            .unwrap(),
        0
    );
    assert_eq!(
        storage
            .produced_accessor()
            .entry(0u64)
            .entry(0u32)
            .get_checked(&harness.sdk)
            .unwrap(),
        0
    );
    assert_eq!(production_record(&harness.sdk, members[0]), (0, 0, 0, 0, 0));

    harness.set_block_number(1_200);
    harness.sdk.take_logs();
    assert_eq!(record_production(&mut harness, 1_200, 0), ExitCode::Ok);
    let logs = harness.sdk.take_logs();
    assert_eq!(
        decode_output::<(u32, u32)>(&logs_of(&logs, events::PartialEpoch::SELECTOR)[0].0),
        (0, DEFAULT_EPOCH_BLOCK_INTERVAL as u32),
        "a fully parked epoch is tainted rather than silently complete"
    );
    assert!(
        logs_of(&logs, events::EpochBlendRewardsCommitted::SELECTOR).is_empty(),
        "an epoch with no recorded block must not draw a pot"
    );
}

fn set_min_verdict_due_blocks(harness: &mut Harness, value: u32) {
    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_MIN_VERDICT_DUE_BLOCKS,
                &U32Command { value },
            ))
            .0,
        ExitCode::Ok
    );
    harness.set_caller(SYSTEM_CALLER);
}

fn equal_weight_committee(sdk: &mut TestingContextImpl, epoch: u64, members: &[Address]) {
    let weights: Vec<(Address, U256)> = members
        .iter()
        .map(|member| (*member, DEFAULT_MIN_VALIDATOR_STAKE))
        .collect();
    commit_test_committee(sdk, epoch, &weights);
}

// The taint is derived from the block count, so the identical verdict input
// judges nobody at 199 blocks and judges normally at 200. Anything that drops a
// record therefore disables the tier for a whole epoch while it reads as on.
#[test]
fn a_partial_epoch_suppresses_judging_entirely() {
    let (mut harness, members) =
        liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2); 4], 2);
    set_min_verdict_due_blocks(&mut harness, 10);
    equal_weight_committee(&mut harness.sdk, 0, &members);
    equal_weight_committee(&mut harness.sdk, 1, &members);

    let produced = [100, 100, 0, 0];
    seed_epoch_production(&mut harness.sdk, 0, &produced, 199);
    harness.sdk.take_logs();
    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);

    let logs = harness.sdk.take_logs();
    assert_eq!(logs_of(&logs, events::PartialEpoch::SELECTOR).len(), 1);
    assert!(
        logs_of(&logs, events::ProductionVerdictFailed::SELECTOR).is_empty(),
        "one missing record must cost the whole epoch its verdicts"
    );
    for member in &members {
        assert_eq!(production_record(&harness.sdk, *member).2, 0);
    }
    assert!(pending_exclusion_set(&harness.sdk).is_empty());

    seed_epoch_production(&mut harness.sdk, 1, &produced, 200);
    harness.sdk.take_logs();
    assert_eq!(close_epoch_via_record(&mut harness, 1), ExitCode::Ok);
    let logs = harness.sdk.take_logs();
    assert!(logs_of(&logs, events::PartialEpoch::SELECTOR).is_empty());
    assert_eq!(
        logs_of(&logs, events::ProductionVerdictFailed::SELECTOR).len(),
        2,
        "the same production judges normally once the epoch is complete"
    );
}

// Both predicates are cross-multiplied against the frozen weights: a member
// whose due share falls under the floor holds no verdict at all, and a member
// producing exactly half its due passes.
// The kill switch has two halves. Freezing releases is covered elsewhere; this
// covers the other half, which is only reachable on a COMPLETE epoch — a partial
// one takes the taint arm and never reaches judging at all.
#[test]
fn the_kill_switch_also_suppresses_verdicts() {
    let token = DEFAULT_MIN_VALIDATOR_STAKE;
    let (mut harness, members) = liveness_harness(&[token * U256::from(50); 4], 2);
    set_min_verdict_due_blocks(&mut harness, 10);
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[
            (members[0], token * U256::from(49)),
            (members[1], token * U256::from(25)),
            (members[2], token * U256::from(25)),
            (members[3], token),
        ],
    );
    seed_epoch_production(&mut harness.sdk, 0, &[151, 24, 25, 0], 200);

    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_PRODUCTION_LIVENESS_DISABLED,
                &BoolCommand { value: true },
            ))
            .0,
        ExitCode::Ok
    );
    harness.sdk.take_logs();

    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);

    let logs = harness.sdk.take_logs();
    assert!(
        logs_of(&logs, events::ProductionVerdictFailed::SELECTOR).is_empty(),
        "a complete epoch must still produce no verdict while the tier is off"
    );
    assert_eq!(
        production_record(&harness.sdk, members[1]).2,
        0,
        "and no failure is recorded against the member that would have failed"
    );
}

#[test]
fn verdicts_come_from_the_frozen_weights_and_are_never_divided() {
    let token = DEFAULT_MIN_VALIDATOR_STAKE;
    let (mut harness, members) = liveness_harness(&[token * U256::from(50); 4], 2);
    set_min_verdict_due_blocks(&mut harness, 10);
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[
            (members[0], token * U256::from(49)),
            (members[1], token * U256::from(25)),
            (members[2], token * U256::from(25)),
            (members[3], token),
        ],
    );
    // due = 98 / 50 / 50 / 2 blocks out of 200 recorded, floor 10.
    seed_epoch_production(&mut harness.sdk, 0, &[151, 24, 25, 0], 200);
    harness.sdk.take_logs();

    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);

    let logs = harness.sdk.take_logs();
    let failed = logs_of(&logs, events::ProductionVerdictFailed::SELECTOR);
    assert_eq!(failed.len(), 1, "exactly one member is under half its due");
    assert_eq!(&failed[0].1[2].0[12..], members[1].as_slice());
    assert_eq!(
        decode_output::<(u32, U256)>(&failed[0].0),
        (24, U256::from(50))
    );
    assert_eq!(production_record(&harness.sdk, members[1]).2, 1);
    assert_eq!(
        production_record(&harness.sdk, members[2]).2,
        0,
        "producing exactly half the due share passes"
    );
    assert_eq!(
        production_record(&harness.sdk, members[3]).2,
        0,
        "a member due fewer blocks than the floor holds no verdict at zero production"
    );
    assert_eq!(pending_exclusion_set(&harness.sdk), vec![members[1]]);
}

// More than `f` FIRST-TIME failures in one epoch reads as an environment and
// stamps nobody; the same members failing again are no longer new, so the tier
// answers them. At epoch 0 the never-failed sentinel and "failed epoch −1"
// collide, and a bare equality reads a first-ever failure as a repeat.
#[test]
fn the_correlation_guard_keys_on_new_failures_and_frees_the_next_epoch() {
    let (mut harness, members) =
        liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2); 7], 2);
    set_min_verdict_due_blocks(&mut harness, 10);
    for epoch in 0..2 {
        equal_weight_committee(&mut harness.sdk, epoch, &members);
    }
    let produced = [50, 50, 50, 50, 0, 0, 0];

    seed_epoch_production(&mut harness.sdk, 0, &produced, 200);
    harness.sdk.take_logs();
    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);

    let logs = harness.sdk.take_logs();
    let correlated = logs_of(&logs, events::CorrelatedFailureEpoch::SELECTOR);
    assert_eq!(correlated.len(), 1);
    assert_eq!(
        decode_output::<(U256, U256)>(&correlated[0].0),
        (U256::from(3), U256::from(2))
    );
    assert!(
        pending_exclusion_set(&harness.sdk).is_empty(),
        "an environment is not answered by excluding its victims"
    );
    for member in &members[4..] {
        let record = production_record(&harness.sdk, *member);
        assert_eq!(record.2, 1, "the failure bit is written on the guarded path");
        assert_eq!(record.4, 0);
    }

    seed_epoch_production(&mut harness.sdk, 1, &produced, 200);
    harness.sdk.take_logs();
    assert_eq!(close_epoch_via_record(&mut harness, 1), ExitCode::Ok);

    let logs = harness.sdk.take_logs();
    assert!(
        logs_of(&logs, events::CorrelatedFailureEpoch::SELECTOR).is_empty(),
        "a repeat failure is evidence of itself, not of an environment"
    );
    assert_eq!(
        pending_exclusion_set(&harness.sdk),
        vec![members[4], members[5]]
    );
}

// Two stamps per close at most, and never more than `f` concurrent.
#[test]
fn stamps_are_bounded_per_close_and_by_the_concurrent_budget() {
    let (mut harness, members) =
        liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2); 10], 2);
    set_min_verdict_due_blocks(&mut harness, 10);
    for epoch in 0..3 {
        equal_weight_committee(&mut harness.sdk, epoch, &members);
    }
    // A ladder already four episodes deep, so a stamp outlives the next close
    // and the concurrent budget is what stops the third one.
    for member in &members {
        production_liveness_storage()
            .validators_accessor()
            .entry(*member)
            .kick_count_accessor()
            .set_checked(&mut harness.sdk, 4)
            .unwrap();
    }

    seed_epoch_production(
        &mut harness.sdk,
        0,
        &[29, 29, 29, 29, 28, 28, 28, 0, 0, 0],
        200,
    );
    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);
    assert_eq!(
        pending_exclusion_set(&harness.sdk),
        vec![members[7], members[8]],
        "three failers, two stamps: the per-close cap, ordered by address"
    );

    seed_epoch_production(
        &mut harness.sdk,
        1,
        &[34, 33, 33, 33, 33, 34, 0, 0, 0, 0],
        200,
    );
    assert_eq!(close_epoch_via_record(&mut harness, 1), ExitCode::Ok);
    assert_eq!(
        pending_exclusion_set(&harness.sdk),
        vec![members[7], members[8], members[6]],
        "four failers, one stamp: `f` concurrent exclusions is the ceiling"
    );
    assert_eq!(production_record(&harness.sdk, members[9]).3, 0);

    seed_epoch_production(
        &mut harness.sdk,
        2,
        &[34, 33, 33, 33, 33, 34, 0, 0, 0, 0],
        200,
    );
    assert_eq!(close_epoch_via_record(&mut harness, 2), ExitCode::Ok);
    assert_eq!(pending_exclusion_set(&harness.sdk).len(), 3);
}

// Releases are frozen with the rest of the verdict state under the kill switch,
// so no exclusion expires unnoticed while the tier is off.
#[test]
fn the_kill_switch_freezes_releases() {
    let (mut harness, members) =
        liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2); 3], 1);
    let excluded = members[2];
    assert!(staking::apply_production_exclusion(&mut harness.sdk, excluded).unwrap());
    let record = production_liveness_storage()
        .validators_accessor()
        .entry(excluded);
    record
        .readmit_at_epoch_accessor()
        .set_checked(&mut harness.sdk, 1)
        .unwrap();
    production_liveness_storage()
        .pending_exclusions_accessor()
        .push_checked(&mut harness.sdk, excluded)
        .unwrap();

    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_PRODUCTION_LIVENESS_DISABLED,
                &BoolCommand { value: true },
            ))
            .0,
        ExitCode::Ok
    );
    harness.sdk.take_logs();
    assert_eq!(close_epoch_via_record(&mut harness, 1), ExitCode::Ok);

    assert_eq!(pending_exclusion_set(&harness.sdk), vec![excluded]);
    assert_eq!(record.readmit_at_epoch_accessor().get_checked(&harness.sdk).unwrap(), 1);
    assert!(!staking::selection_visible_at(&harness.sdk, excluded, 3).unwrap());
    assert!(harness
        .sdk
        .take_logs()
        .iter()
        .all(|(_, topics)| topics.first()
            != Some(&B256::new(events::ProductionExclusionReleased::SELECTOR))));

    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_PRODUCTION_LIVENESS_DISABLED,
                &BoolCommand { value: false },
            ))
            .0,
        ExitCode::Ok
    );
    harness.sdk.take_logs();
    assert_eq!(close_epoch_via_record(&mut harness, 2), ExitCode::Ok);

    assert!(pending_exclusion_set(&harness.sdk).is_empty());
    assert_eq!(record.readmit_at_epoch_accessor().get_checked(&harness.sdk).unwrap(), 0);
    assert!(staking::selection_visible_at(&harness.sdk, excluded, 4).unwrap());
}

struct CloseCallState {
    reserve_balances: VecDeque<Option<U256>>,
    disbursed: Vec<U256>,
    self_call_fuel: Option<u64>,
    self_calls: usize,
}

fn reserve_reply(input: &[u8], state: &Rc<RefCell<CloseCallState>>) -> SyscallResult<Bytes> {
    let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
    match selector {
        SIG_RESERVE_BALANCE => match state.borrow_mut().reserve_balances.pop_front() {
            Some(Some(balance)) => {
                SyscallResult::new(encode_mock_return(&balance), 0, 0, ExitCode::Ok)
            }
            _ => SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic),
        },
        SIG_RESERVE_DISBURSE => {
            let params = &input[SIG_LEN_BYTES..];
            let (_, amount) = SolidityABI::<(Address, U256)>::decode(&params, 0).unwrap();
            state.borrow_mut().disbursed.push(amount);
            SyscallResult::new(encode_mock_return(&amount), 0, 0, ExitCode::Ok)
        }
        _ => SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic),
    }
}

/// Mocks the stipend leg's fuel-capped self-call.
///
/// Everything about the nested frame is real — same storage, same dispatch,
/// caller rewritten to the contract itself, failure returned as a status. Only
/// the journal is emulated: this host has no checkpoint, so storage is
/// snapshotted before the re-entry and restored when the frame fails, which is
/// what `checkpoint_revert` does on the real one.
fn install_close_call_handler(harness: &Harness, state: Rc<RefCell<CloseCallState>>) {
    let host = harness.sdk.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, fuel_limit| {
            if address != GENESIS_STAKING {
                return reserve_reply(input, &state);
            }
            {
                let mut observed = state.borrow_mut();
                observed.self_calls += 1;
                observed.self_call_fuel = fuel_limit;
            }
            let snapshot = host.dump_storage();
            let outer_caller = host.context().contract_caller();
            host.context_mut().caller = GENESIS_STAKING;
            let mut nested = host.clone().with_input(Bytes::from(input.to_vec()));
            let inner = state.clone();
            nested.set_call_handler(move |_address, _value, input, _fuel| {
                reserve_reply(input, &inner)
            });
            let outcome = main_entry(&mut nested);
            nested.context_mut().caller = outer_caller;
            let data = Bytes::from(nested.take_output());
            match outcome {
                Ok(()) => SyscallResult::new(data, 0, 0, ExitCode::Ok),
                Err(status) => {
                    nested.restore_storage(snapshot);
                    SyscallResult::new(data, 0, 0, status)
                }
            }
        });
}

// The tolerant leg. A stipend that dies mid-catch-up discards its own frame and
// nothing else: the release and the verdict of the same close survive, the
// reward cursor does not advance, and the outer frame reports the failure with
// an event of its own — a log written inside the discarded frame would go with
// it, and a system call leaves no receipt to read instead.
#[test]
fn a_failing_stipend_leg_leaves_the_releases_and_verdicts_of_its_close_intact() {
    let token = DEFAULT_MIN_VALIDATOR_STAKE;
    let (mut harness, members) = liveness_harness(&[token * U256::from(2); 4], 2);
    set_min_verdict_due_blocks(&mut harness, 10);
    equal_weight_committee(&mut harness.sdk, 0, &members);
    equal_weight_committee(&mut harness.sdk, 1, &members);

    let reserve = Address::with_last_byte(0xc0);
    let config = chain_config_storage();
    config
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::from(400))
        .unwrap();
    config
        .blend_reserve_accessor()
        .set_checked(&mut harness.sdk, reserve)
        .unwrap();

    // Leg 1 has an exclusion expiring at this close.
    let releasing = members[3];
    assert!(staking::apply_production_exclusion(&mut harness.sdk, releasing).unwrap());
    let releasing_record = production_liveness_storage()
        .validators_accessor()
        .entry(releasing);
    releasing_record
        .readmit_at_epoch_accessor()
        .set_checked(&mut harness.sdk, 2)
        .unwrap();
    releasing_record
        .last_failed_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, 1)
        .unwrap();
    production_liveness_storage()
        .pending_exclusions_accessor()
        .push_checked(&mut harness.sdk, releasing)
        .unwrap();

    // Leg 2 has one member under half its due.
    seed_epoch_production(&mut harness.sdk, 0, &[50, 50, 50, 50], 200);
    seed_epoch_production(&mut harness.sdk, 1, &[67, 67, 0, 66], 200);

    let state = Rc::new(RefCell::new(CloseCallState {
        // Epoch 0 settles; epoch 1 dies inside the same self-call, so the
        // discarded frame is one that had already written.
        reserve_balances: vec![Some(U256::from(1_000)), None].into(),
        disbursed: Vec::new(),
        self_call_fuel: None,
        self_calls: 0,
    }));
    install_close_call_handler(&harness, state.clone());
    harness.sdk.take_logs();

    assert_eq!(
        close_epoch_via_record(&mut harness, 1),
        ExitCode::Ok,
        "a failing stipend must not take the per-block system call down with it"
    );

    assert_eq!(state.borrow().self_calls, 1);
    assert_eq!(
        state.borrow().self_call_fuel,
        Some(12_000_000 * 20),
        "the cap is a fuel figure, not the Solidity gas figure"
    );
    assert_eq!(
        state.borrow().disbursed,
        vec![U256::from(400)],
        "the mock records the call; on the real runtime the reserve's state change is discarded with the frame for epoch 0"
    );

    let logs = harness.sdk.take_logs();
    let skipped = logs_of(&logs, events::StipendLegSkipped::SELECTOR);
    assert_eq!(skipped.len(), 1);
    assert_eq!(
        SolidityABI::<u64>::decode(&skipped[0].1[1].as_slice(), 0).unwrap(),
        1
    );

    assert_eq!(
        production_record(&harness.sdk, releasing).3,
        0,
        "leg 1's release survives the stipend failure"
    );
    assert!(staking::selection_visible_at(&harness.sdk, releasing, 3).unwrap());
    assert_eq!(
        pending_exclusion_set(&harness.sdk),
        vec![members[2]],
        "leg 2's stamp survives it too"
    );
    let stamped = production_record(&harness.sdk, members[2]);
    assert_eq!((stamped.2, stamped.3, stamped.4), (2, 3, 1));

    let staking_state = staking_storage();
    assert_eq!(
        staking_state
            .last_rewarded_epoch_p1_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "the reward cursor did not advance, so the next close retries contiguously"
    );
    assert_eq!(
        staking_state
            .credited_blend_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        U256::ZERO
    );
    assert_eq!(
        staking_state
            .validator_snapshots_accessor()
            .entry(members[0])
            .entry(0u64)
            .total_blend_rewards_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        crate::math::U96::ZERO,
        "the epoch-0 credit was inside the discarded frame"
    );
}

#[test]
fn the_stipend_re_entry_is_reachable_only_from_the_contract_itself() {
    let (mut harness, _) = liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE], 21);
    let calldata = encode_call(SIG_SETTLE_EPOCH_STIPEND_FROM, &U64Command { value: 0 });
    harness.set_caller(SYSTEM_CALLER);
    assert_revert_selector(harness.call(calldata.clone()), ERR_ONLY_SELF_CALL);
    harness.set_caller(GENESIS_GOVERNANCE);
    assert_revert_selector(harness.call(calldata.clone()), ERR_ONLY_SELF_CALL);
    harness.set_caller(GENESIS_STAKING);
    assert_eq!(harness.call(calldata).0, ExitCode::Ok);
}
