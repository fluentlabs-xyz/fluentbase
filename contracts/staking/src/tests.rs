use super::*;
use crate::{
    consensus::{committee_changed, CommitteeMember},
    consts::{STATUS_ACTIVE, STATUS_JAIL, STATUS_PENDING},
    storage::{
        chain_config_storage, consensus_storage, initializer_storage, production_liveness_storage,
        staking_storage, ConsensusKeysStorage, DelegationOpStorage, EpochIndexStorage,
        ProductionValidatorStorage, UndelegationOpStorage, ValidatorSnapshotStorage,
        ValidatorStorage, WeightPairStorage,
    },
    types::{
        AddressAmountCommand, AddressCommand, AddressU16Command, BoolCommand, ConsensusKeys,
        EpochSignerCommand, EquivocationCommand, InitializeCommand, RecordProductionCommand,
        RegisterValidatorCommand, U256Command, U32Command, U64Command, ValidatorBlockCommand,
        ValidatorDelegatorCommand, ValidatorEpochCommand,
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

    // The whole ring design rests on this fitting one slot: two `uint112`
    // weights and a `uint32` stamp are exactly 32 bytes, so the stamp rides free
    // AND shares the store with the weights it vouches for. At 33 the stamp
    // would cost a second slot and a torn state would become representable.
    assert_eq!(WeightPairStorage::SLOTS, 1);
    assert_eq!(<WeightPairStorage as StorageLayout>::BYTES, 32);
    let pair = WeightPairStorage::new(slot, 0);
    assert_eq!(pair.a_accessor().slot(), slot);
    assert_eq!(pair.b_accessor().slot(), slot);
    assert_eq!(pair.stamp_accessor().slot(), slot);
    assert_eq!(pair.a_accessor().offset(), 18);
    assert_eq!(pair.b_accessor().offset(), 4);
    assert_eq!(pair.stamp_accessor().offset(), 0);

    // Both halves in one slot, so writing an epoch's index is one store.
    assert_eq!(EpochIndexStorage::SLOTS, 1);
    assert_eq!(<EpochIndexStorage as StorageLayout>::BYTES, 8);

    // Nothing on the per-block path writes this record any more, and its three
    // remaining fields are 20 bytes: the whole record is one slot, so an
    // epoch-close write to any of them costs a single store.
    assert_eq!(ProductionValidatorStorage::SLOTS, 1);
    let production = ProductionValidatorStorage::new(slot, 0);
    assert_eq!(production.last_failed_epoch_p1_accessor().slot(), slot);
    assert_eq!(production.last_failed_epoch_p1_accessor().offset(), 24);
    assert_eq!(production.readmit_at_epoch_accessor().slot(), slot);
    assert_eq!(production.readmit_at_epoch_accessor().offset(), 16);
    assert_eq!(production.kick_count_accessor().slot(), slot);
    assert_eq!(production.kick_count_accessor().offset(), 12);
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
        reset_bls_precompiles();
        sdk.set_call_handler(|address, _value, input, _fuel_limit| {
            if let Some(reply) = mock_precompile_reply(address, input) {
                return reply;
            }
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

/// The committee `epoch` would seat, as addresses in the order the stake cut
/// produced. Reads the one selection that survives; the deleted
/// `selected_addresses_at` returned the same shape off the retired one.
fn selected_at(sdk: &TestingContextImpl, epoch: u64) -> Vec<Address> {
    consensus::selected_committee_at(sdk, epoch)
        .unwrap()
        .into_iter()
        .map(|member| member.validator)
        .collect()
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

/// Writes an epoch committee the way `commitEpochCommittee` does: a membership
/// record keyed by the epoch that mints it, an index entry pointing at it, and
/// the frozen leader weights in that epoch's ring frame.
fn commit_test_committee(sdk: &mut TestingContextImpl, epoch: u64, members: &[(Address, U256)]) {
    let consensus = consensus_storage();
    let record = consensus.committee_records_accessor().entry(epoch);
    for (validator, _) in members {
        record
            .grow_checked(sdk)
            .unwrap()
            .set_checked(sdk, *validator)
            .unwrap();
    }
    let index = consensus.epoch_index_accessor().entry(epoch);
    index
        .record_accessor()
        .set_checked(sdk, epoch as u32)
        .unwrap();
    index
        .length_accessor()
        .set_checked(sdk, members.len() as u32)
        .unwrap();
    let ring = consensus.weight_ring_accessor();
    let base = (epoch % WEIGHT_RING_EPOCHS) as usize * PAIRS_MAX;
    let mut i = 0;
    while i < members.len() {
        let pair = ring.at(base + i / 2);
        pair.a_accessor()
            .set_checked(sdk, crate::math::compact_balance(members[i].1).unwrap())
            .unwrap();
        let b = match members.get(i + 1) {
            Some((_, stake)) => crate::math::compact_balance(*stake).unwrap(),
            None => fluentbase_sdk::Uint::ZERO,
        };
        pair.b_accessor().set_checked(sdk, b).unwrap();
        pair.stamp_accessor()
            .set_checked(sdk, epoch as u32)
            .unwrap();
        i += 2;
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

/// Seed an epoch's block counter and accrue it the way `close_epoch` does.
///
/// The close is what decides and records the epoch's payout, so seeding the
/// counters without it leaves an epoch that settlement defers rather than pays.
/// Accruing inside the seeding helpers rather than at their call sites is what
/// keeps the stand-in from drifting away from the production path again — the
/// callers must only make sure the rate and the committee are in place *before*
/// they seed.
fn record_test_production(sdk: &mut TestingContextImpl, epoch: u64, blocks: u32) {
    production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(epoch)
        .set_checked(sdk, blocks)
        .unwrap();
    staking::accrue_epoch(sdk, epoch, blocks).unwrap();
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
    /// Plain `transfer` calls, which spend the contract's OWN balance rather
    /// than the reserve's. Kept apart from `pulls` because the whole point of
    /// the split is that these two are different money.
    transfers: Vec<(Address, U256)>,
}

/// The 96-byte zcash key that a `byte`-filled 256-byte EIP-2537 G2 really
/// compresses to.
///
/// Both halves of `x` are that byte, so the reference is the byte repeated; the
/// leading byte also carries the compression flag and the y-sign. Every fill
/// byte in this file is above the `0x0d` that leads `(p-1)/2`, so the sign is
/// always set — asserted rather than assumed, because a lower fill byte would
/// make this silently wrong.
fn compressed_key_of(byte: u8) -> Vec<u8> {
    assert!(byte > 0x0d, "fill byte {byte:#04x} is not above (p-1)/2");
    let mut key = vec![byte; BLS_PUBKEY_LENGTH];
    key[0] = byte | 0x80 | 0x20;
    key
}

/// A 128-byte EIP-2537 G1 whose UNCHECKED compression is exactly `compressed`.
///
/// Not a curve point, and it does not need to be: the premise of
/// `compress_g1_unchecked` is that it never asks. `x` is the reference itself,
/// flag bits included, because compression only ORs those bits back in; `y`
/// picks the half of the field that reproduces the reference's sign bit.
fn g1_preimage_of(compressed: &[u8]) -> Bytes {
    assert_eq!(compressed.len(), BLS_SIGNATURE_LENGTH);
    let mut point = vec![0u8; BLS_POP_UNCOMPRESSED_LENGTH];
    point[16..64].copy_from_slice(compressed);
    if compressed[0] & 0x20 != 0 {
        point[80..].fill(0xff);
    }
    Bytes::from(point)
}

fn encode_mock_return<T>(value: &T) -> Bytes
where
    T: fluentbase_sdk::codec::Encoder<fluentbase_sdk::byteorder::BE, 32, true, false>,
{
    let mut output = BytesMut::new();
    SolidityABI::<T>::encode(value, &mut output, 0).unwrap();
    output.freeze().into()
}

// ── stand-ins for the five precompiles the inlined verifier calls ──────────
//
// The verifier is no longer an address this contract stores, so there is no
// contract left to mock: `bls.rs` static-calls `0x02`, `0x05`, `0x0b`, `0x0f`
// and `0x10` directly. The substitution point moved one floor down, and so does
// the stub.
//
// SHA-256 is honest — `crypto_sha256` is the same implementation the `0x02`
// predeploy runs (`contracts/sha256`) — so the bytes recorded below are the
// bytes the contract really hashed, and the namespace read back out of them is
// really the namespace it signed under. MODEXP, MAP_FP_TO_G1 and G1ADD return
// deterministic values of the correct width and nothing more; reducing mod p
// and mapping to a curve point are not things a unit harness does.
//
// PAIRING is a POLICY, not a pairing. It answers "the equation holds" unless a
// test says otherwise, and that switch is the whole point: the retired
// external-verifier stub answered `true` unconditionally, so
// `InvalidProofOfPossession` and the verify half of
// `EquivocationSignatureInvalid` were unreachable from any test in this file.
// It decides nothing about BLS12-381. That claim is made in
// `e2e/src/staking.rs` against the real predeploys, with vectors the node's
// blst produced, and again on the devnet.
const PRECOMPILE_SHA256: Address = Address::with_last_byte(0x02);
const PRECOMPILE_MODEXP: Address = Address::with_last_byte(0x05);
const PRECOMPILE_G1ADD: Address = Address::with_last_byte(0x0b);
const PRECOMPILE_PAIRING: Address = Address::with_last_byte(0x0f);
const PRECOMPILE_MAP_FP_TO_G1: Address = Address::with_last_byte(0x10);

#[derive(Default)]
struct BlsPrecompiles {
    /// `None` accepts every pairing. `Some(list)` accepts only a verify whose
    /// hashed preimage carried one of these namespaces; `Some(empty)` accepts
    /// nothing, which is how a forged signature is spelled here.
    accept: Option<Vec<Vec<u8>>>,
    /// A precompile told to answer one byte short of its fixed width, which is
    /// one of the two ways `PrecompileFailed` is reachable.
    truncate: Option<Address>,
    /// A precompile told to refuse the call outright — the other way.
    refuse: Option<Address>,
    /// A precompile told to answer the right width, all zeroes — which for a
    /// point is the EIP-2537 encoding of infinity.
    zero: Option<Address>,
    /// The namespace of the verify in flight, recovered from the
    /// `expand_message_xmd` preimage rather than from an ABI argument — after
    /// the inline there is no argument to read.
    current_namespace: Vec<u8>,
    /// One entry per pairing, in order.
    seen: Rc<RefCell<Vec<Bytes>>>,
}

thread_local! {
    static BLS_PRECOMPILES: RefCell<BlsPrecompiles> = RefCell::new(BlsPrecompiles::default());
}

/// Cleared per `Harness`, because several tests build one per loop iteration and
/// a namespace list left over from the previous one would silently decide the
/// next.
fn reset_bls_precompiles() {
    BLS_PRECOMPILES.with(|mock| *mock.borrow_mut() = BlsPrecompiles::default());
}

/// Deterministic filler of an exact width. Not a curve point and not pretending
/// to be one; what matters is that it is a function of the whole input, so a
/// changed preimage changes everything downstream of it.
fn mock_precompile_output(tag: u8, input: &[u8], length: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(length + 32);
    let mut counter = 0u8;
    while out.len() < length {
        let mut buffer = Vec::with_capacity(input.len() + 2);
        buffer.push(tag);
        buffer.push(counter);
        buffer.extend_from_slice(input);
        out.extend_from_slice(fluentbase_sdk::crypto::crypto_sha256(&buffer).as_slice());
        counter = counter.wrapping_add(1);
    }
    out.truncate(length);
    out
}

/// `Z(64×0x00) ‖ I2OSP(len(ns),1) ‖ ns ‖ msg ‖ I2OSP(128,2) ‖ 0x00 ‖ DST'` is the
/// only SHA-256 input of a verify that begins with a full zero block — the four
/// `b_i` blocks begin with a digest and are 77 bytes wide. Returns the namespace
/// it carries.
fn namespace_from_xmd_preimage(input: &[u8]) -> Option<Vec<u8>> {
    if input.len() <= 65 || input[..64].iter().any(|byte| *byte != 0) {
        return None;
    }
    let length = input[64] as usize;
    input.get(65..65 + length).map(<[u8]>::to_vec)
}

/// Answers a call addressed to one of the five precompiles, or `None` if the
/// call is not one — every handler in this file forwards to it first, because
/// any path that verifies a key now makes these calls.
fn mock_precompile_reply(address: Address, input: &[u8]) -> Option<SyscallResult<Bytes>> {
    let (tag, width) = match address {
        PRECOMPILE_SHA256 => (0x02, 32),
        PRECOMPILE_MODEXP => (0x05, 48),
        PRECOMPILE_G1ADD => (0x0b, 128),
        PRECOMPILE_MAP_FP_TO_G1 => (0x10, 128),
        PRECOMPILE_PAIRING => (0x0f, 32),
        _ => return None,
    };
    if BLS_PRECOMPILES.with(|mock| mock.borrow().refuse == Some(address)) {
        return Some(SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic));
    }
    let truncated = BLS_PRECOMPILES.with(|mock| mock.borrow().truncate == Some(address));
    let width = if truncated { width - 1 } else { width };

    let data = match address {
        PRECOMPILE_SHA256 => {
            if let Some(namespace) = namespace_from_xmd_preimage(input) {
                BLS_PRECOMPILES.with(|mock| mock.borrow_mut().current_namespace = namespace);
            }
            let mut digest = fluentbase_sdk::crypto::crypto_sha256(input).to_vec();
            digest.truncate(width);
            digest
        }
        PRECOMPILE_PAIRING => BLS_PRECOMPILES.with(|mock| {
            let mock = mock.borrow_mut();
            let namespace = mock.current_namespace.clone();
            let holds = match &mock.accept {
                None => true,
                Some(accepted) => accepted.iter().any(|entry| entry == &namespace),
            };
            mock.seen.borrow_mut().push(Bytes::from(namespace));
            let mut word = vec![0u8; width];
            if holds && !word.is_empty() {
                word[width - 1] = 1;
            }
            word
        }),
        _ if BLS_PRECOMPILES.with(|mock| mock.borrow().zero == Some(address)) => vec![0u8; width],
        _ => mock_precompile_output(tag, input, width),
    };
    Some(SyscallResult::new(Bytes::from(data), 0, 0, ExitCode::Ok))
}

/// The staking token as every harness sees it, so a test that needs its own
/// handler can record the call and still answer it here.
fn mock_external_return(selector: u32, args: &[u8]) -> Option<Bytes> {
    let _ = args;
    match selector {
        SIG_ERC20_TRANSFER_FROM | SIG_ERC20_TRANSFER => Some(encode_mock_return(&true)),
        // A reserve that can always cover the epoch. The close asks before it
        // prices an epoch, and a harness that has nothing to say about funding
        // must not accidentally answer "empty" — that would forfeit the epoch
        // and quietly zero every reward assertion downstream. Tests about
        // funding install their own handler.
        SIG_ERC20_BALANCE_OF | SIG_ERC20_ALLOWANCE => Some(encode_mock_return(&U256::MAX)),
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
/// approved this contract for it; anything else fails the call, and a claim that
/// cannot be funded reverts rather than paying short.
///
/// `balanceOf` and `allowance` answer off the same two numbers, because the
/// close reads them to decide whether the epoch is affordable at all and the
/// two answers have to be the same reserve. A `reports_failure` token refuses
/// the reads as well as the pull: that is the "could not read the reserve" arm,
/// which the close is required to treat as zero rather than as an error.
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
        transfers: Vec::new(),
    }));
    let state = funding.clone();
    sdk.set_call_handler(move |address, _value, input, _fuel_limit| {
        if let Some(reply) = mock_precompile_reply(address, input) {
            return reply;
        }
        if input.len() < SIG_LEN_BYTES {
            return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams);
        }
        let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
        if address != STAKING_TOKEN {
            return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic);
        }
        if selector == SIG_ERC20_BALANCE_OF || selector == SIG_ERC20_ALLOWANCE {
            let funding = state.borrow();
            if funding.reports_failure {
                return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic);
            }
            // Answers ABOUT THE SOURCE, and about nobody else. An
            // address-blind mock would pass a contract that asked the wrong
            // party — its own balance, say, which holds everyone's deposits and
            // would read as stipend coverage it is not.
            let value = if selector == SIG_ERC20_BALANCE_OF {
                let (holder,) =
                    SolidityABI::<(Address,)>::decode(&&input[SIG_LEN_BYTES..], 0).unwrap();
                if holder == funding.source {
                    funding.balance
                } else {
                    U256::ZERO
                }
            } else {
                let (holder, spender) =
                    SolidityABI::<(Address, Address)>::decode(&&input[SIG_LEN_BYTES..], 0).unwrap();
                if holder == funding.source && spender == GENESIS_STAKING {
                    funding.allowance
                } else {
                    U256::ZERO
                }
            };
            return SyscallResult::new(encode_mock_return(&value), 0, 0, ExitCode::Ok);
        }
        if selector == SIG_ERC20_TRANSFER {
            // The contract paying out of its own balance. The reserve's two
            // numbers say nothing about it — deposits are not the stipend — so
            // this succeeds however dry the reserve is.
            let (recipient, amount) =
                SolidityABI::<(Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0).unwrap();
            let mut funding = state.borrow_mut();
            funding.transfers.push((recipient, amount));
            if funding.reports_failure {
                return SyscallResult::new(encode_mock_return(&false), 0, 0, ExitCode::Ok);
            }
            return SyscallResult::new(encode_mock_return(&true), 0, 0, ExitCode::Ok);
        }
        if selector != SIG_ERC20_TRANSFER_FROM {
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
    stipend_test_sdk_at_rate(U256::from(100), balance, allowance)
}

fn stipend_test_sdk_at_rate(
    rate: U256,
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
        .set_checked(&mut harness.sdk, rate)
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
    // The token goes in BEFORE the accrual, not after. The close reads the
    // reserve to decide whether it can price the epoch at all, so a fixture that
    // installed the token afterwards would accrue epoch 0 against the permissive
    // default mock and quietly ignore the two numbers it was handed.
    let funding = install_stipend_token(&harness.sdk, source, balance, allowance);
    // Accrues epoch 0. Nothing settles it — the reserve pays each claim
    // directly, so there is no second act between the close and the claim.
    record_test_production(&mut harness.sdk, 0, DEFAULT_EPOCH_BLOCK_INTERVAL as u32);
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL);
    harness.set_caller(SYSTEM_CALLER);
    harness.sdk.take_logs();

    (harness, funding, validator)
}

fn assert_epoch_accrual_event(sdk: &TestingContextImpl, epoch: u64, assigned: U256) {
    let logs = sdk.take_logs();
    assert_eq!(logs.len(), 1);
    let (data, topics) = &logs[0];
    assert_eq!(
        topics[0],
        keccak256(events::EpochBlendRewardsCommitted::SIGNATURE.as_bytes())
    );
    assert_eq!(
        SolidityABI::<u64>::decode(&topics[1].as_slice(), 0).unwrap(),
        epoch
    );
    assert_eq!(decode_output::<U256>(data), assigned);
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
    // "slashEquivocationNotarize(bytes,bytes,bytes,bytes)"
    // 0x01 0x0203 0x04 0x0506
    let slash_equivocation = hex!(
        "e28d2f63
         0000000000000000000000000000000000000000000000000000000000000080
         00000000000000000000000000000000000000000000000000000000000000c0
         0000000000000000000000000000000000000000000000000000000000000100
         0000000000000000000000000000000000000000000000000000000000000140
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
        ERR_INVALID_EVIDENCE_ENCODING,
    );
    let command = consensus::decode_equivocation(&slash_equivocation[4..]).unwrap();
    assert_eq!(&command.evidence[..], &[0x01]);
    assert_eq!(&command.pk_uncompressed[..], &[0x02, 0x03]);
    assert_eq!(&command.sig1_uncompressed[..], &[0x04]);
    assert_eq!(&command.sig2_uncompressed[..], &[0x05, 0x06]);
}

#[test]
fn register_validator_cast_calldata_registers_consensus_keys_atomically() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    let calls = Rc::new(RefCell::new(Vec::new()));
    let recorded_calls = calls.clone();
    let pulls = Rc::new(RefCell::new(Vec::new()));
    let recorded_pulls = pulls.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            if let Some(reply) = mock_precompile_reply(address, input) {
                return reply;
            }
            let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
            if selector == SIG_ERC20_TRANSFER_FROM {
                let (from, to, amount) =
                    decode_output::<(Address, Address, U256)>(&input[SIG_LEN_BYTES..]);
                recorded_pulls
                    .borrow_mut()
                    .push((address, from, to, amount));
                return SyscallResult::new(encode_mock_return(&true), 0, 0, ExitCode::Ok);
            }
            // Anything that is neither a precompile nor the token: there is no
            // third callee left, and recording it is how this test would notice
            // one coming back.
            recorded_calls.borrow_mut().push((address, input.to_vec()));
            SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams)
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

    // The verifier used to be two calls to a stored address. Registration now
    // reaches nothing outside this contract but the BLEND token and the fixed
    // precompiles, and there is no address in storage that could redirect it.
    assert_eq!(*calls.borrow(), Vec::new());
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
        Bytes::from(compressed_key_of(0x11))
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
            bls_pubkey: Bytes::from(compressed_key_of(0x11)),
            peer_pubkey: B256::with_last_byte(1),
            activation_epoch: 0,
        }
    );

    // The two-vector return shape, on the ONE handler that still has it. This
    // assertion used to ride on `getValidatorsWithKeys`, deleted 2026-09-08 for
    // having no consumer; both handlers share `write_validators_with_keys`, so
    // moving it here is what keeps that encoder covered. Without it a
    // `getRegistryWithKeys` that answers two empty vectors passes the suite.
    let (status, multi_value_output) = harness.call(encode_empty_call(SIG_GET_REGISTRY_WITH_KEYS));
    assert_eq!(status, ExitCode::Ok);
    assert_eq!(
        decode_returns::<(Vec<Address>, Vec<ConsensusKeys>)>(&multi_value_output),
        (
            vec![validator],
            vec![ConsensusKeys {
                bls_pubkey: Bytes::from(compressed_key_of(0x11)),
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
    let (_, output) = harness.call(encode_call(
        SIG_SET_BLEND_RESERVE,
        &AddressCommand {
            value: Address::ZERO,
        },
    ));
    assert_eq!(&output[..4], &ERR_ZERO_VALUE.to_be_bytes());
    assert_eq!(decode_output::<String>(&output[4..]), "blendReserve");
}

#[test]
fn derived_selectors_match_independent_hex_pins() {
    for (actual, pinned) in [
        (SIG_INITIALIZE, 0xfecaf0f1),
        (SIG_CURRENT_EPOCH, 0x76671808),
        (SIG_NEXT_EPOCH, 0xaea0e78b),
        (SIG_GET_STAKING_TOKEN, 0x9f9106d1),
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
        (SIG_SET_ACTIVE_VALIDATORS_LENGTH, 0xc227a412),
        (SIG_SET_EPOCH_BLOCK_INTERVAL, 0xaf70fa2c),
        (SIG_SET_DPOS_ACTIVATION_BLOCK, 0xf517ca6a),
        (SIG_SET_SLASH_FUND_ADDRESS, 0xa79e7263),
        (SIG_SET_BLEND_STIPEND_PER_EPOCH, 0x2c91b879),
        (SIG_SET_UNDELEGATE_PERIOD, 0x41d8a080),
        (SIG_SET_MIN_VALIDATOR_STAKE_AMOUNT, 0xe1a2e863),
        (SIG_SET_MIN_STAKING_AMOUNT, 0x612d669e),
        (SIG_GET_BLEND_RESERVE, 0x37dff538),
        (SIG_SET_BLEND_RESERVE, 0x7899ae8f),
        (SIG_GET_VALIDATOR_FEE, 0x457179fd),
        (SIG_CLAIM_VALIDATOR_FEE_AT_EPOCH, 0xadf2a79c),
        (SIG_GET_DELEGATOR_FEE, 0x52b7bea2),
        (SIG_GET_DELEGATOR_PRINCIPAL, 0xa789083d),
        (SIG_WITHDRAW_DELEGATOR_PRINCIPAL, 0xe75f359c),
        (SIG_ERC20_BALANCE_OF, 0x70a08231),
        (SIG_ERC20_ALLOWANCE, 0xdd62ed3e),
        (SIG_COMMIT_EPOCH_COMMITTEE, 0xe505b249),
        (SIG_GET_EPOCH_COMMITTEE_WITH_STAKES, 0xa4d160c1),
        (SIG_SLASH_EQUIVOCATION, 0xdc6fb3f2),
        (SIG_SLASH_EQUIVOCATION_NOTARIZE, 0xe28d2f63),
        (SIG_SLASH_EQUIVOCATION_FINALIZE, 0xadd07a3e),
        (SIG_SLASH_EQUIVOCATION_NULLIFY_FINALIZE, 0xa10827e9),
        (SIG_GET_MIN_VERDICT_DUE_BLOCKS, 0xee3ad0e7),
        (SIG_SET_MIN_VERDICT_DUE_BLOCKS, 0x4fae9dea),
        (SIG_GET_EXCLUSION_BACKOFF_CAP, 0x6bed0322),
        (SIG_SET_EXCLUSION_BACKOFF_CAP, 0x3b543e1c),
        (SIG_GET_PRODUCTION_LIVENESS_DISABLED, 0x9a4c46bb),
        (SIG_SET_PRODUCTION_LIVENESS_DISABLED, 0x8fc07556),
        (SIG_RECORD_PRODUCTION, 0x1752910e),
        (ERR_MIN_VERDICT_DUE_BLOCKS_TOO_HIGH, 0xb1776ed0),
        // The inlined verifier's five errors. They are the ones the retired
        // `BLS12381Verifier` predeploy raised, so a caller that decoded a revert
        // from it decodes the same revert now — which is only true while these
        // strings are exact. Every other assertion about them in this file
        // compares the contract's constant against itself.
        (ERR_BLS_INFINITY_POINT, 0x5a3dde75),
        (ERR_BLS_NAMESPACE_TOO_LONG, 0xcfa39070),
        (ERR_BLS_DST_TOO_LONG, 0x8c978650),
        (ERR_BLS_PRECOMPILE_FAILED, 0x84e81692),
        (ERR_BLS_INVALID_POINT_LENGTH, 0x3532eb3b),
    ] {
        assert_eq!(actual, pinned);
    }
}

// Pinned apart from the table above because these four constants only exist
// under `devnet-views`, and an array literal takes no attributes on its
// elements. `make test-contracts` runs both shapes so this is not a test that
// no configuration executes.
#[cfg(feature = "devnet-views")]
#[test]
fn devnet_view_selectors_match_their_pinned_ids() {
    for (actual, pinned) in [
        (SIG_BLOCKS_IN_EPOCH, 0xf06be669),
        (SIG_PRODUCED_AT, 0x91c7d453),
        (SIG_PENDING_EXCLUSIONS, 0xaef690f9),
        (SIG_LAST_PROCESSED_BLOCK, 0x33de61d2),
    ] {
        assert_eq!(actual, pinned);
    }
}

// The point of the feature is that the shipped artifact answers none of these
// four, and a gated handler that still answered its selector would defeat it.
// Asserted with literal ids so a drifting constant cannot drag the pin with it.
#[cfg(not(feature = "devnet-views"))]
#[test]
fn the_production_shape_answers_no_view_selector() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );
    for selector in [0xf06be669u32, 0x91c7d453, 0xaef690f9, 0x33de61d2] {
        assert_revert_selector(
            harness.call(encode_empty_call(selector)),
            ERR_UNKNOWN_METHOD,
        );
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
    // The two the node's close router decodes for reward observability. Pinned
    // here as well as there because the node's decode chain is CLOSED: a field
    // type changed on this side makes topic0 move, the arm stops matching, and
    // the event is emitted into silence with nothing failing on either side.
    assert_eq!(
        events::EpochBlendRewardsCommitted::SIGNATURE,
        "EpochBlendRewardsCommitted(uint64,uint256)"
    );
    assert_eq!(
        events::EpochWeightsUnavailable::SIGNATURE,
        "EpochWeightsUnavailable(uint64,uint32)"
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

/// `ensure_non_payable` and `ensure_mutable` open EVERY handler in the crate,
/// and until now nothing reached either: the harness leaves `contract_value` at
/// zero and `contract_is_static` at false, so both guards ran on their passing
/// input in all 173 tests and deleting either left the whole suite green.
///
/// Both are set here through the context the SDK already exposes — no host
/// change is needed for this, only a test that bothers to set them.
#[test]
fn handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );
    harness.set_caller(owner);

    let view = || encode_empty_call(SIG_CURRENT_EPOCH);
    let mutation = || {
        encode_call(
            SIG_DELEGATE,
            &AddressAmountCommand {
                validator,
                amount: DEFAULT_MIN_STAKING_AMOUNT,
            },
        )
    };
    // The control: both go through on an ordinary call.
    assert_eq!(harness.call(view()).0, ExitCode::Ok);
    assert_eq!(harness.call(mutation()).0, ExitCode::Ok);

    // Value attached. `ensure_non_payable` is first in every handler, view and
    // mutator alike, and it fails without revert data — the contract holds no
    // native balance and a payment to it would be stranded.
    harness.sdk.context_mut().value = U256::from(1);
    for calldata in [view(), mutation()] {
        assert_eq!(
            harness.call(calldata),
            (ExitCode::Panic, Vec::new()),
            "a payable call must be refused before anything else"
        );
    }
    harness.sdk.context_mut().value = U256::ZERO;

    // Static frame. Views stay legal; anything that writes does not, and it says
    // so with the SDK's own exit code rather than a contract revert.
    harness.sdk.context_mut().is_static = true;
    assert_eq!(
        harness.call(view()).0,
        ExitCode::Ok,
        "a read is still legal inside a static frame"
    );
    assert_eq!(
        harness.call(mutation()),
        (ExitCode::StateChangeDuringStaticCall, Vec::new()),
        "a mutating handler must refuse a static frame"
    );
    harness.sdk.context_mut().is_static = false;
    assert_eq!(harness.call(mutation()).0, ExitCode::Ok);
}

#[test]
fn staking_is_a_genesis_rwasm_contract_not_a_system_precompile() {
    assert!(!is_execute_using_system_runtime(&GENESIS_STAKING));
    assert!(!is_engine_metered_precompile(&GENESIS_STAKING));
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
                SIG_SET_ACTIVE_VALIDATORS_LENGTH,
                &U32Command { value: 31 },
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
        (SIG_GET_ACTIVE_VALIDATORS_LENGTH, 31),
        (SIG_GET_UNDELEGATE_PERIOD, 9),
    ] {
        let (exit, output) = harness.call(encode_empty_call(selector));
        assert_eq!(exit, ExitCode::Ok);
        assert_eq!(decode_output::<u32>(&output), expected);
    }
    for (selector, expected) in [
        (SIG_GET_SLASH_FUND_ADDRESS, slash_fund),
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
            if let Some(reply) = mock_precompile_reply(address, input) {
                return reply;
            }
            let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
            let output = match selector {
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
    let peer_pubkey = B256::with_last_byte(0x11);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
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
        Bytes::from(compressed_key_of(0x11))
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
            .entry(keccak256(compressed_key_of(0x11)))
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
    let first_peer_pubkey = B256::with_last_byte(0x11);
    let second_peer_pubkey = B256::with_last_byte(0x12);
    let bls_pubkey_uncompressed = Bytes::from(vec![0x11; BLS_PUBKEY_UNCOMPRESSED_LENGTH]);
    let bls_pop_uncompressed = Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]);
    let bls_pubkey_hash = keccak256(compressed_key_of(0x11));
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(first_owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

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

/// `owner_validators` is a one-to-one map and `getValidatorByOwner` is its only
/// reader, so a second registration from the same owner would overwrite the
/// first and leave the earlier validator unreachable through the owner it is
/// still owned by — with the bond already pulled.
///
/// The gate that stops it is `set_validator`'s own, and nothing else in the
/// crate re-checks it: the duplicate-VALIDATOR gates guard a different key.
#[test]
fn one_owner_cannot_register_a_second_validator() {
    let owner = Address::with_last_byte(0xa0);
    let first_validator = Address::with_last_byte(0x01);
    let second_validator = Address::with_last_byte(0x02);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    let register = |validator: Address, key: u8, peer: u8| {
        encode_args_call(
            SIG_REGISTER_VALIDATOR,
            &RegisterValidatorCommand {
                validator,
                commission_rate: 0,
                initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                bls_pubkey_uncompressed: Bytes::from(vec![key; BLS_PUBKEY_UNCOMPRESSED_LENGTH]),
                bls_pop_uncompressed: Bytes::from(vec![
                    key.wrapping_add(1);
                    BLS_POP_UNCOMPRESSED_LENGTH
                ]),
                peer_pubkey: B256::with_last_byte(peer),
            },
        )
    };

    harness.set_caller(owner);
    assert_eq!(
        harness.call(register(first_validator, 0x11, 1)).0,
        ExitCode::Ok
    );
    // Fresh validator address, fresh BLS key, fresh peer key: every other
    // uniqueness gate passes, so only the owner gate can refuse this.
    assert_revert_selector(
        harness.call(register(second_validator, 0x33, 2)),
        ERR_VALIDATOR_OWNER_ALREADY_IN_USE,
    );

    assert_eq!(
        staking_storage()
            .owner_validators_accessor()
            .entry(owner)
            .get_checked(&harness.sdk)
            .unwrap(),
        first_validator,
        "the owner still resolves to the validator it actually paid for"
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
}

/// A registration whose proof of possession does not verify writes nothing.
///
/// `InvalidProofOfPossession` had no test at all while the verifier was an
/// external contract, because the shared stub answered `true` to every
/// `verify` and no test installed one that answered `false`. It is the gate
/// that stops a validator from registering a public key it does not hold the
/// secret for — the whole premise of the rogue-key defence — so an unreachable
/// gate was the one worth reaching first.
#[test]
fn registration_rejects_a_forged_proof_of_possession_without_partial_state() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let peer_pubkey = B256::with_last_byte(0x11);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    reject_every_pairing();
    harness.set_caller(owner);

    let (exit, output) = harness.call(encode_args_call(
        SIG_REGISTER_VALIDATOR,
        &RegisterValidatorCommand {
            validator,
            commission_rate: 0,
            initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
            bls_pubkey_uncompressed: Bytes::from(vec![0x11; BLS_PUBKEY_UNCOMPRESSED_LENGTH]),
            bls_pop_uncompressed: Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]),
            peer_pubkey,
        },
    ));
    assert_eq!(exit, ExitCode::Panic);
    assert_eq!(
        &output[..SIG_LEN_BYTES],
        &ERR_INVALID_PROOF_OF_POSSESSION.to_be_bytes()
    );
    // The error names the rejected validator, which is what a caller reads to
    // learn WHOSE registration failed inside a batch.
    assert_eq!(
        decode_output::<Address>(&output[SIG_LEN_BYTES..]),
        validator
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
    let (_, _, stakes, _): (Vec<Address>, Vec<ConsensusKeys>, Vec<U256>, Vec<bool>) =
        decode_returns(&output);
    assert_eq!(
        stakes[0], initial,
        "epoch 2 was selected from epoch 0 and must carry epoch 0's weight"
    );

    staking::delegate_to(&mut harness.sdk, delegator, validator, added, false).unwrap();
    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE_WITH_STAKES,
        &U64Command { value: 2 },
    ));
    let (_, _, stakes, _): (Vec<Address>, Vec<ConsensusKeys>, Vec<U256>, Vec<bool>) =
        decode_returns(&output);
    assert_eq!(
        stakes[0], initial,
        "a committed epoch's weights do not move when stake changes afterwards"
    );
}

