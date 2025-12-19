use crate::{deploy_entry, main_entry};
use alloc::vec::Vec;
use fluentbase_sdk::{
    evm::write_evm_exit_message,
    storage::{StorageMap, StorageU256},
    Address, Bytes, ContractContextV1, ExitCode, SharedAPI, FUEL_DENOM_RATE, U256,
};
use fluentbase_testing::HostTestingContext;
use fluentbase_universal_token::{
    consts::{
        // Storage slots
        BALANCE_STORAGE_SLOT,
        // Errors
        ERR_ALREADY_PAUSED,
        ERR_ALREADY_UNPAUSED,
        ERR_CONTRACT_NOT_MINTABLE,
        ERR_CONTRACT_NOT_PAUSABLE,
        ERR_INSUFFICIENT_ALLOWANCE,
        ERR_INSUFFICIENT_BALANCE,
        ERR_INTEGER_OVERFLOW,
        ERR_INVALID_MINTER,
        ERR_MINTING_PAUSED,
        ERR_PAUSER_MISMATCH,
        ERR_UNKNOWN_METHOD,
        // Selectors
        SIG_ALLOWANCE,
        SIG_APPROVE,
        SIG_BALANCE,
        SIG_BALANCE_OF,
        SIG_DECIMALS,
        SIG_MINT,
        SIG_NAME,
        SIG_PAUSE,
        SIG_SYMBOL,
        SIG_TOTAL_SUPPLY,
        SIG_TRANSFER,
        SIG_TRANSFER_FROM,
        SIG_UNPAUSE,
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

type BalanceStorageMap = StorageMap<Address, StorageU256>;
type AllowanceStorageMap = StorageMap<Address, StorageMap<Address, StorageU256>>;
/// Stateful test harness that preserves storage across calls.
struct Harness {
    sdk: HostTestingContext,
    token: Address,
    gas_limit: u64,
}

impl Harness {
    fn new(token: Address) -> Self {
        let gas_limit = 120_000;
        let sdk = HostTestingContext::default()
            .with_contract_context(ContractContextV1 {
                address: token,
                bytecode_address: token,
                gas_limit,
                ..Default::default()
            })
            .with_gas_limit(gas_limit);
        Self {
            sdk,
            token,
            gas_limit,
        }
    }

    fn set_caller(&mut self, caller: Address) {
        self.sdk = core::mem::take(&mut self.sdk).with_contract_context(ContractContextV1 {
            address: self.token,
            bytecode_address: self.token,
            gas_limit: self.gas_limit,
            caller,
            ..Default::default()
        });
    }

    fn call<I: Into<Bytes>>(&mut self, input: I) -> (ExitCode, Vec<u8>) {
        let input: Bytes = input.into();
        self.sdk = core::mem::take(&mut self.sdk).with_input(input);
        let exit_code = match main_entry(&mut self.sdk) {
            Ok(_) => ExitCode::Ok,
            Err(exit_code) => exit_code,
        };
        let output = self.sdk.take_output();
        (exit_code, output)
    }

    fn deploy<I: Into<Bytes>>(&mut self, input: I, deployer: Address) -> (ExitCode, Vec<u8>) {
        let input: Bytes = input.into();
        self.set_caller(deployer);
        self.sdk = core::mem::take(&mut self.sdk).with_input(input);
        let exit_code = match deploy_entry(&mut self.sdk) {
            Ok(_) => ExitCode::Ok,
            Err(exit_code) => exit_code,
        };
        let output = self.sdk.take_output();
        (exit_code, output)
    }

    fn fuel_spent(&self) -> u64 {
        let gas_remaining = self.sdk.fuel() / FUEL_DENOM_RATE;
        self.gas_limit - gas_remaining
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
    s.minter = Some(minter);
    s.pauser = Some(pauser);
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

    let (exit_code, name) = h.call(with_sig(SIG_NAME, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(core::str::from_utf8(&name).unwrap(), "Hello, World");

    let (exit_code, sym) = h.call(with_sig(SIG_SYMBOL, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(core::str::from_utf8(&sym).unwrap(), "HW");

    let (exit_code, dec) = h.call(with_sig(SIG_DECIMALS, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(dec, abi_word_u256(U256::from(18u64)).to_vec());

    let (exit_code, ts) = h.call(with_sig(SIG_TOTAL_SUPPLY, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(ts, abi_word_u256(U256::ZERO).to_vec());

    // balanceOf(owner) == 0, and balance() for caller == 0.
    h.set_caller(owner);
    let (exit_code, b0) = h.call(with_sig(SIG_BALANCE_OF, &abi_encode_1_addr(owner)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(b0, abi_word_u256(U256::ZERO).to_vec());

    let (exit_code, bself) = h.call(with_sig(SIG_BALANCE, &[]));
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
        SIG_TRANSFER,
        &abi_encode_1_addr_1_u256(bob, amount),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    let (exit_code, out_alice) = h.call(with_sig(SIG_BALANCE_OF, &abi_encode_1_addr(alice)));
    assert_eq!(exit_code, ExitCode::Ok);
    let (exit_code, out_bob) = h.call(with_sig(SIG_BALANCE_OF, &abi_encode_1_addr(bob)));
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
        SIG_TRANSFER,
        &abi_encode_1_addr_1_u256(bob, amount),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_INSUFFICIENT_BALANCE));
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
        SIG_APPROVE,
        &abi_encode_1_addr_1_u256(spender, amt),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    let (exit_code, out2) = h.call(with_sig(SIG_ALLOWANCE, &abi_encode_2_addr(owner, spender)));
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
        SIG_APPROVE,
        &abi_encode_1_addr_1_u256(spender, approve_amt),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    // spender transferFrom(from -> to)
    h.set_caller(spender);
    let amount = U256::from(3u64);
    let (exit_code, out) = h.call(with_sig(
        SIG_TRANSFER_FROM,
        &abi_encode_2_addr_1_u256(from, to, amount),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    // allowance(from, spender) == 7
    let (exit_code, out_allow) = h.call(with_sig(SIG_ALLOWANCE, &abi_encode_2_addr(from, spender)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out_allow, abi_word_u256(U256::from(7u64)).to_vec());

    // balances: from=97, to=3
    let (exit_code, out_from) = h.call(with_sig(SIG_BALANCE_OF, &abi_encode_1_addr(from)));
    assert_eq!(exit_code, ExitCode::Ok);
    let (exit_code, out_to) = h.call(with_sig(SIG_BALANCE_OF, &abi_encode_1_addr(to)));
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
        SIG_APPROVE,
        &abi_encode_1_addr_1_u256(spender, U256::from(10u64)),
    ));
    assert_eq!(exit_code, ExitCode::Ok);

    let (exit_code, _) = h.call(with_sig(
        SIG_APPROVE,
        &abi_encode_1_addr_1_u256(spender, U256::from(3u64)),
    ));
    assert_eq!(exit_code, ExitCode::Ok);

    let (exit_code, out) = h.call(with_sig(SIG_ALLOWANCE, &abi_encode_2_addr(owner, spender)));
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
        SIG_TRANSFER_FROM,
        &abi_encode_2_addr_1_u256(from, to, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_INSUFFICIENT_ALLOWANCE));

    let (exit_code, out_from) = h.call(with_sig(SIG_BALANCE_OF, &abi_encode_1_addr(from)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out_from, abi_word_u256(U256::from(5u64)).to_vec());

    let (exit_code, out_to) = h.call(with_sig(SIG_BALANCE_OF, &abi_encode_1_addr(to)));
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
    let (exit_code, out) = h.call(with_sig(
        SIG_TRANSFER,
        &abi_encode_1_addr_1_u256(bob, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_INTEGER_OVERFLOW));

    let (exit_code, a) = h.call(with_sig(SIG_BALANCE_OF, &abi_encode_1_addr(alice)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(a, abi_word_u256(U256::from(10u64)).to_vec());

    let (exit_code, b) = h.call(with_sig(SIG_BALANCE_OF, &abi_encode_1_addr(bob)));
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
        SIG_APPROVE,
        &abi_encode_1_addr_1_u256(spender, U256::from(5u64)),
    ));
    assert_eq!(exit_code, ExitCode::Ok);

    // Force recipient to overflow.
    set_balance(&mut h, to, U256::MAX);

    h.set_caller(spender);
    let (exit_code, out) = h.call(with_sig(
        SIG_TRANSFER_FROM,
        &abi_encode_2_addr_1_u256(from, to, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_INTEGER_OVERFLOW));

    // allowance unchanged
    let (exit_code, allow) = h.call(with_sig(SIG_ALLOWANCE, &abi_encode_2_addr(from, spender)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(allow, abi_word_u256(U256::from(5u64)).to_vec());

    // balances unchanged
    let (exit_code, b_from) = h.call(with_sig(SIG_BALANCE_OF, &abi_encode_1_addr(from)));
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
        SIG_MINT,
        &abi_encode_1_addr_1_u256(bob, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_CONTRACT_NOT_MINTABLE));

    // Deploy with minter+pauser configured, no initial supply.
    deploy_with_roles(&mut h, owner, U256::ZERO, minter, pauser);

    // Wrong caller.
    h.set_caller(attacker);
    let (exit_code, out) = h.call(with_sig(
        SIG_MINT,
        &abi_encode_1_addr_1_u256(bob, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_INVALID_MINTER));

    // Happy path.
    h.set_caller(minter);
    let (exit_code, out) = h.call(with_sig(
        SIG_MINT,
        &abi_encode_1_addr_1_u256(bob, U256::from(5u64)),
    ));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    let (exit_code, ts) = h.call(with_sig(SIG_TOTAL_SUPPLY, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(ts, abi_word_u256(U256::from(5u64)).to_vec());

    let (exit_code, b) = h.call(with_sig(SIG_BALANCE_OF, &abi_encode_1_addr(bob)));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(b, abi_word_u256(U256::from(5u64)).to_vec());

    // Pause blocks minting and must not mutate supply/balance.
    h.set_caller(pauser);
    let (exit_code, out) = h.call(with_sig(SIG_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    h.set_caller(minter);
    let (exit_code, out) = h.call(with_sig(
        SIG_MINT,
        &abi_encode_1_addr_1_u256(bob, U256::from(1u64)),
    ));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_MINTING_PAUSED));

    let (exit_code, ts2) = h.call(with_sig(SIG_TOTAL_SUPPLY, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(ts2, abi_word_u256(U256::from(5u64)).to_vec());
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
    let (exit_code, out) = h.call(with_sig(SIG_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_CONTRACT_NOT_PAUSABLE));

    // Deploy with pauser configured (minter irrelevant here).
    deploy_with_roles(&mut h, owner, U256::ZERO, Address::ZERO, pauser);

    // Wrong caller cannot pause.
    h.set_caller(attacker);
    let (exit_code, out) = h.call(with_sig(SIG_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_PAUSER_MISMATCH));

    // Pause succeeds.
    h.set_caller(pauser);
    let (exit_code, out) = h.call(with_sig(SIG_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    // Pause twice => already paused.
    let (exit_code, out) = h.call(with_sig(SIG_PAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_ALREADY_PAUSED));

    // Unpause wrong caller.
    h.set_caller(attacker);
    let (exit_code, out) = h.call(with_sig(SIG_UNPAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_PAUSER_MISMATCH));

    // Unpause succeeds.
    h.set_caller(pauser);
    let (exit_code, out) = h.call(with_sig(SIG_UNPAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Ok);
    assert_eq!(out, ok_32());

    // Unpause twice => already unpaused.
    let (exit_code, out) = h.call(with_sig(SIG_UNPAUSE, &[]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_ALREADY_UNPAUSED));
}

#[test]
fn unknown_method_returns_error() {
    let token = Address::with_last_byte(1);
    let mut h = Harness::new(token);

    let (exit_code, out) = h.call(with_sig(0xDEAD_BEEFu32, &[1, 2, 3]));
    assert_eq!(exit_code, ExitCode::Panic);
    assert_eq!(out, evm_exit_bytes(ERR_UNKNOWN_METHOD));
}
