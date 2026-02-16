use crate::{deploy_entry, main_entry};
use alloc::vec::Vec;
use fluentbase_sdk::{
    codec::SolidityABI,
    evm::write_evm_exit_message,
    storage::{StorageMap, StorageU256},
    Address, Bytes, ContextReader, ContractContextV1, ExitCode, SharedAPI, B256, FUEL_DENOM_RATE,
    PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME, U256,
};
use fluentbase_testing::TestingContextImpl;
use fluentbase_universal_token::{
    consts::{
        BALANCE_STORAGE_SLOT, ERR_ERC20_INSUFFICIENT_ALLOWANCE, ERR_ERC20_INSUFFICIENT_BALANCE,
        ERR_ERC20_INVALID_APPROVER, ERR_ERC20_INVALID_RECEIVER, ERR_ERC20_INVALID_SENDER,
        ERR_ERC20_INVALID_SPENDER, ERR_PAUSABLE_ENFORCED_PAUSE, ERR_PAUSABLE_EXPECTED_PAUSE,
        ERR_UST_MINTER_MISMATCH, ERR_UST_NOT_MINTABLE, ERR_UST_NOT_PAUSABLE,
        ERR_UST_PAUSER_MISMATCH, ERR_UST_UNKNOWN_METHOD, SIG_ERC20_ALLOWANCE, SIG_ERC20_APPROVE,
        SIG_ERC20_BALANCE, SIG_ERC20_BALANCE_OF, SIG_ERC20_BURN, SIG_ERC20_DECIMALS,
        SIG_ERC20_MINT, SIG_ERC20_NAME, SIG_ERC20_PAUSE, SIG_ERC20_SYMBOL, SIG_ERC20_TOTAL_SUPPLY,
        SIG_ERC20_TRANSFER, SIG_ERC20_TRANSFER_FROM, SIG_ERC20_UNPAUSE,
    },
    storage::InitialSettings,
};

/// Encodes a `U256` into a 32-byte big-endian ABI word.
fn abi_word_u256(x: U256) -> [u8; 32] {
    x.to_be_bytes::<{ U256::BYTES }>()
}

/// Encodes an `Address` into a 32-byte ABI word (right-aligned).
fn abi_word_addr(a: Address) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a.as_ref()); // right-aligned 20 bytes
    w
}

fn abi_encode_2_addr_1_u256(a1: Address, a2: Address, x: U256) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 * 3);
    out.extend_from_slice(&abi_word_addr(a1));
    out.extend_from_slice(&abi_word_addr(a2));
    out.extend_from_slice(&abi_word_u256(x));
    out
}

fn abi_encode_1_addr_1_u256(a: Address, x: U256) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&abi_word_addr(a));
    out.extend_from_slice(&abi_word_u256(x));
    out
}

fn abi_encode_2_addr(a1: Address, a2: Address) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&abi_word_addr(a1));
    out.extend_from_slice(&abi_word_addr(a2));
    out
}

fn abi_encode_1_addr(a: Address) -> Vec<u8> {
    abi_word_addr(a).to_vec()
}

/// Decode a single 32-byte ABI word as U256.
fn abi_decode_u256_word(bytes: &[u8]) -> U256 {
    assert_eq!(bytes.len(), 32, "expected exactly 32 bytes ABI word");
    U256::from_be_bytes::<{ U256::BYTES }>(bytes.try_into().unwrap())
}

/// Decode a single ABI bool return value (32 bytes; 0 or 1).
fn abi_decode_bool_word(bytes: &[u8]) -> bool {
    let v = abi_decode_u256_word(bytes);
    assert!(
        v == U256::ZERO || v == U256::ONE,
        "bool must be 0 or 1, got {v:?}"
    );
    v == U256::ONE
}

/// Decode `string` ABI output (standard Solidity ABI):
/// head: offset (32)
/// tail: len (32) + data (padded to 32)
fn abi_decode_string(output: &[u8]) -> String {
    assert!(
        output.len() >= 64 && output.len() % 32 == 0,
        "string abi output must be >= 64 bytes and 32-byte aligned"
    );

    let off = abi_decode_u256_word(&output[0..32])
        .try_into()
        .ok()
        .and_then(|x: u128| usize::try_from(x).ok())
        .expect("offset too large");
    assert!(off + 32 <= output.len(), "offset out of bounds");

    let len = abi_decode_u256_word(&output[off..off + 32])
        .try_into()
        .ok()
        .and_then(|x: u128| usize::try_from(x).ok())
        .expect("length too large");
    let data_start = off + 32;
    let data_end = data_start + len;
    assert!(data_end <= output.len(), "string data out of bounds");

    let data = &output[data_start..data_end];
    core::str::from_utf8(data)
        .expect("string must be valid UTF-8")
        .to_string()
}

/// Prefixes calldata with a 4-byte big-endian selector.
fn with_sig(sig: u32, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + data.len());
    out.extend_from_slice(&sig.to_be_bytes());
    out.extend_from_slice(data);
    out
}

/// Builds the EVM exit-message bytes for a given error code.
fn evm_exit_bytes(code: u32) -> Vec<u8> {
    let mut out = Vec::new();
    write_evm_exit_message(code, |slice| out.extend_from_slice(slice));
    out
}

/// Standard ERC-20 success return value: `1` as 32-byte word.
fn ok_32() -> Vec<u8> {
    U256::ONE.to_be_bytes::<{ U256::BYTES }>().to_vec()
}

// keccak256("Transfer(address,address,uint256)")
const TOPIC_TRANSFER: [u8; 32] = [
    0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37, 0x8d, 0xaa,
    0x95, 0x2b, 0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d, 0xf5, 0x23, 0xb3, 0xef,
];

// keccak256("Approval(address,address,uint256)")
const TOPIC_APPROVAL: [u8; 32] = [
    0x8c, 0x5b, 0xe1, 0xe5, 0xeb, 0xec, 0x7d, 0x5b, 0xd1, 0x4f, 0x71, 0x42, 0x7d, 0x1e, 0x84, 0xf3,
    0xdd, 0x03, 0x14, 0xc0, 0xf7, 0xb2, 0x29, 0x1e, 0x5b, 0x20, 0x0a, 0xc8, 0xc7, 0xc3, 0xb9, 0x25,
];

// keccak256("Paused(address)")
const TOPIC_PAUSED: [u8; 32] = [
    0x62, 0xe7, 0x8c, 0xea, 0x01, 0xbe, 0xe3, 0x20, 0xcd, 0x4e, 0x42, 0x02, 0x70, 0xb5, 0xea, 0x74,
    0x00, 0x0d, 0x11, 0xb0, 0xc9, 0xf7, 0x47, 0x54, 0xeb, 0xdb, 0xfc, 0x54, 0x4b, 0x05, 0xa2, 0x58,
];

