use super::*;
use crate::{
    consts::{STATUS_ACTIVE, STATUS_JAIL, STATUS_PENDING},
    storage::{
        chain_config_storage, consensus_storage, initializer_storage, production_liveness_storage,
        staking_storage, CapCheckpointStorage, ConsensusKeysStorage, DelegationOpStorage,
        EpochCommitteeMemberStorage, ProductionValidatorStorage, UndelegationOpStorage,
        ValidatorSnapshotStorage, ValidatorStorage,
    },
    types::{
        AddressAmountCommand, AddressCommand, AddressU16Command, BoolCommand, ConsensusKeys,
        EpochSignerCommand, EquivocationCommand, InitializeCommand, RecordProductionCommand,
        RegisterValidatorCommand, TwoAddressesCommand, U256Command, U32Command, U64Command,
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
use std::{cell::RefCell, rc::Rc};

/// The staking token every `initialize_command` seeds, and therefore the one the
/// stipend pull goes through.
const STAKING_TOKEN: Address = Address::with_last_byte(0xf0);

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
    assert_eq!(<UndelegationOpStorage as StorageLayout>::BYTES, 22);
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
    assert_eq!(
        validator.first_snapshot_epoch_p1_accessor().slot(),
        slot + U256::from(1)
    );
    assert_eq!(validator.first_snapshot_epoch_p1_accessor().offset(), 16);

    assert_eq!(CapCheckpointStorage::SLOTS, 1);
    assert_eq!(<CapCheckpointStorage as StorageLayout>::BYTES, 12);

    // The committee entry replaced two parallel vectors of one slot each, so at
    // two slots the merge is storage-neutral — it buys the impossibility of a
    // misalignment, not a smaller footprint. It does not fit in one slot:
    // 20 bytes of address plus 14 of `uint112` is 34.
    assert_eq!(EpochCommitteeMemberStorage::SLOTS, 2);
    assert_eq!(<EpochCommitteeMemberStorage as StorageLayout>::BYTES, 34);
    let member = EpochCommitteeMemberStorage::new(slot, 0);
    assert_eq!(member.validator_accessor().slot(), slot);
    assert_eq!(member.weight_accessor().slot(), slot + U256::from(1));

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
            match mock_external_return(selector, &input[SIG_LEN_BYTES..]) {
                Some(output) => SyscallResult::new(output, 0, 0, ExitCode::Ok),
                None => SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams),
            }
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
            staking_token: STAKING_TOKEN,
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
            min_undelegate_blocks: U256::ZERO,
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

/// Writes an epoch committee with its frozen leader weights, the pair
/// `commitEpochCommittee` appends and the stipend reads back.
fn commit_test_committee(sdk: &mut TestingContextImpl, epoch: u64, members: &[(Address, U256)]) {
    let committee = consensus_storage().epoch_committees_accessor().entry(epoch);
    for (validator, stake) in members {
        let entry = committee.grow_checked(sdk).unwrap();
        entry
            .validator_accessor()
            .set_checked(sdk, *validator)
            .unwrap();
        entry
            .weight_accessor()
            .set_checked(sdk, crate::math::compact_balance(*stake).unwrap())
            .unwrap();
    }
}

/// The validators a fixture names, plus filler to reach `MIN_COMMITTEE_LENGTH`.
///
/// A commit refuses a committee below the minimum, so a fixture that cares about
/// one or two validators still has to seat enough of them for the commit to be
/// legal.
///
/// Filler must not displace a named validator from a capped committee, and the
/// reason is NOT that the selection sort is stable — it is a selection sort with
/// swaps (`staking.rs`, `top_k_by_stake_at`) and does reorder equals. The reason
/// is that every filler carries `DEFAULT_MIN_VALIDATOR_STAKE`, the floor, so a
/// filler can never be the strictly-greater element a swap targets. Named
/// validators therefore only ever swap with each other and the filler stays in
/// the tail.
///
/// Filler addresses start at `0xe0` deliberately: `0xf0` is `STAKING_TOKEN` and
/// `0xf2` is the default `blend_reserve`, so filling from `0xf0` seats the
/// stipend source as a committee member.
fn with_filler_validators(named: &[(Address, U256)]) -> (Vec<Address>, Vec<U256>) {
    let mut validators: Vec<Address> = named.iter().map(|(validator, _)| *validator).collect();
    let mut stakes: Vec<U256> = named.iter().map(|(_, stake)| *stake).collect();
    for index in 0..MIN_COMMITTEE_LENGTH.saturating_sub(named.len()) {
        validators.push(Address::with_last_byte(0xe0 + index as u8));
        stakes.push(DEFAULT_MIN_VALIDATOR_STAKE);
    }
    (validators, stakes)
}

fn record_test_production(sdk: &mut TestingContextImpl, epoch: u64, blocks: u32) {
    production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(epoch)
        .set_checked(sdk, blocks)
        .unwrap();
    pin_test_stipend_rate(sdk, epoch);
}

/// Pin the stipend rate the way `close_epoch` does.
///
/// Seeding the counters without the rate leaves settlement unable to price the
/// epoch, which the contract now rejects. Pinning inside the seeding helpers
/// rather than at their call sites is what keeps the stand-in from drifting away
/// from the production path again — the callers must only make sure the rate is
/// configured *before* they seed.
fn pin_test_stipend_rate(sdk: &mut TestingContextImpl, epoch: u64) {
    let rate = chain_config_storage()
        .blend_stipend_per_epoch_accessor()
        .get_checked(sdk)
        .unwrap();
    production_liveness_storage()
        .stipend_rate_at_close_p1_accessor()
        .entry(epoch)
        .set_checked(sdk, rate + U256::ONE)
        .unwrap();
}

/// The stipend's funding source as the BLEND token sees it.
///
/// Settlement no longer talks to a reserve contract; it pulls with `transferFrom`
/// off an address that has approved the staking contract. So the two levers a
/// test has are the two an operator has: how much the source holds, and how much
/// of it this contract may take.
struct StipendFunding {
    source: Address,
    balance: U256,
    allowance: U256,
    /// A non-conforming token that answers `false` instead of reverting. The
    /// contract must treat that as a failure too, or an epoch would be credited
    /// against a transfer that never moved anything.
    reports_failure: bool,
    /// Every pull the contract attempted, as `(recipient, amount)`, whether or
    /// not the source could cover it.
    pulls: Vec<(Address, U256)>,
}

fn encode_mock_return<T>(value: &T) -> Bytes
where
    T: fluentbase_sdk::codec::Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    let mut output = BytesMut::new();
    SolidityABI::<T>::encode(value, &mut output, 0).unwrap();
    output.freeze().into()
}

/// The BLS verifier and the staking token as every harness sees them, so a test
/// that needs its own handler can record the call and still answer it here.
///
/// `compressG2Unchecked` derives the compressed key from the first byte of the
/// uncompressed one: a validator registered with `0x11`-filled bytes therefore
/// owns the `0x33`-filled compressed key that `bls_pubkey_owner` is keyed on.
/// `compressG1Unchecked` keeps the leading 48 bytes, which is what lets a test
/// feed the slash path the exact signatures a corpus evidence blob carries.
fn mock_external_return(selector: u32, args: &[u8]) -> Option<Bytes> {
    match selector {
        SIG_BLS_COMPRESS_G2_UNCHECKED => {
            let (uncompressed,) = SolidityABI::<(Bytes,)>::decode_function_args(&args).unwrap();
            let compressed = uncompressed[0].wrapping_add(0x22);
            Some(encode_mock_return(&Bytes::from(vec![
                compressed;
                BLS_PUBKEY_LENGTH
            ])))
        }
        SIG_BLS_COMPRESS_G1_UNCHECKED => {
            let (uncompressed,) = SolidityABI::<(Bytes,)>::decode_function_args(&args).unwrap();
            Some(encode_mock_return(
                &uncompressed.slice(..BLS_SIGNATURE_LENGTH),
            ))
        }
        SIG_BLS_VERIFY => Some(encode_mock_return(&true)),
        SIG_ERC20_TRANSFER_FROM | SIG_ERC20_TRANSFER => Some(encode_mock_return(&true)),
        _ => None,
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

/// Installs the BLEND token behind the stipend.
///
/// `transferFrom` succeeds only while the source both holds the amount and has
/// approved this contract for it; anything else fails the call. That failure is
/// what defers an epoch instead of forfeiting it, so the mock must never answer
/// a short pull with a smaller success.
fn install_stipend_token(
    sdk: &TestingContextImpl,
    source: Address,
    balance: U256,
    allowance: U256,
) -> Rc<RefCell<StipendFunding>> {
    let funding = Rc::new(RefCell::new(StipendFunding {
        source,
        balance,
        allowance,
        reports_failure: false,
        pulls: Vec::new(),
    }));
    let state = funding.clone();
    sdk.set_call_handler(move |address, _value, input, _fuel_limit| {
        if input.len() < SIG_LEN_BYTES {
            return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams);
        }
        let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
        if address != STAKING_TOKEN || selector != SIG_ERC20_TRANSFER_FROM {
            return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic);
        }
        let (from, recipient, amount) =
            SolidityABI::<(Address, Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0).unwrap();
        let mut funding = state.borrow_mut();
        funding.pulls.push((recipient, amount));
        if funding.reports_failure {
            return SyscallResult::new(encode_mock_return(&false), 0, 0, ExitCode::Ok);
        }
        if from != funding.source || amount > funding.balance || amount > funding.allowance {
            return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic);
        }
        funding.balance -= amount;
        funding.allowance -= amount;
        SyscallResult::new(encode_mock_return(&true), 0, 0, ExitCode::Ok)
    });
    funding
}

fn stipend_test_sdk(
    balance: U256,
    allowance: U256,
) -> (Harness, Rc<RefCell<StipendFunding>>, Address) {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let source = Address::with_last_byte(0xc0);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );

    let config = chain_config_storage();
    config
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::from(100))
        .unwrap();
    config
        .blend_reserve_accessor()
        .set_checked(&mut harness.sdk, source)
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

    let funding = install_stipend_token(&harness.sdk, source, balance, allowance);
    (harness, funding, validator)
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
fn register_validator_cast_calldata_registers_consensus_keys_atomically() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let verifier = Address::with_last_byte(0xb0);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    chain_config_storage()
        .bls_verifier_accessor()
        .set_checked(&mut harness.sdk, verifier)
        .unwrap();

    let calls = Rc::new(RefCell::new(Vec::new()));
    let recorded_calls = calls.clone();
    let pulls = Rc::new(RefCell::new(Vec::new()));
    let recorded_pulls = pulls.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
            if selector == SIG_ERC20_TRANSFER_FROM {
                let (from, to, amount) =
                    decode_output::<(Address, Address, U256)>(&input[SIG_LEN_BYTES..]);
                recorded_pulls
                    .borrow_mut()
                    .push((address, from, to, amount));
                return SyscallResult::new(encode_mock_return(&true), 0, 0, ExitCode::Ok);
            }
            assert_eq!(address, verifier);
            recorded_calls.borrow_mut().push(input.to_vec());
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
    // "registerValidator(address,uint16,uint256,bytes,bytes,bytes32)"
    // 0x...01 0 1000000000000000000 0x{11 * 256} 0x{22 * 128} 0x...01
    let calldata = hex!(
        "8d6067ed
         0000000000000000000000000000000000000000000000000000000000000001
         0000000000000000000000000000000000000000000000000000000000000000
         0000000000000000000000000000000000000000000000000de0b6b3a7640000
         00000000000000000000000000000000000000000000000000000000000000c0
         00000000000000000000000000000000000000000000000000000000000001e0
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
    assert_eq!(
        *pulls.borrow(),
        vec![(
            STAKING_TOKEN,
            owner,
            GENESIS_STAKING,
            DEFAULT_MIN_VALIDATOR_STAKE
        )]
    );

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

    // Three elements, not one: a single-element array puts every head slot at offset 0, so it
    // cannot catch a wrong stride between slots.
    let keys = vec![
        ConsensusKeys {
            bls_pubkey: Bytes::from_static(&[0xaa, 0xbb, 0xcc]),
            peer_pubkey: B256::with_last_byte(0x01),
            activation_epoch: 7,
        },
        ConsensusKeys {
            bls_pubkey: Bytes::from_static(&[0xdd, 0xee]),
            peer_pubkey: B256::with_last_byte(0x02),
            activation_epoch: 8,
        },
        ConsensusKeys {
            bls_pubkey: Bytes::from_static(&[0xff]),
            peer_pubkey: B256::with_last_byte(0x03),
            activation_epoch: 9,
        },
    ];
    let mut encoded_keys = BytesMut::new();
    SolidityABI::<Vec<ConsensusKeys>>::encode(&keys, &mut encoded_keys, 0).unwrap();
    // cast abi-encode "f((bytes,bytes32,uint64)[])"
    // "[(0xaabbcc,0x...01,7),(0xddee,0x...02,8),(0xff,0x...03,9)]"
    assert_eq!(
        encoded_keys.as_ref(),
        &hex!(
            "0000000000000000000000000000000000000000000000000000000000000020
             0000000000000000000000000000000000000000000000000000000000000003
             0000000000000000000000000000000000000000000000000000000000000060
             0000000000000000000000000000000000000000000000000000000000000100
             00000000000000000000000000000000000000000000000000000000000001a0
             0000000000000000000000000000000000000000000000000000000000000060
             0000000000000000000000000000000000000000000000000000000000000001
             0000000000000000000000000000000000000000000000000000000000000007
             0000000000000000000000000000000000000000000000000000000000000003
             aabbcc0000000000000000000000000000000000000000000000000000000000
             0000000000000000000000000000000000000000000000000000000000000060
             0000000000000000000000000000000000000000000000000000000000000002
             0000000000000000000000000000000000000000000000000000000000000008
             0000000000000000000000000000000000000000000000000000000000000002
             ddee000000000000000000000000000000000000000000000000000000000000
             0000000000000000000000000000000000000000000000000000000000000060
             0000000000000000000000000000000000000000000000000000000000000003
             0000000000000000000000000000000000000000000000000000000000000009
             0000000000000000000000000000000000000000000000000000000000000001
             ff00000000000000000000000000000000000000000000000000000000000000"
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
    command.blend_reserve = Address::ZERO;
    let (_, output) = harness.call(encode_args_call(SIG_INITIALIZE, &command));
    assert_eq!(&output[..4], &ERR_ZERO_VALUE.to_be_bytes());
    assert_eq!(decode_output::<String>(&output[4..]), "blendReserve");

    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    harness.set_caller(GENESIS_GOVERNANCE);
    for (selector, field) in [
        (SIG_SET_BLS_VERIFIER, "blsVerifier"),
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
        (SIG_INITIALIZE, 0xdfa8efb0),
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
        (SIG_GET_BLEND_RESERVE, 0x37dff538),
        (SIG_SET_BLEND_RESERVE, 0x7899ae8f),
        (SIG_GET_VALIDATOR_FEE, 0x457179fd),
        (SIG_GET_PENDING_VALIDATOR_FEE, 0xc6fb9065),
        (SIG_CLAIM_VALIDATOR_FEE_AT_EPOCH, 0xadf2a79c),
        (SIG_GET_DELEGATOR_FEE, 0x52b7bea2),
        (SIG_CLAIM_DELEGATOR_FEE_AT_EPOCH, 0xfe38ebef),
        (SIG_CALC_AVAILABLE_FOR_REDELEGATE_AMOUNT, 0x5ef9e8c6),
        (SIG_SETTLE_EPOCH_STIPEND, 0xa631344a),
        (SIG_GET_VALIDATORS_WITH_KEYS_AT, 0x7cfba9f3),
        (SIG_COMMIT_EPOCH_COMMITTEE, 0xe505b249),
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

    harness.set_caller(validator);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_REGISTER_VALIDATOR,
                &RegisterValidatorCommand {
                    validator,
                    commission_rate: 0,
                    initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                    bls_pubkey_uncompressed: Bytes::from(vec![
                        0x11;
                        BLS_PUBKEY_UNCOMPRESSED_LENGTH
                    ]),
                    bls_pop_uncompressed: Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]),
                    peer_pubkey: B256::with_last_byte(1),
                },
            ))
            .0,
        ExitCode::Ok
    );
    let record = staking_storage().validators_accessor().entry(validator);
    assert_eq!(
        record.status_accessor().get_checked(&harness.sdk).unwrap(),
        STATUS_PENDING,
        "registration cannot start a validator active"
    );
    let (_, output) = harness.call(encode_call(
        SIG_IS_VALIDATOR_ACTIVE,
        &AddressCommand { value: validator },
    ));
    assert!(!decode_output::<bool>(&output));

    harness.set_caller(outsider);
    let (exit, _) = harness.call(encode_call(
        SIG_ACTIVATE_VALIDATOR,
        &AddressCommand { value: validator },
    ));
    assert_eq!(exit, ExitCode::Panic);

    harness.set_caller(GENESIS_GOVERNANCE);
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