// The other half of the key filter: keys that exist but activate later must be
// treated exactly like absent keys on both legs — filtered before the cut, with
// the seat going to the next eligible validator by stake.
#[test]
fn keys_activating_after_the_selection_epoch_are_filtered_before_the_cut() {
    let owner = Address::with_last_byte(0xa0);
    let future = Address::with_last_byte(0x01);
    let keyed: Vec<Address> = (2..=MIN_COMMITTEE_LENGTH + 1)
        .map(|index| Address::with_last_byte(index as u8))
        .collect();
    let spare = Address::with_last_byte(0xee);
    let mut harness = Harness::new(1_000);
    // On stake alone the cap would seat `future` plus every `keyed` member and
    // leave `spare` below the cut. Dropping `future` for its late activation must
    // reach past the cut for `spare` and keep the committee at the cap.
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

    let mut expected = keyed.clone();
    expected.push(spare);
    assert_eq!(
        selected_at(&harness.sdk, 0),
        expected,
        "a key that activates after the selection epoch takes no slot, and the \
         validator below the cut is promoted into it"
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
    // `spare` IS promoted into the slot the not-yet-activated validator never
    // took: the key filter runs before the stake cut, so the cut reaches further
    // down the ranking instead of handing out a seat nobody can occupy. The
    // committed array is peer-key ascending, so membership and size are pinned
    // here; the stake order is pinned above.
    let committee = decode_output::<Vec<Address>>(&output);
    assert_eq!(
        committee.len(),
        cap,
        "the freed seat is filled, not left empty"
    );
    assert!(
        committee.contains(&spare),
        "the next eligible validator by stake takes the late-activating validator's seat"
    );
    assert!(!committee.contains(&future));
    for validator in &keyed {
        assert!(committee.contains(validator));
    }
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
        selected_at(&harness.sdk, 1)[0],
        validator_b,
        "the E+2 delegation must not lift validator_a above validator_b at E+1"
    );
    assert_eq!(
        selected_at(&harness.sdk, 2)[0],
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
        // The delegation matures at `WARMUP_DELAY` and the seat it funds is
        // selected `MAX_COMMITTEE_LOOKAHEAD_EPOCHS` ahead, so this is the first
        // epoch whose reward that delegation divides.
        .entry(WARMUP_DELAY + MAX_COMMITTEE_LOOKAHEAD_EPOCHS)
        .total_blend_rewards_accessor()
        .set_checked(
            &mut harness.sdk,
            math::narrow_reward(reward).expect("reward fits uint96"),
        )
        .unwrap();
    consensus_storage()
        .tombstoned_accessor()
        .entry(validator)
        .set_checked(&mut harness.sdk, true)
        .unwrap();

    // The reward is pulled off the reserve, so the call is `transferFrom` and the
    // recipient is its SECOND argument.
    let transfers = Rc::new(RefCell::new(Vec::<(Address, U256)>::new()));
    let recorded = transfers.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            if let Some(reply) = mock_precompile_reply(address, input) {
                return reply;
            }
            assert_eq!(address, token);
            let (_, to, amount) =
                SolidityABI::<(Address, Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0)
                    .unwrap();
            recorded.borrow_mut().push((to, amount));
            SyscallResult::new(encode_mock_return(&true), 0, 0, ExitCode::Ok)
        });

    harness.set_caller(delegator);
    harness.set_block_number(
        activation_block
            + DEFAULT_EPOCH_BLOCK_INTERVAL * (WARMUP_DELAY + MAX_COMMITTEE_LOOKAHEAD_EPOCHS + 1),
    );
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
        // The delegation is booked at epoch 2 and the seat it funds is selected
        // two epochs ahead, so epoch 4 is the first reward it divides.
        .entry(4);
    snapshot
        .total_blend_rewards_accessor()
        .set_checked(
            &mut harness.sdk,
            math::narrow_reward(ten_tokens).expect("reward fits uint96"),
        )
        .unwrap();

    harness.set_block_number(2_000);
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
    let (_, output) = harness.call(encode_empty_call(SIG_NEXT_EPOCH_TO_COMMIT));
    assert_eq!(decode_output::<u64>(&output), 1);
    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE_WITH_STAKES,
        &U64Command { value: 0 },
    ));
    let (validators, keys, stakes, _): (Vec<Address>, Vec<ConsensusKeys>, Vec<U256>, Vec<bool>) =
        decode_returns(&output);
    assert_eq!(validators, expected_committee);
    assert_eq!(
        keys.iter()
            .map(|value| value.bls_pubkey[0])
            .collect::<Vec<_>>(),
        vec![0xb1, 0xb2, 0xb3, 0xb4]
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
// selection applies no stake floor, by design. The close then assigns nobody
// anything, so the epoch's pot is simply never owed.
//
// This pins the behaviour rather than endorsing it. The state used to be caught
// by a length-mismatch revert that the paired-storage change made unreachable;
// what that revert stood in for is this, and nothing else covered it.
#[test]
fn a_committee_with_only_zero_weights_assigns_its_epoch_nothing() {
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
    let funding =
        install_stipend_token(&harness.sdk, reserve, U256::from(1_000), U256::from(1_000));
    record_test_production(&mut harness.sdk, 0, DEFAULT_EPOCH_BLOCK_INTERVAL as u32);

    assert_eq!(
        epoch_reward(&harness.sdk, validator, 0),
        U256::ZERO,
        "a committee of zero weights divides nothing — the pot is never owed"
    );
    assert!(
        funding.borrow().pulls.is_empty(),
        "and nothing is drawn off the reserve for it"
    );
}

// An epoch that recorded no block at all draws NOTHING, and the arm that says so
// is `accrue_epoch`'s own — not one of the zero-returns inside
// `assign_epoch_shares`. The difference matters because each of those needs a
// reason of its own to fire — a zero pot, a reserve that cannot cover it, an
// empty committee, a lost ring frame, a zero total weight — and the fixture
// below denies every one of them. A stalled recorder or a pre-activation prefix
// would otherwise buy a full epoch's stipend for zero work.
#[test]
fn an_epoch_with_a_committee_but_no_recorded_block_draws_no_pot() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let reserve = Address::with_last_byte(0xc0);
    let pot = U256::from(400);
    let mut harness = Harness::new(1_000);
    let mut command =
        harness.initialize_command(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0);
    command.blend_reserve = reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);

    chain_config_storage()
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, pot)
        .unwrap();
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[(validator, DEFAULT_MIN_VALIDATOR_STAKE)],
    );
    install_stipend_token(
        &harness.sdk,
        reserve,
        pot * U256::from(10),
        pot * U256::from(10),
    );
    // Everything the accrual needs is in place except the work: this is the same
    // fixture as a funded close, seeded with a zero block count.
    assert!(
        consensus::read_weights(&harness.sdk, 0)
            .unwrap()
            .is_some_and(|weights| weights.iter().any(|weight| !weight.is_zero())),
        "the committee's frozen weights must be present, or the epoch would \
         score zero for a reason this test is not about"
    );
    harness.sdk.take_logs();

    record_test_production(&mut harness.sdk, 0, 0);

    assert_eq!(
        epoch_reward(&harness.sdk, validator, 0),
        U256::ZERO,
        "an epoch nobody recorded a block for owes nothing"
    );
    assert_epoch_accrual_event(&harness.sdk, 0, U256::ZERO);
    // Deliberately no `assert!(pulls.is_empty())` here: the close makes no
    // `transferFrom` on any path, so that assertion cannot fail and would report
    // coverage it does not have. The control below is what carries the weight —
    // and it is the stronger statement, because a reserve that could not cover
    // the pot would fail it.

    // The control: the identical fixture with one recorded block pays in full,
    // so the zero above is the block count and nothing else.
    commit_test_committee(
        &mut harness.sdk,
        1,
        &[(validator, DEFAULT_MIN_VALIDATOR_STAKE)],
    );
    harness.sdk.take_logs();
    record_test_production(&mut harness.sdk, 1, 1);
    assert_eq!(epoch_reward(&harness.sdk, validator, 1), pot);
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

// A keyless validator outranking a keyed one by stake occupies NO slot: it is
// dropped before the cut, and the seat it would have taken goes to the next
// eligible validator by stake. The seat is not spent on nobody.
//
// This reverses the property the test held until 2026-09-07, deliberately. The
// old order was justified by agreement with an off-chain committee deriver; the
// node no longer derives a committee, so the only thing filtering-after-the-cut
// still bought was a committee smaller than the population could fill.
#[test]
fn the_committee_seats_the_next_eligible_validator_instead_of_a_keyless_one() {
    let owner = Address::with_last_byte(0xa0);
    let keyless = Address::with_last_byte(0x01);
    let keyed: Vec<Address> = (2..=MIN_COMMITTEE_LENGTH + 1)
        .map(|index| Address::with_last_byte(index as u8))
        .collect();
    let below_cut = Address::with_last_byte(0xee);
    let mut harness = Harness::new(1_000);
    // On stake alone the cap would seat `keyless` — top-ranked — plus every
    // `keyed` member, leaving `below_cut` outside. Blanking the keyless member's
    // key must promote `below_cut` into the freed seat and keep the committee at
    // the cap, rather than leave it one short at the committee floor.
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

    let mut expected = keyed.clone();
    expected.push(below_cut);
    assert_eq!(
        selected_at(&harness.sdk, 0),
        expected,
        "the key filter runs before the cut, so the top-ranked keyless validator \
         takes no slot and the next validator by stake takes the seat"
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
    // The filter drops `keyless` before the cut, so the cut hands out all `cap`
    // seats to validators that can take them: the four `keyed` members and
    // `below_cut`. The committed array is peer-key ascending, so membership and
    // size are what is pinned here; the stake order is pinned above.
    let committee = decode_output::<Vec<Address>>(&output);
    assert_eq!(
        committee.len(),
        cap,
        "the freed seat is filled, not left empty"
    );
    assert!(
        committee.contains(&below_cut),
        "the next eligible validator by stake takes the keyless validator's seat"
    );
    assert!(!committee.contains(&keyless));
    for validator in &keyed {
        assert!(committee.contains(validator));
    }
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

    assert_eq!(selected_at(&harness.sdk, 0), validators);
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

// `committee_changed` short-circuits to `false` at genesis, which was inert
// while the committee write was unconditional. Making it decide whether a record
// exists at all turns it load-bearing, so epoch 0 has to append anyway — and the
// knock-on is that a record's existence no longer implies the committee changed.
#[test]
fn the_genesis_commit_mints_a_record_although_nothing_changed() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    let (validators, stakes) = with_filler_validators(&[(validator, DEFAULT_MIN_VALIDATOR_STAKE)]);
    assert_eq!(
        harness.initialize(owner, validators.clone(), stakes, 0),
        ExitCode::Ok
    );

    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );

    let consensus = consensus_storage();
    let index = consensus.epoch_index_accessor().entry(0);
    assert_eq!(
        index.record_accessor().get_checked(&harness.sdk).unwrap(),
        0,
        "epoch 0 points at record 0, which is a real pointer and not absence"
    );
    assert_eq!(
        index.length_accessor().get_checked(&harness.sdk).unwrap() as usize,
        validators.len()
    );
    assert_eq!(
        consensus
            .committee_records_accessor()
            .entry(0)
            .len_checked(&harness.sdk)
            .unwrap() as usize,
        validators.len(),
        "the record exists; the else arm would have resolved one that does not"
    );
    assert!(
        !consensus
            .dkg_qual_accessor()
            .entry(0)
            .get_checked(&harness.sdk)
            .unwrap(),
        "a record was appended AND changed is false — the one epoch where \
         inferring the change bit from a record's existence is wrong"
    );

    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE,
        &U64Command { value: 0 },
    ));
    assert_eq!(decode_output::<Vec<Address>>(&output), validators);
}

