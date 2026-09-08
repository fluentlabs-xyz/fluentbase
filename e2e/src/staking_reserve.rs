//! The epoch close against a BLEND token that refuses to answer.
//!
//! `close_epoch` is a PRE-EXECUTION system call: an error propagated out of it
//! is a block-execution failure on every node at once, repairable by no
//! transaction. The contract therefore prices an epoch off
//! `min(balanceOf(reserve), allowance(reserve, staking))` read through two
//! `static_call`s, and treats EVERY failure of the callee as zero
//! (`contracts/staking/src/util.rs`, `erc20_scalar_read`). The doc comment on
//! `reserve_available` says that rule was confirmed on the real rWasm blob —
//! but the two e2e tests it refers to were deleted with the self-call they
//! covered, and what survived proves it only for `call`, never for
//! `static_call`, and only against a unit host in which `static_call` IS `call`.
//!
//! This file is that proof, on real rWasm, in the three shapes a third-party
//! token can refuse in: a revert, a frame that burns every unit of fuel handed
//! to it, and an answer too short to decode. Each one must leave the close
//! SUCCESSFUL, the epoch scored at zero, the accrual event emitted, and the
//! next epoch payable again once the reserve answers honestly.
//!
//! Self-contained, in the same sense as `staking_cost.rs`: it carries its own
//! `sol!` interface and its own fixture and references nothing outside this file
//! but the shared BLS vectors.

use crate::EvmTestingContextWithGenesis;
use alloy_sol_types::{sol, SolCall, SolEvent};
use fluentbase_sdk::{
    address, hex, Address, Bytes, B256, GENESIS_GOVERNANCE, GENESIS_STAKING, U256,
};
use fluentbase_testing::EvmTestingContext;

const SYSTEM_CALLER: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");
const OWNER: Address = Address::repeat_byte(0x11);
const TOKEN: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);

/// The reserve address the token below refuses to answer about.
///
/// The refusal is keyed on the ADDRESS the close asks about rather than on the
/// token, because the contract has no staking-token setter: `setBlendReserve` is
/// the only way to move a live chain from a reserve this token refuses to one it
/// answers for, and the control arm of every test here needs exactly that move.
const HOSTILE_RESERVE: Address = Address::repeat_byte(0x99);

/// Far above any budget these tests configure: a measurement must not be clipped
/// by the transaction limit. Matches `staking_cost.rs`.
const HUGE_GAS: u64 = 2_000_000_000;

/// Per-call budget of a real system call: `revm-rwasm`
/// `crates/handler/src/system_call.rs` builds every one with
/// `gas_limit(30_000_000)`. Nothing in this harness enforces it, so the one test
/// that cares about it imposes it by hand.
const SYSTEM_CALL_BUDGET: u64 = 30_000_000;

/// Committee cap and roster size the property tests use.
///
/// Five seats against a floor of four (`MIN_COMMITTEE_LENGTH`), so the committee
/// is a real one and the close has weights to split a pot across, and small
/// enough that the property is read off a cheap close.
const COMMITTEE: usize = 5;
/// The default and the maximum a chain can configure (`consts.rs`,
/// `DEFAULT_ACTIVE_VALIDATORS_LENGTH` and `MAX_ACTIVE_VALIDATORS_LENGTH`). The
/// budget test runs at both, because what the close can still afford after the
/// burn depends on how much work it has left to do.
const PRODUCTION_COMMITTEES: [usize; 2] = [21, 51];
const INTERVAL: u64 = 5;
/// Must be a multiple of `INTERVAL`, which the config validator enforces.
const ACTIVATION: u64 = INTERVAL * 20;

/// One epoch's stipend. Far below the `2^80 - 1` the token answers honestly with,
/// so an honest read always covers it and a refused read never does.
const POT: U256 = U256::from_limbs([5_000_000_000_000_000_000, 0, 0, 0]);