// Registration is the only place a sub-minimum bond is refused: the activation
// backstop was removed once the bar was pinned to the moment the money is taken.
#[test]
fn register_validator_rejects_a_subminimum_bond() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    chain_config_storage()
        .bls_verifier_accessor()
        .set_checked(&mut harness.sdk, Address::with_last_byte(0xb0))
        .unwrap();
    harness.set_caller(validator);

    assert_revert_selector(
        harness.call(encode_args_call(
            SIG_REGISTER_VALIDATOR,
            &RegisterValidatorCommand {
                validator,
                commission_rate: 0,
                initial_stake: DEFAULT_MIN_VALIDATOR_STAKE - BALANCE_COMPACT_PRECISION,
                bls_pubkey_uncompressed: Bytes::from(vec![0x11; BLS_PUBKEY_UNCOMPRESSED_LENGTH]),
                bls_pop_uncompressed: Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]),
                peer_pubkey: B256::with_last_byte(1),
            },
        )),
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

// `minValidatorStakeAmount` binds where the bond is taken, not at activation. A
// registrant who paid the bar in force when he registered stays activatable
// after governance raises it; the raise governs the next registration, not the
// money already locked.
#[test]
fn a_raised_validator_minimum_does_not_block_activating_an_earlier_registrant() {
    let owner = Address::with_last_byte(0xa0);
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

    harness.set_caller(validator);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_REGISTER_VALIDATOR,
                &RegisterValidatorCommand {
                    validator,
                    commission_rate: 0,
                    initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                    bls_pubkey_uncompressed: Bytes::from(vec![
                        0x11;
                        BLS_PUBKEY_UNCOMPRESSED_LENGTH
                    ]),
                    bls_pop_uncompressed: Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]),
                    peer_pubkey: B256::with_last_byte(1),
                },
            ))
            .0,
        ExitCode::Ok
    );

    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_MIN_VALIDATOR_STAKE_AMOUNT,
                &U256Command {
                    value: DEFAULT_MIN_VALIDATOR_STAKE * U256::from(10),
                },
            ))
            .0,
        ExitCode::Ok
    );
    let (_, raised) = harness.call(encode_empty_call(SIG_GET_MIN_VALIDATOR_STAKE_AMOUNT));
    assert_eq!(
        decode_output::<U256>(&raised),
        DEFAULT_MIN_VALIDATOR_STAKE * U256::from(10)
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
        staking_storage()
            .validators_accessor()
            .entry(validator)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_ACTIVE
    );
}

// A full owner exit leaves the validator pending, the same status a fresh
// registration carries, so nothing in the status alone separates a paid-up
// registrant from one who has taken his bond back and withdrawn it.
#[test]
fn an_owner_who_withdrew_his_whole_bond_cannot_be_activated() {
    let sponsor = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    chain_config_storage()
        .bls_verifier_accessor()
        .set_checked(&mut harness.sdk, Address::with_last_byte(0xb0))
        .unwrap();

    harness.set_caller(validator);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_REGISTER_VALIDATOR,
                &RegisterValidatorCommand {
                    validator,
                    commission_rate: 0,
                    initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                    bls_pubkey_uncompressed: Bytes::from(vec![
                        0x11;
                        BLS_PUBKEY_UNCOMPRESSED_LENGTH
                    ]),
                    bls_pop_uncompressed: Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]),
                    peer_pubkey: B256::with_last_byte(1),
                },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_UNDELEGATE,
                &AddressAmountCommand {
                    validator,
                    amount: DEFAULT_MIN_VALIDATOR_STAKE,
                },
            ))
            .0,
        ExitCode::Ok
    );

    // The exit books at epoch 1 and the principal matures `undelegatePeriod`
    // epochs later, so epoch 8 is the first one that can withdraw it. Block 2_600
    // is epoch 8, which makes 9 the activation epoch the guard reads.
    harness.set_block_number(2_600);
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
        staking::delegated_amount_at(&harness.sdk, validator, validator, 9).unwrap(),
        math::U112::ZERO
    );

    harness.set_caller(GENESIS_GOVERNANCE);
    assert_revert_selector(
        harness.call(encode_call(
            SIG_ACTIVATE_VALIDATOR,
            &AddressCommand { value: validator },
        )),
        ERR_ZERO_OWNER_SELF_STAKE,
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(validator)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_PENDING
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
    let blend_reserve = Address::with_last_byte(0xc5);
    let replacement_reserve = Address::with_last_byte(0xd5);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    let mut command = harness.initialize_command(owner, Vec::new(), Vec::new(), 0);
    command.dpos_activation_block = 2_000;
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
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_BLEND_RESERVE,
            &AddressCommand {
                value: Address::with_last_byte(0xee),
            },
        )),
        ERR_ONLY_GOVERNANCE,
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
        (SIG_GET_BLEND_RESERVE, replacement_reserve),
    ] {
        let (exit, output) = harness.call(encode_empty_call(selector));
        assert_eq!(exit, ExitCode::Ok);
        assert_eq!(decode_output::<Address>(&output), expected);
    }
    let (_, output) = harness.call(encode_empty_call(SIG_GET_BLEND_STIPEND_PER_EPOCH));
    assert_eq!(decode_output::<U256>(&output), U256::from(42));

    let logs = harness.sdk.take_logs();
    let data = &logs
        .iter()
        .find(|(_, topics)| {
            topics.first() == Some(&B256::new(events::BlendReserveChanged::SELECTOR))
        })
        .expect("dependency change event")
        .0;
    assert_eq!(
        decode_output::<(Address, Address)>(data),
        (blend_reserve, replacement_reserve)
    );
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

    assert_eq!(
        events::BlendReserveChanged::SIGNATURE,
        "BlendReserveChanged(address,address)"
    );
    assert_eq!(
        events::BlendReserveChanged::SELECTOR,
        hex!("58bf6b15bd5404c0ab55a8db9e88ce5c154feb8c288266925ea26421253a6390")
    );
    let data = &logs
        .iter()
        .find(|(_, topics)| {
            topics.first() == Some(&B256::new(events::BlendReserveChanged::SELECTOR))
        })
        .expect("initial dependency event")
        .0;
    assert_eq!(
        decode_output::<(Address, Address)>(data),
        (Address::ZERO, command.blend_reserve)
    );
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
    let token = STAKING_TOKEN;
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
    let (validators, stakes) = with_filler_validators(&[(validator, initial)]);
    assert_eq!(
        harness.initialize(owner, validators, stakes, 0),
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
                .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
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
        stakes[0], initial,
        "epoch 2 was selected from epoch 0 and must carry epoch 0's weight"
    );

    staking::delegate_to(&mut harness.sdk, delegator, validator, added, false).unwrap();
    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE_WITH_STAKES,
        &U64Command { value: 2 },
    ));
    let (_, _, stakes): (Vec<Address>, Vec<ConsensusKeys>, Vec<U256>) = decode_returns(&output);
    assert_eq!(
        stakes[0], initial,
        "a committed epoch's weights do not move when stake changes afterwards"
    );
}

#[test]
fn pruning_drops_leader_weights_with_their_committee() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    let (validators, stakes) = with_filler_validators(&[(validator, DEFAULT_MIN_VALIDATOR_STAKE)]);
    assert_eq!(
        harness.initialize(owner, validators, stakes, 0),
        ExitCode::Ok
    );

    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );

    // Pruning is bounded by the stipend cursor, so epoch 0 has to be settled
    // before it can be retired.
    staking_storage()
        .last_rewarded_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, 1)
        .unwrap();
    harness.set_block_number(1_000 + 60 * DEFAULT_EPOCH_BLOCK_INTERVAL);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
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

// Deferring a stipend only postpones the loss unless pruning is held behind the
// settlement cursor too. Retire the committee of an unsettled epoch and
// settlement finds it empty, credits nothing and steps over the epoch — the same
// money gone, just quietly. The length-mismatch guard cannot catch that: members
// and weights are cleared together, so it compares zero against zero.
#[test]
fn pruning_stops_at_the_settlement_cursor() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    let (validators, stakes) = with_filler_validators(&[(validator, DEFAULT_MIN_VALIDATOR_STAKE)]);
    assert_eq!(
        harness.initialize(owner, validators, stakes, 0),
        ExitCode::Ok
    );

    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );

    // Sixty epochs on, far past epoch 0's liability deadline, with the stipend
    // still stalled on epoch 0.
    harness.set_block_number(1_000 + 60 * DEFAULT_EPOCH_BLOCK_INTERVAL);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );

    let consensus = consensus_storage();
    assert_eq!(
        consensus
            .pruned_up_to_p1_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0
    );
    assert_eq!(
        consensus
            .epoch_committees_accessor()
            .entry(0)
            .len_checked(&harness.sdk)
            .unwrap() as usize,
        MIN_COMMITTEE_LENGTH,
        "an unsettled epoch keeps the committee its stipend still has to read"
    );

    // Settling epoch 0 releases it, and the next commit retires it.
    staking_storage()
        .last_rewarded_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, 1)
        .unwrap();
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        consensus
            .pruned_up_to_p1_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1,
        "the bound is the cursor, not a blanket stop: settlement releases epoch 0 \
         and only epoch 0"
    );
    assert_eq!(
        consensus
            .epoch_committees_accessor()
            .entry(0)
            .len_checked(&harness.sdk)
            .unwrap(),
        0
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
        staking::selected_addresses_at(&harness.sdk, 0).unwrap()[0],
        big
    );

    harness.set_caller(GENESIS_GOVERNANCE);
    harness.sdk.take_logs();
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_ACTIVE_VALIDATORS_LENGTH,
                &U32Command {
                    value: MIN_COMMITTEE_LENGTH as u32,
                },
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
        (1, MIN_COMMITTEE_LENGTH as u32, 1),
        "the event must announce the epoch the new cap first governs, not the current one"
    );

    assert_eq!(
        staking::selected_addresses_at(&harness.sdk, 0).unwrap(),
        vec![big],
        "epoch 0 has already started and keeps the cap it was selected under"
    );
    assert_eq!(
        staking::selected_addresses_at(&harness.sdk, 1).unwrap(),
        vec![big, small],
        "the next epoch is governed by the raised cap"
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
    assert_eq!(decode_output::<u64>(&output), MIN_COMMITTEE_LENGTH as u64);
    let (_, output) = harness.call(encode_empty_call(SIG_GET_ACTIVE_VALIDATORS_LENGTH));
    assert_eq!(
        decode_output::<u64>(&output),
        MIN_COMMITTEE_LENGTH as u64,
        "the scalar reports the latest scheduled value immediately"
    );
}