// The pair-0 rule, against the case that motivates it. A long epoch followed by
// a short one `WEIGHT_RING_EPOCHS` later leaves the tail pairs carrying the OLD
// stamp and the old weights, while pairs 0..k hold the new epoch's. A reader
// that checked its last pair would find its own stamp there, accept the frame,
// and then read the successor's weights out of the head.
#[test]
fn a_short_successor_cannot_let_its_predecessor_read_the_wrong_weights() {
    let owner = Address::with_last_byte(0xa0);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    let long: Vec<(Address, U256)> = (1..=8u8)
        .map(|i| {
            (
                Address::with_last_byte(i),
                DEFAULT_MIN_VALIDATOR_STAKE * U256::from(i),
            )
        })
        .collect();
    let short: Vec<(Address, U256)> = (1..=4u8)
        .map(|i| (Address::with_last_byte(i), DEFAULT_MIN_VALIDATOR_STAKE))
        .collect();
    commit_test_committee(&mut harness.sdk, 0, &long);
    commit_test_committee(&mut harness.sdk, WEIGHT_RING_EPOCHS, &short);

    let base = 0usize;
    let ring = consensus_storage().weight_ring_accessor();
    assert_eq!(
        ring.at(base + 3)
            .stamp_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "the tail the short successor did not reach still carries epoch 0's stamp"
    );

    let (exit, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE_WITH_STAKES,
        &U64Command { value: 0 },
    ));
    assert_eq!(exit, ExitCode::Ok);
    let (seated, _, stakes, _): (Vec<Address>, Vec<ConsensusKeys>, Vec<U256>, Vec<bool>) =
        decode_returns(&output);
    assert_eq!(seated.len(), 8, "membership answers at any depth");
    assert!(
        stakes.is_empty(),
        "weights do not: the ring wrapped, so the answer is an explicit \
         not-retained rather than a vector of somebody else's numbers"
    );
}

// The odd-count belt, asserted against the REAL commit.
//
// An earlier version of this drove `commit_test_committee`, which is a
// hand-written mirror of `write_ring` — including its own `b = 0`. That test
// passed with the store deleted from production, which makes it worse than no
// test: it reported coverage it did not have. Here the frame is DIRTIED by the
// fixture and then written by `commitEpochCommittee` itself, so removing the
// store fails this.
//
// Nothing reads the half in question — the count bound stops one short — so what
// is asserted is that the state stops existing, which is what turns the bound
// into a belt instead of the only guard.
#[test]
fn an_odd_member_count_leaves_no_stale_half_in_the_ring() {
    let (mut harness, members) = liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE; 5], 5);

    // Fixture, not the code under test: leave the previous occupant's weight and
    // stamp in the pair the five-member commit will only half-fill.
    let ring = consensus_storage().weight_ring_accessor();
    let stale = ring.at(2);
    stale
        .b_accessor()
        .set_checked(
            &mut harness.sdk,
            crate::math::compact_balance(DEFAULT_MIN_VALIDATOR_STAKE).unwrap(),
        )
        .unwrap();
    stale
        .stamp_accessor()
        .set_checked(&mut harness.sdk, 0xdead_beef)
        .unwrap();

    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );

    assert_eq!(
        consensus_storage()
            .epoch_index_accessor()
            .entry(0)
            .length_accessor()
            .get_checked(&harness.sdk)
            .unwrap() as usize,
        members.len(),
        "five members, so three pairs with the last one half-used"
    );
    let last = consensus_storage().weight_ring_accessor().at(2);
    assert_eq!(
        last.stamp_accessor().get_checked(&harness.sdk).unwrap(),
        0,
        "the commit claimed the pair"
    );
    assert!(
        !last
            .a_accessor()
            .get_checked(&harness.sdk)
            .unwrap()
            .is_zero(),
        "and filled its first half"
    );
    assert!(
        last.b_accessor()
            .get_checked(&harness.sdk)
            .unwrap()
            .is_zero(),
        "the unused half is written to zero, not left holding the previous \
         occupant's weight under this epoch's fresh stamp"
    );
}

// The order-sensitivity half of the alignment proof. `committee_changed` is what
// licenses reusing a record across epochs, and it may only do so when the new
// slice is positionally identical to the incumbent. A set-based comparison would
// call a reordering unchanged, the record would be reused, and every index that
// resolves through it — the slash resolver, the leader weights, `judge` — would
// point at the wrong member. Same members, different order, must read as
// changed.
//
// The length term is asserted separately because the positional loop cannot
// stand in for it: the loop walks the NEW slice, so a successor that is a strict
// prefix of the incumbent compares equal all the way down and reads as
// unchanged. That is the one shape where dropping the term is invisible, and it
// is the dangerous one — the index would then record the shorter length against
// the LONGER record.
#[test]
fn committee_changed_compares_positions_not_membership() {
    let owner = Address::with_last_byte(0xa0);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    let seated: Vec<(Address, U256)> = (1..=4u8)
        .map(|i| (Address::with_last_byte(i), DEFAULT_MIN_VALIDATOR_STAKE))
        .collect();
    commit_test_committee(&mut harness.sdk, 0, &seated);

    let slice_of = |members: &[(Address, U256)]| -> Vec<CommitteeMember> {
        members
            .iter()
            .map(|(validator, weight)| CommitteeMember {
                validator: *validator,
                peer_pubkey: B256::ZERO,
                weight: *weight,
            })
            .collect()
    };

    assert!(
        !committee_changed(&harness.sdk, 1, &slice_of(&seated)).unwrap(),
        "an identical slice is not a change"
    );

    let mut swapped = slice_of(&seated);
    swapped.swap(0, 1);
    assert!(
        committee_changed(&harness.sdk, 1, &swapped).unwrap(),
        "the same set in a different order IS a change"
    );

    // A strict prefix of the incumbent. Every position the loop reaches matches,
    // so only the length term can call this a change.
    assert!(
        committee_changed(&harness.sdk, 1, &slice_of(&seated[..3])).unwrap(),
        "a successor one member shorter IS a change, and the positional walk \
         alone cannot see it"
    );
}

// The key-immutability half of the alignment proof. `committee_changed` compares
// positionally against a frozen record, and both sides are peer-key ordered — so
// the comparison is only meaningful for all time because the sort key can never
// move. Nothing in `committee_changed` would notice if it could; this is where
// that rests.
#[test]
fn a_peer_key_cannot_be_reassigned_so_the_sort_key_is_immutable() {
    let first_owner = Address::with_last_byte(0xa0);
    let second_owner = Address::with_last_byte(0xa1);
    let first_validator = Address::with_last_byte(0x01);
    let second_validator = Address::with_last_byte(0x02);
    let shared_peer_pubkey = B256::with_last_byte(0x11);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(first_owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );

    harness.set_caller(first_owner);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_REGISTER_VALIDATOR,
                &RegisterValidatorCommand {
                    validator: first_validator,
                    commission_rate: 0,
                    initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                    bls_pubkey_uncompressed: Bytes::from(vec![
                        0x11;
                        BLS_PUBKEY_UNCOMPRESSED_LENGTH
                    ]),
                    bls_pop_uncompressed: Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]),
                    peer_pubkey: shared_peer_pubkey,
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
                bls_pubkey_uncompressed: Bytes::from(vec![0x44; BLS_PUBKEY_UNCOMPRESSED_LENGTH]),
                bls_pop_uncompressed: Bytes::from(vec![0x55; BLS_POP_UNCOMPRESSED_LENGTH]),
                peer_pubkey: shared_peer_pubkey,
            },
        )),
        ERR_PEER_PUBKEY_ALREADY_IN_USE,
    );
    assert_eq!(
        consensus_storage()
            .peer_pubkey_owner_accessor()
            .entry(shared_peer_pubkey)
            .get_checked(&harness.sdk)
            .unwrap(),
        first_validator
    );
}

// The retired horizon stood at `target + undelegatePeriod + 8 + 1`. Nothing
// deletes a committee now, so every reader that resolves against one has to keep
// answering arbitrarily far past where the wall used to be. `getEpochRewards` is
// why this is an assertion rather than a formality: it sums the committee, so an
// emptied one made it return a confident zero instead of reverting — a wrong
// answer no caller could tell from a real one.
#[test]
fn a_committee_stays_readable_far_past_the_retired_pruning_horizon() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let activation_block = 1_000;
    let mut harness = Harness::new(activation_block);
    harness.set_caller(owner);
    let (validators, stakes) = with_filler_validators(&[(validator, DEFAULT_MIN_VALIDATOR_STAKE)]);
    assert_eq!(
        harness.initialize(owner, validators.clone(), stakes, 0),
        ExitCode::Ok
    );

    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );
    consensus_storage()
        .dkg_qual_accessor()
        .entry(0)
        .set_checked(&mut harness.sdk, true)
        .unwrap();
    let reward = U256::from(7_000_000u64);
    staking_storage()
        .validator_snapshots_accessor()
        .entry(validators[0])
        .entry(0)
        .total_blend_rewards_accessor()
        .set_checked(
            &mut harness.sdk,
            math::narrow_reward(reward).expect("reward fits uint96"),
        )
        .unwrap();

    harness.set_block_number(
        activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * (DEFAULT_UNDELEGATE_PERIOD + 100),
    );
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
    assert_eq!(decode_output::<Vec<Address>>(&output), validators);
    let (_, output) = harness.call(encode_call(
        SIG_GET_EPOCH_COMMITTEE_WITH_STAKES,
        &U64Command { value: 0 },
    ));
    let (seated, _, weights, _): (Vec<Address>, Vec<ConsensusKeys>, Vec<U256>, Vec<bool>) =
        decode_returns(&output);
    assert_eq!(seated, validators);
    assert!(weights.iter().all(|weight| !weight.is_zero()));
    let (_, output) = harness.call(encode_call(SIG_GET_EPOCH_REWARDS, &U64Command { value: 0 }));
    assert_eq!(decode_output::<U256>(&output), reward);
    assert!(consensus_storage()
        .dkg_qual_accessor()
        .entry(0)
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
    // Selection reads the LIVE active set, so the exit drops the validator out of
    // it at once and for every epoch. A past epoch's seating is no longer
    // re-derived here — it is recorded in the committed committee array.
    //
    // These two reads add little: the empty active set asserted above already
    // forces an empty selection whatever the filter does, so only `remove_active`
    // going missing can redden them, and the assertion above catches that first.
    // They are kept because they read through the selection path rather than the
    // storage vector, not because they pin a property of their own.
    for epoch in [0u64, 1] {
        assert!(
            selected_at(&harness.sdk, epoch).is_empty(),
            "the exited validator is still selected at epoch {epoch}"
        );
    }
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
fn stipend_pays_the_frozen_weights_not_the_stake_at_close_time() {
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
    install_stipend_token(&harness.sdk, reserve, U256::from(100), U256::from(100));
    // Accrues epoch 2 — the close is where the split happens now.
    record_test_production(&mut harness.sdk, 2, DEFAULT_EPOCH_BLOCK_INTERVAL as u32);

    // Equal frozen weights across the whole committee, so the pot splits evenly
    // over `MIN_COMMITTEE_LENGTH` members — the point is that both named members
    // are paid the SAME share despite one of them gaining stake afterwards.
    let share = U256::from(100 / MIN_COMMITTEE_LENGTH);
    assert_eq!(epoch_reward(&harness.sdk, validator_a, 2), share);
    assert_eq!(epoch_reward(&harness.sdk, validator_b, 2), share);
}

#[test]
fn closing_an_epoch_records_what_it_assigned() {
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
    // Written raw rather than through `record_test_production`, which accrues the
    // epoch itself. The whole point of this test is that the contract accrues it,
    // so seeding through the stand-in would prove nothing.
    production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(0)
        .set_checked(&mut harness.sdk, DEFAULT_EPOCH_BLOCK_INTERVAL as u32)
        .unwrap();
    assert_eq!(epoch_reward(&harness.sdk, validator, 0), U256::ZERO);
    harness.sdk.take_logs();

    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);

    // The event is the whole record now. The `assigned + 1` scalar existed so a
    // later payment could tell "never closed" from "closed owing nothing"; there
    // is no later payment, so there is no second reader to disagree with the
    // credits.
    let logs = harness.sdk.take_logs();
    let (data, _) = find_log(
        &logs,
        events::EpochBlendRewardsCommitted::SELECTOR,
        "EpochBlendRewardsCommitted",
    );
    assert_eq!(decode_output::<U256>(data), U256::from(250));
    assert_eq!(epoch_reward(&harness.sdk, validator, 0), U256::from(250));
}

// The scalar is what the payment pulls and the credits are what the claims walk.
// If they can disagree the contract either short-changes a validator or owes more
// than it computed, so they are written from one `assigned` in one frame — and
// this is the test of that.
//
// A single paid seat divides the pot exactly whatever the pot is, so the (1, 0)
// case is the degenerate one and carries no remainder by construction. The other
// three leave 3, 3 and 6 PAID seats against a pot of 1_000 — remainders of 1, 1
// and 4 — so the floored-sum property is exercised at an odd count, at a count
// with one zero-weight seat, and at a count with two.
#[test]
fn the_accrued_total_is_exactly_the_sum_of_the_credits_it_wrote() {
    let owner = Address::with_last_byte(0xa0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let pot = U256::from(1_000);
    for (size, zero_weight_seats) in [(1usize, 0usize), (3, 0), (4, 1), (8, 2)] {
        let mut harness = Harness::new(1_000);
        assert_eq!(
            harness.initialize(owner, vec![Address::with_last_byte(0x01)], vec![stake], 0),
            ExitCode::Ok
        );
        chain_config_storage()
            .blend_stipend_per_epoch_accessor()
            .set_checked(&mut harness.sdk, pot)
            .unwrap();
        let seats: Vec<(Address, U256)> = (0..size)
            .map(|index| {
                let weight = if index < zero_weight_seats {
                    U256::ZERO
                } else {
                    stake
                };
                (Address::with_last_byte(0x01 + index as u8), weight)
            })
            .collect();
        commit_test_committee(&mut harness.sdk, 0, &seats);
        harness.sdk.take_logs();
        record_test_production(&mut harness.sdk, 0, DEFAULT_EPOCH_BLOCK_INTERVAL as u32);

        let credited = seats.iter().fold(U256::ZERO, |sum, (member, _)| {
            sum + epoch_reward(&harness.sdk, *member, 0)
        });
        assert!(
            pot - credited < U256::from(size - zero_weight_seats),
            "the unassigned remainder is under one base unit per paid seat, \
             committee of {size} with {zero_weight_seats} zero-weight seats"
        );
        // The reported total and the sum of the credits come out of one
        // `assigned` in one frame, so the announcement can never overstate what
        // was actually written to the seats.
        assert_epoch_accrual_event(&harness.sdk, 0, credited);
    }
}

// The once-per-epoch guarantee used to belong to the monotone settlement cursor;
// it belongs to the close now, and nothing but `record_production`'s monotone
// height stands between an accrual and a second one. The credit is an assignment
// so that a second pass rewrites instead of doubling.
#[test]
fn re_accruing_an_epoch_rewrites_rather_than_doubles_it() {
    let pot = U256::from(100);
    let (mut harness, _funding, validator) = stipend_test_sdk(pot, pot);
    assert_eq!(epoch_reward(&harness.sdk, validator, 0), pot);

    staking::accrue_epoch(&mut harness.sdk, 0, DEFAULT_EPOCH_BLOCK_INTERVAL as u32).unwrap();

    assert_eq!(epoch_reward(&harness.sdk, validator, 0), pot);
}

// The claim walks read the per-epoch credit and the settlement cursor, and the
// split changes only which call writes the credit. Driven end to end rather than
// from a seeded snapshot, so the amounts are the ones the whole path produces.
#[test]
fn the_owner_and_delegator_claims_split_an_accrued_epoch_by_its_commission() {
    let pot = U256::from(100);
    let (mut harness, _funding, validator) = stipend_test_sdk(pot, pot);
    staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(0)
        .commission_rate_accessor()
        .set_checked(&mut harness.sdk, 1_000)
        .unwrap();

    let transfers = Rc::new(RefCell::new(Vec::<(Address, U256)>::new()));
    let recorded = transfers.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            if let Some(reply) = mock_precompile_reply(address, input) {
                return reply;
            }
            let (_, to, amount) =
                SolidityABI::<(Address, Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0)
                    .unwrap();
            recorded.borrow_mut().push((to, amount));
            SyscallResult::new(encode_mock_return(&true), 0, 0, ExitCode::Ok)
        });

    // The genesis validator is its own owner and its own sole delegator, so both
    // walks name the same address and the two legs of the split are told apart by
    // amount, not by recipient.
    harness.set_caller(validator);
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
        &[(validator, U256::from(10)), (validator, U256::from(90))],
        "the commission leg and the delegated leg together are the epoch's credit"
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

    assert_eq!(epoch_reward(&harness.sdk, validator_a, 0), U256::ZERO);
    assert_eq!(
        epoch_reward(&harness.sdk, validator_b, 0),
        U256::from(100),
        "the tombstoned seat's share goes to the rest, not to nobody"
    );
}

// A claim anyone may call, whose money goes to the OWNER and comes off the
// RESERVE. Both halves matter: the caller must not be able to redirect the
// payment to themselves, and the payment must not be taken out of the deposits
// this contract is holding for other people.
#[test]
fn a_permissionless_owner_claim_pays_the_owner_off_the_reserve() {
    let assigned = U256::from(100);
    let attacker = Address::with_last_byte(0xd0);
    let reserve = Address::with_last_byte(0xc0);
    let (mut harness, _funding, validator) = stipend_test_sdk(assigned, assigned);
    staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(0)
        .commission_rate_accessor()
        .set_checked(&mut harness.sdk, 1_000)
        .unwrap();
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL);

    // Records `transferFrom(from, to, amount)` in full: `from` is the assertion
    // that the money left the reserve and not this contract's own balance, and a
    // plain `transfer` would not carry it.
    let pulls = Rc::new(RefCell::new(Vec::<(Address, Address, U256)>::new()));
    let recorded = pulls.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            if let Some(reply) = mock_precompile_reply(address, input) {
                return reply;
            }
            assert_eq!(address, STAKING_TOKEN);
            assert_eq!(
                u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap()),
                SIG_ERC20_TRANSFER_FROM,
                "a stipend claim pulls from the reserve; a plain transfer would \
                 spend this contract's own balance"
            );
            let pull = SolidityABI::<(Address, Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0)
                .unwrap();
            recorded.borrow_mut().push(pull);
            SyscallResult::new(encode_mock_return(&true), 0, 0, ExitCode::Ok)
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
        pulls.borrow().as_slice(),
        &[(reserve, validator, U256::from(10))],
        "the reserve pays the owner; the caller is neither end of it"
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

// The reward and the principal are two claims over two cursors and two sources,
// and neither can hold the other up. The reward is pulled off the BLEND reserve;
// the principal comes out of this contract, where the delegation put it.
//
// The order matters to what this proves. The reward is claimed FIRST, while a
// matured withdrawal is sitting in the queue, so the reward claim has something
// to swallow if the two paths were ever re-merged; then the reserve is emptied
// again and the withdrawal has to come out regardless.
#[test]
fn a_reward_and_a_matured_principal_claim_are_independent() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let delegator = Address::with_last_byte(0x02);
    let reserve = Address::with_last_byte(0xc0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let reward = DEFAULT_MIN_STAKING_AMOUNT;
    let activation_block = 1_000;
    let mut harness = Harness::new(activation_block);
    harness.set_caller(owner);
    let mut command = harness.initialize_command(owner, vec![validator], vec![stake], 0);
    command.blend_reserve = reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
    staking::delegate_to(&mut harness.sdk, delegator, validator, stake, false).unwrap();
    harness.set_block_number(activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * WARMUP_DELAY);
    staking::undelegate_from(&mut harness.sdk, delegator, validator, stake).unwrap();
    staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        // The delegation matures at `WARMUP_DELAY` and the seat it funds is
        // selected `MAX_COMMITTEE_LOOKAHEAD_EPOCHS` ahead, so this is the first
        // epoch whose reward that delegation divides.
        .entry(WARMUP_DELAY + MAX_COMMITTEE_LOOKAHEAD_EPOCHS)
        .total_blend_rewards_accessor()
        .set_checked(
            &mut harness.sdk,
            math::narrow_reward(reward * U256::from(2)).expect("reward fits uint96"),
        )
        .unwrap();
    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator);

    // A reserve that holds nothing and has approved nothing: every `transferFrom`
    // off it fails. Deposits held by this contract go out as a plain `transfer`
    // and are unaffected.
    let funding = install_stipend_token(&harness.sdk, reserve, U256::ZERO, U256::ZERO);

    harness.set_caller(delegator);
    let maturity_epoch = WARMUP_DELAY + 1 + DEFAULT_UNDELEGATE_PERIOD;
    harness.set_block_number(activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * maturity_epoch);
    assert_eq!(
        delegation
            .pending_undelegated_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        stake,
        "the withdrawal is matured and waiting before either claim runs"
    );

    // 1. The reward is owed but the reserve cannot pay it, so the claim reverts
    //    rather than paying short.
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CLAIM_DELEGATOR_FEE,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Panic,
        "an empty reserve cannot pay, and the claim says so rather than paying short"
    );

    // 2. The treasury funds the reserve and the same claim goes through — taking
    //    the reward and leaving the matured deposit exactly where it was.
    funding.borrow_mut().balance = reward * U256::from(4);
    funding.borrow_mut().allowance = reward * U256::from(4);
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
        funding.borrow().pulls,
        vec![(delegator, reward), (delegator, reward)],
        "twice: the mock records the attempt the dry reserve refused as well as \
         the one it honoured, and both name the delegator as the recipient — the \
         reward never lands on this contract on its way"
    );
    assert!(
        funding.borrow().transfers.is_empty(),
        "the reward did not come out of the contract's own balance"
    );
    // Up to the CURRENT epoch, not to a settlement frontier — there is no longer
    // a global cursor deciding which epochs are payable, so the walk stops where
    // the ledger stops. Only `WARMUP_DELAY` carried a credit, which is why the
    // amount above is one epoch's reward and not eight.
    assert_eq!(
        delegation
            .claimed_through_epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        maturity_epoch,
        "the claim walks every closed epoch, and every one of them is now paid"
    );
    assert_eq!(
        delegation
            .undelegate_gap_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "a reward claim must not move the withdrawal cursor"
    );
    assert_eq!(
        delegation
            .pending_undelegated_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        stake,
        "nor pay out the deposit the delegator asked to withdraw"
    );

    // 3. The reserve dries up again. The matured deposit still comes out: it was
    //    never the reserve's money.
    funding.borrow_mut().balance = U256::ZERO;
    funding.borrow_mut().allowance = U256::ZERO;
    assert_eq!(
        harness
            .call(encode_call(
                SIG_WITHDRAW_DELEGATOR_PRINCIPAL,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok,
        "an unfunded reserve must not hold up a deposit this contract is holding"
    );
    assert_eq!(
        funding.borrow().transfers,
        vec![(delegator, stake)],
        "the deposit goes out of this contract's own balance"
    );
    assert_eq!(
        funding.borrow().pulls.len(),
        2,
        "and the withdrawal did not touch the reserve at all"
    );
    assert_eq!(
        delegation
            .undelegate_gap_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1
    );
    assert_eq!(
        delegation
            .claimed_through_epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        maturity_epoch,
        "and a withdrawal must not move the reward cursor"
    );
}

#[test]
fn claiming_rewards_does_not_rewrite_historical_self_stake() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    // NOT block 0. A zero activation block is the unarmed sentinel and pins
    // every epoch at zero, so the claim below would walk no epochs at all and
    // consume nothing — which is how this test passed for a claim that never
    // ran. Armed one epoch in, the claim really does walk forty of them.
    let activation = DEFAULT_EPOCH_BLOCK_INTERVAL;
    let mut harness = Harness::new(activation);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );

    // A genesis validator is its own delegator, so its self-stake is the single delegate-queue
    // entry the claim walks.
    let past_epoch = 20;
    let now = 40;
    harness.set_block_number(activation + DEFAULT_EPOCH_BLOCK_INTERVAL * now);
    let (_, current) = harness.call(encode_empty_call(SIG_CURRENT_EPOCH));
    assert_eq!(
        decode_output::<u64>(&current),
        now,
        "the chain must really be `now` epochs in, or the claim walks nothing"
    );

    let stake_before =
        staking::delegated_amount_at(&harness.sdk, validator, validator, past_epoch).unwrap();
    assert!(!stake_before.is_zero());
    assert!(selected_at(&harness.sdk, past_epoch).contains(&validator));

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
    // Anti-vacuum: the claim consumed its window. Without this the assertions
    // below hold for a claim that did nothing at all.
    assert_eq!(
        staking_storage()
            .validator_delegations_accessor()
            .entry(validator)
            .entry(validator)
            .claimed_through_epoch_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        now,
        "the claim must have advanced its own cursor across the window"
    );

    assert_eq!(
        staking::delegated_amount_at(&harness.sdk, validator, validator, past_epoch).unwrap(),
        stake_before,
        "a claim must not change what the self-stake was at an already-committed epoch"
    );
    assert!(
        selected_at(&harness.sdk, past_epoch).contains(&validator),
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

/// A validator that registers at a high epoch starts its reward cursor there,
/// not at zero. Left at zero the cursor would have to be walked forward
/// MAX_EPOCHS_PER_CLAIM epochs per call across history the validator did not
/// exist for, so the first payable epoch would cost hundreds of transactions
/// that all sum to zero.
#[test]
fn a_validator_registered_late_starts_its_reward_cursor_at_registration() {
    let owner = Address::with_last_byte(0xa0);
    let genesis = Address::with_last_byte(0x01);
    let latecomer = Address::with_last_byte(0x02);
    let activation = DEFAULT_EPOCH_BLOCK_INTERVAL;
    let mut harness = Harness::new(activation);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![genesis], vec![DEFAULT_MIN_VALIDATOR_STAKE], 500),
        ExitCode::Ok
    );

    // The genesis validator is the control: its changed_at is 0, so a zero
    // cursor was always correct for it and must stay zero.
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(genesis)
            .claimed_at_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "the genesis path is unchanged: changed_at is 0, so the cursor is 0"
    );

    // Register far enough in that a zero cursor would be unusable.
    let late_epoch = MAX_EPOCHS_PER_CLAIM * 3;
    harness.set_block_number(activation + DEFAULT_EPOCH_BLOCK_INTERVAL * late_epoch);
    harness.set_caller(latecomer);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_REGISTER_VALIDATOR,
                &RegisterValidatorCommand {
                    validator: latecomer,
                    commission_rate: 500,
                    initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                    bls_pubkey_uncompressed: Bytes::from(vec![
                        0x33;
                        BLS_PUBKEY_UNCOMPRESSED_LENGTH
                    ]),
                    bls_pop_uncompressed: Bytes::from(vec![0x44; BLS_POP_UNCOMPRESSED_LENGTH]),
                    peer_pubkey: B256::with_last_byte(2),
                },
            ))
            .0,
        ExitCode::Ok
    );

    let record = staking_storage().validators_accessor().entry(latecomer);
    let changed_at = record
        .changed_at_accessor()
        .get_checked(&harness.sdk)
        .unwrap();
    assert_eq!(
        changed_at,
        late_epoch + 1,
        "registration lands at next_epoch"
    );
    assert_eq!(
        record
            .claimed_at_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        changed_at,
        "the cursor starts at registration, so the first claim window covers \
         the validator's own history instead of the empty epochs before it"
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
    // The disable reaches the selection AT ONCE, not two epochs later: the
    // population is the live active set, and `disableValidator` removes the
    // validator from it in the same transaction. So the first commit after the
    // disable already derives a different set and mints the bit, and the two
    // commits after that derive the same set again and must not.
    let (_, output) = harness.call(encode_call(SIG_GET_DKG_QUAL, &U64Command { value: 1 }));
    assert!(
        decode_output::<bool>(&output),
        "a membership change must mint a DKG ceremony for the epoch that first seats it"
    );
    for unchanged_epoch in [2u64, 3] {
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
            ),
        )),
        ERR_INVALID_EVIDENCE_ENCODING,
    );
}