// keccak256("Unpaused(address)")
const TOPIC_UNPAUSED: [u8; 32] = [
    0x5d, 0xb9, 0xee, 0x0a, 0x49, 0x5b, 0xf2, 0xe6, 0xff, 0x9c, 0x91, 0xa7, 0x83, 0x4c, 0x1b, 0xa4,
    0xfd, 0xd2, 0x44, 0xa5, 0xe8, 0xaa, 0x4e, 0x53, 0x7b, 0xd3, 0x8a, 0xea, 0xe4, 0xb0, 0x73, 0xaa,
];

fn topic_addr(a: Address) -> [u8; 32] {
    abi_word_addr(a)
}

fn data_u256(x: U256) -> Vec<u8> {
    abi_word_u256(x).to_vec()
}

#[derive(Clone, Debug)]
struct EvmLog {
    address: Address,
    topics: Vec<B256>,
    data: Bytes,
}

impl EvmLog {
    fn new(address: Address, topics: Vec<B256>, data: Bytes) -> Self {
        Self {
            address,
            topics,
            data,
        }
    }
}

fn assert_single_transfer_log(
    logs: &[EvmLog],
    token: Address,
    from: Address,
    to: Address,
    amount: U256,
) {
    assert_eq!(logs.len(), 1, "expected exactly 1 log, got {}", logs.len());
    let l = &logs[0];
    assert_eq!(l.address, token, "log emitter must be token contract");
    assert_eq!(l.topics.len(), 3, "Transfer must have 3 topics");

    assert_eq!(l.topics[0], TOPIC_TRANSFER, "wrong Transfer topic0");
    assert_eq!(l.topics[1], topic_addr(from), "wrong indexed from topic");
    assert_eq!(l.topics[2], topic_addr(to), "wrong indexed to topic");

    assert_eq!(l.data, data_u256(amount), "wrong Transfer data payload");
}

fn assert_single_approval_log(
    logs: &[EvmLog],
    token: Address,
    owner: Address,
    spender: Address,
    amount: U256,
) {
    assert_eq!(logs.len(), 1, "expected exactly 1 log, got {}", logs.len());
    let l = &logs[0];
    assert_eq!(l.address, token, "log emitter must be token contract");
    assert_eq!(l.topics.len(), 3, "Approval must have 3 topics");

    assert_eq!(l.topics[0], TOPIC_APPROVAL, "wrong Approval topic0");
    assert_eq!(l.topics[1], topic_addr(owner), "wrong indexed owner topic");
    assert_eq!(
        l.topics[2],
        topic_addr(spender),
        "wrong indexed spender topic"
    );

    assert_eq!(l.data, data_u256(amount), "wrong Approval data payload");
}

fn assert_single_paused_log(logs: &[EvmLog], token: Address, account: Address) {
    assert_eq!(logs.len(), 1, "expected exactly 1 log, got {}", logs.len());
    let l = &logs[0];
    assert_eq!(l.address, token, "log emitter must be token contract");
    assert_eq!(l.topics.len(), 1, "Paused must have 1 topic");

    assert_eq!(l.topics[0], TOPIC_PAUSED, "wrong Paused topic0");
    assert_eq!(
        l.data.as_ref(),
        topic_addr(account).as_slice(),
        "Paused event must have empty data"
    );
}

fn assert_single_unpaused_log(logs: &[EvmLog], token: Address, account: Address) {
    assert_eq!(logs.len(), 1, "expected exactly 1 log, got {}", logs.len());
    let l = &logs[0];
    assert_eq!(l.address, token, "log emitter must be token contract");
    assert_eq!(l.topics.len(), 1, "Unpaused must have 1 topics");

    assert_eq!(l.topics[0], TOPIC_UNPAUSED, "wrong Unpaused topic0");
    assert_eq!(
        l.data.as_ref(),
        topic_addr(account).as_slice(),
        "Unpaused event must have empty data"
    );
}

type BalanceStorageMap = StorageMap<Address, StorageU256>;

/// Stateful test harness that preserves storage across calls.
struct Harness {
    sdk: TestingContextImpl,
}

impl Harness {
    fn new(token_address: Address) -> Self {
        let gas_limit = 120_000;
        let sdk = TestingContextImpl::default()
            .with_contract_context(ContractContextV1 {
                address: token_address,
                bytecode_address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
                gas_limit,
                ..Default::default()
            })
            .with_gas_limit(gas_limit);
        Self { sdk }
    }

    fn set_caller(&mut self, caller: Address) {
        let mut ctx = self.sdk.context_mut();
        ctx.caller = caller;
    }

    /// Sets the `STATICCALL` flag for subsequent calls.
    ///
    /// When `is_static = true`, any method that attempts to mutate state must fail with
    /// `ExitCode::StateChangeDuringStaticCall`.
    fn set_static(&mut self, is_static: bool) {
        let mut ctx = self.sdk.context_mut();
        ctx.is_static = is_static;
    }

    fn call<I: Into<Bytes>>(&mut self, input: I) -> (ExitCode, Vec<u8>) {
        let input: Bytes = input.into();
        self.sdk = core::mem::take(&mut self.sdk).with_input(input);
        let storage_before_the_call = self.sdk.dump_storage();
        let exit_code = match main_entry(&mut self.sdk) {
            Ok(_) => ExitCode::Ok,
            Err(exit_code) => exit_code,
        };
        if !exit_code.is_ok() {
            self.sdk.restore_storage(storage_before_the_call);
        }
        let output = self.sdk.take_output();
        (exit_code, output)
    }

    fn deploy<I: Into<Bytes>>(&mut self, input: I, deployer: Address) -> (ExitCode, Vec<u8>) {
        let input: Bytes = input.into();
        self.set_caller(deployer);
        let storage_before_the_call = self.sdk.dump_storage();
        self.sdk = core::mem::take(&mut self.sdk).with_input(input);
        let exit_code = match deploy_entry(&mut self.sdk) {
            Ok(_) => ExitCode::Ok,
            Err(exit_code) => exit_code,
        };
        if !exit_code.is_ok() {
            self.sdk.restore_storage(storage_before_the_call);
        }
        let output = self.sdk.take_output();
        (exit_code, output)
    }

    #[allow(unused)]
    fn fuel_spent(&self) -> u64 {
        let gas_limit = self.sdk.context_mut().gas_limit;
        let gas_remaining = self.sdk.fuel() / FUEL_DENOM_RATE;
        gas_limit - gas_remaining
    }

    fn take_logs(&mut self) -> Vec<EvmLog> {
        let target_address = self.sdk.context().contract_address();
        self.sdk
            .take_logs()
            .into_iter()
            .map(|(data, topics)| EvmLog::new(target_address, topics.clone(), data.clone()))
            .collect()
    }
}

/// Deploys the token with `initial_supply` credited to `owner`.
fn deploy_with_supply_to(h: &mut Harness, owner: Address, supply: U256) {
    let mut s = InitialSettings::default();
    s.token_name = "TestToken".into();
    s.token_symbol = "TST".into();
    s.decimals = 18;
    s.initial_supply = supply;
    // optional: s.minter = None; s.pauser = None;
    let input = s.encode_with_prefix();

    // IMPORTANT: constructor mints to sdk.context().contract_caller()
    let _ = h.deploy(input, owner);
}