// The other half of the key filter: keys that exist but activate later must be
// treated exactly like absent keys on both legs.
#[test]
fn keys_activating_after_the_selection_epoch_are_filtered_after_the_cut() {
    let owner = Address::with_last_byte(0xa0);
    let future = Address::with_last_byte(0x01);
    let keyed: Vec<Address> = (2..=MIN_COMMITTEE_LENGTH + 1)
        .map(|index| Address::with_last_byte(index as u8))
        .collect();
    let spare = Address::with_last_byte(0xee);
    let mut harness = Harness::new(1_000);
    // The cap seats `future` plus every `keyed` member and leaves `spare` below
    // the cut. Dropping `future` for its late activation must leave exactly the
    // committee floor — not reach past the cut for `spare`.
    let cap = MIN_COMMITTEE_LENGTH + 1;
    let mut validators = vec![future];
    validators.extend(keyed.iter().copied());
    validators.push(spare);
    let mut stakes = vec![DEFAULT_MIN_VALIDATOR_STAKE * U256::from(9)];
    stakes.extend(
        keyed
            .iter()
            .map(|_| DEFAULT_MIN_VALIDATOR_STAKE * U256::from(3)),
    );
    stakes.push(DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2));
    let mut command = harness.initialize_command(owner, validators, stakes, 500);
    command.active_validators_length = cap as u32;
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
    assert_eq!(view.len(), cap);
    assert_eq!(
        view[0], future,
        "the not-yet-activated validator still occupies its top-k slot"
    );
    assert!(
        !view.contains(&spare),
        "the below-the-cut validator is not seated"
    );
    assert!(view_keys[0].bls_pubkey.is_empty());

    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );
    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE,
        &U64Command { value: 0 },
    ));
    // `spare` is never promoted into the slot the not-yet-activated validator
    // vacates: the key filter runs after the stake cut, so the committee shrinks
    // rather than reaching further down the ranking.
    assert_eq!(decode_output::<Vec<Address>>(&output), keyed);
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
    for value in [
        MIN_COMMITTEE_LENGTH as u32,
        MIN_COMMITTEE_LENGTH as u32 + 1,
        MIN_COMMITTEE_LENGTH as u32 + 2,
    ] {
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
    assert_eq!(
        decode_output::<u64>(&output),
        MIN_COMMITTEE_LENGTH as u64 + 2,
        "the collapsed checkpoint carries the last value of the walk"
    );
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
                &U32Command {
                    value: MIN_COMMITTEE_LENGTH as u32,
                },
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
        staking::selected_addresses_at(&harness.sdk, 1).unwrap()[0],
        validator_b,
        "the E+2 delegation must not lift validator_a above validator_b at E+1"
    );
    assert_eq!(
        staking::selected_addresses_at(&harness.sdk, 2).unwrap()[0],
        validator_a,
        "the matured delegation lifts validator_a to the top at E+2"
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

// The tombstone is set on a validator left deliberately Active, because the
// status check alone already passes there: only a dedicated read of the flag can
// refuse the delegation.
#[test]
fn delegation_into_a_tombstoned_validator_is_refused() {
    let owner = Address::with_last_byte(0xa0);
    let delegator = Address::with_last_byte(0xb0);
    let tombstoned = Address::with_last_byte(0x01);
    let healthy = Address::with_last_byte(0x02);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![tombstoned, healthy], vec![stake, stake], 500),
        ExitCode::Ok
    );
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(tombstoned)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_ACTIVE
    );
    consensus_storage()
        .tombstoned_accessor()
        .entry(tombstoned)
        .set_checked(&mut harness.sdk, true)
        .unwrap();

    assert_direct_revert(
        staking::delegate_to(
            &mut harness.sdk,
            delegator,
            tombstoned,
            DEFAULT_MIN_STAKING_AMOUNT,
            false,
        ),
        &harness.sdk,
        ERR_VALIDATOR_TOMBSTONED,
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, tombstoned, 2).unwrap(),
        stake,
        "a refused delegation must leave the future snapshot untouched"
    );

    staking::delegate_to(
        &mut harness.sdk,
        delegator,
        healthy,
        DEFAULT_MIN_STAKING_AMOUNT,
        false,
    )
    .unwrap();
    assert_eq!(
        staking::validator_total_at(&harness.sdk, healthy, 2).unwrap(),
        stake + DEFAULT_MIN_STAKING_AMOUNT
    );
}

// `claim_delegator_before` is the gate's second caller, so the tombstone reaches
// `redelegateDelegatorFee` as well. The refusal must not cost the delegator the
// claim: `claimDelegatorFee` walks the identical path with `redelegate: false`
// and still pays it out whole.
#[test]
fn a_tombstone_refuses_redelegation_without_stranding_the_claim() {
    let owner = Address::with_last_byte(0xa0);
    let delegator = Address::with_last_byte(0xb0);
    let validator = Address::with_last_byte(0x01);
    let token = STAKING_TOKEN;
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let delegated = DEFAULT_MIN_VALIDATOR_STAKE;
    let reward = DEFAULT_MIN_STAKING_AMOUNT * U256::from(4);
    let activation_block = 1_000;
    let mut harness = Harness::new(activation_block);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![stake], 0),
        ExitCode::Ok
    );
    staking::delegate_to(&mut harness.sdk, delegator, validator, delegated, false).unwrap();

    staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(WARMUP_DELAY)
        .total_blend_rewards_accessor()
        .set_checked(
            &mut harness.sdk,
            math::narrow_reward(reward).expect("reward fits uint96"),
        )
        .unwrap();
    staking_storage()
        .last_rewarded_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, WARMUP_DELAY + 1)
        .unwrap();
    consensus_storage()
        .tombstoned_accessor()
        .entry(validator)
        .set_checked(&mut harness.sdk, true)
        .unwrap();

    let transfers = Rc::new(RefCell::new(Vec::<(Address, U256)>::new()));
    let recorded = transfers.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            assert_eq!(address, token);
            let transfer =
                SolidityABI::<(Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0).unwrap();
            recorded.borrow_mut().push(transfer);
            SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Ok)
        });

    harness.set_caller(delegator);
    harness.set_block_number(activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * (WARMUP_DELAY + 1));
    // Commission is zero and the delegator held `delegated` of `stake +
    // delegated` at the settled epoch, so half the blend is theirs. It clears the
    // staking minimum, which is what makes the redelegate branch reach the gate
    // instead of falling through as an all-dust claim.
    let claimable = reward * delegated / (stake + delegated);
    assert!(claimable >= DEFAULT_MIN_STAKING_AMOUNT);

    assert_revert_selector(
        harness.call(encode_call(
            SIG_REDELEGATE_DELEGATOR_FEE,
            &AddressCommand { value: validator },
        )),
        ERR_VALIDATOR_TOMBSTONED,
    );
    assert!(
        transfers.borrow().is_empty(),
        "the refused redelegation must not have paid out the dust leg either"
    );

    assert_eq!(
        harness
            .call(encode_call(
                SIG_CLAIM_DELEGATOR_FEE,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok,
        "the tombstone gates redelegation only, never the claim itself"
    );
    assert_eq!(transfers.borrow().as_slice(), &[(delegator, claimable)]);
}

// Four cases. Two carry the rule itself: a partial exit may not strand dust, and
// an underwater position still closes in full. The other two scope it — the size
// of the withdrawal is not what the minimum binds, so a sub-minimum withdrawal
// passes when what it leaves behind is healthy, and an ordinary withdrawal is
// untouched.
#[test]
fn undelegation_binds_the_minimum_to_the_remainder_not_the_withdrawal() {
    let owner = Address::with_last_byte(0xa0);
    let delegator = Address::with_last_byte(0xb0);
    let validator = Address::with_last_byte(0x01);
    let delegated = DEFAULT_MIN_STAKING_AMOUNT * U256::from(3);
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
    harness.set_block_number(1_200);

    let dust = DEFAULT_MIN_STAKING_AMOUNT / U256::from(2);
    let before = harness.sdk.dump_storage();
    assert_direct_revert(
        staking::undelegate_from(&mut harness.sdk, delegator, validator, delegated - dust),
        &harness.sdk,
        ERR_REMAINING_DELEGATION_TOO_LOW,
    );
    harness.sdk.restore_storage(before.clone());

    // The same sub-minimum amount as above, but now it is what leaves rather than
    // what stays. Nothing about the withdrawal's own size can refuse it, and this
    // is the case that separates the two readings of the minimum.
    staking::undelegate_from(&mut harness.sdk, delegator, validator, dust)
        .expect("a sub-minimum withdrawal is allowed when the remainder stays healthy");
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 2).unwrap(),
        DEFAULT_MIN_VALIDATOR_STAKE + delegated - dust
    );
    harness.sdk.restore_storage(before.clone());

    staking::undelegate_from(
        &mut harness.sdk,
        delegator,
        validator,
        delegated - DEFAULT_MIN_STAKING_AMOUNT,
    )
    .expect("a remainder of exactly the minimum is allowed");
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 2).unwrap(),
        DEFAULT_MIN_VALIDATOR_STAKE + DEFAULT_MIN_STAKING_AMOUNT
    );
    harness.sdk.restore_storage(before.clone());

    // The whole position is now smaller than the minimum. Under the old rule the
    // small withdrawal was AmountTooLow and the large one InsufficientBalance,
    // which left the stake with no exit at all.
    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_MIN_STAKING_AMOUNT,
                &U256Command {
                    value: DEFAULT_MIN_STAKING_AMOUNT * U256::from(10),
                },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 2).unwrap(),
        DEFAULT_MIN_VALIDATOR_STAKE + delegated
    );
    staking::undelegate_from(&mut harness.sdk, delegator, validator, delegated)
        .expect("a full exit is never blocked by a raised minimum");
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 2).unwrap(),
        DEFAULT_MIN_VALIDATOR_STAKE
    );
}

// `minValidatorStakeAmount` and `minStakingAmount` are set independently and are
// not ordered against each other, so a raised delegation minimum must not reach
// past its own parameter and hold self-stake to the stricter of the two. The
// second leg is what scopes the exemption: deleting the remainder rule outright
// would satisfy the first leg just as well.
#[test]
fn a_raised_delegation_minimum_does_not_govern_the_owner_self_stake() {
    let owner = Address::with_last_byte(0xa0);
    let delegator = Address::with_last_byte(0xb0);
    let validator = Address::with_last_byte(0x01);
    let self_stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(10);
    let delegated = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(6);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![self_stake], 0),
        ExitCode::Ok
    );
    staking::delegate_to(&mut harness.sdk, delegator, validator, delegated, false).unwrap();
    harness.set_block_number(1_200);

    // Only the delegation minimum moves. The validator minimum, which is the one
    // that governs self-stake, stays where genesis put it.
    harness.set_caller(GENESIS_GOVERNANCE);
    let raised = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(5);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_MIN_STAKING_AMOUNT,
                &U256Command { value: raised },
            ))
            .0,
        ExitCode::Ok
    );

    // Down to twice the validator minimum: clear of the parameter that governs
    // self-stake, far under the one that does not.
    let remainder = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2);
    assert!(remainder < raised);
    staking::undelegate_from(
        &mut harness.sdk,
        validator,
        validator,
        self_stake - remainder,
    )
    .expect("self-stake answers to the validator minimum, not the delegation minimum");
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 2).unwrap(),
        remainder + delegated
    );

    // A plain delegator on the same validator, under the same raised minimum, is
    // still held to it.
    assert_direct_revert(
        staking::undelegate_from(
            &mut harness.sdk,
            delegator,
            validator,
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2),
        ),
        &harness.sdk,
        ERR_REMAINING_DELEGATION_TOO_LOW,
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
    let (validators, stakes) =
        with_filler_validators(&[(validator_a, stake_a), (validator_b, stake_b)]);
    let expected_committee = validators.clone();
    let expected_stakes = stakes.clone();
    assert_eq!(
        harness.initialize(owner, validators, stakes, 500),
        ExitCode::Ok
    );

    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Panic
    );
    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );

    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE,
        &U64Command { value: 0 },
    ));
    // Peer keys are handed out in fixture order and the sort is on that key, so
    // the committed committee is the fixture list exactly — pin all of it.
    assert_eq!(decode_output::<Vec<Address>>(&output), expected_committee);
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
    assert_eq!(validators, expected_committee);
    assert_eq!(
        keys.iter()
            .map(|value| value.bls_pubkey[0])
            .collect::<Vec<_>>(),
        vec![0x33, 0x34, 0x35, 0x36]
    );
    assert_eq!(stakes, expected_stakes);

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
    assert_eq!(event_committee, expected_committee);
}

// The cap truncates the selection, so a cap below the committee floor makes every
// commit derive too few members and revert — on a pre-execution system call,
// which stops the chain with nothing able to put the cap back. The setter is the
// last place the two can still be reconciled by a transaction.
#[test]
fn the_cap_setter_refuses_a_value_below_the_committee_floor() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );

    harness.set_caller(GENESIS_GOVERNANCE);
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_ACTIVE_VALIDATORS_LENGTH,
            &U32Command {
                value: MIN_COMMITTEE_LENGTH as u32 - 1,
            },
        )),
        ERR_ACTIVE_VALIDATORS_LENGTH_BELOW_COMMITTEE_FLOOR,
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_ACTIVE_VALIDATORS_LENGTH,
                &U32Command {
                    value: MIN_COMMITTEE_LENGTH as u32,
                },
            ))
            .0,
        ExitCode::Ok,
        "the floor itself is legal"
    );
}