#[test]
fn equivocation_seizes_active_and_pending_self_delegation() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
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
            if let Some(reply) = mock_precompile_reply(address, input) {
                return reply;
            }
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

    consensus::seize_self_stake(&mut harness.sdk, validator, validator).unwrap();

    assert_eq!(
        transfers.borrow().as_slice(),
        &[(EQUIVOCATION_BURN_SINK, total_stake)]
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
    assert_eq!(transfers.borrow().len(), 1);
}

/// One slashable conflict shape and the two things that must agree on it: the
/// entry point that accepts it and the corpus blob carrying it.
///
/// Routing a kind to the wrong arm makes that kind permanently unslashable
/// on-chain while every other kind keeps working, so each test names the route
/// it drives instead of inheriting one.
struct ProofRoute {
    selector: u32,
    shape: evidence::EvidenceShape,
    blob: &'static [u8],
}

const NOTARIZE_ROUTE: ProofRoute = ProofRoute {
    selector: SIG_SLASH_EQUIVOCATION_NOTARIZE,
    shape: evidence::EvidenceShape::ConflictingNotarize,
    blob: &evidence::tests::CONFLICTING_NOTARIZE,
};

const FINALIZE_ROUTE: ProofRoute = ProofRoute {
    selector: SIG_SLASH_EQUIVOCATION_FINALIZE,
    shape: evidence::EvidenceShape::ConflictingFinalize,
    blob: &evidence::tests::CONFLICTING_FINALIZE,
};

/// Unlike the other two, this blob is 135 bytes and its first half is a bare
/// round rather than a proposal, so it only parses under its own shape.
const NULLIFY_FINALIZE_ROUTE: ProofRoute = ProofRoute {
    selector: SIG_SLASH_EQUIVOCATION_NULLIFY_FINALIZE,
    shape: evidence::EvidenceShape::NullifyFinalize,
    blob: &evidence::tests::NULLIFY_FINALIZE,
};

/// Builds a report against whichever validator owns the key `pk_byte`
/// compresses to, carrying `route`'s conflict.
///
/// The evidence is the node-side corpus blob and the two uncompressed signatures
/// are 128-byte G1 preimages of that blob's own compressed signatures, so the
/// contract's own `compress_g1_unchecked` reproduces exactly what the parser
/// finds inside the blob.
fn equivocation_report(
    sdk: &mut TestingContextImpl,
    route: &ProofRoute,
    pk_byte: u8,
) -> EquivocationCommand {
    let blob = Bytes::copy_from_slice(route.blob);
    let decoded = evidence::decode(sdk, &blob, route.shape).expect("the corpus blob parses");
    EquivocationCommand {
        evidence: blob,
        pk_uncompressed: Bytes::from(vec![pk_byte; BLS_PUBKEY_UNCOMPRESSED_LENGTH]),
        sig1_uncompressed: g1_preimage_of(&decoded.sig1),
        sig2_uncompressed: g1_preimage_of(&decoded.sig2),
    }
}

/// The epoch every corpus blob names, pinned by the decoder's own corpus tests.
/// It is the epoch a slash used to need a live committee record for.
const CORPUS_EPOCH: u64 = 7;

/// Sends the evidence-carrying slash in every test on that route. It is an
/// ordinary account, deliberately: the route is permissionless and pays its
/// submitter nothing, so who sends it changes nothing about where stake goes.
const EQUIVOCATION_RELAYER: Address = Address::with_last_byte(0xd7);

/// Drives the evidence-carrying route. Logs are drained first, so what the
/// caller reads back belongs to the slash alone.
fn slash_with_evidence(
    harness: &mut Harness,
    route: &ProofRoute,
    command: &EquivocationCommand,
) -> (ExitCode, Vec<u8>) {
    harness.sdk.take_logs();
    harness.set_caller(EQUIVOCATION_RELAYER);
    harness.call(encode_args_call(
        route.selector,
        &(
            command.evidence.clone(),
            command.pk_uncompressed.clone(),
            command.sig1_uncompressed.clone(),
            command.sig2_uncompressed.clone(),
        ),
    ))
}

/// Records every ERC-20 transfer the contract makes while still answering the
/// verifier calls the slash path depends on.
///
/// `signatures_valid` is the verdict the PAIRING stand-in gives; `false` is the
/// only way a test reaches the gate that decides whether the supplied signatures
/// actually belong to the named key.
fn record_transfers(
    harness: &Harness,
    signatures_valid: bool,
) -> Rc<RefCell<Vec<(Address, U256)>>> {
    if !signatures_valid {
        reject_every_pairing();
    }
    let transfers = Rc::new(RefCell::new(Vec::<(Address, U256)>::new()));
    let recorded = transfers.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            if let Some(reply) = mock_precompile_reply(address, input) {
                return reply;
            }
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
            match mock_external_return(selector, &input[SIG_LEN_BYTES..]) {
                Some(output) => SyscallResult::new(output, 0, 0, ExitCode::Ok),
                None => SyscallResult::new(Bytes::new(), 0, 0, ExitCode::MalformedBuiltinParams),
            }
        });
    transfers
}

/// The exact bytes `consensus::namespace` must produce, spelled out rather than
/// derived. Calling the private builder would only assert it equals itself; a
/// swapped kind constant has to fail here.
///
/// Layout: `b"FLUENT_DPOS_V1_"` ‖ chain id as u64 big-endian ‖ suffix. The
/// harness never sets a chain id and `fluentbase-testing` exposes no setter, so
/// it is zero.
const NS_NOTARIZE: &[u8] = b"FLUENT_DPOS_V1_\x00\x00\x00\x00\x00\x00\x00\x00_NOTARIZE";
const NS_NULLIFY: &[u8] = b"FLUENT_DPOS_V1_\x00\x00\x00\x00\x00\x00\x00\x00_NULLIFY";
const NS_FINALIZE: &[u8] = b"FLUENT_DPOS_V1_\x00\x00\x00\x00\x00\x00\x00\x00_FINALIZE";
const NS_ALL: &[&[u8]] = &[NS_NOTARIZE, NS_NULLIFY, NS_FINALIZE];

/// Records the namespace of every verify the contract performs, and accepts a
/// signature only under the namespaces listed in `accept`.
///
/// Answering `true` for any namespace at all makes the whole
/// kind -> domain-separator -> verify chain invisible: the message bytes of a
/// legal notarize/nullify pair and of a slashable pair are identical, and the
/// namespace is the only thing that tells them apart.
///
/// After the inline there is no ABI argument to read the namespace out of. It is
/// read out of the `expand_message_xmd` preimage the contract feeds SHA-256
/// instead — which is strictly better evidence: it is the domain the contract
/// actually hashed under, not the one it said it would.
fn record_verify_namespaces(
    harness: &Harness,
    accept: &'static [&'static [u8]],
) -> Rc<RefCell<Vec<Bytes>>> {
    let _ = harness;
    BLS_PRECOMPILES.with(|mock| {
        let mut mock = mock.borrow_mut();
        mock.accept = Some(accept.iter().map(|entry| entry.to_vec()).collect());
        mock.seen = Rc::new(RefCell::new(Vec::new()));
        mock.seen.clone()
    })
}

/// Every pairing fails, which is what an invalid signature looks like from this
/// contract's side.
fn reject_every_pairing() {
    BLS_PRECOMPILES.with(|mock| mock.borrow_mut().accept = Some(Vec::new()));
}

/// `address` answers one byte short of the width the verifier demands.
fn truncate_precompile(address: Address) {
    BLS_PRECOMPILES.with(|mock| mock.borrow_mut().truncate = Some(address));
}

/// `address` refuses the call.
fn refuse_precompile(address: Address) {
    BLS_PRECOMPILES.with(|mock| mock.borrow_mut().refuse = Some(address));
}

/// `address` answers the right width, all zeroes.
fn zero_precompile(address: Address) {
    BLS_PRECOMPILES.with(|mock| mock.borrow_mut().zero = Some(address));
}

/// A 256-byte EIP-2537 G2 with the four coordinates spelled out, so a test can
/// see which half of `x` the compression puts first and which coordinate the
/// sign is read from.
fn g2_point(x_c0: &[u8], x_c1: &[u8], y_c0: &[u8], y_c1: &[u8]) -> Vec<u8> {
    let mut point = vec![0u8; BLS_PUBKEY_UNCOMPRESSED_LENGTH];
    for (offset, coordinate) in [(16, x_c0), (80, x_c1), (144, y_c0), (208, y_c1)] {
        point[offset..offset + 48].copy_from_slice(coordinate);
    }
    point
}

/// `(p-1)/2`, the exact boundary of the y-sign rule.
const FIELD_HALF: [u8; 48] = hex!(
    "0d0088f51cbff34d258dd3db21a5d66bb23ba5c279c2895fb39869507b587b120f55ffff58a9ffffdcff7fffffffd555"
);

/// The zcash G2 form puts the IMAGINARY coefficient first while EIP-2537 puts
/// the real one first, and the sign comes from `y.c1` and only falls back to
/// `y.c0` when `y.c1` is zero. Both halves of that were a single line in the
/// Solidity and are a single line here; a swap produces a well-formed 96-byte
/// key that matches nothing any node registered.
#[test]
fn g2_compression_swaps_the_halves_and_reads_the_sign_from_c1() {
    let mut harness = Harness::new(0);
    let low = [0x01u8; 48];
    let high = [0xf0u8; 48];
    // Leading bytes deliberately clear of BOTH flag bits: a key whose own first
    // byte already carries `0x80` or `0x20` cannot tell a set flag from an
    // unset one, and reads as passing whatever the sign rule does.
    let x_c0 = [0x0au8; 48];
    let x_c1 = [0x15u8; 48];

    let point = g2_point(&x_c0, &x_c1, &high, &low);
    let compressed = bls::compress_g2_unchecked(&mut harness.sdk, &point).unwrap();
    // x.c1 leads, x.c0 follows, and the leading byte gains only the compression
    // flag: y.c1 is below the half, and y.c0 is NOT consulted because y.c1 is
    // non-zero — even though y.c0 here is above the half.
    assert_eq!(compressed[0], 0x15 | 0x80);
    assert_eq!(&compressed[1..48], &[0x15; 47]);
    assert_eq!(&compressed[48..], &[0x0a; 48]);

    // y.c1 zero hands the decision to y.c0.
    let point = g2_point(&x_c0, &x_c1, &high, &[0x00; 48]);
    let compressed = bls::compress_g2_unchecked(&mut harness.sdk, &point).unwrap();
    assert_eq!(compressed[0], 0x15 | 0x80 | 0x20);
}

/// The comparison against `(p-1)/2` is STRICT. At exactly the half the sign bit
/// stays clear; one above sets it. An off-by-one here mints a key that is the
/// negation of the one the node holds.
#[test]
fn the_y_sign_bit_is_strictly_above_half_the_field() {
    let mut harness = Harness::new(0);
    let mut above = FIELD_HALF;
    above[47] += 1;

    // `0x15` carries neither flag bit of its own, so the two answers differ.
    let at = g2_point(&[0x0a; 48], &[0x15; 48], &[0x00; 48], &FIELD_HALF);
    let over = g2_point(&[0x0a; 48], &[0x15; 48], &[0x00; 48], &above);
    assert_eq!(
        bls::compress_g2_unchecked(&mut harness.sdk, &at).unwrap()[0],
        0x15 | 0x80
    );
    assert_eq!(
        bls::compress_g2_unchecked(&mut harness.sdk, &over).unwrap()[0],
        0x15 | 0x80 | 0x20
    );
}

/// `verify` refuses infinity on all three of the points it handles.
///
/// PAIRING SKIPS an infinity pair rather than rejecting it, so an all-zero point
/// on any side would drop a factor out of `e(sig, -G2gen) · e(H, pk) == 1` and
/// leave the rest to hold on its own. The guard is the whole reason the equation
/// means anything, and it is three separate calls — the signature, the key, and
/// the hash the precompiles just produced. Nothing else in this file reaches any
/// of them: the two callers hand `verify` points that other checks already
/// rejected, and `H` comes back from a precompile that cannot be asked for
/// infinity except by a test that asks.
#[test]
fn verify_refuses_an_infinity_point_on_every_side() {
    let dst = b"BLS_POP_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_";
    let key = vec![0x11u8; BLS_PUBKEY_UNCOMPRESSED_LENGTH];
    let signature = vec![0x22u8; BLS_POP_UNCOMPRESSED_LENGTH];

    for (label, sig, pk, zero_g1add) in [
        (
            "signature",
            vec![0u8; BLS_POP_UNCOMPRESSED_LENGTH],
            key.clone(),
            false,
        ),
        (
            "public key",
            signature.clone(),
            vec![0u8; BLS_PUBKEY_UNCOMPRESSED_LENGTH],
            false,
        ),
        ("hash-to-curve result", signature.clone(), key.clone(), true),
    ] {
        let mut harness = Harness::new(0);
        if zero_g1add {
            zero_precompile(PRECOMPILE_G1ADD);
        }
        assert_eq!(
            bls::verify(&mut harness.sdk, b"ns", b"m", dst, &sig, &pk).err(),
            Some(ExitCode::Panic),
            "{label} at infinity was not refused"
        );
        assert_eq!(
            &harness.sdk.take_output()[..SIG_LEN_BYTES],
            &ERR_BLS_INFINITY_POINT.to_be_bytes(),
            "{label} at infinity raised the wrong error"
        );
    }
}

/// The G1 side of the same sign rule, on its own inputs.
///
/// Nothing else in this file reaches it: every G1 compression here runs over a
/// corpus signature whose leading byte already carries the flag bits, so a
/// dropped sign bit changes no byte. `y` is the only input that moves.
#[test]
fn g1_compression_takes_the_sign_from_y_alone() {
    let mut harness = Harness::new(0);
    let mut point = vec![0u8; BLS_POP_UNCOMPRESSED_LENGTH];
    point[16..64].fill(0x15);

    point[80..].copy_from_slice(&FIELD_HALF);
    let compressed = bls::compress_g1_unchecked(&mut harness.sdk, &point).unwrap();
    assert_eq!(compressed[0], 0x15 | 0x80);
    assert_eq!(&compressed[1..], &[0x15; 47]);

    let mut above = FIELD_HALF;
    above[47] += 1;
    point[80..].copy_from_slice(&above);
    let compressed = bls::compress_g1_unchecked(&mut harness.sdk, &point).unwrap();
    assert_eq!(compressed[0], 0x15 | 0x80 | 0x20);
    // `x` is untouched by the sign: only the leading byte's flags move.
    assert_eq!(&compressed[1..], &[0x15; 47]);
}

/// Compressing the all-zero EIP-2537 encoding would hand back a well-formed
/// `0x80…` reference for the point at infinity, which PAIRING SKIPS rather than
/// rejects — so the equation would hold for free. Both widths refuse it, and
/// both refuse a wrong width outright.
#[test]
fn compression_refuses_infinity_and_a_wrong_width() {
    let mut harness = Harness::new(0);
    type Compress = fn(&mut TestingContextImpl, &[u8]) -> Option<ExitCode>;
    let g2: Compress = |sdk, point| bls::compress_g2_unchecked(sdk, point).err();
    let g1: Compress = |sdk, point| bls::compress_g1_unchecked(sdk, point).err();
    for (compress, point, expected) in [
        (g2, vec![0u8; 256], ERR_BLS_INFINITY_POINT),
        (g1, vec![0u8; 128], ERR_BLS_INFINITY_POINT),
        (g2, vec![0x11u8; 255], ERR_BLS_INVALID_POINT_LENGTH),
        (g1, vec![0x11u8; 129], ERR_BLS_INVALID_POINT_LENGTH),
    ] {
        assert_eq!(compress(&mut harness.sdk, &point), Some(ExitCode::Panic));
        assert_eq!(
            &harness.sdk.take_output()[..SIG_LEN_BYTES],
            &expected.to_be_bytes()
        );
    }
}

/// The length prefix `union_unique` writes is ONE byte; the node writes a full
/// LEB128 varint. They agree only while every namespace stays under 128 bytes,
/// and the guard is what keeps the day they stop agreeing from being silent.
/// No caller can reach this today — both namespaces are built in this contract
/// and are 23 to 32 bytes — which is exactly why it is worth pinning: the
/// pressure on a later reader is to delete it as dead.
#[test]
fn a_namespace_that_would_outgrow_one_length_byte_is_refused() {
    let mut harness = Harness::new(0);
    let key = vec![0x11u8; BLS_PUBKEY_UNCOMPRESSED_LENGTH];
    let signature = vec![0x22u8; BLS_POP_UNCOMPRESSED_LENGTH];
    let dst = b"BLS_POP_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_";

    assert!(bls::verify(&mut harness.sdk, &[0x5a; 127], b"m", dst, &signature, &key).unwrap());
    assert_eq!(
        bls::verify(&mut harness.sdk, &[0x5a; 128], b"m", dst, &signature, &key).err(),
        Some(ExitCode::Panic)
    );
    assert_eq!(
        &harness.sdk.take_output()[..SIG_LEN_BYTES],
        &ERR_BLS_NAMESPACE_TOO_LONG.to_be_bytes()
    );
}

/// RFC 9380's long-DST workaround is deliberately absent, so a DST past the
/// short-DST limit is refused rather than quietly rehashed into a different
/// domain. Unreachable from a caller for the same reason as the namespace guard.
#[test]
fn a_dst_past_the_short_dst_limit_is_refused() {
    let mut harness = Harness::new(0);
    let key = vec![0x11u8; BLS_PUBKEY_UNCOMPRESSED_LENGTH];
    let signature = vec![0x22u8; BLS_POP_UNCOMPRESSED_LENGTH];

    assert!(bls::verify(
        &mut harness.sdk,
        b"ns",
        b"m",
        &[0x44; 255],
        &signature,
        &key
    )
    .unwrap());
    assert_eq!(
        bls::verify(
            &mut harness.sdk,
            b"ns",
            b"m",
            &[0x44; 256],
            &signature,
            &key
        )
        .err(),
        Some(ExitCode::Panic)
    );
    assert_eq!(
        &harness.sdk.take_output()[..SIG_LEN_BYTES],
        &ERR_BLS_DST_TOO_LONG.to_be_bytes()
    );
}

/// A precompile that answers the wrong width, or refuses, must stop the call.
///
/// A short MODEXP answer shifts the field element inside the 64-byte MAP input;
/// a short MAP or G1ADD answer shifts the 384-byte per-pair boundary inside the
/// PAIRING input. Either would be read as a different, well-formed point, and
/// the verify would then be answering about something nobody signed. Reachable
/// through the public registration path, not only in isolation.
#[test]
fn a_precompile_that_answers_wrongly_stops_the_registration() {
    for (address, break_it) in [
        (PRECOMPILE_MODEXP, truncate_precompile as fn(Address)),
        (PRECOMPILE_MAP_FP_TO_G1, truncate_precompile),
        (PRECOMPILE_G1ADD, truncate_precompile),
        (PRECOMPILE_SHA256, refuse_precompile),
        (PRECOMPILE_MODEXP, refuse_precompile),
    ] {
        let owner = Address::with_last_byte(0xa0);
        let validator = Address::with_last_byte(0x01);
        let mut harness = Harness::new(1_000);
        assert_eq!(
            harness.initialize(owner, Vec::new(), Vec::new(), 0),
            ExitCode::Ok
        );
        break_it(address);
        harness.set_caller(owner);

        let (exit, output) = harness.call(encode_args_call(
            SIG_REGISTER_VALIDATOR,
            &RegisterValidatorCommand {
                validator,
                commission_rate: 0,
                initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                bls_pubkey_uncompressed: Bytes::from(vec![0x11; BLS_PUBKEY_UNCOMPRESSED_LENGTH]),
                bls_pop_uncompressed: Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]),
                peer_pubkey: B256::with_last_byte(0x11),
            },
        ));
        assert_eq!(exit, ExitCode::Panic, "{address} did not stop the call");
        assert_eq!(
            &output[..SIG_LEN_BYTES],
            &ERR_BLS_PRECOMPILE_FAILED.to_be_bytes()
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
}