fn deploy_with_roles(
    h: &mut Harness,
    deployer: Address,
    supply: U256,
    minter: Address,
    pauser: Address,
) {
    let mut s = InitialSettings::default();
    s.token_name = "TestToken".into();
    s.token_symbol = "TST".into();
    s.decimals = 18;
    s.initial_supply = supply;
    s.minter = minter;
    s.pauser = pauser;
    let input = s.encode_with_prefix();
    let _ = h.deploy(input, deployer);
}

fn set_balance(h: &mut Harness, who: Address, value: U256) {
    BalanceStorageMap::new(BALANCE_STORAGE_SLOT)
        .entry(who)
        .set(&mut h.sdk, value);
}

#[test]
fn symbol_name_decimals_total_supply_balance_zero_by_default() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(2);
    let mut h = Harness::new(token);

    let mut initial_settings = InitialSettings::default();
    initial_settings.token_name = "Hello, World".into();
    initial_settings.token_symbol = "HW".into();
    initial_settings.decimals = 18;
    initial_settings.initial_supply = U256::ZERO;
    let input = initial_settings.encode_with_prefix();
    _ = h.deploy(input, owner);

    let (exit_code, name) = h.call(with_sig(SIG_ERC20_NAME, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    let name: String = SolidityABI::decode(&name.as_slice(), 0).unwrap();
    assert_eq!(name, "Hello, World");

    let (exit_code, sym) = h.call(with_sig(SIG_ERC20_SYMBOL, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    let sym: String = SolidityABI::decode(&sym.as_slice(), 0).unwrap();
    assert_eq!(sym, "HW");

    let (exit_code, dec) = h.call(with_sig(SIG_ERC20_DECIMALS, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(dec, abi_word_u256(U256::from(18u64)).to_vec());

    let (exit_code, ts) = h.call(with_sig(SIG_ERC20_TOTAL_SUPPLY, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(ts, abi_word_u256(U256::ZERO).to_vec());

    // balanceOf(owner) == 0, and balance() for caller == 0.
    h.set_caller(owner);
    let (exit_code, b0) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(owner)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(b0, abi_word_u256(U256::ZERO).to_vec());

    let (exit_code, bself) = h.call(with_sig(SIG_ERC20_BALANCE, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(bself, abi_word_u256(U256::ZERO).to_vec());
}

#[test]
fn transfer_happy_path_updates_balances_and_emits_ok() {
    let token = Address::with_last_byte(1);
    let alice = Address::with_last_byte(10);
    let bob = Address::with_last_byte(11);

    let mut h = Harness::new(token);

    deploy_with_supply_to(&mut h, alice, U256::from(100u64));

    h.set_caller(alice);
    let amount = U256::from(7u64);

    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER,
        &abi_encode_1_addr_1_u256(bob, amount),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    let (exit_code, out_alice) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(alice)));
    assert_eq!(exit_code, ExitCode::Ok);
    let (exit_code, out_bob) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(bob)));
    assert_eq!(exit_code, ExitCode::Ok);

    assert_eq!(out_alice, abi_word_u256(U256::from(93u64)).to_vec());
    assert_eq!(out_bob, abi_word_u256(U256::from(7u64)).to_vec());
}

#[test]
fn transfer_insufficient_balance_returns_error() {
    let token = Address::with_last_byte(1);
    let alice = Address::with_last_byte(10);
    let bob = Address::with_last_byte(11);

    let mut h = Harness::new(token);
    h.set_caller(alice);

    let amount = U256::from(1u64);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER,
        &abi_encode_1_addr_1_u256(bob, amount),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_ERC20_INSUFFICIENT_BALANCE));
}

#[test]
fn approve_and_allowance_round_trip() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(20);
    let spender = Address::with_last_byte(21);

    let mut h = Harness::new(token);
    h.set_caller(owner);

    let amt = U256::from(123u64);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_APPROVE,
        &abi_encode_1_addr_1_u256(spender, amt),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    let (exit_code, out2) = h.call(with_sig(
        SIG_ERC20_ALLOWANCE,
        &abi_encode_2_addr(owner, spender),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out2, abi_word_u256(amt).to_vec());
}

#[test]
fn transfer_from_should_spend_allowance_and_move_funds() {
    let token = Address::with_last_byte(1);
    let from = Address::with_last_byte(30);
    let spender = Address::with_last_byte(31);
    let to = Address::with_last_byte(32);

    let mut h = Harness::new(token);

    deploy_with_supply_to(&mut h, from, U256::from(100u64));

    h.set_caller(from);
    let approve_amt = U256::from(10u64);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_APPROVE,
        &abi_encode_1_addr_1_u256(spender, approve_amt),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    // spender transferFrom(from -> to)
    h.set_caller(spender);
    let amount = U256::from(3u64);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER_FROM,
        &abi_encode_2_addr_1_u256(from, to, amount),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    // allowance(from, spender) == 7
    let (exit_code, out_allow) = h.call(with_sig(
        SIG_ERC20_ALLOWANCE,
        &abi_encode_2_addr(from, spender),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out_allow, abi_word_u256(U256::from(7u64)).to_vec());

    // balances: from=97, to=3
    let (exit_code, out_from) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(from)));
    assert_eq!(exit_code, ExitCode::Ok);
    let (exit_code, out_to) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(to)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out_from, abi_word_u256(U256::from(97u64)).to_vec());
    assert_eq!(out_to, abi_word_u256(U256::from(3u64)).to_vec());
}

#[test]
fn approve_overwrites_allowance_not_adds() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(20);
    let spender = Address::with_last_byte(21);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, owner, U256::from(1u64));
    h.set_caller(owner);

    let (exit_code, _) = h.call(with_sig(
        SIG_ERC20_APPROVE,
        &abi_encode_1_addr_1_u256(spender, U256::from(10u64)),
    ));
    assert_eq!(exit_code, ExitCode::Ok);

    let (exit_code, _) = h.call(with_sig(
        SIG_ERC20_APPROVE,
        &abi_encode_1_addr_1_u256(spender, U256::from(3u64)),
    ));
    assert_eq!(exit_code, ExitCode::Ok);

    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_ALLOWANCE,
        &abi_encode_2_addr(owner, spender),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, abi_word_u256(U256::from(3u64)).to_vec());
}

#[test]
fn transfer_from_insufficient_allowance_does_not_move_funds() {
    let token = Address::with_last_byte(1);
    let from = Address::with_last_byte(30);
    let spender = Address::with_last_byte(31);
    let to = Address::with_last_byte(32);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, from, U256::from(5u64));

    // no approval set => allowance 0
    h.set_caller(spender);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER_FROM,
        &abi_encode_2_addr_1_u256(from, to, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_ERC20_INSUFFICIENT_ALLOWANCE));

    let (exit_code, out_from) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(from)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out_from, abi_word_u256(U256::from(5u64)).to_vec());

    let (exit_code, out_to) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(to)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out_to, abi_word_u256(U256::ZERO).to_vec());
}

