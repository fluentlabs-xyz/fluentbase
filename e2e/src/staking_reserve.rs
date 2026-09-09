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
    address, hex, universal_token::InitialSettings, Address, Bytes, B256, GENESIS_GOVERNANCE,
    GENESIS_STAKING, U256,
};
use fluentbase_testing::EvmTestingContext;

const SYSTEM_CALLER: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");
const OWNER: Address = Address::repeat_byte(0x11);
const TOKEN: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);

/// The reserve of the honest-token fixture, and never the hostile one: the two
/// halves of this file must not be able to answer for each other.
const HONEST_RESERVE: Address = Address::repeat_byte(0x77);

/// Neither an owner nor a validator. `claimValidatorFee` is permissionless and
/// pays the OWNER, so a claim driven from here gains its caller nothing — which
/// is exactly what makes K-32 a griefing vector rather than a theft.
const STRANGER: Address = Address::repeat_byte(0x88);

/// The owner's commission, in basis points. Non-zero so the permissionless claim
/// K-32 names actually moves tokens out of the reserve.
const COMMISSION_BPS: u16 = 2_000;

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

/// What must be left of a 30M system call after a fuel-burning token has taken
/// everything it could from the reserve read.
///
/// WHERE THE NUMBER COMES FROM. `erc20_scalar_read` passes `fuel: None`
/// (`contracts/staking/src/util.rs`), so the callee is handed everything the
/// EVM will forward — and the EVM keeps 1/64 back:
/// `crates/revm/src/syscall.rs:423` forwards
/// `call_stipend_reduction(gas.remaining())`, which is
/// `gas_limit - gas_limit / 64` (revm-rwasm `8674122`,
/// `crates/context/interface/src/cfg/gas_params.rs:605-606`, divisor seeded at
/// `:211`). The burning close measures **29_579_516** gas spent out of 30M —
/// identical at committees 5, 21 and 51 — leaving **420_484**.
///
/// WHY THAT REMAINDER IS ENOUGH TODAY, AND WHY IT IS NOT A DESIGN. The reserve
/// read is the LAST outward thing the close does: `close_epoch` ends on
/// `staking::accrue_epoch` (`liveness.rs`), and inside `assign_epoch_shares` the
/// forfeit arm returns before the committee walk. So after the burn the contract
/// has one event left to emit, which is why the number does not move with the
/// committee size.
///
/// WHAT THIS FLOOR IS FOR. It is not a regression guard on 420_484 — nobody
/// chose that number. It is a tripwire on the ORDER of the legs: any work added
/// after the reserve read gets this remainder and not 30M, and the day it costs
/// more than the slack below, the close stops fitting in a system call and the
/// chain halts. Set below the measurement on purpose, so an unrelated
/// gas-schedule change of a few thousand units does not spend a session, while a
/// new leg — a storage write is 2_900 and up, an event 375 plus its data — still
/// trips it long before it can halt a block.
const BURNT_CLOSE_HEADROOM_FLOOR: u64 = 400_000;

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
        function getValidatorFee(address validator) external view returns (uint256);

        function claimValidatorFee(address validator) external;
        function claimDelegatorFee(address validator) external;

        function setBlendStipendPerEpoch(uint256 value) external;
        function setBlendReserve(address value) external;
    }

    interface IErc20 {
        function balanceOf(address holder) external view returns (uint256);
        function allowance(address holder, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
        function transfer(address to, uint256 amount) external returns (bool);
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

    /// `min(balanceOf(reserve), allowance(reserve, staking))`, read off the token
    /// itself — the same two numbers `util.rs::reserve_available` reads.
    ///
    /// Read rather than assumed for the same reason `assert_token_refuses`
    /// exists: "the epoch scored zero" cannot tell a reserve that was really
    /// short from a fixture that failed to fund one.
    fn reserve_available(&mut self) -> U256 {
        let balance = U256::from_be_slice(&erc20(
            &mut self.context,
            OWNER,
            self.token,
            IErc20::balanceOfCall {
                holder: HONEST_RESERVE,
            }
            .abi_encode(),
        ));
        let allowance = U256::from_be_slice(&erc20(
            &mut self.context,
            OWNER,
            self.token,
            IErc20::allowanceCall {
                holder: HONEST_RESERVE,
                spender: GENESIS_STAKING,
            }
            .abi_encode(),
        ));
        core::cmp::min(balance, allowance)
    }

    fn balance_of(&mut self, holder: Address) -> U256 {
        U256::from_be_slice(&erc20(
            &mut self.context,
            OWNER,
            self.token,
            IErc20::balanceOfCall { holder }.abi_encode(),
        ))
    }

    fn validator_fee(&mut self, validator: Address) -> U256 {
        let output = call(
            &mut self.context,
            OWNER,
            IStaking::getValidatorFeeCall { validator }.abi_encode(),
        );
        IStaking::getValidatorFeeCall::abi_decode_returns(&output).unwrap()
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

/// The seed stake `initialize` pulls, per validator.
const SEED_STAKE: U256 = U256::from_limbs([10_000_000_000_000_000_000, 0, 0, 0]);

fn initialize_calldata(
    token: Address,
    reserve: Address,
    committee: usize,
    commission: u16,
) -> Vec<u8> {
    IStaking::initializeCall {
        initialStakeOwner: OWNER,
        validators: (0..committee).map(validator_address).collect(),
        initialStakes: vec![SEED_STAKE; committee],
        blsPubkeysUncompressed: (0..committee)
            .map(|i| Bytes::copy_from_slice(crate::bls_vectors::pubkey(i)))
            .collect(),
        blsPopsUncompressed: (0..committee)
            .map(|i| Bytes::copy_from_slice(crate::bls_vectors::pop(i)))
            .collect(),
        peerPubkeys: (0..committee).map(peer_pubkey).collect(),
        commissionRate: commission,
        stakingToken: token,
        activeValidatorsLength: committee as u32,
        epochBlockInterval: INTERVAL as u32,
        undelegatePeriod: 7,
        minValidatorStakeAmount: TOKEN,
        minStakingAmount: TOKEN,
        dposActivationBlock: ACTIVATION,
        minUndelegateBlocks: U256::ZERO,
        blendReserve: reserve,
    }
    .abi_encode()
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
    call(
        &mut context,
        GENESIS_GOVERNANCE,
        initialize_calldata(token, HOSTILE_RESERVE, committee, 0),
    );

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

/// Genesis against the REAL BLEND token, with a reserve that holds `balance` and
/// has approved this contract for `allowance`.
///
/// Why the real token here and the hand-assembled double above: this half of the
/// rule turns on a read that SUCCEEDS and answers a number smaller than the pot,
/// and — for the K-32 case — on that number going DOWN because a claim was paid
/// out of it. The double answers a constant `2^80 - 1` to everyone it does not
/// refuse and keeps no ledger, so it can express neither. `universal-token` IS
/// the BLEND of the production path, so `balanceOf`, `allowance` and the
/// `transferFrom` a claim makes are the shipped ones rather than stand-ins.
fn honest_fixture(balance: U256, allowance: U256, commission: u16) -> Fixture {
    let mut context = EvmTestingContext::default().with_full_genesis();
    context.cfg.chain_id = crate::bls_vectors::CHAIN_ID;
    set_block(&context, ACTIVATION - 1);
    for account in [OWNER, HONEST_RESERVE, STRANGER] {
        context.add_balance(account, U256::from(10u128).pow(U256::from(20)));
    }

    let token = context.deploy_evm_tx(
        OWNER,
        InitialSettings {
            token_name: "Blend".into(),
            token_symbol: "BLEND".into(),
            decimals: 18,
            initial_supply: TOKEN * U256::from(1_000_000),
            minter: OWNER,
            pauser: Address::ZERO,
            wrapped: None,
        }
        .encode_with_prefix(),
    );

    let seed = SEED_STAKE * U256::from(COMMITTEE as u64);
    // Unlike the double, a real token refuses a `transferFrom` nobody approved —
    // and `initialize` makes one to collect the seed stakes.
    erc20(
        &mut context,
        OWNER,
        token,
        IErc20::approveCall {
            spender: GENESIS_STAKING,
            amount: seed,
        }
        .abi_encode(),
    );
    erc20(
        &mut context,
        OWNER,
        token,
        IErc20::transferCall {
            to: HONEST_RESERVE,
            amount: balance,
        }
        .abi_encode(),
    );
    erc20(
        &mut context,
        HONEST_RESERVE,
        token,
        IErc20::approveCall {
            spender: GENESIS_STAKING,
            amount: allowance,
        }
        .abi_encode(),
    );

    call(
        &mut context,
        GENESIS_GOVERNANCE,
        initialize_calldata(token, HONEST_RESERVE, COMMITTEE, commission),
    );

    let mut fixture = Fixture {
        context,
        committee: COMMITTEE,
        token,
    };
    fixture.govern(IStaking::setBlendStipendPerEpochCall { value: POT }.abi_encode());
    for _ in 0..3 {
        fixture.commit_committee();
    }
    fixture
}

/// A call to a contract that is not the staking one.
fn erc20(
    context: &mut EvmTestingContext,
    caller: Address,
    target: Address,
    input: Vec<u8>,
) -> Vec<u8> {
    let result = context.call_evm_tx(caller, target, input.into(), Some(HUGE_GAS), None);
    assert!(result.is_success(), "call to {target} failed: {result:?}");
    result.output().cloned().unwrap_or_default().to_vec()
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

// ---------------------------------------------------------------------------
// The reserve answers, and answers with LESS than the pot.
// ---------------------------------------------------------------------------

/// The forfeit branch, on the real runtime, with a reserve that READS FINE.
///
/// The three cases above are the arm where the read FAILS. This is the other
/// arm and the one the money actually turns on: `reserve_available` is
/// `min(balanceOf, allowance)` and the close forfeits the whole epoch when that
/// is one base unit under the pot. Until now it had only ever run against a unit
/// mock.
///
/// Both halves of the `min` are driven, because either one alone leaves the
/// other live: a reserve holding less than a pot behind a generous approval, and
/// a reserve holding plenty behind an approval smaller than a pot.
#[test]
fn a_reserve_that_answers_with_less_than_the_pot_forfeits_the_epoch() {
    let short = POT - U256::from(1);
    let plenty = POT * U256::from(100);
    // The top-up repairs the half that was short, and only that half, so each
    // case's recovery proves the same term it was short on.
    type TopUp = fn(&mut Fixture);
    let fund: TopUp = |fixture| {
        let calldata = IErc20::transferCall {
            to: HONEST_RESERVE,
            amount: plenty_amount(),
        }
        .abi_encode();
        let token = fixture.token;
        erc20(&mut fixture.context, OWNER, token, calldata);
    };
    let approve: TopUp = |fixture| {
        let calldata = IErc20::approveCall {
            spender: GENESIS_STAKING,
            amount: plenty_amount(),
        }
        .abi_encode();
        let token = fixture.token;
        erc20(&mut fixture.context, HONEST_RESERVE, token, calldata);
    };

    for (label, balance, allowance, top_up) in [
        ("balance one unit short of the pot", short, plenty, fund),
        ("approval one unit short of the pot", plenty, short, approve),
    ] {
        let mut fixture = honest_fixture(balance, allowance, 0);
        assert_eq!(
            fixture.reserve_available(),
            short,
            "{label}: the reserve must really be short, or the zero below means \
             nothing"
        );

        fixture.record(first_block(0));
        fixture.finish_epoch(0);
        let close = fixture.record(first_block(1));

        assert!(
            close.succeeded,
            "{label}: the close must survive the forfeit"
        );
        assert_eq!(
            accrued(&close, 0),
            U256::ZERO,
            "{label}: one unit short forfeits the whole epoch"
        );
        assert_eq!(fixture.epoch_rewards(0), U256::ZERO, "{label}");

        // A claim for the forfeited epoch pays nothing and takes nothing.
        let seat = validator_address(0);
        let reserve_before = fixture.balance_of(HONEST_RESERVE);
        assert_eq!(fixture.validator_fee(seat), U256::ZERO, "{label}");
        measure(
            &mut fixture.context,
            STRANGER,
            IStaking::claimValidatorFeeCall { validator: seat }.abi_encode(),
        );
        measure(
            &mut fixture.context,
            seat,
            IStaking::claimDelegatorFeeCall { validator: seat }.abi_encode(),
        );
        assert_eq!(
            fixture.balance_of(HONEST_RESERVE),
            reserve_before,
            "{label}: a claim against a forfeited epoch must move nothing"
        );

        // The treasury repairs the short half BEFORE the next close. Epoch 1 is
        // funded; epoch 0 does not come back.
        top_up(&mut fixture);
        assert!(fixture.reserve_available() >= POT, "{label}");
        fixture.finish_epoch(1);
        fixture.commit_committee();
        let recovered = fixture.record(first_block(2));
        assert!(recovered.succeeded, "{label}");
        assert_eq!(accrued(&recovered, 1), fixture.epoch_rewards(1), "{label}");
        assert!(
            fixture.epoch_rewards(1) > U256::ZERO,
            "{label}: the epoch that closes after the top-up is funded"
        );
        assert_eq!(
            fixture.epoch_rewards(0),
            U256::ZERO,
            "{label}: and the forfeited one stays dead"
        );
    }
}

/// `plenty` as a function, because a `fn` pointer cannot close over a local.
fn plenty_amount() -> U256 {
    POT * U256::from(100)
}

/// K-32 on the real runtime: a permissionless claim between two closes drops the
/// reserve under the pot, and the epoch closing next burns for it.
///
/// Every term here is the shipped one. The reserve holds exactly one pot, so any
/// outflow at all takes it under; the claim is `claimValidatorFee`, which any
/// address may send and which pays the OWNER, so the caller gains nothing and
/// spends only gas; and the burn is the same all-or-nothing forfeit as above.
/// The audit records this as a vector the contract cannot fix — this is the
/// vector actually run.
#[test]
fn a_claim_between_two_closes_puts_the_reserve_under_the_pot_and_burns_the_epoch() {
    let mut fixture = honest_fixture(POT, POT, COMMISSION_BPS);
    assert_eq!(
        fixture.reserve_available(),
        POT,
        "the reserve must start on exactly one pot: the vector is that ANY \
         outflow takes it under"
    );

    // Epoch 0 closes against a reserve that covers it, and accrues.
    fixture.record(first_block(0));
    fixture.finish_epoch(0);
    let first = fixture.record(first_block(1));
    assert!(first.succeeded);
    let credited = fixture.epoch_rewards(0);
    assert_eq!(accrued(&first, 0), credited);
    assert!(
        credited > U256::ZERO,
        "epoch 0 must be funded, or the burn below is not caused by the claim"
    );

    // A stranger claims the seat owner's commission before the next boundary.
    let seat = validator_address(0);
    let commission = fixture.validator_fee(seat);
    assert!(
        commission > U256::ZERO,
        "the commission must be non-zero, or the claim moves nothing"
    );
    let owner_before = fixture.balance_of(seat);
    measure(
        &mut fixture.context,
        STRANGER,
        IStaking::claimValidatorFeeCall { validator: seat }.abi_encode(),
    );
    assert_eq!(
        fixture.balance_of(seat) - owner_before,
        commission,
        "the claim pays the OWNER, not its caller"
    );
    assert_eq!(
        fixture.balance_of(STRANGER),
        U256::ZERO,
        "and the caller is paid nothing for making it"
    );
    let available = fixture.reserve_available();
    assert!(
        available < POT,
        "the claim must have taken the reserve under one pot, it holds {available}"
    );

    // Epoch 1 closes short and burns — permanently.
    fixture.finish_epoch(1);
    fixture.commit_committee();
    let second = fixture.record(first_block(2));
    assert!(second.succeeded, "the close survives the forfeit");
    assert_eq!(
        accrued(&second, 1),
        U256::ZERO,
        "a reserve one claim under the pot forfeits the closing epoch whole"
    );
    assert_eq!(fixture.epoch_rewards(1), U256::ZERO);

    // The treasury refills afterwards. Epoch 2 is funded; epoch 1 is gone.
    let calldata = IErc20::transferCall {
        to: HONEST_RESERVE,
        amount: POT * U256::from(10),
    }
    .abi_encode();
    let token = fixture.token;
    erc20(&mut fixture.context, OWNER, token, calldata);
    erc20(
        &mut fixture.context,
        HONEST_RESERVE,
        token,
        IErc20::approveCall {
            spender: GENESIS_STAKING,
            amount: POT * U256::from(10),
        }
        .abi_encode(),
    );
    fixture.finish_epoch(2);
    fixture.commit_committee();
    let third = fixture.record(first_block(3));
    assert!(third.succeeded);
    assert!(
        fixture.epoch_rewards(2) > U256::ZERO,
        "the epoch closing after the refill is funded"
    );
    assert_eq!(
        fixture.epoch_rewards(1),
        U256::ZERO,
        "and the burnt one is not revived by it"
    );
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
///
/// It ASSERTS the burning arm rather than printing SURVIVED/FAILED beside it.
/// The earlier version reported and asserted nothing, on the ground that the
/// outcome is a fact rather than a chosen number — but the outcome is the fact
/// that matters most here, because "FAILED" spells a chain halt reachable by a
/// hostile reserve. The floor on the remainder carries the second half of it:
/// see `BURNT_CLOSE_HEADROOM_FLOOR` for what it is and is not guarding. No fuel
/// cap is imposed on the read itself; that decision stands.
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
        let headroom = SYSTEM_CALL_BUDGET.saturating_sub(burnt_close.frame_gas);
        println!(
            "    committee {committee:>2}: honest close {:>9} gas ({}), burning close {:>9} gas, \
             headroom {:>7} — {}",
            honest_close.frame_gas,
            if honest_close.succeeded {
                "ok"
            } else {
                "FAILED"
            },
            burnt_close.frame_gas,
            headroom,
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
        assert!(
            burnt_close.succeeded,
            "committee {committee}: a fuel-burning token in the reserve slot \
             HALTS THE BLOCK. The close is a pre-execution system call, so this \
             is a chain stop that no transaction can repair, reachable by any \
             address that governance points the reserve at."
        );
        // ANTI-VACUUM, and it is what the two assertions above cannot do for
        // themselves: both are ONE-SIDED. A token that quietly stopped burning
        // would leave the close successful and the headroom larger, and every
        // line below would pass while measuring a fixture that no longer poses
        // the question. `assert_token_refuses` runs only in the unbounded arm
        // and only at `COMMITTEE`; these two cover the sizes this loop exists
        // for.
        assert!(
            burnt_close.frame_gas > honest_close.frame_gas,
            "committee {committee}: the burning close spent {} gas against the \
             honest close's {} — the token did not burn, so nothing here is \
             measuring a burn",
            burnt_close.frame_gas,
            honest_close.frame_gas
        );
        assert_eq!(
            accrued(&burnt_close, 0),
            U256::ZERO,
            "committee {committee}: the burnt close must have FORFEITED the \
             epoch. A non-zero accrual means the reserve read came back, which \
             means the token answered and the burn never happened"
        );
        assert!(
            accrued(&honest_close, 0) > U256::ZERO,
            "committee {committee}: the honest close must have FUNDED the epoch, \
             or the zero beside it is not evidence of a refused read"
        );
        assert!(
            headroom >= BURNT_CLOSE_HEADROOM_FLOOR,
            "committee {committee}: only {headroom} gas of the 30M system call \
             survived the burning reserve read, under the {BURNT_CLOSE_HEADROOM_FLOOR} \
             floor. The reserve read is the LAST leg of the close and the EVM \
             hands the callee all but 1/64 of what is left, so whatever now runs \
             after it is living on that remainder — see \
             BURNT_CLOSE_HEADROOM_FLOOR."
        );
    }

    assert!(
        unbounded > honest,
        "the burning read must cost the close something, or this measurement is \
         not measuring the burn"
    );
}