// A committee every member of which carries a zero frozen weight is committable:
// selection applies no stake floor, by design. Settlement then pays nobody and
// ADVANCES its cursor, so the epoch's pot is forfeited rather than deferred.
//
// This pins the behaviour rather than endorsing it. The state used to be caught
// by a length-mismatch revert that the paired-storage change made unreachable;
// what that revert stood in for is this, and nothing else covered it.
#[test]
fn a_committee_with_only_zero_weights_forfeits_its_epoch_rather_than_deferring() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let reserve = Address::with_last_byte(0xc0);
    let mut harness = Harness::new(1_000);
    let mut command =
        harness.initialize_command(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0);
    command.blend_reserve = reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    commit_test_committee(&mut harness.sdk, 0, &[(validator, U256::ZERO)]);
    chain_config_storage()
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::from(100))
        .unwrap();
    install_stipend_token(&harness.sdk, reserve, U256::from(1_000), U256::from(1_000));
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
    assert_eq!(epoch_reward(&harness.sdk, validator, 0), U256::ZERO);
    assert_eq!(
        staking_storage()
            .last_rewarded_epoch_p1_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1,
        "the cursor advances past an epoch nobody was paid for — the pot is gone"
    );
}

#[test]
fn an_eligible_set_one_short_of_the_floor_is_refused() {
    let owner = Address::with_last_byte(0xa0);
    // One below `MIN_COMMITTEE_LENGTH`: enough for a committee to exist, not
    // enough for one that tolerates a fault.
    let validators: Vec<Address> = (1..MIN_COMMITTEE_LENGTH)
        .map(|index| Address::with_last_byte(index as u8))
        .collect();
    assert_eq!(validators.len(), MIN_COMMITTEE_LENGTH - 1);
    // The floor is not an arbitrary number: it is the SMALLEST committee that
    // tolerates a fault. Pinning that relationship rather than the literal keeps
    // this test from being invariant to the constant it exists to defend — a
    // fixture written purely in terms of `MIN_COMMITTEE_LENGTH` survives any
    // value of it, including the value this change replaced.
    assert_eq!(crate::math::fault_tolerance(MIN_COMMITTEE_LENGTH), 1);
    assert_eq!(crate::math::fault_tolerance(MIN_COMMITTEE_LENGTH - 1), 0);

    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(
            owner,
            validators.clone(),
            vec![DEFAULT_MIN_VALIDATOR_STAKE; validators.len()],
            0,
        ),
        ExitCode::Ok
    );

    harness.set_caller(SYSTEM_CALLER);
    assert_revert_selector(
        harness.call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE)),
        ERR_COMMITTEE_TOO_SMALL,
    );
}

// A keyless validator outranking a keyed one by stake occupies a top-k slot and
// is dropped afterwards; it is not skipped over. Ranking before filtering is what
// keeps `getValidatorsWithKeysAt` — the array the off-chain deriver builds from —
// in agreement with the committee `commitEpochCommittee` will accept.
#[test]
fn the_committee_drops_keyless_members_after_the_stake_cut() {
    let owner = Address::with_last_byte(0xa0);
    let keyless = Address::with_last_byte(0x01);
    let keyed: Vec<Address> = (2..=MIN_COMMITTEE_LENGTH + 1)
        .map(|index| Address::with_last_byte(index as u8))
        .collect();
    let below_cut = Address::with_last_byte(0xee);
    let mut harness = Harness::new(1_000);
    // The cap seats `keyless` — top-ranked on stake — plus every `keyed` member,
    // leaving `below_cut` outside. Blanking the keyless member's key must leave
    // exactly the committee floor rather than promote `below_cut` into the gap.
    let cap = MIN_COMMITTEE_LENGTH + 1;
    let mut validators = vec![keyless];
    validators.extend(keyed.iter().copied());
    validators.push(below_cut);
    let mut stakes = vec![DEFAULT_MIN_VALIDATOR_STAKE * U256::from(100)];
    stakes.extend(
        keyed
            .iter()
            .map(|_| DEFAULT_MIN_VALIDATOR_STAKE * U256::from(3)),
    );
    stakes.push(DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2));
    let mut command = harness.initialize_command(owner, validators, stakes, 500);
    command.active_validators_length = cap as u32;
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
    assert_eq!(view.len(), cap);
    assert_eq!(
        view[0], keyless,
        "the selection view ranks by stake before any key filtering"
    );
    assert!(!view.contains(&below_cut));
    assert!(
        view_keys[0].bls_pubkey.is_empty(),
        "the keyless top-ranked validator is surfaced with blank keys, not omitted"
    );

    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );
    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE,
        &U64Command { value: 0 },
    ));
    // The cut takes `keyless` on stake plus every `keyed` member; the key filter
    // then drops `keyless`. `below_cut` is never promoted into the vacancy — that
    // is the whole point of filtering after the cut rather than before it.
    assert_eq!(decode_output::<Vec<Address>>(&output), keyed);
}

// A refused commit must leave no partial state — the cursor stays put and no
// committee is written. In production nobody gets to use that: the commit is a
// pre-execution system call, so this revert halts the chain rather than being
// retried. The property is about state consistency, not about a second attempt.
#[test]
fn a_refused_commit_writes_neither_committee_nor_cursor() {
    let owner = Address::with_last_byte(0xa0);
    let validators: Vec<Address> = (1..=MIN_COMMITTEE_LENGTH)
        .map(|index| Address::with_last_byte(index as u8))
        .collect();
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
        harness.call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE)),
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
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
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

// A minimum raised above every seated validator's self-stake is a single
// governance transaction. Were selection to re-read it live, that one call would
// leave nothing to commit and the chain would have no committee at all.
#[test]
fn a_raised_minimum_does_not_empty_the_next_committee() {
    let owner = Address::with_last_byte(0xa0);
    let first = Address::with_last_byte(0x01);
    let second = Address::with_last_byte(0x02);
    let (validators, stakes) = with_filler_validators(&[
        (first, DEFAULT_MIN_VALIDATOR_STAKE * U256::from(3)),
        (second, DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2)),
    ]);
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(owner, validators.clone(), stakes, 0);
    command.active_validators_length = MIN_COMMITTEE_LENGTH as u32;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );

    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_MIN_VALIDATOR_STAKE_AMOUNT,
                &U256Command {
                    value: DEFAULT_MIN_VALIDATOR_STAKE * U256::from(10),
                },
            ))
            .0,
        ExitCode::Ok
    );
    let (_, raised) = harness.call(encode_empty_call(SIG_GET_MIN_VALIDATOR_STAKE_AMOUNT));
    assert_eq!(
        decode_output::<U256>(&raised),
        DEFAULT_MIN_VALIDATOR_STAKE * U256::from(10)
    );

    assert_eq!(
        staking::selected_addresses_at(&harness.sdk, 0).unwrap(),
        validators
    );
    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );
    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE,
        &U64Command { value: 1 },
    ));
    assert_eq!(decode_output::<Vec<Address>>(&output), validators);
}

#[test]
fn committee_pruning_keeps_dkg_history() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let activation_block = 1_000;
    let mut harness = Harness::new(activation_block);
    harness.set_caller(owner);
    let (validators, stakes) = with_filler_validators(&[(validator, DEFAULT_MIN_VALIDATOR_STAKE)]);
    assert_eq!(
        harness.initialize(owner, validators, stakes, 0),
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
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
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

    // Still unarmed here: the height crosses several interval boundaries and the
    // epoch must not follow it. Configurable and running are two different
    // states, and the zero activation block means the first one.
    harness.set_block_number(4_000);
    let (_, output) = harness.call(encode_empty_call(SIG_CURRENT_EPOCH));
    assert_eq!(decode_output::<u64>(&output), 0);
    harness.set_block_number(0);

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
fn scheduling_activation_never_moves_the_epoch_backwards() {
    let owner = Address::with_last_byte(0xa0);
    let mut harness = Harness::new(0);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    fn epoch(harness: &mut Harness) -> u64 {
        let (_, output) = harness.call(encode_empty_call(SIG_CURRENT_EPOCH));
        decode_output::<u64>(&output)
    }

    harness.set_block_number(4_000);
    assert_eq!(
        epoch(&mut harness),
        0,
        "an unarmed chain must not accrue epochs from genesis"
    );

    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_DPOS_ACTIVATION_BLOCK,
                &U64Command { value: 6_000 },
            ))
            .0,
        ExitCode::Ok
    );

    // Arming is what used to drop a running counter back to zero: the epoch was
    // derived from the height alone while unarmed, so scheduling a real
    // activation rebased it downward and rewrote which checkpoint a stake fell
    // under. Every step from here must be non-decreasing.
    let mut previous = epoch(&mut harness);
    assert_eq!(previous, 0);
    for height in [4_001u64, 5_999, 6_000, 6_199, 6_200, 6_400] {
        harness.set_block_number(height);
        let current = epoch(&mut harness);
        assert!(
            current >= previous,
            "epoch moved backwards at block {height}: {previous} -> {current}"
        );
        previous = current;
    }
    assert_eq!(
        previous, 2,
        "epochs count from the activation block once armed"
    );
}