#[test]
fn transfer_recipient_overflow_reverts_without_state_change() {
    let token = Address::with_last_byte(1);
    let alice = Address::with_last_byte(10);
    let bob = Address::with_last_byte(11);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, alice, U256::from(10u64));

    // Force bob balance to MAX so bob + 1 overflows.
    set_balance(&mut h, bob, U256::MAX);

    h.set_caller(alice);
    let (exit_code, _out) = h.call(with_sig(
        SIG_ERC20_TRANSFER,
        &abi_encode_1_addr_1_u256(bob, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::IntegerOverflow);

    let (exit_code, a) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(alice)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(a, abi_word_u256(U256::from(10u64)).to_vec());

    let (exit_code, b) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(bob)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(b, abi_word_u256(U256::MAX).to_vec());
}

#[test]
fn transfer_from_overflow_does_not_spend_allowance_or_balance() {
    let token = Address::with_last_byte(1);
    let from = Address::with_last_byte(30);
    let spender = Address::with_last_byte(31);
    let to = Address::with_last_byte(32);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, from, U256::from(10u64));

    // Approve 5
    h.set_caller(from);
    let (exit_code, _) = h.call(with_sig(
        SIG_ERC20_APPROVE,
        &abi_encode_1_addr_1_u256(spender, U256::from(5u64)),
    ));
    assert_eq!(exit_code, ExitCode::Ok);

    // Force recipient to overflow.
    set_balance(&mut h, to, U256::MAX);

    h.set_caller(spender);
    let (exit_code, _out) = h.call(with_sig(
        SIG_ERC20_TRANSFER_FROM,
        &abi_encode_2_addr_1_u256(from, to, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::IntegerOverflow);

    // allowance unchanged
    let (exit_code, allow) = h.call(with_sig(
        SIG_ERC20_ALLOWANCE,
        &abi_encode_2_addr(from, spender),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(allow, abi_word_u256(U256::from(5u64)).to_vec());

    // balances unchanged
    let (exit_code, b_from) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(from)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(b_from, abi_word_u256(U256::from(10u64)).to_vec());
}

#[test]
fn mint_requires_minter_and_not_paused_and_updates_supply() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(10);
    let minter = Address::with_last_byte(40);
    let pauser = Address::with_last_byte(50);
    let bob = Address::with_last_byte(41);
    let attacker = Address::with_last_byte(42);

    let mut h = Harness::new(token);

    // Not mintable until a minter is configured.
    h.set_caller(minter);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_MINT,
        &abi_encode_1_addr_1_u256(bob, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_UST_NOT_MINTABLE));

    // Deploy with minter+pauser configured, no initial supply.
    deploy_with_roles(&mut h, owner, U256::ZERO, minter, pauser);

    // Wrong caller.
    h.set_caller(attacker);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_MINT,
        &abi_encode_1_addr_1_u256(bob, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_UST_MINTER_MISMATCH));

    // Happy path.
    h.set_caller(minter);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_MINT,
        &abi_encode_1_addr_1_u256(bob, U256::from(5u64)),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    let (exit_code, ts) = h.call(with_sig(SIG_ERC20_TOTAL_SUPPLY, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(ts, abi_word_u256(U256::from(5u64)).to_vec());

    let (exit_code, b) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(bob)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(b, abi_word_u256(U256::from(5u64)).to_vec());

    // Pause blocks minting and must not mutate supply/balance.
    h.set_caller(pauser);
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    h.set_caller(minter);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_MINT,
        &abi_encode_1_addr_1_u256(bob, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_PAUSABLE_ENFORCED_PAUSE));

    let (exit_code, ts2) = h.call(with_sig(SIG_ERC20_TOTAL_SUPPLY, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(ts2, abi_word_u256(U256::from(5u64)).to_vec());
}

#[test]
fn burn_requires_minter_and_not_paused_and_updates_supply() {
    let token = Address::with_last_byte(2);
    let owner = Address::with_last_byte(20);
    let minter = Address::with_last_byte(40);
    let pauser = Address::with_last_byte(50);
    let victim = Address::with_last_byte(41);
    let attacker = Address::with_last_byte(42);

    let mut h = Harness::new(token);

    h.set_caller(minter);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_BURN,
        &abi_encode_1_addr_1_u256(victim, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_UST_NOT_MINTABLE));

    // Deploy with minter+pauser configured and some initial supply to victim.
    deploy_with_roles(&mut h, owner, U256::from(10u64), minter, pauser);
    h.set_caller(owner);
    let (ec, _) = h.call(with_sig(
        SIG_ERC20_TRANSFER,
        &abi_encode_1_addr_1_u256(victim, U256::from(10u64)),
    ));
    assert_eq!(ec, ExitCode::Ok);

    h.set_caller(attacker);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_BURN,
        &abi_encode_1_addr_1_u256(victim, U256::from(3u64)),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_UST_MINTER_MISMATCH));

    // Happy path: minter can burn from arbitrary victim balance.
    h.set_caller(minter);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_BURN,
        &abi_encode_1_addr_1_u256(victim, U256::from(4u64)),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    // totalSupply decreased from 10 to 6.
    let (exit_code, ts) = h.call(with_sig(SIG_ERC20_TOTAL_SUPPLY, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(ts, abi_word_u256(U256::from(6u64)).to_vec());

    // victim balance decreased from 10 to 6.
    let (exit_code, b) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(victim)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(b, abi_word_u256(U256::from(6u64)).to_vec());

    // Pause blocks burning and must not mutate supply/balance.
    h.set_caller(pauser);
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    h.set_caller(minter);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_BURN,
        &abi_encode_1_addr_1_u256(victim, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_PAUSABLE_ENFORCED_PAUSE));

    let (exit_code, ts2) = h.call(with_sig(SIG_ERC20_TOTAL_SUPPLY, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(ts2, abi_word_u256(U256::from(6u64)).to_vec());
}

#[test]
fn pause_unpause_access_control_and_idempotence() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(10);
    let pauser = Address::with_last_byte(50);
    let attacker = Address::with_last_byte(51);
    let mut h = Harness::new(token);

    // Not pausable until a pauser is configured.
    h.set_caller(pauser);
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_UST_NOT_PAUSABLE));

    // Deploy with pauser configured (minter irrelevant here).
    deploy_with_roles(&mut h, owner, U256::ZERO, Address::ZERO, pauser);

    // Wrong caller cannot pause.
    h.set_caller(attacker);
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_UST_PAUSER_MISMATCH));

    // Pause succeeds.
    h.set_caller(pauser);
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    // Pause twice => already paused.
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_PAUSABLE_ENFORCED_PAUSE));

    // Unpause wrong caller.
    h.set_caller(attacker);
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_UNPAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_UST_PAUSER_MISMATCH));

    // Unpause succeeds.
    h.set_caller(pauser);
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_UNPAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    // Unpause twice => already unpaused.
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_UNPAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_PAUSABLE_EXPECTED_PAUSE));
}

#[test]
fn pause_emits_paused_event() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(10);
    let pauser = Address::with_last_byte(50);

    let mut h = Harness::new(token);
    deploy_with_roles(&mut h, owner, U256::ZERO, Address::ZERO, pauser);
    let _ = h.take_logs(); // drain constructor logs

    h.set_caller(pauser);
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    let logs = h.take_logs();
    assert_single_paused_log(&logs, token, pauser);
}