sol! {
    /// The close's accrual fact: what the epoch owes, decided at its close.
    event EpochBlendRewardsCommitted(uint64 indexed epoch, uint256 blendAmount);

    interface IStaking {
        function initialize(
            address initialStakeOwner,
            address[] validators,
            uint256[] initialStakes,
            bytes[] blsPubkeysUncompressed,
            bytes[] blsPopsUncompressed,
            bytes32[] peerPubkeys,
            uint16 commissionRate,
            address stakingToken,
            uint32 activeValidatorsLength,
            uint32 epochBlockInterval,
            uint32 undelegatePeriod,
            uint256 minValidatorStakeAmount,
            uint256 minStakingAmount,
            uint64 dposActivationBlock,
            uint256 minUndelegateBlocks,
            address blendReserve
        ) external;

        function recordProduction(uint8 leaderIndex) external;
        function commitEpochCommittee() external;

        function blocksInEpoch(uint64 epoch) external view returns (uint32);
        function getEpochRewards(uint64 epoch) external view returns (uint256);

        function setBlendStipendPerEpoch(uint256 value) external;
        function setBlendReserve(address value) external;
    }

    interface IErc20 {
        function balanceOf(address holder) external view returns (uint256);
    }
}

// ---------------------------------------------------------------------------
// The token that refuses.
// ---------------------------------------------------------------------------

/// How the token fails the close's two reserve reads.
#[derive(Clone, Copy, Debug)]
enum Refusal {
    /// The callee reverts. The commonest shape: a token with a blacklist, a
    /// paused token, a proxy pointing at nothing.
    Revert,
    /// The callee consumes every unit of fuel it was given and returns nothing.
    /// The close passes `None` as the fuel limit, so "every unit it was given"
    /// is whatever the runtime forwards.
    BurnFuel,
    /// The callee answers sixteen bytes where a 32-byte word is due. A return
    /// too short to decode, which is not the same failure as a call that failed.
    TruncatedWord,
}

/// ERC-20 stand-in that refuses only about `HOSTILE_RESERVE`.
///
/// `balanceOf(holder)` and `allowance(holder, spender)` both key on the FIRST
/// argument. About `HOSTILE_RESERVE` they perform `refusal`; about anybody else
/// they answer `2^80 - 1`, far above `POT`. Every other selector — the
/// `transferFrom` that `initialize` makes to collect the seed stakes included —
/// returns `true`, so nothing but the reserve read is under test.
///
/// Assembled rather than hand-laid-out in hex: the two jump destinations are
/// computed from the parts, so a change to one part cannot silently point a
/// jump into the middle of an instruction.
fn refusing_token(refusal: Refusal) -> Vec<u8> {
    let refusal_body: Vec<u8> = match refusal {
        // PUSH1 0, PUSH1 0, REVERT
        Refusal::Revert => hex!("60006000fd").to_vec(),
        // PUSH1 0 (value), PUSH4 0xffffffff (offset), MSTORE — a memory
        // expansion to ~4 GiB. One opcode, priced past any budget that exists,
        // so the frame ends out of gas with everything it was given spent.
        Refusal::BurnFuel => hex!("600063ffffffff52").to_vec(),
        // PUSH32 ~0, PUSH1 0, MSTORE, PUSH1 16, PUSH1 0, RETURN — sixteen bytes
        // of 0xff. Non-zero on purpose: a decoder that accepted a short word
        // would read an enormous balance and fund the epoch, which is the
        // failure this arm exists to catch.
        Refusal::TruncatedWord => hex!(
            "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            "60005260106000f3"
        )
        .to_vec(),
    };

    // Dispatch: sel = calldata[0..4]; balanceOf and allowance go to CHECK,
    // anything else returns 1.
    let check = 36u8;
    let mut code = Vec::new();
    code.extend_from_slice(&hex!("60003560e01c")); // PUSH1 0 CALLDATALOAD PUSH1 0xe0 SHR
    code.extend_from_slice(&hex!("806370a0823114")); // DUP1 PUSH4 balanceOf(address) EQ
    code.extend_from_slice(&[0x60, check, 0x57]); // PUSH1 CHECK JUMPI
    code.extend_from_slice(&hex!("8063dd62ed3e14")); // DUP1 PUSH4 allowance(address,address) EQ
    code.extend_from_slice(&[0x60, check, 0x57]); // PUSH1 CHECK JUMPI
    code.extend_from_slice(&hex!("600160005260206000f3")); // return 1
    assert_eq!(code.len(), check as usize, "CHECK label moved");

    // CHECK: is the first argument HOSTILE_RESERVE?
    code.push(0x5b); // JUMPDEST
    code.extend_from_slice(&hex!("600435")); // PUSH1 4 CALLDATALOAD
    code.push(0x73); // PUSH20
    code.extend_from_slice(HOSTILE_RESERVE.as_slice());
    code.push(0x14); // EQ
    let honest_tail = hex!("69ffffffffffffffffffff60005260206000f3"); // return 2^80-1
    let refuse = u8::try_from(code.len() + 3 + honest_tail.len())
        .expect("the token runtime must stay inside one-byte jump destinations");
    code.extend_from_slice(&[0x60, refuse, 0x57]); // PUSH1 REFUSE JUMPI
    code.extend_from_slice(&honest_tail);
    assert_eq!(code.len(), refuse as usize, "REFUSE label moved");

    code.push(0x5b); // JUMPDEST
    code.extend_from_slice(&refusal_body);
    assert!(code.len() <= 0xff, "runtime too long for `deploy_runtime`");
    code
}