#[test]
fn undelegate_period_change_does_not_shorten_queued_principal() {
    let sponsor = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let mut harness = Harness::new(0);
    assert_eq!(
        harness.initialize(sponsor, vec![validator], vec![stake], 0),
        ExitCode::Ok
    );

    staking::undelegate_from(&mut harness.sdk, validator, validator, stake).unwrap();
    let queued = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(validator)
        .undelegate_queue_accessor()
        .at(0)
        .epoch_accessor();
    let maturity_epoch = queued.get_checked(&harness.sdk).unwrap();

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
    assert_eq!(
        queued.get_checked(&harness.sdk).unwrap(),
        maturity_epoch,
        "a shortened period must not release principal queued under the old one"
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
        staking::selected_addresses_at(&harness.sdk, 0).unwrap(),
        vec![validator],
        "the completed epoch remains available for historical committee validation"
    );
    assert!(staking::selected_addresses_at(&harness.sdk, 1)
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
fn a_funded_and_approved_source_pays_the_full_stipend() {
    let pot = U256::from(100);
    let (mut harness, funding, validator) =
        stipend_test_sdk(pot * U256::from(2), pot * U256::from(2));

    assert_eq!(
        harness
            .call(encode_call(
                SIG_SETTLE_EPOCH_STIPEND,
                &U64Command { value: 0 },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(stipend_accounting(&harness.sdk, validator), (pot, pot, 1));
    assert_eq!(
        funding.borrow().pulls,
        vec![(GENESIS_STAKING, pot)],
        "one pull of the whole pot, straight onto the staking contract"
    );
    assert_eq!(funding.borrow().balance, pot);
    assert_eq!(funding.borrow().allowance, pot);
    assert_stipend_events(&harness.sdk, 0, pot, false);
}

// A source that cannot cover the epoch must postpone it. Crediting the shortfall
// as zero and moving on would be permanent: the cursor never walks back.
#[test]
fn an_underfunded_source_defers_the_epoch_instead_of_burning_it() {
    let pot = U256::from(100);
    let (mut harness, funding, validator) = stipend_test_sdk(U256::from(60), pot);

    assert_eq!(
        harness
            .call(encode_call(
                SIG_SETTLE_EPOCH_STIPEND,
                &U64Command { value: 0 },
            ))
            .0,
        ExitCode::Panic
    );
    assert_eq!(
        stipend_accounting(&harness.sdk, validator),
        (U256::ZERO, U256::ZERO, 0),
        "nothing credited and the cursor still on the epoch"
    );
    assert_eq!(funding.borrow().pulls, vec![(GENESIS_STAKING, pot)]);
    assert!(harness.sdk.take_logs().is_empty());

    funding.borrow_mut().balance = pot;
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SETTLE_EPOCH_STIPEND,
                &U64Command { value: 0 },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(stipend_accounting(&harness.sdk, validator), (pot, pot, 1));
    assert_eq!(funding.borrow().balance, U256::ZERO);
    assert_stipend_events(&harness.sdk, 0, pot, false);
}

// Revoking the approval is the operator's pause switch now that the reserve
// contract is gone. It has to hold the epochs, not consume them: a paused
// reserve used to answer zero, which settlement recorded as "paid nothing" and
// stepped over.
#[test]
fn a_revoked_allowance_holds_the_epochs_until_it_is_restored() {
    let pot = U256::from(100);
    let backlog = pot * U256::from(2);
    let (mut harness, funding, validator) = stipend_test_sdk(backlog, U256::ZERO);
    commit_test_committee(
        &mut harness.sdk,
        1,
        &[(validator, DEFAULT_MIN_VALIDATOR_STAKE)],
    );
    record_test_production(&mut harness.sdk, 1, DEFAULT_EPOCH_BLOCK_INTERVAL as u32);
    harness.set_block_number(1_000 + 2 * DEFAULT_EPOCH_BLOCK_INTERVAL);
    harness.set_caller(SYSTEM_CALLER);

    assert_eq!(
        harness
            .call(encode_call(
                SIG_SETTLE_EPOCH_STIPEND,
                &U64Command { value: 1 },
            ))
            .0,
        ExitCode::Panic
    );
    assert_eq!(
        staking_storage()
            .last_rewarded_epoch_p1_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "a revoked approval must not consume the epochs it stops"
    );
    assert_eq!(funding.borrow().balance, backlog);

    funding.borrow_mut().allowance = backlog;
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SETTLE_EPOCH_STIPEND,
                &U64Command { value: 1 },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(epoch_reward(&harness.sdk, validator, 0), pot);
    assert_eq!(
        epoch_reward(&harness.sdk, validator, 1),
        pot,
        "both epochs held during the pause are paid once it lifts"
    );
    assert_eq!(
        staking_storage()
            .last_rewarded_epoch_p1_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        2
    );
    assert_eq!(funding.borrow().balance, U256::ZERO);
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
    let (validators, stakes) =
        with_filler_validators(&[(validator_a, stake), (validator_b, stake)]);
    let mut command = harness.initialize_command(owner, validators, stakes, 0);
    command.blend_reserve = reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    harness.set_caller(SYSTEM_CALLER);
    for _ in 0..3 {
        assert_eq!(
            harness
                .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
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
    install_stipend_token(&harness.sdk, reserve, U256::from(100), U256::from(100));
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
    // Equal frozen weights across the whole committee, so the pot splits evenly
    // over `MIN_COMMITTEE_LENGTH` members — the point is that both named members
    // are paid the SAME share despite one of them gaining stake afterwards.
    let share = U256::from(100 / MIN_COMMITTEE_LENGTH);
    assert_eq!(epoch_reward(&harness.sdk, validator_a, 2), share);
    assert_eq!(epoch_reward(&harness.sdk, validator_b, 2), share);
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
    install_stipend_token(&harness.sdk, reserve, U256::from(1_000), U256::from(1_000));
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
    install_stipend_token(&harness.sdk, reserve, U256::from(1_000), U256::from(1_000));
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
fn stipend_pays_the_rate_pinned_at_close_not_the_live_one() {
    let pot = U256::from(100);
    let (mut harness, funding, validator) = stipend_test_sdk(pot, pot);

    // The rate the epoch worked under is already pinned. Dropping the live one to
    // zero afterwards is the governance action that used to erase the epoch's pay
    // outright: settlement returns Ok, the cursor moves past it, and no later
    // call can revisit it.
    chain_config_storage()
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::ZERO)
        .unwrap();
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
    assert_eq!(stipend_accounting(&harness.sdk, validator), (pot, pot, 1));
    assert_eq!(funding.borrow().pulls, vec![(GENESIS_STAKING, pot)]);
    assert_stipend_events(&harness.sdk, 0, pot, false);

    // The mirror image: a raised live rate must not enrich an epoch that closed
    // under a lower one either.
    let (mut harness, funding, validator) = stipend_test_sdk(pot, pot);
    production_liveness_storage()
        .stipend_rate_at_close_p1_accessor()
        .entry(0)
        .set_checked(&mut harness.sdk, U256::ONE)
        .unwrap();
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
        stipend_accounting(&harness.sdk, validator),
        (U256::ZERO, U256::ZERO, 1)
    );
    assert!(funding.borrow().pulls.is_empty());
    assert_stipend_events(&harness.sdk, 0, U256::ZERO, false);
}

#[test]
fn closing_an_epoch_pins_the_stipend_rate() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );
    chain_config_storage()
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::from(250))
        .unwrap();
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[(validator, DEFAULT_MIN_VALIDATOR_STAKE)],
    );
    // Written raw rather than through `record_test_production`, which pins the
    // rate itself. The whole point of this test is that the contract pins it, so
    // seeding through the stand-in would prove nothing.
    production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(0)
        .set_checked(&mut harness.sdk, DEFAULT_EPOCH_BLOCK_INTERVAL as u32)
        .unwrap();
    let pinned = production_liveness_storage()
        .stipend_rate_at_close_p1_accessor()
        .entry(0);
    assert_eq!(pinned.get_checked(&harness.sdk).unwrap(), U256::ZERO);

    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);

    assert_eq!(
        pinned.get_checked(&harness.sdk).unwrap(),
        U256::from(251),
        "the close pins rate + 1 for the epoch it closes"
    );
}

#[test]
fn settling_an_unclosed_epoch_reverts_instead_of_forfeiting_it() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );
    chain_config_storage()
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::from(100))
        .unwrap();
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[(validator, DEFAULT_MIN_VALIDATOR_STAKE)],
    );
    // An epoch that recorded blocks but whose close never ran: seeded raw, so no
    // rate was pinned for it.
    production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(0)
        .set_checked(&mut harness.sdk, DEFAULT_EPOCH_BLOCK_INTERVAL as u32)
        .unwrap();
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL);
    harness.set_caller(SYSTEM_CALLER);

    assert_revert_selector(
        harness.call(encode_call(
            SIG_SETTLE_EPOCH_STIPEND,
            &U64Command { value: 0 },
        )),
        ERR_STIPEND_RATE_NOT_SNAPSHOTTED,
    );
    assert_eq!(
        staking_storage()
            .last_rewarded_epoch_p1_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "a revert leaves the epoch for a retry; a guard return would forfeit it"
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
    install_stipend_token(&harness.sdk, reserve, U256::from(100), U256::from(100));
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

// A token that refuses the pull, whichever way it reports the refusal. The
// `false` reply is the non-conforming ERC-20 shape: no revert, so a contract
// that only checked the status would credit an epoch nothing moved for.
#[test]
fn a_refused_pull_remains_retryable() {
    for reports_failure in [false, true] {
        let assigned = U256::from(100);
        let (mut harness, funding, validator) = stipend_test_sdk(assigned, assigned);
        if reports_failure {
            funding.borrow_mut().reports_failure = true;
        } else {
            funding.borrow_mut().allowance = U256::ZERO;
        }

        assert_eq!(
            harness
                .call(encode_call(
                    SIG_SETTLE_EPOCH_STIPEND,
                    &U64Command { value: 0 },
                ))
                .0,
            ExitCode::Panic
        );
        assert_eq!(
            stipend_accounting(&harness.sdk, validator),
            (U256::ZERO, U256::ZERO, 0)
        );
        assert!(harness.sdk.take_logs().is_empty());

        funding.borrow_mut().reports_failure = false;
        funding.borrow_mut().allowance = assigned;
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
        assert_eq!(funding.borrow().pulls.len(), 2);
        assert_stipend_events(&harness.sdk, 0, assigned, false);
    }
}

#[test]
fn permissionless_validator_claim_waits_for_stipend_settlement() {
    let assigned = U256::from(100);
    let attacker = Address::with_last_byte(0xd0);
    let (mut harness, _funding, validator) = stipend_test_sdk(assigned, assigned);
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
            assert_eq!(address, STAKING_TOKEN);
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
    let token = STAKING_TOKEN;
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let reward = DEFAULT_MIN_STAKING_AMOUNT;
    let activation_block = 1_000;
    let mut harness = Harness::new(activation_block);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![stake], 0),
        ExitCode::Ok
    );
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
            .claimed_through_epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        WARMUP_DELAY,
        "principal maturity must not advance the reward cursor past the settled frontier"
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
            .claimed_through_epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        WARMUP_DELAY + 1,
        "the settled epoch is now paid, so the cursor sits one past it"
    );
}

#[test]
fn claiming_rewards_does_not_rewrite_historical_self_stake() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(0);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );

    // A genesis validator is its own delegator, so its self-stake is the single delegate-queue
    // entry the claim walks. Settling a frontier is what makes the claim advance at all.
    let past_epoch = 20;
    harness.set_block_number(DEFAULT_EPOCH_BLOCK_INTERVAL * 40);
    staking_storage()
        .last_rewarded_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, 40)
        .unwrap();

    let stake_before =
        staking::delegated_amount_at(&harness.sdk, validator, validator, past_epoch).unwrap();
    assert!(!stake_before.is_zero());
    assert!(staking::selected_addresses_at(&harness.sdk, past_epoch)
        .unwrap()
        .contains(&validator));

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
        staking::delegated_amount_at(&harness.sdk, validator, validator, past_epoch).unwrap(),
        stake_before,
        "a claim must not change what the self-stake was at an already-committed epoch"
    );
    assert!(
        staking::selected_addresses_at(&harness.sdk, past_epoch)
            .unwrap()
            .contains(&validator),
        "the off-chain deriver re-reads past-epoch selection to rebuild committees; a claim must \
         not drop the validator out of it"
    );
}

#[test]
fn reward_claims_are_bounded_to_one_thousand_epochs() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    // Not block 0: the harness seeds the activation block from the current
    // height, and a zero activation is the unarmed sentinel, which pins every
    // epoch at 0 and leaves this claim window empty.
    let activation = DEFAULT_EPOCH_BLOCK_INTERVAL;
    let mut harness = Harness::new(activation);
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
    harness
        .set_block_number(activation + DEFAULT_EPOCH_BLOCK_INTERVAL * (MAX_EPOCHS_PER_CLAIM + 1));
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
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(validator);
    assert_eq!(
        delegation
            .claimed_through_epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        MAX_EPOCHS_PER_CLAIM,
        "the claim advances its own cursor by at most the per-claim bound"
    );
    assert_eq!(
        delegation
            .delegate_queue_accessor()
            .at(0)
            .epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "the effective-from epoch is immutable: historical self-stake lookups binary-search it, \
         so a claim that moved it would rewrite past-epoch committee views"
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
fn the_commit_orders_by_peer_key_and_membership_changes_mint_the_dkg_bit() {
    let owner = Address::with_last_byte(0xa0);
    let validator_a = Address::with_last_byte(0x01);
    let validator_b = Address::with_last_byte(0x02);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    // One member is disabled part-way through, so seat one more than the floor:
    // the commits after the disable must still find a legal committee.
    // Stakes ASCEND with the roster, and peer keys ascend with it too. Ranking is
    // by stake DESCENDING, so it hands the sort a peer-key-descending list and the
    // sort has real work to do. Get this backwards — stakes descending — and the
    // ranked order already equals the sorted order, deleting the sort changes
    // nothing, and the assertion below passes on a contract that never sorts.
    let mut named = vec![
        (validator_a, DEFAULT_MIN_VALIDATOR_STAKE * U256::from(7)),
        (validator_b, DEFAULT_MIN_VALIDATOR_STAKE * U256::from(8)),
    ];
    named.push((
        Address::with_last_byte(0x03),
        DEFAULT_MIN_VALIDATOR_STAKE * U256::from(9),
    ));
    let (mut validators, mut stakes) = with_filler_validators(&named);
    validators.push(Address::with_last_byte(0xd0));
    stakes.push(DEFAULT_MIN_VALIDATOR_STAKE);
    assert_eq!(
        harness.initialize(owner, validators, stakes, 500),
        ExitCode::Ok
    );

    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );

    // Canonical order is now PRODUCED, not checked: nobody told the contract
    // which permutation to store. Assert the property itself rather than a
    // specific address order, which depends on how the harness happens to
    // assign peer keys.
    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE,
        &U64Command { value: 0 },
    ));
    let committed = decode_output::<Vec<Address>>(&output);
    assert_eq!(committed.len(), MIN_COMMITTEE_LENGTH + 1);
    let peer_of = |sdk: &TestingContextImpl, validator: Address| {
        consensus_storage()
            .consensus_keys_accessor()
            .entry(validator)
            .peer_pubkey_accessor()
            .get_checked(sdk)
            .unwrap()
    };
    for pair in committed.windows(2) {
        assert!(
            peer_of(&harness.sdk, pair[0]) < peer_of(&harness.sdk, pair[1]),
            "the committee the contract derived must be strictly ascending by peer key"
        );
    }

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
                .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
                .0,
            ExitCode::Ok
        );
    }
    harness.set_block_number(1_200);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );
    for unchanged_epoch in [1u64, 2] {
        let (_, output) = harness.call(encode_call(
            SIG_GET_DKG_QUAL,
            &U64Command {
                value: unchanged_epoch,
            },
        ));
        assert!(
            !decode_output::<bool>(&output),
            "an unchanged committee must not mint a DKG ceremony for epoch {unchanged_epoch}"
        );
    }
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
            .claimed_through_epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "seizure resets the reward cursor with the queues"
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

/// One slashable conflict shape and the three things that must agree on it: the
/// entry point that accepts it, the proof kind its commitment is bound to, and
/// the corpus blob carrying it.
///
/// Routing a kind to the wrong arm makes that kind permanently unslashable
/// on-chain while every other kind keeps working, so each test names the route
/// it drives instead of inheriting one.
struct ProofRoute {
    selector: u32,
    proof_kind: u8,
    shape: evidence::EvidenceShape,
    blob: &'static [u8],
}

const NOTARIZE_ROUTE: ProofRoute = ProofRoute {
    selector: SIG_SLASH_EQUIVOCATION_NOTARIZE,
    proof_kind: EQUIVOCATION_PROOF_KIND_NOTARIZE,
    shape: evidence::EvidenceShape::ConflictingNotarize,
    blob: &evidence::tests::CONFLICTING_NOTARIZE,
};

const FINALIZE_ROUTE: ProofRoute = ProofRoute {
    selector: SIG_SLASH_EQUIVOCATION_FINALIZE,
    proof_kind: EQUIVOCATION_PROOF_KIND_FINALIZE,
    shape: evidence::EvidenceShape::ConflictingFinalize,
    blob: &evidence::tests::CONFLICTING_FINALIZE,
};