#[test]
fn unpause_emits_unpaused_event() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(10);
    let pauser = Address::with_last_byte(50);

    let mut h = Harness::new(token);
    deploy_with_roles(&mut h, owner, U256::ZERO, Address::ZERO, pauser);
    let _ = h.take_logs(); // drain constructor logs

    // Pause first (ignore Paused log).
    h.set_caller(pauser);
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());
    let _ = h.take_logs();

    // Unpause should emit Unpaused(pauser).
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_UNPAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    let logs = h.take_logs();
    assert_single_unpaused_log(&logs, token, pauser);
}

#[test]
fn pause_unpause_reverts_do_not_emit_logs() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(10);
    let pauser = Address::with_last_byte(50);
    let attacker = Address::with_last_byte(51);

    let mut h = Harness::new(token);
    deploy_with_roles(&mut h, owner, U256::ZERO, Address::ZERO, pauser);
    let _ = h.take_logs(); // drain constructor logs

    // Wrong caller cannot pause.
    h.set_caller(attacker);
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_UST_PAUSER_MISMATCH));
    assert!(h.take_logs().is_empty());

    // Pause once (drain Paused log).
    h.set_caller(pauser);
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());
    let _ = h.take_logs();

    // Pause twice -> revert, no logs.
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_PAUSABLE_ENFORCED_PAUSE));
    assert!(h.take_logs().is_empty());

    // Unpause twice -> revert, no logs (first unpause drains log).
    let (exit_code, out) = h.call(with_sig(SIG_ERC20_UNPAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());
    let _ = h.take_logs();

    let (exit_code, out) = h.call(with_sig(SIG_ERC20_UNPAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_PAUSABLE_EXPECTED_PAUSE));
    assert!(h.take_logs().is_empty());
}

#[test]
fn unknown_method_returns_error() {
    let token = Address::with_last_byte(1);
    let mut h = Harness::new(token);

    let (exit_code, out) = h.call(with_sig(0xDEAD_BEEFu32, &[1, 2, 3]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_UST_UNKNOWN_METHOD));
}

#[test]
fn selectors_match_canonical_erc20_4bytes() {
    assert_eq!(SIG_ERC20_TOTAL_SUPPLY, 0x1816_0ddd);
    assert_eq!(SIG_ERC20_BALANCE_OF, 0x70a0_8231);
    assert_eq!(SIG_ERC20_TRANSFER, 0xa905_9cbb);
    assert_eq!(SIG_ERC20_TRANSFER_FROM, 0x23b8_72dd);
    assert_eq!(SIG_ERC20_APPROVE, 0x095e_a7b3);
    assert_eq!(SIG_ERC20_ALLOWANCE, 0xdd62_ed3e);
    assert_eq!(SIG_ERC20_NAME, 0x06fd_de03);
    assert_eq!(SIG_ERC20_SYMBOL, 0x95d8_9b41);
    assert_eq!(SIG_ERC20_DECIMALS, 0x313c_e567);
}

#[test]
fn selectors_do_not_collide_across_public_surface() {
    use alloc::collections::BTreeSet;

    let values = [
        // Errors
        ERR_UST_UNKNOWN_METHOD,
        ERR_UST_NOT_PAUSABLE,
        ERR_UST_PAUSER_MISMATCH,
        ERR_UST_NOT_MINTABLE,
        ERR_UST_MINTER_MISMATCH,
        ERR_ERC20_INSUFFICIENT_BALANCE,
        ERR_ERC20_INVALID_SENDER,
        ERR_ERC20_INVALID_RECEIVER,
        ERR_ERC20_INSUFFICIENT_ALLOWANCE,
        ERR_ERC20_INVALID_APPROVER,
        ERR_ERC20_INVALID_SPENDER,
        ERR_PAUSABLE_ENFORCED_PAUSE,
        ERR_PAUSABLE_EXPECTED_PAUSE,
        // Selectors
        SIG_ERC20_SYMBOL,
        SIG_ERC20_NAME,
        SIG_ERC20_DECIMALS,
        SIG_ERC20_TOTAL_SUPPLY,
        SIG_ERC20_BALANCE_OF,
        SIG_ERC20_TRANSFER,
        SIG_ERC20_TRANSFER_FROM,
        SIG_ERC20_ALLOWANCE,
        SIG_ERC20_APPROVE,
        SIG_ERC20_MINT,
        SIG_ERC20_BURN,
        SIG_ERC20_PAUSE,
        SIG_ERC20_UNPAUSE,
    ];

    let mut set = BTreeSet::new();
    for v in values {
        assert!(set.insert(v), "collision detected for value: 0x{v:08x}");
    }
}

#[test]
fn abi_encoding_address_is_left_padded_and_right_aligned() {
    let a = Address::with_last_byte(0xAB);
    let enc = abi_encode_1_addr(a);
    assert_eq!(enc.len(), 32);
    // first 12 bytes must be zero, last 20 bytes are address bytes
    assert!(enc[..12].iter().all(|&b| b == 0));
    assert_eq!(&enc[12..32], a.as_slice());
}

#[test]
fn abi_encoding_u256_is_big_endian_32_bytes() {
    let x = U256::from(0x1122_3344u64);
    let enc = abi_encode_1_addr_1_u256(Address::with_last_byte(1), x);

    // layout: addr word (32) + u256 word (32)
    assert_eq!(enc.len(), 64);
    let w = &enc[32..64];
    assert_eq!(abi_decode_u256_word(w), x);
}

#[test]
fn calldata_prefix_is_big_endian_selector() {
    // with_sig uses sig.to_be_bytes()
    let sig = 0xA1B2_C3D4u32;
    let data = [9u8, 8u8, 7u8];
    let cd = with_sig(sig, &data);
    assert_eq!(&cd[0..4], &sig.to_be_bytes());
    assert_eq!(&cd[4..], &data);
}

#[test]
fn name_symbol_decimals_are_correctly_abi_encoded() {
    let token = Address::with_last_byte(1);
    let deployer = Address::with_last_byte(2);
    let mut h = Harness::new(token);

    // Deploy with some config so contract is initialized.
    // Reuse your existing helper (already present in file).
    deploy_with_supply_to(&mut h, deployer, U256::from(1u64));

    // name()
    let (ec, out) = h.call(with_sig(SIG_ERC20_NAME, &[]));
    assert_eq!(ec, ExitCode::Ok);
    let name = abi_decode_string(&out);
    assert!(!name.is_empty(), "name() must not be empty for this impl");

    // symbol()
    let (ec, out) = h.call(with_sig(SIG_ERC20_SYMBOL, &[]));
    assert_eq!(ec, ExitCode::Ok);
    let sym = abi_decode_string(&out);
    assert!(!sym.is_empty(), "symbol() must not be empty for this impl");

    // decimals()
    let (ec, out) = h.call(with_sig(SIG_ERC20_DECIMALS, &[]));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(out.len(), 32);
    let dec = abi_decode_u256_word(&out);
    assert!(
        dec <= U256::from(255u64),
        "decimals should fit into uint8 semantics"
    );
}