/// PAIRING is the one precompile whose refusal is NOT a revert: a bad point, a
/// dirty EIP-2537 pad, a non-canonical coordinate and a plain wrong signature
/// all arrive there and all mean "this signature is not valid", which is a
/// verdict the caller has to be able to act on rather than a malformed call.
#[test]
fn a_refused_pairing_is_an_invalid_signature_and_not_a_broken_call() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, Vec::new(), Vec::new(), 0),
        ExitCode::Ok
    );
    refuse_precompile(PRECOMPILE_PAIRING);
    harness.set_caller(owner);

    let (exit, output) = harness.call(encode_args_call(
        SIG_REGISTER_VALIDATOR,
        &RegisterValidatorCommand {
            validator,
            commission_rate: 0,
            initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
            bls_pubkey_uncompressed: Bytes::from(vec![0x11; BLS_PUBKEY_UNCOMPRESSED_LENGTH]),
            bls_pop_uncompressed: Bytes::from(vec![0x22; BLS_POP_UNCOMPRESSED_LENGTH]),
            peer_pubkey: B256::with_last_byte(0x11),
        },
    ));
    assert_eq!(exit, ExitCode::Panic);
    assert_eq!(
        &output[..SIG_LEN_BYTES],
        &ERR_INVALID_PROOF_OF_POSSESSION.to_be_bytes()
    );
}

/// Each slash route must hash its two messages under the domain separators its
/// message kinds name. This is the only thing separating a legal Simplex vote
/// pair from a slashable one: in Simplex a correct validator may notarize view
/// V and then nullify V on timeout, and those bytes are shape-identical to a
/// real conflict. Swap a kind constant and every other slash test stays green.
#[test]
fn each_slash_route_verifies_under_the_domain_its_kinds_name() {
    for (route, expected) in [
        (&NOTARIZE_ROUTE, [NS_NOTARIZE, NS_NOTARIZE]),
        (&FINALIZE_ROUTE, [NS_FINALIZE, NS_FINALIZE]),
        (&NULLIFY_FINALIZE_ROUTE, [NS_NULLIFY, NS_FINALIZE]),
    ] {
        let sponsor = Address::with_last_byte(0xa0);
        let offender = Address::with_last_byte(0x01);
        let bystander = Address::with_last_byte(0x02);
        let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
        let mut harness = Harness::new(1_000);
        assert_eq!(
            harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 0),
            ExitCode::Ok
        );
        let seen = record_verify_namespaces(&harness, NS_ALL);

        let command = equivocation_report(&mut harness.sdk, route, 0x11);
        assert_eq!(
            slash_with_evidence(&mut harness, route, &command).0,
            ExitCode::Ok
        );

        assert_eq!(
            seen.borrow()
                .iter()
                .map(|namespace| namespace.to_vec())
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|namespace| namespace.to_vec())
                .collect::<Vec<_>>(),
            "route {:#010x} hashed under the wrong domain separators",
            route.selector
        );
    }
}

/// A conflicting-notarize blob fed to the finalize entry point. The two 168-byte
/// conflicting corpora are byte-structurally identical, so this decodes cleanly
/// and reaches the verifier with `kind1 == kind2 == FINALIZE`; only the domain
/// separator stops it. Without that binding an attacker could route a blob
/// through whichever entry point suits it.
const NOTARIZE_BLOB_ON_FINALIZE_ROUTE: ProofRoute = ProofRoute {
    selector: SIG_SLASH_EQUIVOCATION_FINALIZE,
    shape: evidence::EvidenceShape::ConflictingFinalize,
    blob: &evidence::tests::CONFLICTING_NOTARIZE,
};

#[test]
fn a_blob_routed_through_the_wrong_entry_point_fails_verification() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 0),
        ExitCode::Ok
    );
    // The corpus signatures were minted under the notarize domain, so the
    // verifier accepts nothing else — exactly as a real BLS verifier would.
    let seen = record_verify_namespaces(&harness, &[NS_NOTARIZE]);

    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_BLOB_ON_FINALIZE_ROUTE, 0x11);
    assert_revert_selector(
        slash_with_evidence(&mut harness, &NOTARIZE_BLOB_ON_FINALIZE_ROUTE, &command),
        ERR_EQUIVOCATION_SIGNATURE_INVALID,
    );
    // ERR_EQUIVOCATION_SIGNATURE_INVALID is raised at two sites: the signature
    // hash gate that precedes the verifies, and the verify gate itself. Two
    // recorded namespaces are what tell them apart, so this count is the only
    // evidence the revert came from the domain and not from the earlier gate.
    // Two rather than one because both verifies are computed before the guard
    // reads either.
    assert_eq!(seen.borrow().len(), 2);

    // The rejection must be inert, not poisonous: the same offender still
    // slashes through the entry point its blob was actually signed for. A
    // wrong-route attempt that wrote partial state would let an attacker burn
    // somebody else's evidence for nothing.
    let honest = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11);
    assert_eq!(
        slash_with_evidence(&mut harness, &NOTARIZE_ROUTE, &honest).0,
        ExitCode::Ok
    );
    assert!(consensus_storage()
        .tombstoned_accessor()
        .entry(offender)
        .get_checked(&harness.sdk)
        .unwrap());
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
fn equivocation_slash_tombstones_jails_and_seizes_the_self_stake_whole() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 500),
        ExitCode::Ok
    );
    let transfers = record_transfers(&harness, true);

    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11);
    assert_eq!(
        slash_with_evidence(&mut harness, &NOTARIZE_ROUTE, &command).0,
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

    assert_eq!(
        transfers.borrow().as_slice(),
        &[(EQUIVOCATION_BURN_SINK, stake)],
        "the whole seizure goes to the fund; nobody is paid for reporting"
    );

    let logs = harness.sdk.take_logs();
    assert_eq!(logs.len(), 3);
    let (jailed_data, jailed_topics) = find_log(&logs, events::ValidatorJailed::SELECTOR, "jail");
    assert_eq!(&jailed_topics[1].0[12..], offender.as_slice());
    assert_eq!(decode_output::<u64>(jailed_data), 0);
    let (slashed_data, slashed_topics) =
        find_log(&logs, events::EquivocationSlashed::SELECTOR, "slash");
    assert_eq!(&slashed_topics[1].0[12..], offender.as_slice());
    assert_eq!(
        decode_output::<u64>(slashed_data),
        CORPUS_EPOCH,
        "the event reports the epoch the conflict happened in, not the penalty epoch"
    );
    let (seized_data, seized_topics) =
        find_log(&logs, events::EquivocationStakeSeized::SELECTOR, "seizure");
    assert_eq!(&seized_topics[1].0[12..], offender.as_slice());
    assert_eq!(
        decode_output::<(U256, Address)>(seized_data),
        (stake, EQUIVOCATION_BURN_SINK)
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
            if let Some(reply) = mock_precompile_reply(address, input) {
                return reply;
            }
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
    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11);
    assert_eq!(
        slash_with_evidence(&mut harness, &NOTARIZE_ROUTE, &command).0,
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

    assert_eq!(
        transfers.borrow().as_slice(),
        &[(EQUIVOCATION_BURN_SINK, stake + warmup)]
    );
}

/// The two refusal vectors `try_transfer` handles in different branches, driven
/// against the one recipient a seizure has. Covering only one leaves the other
/// live, and the recipient is a burn sink no caller chooses — so a refusal that
/// propagated would make equivocation unslashable chain-wide.
#[test]
fn a_slash_survives_a_fund_that_refuses_the_seizure() {
    for hard_revert in [false, true] {
        let sponsor = Address::with_last_byte(0xa0);
        let offender = Address::with_last_byte(0x01);
        let bystander = Address::with_last_byte(0x02);
        let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
        let mut harness = Harness::new(1_000);
        assert_eq!(
            harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 500),
            ExitCode::Ok
        );
        let transfers =
            record_transfers_refusing(&harness, vec![EQUIVOCATION_BURN_SINK], hard_revert);

        let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11);
        assert_eq!(
            slash_with_evidence(&mut harness, &NOTARIZE_ROUTE, &command).0,
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

        assert_eq!(
            transfers.borrow().as_slice(),
            &[(EQUIVOCATION_BURN_SINK, stake)],
            "the payout is attempted, and the refusal does not propagate"
        );
        let logs = harness.sdk.take_logs();
        let (seized_data, _) =
            find_log(&logs, events::EquivocationStakeSeized::SELECTOR, "seizure");
        assert_eq!(
            decode_output::<(U256, Address)>(seized_data),
            (U256::ZERO, EQUIVOCATION_BURN_SINK),
            "the event reports what moved, not what was intended"
        );
    }
}

// The evidence route takes identity from the key, never from a committee seat,
// so an epoch with no committee at all must not stop it. This used to be staged
// by pruning the epoch away; committees are never deleted now, so the same state
// is reached the only way still open — by naming an epoch that was never
// committed.
#[test]
fn an_uncommitted_evidence_epoch_does_not_block_a_slash() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    let (validators, stakes) = with_filler_validators(&[(offender, DEFAULT_MIN_VALIDATOR_STAKE)]);
    assert_eq!(
        harness.initialize(sponsor, validators, stakes, 0),
        ExitCode::Ok
    );

    let consensus = consensus_storage();
    assert_eq!(
        consensus
            .epoch_index_accessor()
            .entry(CORPUS_EPOCH)
            .length_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "the epoch the evidence names has no committee"
    );

    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11);
    assert_eq!(
        slash_with_evidence(&mut harness, &NOTARIZE_ROUTE, &command).0,
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

// The index route is the one pruning actually broke. `committee_member_at`
// reverts with `ERR_EPOCH_COMMITTEE_NOT_COMMITTED` on an emptied epoch, and the
// node soft-folds that revert — a real verdict the committee already reached was
// dropped in silence. It is a system call, so nothing downstream could retry it.
#[test]
fn the_index_slash_route_still_resolves_far_past_the_retired_pruning_horizon() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let activation_block = 1_000;
    let mut harness = Harness::new(activation_block);
    let (validators, stakes) = with_filler_validators(&[(offender, DEFAULT_MIN_VALIDATOR_STAKE)]);
    assert_eq!(
        harness.initialize(sponsor, validators.clone(), stakes, 0),
        ExitCode::Ok
    );

    harness.set_caller(SYSTEM_CALLER);
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );
    let seat = validators
        .iter()
        .position(|member| *member == offender)
        .expect("the offender is seated in epoch 0") as u32;

    harness.set_block_number(
        activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * (DEFAULT_UNDELEGATE_PERIOD + 100),
    );
    assert_eq!(
        harness
            .call(encode_empty_call(SIG_COMMIT_EPOCH_COMMITTEE))
            .0,
        ExitCode::Ok
    );

    assert_eq!(system_slash(&mut harness, 0, seat).0, ExitCode::Ok);
    assert!(consensus_storage()
        .tombstoned_accessor()
        .entry(offender)
        .get_checked(&harness.sdk)
        .unwrap());
}

#[test]
fn a_registered_but_never_activated_validator_can_be_slashed() {
    let sponsor = Address::with_last_byte(0xa0);
    let seated = Address::with_last_byte(0x01);
    let offender = Address::with_last_byte(0x02);
    let offender_owner = Address::with_last_byte(0xa2);
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
    let command = equivocation_report(&mut harness.sdk, &FINALIZE_ROUTE, 0x40);
    assert_eq!(
        slash_with_evidence(&mut harness, &FINALIZE_ROUTE, &command).0,
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
    assert_eq!(
        transfers.borrow().as_slice(),
        &[(EQUIVOCATION_BURN_SINK, stake)],
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

/// The third arm decodes under `NullifyFinalize`, so its 135-byte blob would not
/// parse under either conflicting shape — a binding neither of the other two
/// entry points can cover for it.
#[test]
fn a_nullify_finalize_conflict_is_slashable_through_its_own_entry_point() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 0),
        ExitCode::Ok
    );
    let transfers = record_transfers(&harness, true);

    let command = equivocation_report(&mut harness.sdk, &NULLIFY_FINALIZE_ROUTE, 0x11);
    assert_eq!(
        slash_with_evidence(&mut harness, &NULLIFY_FINALIZE_ROUTE, &command).0,
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
        transfers.borrow().as_slice(),
        &[(EQUIVOCATION_BURN_SINK, stake)]
    );

    let logs = harness.sdk.take_logs();
    let (slashed_data, slashed_topics) =
        find_log(&logs, events::EquivocationSlashed::SELECTOR, "slash");
    assert_eq!(&slashed_topics[1].0[12..], offender.as_slice());
    assert_eq!(decode_output::<u64>(slashed_data), CORPUS_EPOCH);
}

#[test]
fn a_slash_with_nothing_to_seize_still_tombstones() {
    let sponsor = Address::with_last_byte(0xa0);
    let seated = Address::with_last_byte(0x01);
    let offender = Address::with_last_byte(0x02);
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
                SIG_WITHDRAW_DELEGATOR_PRINCIPAL,
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
    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x40);
    assert_eq!(
        slash_with_evidence(&mut harness, &NOTARIZE_ROUTE, &command).0,
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
    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x77);
    assert_revert_selector(
        slash_with_evidence(&mut harness, &NOTARIZE_ROUTE, &command),
        ERR_EQUIVOCATION_KEY_NOT_REGISTERED,
    );
}

#[test]
fn a_slash_whose_signatures_fail_verification_is_rejected() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender], vec![stake], 0),
        ExitCode::Ok
    );
    // The blob's signatures are well-formed and belong to a registered key, so
    // everything up to the pairing passes; only the verifier's verdict rejects.
    let transfers = record_transfers(&harness, false);

    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11);
    assert_revert_selector(
        slash_with_evidence(&mut harness, &NOTARIZE_ROUTE, &command),
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
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 0),
        ExitCode::Ok
    );
    let transfers = record_transfers(&harness, true);

    let command = equivocation_report(&mut harness.sdk, &NOTARIZE_ROUTE, 0x11);
    assert_eq!(
        slash_with_evidence(&mut harness, &NOTARIZE_ROUTE, &command).0,
        ExitCode::Ok
    );
    let paid = transfers.borrow().clone();
    assert_eq!(paid.len(), 1);

    assert_revert_selector(
        slash_with_evidence(&mut harness, &NOTARIZE_ROUTE, &command),
        ERR_ALREADY_SLASHED_FOR_EQUIVOCATION,
    );
    assert_eq!(
        transfers.borrow().as_slice(),
        paid.as_slice(),
        "the refused re-slash must not seize a second time"
    );
    assert!(harness.sdk.take_logs().is_empty());
}

fn system_slash_calldata(epoch: u64, signer_idx: u32) -> Vec<u8> {
    encode_call(
        SIG_SLASH_EQUIVOCATION,
        &EpochSignerCommand { epoch, signer_idx },
    )
}

/// Drives the system entry the way the node's pre-execution stage does. Logs are
/// drained first, so what the caller reads back belongs to this verdict alone.
fn system_slash(harness: &mut Harness, epoch: u64, signer_idx: u32) -> (ExitCode, Vec<u8>) {
    harness.sdk.take_logs();
    harness.set_caller(SYSTEM_CALLER);
    harness.call(system_slash_calldata(epoch, signer_idx))
}

#[test]
fn the_system_slash_entry_refuses_every_other_caller() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 0),
        ExitCode::Ok
    );
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[(offender, stake), (bystander, stake)],
    );

    for caller in [Address::with_last_byte(0xb0), GENESIS_GOVERNANCE, offender] {
        harness.set_caller(caller);
        assert_revert_selector(
            harness.call(system_slash_calldata(0, 0)),
            ERR_ONLY_SYSTEM_CALL,
        );
    }
    assert!(!consensus_storage()
        .tombstoned_accessor()
        .entry(offender)
        .get_checked(&harness.sdk)
        .unwrap());
}

/// The index route resolves a seat number against ONE array, and both ways of
/// naming a seat that array does not have must be refused before anything is
/// written.
///
/// Neither refusal had a test. Without them an uncommitted epoch reads the index
/// pair `(record 0, length 0)` and resolves seat 0 against whatever record 0
/// holds — some other epoch's committee — and an out-of-range seat reads past the
/// end of a real record. Both tombstone a validator the verdict never named, and
/// the caller is the node's pre-execution stage, which cannot take it back.
#[test]
fn a_system_verdict_naming_a_seat_the_committee_does_not_have_is_refused() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let uncommitted_epoch = 9u64;
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 0),
        ExitCode::Ok
    );
    // Record 0 exists and holds two members; epoch 9 has no index entry at all,
    // so its `(record, length)` reads `(0, 0)` — which is exactly the state that
    // makes an unguarded resolve land on THIS record.
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[(offender, stake), (bystander, stake)],
    );
    assert_eq!(
        consensus::committee_at(&harness.sdk, uncommitted_epoch).unwrap(),
        (0, 0),
        "the uncommitted epoch must point at record 0 with length 0"
    );

    assert_revert_selector(
        system_slash(&mut harness, uncommitted_epoch, 0),
        ERR_EPOCH_COMMITTEE_NOT_COMMITTED,
    );
    assert_revert_selector(
        system_slash(&mut harness, 0, 2),
        ERR_SIGNER_INDEX_OUT_OF_RANGE,
    );

    for validator in [offender, bystander] {
        assert!(
            !consensus_storage()
                .tombstoned_accessor()
                .entry(validator)
                .get_checked(&harness.sdk)
                .unwrap(),
            "a verdict that named no real seat must tombstone nobody"
        );
    }
    // And the route still works for a seat the committee does have, so the
    // refusals above are not a blanket one.
    assert_eq!(system_slash(&mut harness, 0, 1).0, ExitCode::Ok);
    assert!(consensus_storage()
        .tombstoned_accessor()
        .entry(bystander)
        .get_checked(&harness.sdk)
        .unwrap());
}

#[test]
fn a_system_verdict_tombstones_jails_and_sends_the_whole_seizure_to_the_fund() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let fund = Address::with_last_byte(0xc1);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 500),
        ExitCode::Ok
    );
    harness.set_caller(GENESIS_GOVERNANCE);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_SET_SLASH_FUND_ADDRESS,
                &AddressCommand { value: fund },
            ))
            .0,
        ExitCode::Ok
    );
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[(offender, stake), (bystander, stake)],
    );
    let transfers = record_transfers(&harness, true);

    assert_eq!(system_slash(&mut harness, 0, 0).0, ExitCode::Ok);

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
    assert_eq!(active.at(0).get_checked(&harness.sdk).unwrap(), bystander);
    // The penalty is stamped at epoch 0, so it lands from epoch 1 on.
    assert!(!staking::selection_visible_at(&harness.sdk, offender, 1).unwrap());
    assert!(staking::selection_visible_at(&harness.sdk, bystander, 1).unwrap());

    assert_eq!(
        transfers.borrow().as_slice(),
        &[(fund, stake)],
        "the configured fund receives the seizure whole"
    );
    let logs = harness.sdk.take_logs();
    let (seized_data, _) = find_log(&logs, events::EquivocationStakeSeized::SELECTOR, "seizure");
    assert_eq!(decode_output::<(U256, Address)>(seized_data), (stake, fund));
    let (slashed_data, _) = find_log(&logs, events::EquivocationSlashed::SELECTOR, "slash");
    assert_eq!(decode_output::<u64>(slashed_data), 0);
}

/// The node decides whose proposals it will still bind from the committee
/// snapshot it already reads, so a verdict invisible there changes nothing off
/// chain. The flag is positional: it names the member at its own index and no
/// other.
#[test]
fn the_committee_snapshot_reports_the_tombstone_against_its_own_member() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 0),
        ExitCode::Ok
    );
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[(offender, stake), (bystander, stake)],
    );

    let snapshot = |harness: &mut Harness| -> (Vec<Address>, Vec<bool>) {
        let (exit, output) = harness.call(encode_call(
            SIG_GET_EPOCH_COMMITTEE_WITH_STAKES,
            &U64Command { value: 0 },
        ));
        assert_eq!(exit, ExitCode::Ok);
        let (validators, _, _, tombstoned): (
            Vec<Address>,
            Vec<ConsensusKeys>,
            Vec<U256>,
            Vec<bool>,
        ) = decode_returns(&output);
        (validators, tombstoned)
    };

    let (validators, tombstoned) = snapshot(&mut harness);
    assert_eq!(validators, vec![offender, bystander]);
    assert_eq!(tombstoned, vec![false, false]);

    assert_eq!(system_slash(&mut harness, 0, 0).0, ExitCode::Ok);

    let (validators, tombstoned) = snapshot(&mut harness);
    assert_eq!(
        validators,
        vec![offender, bystander],
        "the frozen membership does not move when a member is slashed"
    );
    assert_eq!(
        tombstoned,
        vec![true, false],
        "the flag names the slashed member and only it"
    );
}

/// Two proposers may carry the same charge, and the epoch-boundary fallback may
/// land beside a block-borne one. The second verdict has to be inert rather than
/// a failed system call.
#[test]
fn a_second_system_verdict_against_the_same_validator_is_a_no_op() {
    let sponsor = Address::with_last_byte(0xa0);
    let offender = Address::with_last_byte(0x01);
    let bystander = Address::with_last_byte(0x02);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![offender, bystander], vec![stake, stake], 500),
        ExitCode::Ok
    );
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[(offender, stake), (bystander, stake)],
    );
    let transfers = record_transfers(&harness, true);

    assert_eq!(system_slash(&mut harness, 0, 0).0, ExitCode::Ok);
    let paid = transfers.borrow().clone();
    assert_eq!(paid.as_slice(), &[(EQUIVOCATION_BURN_SINK, stake)]);

    assert_eq!(system_slash(&mut harness, 0, 0).0, ExitCode::Ok);
    assert_eq!(
        transfers.borrow().as_slice(),
        paid.as_slice(),
        "the repeat must not seize a second time"
    );
    assert!(harness.sdk.take_logs().is_empty());
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

/// Called directly rather than through `Harness::call`, which restores storage on
/// any revert and would hide the write this pins. On a chain the revert unwinds
/// the frame too — the point is that the tombstone is never written, not that
/// something else undoes it.
#[test]
fn a_verdict_naming_a_validator_with_no_record_writes_no_tombstone() {
    let sponsor = Address::with_last_byte(0xa0);
    let seated = Address::with_last_byte(0x01);
    let stranger = Address::with_last_byte(0x03);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![seated], vec![stake], 0),
        ExitCode::Ok
    );
    commit_test_committee(&mut harness.sdk, 0, &[(seated, stake), (stranger, stake)]);
    assert_eq!(
        staking_storage()
            .validators_accessor()
            .entry(stranger)
            .status_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        STATUS_NOT_FOUND
    );

    harness.set_caller(SYSTEM_CALLER);
    let calldata = system_slash_calldata(0, 1);
    assert_direct_revert(
        consensus::slash_equivocation(&mut harness.sdk, &calldata[SIG_LEN_BYTES..]),
        &harness.sdk,
        ERR_VALIDATOR_NOT_FOUND,
    );
    assert!(!consensus_storage()
        .tombstoned_accessor()
        .entry(stranger)
        .get_checked(&harness.sdk)
        .unwrap());
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