/// Unlike the other two, this blob is 135 bytes and its first half is a bare
/// round rather than a proposal, so it only parses under its own shape.
const NULLIFY_FINALIZE_ROUTE: ProofRoute = ProofRoute {
    selector: SIG_SLASH_EQUIVOCATION_NULLIFY_FINALIZE,
    proof_kind: EQUIVOCATION_PROOF_KIND_NULLIFY_FINALIZE,
    shape: evidence::EvidenceShape::NullifyFinalize,
    blob: &evidence::tests::NULLIFY_FINALIZE,
};

/// Builds a report against whichever validator owns the key `pk_byte`
/// compresses to, carrying `route`'s conflict.
///
/// The evidence is the node-side corpus blob and the two uncompressed signatures
/// are that blob's own signature bytes padded to the 128-byte G1 width the mock
/// verifier compresses back down, so the reveal carries exactly what the parser
/// will find inside the blob.
fn equivocation_report(
    sdk: &mut TestingContextImpl,
    route: &ProofRoute,
    pk_byte: u8,
    beneficiary: Address,
) -> EquivocationCommand {
    let blob = Bytes::copy_from_slice(route.blob);
    let decoded = evidence::decode(sdk, &blob, route.shape).expect("the corpus blob parses");
    let uncompressed = |signature: &Bytes| {
        let mut padded = signature.to_vec();
        padded.resize(BLS_POP_UNCOMPRESSED_LENGTH, 0);
        Bytes::from(padded)
    };
    EquivocationCommand {
        evidence: blob,
        pk_uncompressed: Bytes::from(vec![pk_byte; BLS_PUBKEY_UNCOMPRESSED_LENGTH]),
        sig1_uncompressed: uncompressed(&decoded.sig1),
        sig2_uncompressed: uncompressed(&decoded.sig2),
        beneficiary,
        salt: B256::with_last_byte(0x51),
    }
}

/// The epoch every corpus blob names, pinned by the decoder's own corpus tests.
/// It is the epoch a slash used to need a live committee record for.
const CORPUS_EPOCH: u64 = 7;

/// Sends the reveal in every slash test.
///
/// Never a beneficiary, because the reward must follow the committed
/// beneficiary rather than whoever submits the reveal — a reveal sitting in the
/// mempool is copyable, and paying its sender is exactly the theft the
/// commit/reveal split exists to stop.
const EQUIVOCATION_RELAYER: Address = Address::with_last_byte(0xd7);

/// Drives the real two-step report: the beneficiary commits, a block passes so
/// the commitment matures, and the relayer reveals. Logs are drained in
/// between, so what the caller reads back belongs to the slash alone.
fn commit_and_slash(
    harness: &mut Harness,
    route: &ProofRoute,
    command: &EquivocationCommand,
) -> (ExitCode, Vec<u8>) {
    let commitment = consensus::report_commitment_hash(
        harness.sdk.context().block_chain_id(),
        GENESIS_STAKING,
        route.proof_kind,
        keccak256(&command.evidence),
        command.beneficiary,
        command.salt,
    );
    harness.set_caller(command.beneficiary);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_COMMIT_EQUIVOCATION_REPORT,
                &(commitment,),
            ))
            .0,
        ExitCode::Ok
    );
    let matured = harness.sdk.context().block_number() + 1;
    harness.set_block_number(matured);
    harness.sdk.take_logs();
    harness.set_caller(EQUIVOCATION_RELAYER);
    harness.call(encode_args_call(
        route.selector,
        &(
            command.evidence.clone(),
            command.pk_uncompressed.clone(),
            command.sig1_uncompressed.clone(),
            command.sig2_uncompressed.clone(),
            command.beneficiary,
            command.salt,
        ),
    ))
}

/// Records every ERC-20 transfer the contract makes while still answering the
/// verifier calls the slash path depends on.
///
/// `signatures_valid` is the answer the stand-in verifier gives to
/// `verifyPairing`; `false` is the only way a test reaches the gate that decides
/// whether the supplied signatures actually belong to the named key.
fn record_transfers(
    harness: &Harness,
    signatures_valid: bool,
) -> Rc<RefCell<Vec<(Address, U256)>>> {
    let transfers = Rc::new(RefCell::new(Vec::<(Address, U256)>::new()));
    let recorded = transfers.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            if input.len() < SIG_LEN_BYTES {
                return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams);
            }
            let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
            if selector == SIG_ERC20_TRANSFER {
                assert_eq!(address, STAKING_TOKEN);
                recorded.borrow_mut().push(
                    SolidityABI::<(Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0).unwrap(),
                );
            }
            if selector == SIG_BLS_VERIFY && !signatures_valid {
                return SyscallResult::new(encode_mock_return(&false), 0, 0, ExitCode::Ok);
            }
            match mock_external_return(selector, &input[SIG_LEN_BYTES..]) {
                Some(output) => SyscallResult::new(output, 0, 0, ExitCode::Ok),
                None => SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams),
            }
        });
    transfers
}

fn find_log<'a>(
    logs: &'a [(Bytes, Vec<B256>)],
    selector: [u8; 32],
    name: &str,
) -> &'a (Bytes, Vec<B256>) {
    logs.iter()
        .find(|(_, topics)| topics.first() == Some(&B256::new(selector)))
        .unwrap_or_else(|| panic!("{name} must be emitted"))
}

#[test]
fn equivocation_slash_tombstones_jails_and_splits_the_seized_self_stake() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let reporter = Address::with_last_byte(0xb0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 500),
        ExitCode::Ok
    );
    let transfers = record_transfers(&harness, true);

    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11, reporter);
    assert_eq!(
        commit_and_slash(&mut harness, &NOTARIZE_ROUTE, &command).0,
        ExitCode::Ok
    );

    assert!(consensus_storage()
        .tombstoned_accessor()
        .entry(offender)
        .get_checked(&harness.sdk)
        .unwrap());
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(offender)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_JAIL
    );
    let active = staking_storage().active_validators_accessor();
    assert_eq!(active.len_checked(&harness.sdk).unwrap(), 1);
    assert_eq!(
        active.at(0).get_checked(&harness.sdk).unwrap(),
        bystander,
        "the slash removes the offender from the active set and nobody else"
    );
    // The penalty is stamped at epoch 0, so it lands from epoch 1 on.
    assert!(!staking::selection_visible_at(&harness.sdk, offender, 1).unwrap());
    assert!(staking::selection_visible_at(&harness.sdk, bystander, 1).unwrap());
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(offender)
        .entry(offender);
    assert_eq!(
        delegation
            .delegate_queue_accessor()
            .len_checked(&harness.sdk)
            .unwrap(),
        0
    );

    let reward = stake * U256::from(DEFAULT_SLASH_REPORTER_REWARD_BPS) / U256::from(10_000);
    assert_eq!(
        transfers.borrow().as_slice(),
        &[(reporter, reward), (EQUIVOCATION_BURN_SINK, stake - reward)],
        "the reward follows the committed beneficiary, not the relayer that revealed"
    );

    let logs = harness.sdk.take_logs();
    assert_eq!(logs.len(), 3);
    let (jailed_data, jailed_topics) = find_log(&logs, events::ValidatorJailed::SELECTOR, "jail");
    assert_eq!(&jailed_topics[1].0[12..], offender.as_slice());
    assert_eq!(decode_output::<u64>(jailed_data), 0);
    let (slashed_data, slashed_topics) =
        find_log(&logs, events::EquivocationSlashed::SELECTOR, "slash");
    assert_eq!(&slashed_topics[1].0[12..], offender.as_slice());
    assert_eq!(&slashed_topics[2].0[12..], reporter.as_slice());
    assert_eq!(
        decode_output::<u64>(slashed_data),
        CORPUS_EPOCH,
        "the event reports the epoch the conflict happened in, not the penalty epoch"
    );
    let (seized_data, seized_topics) =
        find_log(&logs, events::EquivocationStakeSeized::SELECTOR, "seizure");
    assert_eq!(&seized_topics[1].0[12..], offender.as_slice());
    assert_eq!(
        decode_output::<(U256, U256, Address)>(seized_data),
        (reward, stake - reward, EQUIVOCATION_BURN_SINK)
    );
}

/// Records transfers like `record_transfers`, but the stand-in token refuses
/// every recipient in `refused`. The attempt is recorded before the refusal, so
/// a test can see what the seizure tried before it was turned away.
///
/// `hard_revert` picks the refusal vector: a reverting call, or the `false` a
/// plain ERC-20 returns instead. `try_transfer` handles the two in different
/// branches, so covering only one of them would leave the other live.
fn record_transfers_refusing(
    harness: &Harness,
    refused: Vec<Address>,
    hard_revert: bool,
) -> Rc<RefCell<Vec<(Address, U256)>>> {
    let transfers = Rc::new(RefCell::new(Vec::<(Address, U256)>::new()));
    let recorded = transfers.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            if input.len() < SIG_LEN_BYTES {
                return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams);
            }
            let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
            if selector == SIG_ERC20_TRANSFER {
                assert_eq!(address, STAKING_TOKEN);
                let transfer =
                    SolidityABI::<(Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0).unwrap();
                recorded.borrow_mut().push(transfer);
                if refused.contains(&transfer.0) {
                    return if hard_revert {
                        SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic)
                    } else {
                        SyscallResult::new(encode_mock_return(&false), 0, 0, ExitCode::Ok)
                    };
                }
            }
            match mock_external_return(selector, &input[SIG_LEN_BYTES..]) {
                Some(output) => SyscallResult::new(output, 0, 0, ExitCode::Ok),
                None => SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams),
            }
        });
    transfers
}

#[test]
fn a_seizure_stops_the_seized_bond_counting_as_stake() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let reporter = Address::with_last_byte(0xb0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let warmup = DEFAULT_MIN_STAKING_AMOUNT;
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 500),
        ExitCode::Ok
    );
    // Booked at epoch 2, so epoch 1 never counts it and the queue tail is larger
    // than what the nearer snapshot holds.
    staking::delegate_to(&mut harness.sdk, offender, offender, warmup, false).unwrap();
    assert_eq!(
        staking::validator_total_at(&harness.sdk, offender, 1).unwrap(),
        stake
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, offender, 2).unwrap(),
        stake + warmup
    );

    let transfers = record_transfers(&harness, true);
    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11, reporter);
    assert_eq!(
        commit_and_slash(&mut harness, &NOTARIZE_ROUTE, &command).0,
        ExitCode::Ok
    );

    assert_eq!(
        staking::validator_total_at(&harness.sdk, offender, 0).unwrap(),
        stake,
        "the epoch the bond was still securing keeps the denominator it accrued under"
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, offender, 1).unwrap(),
        U256::ZERO,
        "stake that left for the burn sink stops counting from the next epoch on"
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, offender, 2).unwrap(),
        U256::ZERO,
        "the warm-up cohort is removed from the epoch it would have taken effect in"
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, bystander, 1).unwrap(),
        stake,
        "nobody else moves"
    );

    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATOR_STATUS,
        &AddressCommand { value: offender },
    ));
    let status: (Address, u8, U256, u64, u64, u16) = decode_output(&output);
    assert_eq!(
        status.2,
        U256::ZERO,
        "the view stops reporting a bond the contract no longer holds"
    );

    let reward =
        (stake + warmup) * U256::from(DEFAULT_SLASH_REPORTER_REWARD_BPS) / U256::from(10_000);
    assert_eq!(
        transfers.borrow().as_slice(),
        &[
            (reporter, reward),
            (EQUIVOCATION_BURN_SINK, stake + warmup - reward),
        ]
    );
}

#[test]
fn a_refused_reporter_payment_folds_into_the_remainder() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let reporter = Address::with_last_byte(0xb0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 500),
        ExitCode::Ok
    );
    let transfers = record_transfers_refusing(&harness, vec![reporter], false);

    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11, reporter);
    assert_eq!(
        commit_and_slash(&mut harness, &NOTARIZE_ROUTE, &command).0,
        ExitCode::Ok,
        "a token that turns the reporter away must not roll the slash back"
    );
    assert!(consensus_storage()
        .tombstoned_accessor()
        .entry(offender)
        .get_checked(&harness.sdk)
        .unwrap());
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(offender)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_JAIL
    );

    let reward = stake * U256::from(DEFAULT_SLASH_REPORTER_REWARD_BPS) / U256::from(10_000);
    assert_eq!(
        transfers.borrow().as_slice(),
        &[(reporter, reward), (EQUIVOCATION_BURN_SINK, stake)],
        "the refused cut joins the remainder instead of stranding here"
    );
    let logs = harness.sdk.take_logs();
    let (seized_data, _) = find_log(&logs, events::EquivocationStakeSeized::SELECTOR, "seizure");
    assert_eq!(
        decode_output::<(U256, U256, Address)>(seized_data),
        (U256::ZERO, stake, EQUIVOCATION_BURN_SINK),
        "the event reports what moved, not what was intended"
    );
}