#[test]
fn erc20_functions_return_exact_32_byte_words() {
    let token = Address::with_last_byte(1);
    let alice = Address::with_last_byte(10);
    let bob = Address::with_last_byte(11);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, alice, U256::from(10u64));

    // totalSupply() -> uint256 (32 bytes)
    let (ec, out) = h.call(with_sig(SIG_ERC20_TOTAL_SUPPLY, &[]));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(out.len(), 32);

    // balanceOf(address) -> uint256 (32 bytes)
    let (ec, out) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(alice)));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(out.len(), 32);

    // allowance(owner,spender) -> uint256 (32 bytes)
    let (ec, out) = h.call(with_sig(
        SIG_ERC20_ALLOWANCE,
        &abi_encode_2_addr(alice, bob),
    ));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(out.len(), 32);

    // transfer/approve/transferFrom -> bool (32 bytes)
    h.set_caller(alice);
    let (ec, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER,
        &abi_encode_1_addr_1_u256(bob, U256::from(1u64)),
    ));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(out.len(), 32);
    assert!(abi_decode_bool_word(&out));
}

#[test]
fn transfer_to_zero_address_reverts_and_preserves_state() {
    let token = Address::with_last_byte(1);
    let alice = Address::with_last_byte(10);
    let zero = Address::ZERO;

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, alice, U256::from(10u64));

    h.set_caller(alice);
    let (ec, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER,
        &abi_encode_1_addr_1_u256(zero, U256::from(1u64)),
    ));
    assert_eq!(ec, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_ERC20_INVALID_RECEIVER));

    // balances unchanged
    let (ec, bal) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(alice)));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(bal, abi_word_u256(U256::from(10u64)).to_vec());
}

#[test]
fn transfer_from_to_zero_address_reverts_and_preserves_allowance_and_balances() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(10);
    let spender = Address::with_last_byte(11);
    let zero = Address::ZERO;

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, owner, U256::from(10u64));

    // approve spender for 5
    h.set_caller(owner);
    let (ec, _) = h.call(with_sig(
        SIG_ERC20_APPROVE,
        &abi_encode_1_addr_1_u256(spender, U256::from(5u64)),
    ));
    assert_eq!(ec, ExitCode::Ok);

    // spender tries transferFrom(owner -> zero)
    h.set_caller(spender);
    let (ec, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER_FROM,
        &abi_encode_2_addr(owner, zero)
            .into_iter()
            .chain(abi_word_u256(U256::from(1u64)).into_iter())
            .collect::<Vec<_>>(),
    ));
    assert_eq!(ec, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_ERC20_INVALID_RECEIVER));

    // allowance unchanged
    let (ec, allow) = h.call(with_sig(
        SIG_ERC20_ALLOWANCE,
        &abi_encode_2_addr(owner, spender),
    ));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(allow, abi_word_u256(U256::from(5u64)).to_vec());

    // balances unchanged
    let (ec, bal_owner) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(owner)));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(bal_owner, abi_word_u256(U256::from(10u64)).to_vec());
}

#[test]
fn approve_then_transfer_from_exactly_consumes_allowance_to_zero() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(10);
    let spender = Address::with_last_byte(11);
    let to = Address::with_last_byte(12);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, owner, U256::from(10u64));

    h.set_caller(owner);
    let (ec, _) = h.call(with_sig(
        SIG_ERC20_APPROVE,
        &abi_encode_1_addr_1_u256(spender, U256::from(4u64)),
    ));
    assert_eq!(ec, ExitCode::Ok);

    h.set_caller(spender);
    let (ec, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER_FROM,
        &abi_encode_2_addr(owner, to)
            .into_iter()
            .chain(abi_word_u256(U256::from(4u64)).into_iter())
            .collect::<Vec<_>>(),
    ));
    assert_eq!(ec, ExitCode::Ok);
    assert!(abi_decode_bool_word(&out));

    // allowance now 0
    let (ec, allow) = h.call(with_sig(
        SIG_ERC20_ALLOWANCE,
        &abi_encode_2_addr(owner, spender),
    ));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(allow, abi_word_u256(U256::ZERO).to_vec());
}

#[test]
fn insufficient_balance_transfer_from_does_not_change_allowance() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(10);
    let spender = Address::with_last_byte(11);
    let to = Address::with_last_byte(12);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, owner, U256::from(1u64));

    h.set_caller(owner);
    let (ec, _) = h.call(with_sig(
        SIG_ERC20_APPROVE,
        &abi_encode_1_addr_1_u256(spender, U256::from(10u64)),
    ));
    assert_eq!(ec, ExitCode::Ok);

    h.set_caller(spender);
    let (ec, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER_FROM,
        &abi_encode_2_addr(owner, to)
            .into_iter()
            .chain(abi_word_u256(U256::from(2u64)).into_iter())
            .collect::<Vec<_>>(),
    ));
    assert_eq!(ec, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_ERC20_INSUFFICIENT_BALANCE));

    // allowance preserved (standard expectation: failed transferFrom doesn't mutate allowance)
    let (ec, allow) = h.call(with_sig(
        SIG_ERC20_ALLOWANCE,
        &abi_encode_2_addr(owner, spender),
    ));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(allow, abi_word_u256(U256::from(10u64)).to_vec());
}

#[test]
fn total_supply_is_invariant_under_transfers() {
    let token = Address::with_last_byte(1);
    let alice = Address::with_last_byte(10);
    let bob = Address::with_last_byte(11);
    let carol = Address::with_last_byte(12);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, alice, U256::from(100u64));

    // totalSupply at start
    let (ec, ts0) = h.call(with_sig(SIG_ERC20_TOTAL_SUPPLY, &[]));
    assert_eq!(ec, ExitCode::Ok);

    // alice -> bob 7
    h.set_caller(alice);
    let (ec, _) = h.call(with_sig(
        SIG_ERC20_TRANSFER,
        &abi_encode_1_addr_1_u256(bob, U256::from(7u64)),
    ));
    assert_eq!(ec, ExitCode::Ok);

    // bob -> carol 2
    h.set_caller(bob);
    let (ec, _) = h.call(with_sig(
        SIG_ERC20_TRANSFER,
        &abi_encode_1_addr_1_u256(carol, U256::from(2u64)),
    ));
    assert_eq!(ec, ExitCode::Ok);

    // totalSupply unchanged
    let (ec, ts1) = h.call(with_sig(SIG_ERC20_TOTAL_SUPPLY, &[]));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(ts0, ts1);

    // sum balances equals totalSupply
    let (ec, ba) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(alice)));
    assert_eq!(ec, ExitCode::Ok);
    let (ec, bb) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(bob)));
    assert_eq!(ec, ExitCode::Ok);
    let (ec, bc) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(carol)));
    assert_eq!(ec, ExitCode::Ok);

    let sum = abi_decode_u256_word(&ba) + abi_decode_u256_word(&bb) + abi_decode_u256_word(&bc);
    assert_eq!(sum.to_be_bytes::<{ U256::BYTES }>().to_vec(), ts1);
}