// The refusal reason changed on 2026-09-07 with the selection: it used to be "no
// replacement below the cut, so the committee would shrink". A short committee is
// no longer the failure — a committee under MIN_COMMITTEE_LENGTH is, because the
// commit reverts there and the commit is a pre-execution system call.
#[test]
fn production_exclusion_refuses_at_the_committee_floor_and_leaves_no_trace() {
    let owner = Address::with_last_byte(0xa0);
    let validators: Vec<Address> = (1..=MIN_COMMITTEE_LENGTH)
        .map(|index| Address::with_last_byte(index as u8))
        .collect();
    let victim = validators[MIN_COMMITTEE_LENGTH - 1];
    let mut harness = Harness::new(1_000);
    let mut command = harness.initialize_command(
        owner,
        validators.clone(),
        vec![DEFAULT_MIN_VALIDATOR_STAKE * U256::from(3); MIN_COMMITTEE_LENGTH],
        0,
    );
    command.active_validators_length = MIN_COMMITTEE_LENGTH as u32;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
    harness.sdk.take_logs();

    assert!(
        !staking::apply_production_exclusion(&mut harness.sdk, victim).unwrap(),
        "the population sits exactly on the floor, so excluding a member would make \
         the next commit revert"
    );
    assert!(staking::selection_visible_at(&harness.sdk, victim, 1).unwrap());
    assert_eq!(selected_at(&harness.sdk, 1), validators);
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
    let fourth = Address::with_last_byte(0x04);
    let fifth = Address::with_last_byte(0x05);
    let sixth = Address::with_last_byte(0x06);
    let mut harness = Harness::new(1_000);
    // SIX Active validators against a cap of four: one below the cut to take the
    // freed seat, and — the reason for the sixth — a population that still
    // clears `MIN_COMMITTEE_LENGTH + 1` AFTER the first exclusion. At five the
    // floor guard alone refuses the repeat stamp, so the repeat leg below was
    // green whether or not the already-invisible refusal existed at all.
    let mut command = harness.initialize_command(
        owner,
        vec![first, second, third, fourth, fifth, sixth],
        vec![
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(6),
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(5),
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4),
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(3),
            DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2),
            DEFAULT_MIN_VALIDATOR_STAKE,
        ],
        0,
    );
    command.active_validators_length = MIN_COMMITTEE_LENGTH as u32;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
    harness.sdk.take_logs();

    assert!(staking::apply_production_exclusion(&mut harness.sdk, second).unwrap());
    assert!(
        staking::selection_visible_at(&harness.sdk, second, 0).unwrap(),
        "epoch 0 has already started and its selection view must not be rewritten"
    );
    assert!(!staking::selection_visible_at(&harness.sdk, second, 1).unwrap());
    assert_eq!(
        selected_at(&harness.sdk, 0),
        vec![first, second, third, fourth],
        "epoch 0 has already started and keeps the member the stamp will remove"
    );
    assert_eq!(
        selected_at(&harness.sdk, 1),
        vec![first, third, fourth, fifth],
        "the freed seat is taken by the next validator by stake, not left empty"
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
    // Anti-vacuum: the repeat leg proves something only while the OTHER refusal
    // — the committee floor — would still let a second stamp through. Asserted
    // rather than left to the fixture's member count, because shrinking the
    // roster by one silently turns the leg below into a test of the floor guard.
    assert!(
        staking::eligible_population_at_least(&harness.sdk, 1, MIN_COMMITTEE_LENGTH as u64 + 1)
            .unwrap(),
        "the population must still clear the floor after the first exclusion, or \
         the repeat below is refused for the wrong reason"
    );
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
    // Seven Active validators: the test excludes three in a row, and each exclusion
    // has to leave MIN_COMMITTEE_LENGTH behind it, so the population walks 7-6-5-4.
    let spare: Vec<Address> = (5..=7).map(Address::with_last_byte).collect();
    let mut validators = vec![keeper, tombstoned, demoted, healthy];
    validators.extend(spare);
    let mut command = harness.initialize_command(
        owner,
        validators.clone(),
        vec![DEFAULT_MIN_VALIDATOR_STAKE * U256::from(4); 7],
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
    // `demoted` was written straight to storage, so it is STILL in
    // `active_validators` — the one state where the population count's
    // `STATUS_ACTIVE` term is load-bearing rather than redundant with
    // `remove_active`. Six, not seven: drop that term and this reads 7 and the
    // exclusion guard starts sanctioning a validator the commit will not seat.
    assert!(
        staking::eligible_population_at_least(&harness.sdk, 0, (validators.len() - 1) as u64)
            .unwrap(),
        "the six still-Active validators must be counted"
    );
    assert!(
        !staking::eligible_population_at_least(&harness.sdk, 0, validators.len() as u64).unwrap(),
        "a non-Active validator left in the active list must not be counted"
    );
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
    assert_eq!(selected_at(&harness.sdk, 1), vec![keeper]);

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

#[cfg(feature = "devnet-views")]
#[test]
fn production_liveness_views_read_the_new_namespace() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
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

/// Drives the real selector at `block_number`.
///
/// The height is the block context now, not an argument, so it is set here
/// rather than at each call site — a caller that wants the contract to see a
/// height cannot express it any other way.
fn record_production(harness: &mut Harness, block_number: u64, leader_index: u8) -> ExitCode {
    harness.set_caller(SYSTEM_CALLER);
    harness.set_block_number(block_number);
    harness
        .call(encode_call(
            SIG_RECORD_PRODUCTION,
            &RecordProductionCommand { leader_index },
        ))
        .0
}

fn seed_epoch_production(
    sdk: &mut TestingContextImpl,
    epoch: u64,
    produced: &[u32],
    recorded: u32,
) {
    seed_epoch_counters(sdk, epoch, produced, recorded);
    staking::accrue_epoch(sdk, epoch, recorded).unwrap();
}

/// The block counters alone, with no accrual.
///
/// For a test whose subject is the accrual itself: seeding through
/// `seed_epoch_production` would price the epoch once already, against whatever
/// mock happened to be installed at the time.
fn seed_epoch_counters(sdk: &mut TestingContextImpl, epoch: u64, produced: &[u32], recorded: u32) {
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
    record_production(harness, boundary, 0)
}

fn production_record(sdk: &TestingContextImpl, validator: Address) -> (u64, u64, u32) {
    let record = production_liveness_storage()
        .validators_accessor()
        .entry(validator);
    (
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

    assert_eq!(record_production(&mut harness, 1_001, 0), ExitCode::Ok);
    assert_eq!(record_production(&mut harness, 1_001, 0), ExitCode::Ok);
    assert_eq!(record_production(&mut harness, 1_000, 0), ExitCode::Ok);

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
    assert_eq!(
        storage
            .produced_accessor()
            .entry(0u64)
            .entry(0u32)
            .get_checked(&harness.sdk)
            .unwrap(),
        1
    );
    assert_eq!(
        storage
            .last_processed_block_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1_001,
        "the lower arrival must not roll the cursor back"
    );

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
    assert_eq!(decode_output::<(u32, u32)>(&partial[0].0).0, 1);
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
    assert_eq!(production_record(&harness.sdk, members[0]), (0, 0, 0));

    harness.set_block_number(1_200);
    harness.sdk.take_logs();
    assert_eq!(record_production(&mut harness, 1_200, 0), ExitCode::Ok);
    let logs = harness.sdk.take_logs();
    assert_eq!(
        decode_output::<(u32, u32)>(&logs_of(&logs, events::PartialEpoch::SELECTOR)[0].0).0,
        0,
        "a fully parked epoch is tainted rather than silently complete"
    );
    let committed = logs_of(&logs, events::EpochBlendRewardsCommitted::SELECTOR);
    assert_eq!(committed.len(), 1);
    assert_eq!(
        decode_output::<U256>(&committed[0].0),
        U256::ZERO,
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
    equal_weight_committee(&mut harness.sdk, 1, &members);
    equal_weight_committee(&mut harness.sdk, 2, &members);

    let produced = [100, 100, 0, 0];
    seed_epoch_production(&mut harness.sdk, 1, &produced, 199);
    harness.sdk.take_logs();
    assert_eq!(close_epoch_via_record(&mut harness, 1), ExitCode::Ok);

    let logs = harness.sdk.take_logs();
    assert_eq!(logs_of(&logs, events::PartialEpoch::SELECTOR).len(), 1);
    assert!(
        logs_of(&logs, events::ProductionVerdictFailed::SELECTOR).is_empty(),
        "one missing record must cost the whole epoch its verdicts"
    );
    for member in &members {
        assert_eq!(production_record(&harness.sdk, *member).0, 0);
    }
    assert!(pending_exclusion_set(&harness.sdk).is_empty());

    seed_epoch_production(&mut harness.sdk, 2, &produced, 200);
    harness.sdk.take_logs();
    assert_eq!(close_epoch_via_record(&mut harness, 2), ExitCode::Ok);
    let logs = harness.sdk.take_logs();
    assert!(logs_of(&logs, events::PartialEpoch::SELECTOR).is_empty());
    assert_eq!(
        logs_of(&logs, events::ProductionVerdictFailed::SELECTOR).len(),
        2,
        "the same production judges normally once the epoch is complete"
    );
}

// The height is the contract's own block context, not an argument, so there is
// no channel through which a node could report a height it is not executing at.
// The belt still has to hold against a replay of that same block.
#[test]
fn the_recorder_takes_its_height_from_the_block_context() {
    let (mut harness, members) = liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE], 21);
    commit_test_committee(
        &mut harness.sdk,
        0,
        &[(members[0], DEFAULT_MIN_VALIDATOR_STAKE)],
    );

    assert_eq!(record_production(&mut harness, 1_150, 0), ExitCode::Ok);
    let storage = production_liveness_storage();
    assert_eq!(
        storage
            .last_processed_block_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        1_150,
        "the cursor lands on the height the block is executing at"
    );

    assert_eq!(record_production(&mut harness, 1_150, 0), ExitCode::Ok);
    assert_eq!(
        storage
            .blocks_in_epoch_accessor()
            .entry(0u64)
            .get_checked(&harness.sdk)
            .unwrap(),
        1,
        "re-executing the same block credits it once"
    );
}

// `sum(produced) == blocks_in_epoch` is what makes the taint derivable, and it
// rests entirely on the two increments sitting after both park arms: a parked
// block must move neither counter. Every other test in this file seeds the two
// counters directly, so this is the only place the real recorder maintains them.
#[test]
fn the_recorder_keeps_the_block_count_equal_to_the_sum_of_its_credits() {
    let (mut harness, members) = liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE; 3], 21);
    equal_weight_committee(&mut harness.sdk, 0, &members);

    // From `activation + 1`: the activation height itself carries empty
    // `extra_data` and is never recorded, which is what makes epoch 0 one block
    // shorter than the interval.
    for (block, leader) in [(1_001u64, 0u8), (1_002, 1), (1_003, 7), (1_004, 1)] {
        assert_eq!(record_production(&mut harness, block, leader), ExitCode::Ok);
    }

    let storage = production_liveness_storage();
    let recorded = storage
        .blocks_in_epoch_accessor()
        .entry(0u64)
        .get_checked(&harness.sdk)
        .unwrap();
    let credited: u32 = (0..8)
        .map(|index| {
            storage
                .produced_accessor()
                .entry(0u64)
                .entry(index)
                .get_checked(&harness.sdk)
                .unwrap()
        })
        .sum();
    assert_eq!(
        (recorded, credited),
        (3, 3),
        "the out-of-range leader index is neither counted nor credited"
    );
}

// Epoch 0 is one block shorter than every other epoch by construction: the
// activation block is produced by the pre-DPoS sequencer, which holds no
// committee position, so no record is ever issued for it. Expecting the full
// interval there would taint a healthy first day on every chain and leave it
// permanently unjudged.
#[test]
fn a_healthy_epoch_zero_is_complete_one_block_short_of_the_interval() {
    let (mut harness, members) =
        liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2); 4], 2);
    set_min_verdict_due_blocks(&mut harness, 10);
    equal_weight_committee(&mut harness.sdk, 0, &members);

    seed_epoch_production(
        &mut harness.sdk,
        0,
        &[100, 99, 0, 0],
        DEFAULT_EPOCH_BLOCK_INTERVAL as u32 - 1,
    );
    harness.sdk.take_logs();
    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);

    let logs = harness.sdk.take_logs();
    assert!(
        logs_of(&logs, events::PartialEpoch::SELECTOR).is_empty(),
        "one block short IS the full length of epoch 0"
    );
    assert_eq!(
        logs_of(&logs, events::ProductionVerdictFailed::SELECTOR).len(),
        2,
        "and a complete epoch 0 is judged like any other"
    );
}

// The shortened expectation is a property of epoch 0 alone. Every later epoch
// spans `interval` recordable heights, so the same count that completes epoch 0
// taints epoch 1.
#[test]
fn only_epoch_zero_gets_the_shortened_expectation() {
    let (mut harness, members) =
        liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2); 4], 2);
    set_min_verdict_due_blocks(&mut harness, 10);
    equal_weight_committee(&mut harness.sdk, 1, &members);

    seed_epoch_production(
        &mut harness.sdk,
        1,
        &[100, 99, 0, 0],
        DEFAULT_EPOCH_BLOCK_INTERVAL as u32 - 1,
    );
    harness.sdk.take_logs();
    assert_eq!(close_epoch_via_record(&mut harness, 1), ExitCode::Ok);

    let logs = harness.sdk.take_logs();
    let partial = logs_of(&logs, events::PartialEpoch::SELECTOR);
    assert_eq!(partial.len(), 1);
    assert_eq!(
        decode_output::<(u32, u32)>(&partial[0].0),
        (
            DEFAULT_EPOCH_BLOCK_INTERVAL as u32 - 1,
            DEFAULT_EPOCH_BLOCK_INTERVAL as u32
        )
    );
}

// The expectation moved by exactly one, not to "anything below the interval":
// epoch 0 still has to record every height it actually owns.
#[test]
fn an_epoch_zero_two_blocks_short_is_still_partial() {
    let (mut harness, members) =
        liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2); 4], 2);
    set_min_verdict_due_blocks(&mut harness, 10);
    equal_weight_committee(&mut harness.sdk, 0, &members);

    seed_epoch_production(
        &mut harness.sdk,
        0,
        &[100, 98, 0, 0],
        DEFAULT_EPOCH_BLOCK_INTERVAL as u32 - 2,
    );
    harness.sdk.take_logs();
    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);

    let logs = harness.sdk.take_logs();
    let partial = logs_of(&logs, events::PartialEpoch::SELECTOR);
    assert_eq!(partial.len(), 1);
    assert_eq!(
        decode_output::<(u32, u32)>(&partial[0].0),
        (
            DEFAULT_EPOCH_BLOCK_INTERVAL as u32 - 2,
            DEFAULT_EPOCH_BLOCK_INTERVAL as u32 - 1
        )
    );
    assert!(
        logs_of(&logs, events::ProductionVerdictFailed::SELECTOR).is_empty(),
        "a short epoch 0 forfeits its verdicts like any other partial epoch"
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
        1,
        &[
            (members[0], token * U256::from(49)),
            (members[1], token * U256::from(25)),
            (members[2], token * U256::from(25)),
            (members[3], token),
        ],
    );
    seed_epoch_production(&mut harness.sdk, 1, &[151, 24, 25, 0], 200);

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

    let logs = harness.sdk.take_logs();
    assert!(
        logs_of(&logs, events::ProductionVerdictFailed::SELECTOR).is_empty(),
        "a complete epoch must still produce no verdict while the tier is off"
    );
    assert_eq!(
        production_record(&harness.sdk, members[1]).0,
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
    // Five Active validators, four seated: the exclusion guard now refuses to take the
    // population below MIN_COMMITTEE_LENGTH, so a fixture at the floor could never
    // exclude anyone. The fifth is Active and unseated; the committee below is explicit.
    let (mut harness, members) = liveness_harness(&[token * U256::from(50); 5], 2);
    set_min_verdict_due_blocks(&mut harness, 10);
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
    // due = 98 / 50 / 50 / 2 blocks out of 200 recorded, floor 10.
    seed_epoch_production(&mut harness.sdk, 1, &[151, 24, 25, 0], 200);
    harness.sdk.take_logs();

    assert_eq!(close_epoch_via_record(&mut harness, 1), ExitCode::Ok);

    let logs = harness.sdk.take_logs();
    let failed = logs_of(&logs, events::ProductionVerdictFailed::SELECTOR);
    assert_eq!(failed.len(), 1, "exactly one member is under half its due");
    assert_eq!(&failed[0].1[2].0[12..], members[1].as_slice());
    assert_eq!(
        decode_output::<(u32, U256)>(&failed[0].0),
        (24, U256::from(50))
    );
    assert_eq!(production_record(&harness.sdk, members[1]).0, 2);
    assert_eq!(
        production_record(&harness.sdk, members[2]).0,
        0,
        "producing exactly half the due share passes"
    );
    assert_eq!(
        production_record(&harness.sdk, members[3]).0,
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
    seed_epoch_production(&mut harness.sdk, 1, &[0, 0], interval);
    harness.sdk.take_logs();

    let boundary = u64::from(interval) * 3;
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
    assert_eq!(production_record(&harness.sdk, heavy).0, 2);
    assert_eq!(
        production_record(&harness.sdk, light).0,
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
    // Epoch 0 stays: the sentinel collision this guards is an epoch-0 property.
    // Its complete length is one block short of the interval, and the credits
    // sum to it — the taint arm would otherwise swallow the whole test.
    let produced = [50, 50, 50, 49, 0, 0, 0];

    seed_epoch_production(&mut harness.sdk, 0, &produced, 199);
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
            record.0, 1,
            "the failure bit is written on the guarded path"
        );
        assert_eq!(record.2, 0);
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
    for epoch in 1..4 {
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
        1,
        &[29, 29, 29, 29, 28, 28, 28, 0, 0, 0],
        200,
    );
    assert_eq!(close_epoch_via_record(&mut harness, 1), ExitCode::Ok);
    assert_eq!(
        pending_exclusion_set(&harness.sdk),
        vec![members[7], members[8]],
        "three failers, two stamps: the per-close cap, ordered by address"
    );

    seed_epoch_production(
        &mut harness.sdk,
        2,
        &[34, 33, 33, 33, 33, 34, 0, 0, 0, 0],
        200,
    );
    assert_eq!(close_epoch_via_record(&mut harness, 2), ExitCode::Ok);
    assert_eq!(
        pending_exclusion_set(&harness.sdk),
        vec![members[7], members[8], members[6]],
        "four failers, one stamp: `f` concurrent exclusions is the ceiling"
    );
    assert_eq!(production_record(&harness.sdk, members[9]).1, 0);

    seed_epoch_production(
        &mut harness.sdk,
        3,
        &[34, 33, 33, 33, 33, 34, 0, 0, 0, 0],
        200,
    );
    assert_eq!(close_epoch_via_record(&mut harness, 3), ExitCode::Ok);
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
    // Five Active validators, four seated — see the note in
    // `verdicts_come_from_the_frozen_weights_and_are_never_divided`.
    let (mut harness, members) = liveness_harness(&[token * U256::from(50); 5], 2);
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
        production_record(&harness.sdk, members[1]).0,
        0,
        "nor is a failure recorded against the member that would have failed"
    );
}

struct CloseCallState {
    /// What the reserve holds AND has approved, as one number: the close asks
    /// for both and an operator who funds without approving is a separate case
    /// with its own test.
    balance: U256,
    pulled: Vec<U256>,
    /// A reserve whose token answers nothing at all — the "could not read it"
    /// arm the close is required to score as zero rather than as an error.
    unreadable: bool,
}

fn token_reply(input: &[u8], state: &Rc<RefCell<CloseCallState>>) -> SyscallResult<Bytes> {
    let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
    let mut state = state.borrow_mut();
    if state.unreadable {
        return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic);
    }
    if selector == SIG_ERC20_BALANCE_OF || selector == SIG_ERC20_ALLOWANCE {
        return SyscallResult::new(encode_mock_return(&state.balance), 0, 0, ExitCode::Ok);
    }
    if selector != SIG_ERC20_TRANSFER_FROM {
        return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic);
    }
    let (_, _, amount) =
        SolidityABI::<(Address, Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0).unwrap();
    if amount > state.balance {
        return SyscallResult::new(Bytes::new(), 0, 0, ExitCode::Panic);
    }
    state.balance -= amount;
    state.pulled.push(amount);
    SyscallResult::new(encode_mock_return(&true), 0, 0, ExitCode::Ok)
}

/// Mocks the BLEND token the close reads and the claims pull from.
///
/// There is no self-call left to emulate. The close used to forward its payment
/// into a fuel-capped re-entry, and this helper snapshotted storage around it to
/// stand in for a journal checkpoint the test host does not have; the payment is
/// gone and the close's only outward call is a read.
fn install_close_call_handler(harness: &Harness, state: Rc<RefCell<CloseCallState>>) {
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            if let Some(reply) = mock_precompile_reply(address, input) {
                return reply;
            }
            token_reply(input, &state)
        });
}

// The close accrues and then pays inside one call, so the order of the two is
// load-bearing. A leg that ran first would find no scalar for the epoch just
// closed, take the deferral arm meant for an epoch whose close is still coming,
// and fail on every boundary from then on.
// Closes `epoch` from a block inside `at_epoch`, i.e. after a run of blocks that
// were produced but never recorded. That run is what consumes the weight ring's
// margin; parking does not, because a parked block never reaches the close.
fn close_epoch_late(harness: &mut Harness, epoch: u64, at_epoch: u64) -> ExitCode {
    production_liveness_storage()
        .last_processed_block_accessor()
        .set_checked(
            &mut harness.sdk,
            1_000 + (epoch + 1) * DEFAULT_EPOCH_BLOCK_INTERVAL - 1,
        )
        .unwrap();
    record_production(harness, 1_000 + at_epoch * DEFAULT_EPOCH_BLOCK_INTERVAL, 0)
}

// This replaces a test that asserted nothing. The old one parked a run of blocks
// and expected the ring to have turned; parking happens BELOW the close, so a
// parked run costs no ring revolutions whatever. The margin is consumed by
// blocks that were produced and never recorded — the close then runs late and
// has to find its own epoch's weights still in the frame.
#[test]
fn a_late_close_still_reads_its_own_epochs_weights() {
    let pot = U256::from(400);
    let (mut harness, members) = liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE; 4], 4);
    equal_weight_committee(&mut harness.sdk, 0, &members);
    let config = chain_config_storage();
    config
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, pot)
        .unwrap();
    config
        .blend_reserve_accessor()
        .set_checked(&mut harness.sdk, Address::with_last_byte(0xc0))
        .unwrap();
    production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(0)
        .set_checked(&mut harness.sdk, DEFAULT_EPOCH_BLOCK_INTERVAL as u32 / 2)
        .unwrap();

    let state = Rc::new(RefCell::new(CloseCallState {
        balance: pot,
        pulled: Vec::new(),
        unreadable: false,
    }));
    install_close_call_handler(&harness, state.clone());

    // Twelve epochs of unrecorded blocks — inside the bound, since nothing
    // committed in between has claimed frame 0.
    assert_eq!(close_epoch_late(&mut harness, 0, 12), ExitCode::Ok);

    for member in &members {
        assert!(
            !staking_storage()
                .validator_snapshots_accessor()
                .entry(*member)
                .entry(0)
                .total_blend_rewards_accessor()
                .get_checked(&harness.sdk)
                .unwrap()
                .is_zero(),
            "the late close paid from epoch 0's own frozen weights"
        );
    }
}

// And once a later epoch has actually claimed the frame, the close forfeits and
// announces it. Not a revert — the close is a pre-execution system call, so a
// propagated error is a chain halt nothing can repair. Not a silent zero either,
// which is the whole point of the event.
#[test]
fn a_close_past_the_ring_forfeits_the_epoch_and_says_so() {
    let pot = U256::from(400);
    let (mut harness, members) = liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE; 4], 4);
    equal_weight_committee(&mut harness.sdk, 0, &members);
    // The frame is only lost when a later epoch writes it — the ring does not
    // age, it is overwritten.
    equal_weight_committee(&mut harness.sdk, WEIGHT_RING_EPOCHS, &members);
    let config = chain_config_storage();
    config
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, pot)
        .unwrap();
    config
        .blend_reserve_accessor()
        .set_checked(&mut harness.sdk, Address::with_last_byte(0xc0))
        .unwrap();
    production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(0)
        .set_checked(&mut harness.sdk, DEFAULT_EPOCH_BLOCK_INTERVAL as u32 / 2)
        .unwrap();

    let state = Rc::new(RefCell::new(CloseCallState {
        balance: pot,
        pulled: Vec::new(),
        unreadable: false,
    }));
    install_close_call_handler(&harness, state.clone());
    harness.sdk.take_logs();

    assert_eq!(
        close_epoch_late(&mut harness, 0, WEIGHT_RING_EPOCHS + 1),
        ExitCode::Ok,
        "the chain keeps running"
    );

    let logs = harness.sdk.take_logs();
    let (data, _) = find_log(
        &logs,
        events::EpochWeightsUnavailable::SELECTOR,
        "EpochWeightsUnavailable",
    );
    assert_eq!(decode_output::<u32>(data), members.len() as u32);
    for member in &members {
        assert!(
            staking_storage()
                .validator_snapshots_accessor()
                .entry(*member)
                .entry(0)
                .total_blend_rewards_accessor()
                .get_checked(&harness.sdk)
                .unwrap()
                .is_zero(),
            "forfeited, not paid from the successor's weights"
        );
    }
}