/// Deploys runtime bytecode behind a minimal constructor prologue.
///
/// Same shape as `staking_cost.rs`'s: there is no solc in this tree.
fn deploy_runtime(context: &mut EvmTestingContext, runtime: &[u8]) -> Address {
    assert!(runtime.len() <= 0xff);
    let mut init = vec![
        0x60,
        runtime.len() as u8, // PUSH1 len
        0x80,                // DUP1
        0x60,
        0x0b, // PUSH1 11 (offset of the runtime inside this initcode)
        0x60,
        0x00, // PUSH1 0
        0x39, // CODECOPY
        0x60,
        0x00, // PUSH1 0
        0xf3, // RETURN
    ];
    init.extend_from_slice(runtime);
    context.deploy_evm_tx(OWNER, Bytes::from(init))
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn set_block(context: &EvmTestingContext, number: u64) {
    let _ = context.sdk.clone().with_block_number(number);
}

/// The EVM transaction intrinsic the harness charges and a system call does not.
fn intrinsic(calldata: &[u8]) -> u64 {
    let mut cost = 21_000u64;
    for byte in calldata {
        cost += if *byte == 0 { 4 } else { 16 };
    }
    cost
}

struct Measured {
    succeeded: bool,
    output: Vec<u8>,
    /// Gas spent during execution, less the transaction intrinsic: what the
    /// system call itself would spend against its 30M budget.
    frame_gas: u64,
    /// `(topic0, data)` of every log the call COMMITTED. A log written inside a
    /// discarded frame is absent, which is what lets an assertion tell an
    /// accrual that happened from one that was rolled back.
    logs: Vec<(B256, Vec<u8>)>,
}

impl Measured {
    /// Payloads of the committed logs whose `topic0` is `signature_hash`.
    fn event_data(&self, signature_hash: B256) -> Vec<&[u8]> {
        self.logs
            .iter()
            .filter(|(topic, _)| *topic == signature_hash)
            .map(|(_, data)| data.as_slice())
            .collect()
    }
}

fn measure_with_limit(
    context: &mut EvmTestingContext,
    caller: Address,
    input: Vec<u8>,
    gas_limit: u64,
) -> Measured {
    let overhead = intrinsic(&input);
    let result = context.call_evm_tx(caller, GENESIS_STAKING, input.into(), Some(gas_limit), None);
    let tx_gas = result.gas().total_gas_spent();
    Measured {
        succeeded: result.is_success(),
        output: result.output().cloned().unwrap_or_default().to_vec(),
        frame_gas: tx_gas.saturating_sub(overhead),
        logs: result
            .logs()
            .iter()
            .filter_map(|log| {
                log.topics()
                    .first()
                    .map(|topic| (*topic, log.data.data.to_vec()))
            })
            .collect(),
    }
}

fn measure(context: &mut EvmTestingContext, caller: Address, input: Vec<u8>) -> Measured {
    let measured = measure_with_limit(context, caller, input, HUGE_GAS);
    assert!(measured.succeeded, "call failed");
    measured
}

fn call(context: &mut EvmTestingContext, caller: Address, input: Vec<u8>) -> Vec<u8> {
    measure(context, caller, input).output
}

fn validator_address(index: usize) -> Address {
    let mut bytes = [0u8; 20];
    bytes[0] = 0xa0;
    bytes[19] = index as u8;
    Address::from(bytes)
}

/// Strictly ascending in `index`, the order `commitEpochCommittee` sorts into.
fn peer_pubkey(index: usize) -> B256 {
    let mut bytes = [0u8; 32];
    bytes[31] = (index + 1) as u8;
    B256::from(bytes)
}

struct Fixture {
    context: EvmTestingContext,
    committee: usize,
    token: Address,
}

impl Fixture {
    fn commit_committee(&mut self) {
        measure(
            &mut self.context,
            SYSTEM_CALLER,
            IStaking::commitEpochCommitteeCall {}.abi_encode(),
        );
    }

    fn record(&mut self, block: u64) -> Measured {
        set_block(&self.context, block);
        measure(
            &mut self.context,
            SYSTEM_CALLER,
            IStaking::recordProductionCall { leaderIndex: 0 }.abi_encode(),
        )
    }

    /// `recordProduction` under an explicit limit, without demanding success.
    fn record_within(&mut self, block: u64, gas_limit: u64) -> Measured {
        set_block(&self.context, block);
        measure_with_limit(
            &mut self.context,
            SYSTEM_CALLER,
            IStaking::recordProductionCall { leaderIndex: 0 }.abi_encode(),
            gas_limit,
        )
    }

    /// Asks the token directly what it does about `HOSTILE_RESERVE`, and asserts
    /// it is the refusal this fixture was built for.
    ///
    /// A double that quietly stopped refusing would make every assertion below
    /// pass while proving nothing, and "the epoch scored zero" cannot tell the
    /// two apart on its own. So the double is interrogated before it is trusted.
    fn assert_token_refuses(&mut self, refusal: Refusal) {
        let calldata = IErc20::balanceOfCall {
            holder: HOSTILE_RESERVE,
        }
        .abi_encode();
        let result = self.context.call_evm_tx(
            OWNER,
            self.token,
            calldata.clone().into(),
            Some(HUGE_GAS),
            None,
        );
        let returned = result.output().cloned().unwrap_or_default().to_vec();
        match refusal {
            Refusal::Revert => assert!(
                !result.is_success() && returned.is_empty(),
                "the token must revert about the hostile reserve, got {result:?}"
            ),
            Refusal::BurnFuel => {
                assert!(
                    !result.is_success(),
                    "the token must not return about the hostile reserve, got {result:?}"
                );
                let spent = result.gas().total_gas_spent() - intrinsic(&calldata);
                assert!(
                    spent > HUGE_GAS / 2,
                    "the token must burn the fuel it was given, spent only {spent}"
                );
            }
            Refusal::TruncatedWord => {
                assert!(result.is_success(), "the token must RETURN, not fail");
                assert_eq!(
                    returned,
                    vec![0xffu8; 16],
                    "the token must answer sixteen non-zero bytes where a word is due"
                );
            }
        }
        // And that it answers honestly about anybody else, so the control arm
        // below is a control and not a second refusal.
        let honest = self.context.call_evm_tx(
            OWNER,
            self.token,
            IErc20::balanceOfCall { holder: OWNER }.abi_encode().into(),
            Some(HUGE_GAS),
            None,
        );
        assert!(honest.is_success());
        assert_eq!(
            U256::from_be_slice(&honest.output().cloned().unwrap_or_default()),
            (U256::from(1) << 80) - U256::from(1),
            "the token must answer 2^80-1 about an address it does not refuse"
        );
    }

    fn govern(&mut self, calldata: Vec<u8>) {
        call(&mut self.context, GENESIS_GOVERNANCE, calldata);
    }

    fn blocks_in_epoch(&mut self, epoch: u64) -> u32 {
        let output = call(
            &mut self.context,
            OWNER,
            IStaking::blocksInEpochCall { epoch }.abi_encode(),
        );
        IStaking::blocksInEpochCall::abi_decode_returns(&output).unwrap()
    }

    fn epoch_rewards(&mut self, epoch: u64) -> U256 {
        let output = call(
            &mut self.context,
            OWNER,
            IStaking::getEpochRewardsCall { epoch }.abi_encode(),
        );
        IStaking::getEpochRewardsCall::abi_decode_returns(&output).unwrap()
    }

    /// Records every block of `epoch` but its first, which the caller has
    /// already recorded, and asserts the epoch is fully recorded afterwards.
    ///
    /// Load-bearing: `accrue_epoch` scores an epoch with ZERO recorded blocks at
    /// zero before it reads anything, so a partially recorded epoch would make
    /// every assertion in this file pass for the wrong reason.
    fn finish_epoch(&mut self, epoch: u64) {
        for offset in 1..INTERVAL {
            self.record(first_block(epoch) + offset);
        }
        assert_eq!(
            self.blocks_in_epoch(epoch),
            INTERVAL as u32,
            "epoch {epoch} must be fully recorded or its close scores zero for want of work"
        );
    }
}

/// First block of `epoch`.
fn first_block(epoch: u64) -> u64 {
    ACTIVATION + epoch * INTERVAL
}

/// Genesis with `COMMITTEE` active, equally staked, key-carrying validators and
/// the BLEND reserve pointed at an address the token refuses to answer about.
fn fixture(refusal: Refusal, committee: usize) -> Fixture {
    let mut context = EvmTestingContext::default().with_full_genesis();
    // Pinned, not inherited: each proof of possession below was signed under
    // this chain id, and a mismatch fails the whole genesis.
    context.cfg.chain_id = crate::bls_vectors::CHAIN_ID;
    set_block(&context, ACTIVATION - 1);

    let token = deploy_runtime(&mut context, &refusing_token(refusal));

    let validators: Vec<Address> = (0..committee).map(validator_address).collect();
    let calldata = IStaking::initializeCall {
        initialStakeOwner: OWNER,
        validators,
        initialStakes: vec![TOKEN * U256::from(10); committee],
        blsPubkeysUncompressed: (0..committee)
            .map(|i| Bytes::copy_from_slice(crate::bls_vectors::pubkey(i)))
            .collect(),
        blsPopsUncompressed: (0..committee)
            .map(|i| Bytes::copy_from_slice(crate::bls_vectors::pop(i)))
            .collect(),
        peerPubkeys: (0..committee).map(peer_pubkey).collect(),
        commissionRate: 0,
        stakingToken: token,
        activeValidatorsLength: committee as u32,
        epochBlockInterval: INTERVAL as u32,
        undelegatePeriod: 7,
        minValidatorStakeAmount: TOKEN,
        minStakingAmount: TOKEN,
        dposActivationBlock: ACTIVATION,
        minUndelegateBlocks: U256::ZERO,
        blendReserve: HOSTILE_RESERVE,
    }
    .abi_encode();
    call(&mut context, GENESIS_GOVERNANCE, calldata);

    let mut fixture = Fixture {
        context,
        committee,
        token,
    };
    fixture.govern(IStaking::setBlendStipendPerEpochCall { value: POT }.abi_encode());
    // Committees 0..=2 at once; the lookahead is two epochs.
    for _ in 0..3 {
        fixture.commit_committee();
    }
    fixture
}

/// The one committed accrual of a close, decoded.
fn accrued(close: &Measured, epoch: u64) -> U256 {
    let payloads = close.event_data(EpochBlendRewardsCommitted::SIGNATURE_HASH);
    assert_eq!(
        payloads.len(),
        1,
        "close(epoch {epoch}) must commit exactly one accrual event"
    );
    assert_eq!(payloads[0].len(), 32, "the event carries one uint256");
    U256::from_be_slice(payloads[0])
}

// ---------------------------------------------------------------------------
// The property
// ---------------------------------------------------------------------------

/// Drives epochs 0 and 1 through the close, the first against a reserve the
/// token refuses to answer about and the second against one it answers for.
///
/// Returns the frame gas of the refused close, which is the figure the fuel
/// question turns on.
fn a_refused_reserve_read_scores_zero_and_the_next_epoch_recovers(refusal: Refusal) -> u64 {
    let mut fixture = fixture(refusal, COMMITTEE);
    fixture.assert_token_refuses(refusal);

    // --- epoch 0, priced against a reserve the token will not answer about.
    fixture.record(first_block(0));
    fixture.finish_epoch(0);
    let close = fixture.record(first_block(1));

    println!("\n=== {refusal:?} ===");
    println!("  close(epoch 0) frame gas: {}", close.frame_gas);

    assert!(
        close.succeeded,
        "the close must survive a reserve read that {refusal:?}: it is a \
         pre-execution system call, and a propagated failure halts every node"
    );
    assert_eq!(
        accrued(&close, 0),
        U256::ZERO,
        "a reserve that could not be read must price the epoch at zero"
    );
    assert_eq!(
        fixture.epoch_rewards(0),
        U256::ZERO,
        "and the stored credit must agree with the event"
    );

    // --- epoch 1, after the reserve moves to an address the token answers for.
    fixture.govern(IStaking::setBlendReserveCall { value: OWNER }.abi_encode());
    fixture.finish_epoch(1);
    fixture.commit_committee();
    let recovered = fixture.record(first_block(2));
    assert!(recovered.succeeded);
    assert_eq!(
        accrued(&recovered, 1),
        fixture.epoch_rewards(1),
        "the recovered close must store what it announced"
    );
    assert!(
        fixture.epoch_rewards(1) > U256::ZERO,
        "an answerable reserve must fund the next epoch: a close that scored \
         zero for one epoch must not have poisoned the mechanism"
    );

    close.frame_gas
}

#[test]
fn a_reverting_reserve_read_closes_the_epoch_at_zero() {
    a_refused_reserve_read_scores_zero_and_the_next_epoch_recovers(Refusal::Revert);
}

#[test]
fn a_fuel_burning_reserve_read_closes_the_epoch_at_zero() {
    a_refused_reserve_read_scores_zero_and_the_next_epoch_recovers(Refusal::BurnFuel);
}

#[test]
fn a_truncated_reserve_read_closes_the_epoch_at_zero() {
    a_refused_reserve_read_scores_zero_and_the_next_epoch_recovers(Refusal::TruncatedWord);
}

/// What the fuel-burning token costs the close, and whether a real system call
/// could afford it.
///
/// The close passes `None` as the fuel limit (`util.rs`, `erc20_scalar_read`),
/// so the callee is handed whatever the runtime forwards out of the frame's
/// remaining gas. The three tests above run under `HUGE_GAS`, which is two
/// billion — far more than any node ever gives a system call. This one imposes
/// the real budget, 30M, and reports what happens, because the answer is a fact
/// about the shipped contract and not about this harness.
///
/// It runs the budgeted close at three committee sizes on purpose. What the
/// burner leaves behind is a FRACTION of what was unspent when the read was
/// made, so whether the close can finish depends on how much work it has left
/// — and the close's remaining work grows with the committee it splits the pot
/// across. A five-seat committee says nothing about a fifty-one-seat one.
#[test]
fn the_fuel_burning_read_against_the_production_system_call_budget() {
    let unbounded =
        a_refused_reserve_read_scores_zero_and_the_next_epoch_recovers(Refusal::BurnFuel);
    let honest = {
        let mut fixture = fixture(Refusal::Revert, COMMITTEE);
        fixture.govern(IStaking::setBlendReserveCall { value: OWNER }.abi_encode());
        fixture.record(first_block(0));
        fixture.finish_epoch(0);
        fixture.record(first_block(1)).frame_gas
    };

    println!("\n=== the fuel a burning reserve read costs the close ===");
    println!("  committee {COMMITTEE}, honest reserve read : {honest:>12}");
    println!(
        "  committee {COMMITTEE}, burning reserve read: {unbounded:>12}  (tx limit {HUGE_GAS})"
    );
    println!(
        "  burned by the two reads             : {:>12}",
        unbounded.saturating_sub(honest)
    );
    println!("  production system-call budget       : {SYSTEM_CALL_BUDGET:>12}");

    // The same close under the budget a node actually gives it, at the sizes a
    // production chain runs at.
    println!("\n  under a 30M frame budget:");
    let budget = SYSTEM_CALL_BUDGET
        + intrinsic(&IStaking::recordProductionCall { leaderIndex: 0 }.abi_encode());
    let mut committees = vec![COMMITTEE];
    committees.extend_from_slice(&PRODUCTION_COMMITTEES);
    for committee in committees {
        let mut honest = fixture(Refusal::Revert, committee);
        honest.govern(IStaking::setBlendReserveCall { value: OWNER }.abi_encode());
        honest.record(first_block(0));
        honest.finish_epoch(0);
        let honest_close = honest.record_within(first_block(1), budget);

        let mut burning = fixture(Refusal::BurnFuel, committee);
        burning.record(first_block(0));
        burning.finish_epoch(0);
        let burnt_close = burning.record_within(first_block(1), budget);
        println!(
            "    committee {committee:>2}: honest close {:>9} gas ({}), burning close {:>9} gas — {}",
            honest_close.frame_gas,
            if honest_close.succeeded { "ok" } else { "FAILED" },
            burnt_close.frame_gas,
            if burnt_close.succeeded {
                "SURVIVED"
            } else {
                "FAILED: a burner token in the reserve slot halts the block"
            },
        );
        assert!(
            honest_close.succeeded,
            "committee {committee}: the honest close must fit in 30M, or the \
             burning arm beside it measures the wrong thing"
        );
    }

    // No assertion on the burning arm's outcome at any size. This test reports a
    // measurement: whichever way it comes out is a fact about the shipped
    // contract, and pinning it would turn a finding into a regression guard for
    // a number nobody chose.
    assert!(
        unbounded > honest,
        "the burning read must cost the close something, or this measurement is \
         not measuring the burn"
    );
}