#[test]
fn malformed_calldata_wrong_length_is_not_silently_accepted() {
    let token = Address::with_last_byte(1);
    let alice = Address::with_last_byte(10);
    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, alice, U256::from(1u64));
    h.set_caller(alice);

    // transfer(address,uint256) but only provide 1 byte of args => must revert/panic
    let cd = with_sig(SIG_ERC20_TRANSFER, &[0xAA]);
    let (ec, _out) = h.call(cd);
    assert_eq!(
        ec,
        ExitCode::MalformedBuiltinParams,
        "malformed calldata must not succeed"
    );

    // balanceOf(address) but provide empty args
    let cd = with_sig(SIG_ERC20_BALANCE_OF, &[]);
    let (ec, _out) = h.call(cd);
    assert_eq!(
        ec,
        ExitCode::MalformedBuiltinParams,
        "malformed calldata must not succeed"
    );
}

#[test]
fn transfer_emits_transfer_event_exactly() {
    let token = Address::with_last_byte(1);
    let alice = Address::with_last_byte(10);
    let bob = Address::with_last_byte(11);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, alice, U256::from(100u64));

    // clear any constructor logs (mint Transfer) so we only check this call
    let _ = h.take_logs();

    h.set_caller(alice);
    let amount = U256::from(7u64);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER,
        &abi_encode_1_addr_1_u256(bob, amount),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    let logs = h.take_logs();
    assert_single_transfer_log(&logs, token, alice, bob, amount);
}

#[test]
fn burn_emits_transfer_to_zero_exactly() {
    let token = Address::with_last_byte(3);
    let owner = Address::with_last_byte(10);
    let minter = Address::with_last_byte(40);
    let victim = Address::with_last_byte(41);

    let mut h = Harness::new(token);
    deploy_with_roles(&mut h, owner, U256::from(10u64), minter, Address::ZERO);

    // Move full supply to victim.
    h.set_caller(owner);
    let (ec, _) = h.call(with_sig(
        SIG_ERC20_TRANSFER,
        &abi_encode_1_addr_1_u256(victim, U256::from(10u64)),
    ));
    assert_eq!(ec, ExitCode::Ok);

    let _ = h.take_logs(); // clear constructor Transfer log and transfer log

    // Minter burns a portion from victim.
    h.set_caller(minter);
    let amount = U256::from(4u64);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_BURN,
        &abi_encode_1_addr_1_u256(victim, amount),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    let logs = h.take_logs();
    // Expect a single Transfer(victim, 0x0, amount)
    assert_single_transfer_log(&logs, token, victim, Address::ZERO, amount);
}

#[test]
fn approve_emits_approval_event_exactly() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(20);
    let spender = Address::with_last_byte(21);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, owner, U256::from(1u64));

    let _ = h.take_logs();

    h.set_caller(owner);
    let amt = U256::from(123u64);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_APPROVE,
        &abi_encode_1_addr_1_u256(spender, amt),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    let logs = h.take_logs();
    assert_single_approval_log(&logs, token, owner, spender, amt);
}

#[test]
fn transfer_from_emits_transfer_event_exactly() {
    let token = Address::with_last_byte(1);
    let from = Address::with_last_byte(30);
    let spender = Address::with_last_byte(31);
    let to = Address::with_last_byte(32);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, from, U256::from(100u64));
    let _ = h.take_logs();

    // approve
    h.set_caller(from);
    let (exit_code, _) = h.call(with_sig(
        SIG_ERC20_APPROVE,
        &abi_encode_1_addr_1_u256(spender, U256::from(10u64)),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    let _ = h.take_logs(); // ignore Approval here

    // transferFrom
    h.set_caller(spender);
    let amount = U256::from(3u64);
    let (exit_code, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER_FROM,
        &abi_encode_2_addr_1_u256(from, to, amount),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    let logs = h.take_logs();
    assert_single_transfer_log(&logs, token, from, to, amount);
}

#[test]
fn reverts_emit_no_logs() {
    let token = Address::with_last_byte(1);
    let alice = Address::with_last_byte(10);
    let bob = Address::with_last_byte(11);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, alice, U256::from(1u64));
    let _ = h.take_logs();

    // Force overflow on recipient so transfer reverts.
    set_balance(&mut h, bob, U256::MAX);

    h.set_caller(alice);
    let (exit_code, _out) = h.call(with_sig(
        SIG_ERC20_TRANSFER,
        &abi_encode_1_addr_1_u256(bob, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::IntegerOverflow);

    let logs = h.take_logs();
    assert!(logs.is_empty(), "reverted call must not emit logs");
}

#[test]
fn constructor_mint_emits_transfer_from_zero_if_supply_nonzero() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(2);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, owner, U256::from(42u64));

    let logs = h.take_logs();

    // Many ERC-20s emit Transfer(0x0 -> owner, supply) on mint during construction.
    // This is important for indexers to detect initial distribution.
    assert_single_transfer_log(&logs, token, Address::ZERO, owner, U256::from(42u64));
}

#[test]
fn staticcall_blocks_transfer_and_emits_no_logs_and_no_state_change() {
    let token = Address::with_last_byte(1);
    let alice = Address::with_last_byte(10);
    let bob = Address::with_last_byte(11);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, alice, U256::from(100u64));
    let _ = h.take_logs();

    h.set_caller(alice);
    h.set_static(true);

    let amount = U256::from(7u64);
    let (ec, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER,
        &abi_encode_1_addr_1_u256(bob, amount),
    ));
    assert_eq!(ec, ExitCode::StateChangeDuringStaticCall);
    assert!(out.is_empty(), "staticcall must not return a success word");

    // No logs on staticcall failure.
    assert!(h.take_logs().is_empty());

    // State must be unchanged.
    h.set_static(false);
    let (ec, bal_a) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(alice)));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(bal_a, abi_word_u256(U256::from(100u64)).to_vec());

    let (ec, bal_b) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(bob)));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(bal_b, abi_word_u256(U256::ZERO).to_vec());
}

#[test]
fn staticcall_blocks_approve_and_emits_no_logs_and_no_state_change() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(20);
    let spender = Address::with_last_byte(21);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, owner, U256::from(1u64));
    let _ = h.take_logs();

    h.set_caller(owner);
    h.set_static(true);

    let amt = U256::from(123u64);
    let (ec, out) = h.call(with_sig(
        SIG_ERC20_APPROVE,
        &abi_encode_1_addr_1_u256(spender, amt),
    ));
    assert_eq!(ec, ExitCode::StateChangeDuringStaticCall);
    assert!(out.is_empty());

    assert!(h.take_logs().is_empty());

    h.set_static(false);
    let (ec, allow) = h.call(with_sig(
        SIG_ERC20_ALLOWANCE,
        &abi_encode_2_addr(owner, spender),
    ));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(allow, abi_word_u256(U256::ZERO).to_vec());
}