// The close prices the epoch and moves nothing. The pot used to be pulled onto
// this contract here, and the delta is what every assertion below is about: the
// credits appear, the reserve is READ, and not one token leaves it until
// somebody claims.
#[test]
fn the_close_accrues_the_epoch_without_moving_any_money() {
    let pot = U256::from(400);
    let (mut harness, members) = liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE; 4], 4);
    equal_weight_committee(&mut harness.sdk, 0, &members);
    let config = chain_config_storage();
    config
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, pot)
        .unwrap();
    config
        .blend_reserve_accessor()
        .set_checked(&mut harness.sdk, Address::with_last_byte(0xc0))
        .unwrap();
    // Seeded raw, so nothing has accrued for epoch 0 when its close begins. Short
    // of the interval on purpose: a partial epoch draws no verdicts, which keeps
    // this about the stipend.
    production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(0)
        .set_checked(&mut harness.sdk, DEFAULT_EPOCH_BLOCK_INTERVAL as u32 / 2)
        .unwrap();

    let state = Rc::new(RefCell::new(CloseCallState {
        balance: pot,
        pulled: Vec::new(),
        unreadable: false,
    }));
    install_close_call_handler(&harness, state.clone());

    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);

    assert!(
        state.borrow().pulled.is_empty(),
        "the close must not pull the pot; the reserve pays each claim directly"
    );
    assert_eq!(
        state.borrow().balance,
        pot,
        "the reserve is untouched by the close"
    );
    for member in &members {
        assert_eq!(epoch_reward(&harness.sdk, *member, 0), U256::from(100));
    }
}

// A reserve the close cannot read must cost the epoch its pot and NOTHING else.
// The close is a pre-execution system call, so an error propagated out of that
// read is a block-execution failure on every node with no transaction able to
// repair it; the read is therefore required to score a failure as zero. This is
// what the fuel-capped self-call used to buy, bought instead by never raising.
//
// Both of the close's other legs run in the same call and must be untouched.
#[test]
fn an_unreadable_reserve_zeroes_the_epoch_without_failing_the_close() {
    let token = DEFAULT_MIN_VALIDATOR_STAKE;
    // Five Active validators, four seated — see the note in
    // `verdicts_come_from_the_frozen_weights_and_are_never_divided`.
    let (mut harness, members) = liveness_harness(&[token * U256::from(2); 5], 2);
    set_min_verdict_due_blocks(&mut harness, 10);
    equal_weight_committee(&mut harness.sdk, 0, &members[..4]);
    equal_weight_committee(&mut harness.sdk, 1, &members[..4]);

    let config = chain_config_storage();
    config
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::from(400))
        .unwrap();
    config
        .blend_reserve_accessor()
        .set_checked(&mut harness.sdk, Address::with_last_byte(0xc0))
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

    // Leg 2 has one member under half its due. Epoch 1's counters are seeded
    // WITHOUT accruing: the close under test has to be the first accrual for it,
    // or the credits it is asserting to be zero were written by the seed against
    // a reserve that could still be read.
    seed_epoch_production(&mut harness.sdk, 0, &[50, 50, 50, 50], 200);
    seed_epoch_counters(&mut harness.sdk, 1, &[67, 67, 0, 66], 200);

    let state = Rc::new(RefCell::new(CloseCallState {
        balance: U256::from(400),
        pulled: Vec::new(),
        unreadable: true,
    }));
    install_close_call_handler(&harness, state.clone());
    harness.sdk.take_logs();

    assert_eq!(
        close_epoch_via_record(&mut harness, 1),
        ExitCode::Ok,
        "an unreadable reserve must not take the per-block system call down with it"
    );

    let logs = harness.sdk.take_logs();
    let (data, topics) = find_log(
        &logs,
        events::EpochBlendRewardsCommitted::SELECTOR,
        "EpochBlendRewardsCommitted",
    );
    assert_eq!(
        SolidityABI::<u64>::decode(&topics[1].as_slice(), 0).unwrap(),
        1
    );
    assert_eq!(
        decode_output::<U256>(data),
        U256::ZERO,
        "the epoch closed owing nothing, and says so"
    );

    assert_eq!(
        production_record(&harness.sdk, releasing).1,
        0,
        "leg 1's release survives the unreadable reserve"
    );
    assert!(staking::selection_visible_at(&harness.sdk, releasing, 3).unwrap());
    assert_eq!(
        pending_exclusion_set(&harness.sdk),
        vec![members[2]],
        "leg 2's stamp survives it too"
    );
    let stamped = production_record(&harness.sdk, members[2]);
    assert_eq!((stamped.0, stamped.1, stamped.2), (2, 3, 1));

    // Zero for every seat, not just the first: a forfeit that credited even one
    // member would be a partial payment, which is the policy that was rejected.
    for member in &members[..4] {
        assert_eq!(
            epoch_reward(&harness.sdk, *member, 1),
            U256::ZERO,
            "an epoch the reserve could not cover pays nobody"
        );
    }
    assert!(state.borrow().pulled.is_empty());
}

// The genesis gap, pinned as ACCEPTED behaviour rather than left to be
// discovered. `initialize` requires only a non-zero reserve ADDRESS, not an
// approval, so a fresh chain runs its first epochs against a treasury that has
// funded the reserve but not yet approved this contract for it. Those epochs
// close at zero and stay there; the approval only reaches epochs that close
// after it.
//
// The reserve here is rich and unapproved, which is also the exact shape of an
// operator revoking approval later, or of a `setBlendReserve` rotation to an
// address nobody has approved yet.
#[test]
fn epochs_that_close_before_the_treasury_approves_burn_for_good() {
    let reserve = Address::with_last_byte(0xc0);
    let pot = U256::from(400);
    let (mut harness, members) = liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE; 4], 4);
    equal_weight_committee(&mut harness.sdk, 0, &members);
    equal_weight_committee(&mut harness.sdk, 1, &members);
    let config = chain_config_storage();
    config
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, pot)
        .unwrap();
    config
        .blend_reserve_accessor()
        .set_checked(&mut harness.sdk, reserve)
        .unwrap();
    for epoch in [0u64, 1] {
        production_liveness_storage()
            .blocks_in_epoch_accessor()
            .entry(epoch)
            .set_checked(&mut harness.sdk, DEFAULT_EPOCH_BLOCK_INTERVAL as u32 / 2)
            .unwrap();
    }

    // Funded a hundred times over, approved for nothing.
    let funding = install_stipend_token(&harness.sdk, reserve, pot * U256::from(100), U256::ZERO);

    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);
    for member in &members {
        assert_eq!(
            epoch_reward(&harness.sdk, *member, 0),
            U256::ZERO,
            "a balance nobody approved cannot cover the epoch"
        );
    }

    // The treasury approves. Epoch 1 closes funded; epoch 0 does not come back.
    funding.borrow_mut().allowance = pot * U256::from(100);
    assert_eq!(close_epoch_via_record(&mut harness, 1), ExitCode::Ok);
    for member in &members {
        assert_eq!(
            epoch_reward(&harness.sdk, *member, 1),
            U256::from(100),
            "the first epoch to close after the approval is paid in full"
        );
        assert_eq!(
            epoch_reward(&harness.sdk, *member, 0),
            U256::ZERO,
            "the epochs that ran before the approval are burnt, not deferred"
        );
    }
    assert!(
        funding.borrow().pulls.is_empty(),
        "and neither close moved a token: the reserve is only READ here"
    );
}

// The half of `min(balance, allowance)` the other tests do not exercise: a
// generous approval laid over a reserve that does not hold the money. Every
// other fixture in this file has `balance >= allowance`, so a contract that
// dropped the balance term and read the allowance alone would pass all of them
// and accrue a full epoch against a reserve that cannot pay a penny of it.
#[test]
fn an_approval_over_an_empty_reserve_covers_nothing() {
    let reserve = Address::with_last_byte(0xc0);
    let pot = U256::from(400);
    let (mut harness, members) = liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE; 4], 4);
    equal_weight_committee(&mut harness.sdk, 0, &members);
    equal_weight_committee(&mut harness.sdk, 1, &members);
    let config = chain_config_storage();
    config
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, pot)
        .unwrap();
    config
        .blend_reserve_accessor()
        .set_checked(&mut harness.sdk, reserve)
        .unwrap();
    for epoch in [0u64, 1] {
        production_liveness_storage()
            .blocks_in_epoch_accessor()
            .entry(epoch)
            .set_checked(&mut harness.sdk, DEFAULT_EPOCH_BLOCK_INTERVAL as u32 / 2)
            .unwrap();
    }

    // Approved for a hundred epochs, holding a quarter of one.
    let funding = install_stipend_token(
        &harness.sdk,
        reserve,
        pot / U256::from(4),
        pot * U256::from(100),
    );

    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);
    for member in &members {
        assert_eq!(
            epoch_reward(&harness.sdk, *member, 0),
            U256::ZERO,
            "an approval the reserve cannot back covers nothing"
        );
    }

    // The treasury funds it; the approval was never the missing half.
    funding.borrow_mut().balance = pot * U256::from(100);
    assert_eq!(close_epoch_via_record(&mut harness, 1), ExitCode::Ok);
    for member in &members {
        assert_eq!(epoch_reward(&harness.sdk, *member, 1), U256::from(100));
        assert_eq!(
            epoch_reward(&harness.sdk, *member, 0),
            U256::ZERO,
            "and the epoch that closed while it was empty stays dead"
        );
    }
}

// The close must ask about the RESERVE, not about some other party. Reading its
// own balance would be the dangerous mistake — this contract holds every
// delegator's deposit, so its balance is large and says nothing whatever about
// stipend coverage. The mock answers zero for any holder but the configured one,
// which is what turns that mistake into a forfeit here.
#[test]
fn the_close_asks_the_reserve_about_itself_and_not_about_the_contract() {
    let reserve = Address::with_last_byte(0xc0);
    let pot = U256::from(400);
    let (mut harness, members) = liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE; 4], 4);
    equal_weight_committee(&mut harness.sdk, 0, &members);
    let config = chain_config_storage();
    config
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, pot)
        .unwrap();
    config
        .blend_reserve_accessor()
        .set_checked(&mut harness.sdk, reserve)
        .unwrap();
    production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(0)
        .set_checked(&mut harness.sdk, DEFAULT_EPOCH_BLOCK_INTERVAL as u32 / 2)
        .unwrap();
    install_stipend_token(
        &harness.sdk,
        reserve,
        pot * U256::from(10),
        pot * U256::from(10),
    );

    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);
    for member in &members {
        assert_eq!(
            epoch_reward(&harness.sdk, *member, 0),
            U256::from(100),
            "the close read the configured reserve, so the epoch is funded"
        );
    }
}

// The two read paths duplicate the two consume paths, walk for walk, so they can
// drift apart in silence. `getDelegatorFee` must report the REWARD alone and
// `getDelegatorPrincipal` the deposit alone, with a matured withdrawal sitting
// in the queue — which is exactly the state in which a re-merged view would
// report their sum.
#[test]
fn the_delegator_views_report_the_reward_and_the_deposit_apart() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let delegator = Address::with_last_byte(0x02);
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
    staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        // The delegation matures at `WARMUP_DELAY` and the seat it funds is
        // selected `MAX_COMMITTEE_LOOKAHEAD_EPOCHS` ahead, so this is the first
        // epoch whose reward that delegation divides.
        .entry(WARMUP_DELAY + MAX_COMMITTEE_LOOKAHEAD_EPOCHS)
        .total_blend_rewards_accessor()
        .set_checked(
            &mut harness.sdk,
            math::narrow_reward(reward * U256::from(2)).expect("reward fits uint96"),
        )
        .unwrap();

    let maturity_epoch = WARMUP_DELAY + 1 + DEFAULT_UNDELEGATE_PERIOD;
    harness.set_block_number(activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * maturity_epoch);
    harness.set_caller(delegator);
    let command = ValidatorDelegatorCommand {
        validator,
        delegator,
    };

    // The delegator holds half the stake, so half of the epoch's blend.
    let (_, output) = harness.call(encode_call(SIG_GET_DELEGATOR_FEE, &command));
    assert_eq!(
        decode_output::<U256>(&output),
        reward,
        "a reward view must not fold in the matured deposit"
    );
    let (_, output) = harness.call(encode_call(SIG_GET_DELEGATOR_PRINCIPAL, &command));
    assert_eq!(decode_output::<U256>(&output), stake);

    // The views agree with what the claims actually pay: same numbers, and the
    // two together are what the merged view used to return.
    assert_eq!(
        delegator_claim_amounts(&mut harness, validator, delegator),
        (reward, stake)
    );
}

/// Runs both claims against a recording token and returns `(reward, principal)`.
fn delegator_claim_amounts(
    harness: &mut Harness,
    validator: Address,
    delegator: Address,
) -> (U256, U256) {
    let pulled = Rc::new(RefCell::new(U256::ZERO));
    let transferred = Rc::new(RefCell::new(U256::ZERO));
    let (a, b) = (pulled.clone(), transferred.clone());
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            if let Some(reply) = mock_precompile_reply(address, input) {
                return reply;
            }
            let selector = u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap());
            if selector == SIG_ERC20_TRANSFER_FROM {
                let (_, _, amount) =
                    SolidityABI::<(Address, Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0)
                        .unwrap();
                *a.borrow_mut() += amount;
            } else if selector == SIG_ERC20_TRANSFER {
                let (_, amount) =
                    SolidityABI::<(Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0).unwrap();
                *b.borrow_mut() += amount;
            }
            SyscallResult::new(encode_mock_return(&true), 0, 0, ExitCode::Ok)
        });
    harness.set_caller(delegator);
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
        harness
            .call(encode_call(
                SIG_WITHDRAW_DELEGATOR_PRINCIPAL,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );
    let result = (*pulled.borrow(), *transferred.borrow());
    result
}

// Redelegation now re-stakes the REWARD and nothing else. It used to be handed a
// single number with the matured withdrawal folded into it, so a delegator who
// had asked to exit and then claimed a reward had their exit silently re-staked.
#[test]
fn redelegation_takes_the_reward_and_leaves_the_withdrawal_queue_alone() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let delegator = Address::with_last_byte(0x02);
    let reserve = Address::with_last_byte(0xc0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let reward = DEFAULT_MIN_STAKING_AMOUNT * U256::from(4);
    let activation_block = 1_000;
    let mut harness = Harness::new(activation_block);
    harness.set_caller(owner);
    let mut command = harness.initialize_command(owner, vec![validator], vec![stake], 0);
    command.blend_reserve = reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
    // Twice the validator's own stake, so the delegator's share of the blend is
    // two thirds — comfortably over the staking minimum, which is what makes the
    // redelegate branch reach the delegation instead of falling through as dust.
    staking::delegate_to(
        &mut harness.sdk,
        delegator,
        validator,
        stake * U256::from(2),
        false,
    )
    .unwrap();
    harness.set_block_number(activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * WARMUP_DELAY);
    // The whole delegation is on its way out.
    staking::undelegate_from(
        &mut harness.sdk,
        delegator,
        validator,
        stake * U256::from(2),
    )
    .unwrap();
    staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        // The delegation matures at `WARMUP_DELAY` and the seat it funds is
        // selected `MAX_COMMITTEE_LOOKAHEAD_EPOCHS` ahead, so this is the first
        // epoch whose reward that delegation divides.
        .entry(WARMUP_DELAY + MAX_COMMITTEE_LOOKAHEAD_EPOCHS)
        .total_blend_rewards_accessor()
        .set_checked(
            &mut harness.sdk,
            math::narrow_reward(reward).expect("reward fits uint96"),
        )
        .unwrap();

    let funding = install_stipend_token(&harness.sdk, reserve, reward, reward);
    harness.set_caller(delegator);
    let maturity_epoch = WARMUP_DELAY + 1 + DEFAULT_UNDELEGATE_PERIOD;
    harness.set_block_number(activation_block + DEFAULT_EPOCH_BLOCK_INTERVAL * maturity_epoch);

    let delegation = staking_storage()
        .validator_delegations_accessor()
        .entry(validator)
        .entry(delegator);
    let matured_before = delegation
        .pending_undelegated_accessor()
        .get_checked(&harness.sdk)
        .unwrap();
    assert_eq!(matured_before, stake * U256::from(2));
    // The view says the same thing, and it is the only way to see it.
    let (_, output) = harness.call(encode_call(
        SIG_GET_DELEGATOR_PRINCIPAL,
        &ValidatorDelegatorCommand {
            validator,
            delegator,
        },
    ));
    assert_eq!(decode_output::<U256>(&output), matured_before);

    assert_eq!(
        harness
            .call(encode_call(
                SIG_REDELEGATE_DELEGATOR_FEE,
                &AddressCommand { value: validator },
            ))
            .0,
        ExitCode::Ok
    );

    // Two thirds of the blend, split the way `available_for_redelegate` splits
    // it: whole units of `BALANCE_COMPACT_PRECISION` become stake, the remainder
    // is paid out as dust.
    let claimable = reward * U256::from(2) / U256::from(3);
    let re_staked = claimable / BALANCE_COMPACT_PRECISION * BALANCE_COMPACT_PRECISION;
    let dust = claimable - re_staked;
    assert!(!dust.is_zero(), "the dust leg must actually be exercised");
    assert_eq!(
        funding.borrow().pulls,
        vec![(GENESIS_STAKING, re_staked), (delegator, dust)],
        "both legs come off the RESERVE: the stake onto this contract, where \
         every other delegation is held, and the dust straight to the delegator"
    );
    assert!(
        funding.borrow().transfers.is_empty(),
        "and nothing came out of this contract's own balance"
    );
    assert_eq!(
        staking::delegated_amount_at(
            &harness.sdk,
            validator,
            delegator,
            maturity_epoch + WARMUP_DELAY
        )
        .unwrap(),
        math::compact_balance(re_staked).unwrap(),
        "the reward is re-staked"
    );
    assert_eq!(
        delegation
            .pending_undelegated_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        matured_before,
        "the matured withdrawal is untouched — redelegate no longer swallows it"
    );
    assert_eq!(
        delegation
            .undelegate_gap_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        0,
        "and the withdrawal cursor has not moved"
    );
}

// And the forfeit is FINAL. A reserve funded after the fact does not revive the
// epoch it missed: nothing revisits a closed epoch, so the credits stay zero
// however rich the reserve later becomes.
#[test]
fn funding_the_reserve_after_the_close_does_not_revive_the_epoch() {
    let token = DEFAULT_MIN_VALIDATOR_STAKE;
    let (mut harness, members) = liveness_harness(&[token * U256::from(2); 4], 4);
    equal_weight_committee(&mut harness.sdk, 0, &members);
    equal_weight_committee(&mut harness.sdk, 1, &members);
    let config = chain_config_storage();
    config
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, U256::from(400))
        .unwrap();
    config
        .blend_reserve_accessor()
        .set_checked(&mut harness.sdk, Address::with_last_byte(0xc0))
        .unwrap();
    production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(0)
        .set_checked(&mut harness.sdk, DEFAULT_EPOCH_BLOCK_INTERVAL as u32 / 2)
        .unwrap();

    // One unit short of the pot. Short is short: the policy is a cliff, not a
    // pro-rata split, so 399 of 400 pays exactly as much as 0 of 400.
    let state = Rc::new(RefCell::new(CloseCallState {
        balance: U256::from(399),
        pulled: Vec::new(),
        unreadable: false,
    }));
    install_close_call_handler(&harness, state.clone());

    assert_eq!(close_epoch_via_record(&mut harness, 0), ExitCode::Ok);
    for member in &members {
        assert_eq!(
            epoch_reward(&harness.sdk, *member, 0),
            U256::ZERO,
            "one unit short forfeits the whole epoch"
        );
    }

    // The treasury tops the reserve up. Epoch 1 is funded; epoch 0 stays dead.
    state.borrow_mut().balance = U256::from(10_000);
    production_liveness_storage()
        .blocks_in_epoch_accessor()
        .entry(1)
        .set_checked(&mut harness.sdk, DEFAULT_EPOCH_BLOCK_INTERVAL as u32 / 2)
        .unwrap();
    assert_eq!(close_epoch_via_record(&mut harness, 1), ExitCode::Ok);

    for member in &members {
        assert_eq!(
            epoch_reward(&harness.sdk, *member, 1),
            U256::from(100),
            "the epoch closed after the top-up is funded"
        );
        assert_eq!(
            epoch_reward(&harness.sdk, *member, 0),
            U256::ZERO,
            "a later top-up does not revive an epoch that already closed at zero"
        );
    }
}

// --- Payout policy: tombstoned commission, the E-2 split, and rate vintage ---

/// Every `transferFrom` the reserve saw during `body`, as `(recipient, amount)`.
fn reserve_pulls_during(
    harness: &mut Harness,
    body: impl FnOnce(&mut Harness),
) -> Vec<(Address, U256)> {
    let pulls = Rc::new(RefCell::new(Vec::<(Address, U256)>::new()));
    let recorded = pulls.clone();
    harness
        .sdk
        .set_call_handler(move |address, _value, input, _fuel_limit| {
            if let Some(reply) = mock_precompile_reply(address, input) {
                return reply;
            }
            if u32::from_be_bytes(input[..SIG_LEN_BYTES].try_into().unwrap())
                == SIG_ERC20_TRANSFER_FROM
            {
                let (_, to, amount) =
                    SolidityABI::<(Address, Address, U256)>::decode(&&input[SIG_LEN_BYTES..], 0)
                        .unwrap();
                recorded.borrow_mut().push((to, amount));
            }
            SyscallResult::new(encode_mock_return(&true), 0, 0, ExitCode::Ok)
        });
    body(harness);
    let taken = pulls.borrow().clone();
    taken
}

/// The epochs `validator` actually has a snapshot for, in order.
fn materialized_snapshot_epochs(sdk: &TestingContextImpl, validator: Address) -> Vec<u64> {
    let epochs = staking_storage()
        .validator_snapshot_epochs_accessor()
        .entry(validator);
    (0..epochs.len_checked(sdk).unwrap())
        .map(|index| epochs.at(index).get_checked(sdk).unwrap())
        .collect()
}

fn credit_epoch_reward(harness: &mut Harness, validator: Address, epoch: u64, reward: U256) {
    staking_storage()
        .validator_snapshots_accessor()
        .entry(validator)
        .entry(epoch)
        .total_blend_rewards_accessor()
        .set_checked(
            &mut harness.sdk,
            math::narrow_reward(reward).expect("reward fits uint96"),
        )
        .unwrap();
}

fn delegator_fee_of(harness: &mut Harness, validator: Address, delegator: Address) -> U256 {
    let (_, output) = harness.call(encode_call(
        SIG_GET_DELEGATOR_FEE,
        &ValidatorDelegatorCommand {
            validator,
            delegator,
        },
    ));
    decode_output::<U256>(&output)
}

fn validator_fee_of(harness: &mut Harness, validator: Address) -> U256 {
    let (_, output) = harness.call(encode_call(
        SIG_GET_VALIDATOR_FEE,
        &AddressCommand { value: validator },
    ));
    decode_output::<U256>(&output)
}