#[test]
fn a_slash_survives_a_token_that_refuses_every_recipient() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let reporter = Address::with_last_byte(0xb0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 500),
        ExitCode::Ok
    );
    // The remainder's default recipient is a burn sink the caller does not
    // choose, so a token that rejects it would otherwise make equivocation
    // unslashable chain-wide.
    let transfers =
        record_transfers_refusing(&harness, vec![reporter, EQUIVOCATION_BURN_SINK], true);

    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11, reporter);
    assert_eq!(
        commit_and_slash(&mut harness, &NOTARIZE_ROUTE, &command).0,
        ExitCode::Ok
    );
    assert!(consensus_storage()
        .tombstoned_accessor()
        .entry(offender)
        .get_checked(&harness.sdk)
        .unwrap());
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(offender)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_JAIL
    );
    assert_eq!(
        staking_storage()
            .validator_delegations_accessor()
            .entry(offender)
            .entry(offender)
            .delegate_queue_accessor()
            .len_checked(&harness.sdk)
            .unwrap(),
        0
    );

    let reward = stake * U256::from(DEFAULT_SLASH_REPORTER_REWARD_BPS) / U256::from(10_000);
    assert_eq!(
        transfers.borrow().as_slice(),
        &[(reporter, reward), (EQUIVOCATION_BURN_SINK, stake)],
        "both legs are attempted, and neither refusal propagates"
    );
    let logs = harness.sdk.take_logs();
    let (seized_data, _) = find_log(&logs, events::EquivocationStakeSeized::SELECTOR, "seizure");
    assert_eq!(
        decode_output::<(U256, U256, Address)>(seized_data),
        (U256::ZERO, U256::ZERO, EQUIVOCATION_BURN_SINK)
    );
}

#[test]
fn a_pruned_evidence_epoch_no_longer_blocks_a_slash() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let reporter = Address::with_last_byte(0xb0);
    let mut harness = Harness::new(1_000);
    let (validators, stakes) = with_filler_validators(&[(offender, DEFAULT_MIN_VALIDATOR_STAKE)]);
    assert_eq!(
        harness.initialize(sponsor, validators, stakes, 0),
        ExitCode::Ok
    );

    // Commit every epoch through the evidence's own, one per epoch because the
    // commit pointer may not run more than `MAX_COMMITTEE_LOOKAHEAD_EPOCHS` ahead.
    harness.set_caller(SYSTEM_CALLER);
    for epoch in 0..=CORPUS_EPOCH {
        harness.set_block_number(1_000 + epoch * DEFAULT_EPOCH_BLOCK_INTERVAL);
        assert_eq!(
            harness
                .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
                .0,
            ExitCode::Ok
        );
    }
    let consensus = consensus_storage();
    assert_eq!(
        consensus
            .epoch_committees_accessor()
            .entry(CORPUS_EPOCH)
            .len_checked(&harness.sdk)
            .unwrap() as usize,
        MIN_COMMITTEE_LENGTH
    );

    // Pruning is held behind the settlement cursor and the liability deadline, so
    // release both and let the next commit retire everything committed so far.
    staking_storage()
        .last_rewarded_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, CORPUS_EPOCH + 1)
        .unwrap();
    harness.set_block_number(1_000 + 60 * DEFAULT_EPOCH_BLOCK_INTERVAL);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        consensus
            .epoch_committees_accessor()
            .entry(CORPUS_EPOCH)
            .len_checked(&harness.sdk)
            .unwrap(),
        0,
        "the epoch the evidence names has been retired"
    );

    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11, reporter);
    assert_eq!(
        commit_and_slash(&mut harness, &NOTARIZE_ROUTE, &command).0,
        ExitCode::Ok
    );
    assert!(consensus
        .tombstoned_accessor()
        .entry(offender)
        .get_checked(&harness.sdk)
        .unwrap());
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(offender)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_JAIL
    );
}

#[test]
fn a_registered_but_never_activated_validator_can_be_slashed() {
    let sponsor = Address::with_last_byte(0xa0);
    let seated = Address::with_last_byte(0x01);
    let offender = Address::with_last_byte(0x02);
    let offender_owner = Address::with_last_byte(0xa2);
    let reporter = Address::with_last_byte(0xb0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![seated], vec![stake], 0),
        ExitCode::Ok
    );
    let transfers = record_transfers(&harness, true);

    harness.set_caller(offender_owner);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_REGISTER_VALIDATOR,
                &RegisterValidatorCommand {
                    validator: offender,
                    commission_rate: 0,
                    initial_stake: stake,
                    bls_pubkey_uncompressed: Bytes::from(vec![
                        0x40;
                        BLS_PUBKEY_UNCOMPRESSED_LENGTH
                    ]),
                    bls_pop_uncompressed: Bytes::from(vec![0x41; BLS_POP_UNCOMPRESSED_LENGTH]),
                    peer_pubkey: B256::with_last_byte(0x42),
                },
            ))
            .0,
        ExitCode::Ok
    );
    let record = staking_storage().validators_accessor().entry(offender);
    assert_eq!(
        record.status_accessor().get_checked(&harness.sdk).unwrap(),
        STATUS_PENDING,
        "the offender never took a committee seat"
    );

    // Driven through the finalize entry point: a conflicting finalize is as
    // slashable as a conflicting notarize, and an arm is only told apart from
    // the other two by a test that reveals through it.
    let command = equivocation_report(&mut harness.sdk, &FINALIZE_ROUTE, 0x40, reporter);
    assert_eq!(
        commit_and_slash(&mut harness, &FINALIZE_ROUTE, &command).0,
        ExitCode::Ok
    );

    assert!(consensus_storage()
        .tombstoned_accessor()
        .entry(offender)
        .get_checked(&harness.sdk)
        .unwrap());
    assert_eq!(
        record.status_accessor().get_checked(&harness.sdk).unwrap(),
        STATUS_JAIL
    );
    let reward = stake * U256::from(DEFAULT_SLASH_REPORTER_REWARD_BPS) / U256::from(10_000);
    assert_eq!(
        transfers.borrow().as_slice(),
        &[(reporter, reward), (EQUIVOCATION_BURN_SINK, stake - reward)],
        "the bond of a validator that never activated is still seizable"
    );
    assert_eq!(
        staking_storage()
            .active_validators_accessor()
            .len_checked(&harness.sdk)
            .unwrap(),
        1,
        "the seated validator is untouched"
    );
}

/// The third arm carries both bindings the other two cannot cover for it: the
/// entry point decodes under `NullifyFinalize`, so the 135-byte blob would not
/// parse under either conflicting shape, and the reveal only finds its
/// commitment if the handler names the nullify/finalize proof kind.
#[test]
fn a_nullify_finalize_conflict_is_slashable_through_its_own_entry_point() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let reporter = Address::with_last_byte(0xb0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 0),
        ExitCode::Ok
    );
    let transfers = record_transfers(&harness, true);

    let command = equivocation_report(&mut harness.sdk, &NULLIFY_FINALIZE_ROUTE, 0x11, reporter);
    assert_eq!(
        commit_and_slash(&mut harness, &NULLIFY_FINALIZE_ROUTE, &command).0,
        ExitCode::Ok
    );

    assert!(consensus_storage()
        .tombstoned_accessor()
        .entry(offender)
        .get_checked(&harness.sdk)
        .unwrap());
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(offender)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_JAIL
    );
    let reward = stake * U256::from(DEFAULT_SLASH_REPORTER_REWARD_BPS) / U256::from(10_000);
    assert_eq!(
        transfers.borrow().as_slice(),
        &[(reporter, reward), (EQUIVOCATION_BURN_SINK, stake - reward)]
    );

    let logs = harness.sdk.take_logs();
    let (slashed_data, slashed_topics) =
        find_log(&logs, events::EquivocationSlashed::SELECTOR, "slash");
    assert_eq!(&slashed_topics[1].0[12..], offender.as_slice());
    assert_eq!(&slashed_topics[2].0[12..], reporter.as_slice());
    assert_eq!(decode_output::<u64>(slashed_data), CORPUS_EPOCH);
}

#[test]
fn a_slash_with_nothing_to_seize_still_tombstones() {
    let sponsor = Address::with_last_byte(0xa0);
    let seated = Address::with_last_byte(0x01);
    let offender = Address::with_last_byte(0x02);
    let reporter = Address::with_last_byte(0xb0);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![seated], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );

    // Registration always bonds the minimum, so the only route to a validator
    // with registered keys and nothing to seize is a full owner exit followed by
    // the withdrawal of the matured principal.
    harness.set_caller(offender);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_REGISTER_VALIDATOR,
                &RegisterValidatorCommand {
                    validator: offender,
                    commission_rate: 0,
                    initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                    bls_pubkey_uncompressed: Bytes::from(vec![
                        0x40;
                        BLS_PUBKEY_UNCOMPRESSED_LENGTH
                    ]),
                    bls_pop_uncompressed: Bytes::from(vec![0x41; BLS_POP_UNCOMPRESSED_LENGTH]),
                    peer_pubkey: B256::with_last_byte(0x42),
                },
            ))
            .0,
        ExitCode::Ok
    );
    staking::undelegate_from(
        &mut harness.sdk,
        offender,
        offender,
        DEFAULT_MIN_VALIDATOR_STAKE,
    )
    .unwrap();
    // The exit books at epoch 1 and the principal matures `undelegatePeriod`
    // epochs later, so epoch 8 is the first one that can withdraw it.
    harness.set_block_number(2_600);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CLAIM_DELEGATOR_FEE,
                &AddressCommand { value: offender },
            ))
            .0,
        ExitCode::Ok
    );
    // The two amounts `seize_self_stake` adds up.
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(offender)
        .entry(offender);
    assert_eq!(
        staking::delegated_amount_at(&harness.sdk, offender, offender, 8).unwrap(),
        math::U112::ZERO
    );
    assert_eq!(
        delegation
            .pending_undelegated_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        U256::ZERO
    );

    let transfers = record_transfers(&harness, true);
    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x40, reporter);
    assert_eq!(
        commit_and_slash(&mut harness, &NOTARIZE_ROUTE, &command).0,
        ExitCode::Ok
    );

    assert!(consensus_storage()
        .tombstoned_accessor()
        .entry(offender)
        .get_checked(&harness.sdk)
        .unwrap());
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(offender)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_JAIL
    );
    assert!(transfers.borrow().is_empty());
    let logs = harness.sdk.take_logs();
    assert_eq!(logs.len(), 2);
    find_log(&logs, events::ValidatorJailed::SELECTOR, "jail");
    find_log(&logs, events::EquivocationSlashed::SELECTOR, "slash");
}

#[test]
fn a_slash_naming_an_unregistered_key_is_rejected() {
    let sponsor = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let reporter = Address::with_last_byte(0xb0);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(
            sponsor,
            vec![validator],
            vec![DEFAULT_MIN_VALIDATOR_STAKE],
            0,
        ),
        ExitCode::Ok
    );

    // 0x77 compresses to a key nobody registered; the seated validator's is 0x11.
    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x77, reporter);
    assert_revert_selector(
        commit_and_slash(&mut harness, &NOTARIZE_ROUTE, &command),
        ERR_EQUIVOCATION_KEY_NOT_REGISTERED,
    );
}

#[test]
fn a_slash_whose_signatures_fail_verification_is_rejected() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let reporter = Address::with_last_byte(0xb0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender], vec![stake], 0),
        ExitCode::Ok
    );
    // The blob's signatures are well-formed and belong to a registered key, so
    // everything up to the pairing passes; only the verifier's verdict rejects.
    let transfers = record_transfers(&harness, false);

    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11, reporter);
    assert_revert_selector(
        commit_and_slash(&mut harness, &NOTARIZE_ROUTE, &command),
        ERR_EQUIVOCATION_SIGNATURE_INVALID,
    );

    // Storage is rolled back by the harness on any revert, so the seizure and the
    // penalty are read off the two channels a revert does not unwind.
    assert!(transfers.borrow().is_empty());
    assert!(harness.sdk.take_logs().is_empty());
}