#[test]
fn staticcall_blocks_transfer_from_and_emits_no_logs_and_no_state_change() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(30);
    let spender = Address::with_last_byte(31);
    let to = Address::with_last_byte(32);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, owner, U256::from(100u64));
    let _ = h.take_logs();

    // approve spender for 10 in a non-static call
    h.set_caller(owner);
    let (ec, _) = h.call(with_sig(
        SIG_ERC20_APPROVE,
        &abi_encode_1_addr_1_u256(spender, U256::from(10u64)),
    ));
    assert_eq!(ec, ExitCode::Ok);
    let _ = h.take_logs();

    // transferFrom under staticcall must fail and not mutate allowance/balances
    h.set_caller(spender);
    h.set_static(true);

    let amount = U256::from(3u64);
    let (ec, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER_FROM,
        &abi_encode_2_addr_1_u256(owner, to, amount),
    ));
    assert_eq!(ec, ExitCode::StateChangeDuringStaticCall);
    assert!(out.is_empty());
    assert!(h.take_logs().is_empty());

    h.set_static(false);

    let (ec, allow) = h.call(with_sig(
        SIG_ERC20_ALLOWANCE,
        &abi_encode_2_addr(owner, spender),
    ));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(allow, abi_word_u256(U256::from(10u64)).to_vec());

    let (ec, bal_owner) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(owner)));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(bal_owner, abi_word_u256(U256::from(100u64)).to_vec());
}

#[test]
fn staticcall_blocks_pause_and_unpause_and_emits_no_logs() {
    let token = Address::with_last_byte(1);
    let pauser = Address::with_last_byte(40);

    let mut h = Harness::new(token);

    // Deploy with pauser configured if your contract supports it; otherwise these calls already revert.
    let mut s = InitialSettings::default();
    s.token_name = "TestToken".into();
    s.token_symbol = "TST".into();
    s.decimals = 18;
    s.initial_supply = U256::from(0u64);
    s.pauser = pauser;
    let input = s.encode_with_prefix();
    let _ = h.deploy(input, pauser);

    let _ = h.take_logs();

    h.set_caller(pauser);
    h.set_static(true);

    let (ec, out) = h.call(with_sig(SIG_ERC20_PAUSE, &[]));
    assert_eq!(ec, ExitCode::StateChangeDuringStaticCall);
    assert!(out.is_empty());
    assert!(h.take_logs().is_empty());

    let (ec, out) = h.call(with_sig(SIG_ERC20_UNPAUSE, &[]));
    assert_eq!(ec, ExitCode::StateChangeDuringStaticCall);
    assert!(out.is_empty());
    assert!(h.take_logs().is_empty());
}

#[test]
fn staticcall_blocks_mint_and_emits_no_logs_and_no_state_change() {
    let token = Address::with_last_byte(1);
    let minter = Address::with_last_byte(50);
    let recipient = Address::with_last_byte(51);

    let mut h = Harness::new(token);

    // Deploy mintable with `minter` configured.
    let mut s = InitialSettings::default();
    s.token_name = "TestToken".into();
    s.token_symbol = "TST".into();
    s.decimals = 18;
    s.initial_supply = U256::from(0u64);
    s.minter = minter;
    let input = s.encode_with_prefix();
    let _ = h.deploy(input, minter);

    let _ = h.take_logs();

    h.set_caller(minter);
    h.set_static(true);

    let amt = U256::from(9u64);
    let (ec, out) = h.call(with_sig(
        SIG_ERC20_MINT,
        &abi_encode_1_addr_1_u256(recipient, amt),
    ));
    assert_eq!(ec, ExitCode::StateChangeDuringStaticCall);
    assert!(out.is_empty());
    assert!(h.take_logs().is_empty());

    h.set_static(false);
    let (ec, bal) = h.call(with_sig(
        SIG_ERC20_BALANCE_OF,
        &abi_encode_1_addr(recipient),
    ));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(bal, abi_word_u256(U256::ZERO).to_vec());
}

#[test]
fn staticcall_blocks_burn_and_emits_no_logs_and_no_state_change() {
    let token = Address::with_last_byte(1);
    let minter = Address::with_last_byte(50);
    let from = Address::with_last_byte(51);

    let mut h = Harness::new(token);

    // Deploy burnable with `minter` configured and mint some tokens to `minter`.
    deploy_with_roles(&mut h, minter, U256::from(100u64), minter, Address::ZERO);
    let _ = h.take_logs();

    // Transfer tokens from `minter` to `from` so we can test burning from `from`.
    h.set_caller(minter);
    let (ec, _) = h.call(with_sig(
        SIG_ERC20_TRANSFER,
        &abi_encode_1_addr_1_u256(from, U256::from(100u64)),
    ));
    assert_eq!(ec, ExitCode::Ok);
    let _ = h.take_logs();

    // Try to burn from `from` under staticcall.
    h.set_caller(minter);
    h.set_static(true);

    // check initial balance of `from`
    let (ec, bal) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(from)));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(bal, abi_word_u256(U256::from(100u64)).to_vec());

    let amt = U256::from(9u64);
    let (ec, out) = h.call(with_sig(
        SIG_ERC20_BURN,
        &abi_encode_1_addr_1_u256(from, amt),
    ));
    assert_eq!(ec, ExitCode::StateChangeDuringStaticCall);
    assert!(out.is_empty());
    assert!(h.take_logs().is_empty());

    h.set_static(false);
    let (ec, bal) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(from)));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(bal, abi_word_u256(U256::from(100u64)).to_vec());
}

#[test]
fn transfer_from_does_not_decrease_allowance_when_allowance_is_u256_max() {
    let token = Address::with_last_byte(1);
    let owner = Address::with_last_byte(10);
    let spender = Address::with_last_byte(11);
    let to = Address::with_last_byte(12);

    let mut h = Harness::new(token);
    deploy_with_supply_to(&mut h, owner, U256::from(100u64));

    // owner approves "infinite" allowance
    h.set_caller(owner);
    let (ec, out) = h.call(with_sig(
        SIG_ERC20_APPROVE,
        &abi_encode_1_addr_1_u256(spender, U256::MAX),
    ));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(out, ok_32());

    // spender spends some amount via transferFrom
    h.set_caller(spender);
    let amount = U256::from(7u64);
    let (ec, out) = h.call(with_sig(
        SIG_ERC20_TRANSFER_FROM,
        &abi_encode_2_addr_1_u256(owner, to, amount),
    ));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(out, ok_32());

    // allowance must remain U256::MAX (not charged)
    let (ec, allow) = h.call(with_sig(
        SIG_ERC20_ALLOWANCE,
        &abi_encode_2_addr(owner, spender),
    ));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(allow, abi_word_u256(U256::MAX).to_vec());

    // sanity: balances moved correctly
    let (ec, bal_owner) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(owner)));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(bal_owner, abi_word_u256(U256::from(93u64)).to_vec());

    let (ec, bal_to) = h.call(with_sig(SIG_ERC20_BALANCE_OF, &abi_encode_1_addr(to)));
    assert_eq!(ec, ExitCode::Ok);
    assert_eq!(bal_to, abi_word_u256(amount).to_vec());
}