// A proven equivocator's commission is extinguished — refused at the claim and
// quoted as nothing by the view — while his delegators still collect their share
// of the very same epochs. The gate belongs on the owner leg alone: the seizure
// takes the owner's own bond and never touches a delegator queue.
//
// Run twice against one fixture, tombstone off then on, so the "nothing" is
// measured against the amount that was actually there to pay.
#[test]
fn a_tombstone_extinguishes_the_owner_commission_but_not_the_delegator_share() {
    let owner = Address::with_last_byte(0xa0);
    let delegator = Address::with_last_byte(0xb0);
    let validator = Address::with_last_byte(0x01);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let reward = DEFAULT_MIN_STAKING_AMOUNT * U256::from(10);

    for tombstoned in [false, true] {
        let mut harness = Harness::new(1_000);
        harness.set_caller(owner);
        assert_eq!(
            harness.initialize(owner, vec![validator], vec![stake], 1_000),
            ExitCode::Ok
        );
        staking::delegate_to(&mut harness.sdk, delegator, validator, stake, false).unwrap();
        // Booked at epoch 2, and the seat it funds is selected two epochs ahead.
        credit_epoch_reward(&mut harness, validator, 4, reward);
        consensus_storage()
            .tombstoned_accessor()
            .entry(validator)
            .set_checked(&mut harness.sdk, tombstoned)
            .unwrap();
        harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL * 5);

        let commission = reward / U256::from(10);
        let delegator_share = (reward - commission) / U256::from(2);
        harness.set_caller(delegator);

        if tombstoned {
            assert_eq!(
                validator_fee_of(&mut harness, validator),
                U256::ZERO,
                "the owner view must not quote a commission no call can collect"
            );
            assert_revert_selector(
                harness.call(encode_call(
                    SIG_CLAIM_VALIDATOR_FEE,
                    &AddressCommand { value: validator },
                )),
                ERR_VALIDATOR_TOMBSTONED,
            );
        } else {
            assert_eq!(validator_fee_of(&mut harness, validator), commission);
            let paid = reserve_pulls_during(&mut harness, |harness| {
                assert_eq!(
                    harness
                        .call(encode_call(
                            SIG_CLAIM_VALIDATOR_FEE,
                            &AddressCommand { value: validator },
                        ))
                        .0,
                    ExitCode::Ok
                );
            });
            assert_eq!(paid, vec![(validator, commission)]);
        }

        // The delegator leg is untouched either way.
        assert_eq!(
            delegator_fee_of(&mut harness, validator, delegator),
            delegator_share,
            "the delegator's share of the same epoch survives the owner's tombstone"
        );
        let paid = reserve_pulls_during(&mut harness, |harness| {
            harness.set_caller(delegator);
            assert_eq!(
                harness
                    .call(encode_call(
                        SIG_CLAIM_DELEGATOR_FEE,
                        &AddressCommand { value: validator },
                    ))
                    .0,
                ExitCode::Ok
            );
        });
        assert_eq!(paid, vec![(delegator, delegator_share)]);
    }
}

// The delegator split is the seat's own weight, redistributed. The seat for
// epoch 4 was weighed at epoch 2, so what divides epoch 4's credit is the epoch-2
// snapshot and nothing later.
//
// The fixture separates the three candidate vintages: a delegation matures at 2,
// another at 3, and epoch 4 adds nothing. Splitting at 4 or at 3 both give the
// early delegator a third; only splitting at 2 gives a half. The late delegator
// is the same boundary read from the other side — its stake matured at 3, so its
// first reward epoch is 5 and epoch 4 owes it nothing.
#[test]
fn the_delegator_split_reproduces_the_seat_weight_frozen_two_epochs_back() {
    let owner = Address::with_last_byte(0xa0);
    let early = Address::with_last_byte(0xb0);
    let late = Address::with_last_byte(0xb1);
    let validator = Address::with_last_byte(0x01);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let reward = DEFAULT_MIN_STAKING_AMOUNT * U256::from(12);

    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![stake], 0),
        ExitCode::Ok
    );
    staking::delegate_to(&mut harness.sdk, early, validator, stake, false).unwrap();
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL);
    staking::delegate_to(&mut harness.sdk, late, validator, stake, false).unwrap();
    credit_epoch_reward(&mut harness, validator, 4, reward);
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL * 5);

    let seat_weight = staking::validator_total_at(&harness.sdk, validator, 2).unwrap();
    assert_eq!(
        seat_weight,
        stake * U256::from(2),
        "the seat epoch 4 was selected on holds the owner and the early delegator only"
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 3).unwrap(),
        stake * U256::from(3),
        "and it is a different number one epoch later, so the two vintages are told apart"
    );

    harness.set_caller(early);
    assert_eq!(
        delegator_fee_of(&mut harness, validator, early),
        reward * stake / seat_weight,
        "the share is the delegator's weight in the frozen seat, not in a later total"
    );
    assert_eq!(
        delegator_fee_of(&mut harness, validator, late),
        U256::ZERO,
        "stake that matured after the selection earns nothing from that seat"
    );
}

// The other side of the same lag: a seat credited for an epoch whose SELECTION
// epoch predates the validator's first snapshot has no denominator to divide by,
// and the whole credit goes to the owner rather than into a delegator pool
// nobody can draw from.
//
// Reachable, and reached the ordinary way: a validator registered at epoch 5
// materializes its first snapshot at 6, so epoch 7 — whose selection epoch is 5
// — finds nothing at or before it. Whichever way this arm is decided it decides
// where a whole epoch's credit goes, and it had no test at all.
#[test]
fn a_seat_with_no_snapshot_at_its_selection_epoch_pays_its_whole_credit_to_the_owner() {
    let sponsor = Address::with_last_byte(0xa0);
    let seated = Address::with_last_byte(0x01);
    let latecomer = Address::with_last_byte(0x02);
    let reward = DEFAULT_MIN_STAKING_AMOUNT * U256::from(100);
    let mut harness = Harness::new(1_000);
    assert_eq!(
        harness.initialize(sponsor, vec![seated], vec![DEFAULT_MIN_VALIDATOR_STAKE], 0),
        ExitCode::Ok
    );

    // Registers during epoch 5, so `changed_at` — and the first snapshot — is 6.
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL * 5);
    harness.set_caller(latecomer);
    assert_eq!(
        harness
            .call(encode_args_call(
                SIG_REGISTER_VALIDATOR,
                &RegisterValidatorCommand {
                    validator: latecomer,
                    commission_rate: 500,
                    initial_stake: DEFAULT_MIN_VALIDATOR_STAKE,
                    bls_pubkey_uncompressed: Bytes::from(vec![
                        0x33;
                        BLS_PUBKEY_UNCOMPRESSED_LENGTH
                    ]),
                    bls_pop_uncompressed: Bytes::from(vec![0x44; BLS_POP_UNCOMPRESSED_LENGTH]),
                    peer_pubkey: B256::with_last_byte(2),
                },
            ))
            .0,
        ExitCode::Ok
    );
    assert_eq!(
        materialized_snapshot_epochs(&harness.sdk, latecomer),
        vec![6],
        "the earliest snapshot is above the selection epoch of the credit below"
    );

    // Epoch 7's seat was weighed at epoch 5, where this validator did not exist.
    credit_epoch_reward(&mut harness, latecomer, 7, reward);
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL * 8);
    assert_eq!(
        staking::validator_total_at(&harness.sdk, latecomer, 5).unwrap(),
        U256::ZERO,
        "there is no seat weight at the selection epoch to divide by"
    );

    assert_eq!(
        validator_fee_of(&mut harness, latecomer),
        reward,
        "with no denominator the credit is the owner's whole, not a commission \
         slice with the rest stranded in a pool nobody can draw"
    );
    assert_eq!(
        delegator_fee_of(&mut harness, latecomer, latecomer),
        U256::ZERO,
        "and the delegator walk pays nothing out of it"
    );
    let paid = reserve_pulls_during(&mut harness, |harness| {
        assert_eq!(
            harness
                .call(encode_call(
                    SIG_CLAIM_VALIDATOR_FEE,
                    &AddressCommand { value: latecomer },
                ))
                .0,
            ExitCode::Ok
        );
    });
    assert_eq!(
        paid,
        vec![(latecomer, reward)],
        "the claim pays what the view quoted"
    );
}

// A run of `KICK_LADDER_RESET_EPOCHS` epochs without a failure retires the
// ladder, and the threshold is a real boundary rather than a direction: the same
// fixture one epoch shorter leaves the count standing.
#[test]
fn a_clean_run_retires_the_kick_ladder_and_one_epoch_short_does_not() {
    // Literal 30 and 29, NOT `KICK_LADDER_RESET_EPOCHS` and one less: a fixture
    // written off the constant moves with it, and would stay green for any
    // threshold at all.
    for (run, expected) in [(30u64, 0u32), (29u64, 5u32)] {
        let (mut harness, members) =
            liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2); 4], 2);
        set_min_verdict_due_blocks(&mut harness, 10);
        // Failed at epoch 0 — `last_failed_epoch_p1` is epoch+1 — five episodes
        // deep. The judged epoch below is therefore `run` epochs after it.
        let record = production_liveness_storage()
            .validators_accessor()
            .entry(members[0]);
        record
            .last_failed_epoch_p1_accessor()
            .set_checked(&mut harness.sdk, 1)
            .unwrap();
        record
            .kick_count_accessor()
            .set_checked(&mut harness.sdk, 5)
            .unwrap();

        equal_weight_committee(&mut harness.sdk, run, &members);
        // Everyone clears the bar, so the close judges without stamping anyone.
        seed_epoch_production(&mut harness.sdk, run, &[50, 50, 50, 50], 200);
        assert_eq!(close_epoch_via_record(&mut harness, run), ExitCode::Ok);

        assert_eq!(
            production_record(&harness.sdk, members[0]).2,
            expected,
            "a {run}-epoch run since the last failure"
        );
        assert!(
            pending_exclusion_set(&harness.sdk).is_empty(),
            "nobody failed this epoch, so nothing was stamped"
        );
    }
}

// The rate that prices an epoch is `min(selection-epoch rate, epoch rate)`. A
// rise therefore has to clear both vintages before it reaches money — three
// epochs from the announcement — while a cut lands on the next epoch through the
// second term.
//
// Claimed epoch by epoch rather than in one window, so the assertion names which
// epoch paid what instead of a sum three vintages could reach by other routes.
#[test]
fn a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next() {
    let owner = Address::with_last_byte(0xa0);
    let delegator = Address::with_last_byte(0xb0);
    let validator = Address::with_last_byte(0x01);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let reward = DEFAULT_MIN_STAKING_AMOUNT * U256::from(100);

    // Epoch 3 carries a credit too, and it PRECEDES the epoch the change takes
    // effect in: without it nothing would tell "the second term is read at E"
    // from "read at E+1". The third pair cuts to a NONZERO rate — with only the
    // two cuts-to-zero, "min" is indistinguishable from "pay the owner nothing".
    for (initial, changed, expected) in [
        (0u16, COMMISSION_RATE_MAX, [0u16, 0, 0, COMMISSION_RATE_MAX]),
        (COMMISSION_RATE_MAX, 0u16, [COMMISSION_RATE_MAX, 0, 0, 0]),
        (
            COMMISSION_RATE_MAX,
            1_000u16,
            [COMMISSION_RATE_MAX, 1_000, 1_000, 1_000],
        ),
    ] {
        let mut harness = Harness::new(1_000);
        harness.set_caller(owner);
        assert_eq!(
            harness.initialize(owner, vec![validator], vec![stake], initial),
            ExitCode::Ok
        );
        staking::delegate_to(&mut harness.sdk, delegator, validator, stake, false).unwrap();
        for epoch in 3..7 {
            credit_epoch_reward(&mut harness, validator, epoch, reward);
        }

        // Announced during epoch 3, which schedules it for epoch 4. The genesis
        // validator is its own owner, so it is the only accepted caller and the
        // commission recipient.
        harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL * 3);
        harness.set_caller(validator);
        assert_eq!(
            harness
                .call(encode_call(
                    SIG_CHANGE_VALIDATOR_COMMISSION_RATE,
                    &AddressU16Command {
                        validator,
                        value: changed,
                    },
                ))
                .0,
            ExitCode::Ok
        );
        harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL * 7);

        for (index, rate) in expected.into_iter().enumerate() {
            let epoch = 3 + index as u64;
            let paid = reserve_pulls_during(&mut harness, |harness| {
                assert_eq!(
                    harness
                        .call(encode_call(
                            SIG_CLAIM_VALIDATOR_FEE_AT_EPOCH,
                            &ValidatorEpochCommand {
                                validator,
                                before_epoch: epoch + 1,
                            },
                        ))
                        .0,
                    ExitCode::Ok
                );
            });
            let owed = reward * U256::from(rate) / U256::from(BPS_DENOMINATOR);
            let expected_pulls = if owed.is_zero() {
                Vec::new()
            } else {
                vec![(validator, owed)]
            };
            assert_eq!(
                paid, expected_pulls,
                "epoch {epoch} priced at {rate} bps after {initial} -> {changed}"
            );
        }
    }
}

// Withdrawal is not the same act as leaving the seat. A delegator whose stake
// counted at the selection epoch earns that epoch even if it withdrew afterwards,
// and it earns it at the rate in force back then, not at one raised in the
// meantime. `undelegate_period` is orthogonal: it holds the principal, and holds
// it whether or not the reward has been paid.
#[test]
fn a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate() {
    let owner = Address::with_last_byte(0xa0);
    let left_at_two = Address::with_last_byte(0xb0);
    let left_at_three = Address::with_last_byte(0xb1);
    let validator = Address::with_last_byte(0x01);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let reward = DEFAULT_MIN_STAKING_AMOUNT * U256::from(60);

    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![stake], 500),
        ExitCode::Ok
    );
    staking::delegate_to(&mut harness.sdk, left_at_two, validator, stake, false).unwrap();
    staking::delegate_to(&mut harness.sdk, left_at_three, validator, stake, false).unwrap();
    let seat_weight = staking::validator_total_at(&harness.sdk, validator, 2).unwrap();
    assert_eq!(seat_weight, stake * U256::from(3));

    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL * 2);
    staking::undelegate_from(&mut harness.sdk, left_at_two, validator, stake).unwrap();
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL * 3);
    staking::undelegate_from(&mut harness.sdk, left_at_three, validator, stake).unwrap();
    harness.set_caller(validator);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CHANGE_VALIDATOR_COMMISSION_RATE,
                &AddressU16Command {
                    validator,
                    value: COMMISSION_RATE_MAX,
                },
            ))
            .0,
        ExitCode::Ok
    );

    credit_epoch_reward(&mut harness, validator, 4, reward);
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL * 5);

    let commission = reward * U256::from(500) / U256::from(BPS_DENOMINATOR);
    let share = (reward - commission) * stake / seat_weight;
    for delegator in [left_at_two, left_at_three] {
        harness.set_caller(delegator);
        assert_eq!(
            delegator_fee_of(&mut harness, validator, delegator),
            share,
            "a withdrawal after the selection epoch does not unmake the earning"
        );
        let (_, output) = harness.call(encode_call(
            SIG_GET_DELEGATOR_PRINCIPAL,
            &ValidatorDelegatorCommand {
                validator,
                delegator,
            },
        ));
        assert_eq!(
            decode_output::<U256>(&output),
            U256::ZERO,
            "the undelegation period still holds the deposit; only the tokens wait"
        );
    }

    // The claims agree with the views, stop at the requested epoch, and do not
    // pay twice. `left_at_three`'s queue closes its first entry at reward epoch
    // 6, past the epoch being claimed through, so its cursor is what proves the
    // walk stopped where it was told rather than where the queue ended.
    for delegator in [left_at_two, left_at_three] {
        let paid = reserve_pulls_during(&mut harness, |harness| {
            harness.set_caller(delegator);
            assert_eq!(
                harness
                    .call(encode_call(
                        SIG_CLAIM_DELEGATOR_FEE,
                        &AddressCommand { value: validator },
                    ))
                    .0,
                ExitCode::Ok
            );
        });
        assert_eq!(paid, vec![(delegator, share)]);
        assert_eq!(
            staking_storage()
                .validator_delegations_accessor()
                .entry(validator)
                .entry(delegator)
                .claimed_through_epoch_accessor()
                .get_checked(&harness.sdk)
                .unwrap(),
            5,
            "the cursor stops at the claimed epoch, not at the queue entry's end"
        );

        let again = reserve_pulls_during(&mut harness, |harness| {
            harness.set_caller(delegator);
            assert_eq!(
                harness
                    .call(encode_call(
                        SIG_CLAIM_DELEGATOR_FEE,
                        &AddressCommand { value: validator },
                    ))
                    .0,
                ExitCode::Ok
            );
        });
        assert!(
            again.is_empty(),
            "a second claim over the same window pays nothing"
        );
    }
}

// The load-bearing property under the rate rule: lazy materialization copies
// `commission_rate` from the nearest earlier snapshot, so the rate history is
// recoverable at any depth however late the snapshot is created. Without it,
// reading the selection epoch would read a zero for every epoch nothing had
// happened in.
//
// Epoch 5 is deliberately never materialized while it is current: the rise
// announced during it schedules epoch 6, epoch 7's reward is priced off epoch 5,
// and only then is epoch 5 brought into being.
#[test]
fn a_snapshot_materialized_after_its_epoch_passed_still_carries_the_old_rate() {
    let owner = Address::with_last_byte(0xa0);
    let validator = Address::with_last_byte(0x01);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let reward = DEFAULT_MIN_STAKING_AMOUNT * U256::from(100);
    let old_rate = 500u16;

    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    assert_eq!(
        harness.initialize(owner, vec![validator], vec![stake], old_rate),
        ExitCode::Ok
    );
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL * 5);
    harness.set_caller(validator);
    assert_eq!(
        harness
            .call(encode_call(
                SIG_CHANGE_VALIDATOR_COMMISSION_RATE,
                &AddressU16Command {
                    validator,
                    value: COMMISSION_RATE_MAX,
                },
            ))
            .0,
        ExitCode::Ok
    );
    let epochs = staking_storage()
        .validator_snapshot_epochs_accessor()
        .entry(validator);
    let materialized: Vec<u64> = (0..epochs.len_checked(&harness.sdk).unwrap())
        .map(|index| epochs.at(index).get_checked(&harness.sdk).unwrap())
        .collect();
    assert_eq!(
        materialized,
        vec![0, 6],
        "epoch 5 has no snapshot of its own when the rise is scheduled"
    );

    credit_epoch_reward(&mut harness, validator, 7, reward);
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL * 8);
    let owed = reward * U256::from(old_rate) / U256::from(BPS_DENOMINATOR);
    assert_eq!(
        validator_fee_of(&mut harness, validator),
        owed,
        "epoch 7 is priced off epoch 5, which still reads the pre-rise rate"
    );

    // Now materialize epoch 5, long after epoch 7 has gone by.
    let late = staking::touch_snapshot_at_or_before(&mut harness.sdk, validator, 5).unwrap();
    assert_eq!(
        late.commission_rate_accessor()
            .get_checked(&harness.sdk)
            .unwrap(),
        old_rate,
        "materialization copies the rate that was in force, not the current one"
    );
    assert_eq!(
        validator_fee_of(&mut harness, validator),
        owed,
        "and the payout does not move when the snapshot is finally written"
    );
}

// The same split, but with the credit written by the accrual path instead of by
// the fixture. Everything above seeds `total_blend_rewards` directly, which
// leaves the one thing the close does for itself untested: it materializes the
// snapshot of the epoch it is crediting. That snapshot is NOT the one the split
// reads, and here the two hold different totals, so a split that drifted onto
// the epoch it was credited at would divide by three instead of by two.
#[test]
fn the_accrual_path_credits_an_epoch_that_is_still_divided_two_epochs_back() {
    let owner = Address::with_last_byte(0xa0);
    let early = Address::with_last_byte(0xb0);
    // Books at epoch 3 purely so the epoch-3 total differs from both its
    // neighbours: without it, epochs 2 and 3 hold the same number and the
    // fixture would read the same whether the lag were one epoch or two.
    let mid = Address::with_last_byte(0xb2);
    let late = Address::with_last_byte(0xb1);
    let validator = Address::with_last_byte(0x01);
    let reserve = Address::with_last_byte(0xc0);
    let stake = DEFAULT_MIN_VALIDATOR_STAKE;
    let pot = DEFAULT_MIN_STAKING_AMOUNT * U256::from(30);

    let mut harness = Harness::new(1_000);
    harness.set_caller(owner);
    let mut command = harness.initialize_command(owner, vec![validator], vec![stake], 1_000);
    command.blend_reserve = reserve;
    assert_eq!(harness.initialize_with(command), ExitCode::Ok);
    chain_config_storage()
        .blend_stipend_per_epoch_accessor()
        .set_checked(&mut harness.sdk, pot)
        .unwrap();

    staking::delegate_to(&mut harness.sdk, early, validator, stake, false).unwrap();
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL);
    staking::delegate_to(&mut harness.sdk, mid, validator, stake, false).unwrap();
    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL * 3);
    staking::delegate_to(&mut harness.sdk, late, validator, stake, false).unwrap();

    // Nothing has booked stake at epoch 4, so no entrypoint has materialized its
    // snapshot. That is the state the close has to handle on its own, and a
    // fixture where some delegation already created epoch 4 would let the close
    // skip the materialization entirely and still look right.
    assert_eq!(
        materialized_snapshot_epochs(&harness.sdk, validator),
        vec![0, 2, 3, 5],
        "epoch 4 has no snapshot before the close runs"
    );

    commit_test_committee(&mut harness.sdk, 4, &[(validator, stake)]);
    install_stipend_token(
        &harness.sdk,
        reserve,
        pot * U256::from(10),
        pot * U256::from(10),
    );
    record_test_production(&mut harness.sdk, 4, DEFAULT_EPOCH_BLOCK_INTERVAL as u32);
    assert_eq!(epoch_reward(&harness.sdk, validator, 4), pot);
    assert_eq!(
        materialized_snapshot_epochs(&harness.sdk, validator),
        vec![0, 2, 3, 4, 5],
        "the close materialized the epoch it credited"
    );

    let seat_weight = staking::validator_total_at(&harness.sdk, validator, 2).unwrap();
    assert_eq!(seat_weight, stake * U256::from(2));
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 3).unwrap(),
        stake * U256::from(3),
        "epoch 3 is a third distinct total, so a one-epoch lag would read differently"
    );
    assert_eq!(
        staking::validator_total_at(&harness.sdk, validator, 4).unwrap(),
        stake * U256::from(3),
        "the snapshot the close created carries epoch 3's total forward"
    );

    harness.set_block_number(1_000 + DEFAULT_EPOCH_BLOCK_INTERVAL * 5);
    let pool = pot - pot / U256::from(10);
    harness.set_caller(early);
    assert_eq!(
        delegator_fee_of(&mut harness, validator, early),
        pool * stake / seat_weight
    );
    for later in [mid, late] {
        assert_eq!(
            delegator_fee_of(&mut harness, validator, later),
            U256::ZERO,
            "stake booked after the selection epoch earns nothing from that credit"
        );
    }
}

// The other half of the reset, and the half the clean-run fixture cannot reach:
// it lands on the member who fails in the very epoch the run completes. The
// ladder restarts at its first rung instead of resuming a years-old climb, so
// the exclusion this failure earns is one epoch, not six.
//
// Ten members, not four: `apply_production_exclusion` refuses a stamp that would
// take the eligible set below the committee floor, and a refused stamp leaves no
// ladder increment to measure.
#[test]
fn the_ladder_reset_also_lands_on_the_member_failing_that_same_epoch() {
    let (mut harness, members) =
        liveness_harness(&[DEFAULT_MIN_VALIDATOR_STAKE * U256::from(2); 10], 2);
    set_min_verdict_due_blocks(&mut harness, 10);
    let record = production_liveness_storage()
        .validators_accessor()
        .entry(members[0]);
    record
        .last_failed_epoch_p1_accessor()
        .set_checked(&mut harness.sdk, 1)
        .unwrap();
    record
        .kick_count_accessor()
        .set_checked(&mut harness.sdk, 5)
        .unwrap();

    equal_weight_committee(&mut harness.sdk, 30, &members);
    // Due is 20 apiece; members[0] produced nothing and fails, the rest clear it.
    seed_epoch_production(
        &mut harness.sdk,
        30,
        &[0, 20, 20, 20, 20, 20, 20, 20, 20, 20],
        200,
    );
    assert_eq!(close_epoch_via_record(&mut harness, 30), ExitCode::Ok);

    let (stamp, readmit, kicks) = production_record(&harness.sdk, members[0]);
    assert_eq!(
        kicks, 1,
        "the run was retired before this failure was counted, so the ladder \
         restarts rather than reaching its sixth rung"
    );
    assert_eq!(stamp, 31, "and the failure re-stamps the run");
    assert_eq!(
        readmit, 32,
        "one epoch of exclusion, the first rung — not the six the old count \
         would have bought"
    );
}