#[test]
fn re_slashing_a_tombstoned_validator_is_refused() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let reporter = Address::with_last_byte(0xb0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 0),
        ExitCode::Ok
    );
    let transfers = record_transfers(&harness, true);

    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11, reporter);
    assert_eq!(
        commit_and_slash(&mut harness, &NOTARIZE_ROUTE, &command).0,
        ExitCode::Ok
    );
    let paid = transfers.borrow().clone();
    assert_eq!(paid.len(), 2);

    assert_revert_selector(
        commit_and_slash(&mut harness, &NOTARIZE_ROUTE, &command),
        ERR_ALREADY_SLASHED_FOR_EQUIVOCATION,
    );
    assert_eq!(
        transfers.borrow().as_slice(),
        paid.as_slice(),
        "the refused re-slash must not pay a second reward"
    );
    assert!(harness.sdk.take_logs().is_empty());
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
        (
            SIG_GET_MIN_VERDICT_DUE_BLOCKS,
            DEFAULT_MIN_VERDICT_DUE_BLOCKS,
        ),
        (SIG_GET_EXCLUSION_BACKOFF_CAP, DEFAULT_EXCLUSION_BACKOFF_CAP),
        (
            SIG_DEFAULT_MIN_VERDICT_DUE_BLOCKS,
            DEFAULT_MIN_VERDICT_DUE_BLOCKS,
        ),
        (
            SIG_DEFAULT_EXCLUSION_BACKOFF_CAP,
            DEFAULT_EXCLUSION_BACKOFF_CAP,
        ),
        (
            SIG_MAX_MIN_VERDICT_DUE_BLOCKS,
            DEFAULT_MIN_VERDICT_DUE_BLOCKS,
        ),
    ] {
        let (exit, output) = harness.call(encode_empty_call(selector));
        assert_eq!(exit, ExitCode::Ok);
        assert_eq!(decode_output::<u32>(&output), expected);
    }

    let logs = harness.sdk.take_logs();
    let disabled_data = &logs
        .iter()
        .find(|(_, topics)| {
            topics.first()
                == Some(&B256::new(
                    events::ProductionLivenessDisabledChanged::SELECTOR,
                ))
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
    // The bound is driven by literals. Written as `CONSTANT + 1` the input tracks
    // whatever the constant is set to, the assertion can never catch a bound that
    // has drifted, and the edit passes review unseen.
    assert_eq!(DEFAULT_MIN_VERDICT_DUE_BLOCKS, 100);
    assert_revert_selector(
        harness.call(encode_call(
            SIG_SET_MIN_VERDICT_DUE_BLOCKS,
            &U32Command { value: 101 },
        )),
        ERR_MIN_VERDICT_DUE_BLOCKS_TOO_HIGH,
    );
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_MIN_VERDICT_DUE_BLOCKS,
                &U32Command { value: 100 },
            ))
            .0,
        ExitCode::Ok,
        "governance may lower the floor and may never raise it past the default"
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
    assert_eq!(decode_output::<u32>(&output), 100);
    let (_, output) = harness.call(encode_empty_call(SIG_GET_EXCLUSION_BACKOFF_CAP));
    assert_eq!(
        decode_output::<u32>(&output),
        DEFAULT_EXCLUSION_BACKOFF_CAP,
        "the refused zero must leave the seeded ladder cap untouched"
    );
    let (_, output) = harness.call(encode_empty_call(SIG_GET_PRODUCTION_LIVENESS_DISABLED));
    assert!(!decode_output::<bool>(&output));

    // Only zero is out of bounds for the ladder cap, so any other value is a
    // legal move and 64 is one the seed is not already sitting on.
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_EXCLUSION_BACKOFF_CAP,
                &U32Command { value: 64 },
            ))
            .0,
        ExitCode::Ok
    );
    let (_, output) = harness.call(encode_empty_call(SIG_GET_EXCLUSION_BACKOFF_CAP));
    assert_eq!(
        decode_output::<u32>(&output),
        64,
        "governance keeps control of the ceiling: the setter persists what the getter reads back"
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
        staking::selected_addresses_at(&harness.sdk, 1).unwrap(),
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
        staking::selected_addresses_at(&harness.sdk, 0).unwrap(),
        vec![first, second]
    );
    assert_eq!(
        staking::selected_addresses_at(&harness.sdk, 1).unwrap(),
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
        staking::selected_addresses_at(&harness.sdk, 1).unwrap(),
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
fn a_second_status_transition_does_not_rewrite_the_epoch_before_the_first() {
    let owner = Address::with_last_byte(0xa0);
    let keeper = Address::with_last_byte(0x01);
    let subject = Address::with_last_byte(0x02);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(
            owner,
            vec![keeper],
            vec![DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2)],
            0,
        ),
        ExitCode::Ok
    );

    // A freshly registered validator is Pending and invisible, so the two
    // transitions below are the first two this record ever holds.
    harness.set_caller(subject);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_REGISTER_VALIDATOR,
                &RegisterValidatorCommand {
                    validator: subject,
                    commission_rate: 0,
                    initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                    bls_pubkey_uncompressed: Bytes::from(vec![
                        0x33;
                        BLS_PUBKEY_UNCOMPRESSED_LENGTH
                    ]),
                    bls_pop_uncompressed: Bytes::from(vec![0x44; BLS_POP_UNCOMPRESSED_LENGTH]),
                    peer_pubkey: B256::with_last_byte(9),
                },
            ))
            .0,
        ExitCode::Ok
    );
    harness.set_caller(GENESIS_GOVERNANCE);

    // Epoch 1: the first transition, visible from epoch 2 onward.
    harness.set_block_number(1_200);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_ACTIVATE_VALIDATOR,
                &AddressCommand { value: subject },
            ))
            .0,
        ExitCode::Ok
    );
    assert!(!staking::selection_visible_at(&harness.sdk, subject, 1).unwrap());
    assert!(staking::selection_visible_at(&harness.sdk, subject, 2).unwrap());

    // Epoch 2: the second transition. With a single history slot this stamp
    // overwrote the pending-era `false` with the activation's `true`, and every
    // epoch below the activation started answering `true` — including epoch 1,
    // which a committed committee still reads two selection epochs later.
    harness.set_block_number(1_400);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_DISABLE_VALIDATOR,
                &AddressCommand { value: subject },
            ))
            .0,
        ExitCode::Ok
    );
    assert!(
        !staking::selection_visible_at(&harness.sdk, subject, 1).unwrap(),
        "the epoch before the first transition must read the same after the second"
    );
    assert!(
        staking::selection_visible_at(&harness.sdk, subject, 2).unwrap(),
        "the activation epoch keeps the value it was seated with"
    );
    assert!(!staking::selection_visible_at(&harness.sdk, subject, 3).unwrap());

    // Epoch 3: two stamps inside one epoch close are one transition, not two,
    // so the oldest recorded segment is not pushed out to pay for the pair.
    harness.set_block_number(1_600);
    for selector in [SIG_ACTIVATE_VALIDATOR, SIG_DISABLE_VALIDATOR] {
        assert_eq!(
            harness
                .call(encode_call(selector, &AddressCommand { value: subject }))
                .0,
            ExitCode::Ok
        );
    }
    assert!(
        staking::selection_visible_at(&harness.sdk, subject, 2).unwrap(),
        "collapsing the same-epoch pair keeps the activation epoch in the history"
    );
    assert!(!staking::selection_visible_at(&harness.sdk, subject, 3).unwrap());
    assert!(!staking::selection_visible_at(&harness.sdk, subject, 4).unwrap());
}

#[test]
fn production_liveness_views_read_the_new_namespace() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let stranger = Address::with_last_byte(0x0f);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );
    commit_test_committee(
        &mut harness.sdk,
        4,
        &[(validator, DEFAULT_MIN_VALIDATOR_STAKE)],
    );

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
    pin_test_stipend_rate(sdk, epoch);
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

fn production_record(sdk: &TestingContextImpl, validator: Address) -> (u64, u64, u64, u64, u32) {
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

// Judging is the one leg the kill switch holds — that releases run regardless is
// covered elsewhere. Suppression is only observable on a COMPLETE epoch: a partial
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

// Both predicates are cross-multiplied against the frozen weights: a member
// whose due share falls under the floor holds no verdict at all, and a member
// producing exactly half its due passes.
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

// The floor is a share threshold wearing block units: a member is judged once its
// stake share reaches `floor / epochBlockInterval`. Every other judging test runs
// at an interval of 200 with the floor lowered to 10 — a 5 % share, a regime no
// real chain is in. This one runs the shipped default of 100 against a one-day
// epoch at one block per second, where the same parameter means about 0.12 %.
//
// Both members produce nothing, so whichever is judged must fail. The other's
// clean record is therefore the skip itself and not a pass — with zero produced
// there is no verdict that leaves no mark.
#[test]
fn the_verdict_floor_is_a_stake_share_at_the_production_epoch_length() {
    let owner = Address::with_last_byte(0xa0);
    let heavy = Address::with_last_byte(0x01);
    let light = Address::with_last_byte(0x02);
    let unit = DEFAULT_MIN_VALIDATOR_STAKE;
    let interval: u32 = 86_400;
    // 99.99 % and 0.01 %, against a threshold of 100 / 86_400.
    let heavy_weight = unit * U256::from(9_999);
    let light_weight = unit;

    // A zero activation block means "configurable but not running", and the epoch
    // then never advances off zero. Arming it one full epoch in keeps the
    // activation aligned to the interval.
    let mut harness = Harness::new(u64::from(interval));
    let mut command = harness.initialize_command(
        owner,
        vec![heavy, light],
        vec![heavy_weight, light_weight],
        0,
    );
    command.epoch_block_interval = interval;
    command.active_validators_length = 2;
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

    for epoch in 0..2 {
        commit_test_committee(
            &mut harness.sdk,
            epoch,
            &[(heavy, heavy_weight), (light, light_weight)],
        );
    }
    assert_eq!(
        chain_config_storage()
            .min_verdict_due_blocks_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        DEFAULT_MIN_VERDICT_DUE_BLOCKS,
        "the point of this test is the shipped floor, not a lowered one"
    );
    // Seeded rather than recorded block by block, so a day-long epoch is free.
    seed_epoch_production(&mut harness.sdk, 0, &[0, 0], interval);
    harness.sdk.take_logs();

    let boundary = u64::from(interval) * 2;
    production_liveness_storage()
        .last_processed_block_accessor()
        .set_checked(&mut harness.sdk, boundary - 1)
        .unwrap();
    harness.set_block_number(boundary);
    assert_eq!(record_production(&mut harness, boundary, 0), ExitCode::Ok);

    let logs = harness.sdk.take_logs();
    assert!(
        logs_of(&logs, events::PartialEpoch::SELECTOR).is_empty(),
        "a tainted epoch is never judged, which would make the rest vacuous"
    );
    let failed = logs_of(&logs, events::ProductionVerdictFailed::SELECTOR);
    assert_eq!(failed.len(), 1);
    assert_eq!(&failed[0].1[2].0[12..], heavy.as_slice());
    assert_eq!(production_record(&harness.sdk, heavy).2, 1);
    assert_eq!(
        production_record(&harness.sdk, light).2,
        0,
        "under the share threshold the member is skipped entirely"
    );
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
        assert_eq!(
            record.2, 1,
            "the failure bit is written on the guarded path"
        );
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

// The kill switch stops the tier punishing; it does not stop the clock. Gating
// the release leg on it would freeze the stamp while `current` ran past
// `readmit_at_epoch`, silently lengthening the exclusion — and `activate_validator`
// refuses to rescue a validator that still carries an outstanding stamp, so the
// seat would be stranded until governance turned the tier back on. Releases
// therefore run whether the tier is on or off; judging is the only leg held.
//
// Both halves are asserted here on purpose: dropping either gate must fail this
// test, and asserting only the release would pass equally if someone deleted both.
#[test]
fn the_kill_switch_suspends_judging_but_never_releases() {
    let token = DEFAULT_MIN_VALIDATOR_STAKE;
    let (mut harness, members) = liveness_harness(&[token * U256::from(50); 4], 2);
    set_min_verdict_due_blocks(&mut harness, 10);
    // A complete epoch 1 with one member under half its due, so the judging leg
    // has a verdict to suppress rather than nothing to do.
    commit_test_committee(
        &mut harness.sdk,
        1,
        &[
            (members[0], token * U256::from(49)),
            (members[1], token * U256::from(25)),
            (members[2], token * U256::from(25)),
            (members[3], token),
        ],
    );
    seed_epoch_production(&mut harness.sdk, 1, &[151, 24, 25, 0], 200);

    // Excluded at epoch 0, due back at the close that lands on epoch 2.
    let excluded = members[3];
    assert!(staking::apply_production_exclusion(&mut harness.sdk, excluded).unwrap());
    let record = production_liveness_storage()
        .validators_accessor()
        .entry(excluded);
    record
        .readmit_at_epoch_accessor()
        .set_checked(&mut harness.sdk, 2)
        .unwrap();
    production_liveness_storage()
        .pending_exclusions_accessor()
        .push_checked(&mut harness.sdk, excluded)
        .unwrap();
    assert!(!staking::selection_visible_at(&harness.sdk, excluded, 2).unwrap());

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

    // Closes epoch 1 from the first block of epoch 2, so `current` has reached
    // the readmit epoch with the tier off.
    assert_eq!(close_epoch_via_record(&mut harness, 1), ExitCode::Ok);

    assert!(
        pending_exclusion_set(&harness.sdk).is_empty(),
        "an expiring exclusion is released with the tier off, not held"
    );
    assert_eq!(
        record
            .readmit_at_epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "and its stamp is cleared, so the validator is reachable again"
    );
    assert!(staking::selection_visible_at(&harness.sdk, excluded, 3).unwrap());

    let logs = harness.sdk.take_logs();
    assert_eq!(
        logs_of(&logs, events::ProductionExclusionReleased::SELECTOR).len(),
        1
    );
    assert!(
        logs_of(&logs, events::ProductionVerdictFailed::SELECTOR).is_empty(),
        "judging stays suspended: a complete epoch draws no verdict while off"
    );
    assert!(
        logs_of(&logs, events::ProductionExclusionApplied::SELECTOR).is_empty(),
        "and no new stamp is issued"
    );
    assert_eq!(
        production_record(&harness.sdk, members[1]).2,
        0,
        "nor is a failure recorded against the member that would have failed"
    );
}

struct CloseCallState {
    /// What the funding source holds. Sized so the catch-up runs out of money
    /// partway through, which is how a leg is made to fail after it has written.
    balance: U256,
    pulled: Vec<U256>,
    self_call_fuel: Option<u64>,
    self_calls: usize,
}

fn token_reply(input: &[u8], state: &Rc<RefCell<CloseCallState>>) -> SyscallResult<Bytes> {
    let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
    if selector != SIG_ERC20_TRANSFER_FROM {
        return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic);
    }
    let (_, _, amount) =
        SolidityABI::<(Address, Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0).unwrap();
    let mut state = state.borrow_mut();
    if amount > state.balance {
        return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic);
    }
    state.balance -= amount;
    state.pulled.push(amount);
    SyscallResult::new(encode_mock_return(&true), 0, 0, ExitCode::Ok)
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
                return token_reply(input, &state);
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
            nested
                .set_call_handler(move |_address, _value, input, _fuel| token_reply(input, &inner));
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
        // Exactly one epoch's pot: epoch 0 settles, epoch 1 dies inside the same
        // self-call, so the discarded frame is one that had already written.
        balance: U256::from(400),
        pulled: Vec::new(),
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
        state.borrow().pulled,
        vec![U256::from(400)],
        "the mock records the pull; on the real runtime the token transfer for epoch 0 is discarded with the frame"
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
